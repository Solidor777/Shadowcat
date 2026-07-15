//! Singleton config-document engine bands: `channel-registry`,
//! `faction-registry`, `condition-registry`, `chat-settings`, `dice-settings`.
//! Field shapes transcribed verbatim from `chat-docs.ts` / `scene-docs.ts`.
//!
//! `ChatSettingsEngine`/`DiceSettingsEngine` are independent struct
//! definitions: `chat::settings::ChatContentPolicy`/`DiceSettingsBody` are a
//! separate, structurally-equivalent definition that reads `system` today.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A chat channel's display config (chat-docs.ts:131-133 `ChatChannel`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct Channel {
    pub name: String,
}

/// The world's channel registry: a singleton config document. Keyed by
/// channel id — a MAP, not an array, so add/rename/remove are single-key
/// field Updates (`set_pointer` cannot grow arrays).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct ChannelRegistryEngine {
    pub channels: BTreeMap<String, Channel>,
}

/// A faction's stance toward the party (scene-docs.ts:409-414
/// `FactionStance`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "lowercase")]
pub enum FactionStance {
    Friendly,
    Neutral,
    Hostile,
}

/// A faction's display + stance (scene-docs.ts:410-414 `Faction`). `color`
/// is "#rrggbb" (the token border color).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct Faction {
    pub name: String,
    pub color: String,
    pub stance: FactionStance,
}

/// The world's faction registry: a singleton config document. Keyed by
/// faction id — an actor's `faction` field references a key. A MAP, not an
/// array, for the same single-key-Update reason as `ChannelRegistryEngine`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct FactionRegistryEngine {
    pub factions: BTreeMap<String, Faction>,
}

/// A status condition's display (scene-docs.ts:429-432 `Condition`). `icon`
/// is a short glyph (emoji) rendered as a token badge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct Condition {
    pub name: String,
    pub icon: String,
}

/// The world's condition registry: a singleton config document. Keyed by
/// condition id — an actor's `conditions` array holds keys. A MAP, not an
/// array, for the same single-key-Update reason as `ChannelRegistryEngine`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct ConditionRegistryEngine {
    pub conditions: BTreeMap<String, Condition>,
}

/// GM-configured chat content policy (chat-docs.ts:176-183
/// `ChatSettingsSystem`). Every field optional/absent-safe; a partial body
/// is a valid engine band.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, default)]
pub struct ChatSettingsEngine {
    pub markdown: Option<bool>,
    pub html: Option<bool>,
    pub images: Option<bool>,
    pub hyperlinks: Option<bool>,
    pub emails: Option<bool>,
    /// Tri-state: absent is the spec'd default-on-when-hyperlinks-on
    /// behavior; `Some(true)`/`Some(false)` are an explicit GM override.
    pub link_previews: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "snake_case")]
pub enum DiceModeSetting {
    #[default]
    Total,
    SuccessCount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "snake_case")]
pub enum DiceDirectionSetting {
    #[default]
    HighWins,
    LowWins,
}

/// GM-configured ambient dice-notation context (chat-docs.ts:154-157
/// `DiceSettingsSystem`). `#[serde(default)]` on the struct means a partial
/// or absent body fills the rest with the safe default (Total + HighWins).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, default)]
pub struct DiceSettingsEngine {
    pub mode: DiceModeSetting,
    pub direction: DiceDirectionSetting,
}
