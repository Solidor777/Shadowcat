//! Notation-template rewriting: a template such as `1d20 + str` — a mix of
//! dice-notation atoms (numbers, the dice operator, `NOTATION_KEYWORDS`
//! modifiers, bracketed label spans) and dotted identifier references — is
//! rewritten into plain dice notation by substituting each reference with its
//! resolved value as a labeled constant (`1d20 + 3[str]`). Twin of the client
//! package's `template` module's rewrite half; the two are pinned together by
//! the `templates` section of the shared conformance corpus. The client
//! module's `checkNotationKey` half has no twin: it is an authoring aid, and
//! the server answers "does this reference run" by running it.
//!
//! INVARIANT: never panics on any input; every failure is a `FormulaError`
//! value. INVARIANT: the rewritten text carries the author's case and the
//! author's labels verbatim — only identifier claims are rewritten.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use super::chars::{is_digit, is_word_char, is_word_start};
use super::evaluate::Resolve;
use super::types::{finite, js_number, FormulaError, FormulaErrorKind, MAX_FORMULA_LENGTH};

/// The dice operator: both a `NOTATION_KEYWORDS` member and the one keyword
/// whose emission is rewritten — with no integer claim immediately before it a
/// count of `1` is synthesized, because the notation parser requires a count.
/// Declared once and used both as the list's first member and as
/// `emit_claim`'s test, so the two cannot disagree about which keyword that is.
const DICE_OPERATOR: &str = "d";

/// Identifier words that mean dice notation rather than a stat. Mirrors
/// `dice::notation`'s modifier keyword match, plus the dice operator; this is
/// one of three declarations of one decision (the client package's own list
/// and the parser's match arms are the other two), and none of the three can
/// read another — the notation-modifier parity gate reads all three and fails
/// the script-test gate on any difference.
///
/// **This list is not the set of unsafe stat keys, and no list is.** The
/// notation grammar reserves more than these words, and what it reserves has
/// no closed-form description — it is negative space over the recognizer
/// chain. A consuming system's stat-key authoring validation calls the client
/// package's `checkNotationKey`, which RUNS that chain, and must not
/// reimplement a rule over this list instead.
pub(crate) const NOTATION_KEYWORDS: [&str; 15] = [
    DICE_OPERATOR,
    "kh",
    "kl",
    "dh",
    "dl",
    "r",
    "ro",
    "cs",
    "cf",
    "t",
    "e",
    "tr",
    "rs",
    "xs",
    "xf",
];

/// The dice-notation grammar's math-function vocabulary (the six spellings
/// `dice::spec::FnName` declares and `dice::notation::parser`'s `fn_call`
/// recognizes). NOT `NOTATION_KEYWORDS` members: the keyword list guards the
/// dice-MECHANIC modifier vocabulary, while function names are notation only
/// when followed by `(` after any spaces/tabs — `claim_notation_function` tests exactly
/// that, and anywhere else the same word is an ordinary identifier a resolver
/// may answer. One of three declarations of one decision (the client
/// template module's own list and the dice parser's match arms are the other
/// two); the notation-modifier parity gate reads all three.
pub(crate) const NOTATION_FUNCTIONS: [&str; 6] = ["floor", "ceil", "round", "abs", "min", "max"];

/// Largest magnitude a substituted value may have (asymmetric about zero on
/// purpose: the most negative representable i32 exceeds it and is rejected).
const I32_MAX: f64 = 2147483647.0;

/// Which recognizer claimed a span of template source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaimKind {
    /// A bracketed span.
    Label,
    /// A run of digits.
    Integer,
    /// An identifier-start run that is a `NOTATION_KEYWORDS` member when
    /// lowercased, or a `NOTATION_FUNCTIONS` member immediately followed by
    /// `(`.
    Keyword,
    /// A dotted reference span — the only kind whose text reaches the
    /// consumer's resolver.
    Identifier,
    /// One character no recognizer claimed.
    Literal,
}

/// One recognizer's claim over a span of source, as a half-open range into
/// the scan's character vector.
#[derive(Debug, Clone, Copy)]
struct Claim {
    /// Which recognizer claimed the span.
    kind: ClaimKind,
    /// First claimed character index.
    start: usize,
    /// One past the last claimed character index.
    end: usize,
}

/// The claimed text, collected from the scan's character vector.
fn claim_text(chars: &[(char, usize)], claim: &Claim) -> String {
    chars[claim.start..claim.end]
        .iter()
        .map(|(c, _)| c)
        .collect()
}

/// A bracketed label span, from `[` through the next `]`. An unterminated
/// bracket rejects the whole scan, with the bracket's UTF-16 offset in the
/// detail (the client scans code units, so an astral character ahead of the
/// bracket counts as two). A label's contents are never scanned for keywords
/// or identifiers, so an author-written label survives the rewrite whatever
/// it spells; that follows from the claim's EXTENT rather than from this
/// recognizer's place in the chain, because `[` is in no other recognizer's
/// start set.
fn claim_label_span(chars: &[(char, usize)], at: usize) -> Result<Option<usize>, FormulaError> {
    if chars[at].0 != '[' {
        return Ok(None);
    }
    let mut j = at + 1;
    while j < chars.len() {
        if chars[j].0 == ']' {
            return Ok(Some(j + 1));
        }
        j += 1;
    }
    Err(FormulaError::new(
        FormulaErrorKind::Parse,
        format!("unterminated '[' label at position {}", chars[at].1),
    ))
}

/// A maximal run of digits. Has no rejecting branch — it only declines or
/// claims.
fn claim_integer_run(chars: &[(char, usize)], at: usize) -> Option<usize> {
    if !is_digit(chars[at].0) {
        return None;
    }
    let mut j = at;
    while j < chars.len() && is_digit(chars[j].0) {
        j += 1;
    }
    Some(j)
}

/// An identifier-start run that is a `NOTATION_KEYWORDS` member when
/// lowercased. The run's extent is `is_word_start`'s alone (over `kh_max` it
/// is the whole six characters; over `kh1` only `kh`), and only the whole run
/// is tested for membership. Ordered before `claim_identifier_span`, which is
/// what makes the reserved set wider than the list — the one adjacency in the
/// chain whose order is observable, since the two share `is_word_start` as
/// their start set.
fn claim_notation_keyword(chars: &[(char, usize)], at: usize) -> Option<usize> {
    if !is_word_start(chars[at].0) {
        return None;
    }
    let mut j = at;
    while j < chars.len() && is_word_start(chars[j].0) {
        j += 1;
    }
    let run: String = chars[at..j].iter().map(|(c, _)| c).collect();
    NOTATION_KEYWORDS
        .contains(&run.to_ascii_lowercase().as_str())
        .then_some(j)
}

/// An identifier-start run that is a `NOTATION_FUNCTIONS` member when
/// lowercased AND is followed by `(` after any spaces/tabs — the dice parser
/// decides `fn_call` at TOKEN level, where the lexer's space/tab skip has
/// already happened, so `floor (2d6)` is a function call there and must read
/// as one here too (a newline is NOT skipped by that lexer, so it does not
/// count here either). Reserved because the server runs every roll through
/// this scan: without
/// it, `floor(101d6/2)` would read `floor` as a stat reference and the roll
/// would fail (or, under placeholder validation, break shape). Ordered after
/// `claim_notation_keyword`; the two sets are disjoint, so that adjacency is
/// unobservable.
fn claim_notation_function(chars: &[(char, usize)], at: usize) -> Option<usize> {
    if !is_word_start(chars[at].0) {
        return None;
    }
    let mut j = at;
    while j < chars.len() && is_word_start(chars[j].0) {
        j += 1;
    }
    let run: String = chars[at..j].iter().map(|(c, _)| c).collect();
    let mut k = j;
    while k < chars.len() && matches!(chars[k].0, ' ' | '\t') {
        k += 1;
    }
    (NOTATION_FUNCTIONS.contains(&run.to_ascii_lowercase().as_str())
        && k < chars.len()
        && chars[k].0 == '(')
        .then_some(j)
}

/// A dotted reference span: an `is_word_start` run continued by
/// `is_word_char`, joined by a `.` to a further such run only when the
/// character immediately after that `.` is itself an identifier-start
/// character — a `.` not followed by one is not crossed and the span ends
/// before it.
fn claim_identifier_span(chars: &[(char, usize)], at: usize) -> Option<usize> {
    if !is_word_start(chars[at].0) {
        return None;
    }
    let mut j = at;
    while j < chars.len() && is_word_char(chars[j].0) {
        j += 1;
    }
    while j + 1 < chars.len() && chars[j].0 == '.' && is_word_start(chars[j + 1].0) {
        let mut k = j + 1;
        while k < chars.len() && is_word_char(chars[k].0) {
            k += 1;
        }
        j = k;
    }
    Some(j)
}

/// Runs the recognizer chain at one position. Total: when every recognizer
/// declines, one character passes through as a `Literal` claim, so the scan
/// always advances.
fn claim_at(chars: &[(char, usize)], at: usize) -> Result<Claim, FormulaError> {
    if let Some(end) = claim_label_span(chars, at)? {
        return Ok(Claim {
            kind: ClaimKind::Label,
            start: at,
            end,
        });
    }
    if let Some(end) = claim_integer_run(chars, at) {
        return Ok(Claim {
            kind: ClaimKind::Integer,
            start: at,
            end,
        });
    }
    if let Some(end) = claim_notation_keyword(chars, at) {
        return Ok(Claim {
            kind: ClaimKind::Keyword,
            start: at,
            end,
        });
    }
    if let Some(end) = claim_notation_function(chars, at) {
        return Ok(Claim {
            kind: ClaimKind::Keyword,
            start: at,
            end,
        });
    }
    if let Some(end) = claim_identifier_span(chars, at) {
        return Ok(Claim {
            kind: ClaimKind::Identifier,
            start: at,
            end,
        });
    }
    Ok(Claim {
        kind: ClaimKind::Literal,
        start: at,
        end: at + 1,
    })
}

/// Turns a claim into the text it contributes to the rewritten notation. The
/// ONLY stage that reads the scan's carried state or calls the consumer's
/// resolver; recognition does neither.
fn emit_claim(
    chars: &[(char, usize)],
    claim: &Claim,
    prev_was_int: bool,
    resolve: &dyn Resolve,
    out: &mut String,
) -> Result<(), FormulaError> {
    if claim.kind == ClaimKind::Identifier {
        out.push_str(&substitute_identifier(&claim_text(chars, claim), resolve)?);
        return Ok(());
    }
    let text = claim_text(chars, claim);
    if claim.kind == ClaimKind::Keyword && text.eq_ignore_ascii_case(DICE_OPERATOR) && !prev_was_int
    {
        out.push('1');
    }
    out.push_str(&text);
    Ok(())
}

/// Resolves a `.`-joined identifier path (e.g. `hp.max`) to a labeled
/// substitution. The resolver's `Err` passes through verbatim; its `Ok` passes
/// the same finite gate every other resolver boundary in this crate applies,
/// so a non-finite value becomes a `NonFinite` error rather than leaking into
/// the notation. A non-integer value is a `Type` error (roll templates require
/// integers) and a magnitude past `I32_MAX` a `Cap` — the cap is a MAGNITUDE
/// test, asymmetric about zero on purpose. A negative value emits the same
/// labeled shape as a positive one, prefixed with a unary minus, so the
/// notation parser reads a negation of a labeled constant rather than two
/// unlabeled constants and the roll breakdown surfaces a correctly-signed
/// chip.
fn substitute_identifier(text: &str, resolve: &dyn Resolve) -> Result<String, FormulaError> {
    let path: Vec<String> = text.split('.').map(str::to_owned).collect();
    let n = finite(resolve.resolve(&path)?)?;
    if n.fract() != 0.0 {
        return Err(FormulaError::new(
            FormulaErrorKind::Type,
            format!(
                "'{text}' = {}: roll templates require integers (use floor/round in the stat formula)",
                js_number(n)
            ),
        ));
    }
    if n.abs() > I32_MAX {
        return Err(FormulaError::new(
            FormulaErrorKind::Cap,
            format!("'{text}' = {}: out of i32 range", js_number(n)),
        ));
    }
    if n < 0.0 {
        return Ok(format!("-{}[{text}]", js_number(-n)));
    }
    Ok(format!("{}[{text}]", js_number(n)))
}

/// Rewrites a notation template into plain dice notation by substituting every
/// identifier reference with its resolved value as a labeled constant
/// (`1d20 + str` → `1d20 + 3[str]`). The scan runs the recognizer chain left
/// to right through `claim_at`, emitting via `emit_claim`; an unclaimed
/// character always passes through as a `Literal` claim, so the scan never
/// fails on unfamiliar input and the returned notation is NOT guaranteed to
/// be text the dice parser accepts — parse validity is the caller's next step.
///
/// The resolver is offered each identifier's path in the author's RAW case
/// (unlike the formula grammar's lowercasing lexer). A template longer than
/// `MAX_FORMULA_LENGTH` UTF-16 code units is refused before any recognizer
/// runs.
pub fn resolve_notation_template(src: &str, resolve: &dyn Resolve) -> Result<String, FormulaError> {
    if src.encode_utf16().count() > MAX_FORMULA_LENGTH {
        return Err(FormulaError::new(
            FormulaErrorKind::Cap,
            format!("template exceeds {MAX_FORMULA_LENGTH} characters"),
        ));
    }
    // (char, utf16 offset) pairs so the scan indexes by character while error
    // positions stay UTF-16 code-unit offsets — the client scans code units
    // and an astral character is two of them.
    let chars: Vec<(char, usize)> = {
        let mut out = Vec::with_capacity(src.len());
        let mut pos = 0usize;
        for c in src.chars() {
            out.push((c, pos));
            pos += c.len_utf16();
        }
        out
    };
    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;
    // The scan's only carried state: whether the immediately preceding claim
    // was an integer run.
    let mut prev_was_int = false;
    while i < chars.len() {
        let claim = claim_at(&chars, i)?;
        emit_claim(&chars, &claim, prev_was_int, resolve, &mut out)?;
        i = claim.end;
        prev_was_int = claim.kind == ClaimKind::Integer;
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
