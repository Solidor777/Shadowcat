//! The cut counters, the write-side counting rules, the frame parser's cost and splits, and the
//! sliding sampler.

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
    for unit in [StallUnit::Stream, StallUnit::Connection] {
        let counters = StallCounters::default();
        counters.record_stall(unit);
        let expected = match unit {
            StallUnit::Stream => StallCuts {
                stream: 1,
                connection: 0,
            },
            StallUnit::Connection => StallCuts {
                stream: 0,
                connection: 1,
            },
        };
        assert_eq!(counters.snapshot(), expected, "{unit:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// The frame parser
// ---------------------------------------------------------------------------------------------

/// One HTTP/2 frame of `kind` with `payload`.
fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
    let n = payload.len() as u32;
    let mut out = vec![
        (n >> 16) as u8,
        (n >> 8) as u8,
        n as u8,
        kind,
        0,
        0,
        0,
        0,
        1,
    ];
    out.extend_from_slice(payload);
    out
}

/// A DATA frame with `n` payload bytes.
fn data_frame(n: usize) -> Vec<u8> {
    frame(0, &vec![0xAB; n])
}

#[test]
fn a_data_frame_counts_its_payload_split_at_every_header_offset() {
    let bytes = data_frame(100);
    for split in 0..=bytes.len() {
        let mut parser = FrameParser::default();
        let counted =
            parser.data_payload_in(&bytes[..split]) + parser.data_payload_in(&bytes[split..]);
        assert_eq!(counted, 100, "split at {split}");
    }
}

#[test]
fn frames_that_are_not_data_count_nothing() {
    for kind in [1u8, 3, 4, 6, 7, 8, 9] {
        let mut parser = FrameParser::default();
        assert_eq!(
            parser.data_payload_in(&frame(kind, &[1; 50])),
            0,
            "type {kind}"
        );
    }
}

#[test]
fn zero_length_data_and_back_to_back_frames_are_walked_by_their_headers() {
    let mut bytes = data_frame(0);
    bytes.extend(frame(4, &[0; 6]));
    bytes.extend(data_frame(7));
    bytes.extend(data_frame(0));
    bytes.extend(data_frame(1));
    let mut parser = FrameParser::default();
    assert_eq!(parser.data_payload_in(&bytes), 8);
}

#[test]
fn frames_over_the_default_size_count_their_whole_length_field() {
    for n in [16 * 1024, 20_000, 1 << 20] {
        let mut parser = FrameParser::default();
        assert_eq!(parser.data_payload_in(&data_frame(n)), n as u64, "{n}");
    }
}

#[test]
fn a_padded_data_frame_counts_its_length_field() {
    // The pad length byte and the padding are part of the length field: at most 255 + 1 bytes over.
    let mut padded = vec![0, 0, 12, 0, 0x8, 0, 0, 0, 1, 5];
    padded.extend_from_slice(&[0xCD; 11]);
    let mut parser = FrameParser::default();
    assert_eq!(parser.data_payload_in(&padded), 12);
}

#[test]
fn the_parser_takes_a_step_per_frame_part_and_not_per_byte() {
    let mut parser = FrameParser::default();
    assert_eq!(parser.data_payload_in(&data_frame(1 << 20)), 1 << 20);
    assert!(
        parser.steps <= 2,
        "a one-mebibyte frame took {} steps",
        parser.steps
    );
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
