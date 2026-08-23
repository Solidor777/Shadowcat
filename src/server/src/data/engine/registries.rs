//! Singleton config-document engine bands: `channel-registry`,
//! `faction-registry`, `condition-registry`, `chat-settings`, `dice-settings`.
//! Field shapes mirror the client's re-exported `Channel`, `FactionStance`,
//! `Faction`, `Condition`, `ChatSettingsEngine`, `DiceSettingsEngine`, and
//! `ChannelDiceOverride` (`DiceSettingsEngine.channel_overrides`'s value type).
//!
//! `chat::settings::ChatContentPolicy` is a type alias onto
//! `ChatSettingsEngine`; `chat::rolls`/`chat::settings` read `DiceSettingsEngine`
//! directly. Both bodies live on the `engine` band, ingress-validated same as
//! every other engine-defined doc_type (see `chat::settings::ChatContentPolicy`).

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two crate-level deny attributes this module declares.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A chat channel's display config (mirrors the client's `Channel`).
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

/// A faction's stance toward the party (mirrors the client's `FactionStance`).
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

/// A faction's display + stance (mirrors the client's `Faction`). `color`
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

/// A status condition's display (mirrors the client's `Condition`). `icon`
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

/// GM-configured chat content policy (mirrors the client's `ChatSettingsEngine`).
/// Every field optional/absent-safe; a partial body is a valid engine band.
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
    /// Tri-state: absent defaults ON whenever `hyperlinks` is also on, per
    /// `ChatSettingsEngine::previews_enabled`; `Some(true)`/`Some(false)` are
    /// an explicit GM override.
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

/// A single channel's full override of the world-default dice aggregation
/// mode + win direction (`DiceSettingsEngine.channel_overrides`'s value
/// type). Mirrors `DiceSettingsEngine`'s own `{mode, direction}` shape
/// exactly — full replacement, never a partial-field merge: an override
/// always carries BOTH fields, so a channel either fully overrides the
/// world default or (absent from the map) fully inherits it; there is no
/// "override just mode, inherit direction" state to resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct ChannelDiceOverride {
    /// This channel's aggregation mode, overriding `DiceSettingsEngine.mode`.
    pub mode: DiceModeSetting,
    /// This channel's win direction, overriding `DiceSettingsEngine.direction`.
    pub direction: DiceDirectionSetting,
}

/// GM-configured ambient dice-notation context (mirrors the client's
/// `DiceSettingsEngine`). `#[serde(default)]` on the struct means a partial
/// or absent body fills the rest with the safe default (Total + HighWins,
/// empty `channel_overrides`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, default)]
pub struct DiceSettingsEngine {
    /// Aggregation mode ambient dice notation resolves under (the world default).
    pub mode: DiceModeSetting,
    /// Win direction ambient dice notation resolves under (the world default).
    pub direction: DiceDirectionSetting,
    /// Per-channel full overrides, keyed by `channel-registry`'s channel id.
    /// A channel absent from this map (including every channel when the map
    /// is empty) resolves against `mode`/`direction` above — this is a
    /// full-replacement override, not a partial merge, matching how the
    /// world default itself is an unconditional pair rather than
    /// independently-optional fields (see `ChannelDiceOverride`'s doc).
    #[serde(default)]
    pub channel_overrides: BTreeMap<String, ChannelDiceOverride>,
}
