//! The stream table's wire-fed transitions: creation at parse time, removal only on wire evidence,
//! the GOAWAY filter and purge, and the fail-closed cap.

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
            0 => t.read_reset(1),
            1 => t.write_ended(1),
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

/// One wire event of the ordering test, all on stream 1.
#[derive(Debug, Clone, Copy)]
enum Wire {
    /// The request HEADERS frame header is parsed.
    Parse,
    /// The writer accepts the last byte of the response's END_STREAM frame.
    WriteEnd,
    /// The peer's RST_STREAM is parsed.
    ReadReset,
}

#[test]
fn after_any_terminal_event_no_order_of_wire_events_gives_the_stream_an_entry() {
    let events = [Wire::Parse, Wire::WriteEnd, Wire::ReadReset];
    let mut sequences: Vec<Vec<Wire>> = vec![Vec::new()];
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
            // The wire cannot end a stream it never carried.
            let first_parse = seq.iter().position(|e| matches!(e, Wire::Parse));
            let first_terminal = seq
                .iter()
                .position(|e| matches!(e, Wire::WriteEnd | Wire::ReadReset));
            if let (Some(t), Some(p)) = (first_terminal, first_parse) {
                if t < p {
                    continue;
                }
            }
            if first_terminal.is_some() && first_parse.is_none() {
                continue;
            }
            let mut t = table(100);
            let mut terminal = false;
            for event in seq {
                match event {
                    Wire::Parse => {
                        t.read_headers(1);
                    }
                    Wire::WriteEnd => {
                        t.write_ended(1);
                        terminal = true;
                    }
                    Wire::ReadReset => {
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
    assert!(checked > 100, "only {checked} admissible sequences ran");
}
