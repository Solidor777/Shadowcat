//! The cut counters: one counter per unit, moved only through `record_stall`.

use super::*;

#[test]
fn a_fresh_counter_set_reports_no_cuts() {
    assert_eq!(StallCounters::default().snapshot(), StallCuts::default());
}

#[test]
fn recording_a_stream_cut_moves_the_stream_count_by_one_and_nothing_else() {
    let counters = StallCounters::default();
    counters.record_stall(StallUnit::Stream);
    assert_eq!(counters.snapshot(), StallCuts { stream: 1 });
    counters.record_stall(StallUnit::Stream);
    assert_eq!(counters.snapshot(), StallCuts { stream: 2 });
}
