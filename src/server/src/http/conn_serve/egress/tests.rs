//! The cut counters, the write-side counting rules, the stream table kept in step with the wire
//! from both directions, and the sliding sampler.

use std::io::IoSlice;

use super::*;

// ---------------------------------------------------------------------------------------------
// Counters
// ---------------------------------------------------------------------------------------------

#[test]
fn a_fresh_counter_set_reports_no_cuts() {
    assert_eq!(StallCounters::default().snapshot(), StallCuts::default());
}

#[test]
fn recording_a_cut_moves_exactly_its_own_counter_by_one() {
    for unit in [
        StallUnit::Stream,
        StallUnit::Connection,
        StallUnit::EndedUnflushed,
        StallUnit::TrackerOverflow,
    ] {
        let counters = StallCounters::default();
        counters.record_stall(unit);
        let expected = match unit {
            StallUnit::Stream => StallCuts {
                stream: 1,
                ..StallCuts::default()
            },
            StallUnit::Connection => StallCuts {
                connection: 1,
                ..StallCuts::default()
            },
            StallUnit::EndedUnflushed => StallCuts {
                ended_unflushed: 1,
                ..StallCuts::default()
            },
            StallUnit::TrackerOverflow => StallCuts {
                tracker_overflow: 1,
                ..StallCuts::default()
            },
        };
        assert_eq!(counters.snapshot(), expected, "{unit:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// Frame builders
// ---------------------------------------------------------------------------------------------

/// One HTTP/2 frame of `kind` with `flags` on `sid` and `payload`.
fn frame_on(kind: u8, flags: u8, sid: u32, payload: &[u8]) -> Vec<u8> {
    let n = payload.len() as u32;
    let mut out = vec![(n >> 16) as u8, (n >> 8) as u8, n as u8, kind, flags];
    out.extend_from_slice(&sid.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// One frame of `kind` with no flags on stream 1.
fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
    frame_on(kind, 0, 1, payload)
}

/// A DATA frame with `n` payload bytes on stream 1.
fn data_frame(n: usize) -> Vec<u8> {
    frame(FRAME_TYPE_DATA, &vec![0xAB; n])
}

// ---------------------------------------------------------------------------------------------
// Counting what a write accepted
// ---------------------------------------------------------------------------------------------

fn counter(is_h2: bool) -> (WriteCounter, Arc<Egress>) {
    let egress = Arc::new(Egress::default());
    (WriteCounter::new(egress.clone(), is_h2), egress)
}

#[test]
fn an_http1_writer_counts_every_accepted_byte() {
    let (mut counter, egress) = counter(false);
    counter.after_write(&Poll::Ready(Ok(10)), &[IoSlice::new(&[0; 10])]);
    counter.after_write(&Poll::Ready(Ok(3)), &[IoSlice::new(&[0; 10])]);
    assert_eq!(egress.accepted(), 13);
}

#[test]
fn an_http2_writer_counts_only_data_payload_and_not_other_frames() {
    let (mut counter, egress) = counter(true);
    let mut bytes = frame(4, &[0; 6]);
    bytes.extend(frame(6, &[0; 8]));
    bytes.extend(data_frame(40));
    counter.after_write(&Poll::Ready(Ok(bytes.len())), &[IoSlice::new(&bytes)]);
    assert_eq!(egress.accepted(), 40);
}

#[test]
fn a_vectored_write_counts_the_bytes_accepted_and_not_the_bytes_offered() {
    let (mut counter, egress) = counter(true);
    let mut first = data_frame(30);
    first.extend(data_frame(30));
    let second = data_frame(30);
    let offered = [IoSlice::new(&first), IoSlice::new(&second)];
    // The writer accepts all of the first slice and 20 bytes of the second: the second frame's
    // header (9 bytes) and 11 of its 30 payload bytes. It stops mid-frame and mid-slice.
    let accepted = first.len() + 20;
    counter.after_write(&Poll::Ready(Ok(accepted)), &offered);
    assert_eq!(egress.accepted(), 60 + 11);
    // The rest of the offered bytes continues where the writer stopped.
    let rest = &second[20..];
    counter.after_write(&Poll::Ready(Ok(rest.len())), &[IoSlice::new(rest)]);
    assert_eq!(egress.accepted(), 90, "30 + 30 + 30 payload bytes in all");
}

#[test]
fn a_header_split_across_io_slices_is_still_one_header() {
    let (mut counter, egress) = counter(true);
    let bytes = data_frame(21);
    let (a, rest) = bytes.split_at(4);
    let (b, c) = rest.split_at(3);
    counter.after_write(
        &Poll::Ready(Ok(bytes.len())),
        &[IoSlice::new(a), IoSlice::new(b), IoSlice::new(c)],
    );
    assert_eq!(egress.accepted(), 21);
}

#[test]
fn the_parked_level_follows_the_last_write_side_poll() {
    let (mut counter, egress) = counter(false);
    assert!(!egress.parked());
    counter.after_write(&Poll::Pending, &[]);
    assert!(egress.parked(), "a pending write parks the writer");
    counter.after_flush(&Poll::Pending);
    assert!(egress.parked(), "a pending flush keeps it parked");
    counter.after_write(&Poll::Ready(Err(io::Error::other("x"))), &[]);
    assert!(egress.parked(), "an error does not unpark");
    counter.after_flush(&Poll::Ready(Ok(())));
    assert!(!egress.parked(), "a completed flush unparks");
    counter.after_flush(&Poll::Pending);
    counter.after_write(&Poll::Ready(Ok(1)), &[IoSlice::new(&[0])]);
    assert!(!egress.parked(), "an accepted write unparks");
}

// ---------------------------------------------------------------------------------------------
// The sliding sampler
// ---------------------------------------------------------------------------------------------

/// A sampler at window 8 s, tick 2 s (a ring of five), floor 100 bytes, and the base instant.
fn sampler() -> (Sampler, Instant) {
    (
        Sampler::new(Duration::from_secs(8), Duration::from_secs(2), 100),
        Instant::now(),
    )
}

/// The second `n` of the sampler's timeline.
fn at(base: Instant, n: u64) -> Instant {
    base + Duration::from_secs(n)
}

#[test]
fn a_parked_writer_with_flat_bytes_fires_when_the_ring_is_full_and_spans_the_window() {
    let (mut s, base) = sampler();
    for n in [0, 2, 4, 6] {
        assert!(s.observe(at(base, n), 500, true).is_none(), "tick at {n}");
    }
    let stall = s
        .observe(at(base, 8), 500, true)
        .expect("fires at the full window");
    assert_eq!(stall.span, Duration::from_secs(8));
    assert_eq!(stall.accepted, 0);
}

#[test]
fn progress_of_at_least_the_floor_inside_the_ring_delays_the_fire() {
    let (mut s, base) = sampler();
    let mut accepted = 0;
    for n in [0, 2, 4, 6, 8] {
        accepted += 25;
        // 100 bytes over the ring's four intervals: exactly the floor, which is progress.
        let fired = s.observe(at(base, n), accepted, true);
        assert!(
            fired.is_none(),
            "tick at {n} fired with progress at the floor"
        );
    }
    // The next tick adds nothing: the ring now holds 75 bytes of progress, below the floor.
    assert!(s.observe(at(base, 10), accepted, true).is_some());
}

#[test]
fn a_stop_that_begins_mid_window_after_progress_fires_within_a_window_plus_a_tick() {
    let (mut s, base) = sampler();
    // Steady progress, then a stop right after the sample at 20 s.
    let mut accepted = 0;
    for n in (0..=20).step_by(2) {
        accepted += 1000;
        assert!(s.observe(at(base, n), accepted, true).is_none());
    }
    let mut fired_at = None;
    for n in (22..=40).step_by(2) {
        if s.observe(at(base, n), accepted, true).is_some() {
            fired_at = Some(n);
            break;
        }
    }
    let n = fired_at.expect("a stalled writer was never cut");
    // The stop is at 20 s; a fixed window anchored at 0 would not fire before 24 s and could fire
    // up to a window plus a tick later. The sliding ring fires once its oldest sample is a flat one.
    assert_eq!(n, 28, "fired at {n} s");
    assert!(n - 20 <= 8 + 2);
}

#[test]
fn exactly_the_floor_over_the_span_is_progress_and_one_byte_less_is_a_stall() {
    for (progress, stalls) in [(100u64, false), (99, true)] {
        let (mut s, base) = sampler();
        let mut fired = false;
        for n in [0u64, 2, 4, 6, 8] {
            let accepted = if n == 8 { progress } else { 0 };
            fired = s.observe(at(base, n), accepted, true).is_some();
        }
        assert_eq!(fired, stalls, "{progress} bytes over the window");
    }
}

#[test]
fn a_sample_that_saw_the_writer_unparked_anywhere_in_the_ring_prevents_the_fire() {
    for unparked in 0..5 {
        let (mut s, base) = sampler();
        let mut fired = false;
        for (i, n) in [0u64, 2, 4, 6, 8].into_iter().enumerate() {
            fired |= s.observe(at(base, n), 0, i != unparked).is_some();
        }
        assert!(
            !fired,
            "an unparked sample at index {unparked} did not prevent the fire"
        );
    }
}

#[test]
fn a_level_that_stays_parked_counts_across_every_tick_without_the_writer_being_polled() {
    // The writer is never re-polled, so its parked level is set once; each tick reads the level.
    let egress = Egress::default();
    let (mut s, base) = sampler();
    egress.write_parked.store(true, Ordering::Relaxed);
    let mut fired = false;
    for n in [0u64, 2, 4, 6, 8] {
        fired = s
            .observe(at(base, n), egress.accepted(), egress.parked())
            .is_some();
    }
    assert!(fired, "a level that stays parked must count at every tick");
}

#[test]
fn ticks_bunched_by_a_runtime_stall_do_not_fill_the_ring_into_a_premature_fire() {
    let (mut s, base) = sampler();
    // Five samples within milliseconds: the ring is full but spans nothing like the window.
    for ms in 0..5 {
        let fired = s.observe(base + Duration::from_millis(ms), 0, true);
        assert!(fired.is_none(), "fired on {ms} ms of evidence");
    }
}

#[test]
fn a_window_shorter_than_the_tick_still_keeps_a_ring_with_a_span() {
    let mut s = Sampler::new(Duration::ZERO, Duration::from_millis(1), 100);
    let base = Instant::now();
    assert!(s.observe(base, 0, true).is_none(), "one sample has no span");
    assert!(s
        .observe(base + Duration::from_millis(1), 0, true)
        .is_some());
}

// ---------------------------------------------------------------------------------------------
// The stream table kept in step with the wire
// ---------------------------------------------------------------------------------------------

/// The client connection preface.
const PREFACE: &[u8; 24] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

/// One HTTP/2 connection's IO bookkeeping over a table of `cap` entries.
struct Wire {
    /// The connection's record.
    egress: Arc<Egress>,
    /// The serve loop's cut counters.
    counters: Arc<StallCounters>,
    /// The connection's kill switch.
    kill: Arc<ConnKill>,
    /// What the writer accepted.
    writes: WriteCounter,
    /// What the codec read.
    reads: ReadObserver,
}

impl Wire {
    /// A connection whose table holds at most `cap` entries.
    fn new(cap: usize) -> Self {
        let counters = Arc::new(StallCounters::default());
        let egress = Arc::new(Egress::new(counters.clone(), cap));
        let writes = WriteCounter::new(egress.clone(), true);
        let reads = writes.read_observer();
        Self {
            egress,
            counters,
            kill: ConnKill::new(),
            writes,
            reads,
        }
    }

    /// The codec reads `bytes`.
    fn read(&mut self, bytes: &[u8]) {
        self.reads.after_read(bytes, &self.kill);
    }

    /// The writer accepts all of `bytes`.
    fn write(&mut self, bytes: &[u8]) {
        self.writes
            .after_write(&Poll::Ready(Ok(bytes.len())), &[IoSlice::new(bytes)]);
    }

    /// The stream ids with an entry.
    fn ids(&self) -> Vec<u32> {
        self.egress.stream_ids()
    }

    /// Whether the kill switch has fired.
    fn killed(&self) -> bool {
        use futures_util::FutureExt;
        self.kill.fired().now_or_never().is_some()
    }
}

/// A request HEADERS frame (END_STREAM | END_HEADERS) on `sid`.
fn request(sid: u32) -> Vec<u8> {
    frame_on(
        FRAME_TYPE_HEADERS,
        FLAG_END_STREAM | FLAG_END_HEADERS,
        sid,
        &[0x82, 0x87, 0x84],
    )
}

/// A peer RST_STREAM on `sid`.
fn peer_reset(sid: u32) -> Vec<u8> {
    frame_on(FRAME_TYPE_RST_STREAM, 0, sid, &[0, 0, 0, 8])
}

/// A server GOAWAY with `last` as its last stream id.
fn goaway(last: u32) -> Vec<u8> {
    let mut payload = last.to_be_bytes().to_vec();
    payload.extend_from_slice(&[0, 0, 0, 0]);
    frame_on(7, 0, 0, &payload)
}

#[test]
fn requests_read_after_the_preface_create_entries_whatever_the_read_boundaries() {
    let mut bytes = PREFACE.to_vec();
    bytes.extend(request(1));
    bytes.extend(request(3));
    for split in 0..=bytes.len() {
        let mut wire = Wire::new(100);
        wire.read(&bytes[..split]);
        wire.read(&bytes[split..]);
        assert_eq!(wire.ids(), [1, 3], "split at {split}");
    }
}

#[test]
fn a_peer_reset_removes_the_entry_and_a_data_frame_without_end_stream_does_not() {
    let mut wire = Wire::new(100);
    wire.read(PREFACE);
    wire.read(&request(1));
    wire.read(&request(3));
    wire.write(&data_frame(10));
    assert_eq!(wire.ids(), [1, 3], "DATA without END_STREAM ends nothing");
    wire.read(&peer_reset(1));
    assert_eq!(wire.ids(), [3]);
}

#[test]
fn a_response_ends_its_entry_only_when_the_last_byte_of_its_end_stream_frame_is_accepted() {
    let mut wire = Wire::new(100);
    wire.read(PREFACE);
    wire.read(&request(1));
    let end = frame_on(FRAME_TYPE_DATA, FLAG_END_STREAM, 1, &[7; 20]);
    wire.write(&end[..end.len() - 1]);
    assert_eq!(wire.ids(), [1], "one byte short of the frame's end");
    wire.write(&end[end.len() - 1..]);
    assert!(wire.ids().is_empty());
}

#[test]
fn a_header_block_with_end_stream_ends_the_entry_at_the_continuation_that_ends_the_block() {
    let mut wire = Wire::new(100);
    wire.read(PREFACE);
    wire.read(&request(1));
    wire.write(&frame_on(FRAME_TYPE_HEADERS, FLAG_END_STREAM, 1, &[0x88]));
    assert_eq!(wire.ids(), [1], "the block continues");
    wire.write(&frame_on(
        FRAME_TYPE_CONTINUATION,
        FLAG_END_HEADERS,
        1,
        &[0x88],
    ));
    assert!(wire.ids().is_empty());
}

#[test]
fn a_reset_the_server_writes_ends_the_entry() {
    let mut wire = Wire::new(100);
    wire.read(PREFACE);
    wire.read(&request(1));
    wire.write(&frame_on(FRAME_TYPE_RST_STREAM, 0, 1, &[0, 0, 0, 8]));
    assert!(wire.ids().is_empty());
}

#[test]
fn a_goaway_the_server_writes_drops_the_entries_above_its_last_stream_id() {
    let mut wire = Wire::new(100);
    wire.read(PREFACE);
    for sid in [1, 3, 5] {
        wire.read(&request(sid));
    }
    wire.write(&goaway(1));
    assert_eq!(wire.ids(), [1]);
}

#[test]
fn a_request_read_before_a_goaway_is_written_is_purged_and_one_read_after_it_is_never_created() {
    // h2 lowers its own last stream id when the PONG arrives while the GOAWAY is still buffered, so
    // a client write [PONG, HEADERS(3)] is parsed off the wire before the GOAWAY is written.
    let mut before = Wire::new(100);
    before.read(PREFACE);
    before.read(&request(1));
    before.read(&request(3));
    assert_eq!(before.ids(), [1, 3]);
    before.write(&goaway(1));
    assert_eq!(
        before.ids(),
        [1],
        "the purge removes what the read side created"
    );

    let mut after = Wire::new(100);
    after.read(PREFACE);
    after.read(&request(1));
    after.write(&goaway(1));
    after.read(&request(3));
    assert_eq!(
        after.ids(),
        [1],
        "the filter refuses what the purge cannot reach"
    );
}

#[test]
fn an_http1_connection_has_no_table_and_reads_nothing_as_frames() {
    let egress = Arc::new(Egress::default());
    let writes = WriteCounter::new(egress.clone(), false);
    let mut reads = writes.read_observer();
    let kill = ConnKill::new();
    reads.after_read(&request(1), &kill);
    reads.after_read(&request(3), &kill);
    assert!(egress.stream_ids().is_empty());
}

#[test]
fn a_table_that_outgrows_its_cap_cuts_the_connection_once_and_stops_inserting() {
    let mut wire = Wire::new(3);
    wire.read(PREFACE);
    let mut burst = Vec::new();
    for sid in [1, 3, 5, 7, 9] {
        burst.extend(request(sid));
    }
    wire.read(&burst);
    assert!(wire.killed(), "the overflow did not cut the connection");
    assert_eq!(
        wire.counters.snapshot(),
        StallCuts {
            tracker_overflow: 1,
            ..StallCuts::default()
        }
    );
    assert_eq!(wire.ids(), [1, 3, 5], "nothing past the cap is inserted");
    wire.read(&request(11));
    assert_eq!(wire.counters.snapshot().tracker_overflow, 1, "counted once");
    assert_eq!(wire.counters.table_peak(), 3);
}

#[test]
fn an_overflow_on_a_connection_the_drain_already_ended_is_not_counted_as_a_cut() {
    let mut wire = Wire::new(1);
    assert!(wire.kill.fire(KillReason::Drain));
    wire.read(PREFACE);
    wire.read(&request(1));
    wire.read(&request(3));
    assert_eq!(wire.counters.snapshot(), StallCuts::default());
    assert_eq!(wire.ids(), [1], "the table still fails closed");
}

#[test]
fn the_peak_is_the_longest_the_table_ever_was() {
    let mut wire = Wire::new(100);
    wire.read(PREFACE);
    for sid in [1, 3, 5] {
        wire.read(&request(sid));
    }
    for sid in [1, 3, 5] {
        wire.read(&peer_reset(sid));
    }
    wire.read(&request(7));
    assert_eq!(wire.ids(), [7]);
    assert_eq!(wire.counters.table_peak(), 3);
}

#[test]
fn data_payload_is_attributed_to_its_own_stream_and_a_partial_accept_counts_only_what_was_accepted()
{
    let mut wire = Wire::new(100);
    wire.read(PREFACE);
    wire.read(&request(1));
    wire.read(&request(3));
    // DATA for stream 1 (30 bytes), then DATA for stream 3 (30 bytes), offered in two slices; the
    // writer accepts all of the first and 20 bytes of the second: its 9-byte header and 11 payload
    // bytes.
    let mut first = frame_on(FRAME_TYPE_DATA, 0, 1, &[1; 30]);
    first.extend(frame_on(FRAME_TYPE_DATA, 0, 3, &[3; 30]));
    let accepted = frame_on(FRAME_TYPE_DATA, 0, 1, &[1; 30]).len() + 20;
    wire.writes.after_write(
        &Poll::Ready(Ok(accepted)),
        &[
            IoSlice::new(&first[..30 + 9]),
            IoSlice::new(&first[30 + 9..]),
        ],
    );
    assert_eq!(wire.egress.stream_written(1), Some(30));
    assert_eq!(wire.egress.stream_written(3), Some(11));
    assert_eq!(
        wire.egress.accepted(),
        41,
        "the connection unit counts the same bytes"
    );
    // The rest of stream 3's frame.
    let rest = &first[accepted..];
    wire.write(rest);
    assert_eq!(wire.egress.stream_written(3), Some(30));
    assert_eq!(
        wire.egress.stream_written(7),
        None,
        "no entry for an id never parsed"
    );
}
