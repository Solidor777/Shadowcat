//! This module is the kill switch that fails a connection's IO on purpose, once and for every
//! task that polls it.
//!
//! A [`KillSignal`] (one per serve loop) hands each connection a [`ConnKill`]; firing the signal
//! fires every live connection's [`ConnKill`], and a connection's own [`ConnKill`] can be fired
//! alone, so "make this connection's IO fail" is ONE mechanism for the drain deadline and for any
//! per-connection close.
//!
//! WHY the end is delivered as an IO error and not by aborting the connection task: an HTTP/2
//! extended-CONNECT tunnel (a WebSocket) runs as a task hyper detaches from its connection, and
//! aborting the connection task while such a tunnel is live drops the h2 connection's state out
//! from under it, which trips a debug assertion in h2's stream counting (`Counts::drop`) in debug
//! builds. An IO error instead makes the h2 codec end the connection through its own error path:
//! every stream, tunnels included, is reset by the protocol. The drop and the abort remain only as
//! counted last resorts for a connection that does not end.
//!
//! INVARIANT: After [`ConnKill::fire`], every poll of an IO method of a [`Replay`](super::Replay)
//! holding that [`ConnKill`] returns an error. All five methods must check it (`poll_read`,
//! `poll_write`, `poll_write_vectored`, `poll_flush`, `poll_shutdown`): the h2 codec can be parked
//! in any of them, and `poll_shutdown` is what a connection that is already closing waits in.
//!
//! INVARIANT: Every task parked in any IO method of the connection, however many tasks poll it, is
//! woken by the firing. Registration and firing are ordered by the one lock of [`ConnKill`], so a
//! waker is either registered before the firing (and woken by it) or finds the reason set when it
//! polls.
//!
//! INVARIANT: Dropping the [`KillSignal`] does not fire it: a connection outlives a dropped signal
//! only when the serve loop was cancelled, and its tasks are then dropped with it.
//!
//! INVARIANT: The first reason wins: the reason of a connection that has fired never changes, so
//! the error every IO method returns and the reason the serve loop acts on cannot disagree.
//!
//! INVARIANT: Wakers are woken outside the lock, so a waker that re-polls the connection cannot
//! deadlock.
//!
//! The waker list of a [`ConnKill`] is bounded by the number of distinct tasks polling the
//! connection (a [`Replay`](super::Replay) is polled by the connection task and, after an HTTP/1
//! WebSocket split, by its reader and writer task) ONLY while each task presents a stable waker
//! ([`Waker::will_wake`] deduplicates). A wrapper that mints a new waker per poll would break the
//! bound, so a `Replay` is polled by tokio tasks directly.

use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};

use super::pruned::Pruned;

/// Why a connection's IO was failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum KillReason {
    /// The shutdown drain deadline passed.
    Drain,
    /// An egress bound cut the connection: its socket accepted almost nothing for the window, or its
    /// stream table outgrew its cap.
    Stall,
}

impl KillReason {
    /// The message of the error every IO method returns for this reason.
    fn message(self) -> &'static str {
        match self {
            Self::Drain => "connection ended: server shutdown deadline",
            Self::Stall => "connection ended: response made no progress",
        }
    }
}

/// The state of one connection's kill switch, behind its lock.
struct KillInner {
    /// Set once, by the first [`ConnKill::fire`].
    reason: Option<KillReason>,
    /// The wakers of the tasks parked in an IO method of the connection, one per distinct task.
    wakers: Vec<Waker>,
}

/// One connection's kill switch, shared by the connection's IO and the serve loop. See the module
/// documentation for the invariants.
pub(super) struct ConnKill {
    /// The reason and the registered wakers; one lock orders registration against firing.
    inner: Mutex<KillInner>,
}

/// Locks `m`, ignoring poison: the state it guards is always consistent between statements.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ConnKill {
    /// A switch that has not fired.
    pub(super) fn new() -> Arc<Self> {
        Self::with_reason(None)
    }

    /// A switch in the state `reason`.
    fn with_reason(reason: Option<KillReason>) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(KillInner {
                reason,
                wakers: Vec::new(),
            }),
        })
    }

    /// Fires the switch with `reason` and wakes every parked task. A switch that has already fired
    /// keeps its first reason and wakes nobody. Returns whether this call fired it, so a caller
    /// that counts what it fired does not count a connection another reason already ended.
    pub(super) fn fire(&self, reason: KillReason) -> bool {
        let wakers = {
            let mut inner = lock(&self.inner);
            if inner.reason.is_some() {
                return false;
            }
            inner.reason = Some(reason);
            std::mem::take(&mut inner.wakers)
        };
        for waker in wakers {
            waker.wake();
        }
        true
    }

    /// `Ready(reason)` once fired; otherwise registers `cx`'s waker (once per distinct task) for
    /// the firing and returns `Pending`. This is the one place both consumers read the state.
    pub(super) fn poll_reason(&self, cx: &mut Context<'_>) -> Poll<KillReason> {
        let mut inner = lock(&self.inner);
        if let Some(reason) = inner.reason {
            return Poll::Ready(reason);
        }
        if !inner.wakers.iter().any(|w| w.will_wake(cx.waker())) {
            inner.wakers.push(cx.waker().clone());
        }
        Poll::Pending
    }

    /// Resolves with the reason once the switch has fired. A pending `fired()` registers the
    /// polling task's waker like an IO method does, so one task polling both stays one waker.
    pub(super) async fn fired(&self) -> KillReason {
        std::future::poll_fn(|cx| self.poll_reason(cx)).await
    }

    /// `Ok` while the switch has not fired (registering `cx`'s waker for the firing), otherwise the
    /// error every IO method returns.
    pub(super) fn check(&self, cx: &mut Context<'_>) -> io::Result<()> {
        match self.poll_reason(cx) {
            Poll::Pending => Ok(()),
            Poll::Ready(reason) => Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                reason.message(),
            )),
        }
    }

    /// How many wakers are registered.
    #[cfg(test)]
    pub(super) fn waker_count(&self) -> usize {
        lock(&self.inner).wakers.len()
    }
}

/// The signal's state behind its lock.
struct SignalState {
    /// Set by [`KillSignal::fire`]; a connection created afterwards starts fired.
    fired: bool,
    /// Every connection switch handed out, as a weak reference so an ended connection's switch is
    /// freed with its IO; pruned on insert, so its length is bounded by the live connections and
    /// not by the number ever accepted.
    conns: Pruned<ConnKill>,
}

/// The serve loop's side: hands out a [`ConnKill`] per connection and fires them all at the drain
/// deadline.
pub(super) struct KillSignal {
    /// One lock decides which side of the firing a new connection falls on.
    state: Mutex<SignalState>,
}

impl KillSignal {
    /// A signal that has not fired.
    pub(super) fn new() -> Self {
        Self {
            state: Mutex::new(SignalState {
                fired: false,
                conns: Pruned::new(),
            }),
        }
    }

    /// The switch of one new connection: already fired (with [`KillReason::Drain`]) when the
    /// signal has fired, so a connection accepted while the drain deadline passes is not missed.
    pub(super) fn connection(&self) -> Arc<ConnKill> {
        let mut state = lock(&self.state);
        if state.fired {
            return ConnKill::with_reason(Some(KillReason::Drain));
        }
        let conn = ConnKill::new();
        state.conns.push(Arc::downgrade(&conn));
        conn
    }

    /// How many switch references the signal holds, live and dead.
    #[cfg(test)]
    pub(super) fn registry_len(&self) -> usize {
        lock(&self.state).conns.len()
    }

    /// Fires every connection's switch with [`KillReason::Drain`].
    pub(super) fn fire(&self) {
        let live: Vec<Arc<ConnKill>> = {
            let mut state = lock(&self.state);
            state.fired = true;
            state.conns.live()
        };
        for conn in live {
            conn.fire(KillReason::Drain);
        }
    }
}

#[cfg(test)]
mod tests;
