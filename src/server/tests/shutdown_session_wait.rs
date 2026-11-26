//! The shutdown wait for WebSocket session tasks after the connection drain has returned.
//!
//! One deadline (`drain_timeout` after the drain hook ran) bounds the whole shutdown, and sessions
//! are never given less than `SESSION_GRACE` even when the connection drain consumed the bound.
//!
//! A session task that has not ended is represented by a held `ConnectionGuard`, so the wait is
//! driven without a socket. A connection that outlasts the drain is an HTTP/1.1 request whose body
//! never finishes, which keeps the connection in flight until the deadline closes it.
//!
//! These run on the default current-thread runtime so the log capture sees every line.

use std::time::{Duration, Instant};

use shadowcat::http::{self, conn_serve::ConnLimits, SESSION_GRACE};
use shadowcat_test_support::h2_session::seeded_state;
use shadowcat_test_support::logs::capture_current_thread;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

/// The connection drain bound of these tests.
const DRAIN: Duration = Duration::from_millis(500);

/// The warning `serve_with` logs when sessions outlive their wait.
const TIMEOUT_WARNING: &str = "timed out waiting for websocket connections";

/// Room for scheduling jitter when asserting a wait's lower bound.
const JITTER: Duration = Duration::from_millis(150);

/// Room for scheduling latency when asserting a wait's upper bound. It stays below
/// `conn_serve::KILL_GRACE`: a connection that needed its kill grace would end the shutdown
/// that much later, and a slack of that size would hide it.
const UPPER_SLACK: Duration = Duration::from_millis(1500);

/// The errors `serve_with` logs when a connection outlives its failed IO (the kill-grace and abort
/// paths). A shutdown that ends on time logs neither; the drain-timeout warning is not among them
/// because the stuck request in these tests passes the drain bound by design.
const KILL_PATH_ERRORS: [&str; 2] = [
    "connection dropped: it did not end after its IO failed",
    "connection tasks did not end after their IO was failed; aborting them",
];

/// Asserts that the captured log text `text` holds no kill-path error.
fn assert_no_kill_path(text: &str) {
    for error in KILL_PATH_ERRORS {
        assert!(
            !text.contains(error),
            "the shutdown needed the kill path ({error}):\n{text}"
        );
    }
}

/// A running `serve_with`, a request that keeps its connection in flight past the drain, and the
/// instant shutdown was requested.
struct Shutdown {
    /// The serve task's result.
    result: anyhow::Result<()>,
    /// Time from the shutdown request until `serve_with` returned.
    elapsed: Duration,
}

/// Starts `serve_with`, holds a connection in flight, holds one session guard until `release`
/// (never, when `None`), requests shutdown and measures how long `serve_with` takes.
async fn shut_down(release: Option<Duration>) -> Shutdown {
    let (state, _world) = seeded_state().await;
    let session = state.ws.connection_guard();
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

    // A request whose body never completes: the connection stays in flight until it is closed.
    let mut stuck = TcpStream::connect(addr).await.unwrap();
    stuck
        .write_all(
            b"POST /api/login HTTP/1.1\r\nHost: test\r\nContent-Type: application/json\r\n\
              Content-Length: 1000\r\n\r\n{",
        )
        .await
        .unwrap();
    stuck.flush().await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    let started = Instant::now();
    shutdown_tx.send(()).unwrap();
    let releaser = tokio::spawn(async move {
        match release {
            Some(after) => {
                tokio::time::sleep(after).await;
                drop(session);
            }
            None => std::future::pending::<()>().await,
        }
    });
    let result = tokio::time::timeout(Duration::from_secs(30), serve)
        .await
        .expect("serve_with never returned")
        .unwrap();
    let elapsed = started.elapsed();
    releaser.abort();
    drop(stuck);
    Shutdown { result, elapsed }
}

#[tokio::test]
async fn a_session_that_releases_within_the_grace_after_the_deadline_is_waited_for_quietly() {
    let (_guard, log) = capture_current_thread();
    let release = DRAIN + Duration::from_secs(1);
    let s = shut_down(Some(release)).await;
    s.result.expect("a requested shutdown is not an error");
    assert!(
        s.elapsed + JITTER >= release,
        "serve_with returned before the session released: {:?}",
        s.elapsed
    );
    assert!(
        !log.text().contains(TIMEOUT_WARNING),
        "sessions that released inside the grace were reported as timed out:\n{}",
        log.text()
    );
    assert_no_kill_path(&log.text());
}

#[tokio::test]
async fn a_session_that_never_releases_ends_the_wait_at_the_drain_bound_plus_the_grace() {
    let (_guard, log) = capture_current_thread();
    let s = shut_down(None).await;
    s.result.expect("a requested shutdown is not an error");
    let expected = DRAIN + SESSION_GRACE;
    assert!(
        s.elapsed + JITTER >= expected,
        "serve_with returned before the drain bound plus the session grace: {:?} < {expected:?}",
        s.elapsed
    );
    assert!(
        s.elapsed < expected + UPPER_SLACK,
        "serve_with outlasted the drain bound plus the session grace: {:?} > {expected:?}",
        s.elapsed
    );
    assert!(
        log.text().contains(TIMEOUT_WARNING),
        "a session that never released was not reported:\n{}",
        log.text()
    );
    assert!(
        log.text().contains("blocked_sends=0"),
        "the timeout warning does not report the pending-sends gauge:\n{}",
        log.text()
    );
    assert_no_kill_path(&log.text());
}
