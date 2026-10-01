//! The drain deadline's delivery to connection IO: a signal every connection's IO observes and
//! answers with an I/O error.
//!
//! WHY the deadline is delivered as an IO error and not by aborting the connection task: an HTTP/2
//! extended-CONNECT tunnel (a WebSocket) runs as a task hyper detaches from its connection, and
//! aborting the connection task while such a tunnel is live drops the h2 connection's state out
//! from under it, which trips a debug assertion in h2's stream counting (`Counts::drop`, measured
//! on h2 0.4.19 debug builds). An IO error instead makes the h2 codec end the connection through
//! its own error path: every stream, tunnels included, is reset by the protocol.
//!
//! INVARIANTS:
//! 1. After [`KillSignal::fire`], every poll of an IO method of a [`Replay`](super::Replay)
//!    holding a [`KillSwitch`] of that signal returns an error. All five methods must check it
//!    (`poll_read`, `poll_write`, `poll_write_vectored`, `poll_flush`, `poll_shutdown`): the h2
//!    codec can be parked in any of them, and `poll_shutdown` is what a connection that is
//!    already closing waits in.
//! 2. A switch registers the polling task's waker on every poll that returns `Ok`, so a
//!    connection parked on a peer that never reads or writes is woken by the firing.
//! 3. Dropping the signal does not fire it: a connection outlives a dropped signal only when the
//!    serve loop was cancelled, and its tasks are then dropped with it.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::sync::watch;

/// The sending half: fired once, by the serve loop, when the drain deadline passes.
pub(super) struct KillSignal(watch::Sender<bool>);

impl KillSignal {
    /// A signal that has not fired.
    pub(super) fn new() -> Self {
        Self(watch::channel(false).0)
    }

    /// A switch for one connection's IO.
    pub(super) fn switch(&self) -> KillSwitch {
        let mut rx = self.0.subscribe();
        KillSwitch {
            wait: Box::pin(async move {
                if rx.wait_for(|fired| *fired).await.is_err() {
                    std::future::pending::<()>().await;
                }
            }),
            fired: false,
        }
    }

    /// Fires every switch of this signal.
    pub(super) fn fire(&self) {
        self.0.send_replace(true);
    }
}

/// The receiving half held by one connection's IO.
pub(super) struct KillSwitch {
    /// Resolves once the signal has fired.
    wait: Pin<Box<dyn Future<Output = ()> + Send>>,
    /// Latched once observed, so later polls need not re-poll `wait`.
    fired: bool,
}

impl KillSwitch {
    /// A switch that never fires, for IO tests that do not exercise the deadline.
    #[cfg(test)]
    pub(super) fn never() -> Self {
        Self {
            wait: Box::pin(std::future::pending()),
            fired: false,
        }
    }

    /// `Ok` while the signal has not fired (registering `cx`'s waker for the firing), otherwise
    /// the error every IO method returns.
    pub(super) fn check(&mut self, cx: &mut Context<'_>) -> io::Result<()> {
        if !self.fired && self.wait.as_mut().poll(cx) == Poll::Ready(()) {
            self.fired = true;
        }
        if self.fired {
            Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "connection closed: the shutdown drain deadline passed",
            ))
        } else {
            Ok(())
        }
    }
}
