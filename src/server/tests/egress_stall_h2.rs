//! The HTTP/2 stream unit of the egress bounds over real sockets: a response whose peer returns no
//! flow-control credit for a pending chunk is reset (`RST_STREAM(CANCEL)`) after
//! `ConnLimits::egress_window`, and nothing else on the connection is touched.
//!
//! Every window-sized wait asserts the cut did NOT happen before the window, from what the client
//! observes (the time its stream was reset), and that it happened within the window plus one tick
//! plus `SLACK`. Cut counts are read from the `DrainReport` the serve loop returns at shutdown,
//! never mid-run.

use std::time::Duration;

use h2::Reason;
use shadowcat::http::conn_serve::ConnLimits;
use shadowcat_test_support::egress::{
    get_path, paced_read, read_unreleased, EgressServer, PacedEnd,
};
use tokio::time::Instant;

/// The no-progress window the tests run at.
const WINDOW: Duration = Duration::from_secs(1);

/// The sampler period at [`WINDOW`]: a quarter of it.
const TICK: Duration = Duration::from_millis(250);

/// Scheduling margin past `WINDOW + TICK` before a cut counts as late. The cut is detected at the
/// first sampler tick at or after the window, so the derived bound is `WINDOW + TICK`; this covers
/// timer and task wake-up latency of a loaded machine.
const SLACK: Duration = Duration::from_millis(750);

/// A server whose response bound is [`WINDOW`] and whose drain ends quickly.
async fn serve() -> EgressServer {
    EgressServer::start(ConnLimits {
        egress_window: WINDOW,
        drain_timeout: Duration::from_millis(500),
        ..ConnLimits::default()
    })
    .await
}

/// Asserts a cut observed `elapsed` after the request was sent respects the window: not before
/// it, and within one tick plus the margin after it.
fn assert_cut_timing(elapsed: Duration, what: &str) {
    assert!(
        elapsed >= WINDOW,
        "{what}: cut after {elapsed:?}, before the {WINDOW:?} window"
    );
    assert!(
        elapsed <= WINDOW + TICK + SLACK,
        "{what}: cut after {elapsed:?}, later than {:?}",
        WINDOW + TICK + SLACK
    );
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
    assert_eq!(drain.stall_cuts.stream, 1, "{drain:?}");
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
    assert_eq!(drain.stall_cuts.stream, 1, "{drain:?}");
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
    assert_eq!(drain.stall_cuts.stream, 1, "{drain:?}");
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
        assert_eq!(drain.stall_cuts.stream, 0, "gap {gap_factor}: {drain:?}");
    }
}
