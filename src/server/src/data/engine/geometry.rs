//! This module defines the `wall`, `region`, `drawing` and `template` engine bands.
//!
//! Each band struct is exported to TypeScript through ts-rs, and `@shadowcat/core` re-exports the
//! generated types (`WallEngine`, `RegionEngine`, `DrawingEngine`, `TemplateEngine`). The server
//! reads walls and regions through `SceneEcs::engine_as_cached::<WallEngine>`/`<RegionEngine>`.
//! These structs are the only authoritative shapes, and scene-tools writers must round-trip
//! byte-identically against them.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two crate-level deny attributes this module declares.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[cfg(test)]
mod tests;

/// A line segment in scene units (the scene's continuous coordinate space,
/// not grid cells).
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::Seg;
///
/// let seg = Seg { x1: 0.0, y1: 0.0, x2: 3.0, y2: 4.0 };
/// let len = ((seg.x2 - seg.x1).powi(2) + (seg.y2 - seg.y1).powi(2)).sqrt();
/// assert_eq!(len, 5.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct Seg {
    /// Start point x, scene units.
    pub x1: f64,
    /// Start point y, scene units.
    pub y1: f64,
    /// End point x, scene units.
    pub x2: f64,
    /// End point y, scene units.
    pub y2: f64,
}

/// The elevation interval a banded geometry occupies. ONE type shared by every
/// banded-geometry engine — a wall's occlusion band (`WallEngine::elevation`) and
/// the level band a region/drawing/template sits on (`RegionEngine::elevation`,
/// `DrawingEngine::elevation`, `TemplateEngine::elevation`) — never a per-engine
/// copy. A sight/light source at elevation `e` is occluded by a wall iff
/// `bottom ≤ e ≤ top`; an absent end is unbounded (`bottom: None` = −∞,
/// `top: None` = +∞). An absent band applies at every elevation, and a malformed
/// interval (`bottom > top`, or a non-finite endpoint) fails closed to applying
/// everywhere — see `scene::elevation::band_contains`, the ONE point-in-band
/// predicate every consumer (`scene::elevation::wall_occludes`, the movement
/// gate, region selection, the render filters) calls.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::ElevationBand;
///
/// let unbounded = ElevationBand { bottom: None, top: None };
/// assert!(unbounded.bottom.is_none());
///
/// let band = ElevationBand { bottom: Some(0.0), top: Some(10.0) };
/// assert_eq!(band.top, Some(10.0));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct ElevationBand {
    /// Lower end of the occluded band; absent = unbounded below.
    #[serde(default)]
    pub bottom: Option<f64>,
    /// Upper end of the occluded band; absent = unbounded above.
    #[serde(default)]
    pub top: Option<f64>,
}

/// A wall's door state (`WallEngine::door`): whether the segment is a door at
/// all, and when it is, whether it stands closed or open. `Closed` and `Locked`
/// both apply the wall's authored `blocks_*` flags unchanged — `Locked` differs
/// from `Closed` only in who may flip it (a non-GM's `ClientMsg::ToggleDoor`
/// may flip `Closed`↔`Open` only, never onto or off `Locked`; see
/// `ws::room::Room::toggle_door`), never in what it blocks. `Open` resolves
/// every blocking flag to `false` through `WallEngine::effective_flags` — an
/// open door is a HOLE, not merely a wall with one axis un-blocked.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::DoorState;
///
/// let json = serde_json::to_value(DoorState::Open).unwrap();
/// assert_eq!(json, serde_json::json!("open"));
/// assert_eq!(serde_json::from_value::<DoorState>(json).unwrap(), DoorState::Open);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "snake_case")]
pub enum DoorState {
    /// Shut but unlatched: a token's owner standing near and in sight of the
    /// door may open it.
    Closed,
    /// Open: the wall blocks nothing until shut again.
    Open,
    /// Locked: blocks exactly like `Closed`, but only a GM may change the state.
    Locked,
}

/// A wall's one-way direction (`WallEngine::direction`): which side of the segment `a→b`
/// the wall behaves as SOLID from, expressed as the sign of the standard 2D cross product
/// `(b - a) × (p - a)` evaluated at the observation point `p`. `blocking_side: 1.0` puts the
/// blocking side where that cross product is POSITIVE; `-1.0` puts it where the product is
/// negative. From the transparent side the wall is absent for the sight raycast and the
/// movement gate — the same one sign serves both, evaluated against a different "viewpoint"
/// (the ray's origin for sight, the mover's pre-step position for movement; see
/// `vision::Seg::transparent_from`), so the two gates cannot fork on which side is solid.
/// Light and sound occlusion never read this field: they stay symmetric.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::WallDirection;
///
/// let json = serde_json::json!({ "blockingSide": 1.0 });
/// let dir: WallDirection = serde_json::from_value(json).unwrap();
/// assert_eq!(dir.blocking_side, 1.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WallDirection {
    /// The side the wall is solid from: exactly `1.0` or `-1.0`, enforced at ingress by
    /// `WallEngine::validate` — any other value is meaningless to the cross-product side
    /// test (a `0.0` would author a wall solid from NEITHER side) and is rejected rather
    /// than silently normalized.
    pub blocking_side: f64,
}

/// A wall's partial-blocking sight tier (`WallEngine::sight_tier`): `Limited` lets a sight
/// ray cross ONE such wall and continue, stopping at the next crossing of any wall — `Limited`
/// or ordinary — along that ray ("see through one such wall, not two"), implemented as
/// `vision::nearest_hit`'s per-ray `MAX_LIMITED_CROSSINGS` budget. An enum with one variant
/// rather than a bool, so a future tier is an additive variant rather than a breaking
/// field-type change. Sight only: movement, light and sound never read this field.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::WallSightTier;
///
/// let json = serde_json::to_value(WallSightTier::Limited).unwrap();
/// assert_eq!(json, serde_json::json!("limited"));
/// assert_eq!(serde_json::from_value::<WallSightTier>(json).unwrap(), WallSightTier::Limited);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "snake_case")]
pub enum WallSightTier {
    /// A sight ray crosses one `Limited` wall per ray; a second wall of any kind stops it.
    Limited,
}

/// The door-resolved blocking flags of a `WallEngine` — the ONE resolver output
/// every wall collector (`SceneEcs::sight_wall_entries`/`light_wall_entries`/
/// `sound_wall_entries`/`move_wall_entries`) reads, never the authored `blocks_*`
/// fields directly. Returned as one struct so no caller can read a subset of the
/// flags and silently mis-derive the rest.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::EffectiveWallFlags;
///
/// let flags = EffectiveWallFlags { blocks_move: true, blocks_sight: false, blocks_light: true, blocks_sound: false };
/// assert!(flags.blocks_move);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveWallFlags {
    /// Whether the wall blocks token movement.
    pub blocks_move: bool,
    /// Whether the wall occludes vision rays.
    pub blocks_sight: bool,
    /// Whether the wall occludes light propagation.
    pub blocks_light: bool,
    /// Whether the wall occludes carried-sound emitters. The one flag with an
    /// inheritance rule: an unauthored `WallEngine::blocks_sound` resolves to the
    /// door-resolved `blocks_sight`, so a wall that never authors the field
    /// occludes sound exactly as it occludes sight.
    pub blocks_sound: bool,
}

/// A wall's segment + sight/light/movement/sound-blocking flags. Absent/false
/// flags exclude the wall from that gate, matching how each gate
/// (`move_exec`/`pathfinding`/`lighting`/`scene::audibility`) reads these fields.
/// `blocks_sound` is the one flag with an inheritance default: absent, it resolves
/// to the door-resolved `blocks_sight` (`WallEngine::effective_flags`), so a wall
/// that never authors the field occludes sound exactly as it occludes sight.
///
/// Three orthogonal modifiers sit beside the flags: `door` (a hole when open, resolved by
/// `WallEngine::effective_flags`), `direction` (a one-way wall, solid from one side of the
/// segment only — sight and movement, never light/sound) and `sight_tier` (a sight-only
/// partial-blocking tier — see `WallSightTier`). None of them changes the authored flags'
/// meaning; they compose with `effective_flags`'s output at the collectors.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::{Seg, WallEngine};
///
/// let wall = WallEngine {
///     seg: Seg { x1: 0.0, y1: 0.0, x2: 10.0, y2: 0.0 },
///     blocks_sight: Some(true),
///     blocks_light: Some(true),
///     blocks_move: Some(true),
///     blocks_sound: None,
///     elevation: None,
///     door: None,
///     direction: None,
///     sight_tier: None,
/// };
/// assert_eq!(wall.blocks_move, Some(true));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WallEngine {
    /// The wall's segment, scene units.
    pub seg: Seg,
    /// Occludes vision rays; absent/false = transparent to sight.
    #[serde(default)]
    pub blocks_sight: Option<bool>,
    /// Occludes light propagation; absent/false = transparent to light.
    #[serde(default)]
    pub blocks_light: Option<bool>,
    /// Blocks token movement: read via `SceneEcs::move_walls` and enforced by
    /// `scene::move_exec::execute_move`/`gate_walk` (the sole per-cell
    /// traversal decision). Absent/false = passable.
    #[serde(default)]
    pub blocks_move: Option<bool>,
    /// Occludes carried-sound emitters (`scene::audibility::compute_audibility`'s
    /// occlusion test). Absent = inherit the door-resolved `blocks_sight`
    /// (`effective_flags`): a stored document carrying no `blocksSound` key
    /// occludes sound exactly as it occludes sight.
    #[serde(default)]
    pub blocks_sound: Option<bool>,
    /// The elevation band this wall's sight/light/sound occlusion applies to;
    /// absent = occludes every elevation. Consulted by the movement gate exactly
    /// as it is by sight/light/sound occlusion — see `scene::elevation::band_contains`
    /// and `SceneEcs::move_wall_entries`.
    #[serde(default)]
    pub elevation: Option<ElevationBand>,
    /// The wall's door state; absent = not a door (an ordinary wall —
    /// `effective_flags` applies the authored flags unchanged).
    #[serde(default)]
    pub door: Option<DoorState>,
    /// One-way direction; absent = solid from both sides. Read by the sight raycast and
    /// the movement gate only (never by light/sound occlusion) — see `WallDirection` for
    /// the sign convention.
    #[serde(default)]
    pub direction: Option<WallDirection>,
    /// Partial-blocking sight tier; absent = the wall blocks sight immediately. Read by the
    /// sight raycast only (`vision::nearest_hit`'s crossing budget) — see `WallSightTier`.
    #[serde(default)]
    pub sight_tier: Option<WallSightTier>,
}

impl WallEngine {
    /// Resolve the wall's effective blocking flags through its door state: an
    /// `Open` door makes the wall fully passable/see-through/light-through/
    /// sound-transparent regardless of the authored flags; every other state
    /// (no door, `Closed`, `Locked`) applies the authored flags unchanged (each
    /// `Option<bool>` read as `== Some(true)`, matching every collector's
    /// convention), except `blocks_sound`, which alone inherits: an absent
    /// `blocks_sound` resolves to the door-resolved `blocks_sight`. This is the
    /// ONE resolver over authored flags + door state — never a second copy of
    /// the flags at a read site.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::engine::{DoorState, Seg, WallEngine};
    ///
    /// let mut wall = WallEngine {
    ///     seg: Seg { x1: 0.0, y1: 0.0, x2: 10.0, y2: 0.0 },
    ///     blocks_sight: Some(true),
    ///     blocks_light: Some(true),
    ///     blocks_move: Some(true),
    ///     blocks_sound: None,
    ///     elevation: None,
    ///     door: Some(DoorState::Open),
    ///     direction: None,
    ///     sight_tier: None,
    /// };
    /// assert!(!wall.effective_flags().blocks_move);
    /// wall.door = Some(DoorState::Closed);
    /// assert!(wall.effective_flags().blocks_move);
    /// // `blocks_sound` unauthored: inherits the door-resolved `blocks_sight`.
    /// assert!(wall.effective_flags().blocks_sound);
    /// ```
    pub fn effective_flags(&self) -> EffectiveWallFlags {
        if self.door == Some(DoorState::Open) {
            return EffectiveWallFlags {
                blocks_move: false,
                blocks_sight: false,
                blocks_light: false,
                blocks_sound: false,
            };
        }
        let blocks_sight = self.blocks_sight == Some(true);
        EffectiveWallFlags {
            blocks_move: self.blocks_move == Some(true),
            blocks_sight,
            blocks_light: self.blocks_light == Some(true),
            blocks_sound: self.blocks_sound.unwrap_or(blocks_sight),
        }
    }

    /// Ingress validation beyond serde shape: a present `direction.blocking_side` must be
    /// exactly `1.0` or `-1.0`. The cross-product side test
    /// (`vision::Seg::transparent_from`) reads it as a sign, so any other value — `0.0`
    /// above all, which would author a wall solid from NEITHER side — must fail closed at
    /// ingress rather than be silently signum-normalized into an unintended
    /// full-transparency wall.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::engine::{Seg, WallDirection, WallEngine};
    ///
    /// let mut wall = WallEngine {
    ///     seg: Seg { x1: 0.0, y1: 0.0, x2: 10.0, y2: 0.0 },
    ///     blocks_sight: None,
    ///     blocks_light: None,
    ///     blocks_move: None,
    ///     blocks_sound: None,
    ///     elevation: None,
    ///     door: None,
    ///     direction: Some(WallDirection { blocking_side: 1.0 }),
    ///     sight_tier: None,
    /// };
    /// assert!(wall.validate().is_ok());
    /// wall.direction = Some(WallDirection { blocking_side: 0.0 });
    /// assert!(wall.validate().is_err());
    /// ```
    pub fn validate(&self) -> Result<(), String> {
        if let Some(d) = self.direction {
            if d.blocking_side != 1.0 && d.blocking_side != -1.0 {
                return Err("direction.blockingSide must be exactly 1.0 or -1.0".to_string());
            }
        }
        Ok(())
    }
}

/// A region's vector geometry. `points` layout by
/// kind: rect: `[x0,y0,x1,y1]`; circle: `[cx,cy,r]`; polygon:
/// `[x0,y0,x1,y1,...]` (>=3 vertices, even length).
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::RegionShape;
///
/// let circle = RegionShape { kind: "circle".to_string(), points: vec![5.0, 5.0, 2.0] };
/// assert_eq!(circle.points.len(), 3);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct RegionShape {
    /// "rect" | "circle" | "polygon"; a `String`, not a Rust enum.
    pub kind: String,
    /// Flat coordinate list in scene units; layout depends on `kind` (see the
    /// struct doc).
    pub points: Vec<f64>,
}

/// Upper bound on a compound region's authored shape-part count (`RegionGeometry::Compound`).
/// DoS-bounds how many shapes ONE region document can declare, independent of the per-part
/// raster-work caps (`regions::rasterize`'s `MAX_REGION_CELLS` AABB guard, applied per part).
pub const MAX_REGION_SHAPE_PARTS: usize = 32;

/// One part of a compound region's geometry: a vector shape plus whether it CUTS a hole out of
/// the region. `hole: false` (the default) contributes the shape additively; `hole: true`
/// subtracts its cells from the union of the list's additive parts at rasterization time
/// (`regions::rasterize_compound`).
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::{CompoundShapePart, RegionShape};
///
/// let part = CompoundShapePart {
///     shape: RegionShape { kind: "circle".to_string(), points: vec![5.0, 5.0, 2.0] },
///     hole: true,
/// };
/// let json = serde_json::to_value(&part).unwrap();
/// assert_eq!(json["hole"], true);
/// // `hole` defaults to `false` when absent from the wire form.
/// let bare: CompoundShapePart = serde_json::from_value(
///     serde_json::json!({ "shape": { "kind": "rect", "points": [0.0, 0.0, 1.0, 1.0] } }),
/// )
/// .unwrap();
/// assert!(!bare.hole);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct CompoundShapePart {
    /// The part's vector geometry.
    pub shape: RegionShape,
    /// `true` = subtract this shape's cells from the union of the additive parts.
    #[serde(default)]
    pub hole: bool,
}

/// A region document's geometry on the wire: EITHER the legacy singular `RegionShape` (every
/// document authored before compound regions) OR a list of parts carrying per-part `hole`
/// flags. Untagged so both forms share the one `shape` field with no document migration: an
/// object value parses as `Single`, an array value as `Compound`, and each re-serializes to its
/// own form, so a stored document's wire shape is preserved byte-for-byte through
/// `normalize_engine`'s round-trip. `regions::parse_region_shape` normalizes BOTH forms to the
/// same `(shape, hole)` part list the moment parsing completes, so every downstream consumer
/// (rasterization, the composed `RegionField`, the trigger identity rows) sees one shape — the
/// "ONE resolver" discipline applied to a data-shape migration rather than a flag.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::RegionGeometry;
///
/// // The legacy singular form parses unchanged (zero migration).
/// let legacy: RegionGeometry = serde_json::from_value(
///     serde_json::json!({ "kind": "rect", "points": [0.0, 0.0, 5.0, 5.0] }),
/// )
/// .unwrap();
/// assert!(matches!(legacy, RegionGeometry::Single(_)));
/// assert_eq!(
///     serde_json::to_value(&legacy).unwrap(),
///     serde_json::json!({ "kind": "rect", "points": [0.0, 0.0, 5.0, 5.0] })
/// );
///
/// // The compound form is an array of `{shape, hole}` parts.
/// let compound: RegionGeometry = serde_json::from_value(serde_json::json!([
///     { "shape": { "kind": "rect", "points": [0.0, 0.0, 5.0, 5.0] }, "hole": false },
///     { "shape": { "kind": "circle", "points": [2.0, 2.0, 1.0] }, "hole": true }
/// ]))
/// .unwrap();
/// assert!(matches!(compound, RegionGeometry::Compound(ref parts) if parts.len() == 2));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(untagged)]
pub enum RegionGeometry {
    /// Legacy: one solid shape, no holes.
    Single(RegionShape),
    /// Compound: the union of the additive parts minus every hole part.
    Compound(Vec<CompoundShapePart>),
}

impl<'de> Deserialize<'de> for RegionGeometry {
    // Hand-written (the `AdditionalProperties` precedent) rather than a derive: the JSON
    // SHAPE selects the variant — an object routes straight into `RegionShape` via
    // `MapAccessDeserializer`, an array into `Vec<CompoundShapePart>` — so each inner
    // struct's own `deny_unknown_fields` runs against real map access with its own error
    // message (an unknown key inside either form names the offending key instead of the
    // untagged derive's generic "did not match any variant" mismatch).
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct GeometryVisitor;
        impl<'de> serde::de::Visitor<'de> for GeometryVisitor {
            type Value = RegionGeometry;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a region shape object or a list of {shape, hole} parts")
            }
            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let shape =
                    RegionShape::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                Ok(RegionGeometry::Single(shape))
            }
            fn visit_seq<A>(self, seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let parts = Vec::<CompoundShapePart>::deserialize(
                    serde::de::value::SeqAccessDeserializer::new(seq),
                )?;
                Ok(RegionGeometry::Compound(parts))
            }
        }
        deserializer.deserialize_any(GeometryVisitor)
    }
}

/// Upper bound (chars) for a trigger's condition/resource id. Ids are
/// free-form strings naming registry entries; the bound exists because a
/// trigger is an engine-EXECUTED payload, so its fields are validated at
/// ingress rather than trusted read-side.
pub const MAX_TRIGGER_ID_CHARS: usize = 128;

/// The moment a region trigger fires.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::TriggerEvent;
///
/// assert_ne!(TriggerEvent::Enter, TriggerEvent::Arrest);
/// assert_ne!(TriggerEvent::Arrest, TriggerEvent::Exit);
/// // The four turn/round variants spell exactly like `combat::transition`'s
/// // own phase vocabulary (`phase_name`).
/// assert_eq!(
///     serde_json::to_value(TriggerEvent::TurnStart).unwrap(),
///     serde_json::json!("turn_start")
/// );
/// assert_eq!(
///     serde_json::to_value(TriggerEvent::TurnEnd).unwrap(),
///     serde_json::json!("turn_end")
/// );
/// assert_eq!(
///     serde_json::to_value(TriggerEvent::RoundStart).unwrap(),
///     serde_json::json!("round_start")
/// );
/// assert_eq!(
///     serde_json::to_value(TriggerEvent::RoundEnd).unwrap(),
///     serde_json::json!("round_end")
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "snake_case")]
pub enum TriggerEvent {
    /// The token's center cell (a move) or footprint cells (a placement)
    /// intersect the region.
    Enter,
    /// The walk was arrested while inside the region.
    Arrest,
    /// The token's post-event OCCUPANCY no longer intersects the region while
    /// its stored membership (`SceneEcs::region_membership`, the prior event's
    /// occupancy-derived set) still named it — the token left the region.
    /// "Occupancy" is the firing report's own Enter basis
    /// (`TriggerReport::occupancy`): a move's stop center cell, a placement's
    /// footprint.
    Exit,
    /// The combatant whose turn is starting has its token standing in the
    /// region (by `SceneEcs::region_membership`) when `combat::transition`
    /// reaches that combatant's `turn_start` boundary.
    TurnStart,
    /// The combatant whose turn is ending has its token standing in the
    /// region when `combat::transition` reaches its `turn_end` boundary.
    TurnEnd,
    /// Any combatant of the combat has its token standing in the region when
    /// `combat::transition` crosses a `round_start` boundary (once per round
    /// boundary per member token, matching `round_wrap`'s structure).
    RoundStart,
    /// Any combatant of the combat has its token standing in the region when
    /// `combat::transition` crosses a `round_end` boundary.
    RoundEnd,
}

/// Who a trigger's chat notice may reach. `Owner` means the token's
/// effective owner plus every GM.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::NoticeAudience;
///
/// let audience = NoticeAudience::Owner;
/// assert_eq!(audience, NoticeAudience::Owner);
/// assert_ne!(audience, NoticeAudience::Public);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "snake_case")]
pub enum NoticeAudience {
    /// Every world member.
    Public,
    /// GMs only; forced onto every notice when the region itself is not
    /// visible to all (a public side-channel would leak a secret region).
    GmOnly,
    /// The token's effective owner, plus every GM.
    Owner,
}

/// The effect a fired trigger applies to the entering token. Internally
/// tagged, so `deny_unknown_fields` is unavailable (the
/// `CombatantKind`/`ResourceBinding` precedent); `normalize_engine`'s
/// re-serialization still drops smuggled keys.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::TriggerEffect;
///
/// let effect = TriggerEffect::ConditionAdd { condition: "prone".to_string() };
/// let json = serde_json::to_value(&effect).unwrap();
/// // Internally tagged on `type`, snake_case: the variant name is the discriminant.
/// assert_eq!(json["type"], "condition_add");
/// assert_eq!(json["condition"], "prone");
/// assert_eq!(serde_json::from_value::<TriggerEffect>(json).unwrap(), effect);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerEffect {
    /// Add a condition id to the token's actor host (no-op when present).
    ConditionAdd {
        /// The condition id (non-empty, `MAX_TRIGGER_ID_CHARS`-bounded).
        condition: String,
    },
    /// Remove a condition id from the token's actor host (no-op when absent).
    ConditionRemove {
        /// The condition id (non-empty, `MAX_TRIGGER_ID_CHARS`-bounded).
        condition: String,
    },
    /// Adjust a tracked resource of the token's combatant in the scene's
    /// active combat. No active combat, no combatant, a `Mirror` binding, or
    /// an amount that fails to evaluate is a no-op surfaced as a GM-only
    /// notice.
    ResourceDelta {
        /// The resource-registry key (non-empty, `MAX_TRIGGER_ID_CHARS`-bounded).
        resource: String,
        /// The signed amount, evaluated against the token's actor host.
        amount: crate::data::engine::combat::Formula,
    },
    /// Post a chat notice.
    ChatNotice {
        /// Notice body (`chat::MAX_MESSAGE_CHARS`-bounded).
        text: String,
        /// Intended readership; forced to `GmOnly` for a region not visible
        /// to all.
        audience: NoticeAudience,
    },
    /// Move the entering token to `target` — within the same scene
    /// (`target.scene: None`) or to another scene entirely. Fires wherever its
    /// trigger's `on` event matches (conventionally `TriggerEvent::Enter`; an
    /// `Exit`-authored teleport snaps a leaving token back); see
    /// `ws::room::Room::fire_region_triggers`
    /// for application (same-scene Update vs cross-scene Move+Update) and the
    /// one-hop anti-loop (a destination's OWN `Enter` effects fire except
    /// another `Teleport`).
    Teleport {
        /// Where to send the token.
        target: PortalTarget,
    },
    /// Play a floating overlay above the triggering token — reuses
    /// `EmoteView`'s existing floating-glyph mechanism (a server-authored
    /// emote) rather than a second floating-text renderer. Text is bounded by
    /// `chat::MAX_MESSAGE_CHARS` at ingress (one constant, one precedent — the
    /// SAME bound `ChatNotice` uses; a tighter server-side cap would silently
    /// narrow what a GM may author, an authoring-policy call this ingress
    /// validator does not make). The CLIENT overlay display-truncates a long
    /// text regardless — see `EmoteView`'s own doc for the display-side limit
    /// this variant's own render treatment enforces. Secrecy is forced to
    /// GM-only when the firing region's `/engine` band is not visible to
    /// every world member (`ws::room::Room::apply_region_trigger_effects`),
    /// mirroring `ChatNotice`'s own secrecy forcing.
    ScrollingText {
        /// Overlay text (`chat::MAX_MESSAGE_CHARS`-bounded at ingress; the client
        /// display-truncates further — see `EmoteView`'s doc).
        text: String,
    },
}

/// A teleport's destination. `scene: None` = the same scene the portal fired
/// in.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::PortalTarget;
///
/// let target = PortalTarget { scene: None, x: 10.0, y: 10.0, elevation: None, vfx: None };
/// assert!(target.scene.is_none());
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct PortalTarget {
    /// Destination scene id; `None` = the portal's own scene.
    #[serde(default)]
    pub scene: Option<uuid::Uuid>,
    /// Destination x, destination scene units.
    pub x: f64,
    /// Destination y, destination scene units.
    pub y: f64,
    /// Destination elevation; `None` leaves the token's current elevation
    /// unchanged.
    #[serde(default)]
    pub elevation: Option<f64>,
    /// Asset id of a VFX played at BOTH ends on teleport (source +
    /// destination); validated and carried here; PLAYED once `ws::room`
    /// broadcasts it through `ServerMsg::Vfx` (a validated id with no
    /// broadcaster is stored, never dropped).
    #[serde(default)]
    pub vfx: Option<String>,
}

impl PortalTarget {
    /// Ingress validation: `x`/`y` finite and `MAX_GATE_WALK_COORD`-bounded
    /// (the same bound the movement gate and `TokenEngine::validate` enforce —
    /// a teleport must never place a token past the coordinate ceiling every
    /// other write path already refuses), `elevation` finite when present,
    /// `vfx` a valid asset ref when present.
    pub(crate) fn validate(&self) -> Result<(), String> {
        let bound = crate::scene::move_exec::MAX_GATE_WALK_COORD;
        for (name, v) in [("x", self.x), ("y", self.y)] {
            if !v.is_finite() {
                return Err(format!("{name} must be finite"));
            }
        }
        if self.x.abs() > bound || self.y.abs() > bound {
            return Err(format!("teleport target exceeds coordinate bound {bound}"));
        }
        if let Some(e) = self.elevation {
            if !e.is_finite() {
                return Err("elevation must be finite".to_string());
            }
        }
        if let Some(vfx) = &self.vfx {
            validate_asset_ref(vfx).map_err(|m| format!("vfx: {m}"))?;
        }
        Ok(())
    }
}

/// One region trigger: when `on` occurs for a token inside the region,
/// apply `effect`.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::{RegionTrigger, TriggerEffect, TriggerEvent};
///
/// let trigger = RegionTrigger {
///     on: TriggerEvent::Enter,
///     effect: TriggerEffect::ConditionAdd { condition: "prone".to_string() },
/// };
/// assert_eq!(trigger.on, TriggerEvent::Enter);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct RegionTrigger {
    /// The firing moment.
    pub on: TriggerEvent,
    /// The effect to apply.
    pub effect: TriggerEffect,
}

/// How an `ambient_light` region edits the composed illumination of the cells
/// it covers (`RegionEngine::ambient_override`). Applied as the FINAL per-cell
/// adjustment over `LightingInputs::cell_light`'s base composition — placed
/// lights, carried lights and the boundary-projected environment all compose
/// first, then this edit — including under `LightMode::GlobalIllumination`,
/// whose flat all-bright baseline is itself edited (the `all_bright`
/// short-circuit skips gathering placed lights and light walls, never this
/// adjustment).
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::AmbientLightMode;
///
/// let json = serde_json::to_value(AmbientLightMode::Darken).unwrap();
/// assert_eq!(json, serde_json::json!("darken"));
/// assert_eq!(
///     serde_json::from_value::<AmbientLightMode>(json).unwrap(),
///     AmbientLightMode::Darken
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "snake_case")]
pub enum AmbientLightMode {
    /// Subtract `level` from the cell's composed illumination (the result
    /// never drops below 0).
    Darken,
    /// Add `level` to the cell's composed illumination (saturating at 1, the
    /// same ceiling the additive-superposition composition applies).
    Brighten,
    /// Replace the cell's composed illumination with `level` outright,
    /// regardless of every other contributor (placed lights, environment,
    /// other regions). The strongest form: any overlapping `Override` wins
    /// over `Darken`/`Brighten` contributions, and two overlapping `Override`s
    /// resolve to the DARKER level (the fail-closed under-reveal convention).
    Override,
}

/// An `ambient_light` region's authored light edit: `mode` selects how `level`
/// applies (`AmbientLightMode`); `level` is an absolute illumination for
/// `Override` (validated to `[0, 1]`, the illumination scale's own range) or a
/// non-negative delta for `Darken`/`Brighten`. Meaningful only when
/// `RegionEngine::behavior` is `"ambient_light"` — `RegionEngine::validate`
/// enforces that pairing.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::{AmbientLightMode, AmbientLightOverride};
///
/// let zone = AmbientLightOverride { mode: AmbientLightMode::Darken, level: 0.5 };
/// let json = serde_json::to_value(zone).unwrap();
/// assert_eq!(json, serde_json::json!({ "mode": "darken", "level": 0.5 }));
/// assert_eq!(serde_json::from_value::<AmbientLightOverride>(json).unwrap(), zone);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct AmbientLightOverride {
    /// How `level` applies to a covered cell's composed illumination.
    pub mode: AmbientLightMode,
    /// The absolute illumination (`Override`, `[0, 1]`) or non-negative delta
    /// (`Darken`/`Brighten`). Must be finite.
    pub level: f64,
}

/// A region document's engine body: a vector-shaped zone that weights,
/// blocks, or arrests grid movement, or edits its cells' ambient light, and
/// optionally fires triggers on entering tokens. Client mirror: `RegionEngine` (`@shadowcat/core`).
/// `cost` is a multiplier (>=1, clamped read-side) meaningful only for
/// `behavior:"terrain"`. `enabled` lets a GM toggle a region off without
/// deleting it (disabled regions are dropped entirely at read time).
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::{RegionEngine, RegionGeometry, RegionShape};
///
/// let region = RegionEngine {
///     shape: RegionGeometry::Single(RegionShape {
///         kind: "rect".to_string(),
///         points: vec![0.0, 0.0, 5.0, 5.0],
///     }),
///     behavior: "terrain".to_string(),
///     cost: 2.0,
///     cost_by_tag: std::collections::BTreeMap::new(),
///     enabled: true,
///     triggers: Vec::new(),
///     elevation: None,
///     ambient_override: None,
/// };
/// assert!(region.validate().is_ok());
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct RegionEngine {
    /// The zone's geometry, scene units: a single shape (the legacy wire form,
    /// still emitted by the default authoring gesture) or a compound list of
    /// parts with per-part hole flags — see `RegionGeometry`.
    pub shape: RegionGeometry,
    /// "terrain" | "impassable" | "arrest" | "ambient_light". Ingress does not check the
    /// spelling; `RegionEngine::validate` only requires `"ambient_light"` when
    /// `ambient_override` is present. `SceneEcs::region_field_if` reads `"impassable"` and
    /// `"arrest"`; it reads `"ambient_light"` only with a finite `ambient_override` (otherwise
    /// the region contributes nothing); every other spelling is read as terrain.
    pub behavior: String,
    /// Movement-cost multiplier (>= 1, clamped read-side); meaningful only
    /// for `behavior: "terrain"`.
    pub cost: f64,
    /// Per-movement-tag cost overrides (>= 1, clamped read-side exactly like `cost`);
    /// meaningful only for `behavior: "terrain"`. A mover carrying one or more matching
    /// tags (`scene::regions::RegionField::resolved_terrain_multiplier`) pays the MINIMUM
    /// matching value rather than `cost` — "you navigate however is easiest for you". `cost`
    /// is the fallback for a mover whose tags match no key here. An absent key deserializes
    /// to an empty map (serde default).
    #[serde(default)]
    pub cost_by_tag: std::collections::BTreeMap<String, f64>,
    /// GM toggle; a disabled region is dropped entirely at read time.
    pub enabled: bool,
    /// Effects fired on tokens entering, arrested inside, or leaving the
    /// region, or at a combat turn/round boundary while a combatant's token
    /// stands in the region. Absent on documents written before triggers
    /// existed (serde default).
    #[serde(default)]
    pub triggers: Vec<RegionTrigger>,
    /// The elevation band this region's geometry occupies; absent = every
    /// level (pre-levels authoring, or a level-less scene). Read by
    /// `scene::elevation::band_contains` — the SAME predicate
    /// `WallEngine::elevation`'s occlusion test and the movement gate consult.
    #[serde(default)]
    pub elevation: Option<ElevationBand>,
    /// The ambient-light edit this region applies to its cells' composed
    /// illumination (`scene::regions::AmbientAdjustment`, read through
    /// `SceneEcs::ambient_light_map`, which passes no viewer so a `gm_only`
    /// region's value still applies); meaningful only for
    /// `behavior: "ambient_light"` — the
    /// behavior-conditional-field precedent `cost` sets for `"terrain"`.
    /// Absent on documents written before ambient-light regions existed
    /// (serde default).
    #[serde(default)]
    pub ambient_override: Option<AmbientLightOverride>,
}

impl RegionEngine {
    /// Ingress validation: the shape geometry must be structurally rasterizable
    /// (`regions::parse_region_shape`'s own kind/point-count dispatch — the ONE structural
    /// shape check, shared so ingress and the read-side can never drift), a compound list
    /// must declare at least one part and no more than `MAX_REGION_SHAPE_PARTS`, and the
    /// trigger payloads — the one engine-EXECUTED part of this body (the movement fields
    /// keep their read-side fail-closed semantics and are not re-validated here) — must
    /// satisfy the per-effect rules. Ids must be non-empty and
    /// `MAX_TRIGGER_ID_CHARS`-bounded, `amount` must satisfy `Formula::validate` (finite
    /// literal or parseable formula source), and notice text is bounded by
    /// `chat::MAX_MESSAGE_CHARS`. An `ambient_override` is engine-executed too (it edits the
    /// lighting field): it requires `behavior == "ambient_light"`, a finite `level`, and the
    /// mode's own range — `[0, 1]` for `Override`, non-negative for `Darken`/`Brighten`.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::engine::{
    ///     RegionEngine, RegionGeometry, RegionShape, RegionTrigger, TriggerEffect, TriggerEvent,
    /// };
    ///
    /// let region = RegionEngine {
    ///     shape: RegionGeometry::Single(RegionShape {
    ///         kind: "rect".to_string(),
    ///         points: vec![0.0, 0.0, 5.0, 5.0],
    ///     }),
    ///     behavior: "terrain".to_string(),
    ///     cost: 1.0,
    ///     cost_by_tag: std::collections::BTreeMap::new(),
    ///     enabled: true,
    ///     triggers: vec![RegionTrigger {
    ///         on: TriggerEvent::Enter,
    ///         effect: TriggerEffect::ConditionAdd { condition: String::new() },
    ///     }],
    ///     elevation: None,
    ///     ambient_override: None,
    /// };
    /// assert!(region.validate().is_err()); // empty condition id
    /// ```
    pub fn validate(&self) -> Result<(), String> {
        if let RegionGeometry::Compound(parts) = &self.shape {
            if parts.is_empty() {
                return Err("compound region declares no shape parts".to_string());
            }
            if parts.len() > MAX_REGION_SHAPE_PARTS {
                return Err(format!(
                    "compound region exceeds {MAX_REGION_SHAPE_PARTS} shape parts"
                ));
            }
        }
        if crate::scene::regions::parse_region_shape(&self.shape).is_none() {
            return Err(
                "shape has an unrecognized kind or a point count that does not match it"
                    .to_string(),
            );
        }
        if let Some(ambient) = &self.ambient_override {
            if self.behavior != "ambient_light" {
                return Err("ambient_override requires behavior \"ambient_light\"".to_string());
            }
            if !ambient.level.is_finite() {
                return Err("ambient_override.level must be finite".to_string());
            }
            match ambient.mode {
                AmbientLightMode::Override if !(0.0..=1.0).contains(&ambient.level) => {
                    return Err(
                        "ambient_override.level for mode \"override\" must be within [0, 1]"
                            .to_string(),
                    );
                }
                AmbientLightMode::Darken | AmbientLightMode::Brighten if ambient.level < 0.0 => {
                    return Err(
                        "ambient_override.level for mode \"darken\"/\"brighten\" must be >= 0"
                            .to_string(),
                    );
                }
                _ => {}
            }
        }
        for trigger in &self.triggers {
            match &trigger.effect {
                TriggerEffect::ConditionAdd { condition }
                | TriggerEffect::ConditionRemove { condition } => {
                    validate_trigger_id(condition, "condition")?;
                }
                TriggerEffect::ResourceDelta { resource, amount } => {
                    validate_trigger_id(resource, "resource")?;
                    amount.validate("resource_delta amount")?;
                }
                TriggerEffect::ChatNotice { text, .. } => {
                    if text.chars().count() > crate::chat::MAX_MESSAGE_CHARS {
                        return Err(format!(
                            "chat_notice text exceeds {} chars",
                            crate::chat::MAX_MESSAGE_CHARS
                        ));
                    }
                }
                TriggerEffect::Teleport { target } => {
                    target.validate()?;
                }
                TriggerEffect::ScrollingText { text } => {
                    if text.chars().count() > crate::chat::MAX_MESSAGE_CHARS {
                        return Err(format!(
                            "scrolling_text text exceeds {} chars",
                            crate::chat::MAX_MESSAGE_CHARS
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

/// One trigger id (condition/resource): non-empty and char-bounded.
fn validate_trigger_id(id: &str, what: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err(format!("{what} id must be non-empty"));
    }
    if id.chars().count() > MAX_TRIGGER_ID_CHARS {
        return Err(format!("{what} id exceeds {MAX_TRIGGER_ID_CHARS} chars"));
    }
    Ok(())
}

/// `points` layout mirrors `RegionShape` (path vertices for freehand/line/
/// polygon, or bbox corners `[x0,y0,x1,y1]` for rect/ellipse).
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::DrawingShape;
///
/// let shape = DrawingShape { kind: "rect".to_string(), points: vec![0.0, 0.0, 4.0, 4.0] };
/// assert_eq!(shape.kind, "rect");
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct DrawingShape {
    /// "freehand" | "line" | "polygon" | "rect" | "ellipse" (render-layer vocabulary). The
    /// server stores any string and does not read it.
    pub kind: String,
    /// Flat coordinate list in scene units; layout depends on `kind` (see the
    /// struct doc).
    pub points: Vec<f64>,
}

/// The one shared colour-field shape check across every engine doc type: exactly `#` + 6 hex
/// digits (a css `#rrggbb` string, the only syntax `<input type="color">` ever emits — every
/// client color-authoring path in the tree uses that input, never a hand-typed field, so this
/// shape is safe to enforce at ingress). Replaces the two former per-doc-type copies
/// (`token::validate_emission_color`/`registries::validate_fx_color`) that duplicated this exact
/// check — a colour field anywhere in the `data::engine` tree calls this one function.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::geometry::validate_hex_color;
///
/// assert!(validate_hex_color("#ff00aa").is_ok());
/// assert!(validate_hex_color("ff00aa").is_err()); // missing '#'
/// assert!(validate_hex_color("#ff00a").is_err()); // 5 digits
/// assert!(validate_hex_color("#gg0000").is_err()); // non-hex digit
/// ```
pub fn validate_hex_color(color: &str) -> Result<(), String> {
    match color.strip_prefix('#') {
        Some(hex) if hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()) => Ok(()),
        _ => Err("color must be a css #rrggbb string".to_string()),
    }
}

/// The one shared asset-reference shape check across every engine doc type: a valid UUID string
/// (`data::asset::Asset.id: Uuid` — every asset ref in the tree names a row by that same id).
/// FORMAT ONLY: this never queries the repository, so it says nothing about whether the id
/// actually resolves to a stored asset — existence is a read-time concern (a dangling ref reads
/// as "no asset" at every consumer), not an ingress structural one.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::geometry::validate_asset_ref;
///
/// assert!(validate_asset_ref("3fa85f64-5717-4562-b3fc-2c963f66afa6").is_ok());
/// assert!(validate_asset_ref("").is_err());
/// assert!(validate_asset_ref("not-a-uuid").is_err());
/// ```
pub fn validate_asset_ref(asset: &str) -> Result<(), String> {
    uuid::Uuid::parse_str(asset)
        .map(|_| ())
        .map_err(|_| "asset ref must be a uuid".to_string())
}

/// A drawing's outline style.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::Stroke;
///
/// let stroke = Stroke { color: "#ff0000".to_string(), width: 2.0 };
/// assert_eq!(stroke.width, 2.0);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct Stroke {
    /// `#rrggbb` stroke color.
    pub color: String,
    /// Stroke width, scene units.
    pub width: f64,
}

impl Stroke {
    /// Ingress validation beyond serde shape: css-`#rrggbb` color and a finite width.
    pub(crate) fn validate(&self) -> Result<(), String> {
        validate_hex_color(&self.color)?;
        if !self.width.is_finite() {
            return Err("stroke width must be finite".to_string());
        }
        Ok(())
    }
}

/// A drawing's fill style.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::Fill;
///
/// let fill = Fill { color: "#00ff00".to_string(), alpha: Some(0.5) };
/// assert_eq!(fill.alpha, Some(0.5));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct Fill {
    /// `#rrggbb` fill color.
    pub color: String,
    /// Fill opacity 0..=1; absent = opaque.
    #[serde(default)]
    pub alpha: Option<f64>,
}

impl Fill {
    /// Ingress validation beyond serde shape: css-`#rrggbb` color and, when present, a finite
    /// `alpha` (the presentation range `0..=1` is clamped read-side where consumed, matching
    /// every other emission-scalar convention in this tree).
    pub(crate) fn validate(&self) -> Result<(), String> {
        validate_hex_color(&self.color)?;
        if let Some(a) = self.alpha {
            if !a.is_finite() {
                return Err("fill alpha must be finite".to_string());
            }
        }
        Ok(())
    }
}

/// A drawing document's engine body. Client mirror: `DrawingEngine` (`@shadowcat/core`).
/// `stroke`/`fill` are each a required-but-nullable field on the wire
/// (`{...} | null`, not optional) — `Option<T>` without a serde default
/// mirrors that exactly (the key must be present, either an object or
/// `null`).
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::{DrawingEngine, DrawingShape};
///
/// let drawing = DrawingEngine {
///     shape: DrawingShape { kind: "rect".to_string(), points: vec![0.0, 0.0, 4.0, 4.0] },
///     stroke: None,
///     fill: None,
///     elevation: None,
/// };
/// assert!(drawing.stroke.is_none());
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct DrawingEngine {
    /// The drawing's geometry, scene units.
    pub shape: DrawingShape,
    /// Outline style; wire-required but nullable (`Stroke | null`).
    pub stroke: Option<Stroke>,
    /// Fill style; wire-required but nullable (`Fill | null`).
    pub fill: Option<Fill>,
    /// The elevation band this drawing's geometry occupies; absent = every
    /// level (pre-levels authoring, or a level-less scene). Read by
    /// `scene::elevation::band_contains` — the SAME predicate
    /// `WallEngine::elevation`'s occlusion test and the movement gate consult.
    #[serde(default)]
    pub elevation: Option<ElevationBand>,
}

impl DrawingEngine {
    /// Ingress validation beyond serde shape: `stroke`/`fill`, when present, validate through
    /// their own `Stroke`/`Fill::validate`.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if let Some(stroke) = &self.stroke {
            stroke.validate().map_err(|m| format!("stroke: {m}"))?;
        }
        if let Some(fill) = &self.fill {
            fill.validate().map_err(|m| format!("fill: {m}"))?;
        }
        Ok(())
    }
}

/// A template's area anchored at `(x,y)` with a `size` and `direction`
/// (degrees), tessellated per `kind`. Client mirror: `TemplateEngine["shape"]`
/// (`@shadowcat/core`) — the shape lives one level inside the engine body.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::TemplateShape;
///
/// let shape = TemplateShape { kind: "cone".to_string(), x: 0.0, y: 0.0, size: 15.0, direction: 90.0, angle: None, width: None };
/// assert_eq!(shape.size, 15.0);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct TemplateShape {
    /// "circle" | "cone" | "rect" | "line" (`TemplateView.toSpec`'s tessellation
    /// vocabulary). The server stores any string and does not read it.
    pub kind: String,
    /// Anchor x, scene units.
    pub x: f64,
    /// Anchor y, scene units.
    pub y: f64,
    /// The scale of this field is per-`kind` (`width`'s doc details the full
    /// convention): `"circle"` — radius; `"cone"` — the apex-to-base-corner
    /// distance; `"rect"` — HALF the first-axis side (full side `2*size`);
    /// `"line"` — the full segment length.
    pub size: f64,
    /// Orientation in degrees; the render layer converts via standard radian
    /// math (`TemplateView.toSpec`).
    pub direction: f64,
    /// The cone's full aperture — the angle BETWEEN the two base corners
    /// (`TemplateView.toSpec`'s `conePoints` halves it to sweep `direction ±
    /// angle/2`), degrees, `(0.0, 180.0)` EXCLUSIVE on both ends: `conePoints`
    /// returns a three-point wedge (apex + two base corners), which is
    /// collinear (zero area) at exactly 180° and opens AWAY from `direction`
    /// past 180° — neither is a representable cone, so both are refused at
    /// ingress rather than silently rendering nothing or backwards. Absent
    /// reproduces the render layer's default 60° cone
    /// (`DEFAULT_CONE_ANGLE_DEGREES` client-side). Unused for
    /// `"circle"`/`"rect"`/`"line"`.
    #[serde(default)]
    pub angle: Option<f64>,
    /// The shape's second/perpendicular dimension, scaled by `kind`: `"rect"` — HALF the
    /// second-axis side (full side `2*width`; the client reads an absent `width` as `size`,
    /// a square of side `2*size`); `"line"` — the FULL perpendicular thickness (absent or
    /// `0.0` is a zero-width segment). Unused for `"circle"`/`"cone"`. `>= 0.0` when present.
    #[serde(default)]
    pub width: Option<f64>,
}

/// A template document's engine body: a measured-area overlay.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::{TemplateEngine, TemplateShape};
///
/// let template = TemplateEngine {
///     shape: TemplateShape { kind: "circle".to_string(), x: 0.0, y: 0.0, size: 10.0, direction: 0.0, angle: None, width: None },
///     color: "#ffaa00".to_string(),
///     elevation: None,
/// };
/// assert_eq!(template.color, "#ffaa00");
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct TemplateEngine {
    /// The template's area, scene units.
    pub shape: TemplateShape,
    /// `#rrggbb` overlay color.
    pub color: String,
    /// The elevation band this template's geometry occupies; absent = every
    /// level (pre-levels authoring, or a level-less scene). Read by
    /// `scene::elevation::band_contains` — the SAME predicate
    /// `WallEngine::elevation`'s occlusion test and the movement gate consult.
    #[serde(default)]
    pub elevation: Option<ElevationBand>,
}

impl TemplateEngine {
    /// Ingress validation beyond serde shape: css-`#rrggbb` `color`, and the
    /// shape's `angle`/`width` bounds when present. `angle`'s upper bound is
    /// exclusive at `180.0` (not `360.0`): `conePoints`' three-point wedge is
    /// collinear at exactly 180° and opens away from `direction` beyond it —
    /// see `TemplateShape::angle`'s own doc for the full geometric argument.
    pub(crate) fn validate(&self) -> Result<(), String> {
        validate_hex_color(&self.color)?;
        if let Some(angle) = self.shape.angle {
            if !angle.is_finite() || angle <= 0.0 || angle >= 180.0 {
                return Err("shape.angle: must be finite and in (0.0, 180.0)".to_string());
            }
        }
        if let Some(width) = self.shape.width {
            if !width.is_finite() || width < 0.0 {
                return Err("shape.width: must be finite and >= 0.0".to_string());
            }
        }
        Ok(())
    }
}
