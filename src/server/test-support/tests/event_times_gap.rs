//! `EventTimes::assert_no_gap_reaches` judges a ping-flood control by the widest gap between its
//! events over an interval, counting the stretches to both ends, and ignores how many events
//! there are.

use std::time::Duration;

use shadowcat_test_support::egress::EventTimes;
use tokio::time::Instant;

/// An interval of `len` from a fixed origin, and the origin.
fn interval(len: Duration) -> (Instant, Instant) {
    let from = Instant::now();
    (from, from + len)
}

/// Events at each of `offsets` after `from`.
fn events(from: Instant, offsets: &[u64]) -> EventTimes {
    let times = EventTimes::default();
    for &ms in offsets {
        times.note_at(from + Duration::from_millis(ms));
    }
    times
}

#[test]
fn the_widest_gap_counts_the_stretch_from_each_end() {
    let (from, to) = interval(Duration::from_millis(1000));
    assert_eq!(
        events(from, &[100, 150, 900]).widest_gap(from, to),
        Duration::from_millis(750)
    );
    assert_eq!(
        events(from, &[400, 450]).widest_gap(from, to),
        Duration::from_millis(550)
    );
}

#[test]
fn an_interval_with_no_event_is_one_gap() {
    let (from, to) = interval(Duration::from_millis(1000));
    assert_eq!(
        events(from, &[]).widest_gap(from, to),
        Duration::from_millis(1000)
    );
}

#[test]
fn events_outside_the_interval_do_not_close_a_gap() {
    let (from, to) = interval(Duration::from_millis(1000));
    let times = EventTimes::default();
    times.note_at(from - Duration::from_millis(1));
    times.note_at(to + Duration::from_millis(1));
    assert_eq!(times.widest_gap(from, to), Duration::from_millis(1000));
}

#[test]
fn a_flood_of_forty_six_events_with_small_gaps_holds_the_control() {
    let (from, to) = interval(Duration::from_millis(1000));
    let offsets: Vec<u64> = (1..=46).map(|i| i * 21).collect();
    events(from, &offsets).assert_no_gap_reaches(from, to, Duration::from_millis(500), "flood");
}

#[test]
#[should_panic(expected = "without an event")]
fn a_gap_of_the_bound_fails_the_control() {
    let (from, to) = interval(Duration::from_millis(1000));
    events(from, &[500, 1000]).assert_no_gap_reaches(from, to, Duration::from_millis(500), "flood");
}
