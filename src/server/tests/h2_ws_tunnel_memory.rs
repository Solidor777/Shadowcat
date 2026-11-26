//! The memory a WebSocket tunnel inside an HTTP/2 stream pins when its peer returns no
//! flow-control credit.
//!
//! hyper's tunnel task passes every write on to h2, which buffers beyond the stream's window
//! without bound, so without a bound of its own a session keeps producing and the server's heap
//! grows with the room's traffic. The bound is `TunnelGate`: a session more than one send buffer
//! ahead of the wire is not ready for its next frame.
//!
//! The instrument is a counting global allocator, so this file holds exactly one test: another
//! test's allocations would be read as this one's. Its premises are asserted: the broadcast ring
//! is full before the baseline (so the ring's own retention is constant across the measurement),
//! and the allocator moves with the traffic it is asked to measure.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use shadowcat::http::{self, conn_serve::ConnLimits};
use shadowcat::ws::protocol::{ServerMsg, WsErrorCode};
use shadowcat_test_support::h2::client::Builder as H2Builder;
use shadowcat_test_support::h2_session::{login, seeded_state};
use shadowcat_test_support::h2c::{h2c_connect, H2cSource};
use tokio::net::TcpListener;

/// Bytes currently allocated through the global allocator.
static LIVE: AtomicUsize = AtomicUsize::new(0);

/// The system allocator with a live-byte count.
struct Counting;

// SAFETY: every method forwards to `System` with the arguments it received and only adjusts a
// counter, so `System`'s own contract carries over unchanged.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        System.dealloc(ptr, layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size >= layout.size() {
            LIVE.fetch_add(new_size - layout.size(), Ordering::Relaxed);
        } else {
            LIVE.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
        }
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Payload bytes of one broadcast frame.
const FRAME_BYTES: usize = 60 * 1024;

/// Frames broadcast before the baseline: more than the room's broadcast ring holds
/// (`BROADCAST_CAPACITY`, 256), so the ring's own retention is constant afterwards.
const WARM_FRAMES: usize = 400;

/// Frames broadcast after the baseline.
const MEASURED_FRAMES: usize = 600;

/// How long the session may take to be observed blocked: a liveness guard, not a measured
/// quantity.
const BLOCKED_DEADLINE: Duration = Duration::from_secs(20);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tunnel_whose_peer_returns_no_credit_pins_no_more_than_a_send_buffer() {
    let (state, world) = seeded_state().await;
    let ws = state.ws.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (source, _end) = H2cSource::new(listener);
    let (_shutdown, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let _serve = tokio::spawn(http::serve_with(
        source,
        state,
        async move {
            let _ = shutdown_rx.await;
        },
        ConnLimits::default(),
    ));
    let mut login_client = h2c_connect(addr, H2Builder::new()).await.unwrap();
    let cookie = login(&mut login_client).await;
    // A zero stream window: the client takes no tunnel DATA at all.
    let mut builder = H2Builder::new();
    builder.initial_window_size(0);
    let mut client = h2c_connect(addr, builder).await.unwrap();
    let (status, _tunnel) = client
        .websocket(&addr.to_string(), &format!("/ws?world={world}"), &cookie)
        .await
        .unwrap();
    assert_eq!(status, 200);
    let deadline = Instant::now() + BLOCKED_DEADLINE;
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
    let frame = ServerMsg::Error {
        code: WsErrorCode::Internal,
        message: "x".repeat(FRAME_BYTES),
    };

    for _ in 0..WARM_FRAMES {
        room.broadcast_aux(frame.clone());
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert!(
        ws.pending_sends() >= 1,
        "the session is not blocked: nothing stops it writing to a peer that takes nothing"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let baseline = LIVE.load(Ordering::Relaxed);
    for _ in 0..MEASURED_FRAMES {
        room.broadcast_aux(frame.clone());
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let grown = LIVE.load(Ordering::Relaxed).saturating_sub(baseline);

    // Unbounded buffering retains every frame the session wrote (each frame's JSON is at least
    // `FRAME_BYTES`); a gated session retains none beyond a send buffer. A quarter of what was
    // broadcast separates the two by construction.
    let sent = MEASURED_FRAMES * FRAME_BYTES;
    assert!(
        grown < sent / 4,
        "the heap grew by {grown} bytes while {sent} bytes were broadcast to a peer that takes nothing"
    );
    assert!(
        ws.pending_sends() >= 1,
        "the blocked session stopped being pending"
    );
}
