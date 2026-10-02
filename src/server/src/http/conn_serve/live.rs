//! Per-connection liveness: the connection's cap slot, and the count of in-flight streams the
//! HTTP/2 idle bound reads.
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
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::Request;
use axum::response::Response;
use http_body::{Frame, SizeHint};
use tokio::sync::watch;
use tower::Service;

use super::EGRESS_FLOOR_BYTES;
use crate::http::peer_slots::Slot;

/// What a connection holds for its whole life: its cap slot and the in-flight stream count.
pub(super) struct LiveState {
    /// The connection's live-cap slot, released when the state (the connection task and every
    /// lease) is dropped.
    _slot: Slot,
    /// Number of leases currently held; the HTTP/2 idle bound watches it.
    streams: watch::Sender<usize>,
}

impl LiveState {
    /// State for a connection that holds `slot`, with no stream in flight.
    pub(super) fn new(slot: Slot) -> Arc<Self> {
        Arc::new(Self {
            _slot: slot,
            streams: watch::channel(0).0,
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

/// A response body that keeps its request's lease until the body is dropped, and yields data in
/// frames of at most [`EGRESS_FLOOR_BYTES`].
///
/// WHY the bound: hyper hands each data frame to HTTP/2 whole and waits for send capacity for all
/// of it, so a multi-megabyte frame would be one indivisible wait and a stall inside it could not
/// be told from a slow flush. At the floor, a stalled peer is visible at a frame boundary, and the
/// floor is also the size of the smallest progress the egress bounds demand.
struct LeasedBody {
    /// The real body.
    inner: Body,
    /// The rest of the last inner data frame, yielded in slices before the inner body is polled
    /// again. A zero-copy view of the inner frame's allocation.
    pending: Bytes,
    /// Dropped with the body.
    _lease: ConnLease,
}

impl LeasedBody {
    /// Wraps `inner`, holding `lease` until the body drops.
    fn new(inner: Body, lease: ConnLease) -> Self {
        Self {
            inner,
            pending: Bytes::new(),
            _lease: lease,
        }
    }

    /// The next slice of `pending`, at most [`EGRESS_FLOOR_BYTES`] long.
    fn next_slice(&mut self) -> Bytes {
        let n = self.pending.len().min(EGRESS_FLOOR_BYTES);
        self.pending.split_to(n)
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
        if !this.pending.is_empty() {
            return Poll::Ready(Some(Ok(Frame::data(this.next_slice()))));
        }
        match Pin::new(&mut this.inner).poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => match frame.into_data() {
                Ok(data) => {
                    this.pending = data;
                    Poll::Ready(Some(Ok(Frame::data(this.next_slice()))))
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
        let lease = self.live.lease();
        req.extensions_mut().insert(lease.clone());
        let fut = self.inner.call(req);
        Box::pin(async move {
            let res = fut.await?;
            Ok(res.map(|inner| Body::new(LeasedBody::new(inner, lease))))
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
