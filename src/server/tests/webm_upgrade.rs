//! The derivative upgrade sweep as a client of the asset routes sees it.
//!
//! A rewritten Opus sibling is a new validator without a version change, so a cached copy, a
//! conditional request and a resumed ranged read all notice, and the asset row is not touched.
//!
//! The unmarked-layout siblings are the committed output of the unmarked-layout muxer for a 48 kHz mono sine of
//! `12 * 960 + residue` samples; 648 is outside the tail-loss range (the WebM is remuxed, the Ogg
//! kept) and 649 inside it (both siblings are encoded again from the stored original).

use shadowcat::data::asset::audio_upgrade::upgrade_audio_derivatives;
use shadowcat::data::asset::AssetMeta;
use shadowcat_test_support as common;
use shadowcat_test_support::Harness;
use uuid::Uuid;

/// A mono 48 kHz PCM16 WAV of a 440 Hz sine with exactly `frames` samples.
fn sine_wav(frames: usize) -> Vec<u8> {
    let mut pcm = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let t = i as f64 / 48_000.0;
        let sample = (t * 440.0 * std::f64::consts::TAU).sin();
        pcm.extend_from_slice(&((sample * i16::MAX as f64) as i16).to_le_bytes());
    }
    let data_len = pcm.len() as u32;
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&48_000u32.to_le_bytes());
    wav.extend_from_slice(&96_000u32.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(&pcm);
    wav
}

/// The committed unmarked-layout sibling bytes.
fn legacy(residue: usize, suffix: &str) -> Vec<u8> {
    let path = format!(
        "{}/tests/fixtures/legacy-opus/residue-{residue}{suffix}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(path).unwrap()
}

/// Seeds an asset whose canonical is the sine WAV and whose siblings are the unmarked-layout files.
async fn seed(h: &Harness, residue: usize) -> Uuid {
    let ogg = legacy(residue, ".opus.ogg");
    let webm = legacy(residue, ".opus.webm");
    let wav = sine_wav(12 * 960 + residue);
    let mut meta = AssetMeta::unprocessed("audio/wav", wav.len() as i64);
    meta.duration_ms = Some(250);
    meta.sample_rate = Some(48_000);
    h.seed_asset(
        "audio/wav",
        meta,
        &[("", &wav), (".opus.ogg", &ogg), (".opus.webm", &webm)],
    )
    .await
}

/// A GET of `variant` with `headers`.
async fn get(
    h: &Harness,
    id: Uuid,
    variant: &str,
    headers: &[(&str, String)],
) -> reqwest::Response {
    let mut request = h.client.get(format!(
        "http://{}/api/assets/{id}?variant={variant}",
        h.addr
    ));
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    request.send().await.unwrap()
}

/// The `ETag` of `variant`.
async fn etag(h: &Harness, id: Uuid, variant: &str) -> String {
    let response = get(h, id, variant, &[]).await;
    assert_eq!(response.status(), 200);
    response.headers()["etag"].to_str().unwrap().to_string()
}

/// The sweep outside the loss range replaces the WebM and keeps the Ogg byte-identical, so the
/// WebM's validator changes and the Ogg's does not, and the old validator is neither a 304 nor a
/// valid `If-Range`; the asset row is unchanged.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_remuxed_webm_is_a_new_validator_and_the_ogg_is_unchanged() {
    let h = common::spawn().await;
    let id = seed(&h, 648).await;
    let old_webm = etag(&h, id, "opus-webm").await;
    let old_ogg = etag(&h, id, "opus").await;
    let barrier = tokio::sync::RwLock::new(());

    let report = upgrade_audio_derivatives(&h.repo, &h.assets_dir, &barrier).await;
    assert_eq!(
        (report.remuxed, report.retranscoded, report.failed),
        (1, 0, 0)
    );

    let new_webm = etag(&h, id, "opus-webm").await;
    assert_ne!(new_webm, old_webm);
    assert_eq!(
        etag(&h, id, "opus").await,
        old_ogg,
        "the Ogg was not rewritten"
    );
    assert_eq!(h.repo.get_asset(id).await.unwrap().unwrap().version, 3);

    let revalidated = get(&h, id, "opus-webm", &[("if-none-match", old_webm.clone())]).await;
    assert_eq!(revalidated.status(), 200, "the old validator is not a 304");
    let current = get(&h, id, "opus-webm", &[("if-none-match", new_webm.clone())]).await;
    assert_eq!(current.status(), 304);

    // A stream opened before the swap and resumed after: the old validator in `If-Range` yields the
    // whole new file, never a slice of the new bytes.
    let resumed = get(
        &h,
        id,
        "opus-webm",
        &[("range", "bytes=100-".into()), ("if-range", old_webm)],
    )
    .await;
    assert_eq!(resumed.status(), 200);
    let sliced = get(
        &h,
        id,
        "opus-webm",
        &[("range", "bytes=100-199".into()), ("if-range", new_webm)],
    )
    .await;
    assert_eq!(sliced.status(), 206);
}

/// Inside the loss range both siblings are encoded again, so BOTH validators change, the version
/// stays and a second pass finds nothing to do.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_loss_range_asset_changes_both_validators() {
    let h = common::spawn().await;
    let id = seed(&h, 649).await;
    let before = (etag(&h, id, "opus-webm").await, etag(&h, id, "opus").await);
    let barrier = tokio::sync::RwLock::new(());

    let report = upgrade_audio_derivatives(&h.repo, &h.assets_dir, &barrier).await;
    assert_eq!((report.retranscoded, report.failed), (1, 0));
    let after = (etag(&h, id, "opus-webm").await, etag(&h, id, "opus").await);
    assert_ne!(after.0, before.0);
    assert_ne!(after.1, before.1);
    assert_eq!(h.repo.get_asset(id).await.unwrap().unwrap().version, 3);

    let again = upgrade_audio_derivatives(&h.repo, &h.assets_dir, &barrier).await;
    assert_eq!((again.current, again.written()), (1, 0));
    assert_eq!(etag(&h, id, "opus-webm").await, after.0);
}
