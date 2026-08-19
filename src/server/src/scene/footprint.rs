//! The single resolved-footprint definition, and the wire payload that carries it to the client.
//!
//! A token's footprint is one geometry with two consumers that must never disagree: the
//! movement/routing gate collides with its bounding-disc radius, and the client draws and
//! hit-tests its bounding box. `resolve_footprint_cells` is the only place either is computed —
//! the client re-derives nothing and reads `FootprintsPayload` instead.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::GridKind;

/// A token's resolved footprint in GRID UNITS (cells on square, hexes on hex): the drawn bounding
/// box and the bounding-disc collision radius, produced together by `resolve_footprint_cells` so
/// the drawn geometry and the collided geometry cannot describe different tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FootprintCells {
    /// Bounding-box width, in grid units.
    pub(crate) box_w: f64,
    /// Bounding-box height, in grid units.
    pub(crate) box_h: f64,
    /// Bounding-disc collision radius, in grid units.
    pub(crate) radius: f64,
}

/// THE footprint definition: given a token's render shape, its authored size, and the scene's
/// grid kind, the drawn box and the collision radius in GRID UNITS. Every other footprint value
/// in the engine — the gate radius `SceneEcs::resolve_token_footprint` returns, and the scene-unit
/// extents `SceneEcs::resolved_footprints` puts on the wire — is a read of this function's
/// output, never a second expression.
///
/// Square: the drawn box is the authored `w × h` block itself; the collision radius is that
/// block's conservative enclosure — a circle uses `max(w,h)/2`, any other shape its half-diagonal
/// `hypot(w,h)/2`.
///
/// Hex: a token's authored size counts HEXES, so `shape` is inert — a hex tessellation draws no
/// square/circle footprint distinction. `n = max(w,h)` hexes; the box is `n` hexes' own bounding
/// box (`n·√3` wide by `n·2` tall, the pointy-top hex's across-the-flats width and
/// point-to-point height in circumradii), and the conservative enclosure of a hex is its own
/// circumradius, so the radius is `n`.
///
/// Both outputs are expressed in the grid's INDEXING scale (`grid.size`, the circumradius on hex),
/// never `GridShape::world_units_per_cell`: a caller converts to scene units by multiplying by
/// `grid.size` alone.
pub(crate) fn resolve_footprint_cells(
    kind: GridKind,
    shape: &str,
    w: f64,
    h: f64,
) -> FootprintCells {
    match kind {
        GridKind::Hex => {
            let n = w.max(h);
            FootprintCells {
                box_w: n * 3f64.sqrt(),
                box_h: n * 2.0,
                radius: n,
            }
        }
        GridKind::Square => FootprintCells {
            box_w: w,
            box_h: h,
            radius: if shape == "circle" {
                w.max(h) / 2.0
            } else {
                w.hypot(h) / 2.0
            },
        },
    }
}

/// Why the server declines to state a token's footprint. Both arms fail the token closed: the
/// movement gate refuses the move outright, and the wire carries no extent for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FootprintRefusal {
    /// A non-finite or negative authored size.
    DegenerateSize,
    /// The derived radius leaves `[0, MAX_FOOTPRINT_CELLS]`.
    OverCap,
}

/// `resolve_footprint_cells` behind the ONE refusal decision, so the gate radius and the drawn
/// extent refuse for the same inputs or not at all. Clamping instead of refusing would route and
/// gate an oversized token as a smaller disc, letting it enter gaps its real footprint cannot — a
/// geometric fail-open — so every caller propagates the refusal rather than substituting a value.
pub(crate) fn resolve_checked(
    kind: GridKind,
    shape: &str,
    w: f64,
    h: f64,
) -> Result<FootprintCells, FootprintRefusal> {
    if !w.is_finite() || !h.is_finite() || w < 0.0 || h < 0.0 {
        return Err(FootprintRefusal::DegenerateSize);
    }
    let f = resolve_footprint_cells(kind, shape, w, h);
    if !(0.0..=super::pathfinding::MAX_FOOTPRINT_CELLS).contains(&f.radius) {
        return Err(FootprintRefusal::OverCap);
    }
    Ok(f)
}

/// A drawn footprint's extent in SCENE units (already scaled by the scene's `grid.size`), as the
/// client renders and hit-tests it. Carries no radius: the collision radius is a server-side gate
/// quantity and is deliberately not disclosed as a client-consumable number.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct FootprintExtent {
    /// Bounding-box width in scene units.
    pub w: f64,
    /// Bounding-box height in scene units.
    pub h: f64,
}

/// One token's resolved drawn extent.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct TokenFootprint {
    /// The token document this extent belongs to.
    pub token: Uuid,
    /// The resolved extent, or `None` where the server REFUSES to state one because the authored
    /// size is degenerate or exceeds `pathfinding::MAX_FOOTPRINT_CELLS` — the same refusal
    /// `SceneEcs::resolve_token_footprint` returns `None` for. A refusal is not a permission: the
    /// client's only uses of an extent are drawing and picking, and the movement gate that does
    /// grant passage runs server-side off the radius, where the identical refusal blocks the move
    /// outright.
    pub extent: Option<FootprintExtent>,
}

/// Every resolved token extent in one scene, plus that scene's unit footprint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct SceneFootprints {
    /// The scene these extents are resolved against.
    pub scene: Uuid,
    /// The resolved extent of a 1x1 token in this scene's grid — what the optimistic placement
    /// path stamps onto a token it creates, so a freshly placed token draws at one cell until its
    /// own resolved extent arrives.
    pub unit: FootprintExtent,
    /// One entry per token in this scene whose footprint the server resolves and the recipient
    /// may read. A token absent from this list has none: it carries no actor, its actor link
    /// dangles, or the recipient cannot read the document.
    pub tokens: Vec<TokenFootprint>,
}

/// The `"footprints"` derived channel's payload: the resolved drawn geometry for every scene the
/// recipient can see, so the client renders authoritative footprints instead of mirroring the
/// formula that produced them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct FootprintsPayload {
    /// One entry per scene that has a resolvable grid.
    pub scenes: Vec<SceneFootprints>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One hex spans `√3` circumradii across the flats and `2` point-to-point, and its
    /// conservative enclosure is its own circumradius.
    #[test]
    fn hex_bounding_box_is_root_three_by_two() {
        let f = resolve_footprint_cells(GridKind::Hex, "square", 1.0, 1.0);
        assert_eq!(f.box_w, 3f64.sqrt());
        assert_eq!(f.box_h, 2.0);
        assert_eq!(f.radius, 1.0);
    }

    /// On hex a token's size counts hexes, so the render shape carries no footprint meaning.
    #[test]
    fn hex_ignores_shape_and_scales_by_the_larger_authored_axis() {
        assert_eq!(
            resolve_footprint_cells(GridKind::Hex, "circle", 2.0, 1.0),
            resolve_footprint_cells(GridKind::Hex, "square", 2.0, 1.0)
        );
        let f = resolve_footprint_cells(GridKind::Hex, "square", 2.0, 1.0);
        assert_eq!(f.box_w, 2.0 * 3f64.sqrt());
        assert_eq!(f.box_h, 4.0);
        assert_eq!(f.radius, 2.0);
    }

    /// Square keeps the authored block as its box and the conservative enclosure as its radius:
    /// a circle's own radius, any other shape's half-diagonal.
    #[test]
    fn square_box_is_the_authored_block_and_radius_is_the_conservative_enclosure() {
        let sq = resolve_footprint_cells(GridKind::Square, "square", 1.0, 1.0);
        assert_eq!((sq.box_w, sq.box_h), (1.0, 1.0));
        assert_eq!(sq.radius, 2f64.sqrt() / 2.0);
        let ci = resolve_footprint_cells(GridKind::Square, "circle", 2.0, 4.0);
        assert_eq!((ci.box_w, ci.box_h), (2.0, 4.0));
        assert_eq!(ci.radius, 2.0);
    }
}
