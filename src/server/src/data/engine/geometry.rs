//! `wall`, `region`, `drawing`, `template` engine bands (M13-0 S1/S3). Field
//! shapes transcribed verbatim from the server's existing pointer-walk
//! consumers (wall/region: scene-docs.ts has no dedicated wall type — the
//! shape below mirrors `scene/mod.rs`'s current reads) and the render
//! layer's local shapes (`drawing-view.ts:9-13`, `template-view.ts:9-11`) —
//! the only authoritative shapes today; scene-tools writers must round-trip
//! byte-identically against these.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
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

/// A wall's segment + sight/light/movement-blocking flags. Absent/false
/// flags exclude the wall from that gate exactly as the pre-M13-0 pointer
/// read did (read-side backstop unchanged).
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
}

/// A region's vector geometry (M8d-3a shape vocabulary). `points` layout by
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

/// A region document's engine body: a vector-shaped zone that weights,
/// blocks, or arrests grid movement. Client mirror: `RegionEngine` (`@shadowcat/core`).
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
/// (degrees), tessellated per `kind`. Client mirror: `TemplateEngine`
/// (`@shadowcat/core`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct TemplateShape {
    /// "circle" | "cone" | "rect" | "line" (`template-view.ts`'s tessellation
    /// vocabulary; kept a `String` in v1).
    pub kind: String,
    /// Anchor x, scene units.
    pub x: f64,
    /// Anchor y, scene units.
    pub y: f64,
    /// Radius/length, scene units.
    pub size: f64,
    /// Orientation in degrees; the render layer converts via standard radian
    /// math (`template-view.ts`).
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
