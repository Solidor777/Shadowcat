//! This module is the WebM (EBML) muxer for the Opus derivative.
//!
//! It writes a minimal single-audio-track file from the encoded packets, plus the helpers the
//! upgrade sweep uses to recognise the layout it wrote.
//!
//! The file is an EBML header followed by one Segment of KNOWN size holding, in this order,
//! `SeekHead`, `Info`, `Tracks`, `Cues`, then the Clusters. The size of the Segment, of each
//! Cluster and of every other element is computed, never the unknown-size marker. `SeekHead` and
//! `Cues` carry fixed-width positions, so their sizes do not depend on the values they hold and
//! the cluster positions they point at are computed in one pass without iterating.

use super::{opus_head, OpusStream, OPUS_FRAME_MS, OPUS_FRAME_SAMPLES, OPUS_TARGET_SAMPLE_RATE};

/// Frames per WebM cluster: the relative block timecode is a signed i16 of milliseconds, so a
/// cluster must span well under 32.7 seconds.
const WEBM_CLUSTER_FRAMES: usize = 64;

/// The `WritingApp` string every file written by the current muxer carries. The upgrade sweep
/// selects a WebM sibling (`WEBM_SUFFIX`) whose head lacks it.
pub(crate) const WEBM_WRITING_APP: &str = "shadowcat-webm/2";

/// Bytes at the start of a WebM sibling (`WEBM_SUFFIX`) that always contain `WEBM_WRITING_APP` when the file was
/// written by the current muxer, and that the derivative `ETag` hashes: the EBML header, the
/// Segment header, `SeekHead` and `Info` precede the first `Tracks` byte and total well under it.
pub(crate) const WEBM_HEAD_BYTES: usize = 4096;

/// EBML element id: Segment.
const ID_SEGMENT: [u8; 4] = [0x18, 0x53, 0x80, 0x67];
/// EBML element id: SeekHead.
const ID_SEEK_HEAD: [u8; 4] = [0x11, 0x4D, 0x9B, 0x74];
/// EBML element id: Info.
const ID_INFO: [u8; 4] = [0x15, 0x49, 0xA9, 0x66];
/// EBML element id: Tracks.
const ID_TRACKS: [u8; 4] = [0x16, 0x54, 0xAE, 0x6B];
/// EBML element id: Cues.
const ID_CUES: [u8; 4] = [0x1C, 0x53, 0xBB, 0x6B];
/// EBML element id: Cluster.
const ID_CLUSTER: [u8; 4] = [0x1F, 0x43, 0xB6, 0x75];

/// An EBML element id-length-prefixed size, or the all-ones "unknown size" marker used only
/// where a length is genuinely not computed (never emitted by this writer: every element's
/// size is known up front, the strictest shape a demuxer accepts).
fn ebml_size(out: &mut Vec<u8>, value: u64) {
    let bytes = if value < 0x7F {
        1
    } else if value < 0x3FFF {
        2
    } else if value < 0x1F_FFFF {
        3
    } else if value < 0xFFF_FFFF {
        4
    } else {
        8
    };
    let marker = 1u64 << (7 * bytes);
    let encoded = marker | value;
    out.extend_from_slice(&encoded.to_be_bytes()[8 - bytes as usize..]);
}

/// Append one EBML element: `id` (already the wire bytes), then vint size, then `payload`.
fn ebml_element(out: &mut Vec<u8>, id: &[u8], payload: &[u8]) {
    out.extend_from_slice(id);
    ebml_size(out, payload.len() as u64);
    out.extend_from_slice(payload);
}

/// Append one EBML unsigned-integer element, zero-extended to `width` bytes.
fn ebml_uint(out: &mut Vec<u8>, id: &[u8], value: u64, width: usize) {
    out.extend_from_slice(id);
    ebml_size(out, width as u64);
    out.extend_from_slice(&value.to_be_bytes()[8 - width..]);
}

/// Append one EBML signed-integer element, always 8 bytes (two's complement, big endian).
fn ebml_int(out: &mut Vec<u8>, id: &[u8], value: i64) {
    out.extend_from_slice(id);
    ebml_size(out, 8);
    out.extend_from_slice(&value.to_be_bytes());
}

/// Append one EBML float element (always 8 bytes, f64).
fn ebml_float(out: &mut Vec<u8>, id: &[u8], value: f64) {
    out.extend_from_slice(id);
    ebml_size(out, 8);
    out.extend_from_slice(&value.to_be_bytes());
}

/// `samples` at 48 kHz as nanoseconds, rounded UP. `CodecDelay` and `DiscardPadding` are whole
/// nanoseconds while 48 kHz samples are not (one sample is 20833.33 ns), so a decoder converting
/// back to samples recovers the exact count when it floors or rounds only if the value was rounded
/// up here; rounding down loses a sample under a flooring decoder.
fn samples_to_ns(samples: u64) -> u64 {
    (samples * 1_000_000_000).div_ceil(OPUS_TARGET_SAMPLE_RATE as u64)
}

/// A position as the fixed 4-byte unsigned width `SeekHead` and `Cues` carry, or an error when the
/// file is too large for that width (a 30-minute derivative is far below 4 GiB).
fn position_u32(position: u64) -> Result<u64, String> {
    if position > u64::from(u32::MAX) {
        return Err("webm segment exceeds the 4-byte position width".to_string());
    }
    Ok(position)
}

/// `SeekHead` pointing at `Info`, `Tracks` and `Cues` (positions relative to the first byte of
/// the Segment's payload). Fixed-width positions: the byte length is the same for any values.
fn seek_head(info: u64, tracks: u64, cues: u64) -> Vec<u8> {
    let mut payload = Vec::new();
    for (id, position) in [
        (ID_INFO.as_slice(), info),
        (ID_TRACKS.as_slice(), tracks),
        (ID_CUES.as_slice(), cues),
    ] {
        let mut seek = Vec::new();
        ebml_element(&mut seek, &[0x53, 0xAB], id); // SeekID
        ebml_uint(&mut seek, &[0x53, 0xAC], position, 4); // SeekPosition
        ebml_element(&mut payload, &[0x4D, 0xBB], &seek); // Seek
    }
    let mut out = Vec::new();
    ebml_element(&mut out, &ID_SEEK_HEAD, &payload);
    out
}

/// `Cues` with one `CuePoint` per cluster: `(cluster start in ms, cluster position)`. Fixed-width
/// fields: the byte length depends only on the number of clusters.
fn cues(points: &[(u64, u64)]) -> Vec<u8> {
    let mut payload = Vec::new();
    for &(time_ms, position) in points {
        let mut positions = Vec::new();
        ebml_uint(&mut positions, &[0xF7], 1, 1); // CueTrack
        ebml_uint(&mut positions, &[0xF1], position, 4); // CueClusterPosition
        let mut point = Vec::new();
        ebml_uint(&mut point, &[0xB3], time_ms, 8); // CueTime
        ebml_element(&mut point, &[0xB7], &positions); // CueTrackPositions
        ebml_element(&mut payload, &[0xBB], &point); // CuePoint
    }
    let mut out = Vec::new();
    ebml_element(&mut out, &ID_CUES, &payload);
    out
}

/// One cluster of `chunk` packets starting at `base_ms`. When `padding_ns` is `Some`, the LAST
/// packet is written as a `BlockGroup` carrying `DiscardPadding`; every other packet is a
/// `SimpleBlock`.
fn cluster(base_ms: u64, chunk: &[Vec<u8>], padding_ns: Option<i64>) -> Vec<u8> {
    let mut body = Vec::new();
    ebml_uint(&mut body, &[0xE7], base_ms, 8); // Cluster Timecode
    let last = chunk.len() - 1;
    for (i, packet) in chunk.iter().enumerate() {
        let rel_ms = (i * OPUS_FRAME_MS as usize) as i16;
        let mut block = Vec::with_capacity(4 + packet.len());
        block.push(0x81); // track number 1 as a 1-byte vint
        block.extend_from_slice(&rel_ms.to_be_bytes());
        match padding_ns {
            Some(padding) if i == last => {
                block.push(0x00); // Block flags: no lacing
                block.extend_from_slice(packet);
                let mut group = Vec::new();
                ebml_element(&mut group, &[0xA1], &block); // Block
                ebml_int(&mut group, &[0x75, 0xA2], padding); // DiscardPadding
                ebml_element(&mut body, &[0xA0], &group); // BlockGroup
            }
            _ => {
                block.push(0x80); // SimpleBlock flags: keyframe, no lacing
                block.extend_from_slice(packet);
                ebml_element(&mut body, &[0xA3], &block); // SimpleBlock
            }
        }
    }
    let mut out = Vec::new();
    ebml_element(&mut out, &ID_CLUSTER, &body);
    out
}

/// Mux the Opus frames into a minimal single-audio-track WebM (EBML): header, Segment with
/// `SeekHead`, `Info` (`Duration`), `Tracks` (`A_OPUS`, `CodecPrivate` = `OpusHead`, `CodecDelay` =
/// the pre-skip, `SeekPreRoll` = 80 ms per the Opus-in-WebM mapping), `Cues` (one `CuePoint` per
/// cluster), then clusters of `WEBM_CLUSTER_FRAMES` blocks each. A decoder drops `CodecDelay`
/// from the front and `DiscardPadding` from the end, so `Duration` is the audible length
/// `stream.total_valid` and the LAST block alone carries `DiscardPadding`, the zero-padding past
/// the valid samples: `frames * 960 - pre_skip - total_valid`, in nanoseconds. Only the last
/// packet may carry it (a decoder rejects padding on an earlier one). No padding element is
/// written when the value is zero.
///
/// A negative padding means the packets cannot cover `pre_skip + total_valid` samples (the audio
/// would be cut short by a decoder), so it is an error, never a clamp and never a negative write.
pub(super) fn mux_webm(
    stream: &OpusStream,
    channels: u16,
    source_rate: u32,
) -> Result<Vec<u8>, String> {
    let covered = stream.packets.len() as u64 * OPUS_FRAME_SAMPLES;
    let padding_samples = covered as i64 - stream.pre_skip as i64 - stream.total_valid as i64;
    if padding_samples < 0 {
        return Err(format!(
            "negative DiscardPadding: {} frames cover {covered} samples, need {} (pre-skip {} + valid {})",
            stream.packets.len(),
            u64::from(stream.pre_skip) + stream.total_valid,
            stream.pre_skip,
            stream.total_valid,
        ));
    }
    let padding_ns = (padding_samples > 0).then(|| samples_to_ns(padding_samples as u64) as i64);

    let mut out = Vec::new();

    // EBML header.
    let mut header = Vec::new();
    ebml_uint(&mut header, &[0x42, 0x86], 1, 1); // EBMLVersion
    ebml_uint(&mut header, &[0x42, 0xF7], 1, 1); // EBMLReadVersion
    ebml_uint(&mut header, &[0x42, 0xF2], 4, 1); // EBMLMaxIDLength
    ebml_uint(&mut header, &[0x42, 0xF3], 8, 1); // EBMLMaxSizeLength
    ebml_element(&mut header, &[0x42, 0x82], b"webm"); // DocType
    ebml_uint(&mut header, &[0x42, 0x87], 4, 1); // DocTypeVersion
    ebml_uint(&mut header, &[0x42, 0x85], 2, 1); // DocTypeReadVersion
    ebml_element(&mut out, &[0x1A, 0x45, 0xDF, 0xA3], &header);

    // Info: millisecond timecode scale; Duration is in those ticks.
    let mut info_payload = Vec::new();
    ebml_uint(&mut info_payload, &[0x2A, 0xD7, 0xB1], 1_000_000, 4); // TimecodeScale (ns per tick)
    ebml_float(
        &mut info_payload,
        &[0x44, 0x89],
        stream.total_valid as f64 * 1000.0 / OPUS_TARGET_SAMPLE_RATE as f64,
    ); // Duration
    ebml_element(&mut info_payload, &[0x4D, 0x80], b"shadowcat"); // MuxingApp
    ebml_element(
        &mut info_payload,
        &[0x57, 0x41],
        WEBM_WRITING_APP.as_bytes(),
    ); // WritingApp
    let mut info = Vec::new();
    ebml_element(&mut info, &ID_INFO, &info_payload);

    // Tracks: one audio track, A_OPUS.
    let mut entry = Vec::new();
    ebml_uint(&mut entry, &[0xD7], 1, 1); // TrackNumber
    ebml_uint(&mut entry, &[0x73, 0xC5], 1, 4); // TrackUID
    ebml_uint(&mut entry, &[0x83], 2, 1); // TrackType: 2 = audio (1 is video)
    ebml_element(&mut entry, &[0x86], b"A_OPUS"); // CodecID
    ebml_element(
        &mut entry,
        &[0x63, 0xA2],
        &opus_head(channels, stream.pre_skip, source_rate),
    );
    // CodecDelay (ns) + SeekPreRoll (ns), per the Opus-in-WebM mapping.
    ebml_uint(
        &mut entry,
        &[0x56, 0xAA],
        samples_to_ns(stream.pre_skip as u64),
        8,
    );
    ebml_uint(&mut entry, &[0x56, 0xBB], 80_000_000, 8);
    let mut audio = Vec::new();
    ebml_float(&mut audio, &[0xB5], OPUS_TARGET_SAMPLE_RATE as f64); // SamplingFrequency
    ebml_uint(&mut audio, &[0x9F], channels as u64, 1); // Channels
    ebml_element(&mut entry, &[0xE1], &audio);
    let mut track_entry = Vec::new();
    ebml_element(&mut track_entry, &[0xAE], &entry);
    let mut tracks = Vec::new();
    ebml_element(&mut tracks, &ID_TRACKS, &track_entry);

    // Clusters, built first: their sizes fix every position the SeekHead and Cues carry.
    let chunk_count = stream.packets.chunks(WEBM_CLUSTER_FRAMES).count();
    let clusters: Vec<(u64, Vec<u8>)> = stream
        .packets
        .chunks(WEBM_CLUSTER_FRAMES)
        .enumerate()
        .map(|(index, chunk)| {
            let base_ms = (index * WEBM_CLUSTER_FRAMES) as u64 * OPUS_FRAME_MS as u64;
            let last = index + 1 == chunk_count;
            (
                base_ms,
                cluster(base_ms, chunk, padding_ns.filter(|_| last)),
            )
        })
        .collect();

    // Positions are relative to the first byte of the Segment payload.
    let seek_head_len = seek_head(0, 0, 0).len() as u64;
    let info_pos = seek_head_len;
    let tracks_pos = info_pos + info.len() as u64;
    let cues_pos = tracks_pos + tracks.len() as u64;
    let cues_len = cues(&vec![(0, 0); clusters.len()]).len() as u64;
    let mut position = cues_pos + cues_len;
    let mut points = Vec::with_capacity(clusters.len());
    for (base_ms, bytes) in &clusters {
        points.push((*base_ms, position_u32(position)?));
        position += bytes.len() as u64;
    }

    let mut segment = Vec::new();
    segment.extend_from_slice(&seek_head(
        position_u32(info_pos)?,
        position_u32(tracks_pos)?,
        position_u32(cues_pos)?,
    ));
    segment.extend_from_slice(&info);
    segment.extend_from_slice(&tracks);
    let cues_bytes = cues(&points);
    debug_assert_eq!(cues_bytes.len() as u64, cues_len);
    segment.extend_from_slice(&cues_bytes);
    for (_, bytes) in &clusters {
        segment.extend_from_slice(bytes);
    }
    ebml_element(&mut out, &ID_SEGMENT, &segment);
    Ok(out)
}

/// Whether the head of a WebM sibling (its first `WEBM_HEAD_BYTES` bytes, or the whole file when
/// shorter) was written by the current muxer: the `WritingApp` marker is present.
pub(crate) fn head_is_current(head: &[u8]) -> bool {
    let marker = WEBM_WRITING_APP.as_bytes();
    head.windows(marker.len()).any(|w| w == marker)
}

#[cfg(test)]
pub(super) mod tests;
