//! The HTTP/2 stream unit of the egress bounds over real sockets.
//!
//! A response whose peer returns no flow-control credit for a pending chunk is reset
//! (`RST_STREAM(CANCEL)`) after `ConnLimits::egress_window`, and nothing else on the connection is
//! touched. And the ended-unflushed unit: a response whose body has ended but whose tail is still
//! queued behind a stalled stream window has no handle left to reset, so its connection is cut when
//! that stream's own DATA stops progressing for the window.
//!
//! Every window-sized wait asserts the cut did NOT happen before the window, from what the client
//! observes (the time its stream was reset). How long after the window plus one sampler tick it
//! happened is recorded per operating system and bounded by `CUT_OVERSHOOT_MAX`
//! (`record_cut_overshoot`): the overshoot is timer and task wake-up latency, which no premise
//! bounds, so its ceiling is empirical. The tighter upper bound asserted is the partly drained
//! tail's, derived from the window with its premises asserted beside it
//! (`a_tail_the_reader_drains_only_in_part_is_cut_at_the_end_of_the_window`). Cut counts
//! are read from the `DrainReport` the serve loop returns at shutdown, never mid-run.

use std::time::Duration;

use h2::Reason;
use shadowcat::http::conn_serve::{egress_tick, ConnLimits, StallCuts, EGRESS_TICKS_PER_WINDOW};
use shadowcat_test_support::egress::{
    first_close, get_path, paced_read, ping_frame, raw_h2_request, read_unreleased, send_scripted,
    wait_until_flat, BodyEnd, EgressServer, PacedEnd, RawPeer,
};
use shadowcat_test_support::measure::record_cut_overshoot;
use tokio::time::Instant;

/// The no-progress window the tests run at.
const WINDOW: Duration = Duration::from_secs(1);

/// The sampler period at [`WINDOW`].
fn tick() -> Duration {
    egress_tick(WINDOW)
}

/// The no-progress window of the partly drained tail's test. The test's margins on each side are
/// three eighths of the window, so this is the one dial for them: 0.9 s at 2.4 s, under the 1 s
/// ceiling a timing assertion's margin carries.
const PARTIAL_TAIL_WINDOW: Duration = Duration::from_millis(2400);

/// How often the producer's progress is sampled.
const POLL: Duration = Duration::from_millis(50);

/// How long the producer must stay quiet before it counts as parked.
const SETTLE: Duration = Duration::from_millis(400);

/// A server whose response bound is [`WINDOW`] and whose drain ends quickly.
async fn serve() -> EgressServer {
    serve_at(WINDOW).await
}

/// A server whose response bound is `window` and whose drain ends quickly.
async fn serve_at(window: Duration) -> EgressServer {
    EgressServer::start(ConnLimits {
        egress_window: window,
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
    record_cut_overshoot(what, elapsed, WINDOW + tick());
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
            ended_unflushed: 0,
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
            ended_unflushed: 0,
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
            ended_unflushed: 0,
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
                ended_unflushed: 0,
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
        WINDOW + tick(),
    );

    drop(client);
    let drain = server.finish().await;
    // One connection cut; the stream behind it is not also counted as cut.
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
        WINDOW + tick(),
    );
    flood.abort();
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
}

// ---------------------------------------------------------------------------------------------
// Ended-but-unflushed responses: the stream table's cut
// ---------------------------------------------------------------------------------------------

/// The stream window of the clients that never (or barely) return credit: one KiB, so every
/// response larger than that parks its tail behind a zero window.
const SMALL_WINDOW: u32 = 1024;

/// The connection unit's floor, bytes per window: also what an ended response's own stream must
/// write per window (or what remains of its tail, if less).
fn floor() -> u64 {
    ConnLimits::default().egress_floor as u64
}

/// The instant the first scripted body was dropped, waiting up to five seconds for it. A drop is
/// hyper's hand-over of the body's last byte to h2 (or a cut stream), so it starts the tail's clock.
async fn first_body_drop(server: &EgressServer) -> Instant {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(at) = server.drops.lock().unwrap().first().copied() {
            return at;
        }
        assert!(
            Instant::now() < deadline,
            "the route's body was never dropped"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The first connection close logged after the first `already` entries, waiting up to `limit`.
async fn close_after(server: &EgressServer, already: usize, limit: Duration) -> Option<Instant> {
    let deadline = Instant::now() + limit;
    loop {
        if let Some(at) = server.closed.lock().unwrap().get(already).copied() {
            return Some(at);
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Waits until the server has logged `count` connection closes (the login connection a tunnel
/// opens on the side), so a later close is attributable to the connection under test.
async fn wait_for_closes(server: &EgressServer, count: usize) {
    assert!(
        close_after(server, count - 1, Duration::from_secs(5))
            .await
            .is_some(),
        "the server never logged {count} closes"
    );
}

/// How many connection closes the server has logged.
fn closes(server: &EgressServer) -> usize {
    server.closed.lock().unwrap().len()
}

/// Reads everything that arrives on `body` without returning credit, until nothing has arrived
/// for 300 ms, and returns the bytes read.
async fn drain_without_credit(body: &mut h2::RecvStream) -> u64 {
    let mut got = 0u64;
    while let Ok(Some(Ok(chunk))) =
        tokio::time::timeout(Duration::from_millis(300), body.data()).await
    {
        got += chunk.len() as u64;
    }
    got
}

/// Asserts a cut observed `cut_at` is not before `since + window` and records how far past the
/// window plus one tick it landed, bounded by `CUT_OVERSHOOT_MAX`.
///
/// The lower bound is derived: an entry's window starts no earlier than the body's drop, so no cut
/// can precede it. The upper bound asserted here is only the empirical ceiling, because how late
/// the sampler task runs is scheduling, not the unit's contract; the overshoot is recorded for
/// each OS leg. That an
/// entry was not refreshed by traffic that must not count is established by the tests that drive
/// such traffic: from the order of events they cause, or, for the partly drained tail, from a
/// bound derived from the window with its premises asserted (see
/// `a_tail_the_reader_drains_only_in_part_is_cut_at_the_end_of_the_window`). The tick is
/// `egress_tick(window)`, the sampler period `ConnLimits::egress_window` yields.
fn assert_tail_cut_timing(cut_at: Instant, since: Instant, window: Duration, what: &str) {
    let elapsed = cut_at.duration_since(since);
    assert!(
        elapsed >= window,
        "{what}: cut {elapsed:?} after the body dropped, before the {window:?} window"
    );
    record_cut_overshoot(what, elapsed, window + egress_tick(window));
}

/// The drain report's cut counts for `ended_unflushed` only.
fn only_ended_unflushed(count: usize) -> StallCuts {
    StallCuts {
        ended_unflushed: count,
        ..StallCuts::default()
    }
}

/// A response whose body ended (8 KiB, so one chunk) behind a client that never returns credit is
/// a zombie: its stream has no handle left to reset, so the connection is cut. `post` leaves the
/// request side of the stream open; `tunnel` keeps a WebSocket tunnel on the connection.
async fn zombie_is_cut_as_a_connection(post: bool, tunnel: bool, end: BodyEnd) {
    let server = serve().await;
    let mut client = server.connect(SMALL_WINDOW, 1 << 20).await;
    let _tunnel = if tunnel {
        let tunnel = server.open_tunnel(&mut client).await;
        wait_for_closes(&server, 1).await;
        Some(tunnel)
    } else {
        None
    };
    let already = closes(&server);
    let (response, _request_side) =
        send_scripted(&mut client, server.addr, &[8192], end, post).await;
    let mut body = response.await.unwrap().into_body();
    let dropped_at = first_body_drop(&server).await;
    let got = drain_without_credit(&mut body).await;
    assert!(got < 8192, "the client received the whole body ({got} B)");
    let cut_at = close_after(&server, already, Duration::from_secs(20))
        .await
        .expect("a zombie response never had its connection cut");
    assert_tail_cut_timing(
        cut_at,
        dropped_at,
        WINDOW,
        &format!("the zombie (post {post}, tunnel {tunnel}, {end:?})"),
    );
    drop(client);
    let drain = server.finish().await;
    assert_eq!(drain.stall_cuts, only_ended_unflushed(1), "{drain:?}");
    assert_eq!(drain.dropped, 0, "{drain:?}");
    assert_eq!(drain.aborted, 0, "{drain:?}");
    assert_eq!(drain.panicked, 0, "{drain:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_get_zombie_is_cut_as_a_connection() {
    zombie_is_cut_as_a_connection(false, false, BodyEnd::Final).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_get_zombie_is_cut_as_a_connection_with_a_tunnel_on_it() {
    zombie_is_cut_as_a_connection(false, true, BodyEnd::Final).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_post_zombie_whose_request_side_is_open_is_cut_and_ends_on_the_io_failure() {
    zombie_is_cut_as_a_connection(true, false, BodyEnd::Final).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_post_zombie_with_a_tunnel_is_cut_and_ends_on_the_io_failure() {
    zombie_is_cut_as_a_connection(true, true, BodyEnd::Final).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_body_that_ends_with_an_empty_data_frame_is_cut_as_a_connection() {
    zombie_is_cut_as_a_connection(false, false, BodyEnd::Empty).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_body_that_ends_with_trailers_is_cut_as_a_connection() {
    zombie_is_cut_as_a_connection(false, false, BodyEnd::Trailers).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_final_chunk_waiting_at_zero_credit_is_cut_as_a_stream_and_the_tunnel_survives() {
    let server = serve().await;
    let mut client = server.connect(SMALL_WINDOW, 1 << 20).await;
    let _tunnel = server.open_tunnel(&mut client).await;
    wait_for_closes(&server, 1).await;
    let already = closes(&server);
    let t0 = Instant::now();
    // Two 1 KiB chunks behind a 1 KiB window: the first is accepted, the FINAL one waits for
    // credit the client never returns, so the body has not ended and the marker acts, not the
    // table.
    let (response, _) = send_scripted(
        &mut client,
        server.addr,
        &[1024, 1024],
        BodyEnd::Final,
        false,
    )
    .await;
    let mut body = response.await.unwrap().into_body();
    let (end, _) = read_unreleased(&mut body, Duration::from_secs(10)).await;
    assert_eq!(end, PacedEnd::Reset(Reason::CANCEL));
    assert_cut_timing(t0.elapsed(), "the stalled final chunk");
    // The connection stays up for at least three more windows and the tunnel still lives.
    tokio::time::sleep(WINDOW * 3).await;
    assert_eq!(closes(&server), already, "the connection was cut");
    assert_eq!(server.ws.live_connection_count(), 1, "the tunnel was cut");
    assert!(!client.driver.is_finished());
    drop(client);
    let drain = server.finish().await;
    assert_eq!(
        drain.stall_cuts,
        StallCuts {
            stream: 1,
            ..StallCuts::default()
        },
        "{drain:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_body_with_more_than_one_chunk_past_the_window_is_cut_as_a_stream_and_not_a_connection() {
    // h2 lets hyper hand over a chunk only while the stream's send buffer holds less than the
    // client's window, so behind a 1 KiB window the second chunk waits at zero capacity: the body
    // has not ended, the stream marker acts, and nothing is left queued for the table to track. The
    // 80 KiB body is one frame slice per 16 KiB chunk, so only its first slice is ever handed over.
    for sizes in [&[16 * 1024, 1024][..], &[80 * 1024][..]] {
        let server = serve().await;
        let mut client = server.connect(SMALL_WINDOW, 1 << 20).await;
        let (response, _) =
            send_scripted(&mut client, server.addr, sizes, BodyEnd::Final, false).await;
        let mut body = response.await.unwrap().into_body();
        let (end, _) = read_unreleased(&mut body, Duration::from_secs(10)).await;
        assert_eq!(end, PacedEnd::Reset(Reason::CANCEL), "{sizes:?}");
        drop(client);
        let drain = server.finish().await;
        assert_eq!(
            drain.stall_cuts,
            StallCuts {
                stream: 1,
                ..StallCuts::default()
            },
            "{sizes:?}: {drain:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tail_the_reader_drains_inside_the_window_is_not_cut() {
    // The unwritten tail of an ended response is at most one chunk past the client's window: h2
    // lets hyper hand over a chunk only while the send buffer holds less than the window. So the
    // tail is at most one floor, and `min(floor, remaining)` is what remains. 8 KiB is a tail
    // below the floor; 16 KiB is a single full chunk, whose tail after the window's first KiB is
    // just below the floor.
    for size in [8192usize, 16 * 1024] {
        let server = serve().await;
        let mut client = server.connect(SMALL_WINDOW, 1 << 20).await;
        let already = closes(&server);
        let (response, _) =
            send_scripted(&mut client, server.addr, &[size], BodyEnd::Final, false).await;
        let mut body = response.await.unwrap().into_body();
        first_body_drop(&server).await;
        tokio::time::sleep(WINDOW.mul_f64(0.3)).await;
        let paced = paced_read(&mut body, 10_000_000, Duration::from_secs(10)).await;
        assert_eq!(paced.end, PacedEnd::Ended, "{size}: {paced:?}");
        assert_eq!(paced.got, size as u64);
        tokio::time::sleep(WINDOW * 3).await;
        assert_eq!(
            closes(&server),
            already,
            "a drained {size} B tail cost the connection"
        );
        drop(client);
        let drain = server.finish().await;
        assert_eq!(drain.stall_cuts, StallCuts::default(), "{drain:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tail_the_reader_drains_only_in_part_is_cut_at_the_end_of_the_window() {
    let window = PARTIAL_TAIL_WINDOW;
    assert_eq!(
        EGRESS_TICKS_PER_WINDOW, 4,
        "the release at five eighths of a window leaves three eighths on each side of the cut interval only while the sampler takes four ticks per window"
    );
    for size in [8192usize, 16 * 1024] {
        let server = serve_at(window).await;
        let mut client = server.connect(SMALL_WINDOW, 1 << 20).await;
        let already = closes(&server);
        let (response, _) =
            send_scripted(&mut client, server.addr, &[size], BodyEnd::Final, false).await;
        let mut body = response.await.unwrap().into_body();
        let dropped_at = first_body_drop(&server).await;
        // One KiB arrives with the window; five eighths of a window later it is released so a
        // second KiB flows, then the reader stops: 2 KiB of the tail are written, below what
        // remained, and that progress must not refresh the entry because the threshold is the
        // whole remaining tail (a tail is at most one chunk).
        let first = tokio::time::timeout(Duration::from_secs(5), body.data())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        tokio::time::sleep(window * 5 / 8).await;
        // Taken before the release: the server cannot write the second KiB earlier, so an entry
        // that the progress refreshed is cut no earlier than a whole window after this instant.
        let released_at = Instant::now();
        // Margins, with the sampler period `egress_tick(window)` (the period `ConnLimits::egress_window`
        // yields): an entry not refreshed is cut in `[dropped_at + window, dropped_at + 1.25 *
        // window]` plus scheduling lateness. The release at five eighths of a window leaves
        // three eighths of a window on each side: before the earliest unrefreshed cut, and
        // between the latest unrefreshed cut and the earliest refreshed one.
        assert!(
            released_at < dropped_at + window,
            "the release came {:?} after the body dropped, past the {window:?} window",
            released_at.duration_since(dropped_at)
        );
        body.flow_control().release_capacity(first.len()).unwrap();
        let got = drain_without_credit(&mut body).await;
        assert!(
            got > 0,
            "no byte arrived after the release, so the entry saw no progress"
        );
        assert!(
            got < size as u64 - first.len() as u64,
            "the tail was drained"
        );
        let cut_at = close_after(&server, already, Duration::from_secs(20))
            .await
            .expect("a partly drained tail was never cut");
        assert_tail_cut_timing(
            cut_at,
            dropped_at,
            window,
            &format!("a partly drained {size} B tail"),
        );
        assert!(
            cut_at < released_at + window,
            "a {size} B tail was cut {:?} after the progress: the progress refreshed the entry, or the sampler ran over 3/8 of a window late",
            cut_at.duration_since(released_at)
        );
        drop(client);
        let drain = server.finish().await;
        assert_eq!(drain.stall_cuts, only_ended_unflushed(1), "{drain:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_that_cancels_a_queued_tail_costs_the_connection_nothing() {
    let server = serve().await;
    let mut client = server.connect(SMALL_WINDOW, 1 << 20).await;
    let already = closes(&server);
    let (response, _) =
        send_scripted(&mut client, server.addr, &[8192], BodyEnd::Final, false).await;
    let body = response.await.unwrap().into_body();
    first_body_drop(&server).await;
    tokio::time::sleep(WINDOW.mul_f64(0.3)).await;
    drop(body);
    tokio::time::sleep(WINDOW * 3).await;
    assert_eq!(
        closes(&server),
        already,
        "a peer cancel cost the connection"
    );
    // The connection still serves.
    let mut second = get_path(&mut client, server.addr, "/empty")
        .await
        .into_body();
    assert!(second.data().await.is_none());
    drop(client);
    let drain = server.finish().await;
    assert_eq!(drain.stall_cuts, StallCuts::default(), "{drain:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_zombie_is_cut_while_a_sibling_stream_carries_steady_data_and_a_tunnel_echoes() {
    let server = serve().await;
    let mut client = server.connect(SMALL_WINDOW, 16 << 20).await;
    let _tunnel = server.open_tunnel(&mut client).await;
    wait_for_closes(&server, 1).await;
    let already = closes(&server);
    let (response, _) =
        send_scripted(&mut client, server.addr, &[8192], BodyEnd::Final, false).await;
    let mut zombie = response.await.unwrap().into_body();
    let dropped_at = first_body_drop(&server).await;
    drain_without_credit(&mut zombie).await;
    let mut steady = get_path(&mut client, server.addr, "/endless")
        .await
        .into_body();
    let sibling =
        tokio::spawn(
            async move { paced_read(&mut steady, 2_000_000, Duration::from_secs(8)).await },
        );
    let cut_at = close_after(&server, already, Duration::from_secs(20))
        .await
        .expect("a zombie was never cut while its sibling kept the connection busy");
    assert_tail_cut_timing(cut_at, dropped_at, WINDOW, "a zombie beside a busy sibling");
    let steady = sibling.await.unwrap();
    // The sibling's reader runs for eight seconds unless the connection ends first. Reaching its
    // deadline means the connection outlived the sibling's traffic, which is what a zombie
    // refreshed by a sibling's DATA looks like.
    assert!(
        matches!(steady.end, PacedEnd::Reset(_)),
        "the sibling read until its deadline: its DATA postponed the zombie's cut ({:?})",
        steady.end
    );
    assert!(
        steady.got >= 2 * floor(),
        "the sibling moved too little DATA ({} B) for the control to mean anything",
        steady.got
    );
    drop(client);
    let drain = server.finish().await;
    assert_eq!(drain.stall_cuts, only_ended_unflushed(1), "{drain:?}");
}

/// A raw peer that requests `path` behind a `SMALL_WINDOW` stream window, never returns credit and
/// floods PINGs about every 10 ms: the server answers each with an acknowledgement, which is
/// liveness the peer can advertise without draining anything.
async fn ping_flooded_peer(
    server: &EgressServer,
    path: &str,
) -> (
    tokio::task::JoinHandle<()>,
    std::sync::Arc<std::sync::atomic::AtomicU64>,
    std::sync::Arc<shadowcat_test_support::egress::RawSeen>,
) {
    RawPeer::get(server.addr, path, SMALL_WINDOW)
        .await
        .flood_pings(Duration::from_millis(10))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_ping_flood_does_not_postpone_the_cut_of_a_zombie_connection() {
    use std::sync::atomic::Ordering;

    let server = serve().await;
    let (flood, sent, seen) = ping_flooded_peer(&server, "/scripted?sizes=8192&end=final").await;
    let dropped_at = first_body_drop(&server).await;
    let cut_at = close_after(&server, 0, Duration::from_secs(20))
        .await
        .expect("a ping flood kept a zombie connection alive");
    assert_tail_cut_timing(cut_at, dropped_at, WINDOW, "a zombie under a ping flood");
    assert!(
        seen.pongs.load(Ordering::SeqCst) >= 50 && sent.load(Ordering::SeqCst) >= 50,
        "{} pings sent, {} acknowledged: too few for the control to mean anything",
        sent.load(Ordering::SeqCst),
        seen.pongs.load(Ordering::SeqCst)
    );
    flood.abort();
    let drain = server.finish().await;
    assert_eq!(drain.stall_cuts, only_ended_unflushed(1), "{drain:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_ping_flood_does_not_postpone_the_cut_of_a_stream_waiting_at_zero_credit() {
    use std::sync::atomic::Ordering;

    let server = serve().await;
    let started = Instant::now();
    let (flood, sent, seen) =
        ping_flooded_peer(&server, "/scripted?sizes=1024,1024&end=final").await;
    // The stream is cut alone: the connection stays open and keeps acknowledging PINGs.
    tokio::time::sleep(WINDOW * 4).await;
    assert!(
        !seen.closed.load(Ordering::SeqCst),
        "the connection was cut"
    );
    assert!(
        seen.pongs.load(Ordering::SeqCst) >= 50 && sent.load(Ordering::SeqCst) >= 50,
        "too few pings for the control to mean anything"
    );
    flood.abort();
    let drain = server.finish().await;
    assert_eq!(
        drain.stall_cuts,
        StallCuts {
            stream: 1,
            ..StallCuts::default()
        },
        "{drain:?}"
    );
    assert!(started.elapsed() >= WINDOW);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_browser_style_cancel_and_read_soak_with_a_tunnel_on_the_connection_cuts_nothing() {
    let server = serve().await;
    let mut client = server.connect(64 * 1024, 16 << 20).await;
    let _tunnel = server.open_tunnel(&mut client).await;
    wait_for_closes(&server, 1).await;
    let already = closes(&server);
    let paths = [
        "/empty",
        "/scripted?sizes=500&end=final",
        "/scripted?sizes=20000&end=final",
        "/scripted?sizes=3000,3000&end=empty",
        "/scripted?sizes=2000&end=trailers",
        "/endless",
    ];
    let mut workers = Vec::new();
    for worker in 0..6u64 {
        let mut sender = client.sender.clone();
        let addr = server.addr;
        workers.push(tokio::spawn(async move {
            let mut rng = 0x5EED_0000u64 ^ (worker + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let mut next = move || {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                rng
            };
            for _ in 0..50 {
                let path = paths[(next() % paths.len() as u64) as usize];
                let action = next() % 3;
                let request = axum::http::Request::get(format!("http://{addr}{path}"))
                    .body(())
                    .unwrap();
                sender = sender.ready().await.unwrap();
                let (response, _) = sender.send_request(request, true).unwrap();
                if action == 0 {
                    tokio::time::sleep(Duration::from_micros(next() % 40_000)).await;
                    drop(response);
                    continue;
                }
                let Ok(response) = response.await else {
                    continue;
                };
                let mut body = response.into_body();
                let mut reads = if action == 1 {
                    u32::MAX
                } else {
                    1 + (next() % 3) as u32
                };
                if path == "/endless" {
                    reads = reads.min(3);
                }
                while reads > 0 {
                    match body.data().await {
                        Some(Ok(chunk)) => {
                            let _ = body.flow_control().release_capacity(chunk.len());
                        }
                        _ => break,
                    }
                    reads -= 1;
                }
            }
        }));
    }
    for worker in workers {
        worker.await.unwrap();
    }
    // Quiescence, then a window and more: nothing is left that could still be cut.
    tokio::time::sleep(WINDOW * 2).await;
    assert_eq!(closes(&server), already, "the churn cost the connection");
    assert_eq!(server.ws.live_connection_count(), 1, "the tunnel was cut");
    drop(client);
    let drain = server.finish().await;
    assert_eq!(drain.stall_cuts, StallCuts::default(), "{drain:?}");
    assert_eq!(drain.dropped, 0, "{drain:?}");
}
