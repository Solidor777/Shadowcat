//! `Acceptances::assert_premise` fails a keeps-reading control whose socket went a half window
//! without accepting bytes, or whose reader took too little, and passes a reader that sees a lump
//! every eighth of a window.

use std::time::Duration;

use shadowcat::http::conn_serve::ConnLimits;
use shadowcat_test_support::acceptance::{
    progress_bytes_per_window, Acceptances, LUMP_GAP_BOUND, MIN_FLOORS_PER_WINDOW_READ,
    PROGRESS_FLOORS_PER_WINDOW,
};

/// Bytes a reader must have taken for the premise's reading bound to hold.
fn enough_read() -> usize {
    2 * MIN_FLOORS_PER_WINDOW_READ * ConnLimits::default().egress_floor
}

/// A record whose grants land `gap` apart, `count` of them, one lump each.
fn evenly_spaced(gap: Duration, count: u32) -> Acceptances {
    let mut acceptances = Acceptances::new();
    let lump = 1_000_000u64;
    for i in 1..=count {
        acceptances.sample_at(gap * i, lump * u64::from(i));
    }
    acceptances
}

#[test]
fn the_progress_rate_is_the_declared_multiple_of_the_floor() {
    assert_eq!(
        progress_bytes_per_window(),
        PROGRESS_FLOORS_PER_WINDOW * ConnLimits::default().egress_floor
    );
}

#[test]
fn a_lump_every_eighth_of_a_window_holds_the_premise() {
    let acceptances = evenly_spaced(Duration::from_millis(125), 24);
    assert_eq!(acceptances.widest_gap(), Duration::from_millis(125));
    acceptances.assert_premise(enough_read());
}

#[test]
fn an_unchanged_total_adds_no_grant() {
    let mut acceptances = Acceptances::new();
    acceptances.sample_at(Duration::from_millis(0), 10);
    acceptances.sample_at(Duration::from_millis(100), 10);
    acceptances.sample_at(Duration::from_millis(900), 10);
    acceptances.sample_at(Duration::from_millis(1000), 20);
    acceptances.sample_at(Duration::from_millis(1100), 30);
    // Grants at 0, 1000 and 1100 ms; the first gap (0 to 1000) is the buffer filling and is
    // skipped, so the widest counted gap is 1000 to 1100.
    assert_eq!(acceptances.widest_gap(), Duration::from_millis(100));
    assert_eq!(
        acceptances.evidence(7),
        "read 7 B; socket accepted (ms, B) [(0, 10), (1000, 10), (1100, 10)]"
    );
}

#[test]
#[should_panic(expected = "between acceptances")]
fn a_gap_of_the_bound_fails_the_premise() {
    evenly_spaced(LUMP_GAP_BOUND, 6).assert_premise(enough_read());
}

#[test]
fn a_gap_one_millisecond_under_the_bound_holds_the_premise() {
    evenly_spaced(LUMP_GAP_BOUND - Duration::from_millis(1), 6).assert_premise(enough_read());
}

#[test]
#[should_panic(expected = "the control needs to mean anything")]
fn a_reader_that_took_too_little_fails_the_premise() {
    evenly_spaced(Duration::from_millis(125), 24).assert_premise(enough_read() - 1);
}
