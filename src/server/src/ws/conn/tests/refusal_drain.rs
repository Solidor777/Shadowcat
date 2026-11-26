//! These tests check the wait a refused join adds after its refusal frame and Close are sent: the
//! drain of the peer's inbound frames (`drain_inbound`) and the refusal's total wait once the
//! server is shutting down.
//!
//! Every test runs on a paused clock, so the bounds are measured exactly and no test sleeps.

use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use tokio::time::Instant;

use super::shutdown_bound::refusal_sink;
use super::*;

/// The extra time a bound check allows for the timer's millisecond rounding.
const SLACK: Duration = Duration::from_millis(100);

/// A peer's inbound half: yields `frames` in order, counts each one it hands out, then stays
/// silent forever (a peer that never closes).
fn peer(
    frames: Vec<Message>,
) -> (
    impl Stream<Item = Result<Message, axum::Error>> + Unpin,
    Arc<AtomicUsize>,
) {
    let consumed = Arc::new(AtomicUsize::new(0));
    let counter = consumed.clone();
    let stream = futures_util::stream::iter(frames)
        .inspect(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        })
        .map(Ok)
        .chain(futures_util::stream::pending());
    (stream, consumed)
}

/// A shutdown channel whose signal is not set.
fn quiet_shutdown() -> (
    tokio::sync::watch::Sender<bool>,
    tokio::sync::watch::Receiver<bool>,
) {
    tokio::sync::watch::channel(false)
}

/// A refusal reads every frame the peer already sent, up to and including its Close, and no
/// frame after it, before it returns.
#[tokio::test(start_paused = true)]
async fn a_refusal_consumes_the_peers_pending_frames_through_its_close() {
    let (mut sink, mut out, _tx) = refusal_sink(10, Duration::from_secs(5));
    let (_shutdown_tx, shutdown) = quiet_shutdown();
    let (mut stream, consumed) = peer(vec![
        Message::Text("hello".into()),
        Message::Text("resync".into()),
        Message::Close(None),
        Message::Text("after close".into()),
    ]);

    refuse_streams(
        &mut sink,
        &mut stream,
        shutdown,
        WsErrorCode::Internal,
        "internal",
    )
    .await;

    assert_eq!(
        consumed.load(Ordering::SeqCst),
        3,
        "the refusal must read the pending frames through the peer's Close and stop there"
    );
    assert!(msg_text(&out.try_recv().expect("the refusal frame")).contains("internal"));
    assert!(matches!(out.try_recv(), Ok(Message::Close(_))));
}

/// A peer that never sends anything ends the drain at `CLOSE_SEND_TIMEOUT`, not earlier and not
/// never.
#[tokio::test(start_paused = true)]
async fn a_silent_peer_ends_the_drain_at_the_close_bound() {
    let (_shutdown_tx, shutdown) = quiet_shutdown();
    let (mut stream, consumed) = peer(Vec::new());
    let started = Instant::now();

    tokio::time::timeout(CLOSE_SEND_TIMEOUT * 3, drain_inbound(&mut stream, shutdown))
        .await
        .expect("the drain waited on a peer that never closes");

    let waited = started.elapsed();
    assert!(
        waited >= CLOSE_SEND_TIMEOUT,
        "the drain ended early: {waited:?}"
    );
    assert!(
        waited < CLOSE_SEND_TIMEOUT + SLACK,
        "the drain overran its bound: {waited:?}"
    );
    assert_eq!(consumed.load(Ordering::SeqCst), 0);
}

/// A shutdown signalled while the drain waits ends it at once.
#[tokio::test(start_paused = true)]
async fn shutdown_signalled_during_the_drain_ends_it_at_once() {
    let (shutdown_tx, shutdown) = quiet_shutdown();
    let (mut stream, _consumed) = peer(Vec::new());
    let signal_at = Duration::from_millis(300);
    let started = Instant::now();

    let signal = async {
        tokio::time::sleep(signal_at).await;
        shutdown_tx.send(true).unwrap();
    };
    tokio::join!(drain_inbound(&mut stream, shutdown), signal);

    let waited = started.elapsed();
    assert!(
        waited < signal_at + SLACK,
        "the drain outlived the shutdown signal: {waited:?}"
    );
}

/// A drain that starts after shutdown was signalled does not wait at all: the refusal's own sends
/// may have consumed the time already.
#[tokio::test(start_paused = true)]
async fn a_drain_started_after_shutdown_returns_at_once() {
    let (shutdown_tx, shutdown) = quiet_shutdown();
    shutdown_tx.send(true).unwrap();
    let (mut stream, _consumed) = peer(Vec::new());
    let started = Instant::now();

    drain_inbound(&mut stream, shutdown).await;

    assert!(
        started.elapsed() < SLACK,
        "a drain begun after shutdown waited: {:?}",
        started.elapsed()
    );
}

/// With shutdown signalled, a peer that neither reads nor sends holds the whole refusal (error
/// send, Close and drain) to one `CLOSE_SEND_TIMEOUT`, the bound `SESSION_GRACE` derives from.
#[tokio::test(start_paused = true)]
async fn after_shutdown_a_refusal_to_a_dead_peer_returns_within_one_close_bound() {
    // A stall bound far above the Close bound: only the shutdown shortening can end the sends.
    let (mut sink, _out, tx) = refusal_sink(0, Duration::from_secs(600));
    tx.send(true).unwrap();
    let shutdown = tx.subscribe();
    let (mut stream, _consumed) = peer(Vec::new());
    let started = Instant::now();

    tokio::time::timeout(
        CLOSE_SEND_TIMEOUT * 5,
        refuse_streams(
            &mut sink,
            &mut stream,
            shutdown,
            WsErrorCode::Internal,
            "internal",
        ),
    )
    .await
    .expect("a refusal outlived every bound after shutdown");

    let waited = started.elapsed();
    assert!(
        waited < CLOSE_SEND_TIMEOUT + SLACK,
        "the refusal needed more than one Close bound after shutdown: {waited:?}"
    );
}
