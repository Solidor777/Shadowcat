//! A session whose peer has stopped reading must still end: in steady state, after shutdown, and
//! while a join is being refused.

use std::time::Duration;

use super::*;

/// `send_close` returns once `CLOSE_SEND_TIMEOUT` elapses, however long the sink stays unready.
#[tokio::test]
async fn send_close_gives_up_on_a_sink_that_never_becomes_ready() {
    let (out_tx, _out_rx) = mpsc::unbounded_channel::<Message>();
    let mut sink = GatedSink {
        out: out_tx,
        credits: Arc::new(Semaphore::new(0)),
        acquiring: None,
    };
    tokio::time::timeout(CLOSE_SEND_TIMEOUT * 3, send_close(&mut sink))
        .await
        .expect("send_close waited on a sink that can never send");
}

/// A running egress loop over a gated sink that has been wrapped in a `BoundedSink`.
struct Running {
    /// The egress task.
    egress: tokio::task::JoinHandle<()>,
    /// Frames the sink accepted.
    out: mpsc::UnboundedReceiver<Message>,
    /// The ingress-side sender `egress_loop` reads; held so the loop does not end on its own.
    etx: mpsc::Sender<Egress>,
    /// The shutdown signal's sender, when the test owns the channel.
    shutdown: Option<tokio::sync::watch::Sender<bool>>,
}

impl Running {
    /// Requests shutdown through the test-owned channel.
    fn request_shutdown(&self) {
        self.shutdown
            .as_ref()
            .expect("this session's shutdown channel is not owned by the test")
            .send(true)
            .unwrap();
    }
}

/// Starts `egress_loop` over a sink with `credits` permits, bounded by `stall`, and drains its
/// Welcome (which consumes one credit).
async fn running(credits: usize, stall: Duration) -> Running {
    let (shutdown, shutdown_rx) = tokio::sync::watch::channel(false);
    let mut r = running_with(credits, stall, shutdown_rx).await;
    r.shutdown = Some(shutdown);
    r
}

/// [`running`] over a caller-supplied shutdown receiver.
async fn running_with(
    credits: usize,
    stall: Duration,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> Running {
    use crate::data::document::WorldRole;

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let author = repo
        .create_user("a", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", author, 0).await.unwrap();
    let ctx = PermissionContext {
        user_id: author,
        world_role: WorldRole::Gm,
    };
    let reg = crate::ws::room::RoomRegistry::new();
    let room = reg
        .get_or_create(repo.as_ref(), world.id)
        .await
        .unwrap()
        .unwrap();
    let (rx, current_seq) = room.subscribe();
    let (out_tx, mut out) = mpsc::unbounded_channel::<Message>();
    let (etx, erx) = mpsc::channel::<Egress>(8);
    let sink = BoundedSink::new(
        GatedSink {
            out: out_tx,
            credits: Arc::new(Semaphore::new(credits)),
            acquiring: None,
        },
        shutdown_rx.clone(),
        stall,
    );
    let egress = tokio::spawn(egress_loop(
        sink,
        rx,
        erx,
        EgressConnState {
            room: room.clone(),
            repo: repo.clone(),
            ctx,
            current_seq,
            modules_dir: std::path::PathBuf::from("nonexistent-modules-dir-for-close-bound-test"),
            module_scan_cache: Arc::new(crate::modules::ModuleScanCache::new()),
            shutdown: shutdown_rx,
            last_activity: fresh_last_activity(),
        },
    ));
    let welcome = tokio::time::timeout(Duration::from_secs(5), out.recv())
        .await
        .expect("egress did not emit Welcome")
        .expect("egress sink closed before Welcome");
    assert!(msg_text(&welcome).contains("welcome"));
    Running {
        egress,
        out,
        etx,
        shutdown: None,
    }
}

/// A frame the egress loop must send to the peer.
fn frame() -> Egress {
    Egress::Frame(Arc::new(ServerMsg::Ping))
}

#[tokio::test]
async fn a_session_blocked_sending_a_frame_ends_at_the_stall_bound_without_any_shutdown() {
    // One credit: the Welcome. The next frame can never be sent.
    let mut r = running(1, Duration::from_millis(300)).await;
    r.etx.send(frame()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), &mut r.egress)
        .await
        .expect("a session whose peer stopped reading never ended")
        .unwrap();
    assert!(r.out.try_recv().is_err(), "no further frame was accepted");
}

#[tokio::test]
async fn shutdown_while_a_send_is_pending_ends_the_session_within_the_close_bound() {
    // A stall bound far above the test's patience: only the shutdown shortening can end it.
    let mut r = running(1, Duration::from_secs(600)).await;
    r.etx.send(frame()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!r.egress.is_finished(), "the session ended before shutdown");
    r.request_shutdown();
    tokio::time::timeout(CLOSE_SEND_TIMEOUT * 3, &mut r.egress)
        .await
        .expect("a pending send outlived the Close bound after shutdown")
        .unwrap();
}

#[tokio::test]
async fn an_idle_session_still_closes_with_a_close_frame_on_shutdown() {
    let mut r = running(10, Duration::from_secs(600)).await;
    r.request_shutdown();
    tokio::time::timeout(Duration::from_secs(5), &mut r.egress)
        .await
        .expect("egress did not exit after the shutdown signal")
        .unwrap();
    let mut saw_close = false;
    while let Ok(m) = r.out.try_recv() {
        saw_close |= matches!(m, Message::Close(_));
    }
    assert!(saw_close, "egress must send a Close frame before exiting");
}

/// A sink over `credits` permits, bounded by `stall`, and the channel its accepted frames reach.
fn refusal_sink(
    credits: usize,
    stall: Duration,
) -> (
    BoundedSink<GatedSink>,
    mpsc::UnboundedReceiver<Message>,
    tokio::sync::watch::Sender<bool>,
) {
    let (out_tx, out_rx) = mpsc::unbounded_channel::<Message>();
    let (tx, rx) = tokio::sync::watch::channel(false);
    let sink = BoundedSink::new(
        GatedSink {
            out: out_tx,
            credits: Arc::new(Semaphore::new(credits)),
            acquiring: None,
        },
        rx,
        stall,
    );
    (sink, out_rx, tx)
}

#[tokio::test]
async fn refusing_a_join_to_a_peer_that_is_not_reading_returns_at_the_stall_bound() {
    let (mut sink, _out, _tx) = refusal_sink(0, Duration::from_millis(300));
    tokio::time::timeout(
        Duration::from_secs(10),
        reject_join(&mut sink, WsErrorCode::Internal, "internal"),
    )
    .await
    .expect("a pre-join refusal waited on a peer that is not reading");
}

#[tokio::test]
async fn a_refused_join_sends_the_error_and_then_a_close_frame() {
    let (mut sink, mut out, _tx) = refusal_sink(10, Duration::from_secs(5));
    reject_join(&mut sink, WsErrorCode::Internal, "internal").await;
    let first = out.try_recv().expect("the error frame");
    assert!(msg_text(&first).contains("internal"));
    assert!(matches!(out.try_recv(), Ok(Message::Close(_))));
}

#[test]
fn a_receiver_cloned_after_shutdown_was_requested_still_observes_it() {
    let ws = crate::ws::WsState::new();
    ws.trigger_shutdown();
    // `handle_socket` clones the receiver from `WsState` when a session starts; one that starts
    // after the signal must still see it as unseen.
    let late = ws.shutdown.clone();
    assert!(late.has_changed().unwrap());
    assert!(*late.borrow());
}

#[tokio::test]
async fn a_session_that_starts_after_shutdown_was_requested_closes_at_once() {
    let ws = crate::ws::WsState::new();
    ws.trigger_shutdown();
    let mut r = running_with(10, Duration::from_secs(600), ws.shutdown.clone()).await;
    tokio::time::timeout(Duration::from_secs(5), &mut r.egress)
        .await
        .expect("a session started after shutdown never closed")
        .unwrap();
    let mut saw_close = false;
    while let Ok(m) = r.out.try_recv() {
        saw_close |= matches!(m, Message::Close(_));
    }
    assert!(saw_close, "the late session must send a Close frame");
}
