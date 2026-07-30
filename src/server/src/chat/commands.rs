//! Leading chat-command parser: `/me`/`/em`/`/emote`, `/roll`|`/r`|`/NdM`
//! shorthand, and `/w @user...` whisper-target extraction. Pure (no repo/
//! async) — the async caller (`handle_send_message`) resolves `/w` usernames
//! to member uuids and re-validates the resulting audience.

use crate::chat::MessageKind;

/// Result of parsing a message's leading command token, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommand {
    /// The message kind the command selects (never `System`).
    pub kind: MessageKind,
    /// Raw `@usernames` from a `/w` command (unresolved). The async caller
    /// resolves these to UUIDs and builds `Audience::Whisper`.
    pub whisper_to: Option<Vec<String>>,
    /// The message body with the command token stripped.
    pub body: String,
}

/// Parse a leading chat command. Pure (no repo/async). `kind` is
/// server-authoritative and can NEVER be `System`. Only a leading token counts;
/// a command mid-message is literal text.
pub fn parse_command(raw: &str) -> ParsedCommand {
    let trimmed = raw.trim_start();
    // Emote.
    for tok in ["/me ", "/em ", "/emote "] {
        if let Some(rest) = trimmed.strip_prefix(tok) {
            return ParsedCommand {
                kind: MessageKind::Emote,
                whisper_to: None,
                body: rest.trim().to_string(),
            };
        }
    }
    // Roll: explicit /roll|/r, or /NdM shorthand.
    for tok in ["/roll ", "/r "] {
        if let Some(rest) = trimmed.strip_prefix(tok) {
            return ParsedCommand {
                kind: MessageKind::Roll,
                whisper_to: None,
                body: rest.trim().to_string(),
            };
        }
    }
    if let Some(expr) = trimmed.strip_prefix('/') {
        if is_dice_shorthand(expr) {
            return ParsedCommand {
                kind: MessageKind::Roll,
                whisper_to: None,
                body: expr.to_string(),
            };
        }
    }
    // Whisper: /w @a @b message
    if let Some(rest) = trimmed.strip_prefix("/w ") {
        let mut names = Vec::new();
        let mut remaining_words = rest.split_whitespace().peekable();
        while let Some(word) = remaining_words.peek() {
            match word.strip_prefix('@') {
                Some(name) => {
                    names.push(name.to_string());
                    remaining_words.next();
                }
                None => break,
            }
        }
        let body: String = remaining_words.collect::<Vec<_>>().join(" ");
        return ParsedCommand {
            kind: MessageKind::Normal,
            whisper_to: if names.is_empty() { None } else { Some(names) },
            body: body.trim().to_string(),
        };
    }
    ParsedCommand {
        kind: MessageKind::Normal,
        whisper_to: None,
        body: raw.to_string(),
    }
}

/// `NdM` (optionally with a trailing `+K`/`-K`) — the dice shorthand after `/`.
fn is_dice_shorthand(s: &str) -> bool {
    let core = s.split_whitespace().next().unwrap_or("");
    let mut parts = core.splitn(2, 'd');
    let (Some(n), Some(rest)) = (parts.next(), parts.next()) else {
        return false;
    };
    !n.is_empty()
        && n.chars().all(|c| c.is_ascii_digit())
        && rest.chars().next().is_some_and(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
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
        assert_eq!(p.body, "2d6+3"); // stored unparsed/unexecuted (M11d runs it)
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
}
