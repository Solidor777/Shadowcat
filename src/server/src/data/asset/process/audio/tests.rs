use super::*;

/// A synthesized 440 Hz sine WAV, `secs` long, mono 44100 Hz 16-bit PCM — the RIFF/WAVE
/// header is built by hand (no `hound` dev-dependency: ~30 lines once, zero new crates).
fn synth_wav_440hz(secs: f64) -> Vec<u8> {
    let sample_rate = 44_100u32;
    let n = (sample_rate as f64 * secs) as u32;
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
