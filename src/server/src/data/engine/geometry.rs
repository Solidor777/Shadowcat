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

/// The elevation interval a wall's occlusion applies to. A sight/light source at
/// elevation `e` is occluded by the wall iff `bottom ≤ e ≤ top`; an absent end is
/// unbounded (`bottom: None` = −∞, `top: None` = +∞). A wall whose `elevation`
/// field is absent occludes every elevation, and a malformed interval
/// (`bottom > top`, or a non-finite endpoint) fails closed to occluding
/// everything — see `scene::elevation::wall_occludes`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct WallElevation {
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
    /// absent = occludes every elevation. Never consulted by the movement
    /// gate (movement is ground-plane).
    #[serde(default)]
    pub elevation: Option<WallElevation>,
}

/// A region's vector geometry. `points` layout by
/// kind: rect: `[x0,y0,x1,y1]`; circle: `[cx,cy,r]`; polygon:
/// `[x0,y0,x1,y1,...]` (>=3 vertices, even length).
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
}

/// One region trigger: when `on` occurs for a token inside the region,
/// apply `effect`.
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
}

impl RegionEngine {
    /// Ingress validation for the trigger payloads — the one engine-EXECUTED
    /// part of this body (the movement fields keep their read-side
    /// fail-closed semantics and are not re-validated here). Ids must be
    /// non-empty and `MAX_TRIGGER_ID_CHARS`-bounded, `amount` must satisfy
    /// `Formula::validate` (finite literal or parseable formula source), and
    /// notice text is bounded by `chat::MAX_MESSAGE_CHARS`.
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
}

/// A template's area anchored at `(x,y)` with a `size` and `direction`
/// (degrees), tessellated per `kind`. Client mirror: `TemplateEngine["shape"]`
/// (`@shadowcat/core`) — the shape lives one level inside the engine body.
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct TemplateEngine {
    /// The template's area, scene units.
    pub shape: TemplateShape,
    /// `#rrggbb` overlay color.
    pub color: String,
}
