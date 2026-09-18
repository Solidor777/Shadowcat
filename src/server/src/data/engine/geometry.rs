//! `wall`, `region`, `drawing`, `template` engine bands. The
//! wall/region shapes match what `SceneEcs::engine_as_cached::<WallEngine>`/
//! `<RegionEngine>` read (the client has no separately-declared wall/region
//! type to mirror); the drawing/template shapes mirror the client's
//! re-exported `DrawingEngine`/`TemplateEngine` — the only authoritative
//! shapes today; scene-tools writers must round-trip byte-identically
//! against these.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two crate-level deny attributes this module declares.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde::{Deserialize, Serialize};
use ts_rs::TS;

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

/// A wall's segment + sight/light/movement-blocking flags. Absent/false
/// flags exclude the wall from that gate, matching how each gate
/// (`move_exec`/`pathfinding`/`lighting`) already reads these fields.
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
///     elevation: None,
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
    /// The elevation band this wall's sight/light occlusion applies to;
    /// absent = occludes every elevation. Consulted by the movement gate exactly
    /// as it is by sight/light occlusion — see `scene::elevation::band_contains`
    /// and `SceneEcs::move_wall_entries`.
    #[serde(default)]
    pub elevation: Option<ElevationBand>,
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
    /// "rect" | "circle" | "polygon" — kept a `String` in v1 (asserted by
    /// the unit battery).
    pub kind: String,
    /// Flat coordinate list in scene units; layout depends on `kind` (see the
    /// struct doc).
    pub points: Vec<f64>,
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
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "snake_case")]
pub enum TriggerEvent {
    /// The token's center cell (a move) or footprint cells (a placement)
    /// intersect the region.
    Enter,
    /// The walk was arrested while inside the region.
    Arrest,
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
    /// (`target.scene: None`) or to another scene entirely. Fires from
    /// `TriggerEvent::Enter` only; see `ws::room::Room::fire_region_triggers`
    /// for application (same-scene Update vs cross-scene Move+Update) and the
    /// one-hop anti-loop (a destination's OWN `Enter` effects fire except
    /// another `Teleport`).
    Teleport {
        /// Where to send the token.
        target: PortalTarget,
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
    /// `vfx` non-empty when present.
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
        if self.vfx.as_deref() == Some("") {
            return Err("vfx must be non-empty when present".to_string());
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

/// A region document's engine body: a vector-shaped zone that weights,
/// blocks, or arrests grid movement, and optionally fires triggers on
/// entering tokens. Client mirror: `RegionEngine` (`@shadowcat/core`).
/// `cost` is a multiplier (>=1, clamped read-side) meaningful only for
/// `behavior:"terrain"`. `enabled` lets a GM toggle a region off without
/// deleting it (disabled regions are dropped entirely at read time).
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::{RegionEngine, RegionShape};
///
/// let region = RegionEngine {
///     shape: RegionShape { kind: "rect".to_string(), points: vec![0.0, 0.0, 5.0, 5.0] },
///     behavior: "terrain".to_string(),
///     cost: 2.0,
///     enabled: true,
///     triggers: Vec::new(),
///     elevation: None,
/// };
/// assert!(region.validate().is_ok());
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct RegionEngine {
    /// The zone's vector geometry, scene units.
    pub shape: RegionShape,
    /// "terrain" | "impassable" | "arrest" — kept a `String` in v1 (asserted
    /// by the battery).
    pub behavior: String,
    /// Movement-cost multiplier (>= 1, clamped read-side); meaningful only
    /// for `behavior: "terrain"`.
    pub cost: f64,
    /// GM toggle; a disabled region is dropped entirely at read time.
    pub enabled: bool,
    /// Effects fired on tokens entering (or arrested inside) the region.
    /// Absent on documents written before triggers existed (serde default).
    #[serde(default)]
    pub triggers: Vec<RegionTrigger>,
    /// The elevation band this region's geometry occupies; absent = every
    /// level (pre-levels authoring, or a level-less scene). Read by
    /// `scene::elevation::band_contains` — the SAME predicate
    /// `WallEngine::elevation`'s occlusion test and the movement gate consult.
    #[serde(default)]
    pub elevation: Option<ElevationBand>,
}

impl RegionEngine {
    /// Ingress validation for the trigger payloads — the one engine-EXECUTED
    /// part of this body (the movement fields keep their read-side
    /// fail-closed semantics and are not re-validated here). Ids must be
    /// non-empty and `MAX_TRIGGER_ID_CHARS`-bounded, `amount` must satisfy
    /// `Formula::validate` (finite literal or parseable formula source), and
    /// notice text is bounded by `chat::MAX_MESSAGE_CHARS`.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::engine::{
    ///     RegionEngine, RegionShape, RegionTrigger, TriggerEffect, TriggerEvent,
    /// };
    ///
    /// let region = RegionEngine {
    ///     shape: RegionShape { kind: "rect".to_string(), points: vec![0.0, 0.0, 5.0, 5.0] },
    ///     behavior: "terrain".to_string(),
    ///     cost: 1.0,
    ///     enabled: true,
    ///     triggers: vec![RegionTrigger {
    ///         on: TriggerEvent::Enter,
    ///         effect: TriggerEffect::ConditionAdd { condition: String::new() },
    ///     }],
    ///     elevation: None,
    /// };
    /// assert!(region.validate().is_err()); // empty condition id
    /// ```
    pub fn validate(&self) -> Result<(), String> {
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
    /// "freehand" | "line" | "polygon" | "rect" | "ellipse" (render-layer
    /// vocabulary; kept a `String` in v1).
    pub kind: String,
    /// Flat coordinate list in scene units; layout depends on `kind` (see the
    /// struct doc).
    pub points: Vec<f64>,
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

/// A template's area anchored at `(x,y)` with a `size` and `direction`
/// (degrees), tessellated per `kind`. Client mirror: `TemplateEngine["shape"]`
/// (`@shadowcat/core`) — the shape lives one level inside the engine body.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::TemplateShape;
///
/// let shape = TemplateShape { kind: "cone".to_string(), x: 0.0, y: 0.0, size: 15.0, direction: 90.0 };
/// assert_eq!(shape.size, 15.0);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct TemplateShape {
    /// "circle" | "cone" | "rect" | "line" (`TemplateView.toSpec`'s tessellation
    /// vocabulary; kept a `String` in v1).
    pub kind: String,
    /// Anchor x, scene units.
    pub x: f64,
    /// Anchor y, scene units.
    pub y: f64,
    /// Radius/length, scene units.
    pub size: f64,
    /// Orientation in degrees; the render layer converts via standard radian
    /// math (`TemplateView.toSpec`).
    pub direction: f64,
}

/// A template document's engine body: a measured-area overlay.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::{TemplateEngine, TemplateShape};
///
/// let template = TemplateEngine {
///     shape: TemplateShape { kind: "circle".to_string(), x: 0.0, y: 0.0, size: 10.0, direction: 0.0 },
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
