//! The vendored `h2` releases the stream slots of a stream whose reset is scheduled when the
//! connection ends.
//!
//! A server that finishes its send half while the response DATA is queued behind a zero
//! send window, with the request still open, drops its handles and leaves the stream in
//! `Closed(ScheduledLibraryReset(NO_ERROR))`: the reset can never be written, and unpatched `h2`
//! never converts the state, so the stream stays counted and `Counts::drop` asserts
//! `!self.has_streams()` when the connection is dropped. The assertion exists only under
//! `debug_assertions`; without them every test in this file would pass vacuously, so the whole
//! file is compiled out there and the default-profile `cargo test` is the run that counts.
//!
//! Each connection end is driven over `tokio::io::duplex` (no sockets) through `Gate`, whose
//! writes park once `block` is set (a peer that stopped reading) and whose reads fail on demand.
//!
//! Two arms of the vendored `State` are under test. The clean-EOF test needs the
//! `State::recv_eof` arm. A transport failure runs `Streams::handle_error` (the failure sweep)
//! and then, when the connection drops, `Streams::recv_eof` (the drop sweep). A stream whose
//! reset-expiration window is still open (`Stream::reset_at` is `Some`, so
//! `Stream::is_released` is false) survives the failure sweep and is released by the drop sweep's
//! `State::recv_eof` arm. A stream whose window has already expired is removed from the store by
//! the failure sweep's `Counts::transition_after` while still counted, so the drop sweep never
//! finds it and only the `State::handle_error` arm releases it. The reset-expiration window is
//! `h2::server::Builder::reset_stream_duration` (default `h2`'s `DEFAULT_RESET_STREAM_SECS`, one
//! second); `Duration::ZERO` makes the window expired by the time the failure is injected, so
//! the state that needs the `handle_error` arm is constructed directly rather than waited for.
#![cfg(debug_assertions)]

use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use bytes::Bytes;
use http::{Method, Request, Response};
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};

/// Failure injection shared between a `Gate` and the test that drives it.
#[derive(Default)]
struct Fail {
    kind: Option<io::ErrorKind>,
    waker: Option<Waker>,
}

/// Server-side transport: writes park forever once `block` is set, and every read and write
/// fails with `Fail::kind` once it is set.
struct Gate {
    inner: DuplexStream,
    block: Arc<AtomicBool>,
    fail: Arc<Mutex<Fail>>,
}

/// Makes every later read and write of `fail`'s `Gate` return `kind`, and wakes a parked read.
fn fail_io(fail: &Mutex<Fail>, kind: io::ErrorKind) {
    let mut guard = fail.lock().unwrap();
    guard.kind = Some(kind);
    if let Some(waker) = guard.waker.take() {
        waker.wake();
    }
}

impl AsyncRead for Gate {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        {
            let mut guard = self.fail.lock().unwrap();
            if let Some(kind) = guard.kind {
                return Poll::Ready(Err(kind.into()));
            }
            guard.waker = Some(cx.waker().clone());
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for Gate {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if let Some(kind) = self.fail.lock().unwrap().kind {
            return Poll::Ready(Err(kind.into()));
        }
        if self.block.load(Ordering::SeqCst) {
            return Poll::Pending;
        }
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Some(kind) = self.fail.lock().unwrap().kind {
            return Poll::Ready(Err(kind.into()));
        }
        if self.block.load(Ordering::SeqCst) {
            return Poll::Pending;
        }
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// How the connection ends once the stream is in its scheduled-reset state.
#[derive(Clone, Copy, Debug)]
enum End {
    /// The peer closes its side: the server reads end-of-file.
    CleanEof,
    /// The transport fails with the given kind.
    Io(io::ErrorKind),
}

/// Runs one connection to its end and returns whether the server task panicked.
///
/// `reset_duration` is the server's `reset_stream_duration` (`None` keeps `h2`'s default).
///
/// The client opens a POST with the initial send `window`, so the server's response DATA and its
/// END_STREAM stay queued; the server drops every handle while the request is still open
/// (`NO_ERROR` scheduled reset), then the connection ends as `end` says.
async fn connection_end_panicked(
    end: End,
    writer_parked: bool,
    window: u32,
    reset_duration: Option<Duration>,
) -> bool {
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let block = Arc::new(AtomicBool::new(false));
    let fail = Arc::new(Mutex::new(Fail::default()));
    let gate = Gate {
        inner: server_io,
        block: block.clone(),
        fail: fail.clone(),
    };
    let (queued_tx, queued_rx) = tokio::sync::oneshot::channel::<()>();
    let (ended_tx, ended_rx) = tokio::sync::oneshot::channel::<()>();

    let server = tokio::spawn(async move {
        let mut builder = h2::server::Builder::new();
        if let Some(duration) = reset_duration {
            builder.reset_stream_duration(duration);
        }
        let mut conn = builder.handshake::<_, Bytes>(gate).await.unwrap();
        let (request, mut respond) = conn.accept().await.unwrap().unwrap();
        let body = request.into_body();
        let mut send = respond
            .send_response(Response::builder().status(200).body(()).unwrap(), false)
            .unwrap();
        // The connection future is what writes frames: poll it between the sends.
        let _ = tokio::time::timeout(Duration::from_millis(100), conn.accept()).await;
        send.send_data(Bytes::from(vec![7u8; 256 * 1024]), false)
            .unwrap();
        let _ = tokio::time::timeout(Duration::from_millis(100), conn.accept()).await;
        if writer_parked {
            block.store(true, Ordering::SeqCst);
        }
        send.send_data(Bytes::new(), true).unwrap();
        drop(send);
        drop(respond);
        drop(body);
        queued_tx.send(()).unwrap();
        // Drives the connection until it ends; the handles are gone, so only the end of the
        // connection itself is left to observe.
        let _ = tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(accepted) = conn.accept().await {
                if accepted.is_err() {
                    break;
                }
            }
        })
        .await;
        drop(conn);
        let _ = ended_tx.send(());
    });

    let fail_for_client = fail.clone();
    let client = tokio::spawn(async move {
        let (mut send_request, connection) = h2::client::Builder::new()
            .initial_window_size(window)
            .handshake::<_, Bytes>(client_io)
            .await
            .unwrap();
        let driver = tokio::spawn(connection);
        let request = Request::builder()
            .method(Method::POST)
            .uri("https://example.com/")
            .body(())
            .unwrap();
        let (response, _request_body) = send_request.send_request(request, false).unwrap();
        let _response = response.await.unwrap();
        queued_rx.await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        match end {
            End::CleanEof => {
                driver.abort();
                let _ = driver.await;
                drop(send_request);
            }
            End::Io(kind) => {
                fail_io(&fail_for_client, kind);
                let _ = ended_rx.await;
                driver.abort();
                let _ = driver.await;
            }
        }
    });
    client.await.unwrap();
    server.await.is_err()
}

/// The client's initial window values under test: a zero window leaves the response DATA with no
/// send capacity at all, a nonzero window lets the first part of it reach the pending-send queue.
/// Each connection end must release the stream in both.
const WINDOWS: [u32; 2] = [0, 1024];

/// The server's reset-expiration windows under test: `h2`'s default leaves the stream inside its
/// window when the connection ends, `Duration::ZERO` leaves it expired. Each connection end must
/// release the stream in both.
const RESET_DURATIONS: [Option<Duration>; 2] = [None, Some(Duration::ZERO)];

/// Runs every window, writer state and reset-expiration window for `end` and returns the
/// combinations that panicked.
async fn panicking_combinations(end: End) -> Vec<String> {
    let mut panicked = Vec::new();
    for window in WINDOWS {
        for parked in [true, false] {
            for reset_duration in RESET_DURATIONS {
                if connection_end_panicked(end, parked, window, reset_duration).await {
                    panicked.push(format!(
                        "window={window} writer_parked={parked} reset_duration={reset_duration:?}"
                    ));
                }
            }
        }
    }
    panicked
}

#[tokio::test]
async fn scheduled_reset_stream_is_released_when_the_connection_ends_by_clean_eof() {
    let panicked = panicking_combinations(End::CleanEof).await;
    assert!(
        panicked.is_empty(),
        "clean EOF left a counted stream behind: {panicked:?}"
    );
}

#[tokio::test]
async fn scheduled_reset_stream_is_released_when_the_connection_ends_by_io_error() {
    let panicked = panicking_combinations(End::Io(io::ErrorKind::ConnectionReset)).await;
    assert!(
        panicked.is_empty(),
        "an io error left a counted stream behind: {panicked:?}"
    );
}

#[tokio::test]
async fn scheduled_reset_stream_is_released_when_the_connection_ends_by_connection_aborted() {
    let panicked = panicking_combinations(End::Io(io::ErrorKind::ConnectionAborted)).await;
    assert!(
        panicked.is_empty(),
        "a connection abort left a counted stream behind: {panicked:?}"
    );
}
