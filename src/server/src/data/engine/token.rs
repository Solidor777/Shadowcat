//! This module defines the `token` and `actor` engine bands.
//!
//! The client's `TokenEngine` and `ActorEngine` are the TypeScript types generated from these
//! structs. Neither carries a `name` field, because the envelope `name` holds it.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two crate-level deny attributes this module declares.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::data::engine::bars::{validate_bars, BarAttribute, BarDisplayMode};
use crate::data::engine::registries::FactionStance;
use crate::data::engine::scene::LightEmission;

/// A token's transform + visual (exported to the client as `TokenEngine`). `(x,y)`
/// is the token CENTER. `visual` is set only on raw (actorless) tokens —
/// actor-backed tokens resolve their visual via the linked/embedded actor.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::TokenEngine;
///
/// let token = TokenEngine {
///     x: 0.0,
///     y: 0.0,
///     w: 50.0,
///     h: 50.0,
///     rotation: 0.0,
///     visual: None,
///     actor_id: None,
///     overrides: None,
///     face: None,
///     elevation: None,
///     lock_rotation: false,
///     identity_hidden: false,
///     facing_vision: None,
/// };
/// assert_eq!(token.w, 50.0);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct TokenEngine {
    /// Token CENTER x, scene units.
    pub x: f64,
    /// Token CENTER y, scene units.
    pub y: f64,
    /// Width, scene units.
    pub w: f64,
    /// Height, scene units.
    pub h: f64,
    /// Rotation in degrees.
    pub rotation: f64,
    /// Raw (actorless) token's own visual; actor-backed tokens resolve via
    /// the linked/embedded actor instead.
    #[serde(default)]
    pub visual: Option<TokenVisual>,
    /// Linked token: the shared actor's id (absent/null ⇒ instanced, see
    /// `Document.embedded["actor"]`).
    #[serde(default)]
    pub actor_id: Option<Uuid>,
    /// Per-token whitelisted overrides of the linked actor's presentation.
    #[serde(default)]
    pub overrides: Option<TokenOverrides>,
    /// Active face name when the effective visual is a `faces` union member;
    /// token-local always (not part of `overrides` — selects INTO the
    /// actor's faces map, not an override of actor data).
    #[serde(default)]
    pub face: Option<String>,
    /// Elevation above the scene's ground plane (`None`/absent = 0, grounded).
    /// Token state, not actor state: altitude is per-token. Read through
    /// `scene::elevation::elevation_or_ground`, which clamps a non-finite
    /// stored value to ground.
    #[serde(default)]
    pub elevation: Option<f64>,
    /// When true, a move never auto-rotates this token to face its movement direction (see
    /// `scene::move_exec::bearing_degrees`), whatever the resolved scene's `auto_rotate_tokens`
    /// says. Per-token presentation state like `elevation`, so linked and instanced tokens
    /// carry it identically (it is not an `overrides` field). Writing it is GM-only
    /// (`permission::rotation_lock_touched`): the lock binds a player's rotation
    /// (`scene::rotation_gate::rotation_refusal`), so an owner who could clear it would not be
    /// bound by it.
    #[serde(default)]
    pub lock_rotation: bool,
    /// Secret-identity mode: when true, every recipient who is neither the token's effective
    /// owner nor a GM receives the token without its presented identity (`/name`,
    /// `/engine/visual`, `/engine/overrides/visual`, `/engine/overrides/name`,
    /// `/engine/overrides/disposition` and `/engine/actor_id`, so the linked actor's own
    /// visual, faction and disposition cannot be joined back). `permission::own_overrides`
    /// synthesizes that set from this one flag, so it cannot apply to some readers and not
    /// others. Writing it is GM-only (`permission::identity_hidden_touched`): it changes what
    /// every other viewer perceives.
    #[serde(default)]
    pub identity_hidden: bool,
    /// Facing-limited field of view of an ACTORLESS token (no `actor_id`, no embedded actor);
    /// every other token resolves it from its actor or `TokenOverrides::facing_vision`
    /// (`SceneEcs::token_facing_vision`). Ignored unless the scene's `facing_vision` switch is on.
    /// Writing it is GM-only (`permission::facing_vision_touched`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub facing_vision: Option<FacingVision>,
}

impl TokenEngine {
    /// Ingress validation beyond serde shape: every numeric field finite, and
    /// the position inside the ONE shared movement-coordinate bound
    /// (`scene::move_exec::MAX_GATE_WALK_COORD`) — the GM-write/Create path
    /// and the move gate must agree on admissible coordinates structurally,
    /// never by call ordering. A present raw `visual` validates through
    /// `TokenVisual::validate` (asset refs + colour fields). The override whitelist's emission
    /// payloads validate through `TokenOverrides::validate` here, so an actor-backed
    /// token can never carry an emission the actor arm would have refused.
    ///
    /// # Examples
    ///
    /// ```text
    /// token.validate()  // Err("x must be finite") for NaN; Err(bound) past the gate bound
    /// ```
    pub(crate) fn validate(&self) -> Result<(), String> {
        for (name, v) in [
            ("x", self.x),
            ("y", self.y),
            ("w", self.w),
            ("h", self.h),
            ("rotation", self.rotation),
        ] {
            if !v.is_finite() {
                return Err(format!("{name} must be finite"));
            }
        }
        if let Some(e) = self.elevation {
            if !e.is_finite() {
                return Err("elevation must be finite".to_string());
            }
        }
        if let Some(f) = &self.facing_vision {
            f.validate()?;
        }
        let bound = crate::scene::move_exec::MAX_GATE_WALK_COORD;
        if self.x.abs() > bound || self.y.abs() > bound {
            return Err(format!("position exceeds coordinate bound {bound}"));
        }
        if let Some(visual) = &self.visual {
            visual.validate().map_err(|m| format!("visual: {m}"))?;
        }
        if let Some(overrides) = &self.overrides {
            overrides.validate()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

/// Where a token-anchored VFX emission renders relative to the token's art.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::token::VfxAnchor;
///
/// assert_ne!(VfxAnchor::Above, VfxAnchor::Below);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "snake_case")]
pub enum VfxAnchor {
    /// Centred on the token's own centre, in front of the art (`vfxAnchorZIndex` 1) — the
    /// default "on the token" placement.
    Token,
    /// Centred on the token's top edge (`vfxAnchorZIndex` 2, drawn above `Token`) — e.g. a
    /// buff glow rising off a creature's head.
    Above,
    /// Centred on the token's footprint base, its bottom edge (`vfxAnchorZIndex` 0, drawn
    /// below `Token`) — e.g. a ground-hugging aura or a shadow-pool effect.
    Below,
}

/// An aura emission: a colored disc radiating `radius` grid cells from the
/// token's center, drawn UNDER its art. Purely presentational — nothing
/// server-side consumes it.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::token::AuraEmission;
///
/// let aura = AuraEmission { color: "#ff00ff".to_string(), opacity: 0.5, radius: 2.0, enabled: true };
/// assert_eq!(aura.radius, 2.0);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct AuraEmission {
    /// Disc color, a css `#rrggbb` string.
    pub color: String,
    /// Disc opacity; the presentation range `0..=1` is clamped read-side where
    /// consumed, so ingress validates finiteness only.
    pub opacity: f64,
    /// Disc radius in GRID CELLS (never scene units — the client converts at
    /// the render boundary).
    pub radius: f64,
    /// Master switch; `false` suppresses the emission without dropping the
    /// payload.
    pub enabled: bool,
}

impl AuraEmission {
    /// Ingress validation beyond serde shape: css-`#rrggbb` color, finite
    /// `opacity`, and a finite, non-negative, cell-cap-bounded `radius` (see
    /// `validate_emission_radius`).
    pub(crate) fn validate(&self) -> Result<(), String> {
        crate::data::engine::geometry::validate_hex_color(&self.color)?;
        validate_emission_scalar("opacity", self.opacity)?;
        validate_emission_radius(self.radius)?;
        Ok(())
    }
}

/// A sound emission: a looping or one-shot audio asset audible within
/// `radius` grid cells. Playback-ready data only — no playback consumer exists
/// yet, so nothing server-side or client-side reads it beyond storage.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::token::SoundEmission;
///
/// let sound = SoundEmission {
///     asset: "asset-001".to_string(),
///     radius: 6.0,
///     volume: 0.7,
///     loop_: true,
///     enabled: true,
/// };
/// assert!(sound.loop_);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct SoundEmission {
    /// Asset id of the audio to play.
    pub asset: String,
    /// Audible radius in GRID CELLS.
    pub radius: f64,
    /// Playback volume; the presentation range `0..=1` is clamped read-side
    /// where consumed, so ingress validates finiteness only.
    pub volume: f64,
    /// Loop playback; false = play once.
    #[serde(rename = "loop")]
    pub loop_: bool,
    /// Master switch; `false` suppresses the emission without dropping the
    /// payload.
    pub enabled: bool,
}

impl SoundEmission {
    /// Ingress validation beyond serde shape: non-empty `asset`, finite
    /// `volume`, and a finite, non-negative, cell-cap-bounded `radius` (see
    /// `validate_emission_radius`).
    pub(crate) fn validate(&self) -> Result<(), String> {
        validate_emission_asset(&self.asset)?;
        validate_emission_scalar("volume", self.volume)?;
        validate_emission_radius(self.radius)?;
        Ok(())
    }
}

/// A VFX emission: a visual effect asset anchored to the token. The render client's
/// `VfxView` plays it (an `emitter:<token>` node tracking the token's live transform);
/// nothing server-side reads it beyond storage/validation.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::token::{VfxAnchor, VfxEmission};
///
/// let vfx = VfxEmission {
///     asset: "fx-glow".to_string(),
///     anchor: VfxAnchor::Above,
///     loop_: false,
///     enabled: true,
/// };
/// assert_eq!(vfx.anchor, VfxAnchor::Above);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct VfxEmission {
    /// Asset id of the effect art.
    pub asset: String,
    /// Where the effect renders relative to the token's art.
    pub anchor: VfxAnchor,
    /// Loop playback; false = play once.
    #[serde(rename = "loop")]
    pub loop_: bool,
    /// Master switch; `false` suppresses the emission without dropping the
    /// payload.
    pub enabled: bool,
}

impl VfxEmission {
    /// Ingress validation beyond serde shape: non-empty `asset`.
    pub(crate) fn validate(&self) -> Result<(), String> {
        validate_emission_asset(&self.asset)?;
        Ok(())
    }
}

/// Shared emission-payload radius check: finite, non-negative, and within the
/// ONE cell-radius cap every cell-measured radius reads
/// (`scene::pathfinding::MAX_FOOTPRINT_CELLS`) — a second, emission-specific
/// bound would fork the cap convention.
fn validate_emission_radius(radius: f64) -> Result<(), String> {
    if !radius.is_finite() {
        return Err("radius must be finite".to_string());
    }
    if radius < 0.0 {
        return Err("radius must be non-negative".to_string());
    }
    let bound = crate::scene::pathfinding::MAX_FOOTPRINT_CELLS;
    if radius > bound {
        return Err(format!("radius exceeds cell bound {bound}"));
    }
    Ok(())
}

/// Shared emission-payload scalar check (`opacity`/`volume`): finite only —
/// the `0..=1` presentation range is a read-side clamp where consumed, not an
/// ingress rejection.
fn validate_emission_scalar(name: &str, v: f64) -> Result<(), String> {
    if !v.is_finite() {
        return Err(format!("{name} must be finite"));
    }
    Ok(())
}

/// Shared emission-payload asset check: delegates to the ONE cross-doc-type asset-ref
/// validator (`geometry::validate_asset_ref`) — every asset id in the engine tree is a UUID
/// string, format-only, never an existence check.
fn validate_emission_asset(asset: &str) -> Result<(), String> {
    crate::data::engine::geometry::validate_asset_ref(asset)
}

/// The per-token override whitelist for a linked token.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::TokenOverrides;
///
/// let overrides = TokenOverrides {
///     name: Some("Goblin Scout".to_string()),
///     visual: None,
///     size: None,
///     shape: None,
///     vision: None,
///     facing_vision: None,
///     light: None,
///     movement: None,
///     aura: None,
///     sound: None,
///     vfx: None,
///     disposition: None,
///     bars: None,
///     nameplate_mode: None,
/// };
/// assert_eq!(overrides.name.as_deref(), Some("Goblin Scout"));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct TokenOverrides {
    /// Display-name override (subject to the name-privacy rules).
    #[serde(default)]
    pub name: Option<String>,
    /// Visual override; replaces the actor's visual for this token.
    #[serde(default)]
    pub visual: Option<TokenVisual>,
    /// Size override in GRID UNITS, replacing the actor's own `size`: a medium creature is `1` —
    /// one CELL on a square scene, one HEX on a hex scene (never a square block there; the
    /// footprint derives from the hex's own dimensions). `resolve_token_footprint` derives the
    /// footprint radius from the resolved value and bounds it by `MAX_FOOTPRINT_CELLS`, and the
    /// client multiplies it by the scene's cell size to reach scene units. Not to be confused with
    /// `TokenEngine.w`/`h`, which are the token's rendered box in scene units.
    #[serde(default)]
    pub size: Option<Size>,
    /// "square" | "circle" by client convention; ingress accepts any string within the
    /// engine band's `MAX_SYSTEM_BYTES` cap. A square scene's `resolve_footprint_cells` reads
    /// `"circle"` as a disc and every other value as a box; a hex scene ignores it.
    #[serde(default)]
    pub shape: Option<String>,
    /// Per-token vision override: replaces the actor's `vision[]` entirely
    /// when present.
    #[serde(default)]
    pub vision: Option<Vec<VisionAssignment>>,
    /// Per-token facing-limited field of view: replaces the actor's `facing_vision` wholesale
    /// when present. Ignored unless the scene's `facing_vision` switch is on. Writing it is
    /// GM-only (`permission::facing_vision_touched`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub facing_vision: Option<FacingVision>,
    /// Per-token light override: replaces the actor's `light` entirely when
    /// present (wholesale, same shape as `vision`); an emission with
    /// `enabled: false` suppresses this token's carried light. Authoring it is
    /// GM-only at ingress (`permission::carried_light_touched`): an emission joins the shared
    /// illumination field every viewer's mask reads, unlike the other owner-writable overrides.
    #[serde(default)]
    pub light: Option<LightEmission>,
    /// Per-token movement-tag override: replaces the actor's resolved movement set (actor ∪
    /// faction) entirely when present — wholesale, same shape as `vision`. The engine-reserved
    /// semantics of `"flying"`/`"incorporeal"` are stated on `ActorEngine::movement`; they apply
    /// identically to tags arriving through this override.
    #[serde(default)]
    pub movement: Option<Vec<String>>,
    /// Per-token aura override: replaces the actor's `aura` entirely when
    /// present (wholesale, never merged).
    #[serde(default)]
    pub aura: Option<AuraEmission>,
    /// Per-token sound override: replaces the actor's `sound` entirely when
    /// present (wholesale, never merged).
    #[serde(default)]
    pub sound: Option<SoundEmission>,
    /// Per-token VFX override: replaces the actor's `vfx` entirely when
    /// present (wholesale, never merged).
    #[serde(default)]
    pub vfx: Option<VfxEmission>,
    /// Per-token disposition override (friendly/neutral/hostile), independent of the linked
    /// faction's `Faction.stance`. Precedence: this value, else the faction's stance, else
    /// neutral. Reuses `FactionStance`, the one three-value stance enum.
    #[serde(default)]
    pub disposition: Option<FactionStance>,
    /// Per-token bar list: replaces the actor's `bars` entirely when present (wholesale, never
    /// merged); at most `MAX_BAR_ATTRIBUTES` entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub bars: Option<Vec<BarAttribute>>,
    /// Per-token nameplate delivery override, replacing the actor's `nameplate_mode` when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub nameplate_mode: Option<BarDisplayMode>,
}

impl TokenOverrides {
    /// Ingress validation of the emission payloads (`light`/`aura`/`sound`/`vfx`) and, when
    /// present, `visual` (through `TokenVisual::validate`); every other whitelisted field's
    /// shape is serde-enforced.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if let Some(visual) = &self.visual {
            visual
                .validate()
                .map_err(|m| format!("overrides.visual: {m}"))?;
        }
        if let Some(f) = &self.facing_vision {
            f.validate().map_err(|m| format!("overrides.{m}"))?;
        }
        if let Some(light) = &self.light {
            light
                .validate()
                .map_err(|m| format!("overrides.light: {m}"))?;
        }
        if let Some(aura) = &self.aura {
            aura.validate()?;
        }
        if let Some(sound) = &self.sound {
            sound.validate()?;
        }
        if let Some(vfx) = &self.vfx {
            vfx.validate()?;
        }
        if let Some(bars) = &self.bars {
            validate_bars(bars, "overrides.bars")?;
        }
        Ok(())
    }
}

/// A width/height pair in GRID UNITS (cells) — an actor's occupied block, not a pixel box.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::Size;
///
/// let size = Size { w: 2.0, h: 2.0 };
/// assert_eq!(size.w, 2.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct Size {
    /// Width in GRID UNITS: a medium creature is `1` — one CELL on square, one HEX on hex.
    /// `resolve_token_footprint` derives the footprint radius from this directly (via each grid
    /// kind's own model — a square block's half-diagonal, or a hex count's circumscribing radius)
    /// and bounds it by `MAX_FOOTPRINT_CELLS`, and the client multiplies it by the scene's cell
    /// size to reach scene units. Not to be confused with `TokenEngine.w`, which is the token's
    /// rendered box in scene units.
    pub w: f64,
    /// Height in GRID UNITS: a medium creature is `1` — one CELL on square, one HEX on hex.
    /// `resolve_token_footprint` derives the footprint radius from this directly (via each grid
    /// kind's own model — a square block's half-diagonal, or a hex count's circumscribing radius)
    /// and bounds it by `MAX_FOOTPRINT_CELLS`, and the client multiplies it by the scene's cell
    /// size to reach scene units. Not to be confused with `TokenEngine.h`, which is the token's
    /// rendered box in scene units.
    pub h: f64,
}

/// A per-actor or per-token vision assignment: which mode (by id, referencing
/// a `vision-modes` registry entry) + effective range in grid cells.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::VisionAssignment;
///
/// let assignment = VisionAssignment { mode: "darkvision".to_string(), range: Some(12.0) };
/// assert_eq!(assignment.range, Some(12.0));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct VisionAssignment {
    /// `vision-modes` registry entry id.
    pub mode: String,
    /// Effective range in grid CELLS (not scene units). `None` inherits the
    /// referenced mode's own `VisionMode::default_range` — an omitted key
    /// deserializes to `None` (serde special-cases `Option` regardless of
    /// `#[serde(default)]`; verified in `token_vision_floors_falls_back_to_mode_default_range_when_assignment_omits_range`),
    /// so the GM-authored mode default becomes live only once a caller resolves
    /// it against the mode, never by a struct-level fallback here.
    pub range: Option<f64>,
}

/// A creature's facing-limited field of view: it sees a forward arc of `arc` degrees instead of
/// all round, plus an all-round awareness bubble of `near` cells. It takes effect only on a scene
/// whose resolved `facing_vision` switch is on (`ResolvedScene::facing_vision`); the forward
/// direction is the token's `TokenEngine::rotation`.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::FacingVision;
///
/// let guard = FacingVision { arc: 120.0, near: 1.0 };
/// assert!(guard.validate().is_ok());
/// assert!(FacingVision { arc: 0.0, near: 0.0 }.validate().is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct FacingVision {
    /// Full width of the forward arc in degrees, finite and in `(0, 360]`; `360` is no restriction.
    pub arc: f64,
    /// All-round awareness radius in grid CELLS, finite and `>= 0`: inside it facing does not
    /// matter.
    pub near: f64,
}

impl FacingVision {
    /// Ingress validation: `arc` finite and in `(0, 360]`, `near` finite and `>= 0`. A stored value
    /// that fails this at read time resolves to the bubble-only view (`scene::facing_arc_for`),
    /// never to full sight.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::engine::FacingVision;
    ///
    /// assert!(FacingVision { arc: 360.0, near: 0.0 }.validate().is_ok());
    /// assert!(FacingVision { arc: 361.0, near: 0.0 }.validate().is_err());
    /// ```
    pub fn validate(&self) -> Result<(), String> {
        if !self.arc.is_finite() || self.arc <= 0.0 || self.arc > 360.0 {
            return Err("facing_vision.arc must be finite and in (0, 360]".to_string());
        }
        if !self.near.is_finite() || self.near < 0.0 {
            return Err("facing_vision.near must be finite and >= 0".to_string());
        }
        Ok(())
    }
}

/// The client-owned token/actor visual union. Internally tagged on
/// `kind`; serde does not support `deny_unknown_fields` on an internally
/// tagged enum (a documented limitation — NOT applied here).
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::TokenVisual;
///
/// let visual = TokenVisual::Image { asset: "token-goblin.png".to_string(), transform: None };
/// let json = serde_json::to_value(&visual).unwrap();
/// // Internally tagged on `kind`, lowercase.
/// assert_eq!(json["kind"], "image");
/// assert_eq!(json["asset"], "token-goblin.png");
/// assert_eq!(serde_json::from_value::<TokenVisual>(json).unwrap(), visual);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TokenVisual {
    /// One static image.
    Image {
        /// Asset id of the image.
        asset: String,
        /// Authored anchor/scale/tint of the art, or `None` for the centered, unscaled, untinted
        /// default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        transform: Option<VisualTransform>,
    },
    /// A frame-animated visual.
    Animated {
        /// Where the frames come from.
        source: AnimatedSource,
        /// Playback rate, frames per second.
        fps: f64,
        /// Loop playback; false = play once and hold the last frame.
        #[serde(rename = "loop")]
        loop_: bool,
        /// Authored anchor/scale/tint of the art, or `None` for the centered, unscaled, untinted
        /// default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        transform: Option<VisualTransform>,
    },
    /// A named set of switchable faces.
    Faces {
        /// Face name -> drawable visual (never nested `faces`).
        faces: BTreeMap<String, RenderVisual>,
        /// Face shown when nothing selects otherwise.
        default: String,
        /// Optional conditionId -> face name map; the first match (in the
        /// token's effective `conditions[]` order) wins over `default`, but
        /// never over a manual `token.engine.face`.
        #[serde(default, rename = "faceMap")]
        face_map: Option<BTreeMap<String, String>>,
    },
    /// A generated visual — see `RenderVisual::Generated`, whose payload this
    /// mirrors so an actor's whole visual can be a generated composition.
    Generated {
        /// The art being framed — an `Image` or `Animated` visual only.
        art: Box<RenderVisual>,
        /// The shape the art is cropped to.
        crop: GeneratedCrop,
        /// Decorative ring drawn around the cropped art, or `None` for none.
        #[serde(default)]
        border: Option<GeneratedBorder>,
        /// Fill drawn behind the cropped art, or `None` for none.
        #[serde(default)]
        background: Option<GeneratedBackground>,
    },
    /// A pool of candidate images, resolved to one concrete `Image` server-side on every save
    /// that writes it (`SqliteRepository::resolve_token_random_variants`, at a token's Create
    /// and at a token's Update), for every per-token storage location it appears in
    /// (`TokenEngine.visual`, `TokenOverrides.visual`, each embedded-actor copy's
    /// `engine.visual` of an instanced token). A token document never keeps a `RandomVariant`
    /// past the save that set it. The one place it persists is a SHARED actor's own
    /// `ActorEngine.visual`: there it is the template, each linked token draws independently
    /// into its own `overrides.visual` when created, and every save of the actor's pool draws
    /// again for each token already linked (`SqliteRepository::redraw_linked_token_art`).
    #[serde(rename = "randomVariant")]
    RandomVariant {
        /// Candidate pool, scoped to the token's world and to image assets at draw time.
        filter: TokenArtFilter,
        /// Used verbatim when `filter` matches no asset, so a token is never left with no visual.
        /// `TokenVisual::validate` refuses a `RandomVariant` here: `Box<TokenVisual>` does not
        /// exclude the recursive case at the type level the way `Box<RenderVisual>` does.
        fallback: Box<TokenVisual>,
    },
}

/// The pool specification of `TokenVisual::RandomVariant`: a restricted mirror of
/// `data::asset::query::AssetFilter` (folder and tags only; a pool is authored, never searched).
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::TokenArtFilter;
///
/// let filter = TokenArtFilter::default();
/// assert!(filter.folder.is_none() && filter.tags.is_empty());
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct TokenArtFilter {
    /// Restrict the pool to this `asset_folder`'s subtree (`FolderFilter::In` with
    /// `recursive: true`); `None` is the whole world (`FolderFilter::Any`).
    #[ts(optional)]
    pub folder: Option<Uuid>,
    /// Every listed tag must be present (explicit or derived), the all-of semantics of
    /// `AssetFilter.tags`.
    pub tags: Vec<String>,
}

impl TokenVisual {
    /// Ingress validation beyond serde shape: every asset ref (`Image`/`Animated`'s source)
    /// and every colour field (`Generated`'s border/background) validates through their own
    /// `validate`; `Faces`' member visuals and `Generated`'s `art` recurse through
    /// `RenderVisual::validate`.
    pub(crate) fn validate(&self) -> Result<(), String> {
        match self {
            TokenVisual::Image { asset, transform } => {
                crate::data::engine::geometry::validate_asset_ref(asset)
                    .map_err(|m| format!("asset: {m}"))?;
                validate_transform(transform)
            }
            TokenVisual::Animated {
                source, transform, ..
            } => {
                source.validate()?;
                validate_transform(transform)
            }
            TokenVisual::Faces { faces, .. } => {
                for (name, visual) in faces {
                    visual
                        .validate()
                        .map_err(|m| format!("faces.{name}: {m}"))?;
                }
                Ok(())
            }
            TokenVisual::Generated {
                art,
                border,
                background,
                ..
            } => {
                art.validate().map_err(|m| format!("art: {m}"))?;
                if let Some(b) = border {
                    b.validate().map_err(|m| format!("border: {m}"))?;
                }
                if let Some(b) = background {
                    b.validate().map_err(|m| format!("background: {m}"))?;
                }
                Ok(())
            }
            TokenVisual::RandomVariant { fallback, .. } => {
                if matches!(**fallback, TokenVisual::RandomVariant { .. }) {
                    return Err("a random-variant pool's fallback cannot itself be a pool".into());
                }
                fallback.validate().map_err(|m| format!("fallback: {m}"))
            }
        }
    }
}

/// The kinds the render layer actually draws — the render/resolution
/// boundary. A face's own visual is always one of these — no `faces` nesting
/// (a face can never itself be `{kind:"faces"}`).
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::RenderVisual;
///
/// let visual = RenderVisual::Image { asset: "token-goblin.png".to_string(), transform: None };
/// let json = serde_json::to_value(&visual).unwrap();
/// assert_eq!(json["kind"], "image");
/// // A face's visual is a leaf kind: the `faces` discriminant is refused here.
/// let nested = serde_json::json!({ "kind": "faces", "faces": [] });
/// assert!(serde_json::from_value::<RenderVisual>(nested).is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RenderVisual {
    /// One static image.
    Image {
        /// Asset id of the image.
        asset: String,
        /// Authored anchor/scale/tint of the art, or `None` for the centered, unscaled, untinted
        /// default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        transform: Option<VisualTransform>,
    },
    /// A frame-animated visual.
    Animated {
        /// Where the frames come from.
        source: AnimatedSource,
        /// Playback rate, frames per second.
        fps: f64,
        /// Loop playback; false = play once and hold the last frame.
        #[serde(rename = "loop")]
        loop_: bool,
        /// Authored anchor/scale/tint of the art, or `None` for the centered, unscaled, untinted
        /// default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        transform: Option<VisualTransform>,
    },
    /// A generated token visual: existing art framed by a shape crop, an
    /// optional decorative border ring, and an optional background fill. The
    /// decorative ring is authored data, distinct from the faction ring the
    /// render layer draws from the faction registry.
    Generated {
        /// The art being framed — an `Image` or `Animated` visual only; a
        /// nested `Generated` (or anything else) fails closed to no visual at
        /// the resolution boundary (`resolveTokenVisual`), which is the read-side
        /// guard compensating for this field's unrestricted serde shape.
        art: Box<RenderVisual>,
        /// The shape the art is cropped to.
        crop: GeneratedCrop,
        /// Decorative ring drawn around the cropped art, or `None` for none.
        #[serde(default)]
        border: Option<GeneratedBorder>,
        /// Fill drawn behind the cropped art, or `None` for none.
        #[serde(default)]
        background: Option<GeneratedBackground>,
    },
}

impl RenderVisual {
    /// Ingress validation beyond serde shape: every asset ref and every colour field, mirroring
    /// `TokenVisual::validate` minus the `Faces` arm (a face's own visual is always a leaf kind
    /// here, never `Faces` itself — see this enum's own doc).
    pub(crate) fn validate(&self) -> Result<(), String> {
        match self {
            RenderVisual::Image { asset, transform } => {
                crate::data::engine::geometry::validate_asset_ref(asset)
                    .map_err(|m| format!("asset: {m}"))?;
                validate_transform(transform)
            }
            RenderVisual::Animated {
                source, transform, ..
            } => {
                source.validate()?;
                validate_transform(transform)
            }
            RenderVisual::Generated {
                art,
                border,
                background,
                ..
            } => {
                art.validate().map_err(|m| format!("art: {m}"))?;
                if let Some(b) = border {
                    b.validate().map_err(|m| format!("border: {m}"))?;
                }
                if let Some(b) = background {
                    b.validate().map_err(|m| format!("background: {m}"))?;
                }
                Ok(())
            }
        }
    }
}

/// The crop shape of a generated token visual (`RenderVisual::Generated`).
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::GeneratedCrop;
///
/// assert_ne!(GeneratedCrop::Circle, GeneratedCrop::Square);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "lowercase")]
pub enum GeneratedCrop {
    /// The inscribed ellipse of the token's extent.
    Circle,
    /// The token's extent rectangle.
    Square,
}

/// A generated token visual's decorative border ring
/// (`RenderVisual::Generated`), distinct from the faction ring.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::GeneratedBorder;
///
/// let border = GeneratedBorder { color: "#000000".to_string(), width: 0.1 };
/// assert_eq!(border.width, 0.1);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct GeneratedBorder {
    /// Ring color, a css `#rrggbb` string.
    pub color: String,
    /// Ring width, in token-fraction px.
    pub width: f64,
}

impl GeneratedBorder {
    /// Ingress validation beyond serde shape: css-`#rrggbb` color and a finite width.
    pub(crate) fn validate(&self) -> Result<(), String> {
        crate::data::engine::geometry::validate_hex_color(&self.color)?;
        if !self.width.is_finite() {
            return Err("border width must be finite".to_string());
        }
        Ok(())
    }
}

/// A generated token visual's background fill (`RenderVisual::Generated`),
/// drawn behind the cropped art in the crop shape.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::GeneratedBackground;
///
/// let background = GeneratedBackground { color: "#ffffff".to_string() };
/// assert_eq!(background.color, "#ffffff");
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct GeneratedBackground {
    /// Fill color, a css `#rrggbb` string.
    pub color: String,
}

impl GeneratedBackground {
    /// Ingress validation beyond serde shape: css-`#rrggbb` color.
    pub(crate) fn validate(&self) -> Result<(), String> {
        crate::data::engine::geometry::validate_hex_color(&self.color)
    }
}

/// The point of the art that aligns to the token's footprint origin, as a fraction of the art's
/// own box (`{0.5, 0.5}` is the centered default). Not clamped to `[0, 1]`: an author may anchor
/// outside the box on purpose.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::TransformAnchor;
///
/// let anchor = TransformAnchor { x: 0.5, y: 0.75 };
/// assert_eq!(anchor.y, 0.75);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct TransformAnchor {
    /// Horizontal anchor fraction.
    pub x: f64,
    /// Vertical anchor fraction.
    pub y: f64,
}

/// Authored framing of a plain image or animated token visual: where the art is anchored, how
/// much it is scaled against the token's box, and a static multiply tint. `"generated"` visuals
/// frame their art with crop/border/background instead and carry no transform of their own; a
/// generated visual's nested `art` may carry one.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::{TransformAnchor, VisualTransform};
///
/// let transform = VisualTransform {
///     anchor: TransformAnchor { x: 0.5, y: 0.6 },
///     scale: 1.25,
///     tint: Some("#ffcc88".to_string()),
///     facing: Some(270.0),
/// };
/// assert_eq!(transform.scale, 1.25);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct VisualTransform {
    /// Art point aligned to the token's footprint origin.
    pub anchor: TransformAnchor,
    /// Multiplier on the art's drawn size against the token's box; `> 1` bleeds past the
    /// footprint. Finite and positive.
    pub scale: f64,
    /// Multiply tint, a css `#rrggbb` string, or `None` for none.
    #[serde(default)]
    #[ts(optional)]
    pub tint: Option<String>,
    /// The direction the art itself faces as drawn, in degrees in the `TokenEngine::rotation`
    /// convention (0 faces +x, clockwise on the y-down canvas, so 90 faces down and 270 faces
    /// up), in `[0, 360)`. The renderer turns the art by the token's rotation minus this angle,
    /// so the art points along the token's facing. `None` means the art is drawn facing +x.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub facing: Option<f64>,
}

impl VisualTransform {
    /// Ingress validation beyond serde shape: finite anchor, finite positive scale, a css
    /// `#rrggbb` tint when present, and a finite `facing` in `[0, 360)` when present. The anchor
    /// is deliberately not range-clamped.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if let Some(facing) = self.facing {
            if !facing.is_finite() || !(0.0..360.0).contains(&facing) {
                return Err("transform facing must be finite and in [0, 360)".to_string());
            }
        }
        if !self.anchor.x.is_finite() || !self.anchor.y.is_finite() {
            return Err("transform anchor must be finite".to_string());
        }
        if !self.scale.is_finite() || self.scale <= 0.0 {
            return Err("transform scale must be finite and positive".to_string());
        }
        if let Some(tint) = &self.tint {
            crate::data::engine::geometry::validate_hex_color(tint)?;
        }
        Ok(())
    }
}

/// Validates an optional `VisualTransform`, prefixing a failure with its field name; `None`
/// passes. Shared by every `TokenVisual`/`RenderVisual` arm that carries a transform.
fn validate_transform(transform: &Option<VisualTransform>) -> Result<(), String> {
    match transform {
        Some(t) => t.validate().map_err(|m| format!("transform: {m}")),
        None => Ok(()),
    }
}

/// An animated visual's frame source: an ordered list of individually
/// uploaded assets, or one grid-sliced sheet asset.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::AnimatedSource;
///
/// let source = AnimatedSource::Sheet {
///     asset: "sheet.png".to_string(),
///     rows: 4,
///     cols: 4,
///     count: Some(12),
/// };
/// let json = serde_json::to_value(&source).unwrap();
/// // Internally tagged on `type`, lowercase.
/// assert_eq!(json["type"], "sheet");
/// assert_eq!(json["count"], 12);
/// assert_eq!(serde_json::from_value::<AnimatedSource>(json).unwrap(), source);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum AnimatedSource {
    /// An ordered list of individually uploaded frame assets.
    Frames {
        /// Asset ids, playback order.
        frames: Vec<String>,
    },
    /// One sheet asset sliced into a grid of frames.
    Sheet {
        /// Asset id of the sheet image.
        asset: String,
        /// Grid rows in the sheet.
        rows: u32,
        /// Grid columns in the sheet.
        cols: u32,
        /// Frames actually used (row-major from the top-left); absent =
        /// rows * cols.
        #[serde(default)]
        count: Option<u32>,
    },
}

impl AnimatedSource {
    /// Ingress validation beyond serde shape: every asset ref (`Frames`' whole list, or
    /// `Sheet`'s single `asset`) validates through `geometry::validate_asset_ref`.
    pub(crate) fn validate(&self) -> Result<(), String> {
        match self {
            AnimatedSource::Frames { frames } => {
                for (i, asset) in frames.iter().enumerate() {
                    crate::data::engine::geometry::validate_asset_ref(asset)
                        .map_err(|m| format!("frames[{i}]: {m}"))?;
                }
                Ok(())
            }
            AnimatedSource::Sheet { asset, .. } => {
                crate::data::engine::geometry::validate_asset_ref(asset)
                    .map_err(|m| format!("asset: {m}"))
            }
        }
    }
}

/// An actor's engine-owned body (exported to the client as `ActorEngine`).
/// `name` lives on the envelope and the actor's other data (inventory,
/// stats, …) lives in `system` — this is a SPLIT type.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::{ActorEngine, Size, TokenVisual};
///
/// let actor = ActorEngine {
///     display_name: "Goblin Scout".to_string(),
///     visual: TokenVisual::Image { asset: "token-goblin.png".to_string(), transform: None },
///     size: Size { w: 1.0, h: 1.0 },
///     shape: "square".to_string(),
///     faction: None,
///     conditions: Vec::new(),
///     prototype: false,
///     vision: None,
///     facing_vision: None,
///     light: None,
///     movement: Vec::new(),
///     aura: None,
///     sound: None,
///     vfx: None,
///     bars: None,
///     nameplate_mode: None,
/// };
/// assert_eq!(actor.display_name, "Goblin Scout");
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct ActorEngine {
    /// The client's `ActorEngine.displayName: string` — required, non-nullable.
    #[serde(rename = "displayName")]
    pub display_name: String,
    /// The actor's visual, inherited by linked tokens (raw-token/override
    /// visuals take precedence per `resolveTokenVisual`'s resolution order).
    pub visual: TokenVisual,
    /// Default token size for this actor in GRID UNITS: a medium creature is `1` — one CELL on
    /// square, one HEX on hex. `resolve_token_footprint` derives the footprint radius from this
    /// directly (via each grid kind's own model) and bounds it by `MAX_FOOTPRINT_CELLS`, and the
    /// client multiplies it by the scene's cell size to reach scene units. Not to be confused with
    /// `TokenEngine.w`/`h`, which are the token's rendered box in scene units.
    pub size: Size,
    /// "square" | "circle"; a `String`, not a Rust enum. `shape_literal_set_deserializes` checks
    /// that both literals pass `validate_engine`.
    pub shape: String,
    /// The client's `ActorEngine.faction: string | null`. INVARIANT: `Option<T>`
    /// always accepts a missing key as `None` (serde special-cases `Option`
    /// regardless of `#[serde(default)]`) — absent and explicit `null` are
    /// ingress-equivalent here, not distinguishable. The re-serialized,
    /// persisted form always writes an explicit `null` for `None`, restoring
    /// exact parity with the client's `faction: string | null` contract on
    /// the stored/broadcast side.
    pub faction: Option<String>,
    /// The client's `ActorEngine.conditions: string[]` — the key is required.
    pub conditions: Vec<String>,
    /// Default place-mode: true ⇒ instance (independent copy) on drop; false
    /// ⇒ link (shared).
    pub prototype: bool,
    /// Vision modes granted to this actor; each references a `VisionMode` id
    /// + range in grid cells.
    #[serde(default)]
    pub vision: Option<Vec<VisionAssignment>>,
    /// The actor's facing-limited field of view, inherited by every token resolving this actor
    /// (a per-token `TokenOverrides::facing_vision` replaces it wholesale). Ignored unless the
    /// scene's `facing_vision` switch is on. Writing it is GM-only
    /// (`permission::facing_vision_touched`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub facing_vision: Option<FacingVision>,
    /// Light this actor's tokens carry: every token resolving this actor
    /// emits it at its live position unless the token's override replaces or
    /// suppresses it (`TokenOverrides::light`). Writing it is GM-only at ingress
    /// (`permission::carried_light_touched`), since an emission edits the shared illumination
    /// field every viewer's mask reads.
    #[serde(default)]
    pub light: Option<LightEmission>,
    /// Movement-type tags (system vocabulary space, same posture as `conditions`). The engine
    /// reserves exactly two — `"flying"` and `"incorporeal"` — each meaning the mover ignores
    /// difficult-terrain COST (`RegionField::terrain_multiplier` reads as 1.0) and NOTHING else:
    /// walls still gate, impassable regions still block, arrest regions still stop, and the
    /// visibility mask still gates. Unknown tags are carried as inert data for system modules to
    /// interpret. Resolution (`SceneEcs::token_movement_tags` / `resolveTokenActor`) unions these
    /// with the linked faction's `Faction.movement`; a token override replaces the whole set.
    #[serde(default)]
    pub movement: Vec<String>,
    /// The actor's aura emission, inherited by linked tokens (a per-token
    /// `TokenOverrides.aura` replaces it wholesale).
    #[serde(default)]
    pub aura: Option<AuraEmission>,
    /// The actor's sound emission, inherited by linked tokens (a per-token
    /// `TokenOverrides.sound` replaces it wholesale).
    #[serde(default)]
    pub sound: Option<SoundEmission>,
    /// The actor's VFX emission, inherited by linked tokens (a per-token
    /// `TokenOverrides.vfx` replaces it wholesale).
    #[serde(default)]
    pub vfx: Option<VfxEmission>,
    /// Attribute bars drawn on this actor's tokens, inherited by linked tokens (a per-token
    /// `TokenOverrides.bars` replaces the list wholesale); at most `MAX_BAR_ATTRIBUTES`. Absent
    /// means none. Delivered as evaluated numbers through the `"bars"` derived channel, never
    /// as this authored form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub bars: Option<Vec<BarAttribute>>,
    /// Who receives the nameplate of this actor's tokens; absent means `BarDisplayMode::Hover`
    /// (a per-token `TokenOverrides.nameplate_mode` replaces it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub nameplate_mode: Option<BarDisplayMode>,
}

impl ActorEngine {
    /// Ingress validation beyond serde shape: the required `visual` (through
    /// `TokenVisual::validate`) and the emission payloads (`light` through
    /// `LightEmission::validate`, plus `aura`/`sound`/`vfx`) — every other field keeps its
    /// serde-enforced shape.
    pub(crate) fn validate(&self) -> Result<(), String> {
        self.visual.validate().map_err(|m| format!("visual: {m}"))?;
        if let Some(f) = &self.facing_vision {
            f.validate()?;
        }
        if let Some(light) = &self.light {
            light.validate().map_err(|m| format!("light: {m}"))?;
        }
        if let Some(aura) = &self.aura {
            aura.validate()?;
        }
        if let Some(sound) = &self.sound {
            sound.validate()?;
        }
        if let Some(vfx) = &self.vfx {
            vfx.validate()?;
        }
        if let Some(bars) = &self.bars {
            validate_bars(bars, "bars")?;
        }
        Ok(())
    }
}
