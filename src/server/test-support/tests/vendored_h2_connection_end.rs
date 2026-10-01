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
//! Two arms of the vendored `state.rs` are under test. The clean-EOF test needs the
//! `State::recv_eof` arm. The two transport-failure tests over `Gate` are released by either arm
//! (the sweep of `State::recv_eof` that runs when the connection drops finds the stream still in
//! the store); the hyper test is the one that needs the `State::handle_error` arm, because hyper's
//! stream task has finished by the time the transport fails and the failure sweep removes the
//! unreferenced stream from the store before any later sweep can find it.
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
/// The client opens a POST with the initial send `window`, so the server's response DATA and its
/// END_STREAM stay queued; the server drops every handle while the request is still open
/// (`NO_ERROR` scheduled reset), then the connection ends as `end` says.
async fn connection_end_panicked(end: End, writer_parked: bool, window: u32) -> bool {
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
        let mut conn = h2::server::handshake(gate).await.unwrap();
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

/// Runs every window and writer state for `end` and returns the combinations that panicked.
async fn panicking_combinations(end: End) -> Vec<String> {
    let mut panicked = Vec::new();
    for window in WINDOWS {
        for parked in [true, false] {
            if connection_end_panicked(end, parked, window).await {
                panicked.push(format!("window={window} writer_parked={parked}"));
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

/// Quiet interval between the response being fully handed to `h2` and the transport failing.
///
/// Measured against the unpatched `State::handle_error`: a failure injected after 0.6 s or less
/// leaves the stream reachable by the later `State::recv_eof` sweep, from 1.0 s on the stream is
/// already gone from the store when that sweep runs, so only the `handle_error` arm releases it.
/// 1.5 s keeps the margin above that boundary.
const HYPER_WAIT_MS: u64 = 1500;

/// hyper's h2 server answers a request whose body stays open (POST) with a response larger than
/// the client's stream window, so the response DATA and its END_STREAM sit queued behind the
/// window and the stream becomes a scheduled `NO_ERROR` reset once hyper drops its handles; the
/// transport then fails (`ConnectionAborted`, the kind the connection kill produces). Returns
/// whether the server task panicked.
async fn hyper_connection_end_panicked(post: bool) -> bool {
    let (server_io, client_io) = tokio::io::duplex(1 << 20);
    let block = Arc::new(AtomicBool::new(false));
    let fail = Arc::new(Mutex::new(Fail::default()));
    let gate = Gate {
        inner: server_io,
        block: block.clone(),
        fail: fail.clone(),
    };
    let service = hyper::service::service_fn(|_request: Request<hyper::body::Incoming>| async {
        Ok::<_, std::convert::Infallible>(Response::new(http_body_util::Full::new(Bytes::from(
            vec![2u8; 8 * 1024],
        ))))
    });
    let (ended_tx, ended_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let connection =
            hyper::server::conn::http2::Builder::new(hyper_util::rt::TokioExecutor::new())
                .serve_connection(hyper_util::rt::TokioIo::new(gate), service);
        let _ = connection.await;
        let _ = ended_tx.send(());
    });
    let client = tokio::spawn(async move {
        let (mut send_request, connection) = h2::client::Builder::new()
            .initial_window_size(1024)
            .handshake::<_, Bytes>(client_io)
            .await
            .unwrap();
        let driver = tokio::spawn(connection);
        let request = Request::builder()
            .method(if post { Method::POST } else { Method::GET })
            .uri("https://example.com/")
            .body(())
            .unwrap();
        let (response, _request_body) = send_request.send_request(request, !post).unwrap();
        let _response = response.await.unwrap();
        tokio::time::sleep(Duration::from_millis(HYPER_WAIT_MS)).await;
        fail_io(&fail, io::ErrorKind::ConnectionAborted);
        let _ = ended_rx.await;
        driver.abort();
        let _ = driver.await;
    });
    client.await.unwrap();
    server.await.is_err()
}

#[tokio::test]
async fn hyper_zombie_stream_is_released_when_the_transport_fails() {
    for post in [true, false] {
        assert!(
            !hyper_connection_end_panicked(post).await,
            "post={post}: a counted stream survived the failed transport"
        );
    }
}
