//! Singleton config-document engine bands: `channel-registry`,
//! `faction-registry`, `condition-registry`, `chat-settings`, `dice-settings`.
//! Field shapes transcribed verbatim from `chat-docs.ts` / `scene-docs.ts`.
//!
//! `chat::settings::ChatContentPolicy` is a type alias onto
//! `ChatSettingsEngine`; `chat::rolls`/`chat::settings` read `DiceSettingsEngine`
//! directly. Both bodies live on the `engine` band, ingress-validated same as
//! every other engine-defined doc_type (see `chat/settings.rs`).

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A chat channel's display config (chat-docs.ts:131-133 `ChatChannel`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct Channel {
    /// Display name shown on the channel tab.
    pub name: String,
}

/// The world's channel registry: a singleton config document. Keyed by
/// channel id — a MAP, not an array, so add/rename/remove are single-key
/// field Updates (`set_pointer` cannot grow arrays).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct ChannelRegistryEngine {
    /// Channels keyed by channel id (message docs reference the key).
    pub channels: BTreeMap<String, Channel>,
}

/// A faction's stance toward the party (scene-docs.ts:409-414
/// `FactionStance`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "lowercase")]
pub enum FactionStance {
    /// Allied with the party.
    Friendly,
    /// Neither allied nor opposed.
    Neutral,
    /// Opposed to the party.
    Hostile,
}

/// A faction's display + stance (scene-docs.ts:410-414 `Faction`). `color`
/// is "#rrggbb" (the token border color).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct Faction {
    /// Display name (factions panel, sheets).
    pub name: String,
    /// `#rrggbb` token border color (render layer reads it).
    pub color: String,
    /// Stance toward the party.
    pub stance: FactionStance,
}

/// The world's faction registry: a singleton config document. Keyed by
/// faction id — an actor's `faction` field references a key. A MAP, not an
/// array, for the same single-key-Update reason as `ChannelRegistryEngine`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct FactionRegistryEngine {
    /// Factions keyed by faction id (`ActorEngine.faction` references a key).
    pub factions: BTreeMap<String, Faction>,
}

/// A status condition's display (scene-docs.ts:429-432 `Condition`). `icon`
/// is a short glyph (emoji) rendered as a token badge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct Condition {
    /// Display name (conditions panel, tooltips).
    pub name: String,
    /// Short glyph (emoji) rendered as a token badge.
    pub icon: String,
}

/// The world's condition registry: a singleton config document. Keyed by
/// condition id — an actor's `conditions` array holds keys. A MAP, not an
/// array, for the same single-key-Update reason as `ChannelRegistryEngine`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct ConditionRegistryEngine {
    /// Conditions keyed by condition id (`ActorEngine.conditions` holds keys).
    pub conditions: BTreeMap<String, Condition>,
}

/// GM-configured chat content policy (chat-docs.ts:176-183
/// `ChatSettingsSystem`). Every field optional/absent-safe; a partial body
/// is a valid engine band.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, default)]
pub struct ChatSettingsEngine {
    /// Allow markdown rendering in message bodies.
    pub markdown: Option<bool>,
    /// Allow sanitized inline HTML in message bodies.
    pub html: Option<bool>,
    /// Allow image embeds.
    pub images: Option<bool>,
    /// Allow clickable hyperlinks.
    pub hyperlinks: Option<bool>,
    /// Allow mailto links.
    pub emails: Option<bool>,
    /// Tri-state: absent is the spec'd default-on-when-hyperlinks-on
    /// behavior; `Some(true)`/`Some(false)` are an explicit GM override.
    pub link_previews: Option<bool>,
}

/// World-default dice aggregation mode (`DiceSettingsEngine.mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "snake_case")]
pub enum DiceModeSetting {
    /// Sum the kept dice (the default).
    #[default]
    Total,
    /// Count successes against a threshold.
    SuccessCount,
}

/// World-default roll direction (`DiceSettingsEngine.direction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "snake_case")]
pub enum DiceDirectionSetting {
    /// Higher totals win (the default).
    #[default]
    HighWins,
    /// Lower totals win.
    LowWins,
}

/// GM-configured ambient dice-notation context (chat-docs.ts:154-157
/// `DiceSettingsSystem`). `#[serde(default)]` on the struct means a partial
/// or absent body fills the rest with the safe default (Total + HighWins).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, default)]
pub struct DiceSettingsEngine {
    /// Aggregation mode ambient dice notation resolves under.
    pub mode: DiceModeSetting,
    /// Win direction ambient dice notation resolves under.
    pub direction: DiceDirectionSetting,
}
