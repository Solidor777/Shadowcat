//! The HTTP/2 stream unit of the egress bounds over real sockets: a response whose peer returns no
//! flow-control credit for a pending chunk is reset (`RST_STREAM(CANCEL)`) after
//! `ConnLimits::egress_window`, and nothing else on the connection is touched.
//!
//! Every window-sized wait asserts the cut did NOT happen before the window, from what the client
//! observes (the time its stream was reset). How long after the window plus one sampler tick it
//! happened is recorded per operating system (`record_cut_overshoot`), not asserted: the
//! overshoot is timer and task wake-up latency, which no premise bounds. Cut counts are read from the `DrainReport` the serve loop returns at shutdown,
//! never mid-run.

use std::time::Duration;

use h2::Reason;
use shadowcat::http::conn_serve::{ConnLimits, StallCuts};
use shadowcat_test_support::egress::{
    first_close, get_path, paced_read, ping_frame, raw_h2_request, read_unreleased,
    wait_until_flat, EgressServer, PacedEnd,
};
use shadowcat_test_support::measure::record_cut_overshoot;
use tokio::time::Instant;

/// The no-progress window the tests run at.
const WINDOW: Duration = Duration::from_secs(1);

/// The sampler period at [`WINDOW`]: a quarter of it.
const TICK: Duration = Duration::from_millis(250);

/// How often the producer's progress is sampled.
const POLL: Duration = Duration::from_millis(50);

/// How long the producer must stay quiet before it counts as parked.
const SETTLE: Duration = Duration::from_millis(400);

/// A server whose response bound is [`WINDOW`] and whose drain ends quickly.
async fn serve() -> EgressServer {
    EgressServer::start(ConnLimits {
        egress_window: WINDOW,
        drain_timeout: Duration::from_millis(500),
        ..ConnLimits::default()
    })
    .await
}

/// Asserts a cut observed `elapsed` after the request was sent is not before the window, and
/// records how far past the window plus one tick it landed.
fn assert_cut_timing(elapsed: Duration, what: &str) {
    assert!(
        elapsed >= WINDOW,
        "{what}: cut after {elapsed:?}, before the {WINDOW:?} window"
    );
    record_cut_overshoot(what, elapsed, WINDOW + TICK);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stream_with_no_credit_is_cut_alone_and_the_connection_keeps_serving() {
    let server = serve().await;
    // A zero stream window: no DATA can be sent on any stream, so the first chunk waits.
    let mut client = server.connect(0, 1 << 20).await;
    let t0 = Instant::now();
    let response = get_path(&mut client, server.addr, "/endless").await;
    let mut body = response.into_body();
    let (end, _) = read_unreleased(&mut body, Duration::from_secs(10)).await;
    let elapsed = t0.elapsed();
    assert_eq!(end, PacedEnd::Reset(Reason::CANCEL));
    assert_cut_timing(elapsed, "the stalled stream");

    // The connection is open and another stream completes: an empty body needs no credit.
    let mut second = get_path(&mut client, server.addr, "/empty")
        .await
        .into_body();
    assert!(tokio::time::timeout(Duration::from_secs(5), second.data())
        .await
        .expect("the second stream never completed")
        .is_none());
    assert!(!client.driver.is_finished(), "the connection ended");

    drop(client);
    let drain = server.finish().await;
    assert_eq!(
        drain.stall_cuts,
        StallCuts {
            stream: 1,
            connection: 0,
            tracker_overflow: 0,
        },
        "{drain:?}"
    );
    assert_eq!(drain.dropped, 0, "{drain:?}");
    assert_eq!(drain.aborted, 0, "{drain:?}");
    assert_eq!(drain.panicked, 0, "{drain:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cut_frees_the_stream_slot_for_a_new_request() {
    let server = serve().await;
    let mut client = server.connect(0, 1 << 20).await;
    // One stream first, so the client has the server's advertised stream limit.
    let mut first = get_path(&mut client, server.addr, "/empty")
        .await
        .into_body();
    assert!(first.data().await.is_none());
    let limit = client.sender.current_max_send_streams();
    assert!(
        limit >= 10,
        "the server advertises {limit} concurrent streams"
    );

    // Fill every slot with a stalled stream, and wait for each to be cut.
    let mut bodies = Vec::new();
    for _ in 0..limit {
        bodies.push(
            get_path(&mut client, server.addr, "/endless")
                .await
                .into_body(),
        );
    }
    let cuts = futures_util::future::join_all(
        bodies
            .iter_mut()
            .map(|b| read_unreleased(b, Duration::from_secs(10))),
    )
    .await;
    for (end, _) in &cuts {
        assert_eq!(*end, PacedEnd::Reset(Reason::CANCEL));
    }

    // The slots are free again: the same number of requests is accepted.
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut again = Vec::new();
        for _ in 0..limit {
            again.push(
                get_path(&mut client, server.addr, "/endless")
                    .await
                    .into_body(),
            );
        }
        again
    })
    .await
    .expect("a cut stream's slot was not released");

    drop(client);
    let drain = server.finish().await;
    assert!(drain.stall_cuts.stream >= limit, "{drain:?}");
    assert_eq!(drain.stall_cuts.connection, 0, "{drain:?}");
    assert_eq!(drain.dropped, 0, "{drain:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn steady_data_on_a_sibling_and_a_live_tunnel_do_not_postpone_a_stalled_streams_cut() {
    let server = serve().await;
    // A 64 KiB stream window with a connection window far above it: an unread stream cannot
    // exhaust the credit its sibling needs.
    let mut client = server.connect(64 * 1024, 16 << 20).await;
    let _tunnel = server.open_tunnel(&mut client).await;

    let t0 = Instant::now();
    let mut stalled = get_path(&mut client, server.addr, "/endless")
        .await
        .into_body();
    let mut steady = get_path(&mut client, server.addr, "/endless")
        .await
        .into_body();
    let sibling =
        tokio::spawn(
            async move { paced_read(&mut steady, 2_000_000, Duration::from_secs(4)).await },
        );

    // The stalled stream's own pending chunk is what is cut, however much DATA its sibling and
    // the tunnel move meanwhile.
    let (end, _) = read_unreleased(&mut stalled, Duration::from_secs(10)).await;
    let elapsed = t0.elapsed();
    assert_eq!(end, PacedEnd::Reset(Reason::CANCEL));
    assert_cut_timing(elapsed, "the stalled stream with a busy sibling");

    let steady = sibling.await.unwrap();
    assert_eq!(
        steady.end,
        PacedEnd::Deadline,
        "the steady sibling was reset or ended: {steady:?}"
    );
    assert!(
        steady.got >= 2 * 32 * 1024,
        "the sibling moved too little DATA ({} B) for the control to mean anything",
        steady.got
    );
    assert_eq!(server.ws.live_connection_count(), 1, "the tunnel was cut");
    assert!(!client.driver.is_finished(), "the connection ended");

    drop(client);
    let drain = server.finish().await;
    assert_eq!(
        drain.stall_cuts,
        StallCuts {
            stream: 1,
            connection: 0,
            tracker_overflow: 0,
        },
        "{drain:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_live_tunnel_with_no_response_is_never_cut_and_survives_a_siblings_cut() {
    let server = serve().await;
    let mut client = server.connect(0, 1 << 20).await;
    let _tunnel = server.open_tunnel(&mut client).await;

    tokio::time::sleep(WINDOW * 3).await;
    assert_eq!(
        server.ws.live_connection_count(),
        1,
        "an idle tunnel was cut"
    );
    assert!(!client.driver.is_finished());

    let mut stalled = get_path(&mut client, server.addr, "/endless")
        .await
        .into_body();
    let (end, _) = read_unreleased(&mut stalled, Duration::from_secs(10)).await;
    assert_eq!(end, PacedEnd::Reset(Reason::CANCEL));
    assert_eq!(
        server.ws.live_connection_count(),
        1,
        "a sibling's cut took the tunnel"
    );
    assert!(!client.driver.is_finished());

    drop(client);
    let drain = server.finish().await;
    assert_eq!(
        drain.stall_cuts,
        StallCuts {
            stream: 1,
            connection: 0,
            tracker_overflow: 0,
        },
        "{drain:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sparse_producer_with_a_healthy_reader_is_not_cut_across_five_windows() {
    // Producer gaps below and above the window: the stream waits on its producer, never on the
    // peer, so the marker is clear while hyper waits and no gap length is a stall. The gap above
    // the window is the one that distinguishes a marker cleared when hyper polls the body from
    // one cleared only on delivery.
    for (gap_factor, chunks) in [(0.7f64, 7u32), (1.5, 4)] {
        let server = serve().await;
        let mut client = server.connect(64 * 1024, 1 << 20).await;
        let gap = WINDOW.mul_f64(gap_factor).as_millis();
        let mut body = get_path(
            &mut client,
            server.addr,
            &format!("/sparse?chunks={chunks}&gap_ms={gap}"),
        )
        .await
        .into_body();
        let paced = paced_read(&mut body, 1_000_000, Duration::from_secs(20)).await;
        assert_eq!(paced.end, PacedEnd::Ended, "gap {gap_factor}: {paced:?}");
        assert_eq!(paced.got, 100 * u64::from(chunks));
        assert!(
            paced.elapsed >= WINDOW.mul_f64(gap_factor * f64::from(chunks - 1)),
            "gap {gap_factor}: {paced:?}"
        );

        drop(client);
        let drain = server.finish().await;
        assert_eq!(
            drain.stall_cuts,
            StallCuts {
                stream: 0,
                connection: 0,
                tracker_overflow: 0,
            },
            "gap {gap_factor}: {drain:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_http2_peer_that_stops_reading_the_socket_has_its_connection_cut_not_a_stream() {
    let server = serve().await;
    let started = Instant::now();
    // Windows at the protocol maximum: only the socket can stop the server.
    let (mut client, reading) = server.connect_gated(i32::MAX as u32, i32::MAX as u32).await;
    let _body = get_path(&mut client, server.addr, "/endless")
        .await
        .into_body();
    // From here the peer reads nothing: its kernel buffer fills and the server's writer parks.
    reading.store(false, std::sync::atomic::Ordering::SeqCst);
    let quiet = wait_until_flat(&server.produced, POLL, SETTLE, Duration::from_secs(30)).await;

    let closed_at = first_close(&server.closed, Duration::from_secs(20))
        .await
        .expect("the server never ended a connection whose peer stopped reading");
    assert!(
        closed_at >= started + WINDOW,
        "cut {:?} after the start, before the {WINDOW:?} window",
        closed_at.duration_since(started)
    );
    record_cut_overshoot(
        "h2 connection cut after the producer went quiet",
        closed_at.duration_since(quiet),
        WINDOW + TICK,
    );

    drop(client);
    let drain = server.finish().await;
    // One connection cut; the stream behind it is not also counted as cut.
    assert_eq!(
        drain.stall_cuts,
        StallCuts {
            stream: 0,
            connection: 1,
            tracker_overflow: 0,
        },
        "{drain:?}"
    );
    assert_eq!(drain.dropped, 0, "{drain:?}");
    assert_eq!(drain.aborted, 0, "{drain:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_ping_flood_from_a_peer_that_stops_reading_does_not_postpone_the_connection_cut() {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    use tokio::io::AsyncWriteExt;

    let server = serve().await;
    let started = Instant::now();
    let mut peer = raw_h2_request(server.addr, "/endless").await;
    // From here the peer reads nothing, and sends a PING about every 10 ms: the server answers
    // each with an acknowledgement it cannot deliver. Acknowledgements are not response bytes.
    let sent = Arc::new(AtomicU64::new(0));
    let counter = sent.clone();
    let flood = tokio::spawn(async move {
        let ping = ping_frame();
        while peer.write_all(&ping).await.is_ok() {
            counter.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    let quiet = wait_until_flat(&server.produced, POLL, SETTLE, Duration::from_secs(30)).await;

    let closed_at = first_close(&server.closed, Duration::from_secs(20))
        .await
        .expect("a ping flood kept a connection whose peer stopped reading alive");
    assert!(
        sent.load(Ordering::SeqCst) >= 50,
        "only {} pings were sent, too few for the control to mean anything",
        sent.load(Ordering::SeqCst)
    );
    assert!(
        closed_at >= started + WINDOW,
        "cut {:?} after the start, before the {WINDOW:?} window",
        closed_at.duration_since(started)
    );
    record_cut_overshoot(
        "h2 connection cut under a ping flood after the producer went quiet",
        closed_at.duration_since(quiet),
        WINDOW + TICK,
    );
    flood.abort();
    let drain = server.finish().await;
    assert_eq!(
        drain.stall_cuts,
        StallCuts {
            stream: 0,
            connection: 1,
            tracker_overflow: 0,
        },
        "{drain:?}"
    );
}
