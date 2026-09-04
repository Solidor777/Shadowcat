//! Per-(scene, player) explored fog memory: a sparse set of visited grid cells,
//! accumulated monotonically from each vision recompute. Engine-owned geometry,
//! headless + pure (the DB round-trip lives in the repository). Clean-room.
//!
//! A cell `(i, j)` covers world rect `[i*size, (i+1)*size) × [j*size, (j+1)*size)`. A vision
//! recompute marks every cell whose CENTER lies inside any `visible` polygon (resolution = one
//! grid cell — sufficient for the dimmed "explored memory" layer). Accumulation is a
//! set union, so revisiting marks nothing new (bounded by O(explored area), no growth on revisit).

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::scene::grid_shape::{exceeds_cell_cap, GridShape};
use crate::scene::vision;
use crate::scene::GridKind;
use std::collections::BTreeSet;

/// Magic prefix of a serialized `ExploredSet`. A blob without it states no coordinate system for
/// its records and is unusable rather than assumed.
const EXPLORED_MAGIC: [u8; 4] = *b"SCEF";

/// Serialization format version. A blob at any other version is not decoded.
const EXPLORED_VERSION: u8 = 1;

/// Header length: magic, version, grid-kind tag.
const EXPLORED_HEADER_LEN: usize = EXPLORED_MAGIC.len() + 2;

/// Grid-kind tag byte for `kind`.
fn kind_tag(kind: GridKind) -> u8 {
    match kind {
        GridKind::Square => 0,
        GridKind::Hex => 1,
    }
}

/// A grid-cell coordinate. `BTreeSet` ordering gives a deterministic serialization.
pub type Cell = (i32, i32);

/// Hard cap on candidate cells scanned per polygon/source. A wall or LOS bbox authored at an
/// extreme coordinate with a tiny grid size could otherwise span billions of cells and stall the
/// dispatch path. Every production call site (`accumulate_visible_cells`, `player_lit_mask`)
/// reaches its scan box through `scan_box_for`, which intersects an over-cap box
/// with a bounded window around the source's own focus before any of them enumerate.
/// `cells_in_bounds` enforces this cap directly and returns `None` — the candidate set is then
/// skipped, under-reveal — when the box it receives is still over it (a degenerate window) or for
/// any caller that bypasses `scan_box_for`.
pub(crate) const MAX_CELLS_PER_POLYGON: i64 = 4_000_000;

/// Half-extent, in CELLS, of the window an over-cap candidate scan is clamped to.
///
/// Sized so the window itself can never be refused by `MAX_CELLS_PER_POLYGON`: a square window of
/// `2*HALF + 1` cells per side enumerates `(2*HALF + 1)^2` cells, and `HALF` is the largest value
/// keeping that product at or under the cap. Hex enumerates FEWER cells for the same pixel window
/// — the axial preimage of a pixel box is a sheared parallelogram whose integer bounding box is
/// smaller than the square index rectangle of the same box — so bounding the square case bounds
/// both, and `a_clamped_hex_window_also_stays_inside_the_per_polygon_cap` measures that through
/// `HexGrid::cell_bounds` rather than assuming it.
pub(crate) const SCAN_WINDOW_HALF_CELLS: i64 = 999;

/// Which sampling mode a caller of `scan_box_for` wants for the box IT scans — never which box the
/// clamp decision is made from; `scan_box_for` always decides from the padded box regardless of
/// `mode` (see its doc).
pub(crate) enum ScanMode {
    /// The source's box as authored, unpadded: `player_lit_mask`'s scan and
    /// `accumulate_visible_cells`'s strict invocation.
    Strict,
    /// The box padded by one cell on every side (corner-sampling headroom):
    /// `accumulate_visible_cells`'s lenient invocation.
    Lenient,
}

/// Pad an AABB by `pad` scene units on every side. The one place a leniency pad is spelled out —
/// `scan_box_for` derives both the box a `Lenient` caller scans AND the (always-padded) decision
/// box from this single call, so widening leniency is a one-line change to `scan_box_for` alone.
pub(crate) fn pad_box(bbox: (vision::P, vision::P), pad: f64) -> (vision::P, vision::P) {
    let (min, max) = bbox;
    ((min.0 - pad, min.1 - pad), (max.0 + pad, max.1 + pad))
}

/// The ONE symbol that owns scan geometry for a source. Given the grid, the source's own focus
/// (a viewpoint, or a polygon's bbox centre), its unpadded `bbox`, the cell size and the cap,
/// returns the box a caller wanting `mode` should scan. No call site computes a pad, a decision
/// box, a span, or a cap comparison of its own — every production caller (`player_lit_mask`'s
/// scan, both `accumulate_visible_cells` invocations) calls this and scans exactly what it
/// returns.
///
/// The clamp DECISION is always made from `pad_box(bbox, cell)` — the one-cell-padded box —
/// regardless of `mode`: it is the largest box ANY mode would scan for this source, so every
/// mode's own box, padded or not, is intersected against the SAME window. `Strict`'s unpadded box
/// is a subset of `Lenient`'s padded one (`A ⊆ A'`), and intersecting both with one window `W`
/// gives `A ∩ W ⊆ A' ∩ W` — GIVEN the PRECONDITION stated here, `strict ⊆ lenient` holds structurally,
/// not by argument about which branch ran; `clamp_scan_window`'s inverted-window fallback is the
/// one branch this does NOT hold across, since it returns `actual` unchanged rather than
/// `actual ∩ W` and is reachable only when the precondition fails (see PRECONDITION). Deciding
/// independently per mode instead leaves a reachable band where the smaller unpadded box sits at
/// or under the cap (returned whole) while the larger padded box exceeds it (windowed), so the
/// unclamped result can hold a cell the windowed one does not.
///
/// This is also why `player_lit_mask`'s scan — which has no lenient counterpart of its own — must
/// still call this with `Strict` rather than compute its box directly: its decision is the SAME
/// padded box `accumulate_visible_cells`'s strict call decides from, so the two produce IDENTICAL
/// candidate sets for the same source (`cell_visible`'s own doc states this parity as an
/// invariant). A caller that instead decided from its own unpadded box alone would sit below the
/// cap in a band where the padded decision clamps — enumerating strictly more cells than the
/// movement gate's own strict scan for the same source, an under-permissive divergence between
/// what a player is shown and what they may move through.
///
/// PRECONDITION: `focus` lies inside `bbox` (and therefore inside the padded box, which only grows
/// it). Every caller satisfies it: a visibility source sits inside its own LOS polygon's bbox. A
/// focus far enough outside `bbox` that the window
/// misses the box being scanned returns that box unchanged and lets the callee's own cap decide,
/// rather than yielding an inverted, enumerates-nothing rectangle — this is the inverted-window
/// fallback this doc names.
///
/// Returns the mode's own box unchanged for a degenerate `cell`, `focus`, or `bbox` as well — the
/// callee's fail-closed `None` on a degenerate input is the correct outcome there and must not be
/// masked.
pub(crate) fn scan_box_for(
    grid: &dyn GridShape,
    focus: vision::P,
    bbox: (vision::P, vision::P),
    cell: f64,
    max_cells: i64,
    mode: ScanMode,
) -> (vision::P, vision::P) {
    let padded = pad_box(bbox, cell);
    let actual = match mode {
        ScanMode::Strict => bbox,
        ScanMode::Lenient => padded,
    };
    clamp_scan_window(grid, focus, actual, padded, cell, max_cells)
}

/// Intersect a candidate-scan AABB `actual = (min, max)` with a window of `SCAN_WINDOW_HALF_CELLS`
/// cells around `focus`, but ONLY when `decision = (decision_min, decision_max)`'s own candidate
/// count exceeds `max_cells`. The low-level intersection primitive `scan_box_for` builds on — see
/// that function's doc for why `decision` and `actual` must be allowed to differ.
///
/// An over-cap scan makes `GridShape::cells_in_bounds` return `None`, and every caller of that
/// primitive treats `None` as "skip this source/polygon" — an empty mask, which on the movement
/// gate refuses every move and on egress ships no cells. Clamping keeps such a scan enumerable, at
/// a bounded SUBSET of the unclamped candidate set: each caller's fail direction stays the
/// under-revealing one (fewer cells admitted, fewer cells shipped, fewer cells remembered), and
/// the outcome is a degradation the source survives rather than the source's whole contribution.
///
/// The span test is what keeps this from taking cells away from a scan that was never in trouble.
/// The cap bounds a PRODUCT of two cell counts; the window bounds a PER-AXIS distance from a focus
/// that sits wherever the source does, not at the box's centre. A box can therefore reach far
/// beyond the window on both axes and still enumerate fewer cells than the cap allows, and those
/// cells are in the mask a player moves through. So the span is computed first, and `exceeds_cell_cap`
/// — the SAME predicate `cells_in_bounds` enforces against — decides whether `decision` is over the
/// cap; a `decision` within the cap returns `actual` untouched.
///
/// PRECONDITION: `focus` lies inside `actual` and inside `decision`. A focus far enough outside
/// `actual` that the window misses it would otherwise yield `min > max`, an inverted rectangle that
/// enumerates nothing, so that case returns `actual` unchanged and lets the callee's own cap
/// decide.
///
/// Returns `actual` unchanged for a degenerate `cell`, `focus`, or either box as well — the
/// callee's fail-closed `None` on a degenerate input is the correct outcome there and must not be
/// masked.
fn clamp_scan_window(
    grid: &dyn GridShape,
    focus: vision::P,
    actual: (vision::P, vision::P),
    decision: (vision::P, vision::P),
    cell: f64,
    max_cells: i64,
) -> (vision::P, vision::P) {
    let (min, max) = actual;
    let (decision_min, decision_max) = decision;
    if !cell.is_finite()
        || cell <= 0.0
        || !focus.0.is_finite()
        || !focus.1.is_finite()
        || !min.0.is_finite()
        || !min.1.is_finite()
        || !max.0.is_finite()
        || !max.1.is_finite()
        || !decision_min.0.is_finite()
        || !decision_min.1.is_finite()
        || !decision_max.0.is_finite()
        || !decision_max.1.is_finite()
    {
        return (min, max);
    }
    let bounds = grid.cell_bounds(decision_min, decision_max, cell);
    if !exceeds_cell_cap(bounds, max_cells) {
        return (min, max);
    }
    let half_px = SCAN_WINDOW_HALF_CELLS as f64 * cell;
    let win_min = (min.0.max(focus.0 - half_px), min.1.max(focus.1 - half_px));
    let win_max = (max.0.min(focus.0 + half_px), max.1.min(focus.1 + half_px));
    if win_min.0 > win_max.0 || win_min.1 > win_max.1 {
        return (min, max);
    }
    (win_min, win_max)
}

/// A sparse explored-cell set for one (scene, player).
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct ExploredSet {
    /// Explored cells, ordered (deterministic wire/persistence output).
    cells: BTreeSet<Cell>,
}

impl ExploredSet {
    /// An empty set.
    ///
    /// # Examples
    ///
    /// ```
    /// let s = shadowcat::scene::explored::ExploredSet::new();
    /// assert!(s.is_empty());
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of explored cells.
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Whether no cell has been explored.
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Whether `c` is in the explored memory (the `Revealed` gate's second arm).
    pub fn contains(&self, c: Cell) -> bool {
        self.cells.contains(&c)
    }

    /// The cells in ascending (i, j) order.
    pub fn iter(&self) -> impl Iterator<Item = Cell> + '_ {
        self.cells.iter().copied()
    }

    /// Mark `cells` (already indexed in this scene's grid coordinates — the lit mask's own
    /// `(i, j)` cells, which are center-sampled through the scene's resolved `GridShape`) as
    /// explored. Returns the count of newly-added cells (0 ⇒ no growth). THE explored writer:
    /// `ws::conn::enrich_vision_explored` feeds it the recipient's currently-VISIBLE cells (the
    /// `vision` payload's `lit` set — line of sight ∩ illumination), never a line-of-sight
    /// polygon on its own, so a player remembers only terrain they could actually see.
    /// Correctness (the `Revealed` gate composes this set with `GridShape::line_traversal`
    /// move-cells) requires the cells to be indexed by the SAME resolved shape the gate and the
    /// vision mask use for this scene, which holding them from the mask guarantees.
    pub(crate) fn mark_cells(&mut self, cells: impl IntoIterator<Item = Cell>) -> usize {
        let before = self.cells.len();
        self.cells.extend(cells);
        self.cells.len() - before
    }

    /// Serialize as `SCEF`, a version byte, a grid-kind tag, then 8 bytes per cell (i32 i, i32 j,
    /// little-endian) in ascending order. `kind` is the grid family the cell indices are
    /// expressed in; `from_bytes` refuses a blob whose tag disagrees with the scene's current
    /// kind, because a square index and a hex axial index are different coordinate systems that
    /// share a representation.
    pub fn to_bytes(&self, kind: GridKind) -> Vec<u8> {
        let mut out = Vec::with_capacity(EXPLORED_HEADER_LEN + self.cells.len() * 8);
        out.extend_from_slice(&EXPLORED_MAGIC);
        out.push(EXPLORED_VERSION);
        out.push(kind_tag(kind));
        for &(i, j) in &self.cells {
            out.extend_from_slice(&i.to_le_bytes());
            out.extend_from_slice(&j.to_le_bytes());
        }
        out
    }

    /// Deserialize the `to_bytes` layout, refusing anything that is not this format at this
    /// version indexed in `kind`. Every refusal yields an EMPTY set: explored memory is
    /// best-effort and an empty set under-reveals, which is the safe direction for a fog gate. A
    /// trailing partial record is likewise dropped rather than erroring.
    pub fn from_bytes(b: &[u8], kind: GridKind) -> Self {
        if b.len() < EXPLORED_HEADER_LEN
            || b[..EXPLORED_MAGIC.len()] != EXPLORED_MAGIC
            || b[EXPLORED_MAGIC.len()] != EXPLORED_VERSION
            || b[EXPLORED_MAGIC.len() + 1] != kind_tag(kind)
        {
            return Self::default();
        }
        let mut cells = BTreeSet::new();
        for rec in b[EXPLORED_HEADER_LEN..].as_chunks::<8>().0 {
            let i = i32::from_le_bytes([rec[0], rec[1], rec[2], rec[3]]);
            let j = i32::from_le_bytes([rec[4], rec[5], rec[6], rec[7]]);
            cells.insert((i, j));
        }
        Self { cells }
    }
}

#[cfg(test)]
mod tests;
