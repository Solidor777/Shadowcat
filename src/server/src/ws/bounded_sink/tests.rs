//! The stall bound and its shutdown shortening, over a sink whose readiness a test controls.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use futures_util::SinkExt;

use super::*;

/// A sink that is ready only while `open` is set.
struct Valve {
    /// Whether sends are accepted.
    open: Arc<AtomicBool>,
    /// Items accepted.
    taken: Vec<u8>,
    /// The waker of the task pending on the valve, shared with its `Handle`.
    waker: Arc<Mutex<Option<std::task::Waker>>>,
}

/// Test-side control of a `Valve` that has been moved into a sink.
struct Handle {
    /// Shared with the valve.
    open: Arc<AtomicBool>,
    /// Shared with the valve.
    waker: Arc<Mutex<Option<std::task::Waker>>>,
}

impl Handle {
    /// Sets whether sends are accepted and wakes a task pending on the valve.
    fn set(&self, open: bool) {
        self.open.store(open, Ordering::SeqCst);
        if let Some(w) = self.waker.lock().unwrap().take() {
            w.wake();
        }
    }
}

impl Sink<u8> for Valve {
    type Error = ();

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), ()>> {
        if self.open.load(Ordering::SeqCst) {
            Poll::Ready(Ok(()))
        } else {
            *self.waker.lock().unwrap() = Some(cx.waker().clone());
            Poll::Pending
        }
    }

    fn start_send(mut self: Pin<&mut Self>, item: u8) -> Result<(), ()> {
        self.taken.push(item);
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), ()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), ()>> {
        Poll::Ready(Ok(()))
    }
}

fn valve(open: bool) -> (Valve, Handle) {
    let flag = Arc::new(AtomicBool::new(open));
    let waker = Arc::new(Mutex::new(None));
    (
        Valve {
            open: flag.clone(),
            taken: Vec::new(),
            waker: waker.clone(),
        },
        Handle { open: flag, waker },
    )
}

#[tokio::test]
async fn a_send_that_never_becomes_ready_fails_as_stalled_at_the_bound() {
    let (inner, _handle) = valve(false);
    let (_tx, rx) = watch::channel(false);
    let mut sink = BoundedSink::new(inner, rx, Duration::from_millis(200));
    let started = std::time::Instant::now();
    let r = tokio::time::timeout(Duration::from_secs(5), sink.send(1)).await;
    assert!(matches!(r, Ok(Err(BoundedSendError::Stalled))), "{r:?}");
    assert!(started.elapsed() >= Duration::from_millis(150));
}

#[tokio::test]
async fn a_send_that_completes_after_pending_disarms_the_timer() {
    let (inner, handle) = valve(false);
    let (_tx, rx) = watch::channel(false);
    let mut sink = BoundedSink::new(inner, rx, Duration::from_millis(400));
    // The first send pends (arming the timer), then completes well inside the bound.
    let first = tokio::spawn(async move {
        sink.send(1).await.unwrap();
        sink
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    handle.set(true);
    let mut sink = first.await.unwrap();
    // Past the first send's deadline, a second send that pends must get a fresh bound.
    tokio::time::sleep(Duration::from_millis(600)).await;
    handle.set(false);
    let second = tokio::spawn(async move {
        let r = sink.send(2).await;
        (r, sink)
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    handle.set(true);
    let (r, sink) = second.await.unwrap();
    assert!(
        r.is_ok(),
        "a stale timer failed a send that was within its own bound"
    );
    assert_eq!(sink.inner.taken, vec![1, 2]);
}

#[tokio::test]
async fn shutdown_shortens_a_pending_send_to_the_close_bound() {
    let (inner, _handle) = valve(false);
    let (tx, rx) = watch::channel(false);
    // A stall bound far above the test's patience: only the shortening can end it in time.
    let mut sink = BoundedSink::new(inner, rx, Duration::from_secs(600));
    let send = tokio::spawn(async move { sink.send(1).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    tx.send(true).unwrap();
    let r = tokio::time::timeout(CLOSE_SEND_TIMEOUT * 3, send).await;
    assert!(
        matches!(r, Ok(Ok(Err(BoundedSendError::Stalled)))),
        "a pending send outlived the Close bound after shutdown"
    );
}

#[tokio::test]
async fn a_send_pending_when_the_signal_is_already_set_gets_only_the_close_bound() {
    let (inner, _handle) = valve(false);
    let (_tx, rx) = watch::channel(true);
    let mut sink = BoundedSink::new(inner, rx, Duration::from_secs(600));
    let r = tokio::time::timeout(CLOSE_SEND_TIMEOUT * 3, sink.send(1)).await;
    assert!(matches!(r, Ok(Err(BoundedSendError::Stalled))), "{r:?}");
}
