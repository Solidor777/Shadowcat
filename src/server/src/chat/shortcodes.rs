//! `:shortcode:` → unicode emoji, applied to raw chat input BEFORE any
//! markdown/html processing (sanitize.rs pre-pass) so stored content is final
//! and identical under every content policy. Always-on typing sugar — output is
//! plain unicode text with no security surface. v1 limitation (documented in the
//! design spec): replacement is pre-parse, so a shortcode inside a markdown code
//! span is also replaced.
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

fn lookup(name: &str) -> Option<&'static str> {
    TABLE
        .binary_search_by_key(&name, |(n, _)| n)
        .ok()
        .map(|i| TABLE[i].1)
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '+' | '-')
}

/// Replace every `:name:` whose name is in the table; everything else passes
/// through verbatim. Borrowed (zero-alloc) when nothing matches.
pub(crate) fn replace_shortcodes(raw: &str) -> Cow<'_, str> {
    let bytes = raw.as_bytes();
    let mut out: Option<String> = None;
    let mut i = 0;
    let mut last_emit = 0;
    while i < bytes.len() {
        if bytes[i] == b':' {
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
}
