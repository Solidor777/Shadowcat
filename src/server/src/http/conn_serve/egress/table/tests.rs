//! These tests cover the stream table's lifecycle.
//!
//! They cover creation at parse time, removal only on wire evidence, the GOAWAY filter and purge,
//! the fail-closed cap, the Final and Queued states with the stream's own progress, and the
//! parking of a tunnel session on its stream's backlog.

use super::*;

/// A table with room for `cap` entries.
fn table(cap: usize) -> StreamTable {
    StreamTable::new(cap)
}

#[test]
fn only_a_parsed_odd_request_with_a_rising_id_creates_an_entry() {
    let mut t = table(100);
    assert!(matches!(t.read_headers(1), ParseOutcome::Inserted { .. }));
    assert_eq!(t.read_headers(1), ParseOutcome::Ignored, "a repeat HEADERS");
    assert_eq!(t.read_headers(2), ParseOutcome::Ignored, "an even id");
    assert!(matches!(t.read_headers(5), ParseOutcome::Inserted { .. }));
    assert_eq!(t.read_headers(3), ParseOutcome::Ignored, "a lower id");
    assert_eq!(t.ids(), [1, 5]);
}

#[test]
fn a_terminal_wire_event_removes_the_entry_and_nothing_re_creates_it() {
    for end in 0..4 {
        let mut t = table(100);
        t.read_headers(1);
        match end {
            0 => {
                t.read_reset(1);
            }
            1 => {
                t.write_ended(1);
            }
            2 => {
                t.write_headers_continue(1);
                t.write_continuation_end(1);
            }
            _ => t.goaway(0),
        }
        assert!(t.ids().is_empty(), "terminal event {end}");
        // Trailers or a repeat HEADERS for the same stream carry the same id.
        assert_eq!(t.read_headers(1), ParseOutcome::Ignored, "event {end}");
        assert!(t.ids().is_empty(), "event {end}");
    }
}

#[test]
fn a_terminal_event_for_an_absent_id_changes_nothing() {
    let mut t = table(100);
    t.read_headers(3);
    t.read_reset(1);
    t.write_ended(5);
    t.write_continuation_end(7);
    t.write_headers_continue(9);
    t.write_continuation_end(9);
    t.register_final(11);
    t.hand_over(13, 0, tokio::time::Instant::now());
    assert_eq!(t.ids(), [3]);
}

#[test]
fn a_header_block_that_ends_the_stream_ends_it_at_the_continuation_that_ends_the_block() {
    let mut t = table(100);
    t.read_headers(1);
    t.read_headers(3);
    t.write_headers_continue(1);
    assert_eq!(t.ids(), [1, 3], "the block is still open");
    t.write_continuation_end(3);
    assert_eq!(
        t.ids(),
        [1, 3],
        "another stream's continuation is not its end"
    );
    t.write_continuation_end(1);
    assert_eq!(t.ids(), [3]);
}

#[test]
fn a_goaway_drops_the_entries_above_its_last_stream_id_and_filters_later_requests() {
    let mut t = table(100);
    for sid in [1, 3, 5, 7] {
        t.read_headers(sid);
    }
    t.goaway(3);
    assert_eq!(t.ids(), [1, 3]);
    // h2 ignores HEADERS above the last stream id: they raise the highest seen id and no more.
    assert_eq!(t.read_headers(9), ParseOutcome::Ignored);
    assert_eq!(t.read_headers(11), ParseOutcome::Ignored);
    assert_eq!(t.ids(), [1, 3]);
    // A later, higher GOAWAY does not relax the filter.
    t.goaway(0x7fff_ffff);
    assert_eq!(t.read_headers(13), ParseOutcome::Ignored);
}

#[test]
fn a_flood_of_requests_after_a_goaway_never_grows_the_table() {
    let mut t = table(100);
    t.read_headers(1);
    t.goaway(1);
    for n in 0..10_000u32 {
        assert_eq!(t.read_headers(3 + 2 * n), ParseOutcome::Ignored);
    }
    assert_eq!(t.len(), 1);
}

#[test]
fn an_insert_past_the_cap_inserts_nothing_and_reports_overflow_once() {
    let mut t = table(3);
    for sid in [1, 3, 5] {
        assert!(matches!(t.read_headers(sid), ParseOutcome::Inserted { .. }));
    }
    assert_eq!(t.read_headers(7), ParseOutcome::Overflow);
    assert_eq!(t.len(), 3, "the table never exceeds its cap");
    // After the overflow the table inserts nothing, whatever room it later has.
    t.read_reset(1);
    assert_eq!(t.read_headers(9), ParseOutcome::Ignored);
    assert_eq!(t.ids(), [3, 5]);
}

#[test]
fn the_half_cap_warning_is_due_once() {
    let mut t = table(10);
    let mut due = Vec::new();
    for sid in (1..=19).step_by(2) {
        if let ParseOutcome::Inserted { len, past_half } = t.read_headers(sid) {
            if past_half {
                due.push(len);
            }
        }
    }
    assert_eq!(due, [6], "first past half of 10 is the sixth entry");
}

// ---------------------------------------------------------------------------------------------
// Final and Queued: registration, hand-over and the stream's own progress
// ---------------------------------------------------------------------------------------------

/// The window the progress tests run at.
const WINDOW: Duration = Duration::from_secs(10);

/// The connection unit's floor the progress tests run at.
const FLOOR: u64 = 100;

/// A table with `sid` parsed, registered final and handed over `handed` bytes at `now`.
fn queued(sid: u32, handed: u64, now: Instant) -> StreamTable {
    let mut t = table(100);
    t.read_headers(sid);
    t.register_final(sid);
    t.hand_over(sid, handed, now);
    t
}

/// The second `n` of a test's timeline.
fn at(base: Instant, n: u64) -> Instant {
    base + Duration::from_secs(n)
}

#[test]
fn registration_only_upgrades_an_open_entry_and_never_creates_one() {
    let mut t = table(100);
    t.register_final(1);
    assert!(t.ids().is_empty(), "registration created an entry");
    t.read_headers(1);
    t.register_final(1);
    assert_eq!(t.snapshot(), [(1, StreamState::Final)]);
    t.register_final(1);
    assert_eq!(t.snapshot(), [(1, StreamState::Final)]);
}

#[test]
fn a_hand_over_queues_a_final_entry_and_leaves_open_and_absent_ones_alone() {
    let base = Instant::now();
    let mut t = table(100);
    t.read_headers(1);
    t.read_headers(3);
    t.register_final(3);
    t.hand_over(1, 500, base);
    t.hand_over(3, 500, base);
    t.hand_over(5, 500, base);
    let states = t.snapshot();
    assert_eq!(
        states[0],
        (1, StreamState::Open),
        "an open entry stays open"
    );
    assert!(
        matches!(states[1], (3, StreamState::Queued { since, .. }) if since == base),
        "{states:?}"
    );
    assert_eq!(states.len(), 2, "a hand-over created an entry");
}

#[test]
fn a_final_entry_is_never_aged() {
    let base = Instant::now();
    let mut t = table(100);
    t.read_headers(1);
    t.register_final(1);
    for n in [0, 10, 100, 10_000] {
        assert_eq!(t.scan(at(base, n), WINDOW, FLOOR), None, "at {n} s");
    }
}

#[test]
fn a_reader_that_drains_at_least_the_floor_per_window_is_never_cut_until_its_tail_is_gone() {
    let base = Instant::now();
    let mut t = queued(1, 1000, base);
    for window in 1..=9u64 {
        t.data_written(1, FLOOR);
        assert_eq!(
            t.scan(at(base, window * 10), WINDOW, FLOOR),
            None,
            "window {window}"
        );
    }
    // 900 bytes written; the last 100 go out with the END_STREAM frame, which removes the entry.
    t.data_written(1, FLOOR);
    t.write_ended(1);
    assert_eq!(t.scan(at(base, 200), WINDOW, FLOOR), None);
    assert!(t.ids().is_empty());
}

#[test]
fn a_reader_that_drains_one_byte_per_window_is_cut_at_the_end_of_the_first_window() {
    let base = Instant::now();
    let mut t = queued(1, 1000, base);
    t.data_written(1, 1);
    assert_eq!(
        t.scan(at(base, 9), WINDOW, FLOOR),
        None,
        "inside the window"
    );
    let stall = t
        .scan(at(base, 10), WINDOW, FLOOR)
        .expect("a one byte per window reader was not cut");
    assert_eq!(stall.age, WINDOW);
    assert_eq!(stall.entries, 1);
}

#[test]
fn a_tail_smaller_than_the_floor_drained_completely_inside_the_window_is_not_cut() {
    let base = Instant::now();
    let mut t = queued(1, 60, base);
    t.data_written(1, 60);
    // The whole tail is written (60 = min(100, 60)); only its END_STREAM frame is outstanding.
    assert_eq!(t.scan(at(base, 10), WINDOW, FLOOR), None);
    assert_eq!(t.scan(at(base, 100), WINDOW, FLOOR), None);
}

#[test]
fn a_tail_smaller_than_the_floor_drained_only_in_part_is_cut_when_the_window_ends() {
    let base = Instant::now();
    let mut t = queued(1, 60, base);
    t.data_written(1, 30);
    assert_eq!(t.scan(at(base, 9), WINDOW, FLOOR), None);
    assert!(t.scan(at(base, 10), WINDOW, FLOOR).is_some());
}

#[test]
fn another_streams_data_never_refreshes_an_entry() {
    let base = Instant::now();
    let mut t = queued(1, 1000, base);
    t.read_headers(3);
    for n in 1..=9 {
        t.data_written(3, 10_000);
        assert_eq!(t.scan(at(base, n), WINDOW, FLOOR), None);
    }
    t.data_written(3, 10_000);
    assert!(
        t.scan(at(base, 10), WINDOW, FLOOR).is_some(),
        "a sibling's DATA kept a stalled entry alive"
    );
}

#[test]
fn the_threshold_is_the_floor_the_caller_passes() {
    // 100 bytes written over a window: progress against a floor of 50, a stall against 200.
    for (floor, stalls) in [(50u64, false), (200, true)] {
        let base = Instant::now();
        let mut t = queued(1, 1000, base);
        t.data_written(1, 100);
        assert_eq!(
            t.scan(at(base, 10), WINDOW, floor).is_some(),
            stalls,
            "floor {floor}"
        );
    }
}

#[test]
fn a_floor_that_is_no_multiple_of_a_chunk_is_judged_to_the_byte() {
    // A floor of 1,001 bytes is no multiple of a 16 KiB chunk or of any frame size the writer
    // emits: 1,000 bytes written over the window are a stall and 1,001 are progress, written one
    // byte at a time and in one piece alike.
    const ODD_FLOOR: u64 = 1_001;
    for (written, stalls) in [(ODD_FLOOR - 1, true), (ODD_FLOOR, false)] {
        for one_byte_writes in [false, true] {
            let base = Instant::now();
            let mut t = queued(1, 5_000, base);
            if one_byte_writes {
                for _ in 0..written {
                    t.data_written(1, 1);
                }
            } else {
                t.data_written(1, written);
            }
            assert_eq!(
                t.scan(at(base, 10), WINDOW, ODD_FLOOR).is_some(),
                stalls,
                "{written} bytes written, one-byte writes {one_byte_writes}"
            );
        }
    }
}

#[test]
fn a_new_window_starts_when_the_threshold_is_reached_and_its_target_follows_what_remains() {
    let base = Instant::now();
    let mut t = queued(1, 150, base);
    // 100 of 150 written: the threshold min(100, 150) is reached, a new window begins at 5 s with
    // 50 bytes remaining, so its target is min(100, 50) = 50.
    t.data_written(1, 100);
    assert_eq!(t.scan(at(base, 5), WINDOW, FLOOR), None);
    t.data_written(1, 49);
    assert!(
        t.scan(at(base, 15), WINDOW, FLOOR).is_some(),
        "49 of the 50 remaining bytes made the window's target"
    );
    let mut ok = queued(1, 150, base);
    ok.data_written(1, 100);
    assert_eq!(ok.scan(at(base, 5), WINDOW, FLOOR), None);
    ok.data_written(1, 50);
    assert_eq!(ok.scan(at(base, 15), WINDOW, FLOOR), None);
}

#[test]
fn an_entry_with_nothing_left_to_write_is_always_at_its_threshold() {
    // Only its END_STREAM frame is outstanding: it waits on the socket, which the connection unit
    // bounds, not on the peer's credit.
    let base = Instant::now();
    let mut t = queued(1, 0, base);
    assert_eq!(t.scan(at(base, 1000), WINDOW, FLOOR), None);
}

/// One wire or body event of the ordering test, all on stream 1.
#[derive(Debug, Clone, Copy)]
enum Event {
    /// The request HEADERS frame header is parsed.
    Parse,
    /// The response body returns its END_STREAM-bearing frame.
    Final,
    /// The response body is dropped.
    Drop,
    /// The writer accepts the last byte of the response's END_STREAM frame.
    WriteEnd,
    /// The peer's RST_STREAM is parsed.
    ReadReset,
}

#[test]
fn no_order_of_wire_and_body_events_gives_an_entry_to_a_stream_after_a_terminal_event() {
    let events = [
        Event::Parse,
        Event::Final,
        Event::Drop,
        Event::WriteEnd,
        Event::ReadReset,
    ];
    let base = Instant::now();
    let mut sequences: Vec<Vec<Event>> = vec![Vec::new()];
    let mut checked = 0usize;
    for _ in 0..6 {
        let mut next = Vec::new();
        for prefix in &sequences {
            for event in events {
                let mut seq = prefix.clone();
                seq.push(event);
                next.push(seq);
            }
        }
        for seq in &next {
            // The wire cannot end a stream before it carried its request. A terminal event with no
            // request parsed at all is kept: only a table that creates entries outside a parse
            // could answer it with one.
            let first_parse = seq.iter().position(|e| matches!(e, Event::Parse));
            let first_terminal = seq
                .iter()
                .position(|e| matches!(e, Event::WriteEnd | Event::ReadReset));
            if let (Some(terminal), Some(parse)) = (first_terminal, first_parse) {
                if terminal < parse {
                    continue;
                }
            }
            let mut t = table(100);
            let mut terminal = false;
            for event in seq {
                match event {
                    Event::Parse => {
                        t.read_headers(1);
                    }
                    Event::Final => t.register_final(1),
                    Event::Drop => t.hand_over(1, 100, base),
                    Event::WriteEnd => {
                        t.write_ended(1);
                        terminal = true;
                    }
                    Event::ReadReset => {
                        t.read_reset(1);
                        terminal = true;
                    }
                }
                assert!(
                    !terminal || t.ids().is_empty(),
                    "an entry exists after a terminal event in {seq:?}"
                );
            }
            checked += 1;
        }
        sequences = next;
    }
    assert_eq!(checked, 11_928, "the admissible sequences changed");
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "a GOAWAY purge removed a stream past Open")]
fn a_goaway_purge_that_reaches_a_stream_past_open_trips_its_assertion() {
    let mut t = table(100);
    t.read_headers(3);
    t.register_final(3);
    t.goaway(1);
}

// ---------------------------------------------------------------------------------------------
// A tunnel's backlog against the wire
// ---------------------------------------------------------------------------------------------

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::Wake;

/// A waker that counts its wakes.
struct Count(AtomicUsize);

impl Wake for Count {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// A counting waker and its count.
fn counting() -> (Arc<Count>, Waker) {
    let count = Arc::new(Count(AtomicUsize::new(0)));
    (count.clone(), Waker::from(count))
}

#[test]
fn a_tunnel_within_the_cap_of_the_wire_is_not_parked() {
    let mut t = table(100);
    t.read_headers(1);
    let (count, waker) = counting();
    assert_eq!(
        t.backlog(1, 10_000, 10_000, &waker),
        Backlog::Within,
        "exactly the cap"
    );
    assert!(t.data_written(1, 5).is_none(), "nothing was parked");
    assert_eq!(count.0.load(Ordering::SeqCst), 0);
}

#[test]
fn a_tunnel_over_the_cap_parks_its_waker_and_a_write_returns_it() {
    let mut t = table(100);
    t.read_headers(1);
    let (_count, waker) = counting();
    assert_eq!(t.backlog(1, 10_001, 10_000, &waker), Backlog::Over);
    let parked = t
        .data_written(1, 1)
        .expect("the parked waker comes back with the write");
    assert!(parked.will_wake(&waker));
    assert!(t.data_written(1, 1).is_none(), "the waker is returned once");
    assert_eq!(
        t.backlog(1, 10_001, 10_000, &waker),
        Backlog::Within,
        "2 written, 9,999 ahead"
    );
}

#[test]
fn the_check_and_the_parking_are_one_step() {
    // A write that lands between the application's read of the total and its parking would be
    // lost if they were two steps; as one, a backlog that has already shrunk is never parked.
    let mut t = table(100);
    t.read_headers(1);
    let (_count, waker) = counting();
    t.data_written(1, 5_000);
    assert_eq!(t.backlog(1, 15_000, 10_000, &waker), Backlog::Within);
    assert!(
        t.data_written(1, 1).is_none(),
        "a Within answer parks nothing"
    );
}

#[test]
fn a_second_poll_by_the_same_task_keeps_one_waker() {
    let mut t = table(100);
    t.read_headers(1);
    let (_count, waker) = counting();
    assert_eq!(t.backlog(1, 20_000, 10_000, &waker), Backlog::Over);
    assert_eq!(t.backlog(1, 20_000, 10_000, &waker), Backlog::Over);
    assert!(t.data_written(1, 1).is_some());
    assert!(t.data_written(1, 1).is_none());
}

#[test]
fn the_end_of_a_stream_returns_the_parked_waker_on_every_removal_path() {
    for end in 0..3 {
        let mut t = table(100);
        t.read_headers(1);
        let (_count, waker) = counting();
        assert_eq!(t.backlog(1, 20_000, 10_000, &waker), Backlog::Over);
        let parked = match end {
            0 => t.read_reset(1),
            1 => t.write_ended(1),
            _ => {
                t.write_headers_continue(1);
                t.write_continuation_end(1)
            }
        };
        assert!(parked.is_some(), "removal path {end}");
        assert_eq!(
            t.backlog(1, 20_000, 10_000, &waker),
            Backlog::Within,
            "the entry is gone"
        );
    }
}

#[test]
fn a_stream_with_no_entry_is_never_parked() {
    let mut t = table(100);
    let (_count, waker) = counting();
    assert_eq!(t.backlog(7, u64::MAX, 0, &waker), Backlog::Within);
}

#[test]
fn another_streams_writes_do_not_release_a_parked_tunnel() {
    let mut t = table(100);
    t.read_headers(1);
    t.read_headers(3);
    let (_count, waker) = counting();
    assert_eq!(t.backlog(1, 20_000, 10_000, &waker), Backlog::Over);
    assert!(
        t.data_written(3, 1_000_000).is_none(),
        "stream 3's DATA is not stream 1's progress"
    );
    assert_eq!(t.backlog(1, 20_000, 10_000, &waker), Backlog::Over);
}
