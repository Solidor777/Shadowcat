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
        b.build()
            .has_terrain_or_impassable(&std::collections::BTreeSet::new()),
        "terrain mult>1 counts"
    );

    let mut b = RegionField::builder();
    b.add(&rect, RegionBehavior::Impassable, 1.0, cell, &grid);
    assert!(
        b.build()
            .has_terrain_or_impassable(&std::collections::BTreeSet::new()),
        "impassable counts"
    );

    let mut b = RegionField::builder();
    b.add(&rect, RegionBehavior::Arrest, 1.0, cell, &grid);
    assert!(
        !b.build()
            .has_terrain_or_impassable(&std::collections::BTreeSet::new()),
        "arrest alone does not count"
    );

    let mut b = RegionField::builder();
    b.add(&rect, RegionBehavior::Terrain, 1.0, cell, &grid);
    assert!(
        !b.build()
            .has_terrain_or_impassable(&std::collections::BTreeSet::new()),
        "terrain mult==1 is a no-op, does not count"
    );

    assert!(
        !RegionField::builder()
            .build()
            .has_terrain_or_impassable(&std::collections::BTreeSet::new()),
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
            (
                RegionBehavior::Terrain,
                2.0,
                std::collections::BTreeMap::new()
            ),
            (
                RegionBehavior::Impassable,
                1.0,
                std::collections::BTreeMap::new()
            )
        ]),
        Some(RegionEffect::Impassable)
    );
    assert_eq!(
        compose(&[
            (
                RegionBehavior::Terrain,
                2.0,
                std::collections::BTreeMap::new()
            ),
            (
                RegionBehavior::Arrest,
                1.0,
                std::collections::BTreeMap::new()
            )
        ]),
        Some(RegionEffect::Arrest)
    );
}

#[test]
fn compose_terrain_overlap_takes_max_not_sum() {
    assert_eq!(
        compose(&[
            (
                RegionBehavior::Terrain,
                2.0,
                std::collections::BTreeMap::new()
            ),
            (
                RegionBehavior::Terrain,
                3.0,
                std::collections::BTreeMap::new()
            )
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

/// A single 0..100x0..100 rect region, terrain behavior, `flat` flat cost + `by_tag` per-tag
/// overrides.
fn terrain_field_with_tags(flat: f64, by_tag: &[(&str, f64)]) -> RegionField {
    let grid = sq(100.0);
    let mut b = RegionField::builder();
    let cost_by_tag: std::collections::BTreeMap<String, f64> =
        by_tag.iter().map(|(t, c)| (t.to_string(), *c)).collect();
    b.add_parts(
        &[(
            RegionShape::Rect {
                x0: 0.0,
                y0: 0.0,
                x1: 100.0,
                y1: 100.0,
            },
            false,
        )],
        RegionBehavior::Terrain,
        flat,
        &cost_by_tag,
        100.0,
        &grid,
    );
    b.build()
}

fn tags(names: &[&str]) -> std::collections::BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

#[test]
fn terrain_cost_prefers_matching_tag_cost_over_flat_fallback() {
    let field = terrain_field_with_tags(3.0, &[("swimming", 1.5)]);
    assert_eq!(
        field.resolved_terrain_multiplier((0, 0), &tags(&["swimming"])),
        1.5,
        "a mover with the matching tag pays the tag cost, not the flat 3.0 fallback"
    );
}

#[test]
fn terrain_cost_falls_back_to_flat_cost_when_no_tag_matches() {
    let field = terrain_field_with_tags(3.0, &[("swimming", 1.5)]);
    assert_eq!(
        field.resolved_terrain_multiplier((0, 0), &tags(&["walking"])),
        3.0,
        "no matching key in cost_by_tag ⇒ the flat cost, exactly as today"
    );
    // Every existing region (cost_by_tag empty) resolves identically to the flat-only path for
    // EVERY mover, regardless of tags — the removal/degradation pin for this item.
    let untagged = terrain_field_with_tags(2.0, &[]);
    assert_eq!(
        untagged.resolved_terrain_multiplier((0, 0), &tags(&["flying", "swimming"])),
        2.0,
        "an untagged region ignores the mover's tags entirely and reads as its flat cost"
    );
}

#[test]
fn terrain_cost_takes_the_minimum_across_multiple_matching_tags_on_one_mover() {
    let field = terrain_field_with_tags(5.0, &[("flying", 1.0), ("swimming", 2.0)]);
    assert_eq!(
        field.resolved_terrain_multiplier((0, 0), &tags(&["flying", "swimming"])),
        1.0,
        "a mover with BOTH qualifying tags takes the cheaper (most favorable) one"
    );
}

#[test]
fn terrain_cost_ignore_terrain_bypasses_cost_by_tag_entirely() {
    let field = terrain_field_with_tags(5.0, &[("swimming", 2.0)]);
    let exempt = crate::scene::pathfinding::MoveTraits {
        ignore_terrain: true,
        tags: tags(&["swimming"]),
    };
    assert_eq!(
        crate::scene::pathfinding::terrain_cost(Some(&field), (0, 0), &exempt),
        1.0,
        "ignore_terrain flattens every multiplier to 1.0 regardless of a matching cost_by_tag key"
    );
    let non_exempt = crate::scene::pathfinding::MoveTraits {
        ignore_terrain: false,
        tags: tags(&["swimming"]),
    };
    assert_eq!(
        crate::scene::pathfinding::terrain_cost(Some(&field), (0, 0), &non_exempt),
        2.0,
        "control: the same mover without ignore_terrain pays the matching tag cost"
    );
}

#[test]
fn terrain_cost_max_across_overlapping_regions_composes_over_the_per_region_tag_resolution() {
    // Two terrain regions overlap cell (0,0): region A authors a matching cost_by_tag entry
    // (flat 5.0, "flying": 2.0); region B is flat-only (flat 3.0, no cost_by_tag). Every value
    // is distinct on purpose, so MIN-then-MAX (the correct rule), MIN-across-regions, and
    // dropping the flat-only region B from `tag_cost` each land on a DIFFERENT number for the
    // tagged mover — the three wrong compositions cannot hide behind a coincidental match.
    let grid = sq(100.0);
    let rect = RegionShape::Rect {
        x0: 0.0,
        y0: 0.0,
        x1: 100.0,
        y1: 100.0,
    };
    let mut b = RegionField::builder();
    let mut cost_by_tag_a = std::collections::BTreeMap::new();
    cost_by_tag_a.insert("flying".to_string(), 2.0);
    b.add_parts(
        &[(rect.clone(), false)],
        RegionBehavior::Terrain,
        5.0,
        &cost_by_tag_a,
        100.0,
        &grid,
    );
    b.add_parts(
        &[(rect, false)],
        RegionBehavior::Terrain,
        3.0,
        &std::collections::BTreeMap::new(),
        100.0,
        &grid,
    );
    let field = b.build();

    // Tagged mover: region A resolves to its matching tag cost (2.0, the MIN — its only
    // candidate); region B (no cost_by_tag) resolves to its own flat cost (3.0). The correct
    // composition is the overlapping-regions MAX of the two per-region resolved values:
    // max(2.0, 3.0) = 3.0 — NOT 2.0 (a wrongly MIN-composed outer fold) and NOT 5.0 (region B
    // silently dropped, leaving only region A's flat fallback).
    assert_eq!(
        field.resolved_terrain_multiplier((0, 0), &tags(&["flying"])),
        3.0,
        "MAX across regions of each region's own MIN-matching-tag-or-flat resolution"
    );
    // Untagged control: both regions fall back to their flat cost; max(5.0, 3.0) = 5.0.
    assert_eq!(
        field.resolved_terrain_multiplier((0, 0), &tags(&[])),
        5.0,
        "untagged control: both regions fall back to flat cost, MAX still applies"
    );
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
        parse_region_shape(&crate::data::engine::RegionGeometry::Single(eng_shape(
            "rect",
            vec![0.0, 0.0, 100.0, 100.0]
        ))),
        Some(vec![(
            RegionShape::Rect {
                x0: 0.0,
                y0: 0.0,
                x1: 100.0,
                y1: 100.0
            },
            false
        )])
    );
    assert_eq!(
        parse_region_shape(&crate::data::engine::RegionGeometry::Single(eng_shape(
            "circle",
            vec![50.0, 50.0, 25.0]
        ))),
        Some(vec![(
            RegionShape::Circle {
                cx: 50.0,
                cy: 50.0,
                r: 25.0
            },
            false
        )])
    );
    assert_eq!(
        parse_region_shape(&crate::data::engine::RegionGeometry::Single(eng_shape(
            "polygon",
            vec![0.0, 0.0, 100.0, 0.0, 0.0, 100.0]
        ))),
        Some(vec![(
            RegionShape::Polygon {
                points: vec![(0.0, 0.0), (100.0, 0.0), (0.0, 100.0)]
            },
            false
        )])
    );
}

// A raw `system`-body shape with a missing `/shape` key or a non-numeric point entry cannot
// reach this function at all: `parse_region_shape` takes the typed, already
// ingress-validated `engine::RegionGeometry` directly (a
// document carrying either defect fails ingress validation before persistence, so no caller
// of this function can ever hand it one). Only the two structural checks internal to this
// function's own dispatch — an unrecognized `kind`, or a `points` length that doesn't match
// the kind — remain reachable.
#[test]
fn parse_region_shape_malformed_is_none() {
    assert_eq!(
        parse_region_shape(&crate::data::engine::RegionGeometry::Single(eng_shape(
            "rect",
            vec![1.0, 2.0]
        ))),
        None,
        "rect requires exactly 4 points"
    );
    assert_eq!(
        parse_region_shape(&crate::data::engine::RegionGeometry::Single(eng_shape(
            "hexagon",
            vec![]
        ))),
        None,
        "unknown kind"
    );
}

// -----------------------------------------------------------------------
// Compound parts: normalization + composition
// -----------------------------------------------------------------------

fn eng_geometry(parts: &[(&str, Vec<f64>, bool)]) -> crate::data::engine::RegionGeometry {
    crate::data::engine::RegionGeometry::Compound(
        parts
            .iter()
            .map(
                |(kind, points, hole)| crate::data::engine::CompoundShapePart {
                    shape: eng_shape(kind, points.clone()),
                    hole: *hole,
                },
            )
            .collect(),
    )
}

#[test]
fn parse_region_shape_single_legacy_document_still_parses() {
    // The migration-freedom pin: a region document stored with the pre-compound wire form
    // (`shape` as a bare `{kind, points}` object) deserializes into `RegionGeometry::Single`
    // and normalizes to the identical one-part list the singular parser produced before —
    // zero migration, zero read-side behavior change.
    let legacy = serde_json::json!({ "kind": "rect", "points": [0.0, 0.0, 100.0, 100.0] });
    let geometry: crate::data::engine::RegionGeometry = serde_json::from_value(legacy.clone())
        .expect("the legacy singular form must still deserialize");
    assert!(matches!(
        geometry,
        crate::data::engine::RegionGeometry::Single(_)
    ));
    // Re-serialization preserves the legacy form exactly (the untagged union round-trips
    // each variant to its own wire shape — `normalize_engine` cannot silently rewrite a
    // stored document's form).
    assert_eq!(serde_json::to_value(&geometry).unwrap(), legacy);
    assert_eq!(
        parse_region_shape(&geometry),
        Some(vec![(
            RegionShape::Rect {
                x0: 0.0,
                y0: 0.0,
                x1: 100.0,
                y1: 100.0
            },
            false
        )]),
        "the legacy form normalizes to one additive part"
    );
}

#[test]
fn parse_region_shape_fails_closed_on_one_bad_part_in_a_compound_list() {
    // One structurally-invalid part (a rect with 2 points) drops the WHOLE region: a
    // partially-valid compound is never partially rasterized (a silently-dropped hole could
    // silently make an impassable region passable in a spot the GM thought was excluded).
    assert_eq!(
        parse_region_shape(&eng_geometry(&[
            ("rect", vec![0.0, 0.0, 100.0, 100.0], false),
            ("rect", vec![1.0, 2.0], true),
        ])),
        None,
        "one bad part fails the whole region, whichever side of the hole flag it sits on"
    );
    assert_eq!(
        parse_region_shape(&crate::data::engine::RegionGeometry::Compound(vec![])),
        None,
        "an empty compound list is degenerate"
    );
    // The same list with the bad part fixed parses both parts, preserving the hole flags.
    assert_eq!(
        parse_region_shape(&eng_geometry(&[
            ("rect", vec![0.0, 0.0, 100.0, 100.0], false),
            ("circle", vec![50.0, 50.0, 25.0], true),
        ]))
        .map(|parts| parts.iter().map(|(_, hole)| *hole).collect::<Vec<_>>()),
        Some(vec![false, true]),
        "each part's own hole flag survives normalization"
    );
}

#[test]
fn rasterize_compound_unions_additive_parts() {
    // Two overlapping rects: the overlapping cell appears exactly once (a union, not a
    // concatenation), and both rects' exclusive cells are covered.
    let a = RegionShape::Rect {
        x0: 0.0,
        y0: 0.0,
        x1: 200.0,
        y1: 100.0,
    };
    let b = RegionShape::Rect {
        x0: 100.0,
        y0: 0.0,
        x1: 300.0,
        y1: 100.0,
    };
    let cells = rasterize_compound(&[(a.clone(), false), (b, false)], 100.0, &sq(100.0)).unwrap();
    assert_eq!(
        cells.iter().filter(|c| **c == (1, 0)).count(),
        1,
        "the overlap cell is deduplicated"
    );
    for expected in [(0, 0), (1, 0), (2, 0)] {
        assert!(cells.contains(&expected), "union must cover {expected:?}");
    }
    // A single additive part yields exactly `rasterize`'s own output (byte-identical, order
    // included) — the legacy single-shape path composes to itself.
    assert_eq!(
        rasterize_compound(&[(a.clone(), false)], 100.0, &sq(100.0)).unwrap(),
        rasterize(&a, 100.0, &sq(100.0)).unwrap(),
        "one additive part composes to the single-shape rasterization"
    );
}

#[test]
fn rasterize_compound_subtracts_hole_parts() {
    // A 3-cell additive bar with its middle cell holed out: the holed cell is excluded from
    // the composed set, the flanking cells survive.
    let bar = RegionShape::Rect {
        x0: 0.0,
        y0: 0.0,
        x1: 300.0,
        y1: 100.0,
    };
    let notch = RegionShape::Rect {
        x0: 100.0,
        y0: 0.0,
        x1: 200.0,
        y1: 100.0,
    };
    let cells = rasterize_compound(&[(bar, false), (notch, true)], 100.0, &sq(100.0)).unwrap();
    assert_eq!(
        cells,
        vec![(0, 0), (2, 0)],
        "the hole subtracts exactly its own cells"
    );
    // A compound of ONLY hole parts composes to an empty (but valid) set — the region
    // covers nothing and contributes nothing, never an error.
    assert_eq!(
        rasterize_compound(
            &[(
                RegionShape::Circle {
                    cx: 0.0,
                    cy: 0.0,
                    r: 150.0
                },
                true
            )],
            100.0,
            &sq(100.0)
        ),
        Some(vec![]),
        "a holes-only compound is an empty coverage set, not a failure"
    );
}

#[test]
fn rasterize_compound_a_hole_outside_every_additive_part_changes_nothing() {
    // A hole that overlaps NO additive cell is a no-op: the composed set is exactly the
    // additive union (authoring a hole "just in case" must never widen or shrink coverage).
    let bar = RegionShape::Rect {
        x0: 0.0,
        y0: 0.0,
        x1: 200.0,
        y1: 100.0,
    };
    let far_hole = RegionShape::Rect {
        x0: 900.0,
        y0: 900.0,
        x1: 1000.0,
        y1: 1000.0,
    };
    assert_eq!(
        rasterize_compound(&[(bar.clone(), false), (far_hole, true)], 100.0, &sq(100.0)),
        rasterize(&bar, 100.0, &sq(100.0)),
        "a disjoint hole changes nothing"
    );
    // ANY part failing closed (an over-cap AABB here) fails the whole region, even when the
    // failing part is a hole that would have changed nothing.
    let extreme = RegionShape::Rect {
        x0: 0.0,
        y0: 0.0,
        x1: 1e12,
        y1: 1e12,
    };
    assert_eq!(
        rasterize_compound(&[(bar, false), (extreme, true)], 100.0, &sq(100.0)),
        None,
        "a fail-closed hole part fails the whole region"
    );
}

#[test]
fn rasterize_compound_final_count_is_capped_post_composition() {
    // Two disjoint additive rects, EACH individually under the per-part AABB cap
    // (316×316 = 99,856 candidate cells), compose to 199,712 cells — over the per-region
    // `MAX_REGION_CELLS` bound. The final composed count must fail closed, or a
    // `MAX_REGION_SHAPE_PARTS`-sized list of at-cap parts would relax the DoS bound 32×.
    let a = RegionShape::Rect {
        x0: 0.0,
        y0: 0.0,
        x1: 31_599.0,
        y1: 31_599.0,
    };
    let b = RegionShape::Rect {
        x0: 100_000.0,
        y0: 100_000.0,
        x1: 131_599.0,
        y1: 131_599.0,
    };
    assert_eq!(
        rasterize_compound(&[(a.clone(), false), (b, false)], 100.0, &sq(100.0)),
        None,
        "individually-in-cap parts must not sum past the per-region cell bound"
    );
    // Positive control: the same at-cap additive part with a near-full hole nets a small
    // final area and is NOT spuriously rejected — the cap reads the POST-subtraction count.
    let hole = RegionShape::Rect {
        x0: 100.0,
        y0: 0.0,
        x1: 31_599.0,
        y1: 31_599.0,
    };
    let cells = rasterize_compound(&[(a, false), (hole, true)], 100.0, &sq(100.0))
        .expect("a huge additive shape with a huge hole netting a small area is admitted");
    assert_eq!(cells.len(), 316, "only the left column survives the hole");
    assert!(cells.contains(&(0, 0)));
    assert!(!cells.contains(&(1, 0)));
}

// -----------------------------------------------------------------------
// Trigger identity rows + the firing plan
// -----------------------------------------------------------------------

/// A `TriggerRegion` fixture covering `cells` with `triggers`, id derived from `id`.
fn trigger_region(id: u128, cells: &[(i32, i32)], triggers: Vec<RegionTrigger>) -> TriggerRegion {
    TriggerRegion {
        region_id: crate::test_ids::test_uuid(id),
        visible_to_all: true,
        triggers,
        cells: cells.iter().copied().collect(),
    }
}

/// A single-trigger fixture: `on` + a `condition_add` effect (the effect payload is
/// irrelevant to the firing plan, which keys on `on` and coverage only).
fn on(on: TriggerEvent) -> RegionTrigger {
    RegionTrigger {
        on,
        effect: crate::data::engine::TriggerEffect::ConditionAdd {
            condition: "x".to_string(),
        },
    }
}

#[test]
fn fired_triggers_enter_matches_only_covered_entered_cells() {
    let regions = vec![
        trigger_region(1, &[(1, 0), (2, 0)], vec![on(TriggerEvent::Enter)]),
        trigger_region(2, &[(5, 5)], vec![on(TriggerEvent::Enter)]),
    ];
    let fired = fired_triggers(&regions, &[(0, 0), (1, 0)], None, &[]);
    assert_eq!(fired.len(), 1, "only the covered region's trigger fires");
    assert_eq!(fired[0].0, crate::test_ids::test_uuid(1));

    let none = fired_triggers(&regions, &[(0, 0)], None, &[]);
    assert!(none.is_empty(), "no coverage, no fire");
}

#[test]
fn fired_triggers_arrest_fires_only_on_arrest_inside_the_region() {
    let regions = vec![trigger_region(1, &[(2, 0)], vec![on(TriggerEvent::Arrest)])];
    // Entering the region without an arrest report does not fire an arrest trigger.
    assert!(fired_triggers(&regions, &[(2, 0)], None, &[]).is_empty());
    // An arrest outside the region does not fire it either.
    assert!(fired_triggers(&regions, &[(2, 0)], Some((9, 9)), &[]).is_empty());
    // Only an arrest inside does.
    let fired = fired_triggers(&regions, &[(2, 0)], Some((2, 0)), &[]);
    assert_eq!(fired.len(), 1);
}

#[test]
fn fired_triggers_fires_each_trigger_once_per_report() {
    // Two entered cells in the same region collapse to one firing of each of its triggers;
    // two distinct triggers on the same region each fire.
    let regions = vec![trigger_region(
        1,
        &[(1, 0), (2, 0)],
        vec![on(TriggerEvent::Enter), on(TriggerEvent::Enter)],
    )];
    let fired = fired_triggers(&regions, &[(1, 0), (2, 0)], None, &[]);
    assert_eq!(
        fired.len(),
        2,
        "one firing per authored trigger, not per cell"
    );
}

#[test]
fn fired_triggers_fires_exit_when_a_region_id_is_in_exited_but_not_entered() {
    let regions = vec![
        trigger_region(1, &[(1, 0)], vec![on(TriggerEvent::Exit)]),
        trigger_region(2, &[(5, 5)], vec![on(TriggerEvent::Exit)]),
    ];
    // The exited id names region 1: its Exit trigger fires even though no cell of it was
    // entered (exit is a set-difference event over whole regions, not a cell crossing).
    let fired = fired_triggers(&regions, &[], None, &[crate::test_ids::test_uuid(1)]);
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].0, crate::test_ids::test_uuid(1));

    // An exited id the table does not carry (a deleted or disabled region) fires nothing.
    assert!(
        fired_triggers(&regions, &[], None, &[crate::test_ids::test_uuid(0xDEAD)]).is_empty(),
        "an exited id with no identity row fires nothing"
    );
    // A region whose id is not exited never fires its Exit trigger, even when its cells
    // were entered — Enter/Arrest stay the only cell-crossing events.
    assert!(
        fired_triggers(&regions, &[(1, 0), (5, 5)], None, &[]).is_empty(),
        "cell coverage alone never fires an Exit trigger"
    );
}

#[test]
fn fired_triggers_exit_and_enter_do_not_double_fire_on_the_same_report() {
    // One report can carry BOTH an entered-cell coverage and an exited id for the same
    // region (a token stored as a member whose walk crosses the region again and ends
    // outside it): each authored trigger still fires at most once.
    let regions = vec![trigger_region(
        1,
        &[(1, 0), (2, 0)],
        vec![on(TriggerEvent::Enter), on(TriggerEvent::Exit)],
    )];
    let fired = fired_triggers(
        &regions,
        &[(1, 0), (2, 0)],
        None,
        &[crate::test_ids::test_uuid(1)],
    );
    assert_eq!(
        fired.len(),
        2,
        "the Enter and the Exit each fire exactly once, got {fired:?}"
    );
    let mut events: Vec<TriggerEvent> = fired.iter().map(|(_, t)| t.on).collect();
    events.sort_by_key(|e| match e {
        TriggerEvent::Enter => 0,
        TriggerEvent::Arrest => 1,
        TriggerEvent::Exit => 2,
        TriggerEvent::TurnStart
        | TriggerEvent::TurnEnd
        | TriggerEvent::RoundStart
        | TriggerEvent::RoundEnd => 3,
    });
    assert_eq!(events, vec![TriggerEvent::Enter, TriggerEvent::Exit]);
}

#[test]
fn fired_turn_round_triggers_filters_by_on_event_correctly() {
    // One region carrying every event family plus a SECOND authored
    // turn_start trigger: only triggers whose `on` matches the firing event
    // fire, each distinct authored trigger firing once (its index is its
    // identity — the duplicate turn_start rows are NOT collapsed).
    let regions = vec![trigger_region(
        1,
        &[(1, 0)],
        vec![
            on(TriggerEvent::Enter),
            on(TriggerEvent::TurnStart),
            on(TriggerEvent::RoundEnd),
            on(TriggerEvent::TurnStart),
        ],
    )];
    let member: std::collections::BTreeSet<Uuid> =
        [crate::test_ids::test_uuid(1)].into_iter().collect();

    let fired = fired_turn_round_triggers(&regions, &member, TriggerEvent::TurnStart);
    assert_eq!(
        fired.iter().map(|(r, i, _)| (*r, *i)).collect::<Vec<_>>(),
        vec![
            (crate::test_ids::test_uuid(1), 1),
            (crate::test_ids::test_uuid(1), 3)
        ],
        "both authored turn_start triggers fire, by authored index, and no other event does"
    );

    let fired = fired_turn_round_triggers(&regions, &member, TriggerEvent::RoundEnd);
    assert_eq!(
        fired.iter().map(|(_, i, _)| *i).collect::<Vec<_>>(),
        vec![2],
        "the round_end trigger fires at the round boundary"
    );

    // The filter is event-generic: asked for Enter it returns the Enter
    // trigger like any other — the "a movement report never fires the
    // turn/round family" invariant lives in the CALLERS (the transition's
    // `phase_trigger_event` maps only the four combat phases, so movement
    // events never reach this function in production; `fired_triggers`'s own
    // arms keep the complementary direction).
    assert_eq!(
        fired_turn_round_triggers(&regions, &member, TriggerEvent::Enter).len(),
        1
    );
    // A region the token is NOT a recorded member of fires nothing even at a
    // matching event (the membership gate).
    assert!(
        fired_turn_round_triggers(
            &regions,
            &std::collections::BTreeSet::new(),
            TriggerEvent::TurnStart
        )
        .is_empty(),
        "no recorded membership, no fire"
    );
    // A member id with no identity row (a deleted/disabled region) fires nothing.
    let orphan: std::collections::BTreeSet<Uuid> =
        [crate::test_ids::test_uuid(0xDEAD)].into_iter().collect();
    assert!(fired_turn_round_triggers(&regions, &orphan, TriggerEvent::TurnStart).is_empty());
}

// -----------------------------------------------------------------------
// Ambient-light behavior: the ambient facet of composition
// -----------------------------------------------------------------------

/// An `AmbientLight` behavior contribution fixture (the `f64` slot is unused by the ambient
/// facet — it carries `cost`, which only `Terrain` reads).
fn ambient(
    mode: AmbientLightMode,
    level: f64,
) -> (RegionBehavior, f64, std::collections::BTreeMap<String, f64>) {
    (
        RegionBehavior::AmbientLight(AmbientLightOverride { mode, level }),
        1.0,
        std::collections::BTreeMap::new(),
    )
}

#[test]
fn compose_ambient_darken_reduces_illumination() {
    let Some(AmbientAdjustment::Delta(d)) =
        compose_ambient(&[ambient(AmbientLightMode::Darken, 0.3)])
    else {
        panic!("a darken contribution composes to a negative delta");
    };
    assert_eq!(d, -0.3);
    assert_eq!(AmbientAdjustment::Delta(d).apply(0.8), 0.5);
    // Darken never drives a cell below 0.
    assert_eq!(AmbientAdjustment::Delta(d).apply(0.1), 0.0);
}

#[test]
fn compose_ambient_brighten_increases_illumination_saturating_at_one() {
    let Some(AmbientAdjustment::Delta(d)) =
        compose_ambient(&[ambient(AmbientLightMode::Brighten, 0.5)])
    else {
        panic!("a brighten contribution composes to a positive delta");
    };
    assert_eq!(d, 0.5);
    // Saturation: the same ceiling the additive-superposition composition applies.
    assert_eq!(AmbientAdjustment::Delta(d).apply(0.8), 1.0);
}

#[test]
fn compose_ambient_override_replaces_regardless_of_other_contributors() {
    let Some(AmbientAdjustment::Override(level)) = compose_ambient(&[
        ambient(AmbientLightMode::Darken, 0.9),
        ambient(AmbientLightMode::Override, 0.2),
        ambient(AmbientLightMode::Brighten, 0.5),
    ]) else {
        panic!("an override among deltas wins outright");
    };
    assert_eq!(level, 0.2);
    // The base is irrelevant: every contributor is replaced.
    assert_eq!(AmbientAdjustment::Override(level).apply(1.0), 0.2);
    assert_eq!(AmbientAdjustment::Override(level).apply(0.0), 0.2);
}

#[test]
fn compose_two_overlapping_overrides_resolves_to_the_darker_value() {
    let Some(AmbientAdjustment::Override(level)) = compose_ambient(&[
        ambient(AmbientLightMode::Override, 0.4),
        ambient(AmbientLightMode::Override, 0.1),
    ]) else {
        panic!("two overrides still compose to an override");
    };
    assert_eq!(level, 0.1, "the darker of the two wins (fail-closed)");
}

#[test]
fn compose_two_overlapping_darkens_sums_deltas() {
    let Some(AmbientAdjustment::Delta(d)) = compose_ambient(&[
        ambient(AmbientLightMode::Darken, 0.3),
        ambient(AmbientLightMode::Darken, 0.4),
    ]) else {
        panic!("two darkens compose to a summed delta");
    };
    assert_eq!(d, -0.7, "deltas sum, never max: stacked darkness deepens");
    // Darken and Brighten net against each other (epsilon-compared: the sum is f64 arithmetic).
    let Some(AmbientAdjustment::Delta(net)) = compose_ambient(&[
        ambient(AmbientLightMode::Darken, 0.6),
        ambient(AmbientLightMode::Brighten, 0.2),
    ]) else {
        panic!("a darken and a brighten compose to a net delta");
    };
    assert!(
        (net - -0.4).abs() < 1e-12,
        "net darken 0.6 + brighten 0.2 = -0.4, got {net}"
    );
}

#[test]
fn compose_ambient_zero_net_and_no_contribution_are_both_none() {
    assert_eq!(compose_ambient(&[]), None);
    // A darken and a brighten cancel exactly: identical to no adjustment, and kept out of the
    // field map so the `cell_light` fast path stays a plain emptiness test.
    assert_eq!(
        compose_ambient(&[
            ambient(AmbientLightMode::Darken, 0.3),
            ambient(AmbientLightMode::Brighten, 0.3),
        ]),
        None
    );
    // Movement behaviors never contribute to the ambient facet.
    assert_eq!(
        compose_ambient(&[
            (
                RegionBehavior::Terrain,
                2.0,
                std::collections::BTreeMap::new()
            ),
            (
                RegionBehavior::Impassable,
                1.0,
                std::collections::BTreeMap::new()
            )
        ]),
        None
    );
}

#[test]
fn compose_ambient_read_side_guards_against_values_past_ingress() {
    // A non-finite level contributes nothing (a stored/fixture value is not trusted just
    // because ingress rejected it once).
    assert_eq!(
        compose_ambient(&[ambient(AmbientLightMode::Darken, f64::NAN)]),
        None
    );
    // A negative darken is clamped to a zero delta rather than silently brightening.
    assert_eq!(
        compose_ambient(&[ambient(AmbientLightMode::Darken, -0.5)]),
        None
    );
    // An out-of-scale override level clamps into [0, 1] at application.
    assert_eq!(AmbientAdjustment::Override(1.5).apply(0.5), 1.0);
    assert_eq!(AmbientAdjustment::Override(-1.0).apply(0.5), 0.0);
}

#[test]
fn ambient_contributions_carry_no_movement_effect() {
    // The movement facet ignores ambient contributions entirely, and the ambient facet never
    // produces a movement effect: a cell may carry both, or only one.
    assert_eq!(
        compose(&[ambient(AmbientLightMode::Darken, 0.5)]),
        None,
        "ambient-only contributions compose to no movement effect"
    );
    let grid = sq(100.0);
    let mut b = RegionField::builder();
    b.add(
        &RegionShape::Rect {
            x0: 0.0,
            y0: 0.0,
            x1: 100.0,
            y1: 100.0,
        },
        RegionBehavior::AmbientLight(AmbientLightOverride {
            mode: AmbientLightMode::Darken,
            level: 0.5,
        }),
        1.0,
        100.0,
        &grid,
    );
    let field = b.build();
    assert_eq!(
        field.ambient_at((0, 0)),
        Some(AmbientAdjustment::Delta(-0.5)),
        "the ambient facet covers the region's cells"
    );
    assert_eq!(
        field.ambient_at((5, 5)),
        None,
        "uncovered cell carries no ambient adjustment"
    );
    assert!(
        !field.is_impassable((0, 0)) && !field.is_arrest((0, 0)),
        "an ambient region is neither impassable nor arrest"
    );
    assert_eq!(
        field.terrain_multiplier((0, 0)),
        1.0,
        "an ambient region never weights movement cost"
    );
    assert!(
        !field.has_terrain_or_impassable(&std::collections::BTreeSet::new()),
        "an ambient-only field never forces the router's weighted sub-path"
    );
}

// --- Occluder outlines ---

/// A rect part covering the square-grid cells whose centers fall in `[x0, x1] × [y0, y1]`.
fn rect_part(x0: f64, y0: f64, x1: f64, y1: f64, hole: bool) -> (RegionShape, bool) {
    (RegionShape::Rect { x0, y0, x1, y1 }, hole)
}

/// The set of unordered endpoint pairs of `segs`, rounded, for order-free comparison.
fn segment_set(
    segs: &[(crate::scene::vision::P, crate::scene::vision::P)],
) -> BTreeSet<((i64, i64), (i64, i64))> {
    segs.iter()
        .map(|&(a, b)| {
            let (qa, qb) = (quantize(a), quantize(b));
            if qa < qb {
                (qa, qb)
            } else {
                (qb, qa)
            }
        })
        .collect()
}

#[test]
fn outline_of_a_square_block_merges_to_its_perimeter() {
    // A 3 x 2 block of 100-unit cells: twelve unit cell edges on the boundary, merged by
    // collinearity into the four sides of the block rectangle.
    let grid = sq(100.0);
    let cells = rasterize(
        &RegionShape::Rect {
            x0: 0.0,
            y0: 0.0,
            x1: 300.0,
            y1: 200.0,
        },
        100.0,
        &grid,
    )
    .unwrap();
    assert_eq!(cells.len(), 6);
    let outline = cell_outline_segments(&cells, &grid, 100.0);
    let expected = segment_set(&[
        ((0.0, 0.0), (300.0, 0.0)),
        ((300.0, 0.0), (300.0, 200.0)),
        ((300.0, 200.0), (0.0, 200.0)),
        ((0.0, 200.0), (0.0, 0.0)),
    ]);
    assert_eq!(outline.len(), 4, "collinear runs merge: {outline:?}");
    assert_eq!(segment_set(&outline), expected);
}

#[test]
fn outline_of_a_ring_has_an_inner_loop_for_the_hole() {
    // A 3 x 3 block minus its center cell: an outer square and an inner square.
    let grid = sq(100.0);
    let parts = [
        rect_part(0.0, 0.0, 300.0, 300.0, false),
        rect_part(150.0, 150.0, 160.0, 160.0, true),
    ];
    let cells = rasterize_compound(&parts, 100.0, &grid).unwrap();
    assert_eq!(cells.len(), 8);
    let outline = cell_outline_segments(&cells, &grid, 100.0);
    let expected = segment_set(&[
        ((0.0, 0.0), (300.0, 0.0)),
        ((300.0, 0.0), (300.0, 300.0)),
        ((300.0, 300.0), (0.0, 300.0)),
        ((0.0, 300.0), (0.0, 0.0)),
        ((100.0, 100.0), (200.0, 100.0)),
        ((200.0, 100.0), (200.0, 200.0)),
        ((200.0, 200.0), (100.0, 200.0)),
        ((100.0, 200.0), (100.0, 100.0)),
    ]);
    assert_eq!(segment_set(&outline), expected);
}

#[test]
fn outline_of_a_hex_block_follows_the_hex_boundary_edges() {
    // One hex has six edges; two adjacent hexes share one, leaving 5 + 5 boundary edges (no
    // two of them collinear, so none merge).
    let grid = crate::scene::grid_shape::HexGrid { size: 50.0 };
    let one = cell_outline_segments(&[(0, 0)], &grid, 50.0);
    assert_eq!(one.len(), 6);
    let two = cell_outline_segments(&[(0, 0), (1, 0)], &grid, 50.0);
    assert_eq!(two.len(), 10, "the shared edge is interior: {two:?}");
}

#[test]
fn occluder_outline_falls_back_to_the_bounding_rectangle_over_the_cell_cap() {
    // A shape whose rasterization exceeds `MAX_REGION_CELLS` must occlude by its bounding
    // rectangle, never drop out (a dropped region would reveal what it hides).
    let grid = sq(1.0);
    let parts = [rect_part(0.0, 0.0, 1000.0, 1000.0, false)];
    assert!(rasterize_compound(&parts, 1.0, &grid).is_none());
    let outline = occluder_outline(&parts, 1.0, &grid);
    assert_eq!(
        segment_set(&outline),
        segment_set(&[
            ((0.0, 0.0), (1000.0, 0.0)),
            ((1000.0, 0.0), (1000.0, 1000.0)),
            ((1000.0, 1000.0), (0.0, 1000.0)),
            ((0.0, 1000.0), (0.0, 0.0)),
        ])
    );
}

#[test]
fn occluder_outline_falls_back_to_the_bounding_rectangle_over_the_edge_cap() {
    // A 70 x 70 lattice of isolated one-cell parts: 4900 cells (under the cell cap) with 19,600
    // unmerged boundary edges (over `MAX_OCCLUDER_EDGES`), so the additive bounding rectangle
    // stands in. Fails if the edge cap is not enforced (the full outline would be returned).
    let grid = sq(10.0);
    let mut parts = Vec::new();
    for i in 0..70 {
        for j in 0..70 {
            let (x, y) = (f64::from(i) * 20.0, f64::from(j) * 20.0);
            parts.push(rect_part(x + 2.0, y + 2.0, x + 8.0, y + 8.0, false));
        }
    }
    let cells = rasterize_compound(&parts, 10.0, &grid).unwrap();
    assert_eq!(cells.len(), 4900);
    assert!(cell_outline_segments(&cells, &grid, 10.0).len() > MAX_OCCLUDER_EDGES);
    let outline = occluder_outline(&parts, 10.0, &grid);
    assert_eq!(
        outline.len(),
        4,
        "the bounding rectangle: {}",
        outline.len()
    );
}

#[test]
fn occluder_outline_of_a_region_covering_no_cell_center_is_empty() {
    let grid = sq(100.0);
    let parts = [rect_part(10.0, 10.0, 20.0, 20.0, false)];
    assert!(occluder_outline(&parts, 100.0, &grid).is_empty());
}

// --- Token-anchored geometry ---

/// A rect part given as offset corners.
fn offset_rect(x0: f64, y0: f64, x1: f64, y1: f64, hole: bool) -> (RegionShape, bool) {
    (RegionShape::Rect { x0, y0, x1, y1 }, hole)
}

#[test]
fn anchored_parts_translate_every_shape_kind_and_keep_hole_flags() {
    let parts = vec![
        offset_rect(-10.0, -20.0, 30.0, 40.0, false),
        (
            RegionShape::Circle {
                cx: 5.0,
                cy: -5.0,
                r: 7.0,
            },
            true,
        ),
        (
            RegionShape::Polygon {
                points: vec![(0.0, 0.0), (10.0, 0.0), (0.0, 10.0)],
            },
            false,
        ),
    ];
    let placed = anchored_parts(parts, (100.0, 200.0), 77.0, false);
    assert_eq!(
        placed,
        vec![
            offset_rect(90.0, 180.0, 130.0, 240.0, false),
            (
                RegionShape::Circle {
                    cx: 105.0,
                    cy: 195.0,
                    r: 7.0
                },
                true
            ),
            (
                RegionShape::Polygon {
                    points: vec![(100.0, 200.0), (110.0, 200.0), (100.0, 210.0)]
                },
                false
            ),
        ],
        "translation only: kinds, radii and hole flags survive, rotation is ignored"
    );
}

#[test]
fn anchored_parts_rotate_clockwise_about_the_carrier_when_following_rotation() {
    let placed = anchored_parts(
        vec![offset_rect(0.0, -10.0, 100.0, 10.0, false)],
        (500.0, 500.0),
        90.0,
        true,
    );
    let [(RegionShape::Polygon { points }, false)] = placed.as_slice() else {
        panic!("a rotated rect becomes a polygon: {placed:?}");
    };
    // The +x arm of the offset box now points toward +y (clockwise on a y-down canvas).
    let expected = [
        (510.0, 500.0),
        (510.0, 600.0),
        (490.0, 600.0),
        (490.0, 500.0),
    ];
    assert_eq!(points.len(), 4);
    for (got, want) in points.iter().zip(expected) {
        assert!(
            (got.0 - want.0).abs() < 1e-9 && (got.1 - want.1).abs() < 1e-9,
            "vertex {got:?} != {want:?}"
        );
    }

    let circle = anchored_parts(
        vec![(
            RegionShape::Circle {
                cx: 100.0,
                cy: 0.0,
                r: 5.0,
            },
            false,
        )],
        (0.0, 0.0),
        90.0,
        true,
    );
    let [(RegionShape::Circle { cx, cy, r }, false)] = circle.as_slice() else {
        panic!("a circle stays a circle: {circle:?}");
    };
    assert!(cx.abs() < 1e-9 && (cy - 100.0).abs() < 1e-9 && *r == 5.0);
}

#[test]
fn anchored_parts_with_a_non_finite_pose_fail_closed_at_rasterize() {
    let placed = anchored_parts(
        vec![offset_rect(0.0, 0.0, 10.0, 10.0, false)],
        (f64::NAN, 0.0),
        0.0,
        false,
    );
    assert!(rasterize_compound(&placed, 100.0, &sq(100.0)).is_none());
}
