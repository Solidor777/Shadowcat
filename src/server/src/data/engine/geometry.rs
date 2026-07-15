//! `wall`, `region`, `drawing`, `template` engine bands (M13-0 S1/S3). Field
//! shapes transcribed verbatim from the server's existing pointer-walk
//! consumers (wall/region: scene-docs.ts has no dedicated wall type — the
//! shape below mirrors `scene/mod.rs`'s current reads) and the render
//! layer's local shapes (`drawing-view.ts:9-13`, `template-view.ts:9-11`) —
//! the only authoritative shapes today; scene-tools writers must round-trip
//! byte-identically against these.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct Seg {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

/// A wall's segment + sight/light/movement-blocking flags. Absent/false
/// flags exclude the wall from that gate exactly as the pre-M13-0 pointer
/// read did (read-side backstop unchanged).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WallEngine {
    pub seg: Seg,
    #[serde(default)]
    pub blocks_sight: Option<bool>,
    #[serde(default)]
    pub blocks_light: Option<bool>,
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
    pub points: Vec<f64>,
}

/// A region document's engine body: a vector-shaped zone that weights,
/// blocks, or arrests grid movement (scene-docs.ts:565-570 `RegionSystem`).
/// `cost` is a multiplier (>=1, clamped read-side) meaningful only for
/// `behavior:"terrain"`. `enabled` lets a GM toggle a region off without
/// deleting it (disabled regions are dropped entirely at read time).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct RegionEngine {
    pub shape: RegionShape,
    /// "terrain" | "impassable" | "arrest" — kept a `String` in v1 (asserted
    /// by the battery).
    pub behavior: String,
    pub cost: f64,
    pub enabled: bool,
}

/// `points` layout mirrors `RegionShape` (path vertices for freehand/line/
/// polygon, or bbox corners `[x0,y0,x1,y1]` for rect/ellipse).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct DrawingShape {
    pub kind: String,
    pub points: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct Stroke {
    pub color: String,
    pub width: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct Fill {
    pub color: String,
    #[serde(default)]
    pub alpha: Option<f64>,
}

/// A drawing document's engine body (`drawing-view.ts:9-13` `DrawingSystem`).
/// `stroke`/`fill` are each a required-but-nullable field on the wire
/// (`{...} | null`, not optional) — `Option<T>` without a serde default
/// mirrors that exactly (the key must be present, either an object or
/// `null`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct DrawingEngine {
    pub shape: DrawingShape,
    pub stroke: Option<Stroke>,
    pub fill: Option<Fill>,
}

/// A template's area anchored at `(x,y)` with a `size` and `direction`
/// (degrees), tessellated per `kind` (`template-view.ts:9-11`
/// `TemplateSystem`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct TemplateShape {
    pub kind: String,
    pub x: f64,
    pub y: f64,
    pub size: f64,
    pub direction: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct TemplateEngine {
    pub shape: TemplateShape,
    pub color: String,
}
