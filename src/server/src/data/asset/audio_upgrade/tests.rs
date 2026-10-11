//! The sweep over stored assets.
//!
//! The tests cover selection from the database, the swap under the write barrier, the counts it
//! reports, and its one-file-at-a-time retry of a failed swap.

use std::path::Path;
use std::time::Duration;

use super::*;
use crate::auth::role::ServerRole;
use crate::data::asset::process::audio::{OPUS_SUFFIX, WEBM_SUFFIX};
use crate::data::asset::{Asset, AssetMeta};

/// Where the committed unmarked-layout fixtures live.
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/legacy-opus");

/// An in-memory repository with one world.
async fn repo_with_world() -> (SqliteRepository, Uuid) {
    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", gm, 0).await.unwrap();
    (repo, world.id)
}

/// Seeds an asset row (version 7, `duration_ms` recorded when `transcoded`) and writes the unmarked-layout
/// siblings of `residue` that `ogg` / `webm` ask for, plus the original WAV when `original`.
async fn seed(
    repo: &SqliteRepository,
    world: Uuid,
    root: &Path,
    residue: usize,
    transcoded: bool,
    (ogg, webm, original): (bool, bool, bool),
) -> Uuid {
    let id = Uuid::new_v4();
    let mut meta = AssetMeta::unprocessed("audio/wav", 0);
    if transcoded {
        meta.duration_ms = Some(250);
        meta.sample_rate = Some(48_000);
    }
    repo.insert_asset(&Asset {
        id,
        world_id: world,
        storage_key: format!("{world}/{id}"),
        original_name: "seeded.wav".into(),
        content_type: "audio/wav".into(),
        byte_size: 0,
        created_by: None,
        created_at: 0,
        version: 7,
        folder_id: None,
        tags: vec![],
        derived_tags: vec![],
        meta,
    })
    .await
    .unwrap();
    let dir = root.join(world.to_string());
    std::fs::create_dir_all(&dir).unwrap();
    let canonical = dir.join(id.to_string());
    for (wanted, suffix) in [(ogg, ".opus.ogg"), (webm, ".opus.webm")] {
        if wanted {
            let bytes =
                std::fs::read(Path::new(FIXTURES).join(format!("residue-{residue}{suffix}")))
                    .unwrap();
            std::fs::write(
                crate::data::asset::process::with_suffix(&canonical, suffix),
                bytes,
            )
            .unwrap();
        }
    }
    if original {
        let valid = 12 * 960 + residue;
        let wav = crate::data::asset::process::audio::tests::synth_wav_440hz_at(
            48_000,
            valid as f64 / 48_000.0,
        );
        std::fs::write(&canonical, wav).unwrap();
    }
    id
}

/// The asset's `.opus.webm` path.
fn webm_of(root: &Path, world: Uuid, id: Uuid) -> std::path::PathBuf {
    root.join(world.to_string())
        .join(format!("{id}{WEBM_SUFFIX}"))
}

/// The asset's `.opus.ogg` path.
fn ogg_of(root: &Path, world: Uuid, id: Uuid) -> std::path::PathBuf {
    root.join(world.to_string())
        .join(format!("{id}{OPUS_SUFFIX}"))
}

/// Only assets the audio pipeline transcoded (a recorded duration) are examined.
#[tokio::test]
async fn the_listing_names_only_transcoded_audio_assets() {
    let (repo, world) = repo_with_world().await;
    let root = tempfile::tempdir().unwrap();
    let audio = seed(&repo, world, root.path(), 648, true, (false, false, false)).await;
    let _other = seed(&repo, world, root.path(), 648, false, (false, false, false)).await;
    let listed = repo.list_transcoded_audio_assets().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].0, audio);
    assert_eq!(listed[0].1, format!("{world}/{audio}"));
}

/// One pass remuxes the asset outside the loss range, re-encodes the one inside it and leaves an
/// asset with no siblings alone; a second pass finds everything current and rewrites nothing; no
/// asset version changes.
#[tokio::test]
async fn a_pass_upgrades_old_assets_and_a_second_pass_rewrites_nothing() {
    let (repo, world) = repo_with_world().await;
    let root = tempfile::tempdir().unwrap();
    let outside = seed(&repo, world, root.path(), 648, true, (true, true, true)).await;
    let inside = seed(&repo, world, root.path(), 649, true, (true, true, true)).await;
    let bare = seed(&repo, world, root.path(), 648, true, (false, false, false)).await;
    let barrier = RwLock::new(());

    let first = upgrade_audio_derivatives(&repo, root.path(), &barrier).await;
    assert_eq!(
        first,
        UpgradeReport {
            examined: 3,
            current: 1,
            remuxed: 1,
            retranscoded: 1,
            ..UpgradeReport::default()
        }
    );
    for id in [outside, inside, bare] {
        assert_eq!(repo.get_asset(id).await.unwrap().unwrap().version, 7);
    }

    let stamp = |id| {
        let m = std::fs::metadata(webm_of(root.path(), world, id)).unwrap();
        (m.len(), m.modified().unwrap())
    };
    let before = (stamp(outside), stamp(inside));
    let second = upgrade_audio_derivatives(&repo, root.path(), &barrier).await;
    assert_eq!(
        second,
        UpgradeReport {
            examined: 3,
            current: 3,
            ..UpgradeReport::default()
        }
    );
    assert_eq!(
        before,
        (stamp(outside), stamp(inside)),
        "an already-current file is not rewritten"
    );
}

/// A swap that fails is counted as skipped, leaves the asset's old files intact and is retried and
/// completed by the next pass.
#[tokio::test]
async fn a_failed_swap_is_skipped_and_completed_by_the_next_pass() {
    fn refuse(_: &Path, _: &Path) -> std::io::Result<()> {
        Err(std::io::Error::other("injected"))
    }
    let (repo, world) = repo_with_world().await;
    let root = tempfile::tempdir().unwrap();
    let id = seed(&repo, world, root.path(), 649, true, (true, true, true)).await;
    let old_webm = std::fs::read(webm_of(root.path(), world, id)).unwrap();
    let old_ogg = std::fs::read(ogg_of(root.path(), world, id)).unwrap();
    let barrier = RwLock::new(());

    let first = upgrade_with(&repo, root.path(), &barrier, refuse).await;
    assert_eq!(first.skipped, 1);
    assert_eq!(first.written(), 0);
    assert_eq!(
        std::fs::read(webm_of(root.path(), world, id)).unwrap(),
        old_webm
    );
    assert_eq!(
        std::fs::read(ogg_of(root.path(), world, id)).unwrap(),
        old_ogg
    );

    let second = upgrade_audio_derivatives(&repo, root.path(), &barrier).await;
    assert_eq!(second.retranscoded, 1);
    assert_ne!(
        std::fs::read(ogg_of(root.path(), world, id)).unwrap(),
        old_ogg
    );
}

/// The swap holds the read side of the write barrier: while the write side is held (a backup or a
/// restore), the sweep stages its files and then waits, and it completes once the barrier frees.
#[tokio::test]
async fn the_swap_waits_for_the_write_barrier() {
    let (repo, world) = repo_with_world().await;
    let root = tempfile::tempdir().unwrap();
    let id = seed(&repo, world, root.path(), 648, true, (true, true, true)).await;
    let old_webm = std::fs::read(webm_of(root.path(), world, id)).unwrap();
    let barrier = RwLock::new(());
    let held = barrier.write().await;

    let pass = upgrade_audio_derivatives(&repo, root.path(), &barrier);
    tokio::pin!(pass);
    assert!(
        tokio::time::timeout(Duration::from_millis(500), &mut pass)
            .await
            .is_err(),
        "the pass is waiting on the barrier"
    );
    assert_eq!(
        std::fs::read(webm_of(root.path(), world, id)).unwrap(),
        old_webm
    );
    drop(held);
    let report = pass.await;
    assert_eq!(report.remuxed, 1);
    assert_ne!(
        std::fs::read(webm_of(root.path(), world, id)).unwrap(),
        old_webm
    );
}
