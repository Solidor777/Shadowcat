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
}

impl TokenEngine {
    /// Ingress validation beyond serde shape: every numeric field finite, and
    /// the position inside the ONE shared movement-coordinate bound
    /// (`scene::move_exec::MAX_GATE_WALK_COORD`) — the GM-write/Create path
    /// and the move gate must agree on admissible coordinates structurally,
    /// never by call ordering.
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
        let bound = crate::scene::move_exec::MAX_GATE_WALK_COORD;
        if self.x.abs() > bound || self.y.abs() > bound {
            return Err(format!("position exceeds coordinate bound {bound}"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> TokenEngine {
        TokenEngine {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
            rotation: 0.0,
            visual: None,
            actor_id: None,
            overrides: None,
            face: None,
        }
    }

    #[test]
    fn finite_in_bound_token_validates() {
        assert!(base().validate().is_ok());
    }

    #[test]
    fn non_finite_fields_are_rejected() {
        for f in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut t = base();
            t.x = f;
            assert!(t.validate().is_err(), "x = {f} must be rejected");
            let mut t = base();
            t.rotation = f;
            assert!(t.validate().is_err(), "rotation = {f} must be rejected");
        }
    }

    #[test]
    fn ingress_bound_equals_gate_walks_exactly() {
        // Anti-drift: ingress and the movement gate read ONE symbol with the same
        // strictly-`>` sense.
        let bound = crate::scene::move_exec::MAX_GATE_WALK_COORD;
        let mut t = base();
        t.x = bound;
        assert!(t.validate().is_ok(), "AT the bound is admissible");
        t.x = bound + 1.0;
        assert!(t.validate().is_err(), "over the bound is refused");
        let mut t = base();
        t.y = -(bound + 1.0);
        assert!(t.validate().is_err());

        // The walk side of the same bound, asserted here so one test pins the EQUALITY of the two
        // senses rather than leaving it inferred across two files.
        let at = crate::scene::move_exec::MAX_GATE_WALK_COORD;
        assert!(
            crate::scene::move_exec::gate_walk(&[(at - 100.0, 0.0), (at, 0.0)], 100.0).is_some(),
            "gate_walk admits a coordinate exactly AT the bound"
        );
        assert!(
            crate::scene::move_exec::gate_walk(&[(at - 100.0, 0.0), (at + 1.0, 0.0)], 100.0)
                .is_none(),
            "gate_walk refuses a coordinate over the bound"
        );
    }
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
    /// Size override in GRID UNITS (cells), replacing the actor's own `size`: a medium creature
    /// is `1`. `resolve_token_footprint` derives the footprint radius from the resolved value and
    /// bounds it by `MAX_FOOTPRINT_CELLS`, and the client multiplies it by the scene's cell size
    /// to reach scene units. Not to be confused with `TokenEngine.w`/`h`, which are the token's
    /// rendered box in scene units.
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

/// A width/height pair in GRID UNITS (cells) — an actor's occupied block, not a pixel box.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct Size {
    /// Width in GRID UNITS (cells): a medium creature is `1`. `resolve_token_footprint` derives
    /// the footprint radius from this directly and bounds it by `MAX_FOOTPRINT_CELLS`, and the
    /// client multiplies it by the scene's cell size to reach scene units. Not to be confused
    /// with `TokenEngine.w`, which is the token's rendered box in scene units.
    pub w: f64,
    /// Height in GRID UNITS (cells): a medium creature is `1`. `resolve_token_footprint` derives
    /// the footprint radius from this directly and bounds it by `MAX_FOOTPRINT_CELLS`, and the
    /// client multiplies it by the scene's cell size to reach scene units. Not to be confused
    /// with `TokenEngine.h`, which is the token's rendered box in scene units.
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
    /// Effective range in grid CELLS (not scene units).
    pub range: f64,
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
}

/// The two kinds the render layer actually draws — the render/resolution
/// boundary. A face's own visual is always one of these — no nesting
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
    /// Default token size for this actor in GRID UNITS (cells): a medium creature is `1`.
    /// `resolve_token_footprint` derives the footprint radius from this directly and bounds it by
    /// `MAX_FOOTPRINT_CELLS`, and the client multiplies it by the scene's cell size to reach scene
    /// units. Not to be confused with `TokenEngine.w`/`h`, which are the token's rendered box in
    /// scene units.
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
}
