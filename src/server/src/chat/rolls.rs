//! Roll execution core: the ONLY untrusted-notation execution path in chat.
//!
//! `execute_roll`/`validate_formula` are the sole entry points from the chat
//! ingest stage. The caller's formula may be a TEMPLATE carrying dotted
//! references (`1d20 + str`): the first step rewrites it through
//! `crate::formula::resolve_notation_template`, resolving each reference
//! against the roll's host document (the bound actor's `system` band, or the
//! no-host resolver when nothing is bound — a referencing roll then fails
//! `unknown-ref`), so the server — not the sending client — decides what a
//! reference read. The substituted notation then parses against the dice
//! crate's `notation::parse`, enforces the wire-boundary caps below
//! (`MAX_ROLL_DICE`,
//! `MAX_ROLL_RECORDS`, `MAX_EXPERTISE`, `MAX_DIE_SIDES` — the dice crate's own
//! types stay unbounded, so an untrusted formula has no size limit until it
//! crosses this boundary), then rolls/evaluates. The dice crate itself stays
//! pure — it has no notion of these caps, reference resolution, entropy
//! seeding, or chat settings;
//! those are transport policy that belongs here, not in `dice/`.
//!
//! `execute_roll`/`validate_formula`/`BodyChunk`/`scan_body` are called from
//! `handle_send_message`'s roll stage — the sole ingest path
//! that may execute untrusted dice notation.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use uuid::Uuid;

use super::DocLinkTarget;
use crate::data::document::Document;
use crate::dice::notation::{self, ParseContext, ParseError};
use crate::dice::outcome::RollOutcome;
use crate::dice::rng::NoiseRng;
use crate::dice::spec::{DiceGroup, DieKind, Expr, Mode, RollSpec};
use crate::dice::{eval, roll};
use crate::formula::resolver::{NoHostResolver, SystemLeafResolver};

/// Sum of `DiceGroup.count` across one parsed `Expr` — rejects the unbounded-
/// `count` DoS/overflow class at the source, before any die is ever rolled.
pub(crate) const MAX_ROLL_DICE: u32 = 100;
/// Post-roll guard on `RawRoll.records.len()` — explosion chains are random;
/// `CHAIN_CAP=100/die` x `MAX_ROLL_DICE` dice could reach far past this, so an
/// oversized result rejects the roll outright rather than allocating it.
pub(crate) const MAX_ROLL_RECORDS: usize = 1_000;
/// Cap on `SuccessConfig.expertise` — the DP allocator's cost is `O(N*E^2)`,
/// an unbounded `E` is a DoS vector independent of `MAX_ROLL_DICE`. `N` here
/// is not `MAX_ROLL_DICE`: the DP pools KEPT dice records, which explosions
/// can inflate to just under `MAX_ROLL_RECORDS` (1000). Worst case is
/// therefore ~1000 * 100^2 = 1e7 ops, not `MAX_ROLL_DICE * E^2` -- still
/// cheap and bounded, but the bound is on the record cap, not the dice cap.
pub(crate) const MAX_EXPERTISE: u32 = 100;
/// Cap on a `DieKind::Numeric` die's face count (`max - min + 1`). Per-die
/// values are bounded by this cap and `MAX_ROLL_RECORDS`, but the evaluator's
/// aggregate folds (`eval::sum::fold`'s `Expr::Bin` arithmetic, including a
/// pure-`Const` chain with zero dice groups) are NOT overflow-free by
/// construction -- they saturate at `i64::MAX`/`MIN` on overflow instead of
/// panicking or wrapping (see `eval::sum`'s `*_saturating` helpers).
pub(crate) const MAX_DIE_SIDES: i64 = 10_000;
/// Cap on non-text chunks (`Inline`/`Button`/`DocLink`) `scan_body` may extract from one
/// message body.
pub(crate) const MAX_INLINE_ROLLS: usize = 8;

/// One scanned chunk of a message body: literal text between spans, an
/// inline roll to execute, a button to validate-and-store, or a doc/token
/// link to store directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BodyChunk<'a> {
    /// Literal body text between spans.
    Text(&'a str),
    /// An `[[formula]]` inline roll to execute.
    Inline(&'a str),
    /// A `[[roll:...]]` button to validate and store unexecuted.
    Button {
        /// The formula inside the span.
        formula: &'a str,
        /// Optional label after the `|` separator.
        label: Option<&'a str>,
    },
    /// A `[[doc:<uuid>[/<embedded_path>]|<label>]]` or `[[token:<uuid>|<label>]]` span: a
    /// free-form author-inserted link, captured with its target and display label fully
    /// parsed — `handle_send_message`'s ingest arm does no further parsing (see
    /// `Segment::DocLink`'s own doc comment).
    DocLink {
        /// What the link points at.
        target: DocLinkTarget,
        /// Display text captured at authoring time (the composer's `|<label>` suffix); never
        /// empty (an empty/absent label is a `RollError::MalformedDocLink`).
        label: &'a str,
    },
}

/// Balanced span scanner. A span opens at `[[` and closes at the first `]]`
/// reached while a per-span nesting `depth` is 0: inside the span, a single
/// `[` increments `depth` and a single `]` decrements it (a lone `]` at
/// `depth == 0` that is NOT immediately followed by a second `]` is left as
/// literal content — `depth` never goes negative), so a notation label's own
/// brackets (`[[4d6[atk]]]` -> formula `4d6[atk]`) survive intact. A `doc:`/
/// `token:` prefix on the span's content produces a `DocLink`: grammar
/// `doc:<uuid>[/<embedded_path>]|<label>` or `token:<uuid>|<label>`, fully
/// parsed here (`handle_send_message`'s ingest arm does no further parsing). A `roll:`
/// prefix on the span's content produces a `Button`; the content is then
/// split on the first `|` into `formula`/an optional trimmed `label` (empty
/// after trim => `None`). Every other span is an `Inline`. Errors: a span
/// opened but never closed by a balanced `]]` (`RollError::Unterminated`);
/// more than `MAX_INLINE_ROLLS` non-text chunks (`RollError::TooManyInline`);
/// a `doc:`/`token:`-prefixed span with an unparseable id or a missing/empty
/// `|<label>` suffix (`RollError::MalformedDocLink`).
pub(crate) fn scan_body(body: &str) -> Result<Vec<BodyChunk<'_>>, RollError> {
    let mut chunks = Vec::new();
    let mut non_text = 0usize;
    let mut text_start = 0usize;
    let mut pos = 0usize;
    let bytes = body.as_bytes();

    loop {
        let Some(rel) = body[pos..].find("[[") else {
            if text_start < body.len() {
                chunks.push(BodyChunk::Text(&body[text_start..]));
            }
            break;
        };
        let span_open = pos + rel;
        if text_start < span_open {
            chunks.push(BodyChunk::Text(&body[text_start..span_open]));
        }

        let content_start = span_open + 2;
        let mut i = content_start;
        let mut depth: u32 = 0;
        let mut terminated_at = None;
        while i < bytes.len() {
            match bytes[i] {
                b'[' => {
                    depth += 1;
                    i += 1;
                }
                b']' => {
                    if depth > 0 {
                        depth -= 1;
                        i += 1;
                    } else if i + 1 < bytes.len() && bytes[i + 1] == b']' {
                        terminated_at = Some(i);
                        break;
                    } else {
                        // Lone ']' at depth 0, not part of a "]]" pair: literal content.
                        i += 1;
                    }
                }
                _ => i += 1,
            }
        }
        let Some(content_end) = terminated_at else {
            return Err(RollError::Unterminated);
        };

        let content = &body[content_start..content_end];
        non_text += 1;
        if non_text > MAX_INLINE_ROLLS {
            return Err(RollError::TooManyInline(non_text));
        }
        match parse_doc_link(content) {
            Ok(Some(chunk)) => chunks.push(chunk),
            Err(()) => return Err(RollError::MalformedDocLink),
            Ok(None) => {
                if let Some(rest) = content.strip_prefix("roll:") {
                    let (formula, label) = match rest.split_once('|') {
                        Some((f, l)) => {
                            let l = l.trim();
                            (f, if l.is_empty() { None } else { Some(l) })
                        }
                        None => (rest, None),
                    };
                    chunks.push(BodyChunk::Button { formula, label });
                } else {
                    chunks.push(BodyChunk::Inline(content));
                }
            }
        }

        pos = content_end + 2; // past the terminating "]]"
        text_start = pos;
    }

    Ok(chunks)
}

/// Parses `content` as a `doc:`/`token:`-prefixed span. `Ok(None)` when `content` carries
/// neither prefix (the caller falls through to `roll:`/`Inline` handling); `Err(())` when the
/// prefix is recognized but the id/label grammar is malformed (the caller returns
/// `RollError::MalformedDocLink`); `Ok(Some(chunk))` on success. Grammar:
/// `doc:<uuid>[/<embedded_path>]|<label>` or `token:<uuid>|<label>` — the id/path is
/// everything before the FIRST `|`, split from an optional `/<embedded_path>` at the first `/`
/// after the `doc:`/`token:` prefix; the label is everything after that `|`, trimmed, and must
/// be non-empty (`Segment::DocLink.label` is a required field, unlike `Button`'s optional
/// label).
fn parse_doc_link(content: &str) -> Result<Option<BodyChunk<'_>>, ()> {
    if let Some(rest) = content.strip_prefix("doc:") {
        let (id_and_path, label) = rest.split_once('|').ok_or(())?;
        let label = label.trim();
        if label.is_empty() {
            return Err(());
        }
        let (id_part, embedded_path) = match id_and_path.split_once('/') {
            Some((id, p)) => (id, Some(format!("/{p}"))),
            None => (id_and_path, None),
        };
        let doc_id = Uuid::parse_str(id_part).map_err(|_| ())?;
        return Ok(Some(BodyChunk::DocLink {
            target: DocLinkTarget::Doc {
                doc_id,
                embedded_path,
            },
            label,
        }));
    }
    if let Some(rest) = content.strip_prefix("token:") {
        let (id_part, label) = rest.split_once('|').ok_or(())?;
        let label = label.trim();
        if label.is_empty() {
            return Err(());
        }
        let token_id = Uuid::parse_str(id_part).map_err(|_| ())?;
        return Ok(Some(BodyChunk::DocLink {
            target: DocLinkTarget::Token { token_id },
            label,
        }));
    }
    Ok(None)
}

/// Fresh OS-entropy seed per roll: `Uuid::new_v4` (v4 = 122 random bits from
/// the OS-backed `getrandom` already used for every document id) folded to a
/// `u64` by XOR-ing its high and low halves. Nothing persists the seed — a
/// stored `RawRoll`'s naturals reproduce the outcome without it (the dice
/// engine's `roll`/`evaluate` split), so there is no process-lifetime key to
/// recover or rotate.
pub(crate) fn entropy_seed() -> u64 {
    let bits = Uuid::new_v4().as_u128();
    ((bits >> 64) as u64) ^ (bits as u64)
}

// `pub`, not `pub(crate)`: `SendMessageError::Roll` (a publicly-reachable
// error variant) carries this type, so it must be at least as visible as
// that public enum, even though the `rolls` module itself stays private.
/// Why a roll formula was refused (parse failure or a resource cap).
/// Rendered to the sender as the refusal message via `Display`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RollError {
    /// The formula failed to parse (wraps the parser's error).
    Parse(ParseError),
    /// Total requested dice across all groups above `MAX_ROLL_DICE`.
    TooManyDice(u32),
    /// Evaluation emitted more records than `MAX_ROLL_RECORDS`.
    TooManyRecords(usize),
    /// Expertise die count above `MAX_EXPERTISE`.
    ExpertiseTooLarge(u32),
    /// Die sides above `MAX_DIE_SIDES`.
    SidesTooLarge(i64),
    /// More inline/button spans than `MAX_INLINE_ROLLS` in one body.
    TooManyInline(usize),
    /// A `[[` span never closed with `]]`.
    Unterminated,
    /// Two ladder rungs share one `margin_offset` -- `classify`'s
    /// max_by_key/min_by_key tie is caller-order-dependent, so which rung wins
    /// would be nondeterministic. Refused at construction so every downstream
    /// ladder is unambiguous (`dice::eval::classify`'s doc comment documents the tie).
    DuplicateTierOffset(i32),
    /// A `[[doc:...]]`/`[[token:...]]` span recognized by its prefix but malformed: an
    /// unparseable id, or a missing/empty `|<label>` suffix.
    MalformedDocLink,
    /// A template reference could not be resolved — an unknown path, a
    /// non-integer value, or a scan error from the template grammar itself.
    /// Carries the formula engine's error; its `detail` is player-presentable.
    Reference(crate::formula::FormulaError),
}

/// Player-presentable. `Parse` reuses `ParseError`'s own `Display`; every
/// other variant is authored here directly, never `{:?}` Debug output.
impl std::fmt::Display for RollError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RollError::Parse(e) => write!(f, "{e}"),
            RollError::TooManyDice(n) => {
                write!(
                    f,
                    "that roll asks for {n} dice, more than {MAX_ROLL_DICE} allowed"
                )
            }
            RollError::TooManyRecords(n) => write!(
                f,
                "that roll produced {n} dice results, more than {MAX_ROLL_RECORDS} allowed \
                 (likely a long explosion chain) -- the roll was not made"
            ),
            RollError::ExpertiseTooLarge(n) => write!(
                f,
                "that roll's expertise budget ({n}) exceeds the maximum of {MAX_EXPERTISE}"
            ),
            RollError::SidesTooLarge(span) => write!(
                f,
                "that die has {span} faces, more than {MAX_DIE_SIDES} allowed"
            ),
            RollError::TooManyInline(n) => write!(
                f,
                "that message has {n} inline rolls, more than {MAX_INLINE_ROLLS} allowed"
            ),
            RollError::Unterminated => {
                write!(
                    f,
                    "an inline roll (\"[[...]]\") is missing its closing ']]'"
                )
            }
            RollError::DuplicateTierOffset(o) => {
                write!(f, "duplicate tier margin offset {o}")
            }
            RollError::MalformedDocLink => {
                write!(f, "that document/token link is malformed")
            }
            RollError::Reference(e) => write!(f, "{}", e.detail),
        }
    }
}

impl std::error::Error for RollError {}

/// Recursively sums `DiceGroup.count` over an `Expr` (into a `u64` — the sum
/// of several near-`u32::MAX` counts could overflow a `u32` accumulator) and
/// collects a reference to every `DiceGroup` reached, for the per-group cap
/// checks below. Recurses into `Expr::Call`'s `args` too, so a dice
/// group nested inside a math-function argument still counts toward `MAX_ROLL_DICE` and is validated
/// by the per-group cap checks below — a `Call` node is not a way to smuggle dice groups past this
/// walk.
fn walk_groups<'a>(expr: &'a Expr, total: &mut u64, groups: &mut Vec<&'a DiceGroup>) {
    match expr {
        Expr::Dice(group) => {
            *total += group.count as u64;
            groups.push(group);
        }
        Expr::Const(_) => {}
        Expr::Neg(inner) => walk_groups(inner, total, groups),
        Expr::Bin { lhs, rhs, .. } => {
            walk_groups(lhs, total, groups);
            walk_groups(rhs, total, groups);
        }
        Expr::Call { args, .. } => {
            for arg in args {
                walk_groups(arg, total, groups);
            }
        }
    }
}

/// Pre-roll cap validation, in order: dice-count sum, then per-group
/// `DieKind::validate()` + `Numeric` face-count, then (in `SuccessCount`
/// mode) expertise. Called by both `validate_formula` (buttons, no roll) and
/// `execute_roll_with_seed` (before spending any entropy).
fn validate_pre_roll(spec: &RollSpec) -> Result<(), RollError> {
    let mut total: u64 = 0;
    let mut groups: Vec<&DiceGroup> = Vec::new();
    walk_groups(&spec.expr, &mut total, &mut groups);
    if total > MAX_ROLL_DICE as u64 {
        return Err(RollError::TooManyDice(total.min(u32::MAX as u64) as u32));
    }

    for group in &groups {
        // `DieKind::validate()` rejects an empty `Faces{faces:[]}`. No notation
        // path constructs `Faces` today, so this arm is unreachable via
        // `execute_roll`/`validate_formula`'s only caller (the notation parser);
        // called anyway as defense-in-depth against a future notation extension.
        // `RollError`'s variant set is fixed by this module's interface contract
        // (no dedicated `Faces` variant), so an `EmptyFaces` failure maps onto
        // `SidesTooLarge(0)` -- the closest existing "the die's face space is
        // degenerate" variant. Revisit this mapping if a future change gives
        // `Faces` a real notation constructor.
        if group.kind.validate().is_err() {
            return Err(RollError::SidesTooLarge(0));
        }
        if let DieKind::Numeric { min, max } = &group.kind {
            let faces = (*max as i64) - (*min as i64) + 1;
            if faces > MAX_DIE_SIDES {
                return Err(RollError::SidesTooLarge(faces));
            }
        }
    }

    match &spec.mode {
        Mode::Total(cfg) => validate_tiers(&cfg.tiers)?,
        Mode::SuccessCount(cfg) => {
            validate_tiers(&cfg.tiers)?;
            if cfg.expertise > MAX_EXPERTISE {
                return Err(RollError::ExpertiseTooLarge(cfg.expertise));
            }
        }
    }

    Ok(())
}

/// Uniqueness guard over a classification ladder's `margin_offset`s. Reachable from untrusted
/// notation via the `tr<offset>[:<value>][<label>]` modifier (`dice::notation::parser`'s `"tr"`
/// arm), which can append a duplicate offset; `classify::classify`'s `max_by_key`/`min_by_key`
/// tie on a duplicate `margin_offset` is caller-order-dependent (documented on
/// `dice::eval::classify`), so a malformed ladder with a repeated offset would otherwise resolve
/// nondeterministically.
fn validate_tiers(tiers: &[crate::dice::spec::Tier]) -> Result<(), RollError> {
    let mut seen = std::collections::BTreeSet::new();
    for t in tiers {
        if !seen.insert(t.margin_offset) {
            return Err(RollError::DuplicateTierOffset(t.margin_offset));
        }
    }
    Ok(())
}

/// Rewrites a roll template's references against `host` — the bound actor's
/// `system` band via `SystemLeafResolver`, or every-reference-unknown via
/// `NoHostResolver` when nothing is bound. A template with no references
/// passes through byte-identically.
fn resolve_roll_notation(formula: &str, host: Option<&Document>) -> Result<String, RollError> {
    let notation = match host {
        Some(doc) => {
            crate::formula::resolve_notation_template(formula, &SystemLeafResolver::new(doc))
        }
        None => crate::formula::resolve_notation_template(formula, &NoHostResolver),
    };
    notation.map_err(RollError::Reference)
}

/// Parse a formula and run every pre-roll cap check WITHOUT rolling — used to
/// validate a `[[roll:...]]` button at ingest so a stored button is never
/// structurally broken. A button is a template by nature — its references are
/// per-clicker — so validation substitutes a placeholder zero for every
/// identifier (a substituted reference is always a labeled constant factor,
/// never a dice count, so the placeholder cannot change the shape being
/// checked); value-dependent failures surface per clicker at click time.
pub(crate) fn validate_formula(formula: &str, ctx: ParseContext) -> Result<(), RollError> {
    let notation = crate::formula::resolve_notation_template(formula, &|_: &[String]| Ok(0.0))
        .map_err(RollError::Reference)?;
    let spec = notation::parse(&notation, ctx).map_err(RollError::Parse)?;
    validate_pre_roll(&spec)
}

/// Test seam: identical to `execute_roll` but takes an explicit seed instead
/// of drawing fresh OS entropy, so a test can assert on a deterministic
/// outcome. `execute_roll` is a thin entropy-supplying wrapper over this.
/// `pub(crate)`: also used by `combat::transition`'s own test fixtures
/// (`RollPost::test_with_total`) to build a deterministic outcome.
pub(crate) fn execute_roll_with_seed(
    formula: &str,
    ctx: ParseContext,
    host: Option<&Document>,
    seed: u64,
) -> Result<(String, RollOutcome, RollSpec, crate::dice::RawRoll), RollError> {
    let notation = resolve_roll_notation(formula, host)?;
    let spec = notation::parse(&notation, ctx).map_err(RollError::Parse)?;
    validate_pre_roll(&spec)?;
    let mut rng = NoiseRng::from_seed(seed);
    let raws = roll(&spec, &mut rng);
    if raws.records.len() > MAX_ROLL_RECORDS {
        return Err(RollError::TooManyRecords(raws.records.len()));
    }
    let outcome = eval::evaluate(&spec, &raws);
    Ok((formula.to_owned(), outcome, spec, raws))
}

/// Resolve references -> parse -> cap-validate -> roll -> evaluate. The ONLY
/// untrusted-notation
/// execution path in chat. Seeds from `entropy_seed()` -- fresh OS entropy
/// per call, never a caller-supplied or persisted seed. `host` is the roll's
/// actor binding (see the module doc): `None` resolves every reference as
/// unknown.
///
/// Also returns the parsed `RollSpec` and rolled `RawRoll` alongside the
/// formula/outcome, so a caller can persist them onto `Segment::RollEmbed`
/// for a later GM recalculation. The returned formula string is the author's
/// ORIGINAL template text, not the substituted notation — the embed shows
/// what was asked and the breakdown chips show what each reference read.
pub(crate) fn execute_roll(
    formula: &str,
    ctx: ParseContext,
    host: Option<&Document>,
) -> Result<(String, RollOutcome, RollSpec, crate::dice::RawRoll), RollError> {
    execute_roll_with_seed(formula, ctx, host, entropy_seed())
}

#[cfg(test)]
mod tests;
