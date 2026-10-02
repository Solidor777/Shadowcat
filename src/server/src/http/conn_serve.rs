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
//!    [`super::scheme::into_service`] (for `axum::serve` transports) and nowhere else.
//! 3. Graceful shutdown mirrors `axum::serve`: a watch channel tells every connection task to call
//!    `graceful_shutdown`, and the loop returns once every task has ended. A source that ends
//!    drains the same way and the loop returns [`ServeEnd::SourceEnded`]. Both causes first run
//!    the caller's `on_drain` hook, which is where sessions hyper cannot see (WebSockets) are told
//!    to close: an HTTP/2 connection resolves only when its streams end, and a WebSocket is a
//!    stream, so draining without telling the sessions would wait on them. The drain itself is
//!    bounded by [`ConnLimits::drain_timeout`]: connections still open past it have their IO
//!    failed (the `kill` module) and `drive` drops one that has not ended [`KILL_GRACE`] later
//!    ([`DrainReport::dropped`]); a task still running [`ABORT_WAIT`] after the IO failed (one not
//!    in `drive`, or one `drive` could not run) is aborted and reported in
//!    [`DrainReport::aborted`]. A connection task ends when its IO errors, so each is a defect.
//! 4. Every live connection is bounded, not only the half-open ones. [`ConnLimits::max_live`] and
//!    [`ConnLimits::max_live_per_peer`] cap connections from hand-off until the connection task
//!    AND every [`ConnLease`] taken from its requests have ended, keyed and exempted exactly like
//!    the accept-time caps ([`crate::http::peer_slots::PeerSlots`], `ConnMeta::cap_exempt`); a
//!    connection over a cap is closed unread. [`ConnLimits::h2_idle`] closes an HTTP/2 connection
//!    that has had no request in flight for that long. A lease held by a WebSocket session is
//!    released when the session ends, and a session whose peer has stopped reading ends at the
//!    per-send stall bound ([`crate::ws::bounded_sink::SEND_STALL_TIMEOUT`]), so a lease cannot
//!    pin a slot for longer than that bound past the peer's last read.
//! 5. The pause after an accept error is decided in [`serve_router`], never inside a
//!    [`ConnSource::accept`] future: the loop drops that future on every reaped connection task, so
//!    a pause held inside it would be cut short by the next reap. A source returns the error and
//!    [`accept_error_pause`] is the one policy every caller (this loop and `SniffListener`'s
//!    acceptor task) consults.
//!
//! Timeout semantics measured against hyper 1.x: the HTTP/1 header-read timeout is armed on every
//! wait for a request head, so it bounds both a slow head and an idle keep-alive gap; it does not
//! apply after a WebSocket upgrade, to request bodies, or to a running handler. HTTP/2 keep-alive
//! closes a connection whose peer stops acknowledging pings, and hyper has no idle timeout, so
//! [`ConnLimits::h2_idle`] is enforced here from the in-flight count (see the `live` module).

mod kill;
mod live;
mod pruned;

use std::future::Future;
use std::io;
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
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::{JoinError, JoinSet};
use tokio::time::Instant;
use tower::{Layer, ServiceExt};

use self::kill::{ConnKill, KillSignal};
pub use self::live::ConnLease;
use self::live::{idle_for, LeaseService, LiveState};
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
    /// instead of sleeping on them: the pause is the caller's, see [`accept_error_pause`].
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

impl ConnSource for TcpListener {
    type Io = TcpStream;

    async fn accept(&mut self) -> io::Result<Option<(TcpStream, ConnMeta)>> {
        let (sock, peer) = TcpListener::accept(self).await?;
        Ok(Some((sock, ConnMeta::plain(peer))))
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
    /// stopping one host from holding the server. Trusted proxies are exempt.
    pub max_live_per_peer: u32,
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
            LiveState::new(slot),
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

/// Drives one hyper connection to completion, calling `graceful` once when `signal_tx` closes or
/// `idle` resolves.
///
/// The one force-end path: when `kill` fires the connection's IO already fails (the `kill`
/// module), so the connection normally ends by itself; if it has not ended [`KILL_GRACE`] later it
/// is dropped and counted in `dropped`. Every reason takes this path, so the drain deadline and a
/// per-connection cut end a connection identically.
async fn drive<C>(
    conn: C,
    signal_tx: watch::Sender<()>,
    idle: impl Future<Output = ()>,
    graceful: impl Fn(Pin<&mut C>),
    kill: Arc<ConnKill>,
    dropped: &AtomicUsize,
) where
    C: Future<Output = Result<(), hyper::Error>>,
{
    let mut conn = pin!(conn);
    let mut signal_closed = pin!(signal_tx.closed().fuse());
    let mut idle = pin!(idle.fuse());
    let mut killed = pin!(kill.fired());
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
        }
    }
}

/// Length of the HTTP/2 client connection preface (RFC 9113 section 3.4), the first octets of
/// every HTTP/2 connection.
const H2_PREFACE_LEN: usize = 24;

/// An IO wrapper that yields `prefix` before reading from `inner`; writes go straight through.
/// Every method first asks `kill` (see the `kill` module), so a firing reaches a connection parked
/// in any of them.
struct Replay<I> {
    /// The octets already read from `inner`, served first.
    prefix: Vec<u8>,
    /// How many `prefix` octets have been served.
    pos: usize,
    /// The underlying connection.
    inner: I,
    /// Fails the IO once the connection is told to end.
    kill: Arc<ConnKill>,
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
            this.pos += n;
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut this.inner).poll_read(cx, buf)
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
        Pin::new(&mut this.inner).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        this.kill.check(cx)?;
        Pin::new(&mut this.inner).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        this.kill.check(cx)?;
        Pin::new(&mut this.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        this.kill.check(cx)?;
        Pin::new(&mut this.inner).poll_shutdown(cx)
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
    Ok(Replay {
        prefix,
        pos: 0,
        inner: io,
        kill,
    })
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
    let drive_kill = kill.clone();
    let dropped = ending.dropped.clone();
    let service = TowerToHyperService::new(
        LeaseService::new(Extension(ConnectInfo(meta)).layer(app), live.clone())
            .map_request(|req: Request<Incoming>| req.map(Body::new)),
    );
    let signal_tx = ending.signal_tx.clone();
    tasks.spawn(async move {
        if h2 {
            let preface = read_h2_preface(io, limits.header_read, signal_tx.closed(), kill).await;
            match preface {
                Ok(io) => {
                    // Extended CONNECT carries WebSockets over HTTP/2 (RFC 8441).
                    let conn = http2::Builder::new(TokioExecutor::new())
                        .timer(TokioTimer::new())
                        .enable_connect_protocol()
                        .keep_alive_interval(Some(limits.h2_ping_interval))
                        .keep_alive_timeout(limits.h2_ping_timeout)
                        .serve_connection(TokioIo::new(io), service);
                    let idle = idle_for(live.watch_streams(), limits.h2_idle);
                    drive(
                        conn,
                        signal_tx,
                        idle,
                        |c| c.graceful_shutdown(),
                        drive_kill,
                        &dropped,
                    )
                    .await;
                }
                Err(err) => tracing::trace!("h2 connection closed before its preface: {err}"),
            }
        } else {
            let io = Replay {
                prefix: Vec::new(),
                pos: 0,
                inner: io,
                kill,
            };
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
                drive_kill,
                &dropped,
            )
            .await;
        }
        drop(live);
    });
}

#[cfg(test)]
mod tests;
