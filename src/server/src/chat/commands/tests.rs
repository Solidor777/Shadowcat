use super::*;

#[test]
fn no_command_is_normal_passthrough() {
    let p = parse_command("hello world");
    assert_eq!(p.kind, MessageKind::Normal);
    assert!(p.audience_intent.is_none());
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
    assert_eq!(
        p.audience_intent,
        Some(AudienceIntent::Whisper(vec!["alice".into(), "bob".into()]))
    );
    assert_eq!(p.body, "hey there");
}

#[test]
fn whisper_without_names_carries_no_intent() {
    let p = parse_command("/w hello");
    assert!(p.audience_intent.is_none());
}

#[test]
fn gmroll_and_publicroll_are_rolls_with_an_audience() {
    let g = parse_command("/gmroll 1d20+2");
    assert_eq!(g.kind, MessageKind::Roll);
    assert_eq!(g.audience_intent, Some(AudienceIntent::GmOnly));
    assert_eq!(g.body, "1d20+2");
    let p = parse_command("/publicroll 2d6");
    assert_eq!(p.kind, MessageKind::Roll);
    assert_eq!(p.audience_intent, Some(AudienceIntent::Public));
    assert_eq!(p.body, "2d6");
}

#[test]
fn gm_players_and_reply_are_normal_text_with_an_audience() {
    for (cmd, intent) in [
        ("/gm psst", AudienceIntent::GmOnly),
        ("/players psst", AudienceIntent::Players),
        ("/reply psst", AudienceIntent::ReplyToLastWhisper),
    ] {
        let p = parse_command(cmd);
        assert_eq!(p.kind, MessageKind::Normal, "{cmd}");
        assert_eq!(p.audience_intent, Some(intent), "{cmd}");
        assert_eq!(p.body, "psst", "{cmd}");
    }
}

#[test]
fn ooc_sets_the_ooc_kind_and_ic_resets_to_normal_without_an_audience() {
    let o = parse_command("/ooc brb");
    assert_eq!(o.kind, MessageKind::Ooc);
    assert!(o.audience_intent.is_none());
    assert_eq!(o.body, "brb");
    let i = parse_command("/ic back");
    assert_eq!(i.kind, MessageKind::Normal);
    assert!(i.audience_intent.is_none());
    assert_eq!(i.body, "back");
}

#[test]
fn a_command_token_mid_message_is_literal_text() {
    for s in ["hi /gm there", "hi /ooc there", "/gmroller 1d20"] {
        let p = parse_command(s);
        assert!(p.audience_intent.is_none(), "{s}");
        assert_eq!(p.kind, MessageKind::Normal, "{s}");
    }
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
        "/gmroll 1d4",
        "/publicroll 1d4",
        "/gm x",
        "/players x",
        "/reply x",
        "/ooc x",
        "/ic x",
        "plain",
    ] {
        assert_ne!(parse_command(s).kind, MessageKind::System, "{s}");
    }
}
