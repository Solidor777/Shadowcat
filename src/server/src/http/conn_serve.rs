//! The connection serve loop: accepts from a [`ConnSource`], hands every connection to hyper with
//! the time bounds `axum::serve` cannot express, and drains on shutdown.
//!
//! WHY this owns the loop instead of calling `axum::serve`: hyper's HTTP/1 header-read timeout and
//! HTTP/2 keep-alive pings are inert without a timer, and `axum::serve` installs none. After the
//! listener's hand-off nothing would then bound how long a peer may trickle request headers or sit
//! idle on a kept-alive connection, so a peer past the accept-time caps ([`crate::tls::listener`])
//! could hold a connection indefinitely at no cost. Here every connection is built with a
//! `TokioTimer` and the bounds in [`ConnLimits`].
//!
//! INVARIANTS:
//! 1. The HTTP version is chosen from the negotiated ALPN protocol ([`ServeIo::is_h2`]), never by
//!    reading the connection, and each version's hyper builder is used directly: hyper-util's auto
//!    builder reads a 24-byte preface before any timer exists (and ignores its `http1_only` /
//!    `http2_only` under `serve_connection_with_upgrades`), so a peer sending a partial first line
//!    would stall it unbounded. Plain-text connections therefore speak HTTP/1.1 only (HTTP/2 with
//!    prior knowledge is not supported: no browser uses it); TLS connections speak what ALPN
//!    negotiated, HTTP/1.1 when it negotiated nothing. An `h2` connection's preface is read under
//!    the header-read bound by this module (`read_h2_preface`), because hyper arms no timer during
//!    the HTTP/2 server handshake.
//! 2. `ConnectInfo<ConnMeta>` is inserted into every request here and in
//!    [`super::scheme::into_service`](crate::http::scheme::into_service) (for `axum::serve` transports) and nowhere else.
//! 3. Graceful shutdown mirrors `axum::serve`: a watch channel tells every connection task to call
//!    `graceful_shutdown`, and the loop returns once every task has ended. A source that ends
//!    drains the same way and the loop returns [`ServeEnd::SourceEnded`]. Both causes first run
//!    the caller's `on_drain` hook, which is where sessions hyper cannot see (WebSockets) are told
//!    to close: an HTTP/2 connection resolves only when its streams end, and a WebSocket is a
//!    stream, so draining without telling the sessions would wait on them. The drain itself is
//!    bounded by [`ConnLimits::drain_timeout`](ConnLimits::drain_timeout): connections still open past it have their IO
//!    failed (the `kill` module) and `drive` drops one that has not ended [`KILL_GRACE`] later
//!    ([`DrainReport::dropped`](DrainReport::dropped)); a task still running [`ABORT_WAIT`] after the IO failed (one not
//!    in `drive`, or one `drive` could not run) is aborted and reported in
//!    [`DrainReport::aborted`](DrainReport::aborted). A connection task ends when its IO errors, so each is a defect.
//! 4. Every live connection is bounded, not only the half-open ones. [`ConnLimits::max_live`](ConnLimits::max_live) and
//!    [`ConnLimits::max_live_per_peer`](ConnLimits::max_live_per_peer) cap connections from hand-off until the connection task
//!    AND every [`ConnLease`] taken from its requests have ended, keyed and exempted exactly like
//!    the accept-time caps ([`crate::http::peer_slots::PeerSlots`], `ConnMeta::cap_exempt`, decided
//!    once by `Config::is_cap_exempt`); a
//!    connection over a cap is closed unread. [`ConnLimits::h2_idle`](ConnLimits::h2_idle) closes an HTTP/2 connection
//!    that has had no request in flight for that long. A lease held by a WebSocket session is
//!    released when the session ends, and a session whose peer has stopped reading ends at the
//!    per-send stall bound ([`crate::ws::bounded_sink::BoundedSink::for_config`]), so a lease cannot
//!    pin a slot for longer than that bound past the peer's last read.
//! 5. The pause after an accept error is decided in [`serve_router`], never inside a
//!    [`ConnSource::accept`] future: the loop drops that future on every reaped connection task, so
//!    a pause held inside it would be cut short by the next reap. A source returns the error and
//!    [`accept_error_pause`] is the one policy every caller (this loop and `SniffListener`'s
//!    acceptor task) consults.
//! 6. A response that makes no progress for [`ConnLimits::egress_window`](ConnLimits::egress_window) is cut, by exactly one of
//!    three units that share one definition of no progress. The stream unit resets one HTTP/2
//!    stream whose pending chunk the peer returns no flow-control credit for; the connection unit
//!    closes a connection whose writer stays parked on a socket that accepts fewer than
//!    [`ConnLimits::egress_floor`](ConnLimits::egress_floor) bytes; the ended-unflushed unit closes a connection whose
//!    finished response has DATA still queued in h2 that its own stream writes less than
//!    `min(egress_floor, bytes still queued)` of per window (the stream has no handle left to reset
//!    it, and only a stream's own DATA counts as its progress: not another stream's, a tunnel's,
//!    a PING's or any other write). All three run on `drive`'s sampler tick, in the order
//!    connection, ended-unflushed, stream; the last two run only when the writer is not parked, and
//!    the stream unit runs only when the ended-unflushed unit cut nothing, so a parked writer is
//!    the connection unit's case alone. Every cut is counted in [`DrainReport::stall_cuts`](DrainReport::stall_cuts).
//! 7. An HTTP/2 connection's IO walks the frames of both directions (the `egress` module's frame
//!    parser) and keeps a table of the connection's live client streams in step with the wire: an
//!    entry is created when a request's HEADERS frame is read and removed only when the wire shows
//!    the stream ended. The table's cap is the stream limit (`H2_MAX_CONCURRENT_STREAMS`) plus a
//!    derived parse lag, and it fails closed: a connection whose table would outgrow it is cut and
//!    counted in [`StallCuts::tracker_overflow`](StallCuts::tracker_overflow). The highest length any table reached is
//!    [`DrainReport::stream_table_peak`](DrainReport::stream_table_peak).
//! 8. A WebSocket inside an HTTP/2 stream has no response body for the stream unit to watch, and
//!    hyper's tunnel task hands every write to h2 whether or not the peer has granted credit, so a
//!    session's own sends never go pending. [`TunnelProbe`], which the connection's lease service
//!    puts in every HTTP/2 request's extensions, reports the DATA payload bytes the writer
//!    accepted for the session's stream; `ws::tunnel_gate::TunnelGate` stops the session writing
//!    while it is more than [`H2_SEND_BUF_BYTES`] ahead of that, which makes the send pending and
//!    starts the per-send stall bound.
//!
//! Timeout semantics measured against hyper 1.x: the HTTP/1 header-read timeout is armed on every
//! wait for a request head, so it bounds both a slow head and an idle keep-alive gap; it does not
//! apply after a WebSocket upgrade, to request bodies, or to a running handler. HTTP/2 keep-alive
//! closes a connection whose peer stops acknowledging pings, and hyper has no idle timeout, so
//! [`ConnLimits::h2_idle`](ConnLimits::h2_idle) is enforced here from the in-flight count (see the `live` module).

mod egress;
mod kill;
mod live;
mod pruned;
mod stream_exec;

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::{pin, Pin};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::Body;
use axum::extract::connect_info::ConnectInfo;
use axum::extract::Request;
use axum::{Extension, Router};
use futures_util::FutureExt;
use hyper::body::Incoming;
use hyper::server::conn::{http1, http2};
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::{JoinError, JoinSet};
use tokio::time::Instant;
use tower::{Layer, ServiceExt};

pub use self::egress::TunnelProbe;
use self::egress::{ReadObserver, Sampler, StallCounters, StallUnit, WriteCounter};
use self::kill::{ConnKill, KillReason, KillSignal};
pub use self::live::ConnLease;
use self::live::{idle_for, LeaseService, LiveState};
use self::stream_exec::StreamExec;
use crate::http::peer_slots::PeerSlots;
use crate::tls::ConnMeta;

/// The IO type of an accepted connection, able to say whether it negotiated HTTP/2.
///
/// # Examples
///
/// ```
/// use shadowcat::http::conn_serve::ServeIo;
///
/// fn assert_io<T: ServeIo>() {}
/// assert_io::<tokio::net::TcpStream>();
/// ```
pub trait ServeIo: AsyncRead + AsyncWrite + Unpin + Send + 'static {
    /// Whether ALPN negotiated `h2` on this connection. Plain-text connections are never HTTP/2.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use shadowcat::http::conn_serve::ServeIo;
    ///
    /// # #[tokio::main] async fn main() {
    /// let s = tokio::net::TcpStream::connect("127.0.0.1:1").await.unwrap();
    /// assert!(!s.is_h2());
    /// # }
    /// ```
    fn is_h2(&self) -> bool;
}

impl ServeIo for TcpStream {
    fn is_h2(&self) -> bool {
        false
    }
}

/// A source of ready connections. `Ok(None)` means the source has ended for good (the serve loop
/// then stops accepting and drains); an `Err` is an accept failure the caller paces with
/// [`accept_error_pause`].
///
/// # Examples
///
/// ```
/// use shadowcat::http::conn_serve::ConnSource;
///
/// fn assert_source<T: ConnSource>() {}
/// assert_source::<tokio::net::TcpListener>();
/// ```
pub trait ConnSource: Send + 'static {
    /// The connection IO type.
    type Io: ServeIo;

    /// The next connection with its metadata, `Ok(None)` when the source is finished, or the
    /// accept error. CONTRACT: the future is cancel-safe (dropping it between polls loses no
    /// connection, because [`serve_router`] drops it on every reaped task), and it returns errors
    /// instead of sleeping on them: the pause is the caller's, see [`accept_error_pause`]. A
    /// production implementation that hands out a plain TCP socket (`impl ConnSource for
    /// TcpListener`, `SniffListener`'s accept task) and a source that models the production
    /// listener's throughput path configure the socket through [`set_accepted_nodelay`] before
    /// returning it; a source that only carries test traffic needs no socket policy.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use shadowcat::http::conn_serve::ConnSource;
    ///
    /// # #[tokio::main] async fn main() {
    /// let mut l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    /// let next = ConnSource::accept(&mut l).await.unwrap();
    /// assert!(next.is_some());
    /// # }
    /// ```
    fn accept(&mut self) -> impl Future<Output = io::Result<Option<(Self::Io, ConnMeta)>>> + Send;
}

/// A bare listener sets `TCP_NODELAY` on every socket it accepts through [`set_accepted_nodelay`].
impl ConnSource for TcpListener {
    type Io = TcpStream;

    async fn accept(&mut self) -> io::Result<Option<(TcpStream, ConnMeta)>> {
        let (sock, peer) = TcpListener::accept(self).await?;
        set_accepted_nodelay(&sock, peer);
        Ok(Some((sock, ConnMeta::plain(peer))))
    }
}

/// What the server configures on an accepted TCP socket: `TCP_NODELAY`, so the kernel does not
/// hold back small writes to coalesce them. This function is the one definition of that policy.
/// Its callers are `impl ConnSource for TcpListener`, `SniffListener`'s accept task and
/// [`NodelayListener`] (the listener the audio-monitor server hands to `axum::serve`), each once
/// per accepted socket; the test-support sources that model the production listener call it too.
///
/// Measured effect of the option: a 24 MiB download over HTTP/2 against a bare hyper server with a
/// 64 KiB send buffer, on Windows loopback in a release build (`egress_measure`'s `layers 150 5`,
/// rungs `bare-64k` and `bare-64k-no-nodelay`), median of five run medians: 11.71 ms with the
/// option and 16.93 ms without (run medians 15.81 to 19.79 ms without). Other operating systems
/// and non-loopback links are not measured.
///
/// A failure to set the option is logged at debug level naming `peer`, and the connection proceeds
/// with the system default.
///
/// # Examples
///
/// ```
/// use shadowcat::http::conn_serve::set_accepted_nodelay;
///
/// # #[tokio::main] async fn main() {
/// let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
/// let client = tokio::net::TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
/// let (accepted, peer) = listener.accept().await.unwrap();
/// assert!(!accepted.nodelay().unwrap());
/// set_accepted_nodelay(&accepted, peer);
/// assert!(accepted.nodelay().unwrap());
/// drop(client);
/// # }
/// ```
pub fn set_accepted_nodelay(sock: &TcpStream, peer: SocketAddr) {
    if let Err(e) = sock.set_nodelay(true) {
        tracing::debug!("could not set TCP_NODELAY on a connection from {peer}: {e}");
    }
}

/// A [`TcpListener`] for `axum::serve` whose accepted sockets get `TCP_NODELAY` through
/// [`set_accepted_nodelay`] with the real peer address. `axum::serve::ListenerExt::tap_io` hands
/// its closure only the stream, so it cannot call the helper; this wrapper is the `axum::serve`
/// counterpart of `impl ConnSource for TcpListener`. Accept errors are handled by axum's own
/// `Listener` implementation for [`TcpListener`] (logged, retried with a pause).
///
/// # Examples
///
/// ```
/// use axum::serve::Listener;
/// use shadowcat::http::conn_serve::NodelayListener;
///
/// # #[tokio::main] async fn main() {
/// let mut listener = NodelayListener::new(tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap());
/// let addr = listener.local_addr().unwrap();
/// let client = tokio::net::TcpStream::connect(addr).await.unwrap();
/// let (accepted, _peer) = listener.accept().await;
/// assert!(accepted.nodelay().unwrap());
/// drop(client);
/// # }
/// ```
pub struct NodelayListener(TcpListener);

impl NodelayListener {
    /// Wraps `listener`.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::http::conn_serve::NodelayListener;
    ///
    /// # #[tokio::main] async fn main() {
    /// let _listener = NodelayListener::new(tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap());
    /// # }
    /// ```
    pub fn new(listener: TcpListener) -> Self {
        Self(listener)
    }
}

impl axum::serve::Listener for NodelayListener {
    type Io = TcpStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (TcpStream, SocketAddr) {
        let (sock, peer) = <TcpListener as axum::serve::Listener>::accept(&mut self.0).await;
        set_accepted_nodelay(&sock, peer);
        (sock, peer)
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.0.local_addr()
    }
}

/// Axum's accept-error policy, shared by every caller of a source's `accept`: a connection-level
/// error (refused, aborted, reset: the peer went away) is ignored and yields `None`; anything else
/// (for example running out of file descriptors) is logged at error level and yields the
/// [`ACCEPT_ERROR_PAUSE`] the caller must wait before accepting again, so the loop does not spin.
/// The caller owns the wait: sleeping inside a `ConnSource::accept` future would be undone each
/// time that future is dropped.
///
/// # Examples
///
/// ```
/// use shadowcat::http::conn_serve::{accept_error_pause, ACCEPT_ERROR_PAUSE};
///
/// let reset = std::io::Error::from(std::io::ErrorKind::ConnectionReset);
/// assert_eq!(accept_error_pause(&reset), None);
/// let exhausted = std::io::Error::from(std::io::ErrorKind::Other);
/// assert_eq!(accept_error_pause(&exhausted), Some(ACCEPT_ERROR_PAUSE));
/// ```
pub fn accept_error_pause(e: &io::Error) -> Option<Duration> {
    let pause = pause_for(e.kind());
    if pause.is_some() {
        tracing::error!("accept error: {e}");
    }
    pause
}

/// The decision table of [`accept_error_pause`]: no pause for the connection-level kinds
/// (`ConnectionRefused`, `ConnectionAborted`, `ConnectionReset`), [`ACCEPT_ERROR_PAUSE`] for every
/// other kind. Source: axum's `serve` accept loop, which ignores exactly these three kinds.
fn pause_for(kind: io::ErrorKind) -> Option<Duration> {
    match kind {
        io::ErrorKind::ConnectionRefused
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::ConnectionReset => None,
        _ => Some(ACCEPT_ERROR_PAUSE),
    }
}

/// The pause after a non-connection-level accept error.
pub const ACCEPT_ERROR_PAUSE: Duration = Duration::from_secs(1);

/// The largest data frame a response body hands to hyper, and so the smallest unit whose
/// completion the egress bounds can observe. Equals HTTP/2's default maximum frame size (16 KiB),
/// so a slice is one frame. The stream unit's boundary is this chunk's wait for credit against
/// [`ConnLimits::egress_window`]; it does not read [`ConnLimits::egress_floor`], whose default is
/// this value and which tests vary independently of the chunk.
///
/// # Examples
///
/// ```
/// use shadowcat::http::conn_serve::{ConnLimits, EGRESS_CHUNK_BYTES};
///
/// assert_eq!(ConnLimits::default().egress_floor, EGRESS_CHUNK_BYTES);
/// ```
pub const EGRESS_CHUNK_BYTES: usize = 16 * 1024;

/// The default of [`crate::config::Config::egress_window_secs`]: how long a peer may return no
/// flow-control credit, or a socket accept nothing, before a response is cut, and how long one
/// WebSocket send may stay pending ([`crate::ws::bounded_sink`]). Not below
/// `h2_ping_interval + h2_ping_timeout` (50 s), so a peer that is merely slow to acknowledge pings
/// is ended by keep-alive rather than by a stall cut; it equals the default `h2_idle`. Those two
/// constraints bind the default only: an operator who sets the window is not bound by them.
pub const PEER_STALL_TIMEOUT: Duration = Duration::from_secs(60);

/// The HTTP/2 send buffer per stream, in bytes (`http2::Builder::max_send_buf_size`): how much
/// response data h2 queues for one stream beyond what the peer's window has let out. Bounds the
/// body bytes polled ahead of an unread peer to this plus one chunk, and the unsent backlog behind
/// a socket that accepts nothing. A WebSocket tunnel's lead over the wire is held to the same
/// value ([`crate::ws::tunnel_gate`]), because hyper's tunnel task ignores the buffer. The DATA
/// still queued behind a stalled stream window once a body has ended is at most one chunk, not
/// this: hyper hands a chunk over only while the stream's buffer holds less than the credit the
/// peer has granted. Chosen over hyper's 400 KiB default for these bounds (about a fifth of the
/// memory a stalled peer can pin per stream), and kept above [`EGRESS_CHUNK_BYTES`] so a stream
/// always has room to queue one whole chunk.
///
/// The cost is download throughput on a fast link. Measured on Windows loopback with the `h2`
/// client (24 MiB bodies, 6 MiB stream window, a bare hyper server, 100 downloads per setting, 3
/// runs): 64 KiB reaches 0.94 to 0.97 of the 400 KiB default's median in an optimised build and
/// 0.66 to 0.79 in an unoptimised one, where per-wake-up cost is not hidden. Public so the throughput gate (`h2_send_buf_gate`) serves a
/// bare HTTP/2 connection with this very value.
///
/// # Examples
///
/// ```
/// assert!(shadowcat::http::conn_serve::H2_SEND_BUF_BYTES >= 16 * 1024);
/// ```
pub const H2_SEND_BUF_BYTES: usize = 64 * 1024;

// A buffer smaller than one chunk would make every chunk wait for credit it cannot be granted.
const _: () = assert!(H2_SEND_BUF_BYTES >= EGRESS_CHUNK_BYTES);

/// How many streams a client may have open on one HTTP/2 connection
/// (`http2::Builder::max_concurrent_streams`). Set explicitly so the bound on the egress stream
/// table, `max_concurrent_streams + PARSE_LAG_SLACK`, does not depend on a library default. 100 is
/// the minimum RFC 9113 section 6.5.2 recommends for SETTINGS_MAX_CONCURRENT_STREAMS, and it halves
/// the slots a stalled stream holds against hyper's default of 200.
const H2_MAX_CONCURRENT_STREAMS: u32 = 100;

/// The largest frame payload h2 accepts from a client (`http2::Builder::max_frame_size`, the
/// SETTINGS_MAX_FRAME_SIZE the connection advertises). Equals h2's own default and the minimum RFC
/// 9113 section 6.5.2 allows, and is set explicitly so the egress parser's read bound
/// (`CODEC_READ_MAX_BYTES`) does not depend on a library default.
const H2_MAX_FRAME_BYTES: u32 = 16 * 1024;

/// How many RST_STREAM frames h2 itself generates, over a connection's life, for protocol errors in
/// frames the peer sent before it ends the connection with GOAWAY(ENHANCE_YOUR_CALM)
/// (`http2::Builder::max_local_error_reset_streams`). Equals h2's own default and is set
/// explicitly, as `Some`, so a library change cannot silently move a security limit and `None`
/// (no guard) cannot be reached by accident. The count never decays. A reset hyper or this module
/// sends, a CANCEL, an implicit reset and a stall cut do not count.
const H2_MAX_LOCAL_ERROR_RESETS: usize = 1024;

/// The bound on streams h2 holds that the client has already reset and hyper has not yet accepted
/// (`http2::Builder::max_pending_accept_reset_streams`). Reaching it stops the connection reading
/// frames (backpressure) until hyper's accepts bring the count back under it; the connection is
/// never ended for the queue, so a client that cancels a page's requests in a burst, or whose stalled
/// uplink delivers thousands of HEADERS and RST_STREAM frames in one flush, is served however many
/// requests the page made. Accepting depends only on the queue, never on a handler finishing or on a
/// frame not yet read, so a stopped connection always resumes.
///
/// Each pending stream keeps its decoded request header block (at most hyper's 16 KiB header-list
/// limit) until accepted, so the bound is the connection's worst-case memory pin from this queue:
/// about 16 MiB at 1,024 (the retention per reset stream is read from h2's frame handling, not
/// measured).
///
/// The same value sets the sustained-rate limit on RST_STREAM frames the vendored h2 enforces: 16
/// times the bound per 10 s window (16,384 resets per 10 s at 1,024, an average of about 1,640 a
/// second), a burst of up to that many admitted at once. A connection exceeding it ends with
/// GOAWAY(ENHANCE_YOUR_CALM, `too_many_resets`). The rule is the HTTP/2 rapid-reset protection:
/// opening and cancelling streams never holds the concurrent-stream limit, so only the reset rate
/// bounds the server work a peer can demand. A page makes at most a few thousand requests and
/// cancels them in waves tied to user actions, so a sustained average past one reset per
/// millisecond for ten seconds is not a browser; the limit is derived from the bound, not fitted to
/// one machine, and it is stated here because hyper forwards no other reset setting.
const H2_MAX_PENDING_ACCEPT_RESETS: usize = 1024;

/// The HTTP/2 connection builder every h2 connection is served with: one place sets the stream
/// executor, timer, extended CONNECT, the send buffer, the stream, frame and error-reset limits and the
/// keep-alive pings.
fn h2_builder(limits: &ConnLimits) -> http2::Builder<StreamExec> {
    let mut builder = http2::Builder::new(StreamExec);
    builder
        .timer(TokioTimer::new())
        .enable_connect_protocol()
        .max_send_buf_size(H2_SEND_BUF_BYTES)
        .max_concurrent_streams(H2_MAX_CONCURRENT_STREAMS)
        .max_frame_size(H2_MAX_FRAME_BYTES)
        .max_local_error_reset_streams(Some(H2_MAX_LOCAL_ERROR_RESETS))
        .max_pending_accept_reset_streams(Some(H2_MAX_PENDING_ACCEPT_RESETS))
        .keep_alive_interval(Some(limits.h2_ping_interval))
        .keep_alive_timeout(limits.h2_ping_timeout);
    builder
}

/// Time bounds applied to every served connection.
///
/// # Examples
///
/// ```
/// let l = shadowcat::http::conn_serve::ConnLimits::default();
/// assert!(l.header_read >= std::time::Duration::from_secs(10));
/// ```
#[derive(Debug, Clone, Copy)]
pub struct ConnLimits {
    /// HTTP/1: longest a request head may take to arrive, and longest a kept-alive connection may
    /// wait for its next request. Chosen well above a slow mobile round trip and above the
    /// browser's own reuse pattern; the accept-time handshake deadline is separate.
    pub header_read: Duration,
    /// HTTP/2: interval between keep-alive pings on an otherwise quiet connection.
    pub h2_ping_interval: Duration,
    /// HTTP/2: how long a ping may go unacknowledged before the connection is closed.
    pub h2_ping_timeout: Duration,
    /// HTTP/2: how long a connection may have no request in flight before it is closed. A live
    /// WebSocket session counts as in flight (its [`ConnLease`]). Longer than
    /// `h2_ping_interval + h2_ping_timeout` so a healthy idle connection is ended by this bound
    /// rather than by a missed ping, and far above a browser's own request gaps on a busy table.
    pub h2_idle: Duration,
    /// How long the shutdown drain waits for connections to end before aborting the rest.
    pub drain_timeout: Duration,
    /// Maximum live connections across all peers. A chosen bound on per-connection memory and
    /// descriptors, not a measured capacity: a table needs tens of connections, and the operator
    /// raises the process descriptor limit for anything larger.
    pub max_live: usize,
    /// Maximum live connections from one [`crate::http::peer_slots::peer_key`]. A table of
    /// players behind one address holds a handful of connections each (one HTTP/2 connection, or
    /// up to about six HTTP/1.1 ones plus a WebSocket), so 256 admits a large shared NAT while
    /// stopping one host from holding the server. Trusted proxies and loopback peers are exempt
    /// (`Config::is_cap_exempt`) and stay bound by [`ConnLimits::max_live`].
    pub max_live_per_peer: u32,
    /// How long a response may make no progress before it is cut. An HTTP/2 stream whose pending
    /// chunk the peer returns no flow-control credit for is reset (its connection, other streams
    /// and tunnels are untouched); a connection whose socket accepts less than
    /// [`ConnLimits::egress_floor`] response bytes while its writer is parked is closed; a
    /// connection whose finished response still has DATA queued that its own stream does not write
    /// at the rate [`ConnLimits::egress_floor`] states is closed. Checked [`EGRESS_TICKS_PER_WINDOW`]
    /// times per window, so a cut lands between the window and the window plus one tick after the
    /// last progress. At least [`EGRESS_TICKS_PER_WINDOW`] nanoseconds: the sampler period is the
    /// window divided by that count and must be nonzero, so [`serve_router`] panics on a smaller
    /// window (the configuration admits 1 to 3600 seconds).
    pub egress_window: Duration,
    /// How many response bytes a socket must accept per [`ConnLimits::egress_window`] to count as
    /// making progress while its writer is parked. A connection whose writer has been parked for a
    /// whole window with fewer bytes accepted than this is cut: its peer has stopped reading the
    /// socket. Exactly this many is progress. On HTTP/2 only DATA frame payload counts. The same
    /// number bounds a finished response's queued tail: its own stream must write this many DATA
    /// bytes per window, or all that remains when that is less, and nothing else on the connection
    /// counts as that stream's progress.
    ///
    /// The default is one response chunk (`EGRESS_CHUNK_BYTES`, 16 KiB, one HTTP/2 frame); the field
    /// is separate so tests can vary the floor without changing the chunk. The tail rule counts DATA
    /// payload bytes the writer accepted, a byte at a time, so a floor that is no multiple of the
    /// chunk or of any frame size is judged exactly: it is not rounded to a whole frame.
    pub egress_floor: usize,
}

impl ConnLimits {
    /// The defaults with the no-progress window the configuration sets
    /// ([`crate::config::Config::egress_window`], validated to 1 to 3600 seconds when the
    /// configuration loads). The window also bounds one pending WebSocket send through
    /// `BoundedSink::for_config`, from the same method.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use shadowcat::config::Config;
    /// use shadowcat::http::conn_serve::ConnLimits;
    ///
    /// let config = Config { egress_window_secs: 90, ..Config::default() };
    /// assert_eq!(ConnLimits::for_config(&config).egress_window, Duration::from_secs(90));
    /// ```
    pub fn for_config(config: &crate::config::Config) -> Self {
        Self {
            egress_window: config.egress_window(),
            ..Self::default()
        }
    }
}

impl Default for ConnLimits {
    fn default() -> Self {
        Self {
            header_read: Duration::from_secs(30),
            h2_ping_interval: Duration::from_secs(30),
            h2_ping_timeout: Duration::from_secs(20),
            h2_idle: Duration::from_secs(60),
            drain_timeout: crate::http::SHUTDOWN_DRAIN_TIMEOUT,
            max_live: 2048,
            max_live_per_peer: 256,
            egress_window: PEER_STALL_TIMEOUT,
            egress_floor: EGRESS_CHUNK_BYTES,
        }
    }
}

/// Why [`serve_router`] returned.
///
/// # Examples
///
/// ```
/// use shadowcat::http::conn_serve::ServeEnd;
///
/// assert_ne!(ServeEnd::Shutdown, ServeEnd::SourceEnded);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServeEnd {
    /// The shutdown future resolved; every connection was drained.
    Shutdown,
    /// The [`ConnSource`] ended without a shutdown request; every connection was drained. A
    /// server whose source has died can accept nothing more, so callers treat this as a failure.
    SourceEnded,
}

/// How the connection drain ended.
///
/// # Examples
///
/// ```
/// use shadowcat::http::conn_serve::DrainReport;
///
/// let clean = DrainReport::default();
/// assert_eq!((clean.killed, clean.aborted, clean.panicked, clean.dropped), (0, 0, 0, 0));
/// assert_eq!(clean.stall_cuts.stream, 0);
/// assert_eq!(clean.stream_table_peak, 0);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DrainReport {
    /// Connections still open at the drain deadline, which were then closed by an I/O error
    /// delivered through their IO (see the `kill` module).
    pub killed: usize,
    /// Connection tasks that finished cancelled. Only the final `abort_all` of the drain cancels a
    /// connection task, so each is one that did not end within [`ABORT_WAIT`] of its IO being
    /// failed; a nonzero count is a defect. Counts connection tasks only: WebSocket session tasks
    /// are not in the serve loop's set.
    pub aborted: usize,
    /// Connection tasks that panicked, at any point of the serve loop's life.
    pub panicked: usize,
    /// Connections force-dropped by `drive` because they did not end within [`KILL_GRACE`] after
    /// their IO was failed, for any reason. A connection ends by itself when its IO errors, so a
    /// nonzero count is a defect.
    pub dropped: usize,
    /// Responses cut for making no progress, by the bound that cut them, over the serve loop's
    /// life.
    pub stall_cuts: StallCuts,
    /// The most entries any connection's table of HTTP/2 streams held at once. Bounded by the
    /// table's cap: the streams h2 admits plus the parse lag it tolerates.
    pub stream_table_peak: usize,
}

/// How many responses each egress bound cut. One field per bound; a cut increments exactly one.
///
/// # Examples
///
/// ```
/// use shadowcat::http::conn_serve::StallCuts;
///
/// assert_eq!(StallCuts::default().stream, 0);
/// assert_eq!(StallCuts::default().connection, 0);
/// assert_eq!(StallCuts::default().tracker_overflow, 0);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StallCuts {
    /// HTTP/2 streams reset because the peer returned no flow-control credit for their pending
    /// chunk for [`ConnLimits::egress_window`]. The connection stayed open.
    pub stream: usize,
    /// Connections closed because their socket accepted fewer than [`ConnLimits::egress_floor`]
    /// response bytes over [`ConnLimits::egress_window`] while their writer was parked.
    pub connection: usize,
    /// Connections closed because a finished response's own stream wrote fewer than
    /// `min(`[`ConnLimits::egress_floor`]`, bytes still queued)` DATA bytes over
    /// [`ConnLimits::egress_window`]. The stream had no handle left to reset it.
    pub ended_unflushed: usize,
    /// Connections closed because their table of HTTP/2 streams outgrew its cap, which only a
    /// parser defect or a peer outside the protocol can cause.
    pub tracker_overflow: usize,
}

/// What [`serve_router`] returns.
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use shadowcat::http::conn_serve::{serve_router, ConnLimits, ServeEnd};
///
/// let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
/// let out = serve_router(tcp, axum::Router::new(), async {}, ConnLimits::default(), || {}).await;
/// assert_eq!(out.end, ServeEnd::Shutdown);
/// assert_eq!(out.drain.aborted, 0);
/// # }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct ServeOutcome {
    /// Why the loop stopped.
    pub end: ServeEnd,
    /// When the drain bound expires: [`ConnLimits::drain_timeout`] after `on_drain` ran. Callers
    /// that wait for work the loop cannot see measure their own wait against this instant, so the
    /// whole shutdown shares one bound.
    pub drain_deadline: Instant,
    /// How the connection drain ended.
    pub drain: DrainReport,
}

/// How long the drain waits, after failing every connection's IO, before aborting a task that is
/// still running: [`KILL_GRACE`] plus `ABORT_MARGIN`, so the abort can never race the time a
/// connection is allowed to end by itself. Derived, never typed.
pub const ABORT_WAIT: Duration = KILL_GRACE.saturating_add(ABORT_MARGIN);

/// How long `drive` lets a connection whose IO has been failed end by itself before dropping it.
/// One scheduler turn per connection plus hyper's error path fit well inside it. Chosen, not
/// measured.
pub const KILL_GRACE: Duration = Duration::from_secs(2);

/// Covers the scheduling delay between the end of [`KILL_GRACE`] and the connection task's
/// completion being observed by the serve loop's join. Chosen, not measured.
const ABORT_MARGIN: Duration = Duration::from_secs(1);

/// How many sampler ticks the egress sampler takes per [`ConnLimits::egress_window`]. The ticks
/// bound how late a cut can land past the window to one tick (a window divided by this count), so
/// every reader of the sampler period, tests included, derives it from this count and not from a
/// copy of the quotient.
///
/// # Examples
///
/// ```
/// use shadowcat::http::conn_serve::EGRESS_TICKS_PER_WINDOW;
///
/// assert_eq!(EGRESS_TICKS_PER_WINDOW, 4);
/// ```
pub const EGRESS_TICKS_PER_WINDOW: u32 = 4;

/// The egress sampler's period for `window`: `window` divided by [`EGRESS_TICKS_PER_WINDOW`]. The
/// quotient is zero for a window under that many nanoseconds, which `tokio::time::interval`
/// rejects, so [`serve_router`] checks that the period is nonzero before serving.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use shadowcat::http::conn_serve::{egress_tick, EGRESS_TICKS_PER_WINDOW};
///
/// let window = Duration::from_secs(8);
/// assert_eq!(egress_tick(window) * EGRESS_TICKS_PER_WINDOW, window);
/// ```
pub fn egress_tick(window: Duration) -> Duration {
    window / EGRESS_TICKS_PER_WINDOW
}

/// Records the result of reaping one connection task: a panic is an error-level event, and a
/// cancelled join is an abort (only `drain_tasks`' `abort_all` cancels a connection task).
fn note_join(result: Result<(), JoinError>, report: &mut DrainReport) {
    if let Err(err) = result {
        if err.is_panic() {
            report.panicked += 1;
            tracing::error!("connection task panicked: {err}");
        } else if err.is_cancelled() {
            report.aborted += 1;
        }
    }
}

/// Reaps every task in `tasks`.
async fn join_all(tasks: &mut JoinSet<()>, report: &mut DrainReport) {
    while let Some(result) = tasks.join_next().await {
        note_join(result, report);
    }
}

/// Ends the connection tasks in `tasks`: waits for them until `deadline`, then calls `kill` (which
/// fails every connection's IO) and waits `abort_wait` more, then aborts whatever still runs. Each
/// step is counted in `report`: `killed` is the number open at the deadline, `aborted` the number
/// that finished cancelled.
async fn drain_tasks(
    tasks: &mut JoinSet<()>,
    deadline: Instant,
    kill: impl FnOnce(),
    abort_wait: Duration,
    report: &mut DrainReport,
) {
    if tokio::time::timeout_at(deadline, join_all(tasks, report))
        .await
        .is_ok()
    {
        return;
    }
    report.killed = tasks.len();
    tracing::warn!(
        open = report.killed,
        "connection drain timed out; closing the remaining connections"
    );
    kill();
    if tokio::time::timeout(abort_wait, join_all(tasks, report))
        .await
        .is_err()
    {
        tracing::error!(
            still_running = tasks.len(),
            "connection tasks did not end after their IO was failed; aborting them"
        );
        tasks.abort_all();
        join_all(tasks, report).await;
    }
}

/// Serves `app` on connections from `source` until `shutdown` resolves or the source ends, then
/// runs `on_drain` and drains every live connection. Each connection is told to
/// `graceful_shutdown`; the ones still open [`ConnLimits::drain_timeout`] after `on_drain` are
/// closed by an I/O error on their IO (the `kill` module), and any task still running
/// [`ABORT_WAIT`] later is aborted and reported in [`DrainReport::aborted`].
///
/// `on_drain` runs exactly once, before the drain, for both causes. It is where the caller tells
/// sessions hyper cannot see to end: an HTTP/2 connection resolves only when its streams end, and
/// a WebSocket is one of them.
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use shadowcat::http::conn_serve::{serve_router, ConnLimits, ServeEnd};
///
/// let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
/// let out = serve_router(tcp, axum::Router::new(), async {}, ConnLimits::default(), || {}).await;
/// assert_eq!(out.end, ServeEnd::Shutdown);
/// # }
/// ```
pub async fn serve_router<S>(
    mut source: S,
    app: Router,
    shutdown: impl Future<Output = ()> + Send + 'static,
    limits: ConnLimits,
    on_drain: impl FnOnce() + Send + 'static,
) -> ServeOutcome
where
    S: ConnSource,
{
    assert!(
        !egress_tick(limits.egress_window).is_zero(),
        "ConnLimits::egress_window must give a nonzero sampler period (at least {EGRESS_TICKS_PER_WINDOW} nanoseconds)"
    );
    // `signal_tx.closed()` resolves once the only receiver is dropped: by the watcher task when
    // `shutdown` resolves, or by aborting it when the source ends. Every connection task waits on
    // it, so both causes drain identically.
    let (signal_tx, signal_rx) = watch::channel(());
    let watcher = AbortOnDrop(tokio::spawn(async move {
        shutdown.await;
        drop(signal_rx);
    }));
    let slots = Arc::new(
        PeerSlots::new(limits.max_live, limits.max_live_per_peer).named("live connections"),
    );
    let kill = KillSignal::new();
    let dropped = Arc::new(AtomicUsize::new(0));
    let stalls = Arc::new(StallCounters::default());
    let mut tasks = JoinSet::new();
    let mut drain = DrainReport::default();
    // When accepting resumes after an accept error. Held here, not in the accept future, because
    // that future is dropped on every reaped connection task.
    let mut resume_at: Option<Instant> = None;

    let end = loop {
        let accepted = tokio::select! {
            // The branch expression is evaluated even while the precondition is false (only the
            // resulting future goes unpolled), so it must be valid when `resume_at` is `None`.
            a = source.accept(), if resume_at.is_none() => a,
            _ = tokio::time::sleep_until(resume_at.unwrap_or_else(Instant::now)),
                if resume_at.is_some() => {
                resume_at = None;
                continue;
            }
            _ = signal_tx.closed() => break ServeEnd::Shutdown,
            // Reaps finished connection tasks so the set holds only live ones.
            Some(result) = tasks.join_next(), if !tasks.is_empty() => {
                note_join(result, &mut drain);
                continue;
            }
        };
        let accepted = match accepted {
            Ok(accepted) => accepted,
            Err(e) => {
                resume_at = accept_error_pause(&e).map(|pause| Instant::now() + pause);
                continue;
            }
        };
        let Some((io, meta)) = accepted else {
            tracing::error!("connection source ended; no further connections will be accepted");
            break ServeEnd::SourceEnded;
        };
        let Some(slot) = slots.admit(meta.peer.ip(), meta.cap_exempt) else {
            continue;
        };
        spawn_connection(
            &mut tasks,
            io,
            meta,
            app.clone(),
            LiveState::new(slot, stalls.clone()),
            Ending {
                signal_tx: &signal_tx,
                kill: &kill,
                dropped: &dropped,
            },
            limits,
        );
    };

    on_drain();
    let drain_deadline = Instant::now() + limits.drain_timeout;
    // Dropping the watcher aborts it, which drops its receiver: that is what tells live
    // connections to drain when the loop ended for any reason other than `shutdown` resolving.
    drop(watcher);
    drop(source);
    drain_tasks(
        &mut tasks,
        drain_deadline,
        || kill.fire(),
        ABORT_WAIT,
        &mut drain,
    )
    .await;
    drain.dropped = dropped.load(Ordering::Relaxed);
    drain.stall_cuts = stalls.snapshot();
    drain.stream_table_peak = stalls.table_peak();
    tracing::info!(?end, ?drain, "connection serve loop ended");
    ServeOutcome {
        end,
        drain_deadline,
        drain,
    }
}

/// Aborts its task when dropped, so the shutdown watcher cannot outlive `serve_router` on any
/// path, including the future being dropped mid-loop.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// What `drive` watches besides the connection itself: the kill switch it answers, and the state
/// its sampler scans.
struct Watch {
    /// Fired by the drain deadline or a per-connection cut; every IO method of the connection
    /// answers it with an error.
    kill: Arc<ConnKill>,
    /// The connection's liveness state, whose stream markers the sampler scans.
    live: Arc<LiveState>,
    /// [`ConnLimits::egress_window`].
    egress_window: Duration,
    /// [`ConnLimits::egress_floor`].
    egress_floor: usize,
}

/// Drives one hyper connection to completion, calling `graceful` once when `signal_tx` closes or
/// `idle` resolves.
///
/// The one force-end path: when the kill switch fires the connection's IO already fails (the `kill`
/// module), so the connection normally ends by itself; if it has not ended [`KILL_GRACE`] later it
/// is dropped and counted in `dropped`. Every reason takes this path, so the drain deadline and a
/// per-connection cut end a connection identically.
///
/// The egress sampler is an arm of this one `select!`: every [`egress_tick`] it observes the
/// connection's writer and, when the connection unit finds no progress, fires the kill switch with
/// [`KillReason::Stall`]; otherwise, unless the writer is parked, it scans the table of ended
/// responses (a connection cut) and then the stream markers (a stream cut). A parked writer means
/// the socket, not the peer's credit, is what the response is waiting on, and that case belongs to
/// the connection unit: a stream cut could not even deliver its `RST_STREAM`. The loop then re-polls `conn`, which retries a
/// parked write (hyper's HTTP/1 and HTTP/2 flush paths both retry on a poll), so a parked writer
/// is re-observed each tick with no other wake source.
async fn drive<C>(
    conn: C,
    signal_tx: watch::Sender<()>,
    idle: impl Future<Output = ()>,
    graceful: impl Fn(Pin<&mut C>),
    watch: Watch,
    dropped: &AtomicUsize,
) where
    C: Future<Output = Result<(), hyper::Error>>,
{
    let mut conn = pin!(conn);
    let mut signal_closed = pin!(signal_tx.closed().fuse());
    let mut idle = pin!(idle.fuse());
    let mut killed = pin!(watch.kill.fired());
    let tick = egress_tick(watch.egress_window);
    let mut ticker = tokio::time::interval(tick);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut sampler = Sampler::new(watch.egress_window, tick, watch.egress_floor as u64);
    let egress = watch.live.egress();
    loop {
        tokio::select! {
            result = conn.as_mut() => {
                if let Err(err) = result {
                    tracing::trace!("failed to serve connection: {err:#}");
                }
                return;
            }
            reason = &mut killed => {
                tokio::select! {
                    result = conn.as_mut() => {
                        if let Err(err) = result {
                            tracing::trace!("failed to serve connection: {err:#}");
                        }
                    }
                    () = tokio::time::sleep(KILL_GRACE) => {
                        tracing::error!(
                            ?reason,
                            "connection dropped: it did not end after its IO failed"
                        );
                        dropped.fetch_add(1, Ordering::Relaxed);
                    }
                }
                return;
            }
            _ = &mut signal_closed => graceful(conn.as_mut()),
            _ = &mut idle => graceful(conn.as_mut()),
            _ = ticker.tick() => {
                let now = Instant::now();
                match sampler.observe(now, egress.accepted(), egress.parked()) {
                    Some(stall) => {
                        // Counted only when this call ended the connection: a drain deadline that
                        // fired first is not a stall.
                        if watch.kill.fire(KillReason::Stall) {
                            tracing::warn!(
                                unit = "connection",
                                span_ms = u64::try_from(stall.span.as_millis()).unwrap_or(u64::MAX),
                                bytes_accepted = stall.accepted,
                                "connection cut: its socket accepted almost nothing while its writer was parked"
                            );
                            watch.live.record_stall(StallUnit::Connection);
                        }
                    }
                    None if !egress.parked() => {
                        // The table first: an ended response's tail can only be cut as the whole
                        // connection, and a connection already cut has no streams left to scan.
                        let ended = egress.cut_ended_unflushed(
                            &watch.kill,
                            now,
                            watch.egress_window,
                            watch.egress_floor as u64,
                        );
                        if !ended {
                            watch.live.cut_stalled_streams(now, watch.egress_window);
                        }
                    }
                    None => {}
                }
            }
        }
    }
}

/// Length of the HTTP/2 client connection preface (RFC 9113 section 3.4), the first octets of
/// every HTTP/2 connection.
const H2_PREFACE_LEN: usize = 24;

/// An IO wrapper that yields `prefix` before reading from `inner`; writes go straight through.
/// Every method first asks `kill` (see the `kill` module), so a firing reaches a connection parked
/// in any of them. Every write-side method reports its outcome to `writes`, which is where the
/// connection unit reads whether the writer is parked and how many response bytes it took.
struct Replay<I> {
    /// The octets already read from `inner`, served first.
    prefix: Vec<u8>,
    /// How many `prefix` octets have been served.
    pos: usize,
    /// The underlying connection.
    inner: I,
    /// Fails the IO once the connection is told to end.
    kill: Arc<ConnKill>,
    /// Counts what the writer accepted and tracks whether it is parked: the connection unit's
    /// evidence. On HTTP/2 it also keeps the stream table in step with the frames written.
    writes: WriteCounter,
    /// On HTTP/2, walks what the codec reads: the stream table's creator and its peer-reset
    /// removal.
    reads: ReadObserver,
}

impl<I> Replay<I> {
    /// IO over `inner` that first serves `prefix`, fails once `kill` fires, and reports its
    /// writes to `writes`.
    fn new(prefix: Vec<u8>, inner: I, kill: Arc<ConnKill>, writes: WriteCounter) -> Self {
        let reads = writes.read_observer();
        Self {
            prefix,
            pos: 0,
            inner,
            kill,
            writes,
            reads,
        }
    }
}

impl<I: AsyncRead + Unpin> AsyncRead for Replay<I> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        this.kill.check(cx)?;
        if this.pos < this.prefix.len() {
            let n = (this.prefix.len() - this.pos).min(buf.remaining());
            buf.put_slice(&this.prefix[this.pos..this.pos + n]);
            this.reads
                .after_read(&this.prefix[this.pos..this.pos + n], &this.kill);
            this.pos += n;
            return Poll::Ready(Ok(()));
        }
        let before = buf.filled().len();
        let result = Pin::new(&mut this.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = result {
            this.reads.after_read(&buf.filled()[before..], &this.kill);
        }
        result
    }
}

impl<I: AsyncWrite + Unpin> AsyncWrite for Replay<I> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        this.kill.check(cx)?;
        let result = Pin::new(&mut this.inner).poll_write(cx, buf);
        this.writes.after_write(&result, &[io::IoSlice::new(buf)]);
        result
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        this.kill.check(cx)?;
        let result = Pin::new(&mut this.inner).poll_write_vectored(cx, bufs);
        this.writes.after_write(&result, bufs);
        result
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        this.kill.check(cx)?;
        let result = Pin::new(&mut this.inner).poll_flush(cx);
        this.writes.after_flush(&result);
        result
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        this.kill.check(cx)?;
        let result = Pin::new(&mut this.inner).poll_shutdown(cx);
        this.writes.after_flush(&result);
        result
    }
}

/// Reads the HTTP/2 preface under `limit` and returns the IO with those octets replayed, or
/// fails early when `interrupted` resolves (a drain in progress needs no new connection).
///
/// Why this exists: hyper arms no timer during the HTTP/2 server handshake, so a peer that
/// negotiates `h2` and then sends nothing would hold the connection forever once the accept-time
/// permit is released. Reading the preface here bounds that phase; once it is in, hyper's
/// keep-alive pings bound the rest. An error (EOF, reset, timeout, interruption) closes the
/// connection.
async fn read_h2_preface<I: AsyncRead + Unpin>(
    mut io: I,
    limit: Duration,
    interrupted: impl Future<Output = ()>,
    kill: Arc<ConnKill>,
    writes: WriteCounter,
) -> io::Result<Replay<I>> {
    let mut prefix = vec![0u8; H2_PREFACE_LEN];
    let mut filled = 0;
    let read = tokio::time::timeout(limit, async {
        while filled < H2_PREFACE_LEN {
            let n = io.read(&mut prefix[filled..]).await?;
            if n == 0 {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
            }
            filled += n;
        }
        Ok(())
    });
    tokio::select! {
        r = read => r.map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??,
        () = interrupted => return Err(io::Error::from(io::ErrorKind::Interrupted)),
    }
    Ok(Replay::new(prefix, io, kill, writes))
}

/// What the serve loop ends a connection with: the graceful drain signal, the deadline's kill and
/// the count of connections `drive` had to drop.
#[derive(Clone, Copy)]
struct Ending<'a> {
    /// Closes when the drain begins; every connection task waits on it.
    signal_tx: &'a watch::Sender<()>,
    /// Fires at the drain deadline; every connection's IO answers it with an error.
    kill: &'a KillSignal,
    /// Incremented by `drive` for each connection it force-drops.
    dropped: &'a Arc<AtomicUsize>,
}

/// Spawns one connection onto `tasks`; the task holds `live` (its cap slot) until it ends.
///
/// The HTTP version comes from ALPN and each version's hyper builder is used directly, because
/// hyper-util's auto builder ignores `http1_only`/`http2_only` under
/// `serve_connection_with_upgrades` and always reads a 24-byte preface first, with no timer.
fn spawn_connection<I: ServeIo>(
    tasks: &mut JoinSet<()>,
    io: I,
    meta: ConnMeta,
    app: Router,
    live: Arc<LiveState>,
    ending: Ending<'_>,
    limits: ConnLimits,
) {
    let h2 = io.is_h2();
    let kill = ending.kill.connection();
    let watch = Watch {
        kill: kill.clone(),
        live: live.clone(),
        egress_window: limits.egress_window,
        egress_floor: limits.egress_floor,
    };
    let writes = WriteCounter::new(live.egress(), h2);
    let dropped = ending.dropped.clone();
    let service = TowerToHyperService::new(
        LeaseService::new(Extension(ConnectInfo(meta)).layer(app), live.clone())
            .map_request(|req: Request<Incoming>| req.map(Body::new)),
    );
    let signal_tx = ending.signal_tx.clone();
    tasks.spawn(async move {
        if h2 {
            let preface =
                read_h2_preface(io, limits.header_read, signal_tx.closed(), kill, writes).await;
            match preface {
                Ok(io) => {
                    // Extended CONNECT carries WebSockets over HTTP/2 (RFC 8441).
                    let conn = h2_builder(&limits).serve_connection(TokioIo::new(io), service);
                    let idle = idle_for(live.watch_streams(), limits.h2_idle);
                    drive(
                        conn,
                        signal_tx,
                        idle,
                        |c| c.graceful_shutdown(),
                        watch,
                        &dropped,
                    )
                    .await;
                }
                Err(err) => tracing::trace!("h2 connection closed before its preface: {err}"),
            }
        } else {
            let io = Replay::new(Vec::new(), io, kill, writes);
            let conn = http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(limits.header_read)
                .serve_connection(TokioIo::new(io), service)
                .with_upgrades();
            drive(
                conn,
                signal_tx,
                std::future::pending(),
                |c| c.graceful_shutdown(),
                watch,
                &dropped,
            )
            .await;
        }
        drop(live);
    });
}

#[cfg(test)]
mod reset_guard_tests;
#[cfg(test)]
mod stream_table_tests;
#[cfg(test)]
mod tests;
