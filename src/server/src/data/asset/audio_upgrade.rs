//! This module is the background sweep that upgrades old Opus derivatives.
//!
//! A derivative is old when an older encoder or muxer wrote it.
//!
//! See `process::audio::upgrade` for what makes a file old and how each is repaired.
//!
//! The sweep runs once per server start, after the listener is bound, one asset at a time. The
//! decode and mux work of an asset runs on the blocking pool without any lock, and only the swap
//! (a rename of a staged file over each sibling) holds the read side of the write barrier, like
//! every other commit-and-file-move pair. It writes no database row, bumps no asset version and
//! broadcasts nothing: the derivative's `ETag` is derived from the served file's bytes, so a
//! rewritten sibling is a new validator for every cache without a version change.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::RwLock;
use uuid::Uuid;

use super::process::audio::upgrade::{self, Applied, Kind, Plan, Rename};
use crate::data::sqlite::SqliteRepository;

/// What one pass of the sweep did.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::audio_upgrade::UpgradeReport;
///
/// let report = UpgradeReport::default();
/// assert_eq!(report.examined, 0);
/// assert_eq!(report.written(), 0);
/// ```
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UpgradeReport {
    /// Transcoded audio assets examined.
    pub examined: usize,
    /// Assets whose siblings were already current.
    pub current: usize,
    /// Assets whose `.opus.webm` was remuxed from its `.opus.ogg`'s packets.
    pub remuxed: usize,
    /// Assets re-encoded from the stored original (tail-loss assets and WebM-only assets).
    pub retranscoded: usize,
    /// Tail-loss assets with no stored original: the WebM was remuxed to its decodable length and
    /// the lost tail cannot be recovered.
    pub unrepairable_tail: usize,
    /// Assets selected where no write could improve the files, or whose files changed under the
    /// sweep or whose swap failed: selected again by the next pass.
    pub skipped: usize,
    /// Assets whose staging failed (an unreadable sibling, a failed decode).
    pub failed: usize,
}

impl UpgradeReport {
    /// Assets whose files the pass rewrote.
    pub fn written(&self) -> usize {
        self.remuxed + self.retranscoded + self.unrepairable_tail
    }
}

/// Upgrades every stored audio asset's Opus siblings that an older encoder or muxer wrote, one
/// asset at a time, and reports what it did. `assets_dir` is the asset store root and `barrier`
/// the write barrier whose read side the swap holds.
///
/// # Examples
///
/// ```
/// # #[tokio::main] async fn main() {
/// use shadowcat::data::asset::audio_upgrade::upgrade_audio_derivatives;
/// use shadowcat::data::sqlite::SqliteRepository;
///
/// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
/// let dir = tempfile::tempdir().unwrap();
/// let barrier = tokio::sync::RwLock::new(());
/// let report = upgrade_audio_derivatives(&repo, dir.path(), &barrier).await;
/// assert_eq!(report.examined, 0);
/// # }
/// ```
pub async fn upgrade_audio_derivatives(
    repo: &SqliteRepository,
    assets_dir: &Path,
    barrier: &RwLock<()>,
) -> UpgradeReport {
    upgrade_with(repo, assets_dir, barrier, upgrade::rename_file).await
}

/// `upgrade_audio_derivatives` with the rename each swap performs, so a test can fail one.
pub(crate) async fn upgrade_with(
    repo: &SqliteRepository,
    assets_dir: &Path,
    barrier: &RwLock<()>,
    rename: Rename,
) -> UpgradeReport {
    let mut report = UpgradeReport::default();
    let assets = match repo.list_transcoded_audio_assets().await {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(error = %e, "audio derivative upgrade: listing assets failed");
            return report;
        }
    };
    for (id, storage_key) in assets {
        report.examined += 1;
        upgrade_one(
            id,
            assets_dir.join(storage_key),
            barrier,
            rename,
            &mut report,
        )
        .await;
    }
    report
}

/// Plans, stages and swaps one asset, recording the outcome in `report`.
async fn upgrade_one(
    id: Uuid,
    canonical: PathBuf,
    barrier: &RwLock<()>,
    rename: Rename,
    report: &mut UpgradeReport,
) {
    let planned = tokio::task::spawn_blocking(move || upgrade::plan(&canonical)).await;
    let (kind, staged) = match planned {
        Ok(Ok(Plan::Current)) => {
            report.current += 1;
            return;
        }
        Ok(Ok(Plan::Unrepairable(reason))) => {
            tracing::debug!(%id, reason, "audio derivative upgrade: nothing to write");
            report.skipped += 1;
            return;
        }
        Ok(Ok(Plan::Write { kind, staged })) => (kind, staged),
        Ok(Err(error)) => {
            tracing::warn!(%id, %error, "audio derivative upgrade: staging failed");
            report.failed += 1;
            return;
        }
        Err(error) => {
            tracing::warn!(%id, %error, "audio derivative upgrade: planning task failed");
            report.failed += 1;
            return;
        }
    };
    // The swap is a commit-and-file-move pair, so it holds the read side like every other one.
    let _file_ops = barrier.read().await;
    let applied = tokio::task::spawn_blocking(move || upgrade::apply(&staged, rename)).await;
    match applied {
        Ok(Applied::Done(files)) => match kind {
            Kind::Remux => {
                report.remuxed += 1;
                tracing::info!(%id, files, "audio derivative upgrade: webm remuxed");
            }
            Kind::Retranscode => {
                report.retranscoded += 1;
                tracing::info!(%id, files, "audio derivative upgrade: re-encoded from the stored original");
            }
            Kind::RemuxToDecodableLength => {
                report.unrepairable_tail += 1;
                tracing::warn!(
                    %id,
                    "audio derivative upgrade: the stored original is missing, so the samples an older encoder dropped from the end cannot be recovered; the webm was remuxed to its decodable length"
                );
            }
        },
        Ok(Applied::Skipped(reason)) => {
            tracing::warn!(%id, %reason, "audio derivative upgrade: swap skipped, retried on the next start");
            report.skipped += 1;
        }
        Err(error) => {
            tracing::warn!(%id, %error, "audio derivative upgrade: swap task failed");
            report.failed += 1;
        }
    }
}

/// Starts `upgrade_audio_derivatives` as a detached task, for the server to call once its
/// listener is bound. The task ends when the pass does.
///
/// # Examples
///
/// ```
/// # #[tokio::main] async fn main() {
/// use shadowcat::data::asset::audio_upgrade::spawn_audio_upgrade;
/// use shadowcat::data::sqlite::SqliteRepository;
/// use std::sync::Arc;
///
/// let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
/// let barrier = Arc::new(tokio::sync::RwLock::new(()));
/// let report = spawn_audio_upgrade(repo, std::env::temp_dir(), barrier).await.unwrap();
/// assert_eq!(report.examined, 0);
/// # }
/// ```
pub fn spawn_audio_upgrade(
    repo: Arc<SqliteRepository>,
    assets_dir: PathBuf,
    barrier: Arc<RwLock<()>>,
) -> tokio::task::JoinHandle<UpgradeReport> {
    tokio::spawn(async move { upgrade_audio_derivatives(&repo, &assets_dir, &barrier).await })
}

#[cfg(test)]
mod tests;
