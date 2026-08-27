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
