use super::*;
use crate::scene::grid_shape::{candidate_span, HexGrid, SquareGrid};
use crate::scene::pathfinding::DiagonalRule;

/// A square grid at `cell` size — the shape every square-parity test indexes through. The
/// diagonal rule is irrelevant to `cells_in_bounds`/`cell_center`, so any rule serves.
fn sq(cell: f64) -> SquareGrid {
    SquareGrid {
        cell,
        rule: DiagonalRule::Chebyshev,
    }
}

/// A 3×3 block of cell indices, the seed most tests below share.
fn block(n: i32) -> impl Iterator<Item = (i32, i32)> {
    (0..n).flat_map(move |i| (0..n).map(move |j| (i, j)))
}

#[test]
fn marks_the_cells_it_is_given_and_reports_growth() {
    let mut set = ExploredSet::new();
    let grew = set.mark_cells([(0, 0)]);
    assert_eq!(grew, 1);
    assert!(set.contains((0, 0)));
    assert!(!set.contains((1, 0)));
}

#[test]
fn accumulation_is_monotone_no_growth_on_revisit() {
    let mut set = ExploredSet::new();
    let first = set.mark_cells(block(3));
    assert_eq!(first, 9); // a 3×3 block of cells
    let again = set.mark_cells(block(3));
    assert_eq!(again, 0, "revisiting the same area adds no cells");
    assert_eq!(set.len(), 9);
}

#[test]
fn round_trips_through_bytes_deterministically() {
    let mut set = ExploredSet::new();
    set.mark_cells(block(3));
    let bytes = set.to_bytes(crate::scene::GridKind::Square);
    assert_eq!(bytes.len(), EXPLORED_HEADER_LEN + set.len() * 8);
    let back = ExploredSet::from_bytes(&bytes, crate::scene::GridKind::Square);
    assert_eq!(set, back);
}

#[test]
fn from_bytes_drops_a_truncated_trailing_record() {
    let mut set = ExploredSet::new();
    set.mark_cells([(0, 0)]);
    // One cell, via the real encoder, then a 2-byte truncated tail appended by hand.
    let mut bytes = set.to_bytes(crate::scene::GridKind::Square);
    bytes.extend_from_slice(&[0xAB, 0xCD]);
    let decoded = ExploredSet::from_bytes(&bytes, crate::scene::GridKind::Square);
    assert_eq!(decoded, set);
}

#[test]
fn an_empty_cell_set_marks_nothing() {
    let mut set = ExploredSet::new();
    assert_eq!(set.mark_cells(std::iter::empty()), 0);
    assert!(set.is_empty());
}

#[test]
fn a_blob_written_under_one_grid_kind_does_not_decode_under_the_other() {
    // Discrimination: fails if the header is absent, ignored on read, or compared loosely —
    // the assertion is that the SAME bytes yield cells under one kind and none under the
    // other, which no format lacking the tag can satisfy.
    let mut set = ExploredSet::new();
    set.mark_cells(block(3));
    let bytes = set.to_bytes(crate::scene::GridKind::Square);
    assert_eq!(
        ExploredSet::from_bytes(&bytes, crate::scene::GridKind::Square),
        set
    );
    assert!(
        ExploredSet::from_bytes(&bytes, crate::scene::GridKind::Hex).is_empty(),
        "square-indexed fog must not be reinterpreted as hex axial cells"
    );
}

#[test]
fn a_headerless_blob_decodes_to_nothing() {
    // A blob with no header states no coordinate system for its records, so it is unusable
    // rather than assumed. Under-reveal is the safe direction for fog memory.
    // Discrimination: fails if `from_bytes` parses bare 8-byte records.
    let mut bare = (1_i32).to_le_bytes().to_vec();
    bare.extend_from_slice(&(2_i32).to_le_bytes());
    assert!(ExploredSet::from_bytes(&bare, crate::scene::GridKind::Square).is_empty());
}

#[test]
fn a_hex_blob_round_trips_under_its_own_kind() {
    // Discrimination: fails if the header is written but the record payload is mis-offset,
    // which a square-only round-trip test would not catch.
    let mut set = ExploredSet::new();
    set.mark_cells([(1, 0), (-2, 3)]);
    let bytes = set.to_bytes(crate::scene::GridKind::Hex);
    assert_eq!(
        ExploredSet::from_bytes(&bytes, crate::scene::GridKind::Hex),
        set
    );
}

#[test]
fn a_scan_wider_than_the_window_but_under_the_cap_is_returned_unchanged() {
    // The property the conditional application exists for: the cap bounds a PRODUCT while the
    // window bounds a PER-AXIS distance, so a box can reach far past the window on both axes
    // and still enumerate fewer cells than the cap allows. Such a box must not lose a single
    // candidate — its cells are in the mask today and a player can move to them.
    //
    // Discrimination: fails if the window is applied whenever the box is wider than it,
    // because the returned max would then be the window edge rather than the box edge. The
    // fixture guard on the box's reach past the window keeps the test honest if
    // `SCAN_WINDOW_HALF_CELLS` ever changes.
    let cell = 100.0;
    let g = sq(cell);
    let focus = (50.0, 50.0);
    let min = (-50.0, -50.0);
    let max = (150_000.0, 150_000.0); // 1502 × 1502 = 2_256_004 candidates, under the cap
    assert!(
        max.0 - focus.0 > SCAN_WINDOW_HALF_CELLS as f64 * cell,
        "fixture: the box must reach past the window, or the test proves nothing"
    );
    let (out_min, out_max) = clamp_scan_window(
        &g,
        focus,
        (min, max),
        (min, max),
        cell,
        MAX_CELLS_PER_POLYGON,
    );
    assert_eq!((out_min, out_max), (min, max));
}

#[test]
fn clamp_scan_window_bounds_a_scan_that_exceeds_the_cap() {
    // Discrimination: fails if the window is not centred on `focus`, if its half-extent is not
    // `SCAN_WINDOW_HALF_CELLS` cells, or if it expands rather than intersects — the low edges
    // already sit inside the window and must come back unchanged, while the high edges must
    // come back at the window.
    let cell = 100.0;
    let g = sq(cell);
    let focus = (50.0, 50.0);
    let half_px = SCAN_WINDOW_HALF_CELLS as f64 * cell;
    let (min, max) = ((-50.0, -50.0), (1.0e9, 1.0e9));
    let (out_min, out_max) = clamp_scan_window(
        &g,
        focus,
        (min, max),
        (min, max),
        cell,
        MAX_CELLS_PER_POLYGON,
    );
    assert_eq!(
        out_min, min,
        "an edge already inside the window is untouched"
    );
    assert_eq!(out_max, (focus.0 + half_px, focus.1 + half_px));
}

#[test]
fn a_window_that_misses_the_scan_box_leaves_it_unchanged() {
    // The precondition `clamp_scan_window` states: `focus` lies inside the box. A focus far
    // outside it would otherwise produce min > max — an inverted rectangle that enumerates
    // nothing, which is the total loss this clamp exists to remove, reintroduced as a silent
    // empty result.
    // Discrimination: fails if the intersection is returned without the emptiness check.
    let cell = 100.0;
    let g = sq(cell);
    let (min, max) = ((0.0, 0.0), (1.0e9, 1.0e9));
    assert_eq!(
        clamp_scan_window(
            &g,
            (-1.0e8, -1.0e8),
            (min, max),
            (min, max),
            cell,
            MAX_CELLS_PER_POLYGON
        ),
        (min, max)
    );
}

#[test]
fn clamp_scan_window_decides_from_the_decision_box_not_the_actual_box() {
    // A thin, wide actual box: comfortably under the cap on its own (span ≈ 40,002). The
    // decision box is far over the cap. Discrimination: fails if the clamp decision is
    // computed from the actual box instead of the decision box — the actual box would then
    // be returned unchanged, since it is under the cap by itself.
    let cell = 100.0;
    let g = sq(cell);
    let focus = (50.0, 50.0);
    let actual = ((-1.0e6, -50.0), (1.0e6, 50.0));
    let decision = ((-1.0e8, -1.0e8), (1.0e8, 1.0e8));
    let (out_min, out_max) =
        clamp_scan_window(&g, focus, actual, decision, cell, MAX_CELLS_PER_POLYGON);
    assert_ne!(
        (out_min, out_max),
        actual,
        "an over-cap decision box must clamp the actual box even when it is small alone"
    );
}

#[test]
fn a_clamped_square_window_stays_inside_the_per_polygon_cap() {
    // The window exists so that `cells_in_bounds` cannot refuse it.
    // Discrimination: fails if `SCAN_WINDOW_HALF_CELLS` is raised such that
    // `(2*half + 1)^2 > MAX_CELLS_PER_POLYGON`.
    let side = 2 * SCAN_WINDOW_HALF_CELLS + 1;
    let bounds = (0, 0, (side - 1) as i32, (side - 1) as i32);
    assert!(
        !exceeds_cell_cap(bounds, MAX_CELLS_PER_POLYGON),
        "the window enumerates {} cells against a {MAX_CELLS_PER_POLYGON} cap",
        candidate_span(bounds)
    );
}

#[test]
fn a_clamped_hex_window_also_stays_inside_the_per_polygon_cap() {
    // Square is the denser of the two shapes per unit of pixel area only if hex's axial
    // preimage of the same pixel box enumerates fewer cells. That is a claim about
    // `HexGrid::cell_bounds`, so it is measured through that function rather than argued in
    // prose. Discrimination: fails if the axial padding or the preimage arithmetic changes
    // such that a clamped hex window can be refused by the cap.
    let size = 100.0;
    let g = HexGrid { size };
    let half_px = SCAN_WINDOW_HALF_CELLS as f64 * size;
    let bounds = g.cell_bounds((-half_px, -half_px), (half_px, half_px), size);
    assert!(
        !exceeds_cell_cap(bounds, MAX_CELLS_PER_POLYGON),
        "a clamped hex window enumerates {} cells against a {MAX_CELLS_PER_POLYGON} cap",
        candidate_span(bounds)
    );
}

#[test]
fn scan_box_for_decides_from_the_padded_box_regardless_of_mode() {
    // Discrimination: fails if `scan_box_for` decides from the mode's own box (the unpadded
    // box for `Strict`) instead of always from the padded box — the strict box would then
    // return unclamped in a band where its padded counterpart is over cap, so its candidate
    // cells could reach past the (independently-windowed) lenient result's own edge.
    let cell = 1.0;
    let g = sq(cell);
    let focus = (100.0, 100.0);
    let bbox = ((0.0, 0.0), (1999.0, 1999.0));
    let padded = pad_box(bbox, cell);
    assert_eq!(
        candidate_span(g.cell_bounds(bbox.0, bbox.1, cell)),
        4_000_000,
        "fixture: the unpadded span must sit exactly at the cap"
    );
    assert!(
        candidate_span(g.cell_bounds(padded.0, padded.1, cell)) > MAX_CELLS_PER_POLYGON,
        "fixture: the padded span must exceed the cap"
    );
    let strict = scan_box_for(
        &g,
        focus,
        bbox,
        cell,
        MAX_CELLS_PER_POLYGON,
        ScanMode::Strict,
    );
    let half_px = SCAN_WINDOW_HALF_CELLS as f64 * cell;
    let expected = (
        (
            bbox.0 .0.max(focus.0 - half_px),
            bbox.0 .1.max(focus.1 - half_px),
        ),
        (
            bbox.1 .0.min(focus.0 + half_px),
            bbox.1 .1.min(focus.1 + half_px),
        ),
    );
    assert_eq!(
        strict, expected,
        "the strict box must be the window itself, not merely a different box"
    );
}

#[test]
fn scan_box_for_lenient_mode_scans_the_padded_box() {
    // Discrimination: fails if `Lenient` returns the unpadded box, or pads by an amount other
    // than what `pad_box` derives.
    let cell = 100.0;
    let g = sq(cell);
    let focus = (50.0, 50.0);
    let bbox = ((0.0, 0.0), (100.0, 100.0)); // comfortably under the cap, padded or not
    let got = scan_box_for(
        &g,
        focus,
        bbox,
        cell,
        MAX_CELLS_PER_POLYGON,
        ScanMode::Lenient,
    );
    assert_eq!(got, pad_box(bbox, cell));
}

#[test]
fn hex_strict_candidate_cells_nest_inside_lenient_candidate_cells_at_the_clamp_boundary() {
    // Hex twin of the square band fixture: box inclusion on hex runs through the axial
    // preimage bbox, `cube_round` and `HEX_BOUNDS_PAD` — never argued to inherit square's
    // floor monotonicity. Discrimination: fails if `scan_box_for`'s shared-decision fix does
    // not hold on hex, or if hex candidate-BOX inclusion does not imply candidate-CELL-set
    // inclusion (the concern square's `floor` argument cannot settle).
    let size = 1.0;
    let g = HexGrid { size };
    let focus = (0.0, 0.0);
    let bbox = ((0.0, 0.0), (2562.0, 2562.0));
    let padded = pad_box(bbox, size);
    let strict_span = candidate_span(g.cell_bounds(bbox.0, bbox.1, size));
    let lenient_span = candidate_span(g.cell_bounds(padded.0, padded.1, size));
    assert!(
        strict_span <= MAX_CELLS_PER_POLYGON,
        "fixture: the unpadded span must sit at or under the cap ({strict_span})"
    );
    assert!(
        lenient_span > MAX_CELLS_PER_POLYGON,
        "fixture: the padded span must exceed the cap ({lenient_span})"
    );
    let (strict_min, strict_max) = scan_box_for(
        &g,
        focus,
        bbox,
        size,
        MAX_CELLS_PER_POLYGON,
        ScanMode::Strict,
    );
    let (lenient_min, lenient_max) = scan_box_for(
        &g,
        focus,
        bbox,
        size,
        MAX_CELLS_PER_POLYGON,
        ScanMode::Lenient,
    );
    let strict_cells: BTreeSet<Cell> = g
        .cells_in_bounds(strict_min, strict_max, size, MAX_CELLS_PER_POLYGON)
        .expect("strict window stays inside the cap by construction")
        .into_iter()
        .collect();
    let lenient_cells: BTreeSet<Cell> = g
        .cells_in_bounds(lenient_min, lenient_max, size, MAX_CELLS_PER_POLYGON)
        .expect("lenient window stays inside the cap by construction")
        .into_iter()
        .collect();
    assert!(
        !strict_cells.is_empty(),
        "fixture: the strict scan must reach at least one candidate cell"
    );
    assert!(
        strict_cells.is_subset(&lenient_cells),
        "strict candidate cells must nest inside lenient candidate cells on hex too"
    );
}
