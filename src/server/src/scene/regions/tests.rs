use super::*;

fn sq(cell: f64) -> crate::scene::grid_shape::SquareGrid {
    crate::scene::grid_shape::SquareGrid {
        cell,
        rule: crate::scene::pathfinding::DiagonalRule::Chebyshev,
    }
}

#[test]
fn has_terrain_or_impassable_detects_weight_but_not_arrest() {
    let cell = 100.0;
    let rect = RegionShape::Rect {
        x0: 0.0,
        y0: 0.0,
        x1: 100.0,
        y1: 100.0,
    };

    let grid = sq(cell);

    let mut b = RegionField::builder();
    b.add(&rect, RegionBehavior::Terrain, 2.0, cell, &grid);
    assert!(
        b.build().has_terrain_or_impassable(),
        "terrain mult>1 counts"
    );

    let mut b = RegionField::builder();
    b.add(&rect, RegionBehavior::Impassable, 1.0, cell, &grid);
    assert!(b.build().has_terrain_or_impassable(), "impassable counts");

    let mut b = RegionField::builder();
    b.add(&rect, RegionBehavior::Arrest, 1.0, cell, &grid);
    assert!(
        !b.build().has_terrain_or_impassable(),
        "arrest alone does not count"
    );

    let mut b = RegionField::builder();
    b.add(&rect, RegionBehavior::Terrain, 1.0, cell, &grid);
    assert!(
        !b.build().has_terrain_or_impassable(),
        "terrain mult==1 is a no-op, does not count"
    );

    assert!(
        !RegionField::builder().build().has_terrain_or_impassable(),
        "empty field"
    );
}

#[test]
fn rasterize_routes_through_grid_shape_cell_center_not_hardcoded() {
    use crate::scene::grid_shape::SquareGrid;
    use crate::scene::pathfinding::DiagonalRule;
    let shape = RegionShape::Rect {
        x0: 0.0,
        y0: 0.0,
        x1: 250.0,
        y1: 250.0,
    };
    let grid_shape = SquareGrid {
        cell: 100.0,
        rule: DiagonalRule::Chebyshev,
    };
    let cells = rasterize(&shape, grid_shape.cell, &grid_shape).unwrap();
    assert!(cells.contains(&(0, 0)));
    assert!(cells.contains(&(1, 0)));
    assert!(cells.contains(&(0, 1)));
    assert!(cells.contains(&(1, 1)));
}

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
    let cells = rasterize(&shape, 100.0, &sq(100.0)).unwrap();
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
    let cells = rasterize(&shape, 100.0, &sq(100.0)).unwrap();
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
    let cells = rasterize(&shape, 100.0, &sq(100.0)).unwrap();
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
            100.0,
            &sq(100.0)
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
            100.0,
            &sq(100.0)
        ),
        None
    );
    assert_eq!(
        rasterize(
            &RegionShape::Polygon {
                points: vec![(0.0, 0.0), (1.0, 1.0)]
            },
            100.0,
            &sq(100.0)
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
    assert_eq!(rasterize(&shape, 100.0, &sq(100.0)), None);
}

#[test]
fn extreme_magnitude_coordinates_fail_closed_not_overflow() {
    let shape = RegionShape::Rect {
        x0: -1e300,
        y0: 0.0,
        x1: 0.0,
        y1: 0.0,
    };
    assert_eq!(
        rasterize(&shape, 100.0, &sq(100.0)),
        None,
        "extreme AABB extent must fail closed, not overflow/hang"
    );

    let shape2 = RegionShape::Rect {
        x0: 1e13,
        y0: 0.0,
        x1: 1e13 + 1000.0,
        y1: 100.0,
    };
    assert_eq!(
        rasterize(&shape2, 100.0, &sq(100.0)),
        None,
        "large-magnitude-but-small-span coords must fail closed, not truncate/alias"
    );
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
    let grid = sq(100.0);
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
        &grid,
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
        &grid,
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
        &sq(100.0),
    );
    let field = b.build();
    assert!(
        !field.is_impassable((0, 0)),
        "a degenerate shape contributes nothing, never all-passes"
    );
}

fn eng_shape(kind: &str, points: Vec<f64>) -> crate::data::engine::RegionShape {
    crate::data::engine::RegionShape {
        kind: kind.to_string(),
        points,
    }
}

#[test]
fn parse_region_shape_rect_circle_polygon() {
    assert_eq!(
        parse_region_shape(&eng_shape("rect", vec![0.0, 0.0, 100.0, 100.0])),
        Some(RegionShape::Rect {
            x0: 0.0,
            y0: 0.0,
            x1: 100.0,
            y1: 100.0
        })
    );
    assert_eq!(
        parse_region_shape(&eng_shape("circle", vec![50.0, 50.0, 25.0])),
        Some(RegionShape::Circle {
            cx: 50.0,
            cy: 50.0,
            r: 25.0
        })
    );
    assert_eq!(
        parse_region_shape(&eng_shape(
            "polygon",
            vec![0.0, 0.0, 100.0, 0.0, 0.0, 100.0]
        )),
        Some(RegionShape::Polygon {
            points: vec![(0.0, 0.0), (100.0, 0.0), (0.0, 100.0)]
        })
    );
}

// A raw `system`-body shape with a missing `/shape` key or a non-numeric point entry cannot
// reach this function at all: `parse_region_shape` takes the typed, already
// ingress-validated `engine::RegionShape { kind: String, points: Vec<f64> }` directly (a
// document carrying either defect fails ingress validation before persistence, so no caller
// of this function can ever hand it one). Only the two structural checks internal to this
// function's own dispatch — an unrecognized `kind`, or a `points` length that doesn't match
// the kind — remain reachable.
#[test]
fn parse_region_shape_malformed_is_none() {
    assert_eq!(
        parse_region_shape(&eng_shape("rect", vec![1.0, 2.0])),
        None,
        "rect requires exactly 4 points"
    );
    assert_eq!(
        parse_region_shape(&eng_shape("hexagon", vec![])),
        None,
        "unknown kind"
    );
}
