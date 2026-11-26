//! These tests cover the stream executor: scope visibility across yields, the cut dropping the
//! future, and a cut that fires before the first poll.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use hyper::rt::Executor;
use tokio::sync::oneshot;
use tokio::time::timeout;

use super::*;

/// Sets its flag when dropped.
struct DropFlag(Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn the_wrapped_future_sees_its_cut_across_yields() {
    let (tx, rx) = oneshot::channel();
    StreamExec.execute(async move {
        for _ in 0..3 {
            tokio::task::yield_now().await;
        }
        let _ = tx.send(current_cut().is_some());
    });
    assert!(timeout(Duration::from_secs(5), rx).await.unwrap().unwrap());
}

#[tokio::test]
async fn a_task_spawned_without_the_executor_has_no_cut() {
    let (tx, rx) = oneshot::channel();
    tokio::spawn(async move {
        let _ = tx.send(current_cut().is_none());
    });
    assert!(
        timeout(Duration::from_secs(5), rx).await.unwrap().unwrap(),
        "a directly spawned task saw a cut, so the scope test above proves nothing"
    );
}

#[tokio::test]
async fn firing_the_cut_drops_the_inner_future() {
    let dropped = Arc::new(AtomicBool::new(false));
    let (tx, rx) = oneshot::channel();
    let flag = DropFlag(dropped.clone());
    StreamExec.execute(async move {
        let _flag = flag;
        let _ = tx.send(current_cut().unwrap());
        std::future::pending::<()>().await;
    });
    let cut = timeout(Duration::from_secs(5), rx).await.unwrap().unwrap();
    assert!(!dropped.load(Ordering::SeqCst), "dropped before the cut");
    cut.fire();
    timeout(Duration::from_secs(5), async {
        while !dropped.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the cut did not drop the future");
}

#[tokio::test]
async fn a_cut_fired_before_the_first_poll_still_drops_the_future_unpolled() {
    let dropped = Arc::new(AtomicBool::new(false));
    let polled = Arc::new(AtomicBool::new(false));
    let (flag, seen) = (DropFlag(dropped.clone()), polled.clone());
    let (cut, run) = scoped(async move {
        let _flag = flag;
        seen.store(true, Ordering::SeqCst);
        std::future::pending::<()>().await;
    });
    cut.fire();
    timeout(Duration::from_secs(5), run)
        .await
        .expect("a pre-fired cut did not end the task");
    assert!(dropped.load(Ordering::SeqCst), "the future was not dropped");
    assert!(
        !polled.load(Ordering::SeqCst),
        "the future ran although its cut had already fired"
    );
}

#[tokio::test]
async fn a_future_that_completes_ends_the_task_without_a_cut() {
    let (cut, run) = scoped(async {});
    timeout(Duration::from_secs(5), run).await.unwrap();
    assert!(!cut.has_fired());
}
