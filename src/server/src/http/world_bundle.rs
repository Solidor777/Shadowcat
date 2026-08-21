//! `POST /api/worlds/{id}/export` and `POST /api/worlds/import` — per-world
//! bundle export/import. The bundle format itself (`manifest.json` +
//! `rows/<table>.jsonl` + `assets/<asset_id>`, with `manifest.json` always
//! the tar's first entry) is `crate::world_bundle`'s concern; this module is
//! the HTTP boundary around it. Export is world-GM-gated (`require_gm`,
//! mirroring the existing GM-gated asset routes). Import is
//! server-admin-only: a bulk multi-table insert that bypasses every
//! capability/schema/OCC gate the live write paths enforce (the same
//! trusted-substrate posture `apply_command`'s replay path already has) — a
//! materially more privileged operation than ordinary world CREATION
//! (`POST /api/worlds`, open to any authenticated user) or GM-level world
//! management, so it needs the server's highest tier, not a match to either
//! of those.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

use crate::auth::session::AuthUser;
use crate::http::error::AppError;
use crate::http::routes::require_gm;
use crate::http::AppState;
use crate::world_bundle::write_bundle;

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

/// `POST /api/worlds/{id}/export` — world-GM-gated (server admins resolve to
/// GM via `require_gm`). Streams the world's `.tar` bundle as the response
/// body: `write_bundle` runs on a blocking thread and writes into a
/// `ChannelWriter`, whose bounded channel this function turns directly into
/// the response body stream — bytes reach the client as `write_bundle`
/// produces them, and memory usage stays bounded by
/// `EXPORT_CHANNEL_CAPACITY` regardless of the world's total asset size.
pub async fn export_world(
    State(state): State<AppState>,
    user: AuthUser,
    Path(world): Path<Uuid>,
) -> Result<Response, AppError> {
    require_gm(&state, &user, world).await?;
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
}
