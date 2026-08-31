//! Per-world chat content policy: a single `chat-settings` config `Document`
//! read by the message sanitizer to decide which enrichment producers are
//! allowed. Resolution is fail-closed: an absent doc, a query error, or an
//! `engine` body that does not deserialize into `ChatSettingsEngine` all
//! yield `ChatContentPolicy::default()` (every toggle absent, i.e. plain
//! text). The toggles only ever WIDEN enrichment from that safe baseline, so
//! any failure mode degrades safe rather than open.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use uuid::Uuid;

use crate::data::engine::{
    ChatSettingsEngine, DiceDirectionSetting, DiceModeSetting, DiceSettingsEngine,
};
use crate::data::repository::Repository;
use crate::dice::{Direction, ModeKind, ParseContext};

/// Doc_type for the single per-world chat-settings config `Document`.
pub const CHAT_SETTINGS_DOC_TYPE: &str = "chat-settings";

/// GM-configured chat content policy, stored as the `engine` body of the
/// `chat-settings` doc. The ONE definition —
/// `data::engine::registries::ChatSettingsEngine` (the ts-rs-exported, ingress-
/// validated struct) — re-exported under this chat-domain name rather than
/// duplicated. Every field is `Option<bool>`; absent (`None`) resolves to
/// `false` via the accessor methods below, EXCEPT `link_previews`, whose
/// tri-state semantics are documented on `previews_enabled`.
pub type ChatContentPolicy = ChatSettingsEngine;

impl ChatContentPolicy {
    /// Markdown rendering allowed (absent = false, fail-closed).
    pub fn markdown(&self) -> bool {
        self.markdown.unwrap_or(false)
    }
    /// Raw-HTML input allowed (absent = false; output still ammonia-cleaned).
    pub fn html(&self) -> bool {
        self.html.unwrap_or(false)
    }
    /// Image embeds allowed (absent = false).
    pub fn images(&self) -> bool {
        self.images.unwrap_or(false)
    }
    /// Hyperlink anchors allowed (absent = false).
    pub fn hyperlinks(&self) -> bool {
        self.hyperlinks.unwrap_or(false)
    }
    /// Email autolinks allowed (absent = false).
    pub fn emails(&self) -> bool {
        self.emails.unwrap_or(false)
    }

    /// Resolved link-preview enablement: previews require
    /// `hyperlinks` to be on (a preview with no rendered link is
    /// meaningless), and within that, `link_previews` defaults ON when
    /// absent — a GM must explicitly write `link_previews: false` to opt
    /// out once hyperlinks are enabled. A fail-closed empty/default policy
    /// (`hyperlinks` absent) always resolves to `false` regardless of
    /// `link_previews`.
    pub fn previews_enabled(&self) -> bool {
        self.hyperlinks() && self.link_previews.unwrap_or(true)
    }
}

/// Read the world's chat content policy, fail-closed. A query error, an
/// absent `chat-settings` doc, or an `engine` body that fails to deserialize
/// into `ChatContentPolicy` all yield `ChatContentPolicy::default()`.
///
/// SINGLETON RESOLUTION: `chat-settings` is a per-world singleton, but nothing
/// yet enforces uniqueness at the create chokepoint (the GM editor's seed guard
/// is client-side only). Resolution is DETERMINISTIC regardless: `query_documents`
/// orders `ORDER BY id`, so if two `chat-settings` docs ever coexist the
/// lowest-UUID one always wins — never a nondeterministic policy. The fail-closed
/// direction bounds a stray doc (it can only WIDEN enrichment, which still needs
/// GM-authored content to matter).
/// TODO: enforce construction-time uniqueness via a singleton doc-type
/// create-gate, the stronger half of preventing a stray `chat-settings` doc.
pub async fn resolve_content_policy(repo: &dyn Repository, world_id: Uuid) -> ChatContentPolicy {
    let docs = match repo.query_documents(world_id, CHAT_SETTINGS_DOC_TYPE).await {
        Ok(d) => d,
        Err(_) => return ChatContentPolicy::default(),
    };
    let Some(doc) = docs.into_iter().next() else {
        return ChatContentPolicy::default();
    };
    doc.engine
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

/// Doc_type for the single per-world dice-settings config `Document`.
pub const DICE_SETTINGS_DOC_TYPE: &str = "dice-settings";

/// Read the world's ambient dice-notation `ParseContext` for `channel`,
/// fail-closed. A query error, an absent `dice-settings` doc, or an `engine`
/// body that fails to deserialize into `DiceSettingsEngine` all yield
/// `ParseContext { mode: Total, direction: HighWins }` regardless of
/// `channel` — the same safe baseline `resolve_content_policy` uses for chat
/// enrichment. When the doc IS present and well-formed, `channel_overrides`
/// is checked first: a channel with a registered override resolves under
/// that override's `mode`/`direction` (full replacement, never merged with
/// the world default); a channel absent from the map falls back to the
/// doc's own `mode`/`direction`.
pub async fn resolve_dice_context(
    repo: &dyn Repository,
    world: Uuid,
    channel: &str,
) -> ParseContext {
    let default = ParseContext::default();
    let docs = match repo.query_documents(world, DICE_SETTINGS_DOC_TYPE).await {
        Ok(d) => d,
        Err(_) => return default,
    };
    let Some(doc) = docs.into_iter().next() else {
        return default;
    };
    let body: DiceSettingsEngine = match doc.engine.and_then(|v| serde_json::from_value(v).ok()) {
        Some(b) => b,
        None => return default,
    };
    // A registered override for the SENDING channel wins outright (full
    // replacement, per DiceSettingsEngine.channel_overrides' doc); a
    // channel absent from the map — including every channel when the map
    // is empty — falls back to the doc's own world-default mode/direction.
    let (mode, direction) = match body.channel_overrides.get(channel) {
        Some(o) => (o.mode, o.direction),
        None => (body.mode, body.direction),
    };
    ParseContext {
        mode: match mode {
            DiceModeSetting::Total => ModeKind::Total,
            DiceModeSetting::SuccessCount => ModeKind::SuccessCount,
        },
        direction: match direction {
            DiceDirectionSetting::HighWins => Direction::HighWins,
            DiceDirectionSetting::LowWins => Direction::LowWins,
        },
    }
}

/// Whether `channel` is a registered channel of this world — the ingest gate
/// for `MessageEngine.channel`, consulted by `handle_send_message` and the
/// `CombatRoll` dispatch before any content work runs. A message's channel
/// selects the per-channel dice `ParseContext` (`resolve_dice_context`) and
/// labels the clients' channel views, so a sender naming a channel that does
/// not exist is a mistake to refuse, not to file.
///
/// Fail-closed, but distinguishable: a query error, a registry with no
/// `engine` body, or a body that fails to deserialize is an internal-class
/// failure (`Err`), as is a genuinely ABSENT registry — world creation seeds
/// it and world-join reseeds it, so absence means corruption, not a state to
/// accommodate; an existing registry that simply lacks the key is `Ok(false)`,
/// which the caller renders as the player-presentable unknown-channel refusal.
pub async fn channel_registered(
    repo: &dyn Repository,
    world: Uuid,
    channel: &str,
) -> Result<bool, crate::data::DataError> {
    let docs = repo
        .query_documents(world, crate::data::engine::CHANNEL_REGISTRY_DOC_TYPE)
        .await?;
    let Some(doc) = docs.first() else {
        return Err(crate::data::DataError::NotFound);
    };
    let Some(engine) = doc.engine.clone() else {
        return Err(crate::data::DataError::BadEngine(
            "channel-registry: missing engine body".to_string(),
        ));
    };
    let registry: crate::data::engine::ChannelRegistryEngine =
        serde_json::from_value(engine).map_err(crate::data::DataError::Serde)?;
    Ok(registry.channels.contains_key(channel))
}

#[cfg(test)]
mod tests;
