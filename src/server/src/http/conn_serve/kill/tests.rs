//! The kill switch's ordering, wake and reason rules, driven with counting wakers so every
//! outcome is deterministic.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::Wake;
use std::time::Duration;

use super::*;

/// A waker that counts its wakes.
struct CountWake(AtomicUsize);

impl Wake for CountWake {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// A fresh counting waker and its counter.
fn counting() -> (Arc<CountWake>, Waker) {
    let counter = Arc::new(CountWake(AtomicUsize::new(0)));
    (counter.clone(), Waker::from(counter))
}

/// Polls `kill` once as the task owning `waker`.
fn poll(kill: &ConnKill, waker: &Waker) -> Poll<KillReason> {
    kill.poll_reason(&mut Context::from_waker(waker))
}

#[test]
fn a_switch_fired_before_the_poll_is_ready_errors_and_registers_no_waker() {
    let kill = ConnKill::new();
    kill.fire(KillReason::Drain);
    let (wakes, waker) = counting();
    assert_eq!(poll(&kill, &waker), Poll::Ready(KillReason::Drain));
    let err = kill
        .check(&mut Context::from_waker(&waker))
        .expect_err("a fired switch must fail the IO");
    assert_eq!(err.kind(), io::ErrorKind::ConnectionAborted);
    assert_eq!(kill.waker_count(), 0, "a fired switch registered a waker");
    assert_eq!(wakes.0.load(Ordering::SeqCst), 0);
}

#[test]
fn a_poll_parked_before_the_fire_is_woken_once_and_then_ready() {
    let kill = ConnKill::new();
    let (wakes, waker) = counting();
    assert_eq!(poll(&kill, &waker), Poll::Pending);
    assert_eq!(wakes.0.load(Ordering::SeqCst), 0);
    kill.fire(KillReason::Drain);
    assert_eq!(wakes.0.load(Ordering::SeqCst), 1);
    assert_eq!(poll(&kill, &waker), Poll::Ready(KillReason::Drain));
}

#[test]
fn one_waker_per_distinct_task_however_often_it_polls() {
    let kill = ConnKill::new();
    let (first_wakes, first) = counting();
    for _ in 0..1_000 {
        let clone = first.clone();
        assert_eq!(poll(&kill, &clone), Poll::Pending);
    }
    assert_eq!(kill.waker_count(), 1, "one task registered more than once");
    let (second_wakes, second) = counting();
    assert_eq!(poll(&kill, &second), Poll::Pending);
    assert_eq!(kill.waker_count(), 2);
    kill.fire(KillReason::Drain);
    assert_eq!(kill.waker_count(), 0, "a fire left wakers registered");
    assert_eq!(first_wakes.0.load(Ordering::SeqCst), 1);
    assert_eq!(second_wakes.0.load(Ordering::SeqCst), 1);
}

#[test]
fn both_consumers_see_the_reason_that_fired_the_switch() {
    for (reason, message) in [
        (
            KillReason::Drain,
            "connection ended: server shutdown deadline",
        ),
        (KillReason::Stall, "connection ended: peer stopped reading"),
    ] {
        let kill = ConnKill::new();
        kill.fire(reason);
        let (_wakes, waker) = counting();
        assert_eq!(poll(&kill, &waker), Poll::Ready(reason));
        let err = kill
            .check(&mut Context::from_waker(&waker))
            .expect_err("a fired switch must fail the IO");
        assert_eq!(err.to_string(), message, "{reason:?}");
    }
}

#[test]
fn only_the_call_that_fires_the_switch_reports_having_done_so() {
    let kill = ConnKill::new();
    assert!(kill.fire(KillReason::Stall));
    assert!(!kill.fire(KillReason::Drain));
    assert!(!kill.fire(KillReason::Stall));
}

#[test]
fn the_first_reason_wins() {
    let kill = ConnKill::new();
    let (wakes, waker) = counting();
    assert_eq!(poll(&kill, &waker), Poll::Pending);
    kill.fire(KillReason::Stall);
    kill.fire(KillReason::Drain);
    assert_eq!(poll(&kill, &waker), Poll::Ready(KillReason::Stall));
    assert_eq!(wakes.0.load(Ordering::SeqCst), 1, "the second fire woke");
}

/// A waker whose wake re-enters the switch it was registered with.
struct Reenter(Arc<ConnKill>);

impl Wake for Reenter {
    fn wake(self: Arc<Self>) {
        let _ = self.0.check(&mut Context::from_waker(Waker::noop()));
    }
}

#[test]
fn a_waker_that_re_enters_the_switch_does_not_deadlock() {
    let kill = ConnKill::new();
    let waker = Waker::from(Arc::new(Reenter(kill.clone())));
    assert_eq!(poll(&kill, &waker), Poll::Pending);
    let (done, finished) = std::sync::mpsc::channel();
    let firing = kill.clone();
    std::thread::spawn(move || {
        firing.fire(KillReason::Drain);
        let _ = done.send(());
    });
    finished
        .recv_timeout(Duration::from_secs(2))
        .expect("fire deadlocked against a waker that re-polls the switch");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn every_task_parked_in_check_is_woken_by_the_fire() {
    let kill = ConnKill::new();
    let park = |kill: Arc<ConnKill>| {
        tokio::spawn(std::future::poll_fn(move |cx| match kill.check(cx) {
            Ok(()) => Poll::Pending,
            Err(e) => Poll::Ready(e.kind()),
        }))
    };
    let first = park(kill.clone());
    let second = park(kill.clone());
    // Bounded by iteration count: a busy yield loop never lets the paused clock advance.
    for _ in 0..1_000 {
        if kill.waker_count() == 2 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(kill.waker_count(), 2, "the two tasks did not both register");
    kill.fire(KillReason::Drain);
    for task in [first, second] {
        let kind = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("a parked task was not woken by the fire")
            .unwrap();
        assert_eq!(kind, io::ErrorKind::ConnectionAborted);
    }
}

#[test]
fn a_connection_created_after_the_signal_fired_starts_fired() {
    let signal = KillSignal::new();
    let before = signal.connection();
    let (_wakes, waker) = counting();
    assert_eq!(poll(&before, &waker), Poll::Pending);
    signal.fire();
    assert_eq!(poll(&before, &waker), Poll::Ready(KillReason::Drain));
    let after = signal.connection();
    assert_eq!(
        poll(&after, &waker),
        Poll::Ready(KillReason::Drain),
        "a connection created after the fire missed it"
    );
    assert!(after.check(&mut Context::from_waker(&waker)).is_err());
}

#[test]
fn firing_one_connection_leaves_the_others_alone() {
    let signal = KillSignal::new();
    let (one, other) = (signal.connection(), signal.connection());
    one.fire(KillReason::Stall);
    let (_wakes, waker) = counting();
    assert_eq!(poll(&one, &waker), Poll::Ready(KillReason::Stall));
    assert_eq!(poll(&other, &waker), Poll::Pending);
}

#[test]
fn the_signal_holds_references_for_the_live_connections_not_every_one_accepted() {
    let signal = KillSignal::new();
    let held: Vec<Arc<ConnKill>> = (0..3).map(|_| signal.connection()).collect();
    let mut peak = 0;
    for _ in 0..100_000 {
        drop(signal.connection());
        peak = peak.max(signal.registry_len());
    }
    assert!(
        peak <= (2 * held.len()).max(super::super::pruned::PRUNE_FLOOR) + 1,
        "the registry reached {peak} entries with {} connections live",
        held.len()
    );
    signal.fire();
    let (_wakes, waker) = counting();
    for conn in &held {
        assert_eq!(poll(conn, &waker), Poll::Ready(KillReason::Drain));
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn fired_pends_before_the_fire_and_resolves_with_the_reason_after_it() {
    let kill = ConnKill::new();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), kill.fired())
            .await
            .is_err(),
        "fired() resolved before the fire"
    );
    let waiting = {
        let kill = kill.clone();
        tokio::spawn(async move { kill.fired().await })
    };
    while kill.waker_count() == 0 {
        tokio::task::yield_now().await;
    }
    kill.fire(KillReason::Stall);
    assert_eq!(waiting.await.unwrap(), KillReason::Stall);
    assert_eq!(kill.fired().await, KillReason::Stall);
}
