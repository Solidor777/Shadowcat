use super::*;

#[test]
fn no_command_is_normal_passthrough() {
    let p = parse_command("hello world");
    assert_eq!(p.kind, MessageKind::Normal);
    assert!(p.whisper_to.is_none());
    assert_eq!(p.body, "hello world");
}

#[test]
fn me_variants_set_emote_and_strip_token() {
    for cmd in ["/me waves", "/em waves", "/emote waves"] {
        let p = parse_command(cmd);
        assert_eq!(p.kind, MessageKind::Emote, "{cmd}");
        assert_eq!(p.body, "waves");
    }
}

#[test]
fn roll_sets_roll_kind_and_keeps_expression_verbatim() {
    let p = parse_command("/roll 2d6+3");
    assert_eq!(p.kind, MessageKind::Roll);
    assert_eq!(p.body, "2d6+3"); // stored unparsed; `rolls::execute_roll` evaluates it
    let short = parse_command("/1d20");
    assert_eq!(short.kind, MessageKind::Roll);
    assert_eq!(short.body, "1d20");
}

#[test]
fn whisper_captures_usernames_and_strips_them() {
    let p = parse_command("/w @alice @bob hey there");
    assert_eq!(p.kind, MessageKind::Normal);
    assert_eq!(p.whisper_to, Some(vec!["alice".into(), "bob".into()]));
    assert_eq!(p.body, "hey there");
}

#[test]
fn no_command_yields_system_never() {
    // Exhaustive: no input can produce System via the parser.
    for s in [
        "/system hi",
        "system",
        "/sys",
        "/me x",
        "/roll 1d4",
        "/w @a hi",
        "plain",
    ] {
        assert_ne!(parse_command(s).kind, MessageKind::System, "{s}");
    }
}
