//! A WebSocket tunnel inside an HTTP/2 stream under the tunnel gate (`ws::tunnel_gate`).
//!
//! A reader that takes everything receives everything, a reader that takes a frame in less than the
//! window keeps its session across many windows, and a reader that stops takes its session down at
//! the window.
//!
//! Every test drives the condition it needs (frames are broadcast until a send is observed
//! pending), never a fixed byte count: how many frames fit before the gate engages depends on the
//! h2 buffers in between.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use shadowcat::config::Config;
use shadowcat::http::{self, conn_serve::ConnLimits};
use shadowcat::ws::protocol::{ServerMsg, WsErrorCode};
use shadowcat::ws::WsState;
use shadowcat_test_support::h2::client::Builder as H2Builder;
use shadowcat_test_support::h2_session::{login, seeded_state};
use shadowcat_test_support::h2c::{h2c_connect, H2cSource};
use shadowcat_test_support::measure::record_cut_overshoot;
use shadowcat_test_support::tls::{H2Client, H2Pipe};
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;

/// Payload bytes of one broadcast frame.
const FRAME_BYTES: usize = 60 * 1024;

/// A liveness guard for every wait: not a measured quantity (each wait ends the moment its
/// condition holds).
const DEADLINE: Duration = Duration::from_secs(30);

/// A running server with one tunnel open.
struct Live {
    /// Ends the serve loop when dropped.
    _shutdown: tokio::sync::oneshot::Sender<()>,
    /// Dropping this ends the connection source, which ends the serve loop.
    _end: shadowcat_test_support::h2c::EndHandle,
    /// The serve task.
    _serve: tokio::task::JoinHandle<anyhow::Result<()>>,
    /// The session registry.
    ws: WsState,
    /// The joined world.
    world: uuid::Uuid,
    /// The client connection, kept for the test's duration.
    _client: H2Client,
}

/// Starts a server whose WebSocket send window is `window_secs` and opens a tunnel over a client
/// with `stream_window` bytes of stream window.
async fn open(window_secs: u64, stream_window: u32) -> (Live, H2Pipe) {
    let (mut state, world) = seeded_state().await;
    state.config = Arc::new(Config {
        egress_window_secs: window_secs,
        ..Config::default()
    });
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
        ConnLimits::default(),
    ));
    let mut login_client = h2c_connect(addr, H2Builder::new()).await.unwrap();
    let cookie = login(&mut login_client).await;
    let mut builder = H2Builder::new();
    builder.initial_window_size(stream_window);
    let mut client = h2c_connect(addr, builder).await.unwrap();
    let (status, tunnel) = client
        .websocket(&addr.to_string(), &format!("/ws?world={world}"), &cookie)
        .await
        .unwrap();
    assert_eq!(status, 200);
    let live = Live {
        _shutdown: shutdown,
        _end: end,
        _serve: serve,
        ws,
        world,
        _client: client,
    };
    (live, tunnel)
}

impl Live {
    /// The session's room, once it has joined.
    async fn room(&self) -> Arc<shadowcat::ws::room::Room> {
        let deadline = Instant::now() + DEADLINE;
        loop {
            if let Some(room) = self.ws.rooms.get(self.world) {
                return room;
            }
            assert!(
                Instant::now() < deadline,
                "the session never joined its room"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}

/// One broadcast frame of `FRAME_BYTES` payload.
fn frame() -> ServerMsg {
    ServerMsg::Error {
        code: WsErrorCode::Internal,
        message: "x".repeat(FRAME_BYTES),
    }
}

/// Reads the tunnel to its end in `step`-byte reads with `pause` between them, counting bytes;
/// stops reading (without ending the stream) while `stopped` is set.
fn reader(
    mut tunnel: H2Pipe,
    step: usize,
    pause: Duration,
    stopped: Arc<AtomicBool>,
) -> (Arc<AtomicU64>, tokio::task::JoinHandle<()>) {
    let received = Arc::new(AtomicU64::new(0));
    let counted = received.clone();
    let task = tokio::spawn(async move {
        let mut buf = vec![0u8; step];
        loop {
            while stopped.load(Ordering::Relaxed) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            match tunnel.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => {
                    counted.fetch_add(n as u64, Ordering::Relaxed);
                }
            }
            tokio::time::sleep(pause).await;
        }
    });
    (received, task)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reader_that_takes_everything_receives_every_frame_and_the_gate_releases() {
    const FRAMES: usize = 100;
    let (live, tunnel) = open(60, 6 << 20).await;
    let (received, _task) = reader(
        tunnel,
        64 * 1024,
        Duration::ZERO,
        Arc::new(AtomicBool::new(false)),
    );
    let room = live.room().await;
    // The Welcome is sent after the session subscribed to the room's broadcasts, so a frame
    // broadcast once it has arrived is not lost to a late subscription.
    let deadline = Instant::now() + DEADLINE;
    while received.load(Ordering::Relaxed) == 0 {
        assert!(
            Instant::now() < deadline,
            "the session never sent its Welcome"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    // A burst larger than a send buffer many times over, below the room's ring so no frame lags.
    for _ in 0..FRAMES {
        room.broadcast_aux(frame());
    }
    while received.load(Ordering::Relaxed) < (FRAMES * FRAME_BYTES) as u64 {
        assert!(
            Instant::now() < deadline,
            "the reader received {} of at least {} bytes",
            received.load(Ordering::Relaxed),
            FRAMES * FRAME_BYTES
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    while live.ws.pending_sends() != 0 {
        assert!(Instant::now() < deadline, "the gate never released");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(
        live.ws.live_connection_count(),
        1,
        "a reader that kept up lost its session"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reader_that_takes_a_frame_inside_the_window_keeps_its_session_and_one_that_stops_loses_it(
) {
    /// The window, in seconds.
    const WINDOW_SECS: u64 = 2;
    /// Bytes the paced reader takes per step.
    const STEP: usize = 8 * 1024;
    /// The pause between the paced reader's steps.
    const PAUSE: Duration = Duration::from_millis(100);
    // Premise: a send waits for about one frame's worth of the wire to drain before the gate lets
    // the next in, so the longest wait is a frame over the reader's rate. It must sit well inside
    // the window, or a session ending here would be the window firing on a reader that never
    // stopped.
    let rate = STEP as f64 / PAUSE.as_secs_f64();
    let longest_wait = FRAME_BYTES as f64 / rate;
    assert!(
        longest_wait < WINDOW_SECS as f64 / 2.0,
        "the paced reader is too slow for the window: a frame takes {longest_wait} s"
    );

    let (live, tunnel) = open(WINDOW_SECS, 32 * 1024).await;
    let stopped = Arc::new(AtomicBool::new(false));
    let (received, _task) = reader(tunnel, STEP, PAUSE, stopped.clone());
    let room = live.room().await;

    // Three windows of continuous broadcasting at more than the reader takes.
    let started = Instant::now();
    let mut saw_pending = false;
    while started.elapsed() < Duration::from_secs(3 * WINDOW_SECS) {
        room.broadcast_aux(frame());
        saw_pending |= live.ws.pending_sends() >= 1;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(
        saw_pending,
        "the reader never held the gate: nothing was tested"
    );
    assert_eq!(
        live.ws.live_connection_count(),
        1,
        "a reader taking a frame in {longest_wait} s lost its session inside a {WINDOW_SECS} s window"
    );
    assert!(received.load(Ordering::Relaxed) > 0);

    // The reader stops; the frames already broadcast leave the session blocked.
    stopped.store(true, Ordering::Relaxed);
    let stopped_at = Instant::now();
    let deadline = stopped_at + DEADLINE;
    while live.ws.live_connection_count() != 0 {
        assert!(
            Instant::now() < deadline,
            "a reader that stopped taking data kept its session for {DEADLINE:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    record_cut_overshoot(
        "h2 tunnel gate ends a session whose reader stopped",
        stopped_at.elapsed(),
        Duration::from_secs(WINDOW_SECS),
    );
    assert_eq!(
        live.ws.pending_sends(),
        0,
        "the ended session left its send counted"
    );
}
