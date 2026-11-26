//! These tests cover the frame walker: header splits, event order, payload skipping cost, GOAWAY
//! capture and the client preface.

use super::*;

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

/// Every event `parser` reports for `bytes`.
fn events(parser: &mut FrameParser, bytes: &[u8]) -> Vec<FrameEvent> {
    let mut out = Vec::new();
    parser.feed(bytes, &mut |e| out.push(e));
    out
}

/// The DATA payload bytes among `events`.
fn data_bytes(events: &[FrameEvent]) -> u64 {
    events
        .iter()
        .map(|e| match e {
            FrameEvent::Data { bytes, .. } => *bytes,
            _ => 0,
        })
        .sum()
}

/// A stream of mixed frames: DATA with END_STREAM, an empty SETTINGS, a GOAWAY, HEADERS, RST_STREAM.
fn mixed_stream() -> Vec<u8> {
    let mut bytes = frame_on(FRAME_TYPE_DATA, FLAG_END_STREAM, 3, &[1; 40]);
    bytes.extend(frame_on(4, 0, 0, &[]));
    bytes.extend(frame_on(FRAME_TYPE_GOAWAY, 0, 0, &[0, 0, 0, 5, 0, 0, 0, 0]));
    bytes.extend(frame_on(
        FRAME_TYPE_HEADERS,
        FLAG_END_STREAM | FLAG_END_HEADERS,
        7,
        &[0x82],
    ));
    bytes.extend(frame_on(FRAME_TYPE_RST_STREAM, 0, 9, &[0, 0, 0, 8]));
    bytes
}

#[test]
fn a_data_frame_reports_its_payload_and_one_end_split_at_every_offset() {
    let bytes = frame_on(FRAME_TYPE_DATA, FLAG_END_STREAM, 5, &[0xAB; 100]);
    for split in 0..=bytes.len() {
        let mut parser = FrameParser::new();
        let mut all = events(&mut parser, &bytes[..split]);
        all.extend(events(&mut parser, &bytes[split..]));
        assert_eq!(data_bytes(&all), 100, "split at {split}");
        let ends: Vec<_> = all
            .iter()
            .filter(|e| matches!(e, FrameEvent::End { .. }))
            .collect();
        assert_eq!(
            ends,
            [&FrameEvent::End {
                kind: FRAME_TYPE_DATA,
                flags: FLAG_END_STREAM,
                sid: 5
            }],
            "split at {split}"
        );
    }
}

#[test]
fn a_frame_ends_only_when_its_last_byte_is_consumed() {
    let bytes = data_frame(30);
    let mut parser = FrameParser::new();
    let early = events(&mut parser, &bytes[..bytes.len() - 1]);
    assert!(
        !early.iter().any(|e| matches!(e, FrameEvent::End { .. })),
        "{early:?}"
    );
    let last = events(&mut parser, &bytes[bytes.len() - 1..]);
    assert_eq!(
        last,
        [
            FrameEvent::Data { sid: 1, bytes: 1 },
            FrameEvent::End {
                kind: FRAME_TYPE_DATA,
                flags: 0,
                sid: 1
            }
        ]
    );
}

#[test]
fn frames_that_are_not_data_report_no_data_bytes() {
    for kind in [1u8, 3, 4, 6, 7, 8, 9] {
        let mut parser = FrameParser::new();
        let all = events(&mut parser, &frame(kind, &[1; 50]));
        assert_eq!(data_bytes(&all), 0, "type {kind}");
    }
}

#[test]
fn a_frame_with_no_payload_starts_and_ends_at_its_header() {
    let mut parser = FrameParser::new();
    let all = events(&mut parser, &frame_on(4, 1, 0, &[]));
    assert_eq!(
        all,
        [
            FrameEvent::Start { kind: 4, sid: 0 },
            FrameEvent::End {
                kind: 4,
                flags: 1,
                sid: 0
            }
        ]
    );
}

#[test]
fn zero_length_data_and_back_to_back_frames_are_walked_by_their_headers() {
    let mut bytes = data_frame(0);
    bytes.extend(frame(4, &[0; 6]));
    bytes.extend(data_frame(7));
    bytes.extend(data_frame(0));
    bytes.extend(data_frame(1));
    let mut parser = FrameParser::new();
    let all = events(&mut parser, &bytes);
    assert_eq!(data_bytes(&all), 8);
    let ends = all
        .iter()
        .filter(|e| matches!(e, FrameEvent::End { .. }))
        .count();
    assert_eq!(ends, 5);
}

#[test]
fn frames_over_the_default_size_count_their_whole_length_field() {
    for n in [16 * 1024, 20_000, 1 << 20] {
        let mut parser = FrameParser::new();
        assert_eq!(
            data_bytes(&events(&mut parser, &data_frame(n))),
            n as u64,
            "{n}"
        );
    }
}

#[test]
fn a_padded_data_frame_counts_its_length_field() {
    // The pad length byte and the padding are part of the length field: at most 255 + 1 bytes over.
    let mut padded = frame_on(FRAME_TYPE_DATA, 0x8, 1, &[5]);
    padded.extend_from_slice(&[0xCD; 11]);
    padded[2] = 12;
    let mut parser = FrameParser::new();
    assert_eq!(data_bytes(&events(&mut parser, &padded)), 12);
}

#[test]
fn the_parser_takes_a_step_per_frame_part_and_not_per_byte() {
    let mut parser = FrameParser::new();
    assert_eq!(
        data_bytes(&events(&mut parser, &data_frame(1 << 20))),
        1 << 20
    );
    assert!(
        parser.steps <= 2,
        "a one-mebibyte frame took {} steps",
        parser.steps
    );
}

#[test]
fn feeding_one_byte_at_a_time_reports_the_same_events_as_feeding_it_whole() {
    let bytes = mixed_stream();
    let whole = events(&mut FrameParser::new(), &bytes);
    let mut piecewise = Vec::new();
    let mut parser = FrameParser::new();
    for byte in &bytes {
        parser.feed(&[*byte], &mut |e| piecewise.push(e));
    }
    // Data events are per contiguous run, so compare their total and every other event in order.
    assert_eq!(data_bytes(&piecewise), data_bytes(&whole));
    let others = |all: &[FrameEvent]| -> Vec<FrameEvent> {
        all.iter()
            .copied()
            .filter(|e| !matches!(e, FrameEvent::Data { .. }))
            .collect()
    };
    assert_eq!(others(&piecewise), others(&whole));
}

#[test]
fn a_goaway_reports_its_last_stream_id_at_every_split_just_before_its_end() {
    // The reserved bit of the last-stream-id field is masked off.
    let bytes = frame_on(
        FRAME_TYPE_GOAWAY,
        0,
        0,
        &[0x80, 0, 0, 11, 0, 0, 0, 0, b'x', b'y'],
    );
    for split in 0..=bytes.len() {
        let mut parser = FrameParser::new();
        let mut all = events(&mut parser, &bytes[..split]);
        all.extend(events(&mut parser, &bytes[split..]));
        let tail = &all[all.len() - 2..];
        assert_eq!(
            tail,
            [
                FrameEvent::GoAway(11),
                FrameEvent::End {
                    kind: FRAME_TYPE_GOAWAY,
                    flags: 0,
                    sid: 0
                }
            ],
            "split at {split}"
        );
    }
}

#[test]
fn a_goaway_with_a_payload_too_short_for_a_stream_id_reports_none() {
    let mut parser = FrameParser::new();
    let all = events(&mut parser, &frame_on(FRAME_TYPE_GOAWAY, 0, 0, &[0, 0]));
    assert!(!all.iter().any(|e| matches!(e, FrameEvent::GoAway(_))));
}

#[test]
fn a_stream_id_has_its_reserved_bit_masked_off() {
    let mut parser = FrameParser::new();
    let all = events(
        &mut parser,
        &frame_on(FRAME_TYPE_RST_STREAM, 0, 0x8000_0007, &[0, 0, 0, 8]),
    );
    assert!(all.contains(&FrameEvent::Start {
        kind: FRAME_TYPE_RST_STREAM,
        sid: 7
    }));
}

#[test]
fn the_client_preface_is_skipped_at_every_split_and_the_frames_after_it_are_walked() {
    let mut bytes = vec![b'P'; H2_PREFACE_LEN];
    bytes.extend(frame_on(FRAME_TYPE_RST_STREAM, 0, 0x2B, &[0, 0, 0, 8]));
    for split in 0..=bytes.len() {
        let mut parser = FrameParser::with_preface();
        let mut all = events(&mut parser, &bytes[..split]);
        all.extend(events(&mut parser, &bytes[split..]));
        assert_eq!(
            all,
            [
                FrameEvent::Start {
                    kind: FRAME_TYPE_RST_STREAM,
                    sid: 0x2B
                },
                FrameEvent::End {
                    kind: FRAME_TYPE_RST_STREAM,
                    flags: 0,
                    sid: 0x2B
                }
            ],
            "split at {split}"
        );
    }
}

#[test]
fn a_parser_without_a_preface_reads_the_first_byte_as_a_frame_header() {
    let mut parser = FrameParser::new();
    let all = events(&mut parser, &frame_on(FRAME_TYPE_HEADERS, 4, 3, &[0x82]));
    assert_eq!(all[0], FrameEvent::Start { kind: 1, sid: 3 });
}
