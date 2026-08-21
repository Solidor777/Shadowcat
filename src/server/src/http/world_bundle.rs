//! `POST /api/worlds/{id}/export` and `POST /api/worlds/import` — per-world
//! bundle export/import. The bundle format itself (`manifest.json` +
//! `rows/<table>.jsonl` + `assets/<asset_id>`, with `manifest.json` always
//! the tar's first entry) is `crate::world_bundle`'s concern; this module is
//! the HTTP boundary around it. BOTH routes are server-admin-only
//! (`AdminUser`): export is not GM-gated, because `export_world_rows`
//! (`data::sqlite`) selects every `documents` row for the world verbatim
//! with no `gm_role`-based redaction — a world's own GM could otherwise
//! export and read whisper content (`chat::mod.rs`'s `Audience::Whisper`
//! sets `permissions.gm_role: Some(DocRole::None)` specifically so the GM
//! does not get unconditional access) that the live API denies them, and
//! `world_events.command_json` carries the same unfiltered content. Import
//! is server-admin-only for a second, independent reason: a bulk
//! multi-table insert that bypasses every capability/schema/OCC gate the
//! live write paths enforce (the same trusted-substrate posture
//! `apply_command`'s replay path already has) — a materially more
//! privileged operation than ordinary world CREATION (`POST /api/worlds`,
//! open to any authenticated user) or GM-level world management. Export and
//! import are a full-fidelity data-migration primitive, the same category as
//! `backup`/`restore` (already admin-only and unredacted) — not a live-view
//! read surface, so redacting content at export time would make bundles
//! lossy instead.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use axum::body::Body;
use axum::extract::{Multipart, Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::Json;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::auth::session::AdminUser;
use crate::data::world_bundle::ImportSummary;
use crate::http::error::AppError;
use crate::http::AppState;
use crate::world_bundle::{read_bundle, write_bundle, WorldBundleError};

/// Bounds the export channel's in-flight chunk count — memory usage during
/// export is this times `write_bundle`'s internal write-call size (a few
/// tens of KB per `std::io::copy` buffer), never the total tar size. A full
/// channel makes `ChannelWriter::write` (running on the blocking thread)
/// wait for the HTTP body to drain a prior chunk before producing the next,
/// so `write_bundle` cannot outrun the response.
const EXPORT_CHANNEL_CAPACITY: usize = 8;

/// A `std::io::Write` adapter that forwards each write call as one chunk
/// over a bounded channel, letting `write_bundle` (run inside
/// `spawn_blocking`) feed an HTTP response body incrementally instead of
/// materializing the whole tar in memory first.
struct ChannelWriter {
    /// One chunk per `write` call.
    tx: tokio::sync::mpsc::Sender<Result<Vec<u8>, std::io::Error>>,
}

impl std::io::Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.tx.blocking_send(Ok(buf.to_vec())).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "export receiver dropped")
        })?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// `POST /api/worlds/{id}/export` — server-admin-only (see this module's own
/// doc for why this is not GM-gated: `export_world_rows` has no
/// `gm_role`-based redaction). Holds `state.write_barrier`'s read side across
/// the whole streamed response, so a concurrent `POST /api/admin/backup`
/// snapshot can't interleave with the row read + asset streaming below (same
/// accepted trade-off `assets.rs`'s `DefaultBodyLimit`-disabled upload routes
/// already document: a slow export download can hold the permit a long time,
/// same class as a slow uploader). Streams the world's `.tar` bundle as the
/// response body: `write_bundle` runs on a blocking thread and writes into a
/// `ChannelWriter`, whose bounded channel this function turns directly into
/// the response body stream — bytes reach the client as `write_bundle`
/// produces them, and memory usage stays bounded by
/// `EXPORT_CHANNEL_CAPACITY` regardless of the world's total asset size.
pub async fn export_world(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(world): Path<Uuid>,
) -> Result<Response, AppError> {
    // Owned (not borrowed) so it can move into the detached task below,
    // which outlives this function's own async body — an ordinary
    // `.read().await` guard is tied to a borrow of `state.write_barrier`
    // and cannot cross that move.
    let read_permit = state.write_barrier.clone().read_owned().await;
    let data = state.repo.export_world_rows(world).await?;
    let assets_dir = state.config.assets_path();

    let (tx, rx) =
        tokio::sync::mpsc::channel::<Result<Vec<u8>, std::io::Error>>(EXPORT_CHANNEL_CAPACITY);
    // A second handle to report a mid-stream failure: `write_bundle` takes
    // `tx` by value (via `ChannelWriter`) and drops it on return, so only a
    // clone taken up front survives to send the error chunk afterward.
    let err_tx = tx.clone();
    let handle = tokio::task::spawn_blocking(move || {
        let writer = ChannelWriter { tx };
        if let Err(e) = write_bundle(&data, &assets_dir, writer) {
            let _ = err_tx.blocking_send(Err(std::io::Error::other(e.to_string())));
        }
    });
    tokio::spawn(async move {
        // Held until `write_bundle`'s blocking task has produced every
        // chunk (the whole tar-writing phase), so a concurrent backup
        // snapshot can never interleave with this export's row read +
        // asset file reads.
        let _read_permit = read_permit;
        if let Err(e) = handle.await {
            tracing::error!(?e, %world, "world export task panicked");
        }
    });

    let body_stream = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    });

    Ok((
        [
            (header::CONTENT_TYPE, "application/x-tar".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"world-{world}.tar\""),
            ),
        ],
        Body::from_stream(body_stream),
    )
        .into_response())
}

/// Defensive cap on an uploaded bundle's total byte size — this endpoint has
/// no per-user rate limit (server-admin-only, not a hot path), so an
/// unbounded upload would still be an uncapped-disk-write vector even for a
/// trusted admin session. Generous: a world's assets can legitimately be
/// large.
const MAX_IMPORT_BUNDLE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Removes the wrapped path on drop unless `keep()` was called — the single
/// source of truth for staged-import-upload cleanup, so a temp file cannot
/// survive an early return (present or future) without a `remove_file` call
/// repeated at every failure site. That alone (multipart error, oversize
/// upload, I/O failure, `Err(WorldBundleError)`, `Err(DataError)`) is
/// sufficient reason for this guard to exist: every one of those is an
/// ordinary early return, unwinding nothing, and the guard's `Drop` fires on
/// every one of them exactly like any other scope exit.
///
/// It additionally covers a `spawn_blocking(read_bundle)` panic reaching
/// this guard's scope as an unwind — but only in a build where a panic
/// unwinds at all. This crate's shipped **release** profile sets
/// `panic = "abort"` (`Cargo.toml`), under which a panic on ANY thread,
/// including a `spawn_blocking` worker, aborts the whole process
/// immediately; there is no unwind for any `Drop` to run during, so in a
/// release binary this guard cannot and does not protect against that case
/// — an accepted, pre-existing characteristic of the abort profile, not
/// something a `Drop` impl is ever able to change. In a dev/test build
/// (default `panic = "unwind"`), `spawn_blocking` itself converts a
/// panicked task into a `JoinError`, so the panic is caught and re-surfaced
/// as an ordinary `Err` on the awaiting side well before it would reach
/// this guard — the shape this guard actually sees there is the same
/// ordinary early return as every other failure path. The removal itself is
/// a synchronous `std::fs::remove_file`, not a spawned task: a fire-and-
/// forget `tokio::spawn` from inside `drop` could be dropped before it runs
/// if the runtime is tearing down, where a blocking call on the current
/// thread is guaranteed to complete before `drop` returns.
struct TempFileGuard(Option<std::path::PathBuf>);

impl TempFileGuard {
    /// Arms cleanup for `path`.
    fn new(path: std::path::PathBuf) -> Self {
        Self(Some(path))
    }

    /// Cancels the cleanup. No production call site needs this today (the
    /// one `TempFileGuard` this module creates is always explicitly
    /// `drop`ped once its file is no longer needed — see `import_world`),
    /// so this stays test-only rather than shipping an unused release-build
    /// method; it exists to let a test assert the disarmed half of the
    /// guard's contract directly, the same way
    /// `temp_file_guard_removes_the_file_on_ordinary_drop` asserts the armed
    /// half.
    #[cfg(test)]
    fn keep(mut self) {
        self.0 = None;
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Stream the multipart field named `"file"` to `dest`, enforcing
/// `MAX_IMPORT_BUNDLE_BYTES` as bytes arrive (never buffering the whole
/// body) — including bytes belonging to any non-`"file"` field skipped
/// before it, so a request cannot smuggle unbounded bytes past the cap by
/// stuffing them into an earlier field. Cleanup of `dest` on an early return
/// is the CALLER's responsibility (`import_world` holds the sole
/// `TempFileGuard` for this path, spanning both this call and the
/// extraction step that follows it) — this function creates and writes the
/// file but owns no guard of its own, so exactly one `TempFileGuard`
/// instance ever exists per physical temp file.
async fn stream_bundle_upload(
    mut multipart: Multipart,
    dest: &std::path::Path,
) -> Result<(), AppError> {
    let mut skip_total: u64 = 0;
    let mut field = loop {
        let Some(mut f) = multipart
            .next_field()
            .await
            .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?
        else {
            return Err(AppError::BadRequest("missing file field".into()));
        };
        if f.name() == Some("file") {
            break f;
        }
        // Drain and discard a non-"file" field's bytes ourselves, under the
        // same running cap the "file" field's own chunk loop enforces below
        // — otherwise this field's bytes would arrive (and get discarded by
        // `next_field`'s own internal advance) with no size accounting at
        // all.
        while let Some(c) = f
            .chunk()
            .await
            .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?
        {
            skip_total += c.len() as u64;
            if skip_total > MAX_IMPORT_BUNDLE_BYTES {
                return Err(AppError::PayloadTooLarge(format!(
                    "bundle exceeds {MAX_IMPORT_BUNDLE_BYTES} bytes"
                )));
            }
        }
    };
    let mut file = tokio::fs::File::create(dest).await.map_err(|e| {
        tracing::error!(?e, "failed to create import upload temp file");
        AppError::Internal
    })?;
    let mut total: u64 = 0;
    loop {
        let chunk = match field.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => {
                return Err(AppError::BadRequest(format!("multipart error: {e}")));
            }
        };
        total += chunk.len() as u64;
        if total > MAX_IMPORT_BUNDLE_BYTES {
            return Err(AppError::PayloadTooLarge(format!(
                "bundle exceeds {MAX_IMPORT_BUNDLE_BYTES} bytes"
            )));
        }
        if let Err(e) = file.write_all(&chunk).await {
            tracing::error!(?e, "failed writing import upload temp file");
            return Err(AppError::Internal);
        }
    }
    file.flush().await.map_err(|e| {
        tracing::error!(?e, "failed flushing import upload temp file");
        AppError::Internal
    })?;
    Ok(())
}

/// `POST /api/worlds/import` — server-admin-only multipart upload of a
/// `.tar` bundle. Streams the upload to a local temp file first (never
/// buffers the whole body), extracts it (schema-version- and
/// row-count-checked before any DB row is touched), then inserts everything
/// in one transaction. See `SqliteRepository::import_world` for the
/// collision-reject/username-resolution/asset-finalize behavior. Holds the
/// SOLE `TempFileGuard` for the staged temp file, spanning both the upload
/// (`stream_bundle_upload`) and the extraction (`read_bundle`) steps, so the
/// file is removed on any failure in either — see `TempFileGuard`'s own doc
/// for exactly which failure shapes that covers. Also holds
/// `state.write_barrier`'s read side across the upload, extraction, and the
/// `SqliteRepository::import_world` call (whose asset-finalization step
/// renames staged files into the live asset tree exactly like
/// `assets::upload` does) — the same protection `assets.rs`'s own
/// upload/replace routes give that operation, so a concurrent backup
/// snapshot can't interleave with import's asset writes either.
pub async fn import_world(
    _admin: AdminUser,
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<Json<ImportSummary>, AppError> {
    let _read_permit = state.write_barrier.read().await;
    let tmp_tar = std::env::temp_dir().join(format!("shadowcat-import-{}.tar", Uuid::new_v4()));
    let guard = TempFileGuard::new(tmp_tar.clone());
    stream_bundle_upload(multipart, &tmp_tar).await?;

    let assets_dir = state.config.assets_path();
    let tar_path = tmp_tar.clone();
    let import_result = tokio::task::spawn_blocking(move || read_bundle(&tar_path, &assets_dir))
        .await
        .map_err(|e| {
            tracing::error!(?e, "world import extraction task panicked");
            AppError::Internal
        })?;
    // Extraction is done with the staged file either way — drop the guard
    // here (rather than letting it ride to the end of the function) so the
    // temp file is removed before the DB insert, not after.
    drop(guard);

    let import_data = match import_result {
        Ok(d) => d,
        Err(e) => {
            return Err(match e {
                WorldBundleError::Malformed(m) => AppError::BadRequest(m),
                WorldBundleError::RowCountMismatch {
                    table,
                    expected,
                    actual,
                } => AppError::BadRequest(format!(
                    "row count mismatch for '{table}': expected {expected}, got {actual}"
                )),
                WorldBundleError::UnsupportedSchemaVersion(v) => {
                    AppError::BadRequest(format!("unsupported bundle schema_version {v}"))
                }
                WorldBundleError::Serde(e) => {
                    AppError::BadRequest(format!("malformed bundle content: {e}"))
                }
                WorldBundleError::Io(e) => {
                    // Server-side fault (e.g. disk full during asset
                    // extraction), not necessarily client-caused — mirrors
                    // `DataError::Sqlx`/`Serde`'s logged-only, no-echo
                    // mapping rather than `AppError::BadRequest`'s.
                    tracing::error!(?e, "world import extraction I/O error");
                    AppError::Internal
                }
            });
        }
    };

    let summary = state.repo.import_world(import_data).await?;
    Ok(Json(summary))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Confirms `ChannelWriter` genuinely backpressures its blocking writer
    /// against the channel's capacity, rather than buffering ahead of it —
    /// the property `export_world`'s memory-bounded streaming claim rests
    /// on. With capacity 1, a writer producing two chunks must block inside
    /// the second `blocking_send` until the first chunk is drained: with
    /// nothing draining the channel, the writer thread cannot have finished
    /// both writes no matter how long it runs — proven via a completion
    /// flag the writer thread sets only after `write_all` for "second"
    /// returns, never via a `try_recv` race with the writer thread itself.
    #[test]
    fn channel_writer_blocks_the_writer_thread_once_the_channel_is_full() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<Vec<u8>, std::io::Error>>(1);
        let mut writer = ChannelWriter { tx };
        let second_write_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done_flag = second_write_done.clone();

        let handle = std::thread::spawn(move || {
            std::io::Write::write_all(&mut writer, b"first").unwrap();
            std::io::Write::write_all(&mut writer, b"second").unwrap();
            done_flag.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        // Give the writer thread ample time to run — if it were not
        // backpressured by the full channel, both writes (and the flag
        // store) would already have completed.
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(
            !second_write_done.load(std::sync::atomic::Ordering::SeqCst),
            "second write completed with the channel undrained — no backpressure"
        );

        // Draining the one slot is what unblocks the writer thread.
        let first = rx.blocking_recv().expect("first chunk").unwrap();
        assert_eq!(first, b"first");
        let second = rx.blocking_recv().expect("second chunk").unwrap();
        assert_eq!(second, b"second");

        handle.join().unwrap();
        assert!(second_write_done.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn temp_file_guard_removes_the_file_on_ordinary_drop() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("guarded.tmp");
        std::fs::write(&path, b"x").unwrap();
        {
            let _guard = TempFileGuard::new(path.clone());
        }
        assert!(!path.exists());
    }

    #[test]
    fn temp_file_guard_keep_prevents_removal() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("kept.tmp");
        std::fs::write(&path, b"x").unwrap();
        let guard = TempFileGuard::new(path.clone());
        guard.keep();
        assert!(path.exists());
    }

    /// Models the exact shape `import_world` relies on: a guard held across
    /// a fallible operation, then an early return via `?` (the shape a
    /// `spawn_blocking(read_bundle)` panic takes once `JoinError` propagates
    /// out of the `map_err(..)?` — a normal early return, not a Rust panic,
    /// since `spawn_blocking` converts a panicked task into an `Err` value).
    /// The guard local goes out of scope on that early return exactly as it
    /// would on any other path, so cleanup is unconditional.
    #[test]
    fn temp_file_guard_removes_the_file_on_early_return_via_question_mark() {
        fn helper(path: std::path::PathBuf) -> Result<(), AppError> {
            let _guard = TempFileGuard::new(path);
            let inner: Result<(), AppError> = Err(AppError::Internal);
            inner?;
            Ok(())
        }
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("early-return.tmp");
        std::fs::write(&path, b"x").unwrap();
        assert!(helper(path.clone()).is_err());
        assert!(!path.exists());
    }

    /// A genuine Rust panic while the guard is live — belt-and-suspenders on
    /// top of the `?`-based test above, confirming `Drop::drop` also runs
    /// during unwind, not just on a normal early return.
    #[test]
    fn temp_file_guard_removes_the_file_on_panic_unwind() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("panicked.tmp");
        std::fs::write(&path, b"x").unwrap();
        let path_for_panic = path.clone();
        let result = std::panic::catch_unwind(move || {
            let _guard = TempFileGuard::new(path_for_panic);
            panic!("simulated panic while the guard is live");
        });
        assert!(result.is_err());
        assert!(
            !path.exists(),
            "guard must remove the file during panic unwind"
        );
    }
}
