//! Audio transcode: sniffs the real container/codec via `symphonia`'s probe (never trusting
//! the client-declared `content_type`), decodes, resamples to 48 kHz via `rubato`, encodes
//! Opus (VBR) via `opus`, and muxes the encoded frames into the container(s) the import
//! selected — Ogg pages via `ogg`, WebM via this module's own minimal EBML writer (no extra
//! dependency for a fixed single-audio-track shape) — each written as a SIBLING derivative.
//! The canonical file is NEVER rewritten, unlike the image pipeline's canonical swap; the
//! uploaded original stays every non-GM player's playback fallback on the ordinary serve
//! route. Over-cap input or any pipeline failure falls back to pass-through with
//! `audio:untranscoded` (tagged by the caller via `tags::derive`, not here) and no derivative
//! — there is no lazy on-demand regeneration for a minutes-long transcode.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two crate-level deny attributes this module declares.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{with_suffix, AssetMeta, Processed};

/// File-name suffix of the Ogg/Opus derivative sibling.
pub const OPUS_SUFFIX: &str = ".opus.ogg";
/// MIME type of the Ogg/Opus derivative.
pub const OPUS_CONTENT_TYPE: &str = "audio/ogg; codecs=opus";
/// File-name suffix of the WebM/Opus derivative sibling.
pub const WEBM_SUFFIX: &str = ".opus.webm";
/// MIME type of the WebM/Opus derivative.
pub const WEBM_CONTENT_TYPE: &str = "audio/webm; codecs=opus";
/// Longest admitted source duration; beyond this the upload is stored pass-through
/// (`audio:untranscoded`), never rejected.
pub const MAX_AUDIO_DURATION_SECS: f64 = 30.0 * 60.0;
/// Largest admitted decoded sample count (across all channels), the companion cap to
/// `MAX_AUDIO_DURATION_SECS` for a pathological sample rate.
pub const MAX_AUDIO_SAMPLES: u64 = 1 << 28;
/// Opus's own maximum sample rate, and the pipeline's fixed resample target.
pub const OPUS_TARGET_SAMPLE_RATE: u32 = 48_000;
/// Opus frame size in milliseconds (20 ms is Opus's recommended default for VBR speech/music).
pub const OPUS_FRAME_MS: u32 = 20;
/// Frames per WebM cluster: the relative block timecode is a signed i16 of milliseconds, so a
/// cluster must span well under 32.7 seconds.
const WEBM_CLUSTER_FRAMES: usize = 64;

/// Which Opus derivative container(s) an audio import emits. Selected at import time
/// (`CreateUploadRequest.audio_containers` / the multipart `containers` field); `Both` is the
/// default because any file may be looped (a loop prefers the Ogg derivative for gapless
/// decode) while a WebKit client needs the WebM one (its `canPlayType` for Ogg/Opus is empty).
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::process::audio::AudioContainers;
///
/// assert_eq!(AudioContainers::default(), AudioContainers::Both);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum AudioContainers {
    /// `.opus.ogg` only.
    Ogg,
    /// `.opus.webm` only.
    #[serde(rename = "webm")]
    WebM,
    /// Both derivatives (the import default).
    #[default]
    Both,
}

impl AudioContainers {
    /// The `(suffix, content_type)` pairs this selection emits.
    pub(crate) fn siblings(self) -> &'static [(&'static str, &'static str)] {
        match self {
            AudioContainers::Ogg => &[(OPUS_SUFFIX, OPUS_CONTENT_TYPE)],
            AudioContainers::WebM => &[(WEBM_SUFFIX, WEBM_CONTENT_TYPE)],
            AudioContainers::Both => &[
                (OPUS_SUFFIX, OPUS_CONTENT_TYPE),
                (WEBM_SUFFIX, WEBM_CONTENT_TYPE),
            ],
        }
    }
}

/// The selection a re-transcode (`replace`/`reconvert`) re-emits when the caller made no
/// explicit choice: the derivative set the asset CURRENTLY has on disk, or `Both` when it has
/// none (a never-transcoded or failed import retries everything — the only recovery path
/// from an over-cap or failed first transcode).
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::process::audio::{effective_reencode_selection, AudioContainers};
///
/// assert_eq!(effective_reencode_selection(true, false), AudioContainers::Ogg);
/// assert_eq!(effective_reencode_selection(false, true), AudioContainers::WebM);
/// assert_eq!(effective_reencode_selection(true, true), AudioContainers::Both);
/// assert_eq!(effective_reencode_selection(false, false), AudioContainers::Both);
/// ```
pub fn effective_reencode_selection(has_ogg: bool, has_webm: bool) -> AudioContainers {
    match (has_ogg, has_webm) {
        (true, false) => AudioContainers::Ogg,
        (false, true) => AudioContainers::WebM,
        _ => AudioContainers::Both,
    }
}

/// Decoded PCM plus the facts the caller needs (duration, source sample rate, channel count).
struct Decoded {
    /// Interleaved f32 PCM samples at the SOURCE sample rate.
    samples: Vec<f32>,
    /// Source sample rate, Hz.
    sample_rate: u32,
    /// Channel count (1 = mono, 2 = stereo; the pipeline downmixes anything wider to stereo
    /// before this struct is built).
    channels: u16,
    /// Total duration, seconds (`samples.len() / channels / sample_rate` as f64).
    duration_secs: f64,
}

/// Decode `staged` via `symphonia`'s format/codec registries, sniffing the REAL container from
/// the bytes (never trusting `content_type`). Downmixes to stereo when the source has more than
/// 2 channels (Opus itself supports more, but the pipeline standardizes on mono/stereo).
/// Returns `Err` for any unreadable/unsupported source — the caller falls back to
/// pass-through.
fn decode_audio(staged: &Path) -> Result<Decoded, String> {
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let file = std::fs::File::open(staged).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut format = symphonia::default::get_probe()
        .probe(
            &Hint::new(),
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| e.to_string())?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| "no default audio track".to_string())?
        .clone();
    let audio_params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| "track is not audio".to_string())?
        .clone();
    let mut sample_rate = audio_params.sample_rate.unwrap_or(OPUS_TARGET_SAMPLE_RATE);
    let mut channels = audio_params
        .channels
        .clone()
        .map(|c| c.count() as u16)
        .unwrap_or(1)
        .clamp(1, 2);
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&audio_params, &AudioDecoderOptions::default())
        .map_err(|e| e.to_string())?;

    let mut samples: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(e) => return Err(e.to_string()),
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // A corrupt packet mid-stream is skipped, the same posture symphonia's own
            // interleaved example takes; a wholly undecodable file errors at the probe or
            // first-packet stage instead.
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => return Err(e.to_string()),
        };
        sample_rate = decoded.spec().rate();
        let true_channels = decoded.spec().channels().count().max(1);
        let frames = decoded.samples_interleaved() / true_channels;
        let mut raw = vec![0.0f32; frames * true_channels];
        decoded.copy_to_slice_interleaved(&mut raw);
        channels = true_channels.min(2) as u16;
        if true_channels > 2 {
            // Documented fold-down to stereo: the front pair is kept as-is; every further
            // channel (centre, LFE, surrounds) folds into BOTH sides at half weight. libopus
            // clamps float input internally, so a hot centre channel cannot clip the encode.
            for f in 0..frames {
                let mut l = raw[f * true_channels];
                let mut r = raw[f * true_channels + 1];
                for c in 2..true_channels {
                    let s = 0.5 * raw[f * true_channels + c];
                    l += s;
                    r += s;
                }
                samples.push(l);
                samples.push(r);
            }
        } else if true_channels == 2 {
            samples.extend_from_slice(&raw);
        } else {
            // Mono: one channel, no interleave expansion.
            samples.extend_from_slice(&raw);
        }
        if (samples.len() as u64) > MAX_AUDIO_SAMPLES {
            break; // caller's cap check below catches this via duration/sample-count
        }
    }
    if samples.is_empty() {
        return Err("no decodable audio samples".to_string());
    }

    let frames = samples.len() as f64 / channels as f64;
    let duration_secs = frames / sample_rate as f64;
    Ok(Decoded {
        samples,
        sample_rate,
        channels,
        duration_secs,
    })
}

/// Resample `decoded`'s PCM to `OPUS_TARGET_SAMPLE_RATE` via `rubato`'s sinc resampler,
/// processing ~1-second input chunks so the FFT workspace never scales with the upload's
/// length (a whole-file chunk on a 30-minute source allocates gigabytes). A no-op (clone)
/// when the source is already at the target rate.
fn resample_to_target(decoded: &Decoded) -> Result<Vec<f32>, String> {
    if decoded.sample_rate == OPUS_TARGET_SAMPLE_RATE {
        return Ok(decoded.samples.clone());
    }
    use rubato::audioadapter_buffers::direct::InterleavedSlice;
    use rubato::{
        Async, FixedAsync, Indexing, Resampler, SincInterpolationParameters, SincInterpolationType,
        WindowFunction,
    };
    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: Some(0.95),
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };
    let ratio = OPUS_TARGET_SAMPLE_RATE as f64 / decoded.sample_rate as f64;
    let channels = decoded.channels as usize;
    // ~1 s of source audio per chunk: bounded workspace regardless of input length.
    let chunk_frames = decoded.sample_rate as usize;
    let mut resampler = Async::new_sinc(
        ratio,
        2.0,
        &params,
        chunk_frames,
        channels,
        FixedAsync::Input,
    )
    .map_err(|e| e.to_string())?;
    let total_frames = decoded.samples.len() / channels;
    let mut out: Vec<f32> =
        Vec::with_capacity((total_frames as f64 * ratio) as usize * channels + 8192);
    let mut pos = 0usize;
    while pos < total_frames {
        let n = (total_frames - pos).min(chunk_frames);
        let adapter = InterleavedSlice::new(
            &decoded.samples[pos * channels..(pos + n) * channels],
            channels,
            n,
        )
        .map_err(|e| e.to_string())?;
        let indexing = (n < chunk_frames).then_some(Indexing {
            input_offset: 0,
            output_offset: 0,
            partial_len: Some(n),
            active_channels_mask: None,
        });
        let chunk = resampler
            .process(&adapter, indexing.as_ref())
            .map_err(|e| e.to_string())?;
        out.extend_from_slice(&chunk.take_data());
        pos += n;
    }
    Ok(out)
}

/// One encoded 20 ms Opus frame plus the running granule (48 kHz sample count) it ends at.
struct OpusStream {
    /// The encoded frames, in order.
    packets: Vec<Vec<u8>>,
    /// Encoder lookahead (samples), recorded as the container's pre-skip.
    pre_skip: u16,
    /// Total VALID samples per channel at 48 kHz (before the final frame's zero-padding) —
    /// the Ogg end-trim granule's numerator (RFC 7845: final granule = pre_skip + valid).
    total_valid: u64,
}

/// Encode 48 kHz PCM to 20 ms Opus frames (VBR; 96 kbps stereo / 64 kbps mono). The final
/// partial frame is zero-padded with silence.
fn encode_opus_frames(pcm_48k: &[f32], channels: u16) -> Result<OpusStream, String> {
    let bitrate = if channels >= 2 { 96_000 } else { 64_000 };
    let mut encoder = opus::Encoder::new(
        OPUS_TARGET_SAMPLE_RATE,
        if channels >= 2 {
            opus::Channels::Stereo
        } else {
            opus::Channels::Mono
        },
        opus::Application::Audio,
    )
    .map_err(|e| e.to_string())?;
    encoder
        .set_bitrate(opus::Bitrate::Bits(bitrate))
        .map_err(|e| e.to_string())?;
    let pre_skip = encoder
        .get_lookahead()
        .unwrap_or(0)
        .clamp(0, u16::MAX as i32) as u16;

    let frame_samples =
        (OPUS_TARGET_SAMPLE_RATE as usize * OPUS_FRAME_MS as usize / 1000) * channels as usize;
    let mut packets = Vec::new();
    let mut absolute_pos: usize = 0;
    while absolute_pos < pcm_48k.len() {
        let end = (absolute_pos + frame_samples).min(pcm_48k.len());
        let mut chunk = pcm_48k[absolute_pos..end].to_vec();
        chunk.resize(frame_samples, 0.0); // pad the final partial frame with silence
        packets.push(
            encoder
                .encode_vec_float(&chunk, 4000)
                .map_err(|e| e.to_string())?,
        );
        absolute_pos = end;
    }
    if packets.is_empty() {
        return Err("nothing to encode".to_string());
    }
    Ok(OpusStream {
        packets,
        pre_skip,
        total_valid: pcm_48k.len() as u64 / channels as u64,
    })
}

/// The 19-byte `OpusHead` both containers carry (Ogg as the first header packet, WebM as the
/// track's `CodecPrivate`). Channel mapping family 0 (mono/stereo), zero output gain.
fn opus_head(channels: u16, pre_skip: u16, source_rate: u32) -> Vec<u8> {
    let mut head = Vec::with_capacity(19);
    head.extend_from_slice(b"OpusHead");
    head.push(1); // version
    head.push(channels as u8);
    head.extend_from_slice(&pre_skip.to_le_bytes());
    head.extend_from_slice(&source_rate.to_le_bytes()); // input rate (informational)
    head.extend_from_slice(&0i16.to_le_bytes()); // output gain
    head.push(0); // channel mapping family 0
    head
}

/// A process-lifetime-stable-enough Ogg stream serial (uniqueness within one file is all Ogg
/// requires; a random u32 is ample — collision risk is irrelevant since each derivative is its
/// own single-stream file).
fn rand_serial() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0)
}

/// Mux the Opus frames into Ogg pages: the `OpusHead`/`OpusTags` header packets every Ogg/Opus
/// decoder requires first, then one packet per audio frame, granule counting 48 kHz samples.
fn mux_ogg(stream: &OpusStream, channels: u16, source_rate: u32) -> Result<Vec<u8>, String> {
    use ogg::writing::{PacketWriteEndInfo, PacketWriter};
    let mut out = Vec::new();
    let mut writer = PacketWriter::new(&mut out);
    let serial = rand_serial();
    writer
        .write_packet(
            opus_head(channels, stream.pre_skip, source_rate),
            serial,
            PacketWriteEndInfo::EndPage,
            0,
        )
        .map_err(|e| e.to_string())?;
    // OpusTags: empty vendor string, zero user comments.
    let mut tags = Vec::with_capacity(16);
    tags.extend_from_slice(b"OpusTags");
    tags.extend_from_slice(&0u32.to_le_bytes());
    tags.extend_from_slice(&0u32.to_le_bytes());
    writer
        .write_packet(tags, serial, PacketWriteEndInfo::EndPage, 0)
        .map_err(|e| e.to_string())?;

    let frame_samples = (OPUS_TARGET_SAMPLE_RATE as u64 * OPUS_FRAME_MS as u64) / 1000;
    let mut granule: u64 = 0;
    let last = stream.packets.len() - 1;
    for (i, packet) in stream.packets.iter().enumerate() {
        granule += frame_samples;
        let (end, g) = if i == last {
            // RFC 7845 end-trim: the final page's granule is pre_skip + the VALID sample
            // count, never the zero-padded cumulative — otherwise every loop carries up to
            // ~20ms of trailing silence (or clips real samples when a decoder honors the
            // pre-skip shift) once per repetition.
            (
                PacketWriteEndInfo::EndStream,
                stream.pre_skip as u64 + stream.total_valid,
            )
        } else {
            (PacketWriteEndInfo::NormalPacket, granule)
        };
        writer
            .write_packet(packet.clone(), serial, end, g)
            .map_err(|e| e.to_string())?;
    }
    Ok(out)
}

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

/// Append one EBML float element (always 8 bytes, f64).
fn ebml_float(out: &mut Vec<u8>, id: &[u8], value: f64) {
    out.extend_from_slice(id);
    ebml_size(out, 8);
    out.extend_from_slice(&value.to_be_bytes());
}

/// Mux the Opus frames into a minimal single-audio-track WebM (EBML): header, Segment with
/// Info/Tracks (`A_OPUS`, `CodecPrivate` = `OpusHead`, `CodecDelay` = the pre-skip,
/// `SeekPreRoll` = 80 ms per the Opus-in-WebM mapping), then clusters of
/// `WEBM_CLUSTER_FRAMES` SimpleBlocks each (a block's relative timecode is a signed i16 of
/// milliseconds, so clusters stay small). Every element's size is computed, never unknown —
/// the strictest shape a demuxer accepts.
fn mux_webm(stream: &OpusStream, channels: u16, source_rate: u32) -> Result<Vec<u8>, String> {
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

    // Info: millisecond timecode scale.
    let mut info = Vec::new();
    ebml_uint(&mut info, &[0x2A, 0xD7, 0xB1], 1_000_000, 4); // TimecodeScale (ns per tick)
    ebml_element(&mut info, &[0x4D, 0x80], b"shadowcat"); // MuxingApp
    ebml_element(&mut info, &[0x57, 0x41], b"shadowcat"); // WritingApp

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
    let codec_delay_ns = stream.pre_skip as u64 * 1_000_000_000 / OPUS_TARGET_SAMPLE_RATE as u64;
    ebml_uint(&mut entry, &[0x56, 0xAA], codec_delay_ns, 8);
    ebml_uint(&mut entry, &[0x56, 0xBB], 80_000_000, 8);
    let mut audio = Vec::new();
    ebml_float(&mut audio, &[0xB5], OPUS_TARGET_SAMPLE_RATE as f64); // SamplingFrequency
    ebml_uint(&mut audio, &[0x9F], channels as u64, 1); // Channels
    ebml_element(&mut entry, &[0xE1], &audio);
    let mut tracks = Vec::new();
    ebml_element(&mut tracks, &[0xAE], &entry);

    // Segment payload: Info + Tracks + one Cluster per WEBM_CLUSTER_FRAMES frames.
    let mut segment = Vec::new();
    ebml_element(&mut segment, &[0x15, 0x49, 0xA9, 0x66], &info);
    ebml_element(&mut segment, &[0x16, 0x54, 0xAE, 0x6B], &tracks);
    for (cluster_index, chunk) in stream.packets.chunks(WEBM_CLUSTER_FRAMES).enumerate() {
        let base_ms = (cluster_index * WEBM_CLUSTER_FRAMES) as u64 * OPUS_FRAME_MS as u64;
        let mut cluster = Vec::new();
        ebml_uint(&mut cluster, &[0xE7], base_ms, 8); // Cluster Timecode
        for (i, packet) in chunk.iter().enumerate() {
            let mut block = Vec::new();
            block.push(0x81); // track number 1 as a 1-byte vint
            let rel_ms = (i * OPUS_FRAME_MS as usize) as i16;
            block.extend_from_slice(&rel_ms.to_be_bytes());
            block.push(0x80); // keyframe, no lacing
            block.extend_from_slice(packet);
            ebml_element(&mut cluster, &[0xA3], &block);
        }
        ebml_element(&mut segment, &[0x1F, 0x43, 0xB6, 0x75], &cluster);
    }
    ebml_element(&mut out, &[0x18, 0x53, 0x80, 0x67], &segment);
    Ok(out)
}

/// Whether the `suffix` derivative sibling of `canonical` exists on disk — the re-transcode
/// selection (`effective_reencode_selection`) and the `?variant=opus*` serve branch both ask
/// this of the live file set, never of a recorded flag.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::process::audio::has_sibling;
///
/// let dir = tempfile::tempdir().unwrap();
/// let canonical = dir.path().join("uuid");
/// assert!(!has_sibling(&canonical, ".opus.ogg"));
/// std::fs::write(dir.path().join("uuid.opus.ogg"), b"x").unwrap();
/// assert!(has_sibling(&canonical, ".opus.ogg"));
/// ```
pub fn has_sibling(canonical: &Path, suffix: &str) -> bool {
    with_suffix(canonical, suffix).is_file()
}

/// The pass-through outcome (no derivative, `duration_ms`/`sample_rate` absent), with `note`
/// recorded as the reason — the caller's `tags::derive` reads `conversion_note` to decide
/// whether to apply `audio:untranscoded` (mirroring how the image pipeline's own pass-through
/// note already flows into that same tag-derivation seam).
fn untranscoded(content_type: &str, byte_size: i64, note: String) -> Processed {
    let mut meta = AssetMeta::unprocessed(content_type, byte_size);
    meta.conversion_note = Some(note);
    Processed {
        content_type: content_type.to_string(),
        byte_size,
        meta,
        converted: false,
    }
}

/// Process a staged audio upload (BLOCKING — decode/resample/encode are CPU-bound; the caller,
/// `data::asset::process_staged_blocking`, already runs this under `spawn_blocking`).
///
/// On success: the CANONICAL file at `staged` is left UNTOUCHED (byte-for-byte the arrived
/// upload; `converted: false` on the returned `Processed`, mirroring a pass-through's shape
/// even though this IS the success path — the canonical file is never swapped to the encoded
/// format), and the Opus derivative(s) the import selected are written atomically to the
/// `with_suffix(staged, …)` sibling(s). `duration_ms`/`sample_rate` are populated on the
/// returned `AssetMeta`.
///
/// On over-cap input or any pipeline failure: pass-through, `audio:untranscoded`-eligible (via
/// `conversion_note`), no derivative — never a rejected upload.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::process::audio::{process_staged_audio, AudioContainers};
///
/// let dir = tempfile::tempdir().unwrap();
/// let staged = dir.path().join("upload");
/// // A minimal 0.1 s mono 8 kHz PCM WAV (header + silence), hand-rolled like the pipeline's
/// // own fixtures.
/// let samples = 800u32;
/// let data_len = samples * 2;
/// let mut wav = Vec::new();
/// wav.extend_from_slice(b"RIFF");
/// wav.extend_from_slice(&(36 + data_len).to_le_bytes());
/// wav.extend_from_slice(b"WAVEfmt ");
/// wav.extend_from_slice(&16u32.to_le_bytes());
/// wav.extend_from_slice(&1u16.to_le_bytes());
/// wav.extend_from_slice(&1u16.to_le_bytes());
/// wav.extend_from_slice(&8_000u32.to_le_bytes());
/// wav.extend_from_slice(&16_000u32.to_le_bytes());
/// wav.extend_from_slice(&2u16.to_le_bytes());
/// wav.extend_from_slice(&16u16.to_le_bytes());
/// wav.extend_from_slice(b"data");
/// wav.extend_from_slice(&data_len.to_le_bytes());
/// wav.extend_from_slice(&vec![0u8; data_len as usize]);
/// std::fs::write(&staged, &wav).unwrap();
///
/// let processed = process_staged_audio(&staged, "audio/wav", wav.len() as i64, AudioContainers::Both).unwrap();
/// assert!(!processed.converted); // the canonical is NEVER rewritten for audio
/// assert!(processed.meta.duration_ms.is_some());
/// assert!(staged.with_file_name("upload.opus.ogg").exists());
/// ```
pub fn process_staged_audio(
    staged: &Path,
    original_content_type: &str,
    original_byte_size: i64,
    containers: AudioContainers,
) -> io::Result<Processed> {
    let decoded = match decode_audio(staged) {
        Ok(d) => d,
        Err(e) => {
            return Ok(untranscoded(
                original_content_type,
                original_byte_size,
                format!("decode failed: {e}"),
            ));
        }
    };
    if decoded.duration_secs > MAX_AUDIO_DURATION_SECS
        || (decoded.samples.len() as u64) > MAX_AUDIO_SAMPLES
    {
        return Ok(untranscoded(
            original_content_type,
            original_byte_size,
            "over-cap: duration or sample count".into(),
        ));
    }
    let resampled = match resample_to_target(&decoded) {
        Ok(r) => r,
        Err(e) => {
            return Ok(untranscoded(
                original_content_type,
                original_byte_size,
                format!("resample failed: {e}"),
            ));
        }
    };
    let stream = match encode_opus_frames(&resampled, decoded.channels) {
        Ok(s) => s,
        Err(e) => {
            return Ok(untranscoded(
                original_content_type,
                original_byte_size,
                format!("opus encode failed: {e}"),
            ));
        }
    };
    for (suffix, _content_type) in containers.siblings() {
        let bytes = match *suffix {
            OPUS_SUFFIX => mux_ogg(&stream, decoded.channels, decoded.sample_rate),
            _ => mux_webm(&stream, decoded.channels, decoded.sample_rate),
        };
        let bytes = match bytes {
            Ok(b) => b,
            Err(e) => {
                return Ok(untranscoded(
                    original_content_type,
                    original_byte_size,
                    format!("mux failed: {e}"),
                ));
            }
        };
        let sibling = with_suffix(staged, suffix);
        let tmp = with_suffix(staged, &format!(".{}.tmp", uuid::Uuid::new_v4()));
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &sibling)?;
    }

    Ok(Processed {
        content_type: original_content_type.to_string(), // canonical stays the arrived type
        byte_size: original_byte_size,                   // canonical bytes are unchanged
        meta: AssetMeta {
            duration_ms: Some((decoded.duration_secs * 1000.0).round() as i64),
            sample_rate: Some(decoded.sample_rate as i64),
            ..AssetMeta::unprocessed(original_content_type, original_byte_size)
        },
        converted: false, // the CANONICAL is untouched; only sibling derivatives were added
    })
}

#[cfg(test)]
mod tests;
