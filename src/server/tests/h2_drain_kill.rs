//! The shutdown drain deadline against HTTP/2 connections that cannot end on their own.
//!
//! The connections under test are a live WebSocket tunnel, a peer that has stopped reading a stream,
//! and a peer that has stopped reading the socket. The deadline closes the connection by failing its IO, and the
//! connection must end through that failure: `DrainReport::aborted == 0` (no task was cancelled)
//! and `DrainReport::dropped == 0` (`drive` did not have to drop the connection) are what show the
//! IO failure, and not a last resort, ended it.
//!
//! A task that panics is caught by its `JoinSet` and surfaces only as a log line, so these tests
//! install a process-global panic hook that counts every panic, and assert the count did not move.
//! That is why this is its own test binary and why its tests hold `SERIAL`: the counter is process
//! state, and a panic belongs to a test only while no other test runs.
//!
//! `a_caught_panic_is_counted` is the positive control: it proves the hook is installed and
//! counting before any other test relies on its silence.

use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::routing::get;
use shadowcat::http::conn_serve::{serve_router, ConnLimits, ServeEnd, ServeOutcome};
use shadowcat::http::router;
use shadowcat::ws::WsState;
use shadowcat_test_support::h2::client::Builder as H2Builder;
use shadowcat_test_support::h2_session::{login, seeded_state};
use shadowcat_test_support::h2c::{h2c_connect, EndHandle, H2cSource};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, Mutex as AsyncMutex};
use tokio::task::JoinHandle;
use tokio::time::timeout;

/// Every panic observed process-wide since the hook was installed.
static PANICS: AtomicUsize = AtomicUsize::new(0);

/// The message of each observed panic, for the failure report.
static PANIC_LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Serializes the tests: `PANICS` is process state.
static SERIAL: AsyncMutex<()> = AsyncMutex::const_new(());

/// Installs the counting hook once per process.
fn install_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // Chained to the previous hook so the harness still reports a failing test's panic.
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            PANICS.fetch_add(1, Ordering::SeqCst);
            if let Ok(mut log) = PANIC_LOG.lock() {
                log.push(info.to_string());
            }
            previous(info);
        }));
    });
}

/// The panics recorded so far, for an assertion message.
fn panic_log() -> String {
    PANIC_LOG.lock().map(|l| l.join("\n")).unwrap_or_default()
}

/// Drain bound short enough to reach in a test and long enough that a session which could end
/// gracefully would have.
const DRAIN: Duration = Duration::from_millis(500);

/// A serving loop over a real router plus `/flood`, and what drives it.
struct Scenario {
    /// The bound address.
    addr: std::net::SocketAddr,
    /// Keeps the source open: dropping it ends the source.
    _end: EndHandle,
    /// Resolves the shutdown future.
    shutdown: oneshot::Sender<()>,
    /// The serve loop.
    task: JoinHandle<ServeOutcome>,
    /// The session registry.
    ws: WsState,
    /// The seeded world.
    world: uuid::Uuid,
    /// Bytes `/flood` has produced.
    flooded: Arc<AtomicUsize>,
}

/// Serves the real router. With `tell_sessions` false the drain hook is a no-op: sessions are NOT
/// told to close, so the drain deadline is the only thing that can end a live tunnel. With it
/// true the hook is the server's own (`WsState::trigger_shutdown`), which leaves a tunnel whose
/// peer has stopped reading mid-close when the deadline arrives.
async fn serve(tell_sessions: bool) -> Scenario {
    let (state, world) = seeded_state().await;
    let ws = state.ws.clone();
    let hook_ws = ws.clone();
    let flooded = Arc::new(AtomicUsize::new(0));
    let counter = flooded.clone();
    let app = router(state).await.route(
        "/flood",
        get(move || {
            let counter = counter.clone();
            async move {
                Body::from_stream(futures_util::stream::repeat_with(move || {
                    counter.fetch_add(16_384, Ordering::SeqCst);
                    Ok::<_, Infallible>(Bytes::from_static(&[0u8; 16_384]))
                }))
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (source, end) = H2cSource::new(listener);
    let (shutdown, shutdown_rx) = oneshot::channel::<()>();
    let task = tokio::spawn(serve_router(
        source,
        app,
        async move {
            let _ = shutdown_rx.await;
        },
        ConnLimits {
            drain_timeout: DRAIN,
            ..ConnLimits::default()
        },
        move || {
            if tell_sessions {
                hook_ws.trigger_shutdown();
            }
        },
    ));
    Scenario {
        addr,
        _end: end,
        shutdown,
        task,
        ws,
        world,
        flooded,
    }
}

/// Shuts `s` down and asserts the drain ended by killing connection IO, with no abort and no panic
/// anywhere in the process since `panics_before`.
async fn assert_killed_cleanly(s: Scenario, panics_before: usize) {
    s.shutdown.send(()).unwrap();
    let out = timeout(Duration::from_secs(15), s.task)
        .await
        .expect("the serve loop never returned: the drain deadline did not close the connection")
        .unwrap();
    assert_eq!(out.end, ServeEnd::Shutdown);
    assert!(
        out.drain.killed >= 1,
        "the connection ended before the deadline, so the kill path was not exercised: {:?}",
        out.drain
    );
    assert_eq!(
        out.drain.aborted, 0,
        "a connection task had to be aborted: {:?}",
        out.drain
    );
    assert_eq!(
        out.drain.dropped, 0,
        "a connection had to be dropped after its IO failed: {:?}",
        out.drain
    );
    assert_eq!(out.drain.panicked, 0, "{:?}", out.drain);
    assert_eq!(
        PANICS.load(Ordering::SeqCst),
        panics_before,
        "a panic occurred during the drain:\n{}",
        panic_log()
    );
    assert!(
        s.ws.wait_for_drain(Duration::from_secs(10)).await,
        "a WebSocket session outlived its connection's kill"
    );
}

/// Opens a WebSocket tunnel inside an HTTP/2 connection built with `builder`.
async fn open_tunnel(
    s: &Scenario,
    builder: H2Builder,
) -> (
    shadowcat_test_support::tls::H2Client,
    shadowcat_test_support::tls::H2Pipe,
) {
    let mut login_client = h2c_connect(s.addr, H2Builder::new()).await.unwrap();
    let cookie = login(&mut login_client).await;
    let mut client = h2c_connect(s.addr, builder).await.unwrap();
    let (status, tunnel) = client
        .websocket(
            &s.addr.to_string(),
            &format!("/ws?world={}", s.world),
            &cookie,
        )
        .await
        .unwrap();
    assert_eq!(status, 200);
    let ws = s.ws.clone();
    timeout(Duration::from_secs(5), async move {
        while ws.live_connection_count() != 1 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the session never registered");
    (client, tunnel)
}

#[tokio::test]
async fn a_caught_panic_is_counted() {
    let _serial = SERIAL.lock().await;
    install_hook();
    let before = PANICS.load(Ordering::SeqCst);
    let joined = tokio::spawn(async { panic!("positive control") }).await;
    assert!(joined.unwrap_err().is_panic());
    assert_eq!(
        PANICS.load(Ordering::SeqCst),
        before + 1,
        "the hook did not count a caught task panic, so its silence proves nothing"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_live_tunnel_is_closed_at_the_deadline() {
    let _serial = SERIAL.lock().await;
    install_hook();
    let before = PANICS.load(Ordering::SeqCst);
    let s = serve(false).await;
    let _open = open_tunnel(&s, H2Builder::new()).await;
    assert_killed_cleanly(s, before).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tunnel_whose_peer_stopped_reading_the_stream_is_closed_at_the_deadline() {
    let _serial = SERIAL.lock().await;
    install_hook();
    let before = PANICS.load(Ordering::SeqCst);
    let s = serve(false).await;
    // A zero stream window: the session blocks sending its first frame.
    let mut stalled = H2Builder::new();
    stalled.initial_window_size(0);
    let _open = open_tunnel(&s, stalled).await;
    assert_killed_cleanly(s, before).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stalled_tunnel_told_to_close_is_closed_at_the_deadline() {
    let _serial = SERIAL.lock().await;
    install_hook();
    let before = PANICS.load(Ordering::SeqCst);
    let s = serve(true).await;
    let mut stalled = H2Builder::new();
    stalled.initial_window_size(0);
    let _open = open_tunnel(&s, stalled).await;
    assert_killed_cleanly(s, before).await;
}

/// One HTTP/2 frame.
fn frame(out: &mut Vec<u8>, kind: u8, flags: u8, stream: u32, payload: &[u8]) {
    let len = payload.len() as u32;
    out.extend_from_slice(&len.to_be_bytes()[1..]);
    out.push(kind);
    out.push(flags);
    out.extend_from_slice(&stream.to_be_bytes());
    out.extend_from_slice(payload);
}

/// Connects a raw HTTP/2 client that asks for `/flood` with windows large enough that only the
/// socket can stop the server, and then never reads.
async fn flood_client(addr: std::net::SocketAddr) -> TcpStream {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let mut out = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n".to_vec();
    // SETTINGS: INITIAL_WINDOW_SIZE (id 4) = 2^31 - 1.
    frame(&mut out, 0x4, 0, 0, &[0, 4, 0x7f, 0xff, 0xff, 0xff]);
    // WINDOW_UPDATE on the connection so the connection window is as large as the stream's.
    frame(&mut out, 0x8, 0, 0, &0x7fff_0000u32.to_be_bytes());
    // HEADERS, END_STREAM | END_HEADERS: :method GET, :scheme http, :path /flood (literal value
    // under the static-table name `:path`).
    let mut block = vec![0x82, 0x86, 0x04, 0x06];
    block.extend_from_slice(b"/flood");
    frame(&mut out, 0x1, 0x5, 1, &block);
    stream.write_all(&out).await.unwrap();
    stream
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_connection_blocked_writing_to_a_peer_that_stopped_reading_is_closed_at_the_deadline() {
    let _serial = SERIAL.lock().await;
    install_hook();
    let before = PANICS.load(Ordering::SeqCst);
    let s = serve(false).await;
    let _peer = flood_client(s.addr).await;
    // The flood is stalled once its producer stops being polled: the socket is full.
    let flooded = s.flooded.clone();
    timeout(Duration::from_secs(30), async move {
        let mut last = usize::MAX;
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let now = flooded.load(Ordering::SeqCst);
            if now > 0 && now == last {
                return;
            }
            last = now;
        }
    })
    .await
    .expect("the flood never stalled");
    assert_killed_cleanly(s, before).await;
}
