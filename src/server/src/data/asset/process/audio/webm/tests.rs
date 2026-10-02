use std::ops::Range;

use super::super::tests::synth_wav_440hz_at;
use super::super::{
    encode_opus_frames, process_staged_audio, with_suffix, AudioContainers, OpusStream,
    OPUS_FRAME_SAMPLES, OPUS_SUFFIX, OPUS_VENDOR_MARKER, WEBM_SUFFIX,
};
use super::*;

/// One parsed EBML element: its id as an integer, where its header starts and its payload range.
#[derive(Clone, Debug)]
struct Elem {
    /// Element id bytes read big endian.
    id: u32,
    /// Offset of the first id byte.
    start: usize,
    /// Payload byte range.
    payload: Range<usize>,
}

/// A big-endian unsigned integer of the bytes in `range`.
fn uint(bytes: &[u8], range: Range<usize>) -> u64 {
    bytes[range]
        .iter()
        .fold(0u64, |acc, &b| (acc << 8) | b as u64)
}

/// Parses the sibling elements packed in `bytes[range]`. Sizes are EBML vints; the unknown-size
/// marker is a test failure because the muxer never writes it.
fn children(bytes: &[u8], range: Range<usize>) -> Vec<Elem> {
    let mut out = Vec::new();
    let mut pos = range.start;
    while pos < range.end {
        let first = bytes[pos];
        let id_len = first.leading_zeros() as usize + 1;
        let id = uint(bytes, pos..pos + id_len) as u32;
        let size_pos = pos + id_len;
        let size_first = bytes[size_pos];
        let size_len = size_first.leading_zeros() as usize + 1;
        let mut size = (size_first as u64) & ((1u64 << (8 - size_len)) - 1);
        for &b in &bytes[size_pos + 1..size_pos + size_len] {
            size = (size << 8) | b as u64;
        }
        assert_ne!(
            size,
            (1u64 << (7 * size_len)) - 1,
            "unknown-size marker at {pos}"
        );
        let payload_start = size_pos + size_len;
        let payload_end = payload_start + size as usize;
        assert!(
            payload_end <= range.end,
            "element at {pos} overruns its parent"
        );
        out.push(Elem {
            id,
            start: pos,
            payload: payload_start..payload_end,
        });
        pos = payload_end;
    }
    assert_eq!(pos, range.end);
    out
}

/// The Segment's payload range, after checking the Segment size equals the bytes that follow it.
fn segment(bytes: &[u8]) -> Range<usize> {
    let top = children(bytes, 0..bytes.len());
    assert_eq!(top.len(), 2, "EBML header then Segment, nothing else");
    assert_eq!(top[0].id, 0x1A45DFA3);
    assert_eq!(top[1].id, 0x18538067);
    assert_eq!(
        top[1].payload.end,
        bytes.len(),
        "Segment length equals the bytes"
    );
    top[1].payload.clone()
}

/// Every element of one id in `elems`.
fn of(elems: &[Elem], id: u32) -> Vec<Elem> {
    elems.iter().filter(|e| e.id == id).cloned().collect()
}

/// A stream of `frames` placeholder packets (the muxer never parses packet bytes).
fn fake_stream(frames: usize, pre_skip: u16, total_valid: u64) -> OpusStream {
    OpusStream {
        packets: (0..frames).map(|i| vec![i as u8; 5]).collect(),
        pre_skip,
        total_valid,
    }
}

/// Real encoder output for `valid` mono 48 kHz samples of a sine.
fn encoded(valid: usize) -> OpusStream {
    let pcm: Vec<f32> = (0..valid)
        .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.5)
        .collect();
    encode_opus_frames(&pcm, 1).unwrap()
}

/// The residues of `total_valid` modulo one frame the loss analysis separates: 0, 649 and 959 are
/// the three that lose tail samples without the flush for a pre-skip of 312, and 648 is the last
/// residue that does not.
const RESIDUES: [usize; 6] = [0, 1, 647, 648, 649, 959];

/// A valid-sample count of about 1 s whose remainder modulo one frame is `residue`.
fn valid_with_residue(residue: usize) -> usize {
    50 * OPUS_FRAME_SAMPLES as usize + residue
}

#[test]
fn the_segment_is_a_known_size_holding_seekhead_info_tracks_cues_then_clusters() {
    let stream = encoded(valid_with_residue(1));
    let bytes = mux_webm(&stream, 1, 48_000).unwrap();
    let seg = segment(&bytes);
    let elems = children(&bytes, seg);
    let ids: Vec<u32> = elems.iter().map(|e| e.id).collect();
    assert_eq!(&ids[..4], &[0x114D9B74, 0x1549A966, 0x1654AE6B, 0x1C53BB6B]);
    assert!(
        ids[4..].iter().all(|&id| id == 0x1F43B675),
        "clusters follow Cues: {ids:x?}"
    );
    assert_eq!(ids.len(), 4 + stream.packets.len().div_ceil(64));
}

#[test]
fn duration_is_the_valid_length_in_milliseconds() {
    for residue in RESIDUES {
        let stream = encoded(valid_with_residue(residue));
        let bytes = mux_webm(&stream, 1, 48_000).unwrap();
        let seg = segment(&bytes);
        let info = of(&children(&bytes, seg.clone()), 0x1549A966).remove(0);
        let fields = children(&bytes, info.payload);
        let duration = of(&fields, 0x4489).remove(0);
        assert_eq!(duration.payload.len(), 8);
        let value = f64::from_be_bytes(bytes[duration.payload].try_into().unwrap());
        assert_eq!(value, stream.total_valid as f64 * 1000.0 / 48_000.0);
        let timecode_scale = of(&fields, 0x2AD7B1).remove(0);
        assert_eq!(uint(&bytes, timecode_scale.payload), 1_000_000);
    }
}

#[test]
fn the_writing_app_carries_the_version_marker_inside_the_head() {
    let bytes = mux_webm(&encoded(5000), 1, 48_000).unwrap();
    assert!(head_is_current(&bytes[..WEBM_HEAD_BYTES.min(bytes.len())]));
    let unmarked = bytes
        .windows(WEBM_WRITING_APP.len())
        .position(|w| w == WEBM_WRITING_APP.as_bytes())
        .unwrap();
    let mut old = bytes.clone();
    old[unmarked..unmarked + WEBM_WRITING_APP.len()].copy_from_slice(&[b'x'; 16]);
    assert!(!head_is_current(&old));
}

#[test]
fn seekhead_positions_point_at_info_tracks_and_cues() {
    let bytes = mux_webm(&encoded(valid_with_residue(1)), 1, 48_000).unwrap();
    let seg = segment(&bytes);
    let elems = children(&bytes, seg.clone());
    let head = &elems[0];
    let seeks = children(&bytes, head.payload.clone());
    assert_eq!(seeks.len(), 3);
    let mut targets = Vec::new();
    for seek in seeks {
        assert_eq!(seek.id, 0x4DBB);
        let fields = children(&bytes, seek.payload);
        let id = uint(&bytes, of(&fields, 0x53AB).remove(0).payload);
        let position = uint(&bytes, of(&fields, 0x53AC).remove(0).payload) as usize;
        let at = seg.start + position;
        let width = (id as u32).leading_zeros() as usize / 8;
        assert_eq!(
            uint(&bytes, at..at + (4 - width)),
            id,
            "SeekPosition points at the id"
        );
        targets.push(id as u32);
    }
    assert_eq!(targets, vec![0x1549A966, 0x1654AE6B, 0x1C53BB6B]);
}

#[test]
fn every_cuepoint_points_at_a_cluster_with_the_matching_timecode() {
    let stream = encoded(5 * 64 * 960 + 100);
    let bytes = mux_webm(&stream, 1, 48_000).unwrap();
    let seg = segment(&bytes);
    let elems = children(&bytes, seg.clone());
    let cues = of(&elems, 0x1C53BB6B).remove(0);
    let points = children(&bytes, cues.payload);
    let clusters = of(&elems, 0x1F43B675);
    assert_eq!(points.len(), clusters.len(), "one CuePoint per cluster");
    assert!(clusters.len() > 4);
    for (point, cluster) in points.iter().zip(&clusters) {
        let fields = children(&bytes, point.payload.clone());
        let time = uint(&bytes, of(&fields, 0xB3).remove(0).payload);
        let positions = children(&bytes, of(&fields, 0xB7).remove(0).payload);
        let track = uint(&bytes, of(&positions, 0xF7).remove(0).payload);
        let position = uint(&bytes, of(&positions, 0xF1).remove(0).payload) as usize;
        assert_eq!(track, 1);
        assert_eq!(
            seg.start + position,
            cluster.start,
            "CueClusterPosition is the cluster"
        );
        let timecode = children(&bytes, cluster.payload.clone())
            .into_iter()
            .find(|e| e.id == 0xE7)
            .unwrap();
        assert_eq!(uint(&bytes, timecode.payload), time);
    }
}

#[test]
fn codec_delay_equals_the_opus_head_pre_skip() {
    let stream = encoded(5000);
    let bytes = mux_webm(&stream, 1, 48_000).unwrap();
    let seg = segment(&bytes);
    let tracks = of(&children(&bytes, seg), 0x1654AE6B).remove(0);
    let entry = children(&bytes, tracks.payload).remove(0);
    let fields = children(&bytes, entry.payload);
    let private = of(&fields, 0x63A2).remove(0);
    let head = &bytes[private.payload];
    let head_pre_skip = u16::from_le_bytes([head[10], head[11]]) as u64;
    assert_eq!(head_pre_skip, stream.pre_skip as u64);
    let delay_ns = uint(&bytes, of(&fields, 0x56AA).remove(0).payload);
    assert_eq!(
        delay_ns * 48_000 / 1_000_000_000,
        head_pre_skip,
        "a flooring decoder recovers the pre-skip"
    );
    assert!(delay_ns >= head_pre_skip * 1_000_000_000 / 48_000);
}

/// Every `DiscardPadding` in the file as `(cluster index, group is last in its cluster, value)`,
/// plus the cluster count.
fn discard_paddings(bytes: &[u8]) -> (Vec<(usize, bool, i64)>, usize) {
    let seg = segment(bytes);
    let clusters = of(&children(bytes, seg), 0x1F43B675);
    let mut found = Vec::new();
    for (ci, cluster) in clusters.iter().enumerate() {
        let blocks = children(bytes, cluster.payload.clone());
        for (bi, block) in blocks.iter().enumerate() {
            if block.id == 0xA0 {
                for field in children(bytes, block.payload.clone()) {
                    if field.id == 0x75A2 {
                        let raw = uint(bytes, field.payload.clone()) as i64;
                        found.push((ci, bi == blocks.len() - 1, raw));
                    }
                }
            }
        }
    }
    (found, clusters.len())
}

#[test]
fn exactly_one_discard_padding_sits_in_the_last_blockgroup() {
    let mut saw_padding = 0;
    for residue in RESIDUES {
        let stream = encoded(valid_with_residue(residue));
        let bytes = mux_webm(&stream, 1, 48_000).unwrap();
        let (found, clusters) = discard_paddings(&bytes);
        let padding_samples =
            stream.packets.len() as i64 * 960 - stream.pre_skip as i64 - stream.total_valid as i64;
        assert!(padding_samples >= 0, "residue {residue}");
        if padding_samples == 0 {
            assert!(
                found.is_empty(),
                "no element for a zero padding, residue {residue}"
            );
            continue;
        }
        saw_padding += 1;
        assert_eq!(found.len(), 1, "residue {residue}");
        let (cluster, is_last, value) = found[0];
        assert_eq!(cluster, clusters - 1, "padding is in the final cluster");
        assert!(is_last, "padding is on the last block of the cluster");
        assert_eq!(
            value as u64 * 48_000 / 1_000_000_000,
            padding_samples as u64,
            "a flooring decoder recovers the padding in samples"
        );
        assert!(value >= 0);
    }
    assert!(saw_padding >= 5, "the residues exercise a non-zero padding");
}

#[test]
fn no_padding_element_is_written_when_the_value_is_zero() {
    let stream = fake_stream(10, 312, 10 * 960 - 312);
    let bytes = mux_webm(&stream, 1, 48_000).unwrap();
    let (found, _) = discard_paddings(&bytes);
    assert!(found.is_empty());
}

#[test]
fn a_stream_whose_packets_cannot_cover_the_valid_samples_is_refused() {
    // 10 frames cover 9600 samples; pre-skip 312 + valid 9300 = 9612 > 9600.
    let stream = fake_stream(10, 312, 9300);
    let err = mux_webm(&stream, 1, 48_000).unwrap_err();
    assert!(err.contains("negative DiscardPadding"), "{err}");
    // The exact boundary is accepted with no padding element.
    assert!(mux_webm(&fake_stream(10, 312, 9288), 1, 48_000).is_ok());
    assert!(mux_webm(&fake_stream(10, 312, 9289), 1, 48_000).is_err());
}

#[test]
fn the_encoder_flushes_its_lookahead_for_every_residue() {
    for residue in RESIDUES {
        let valid = valid_with_residue(residue);
        let stream = encoded(valid);
        let covered = stream.packets.len() as u64 * 960;
        assert!(
            covered >= stream.pre_skip as u64 + stream.total_valid,
            "residue {residue}: {covered} < {} + {}",
            stream.pre_skip,
            stream.total_valid
        );
        let unflushed = (valid as u64).div_ceil(960) as usize;
        let expected_extra =
            usize::from(unflushed as u64 * 960 < stream.pre_skip as u64 + valid as u64);
        assert_eq!(
            stream.packets.len(),
            unflushed + expected_extra,
            "residue {residue}: one extra frame exactly when the unflushed frames lose the tail"
        );
    }
    // The three loss residues of a 312-sample lookahead, as the arithmetic predicts.
    let loss: Vec<usize> = RESIDUES
        .into_iter()
        .filter(|&r| {
            let s = encoded(valid_with_residue(r));
            (valid_with_residue(r) as u64).div_ceil(960) * 960 < s.pre_skip as u64 + s.total_valid
        })
        .collect();
    assert_eq!(loss, vec![0, 649, 959]);
}

/// The audio packets of a muxed WebM, in order: the payload of every SimpleBlock and Block after
/// its 4-byte block header (track vint, 16-bit timecode, flags).
fn webm_packets(bytes: &[u8]) -> Vec<Vec<u8>> {
    let seg = segment(bytes);
    let mut packets = Vec::new();
    for cluster in of(&children(bytes, seg), 0x1F43B675) {
        for element in children(bytes, cluster.payload) {
            let block = match element.id {
                0xA3 => element.payload,
                0xA0 => {
                    of(&children(bytes, element.payload), 0xA1)
                        .remove(0)
                        .payload
                }
                _ => continue,
            };
            packets.push(bytes[block.start + 4..block.end].to_vec());
        }
    }
    packets
}

/// What a decoder reads from a WebM: the packets, the front trim in samples (`CodecDelay`), the end
/// trim in samples (`DiscardPadding`, zero when absent) and the `Duration` in milliseconds.
fn webm_trim(bytes: &[u8]) -> (Vec<Vec<u8>>, u64, u64, f64) {
    let seg = segment(bytes);
    let elems = children(bytes, seg);
    let tracks = of(&elems, 0x1654AE6B).remove(0);
    let entry = children(bytes, tracks.payload).remove(0);
    let delay_ns = uint(
        bytes,
        of(&children(bytes, entry.payload), 0x56AA)
            .remove(0)
            .payload,
    );
    let (paddings, _) = discard_paddings(bytes);
    let padding_ns = paddings.first().map_or(0, |p| p.2) as u64;
    let info = of(&elems, 0x1549A966).remove(0);
    let duration = of(&children(bytes, info.payload), 0x4489).remove(0);
    (
        webm_packets(bytes),
        delay_ns * 48_000 / 1_000_000_000,
        padding_ns * 48_000 / 1_000_000_000,
        f64::from_be_bytes(bytes[duration.payload].try_into().unwrap()),
    )
}

/// The packets of an Ogg/Opus file with the pre-skip, vendor string and final granule position.
fn ogg_trim(bytes: &[u8]) -> (Vec<Vec<u8>>, u64, String, u64) {
    let mut reader = ogg::reading::PacketReader::new(std::io::Cursor::new(bytes));
    let head = reader.read_packet_expected().unwrap();
    let pre_skip = u16::from_le_bytes([head.data[10], head.data[11]]) as u64;
    let tags = reader.read_packet_expected().unwrap();
    let vendor_len = u32::from_le_bytes(tags.data[8..12].try_into().unwrap()) as usize;
    let vendor = String::from_utf8(tags.data[12..12 + vendor_len].to_vec()).unwrap();
    let mut packets = Vec::new();
    let mut granule = 0;
    while let Some(packet) = reader.read_packet().unwrap() {
        granule = packet.absgp_page();
        packets.push(packet.data);
    }
    (packets, pre_skip, vendor, granule)
}

/// Decodes `packets` with libopus (mono) to one sample vector.
fn libopus_decode(packets: &[Vec<u8>]) -> Vec<f32> {
    let mut decoder = opus::Decoder::new(48_000, opus::Channels::Mono).unwrap();
    let mut out = Vec::new();
    for packet in packets {
        let mut frame = vec![0.0f32; 960];
        let n = decoder.decode_float(packet, &mut frame, false).unwrap();
        out.extend_from_slice(&frame[..n]);
    }
    out
}

/// RMS of the last `n` samples.
fn tail_rms(samples: &[f32], n: usize) -> f32 {
    let tail = &samples[samples.len() - n..];
    (tail.iter().map(|s| s * s).sum::<f32>() / n as f32).sqrt()
}

#[test]
fn a_decoder_applying_the_container_trim_recovers_exactly_the_valid_samples_from_both_containers() {
    for residue in RESIDUES {
        let dir = tempfile::tempdir().unwrap();
        let staged = dir.path().join("upload");
        let valid = valid_with_residue(residue);
        let wav = synth_wav_440hz_at(48_000, valid as f64 / 48_000.0);
        std::fs::write(&staged, &wav).unwrap();
        process_staged_audio(
            &staged,
            "audio/wav",
            wav.len() as i64,
            AudioContainers::Both,
        )
        .unwrap();

        // WebM: front trim = CodecDelay, end trim = DiscardPadding.
        let webm = std::fs::read(with_suffix(&staged, WEBM_SUFFIX)).unwrap();
        let (packets, delay, padding, duration_ms) = webm_trim(&webm);
        let decoded = libopus_decode(&packets);
        let audible = decoded.len() as u64 - delay - padding;
        assert_eq!(audible, valid as u64, "webm residue {residue}");
        assert_eq!(
            duration_ms,
            valid as f64 * 1000.0 / 48_000.0,
            "Duration, residue {residue}"
        );
        let kept = &decoded[delay as usize..(delay + audible) as usize];
        assert!(
            tail_rms(kept, 100) > 0.2,
            "the tail of the webm is audible, residue {residue}"
        );

        // Ogg: front trim = pre-skip, end trim = final granule position.
        let ogg = std::fs::read(with_suffix(&staged, OPUS_SUFFIX)).unwrap();
        let (packets, pre_skip, vendor, granule) = ogg_trim(&ogg);
        assert_eq!(vendor, OPUS_VENDOR_MARKER);
        let decoded = libopus_decode(&packets);
        assert!(
            decoded.len() as u64 >= granule,
            "the granule lies inside the decoded samples"
        );
        assert_eq!(granule - pre_skip, valid as u64, "ogg residue {residue}");
        let kept = &decoded[pre_skip as usize..granule as usize];
        assert!(
            tail_rms(kept, 100) > 0.2,
            "the tail of the ogg is audible, residue {residue}"
        );
    }
}
