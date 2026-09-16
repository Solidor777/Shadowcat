use super::*;

/// A synthesized 440 Hz sine WAV, `secs` long, mono 44100 Hz 16-bit PCM — the RIFF/WAVE
/// header is built by hand (no `hound` dev-dependency: ~30 lines once, zero new crates).
fn synth_wav_440hz(secs: f64) -> Vec<u8> {
    synth_wav_440hz_at(44_100, secs)
}

/// `synth_wav_440hz` at an explicit sample rate (a 48 kHz source skips the resampler, which
/// makes the end-trim granule's expected value exact rather than approximate).
fn synth_wav_440hz_at(sample_rate: u32, secs: f64) -> Vec<u8> {
    let n = (sample_rate as f64 * secs).round() as u32;
    let mut pcm = Vec::with_capacity(n as usize * 2);
    for i in 0..n {
        let t = i as f64 / sample_rate as f64;
        let sample = (t * 440.0 * std::f64::consts::TAU).sin();
        pcm.extend_from_slice(&((sample * i16::MAX as f64) as i16).to_le_bytes());
    }
    let mut wav = Vec::new();
    let data_len = pcm.len() as u32;
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(&pcm);
    wav
}

/// Probe `path`'s container via symphonia and count the default audio track's packets (no
/// packet DECODE — container validity is what the round-trip asserts; the encoded Opus frames
/// were produced by libopus below, so a container symphonia can walk end to end is a container
/// any browser's demuxer also walks).
fn probe_packet_count(path: &Path) -> usize {
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let file = std::fs::File::open(path).unwrap();
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut format = symphonia::default::get_probe()
        .probe(
            &Hint::new(),
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .unwrap();
    let track_id = format.default_track(TrackType::Audio).unwrap().id;
    let mut count = 0;
    loop {
        match format.next_packet() {
            Ok(Some(packet)) => {
                if packet.track_id == track_id {
                    count += 1;
                }
            }
            Ok(None) => break,
            Err(e) => panic!("container walk failed: {e}"),
        }
    }
    count
}

#[test]
fn a_two_second_wav_with_both_containers_produces_valid_ogg_and_webm_siblings() {
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("upload");
    let wav = synth_wav_440hz(2.0);
    std::fs::write(&staged, &wav).unwrap();
    let processed = process_staged_audio(
        &staged,
        "audio/wav",
        wav.len() as i64,
        AudioContainers::Both,
    )
    .unwrap();
    assert!(!processed.converted); // canonical stays untouched — the Opus derivatives are siblings
    assert_eq!(processed.content_type, "audio/wav");
    let duration_ms = processed.meta.duration_ms.unwrap();
    assert!(
        (duration_ms - 2000).abs() <= 50,
        "expected ~2000ms, got {duration_ms}"
    );
    assert_eq!(processed.meta.sample_rate, Some(44_100));

    let ogg_sibling = with_suffix(&staged, OPUS_SUFFIX);
    let webm_sibling = with_suffix(&staged, WEBM_SUFFIX);
    assert!(ogg_sibling.exists());
    assert!(webm_sibling.exists());
    assert_eq!(&std::fs::read(&ogg_sibling).unwrap()[0..4], b"OggS"); // Ogg page magic
    assert_eq!(
        &std::fs::read(&webm_sibling).unwrap()[0..4],
        b"\x1A\x45\xDF\xA3"
    ); // EBML magic

    // Round-trip: symphonia's own probe walks BOTH containers end to end and finds ~100 audio
    // frames (2 s at 20 ms per frame; the Ogg count excludes the two header packets).
    let ogg_packets = probe_packet_count(&ogg_sibling);
    let webm_packets = probe_packet_count(&webm_sibling);
    assert!(
        (ogg_packets as i64 - 100).abs() <= 2,
        "ogg packets: {ogg_packets}"
    );
    assert!(
        (webm_packets as i64 - 100).abs() <= 2,
        "webm packets: {webm_packets}"
    );
}

#[test]
fn the_ogg_selection_emits_only_the_ogg_sibling() {
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("upload");
    let wav = synth_wav_440hz(0.5);
    std::fs::write(&staged, &wav).unwrap();
    process_staged_audio(&staged, "audio/wav", wav.len() as i64, AudioContainers::Ogg).unwrap();
    assert!(with_suffix(&staged, OPUS_SUFFIX).exists());
    assert!(!with_suffix(&staged, WEBM_SUFFIX).exists());
}

#[test]
fn the_webm_selection_emits_only_the_webm_sibling() {
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("upload");
    let wav = synth_wav_440hz(0.5);
    std::fs::write(&staged, &wav).unwrap();
    process_staged_audio(
        &staged,
        "audio/wav",
        wav.len() as i64,
        AudioContainers::WebM,
    )
    .unwrap();
    assert!(!with_suffix(&staged, OPUS_SUFFIX).exists());
    assert!(with_suffix(&staged, WEBM_SUFFIX).exists());
}

#[test]
fn over_cap_duration_falls_back_to_untranscoded_pass_through() {
    // A cheap over-cap proof: fabricate a WAV whose declared data length implies > 30 minutes
    // at a tiny sample rate, without actually encoding that many samples to disk.
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("upload");
    // 1 Hz sample rate, 2000 samples declared ⇒ 2000 seconds > MAX_AUDIO_DURATION_SECS,
    // while the actual PCM payload stays tiny.
    let mut wav = Vec::new();
    let declared_samples = 2000u32;
    let data_len = declared_samples * 2;
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u32.to_le_bytes()); // 1 Hz
    wav.extend_from_slice(&2u32.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(&vec![0u8; data_len as usize]);
    std::fs::write(&staged, &wav).unwrap();
    let processed = process_staged_audio(
        &staged,
        "audio/wav",
        wav.len() as i64,
        AudioContainers::Both,
    )
    .unwrap();
    assert!(!processed.converted);
    assert!(processed
        .meta
        .conversion_note
        .as_deref()
        .unwrap_or("")
        .contains("over-cap"));
    assert!(!with_suffix(&staged, OPUS_SUFFIX).exists());
    assert!(!with_suffix(&staged, WEBM_SUFFIX).exists());
}

#[test]
fn a_malformed_file_decodes_to_pass_through_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("upload");
    std::fs::write(&staged, b"not actually audio").unwrap();
    let processed = process_staged_audio(&staged, "audio/wav", 18, AudioContainers::Both).unwrap();
    assert!(!processed.converted);
    assert!(processed
        .meta
        .conversion_note
        .as_deref()
        .unwrap_or("")
        .contains("decode failed"));
}

/// Parse the `TrackType` (0x83) value out of the emitted WebM's TrackEntry, scanning only the
/// bytes before the first Cluster (payload bytes could alias the id). Element sizes are EBML
/// vints; this writer only ever emits 1-byte sizes, so the parse stays that narrow.
fn webm_track_type(bytes: &[u8]) -> Option<u64> {
    let cluster_off = bytes
        .windows(4)
        .position(|w| w == [0x1F, 0x43, 0xB6, 0x75])
        .unwrap_or(bytes.len());
    let head = &bytes[..cluster_off];
    let pos = head.iter().position(|&b| b == 0x83)?;
    let size_byte = *head.get(pos + 1)?;
    let size = if size_byte >= 0x80 {
        (size_byte & 0x7F) as usize
    } else {
        return None; // longer vint than this writer ever emits
    };
    let start = pos + 2;
    let mut value = 0u64;
    for &b in head.get(start..start + size)? {
        value = (value << 8) | b as u64;
    }
    Some(value)
}

/// Parse the granule position (8-byte LE at page offset 6) of the LAST Ogg page in `bytes`.
fn ogg_last_granule(bytes: &[u8]) -> Option<u64> {
    let pos = bytes.windows(4).rposition(|w| w == b"OggS")?;
    let g = bytes.get(pos + 6..pos + 14)?;
    Some(u64::from_le_bytes(g.try_into().ok()?))
}

/// Parse the pre-skip (u16 LE at offset 8 of the OpusHead payload) out of the Ogg file's
/// first packet.
fn ogg_head_pre_skip(bytes: &[u8]) -> Option<u16> {
    let pos = bytes.windows(8).position(|w| w == b"OpusHead")?;
    Some(u16::from_le_bytes(
        bytes.get(pos + 10..pos + 12)?.try_into().ok()?,
    ))
}

#[test]
fn the_webm_derivative_marks_its_track_as_audio_not_video() {
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("upload");
    let wav = synth_wav_440hz(0.5);
    std::fs::write(&staged, &wav).unwrap();
    process_staged_audio(
        &staged,
        "audio/wav",
        wav.len() as i64,
        AudioContainers::WebM,
    )
    .unwrap();
    let bytes = std::fs::read(with_suffix(&staged, WEBM_SUFFIX)).unwrap();
    // Matroska TrackType 2 is audio; 1 is video. A strict demuxer (WebKit's included) keys
    // the decode pipeline off this element.
    assert_eq!(webm_track_type(&bytes), Some(2));
}

#[test]
fn the_ogg_final_granule_trims_the_zero_padding_of_the_last_frame() {
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("upload");
    // 1.005 s at 48 kHz = 48240 valid samples = 50 full 960-sample frames + 240 — the padded
    // cumulative (51 × 960 = 48960) must NOT be the final granule.
    let wav = synth_wav_440hz_at(48_000, 1.005);
    std::fs::write(&staged, &wav).unwrap();
    process_staged_audio(&staged, "audio/wav", wav.len() as i64, AudioContainers::Ogg).unwrap();
    let bytes = std::fs::read(with_suffix(&staged, OPUS_SUFFIX)).unwrap();
    let pre_skip = ogg_head_pre_skip(&bytes).expect("OpusHead present");
    let final_granule = ogg_last_granule(&bytes).expect("an Ogg page exists");
    assert_eq!(final_granule, pre_skip as u64 + 48_240);
    assert!(final_granule < 51 * 960, "no padded granule");
}

/// A synthesized 5.1 WAV (6 channels, 44100 Hz, 16-bit PCM): every channel carries a distinct
/// constant level, so a misread channel count or a mismatched interleave would change the
/// decoded length or panic outright.
fn synth_wav_6ch(secs: f64) -> Vec<u8> {
    let sample_rate = 44_100u32;
    let channels = 6u16;
    let frames = (sample_rate as f64 * secs).round() as u32;
    let mut pcm = Vec::with_capacity(frames as usize * channels as usize * 2);
    for _ in 0..frames {
        for c in 0..channels {
            let level = ((c as i32 + 1) * 1000) as i16;
            pcm.extend_from_slice(&level.to_le_bytes());
        }
    }
    let mut wav = Vec::new();
    let data_len = pcm.len() as u32;
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * channels as u32 * 2).to_le_bytes());
    wav.extend_from_slice(&(channels * 2).to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(&pcm);
    wav
}

#[test]
fn a_five_point_one_wav_downmixes_to_stereo_instead_of_rejecting() {
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("upload");
    let wav = synth_wav_6ch(0.5);
    std::fs::write(&staged, &wav).unwrap();
    let processed = process_staged_audio(
        &staged,
        "audio/wav",
        wav.len() as i64,
        AudioContainers::Both,
    )
    .unwrap();
    // Never-reject posture: a >2-channel source transcodes (documented fold-down), it does
    // NOT fall back to pass-through and it cannot panic.
    assert!(processed.meta.conversion_note.is_none());
    let duration_ms = processed.meta.duration_ms.unwrap();
    assert!(
        (duration_ms - 500).abs() <= 60,
        "expected ~500ms, got {duration_ms}"
    );
    assert!(with_suffix(&staged, OPUS_SUFFIX).exists());
}
