//! `http::conn_serve` with HTTP/2 connections: the idle bound, the lease that keeps a connection
//! busy, and a shutdown drain that is bounded and tells sessions to end first. Driven over plain
//! TCP with prior-knowledge HTTP/2 (`shadowcat_test_support::h2c`), so the serve loop and hyper's
//! HTTP/2 builder are the real ones.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::Extension;
use axum::http::Request;
use axum::routing::get;
use axum::Router;
use shadowcat::http::conn_serve::{serve_router, ConnLease, ConnLimits, ServeEnd, ServeOutcome};
use shadowcat_test_support::h2::client::Builder as H2Builder;
use shadowcat_test_support::h2c::{h2c_connect, EndHandle, H2cSource};
use shadowcat_test_support::tls::H2Client;
use tokio::net::TcpListener;
use tokio::sync::{oneshot, Notify};
use tokio::task::JoinHandle;
use tokio::time::timeout;

/// The routes the tests exercise. `/hang` and `/session` signal `entered` when a handler starts,
/// so a test waits for the request to be in flight instead of sleeping.
fn app(session: Arc<Notify>, entered: Arc<Notify>) -> Router {
    let (hang_entered, session_entered) = (entered.clone(), entered);
    Router::new()
        .route("/ok", get(|| async { "ok" }))
        // Detaches work holding the connection's lease, the way a WebSocket session does.
        .route(
            "/detach",
            get(|Extension(lease): Extension<ConnLease>| async move {
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                    drop(lease);
                });
                "ok"
            }),
        )
        .route(
            "/hang",
            get(move || {
                let entered = hang_entered.clone();
                async move {
                    entered.notify_one();
                    std::future::pending::<&'static str>().await
                }
            }),
        )
        // Ends only when the drain hook fires, like a WebSocket session told to close.
        .route(
            "/session",
            get(move || {
                let n = session.clone();
                let entered = session_entered.clone();
                async move {
                    entered.notify_one();
                    n.notified().await;
                    "bye"
                }
            }),
        )
        // A response body that takes about two seconds to finish.
        .route(
            "/stream",
            get(|| async {
                Body::from_stream(futures_util::stream::unfold(0u8, |i| async move {
                    if i == 5 {
                        return None;
                    }
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    Some((
                        Ok::<_, std::convert::Infallible>(Bytes::from_static(b"x")),
                        i + 1,
                    ))
                }))
            }),
        )
}

/// A running serve loop and the handles that stop it.
struct Served {
    /// The bound address.
    addr: std::net::SocketAddr,
    /// Ends the source.
    end: EndHandle,
    /// Resolves the shutdown future.
    shutdown: oneshot::Sender<()>,
    /// The serve loop.
    task: JoinHandle<ServeOutcome>,
    /// How many times the drain hook ran.
    drained: Arc<AtomicUsize>,
    /// Signalled when a `/hang` or `/session` handler starts.
    entered: Arc<Notify>,
}

impl Served {
    /// Waits until a `/hang` or `/session` handler is running.
    async fn request_in_flight(&self) {
        timeout(Duration::from_secs(10), self.entered.notified())
            .await
            .expect("the request never reached its handler");
    }
}

async fn start(limits: ConnLimits) -> Served {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (source, end) = H2cSource::new(listener);
    let (shutdown, shutdown_rx) = oneshot::channel::<()>();
    let session = Arc::new(Notify::new());
    let entered = Arc::new(Notify::new());
    let drained = Arc::new(AtomicUsize::new(0));
    let (hook_count, hook_session) = (drained.clone(), session.clone());
    let task = tokio::spawn(serve_router(
        source,
        app(session, entered.clone()),
        async move {
            let _ = shutdown_rx.await;
        },
        limits,
        move || {
            hook_count.fetch_add(1, Ordering::SeqCst);
            hook_session.notify_one();
        },
    ));
    Served {
        addr,
        end,
        shutdown,
        task,
        drained,
        entered,
    }
}

async fn connect(s: &Served) -> H2Client {
    h2c_connect(s.addr, H2Builder::new()).await.unwrap()
}

/// A request sender of a test client.
type Sender = shadowcat_test_support::h2::client::SendRequest<Bytes>;

/// GETs `path` on `sender` and returns the body.
async fn get_body(sender: &mut Sender, path: &str) -> std::io::Result<String> {
    *sender = sender
        .clone()
        .ready()
        .await
        .map_err(std::io::Error::other)?;
    let req = Request::get(format!("http://test{path}")).body(()).unwrap();
    let (response, _) = sender
        .send_request(req, true)
        .map_err(std::io::Error::other)?;
    let response = response.await.map_err(std::io::Error::other)?;
    let mut body = response.into_body();
    let mut out = Vec::new();
    while let Some(chunk) = body.data().await {
        let chunk = chunk.map_err(std::io::Error::other)?;
        let _ = body.flow_control().release_capacity(chunk.len());
        out.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

fn idle_limits() -> ConnLimits {
    ConnLimits {
        h2_idle: Duration::from_millis(300),
        ..ConnLimits::default()
    }
}

#[tokio::test]
async fn an_idle_h2_connection_is_closed_after_the_idle_bound() {
    let s = start(idle_limits()).await;
    let client = connect(&s).await;
    let started = Instant::now();
    timeout(Duration::from_secs(5), client.driver)
        .await
        .expect("the server kept an idle HTTP/2 connection open")
        .unwrap();
    assert!(
        started.elapsed() >= Duration::from_millis(250),
        "the connection closed before the idle bound: {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_connection_whose_lease_is_held_is_busy_and_closes_once_it_is_released() {
    let s = start(idle_limits()).await;
    let mut client = connect(&s).await;
    assert_eq!(get_body(&mut client.sender, "/detach").await.unwrap(), "ok");
    // Three idle bounds later the lease is still held, so the connection still takes requests.
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert_eq!(
        get_body(&mut client.sender, "/ok").await.unwrap(),
        "ok",
        "a connection with a live lease was treated as idle"
    );
    timeout(Duration::from_secs(5), client.driver)
        .await
        .expect("the connection stayed open after its lease was released")
        .unwrap();
}

#[tokio::test]
async fn a_connection_streaming_a_response_is_busy() {
    let s = start(idle_limits()).await;
    let mut client = connect(&s).await;
    client.sender = client.sender.clone().ready().await.unwrap();
    let req = Request::get("http://test/stream").body(()).unwrap();
    let (response, _) = client.sender.send_request(req, true).unwrap();
    let mut body = response.await.unwrap().into_body();
    // Well past the idle bound, with the body still streaming, another request must be accepted.
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert_eq!(
        get_body(&mut client.sender, "/ok").await.unwrap(),
        "ok",
        "a connection streaming a response was treated as idle"
    );
    let mut total = 0;
    while let Some(chunk) = body.data().await {
        let chunk = chunk.unwrap();
        let _ = body.flow_control().release_capacity(chunk.len());
        total += chunk.len();
    }
    assert_eq!(total, 5);
}

#[tokio::test]
async fn the_drain_hook_runs_before_the_drain_so_a_session_can_end() {
    let s = start(ConnLimits {
        drain_timeout: Duration::from_secs(30),
        ..ConnLimits::default()
    })
    .await;
    let client = connect(&s).await;
    let mut sender = client.sender.clone();
    let request = tokio::spawn(async move { get_body(&mut sender, "/session").await });
    s.request_in_flight().await;
    s.end.end();
    let end = timeout(Duration::from_secs(5), s.task)
        .await
        .expect("a stream waiting on the drain hook held the drain")
        .unwrap();
    assert_eq!(end.end, ServeEnd::SourceEnded);
    assert_eq!(s.drained.load(Ordering::SeqCst), 1);
    assert_eq!(request.await.unwrap().unwrap(), "bye");
}

#[tokio::test]
async fn a_drain_that_outlasts_its_bound_closes_the_remaining_connections_without_aborting_them() {
    let s = start(ConnLimits {
        drain_timeout: Duration::from_millis(300),
        ..ConnLimits::default()
    })
    .await;
    let client = connect(&s).await;
    let mut sender = client.sender.clone();
    let _stuck = tokio::spawn(async move { get_body(&mut sender, "/hang").await });
    s.request_in_flight().await;
    s.shutdown.send(()).unwrap();
    let end = timeout(Duration::from_secs(5), s.task)
        .await
        .expect("a stream that never ends held the shutdown drain forever")
        .unwrap();
    assert_eq!(end.end, ServeEnd::Shutdown);
    assert_eq!(s.drained.load(Ordering::SeqCst), 1);
    assert_eq!(
        (end.drain.killed, end.drain.aborted, end.drain.panicked),
        (1, 0, 0),
        "the stuck connection must be closed through its IO, not aborted"
    );
    timeout(Duration::from_secs(5), client.driver)
        .await
        .expect("a killed connection stayed open")
        .unwrap();
}
