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

use super::MAX_CHANNEL_CHARS;

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

impl ChannelRegistryEngine {
    /// Default world seed: the single `general` channel. The engine
    /// definition — the client renders whatever the registry holds and
    /// declares no seed of its own.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::engine::ChannelRegistryEngine;
    ///
    /// assert_eq!(ChannelRegistryEngine::seed().channels["general"].name, "General");
    /// ```
    pub fn seed() -> Self {
        let mut channels = BTreeMap::new();
        channels.insert(
            "general".to_string(),
            Channel {
                name: "General".to_string(),
            },
        );
        Self { channels }
    }

    /// Ingress validation: the registry must hold at least one channel — an
    /// empty registry wedges all chat, since message ingest validates
    /// `MessageEngine.channel` against membership — every channel needs a
    /// non-empty name, and a key longer than `MAX_CHANNEL_CHARS` could never
    /// be posted to.
    pub fn validate(&self) -> Result<(), String> {
        if self.channels.is_empty() {
            return Err("channel-registry must declare at least one channel".to_string());
        }
        for (id, channel) in &self.channels {
            if id.chars().count() > MAX_CHANNEL_CHARS {
                return Err(format!(
                    "channel id '{id}' exceeds {MAX_CHANNEL_CHARS} characters"
                ));
            }
            if channel.name.trim().is_empty() {
                return Err(format!("channel '{id}' has an empty name"));
            }
        }
        Ok(())
    }
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
    /// Movement-type tags unioned into every member actor's resolved set
    /// (`SceneEcs::token_movement_tags` / `resolveTokenActor`). Same vocabulary and
    /// engine-reserved semantics as `ActorEngine::movement` — `"flying"`/`"incorporeal"`
    /// ignore terrain COST only; unknown tags are inert system vocabulary.
    #[serde(default)]
    pub movement: Vec<String>,
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

impl FactionRegistryEngine {
    /// Default three-faction world seed (friendly / neutral / hostile). The
    /// engine definition — the client's faction UI mirrors these ids and
    /// colors rather than declaring its own seed.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::engine::FactionRegistryEngine;
    ///
    /// let s = FactionRegistryEngine::seed();
    /// assert_eq!(s.factions.len(), 3);
    /// assert_eq!(s.factions["friendly"].color, "#3fb950");
    /// ```
    pub fn seed() -> Self {
        let mut factions = BTreeMap::new();
        factions.insert(
            "friendly".to_string(),
            Faction {
                name: "Friendly".to_string(),
                color: "#3fb950".to_string(),
                stance: FactionStance::Friendly,
                movement: vec![],
            },
        );
        factions.insert(
            "neutral".to_string(),
            Faction {
                name: "Neutral".to_string(),
                color: "#9e9e9e".to_string(),
                stance: FactionStance::Neutral,
                movement: vec![],
            },
        );
        factions.insert(
            "hostile".to_string(),
            Faction {
                name: "Hostile".to_string(),
                color: "#f85149".to_string(),
                stance: FactionStance::Hostile,
                movement: vec![],
            },
        );
        Self { factions }
    }
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
    /// Built-in token-art effects applied to tokens carrying this condition
    /// (folded client-side into the token's render fx, in condition array
    /// order). Presentational only — the server never reads it. Absent = no
    /// fx.
    #[serde(default)]
    #[ts(optional)]
    pub fx: Option<ConditionFx>,
}

impl Condition {
    /// Ingress validation beyond serde shape: authored fx colors are css
    /// `#rrggbb` strings.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if let Some(fx) = &self.fx {
            if let Some(tint) = &fx.tint {
                validate_fx_color(tint)?;
            }
            if let Some(highlight) = &fx.highlight {
                validate_fx_color(highlight)?;
            }
        }
        Ok(())
    }
}

/// A condition's built-in token-art effects (css colors), folded by the
/// client's `TokenView.toSpec` into the token's render fx.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct ConditionFx {
    /// Channel-tint target color (css `#rrggbb`), or absent for none.
    #[serde(default)]
    #[ts(optional)]
    pub tint: Option<String>,
    /// `true` strips the token art to luminance.
    #[serde(default)]
    #[ts(optional)]
    pub desaturate: Option<bool>,
    /// Brighten-toward target color (css `#rrggbb`), or absent for none.
    #[serde(default)]
    #[ts(optional)]
    pub highlight: Option<String>,
}

/// Condition-fx color check: exactly `#` + 6 hex digits (a css `#rrggbb`
/// string) — same shape rule as `token::validate_emission_color`.
fn validate_fx_color(color: &str) -> Result<(), String> {
    match color.strip_prefix('#') {
        Some(hex) if hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()) => Ok(()),
        _ => Err("fx color must be a css #rrggbb string".to_string()),
    }
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

impl ConditionRegistryEngine {
    /// Ingress validation beyond serde shape: every entry's own `validate`
    /// (fx css color shapes), keyed by condition id in the error message.
    pub(crate) fn validate(&self) -> Result<(), String> {
        for (id, condition) in &self.conditions {
            condition
                .validate()
                .map_err(|m| format!("conditions.{id}: {m}"))?;
        }
        Ok(())
    }

    /// Default nine-condition emoji-glyph world seed. The engine definition —
    /// the client's conditions UI renders whatever the registry holds.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::engine::ConditionRegistryEngine;
    ///
    /// let s = ConditionRegistryEngine::seed();
    /// assert_eq!(s.conditions.len(), 9);
    /// assert_eq!(s.conditions["dead"].icon, "💀");
    /// ```
    pub fn seed() -> Self {
        let entries: [(&str, &str, &str); 9] = [
            ("dead", "Dead", "💀"),
            ("unconscious", "Unconscious", "😵"),
            ("prone", "Prone", "🛌"),
            ("stunned", "Stunned", "💫"),
            ("poisoned", "Poisoned", "🤢"),
            ("blinded", "Blinded", "🙈"),
            ("invisible", "Invisible", "👻"),
            ("hasted", "Hasted", "⚡"),
            ("slowed", "Slowed", "🐌"),
        ];
        let mut conditions = BTreeMap::new();
        for (id, name, icon) in entries {
            conditions.insert(
                id.to_string(),
                Condition {
                    name: name.to_string(),
                    icon: icon.to_string(),
                    fx: None,
                },
            );
        }
        Self { conditions }
    }
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
