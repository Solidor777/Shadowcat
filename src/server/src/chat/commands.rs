//! This module parses a message's leading chat command.
//!
//! It recognizes `/me`/`/em`/`/emote`, the `/roll`|`/r`|`/NdM` shorthand, the `/ooc` and `/ic`
//! display kinds, and the audience shortcuts `/w`, `/gm`, `/gmroll`, `/publicroll`, `/players`
//! and `/reply`. The parser is pure (no repo/async): it only records an `AudienceIntent`; the
//! async caller (`handle_send_message`) resolves that intent to a concrete `Audience` through
//! `resolve_audience_intent` and re-validates it.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::chat::MessageKind;

/// The audience a command asks for, before it is resolved against the world's membership.
///
/// # Examples
///
/// ```
/// use shadowcat::chat::{parse_command, AudienceIntent};
///
/// let parsed = parse_command("/gm the door is trapped");
/// assert_eq!(parsed.audience_intent, Some(AudienceIntent::GmOnly));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudienceIntent {
    /// `/w @a @b`: the named members, resolved by username.
    Whisper(Vec<String>),
    /// `/gm`, `/gmroll`: the world's GM, resolved dynamically like `Audience::GmOnly`.
    GmOnly,
    /// `/publicroll`: every member, overriding a non-public frame audience.
    Public,
    /// `/players`: every current non-GM member, frozen at send time as a whisper roster.
    Players,
    /// `/reply`: whoever whispered the sender most recently.
    ReplyToLastWhisper,
}

/// Result of parsing a message's leading command token, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommand {
    /// The message kind the command selects (never `System`).
    pub kind: MessageKind,
    /// The audience the command asks for, unresolved. The async caller resolves it
    /// into a concrete `Audience`.
    pub audience_intent: Option<AudienceIntent>,
    /// The message body with the command token stripped.
    pub body: String,
}

/// Builds a `ParsedCommand` for a command token that names an audience and a kind.
fn with_intent(kind: MessageKind, intent: AudienceIntent, rest: &str) -> ParsedCommand {
    ParsedCommand {
        kind,
        audience_intent: Some(intent),
        body: rest.trim().to_string(),
    }
}

/// Parse a leading chat command. Pure (no repo/async). `kind` is
/// server-authoritative and can NEVER be `System`. Only a leading token counts;
/// a command mid-message is literal text.
///
/// # Examples
///
/// ```
/// use shadowcat::chat::{parse_command, AudienceIntent, MessageKind};
///
/// let parsed = parse_command("/gmroll 1d20");
/// assert_eq!(parsed.kind, MessageKind::Roll);
/// assert_eq!(parsed.audience_intent, Some(AudienceIntent::GmOnly));
/// assert_eq!(parsed.body, "1d20");
/// ```
pub fn parse_command(raw: &str) -> ParsedCommand {
    let trimmed = raw.trim_start();
    // Emote.
    for tok in ["/me ", "/em ", "/emote "] {
        if let Some(rest) = trimmed.strip_prefix(tok) {
            return ParsedCommand {
                kind: MessageKind::Emote,
                audience_intent: None,
                body: rest.trim().to_string(),
            };
        }
    }
    // Out-of-character / in-character display kind. No audience effect.
    if let Some(rest) = trimmed.strip_prefix("/ooc ") {
        return ParsedCommand {
            kind: MessageKind::Ooc,
            audience_intent: None,
            body: rest.trim().to_string(),
        };
    }
    if let Some(rest) = trimmed.strip_prefix("/ic ") {
        return ParsedCommand {
            kind: MessageKind::Normal,
            audience_intent: None,
            body: rest.trim().to_string(),
        };
    }
    // Roll with an explicit audience. Tested before `/roll`|`/r` so no prefix overlaps.
    if let Some(rest) = trimmed.strip_prefix("/gmroll ") {
        return with_intent(MessageKind::Roll, AudienceIntent::GmOnly, rest);
    }
    if let Some(rest) = trimmed.strip_prefix("/publicroll ") {
        return with_intent(MessageKind::Roll, AudienceIntent::Public, rest);
    }
    // Roll: explicit /roll|/r, or /NdM shorthand.
    for tok in ["/roll ", "/r "] {
        if let Some(rest) = trimmed.strip_prefix(tok) {
            return ParsedCommand {
                kind: MessageKind::Roll,
                audience_intent: None,
                body: rest.trim().to_string(),
            };
        }
    }
    if let Some(expr) = trimmed.strip_prefix('/') {
        if is_dice_shorthand(expr) {
            return ParsedCommand {
                kind: MessageKind::Roll,
                audience_intent: None,
                body: expr.to_string(),
            };
        }
    }
    // Text with an explicit audience.
    if let Some(rest) = trimmed.strip_prefix("/gm ") {
        return with_intent(MessageKind::Normal, AudienceIntent::GmOnly, rest);
    }
    if let Some(rest) = trimmed.strip_prefix("/players ") {
        return with_intent(MessageKind::Normal, AudienceIntent::Players, rest);
    }
    if let Some(rest) = trimmed.strip_prefix("/reply ") {
        return with_intent(
            MessageKind::Normal,
            AudienceIntent::ReplyToLastWhisper,
            rest,
        );
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
            audience_intent: if names.is_empty() {
                None
            } else {
                Some(AudienceIntent::Whisper(names))
            },
            body: body.trim().to_string(),
        };
    }
    ParsedCommand {
        kind: MessageKind::Normal,
        audience_intent: None,
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
mod tests;
