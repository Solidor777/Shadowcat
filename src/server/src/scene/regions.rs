//! Region primitive: vector-shaped zones that weight, block, or arrest grid movement, or edit their
//! cells' ambient light.
//!
//! Pure geometry — no ECS, no I/O (mirrors the `scene::movement` module's invariant). Consumed by
//! `SceneEcs::region_field` (hydration + visibility filtering), `scene::pathfinding` /
//! `scene::move_exec` (the two movement enforcement points), and `LightingInputs::cell_light` (the
//! ambient-light adjustment, read through the field's `ambient` facet only — never the movement
//! one).

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::{BTreeMap, BTreeSet};

use uuid::Uuid;

use crate::data::engine::{AmbientLightMode, AmbientLightOverride, RegionTrigger, TriggerEvent};

/// A grid cell `(i, j)` (same convention as `pathfinding::Cell`).
pub(crate) type Cell = (i32, i32);

/// One raw `(behavior, cost, cost_by_tag)` contribution to a cell's composition, pre-fold.
/// `cost_by_tag` is meaningful only for `RegionBehavior::Terrain` and empty for any other
/// behavior or an untagged terrain region.
type RegionContribution = (RegionBehavior, f64, BTreeMap<String, f64>);

/// One terrain region's `(flat cost, cost_by_tag)` pair, as recorded in
/// `RegionField::tag_cost` for a cell where at least one overlapping terrain region authors a
/// non-empty `cost_by_tag`.
type TerrainTagCost = (f64, BTreeMap<String, f64>);

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
    /// Ambient-light edit: adjusts the covered cells' composed illumination
    /// (`compose_ambient` → `LightingInputs::cell_light`'s final per-cell
    /// adjustment). Movement-only readers (`compose`, `terrain_multiplier`,
    /// `is_impassable`, `is_arrest`) never see this variant — it carries no
    /// movement effect.
    AmbientLight(AmbientLightOverride),
}

/// The per-cell composed ambient-light adjustment, folded from every
/// overlapping enabled `ambient_light` region by `compose_ambient` and applied
/// LAST over `LightingInputs::cell_light`'s base composition (placed/carried
/// lights plus the environment ambient — under `all_bright` included, where
/// the base is the flat maximum).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum AmbientAdjustment {
    /// Replace the composed illumination outright (an `Override` region; the
    /// darker of two overlapping `Override`s).
    Override(f64),
    /// Net signed delta over the composed base (`Darken` negative,
    /// `Brighten` positive) — overlapping deltas SUM, never take a max:
    /// darken/brighten are additive perturbations to a baseline, not a
    /// ceiling on it, so a second overlapping darken must genuinely deepen
    /// the dark.
    Delta(f64),
}

impl AmbientAdjustment {
    /// Apply the adjustment to a composed base illumination, clamped to the
    /// `[0, 1]` illumination scale. `Override` clamps its own level read-side
    /// (ingress validation already enforces `[0, 1]`; the clamp here keeps a
    /// fixture or a pre-validation stored document inside the scale).
    pub(crate) fn apply(&self, base: f64) -> f64 {
        match *self {
            AmbientAdjustment::Override(level) => level.clamp(0.0, 1.0),
            AmbientAdjustment::Delta(delta) => (base + delta).clamp(0.0, 1.0),
        }
    }
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

/// Rasterize a region's normalized `(shape, hole)` part list into the COMPOSED cell set: the
/// union of every additive (`hole: false`) part's cells, minus every hole part's cells. This is
/// the one composition rule both region consumers read — `RegionFieldBuilder::add_parts` (the
/// composed `RegionField`) and `SceneEcs::build_trigger_region` (the trigger identity rows) —
/// so a token standing in a hole counts as outside the region for movement AND for triggers
/// alike. Each part rasterizes through `rasterize` itself, so every part's own AABB is
/// individually capped by the existing `MAX_REGION_CELLS` guard (bounding the WORK done before
/// subtraction), and ANY part failing closed fails the WHOLE region to `None` — never a
/// partially-composed set (a silently-dropped hole could silently make an impassable region
/// passable in a spot the GM thought was excluded). The FINAL composed count is additionally
/// capped at `MAX_REGION_CELLS` POST-subtraction: many individually-in-cap additive parts must
/// not sum past the per-region bound (a huge additive shape with a huge hole that nets a small
/// final area is NOT rejected — the cap applies to the result, not to any intermediate).
/// Cells keep first-seen order across the additive parts (a single additive part therefore
/// yields byte-identical output to `rasterize` alone).
pub(crate) fn rasterize_compound(
    parts: &[(RegionShape, bool)],
    cell: f64,
    grid: &dyn crate::scene::grid_shape::GridShape,
) -> Option<Vec<Cell>> {
    let mut union: Vec<Cell> = Vec::new();
    let mut membership: BTreeSet<Cell> = BTreeSet::new();
    for (shape, hole) in parts {
        if *hole {
            continue;
        }
        for c in rasterize(shape, cell, grid)? {
            if membership.insert(c) {
                union.push(c);
            }
        }
    }
    for (shape, hole) in parts {
        if !*hole {
            continue;
        }
        for c in rasterize(shape, cell, grid)? {
            membership.remove(&c);
        }
    }
    union.retain(|c| membership.contains(c));
    if union.len() as i64 > MAX_REGION_CELLS {
        return None;
    }
    Some(union)
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
/// `AmbientLight` contributions produce NO movement effect here — the ambient facet of the
/// same cell is `compose_ambient`'s, and a cell may carry both (a darkened difficult-terrain
/// cell is a coherent authoring); the two facets are folded from the SAME contribution list
/// by `RegionFieldBuilder::build`.
pub(crate) fn compose(contributions: &[RegionContribution]) -> Option<RegionEffect> {
    if contributions
        .iter()
        .any(|(b, _, _)| *b == RegionBehavior::Impassable)
    {
        return Some(RegionEffect::Impassable);
    }
    if contributions
        .iter()
        .any(|(b, _, _)| *b == RegionBehavior::Arrest)
    {
        return Some(RegionEffect::Arrest);
    }
    contributions
        .iter()
        .filter(|(b, _, _)| *b == RegionBehavior::Terrain)
        .map(|(_, cost, _)| *cost)
        .fold(None, |acc: Option<f64>, c| {
            Some(acc.map_or(c, |a| a.max(c)))
        })
        .map(RegionEffect::Terrain)
}

/// The ambient facet of one cell's composition: fold every `AmbientLight` contribution into the
/// ONE adjustment `LightingInputs::cell_light` applies after its base composition. Precedence:
/// any `Override` wins outright over `Darken`/`Brighten` (matching `Impassable > Arrest >
/// Terrain`'s "strongest wins" shape), two `Override`s resolve to the DARKER level (the
/// fail-closed under-reveal convention this subsystem applies to every ambiguous case), and
/// overlapping `Darken`/`Brighten` deltas SUM. `None` when no ambient-light region covers the
/// cell (or the deltas net to exactly zero — identical to no adjustment, kept out of the field
/// map so the `cell_light` fast path stays a plain emptiness test). Read-side fail-closed
/// guards (ingress `RegionEngine::validate` already rejects them, a stored/fixture value is
/// still not trusted): a non-finite level contributes nothing, a negative `Darken`/`Brighten`
/// level clamps to 0, and an `Override` level clamps into `[0, 1]` at `AmbientAdjustment::apply`.
pub(crate) fn compose_ambient(contributions: &[RegionContribution]) -> Option<AmbientAdjustment> {
    let mut override_level: Option<f64> = None;
    let mut delta = 0.0_f64;
    for (behavior, _, _) in contributions {
        let RegionBehavior::AmbientLight(ambient) = behavior else {
            continue;
        };
        if !ambient.level.is_finite() {
            continue;
        }
        match ambient.mode {
            AmbientLightMode::Override => {
                override_level =
                    Some(override_level.map_or(ambient.level, |acc: f64| acc.min(ambient.level)));
            }
            AmbientLightMode::Darken => delta -= ambient.level.max(0.0),
            AmbientLightMode::Brighten => delta += ambient.level.max(0.0),
        }
    }
    if let Some(level) = override_level {
        return Some(AmbientAdjustment::Override(level));
    }
    if delta != 0.0 {
        return Some(AmbientAdjustment::Delta(delta));
    }
    None
}

/// The composed per-cell region effect for one scene, already resolved to either a single
/// requester's visibility (the router's per-requester field) or the full authoritative set
/// (the GM's field / `move_exec`'s field — and `LightingInputs::cell_light`'s, whose ambient
/// adjustment ALWAYS reads the authoritative field, so a `gm_only` region's light edit applies
/// to every viewer). Built by `RegionFieldBuilder`.
#[derive(Debug, Default, Clone)]
pub(crate) struct RegionField {
    /// Composed movement effect per covered cell; absent cell = no movement effect.
    cells: BTreeMap<Cell, RegionEffect>,
    /// Composed ambient-light adjustment per covered cell (`compose_ambient`); absent cell =
    /// no ambient edit. Orthogonal to `cells` — a cell may carry a movement effect, an ambient
    /// adjustment, both, or neither.
    ambient: BTreeMap<Cell, AmbientAdjustment>,
    /// Per-cell terrain contributions carrying a per-tag cost override: `(flat cost,
    /// cost_by_tag)` for every terrain region overlapping the cell, present ONLY for a cell
    /// where at least one overlapping terrain region authors a non-empty `cost_by_tag` — most
    /// scenes author none, so this map is empty in the common case. Every terrain contribution
    /// for such a cell is recorded (not just the tag-bearing ones), because
    /// `resolved_terrain_multiplier` must fold ALL of them through the same overlapping-
    /// regions-take-the-MAX rule `cells`' flat-only composition already applies. Read only by
    /// `resolved_terrain_multiplier`; `cells`/`terrain_multiplier` stay flat-cost-only and
    /// unaware of tags, so a caller with no movement tags to resolve can keep using the cheap
    /// flat path.
    tag_cost: BTreeMap<Cell, Vec<TerrainTagCost>>,
}

impl RegionField {
    /// An empty accumulating builder.
    ///
    /// # Examples
    ///
    /// ```text
    /// let mut b = RegionField::builder();
    /// b.add_parts(&[(shape, false)], RegionBehavior::Terrain, 2.0, &BTreeMap::new(), cell, grid);
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

    /// The effective terrain multiplier for a mover carrying `tags` entering cell `c`. When `c`
    /// carries no `cost_by_tag`-bearing terrain contribution (`tag_cost.get(&c)` is `None` — the
    /// common case), identical to `terrain_multiplier` (the cheap flat-only path). Otherwise,
    /// per contributing terrain region: resolves the MINIMUM `cost_by_tag` value whose key the
    /// mover's tags contain, falling back to that region's own flat cost when no tag matches
    /// ("you navigate however is easiest for you" — a mover with several qualifying tags takes
    /// whichever tag's terrain description favors them most). The per-region resolved values
    /// then fold through the SAME overlapping-regions-take-the-MAX rule `compose` applies to
    /// flat costs — never a second precedence rule, so a cell covered by both a plain
    /// (untagged) terrain region and a tag-costed one still composes correctly: the plain
    /// region's flat cost is one of the folded values, exactly as if it authored an empty
    /// `cost_by_tag`.
    pub(crate) fn resolved_terrain_multiplier(&self, c: Cell, tags: &BTreeSet<String>) -> f64 {
        let Some(contributions) = self.tag_cost.get(&c) else {
            return self.terrain_multiplier(c);
        };
        contributions
            .iter()
            .map(|(flat, by_tag)| {
                by_tag
                    .iter()
                    .filter(|(tag, _)| tags.contains(*tag))
                    .map(|(_, cost)| *cost)
                    .fold(None::<f64>, |acc, v| Some(acc.map_or(v, |a| a.min(v))))
                    .unwrap_or(*flat)
            })
            .fold(None::<f64>, |acc, m| Some(acc.map_or(m, |a| a.max(m))))
            .unwrap_or(1.0)
    }

    /// The composed ambient-light adjustment covering `c`; `None` when no `ambient_light`
    /// region covers it. Test-only: production reads the map through `into_ambient_map` (the
    /// lighting-inputs build), never per cell.
    #[cfg(test)]
    pub(crate) fn ambient_at(&self, c: Cell) -> Option<AmbientAdjustment> {
        self.ambient.get(&c).copied()
    }

    /// Consume the field into JUST its ambient facet (the movement facet dropped) — the shape
    /// `SceneEcs::ambient_light_map` hands the lighting-inputs build, which never reads a
    /// movement effect.
    pub(crate) fn into_ambient_map(self) -> BTreeMap<Cell, AmbientAdjustment> {
        self.ambient
    }

    /// True iff any cell composes to `Impassable`. Split from
    /// `has_terrain_or_impassable` because the continuous router's dispatch predicate reads the
    /// two halves differently for an `MoveTraits::ignore_terrain` mover: impassable still forces
    /// the weighted sub-path (the exemption is terrain-COST only — impassable blocks everyone),
    /// while weighted terrain does not.
    pub(crate) fn has_impassable(&self) -> bool {
        self.cells
            .values()
            .any(|e| matches!(e, RegionEffect::Impassable))
    }

    /// True iff any cell is weighted `Terrain` for a mover carrying `tags` (the tag-resolved
    /// multiplier, via `resolved_terrain_multiplier` — never the flat-only reading, or a
    /// scene where every terrain region's flat cost is 1.0 but a `cost_by_tag` entry weights a
    /// tagged mover would wrongly read as unweighted).
    pub(crate) fn has_weighted_terrain(&self, tags: &BTreeSet<String>) -> bool {
        self.cells.iter().any(|(c, e)| {
            matches!(e, RegionEffect::Terrain(_))
                && self.resolved_terrain_multiplier(*c, tags) > 1.0
        })
    }

    /// True iff any cell is `Impassable` or weighted `Terrain` (tag-resolved, per `tags`) for
    /// a mover carrying `tags`. The dispatch predicate the continuous router uses to decide
    /// between the weighted grid route (terrain/impassable present) and the pure any-angle
    /// polyanya route (neither) for a NON-exempt mover. Arrest is excluded: it neither bends
    /// the route nor requires route-around, so an arrest-only scene stays on the polyanya path
    /// with an arrest post-filter.
    pub(crate) fn has_terrain_or_impassable(&self, tags: &BTreeSet<String>) -> bool {
        self.has_impassable() || self.has_weighted_terrain(tags)
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
/// (`compose`'s movement precedence + MAX-overlap rule, plus `compose_ambient`'s ambient
/// fold) into the final `RegionField`.
#[derive(Default)]
pub(crate) struct RegionFieldBuilder {
    /// Raw `(behavior, cost, cost_by_tag)` contributions per cell, pre-composition.
    /// `cost_by_tag` is empty for every non-`Terrain` behavior and for a `Terrain`
    /// contribution that authors no per-tag override.
    per_cell: BTreeMap<Cell, Vec<RegionContribution>>,
}

impl RegionFieldBuilder {
    /// Add one region's rasterized cells + behavior/cost. Skips (contributes nothing) on a
    /// fail-closed rasterization failure — never silently all-passes an over-cap/degenerate shape.
    /// The single-shape convenience spelling of `add_parts` (a legacy one-part region), kept so
    /// every additive contribution routes through the ONE composition rule. Production callers
    /// always feed the normalized part list from `parse_region_shape`, so this spelling exists
    /// for the single-shape fixtures only. Carries no `cost_by_tag` (an empty map) — a fixture
    /// exercising per-tag cost calls `add_parts` directly.
    #[cfg(test)]
    pub(crate) fn add(
        &mut self,
        shape: &RegionShape,
        behavior: RegionBehavior,
        cost: f64,
        cell: f64,
        grid: &dyn crate::scene::grid_shape::GridShape,
    ) {
        self.add_parts(
            &[(shape.clone(), false)],
            behavior,
            cost,
            &BTreeMap::new(),
            cell,
            grid,
        );
    }

    /// Add one compound region's composed cells + behavior/cost/cost_by_tag: the union of its
    /// additive parts minus its hole parts, through `rasterize_compound`. Skips (contributes
    /// nothing) when the composition fails closed — never silently all-passes a region one of
    /// whose parts is over-cap or degenerate. `cost_by_tag` is meaningful only for
    /// `RegionBehavior::Terrain`; a non-empty map on any other behavior is carried but never
    /// read (`compose`/`compose_ambient` ignore it, and `build` only reads it for `Terrain`
    /// contributions).
    pub(crate) fn add_parts(
        &mut self,
        parts: &[(RegionShape, bool)],
        behavior: RegionBehavior,
        cost: f64,
        cost_by_tag: &BTreeMap<String, f64>,
        cell: f64,
        grid: &dyn crate::scene::grid_shape::GridShape,
    ) {
        let Some(cells) = rasterize_compound(parts, cell, grid) else {
            return;
        };
        for c in cells {
            self.per_cell
                .entry(c)
                .or_default()
                .push((behavior, cost, cost_by_tag.clone()));
        }
    }

    /// Compose all accumulated contributions into the final per-cell field: the movement facet
    /// (`compose`) and the ambient-light facet (`compose_ambient`) fold the SAME per-cell
    /// contribution list, so the two output maps can never disagree about which regions covered
    /// a cell. A cell that composes to `Terrain` additionally populates `tag_cost` when at
    /// least one of its terrain contributions authored a non-empty `cost_by_tag` — recording
    /// EVERY terrain contribution at that cell (not just the tag-bearing ones), since
    /// `resolved_terrain_multiplier` must fold all of them through the same MAX rule `compose`
    /// already applied to their flat costs.
    pub(crate) fn build(self) -> RegionField {
        let mut cells = BTreeMap::new();
        let mut ambient = BTreeMap::new();
        let mut tag_cost = BTreeMap::new();
        for (c, contributions) in self.per_cell {
            if let Some(effect) = compose(&contributions) {
                if matches!(effect, RegionEffect::Terrain(_)) {
                    let terrain: Vec<TerrainTagCost> = contributions
                        .iter()
                        .filter(|(b, _, _)| matches!(b, RegionBehavior::Terrain))
                        .map(|(_, cost, by_tag)| (*cost, by_tag.clone()))
                        .collect();
                    if terrain.iter().any(|(_, by_tag)| !by_tag.is_empty()) {
                        tag_cost.insert(c, terrain);
                    }
                }
                cells.insert(c, effect);
            }
            if let Some(adjustment) = compose_ambient(&contributions) {
                ambient.insert(c, adjustment);
            }
        }
        RegionField {
            cells,
            ambient,
            tag_cost,
        }
    }
}

/// Parse a region doc's ingress-validated `engine.shape` (`data::engine::RegionGeometry` — the
/// untagged legacy/compound union, whose hand-written `Deserialize` routes each wire form
/// through its inner struct's own `deny_unknown_fields` at write time) into
/// the normalized `(shape, hole)` part list every downstream consumer reads. A legacy
/// `RegionGeometry::Single` normalizes to a one-element additive list the moment parsing
/// completes, so `rasterize_compound`, the composed `RegionField` and the trigger identity rows
/// never branch on which wire form was stored. Fails closed to `None` for the WHOLE region when
/// ANY part fails to parse (an unrecognized `kind` or a point count that doesn't match it) or a
/// compound list is empty — a partially-valid compound region is NOT partially rasterized, since
/// a silently-dropped hole could silently make an impassable region passable in a spot the GM
/// thought was excluded, which is far worse than dropping the whole region.
pub(crate) fn parse_region_shape(
    shape: &crate::data::engine::RegionGeometry,
) -> Option<Vec<(RegionShape, bool)>> {
    match shape {
        crate::data::engine::RegionGeometry::Single(s) => Some(vec![(parse_one_shape(s)?, false)]),
        crate::data::engine::RegionGeometry::Compound(parts) => {
            if parts.is_empty() {
                return None;
            }
            let mut out = Vec::with_capacity(parts.len());
            for part in parts {
                out.push((parse_one_shape(&part.shape)?, part.hole));
            }
            Some(out)
        }
    }
}

/// Parse ONE wire `data::engine::RegionShape` (a raw `{kind, points}` pair) into this module's
/// `RegionShape` enum. Still structural-only past this function's own kind/point-count
/// dispatch: an
/// unrecognized `kind` or a `points` length that doesn't match the kind fails closed to `None`
/// (the caller then drops the region entirely, never half-parses it).
fn parse_one_shape(shape: &crate::data::engine::RegionShape) -> Option<RegionShape> {
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
/// band. Built by `SceneEcs::trigger_regions` from the SAME `rasterize_compound` the
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

/// The `(region id, trigger)` pairs that fire for one move or placement
/// report, in region-table order then authored engine order. `entered` is the
/// deduplicated cell-entry sequence (`MoveOutcome::entered_cells`) or a
/// placement's footprint cells; `arrest_cell` is `Some` only when the walk
/// was arrested (`MoveOutcome::arrested`); `exited` is the region-identity
/// list the report's membership diff produced (`SceneEcs::region_membership`'s
/// stored set minus the regions intersecting the report's post-event
/// OCCUPANCY — `TriggerReport::occupancy`, the report kind's own Enter basis:
/// a move's stop center cell, a placement's footprint — a set-difference
/// event over whole regions, never a per-cell crossing like Enter/Arrest).
/// Each listed trigger fires at most once per report no matter how many of
/// its region's cells were entered, and an exited region id fires its
/// region's `Exit` triggers at most once — the same per-report discipline.
/// An exited id no row of `regions` carries (a deleted or disabled region, or
/// one the report's elevation filter excludes) fires nothing: the table has
/// already forgotten it. The pairs are owned (not row references) so a caller
/// firing exits for regions OUTSIDE the passed table can concatenate a second
/// call's plan without a lifetime tie — see `Room::fire_region_triggers`.
/// The four turn/round variants never fire from a movement report: they are
/// combat-clock events, planned by `fired_turn_round_triggers` from
/// `combat::transition`'s boundary record instead.
pub(crate) fn fired_triggers(
    regions: &[TriggerRegion],
    entered: &[Cell],
    arrest_cell: Option<Cell>,
    exited: &[Uuid],
) -> Vec<(Uuid, RegionTrigger)> {
    let mut out = Vec::new();
    for region in regions {
        for trigger in &region.triggers {
            let fires = match trigger.on {
                TriggerEvent::Enter => entered.iter().any(|c| region.cells.contains(c)),
                TriggerEvent::Arrest => arrest_cell.is_some_and(|c| region.cells.contains(&c)),
                TriggerEvent::Exit => exited.contains(&region.region_id),
                // Combat-clock events; a move/placement report never crosses one.
                TriggerEvent::TurnStart
                | TriggerEvent::TurnEnd
                | TriggerEvent::RoundStart
                | TriggerEvent::RoundEnd => false,
            };
            if fires {
                out.push((region.region_id, trigger.clone()));
            }
        }
    }
    out
}

/// The `(region id, trigger index, trigger)` triples that fire for ONE token
/// at ONE combat turn/round boundary: every trigger on a region the token is
/// a recorded member of (`SceneEcs::region_membership` — the occupancy proof,
/// written only by the movement/placement funnel; a clock boundary never
/// re-derives it) whose `on` is exactly `event`. `rows` are the identity
/// rows for the member regions, resolved by id WITHOUT the per-scene
/// elevation filter (`SceneEcs::trigger_region_row` — the same rule Exit
/// firing uses: membership is the occupancy proof, so re-filtering by the
/// token's CURRENT floor or scene would wrongly drop a region the token was
/// legitimately recorded inside). A member id with no live row (a deleted or
/// disabled region) fires nothing, mirroring `fired_triggers`'s exited-id
/// rule. The returned trigger INDEX is the trigger's position in its
/// region's authored `triggers` list — the trigger-identity component of
/// `Room::fire_turn_round_triggers`'s once-per-turn/round dedup key, which
/// content-equality cannot supply (two identical authored triggers on one
/// region are two distinct identities: a `resource_delta` authored twice
/// applies twice). Each matching trigger fires at most once per call — one
/// authored trigger, one firing — the per-report discipline
/// `fired_triggers` already keeps.
pub(crate) fn fired_turn_round_triggers(
    rows: &[TriggerRegion],
    member: &std::collections::BTreeSet<Uuid>,
    event: TriggerEvent,
) -> Vec<(Uuid, usize, RegionTrigger)> {
    let mut out = Vec::new();
    for row in rows {
        if !member.contains(&row.region_id) {
            continue;
        }
        for (index, trigger) in row.triggers.iter().enumerate() {
            if trigger.on == event {
                out.push((row.region_id, index, trigger.clone()));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
