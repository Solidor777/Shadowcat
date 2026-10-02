//! Per-connection liveness: the connection's cap slot, the count of in-flight streams the
//! HTTP/2 idle bound reads, and the stream markers the egress stream unit scans.
//!
//! WHY a lease seam exists: hyper runs an HTTP/2 extended-CONNECT tunnel (an RFC 8441 WebSocket)
//! as a detached upgrade task, so once the handler returns its response no hyper-visible future
//! represents the tunnel, and a request counter alone would call a connection carrying a live
//! WebSocket idle. A handler that detaches long-lived work from its request takes the
//! [`ConnLease`] from the request extensions and moves it into that work; the connection then
//! counts as busy, and keeps its cap slot, for as long as the lease lives.
//!
//! INVARIANT: [`LeaseService`] inserts a lease into every request's extensions and ties one to the
//! response body, so a request is in flight from its arrival until its response body is dropped
//! (a slow download is in flight; a finished response is not).

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::Request;
use axum::response::Response;
use http_body::{Frame, SizeHint};
use tokio::sync::watch;
use tokio::time::Instant;
use tower::Service;

use super::egress::{StallCounters, StallUnit};
use super::pruned::Pruned;
use super::stream_exec::{current_cut, StreamCut};
use super::EGRESS_FLOOR_BYTES;
use crate::http::peer_slots::Slot;

/// What a connection holds for its whole life: its cap slot, the in-flight stream count, the
/// stream markers and the serve loop's cut counters.
pub(super) struct LiveState {
    /// The connection's live-cap slot, released when the state (the connection task and every
    /// lease) is dropped.
    _slot: Slot,
    /// Number of leases currently held; the HTTP/2 idle bound watches it.
    streams: watch::Sender<usize>,
    /// The response streams the stream unit watches, as weak references so a finished response's
    /// marker is freed with its body. Pruned on insert, so its length is bounded by the live
    /// streams and not by the number ever served.
    markers: Mutex<Pruned<StreamMarker>>,
    /// The serve loop's cut counters, shared by every connection.
    stalls: Arc<StallCounters>,
}

/// Locks `m`, ignoring poison: the state it guards is always consistent between statements.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl LiveState {
    /// State for a connection that holds `slot` and counts its cuts in `stalls`, with no stream in
    /// flight.
    pub(super) fn new(slot: Slot, stalls: Arc<StallCounters>) -> Arc<Self> {
        Arc::new(Self {
            _slot: slot,
            streams: watch::channel(0).0,
            markers: Mutex::new(Pruned::new()),
            stalls,
        })
    }

    /// Counts one more stream in flight until the returned lease (and its clones) drop.
    pub(super) fn lease(self: &Arc<Self>) -> ConnLease {
        self.streams.send_modify(|n| *n += 1);
        ConnLease {
            _inner: Arc::new(LeaseInner { live: self.clone() }),
        }
    }

    /// A receiver of the in-flight stream count.
    pub(super) fn watch_streams(&self) -> watch::Receiver<usize> {
        self.streams.subscribe()
    }

    /// Registers a response stream whose task can be ended through `cut`.
    fn register_marker(&self, cut: StreamCut) -> Arc<StreamMarker> {
        let marker = Arc::new(StreamMarker {
            delivered_at: Mutex::new(None),
            cut,
        });
        lock(&self.markers).push(Arc::downgrade(&marker));
        marker
    }

    /// Cuts every stream whose last delivered chunk is at least `window` old and has not been
    /// asked for again, counting each as one [`StallUnit::Stream`] cut. Returns how many it cut.
    ///
    /// A marker's age starts when its body hands a data frame to hyper and stops when hyper polls
    /// the body again, which it does only once the peer has granted send credit for that frame. So
    /// the age is the time the stream's own pending chunk has waited for credit: another stream's
    /// DATA, a tunnel's DATA, a PING or a SETTINGS acknowledgement never reset it.
    pub(super) fn cut_stalled_streams(&self, now: Instant, window: Duration) -> usize {
        let live = lock(&self.markers).live();
        let mut cut = 0;
        for marker in live {
            let Some(age) = marker.take_if_older(now, window) else {
                continue;
            };
            tracing::warn!(
                unit = "stream",
                age_ms = u64::try_from(age.as_millis()).unwrap_or(u64::MAX),
                "response stream cut: the peer returned no flow-control credit for a pending chunk"
            );
            marker.cut.fire();
            self.stalls.record_stall(StallUnit::Stream);
            cut += 1;
        }
        cut
    }

    /// The cut counts of the serve loop this connection belongs to.
    #[cfg(test)]
    pub(super) fn stall_counts(&self) -> super::StallCuts {
        self.stalls.snapshot()
    }

    /// How many markers the registry holds, live and dead.
    #[cfg(test)]
    fn marker_registry_len(&self) -> usize {
        lock(&self.markers).len()
    }

    /// The markers whose bodies are alive.
    #[cfg(test)]
    fn live_markers(&self) -> Vec<Arc<StreamMarker>> {
        lock(&self.markers).live()
    }
}

/// One HTTP/2 response stream under the stream unit's watch.
struct StreamMarker {
    /// When the body last handed hyper a data frame that hyper has not yet asked past; `None`
    /// while hyper is polling the body (the producer is on the clock, not the peer) or nothing is
    /// pending.
    delivered_at: Mutex<Option<Instant>>,
    /// Ends the stream's task.
    cut: StreamCut,
}

impl StreamMarker {
    /// Records that a data frame was just handed to hyper.
    fn delivered(&self) {
        *lock(&self.delivered_at) = Some(Instant::now());
    }

    /// Records that nothing is pending on the peer.
    fn cleared(&self) {
        *lock(&self.delivered_at) = None;
    }

    /// Clears the mark and returns its age when it is at least `window` old at `now`, so one
    /// stall is reported once.
    fn take_if_older(&self, now: Instant, window: Duration) -> Option<Duration> {
        let mut at = lock(&self.delivered_at);
        let age = now.saturating_duration_since((*at)?);
        if age < window {
            return None;
        }
        *at = None;
        Some(age)
    }

    /// When the pending frame was delivered.
    #[cfg(test)]
    fn delivered_at(&self) -> Option<Instant> {
        *lock(&self.delivered_at)
    }
}

/// One in-flight stream's claim on its connection.
struct LeaseInner {
    /// The connection the stream belongs to.
    live: Arc<LiveState>,
}

impl Drop for LeaseInner {
    fn drop(&mut self) {
        self.live.streams.send_modify(|n| *n = n.saturating_sub(1));
    }
}

/// A claim that keeps one stream counted as in flight, and the connection's cap slot held, for
/// as long as any clone lives. [`LeaseService`] puts one in every request's extensions; a handler
/// that detaches long-lived work (a WebSocket session) moves a clone into that work.
///
/// # Examples
///
/// ```
/// use axum::extract::Request;
/// use shadowcat::http::conn_serve::ConnLease;
///
/// // Absent when the router is served without `serve_router` (for example in-process tests).
/// let req = Request::new(axum::body::Body::empty());
/// assert!(req.extensions().get::<ConnLease>().is_none());
/// ```
#[derive(Clone)]
pub struct ConnLease {
    /// Shared so every clone releases the count once, when the last one drops.
    _inner: Arc<LeaseInner>,
}

impl std::fmt::Debug for ConnLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnLease").finish_non_exhaustive()
    }
}

/// A response body that keeps its request's lease until the body is dropped, yields data in
/// frames of at most [`EGRESS_FLOOR_BYTES`], and, on an HTTP/2 stream, tells the stream unit when
/// it is waiting on the peer.
///
/// WHY the bound: hyper hands each data frame to HTTP/2 whole and waits for send capacity for all
/// of it, so a multi-megabyte frame would be one indivisible wait and a stall inside it could not
/// be told from a slow flush. At the floor, a stalled peer is visible at a frame boundary, and the
/// floor is also the size of the smallest progress the egress bounds demand.
///
/// WHY the marker is cleared on entry to `poll_frame`: hyper polls the body again only after the
/// peer granted capacity for the previous frame, so the interval between a frame leaving here and
/// the next poll is exactly the peer's credit wait.
struct LeasedBody {
    /// The real body.
    inner: Body,
    /// The rest of the last inner data frame, yielded in slices before the inner body is polled
    /// again. A zero-copy view of the inner frame's allocation.
    pending: Bytes,
    /// The connection the response belongs to, which holds the marker registry.
    live: Arc<LiveState>,
    /// Whether the first poll has looked for a stream cut.
    registered: bool,
    /// The stream-unit marker; `None` on HTTP/1 and on any task outside a stream executor.
    marker: Option<Arc<StreamMarker>>,
    /// Dropped with the body.
    _lease: ConnLease,
}

impl LeasedBody {
    /// Wraps `inner`, holding `lease` until the body drops.
    fn new(inner: Body, lease: ConnLease, live: Arc<LiveState>) -> Self {
        Self {
            inner,
            pending: Bytes::new(),
            live,
            registered: false,
            marker: None,
            _lease: lease,
        }
    }

    /// Takes the next slice of `pending` (at most [`EGRESS_FLOOR_BYTES`]) as a data frame and
    /// marks it delivered. An empty frame waits for no credit, so it leaves the marker clear.
    fn deliver(&mut self) -> Frame<Bytes> {
        let n = self.pending.len().min(EGRESS_FLOOR_BYTES);
        let slice = self.pending.split_to(n);
        if let (Some(marker), false) = (&self.marker, slice.is_empty()) {
            marker.delivered();
        }
        Frame::data(slice)
    }

    /// Clears the marker: the body is being polled, ended, failed or dropped.
    fn clear_marker(&self) {
        if let Some(marker) = &self.marker {
            marker.cleared();
        }
    }
}

impl http_body::Body for LeasedBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        let this = &mut *self;
        this.clear_marker();
        if !this.registered {
            this.registered = true;
            this.marker = current_cut().map(|cut| this.live.register_marker(cut));
        }
        if !this.pending.is_empty() {
            return Poll::Ready(Some(Ok(this.deliver())));
        }
        match Pin::new(&mut this.inner).poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => match frame.into_data() {
                Ok(data) => {
                    this.pending = data;
                    Poll::Ready(Some(Ok(this.deliver())))
                }
                Err(frame) => Poll::Ready(Some(Ok(frame))),
            },
            other => other,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream() && self.pending.is_empty()
    }

    fn size_hint(&self) -> SizeHint {
        let inner = self.inner.size_hint();
        let pending = self.pending.len() as u64;
        let mut hint = SizeHint::new();
        hint.set_lower(inner.lower().saturating_add(pending));
        if let Some(upper) = inner.upper() {
            hint.set_upper(upper.saturating_add(pending));
        }
        hint
    }
}

impl Drop for LeasedBody {
    fn drop(&mut self) {
        self.clear_marker();
    }
}

/// A response body in `live` that has delivered one chunk and has not been polled since, as a
/// stream waiting on its peer's credit is. Polled inside a stream scope it registers a marker.
#[cfg(test)]
pub(super) async fn stalled_body(live: &Arc<LiveState>) -> impl Send {
    let mut body = LeasedBody::new(
        Body::from(Bytes::from(vec![1u8; 2 * EGRESS_FLOOR_BYTES])),
        live.lease(),
        live.clone(),
    );
    std::future::poll_fn(|cx| http_body::Body::poll_frame(Pin::new(&mut body), cx)).await;
    body
}

/// Wraps the application service so every request carries a [`ConnLease`] and every response
/// body keeps one.
#[derive(Clone)]
pub(super) struct LeaseService<S> {
    /// The application service.
    inner: S,
    /// The connection being served.
    live: Arc<LiveState>,
}

impl<S> LeaseService<S> {
    /// Wraps `inner` for the connection described by `live`.
    pub(super) fn new(inner: S, live: Arc<LiveState>) -> Self {
        Self { inner, live }
    }
}

impl<S> Service<Request> for LeaseService<S>
where
    S: Service<Request, Response = Response> + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Response, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request) -> Self::Future {
        let live = self.live.clone();
        let lease = live.lease();
        req.extensions_mut().insert(lease.clone());
        let fut = self.inner.call(req);
        Box::pin(async move {
            let res = fut.await?;
            Ok(res.map(|inner| Body::new(LeasedBody::new(inner, lease, live))))
        })
    }
}

/// Resolves once no stream has been in flight for `idle`. A stream arriving restarts the wait; a
/// dropped count source ends it.
pub(super) async fn idle_for(mut streams: watch::Receiver<usize>, idle: Duration) {
    loop {
        if *streams.borrow_and_update() == 0 {
            tokio::select! {
                _ = tokio::time::sleep(idle) => return,
                changed = streams.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
            }
        } else if streams.changed().await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests;
