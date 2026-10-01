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

    async fn accept(&mut self) -> Option<(TcpStream, ConnMeta)> {
        None
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

    async fn accept(&mut self) -> Option<(TcpStream, ConnMeta)> {
        let listener = self.listener.as_ref()?;
        let (sock, peer) = listener.accept().await.ok()?;
        self.listener = None;
        Some((sock, ConnMeta::plain(peer)))
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

    async fn accept(&mut self) -> Option<(AlpnH2, ConnMeta)> {
        let (sock, peer) = self.0.accept().await.ok()?;
        Some((AlpnH2(sock), ConnMeta::plain(peer)))
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

fn replay(prefix: &[u8], tail: &[u8]) -> Replay<Probe> {
    Replay {
        prefix: prefix.to_vec(),
        pos: 0,
        inner: Probe::new(tail),
        kill: ConnKill::new(),
    }
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
        let mut io = Replay {
            prefix: b"PRI".to_vec(),
            pos: 0,
            inner: Probe::new(b"tail"),
            kill: signal.connection(),
        };
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
        let mut io = Replay {
            prefix: Vec::new(),
            pos: 0,
            inner: Stuck,
            kill: signal.connection(),
        };
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
    let mut io = Replay {
        prefix: Vec::new(),
        pos: 0,
        inner: Probe::new(b"tail"),
        kill: signal.connection(),
    };
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
const H2_OPEN: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n\x00\x00\x00\x04\x00\x00\x00\x00\x00";

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

    async fn accept(&mut self) -> Option<(TcpStream, ConnMeta)> {
        let (sock, peer) = self.0.accept().await.ok()?;
        let mut meta = ConnMeta::plain(peer);
        meta.cap_exempt = true;
        Some((sock, meta))
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
