//! `:shortcode:` → unicode emoji, applied to raw chat input BEFORE any
//! markdown/html processing (`chat::sanitize` pre-pass) so stored content is final
//! and identical under every content policy. Always-on typing sugar — output is
//! plain unicode text with no security surface. A `:name:` span is skipped when its
//! opening `:` falls inside a markdown inline code span (`code_span_ranges`); this is
//! still pre-parse relative to full markdown/HTML rendering, and it is not a full
//! CommonMark parser — only the backtick-run/code-span rule is implemented.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::borrow::Cow;

/// Sorted by name (binary-searched). Curated common set; extend freely.
/// INVARIANT: must stay lexicographically sorted by name — see
/// `table_is_sorted_by_name` below; a mis-sorted entry silently breaks
/// `binary_search_by_key` in `lookup`.
const TABLE: &[(&str, &str)] = &[
    ("+1", "👍"),
    ("-1", "👎"),
    ("100", "💯"),
    ("angry", "😠"),
    ("cat", "🐱"),
    ("check", "✅"),
    ("clap", "👏"),
    ("cool", "😎"),
    ("crossed_swords", "⚔️"),
    ("crown", "👑"),
    ("cry", "😢"),
    ("d20", "🎲"),
    ("dagger", "🗡️"),
    ("dog", "🐶"),
    ("dragon", "🐉"),
    ("eyes", "👀"),
    ("fire", "🔥"),
    ("ghost", "👻"),
    ("grin", "😁"),
    ("heart", "❤️"),
    ("hourglass", "⏳"),
    ("joy", "😂"),
    ("key", "🗝️"),
    ("laughing", "😆"),
    ("lightning", "⚡"),
    ("mage", "🧙"),
    ("map", "🗺️"),
    ("moneybag", "💰"),
    ("moon", "🌙"),
    ("muscle", "💪"),
    ("neutral_face", "😐"),
    ("party", "🎉"),
    ("pray", "🙏"),
    ("rage", "😡"),
    ("rofl", "🤣"),
    ("sad", "😞"),
    ("scream", "😱"),
    ("shield", "🛡️"),
    ("skull", "💀"),
    ("sleep", "😴"),
    ("smile", "😄"),
    ("smirk", "😏"),
    ("sparkles", "✨"),
    ("star", "⭐"),
    ("sun", "☀️"),
    ("sweat", "😅"),
    ("sword", "🗡️"),
    ("tada", "🎉"),
    ("thinking", "🤔"),
    ("thumbsdown", "👎"),
    ("thumbsup", "👍"),
    ("wave", "👋"),
    ("wink", "😉"),
    ("wizard", "🧙"),
    ("x", "❌"),
    ("zzz", "💤"),
];

/// Binary-search the sorted table for `name`'s emoji.
fn lookup(name: &str) -> Option<&'static str> {
    TABLE
        .binary_search_by_key(&name, |(n, _)| n)
        .ok()
        .map(|i| TABLE[i].1)
}

/// Characters legal inside a `:name:` token (lowercase ASCII, digits, `_+-`).
fn is_name_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '+' | '-')
}

/// Byte ranges in `raw` covered by a markdown inline code span — a backtick run, then the next
/// backtick run of the SAME length; the range spans from the start of the opening run to the end
/// of the closing run (inclusive of the backticks). An unmatched opening run (no closing run of
/// equal length before the string ends) is NOT a code span, matching CommonMark's own rule —
/// scanning resumes immediately after the unmatched run rather than treating the rest of the
/// string as code. Ranges are produced in increasing, non-overlapping order.
/// @param raw The raw chat text to scan.
/// @returns Every code-span byte range, `(start, end)` with `end` exclusive.
fn code_span_ranges(raw: &str) -> Vec<(usize, usize)> {
    let bytes = raw.as_bytes();
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }
        let run_start = i;
        let mut j = i;
        while j < bytes.len() && bytes[j] == b'`' {
            j += 1;
        }
        let run_len = j - run_start;
        let mut k = j;
        let mut closing = None;
        while k < bytes.len() {
            if bytes[k] != b'`' {
                k += 1;
                continue;
            }
            let close_start = k;
            let mut m = k;
            while m < bytes.len() && bytes[m] == b'`' {
                m += 1;
            }
            if m - close_start == run_len {
                closing = Some(m);
                break;
            }
            k = m;
        }
        match closing {
            Some(close_end) => {
                ranges.push((run_start, close_end));
                i = close_end;
            }
            None => i = j, // unmatched opening run: not a code span
        }
    }
    ranges
}

/// Whether byte position `pos` in `raw` falls inside one of `ranges` (`code_span_ranges`'s output).
/// @param ranges Non-overlapping, increasing `(start, end)` byte ranges.
/// @param pos The byte position to test.
/// @returns Whether `pos` is covered by any range.
fn in_code_span(ranges: &[(usize, usize)], pos: usize) -> bool {
    ranges.iter().any(|&(start, end)| pos >= start && pos < end)
}

/// Replace every `:name:` whose name is in the table; everything else passes
/// through verbatim. Borrowed (zero-alloc) when nothing matches.
pub(crate) fn replace_shortcodes(raw: &str) -> Cow<'_, str> {
    let bytes = raw.as_bytes();
    let spans = code_span_ranges(raw);
    let mut out: Option<String> = None;
    let mut i = 0;
    let mut last_emit = 0;
    while i < bytes.len() {
        if bytes[i] == b':' && !in_code_span(&spans, i) {
            // Find a closing ':' with a valid, non-empty name between.
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && is_name_char(bytes[j] as char) {
                j += 1;
            }
            if j > start && j < bytes.len() && bytes[j] == b':' {
                if let Some(emoji) = lookup(&raw[start..j]) {
                    let out = out.get_or_insert_with(|| String::with_capacity(raw.len()));
                    out.push_str(&raw[last_emit..i]);
                    out.push_str(emoji);
                    i = j + 1;
                    last_emit = i;
                    continue;
                }
            }
        }
        i += 1;
    }
    match out {
        None => Cow::Borrowed(raw),
        Some(mut s) => {
            s.push_str(&raw[last_emit..]);
            Cow::Owned(s)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_sorted_by_name() {
        assert!(
            TABLE.windows(2).all(|w| w[0].0 < w[1].0),
            "TABLE must be sorted by name for binary_search_by_key to work"
        );
    }

    #[test]
    fn replaces_known_shortcodes_and_keeps_unknown() {
        assert_eq!(replace_shortcodes("hi :smile:!"), "hi 😄!");
        assert_eq!(
            replace_shortcodes(":+1: and :unknown_thing: and :d20:"),
            "👍 and :unknown_thing: and 🎲"
        );
        assert_eq!(replace_shortcodes("no codes"), "no codes"); // borrowed passthrough
        assert_eq!(replace_shortcodes("a:b: :c"), "a:b: :c"); // malformed → untouched
        assert_eq!(replace_shortcodes("::smile::"), ":😄:"); // inner match only
    }

    #[test]
    fn no_match_returns_borrowed_cow() {
        assert!(matches!(
            replace_shortcodes("no codes here"),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn match_returns_owned_cow() {
        assert!(matches!(replace_shortcodes(":smile:"), Cow::Owned(_)));
    }

    #[test]
    fn shortcode_inside_single_backtick_span_is_left_literal() {
        assert_eq!(
            replace_shortcodes("see `:fire:` for the raw syntax"),
            "see `:fire:` for the raw syntax"
        );
    }

    #[test]
    fn shortcode_outside_code_span_still_replaces_alongside_a_protected_one() {
        assert_eq!(
            replace_shortcodes(":fire: means `:fire:` is the syntax"),
            "🔥 means `:fire:` is the syntax"
        );
    }

    #[test]
    fn double_backtick_fence_protects_content_like_a_single_backtick_span() {
        assert_eq!(replace_shortcodes("``:fire:``"), "``:fire:``");
    }

    #[test]
    fn unmatched_backtick_is_not_treated_as_an_unterminated_code_span() {
        // Discrimination check: a naive "backtick means code until end of string" bug would
        // treat the whole rest of the string as code and leave BOTH shortcodes unreplaced.
        // CommonMark's own rule (no matching closing run ⇒ not a code span at all) requires
        // both to replace normally.
        assert_eq!(replace_shortcodes("`:fire: and :smile:"), "`🔥 and 😄");
    }
}
