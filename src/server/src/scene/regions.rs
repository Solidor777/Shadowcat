//! Region primitive: vector-shaped zones that weight, block, or arrest grid movement.
//! Pure geometry — no ECS, no I/O (mirrors the `scene::movement` module's invariant). Consumed by
//! `SceneEcs::region_field` (hydration + visibility filtering) and `scene::pathfinding` /
//! `scene::move_exec` (the two enforcement points).

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::{BTreeMap, BTreeSet};

use uuid::Uuid;

use crate::data::engine::{RegionTrigger, TriggerEvent};

/// A grid cell `(i, j)` (same convention as `pathfinding::Cell`).
pub(crate) type Cell = (i32, i32);

/// Authored region geometry (vector-shape vocabulary: rect/circle/polygon).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RegionShape {
    /// Axis-aligned rectangle; corners may arrive in any order (normalized
    /// min/max at the containment test).
    Rect {
        /// First corner x, scene units.
        x0: f64,
        /// First corner y, scene units.
        y0: f64,
        /// Opposite corner x, scene units.
        x1: f64,
        /// Opposite corner y, scene units.
        y1: f64,
    },
    /// Circle by center + radius, scene units.
    Circle {
        /// Center x.
        cx: f64,
        /// Center y.
        cy: f64,
        /// Radius.
        r: f64,
    },
    /// Simple polygon by vertex list; `< 3` points fails closed at `rasterize`.
    Polygon {
        /// Vertices in order, scene units.
        points: Vec<(f64, f64)>,
    },
}

/// The region's gameplay effect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RegionBehavior {
    /// Weighted (difficult) terrain: multiplies entry cost by the region cost.
    Terrain,
    /// Cells cannot be entered at all.
    Impassable,
    /// Entering a cell stops the move there (trap/hazard semantics).
    Arrest,
}

/// Per-cell composed effect after precedence + MAX overlap resolution.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RegionEffect {
    /// At least one impassable region covers the cell (highest precedence).
    Impassable,
    /// An arrest region covers the cell (no impassable does).
    Arrest,
    /// Terrain cost multiplier, always >= 1.0 (validated at the doc layer, `region_field`).
    Terrain(f64),
}

/// DoS guard: a region whose rasterized AABB cell count would exceed this is dropped (fails
/// closed to "contributes nothing"), never silently rasterized to a partial/wrong set. Mirrors
/// `pathfinding::MAX_FOOTPRINT_CELLS`'s fail-closed discipline.
pub(crate) const MAX_REGION_CELLS: i64 = 100_000;

/// Rasterize `shape` to the grid cells whose CENTER falls inside it. Fails closed (`None`) on a
/// degenerate shape (non-finite coords, non-positive circle radius, polygon with `< 3` vertices)
/// or an over-cap AABB — never returns a partial or silently-empty result that a caller could
/// mistake for "shape covers no cells".
pub(crate) fn rasterize(
    shape: &RegionShape,
    cell: f64,
    grid: &dyn crate::scene::grid_shape::GridShape,
) -> Option<Vec<Cell>> {
    if !cell.is_finite() || cell <= 0.0 {
        return None;
    }
    let (minx, miny, maxx, maxy) = match shape {
        RegionShape::Rect { x0, y0, x1, y1 } => {
            if ![*x0, *y0, *x1, *y1].iter().all(|v| v.is_finite()) {
                return None;
            }
            (x0.min(*x1), y0.min(*y1), x0.max(*x1), y0.max(*y1))
        }
        RegionShape::Circle { cx, cy, r } => {
            if !cx.is_finite() || !cy.is_finite() || !r.is_finite() || *r <= 0.0 {
                return None;
            }
            (cx - r, cy - r, cx + r, cy + r)
        }
        RegionShape::Polygon { points } => {
            if points.len() < 3 || !points.iter().all(|(x, y)| x.is_finite() && y.is_finite()) {
                return None;
            }
            let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for (x, y) in points {
                minx = minx.min(*x);
                miny = miny.min(*y);
                maxx = maxx.max(*x);
                maxy = maxy.max(*y);
            }
            (minx, miny, maxx, maxy)
        }
    };
    // `Cell = (i32, i32)`: an extreme finite coordinate must fail closed to `None` (reject), never
    // alias onto a real cell. This pre-check is RETAINED rather than delegated to `cells_in_bounds`:
    // that primitive's `f64 as i32` cast SATURATES to `i32::MAX/MIN` on a large-but-small-span
    // coordinate (e.g. 1e13) and the saturated span then passes the cell-count cap, so the aliased
    // candidate survives to the per-cell center test and yields `Some(empty)` — not the `None` this
    // region gate requires. Bounding `floor(coord/cell)` to i32's safe range here forces every such
    // input to `None` before enumeration. `-1e300` (huge span) is also caught here (its floor index
    // exceeds the bound); either way the extreme-coordinate contract is reject, never alias.
    const MAX_CELL_COORD: f64 = (i32::MAX as f64) - 1.0;
    let i0f = (minx / cell).floor();
    let i1f = (maxx / cell).floor();
    let j0f = (miny / cell).floor();
    let j1f = (maxy / cell).floor();
    if !(i0f.abs() <= MAX_CELL_COORD
        && i1f.abs() <= MAX_CELL_COORD
        && j0f.abs() <= MAX_CELL_COORD
        && j1f.abs() <= MAX_CELL_COORD)
    {
        return None;
    }
    // Candidate enumeration via `GridShape::cells_in_bounds` (square: byte-identical
    // `floor(min/cell)..=floor(max/cell)` row-major rectangle; hex: axial-bounds superset), capped
    // at `MAX_REGION_CELLS` — the 40× tighter region DoS bound, passed explicitly so routing
    // through the shared primitive can't loosen it to the vision scans' cap. The per-cell center
    // test (via the SAME `GridShape`) narrows the superset to the exact covered cells, so
    // rasterize and `move_exec`'s `grid.cell_of` lookup agree on which cell (square or hex) the
    // shape occupies.
    let candidates = grid.cells_in_bounds((minx, miny), (maxx, maxy), cell, MAX_REGION_CELLS)?;
    let mut out = Vec::new();
    for c in candidates {
        let ctr = grid.cell_center(c);
        if cell_center_in_shape(ctr, shape) {
            out.push(c);
        }
    }
    Some(out)
}

/// Whether point `p` (a cell center) lies inside `shape`. Rect/circle edges
/// are inclusive; the polygon branch is even-odd PNPOLY, whose exact-boundary
/// behavior is winding-dependent, not uniformly inclusive.
fn cell_center_in_shape(p: (f64, f64), shape: &RegionShape) -> bool {
    match shape {
        RegionShape::Rect { x0, y0, x1, y1 } => {
            let (minx, maxx) = (x0.min(*x1), x0.max(*x1));
            let (miny, maxy) = (y0.min(*y1), y0.max(*y1));
            p.0 >= minx && p.0 <= maxx && p.1 >= miny && p.1 <= maxy
        }
        RegionShape::Circle { cx, cy, r } => {
            let dx = p.0 - cx;
            let dy = p.1 - cy;
            dx * dx + dy * dy <= r * r
        }
        RegionShape::Polygon { points } => point_in_polygon(p, points),
    }
}

/// Even-odd ray-casting point-in-polygon test. Source: Franklin, PNPOLY (standard algorithm,
/// public domain reference implementation). Precondition: `poly.len() >= 3` (unchecked here — the
/// sole call site, `cell_center_in_shape`, only reaches a `Polygon` after `rasterize`'s `len() < 3`
/// guard has already rejected shorter shapes).
fn point_in_polygon(p: (f64, f64), poly: &[(f64, f64)]) -> bool {
    let mut inside = false;
    let n = poly.len();
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if ((yi > p.1) != (yj > p.1)) && (p.0 < (xj - xi) * (p.1 - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Compose zero or more (behavior, cost) contributions for ONE cell into a single effect:
/// precedence `Impassable > Arrest > Terrain`; overlapping Terrain costs take the MAX
/// (not summed — difficulty is not cumulative). `None` when nothing contributes.
pub(crate) fn compose(contributions: &[(RegionBehavior, f64)]) -> Option<RegionEffect> {
    if contributions
        .iter()
        .any(|(b, _)| *b == RegionBehavior::Impassable)
    {
        return Some(RegionEffect::Impassable);
    }
    if contributions
        .iter()
        .any(|(b, _)| *b == RegionBehavior::Arrest)
    {
        return Some(RegionEffect::Arrest);
    }
    contributions
        .iter()
        .filter(|(b, _)| *b == RegionBehavior::Terrain)
        .map(|(_, cost)| *cost)
        .fold(None, |acc: Option<f64>, c| {
            Some(acc.map_or(c, |a| a.max(c)))
        })
        .map(RegionEffect::Terrain)
}

/// The composed per-cell region effect for one scene, already resolved to either a single
/// requester's visibility (the router's per-requester field) or the full authoritative set
/// (the GM's field / `move_exec`'s field). Built by `RegionFieldBuilder`.
#[derive(Debug, Default, Clone)]
pub(crate) struct RegionField {
    /// Composed effect per covered cell; absent cell = no region effect.
    cells: BTreeMap<Cell, RegionEffect>,
}

impl RegionField {
    /// An empty accumulating builder.
    ///
    /// # Examples
    ///
    /// ```text
    /// let mut b = RegionField::builder();
    /// b.add(&shape, RegionBehavior::Terrain, 2.0, cell, grid);
    /// let field = b.build();
    /// ```
    pub(crate) fn builder() -> RegionFieldBuilder {
        RegionFieldBuilder::default()
    }

    /// Whether `c` composes to `Impassable` (router prune + gate refusal).
    pub(crate) fn is_impassable(&self, c: Cell) -> bool {
        matches!(self.cells.get(&c), Some(RegionEffect::Impassable))
    }

    /// Whether `c` composes to `Arrest` (route truncation point).
    pub(crate) fn is_arrest(&self, c: Cell) -> bool {
        matches!(self.cells.get(&c), Some(RegionEffect::Arrest))
    }

    /// Terrain cost multiplier for entering `c`; 1.0 (no weighting) outside any terrain region.
    pub(crate) fn terrain_multiplier(&self, c: Cell) -> f64 {
        match self.cells.get(&c) {
            Some(RegionEffect::Terrain(m)) => *m,
            _ => 1.0,
        }
    }

    /// True iff any cell is `Impassable` or weighted `Terrain` (multiplier > 1.0). The
    /// dispatch predicate the continuous router uses to decide between the weighted grid
    /// route (terrain/impassable present) and the pure any-angle polyanya route (neither).
    /// Arrest is excluded: it neither bends the route nor requires route-around, so an
    /// arrest-only scene stays on the polyanya path with an arrest post-filter.
    pub(crate) fn has_terrain_or_impassable(&self) -> bool {
        self.cells.values().any(|e| match e {
            RegionEffect::Impassable => true,
            RegionEffect::Terrain(m) => *m > 1.0,
            RegionEffect::Arrest => false,
        })
    }

    /// Read-only iteration over the composed per-cell effects, existing for the parity
    /// assertion between this field and the `TriggerRegion` identity rows — comparing
    /// coverage sets is what the single-cell predicates above cannot express; never a
    /// mutation seam.
    #[cfg(test)]
    pub(crate) fn iter_cells(&self) -> impl Iterator<Item = (Cell, RegionEffect)> + '_ {
        self.cells.iter().map(|(c, e)| (*c, *e))
    }
}

/// Accumulates per-region contributions cell-by-cell; `build` composes them
/// (`compose`'s precedence + MAX-overlap rule) into the final `RegionField`.
#[derive(Default)]
pub(crate) struct RegionFieldBuilder {
    /// Raw `(behavior, cost)` contributions per cell, pre-composition.
    per_cell: BTreeMap<Cell, Vec<(RegionBehavior, f64)>>,
}

impl RegionFieldBuilder {
    /// Add one region's rasterized cells + behavior/cost. Skips (contributes nothing) on a
    /// fail-closed rasterization failure — never silently all-passes an over-cap/degenerate shape.
    pub(crate) fn add(
        &mut self,
        shape: &RegionShape,
        behavior: RegionBehavior,
        cost: f64,
        cell: f64,
        grid: &dyn crate::scene::grid_shape::GridShape,
    ) {
        let Some(cells) = rasterize(shape, cell, grid) else {
            return;
        };
        for c in cells {
            self.per_cell.entry(c).or_default().push((behavior, cost));
        }
    }

    /// Compose all accumulated contributions into the final per-cell field.
    pub(crate) fn build(self) -> RegionField {
        let mut cells = BTreeMap::new();
        for (c, contributions) in self.per_cell {
            if let Some(effect) = compose(&contributions) {
                cells.insert(c, effect);
            }
        }
        RegionField { cells }
    }
}

/// Parse a region doc's ingress-validated `engine.shape` (`data::engine::RegionShape` — a raw
/// `{kind, points}` pair, `deny_unknown_fields`-checked at write time) into this module's
/// `RegionShape` enum. Still structural-only past this function's own kind/point-count
/// dispatch: an
/// unrecognized `kind` or a `points` length that doesn't match the kind fails closed to `None`
/// (the caller then drops the region entirely, never half-parses it).
pub(crate) fn parse_region_shape(shape: &crate::data::engine::RegionShape) -> Option<RegionShape> {
    let points = &shape.points;
    match shape.kind.as_str() {
        "rect" if points.len() == 4 => Some(RegionShape::Rect {
            x0: points[0],
            y0: points[1],
            x1: points[2],
            y1: points[3],
        }),
        "circle" if points.len() == 3 => Some(RegionShape::Circle {
            cx: points[0],
            cy: points[1],
            r: points[2],
        }),
        "polygon" if points.len() >= 6 && points.len().is_multiple_of(2) => {
            let pts = points.chunks(2).map(|c| (c[0], c[1])).collect();
            Some(RegionShape::Polygon { points: pts })
        }
        _ => None,
    }
}

/// One trigger-bearing region's identity row: which cells it covers, which
/// triggers it carries, and whether every world member can see its `/engine`
/// band. Built by `SceneEcs::trigger_regions` from the SAME `rasterize` the
/// composed `RegionField` uses — one rasterizer feeds both consumers, so an
/// identity row and the composed field can never disagree about a region's
/// coverage (the parity battery pins this).
pub(crate) struct TriggerRegion {
    /// The region document's id.
    pub region_id: Uuid,
    /// World-default visibility of the region's `/engine` band
    /// (`engine_geometry_visible_to_world`). `false` forces every
    /// `ChatNotice` this region fires to `NoticeAudience::GmOnly`, since a
    /// public notice would name or imply a region some recipients cannot see.
    pub visible_to_all: bool,
    /// The region's authored triggers, in engine order.
    pub triggers: Vec<RegionTrigger>,
    /// The region's rasterized cells.
    pub cells: BTreeSet<Cell>,
}

/// The `(region, trigger)` pairs that fire for one move or placement report,
/// in region-table order then authored engine order. `entered` is the
/// deduplicated cell-entry sequence (`MoveOutcome::entered_cells`) or a
/// placement's footprint cells; `arrest_cell` is `Some` only when the walk
/// was arrested (`MoveOutcome::arrested`). Each listed trigger fires at most
/// once per report no matter how many of its region's cells were entered —
/// the caller applies each returned pair exactly once.
pub(crate) fn fired_triggers<'a>(
    regions: &'a [TriggerRegion],
    entered: &[Cell],
    arrest_cell: Option<Cell>,
) -> Vec<(&'a TriggerRegion, &'a RegionTrigger)> {
    let mut out = Vec::new();
    for region in regions {
        for trigger in &region.triggers {
            let fires = match trigger.on {
                TriggerEvent::Enter => entered.iter().any(|c| region.cells.contains(c)),
                TriggerEvent::Arrest => arrest_cell.is_some_and(|c| region.cells.contains(&c)),
            };
            if fires {
                out.push((region, trigger));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
