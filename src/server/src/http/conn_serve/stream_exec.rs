//! The per-stream cut: how one HTTP/2 stream's response is ended without touching its connection.
//!
//! [`StreamExec`] is the executor hyper's HTTP/2 server spawns every request's future through
//! (`execute_h2stream`; an extended-CONNECT tunnel's upgrade task goes through it too). It runs
//! each future inside a [`STREAM_CUT`] task-local scope and races it against that scope's
//! [`StreamCut`]. Firing the cut drops the future, and dropping a stream's last handles makes h2
//! schedule `RST_STREAM(CANCEL)` for it (h2's `maybe_cancel` and `schedule_implicit_reset`), which
//! discards the stream's buffered DATA and frees its `max_concurrent_streams` slot. The connection,
//! every other stream and every tunnel are untouched.
//!
//! INVARIANTS:
//! 1. The scope is entered around the WHOLE future, so a `LeasedBody` polled by the response pipe
//!    (the same `poll` as the service future in hyper's `H2Stream`) reads it with
//!    [`current_cut`]. A task spawned any other way has no scope, so nothing registers a cut for it.
//! 2. Only a future that runs a `LeasedBody` registers a marker, so a tunnel (no `LeasedBody`) is
//!    never cut by this unit, and the wrapper need not tell the two apart.
//! 3. A cut fired before the future's first poll still drops it: the race is `biased` toward the
//!    cut, and a `watch` value is not lost to a late subscriber.

use std::future::Future;

use tokio::sync::watch;

tokio::task_local! {
    /// The cut of the stream task the current code runs in.
    static STREAM_CUT: StreamCut;
}

/// The handle that ends one stream's task. Cloning shares the same cut.
#[derive(Clone)]
pub(super) struct StreamCut {
    /// `true` once the cut has fired. A `watch` keeps the value, so a firing before the task first
    /// polls its receiver is not lost.
    fired: watch::Sender<bool>,
}

impl StreamCut {
    /// A cut that has not fired.
    fn new() -> Self {
        Self {
            fired: watch::channel(false).0,
        }
    }

    /// Fires the cut: the stream's future is dropped at its task's next poll.
    pub(super) fn fire(&self) {
        self.fired.send_replace(true);
    }

    /// Whether the cut has fired.
    #[cfg(test)]
    pub(super) fn has_fired(&self) -> bool {
        *self.fired.borrow()
    }
}

/// The cut of the stream task the caller runs in, or `None` outside a [`StreamExec`] task (HTTP/1
/// connections, and any task spawned another way).
pub(super) fn current_cut() -> Option<StreamCut> {
    STREAM_CUT.try_with(StreamCut::clone).ok()
}

/// The executor for the HTTP/2 server's stream tasks. See the module documentation.
#[derive(Clone, Copy)]
pub(super) struct StreamExec;

impl<Fut> hyper::rt::Executor<Fut> for StreamExec
where
    Fut: Future + Send + 'static,
    Fut::Output: Send + 'static,
{
    fn execute(&self, fut: Fut) {
        tokio::spawn(run_until_cut(StreamCut::new(), fut));
    }
}

/// Runs `fut` inside `cut`'s scope until it completes or `cut` fires, whichever is first. A cut
/// that fired before the first poll wins: the race is `biased` toward it.
async fn run_until_cut<Fut: Future>(cut: StreamCut, fut: Fut) {
    let mut fired = cut.fired.subscribe();
    tokio::select! {
        biased;
        _ = fired.wait_for(|f| *f) => {}
        _ = STREAM_CUT.scope(cut, fut) => {}
    }
}

/// Runs `fut` inside a fresh cut scope, and returns the cut with the future that runs it. The
/// future ends when `fut` does or the cut fires.
#[cfg(test)]
pub(super) fn scoped<Fut: Future>(fut: Fut) -> (StreamCut, impl Future<Output = ()>) {
    let cut = StreamCut::new();
    (cut.clone(), run_until_cut(cut, fut))
}

impl StreamExec {
    /// Spawns `fut` through the executor, for a test that is not itself a hyper stream task.
    #[cfg(test)]
    pub(super) fn execute_test<Fut>(&self, fut: Fut)
    where
        Fut: Future + Send + 'static,
        Fut::Output: Send + 'static,
    {
        hyper::rt::Executor::execute(self, fut);
    }
}

#[cfg(test)]
mod tests;
