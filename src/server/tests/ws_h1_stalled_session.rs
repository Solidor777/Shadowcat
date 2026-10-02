//! A WebSocket session over HTTP/1.1 whose peer has stopped reading. After the upgrade the
//! connection task has ended, so the session's only bound is its `BoundedSink`; the sink's pending
//! send is the signal `WsState::pending_sends` exposes, and a shutdown shortens that send to
//! `CLOSE_SEND_TIMEOUT` so the session ends inside the shutdown envelope.
//!
//! Every step drives the condition it needs: frames are broadcast until a send is observed
//! pending, never for a fixed count, because the byte count that fills a socket differs per OS.

use std::time::{Duration, Instant};

use shadowcat::config::Config;
use shadowcat::http::{self, conn_serve::ConnLimits, SESSION_GRACE};
use shadowcat::ws::bounded_sink::CLOSE_SEND_TIMEOUT;
use shadowcat::ws::protocol::{ServerMsg, WsErrorCode};
use shadowcat_test_support::h2_session::seeded_state;
use shadowcat_test_support::measure::record_cut_overshoot;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

/// Size of one broadcast frame: below `WS_MAX_MESSAGE_BYTES`, large enough to fill a socket in a
/// bounded number of broadcasts.
const FRAME_BYTES: usize = 60 * 1024;

/// The connection drain bound of the shutdown.
const DRAIN: Duration = Duration::from_secs(1);

/// How long the stalled session is observed alive before shutdown. Premise, asserted below: far
/// below the window, so an end inside it is not the window firing.
const SETTLE: Duration = Duration::from_millis(500);

/// How long the broadcast loop may take to put a send into the pending state; a liveness guard,
/// not a measured quantity (the loop ends the moment the gauge reads 1).
const PENDING_DEADLINE: Duration = Duration::from_secs(20);

/// Logs in over HTTP/1.1 and returns the session cookie.
async fn login(addr: std::net::SocketAddr) -> String {
    let client = reqwest::Client::builder().build().unwrap();
    let response = client
        .post(format!("http://{addr}/api/login"))
        .json(&serde_json::json!({ "username": "u", "password": "pw" }))
        .send()
        .await
        .unwrap();
    response
        .headers()
        .get("set-cookie")
        .expect("login sets a session cookie")
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_blocked_on_a_peer_that_stopped_reading_is_counted_and_ends_inside_the_shutdown_bound(
) {
    assert!(
        Config::default().egress_window() > SETTLE * 10,
        "the window must dwarf the settle interval, or an end inside it could be the window firing"
    );
    let (state, world) = seeded_state().await;
    let ws = state.ws.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let serve = tokio::spawn(http::serve_with(
        listener,
        state,
        async move {
            let _ = shutdown_rx.await;
        },
        ConnLimits {
            drain_timeout: DRAIN,
            ..ConnLimits::default()
        },
    ));

    let cookie = login(addr).await;
    let mut request = format!("ws://{addr}/ws?world={world}")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("cookie", cookie.parse().unwrap());
    // The client is held and never polled again: the peer has stopped reading.
    let (mut client, _) = tokio_tungstenite::connect_async(request).await.unwrap();

    let deadline = Instant::now() + PENDING_DEADLINE;
    let room = loop {
        if let Some(room) = ws.rooms.get(world) {
            break room;
        }
        assert!(
            Instant::now() < deadline,
            "the session never joined its room"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    assert_eq!(ws.live_connection_count(), 1);

    let frame = ServerMsg::Error {
        code: WsErrorCode::Internal,
        message: "x".repeat(FRAME_BYTES),
    };
    while ws.pending_sends() < 1 {
        assert!(
            Instant::now() < deadline,
            "no send went pending after broadcasting for {PENDING_DEADLINE:?}: the stalled peer was never observed"
        );
        room.broadcast_aux(frame.clone());
        tokio::time::sleep(Duration::from_millis(2)).await;
    }

    tokio::time::sleep(SETTLE).await;
    assert_eq!(
        ws.live_connection_count(),
        1,
        "the session ended while its peer was stalled but the window had not elapsed"
    );
    assert!(
        ws.pending_sends() >= 1,
        "the pending send cleared although the peer still reads nothing"
    );

    let started = Instant::now();
    shutdown_tx.send(()).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(30), serve)
        .await
        .expect("serve_with never returned")
        .unwrap();
    let elapsed = started.elapsed();
    result.expect("a requested shutdown is not an error");
    assert_eq!(
        ws.live_connection_count(),
        0,
        "the stalled session outlived the shutdown envelope (the Close bound did not end its send)"
    );
    assert_eq!(
        ws.pending_sends(),
        0,
        "the ended session left its send counted"
    );
    assert!(
        elapsed <= DRAIN + http::conn_serve::ABORT_WAIT + SESSION_GRACE,
        "serve_with outlasted its documented worst case: {elapsed:?}"
    );
    record_cut_overshoot(
        "ws h1 stalled session at shutdown",
        elapsed,
        CLOSE_SEND_TIMEOUT,
    );
    let _ = client.close(None).await;
}
