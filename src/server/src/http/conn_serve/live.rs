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

use crate::http::peer_slots::Slot;

/// What a connection holds for its whole life: its cap slot and the in-flight stream count.
pub(super) struct LiveState {
    /// The connection's live-cap slot; `None` only for a connection admitted without one.
    _slot: Option<Slot>,
    /// Number of leases currently held; the HTTP/2 idle bound watches it.
    streams: watch::Sender<usize>,
}

impl LiveState {
    /// State for a connection that holds `slot`, with no stream in flight.
    pub(super) fn new(slot: Option<Slot>) -> Arc<Self> {
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

/// A response body that keeps its request's lease until the body is dropped.
struct LeasedBody {
    /// The real body.
    inner: Body,
    /// Dropped with the body.
    _lease: ConnLease,
}

impl http_body::Body for LeasedBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        Pin::new(&mut self.inner).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
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
            Ok(res.map(|inner| {
                Body::new(LeasedBody {
                    inner,
                    _lease: lease,
                })
            }))
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
