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
