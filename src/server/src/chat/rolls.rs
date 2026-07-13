//! Roll execution core: the ONLY untrusted-notation execution path in chat.
//!
//! `execute_roll`/`validate_formula` are the sole entry points from the chat
//! ingest stage: parse the caller-supplied formula against the dice crate's
//! `notation::parse`, enforce the wire-boundary caps below (closing the
//! dice-crate DoS/overflow gaps `docs/TODO.md` deferred to this checkpoint),
//! then roll/evaluate. The dice crate itself stays pure — it has no notion of
//! these caps, entropy seeding, or chat settings; those are transport policy
//! that belongs here, not in `dice/`.
//!
//! `execute_roll`/`validate_formula`/`BodyChunk`/`scan_body` are not yet
//! called from `handle_send_message` — the chat ingest stage that wires this
//! module in is a following checkpoint task. Until then every item here is
//! reachable only from this module's own tests, so the crate's dead-code lint
//! is suppressed at the module level; remove this once ingest calls in.
#![allow(dead_code)]

use uuid::Uuid;

use crate::dice::notation::{self, ParseContext, ParseError};
use crate::dice::outcome::RollOutcome;
use crate::dice::rng::NoiseRng;
use crate::dice::spec::{DiceGroup, DieKind, Expr, Mode, RollSpec};
use crate::dice::{eval, roll};

/// Sum of `DiceGroup.count` across one parsed `Expr` — rejects the unbounded-
/// `count` DoS/overflow class at the source, before any die is ever rolled.
pub(crate) const MAX_ROLL_DICE: u32 = 100;
/// Post-roll guard on `RawRoll.records.len()` — explosion chains are random;
/// `CHAIN_CAP=100/die` x `MAX_ROLL_DICE` dice could reach far past this, so an
/// oversized result rejects the roll outright rather than allocating it.
pub(crate) const MAX_ROLL_RECORDS: usize = 1_000;
/// Cap on `SuccessConfig.expertise` — the DP allocator's cost is `O(N*E^2)`,
/// an unbounded `E` is a DoS vector independent of `MAX_ROLL_DICE`.
pub(crate) const MAX_EXPERTISE: u32 = 100;
/// Cap on a `DieKind::Numeric` die's face count (`max - min + 1`). With
/// records <= `MAX_ROLL_RECORDS` and faces <= `MAX_DIE_SIDES`, every i64 sum
/// in the evaluator is overflow-free by construction.
pub(crate) const MAX_DIE_SIDES: i64 = 10_000;
/// Cap on non-text chunks (`Inline`/`Button`) `scan_body` may extract from one
/// message body.
pub(crate) const MAX_INLINE_ROLLS: usize = 8;

/// One scanned chunk of a message body: literal text between spans, an
/// inline roll to execute, or a button to validate-and-store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BodyChunk<'a> {
    Text(&'a str),
    Inline(&'a str),
    Button {
        formula: &'a str,
        label: Option<&'a str>,
    },
}

/// Balanced span scanner. A span opens at `[[` and closes at the first `]]`
/// reached while a per-span nesting `depth` is 0: inside the span, a single
/// `[` increments `depth` and a single `]` decrements it (a lone `]` at
/// `depth == 0` that is NOT immediately followed by a second `]` is left as
/// literal content — `depth` never goes negative), so a notation label's own
/// brackets (`[[4d6[atk]]]` -> formula `4d6[atk]`) survive intact. A `roll:`
/// prefix on the span's content produces a `Button`; the content is then
/// split on the first `|` into `formula`/an optional trimmed `label` (empty
/// after trim => `None`). Every other span is an `Inline`. Errors: a span
/// opened but never closed by a balanced `]]` (`RollError::Unterminated`);
/// more than `MAX_INLINE_ROLLS` non-text chunks (`RollError::TooManyInline`).
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

        pos = content_end + 2; // past the terminating "]]"
        text_start = pos;
    }

    Ok(chunks)
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RollError {
    Parse(ParseError),
    TooManyDice(u32),
    TooManyRecords(usize),
    ExpertiseTooLarge(u32),
    SidesTooLarge(i64),
    TooManyInline(usize),
    Unterminated,
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
        }
    }
}

impl std::error::Error for RollError {}

/// Recursively sums `DiceGroup.count` over an `Expr` (into a `u64` — the sum
/// of several near-`u32::MAX` counts could overflow a `u32` accumulator) and
/// collects a reference to every `DiceGroup` reached, for the per-group cap
/// checks below.
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
        // path constructs `Faces` today (see `shadowcat-codebase-dice`), so this
        // arm is unreachable via `execute_roll`/`validate_formula`'s only caller
        // (the notation parser); called anyway as defense-in-depth against a
        // future notation extension. `RollError`'s variant set is fixed by this
        // module's interface contract (no dedicated `Faces` variant), so an
        // `EmptyFaces` failure maps onto `SidesTooLarge(0)` -- the closest
        // existing "the die's face space is degenerate" variant. Revisit this
        // mapping if a future change gives `Faces` a real notation constructor.
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

    if let Mode::SuccessCount(cfg) = &spec.mode {
        if cfg.expertise > MAX_EXPERTISE {
            return Err(RollError::ExpertiseTooLarge(cfg.expertise));
        }
    }

    Ok(())
}

/// Parse a formula and run every pre-roll cap check WITHOUT rolling — used to
/// validate a `[[roll:...]]` button at ingest so a stored button is never
/// broken.
pub(crate) fn validate_formula(formula: &str, ctx: ParseContext) -> Result<(), RollError> {
    let spec = notation::parse(formula, ctx).map_err(RollError::Parse)?;
    validate_pre_roll(&spec)
}

/// Test seam: identical to `execute_roll` but takes an explicit seed instead
/// of drawing fresh OS entropy, so a test can assert on a deterministic
/// outcome. `execute_roll` is a thin entropy-supplying wrapper over this.
fn execute_roll_with_seed(
    formula: &str,
    ctx: ParseContext,
    seed: u64,
) -> Result<(String, RollOutcome), RollError> {
    let spec = notation::parse(formula, ctx).map_err(RollError::Parse)?;
    validate_pre_roll(&spec)?;
    let mut rng = NoiseRng::from_seed(seed);
    let raws = roll(&spec, &mut rng);
    if raws.records.len() > MAX_ROLL_RECORDS {
        return Err(RollError::TooManyRecords(raws.records.len()));
    }
    let outcome = eval::evaluate(&spec, &raws);
    Ok((formula.to_owned(), outcome))
}

/// Parse -> cap-validate -> roll -> evaluate. The ONLY untrusted-notation
/// execution path in chat. Seeds from `entropy_seed()` -- fresh OS entropy
/// per call, never a caller-supplied or persisted seed.
pub(crate) fn execute_roll(
    formula: &str,
    ctx: ParseContext,
) -> Result<(String, RollOutcome), RollError> {
    execute_roll_with_seed(formula, ctx, entropy_seed())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dice::notation::ModeKind;
    use crate::dice::spec::Direction;

    fn total_ctx() -> ParseContext {
        ParseContext {
            mode: ModeKind::Total,
            direction: Direction::HighWins,
        }
    }

    fn success_ctx() -> ParseContext {
        ParseContext {
            mode: ModeKind::SuccessCount,
            direction: Direction::HighWins,
        }
    }

    // --- Caps ---

    #[test]
    fn dice_count_at_limit_accepts() {
        assert!(validate_formula("100d6", total_ctx()).is_ok());
    }

    #[test]
    fn dice_count_past_limit_rejects() {
        match validate_formula("101d6", total_ctx()) {
            Err(RollError::TooManyDice(101)) => {}
            other => panic!("expected TooManyDice(101), got {other:?}"),
        }
    }

    #[test]
    fn expertise_at_limit_accepts() {
        assert!(validate_formula("1d20t10e100", success_ctx()).is_ok());
    }

    #[test]
    fn expertise_past_limit_rejects() {
        match validate_formula("1d20t10e101", success_ctx()) {
            Err(RollError::ExpertiseTooLarge(101)) => {}
            other => panic!("expected ExpertiseTooLarge(101), got {other:?}"),
        }
    }

    #[test]
    fn sides_at_limit_accepts() {
        assert!(validate_formula("1d10000", total_ctx()).is_ok());
    }

    #[test]
    fn sides_past_limit_rejects() {
        match validate_formula("1d10001", total_ctx()) {
            Err(RollError::SidesTooLarge(10_001)) => {}
            other => panic!("expected SidesTooLarge(10001), got {other:?}"),
        }
    }

    #[test]
    fn records_cap_rejects_post_roll() {
        // The base dice-count cap (100) is enforced before any roll happens, so
        // exceeding `MAX_ROLL_RECORDS` (1000) via base count alone is
        // unreachable through `execute_roll`/`execute_roll_with_seed` -- the
        // records cap exists to bound an explosion chain's fan-out instead.
        // `100d2!>=1` sets an explicit explode target of 1: every `d2` face
        // (1 or 2) satisfies `>=1`, so every base die's chain deterministically
        // runs to `CHAIN_CAP = 100` (`eval::groups`'s per-die chain cap)
        // regardless of seed -- 100 base dice x (1 + 100 chained extras) =
        // 10_100 records, well past `MAX_ROLL_RECORDS`. No seed search needed.
        match execute_roll_with_seed("100d2!>=1", total_ctx(), 42) {
            Err(RollError::TooManyRecords(n)) => assert!(n > MAX_ROLL_RECORDS),
            other => panic!("expected TooManyRecords, got {other:?}"),
        }
    }

    // --- Scanner grammar matrix ---

    #[test]
    fn scan_plain_text_is_one_text_chunk() {
        let chunks = scan_body("hello world").unwrap();
        assert_eq!(chunks, vec![BodyChunk::Text("hello world")]);
    }

    #[test]
    fn scan_single_inline_roll() {
        let chunks = scan_body("rolling [[2d6]] now").unwrap();
        assert_eq!(
            chunks,
            vec![
                BodyChunk::Text("rolling "),
                BodyChunk::Inline("2d6"),
                BodyChunk::Text(" now"),
            ]
        );
    }

    #[test]
    fn scan_multiple_inline_rolls() {
        let chunks = scan_body("[[1d4]] and [[1d6]]").unwrap();
        assert_eq!(
            chunks,
            vec![
                BodyChunk::Inline("1d4"),
                BodyChunk::Text(" and "),
                BodyChunk::Inline("1d6"),
            ]
        );
    }

    #[test]
    fn scan_button_without_label() {
        let chunks = scan_body("[[roll:2d6+3]]").unwrap();
        assert_eq!(
            chunks,
            vec![BodyChunk::Button {
                formula: "2d6+3",
                label: None,
            }]
        );
    }

    #[test]
    fn scan_button_with_label() {
        let chunks = scan_body("[[roll:2d6+3|Attack]]").unwrap();
        assert_eq!(
            chunks,
            vec![BodyChunk::Button {
                formula: "2d6+3",
                label: Some("Attack"),
            }]
        );
    }

    #[test]
    fn scan_button_with_whitespace_only_label_is_none() {
        let chunks = scan_body("[[roll:2d6|   ]]").unwrap();
        assert_eq!(
            chunks,
            vec![BodyChunk::Button {
                formula: "2d6",
                label: None,
            }]
        );
    }

    #[test]
    fn scan_button_empty_formula_parses_then_fails_downstream() {
        let chunks = scan_body("[[roll:]]").unwrap();
        assert_eq!(
            chunks,
            vec![BodyChunk::Button {
                formula: "",
                label: None,
            }]
        );
        match validate_formula("", total_ctx()) {
            Err(RollError::Parse(ParseError::Empty)) => {}
            other => panic!("expected Parse(Empty), got {other:?}"),
        }
    }

    #[test]
    fn scan_nested_label_survives_balanced_brackets() {
        let chunks = scan_body("[[4d6[atk]]]").unwrap();
        assert_eq!(chunks, vec![BodyChunk::Inline("4d6[atk]")]);
    }

    #[test]
    fn scan_adjacent_spans_with_no_text_between() {
        let chunks = scan_body("[[1d4]][[1d6]]").unwrap();
        assert_eq!(
            chunks,
            vec![BodyChunk::Inline("1d4"), BodyChunk::Inline("1d6")]
        );
    }

    #[test]
    fn scan_unterminated_span_errors() {
        assert_eq!(
            scan_body("text [[1d6 no close"),
            Err(RollError::Unterminated)
        );
    }

    #[test]
    fn scan_unterminated_nested_span_errors() {
        assert_eq!(scan_body("[[4d6[atk]"), Err(RollError::Unterminated));
    }

    #[test]
    fn scan_max_inline_rolls_at_limit_accepts() {
        let body = "[[1d6]]".repeat(MAX_INLINE_ROLLS);
        let chunks = scan_body(&body).unwrap();
        assert_eq!(chunks.len(), MAX_INLINE_ROLLS);
    }

    #[test]
    fn scan_max_inline_rolls_past_limit_rejects() {
        let body = "[[1d6]]".repeat(MAX_INLINE_ROLLS + 1);
        match scan_body(&body) {
            Err(RollError::TooManyInline(n)) => assert_eq!(n, MAX_INLINE_ROLLS + 1),
            other => panic!("expected TooManyInline, got {other:?}"),
        }
    }

    // --- Determinism / entropy ---

    #[test]
    fn execute_roll_with_seed_is_deterministic() {
        let a = execute_roll_with_seed("4d6+2", total_ctx(), 12345).unwrap();
        let b = execute_roll_with_seed("4d6+2", total_ctx(), 12345).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn execute_roll_returns_the_formula_verbatim() {
        let (formula, _) = execute_roll_with_seed("2d6+1", total_ctx(), 1).unwrap();
        assert_eq!(formula, "2d6+1");
    }

    #[test]
    fn entropy_seed_two_calls_differ() {
        // Not a formal randomness proof -- a sanity check that two draws are
        // not trivially identical (would indicate a broken/constant seed source).
        let a = entropy_seed();
        let b = entropy_seed();
        assert_ne!(a, b);
    }

    #[test]
    fn execute_roll_rejects_over_cap_formula() {
        match execute_roll("101d6", total_ctx()) {
            Err(RollError::TooManyDice(101)) => {}
            other => panic!("expected TooManyDice(101), got {other:?}"),
        }
    }

    #[test]
    fn validate_formula_rejects_the_same_way_execute_roll_would() {
        // validate_pre_roll runs before any RNG use, so a cap rejection is
        // identical whether reached via validate_formula or execute_roll.
        match validate_formula("101d6", total_ctx()) {
            Err(RollError::TooManyDice(101)) => {}
            other => panic!("expected TooManyDice(101), got {other:?}"),
        }
    }

    #[test]
    fn roll_error_display_has_no_debug_artifacts() {
        let variants = vec![
            RollError::Parse(ParseError::Empty),
            RollError::TooManyDice(200),
            RollError::TooManyRecords(2000),
            RollError::ExpertiseTooLarge(200),
            RollError::SidesTooLarge(20_000),
            RollError::TooManyInline(9),
            RollError::Unterminated,
        ];
        for v in variants {
            let rendered = v.to_string();
            assert!(!rendered.contains("Some("), "{rendered}");
            assert!(!rendered.is_empty());
        }
    }
}
