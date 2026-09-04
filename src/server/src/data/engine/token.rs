//! `token`/`actor` engine bands. Field shapes mirror the
//! client's re-exported `TokenEngine`/`ActorEngine` (minus `name`, which
//! lives on the envelope instead).

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two crate-level deny attributes this module declares.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::data::engine::scene::LightEmission;

/// A token's transform + visual (mirrors the client's `TokenEngine`). `(x,y)`
/// is the token CENTER. `visual` is set only on raw (actorless) tokens —
/// actor-backed tokens resolve their visual via the linked/embedded actor.
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
}

impl TokenEngine {
    /// Ingress validation beyond serde shape: every numeric field finite, and
    /// the position inside the ONE shared movement-coordinate bound
    /// (`scene::move_exec::MAX_GATE_WALK_COORD`) — the GM-write/Create path
    /// and the move gate must agree on admissible coordinates structurally,
    /// never by call ordering. The override whitelist's emission payloads
    /// validate through `TokenOverrides::validate` here, so an actor-backed
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
        let bound = crate::scene::move_exec::MAX_GATE_WALK_COORD;
        if self.x.abs() > bound || self.y.abs() > bound {
            return Err(format!("position exceeds coordinate bound {bound}"));
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "snake_case")]
pub enum VfxAnchor {
    /// On the token itself.
    Token,
    /// Above the token's art.
    Above,
    /// Below the token's art.
    Below,
}

/// An aura emission: a colored disc radiating `radius` grid cells from the
/// token's center, drawn UNDER its art. Purely presentational — nothing
/// server-side consumes it.
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
        validate_emission_color(&self.color)?;
        validate_emission_scalar("opacity", self.opacity)?;
        validate_emission_radius(self.radius)?;
        Ok(())
    }
}

/// A sound emission: a looping or one-shot audio asset audible within
/// `radius` grid cells. Playback-ready data only — no playback consumer exists
/// yet, so nothing server-side or client-side reads it beyond storage.
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

/// A VFX emission: a visual effect asset anchored to the token. Playback-ready
/// data only — no playback consumer exists yet, so nothing server-side or
/// client-side reads it beyond storage.
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

/// Shared emission-payload color check: exactly `#` + 6 hex digits (a css
/// `#rrggbb` string).
fn validate_emission_color(color: &str) -> Result<(), String> {
    match color.strip_prefix('#') {
        Some(hex) if hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()) => Ok(()),
        _ => Err("color must be a css #rrggbb string".to_string()),
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

/// Shared emission-payload asset check: non-empty (an empty id can never
/// resolve to an asset).
fn validate_emission_asset(asset: &str) -> Result<(), String> {
    if asset.is_empty() {
        return Err("asset must be non-empty".to_string());
    }
    Ok(())
}

/// The per-token override whitelist for a linked token.
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
    /// "square" | "circle" — kept a `String` in v1 (the literal set is
    /// asserted by the unit battery, not enforced by a Rust enum).
    #[serde(default)]
    pub shape: Option<String>,
    /// Per-token vision override: replaces the actor's `vision[]` entirely
    /// when present.
    #[serde(default)]
    pub vision: Option<Vec<VisionAssignment>>,
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
}

impl TokenOverrides {
    /// Ingress validation of the emission payloads only (`light`/`aura`/`sound`/`vfx`);
    /// every other whitelisted field's shape is serde-enforced.
    pub(crate) fn validate(&self) -> Result<(), String> {
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
        Ok(())
    }
}

/// A width/height pair in GRID UNITS (cells) — an actor's occupied block, not a pixel box.
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

/// The client-owned token/actor visual union. Internally tagged on
/// `kind`; serde does not support `deny_unknown_fields` on an internally
/// tagged enum (a documented limitation — NOT applied here).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TokenVisual {
    /// One static image.
    Image {
        /// Asset id of the image.
        asset: String,
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
}

/// The kinds the render layer actually draws — the render/resolution
/// boundary. A face's own visual is always one of these — no `faces` nesting
/// (a face can never itself be `{kind:"faces"}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RenderVisual {
    /// One static image.
    Image {
        /// Asset id of the image.
        asset: String,
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

/// The crop shape of a generated token visual (`RenderVisual::Generated`).
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct GeneratedBorder {
    /// Ring color, a css `#rrggbb` string.
    pub color: String,
    /// Ring width, in token-fraction px.
    pub width: f64,
}

/// A generated token visual's background fill (`RenderVisual::Generated`),
/// drawn behind the cropped art in the crop shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct GeneratedBackground {
    /// Fill color, a css `#rrggbb` string.
    pub color: String,
}

/// An animated visual's frame source: an ordered list of individually
/// uploaded assets, or one grid-sliced sheet asset.
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

/// An actor's engine-owned body (mirrors the client's `ActorEngine`, minus
/// `name` which moves to the envelope). Every other field of `ActorEngine`
/// (inventory, stats, …) lives in `system` — this is a SPLIT type.
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
    /// "square" | "circle" — kept a `String` in v1 (asserted by the battery).
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
}

impl ActorEngine {
    /// Ingress validation beyond serde shape, covering ONLY the emission
    /// payloads (`light` through `LightEmission::validate`, plus `aura`/`sound`/`vfx`) — every
    /// other field keeps its serde-enforced shape.
    pub(crate) fn validate(&self) -> Result<(), String> {
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
        Ok(())
    }
}
