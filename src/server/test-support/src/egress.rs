//! Fixtures for the egress no-progress tests: a serve loop over the real router plus routes that
//! stall in controlled ways, and an HTTP/2 client whose receive windows and read pace the test
//! sets, so a stall is driven by the credit the client returns rather than by a sleep.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::Query;
use axum::routing::get;
use shadowcat::http::conn_serve::{serve_router, ConnLimits, DrainReport, ServeEnd, ServeOutcome};
use shadowcat::http::router;
use shadowcat::ws::WsState;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::h2_session::{login, seeded_state};
use crate::h2c::{h2c_connect, EndHandle};
use crate::observed::{ClosedLog, ObservedSource};
use crate::tls::{H2Client, H2Pipe};

/// The size of one chunk the `/endless` route produces.
pub const ENDLESS_CHUNK: usize = 16 * 1024;

/// Which HTTP version an [`EgressServer`]'s connections speak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// HTTP/1.1 over plain TCP.
    H1,
    /// HTTP/2 with prior knowledge over plain TCP.
    H2,
}

/// A serve loop over the real router plus the egress routes, over HTTP/1.1 or HTTP/2 with prior
/// knowledge.
///
/// Routes beyond the application's: `/endless` produces [`ENDLESS_CHUNK`]-byte chunks forever
/// (counted in [`EgressServer::produced`]); `/empty` answers with no body; `/sparse` produces
/// `chunks` chunks of 100 bytes, `gap_ms` milliseconds apart (query parameters).
pub struct EgressServer {
    /// The bound address.
    pub addr: SocketAddr,
    /// The session registry.
    pub ws: WsState,
    /// The seeded world a tunnel joins.
    pub world: uuid::Uuid,
    /// Bytes `/endless` has produced.
    pub produced: Arc<AtomicUsize>,
    /// The instants the server dropped its end of each connection, in drop order.
    pub closed: ClosedLog,
    /// Keeps the source open: dropping it ends the source.
    _end: EndHandle,
    /// Resolves the shutdown future.
    shutdown: oneshot::Sender<()>,
    /// The serve loop.
    task: JoinHandle<ServeOutcome>,
}

impl EgressServer {
    /// Serves HTTP/2 with `limits`. The drain bound is whatever `limits` carries.
    pub async fn start(limits: ConnLimits) -> Self {
        Self::start_with(limits, Protocol::H2).await
    }

    /// Serves `protocol` with `limits`.
    pub async fn start_with(limits: ConnLimits, protocol: Protocol) -> Self {
        let (state, world) = seeded_state().await;
        let ws = state.ws.clone();
        let hook_ws = ws.clone();
        let produced = Arc::new(AtomicUsize::new(0));
        let counter = produced.clone();
        let app = router(state)
            .await
            .route(
                "/endless",
                get(move || {
                    let counter = counter.clone();
                    async move {
                        Body::from_stream(futures_util::stream::repeat_with(move || {
                            counter.fetch_add(ENDLESS_CHUNK, Ordering::SeqCst);
                            Ok::<_, Infallible>(Bytes::from(vec![0u8; ENDLESS_CHUNK]))
                        }))
                    }
                }),
            )
            .route("/empty", get(|| async { Body::empty() }))
            .route(
                "/sparse",
                get(|Query(q): Query<HashMap<String, u64>>| async move {
                    let chunks = q.get("chunks").copied().unwrap_or(1);
                    let gap = Duration::from_millis(q.get("gap_ms").copied().unwrap_or(0));
                    Body::from_stream(futures_util::stream::unfold(0u64, move |sent| async move {
                        if sent == chunks {
                            return None;
                        }
                        tokio::time::sleep(gap).await;
                        Some((Ok::<_, Infallible>(Bytes::from(vec![1u8; 100])), sent + 1))
                    }))
                }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (source, end, closed) = ObservedSource::new(listener, protocol == Protocol::H2);
        let (shutdown, shutdown_rx) = oneshot::channel::<()>();
        let task = tokio::spawn(serve_router(
            source,
            app,
            async move {
                let _ = shutdown_rx.await;
            },
            limits,
            move || hook_ws.trigger_shutdown(),
        ));
        Self {
            addr,
            ws,
            world,
            produced,
            closed,
            _end: end,
            shutdown,
            task,
        }
    }

    /// Shuts the loop down, waits for it to return, and returns its drain report. Asserts the loop
    /// ended by shutdown, not by its source ending.
    pub async fn finish(self) -> DrainReport {
        self.shutdown.send(()).unwrap();
        let out = tokio::time::timeout(Duration::from_secs(30), self.task)
            .await
            .expect("the serve loop never returned")
            .unwrap();
        assert_eq!(out.end, ServeEnd::Shutdown);
        out.drain
    }

    /// Opens a WebSocket tunnel on `client`'s connection (logging in over a separate connection)
    /// and waits until the session is registered.
    pub async fn open_tunnel(&self, client: &mut H2Client) -> H2Pipe {
        let mut login_client = h2c_connect(self.addr, h2::client::Builder::new())
            .await
            .unwrap();
        let cookie = login(&mut login_client).await;
        let (status, tunnel) = client
            .websocket(
                &self.addr.to_string(),
                &format!("/ws?world={}", self.world),
                &cookie,
            )
            .await
            .unwrap();
        assert_eq!(status, 200);
        let ws = self.ws.clone();
        tokio::time::timeout(Duration::from_secs(5), async move {
            while ws.live_connection_count() == 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the session never registered");
        tunnel
    }

    /// Connects an HTTP/2 client with the given receive windows.
    pub async fn connect(&self, stream_window: u32, connection_window: u32) -> H2Client {
        let mut builder = h2::client::Builder::new();
        builder
            .initial_window_size(stream_window)
            .initial_connection_window_size(connection_window)
            .enable_push(false);
        h2c_connect(self.addr, builder).await.unwrap()
    }
}

/// Sends `GET path` on `client` and waits for the response head.
pub async fn get_path(
    client: &mut H2Client,
    addr: SocketAddr,
    path: &str,
) -> http::Response<h2::RecvStream> {
    client.sender = client.sender.clone().ready().await.unwrap();
    let req = http::Request::get(format!("http://{addr}{path}"))
        .body(())
        .unwrap();
    let (response, _) = client.sender.send_request(req, true).unwrap();
    response.await.unwrap()
}

/// How a paced read ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PacedEnd {
    /// The server reset the stream with this reason (a stall cut is `CANCEL`).
    Reset(h2::Reason),
    /// The response body ended.
    Ended,
    /// The stream was still open at the deadline.
    Deadline,
}

/// What a paced read observed.
#[derive(Debug, Clone)]
pub struct Paced {
    /// How the read ended.
    pub end: PacedEnd,
    /// Body bytes read.
    pub got: u64,
    /// Time from the start of the read to its end.
    pub elapsed: Duration,
}

/// Reads `body` at about `bytes_per_sec`, returning each chunk's capacity to the sender as it is
/// read and sleeping `len / bytes_per_sec` per chunk, until the body ends, the server resets the
/// stream, or `limit` elapses.
pub async fn paced_read(body: &mut h2::RecvStream, bytes_per_sec: u64, limit: Duration) -> Paced {
    let start = Instant::now();
    let mut got = 0u64;
    let end = loop {
        if start.elapsed() >= limit {
            break PacedEnd::Deadline;
        }
        match tokio::time::timeout(Duration::from_millis(200), body.data()).await {
            Ok(Some(Ok(chunk))) => {
                got += chunk.len() as u64;
                let _ = body.flow_control().release_capacity(chunk.len());
                tokio::time::sleep(Duration::from_secs_f64(
                    chunk.len() as f64 / bytes_per_sec as f64,
                ))
                .await;
            }
            Ok(Some(Err(e))) => {
                break PacedEnd::Reset(e.reason().unwrap_or(h2::Reason::INTERNAL_ERROR))
            }
            Ok(None) => break PacedEnd::Ended,
            Err(_) => {}
        }
    };
    Paced {
        end,
        got,
        elapsed: start.elapsed(),
    }
}

/// Reads `body` without returning any capacity, until the stream ends or is reset, or `limit`
/// elapses. Returns how it ended and how long it took.
pub async fn read_unreleased(body: &mut h2::RecvStream, limit: Duration) -> (PacedEnd, Duration) {
    let start = Instant::now();
    let end = loop {
        let left = limit.saturating_sub(start.elapsed());
        if left.is_zero() {
            break PacedEnd::Deadline;
        }
        match tokio::time::timeout(left, body.data()).await {
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(e))) => {
                break PacedEnd::Reset(e.reason().unwrap_or(h2::Reason::INTERNAL_ERROR))
            }
            Ok(None) => break PacedEnd::Ended,
            Err(_) => break PacedEnd::Deadline,
        }
    };
    (end, start.elapsed())
}

/// A TCP stream whose reads can be stopped, standing in for a peer that stops reading the socket:
/// once `reading` is cleared its reads never complete and no waker is registered, so the peer's
/// kernel receive buffer fills and the server's writes park.
pub struct GatedStream {
    /// The socket.
    inner: tokio::net::TcpStream,
    /// Cleared to stop reading.
    reading: Arc<std::sync::atomic::AtomicBool>,
}

impl tokio::io::AsyncRead for GatedStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if !self.reading.load(Ordering::SeqCst) {
            return std::task::Poll::Pending;
        }
        std::pin::Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl tokio::io::AsyncWrite for GatedStream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

impl EgressServer {
    /// Connects an HTTP/2 client with the given receive windows over a [`GatedStream`], and
    /// returns it with the flag that stops its socket reads when cleared.
    pub async fn connect_gated(
        &self,
        stream_window: u32,
        connection_window: u32,
    ) -> (H2Client, Arc<std::sync::atomic::AtomicBool>) {
        let inner = tokio::net::TcpStream::connect(self.addr).await.unwrap();
        let reading = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let mut builder = h2::client::Builder::new();
        builder
            .initial_window_size(stream_window)
            .initial_connection_window_size(connection_window)
            .enable_push(false);
        let (sender, conn) = builder
            .handshake(GatedStream {
                inner,
                reading: reading.clone(),
            })
            .await
            .unwrap();
        let driver = tokio::spawn(async move {
            let _ = conn.await;
        });
        (H2Client { sender, driver }, reading)
    }
}

/// Polls `counter` every `poll` until it has not changed for `settle`, or panics after `limit`,
/// and returns the instant at which that was confirmed. Loop-until-parked: a writer parks when the
/// socket buffers are full, which no fixed byte count predicts across operating systems.
///
/// The returned instant is no earlier than the moment the writer parked: the producer is paced by
/// what the writer takes, so it cannot go quiet before the writer stops taking.
pub async fn wait_until_flat(
    counter: &AtomicUsize,
    poll: Duration,
    settle: Duration,
    limit: Duration,
) -> Instant {
    let start = Instant::now();
    let mut seen = counter.load(Ordering::SeqCst);
    let mut last_change = start;
    loop {
        tokio::time::sleep(poll).await;
        let now = Instant::now();
        let value = counter.load(Ordering::SeqCst);
        if value != seen {
            seen = value;
            last_change = now;
        } else if seen > 0 && now.duration_since(last_change) >= settle {
            return now;
        }
        assert!(
            now.duration_since(start) < limit,
            "the producer never went quiet within {limit:?}"
        );
    }
}

/// The first instant at which the server dropped a connection, waiting up to `limit` for it.
pub async fn first_close(closed: &ClosedLog, limit: Duration) -> Option<Instant> {
    let deadline = Instant::now() + limit;
    loop {
        if let Some(at) = closed.lock().unwrap().first().copied() {
            return Some(at);
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// One HTTP/2 frame.
fn raw_frame(kind: u8, flags: u8, stream: u32, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u32;
    let mut out = len.to_be_bytes()[1..].to_vec();
    out.push(kind);
    out.push(flags);
    out.extend_from_slice(&stream.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// One PING frame (opaque payload, not an acknowledgement).
pub fn ping_frame() -> Vec<u8> {
    raw_frame(0x6, 0, 0, &[0u8; 8])
}

/// Connects a raw HTTP/2 client that requests `GET path` with windows at the protocol maximum, so
/// only the socket can stop the server, and returns the socket. The client never reads it.
pub async fn raw_h2_request(addr: SocketAddr, path: &str) -> tokio::net::TcpStream {
    use tokio::io::AsyncWriteExt;

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut out = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n".to_vec();
    // SETTINGS: INITIAL_WINDOW_SIZE (id 4) = 2^31 - 1.
    out.extend(raw_frame(0x4, 0, 0, &[0, 4, 0x7f, 0xff, 0xff, 0xff]));
    // WINDOW_UPDATE on the connection so its window is as large as the streams'.
    out.extend(raw_frame(0x8, 0, 0, &0x7fff_0000u32.to_be_bytes()));
    // HEADERS, END_STREAM | END_HEADERS: :method GET, :scheme http, :path as a literal value
    // under the static-table name.
    let mut block = vec![0x82, 0x86, 0x04, path.len() as u8];
    block.extend_from_slice(path.as_bytes());
    out.extend(raw_frame(0x1, 0x5, 1, &block));
    stream.write_all(&out).await.unwrap();
    stream
}
