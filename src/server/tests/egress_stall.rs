//! The connection unit of the egress bounds over real sockets, HTTP/1.
//!
//! A response whose peer has stopped reading the socket parks the server's writer, and a connection
//! whose writer stays parked with fewer than `ConnLimits::egress_floor` bytes accepted over
//! `ConnLimits::egress_window` is closed. The only thing that wakes the connection while it is
//! parked is the sampler tick.
//!
//! The instant the server ended the connection is read from the server's own drop of the socket
//! (`EgressServer::closed`), never from the peer: reading from a peer that has stopped reading is
//! what would change the outcome. Cut counts are read from the `DrainReport` the serve loop
//! returns at shutdown.

use std::sync::atomic::Ordering;
use std::time::Duration;

use shadowcat::http::conn_serve::{egress_tick, ConnLimits, StallCuts, ABORT_WAIT};
use shadowcat_test_support::acceptance::{progress_bytes_per_window, Acceptances};
use shadowcat_test_support::egress::{first_close, wait_until_flat, EgressServer, Protocol};
use shadowcat_test_support::measure::record_cut_after_progress;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::Instant;

/// The no-progress window the tests run at.
const WINDOW: Duration = Duration::from_secs(1);

/// The sampler period at [`WINDOW`].
fn tick() -> Duration {
    egress_tick(WINDOW)
}

/// How often the producer's progress is sampled.
const POLL: Duration = Duration::from_millis(50);

/// How long the producer must stay quiet before it counts as parked.
const SETTLE: Duration = Duration::from_millis(400);

/// The drain bound the tests run at.
const DRAIN: Duration = Duration::from_millis(500);

async fn serve() -> EgressServer {
    serve_with_window(WINDOW).await
}

async fn serve_with_window(window: Duration) -> EgressServer {
    EgressServer::start_with(
        ConnLimits {
            egress_window: window,
            drain_timeout: DRAIN,
            ..ConnLimits::default()
        },
        Protocol::H1,
    )
    .await
}

/// Connects and requests `/endless`; the response is never read unless the test reads it.
async fn request(server: &EgressServer) -> TcpStream {
    let mut stream = TcpStream::connect(server.addr).await.unwrap();
    stream
        .write_all(b"GET /endless HTTP/1.1\r\nHost: test\r\n\r\n")
        .await
        .unwrap();
    stream
}

async fn quiet_at(server: &EgressServer) -> Instant {
    wait_until_flat(&server.produced, POLL, SETTLE, Duration::from_secs(30)).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_http1_peer_that_stops_reading_a_response_is_cut_within_the_window_plus_a_tick() {
    let server = serve().await;
    let started = Instant::now();
    let mut peer = request(&server).await;

    let closed_at = first_close(&server.closed, Duration::from_secs(20))
        .await
        .expect("the server never ended a connection whose peer stopped reading");
    // Not before a full window of samples has elapsed since the request. How long after the
    // socket last accepted a floor of progress the cut landed, past a window and a tick, is
    // recorded and bounded by `CUT_OVERSHOOT_MAX`.
    assert!(
        closed_at >= started + WINDOW,
        "cut {:?} after the request, before the {WINDOW:?} window",
        closed_at.duration_since(started)
    );
    record_cut_after_progress(
        "h1 connection cut after the socket last accepted progress",
        closed_at,
        &server.writes,
        ConnLimits::default().egress_floor,
        WINDOW,
        tick(),
    );

    // The peer sees the connection end once it has drained what the kernel buffered.
    let ended = tokio::time::timeout(Duration::from_secs(20), async {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match peer.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
        }
    })
    .await;
    assert!(ended.is_ok(), "the peer never saw the connection end");

    drop(peer);
    let drain = server.finish().await;
    assert_eq!(
        drain.stall_cuts,
        StallCuts {
            stream: 0,
            connection: 1,
            ended_unflushed: 0,
            tracker_overflow: 0,
        },
        "{drain:?}"
    );
    assert_eq!(drain.dropped, 0, "{drain:?}");
    assert_eq!(drain.aborted, 0, "{drain:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_http1_peer_that_keeps_reading_is_not_cut_across_three_windows() {
    let server = serve().await;
    let mut peer = request(&server).await;
    let step = Duration::from_millis(100);
    let chunk = progress_bytes_per_window() / (WINDOW.as_millis() / step.as_millis()) as usize;
    let mut buf = vec![0u8; chunk];
    let start = Instant::now();
    let mut read = 0usize;
    let mut acceptances = Acceptances::new();
    while start.elapsed() < WINDOW * 3 {
        tokio::time::sleep(step).await;
        acceptances.sample(server.writes.accepted.load(Ordering::Relaxed));
        let n = peer.read(&mut buf).await.unwrap_or_else(|e| {
            panic!(
                "the connection was cut ({e}): {}",
                acceptances.evidence(read)
            )
        });
        assert!(
            n > 0,
            "the connection ended after {:?}: {}",
            start.elapsed(),
            acceptances.evidence(read)
        );
        read += n;
    }
    acceptances.assert_premise(read);
    assert!(
        server.closed.lock().unwrap().is_empty(),
        "the server ended a connection that was making progress: {}",
        acceptances.evidence(read)
    );
    // Still being produced, not merely buffered.
    assert!(server.produced.load(Ordering::SeqCst) > read);

    drop(peer);
    let drain = server.finish().await;
    assert_eq!(drain.stall_cuts, StallCuts::default(), "{drain:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_shutdown_during_a_stall_returns_within_the_drain_bound() {
    // A window far longer than the drain bound, so the drain's deadline is what ends the
    // connection and no stall cut can land inside the bound.
    let server = serve_with_window(DRAIN * 20).await;
    let _peer = request(&server).await;
    quiet_at(&server).await;

    let started = Instant::now();
    let drain = server.finish().await;
    let elapsed = started.elapsed();
    assert!(
        elapsed <= DRAIN + ABORT_WAIT,
        "the drain took {elapsed:?}, past {:?}",
        DRAIN + ABORT_WAIT
    );
    assert_eq!(drain.killed, 1, "{drain:?}");
    assert_eq!(drain.aborted, 0, "{drain:?}");
    assert_eq!(drain.dropped, 0, "{drain:?}");
    assert_eq!(
        drain.stall_cuts,
        StallCuts::default(),
        "a drain kill is not a stall: {drain:?}"
    );
}
