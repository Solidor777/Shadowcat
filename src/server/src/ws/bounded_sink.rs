//! A `Sink` adapter that bounds how long one send may stay pending.
//!
//! WHY: a peer that has stopped reading (a zero flow-control window, a full socket buffer) leaves
//! `poll_ready` / `poll_flush` of the WebSocket sink pending forever, and a session blocked inside
//! a send never reaches its heartbeat or shutdown arms, so `PING_TIMEOUT` never fires and the
//! session task, its `ConnectionGuard` and its `ConnLease` (the connection's cap slot) live until
//! the process exits. Wrapping the sink once, through `WsState::bounded_sink` (the one production
//! constructor), bounds every send (Welcome, broadcast frames, pre-join errors, Close) with one
//! rule.
//!
//! The bound is on the TIME of one pending flush, not on progress: a frame of F bytes needs at
//! least 8F / `stall_timeout` bits per second to complete (the largest frame,
//! `WS_MAX_MESSAGE_BYTES`, about 1.7 Mbps at the 60 s default; ordinary frames under 10 kbps). A
//! progress-aware bound is not possible through `axum::extract::ws::WebSocket`, which exposes no
//! byte counter; a WebSocket tunnelled inside an HTTP/2 connection has no other bound.
//!
//! INVARIANTS:
//! 1. A send that has been pending for `stall_timeout` fails with [`BoundedSendError::Stalled`];
//!    callers treat any send error as the end of the session.
//! 2. Once the shutdown signal is set, a pending send is given at most [`CLOSE_SEND_TIMEOUT`]
//!    from the moment the signal is observed (or from its start, if later): shutdown must not wait
//!    a whole `stall_timeout` for a peer that is not reading.
//! 3. The timer runs only while a poll returned `Pending`; progress (any `Ready`) disarms it.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::Sink;
use tokio::sync::watch;
use tokio::time::{Instant, Sleep};

/// Longest a Close frame may take to send. A peer that has stopped reading would otherwise hold
/// the session task, and with it `WsState::wait_for_drain`, until the drain bound expires.
pub const CLOSE_SEND_TIMEOUT: Duration = Duration::from_secs(2);

/// Why a bounded send failed.
///
/// # Examples
///
/// ```
/// use shadowcat::ws::bounded_sink::BoundedSendError;
///
/// let e: BoundedSendError<()> = BoundedSendError::Stalled;
/// assert!(matches!(e, BoundedSendError::Stalled));
/// ```
#[derive(Debug)]
pub enum BoundedSendError<E> {
    /// The peer did not accept the frame within the bound.
    Stalled,
    /// The wrapped sink failed.
    Inner(E),
}

/// One send counted into a pending-sends gauge: increments on creation, decrements on drop, so
/// every way a `Stall` ends (the send completing, failing, or the sink dropping) leaves the gauge
/// right.
struct PendingSend {
    /// The gauge this send is counted in.
    gauge: Arc<AtomicUsize>,
}

impl PendingSend {
    /// Counts one more pending send into `gauge`.
    fn new(gauge: Arc<AtomicUsize>) -> Self {
        gauge.fetch_add(1, Ordering::AcqRel);
        Self { gauge }
    }
}

impl Drop for PendingSend {
    fn drop(&mut self) {
        self.gauge.fetch_sub(1, Ordering::AcqRel);
    }
}

/// An armed stall timer.
struct Stall {
    /// The send's slot in the pending-sends gauge, when the sink has one.
    _pending: Option<PendingSend>,
    /// Fires when the pending send has outlived its bound.
    timer: Pin<Box<Sleep>>,
    /// Resolves once the shutdown signal is set.
    shutdown: Pin<Box<dyn Future<Output = ()> + Send>>,
    /// Whether the deadline has already been shortened to the Close bound.
    shortened: bool,
}

/// Wraps a sink so that no send stays pending longer than its bound.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use shadowcat::ws::bounded_sink::BoundedSink;
///
/// let (_tx, rx) = tokio::sync::watch::channel(false);
/// let sink = BoundedSink::new(futures_util::sink::drain::<u8>(), rx, Duration::from_secs(5));
/// drop(sink);
/// ```
pub struct BoundedSink<S> {
    /// The wrapped sink.
    inner: S,
    /// The server's shutdown signal.
    shutdown: watch::Receiver<bool>,
    /// Bound on one pending send.
    stall_timeout: Duration,
    /// Armed while a poll is pending.
    stall: Option<Stall>,
    /// Counts this sink's pending send, when set.
    pending_gauge: Option<Arc<AtomicUsize>>,
}

impl<S> BoundedSink<S> {
    /// Wraps `inner`; `shutdown` shortens pending sends to [`CLOSE_SEND_TIMEOUT`].
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use shadowcat::ws::bounded_sink::BoundedSink;
    ///
    /// let (_tx, rx) = tokio::sync::watch::channel(false);
    /// let _sink = BoundedSink::new(futures_util::sink::drain::<u8>(), rx, Duration::from_secs(1));
    /// ```
    pub fn new(inner: S, shutdown: watch::Receiver<bool>, stall_timeout: Duration) -> Self {
        Self {
            inner,
            shutdown,
            stall_timeout,
            stall: None,
            pending_gauge: None,
        }
    }

    /// Wraps `inner` with the window the configuration sets for every stalled-peer bound
    /// (`Config::egress_window`), the same value `ConnLimits::for_config` gives the response
    /// bounds, so one WebSocket send and one response cannot be tuned apart. Chosen, not measured:
    /// the default is above any single frame's transfer time on a slow mobile link (a frame is at
    /// most the wire message cap) and below `PING_TIMEOUT` (90 s), so a stalled peer is dropped
    /// before the heartbeat would be.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::config::Config;
    /// use shadowcat::ws::bounded_sink::BoundedSink;
    ///
    /// let (_tx, rx) = tokio::sync::watch::channel(false);
    /// let _sink = BoundedSink::for_config(futures_util::sink::drain::<u8>(), rx, &Config::default());
    /// ```
    pub fn for_config(
        inner: S,
        shutdown: watch::Receiver<bool>,
        config: &crate::config::Config,
    ) -> Self {
        Self::new(inner, shutdown, config.egress_window())
    }

    /// Counts this sink's pending send into `gauge` for as long as it is pending.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::atomic::AtomicUsize;
    /// use std::sync::Arc;
    /// use std::time::Duration;
    /// use shadowcat::ws::bounded_sink::BoundedSink;
    ///
    /// let (_tx, rx) = tokio::sync::watch::channel(false);
    /// let gauge = Arc::new(AtomicUsize::new(0));
    /// let _sink = BoundedSink::new(futures_util::sink::drain::<u8>(), rx, Duration::from_secs(1))
    ///     .with_pending_gauge(gauge.clone());
    /// assert_eq!(gauge.load(std::sync::atomic::Ordering::Acquire), 0);
    /// ```
    pub fn with_pending_gauge(mut self, gauge: Arc<AtomicUsize>) -> Self {
        self.pending_gauge = Some(gauge);
        self
    }

    /// The bound on one pending send.
    #[cfg(test)]
    pub(crate) fn stall_timeout(&self) -> Duration {
        self.stall_timeout
    }

    /// Applies the stall bound to the result of a poll of the wrapped sink.
    fn guard<T, E>(
        &mut self,
        cx: &mut Context<'_>,
        polled: Poll<Result<T, E>>,
    ) -> Poll<Result<T, BoundedSendError<E>>> {
        match polled {
            Poll::Ready(result) => {
                self.stall = None;
                Poll::Ready(result.map_err(BoundedSendError::Inner))
            }
            Poll::Pending => {
                let stall_timeout = self.stall_timeout;
                let mut watcher = self.shutdown.clone();
                let pending = self.pending_gauge.clone();
                let stall = self.stall.get_or_insert_with(|| Stall {
                    _pending: pending.map(PendingSend::new),
                    timer: Box::pin(tokio::time::sleep(stall_timeout)),
                    shutdown: Box::pin(async move {
                        loop {
                            if *watcher.borrow_and_update() {
                                return;
                            }
                            if watcher.changed().await.is_err() {
                                return;
                            }
                        }
                    }),
                    shortened: false,
                });
                if !stall.shortened && stall.shutdown.as_mut().poll(cx).is_ready() {
                    stall.shortened = true;
                    let shortened = Instant::now() + CLOSE_SEND_TIMEOUT;
                    if shortened < stall.timer.deadline() {
                        stall.timer.as_mut().reset(shortened);
                    }
                }
                if stall.timer.as_mut().poll(cx).is_ready() {
                    self.stall = None;
                    return Poll::Ready(Err(BoundedSendError::Stalled));
                }
                Poll::Pending
            }
        }
    }
}

impl<S, T> Sink<T> for BoundedSink<S>
where
    S: Sink<T> + Unpin,
{
    type Error = BoundedSendError<S::Error>;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = &mut *self;
        let polled = Pin::new(&mut this.inner).poll_ready(cx);
        this.guard(cx, polled)
    }

    fn start_send(mut self: Pin<&mut Self>, item: T) -> Result<(), Self::Error> {
        Pin::new(&mut self.inner)
            .start_send(item)
            .map_err(BoundedSendError::Inner)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = &mut *self;
        let polled = Pin::new(&mut this.inner).poll_flush(cx);
        this.guard(cx, polled)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = &mut *self;
        let polled = Pin::new(&mut this.inner).poll_close(cx);
        this.guard(cx, polled)
    }
}

#[cfg(test)]
mod tests;
