//! The serve loop over real loopback sockets: the time bounds, connect-info injection, graceful
//! shutdown and source exhaustion.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::extract::ConnectInfo;
use axum::routing::get;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

use super::egress::Egress;
use super::kill::KillReason;
use super::live::{stalled_body, stalled_final_body, stalled_final_body_of};
use super::stream_exec::{scoped, StreamExec};
use super::*;

/// Short bounds so a stall is observable in well under a second.
fn tight() -> ConnLimits {
    ConnLimits {
        header_read: Duration::from_millis(300),
        ..ConnLimits::default()
    }
}

fn app() -> Router {
    Router::new().route("/ok", get(|| async { "ok" })).route(
        "/peer",
        get(|ConnectInfo(meta): ConnectInfo<ConnMeta>| async move { meta.peer.ip().to_string() }),
    )
    // Detaches work holding the connection's lease past the response, as a WebSocket session does.
    .route(
        "/detach",
        get(|Extension(lease): Extension<ConnLease>| async move {
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(800)).await;
                drop(lease);
            });
            "ok"
        }),
    )
}

/// Binds a loopback listener and serves `app()` on it with `limits` until the returned sender fires.
async fn start(
    limits: ConnLimits,
) -> (
    std::net::SocketAddr,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<ServeOutcome>,
) {
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = tcp.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(serve_router(
        tcp,
        app(),
        async move {
            let _ = rx.await;
        },
        limits,
        || {},
    ));
    (addr, tx, task)
}

/// True when the peer closes (EOF or reset) within `within`.
async fn closes_within(s: &mut TcpStream, within: Duration) -> bool {
    let mut buf = [0u8; 512];
    loop {
        match timeout(within, s.read(&mut buf)).await {
            Ok(Ok(0)) | Ok(Err(_)) => return true,
            Ok(Ok(_)) => continue,
            Err(_) => return false,
        }
    }
}

#[tokio::test]
async fn a_request_is_served_with_the_peer_as_connect_info() {
    let (addr, _tx, _task) = start(tight()).await;
    let mut c = TcpStream::connect(addr).await.unwrap();
    c.write_all(b"GET /peer HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut body = String::new();
    timeout(Duration::from_secs(5), c.read_to_string(&mut body))
        .await
        .unwrap()
        .unwrap();
    assert!(body.starts_with("HTTP/1.1 200"), "{body}");
    assert!(body.ends_with("127.0.0.1"), "{body}");
}

#[tokio::test]
async fn a_peer_trickling_a_request_head_is_closed_at_the_header_read_timeout() {
    let (addr, _tx, _task) = start(tight()).await;
    let mut c = TcpStream::connect(addr).await.unwrap();
    // A partial request line, then silence: without a header-read timer this connection would
    // never close.
    c.write_all(b"GET /ok HTTP/1.1\r\nHost: x\r\n")
        .await
        .unwrap();
    assert!(
        closes_within(&mut c, Duration::from_secs(5)).await,
        "the server kept a half-sent request head open"
    );
}

#[tokio::test]
async fn an_idle_keep_alive_connection_is_closed_at_the_header_read_timeout() {
    let (addr, _tx, _task) = start(tight()).await;
    let mut c = TcpStream::connect(addr).await.unwrap();
    c.write_all(b"GET /ok HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    let mut buf = [0u8; 512];
    let n = timeout(Duration::from_secs(5), c.read(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert!(std::str::from_utf8(&buf[..n])
        .unwrap()
        .starts_with("HTTP/1.1 200"));
    assert!(
        closes_within(&mut c, Duration::from_secs(5)).await,
        "the server kept an idle keep-alive connection open"
    );
}

#[tokio::test]
async fn graceful_shutdown_returns_with_an_idle_connection_open() {
    let (addr, tx, task) = start(ConnLimits::default()).await;
    let mut idle = TcpStream::connect(addr).await.unwrap();
    idle.write_all(b"GET /ok HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    let mut buf = [0u8; 512];
    let _ = timeout(Duration::from_secs(5), idle.read(&mut buf))
        .await
        .unwrap()
        .unwrap();
    tx.send(()).unwrap();
    timeout(Duration::from_secs(10), task)
        .await
        .expect("serve_router did not return after shutdown")
        .unwrap();
    assert!(
        TcpStream::connect(addr).await.is_err(),
        "the port must be released after shutdown"
    );
}

/// A source that is already finished.
struct Exhausted;

impl ConnSource for Exhausted {
    type Io = TcpStream;

    async fn accept(&mut self) -> io::Result<Option<(TcpStream, ConnMeta)>> {
        Ok(None)
    }
}

#[tokio::test]
async fn an_exhausted_source_ends_the_serve_loop() {
    let end = timeout(
        Duration::from_secs(5),
        serve_router(
            Exhausted,
            app(),
            std::future::pending(),
            ConnLimits::default(),
            || {},
        ),
    )
    .await
    .expect("serve_router must return when its source ends");
    assert_eq!(end.end, ServeEnd::SourceEnded);
}

/// A source that yields the connections of a real listener and then ends.
struct OneThenEnd {
    listener: Option<TcpListener>,
}

impl ConnSource for OneThenEnd {
    type Io = TcpStream;

    async fn accept(&mut self) -> io::Result<Option<(TcpStream, ConnMeta)>> {
        let Some(listener) = self.listener.as_ref() else {
            return Ok(None);
        };
        let (sock, peer) = listener.accept().await?;
        self.listener = None;
        Ok(Some((sock, ConnMeta::plain(peer))))
    }
}

#[tokio::test]
async fn a_source_that_ends_drains_live_connections_like_a_shutdown() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(serve_router(
        OneThenEnd {
            listener: Some(listener),
        },
        app(),
        std::future::pending(),
        ConnLimits::default(),
        || {},
    ));
    let mut idle = TcpStream::connect(addr).await.unwrap();
    idle.write_all(b"GET /ok HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    let mut buf = [0u8; 512];
    let _ = timeout(Duration::from_secs(5), idle.read(&mut buf))
        .await
        .unwrap()
        .unwrap();
    // The source ended with this keep-alive connection still open: the loop must drain it (the
    // default 30 s header-read bound would otherwise hold the return far past this deadline).
    let end = timeout(Duration::from_secs(5), task)
        .await
        .expect("serve_router did not drain a live connection after its source ended")
        .unwrap();
    assert_eq!(end.end, ServeEnd::SourceEnded);
}

/// A TCP stream that claims to have negotiated `h2`, standing in for a TLS stream with ALPN `h2`.
struct AlpnH2(TcpStream);

impl AsyncRead for AlpnH2 {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for AlpnH2 {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

impl ServeIo for AlpnH2 {
    fn is_h2(&self) -> bool {
        true
    }
}

/// A source whose connections all report ALPN `h2`.
struct H2Source(TcpListener);

impl ConnSource for H2Source {
    type Io = AlpnH2;

    async fn accept(&mut self) -> io::Result<Option<(AlpnH2, ConnMeta)>> {
        let (sock, peer) = self.0.accept().await?;
        Ok(Some((AlpnH2(sock), ConnMeta::plain(peer))))
    }
}

#[tokio::test]
async fn a_plain_peer_that_sends_one_byte_then_stalls_is_closed_at_the_header_read_timeout() {
    let (addr, _tx, _task) = start(tight()).await;
    let mut c = TcpStream::connect(addr).await.unwrap();
    // The first byte of the HTTP/2 preface, then silence: a version-sniffing builder would wait
    // for 24 bytes with no timer.
    c.write_all(b"P").await.unwrap();
    assert!(
        closes_within(&mut c, Duration::from_secs(5)).await,
        "the server kept a one-byte connection open"
    );
}

#[tokio::test]
async fn an_h2_peer_that_sends_nothing_is_closed_at_the_header_read_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _task = tokio::spawn(serve_router(
        H2Source(listener),
        app(),
        std::future::pending(),
        tight(),
        || {},
    ));
    let mut c = TcpStream::connect(addr).await.unwrap();
    assert!(
        closes_within(&mut c, Duration::from_secs(5)).await,
        "the server kept a silent h2 connection open"
    );
}

#[tokio::test]
async fn an_h2_peer_that_trickles_a_partial_preface_is_closed_at_the_header_read_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _task = tokio::spawn(serve_router(
        H2Source(listener),
        app(),
        std::future::pending(),
        tight(),
        || {},
    ));
    let mut c = TcpStream::connect(addr).await.unwrap();
    c.write_all(b"PRI * HTTP/2.0").await.unwrap();
    assert!(
        closes_within(&mut c, Duration::from_secs(5)).await,
        "the server kept a partial h2 preface open"
    );
}

/// The route used to exercise a long-running handler and an upgraded connection.
fn upgrade_app() -> Router {
    Router::new()
        .route(
            "/slow",
            get(|| async {
                tokio::time::sleep(Duration::from_millis(900)).await;
                "done"
            }),
        )
        .route(
            "/up",
            get(|mut req: Request| async move {
                let on_upgrade = hyper::upgrade::on(&mut req);
                tokio::spawn(async move {
                    if let Ok(upgraded) = on_upgrade.await {
                        let mut io = hyper_util::rt::TokioIo::new(upgraded);
                        // Idle for several header-read periods, then speak.
                        tokio::time::sleep(Duration::from_millis(1200)).await;
                        let _ = io.write_all(b"still-open").await;
                    }
                });
                axum::response::Response::builder()
                    .status(101)
                    .header("connection", "upgrade")
                    .header("upgrade", "probe")
                    .body(Body::empty())
                    .unwrap()
            }),
        )
}

async fn start_upgrade_app(limits: ConnLimits) -> std::net::SocketAddr {
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = tcp.local_addr().unwrap();
    tokio::spawn(serve_router(
        tcp,
        upgrade_app(),
        std::future::pending(),
        limits,
        || {},
    ));
    addr
}

#[tokio::test]
async fn an_idle_upgraded_connection_survives_past_the_header_read_timeout() {
    let addr = start_upgrade_app(tight()).await;
    let mut c = TcpStream::connect(addr).await.unwrap();
    c.write_all(b"GET /up HTTP/1.1\r\nHost: x\r\nConnection: Upgrade\r\nUpgrade: probe\r\n\r\n")
        .await
        .unwrap();
    let mut seen = Vec::new();
    let mut buf = [0u8; 512];
    // The 101 head, then (after 4 header-read periods of silence) the upgraded payload.
    timeout(Duration::from_secs(5), async {
        while !String::from_utf8_lossy(&seen).contains("still-open") {
            let n = c.read(&mut buf).await.unwrap();
            assert!(
                n > 0,
                "the upgraded connection was closed: {}",
                String::from_utf8_lossy(&seen)
            );
            seen.extend_from_slice(&buf[..n]);
        }
    })
    .await
    .expect("the upgraded connection never delivered its payload");
    assert!(String::from_utf8_lossy(&seen).starts_with("HTTP/1.1 101"));
}

#[tokio::test]
async fn a_handler_running_longer_than_the_header_read_timeout_completes() {
    let addr = start_upgrade_app(tight()).await;
    let mut c = TcpStream::connect(addr).await.unwrap();
    c.write_all(b"GET /slow HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut body = String::new();
    timeout(Duration::from_secs(5), c.read_to_string(&mut body))
        .await
        .unwrap()
        .unwrap();
    assert!(
        body.starts_with("HTTP/1.1 200") && body.ends_with("done"),
        "{body}"
    );
}

// ---------------------------------------------------------------------------------------------
// Replay
// ---------------------------------------------------------------------------------------------

/// An IO that serves `tail` when read, records what is written, and reports vectored writes.
struct Probe {
    /// Octets returned by reads.
    tail: Vec<u8>,
    /// How many `tail` octets have been read.
    read: usize,
    /// Every octet written, in order.
    written: Vec<u8>,
    /// How many times `poll_write_vectored` was called.
    vectored_calls: usize,
}

impl Probe {
    fn new(tail: &[u8]) -> Self {
        Self {
            tail: tail.to_vec(),
            read: 0,
            written: Vec::new(),
            vectored_calls: 0,
        }
    }
}

impl AsyncRead for Probe {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let n = (self.tail.len() - self.read).min(buf.remaining());
        let from = self.read;
        buf.put_slice(&self.tail[from..from + n]);
        self.read += n;
        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for Probe {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.written.extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        self.vectored_calls += 1;
        let mut n = 0;
        for b in bufs {
            self.written.extend_from_slice(b);
            n += b.len();
        }
        Poll::Ready(Ok(n))
    }

    fn is_write_vectored(&self) -> bool {
        true
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// A write counter feeding an egress record nobody else reads, for IO that no sampler watches.
fn detached_writes() -> WriteCounter {
    WriteCounter::new(Arc::new(Egress::default()), false)
}

fn replay(prefix: &[u8], tail: &[u8]) -> Replay<Probe> {
    Replay::new(
        prefix.to_vec(),
        Probe::new(tail),
        ConnKill::new(),
        detached_writes(),
    )
}

#[tokio::test]
async fn a_prefix_larger_than_the_read_buffer_is_drained_over_several_reads_then_reads_delegate() {
    let mut io = replay(b"0123456789abcdefghijklmn", b"tail");
    let mut buf = [0u8; 10];
    let mut seen = Vec::new();
    for expected in [10, 10, 4] {
        let n = io.read(&mut buf).await.unwrap();
        assert_eq!(
            n, expected,
            "each read serves at most the buffer's capacity"
        );
        seen.extend_from_slice(&buf[..n]);
    }
    assert_eq!(seen, b"0123456789abcdefghijklmn");
    let n = io.read(&mut buf).await.unwrap();
    assert_eq!(
        &buf[..n],
        b"tail",
        "once the prefix is served reads reach the inner io"
    );
}

#[tokio::test]
async fn a_write_before_the_prefix_is_consumed_passes_through_and_leaves_the_prefix() {
    let mut io = replay(b"PRI", b"");
    io.write_all(b"hi").await.unwrap();
    assert_eq!(io.inner.written, b"hi");
    let mut buf = [0u8; 8];
    let n = io.read(&mut buf).await.unwrap();
    assert_eq!(&buf[..n], b"PRI", "the write did not consume the prefix");
}

#[tokio::test]
async fn a_zero_capacity_read_neither_consumes_the_prefix_nor_ends_the_stream() {
    let mut io = replay(b"PRI", b"tail");
    let n = io.read(&mut []).await.unwrap();
    assert_eq!(n, 0);
    assert_eq!(io.pos, 0, "a zero-capacity read consumed prefix octets");
    let mut buf = [0u8; 8];
    let n = io.read(&mut buf).await.unwrap();
    assert_eq!(
        &buf[..n],
        b"PRI",
        "the stream ended after a zero-capacity read"
    );
}

#[tokio::test]
async fn vectored_writes_reach_the_inner_io_as_one_vectored_write() {
    let mut io = replay(b"", b"");
    assert!(
        io.is_write_vectored(),
        "Replay must report its inner io's vectored support"
    );
    let n = io
        .write_vectored(&[io::IoSlice::new(b"ab"), io::IoSlice::new(b"cd")])
        .await
        .unwrap();
    assert_eq!(
        n, 4,
        "a default write_vectored would write only the first slice"
    );
    assert_eq!(io.inner.vectored_calls, 1);
    assert_eq!(io.inner.written, b"abcd");
}

// ---------------------------------------------------------------------------------------------
// The kill switch: every IO method fails once the drain deadline has been signalled
// ---------------------------------------------------------------------------------------------

/// An IO that is never ready and never registers a waker, like a peer that has stopped reading
/// and writing: only the kill switch can wake a task parked on it.
struct Stuck;

impl AsyncRead for Stuck {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Poll::Pending
    }
}

impl AsyncWrite for Stuck {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Pending
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Poll::Pending
    }

    fn is_write_vectored(&self) -> bool {
        true
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Pending
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Pending
    }
}

/// The five IO operations a kill must fail.
#[derive(Debug, Clone, Copy)]
enum Op {
    /// `poll_read`.
    Read,
    /// `poll_write`.
    Write,
    /// `poll_write_vectored`.
    WriteVectored,
    /// `poll_flush`.
    Flush,
    /// `poll_shutdown`.
    Shutdown,
}

const OPS: [Op; 5] = [
    Op::Read,
    Op::Write,
    Op::WriteVectored,
    Op::Flush,
    Op::Shutdown,
];

/// Runs `op` to completion on `io`.
async fn run_op<I: AsyncRead + AsyncWrite + Unpin>(io: &mut I, op: Op) -> io::Result<()> {
    match op {
        Op::Read => io.read(&mut [0u8; 4]).await.map(|_| ()),
        Op::Write => io.write(b"x").await.map(|_| ()),
        Op::WriteVectored => io
            .write_vectored(&[io::IoSlice::new(b"x")])
            .await
            .map(|_| ()),
        Op::Flush => io.flush().await,
        Op::Shutdown => io.shutdown().await,
    }
}

#[tokio::test]
async fn every_io_method_fails_once_the_kill_signal_has_fired() {
    for op in OPS {
        let signal = KillSignal::new();
        let mut io = Replay::new(
            b"PRI".to_vec(),
            Probe::new(b"tail"),
            signal.connection(),
            detached_writes(),
        );
        run_op(&mut io, op)
            .await
            .unwrap_or_else(|e| panic!("{op:?} failed before the signal fired: {e}"));
        signal.fire();
        let err = run_op(&mut io, op)
            .await
            .expect_err(&format!("{op:?} succeeded after the kill signal fired"));
        assert_eq!(err.kind(), io::ErrorKind::ConnectionAborted, "{op:?}");
    }
}

#[tokio::test]
async fn a_task_parked_in_any_io_method_is_woken_and_failed_by_the_kill_signal() {
    for op in OPS {
        let signal = KillSignal::new();
        let mut io = Replay::new(Vec::new(), Stuck, signal.connection(), detached_writes());
        let parked = tokio::spawn(async move { run_op(&mut io, op).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!parked.is_finished(), "{op:?} did not park on a stuck io");
        signal.fire();
        let err = timeout(Duration::from_secs(5), parked)
            .await
            .unwrap_or_else(|_| panic!("{op:?} stayed parked after the kill signal fired"))
            .unwrap()
            .expect_err("a killed io method must fail");
        assert_eq!(err.kind(), io::ErrorKind::ConnectionAborted, "{op:?}");
    }
}

#[tokio::test]
async fn dropping_the_kill_signal_does_not_fail_the_io() {
    let signal = KillSignal::new();
    let mut io = Replay::new(
        Vec::new(),
        Probe::new(b"tail"),
        signal.connection(),
        detached_writes(),
    );
    drop(signal);
    let mut buf = [0u8; 4];
    assert_eq!(io.read(&mut buf).await.unwrap(), 4);
}

// ---------------------------------------------------------------------------------------------
// The HTTP/2 preface races the drain signal; keep-alive pings close a peer that never answers
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn an_h2_peer_that_sends_nothing_does_not_delay_a_shutdown() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(serve_router(
        H2Source(listener),
        app(),
        async move {
            let _ = rx.await;
        },
        // The default 30 s header-read bound is what a preface read that ignored the drain
        // signal would wait out.
        ConnLimits::default(),
        || {},
    ));
    let _silent = TcpStream::connect(addr).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    tx.send(()).unwrap();
    timeout(Duration::from_secs(3), task)
        .await
        .expect("a silent h2 peer held the drain until its preface deadline")
        .unwrap();
}

/// The HTTP/2 client connection preface followed by an empty SETTINGS frame.
pub(super) const H2_OPEN: &[u8] =
    b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n\x00\x00\x00\x04\x00\x00\x00\x00\x00";

/// An acknowledgement of a SETTINGS frame.
const SETTINGS_ACK: &[u8] = b"\x00\x00\x00\x04\x01\x00\x00\x00\x00";

/// Reads frames from `peer` for `for_how_long`, answering PING frames with an acknowledgement
/// when `ack_pings`. Returns whether the server closed the connection within that time.
async fn server_closes_while_pumping(
    peer: &mut TcpStream,
    ack_pings: bool,
    for_how_long: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + for_how_long;
    loop {
        let mut head = [0u8; 9];
        match tokio::time::timeout_at(deadline, peer.read_exact(&mut head)).await {
            Err(_) => return false,
            Ok(Err(_)) => return true,
            Ok(Ok(_)) => {}
        }
        let len = usize::from(head[0]) << 16 | usize::from(head[1]) << 8 | usize::from(head[2]);
        let mut payload = vec![0u8; len];
        match tokio::time::timeout_at(deadline, peer.read_exact(&mut payload)).await {
            Err(_) => return false,
            Ok(Err(_)) => return true,
            Ok(Ok(_)) => {}
        }
        let (kind, flags) = (head[3], head[4]);
        if kind == 0x4 && flags & 0x1 == 0 {
            let _ = peer.write_all(SETTINGS_ACK).await;
        }
        if kind == 0x6 && flags & 0x1 == 0 && ack_pings {
            let mut ack = vec![0, 0, 8, 6, 1, 0, 0, 0, 0];
            ack.extend_from_slice(&payload);
            let _ = peer.write_all(&ack).await;
        }
    }
}

fn keep_alive_limits() -> ConnLimits {
    ConnLimits {
        h2_ping_interval: Duration::from_millis(100),
        h2_ping_timeout: Duration::from_millis(200),
        ..ConnLimits::default()
    }
}

#[tokio::test]
async fn an_h2_peer_that_never_answers_pings_is_closed_by_keep_alive() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _task = tokio::spawn(serve_router(
        H2Source(listener),
        app(),
        std::future::pending(),
        keep_alive_limits(),
        || {},
    ));
    let mut peer = TcpStream::connect(addr).await.unwrap();
    peer.write_all(H2_OPEN).await.unwrap();
    assert!(
        server_closes_while_pumping(&mut peer, false, Duration::from_secs(5)).await,
        "the server kept a connection open whose peer never acknowledged a ping"
    );
}

#[tokio::test]
async fn an_h2_peer_that_answers_pings_is_kept_open() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _task = tokio::spawn(serve_router(
        H2Source(listener),
        app(),
        std::future::pending(),
        keep_alive_limits(),
        || {},
    ));
    let mut peer = TcpStream::connect(addr).await.unwrap();
    peer.write_all(H2_OPEN).await.unwrap();
    assert!(
        !server_closes_while_pumping(&mut peer, true, Duration::from_millis(1500)).await,
        "the control failed: a peer acknowledging pings was closed, so the test above proves nothing"
    );
}

// ---------------------------------------------------------------------------------------------
// Live-connection caps
// ---------------------------------------------------------------------------------------------

/// Opens a connection, completes one keep-alive request and returns it still open.
async fn open_served(addr: std::net::SocketAddr) -> TcpStream {
    let mut c = TcpStream::connect(addr).await.unwrap();
    c.write_all(b"GET /ok HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    let mut buf = [0u8; 512];
    let n = timeout(Duration::from_secs(5), c.read(&mut buf))
        .await
        .expect("the connection was not served")
        .unwrap();
    assert!(
        n > 0 && buf.starts_with(b"HTTP/1.1 200"),
        "not served: {n} bytes"
    );
    c
}

/// True when a new connection is closed without being served.
async fn is_refused(addr: std::net::SocketAddr) -> bool {
    let mut c = TcpStream::connect(addr).await.unwrap();
    let _ = c.write_all(b"GET /ok HTTP/1.1\r\nHost: x\r\n\r\n").await;
    let mut buf = [0u8; 512];
    match timeout(Duration::from_secs(3), c.read(&mut buf)).await {
        Ok(Ok(0)) | Ok(Err(_)) => true,
        Ok(Ok(_)) | Err(_) => false,
    }
}

fn caps(max_live: usize, max_live_per_peer: u32) -> ConnLimits {
    ConnLimits {
        max_live,
        max_live_per_peer,
        ..ConnLimits::default()
    }
}

#[tokio::test]
async fn a_connection_over_the_per_peer_cap_is_refused_and_a_closed_one_frees_its_slot() {
    let (addr, _tx, _task) = start(caps(100, 2)).await;
    let first = open_served(addr).await;
    let _second = open_served(addr).await;
    assert!(
        is_refused(addr).await,
        "the third connection from one peer was served"
    );
    drop(first);
    timeout(Duration::from_secs(5), async {
        loop {
            if !is_refused(addr).await {
                return;
            }
        }
    })
    .await
    .expect("a closed connection never released its slot");
}

#[tokio::test]
async fn a_connection_over_the_global_cap_is_refused() {
    let (addr, _tx, _task) = start(caps(2, 100)).await;
    let _a = open_served(addr).await;
    let _b = open_served(addr).await;
    assert!(
        is_refused(addr).await,
        "a connection beyond the global cap was served"
    );
}

/// A source that marks every connection of a real listener cap-exempt, as the sniffing listener
/// does for a trusted proxy.
struct ExemptSource(TcpListener);

impl ConnSource for ExemptSource {
    type Io = TcpStream;

    async fn accept(&mut self) -> io::Result<Option<(TcpStream, ConnMeta)>> {
        let (sock, peer) = self.0.accept().await?;
        let mut meta = ConnMeta::plain(peer);
        meta.cap_exempt = true;
        Ok(Some((sock, meta)))
    }
}

#[tokio::test]
async fn an_exempt_peer_skips_the_per_peer_cap_but_not_the_global_cap() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _task = tokio::spawn(serve_router(
        ExemptSource(listener),
        app(),
        std::future::pending(),
        caps(3, 1),
        || {},
    ));
    let _a = open_served(addr).await;
    let _b = open_served(addr).await;
    let _c = open_served(addr).await;
    assert!(
        is_refused(addr).await,
        "the global cap must still bind an exempt peer"
    );
}

#[tokio::test]
async fn a_held_lease_keeps_the_slot_after_the_connection_has_ended() {
    let (addr, _tx, _task) = start(caps(100, 1)).await;
    let mut c = TcpStream::connect(addr).await.unwrap();
    c.write_all(b"GET /detach HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut body = String::new();
    timeout(Duration::from_secs(5), c.read_to_string(&mut body))
        .await
        .unwrap()
        .unwrap();
    assert!(body.ends_with("ok"), "{body}");
    // The connection is over, but the detached work still holds its lease and therefore its slot.
    assert!(
        is_refused(addr).await,
        "the slot was released while a lease was still held"
    );
    tokio::time::sleep(Duration::from_millis(900)).await;
    let _served = open_served(addr).await;
}

// ---------------------------------------------------------------------------------------------
// `drain_tasks`: the deadline, the kill, the abort wait and what each is counted as
// ---------------------------------------------------------------------------------------------

/// The drain bound the `drain_tasks` controls use; the paused clock makes its length irrelevant.
const TEST_DEADLINE: Duration = Duration::from_secs(5);

/// Runs `drain_tasks` over `tasks` with the production `abort_wait` and returns the report.
async fn run_drain(tasks: &mut JoinSet<()>, kill: impl FnOnce()) -> DrainReport {
    let mut report = DrainReport::default();
    drain_tasks(
        tasks,
        Instant::now() + TEST_DEADLINE,
        kill,
        ABORT_WAIT,
        &mut report,
    )
    .await;
    report
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_task_parked_past_the_kill_and_the_abort_wait_is_counted_aborted() {
    let mut tasks = JoinSet::new();
    tasks.spawn(std::future::pending::<()>());
    tasks.spawn(async {});
    let report = run_drain(&mut tasks, || {}).await;
    assert_eq!(report.killed, 1, "{report:?}");
    assert_eq!(report.aborted, 1, "{report:?}");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_task_that_ends_when_the_kill_fires_is_killed_and_not_aborted() {
    let mut tasks = JoinSet::new();
    let (fire, fired) = tokio::sync::oneshot::channel::<()>();
    tasks.spawn(async move {
        let _ = fired.await;
    });
    let report = run_drain(&mut tasks, move || {
        let _ = fire.send(());
    })
    .await;
    assert_eq!(report.killed, 1, "{report:?}");
    assert_eq!(report.aborted, 0, "{report:?}");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_drain_with_nothing_open_at_the_deadline_reports_nothing() {
    let mut tasks = JoinSet::new();
    tasks.spawn(async {});
    let report = run_drain(&mut tasks, || {
        panic!("nothing was open, so nothing is killed")
    })
    .await;
    assert_eq!(report, DrainReport::default());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_task_ending_kill_grace_after_the_kill_is_not_aborted() {
    let mut tasks = JoinSet::new();
    let (fire, fired) = tokio::sync::oneshot::channel::<()>();
    // Stands in for `drive`: after the kill it takes its whole `KILL_GRACE` to end.
    tasks.spawn(async move {
        let _ = fired.await;
        tokio::time::sleep(KILL_GRACE).await;
    });
    let report = run_drain(&mut tasks, move || {
        let _ = fire.send(());
    })
    .await;
    assert_eq!(report.killed, 1, "{report:?}");
    assert_eq!(
        report.aborted, 0,
        "the abort raced a connection's own grace: {report:?}"
    );
}

// ---------------------------------------------------------------------------------------------
// `drive`: the one force-end path
// ---------------------------------------------------------------------------------------------

/// Sets its flag when dropped, so a test can tell a dropped `conn` from one that merely stopped
/// being polled.
struct DropFlag(Arc<std::sync::atomic::AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// A connection's watch state: `kill`, fresh liveness state over a fresh slot, and `window`.
fn watch_for(kill: Arc<ConnKill>, window: Duration) -> Watch {
    let slots = Arc::new(PeerSlots::new(4, 4));
    let slot = slots.admit("192.0.2.1".parse().unwrap(), false).unwrap();
    Watch {
        kill,
        live: LiveState::new(slot, Arc::new(StallCounters::default())),
        egress_window: window,
        egress_floor: ConnLimits::default().egress_floor,
    }
}

/// Runs `drive` over `conn` on its own task and returns the handle, with the `signal_tx` receiver
/// that keeps the graceful signal from closing.
fn start_drive(
    conn: impl Future<Output = Result<(), hyper::Error>> + Send + 'static,
    kill: Arc<ConnKill>,
    dropped: Arc<AtomicUsize>,
) -> (tokio::task::JoinHandle<()>, watch::Receiver<()>) {
    let (signal_tx, signal_rx) = watch::channel(());
    let watch = watch_for(kill, ConnLimits::default().egress_window);
    let task = tokio::spawn(async move {
        drive(
            conn,
            signal_tx,
            std::future::pending(),
            |_| {},
            watch,
            &dropped,
        )
        .await;
    });
    (task, signal_rx)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_connection_that_ends_when_its_io_fails_is_not_dropped() {
    for reason in [KillReason::Drain, KillReason::Stall] {
        let kill = ConnKill::new();
        let dropped = Arc::new(AtomicUsize::new(0));
        let mut io = Replay::new(Vec::new(), Stuck, kill.clone(), detached_writes());
        let conn = async move {
            let _ = io.read(&mut [0u8; 4]).await;
            Ok(())
        };
        let (task, _keep) = start_drive(conn, kill.clone(), dropped.clone());
        tokio::task::yield_now().await;
        let fired_at = Instant::now();
        kill.fire(reason);
        timeout(KILL_GRACE, task)
            .await
            .expect("drive did not return once the connection ended")
            .unwrap();
        assert_eq!(dropped.load(Ordering::SeqCst), 0, "{reason:?}");
        assert!(fired_at.elapsed() < KILL_GRACE, "{reason:?}");
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_connection_that_ignores_its_failed_io_is_dropped_exactly_kill_grace_after_the_fire() {
    for reason in [KillReason::Drain, KillReason::Stall] {
        let kill = ConnKill::new();
        let dropped = Arc::new(AtomicUsize::new(0));
        let was_dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = DropFlag(was_dropped.clone());
        let conn = async move {
            let _flag = flag;
            std::future::pending::<Result<(), hyper::Error>>().await
        };
        let (task, _keep) = start_drive(conn, kill.clone(), dropped.clone());
        tokio::task::yield_now().await;
        let fired_at = Instant::now();
        kill.fire(reason);
        timeout(KILL_GRACE * 2, task)
            .await
            .expect("drive never force-ended a connection that ignored its failed IO")
            .unwrap();
        assert_eq!(fired_at.elapsed(), KILL_GRACE, "{reason:?}");
        assert_eq!(dropped.load(Ordering::SeqCst), 1, "{reason:?}");
        assert!(
            was_dropped.load(Ordering::SeqCst),
            "{reason:?}: not dropped"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// The accept-error pause: decided by `serve_router`, so a reaped connection task cannot cut it
// short
// ---------------------------------------------------------------------------------------------

impl ServeIo for tokio::io::DuplexStream {
    fn is_h2(&self) -> bool {
        false
    }
}

/// One scripted `accept` outcome.
type Scripted = io::Result<Option<(tokio::io::DuplexStream, ConnMeta)>>;

/// A source that replays a script, one entry per `accept` call, records the instant of every call
/// that consumed an entry, and then parks (a parked call is dropped and re-made on every reap, so
/// it is not recorded). Runs on the paused clock, so recorded instants are exact.
struct ScriptedSource {
    /// Outcomes still to return, in call order.
    script: std::collections::VecDeque<Scripted>,
    /// The instant of each `accept` call that consumed a script entry.
    calls: Arc<std::sync::Mutex<Vec<Instant>>>,
}

impl ConnSource for ScriptedSource {
    type Io = tokio::io::DuplexStream;

    async fn accept(&mut self) -> Scripted {
        match self.script.pop_front() {
            Some(outcome) => {
                self.calls.lock().unwrap().push(Instant::now());
                outcome
            }
            None => std::future::pending().await,
        }
    }
}

/// A connection whose client end the test keeps: the server side lives until that end drops.
fn duplex_connection() -> (Scripted, tokio::io::DuplexStream) {
    let (server, client) = tokio::io::duplex(1024);
    let meta = ConnMeta::plain("192.0.2.1:4000".parse().unwrap());
    (Ok(Some((server, meta))), client)
}

/// What a scripted serve loop exposes to a test.
struct ScriptedLoop {
    /// The instant of every `accept` call that consumed a script entry.
    calls: Arc<std::sync::Mutex<Vec<Instant>>>,
    /// Ends the loop.
    shutdown: tokio::sync::oneshot::Sender<()>,
    /// The loop.
    task: tokio::task::JoinHandle<ServeOutcome>,
}

/// Serves the scripted source with a plain router.
fn serve_script(script: Vec<Scripted>) -> ScriptedLoop {
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (shutdown, rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(serve_router(
        ScriptedSource {
            script: script.into(),
            calls: calls.clone(),
        },
        app(),
        async move {
            let _ = rx.await;
        },
        ConnLimits::default(),
        || {},
    ));
    ScriptedLoop {
        calls,
        shutdown,
        task,
    }
}

/// Lets the paused clock run `for_how_long` with every ready task polled.
async fn settle(for_how_long: Duration) {
    tokio::time::sleep(for_how_long).await;
}

const TICK: Duration = Duration::from_millis(1);

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_pause_after_an_accept_error_survives_a_connection_task_being_reaped() {
    let (first, client) = duplex_connection();
    let (second, _client2) = duplex_connection();
    let loop_ = serve_script(vec![
        first,
        Err(io::Error::other("out of descriptors")),
        second,
    ]);
    settle(TICK).await;
    assert_eq!(
        loop_.calls.lock().unwrap().len(),
        2,
        "the error was returned"
    );
    // The first connection ends while the loop is paused; reaping it must not resume accepting.
    drop(client);
    settle(TICK).await;
    assert_eq!(
        loop_.calls.lock().unwrap().len(),
        2,
        "a reap cut the accept pause short"
    );
    settle(ACCEPT_ERROR_PAUSE).await;
    let calls = loop_.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 3, "accepting never resumed");
    assert_eq!(
        calls[2] - calls[1],
        ACCEPT_ERROR_PAUSE,
        "the third accept must start exactly one pause after the error"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn accepts_with_no_pending_pause_never_wait_and_never_evaluate_a_resume_instant() {
    let t0 = Instant::now();
    let mut script = Vec::new();
    let mut clients = Vec::new();
    for _ in 0..5 {
        let (connection, client) = duplex_connection();
        script.push(connection);
        clients.push(client);
    }
    let loop_ = serve_script(script);
    // Each connection is reaped between accepts.
    clients.clear();
    settle(TICK).await;
    let calls = loop_.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 5, "the five scripted connections");
    assert!(
        calls.iter().all(|&at| at == t0),
        "an accept waited with no pause pending: {calls:?}"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn connection_level_accept_errors_resume_at_once() {
    let (connection, _client) = duplex_connection();
    let t0 = Instant::now();
    let loop_ = serve_script(vec![
        Err(io::Error::from(io::ErrorKind::ConnectionReset)),
        Err(io::Error::from(io::ErrorKind::ConnectionAborted)),
        Err(io::Error::from(io::ErrorKind::ConnectionRefused)),
        connection,
    ]);
    settle(TICK).await;
    let calls = loop_.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 4, "three ignored errors and the connection");
    assert!(calls.iter().all(|&at| at == t0), "{calls:?}");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_shutdown_during_the_accept_pause_ends_the_loop_at_once() {
    let loop_ = serve_script(vec![Err(io::Error::other("out of descriptors"))]);
    settle(TICK).await;
    let paused_at = Instant::now();
    loop_.shutdown.send(()).unwrap();
    let outcome = loop_.task.await.unwrap();
    assert_eq!(outcome.end, ServeEnd::Shutdown);
    assert!(
        Instant::now() - paused_at < ACCEPT_ERROR_PAUSE,
        "the pause delayed the shutdown"
    );
}

#[test]
fn the_pause_decision_table_ignores_only_the_connection_level_kinds() {
    for kind in [
        io::ErrorKind::ConnectionRefused,
        io::ErrorKind::ConnectionAborted,
        io::ErrorKind::ConnectionReset,
    ] {
        assert_eq!(pause_for(kind), None, "{kind:?}");
    }
    for kind in [io::ErrorKind::Other, io::ErrorKind::OutOfMemory] {
        assert_eq!(pause_for(kind), Some(ACCEPT_ERROR_PAUSE), "{kind:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// The egress sampler tick and the stream unit it drives
// ---------------------------------------------------------------------------------------------

#[test]
fn the_sampler_period_is_the_window_over_the_ticks_per_window() {
    for window in [Duration::from_secs(1), Duration::from_secs(60)] {
        assert_eq!(egress_tick(window) * EGRESS_TICKS_PER_WINDOW, window);
    }
}

#[tokio::test]
#[should_panic(expected = "a nonzero sampler period")]
async fn serve_router_rejects_a_zero_egress_window() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    // A loop that accepted the zero window would serve forever; the timeout turns that into a
    // failure whose message differs from the expected panic.
    timeout(
        Duration::from_secs(5),
        serve_router(
            listener,
            Router::new(),
            std::future::pending::<()>(),
            ConnLimits {
                egress_window: Duration::ZERO,
                ..ConnLimits::default()
            },
            || {},
        ),
    )
    .await
    .expect("serve_router served with a zero egress window");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_sampler_cuts_a_stalled_stream_between_the_window_and_one_tick_after_it() {
    let window = Duration::from_secs(8);
    let tick = egress_tick(window);
    let kill = ConnKill::new();
    let watch = watch_for(kill, window);
    let live = watch.live.clone();
    let (signal_tx, _keep) = watch::channel(());
    let dropped = Arc::new(AtomicUsize::new(0));
    let task = tokio::spawn(async move {
        drive(
            std::future::pending::<Result<(), hyper::Error>>(),
            signal_tx,
            std::future::pending(),
            |_| {},
            watch,
            &dropped,
        )
        .await;
    });

    // A response stream delivers one chunk and then waits on the peer.
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (cut_tx, cut_rx) = tokio::sync::oneshot::channel();
    let l = live.clone();
    let (cut, run) = scoped(async move {
        let _body = stalled_body(&l).await;
        let _ = ready_tx.send(());
        std::future::pending::<()>().await;
    });
    let spawned_cut = cut.clone();
    tokio::spawn(async move {
        run.await;
        let _ = cut_tx.send(());
    });
    ready_rx.await.unwrap();
    let delivered = Instant::now();

    let cut_at = timeout(window + tick * 2, async {
        cut_rx.await.unwrap();
        Instant::now()
    })
    .await
    .expect("the sampler never cut the stalled stream");
    let age = cut_at - delivered;
    assert!(
        age >= window,
        "cut after {age:?}, before the window {window:?}"
    );
    assert!(
        age <= window + tick,
        "cut after {age:?}, later than the window plus one tick {:?}",
        window + tick
    );
    assert!(spawned_cut.has_fired());
    assert_eq!(live.stall_counts().stream, 1);
    task.abort();
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_stream_through_the_executor_is_dropped_by_the_samplers_cut() {
    let window = Duration::from_secs(8);
    let kill = ConnKill::new();
    let watch = watch_for(kill, window);
    let live = watch.live.clone();
    let (signal_tx, _keep) = watch::channel(());
    let dropped = Arc::new(AtomicUsize::new(0));
    let task = tokio::spawn(async move {
        drive(
            std::future::pending::<Result<(), hyper::Error>>(),
            signal_tx,
            std::future::pending(),
            |_| {},
            watch,
            &dropped,
        )
        .await;
    });
    let was_dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = DropFlag(was_dropped.clone());
    let l = live.clone();
    StreamExec.execute_test(async move {
        let _flag = flag;
        let _body = stalled_body(&l).await;
        std::future::pending::<()>().await;
    });
    settle(window + egress_tick(window) * 2).await;
    assert!(
        was_dropped.load(Ordering::SeqCst),
        "the sampler did not cut the stalled stream's task"
    );
    assert_eq!(live.stall_counts().stream, 1);
    task.abort();
}

// ---------------------------------------------------------------------------------------------
// The HTTP/2 send buffer
// ---------------------------------------------------------------------------------------------

/// A duplex stream that reports ALPN `h2`.
struct H2Duplex(tokio::io::DuplexStream);

impl AsyncRead for H2Duplex {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for H2Duplex {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

impl ServeIo for H2Duplex {
    fn is_h2(&self) -> bool {
        true
    }
}

/// A source that hands out one connection and then parks.
struct OneConnection(Option<(H2Duplex, ConnMeta)>);

impl ConnSource for OneConnection {
    type Io = H2Duplex;

    async fn accept(&mut self) -> io::Result<Option<(H2Duplex, ConnMeta)>> {
        match self.0.take() {
            Some(conn) => Ok(Some(conn)),
            None => std::future::pending().await,
        }
    }
}

/// One HTTP/2 frame header and payload.
pub(super) fn h2_frame(kind: u8, flags: u8, stream: u32, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u32;
    let mut out = len.to_be_bytes()[1..].to_vec();
    out.push(kind);
    out.push(flags);
    out.extend_from_slice(&stream.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

#[tokio::test]
async fn the_send_buffer_bounds_what_hyper_queues_behind_a_socket_that_accepts_nothing() {
    use std::sync::atomic::AtomicU64;

    // The socket is a duplex pipe of DUPLEX bytes that its peer never reads, so the server's
    // writer parks once the pipe is full. The peer's windows are far larger than anything the
    // route produces, so credit never limits: what limits hyper is h2's send buffer, which holds
    // the unsent backlog. Derived bounds on the bytes the route was polled for:
    //   at least the send buffer (it must fill), and
    //   at most the send buffer, the one chunk hyper holds waiting for capacity, the one frame the
    //   codec holds being written, and what the pipe took (with its frame headers).
    const DUPLEX: usize = 8 * 1024;
    const FRAME_HEADER: usize = 9;
    let lower = H2_SEND_BUF_BYTES;
    let upper = H2_SEND_BUF_BYTES + 2 * (EGRESS_CHUNK_BYTES + FRAME_HEADER) + DUPLEX;
    assert!(
        upper < 256 * 1024,
        "the derivation must sit far below hyper's 400 KiB default"
    );

    let polled = Arc::new(AtomicU64::new(0));
    let counter = polled.clone();
    let app = Router::new().route(
        "/endless",
        get(move || {
            let counter = counter.clone();
            async move {
                axum::body::Body::from_stream(futures_util::stream::repeat_with(move || {
                    counter.fetch_add(EGRESS_CHUNK_BYTES as u64, Ordering::SeqCst);
                    Ok::<_, std::convert::Infallible>(axum::body::Bytes::from(vec![
                        0u8;
                        EGRESS_CHUNK_BYTES
                    ]))
                }))
            }
        }),
    );
    let (server_end, mut peer) = tokio::io::duplex(DUPLEX);
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(serve_router(
        OneConnection(Some((
            H2Duplex(server_end),
            ConnMeta::plain("192.0.2.1:4000".parse().unwrap()),
        ))),
        app,
        async move {
            let _ = rx.await;
        },
        ConnLimits {
            drain_timeout: Duration::from_millis(300),
            ..ConnLimits::default()
        },
        || {},
    ));

    // Preface; SETTINGS with INITIAL_WINDOW_SIZE (id 4) = 2^31 - 1; a connection WINDOW_UPDATE;
    // HEADERS (END_STREAM | END_HEADERS) for GET /endless. Then the peer never reads.
    let mut open = H2_OPEN[..H2_PREFACE_LEN].to_vec();
    open.extend(h2_frame(0x4, 0, 0, &[0, 4, 0x7f, 0xff, 0xff, 0xff]));
    open.extend(h2_frame(0x8, 0, 0, &0x7fff_0000u32.to_be_bytes()));
    let mut block = vec![0x82, 0x86, 0x04, 0x08];
    block.extend_from_slice(b"/endless");
    open.extend(h2_frame(0x1, 0x5, 1, &block));
    peer.write_all(&open).await.unwrap();

    // Quiescent once the count has not moved for several settle intervals.
    let mut last = u64::MAX;
    let mut steady = 0;
    while steady < 6 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let now = polled.load(Ordering::SeqCst);
        steady = if now == last { steady + 1 } else { 0 };
        last = now;
    }
    assert!(
        last >= lower as u64,
        "hyper polled {last} bytes, fewer than the send buffer {lower}"
    );
    assert!(
        last <= upper as u64,
        "hyper polled {last} bytes, more than the send buffer allows ({upper})"
    );
    tx.send(()).unwrap();
    timeout(Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap();
}

// ---------------------------------------------------------------------------------------------
// The connection unit: a parked writer that takes no bytes is cut
// ---------------------------------------------------------------------------------------------

/// Runs `drive` over a connection whose only work is a write that parks forever on a peer that
/// accepts nothing, reporting its writes to the connection's own `live` state, and returns the
/// task, the state and the instant the drive started.
fn drive_parked_writer(
    window: Duration,
) -> (
    tokio::task::JoinHandle<()>,
    Arc<LiveState>,
    Arc<AtomicUsize>,
    Instant,
) {
    let kill = ConnKill::new();
    let watch = watch_for(kill.clone(), window);
    let live = watch.live.clone();
    let writes = WriteCounter::new(live.egress(), false);
    let mut io = Replay::new(Vec::new(), Stuck, kill, writes);
    let conn = async move {
        let _ = io.write_all(b"response bytes").await;
        Ok(())
    };
    let dropped = Arc::new(AtomicUsize::new(0));
    let (signal_tx, signal_rx) = watch::channel(());
    let counted = dropped.clone();
    let started = Instant::now();
    let task = tokio::spawn(async move {
        let _keep = signal_rx;
        drive(
            conn,
            signal_tx,
            std::future::pending(),
            |_| {},
            watch,
            &counted,
        )
        .await;
    });
    (task, live, dropped, started)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_writer_parked_with_nothing_accepted_is_cut_between_the_window_and_one_tick_after_it() {
    let window = Duration::from_secs(8);
    let tick = egress_tick(window);
    let (task, live, dropped, started) = drive_parked_writer(window);
    timeout(window + tick * 2, task)
        .await
        .expect("a parked writer was never cut")
        .unwrap();
    let elapsed = started.elapsed();
    assert!(
        elapsed >= window,
        "cut after {elapsed:?}, before the window {window:?}"
    );
    assert!(
        elapsed <= window + tick,
        "cut after {elapsed:?}, later than the window plus one tick"
    );
    assert_eq!(
        live.stall_counts(),
        StallCuts {
            stream: 0,
            connection: 1,
            ended_unflushed: 0,
            tracker_overflow: 0,
        }
    );
    assert_eq!(
        dropped.load(Ordering::SeqCst),
        0,
        "the IO failure ends it, not the drop"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_floor_of_zero_never_cuts_a_parked_writer() {
    // Nothing is below a floor of zero, so no amount of parking is a stall: the comparison is
    // against the configured floor and not a private constant.
    let window = Duration::from_secs(8);
    let kill = ConnKill::new();
    let mut watch = watch_for(kill.clone(), window);
    watch.egress_floor = 0;
    let live = watch.live.clone();
    let writes = WriteCounter::new(live.egress(), false);
    let mut io = Replay::new(Vec::new(), Stuck, kill, writes);
    let conn = async move {
        let _ = io.write_all(b"response bytes").await;
        Ok(())
    };
    let dropped = Arc::new(AtomicUsize::new(0));
    let (signal_tx, _keep) = watch::channel(());
    let task = tokio::spawn(async move {
        drive(
            conn,
            signal_tx,
            std::future::pending(),
            |_| {},
            watch,
            &dropped,
        )
        .await;
    });
    settle(window * 4).await;
    assert!(!task.is_finished(), "a floor of zero cut a connection");
    assert_eq!(live.stall_counts(), StallCuts::default());
    task.abort();
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_stream_waiting_behind_a_parked_writer_is_cut_as_a_connection_and_not_as_a_stream() {
    // The stream's pending chunk is a window old at the same tick the writer has been parked for
    // a window: the socket is what the response waits on, so only the connection unit acts.
    let window = Duration::from_secs(8);
    let tick = egress_tick(window);
    let (task, live, _dropped, _started) = drive_parked_writer(window);
    let stream_dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = DropFlag(stream_dropped.clone());
    let l = live.clone();
    StreamExec.execute_test(async move {
        let _flag = flag;
        let _body = stalled_body(&l).await;
        std::future::pending::<()>().await;
    });
    timeout(window + tick * 4, task)
        .await
        .expect("a parked writer was never cut")
        .unwrap();
    assert!(
        !stream_dropped.load(Ordering::SeqCst),
        "the stream unit cut a stream behind a parked writer"
    );
    assert_eq!(
        live.stall_counts(),
        StallCuts {
            stream: 0,
            connection: 1,
            ended_unflushed: 0,
            tracker_overflow: 0,
        }
    );
}

// ---------------------------------------------------------------------------------------------
// The write counter against the real HTTP/2 codec
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_http2_write_counter_equals_the_data_bytes_a_real_client_receives() {
    // Responses of several sizes, an empty one included, through the real h2 server codec writing
    // via `Replay`: every frame type the server writes (SETTINGS, HEADERS, WINDOW_UPDATE, DATA)
    // passes the parser, and only DATA payload is counted.
    let sizes: [usize; 6] = [0, 1, 100, EGRESS_CHUNK_BYTES, 100_000, 1 << 20];
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let egress = Arc::new(Egress::default());
    let kill = ConnKill::new();
    let io = Replay::new(
        Vec::new(),
        server_io,
        kill,
        WriteCounter::new(egress.clone(), true),
    );
    let service = hyper::service::service_fn(|req: axum::http::Request<Incoming>| async move {
        let n: usize = req.uri().path()[1..].parse().unwrap();
        Ok::<_, std::convert::Infallible>(axum::http::Response::new(axum::body::Body::from(
            vec![7u8; n],
        )))
    });
    let server = tokio::spawn(async move {
        let _ = http2::Builder::new(hyper_util::rt::TokioExecutor::new())
            .serve_connection(TokioIo::new(io), service)
            .await;
    });

    let (mut sender, conn) = h2::client::handshake(client_io).await.unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let mut received = 0u64;
    for size in sizes {
        let request = axum::http::Request::get(format!("http://test/{size}"))
            .body(())
            .unwrap();
        sender = sender.ready().await.unwrap();
        let (response, _) = sender.send_request(request, true).unwrap();
        let mut body = response.await.unwrap().into_body();
        while let Some(chunk) = body.data().await {
            let chunk = chunk.unwrap();
            received += chunk.len() as u64;
            let _ = body.flow_control().release_capacity(chunk.len());
        }
    }
    assert_eq!(received, sizes.iter().sum::<usize>() as u64);
    // The server counts a write when the socket accepts it, so everything the client has read has
    // been counted; anything still in flight when the last body ended is not DATA.
    assert_eq!(egress.accepted(), received);
    drop(sender);
    let _ = timeout(Duration::from_secs(5), server).await;
}

// ---------------------------------------------------------------------------------------------
// The stream table against the real HTTP/2 codec
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_stream_table_follows_a_real_client_codec_through_responses_and_cancels() {
    // A real h2 client over a duplex pipe to the real server codec, reading and writing through
    // `Replay`: the read parser creates an entry for every request, and the entry is gone once the
    // client has seen the response end (the writer accepted its END_STREAM frame before the client
    // could read it) or once the server has read the client's cancel.
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let egress = Arc::new(Egress::default());
    let io = Replay::new(
        Vec::new(),
        server_io,
        ConnKill::new(),
        WriteCounter::new(egress.clone(), true),
    );
    let service = hyper::service::service_fn(|req: axum::http::Request<Incoming>| async move {
        if req.uri().path() == "/hang" {
            std::future::pending::<()>().await;
        }
        Ok::<_, std::convert::Infallible>(axum::http::Response::new(axum::body::Body::from(
            vec![7u8; 5000],
        )))
    });
    let server = tokio::spawn(async move {
        let _ = http2::Builder::new(hyper_util::rt::TokioExecutor::new())
            .serve_connection(TokioIo::new(io), service)
            .await;
    });
    let (mut sender, conn) = h2::client::handshake(client_io).await.unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });

    for _ in 0..3 {
        let request = axum::http::Request::get("http://test/ok").body(()).unwrap();
        sender = sender.ready().await.unwrap();
        let (response, _) = sender.send_request(request, true).unwrap();
        let mut body = response.await.unwrap().into_body();
        while let Some(chunk) = body.data().await {
            let chunk = chunk.unwrap();
            let _ = body.flow_control().release_capacity(chunk.len());
        }
        assert_eq!(
            egress.stream_ids(),
            Vec::<u32>::new(),
            "a finished response left an entry"
        );
    }

    // A request the server never answers holds an entry until the client cancels it.
    let request = axum::http::Request::get("http://test/hang")
        .body(())
        .unwrap();
    sender = sender.ready().await.unwrap();
    let (response, mut send_stream) = sender.send_request(request, true).unwrap();
    timeout(Duration::from_secs(5), async {
        while egress.stream_ids() != [7] {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the pending request never gained an entry");
    send_stream.send_reset(h2::Reason::CANCEL);
    drop(response);
    timeout(Duration::from_secs(5), async {
        while !egress.stream_ids().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("a cancelled request kept its entry");
    assert!(egress.counters().table_peak() >= 1);
    drop(sender);
    let _ = timeout(Duration::from_secs(5), server).await;
}

// ---------------------------------------------------------------------------------------------
// The HTTP/2 limits the stream table's cap is derived from
// ---------------------------------------------------------------------------------------------

/// What the peer of a connection served with [`h2_builder`] saw after provoking `streams` library
/// resets.
struct Provoked {
    /// RST_STREAM frames the server wrote for the provoked streams.
    resets: usize,
    /// The error code of the GOAWAY that ended the connection, when one arrived.
    goaway: Option<u32>,
}

/// Serves one connection with the production [`h2_builder`] over a duplex pipe and has a raw client
/// provoke `streams` library resets: each stream opens with a request HEADERS block that does not
/// end the stream, then sends a second HEADERS block that does not end it either, a protocol error
/// h2 answers with an RST_STREAM it generates itself, which counts against
/// [`H2_MAX_LOCAL_ERROR_RESETS`].
async fn provoke_library_resets(streams: u32) -> Provoked {
    let (mut peer, server_io) = tokio::io::duplex(1 << 20);
    let service = hyper::service::service_fn(|_req: axum::http::Request<Incoming>| async move {
        std::future::pending::<Result<axum::http::Response<axum::body::Body>, io::Error>>().await
    });
    let limits = ConnLimits::default();
    let server = tokio::spawn(async move {
        let _ = h2_builder(&limits)
            .serve_connection(TokioIo::new(server_io), service)
            .await;
    });
    peer.write_all(&H2_OPEN[..H2_PREFACE_LEN]).await.unwrap();
    peer.write_all(&h2_frame(0x4, 0, 0, &[])).await.unwrap();
    // GET https, path "/" by static-table index.
    let block = [0x82, 0x87, 0x84];
    let mut provoked = Provoked {
        resets: 0,
        goaway: None,
    };
    let mut inbound = Vec::new();
    let mut scratch = [0u8; 16 * 1024];
    let mut sent = 0u32;
    while provoked.goaway.is_none() {
        if sent < streams {
            let sid = 1 + 2 * sent;
            let mut out = h2_frame(0x1, 0x4, sid, &block);
            out.extend(h2_frame(0x1, 0x4, sid, &[]));
            if peer.write_all(&out).await.is_err() {
                break;
            }
            sent += 1;
        }
        let wait = Duration::from_millis(if sent < streams { 1 } else { 500 });
        match timeout(wait, peer.read(&mut scratch)).await {
            Ok(Ok(0) | Err(_)) => break,
            Ok(Ok(n)) => inbound.extend_from_slice(&scratch[..n]),
            Err(_) if sent >= streams => break,
            Err(_) => {}
        }
        while inbound.len() >= 9 {
            let len = u32::from_be_bytes([0, inbound[0], inbound[1], inbound[2]]) as usize;
            if inbound.len() < 9 + len {
                break;
            }
            let kind = inbound[3];
            let payload = inbound[9..9 + len].to_vec();
            inbound.drain(..9 + len);
            if kind == 0x3 {
                provoked.resets += 1;
            }
            if kind == 0x7 && payload.len() >= 8 {
                provoked.goaway = Some(u32::from_be_bytes([
                    payload[4], payload[5], payload[6], payload[7],
                ]));
            }
        }
    }
    server.abort();
    provoked
}

#[tokio::test]
async fn the_connection_builder_pins_the_library_reset_budget_to_the_constant() {
    let provoked = provoke_library_resets(H2_MAX_LOCAL_ERROR_RESETS as u32 + 60).await;
    assert_eq!(
        provoked.goaway,
        Some(11),
        "the connection did not end with ENHANCE_YOUR_CALM"
    );
    assert_eq!(
        provoked.resets, H2_MAX_LOCAL_ERROR_RESETS,
        "the GOAWAY did not come at exactly the budget"
    );
}

// ---------------------------------------------------------------------------------------------
// The ended-unflushed unit inside `drive`'s tick
// ---------------------------------------------------------------------------------------------

/// The client connection preface.
const PREFACE: &[u8; 24] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

/// A request HEADERS frame (END_STREAM | END_HEADERS) on `sid`.
fn request_headers(sid: u32) -> Vec<u8> {
    h2_frame(0x1, 0x5, sid, &[0x82, 0x87, 0x84])
}

/// Gives `live`'s table an `Open` entry for stream `sid`, as the read parser would.
fn open_stream(live: &Arc<LiveState>, sid: u32) {
    let mut reads = WriteCounter::new(live.egress(), true).read_observer();
    let kill = ConnKill::new();
    reads.after_read(PREFACE, &kill);
    reads.after_read(&request_headers(sid), &kill);
}

/// The writer accepts a DATA frame of `bytes` payload bytes for stream `sid` on `live`'s connection.
fn write_data(live: &Arc<LiveState>, sid: u32, bytes: usize) {
    let mut writes = WriteCounter::new(live.egress(), true);
    let frame = h2_frame(0x0, 0, sid, &vec![0u8; bytes]);
    writes.after_write(&Poll::Ready(Ok(frame.len())), &[io::IoSlice::new(&frame)]);
}

/// Runs `drive` over a connection with no work and no IO (it never ends by itself) on its own
/// task, and returns the task, the kill switch and the count of connections `drive` dropped.
fn drive_idle(
    watch: Watch,
) -> (
    tokio::task::JoinHandle<()>,
    Arc<ConnKill>,
    Arc<AtomicUsize>,
    watch::Receiver<()>,
) {
    let kill = watch.kill.clone();
    let dropped = Arc::new(AtomicUsize::new(0));
    let counted = dropped.clone();
    let (signal_tx, signal_rx) = watch::channel(());
    let task = tokio::spawn(async move {
        drive(
            std::future::pending::<Result<(), hyper::Error>>(),
            signal_tx,
            std::future::pending(),
            |_| {},
            watch,
            &counted,
        )
        .await;
    });
    (task, kill, dropped, signal_rx)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_final_chunk_waiting_at_zero_credit_is_cut_as_a_stream_and_never_as_a_connection() {
    let window = Duration::from_secs(8);
    let kill = ConnKill::new();
    let watch = watch_for(kill.clone(), window);
    let live = watch.live.clone();
    open_stream(&live, 1);
    let stream_dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = DropFlag(stream_dropped.clone());
    let l = live.clone();
    StreamExec.execute_test(async move {
        let _flag = flag;
        // The body's last chunk is handed to hyper and hyper never polls again: zero credit.
        let _body = stalled_final_body(&l, 1).await;
        std::future::pending::<()>().await;
    });
    let (task, kill, dropped, _signal) = drive_idle(watch);
    // Long enough for the marker's window to pass, short enough that the entry the cut leaves
    // `Queued` (until the RST_STREAM the real codec would write) has not aged a window itself.
    settle(window + egress_tick(window) * 2).await;
    assert!(
        stream_dropped.load(Ordering::SeqCst),
        "the stalled stream was never cut"
    );
    assert_eq!(
        live.stall_counts(),
        StallCuts {
            stream: 1,
            ..StallCuts::default()
        }
    );
    assert!(
        kill.fired().now_or_never().is_none(),
        "a final chunk at zero credit cut the connection"
    );
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
    task.abort();
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_queued_tail_with_no_progress_cuts_the_connection_between_the_window_and_one_tick_after_it(
) {
    let window = Duration::from_secs(8);
    let tick = egress_tick(window);
    let kill = ConnKill::new();
    let watch = watch_for(kill.clone(), window);
    let live = watch.live.clone();
    open_stream(&live, 1);
    // The body ends and is dropped with its tail still queued: the entry is `Queued` from now.
    drop(stalled_final_body(&live, 1).await);
    let started = Instant::now();
    let (task, kill, dropped, _signal) = drive_idle(watch);
    let reason = timeout(window * 4, kill.fired())
        .await
        .expect("a queued tail with no progress never cut the connection");
    let elapsed = started.elapsed();
    assert_eq!(reason, KillReason::Stall);
    assert!(
        elapsed >= window && elapsed <= window + tick,
        "cut after {elapsed:?}, not within [{window:?}, {:?}]",
        window + tick
    );
    assert_eq!(
        live.stall_counts(),
        StallCuts {
            ended_unflushed: 1,
            ..StallCuts::default()
        }
    );
    // The idle connection ignores its failed IO, so `drive` ends it at the grace.
    settle(KILL_GRACE * 2).await;
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    task.abort();
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_tail_rule_and_the_connection_sampler_read_the_one_configured_floor() {
    // 30 bytes of a 64-byte tail are written late in the first window: progress against a floor of
    // 20, a stall against 5,000 (the threshold is min(floor, remaining) = 64). The same
    // `Watch::egress_floor` feeds the sampler, so a private constant in the tail rule would not move
    // with it. Ticks fall every `egress_tick(window)`; the write lands between the third and fourth, which the premise assertion below names.
    for (floor, cut) in [(20usize, false), (5_000, true)] {
        let window = Duration::from_secs(8);
        let tick = egress_tick(window);
        assert_eq!(
            EGRESS_TICKS_PER_WINDOW, 4,
            "the write offsets below place the write between the third and fourth of four ticks"
        );
        let kill = ConnKill::new();
        let mut watch = watch_for(kill.clone(), window);
        watch.egress_floor = floor;
        let live = watch.live.clone();
        open_stream(&live, 1);
        drop(stalled_final_body_of(&live, 1, 64).await);
        let (task, kill, _dropped, _signal) = drive_idle(watch);
        settle(window - tick - tick / 2).await;
        write_data(&live, 1, 30);
        // The window ends at the fourth tick; the new window a threshold-reaching write starts at
        // the next tick would not end before the fifth.
        settle(window).await;
        assert_eq!(kill.fired().now_or_never().is_some(), cut, "floor {floor}");
        task.abort();
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn each_cut_site_moves_exactly_its_own_counter_by_one() {
    // Unit by unit through the real firing sites: the marker scan, the byte sampler, the ended
    // scan and the table overflow.
    let window = Duration::from_secs(8);

    // Stream unit: a stalled marker.
    {
        let kill = ConnKill::new();
        let watch = watch_for(kill.clone(), window);
        let live = watch.live.clone();
        let l = live.clone();
        StreamExec.execute_test(async move {
            let _body = stalled_body(&l).await;
            std::future::pending::<()>().await;
        });
        let (task, _kill, _dropped, _signal) = drive_idle(watch);
        settle(window * 3).await;
        assert_eq!(
            live.stall_counts(),
            StallCuts {
                stream: 1,
                ..StallCuts::default()
            }
        );
        task.abort();
    }
    // Connection unit: a parked writer.
    {
        let (task, live, _dropped, _started) = drive_parked_writer(window);
        timeout(window * 2, task).await.unwrap().unwrap();
        assert_eq!(
            live.stall_counts(),
            StallCuts {
                connection: 1,
                ..StallCuts::default()
            }
        );
    }
    // Ended-unflushed unit: a queued tail.
    {
        let kill = ConnKill::new();
        let watch = watch_for(kill.clone(), window);
        let live = watch.live.clone();
        open_stream(&live, 1);
        drop(stalled_final_body(&live, 1).await);
        let (task, _kill, _dropped, _signal) = drive_idle(watch);
        settle(window * 2).await;
        assert_eq!(
            live.stall_counts(),
            StallCuts {
                ended_unflushed: 1,
                ..StallCuts::default()
            }
        );
        task.abort();
    }
    // Table overflow: the read parser past a one-entry cap.
    {
        let counters = Arc::new(StallCounters::default());
        let egress = Arc::new(Egress::new(counters.clone(), 1));
        let mut reads = WriteCounter::new(egress, true).read_observer();
        let kill = ConnKill::new();
        reads.after_read(PREFACE, &kill);
        reads.after_read(&request_headers(1), &kill);
        reads.after_read(&request_headers(3), &kill);
        assert_eq!(
            counters.snapshot(),
            StallCuts {
                tracker_overflow: 1,
                ..StallCuts::default()
            }
        );
    }
}

#[test]
fn the_sampler_period_is_nonzero_from_the_tick_count_in_nanoseconds_up() {
    let ticks = u64::from(EGRESS_TICKS_PER_WINDOW);
    for ns in 1..ticks {
        assert!(
            egress_tick(Duration::from_nanos(ns)).is_zero(),
            "{ns} ns quantises to a zero period"
        );
    }
    assert_eq!(
        egress_tick(Duration::from_nanos(ticks)),
        Duration::from_nanos(1)
    );
}

#[tokio::test]
async fn serve_router_rejects_a_window_whose_sampler_period_is_zero() {
    for ns in [1u64, 2, 3] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ended = tokio::spawn(timeout(
            Duration::from_secs(5),
            serve_router(
                listener,
                Router::new(),
                std::future::pending::<()>(),
                ConnLimits {
                    egress_window: Duration::from_nanos(ns),
                    ..ConnLimits::default()
                },
                || {},
            ),
        ))
        .await;
        let err = ended.expect_err("serve_router served on a window below four nanoseconds");
        assert!(err.is_panic(), "{ns} ns: {err:?}");
    }
}

#[tokio::test]
async fn a_plain_tcp_listener_source_sets_tcp_nodelay_on_accepted_connections() {
    let mut listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (accepted, _meta) = ConnSource::accept(&mut listener)
        .await
        .unwrap()
        .expect("a bound listener never reports the end of the source");
    assert!(
        accepted.nodelay().unwrap(),
        "a socket accepted through the ConnSource impl for TcpListener lacks TCP_NODELAY"
    );
    drop(client);
}

#[tokio::test]
async fn a_nodelay_listener_sets_tcp_nodelay_on_the_socket_it_accepts() {
    use axum::serve::Listener;

    let mut listener = NodelayListener::new(TcpListener::bind("127.0.0.1:0").await.unwrap());
    let addr = listener.local_addr().unwrap();
    let client = TcpStream::connect(addr).await.unwrap();
    let (accepted, peer) = listener.accept().await;
    assert_eq!(peer, client.local_addr().unwrap());
    assert!(
        accepted.nodelay().unwrap(),
        "an accepted socket has Nagle's algorithm on"
    );
}
