//! Region primitive (M10g): vector-shaped zones that weight, block, or arrest grid movement.
//! Pure geometry — no ECS, no I/O (mirrors `scene/movement.rs`'s module invariant). Consumed by
//! `SceneEcs::region_field` (hydration + visibility filtering) and `scene::pathfinding` /
//! `scene::move_exec` (the two enforcement points, spec §5/§6).

use std::collections::BTreeMap;

pub(crate) type Cell = (i32, i32);

/// Authored region geometry (M8d-3a vector-shape vocabulary: rect/circle/polygon).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RegionShape {
    Rect { x0: f64, y0: f64, x1: f64, y1: f64 },
    Circle { cx: f64, cy: f64, r: f64 },
    Polygon { points: Vec<(f64, f64)> },
}

/// The region's gameplay effect (spec §2.1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RegionBehavior {
    Terrain,
    Impassable,
    Arrest,
}

/// Per-cell composed effect after precedence + MAX overlap resolution (spec §2.4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RegionEffect {
    Impassable,
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
pub(crate) fn rasterize(shape: &RegionShape, cell: f64) -> Option<Vec<Cell>> {
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
    let i0 = (minx / cell).floor() as i64;
    let i1 = (maxx / cell).floor() as i64;
    let j0 = (miny / cell).floor() as i64;
    let j1 = (maxy / cell).floor() as i64;
    if i1 < i0 || j1 < j0 {
        return None;
    }
    let count = (i1 - i0 + 1).checked_mul(j1 - j0 + 1)?;
    if count > MAX_REGION_CELLS {
        return None;
    }
    let mut out = Vec::new();
    for i in i0..=i1 {
        for j in j0..=j1 {
            let ctr = ((i as f64 + 0.5) * cell, (j as f64 + 0.5) * cell);
            if cell_center_in_shape(ctr, shape) {
                out.push((i as i32, j as i32));
            }
        }
    }
    Some(out)
}

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
/// public domain reference implementation).
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

/// Compose zero or more (behavior, cost) contributions for ONE cell into a single effect, per
/// spec §2.4: precedence `Impassable > Arrest > Terrain`; overlapping Terrain costs take the MAX
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
    cells: BTreeMap<Cell, RegionEffect>,
}

impl RegionField {
    pub(crate) fn builder() -> RegionFieldBuilder {
        RegionFieldBuilder::default()
    }

    pub(crate) fn is_impassable(&self, c: Cell) -> bool {
        matches!(self.cells.get(&c), Some(RegionEffect::Impassable))
    }

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
}

#[derive(Default)]
pub(crate) struct RegionFieldBuilder {
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
    ) {
        let Some(cells) = rasterize(shape, cell) else {
            return;
        };
        for c in cells {
            self.per_cell.entry(c).or_default().push((behavior, cost));
        }
    }

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

/// Parse a region doc's `system` body (client-owned shape, spec §3 / scene-docs.ts
/// `RegionSystem`) into `RegionShape`. Structural-only: any malformed/missing field fails
/// closed to `None` (the caller then drops the region entirely, never half-parses it).
pub(crate) fn parse_region_shape(system: &serde_json::Value) -> Option<RegionShape> {
    let kind = system.pointer("/shape/kind")?.as_str()?;
    let points: Vec<f64> = system
        .pointer("/shape/points")?
        .as_array()?
        .iter()
        .filter_map(|v| v.as_f64())
        .collect();
    match kind {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_rasterizes_covered_cell_centers() {
        // Rect [0,0]-[250,150] at cell=100: covers columns 0,1,2 and rows 0,1 whose centers fall
        // inside — center of col 2 is x=250, which is exactly on the boundary (>= maxx check
        // includes it since inclusive comparisons are used).
        let shape = RegionShape::Rect {
            x0: 0.0,
            y0: 0.0,
            x1: 250.0,
            y1: 150.0,
        };
        let cells = rasterize(&shape, 100.0).unwrap();
        assert!(cells.contains(&(0, 0)));
        assert!(cells.contains(&(1, 1)));
        assert!(
            !cells.contains(&(3, 0)),
            "column 3 center (350) is outside x1=250"
        );
    }

    #[test]
    fn circle_rasterizes_cells_whose_center_is_within_radius() {
        let shape = RegionShape::Circle {
            cx: 150.0,
            cy: 150.0,
            r: 60.0,
        };
        let cells = rasterize(&shape, 100.0).unwrap();
        assert!(
            cells.contains(&(1, 1)),
            "center cell (150,150) is the circle's own center"
        );
        assert!(
            !cells.contains(&(0, 0)),
            "cell (0,0) center (50,50) is > 60 from (150,150)"
        );
    }

    #[test]
    fn polygon_rasterizes_via_point_in_polygon() {
        // A right triangle (0,0)-(300,0)-(0,300): cell (0,0) center (50,50) is inside;
        // cell (2,2) center (250,250) is outside (past the hypotenuse).
        let shape = RegionShape::Polygon {
            points: vec![(0.0, 0.0), (300.0, 0.0), (0.0, 300.0)],
        };
        let cells = rasterize(&shape, 100.0).unwrap();
        assert!(cells.contains(&(0, 0)));
        assert!(!cells.contains(&(2, 2)));
    }

    #[test]
    fn degenerate_shapes_fail_closed() {
        assert_eq!(
            rasterize(
                &RegionShape::Circle {
                    cx: 0.0,
                    cy: 0.0,
                    r: 0.0
                },
                100.0
            ),
            None
        );
        assert_eq!(
            rasterize(
                &RegionShape::Circle {
                    cx: 0.0,
                    cy: 0.0,
                    r: f64::NAN
                },
                100.0
            ),
            None
        );
        assert_eq!(
            rasterize(
                &RegionShape::Polygon {
                    points: vec![(0.0, 0.0), (1.0, 1.0)]
                },
                100.0
            ),
            None,
            "fewer than 3 vertices is degenerate"
        );
    }

    #[test]
    fn oversized_aabb_fails_closed() {
        let shape = RegionShape::Rect {
            x0: 0.0,
            y0: 0.0,
            x1: 1e12,
            y1: 1e12,
        };
        assert_eq!(rasterize(&shape, 100.0), None);
    }

    #[test]
    fn compose_precedence_impassable_beats_arrest_beats_terrain() {
        assert_eq!(
            compose(&[
                (RegionBehavior::Terrain, 2.0),
                (RegionBehavior::Impassable, 1.0)
            ]),
            Some(RegionEffect::Impassable)
        );
        assert_eq!(
            compose(&[
                (RegionBehavior::Terrain, 2.0),
                (RegionBehavior::Arrest, 1.0)
            ]),
            Some(RegionEffect::Arrest)
        );
    }

    #[test]
    fn compose_terrain_overlap_takes_max_not_sum() {
        assert_eq!(
            compose(&[
                (RegionBehavior::Terrain, 2.0),
                (RegionBehavior::Terrain, 3.0)
            ]),
            Some(RegionEffect::Terrain(3.0))
        );
    }

    #[test]
    fn compose_empty_is_none() {
        assert_eq!(compose(&[]), None);
    }

    #[test]
    fn region_field_builder_composes_across_overlapping_regions() {
        let mut b = RegionField::builder();
        b.add(
            &RegionShape::Rect {
                x0: 0.0,
                y0: 0.0,
                x1: 100.0,
                y1: 100.0,
            },
            RegionBehavior::Terrain,
            2.0,
            100.0,
        );
        b.add(
            &RegionShape::Rect {
                x0: 0.0,
                y0: 0.0,
                x1: 100.0,
                y1: 100.0,
            },
            RegionBehavior::Terrain,
            3.0,
            100.0,
        );
        let field = b.build();
        assert_eq!(
            field.terrain_multiplier((0, 0)),
            3.0,
            "overlapping terrain takes MAX"
        );
        assert_eq!(
            field.terrain_multiplier((5, 5)),
            1.0,
            "uncovered cell is unweighted"
        );
        assert!(!field.is_impassable((0, 0)));
        assert!(!field.is_arrest((0, 0)));
    }

    #[test]
    fn region_field_builder_drops_a_fail_closed_shape_silently() {
        let mut b = RegionField::builder();
        b.add(
            &RegionShape::Circle {
                cx: 0.0,
                cy: 0.0,
                r: -1.0,
            },
            RegionBehavior::Impassable,
            1.0,
            100.0,
        );
        let field = b.build();
        assert!(
            !field.is_impassable((0, 0)),
            "a degenerate shape contributes nothing, never all-passes"
        );
    }

    #[test]
    fn parse_region_shape_rect_circle_polygon() {
        assert_eq!(
            parse_region_shape(
                &serde_json::json!({"shape": {"kind": "rect", "points": [0.0, 0.0, 100.0, 100.0]}})
            ),
            Some(RegionShape::Rect {
                x0: 0.0,
                y0: 0.0,
                x1: 100.0,
                y1: 100.0
            })
        );
        assert_eq!(
            parse_region_shape(
                &serde_json::json!({"shape": {"kind": "circle", "points": [50.0, 50.0, 25.0]}})
            ),
            Some(RegionShape::Circle {
                cx: 50.0,
                cy: 50.0,
                r: 25.0
            })
        );
        assert_eq!(
            parse_region_shape(
                &serde_json::json!({"shape": {"kind": "polygon", "points": [0.0,0.0, 100.0,0.0, 0.0,100.0]}})
            ),
            Some(RegionShape::Polygon {
                points: vec![(0.0, 0.0), (100.0, 0.0), (0.0, 100.0)]
            })
        );
    }

    #[test]
    fn parse_region_shape_malformed_is_none() {
        assert_eq!(parse_region_shape(&serde_json::json!({})), None);
        assert_eq!(
            parse_region_shape(
                &serde_json::json!({"shape": {"kind": "rect", "points": [1.0, 2.0]}})
            ),
            None,
            "rect requires exactly 4 points"
        );
        assert_eq!(
            parse_region_shape(&serde_json::json!({"shape": {"kind": "hexagon", "points": []}})),
            None,
            "unknown kind"
        );
    }
}
