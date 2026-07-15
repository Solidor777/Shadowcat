//! `token`/`actor` engine bands (M13-0 S1/S3). Field shapes are transcribed
//! verbatim from the client's `scene-docs.ts` `TokenSystem`/`ActorSystem`
//! (minus `name`, which moved to the envelope per S2).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// A token's transform + visual (scene-docs.ts:146-162 `TokenSystem`). `(x,y)`
/// is the token CENTER. `visual` is set only on raw (actorless) tokens —
/// actor-backed tokens resolve their visual via the linked/embedded actor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct TokenEngine {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub rotation: f64,
    #[serde(default)]
    pub visual: Option<TokenVisual>,
    /// Linked token: the shared actor's id (absent/null ⇒ instanced, see
    /// `Document.embedded["actor"]`).
    #[serde(default)]
    pub actor_id: Option<Uuid>,
    #[serde(default)]
    pub overrides: Option<TokenOverrides>,
    /// Active face name when the effective visual is a `faces` union member;
    /// token-local always (not part of `overrides` — selects INTO the
    /// actor's faces map, not an override of actor data).
    #[serde(default)]
    pub face: Option<String>,
}

/// The per-token override whitelist for a linked token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct TokenOverrides {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub visual: Option<TokenVisual>,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct Size {
    pub w: f64,
    pub h: f64,
}

/// A per-actor or per-token vision assignment: which mode (by id, referencing
/// a `vision-modes` registry entry) + effective range in grid cells.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct VisionAssignment {
    pub mode: String,
    pub range: f64,
}

/// The client-owned token/actor visual union (M10h). Internally tagged on
/// `kind`; serde does not support `deny_unknown_fields` on an internally
/// tagged enum (a documented limitation — NOT applied here).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TokenVisual {
    Image {
        asset: String,
    },
    Animated {
        source: AnimatedSource,
        fps: f64,
        #[serde(rename = "loop")]
        loop_: bool,
    },
    Faces {
        faces: BTreeMap<String, RenderVisual>,
        default: String,
        /// Optional conditionId -> face name map; the first match (in the
        /// token's effective `conditions[]` order) wins over `default`, but
        /// never over a manual `token.engine.face`.
        #[serde(default, rename = "faceMap")]
        face_map: Option<BTreeMap<String, String>>,
    },
}

/// The two kinds the render layer actually draws — the render/resolution
/// boundary (M10h). A face's own visual is always one of these — no nesting
/// (a face can never itself be `{kind:"faces"}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RenderVisual {
    Image {
        asset: String,
    },
    Animated {
        source: AnimatedSource,
        fps: f64,
        #[serde(rename = "loop")]
        loop_: bool,
    },
}

/// An animated visual's frame source: an ordered list of individually
/// uploaded assets, or one grid-sliced sheet asset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum AnimatedSource {
    Frames {
        frames: Vec<String>,
    },
    Sheet {
        asset: String,
        rows: u32,
        cols: u32,
        #[serde(default)]
        count: Option<u32>,
    },
}

/// An actor's engine-owned body (scene-docs.ts:197-209 `ActorSystem`, minus
/// `name` which moves to the envelope). Every other field of `ActorSystem`
/// (inventory, stats, …) lives in `system` — this is a SPLIT type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct ActorEngine {
    #[serde(default, rename = "displayName")]
    pub display_name: Option<String>,
    pub visual: TokenVisual,
    pub size: Size,
    /// "square" | "circle" — kept a `String` in v1 (asserted by the battery).
    pub shape: String,
    #[serde(default)]
    pub faction: Option<String>,
    #[serde(default)]
    pub conditions: Vec<String>,
    /// Default place-mode: true ⇒ instance (independent copy) on drop; false
    /// ⇒ link (shared).
    pub prototype: bool,
    /// Vision modes granted to this actor; each references a `VisionMode` id
    /// + range in grid cells.
    #[serde(default)]
    pub vision: Option<Vec<VisionAssignment>>,
}
