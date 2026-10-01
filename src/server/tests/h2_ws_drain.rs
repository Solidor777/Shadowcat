//! Shutdown of `http::serve_with` while a WebSocket lives inside an HTTP/2 connection (RFC 8441
//! extended CONNECT). Such a session is a stream of its connection, not a detached upgrade as
//! over HTTP/1.1, so a drain that waits on connections must first tell sessions to close, and
//! must not wait forever on a peer that has stopped reading.

use std::time::Duration;

use axum::http::Request;
use shadowcat::http::conn_serve::ConnLimits;
use shadowcat::http::{self, SESSION_GRACE};
use shadowcat::ws::WsState;
use shadowcat_test_support::h2::client::Builder as H2Builder;
use shadowcat_test_support::h2_session::{login, seeded_state};
use shadowcat_test_support::h2c::{h2c_connect, H2cSource};
use shadowcat_test_support::tls::H2Client;
use tokio::net::TcpListener;
use tokio::time::timeout;

/// A drain bound far above what a graceful close needs, so a close that depends on the bound
/// rather than on the sessions being told to end is visible as a slow return.
const DRAIN_BOUND: Duration = Duration::from_secs(6);

/// How quickly a graceful close must finish: well under `DRAIN_BOUND`.
const GRACEFUL_WITHIN: Duration = Duration::from_secs(3);

/// The drain bound of the stalled-peer test: short, so the abort that ends it is observable.
const STALLED_DRAIN_BOUND: Duration = Duration::from_secs(1);

/// Scheduling slack on a bound derived from the serve constants.
const SLACK: Duration = Duration::from_millis(500);

/// How quickly serve must return once that bound has expired: the documented worst case
/// (`drain_timeout + SESSION_GRACE`) plus slack.
const STALLED_WITHIN: Duration = Duration::from_millis(
    STALLED_DRAIN_BOUND.as_millis() as u64
        + SESSION_GRACE.as_millis() as u64
        + SLACK.as_millis() as u64,
);

/// Default limits with `drain_timeout` set to `bound`.
fn drain_limits(bound: Duration) -> ConnLimits {
    ConnLimits {
        drain_timeout: bound,
        ..ConnLimits::default()
    }
}

/// Everything a test needs once a WebSocket is open inside an HTTP/2 connection.
struct Live {
    /// Resolves the shutdown future.
    shutdown: tokio::sync::oneshot::Sender<()>,
    /// Ends the source, as a dead acceptor would.
    end: shadowcat_test_support::h2c::EndHandle,
    /// The serve loop.
    serve: tokio::task::JoinHandle<anyhow::Result<()>>,
    /// The session registry.
    ws: WsState,
    /// The client connection.
    client: H2Client,
    /// The open tunnel, kept alive for the test's duration.
    _tunnel: shadowcat_test_support::tls::H2Pipe,
}

/// Starts `http::serve_with`, logs in and opens a WebSocket over an HTTP/2 connection built with
/// `builder`.
async fn open_websocket(builder: H2Builder, limits: ConnLimits) -> Live {
    let (state, world) = seeded_state().await;
    let ws = state.ws.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (source, end) = H2cSource::new(listener);
    let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let serve = tokio::spawn(http::serve_with(
        source,
        state,
        async move {
            let _ = shutdown_rx.await;
        },
        limits,
    ));
    let mut login_client = h2c_connect(addr, H2Builder::new()).await.unwrap();
    let cookie = login(&mut login_client).await;
    let mut client = h2c_connect(addr, builder).await.unwrap();
    let (status, tunnel) = client
        .websocket(&addr.to_string(), &format!("/ws?world={world}"), &cookie)
        .await
        .unwrap();
    assert_eq!(status, 200);
    Live {
        shutdown,
        end,
        serve,
        ws,
        client,
        _tunnel: tunnel,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_live_h2_websocket_does_not_block_serve_when_the_source_ends() {
    let live = open_websocket(H2Builder::new(), drain_limits(DRAIN_BOUND)).await;
    assert_eq!(live.ws.live_connection_count(), 1);
    let started = std::time::Instant::now();
    live.end.end();
    let result = timeout(Duration::from_secs(15), live.serve)
        .await
        .expect("serve never returned: a live h2 WebSocket held the drain")
        .unwrap();
    assert!(
        started.elapsed() < GRACEFUL_WITHIN,
        "the session was not told to close before the drain: returned after {:?}",
        started.elapsed()
    );
    assert!(
        result.is_err(),
        "a source that ended must be reported as an error"
    );
    assert_eq!(live.ws.live_connection_count(), 0);
    timeout(Duration::from_secs(5), live.client.driver)
        .await
        .expect("the client connection stayed open")
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_returns_within_the_bound_when_an_h2_client_has_stopped_reading() {
    // A zero stream window: the server can open the tunnel but can send no data on it, so the
    // session is blocked sending its first frame and never reaches its shutdown arm; only the
    // drain bound ends it.
    let mut builder = H2Builder::new();
    builder.initial_window_size(0);
    let live = open_websocket(builder, drain_limits(STALLED_DRAIN_BOUND)).await;
    let started = std::time::Instant::now();
    live.shutdown.send(()).unwrap();
    let result = timeout(Duration::from_secs(15), live.serve)
        .await
        .expect("serve never returned: a peer that stopped reading held the drain")
        .unwrap();
    assert!(
        started.elapsed() < STALLED_WITHIN,
        "serve outlasted the drain bound while a peer had stopped reading: {:?}",
        started.elapsed()
    );
    result.expect("a requested shutdown is not an error");
    assert_eq!(live.ws.live_connection_count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_live_h2_websocket_keeps_its_connection_past_the_idle_bound() {
    let mut live = open_websocket(
        H2Builder::new(),
        ConnLimits {
            h2_idle: Duration::from_millis(300),
            ..ConnLimits::default()
        },
    )
    .await;
    // Three idle bounds with the session open: the connection must still be up, and must still
    // take a request.
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert_eq!(live.ws.live_connection_count(), 1);
    live.client.sender = live
        .client
        .sender
        .clone()
        .ready()
        .await
        .expect("the connection carrying a live WebSocket was closed as idle");
    let req = Request::get("http://test/api/config").body(()).unwrap();
    let (response, _) = live.client.sender.send_request(req, true).unwrap();
    assert!(response.await.unwrap().status().is_success());
    // Once the session ends the connection is idle and closes.
    drop(live._tunnel);
    timeout(Duration::from_secs(5), live.client.driver)
        .await
        .expect("the connection stayed open after its session ended")
        .unwrap();
}
