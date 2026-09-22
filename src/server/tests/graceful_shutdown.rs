//! Reproduces and pins the graceful-shutdown gap named in review: `axum::serve(..)
//! .with_graceful_shutdown(..)` resolves once ordinary HTTP connections drain, NOT once a live
//! WebSocket connection's task has finished exiting -- `WebSocketUpgrade::on_upgrade` spawns
//! that task detached from the hyper connection future that produced the 101 handshake. Drives
//! `shadowcat::http::serve` (the testable wrapper that adds a post-`axum::serve` drain of live
//! WebSocket tasks) over a REAL `TcpListener` with a REAL `tokio_tungstenite` client, so this
//! observes the actual wire behavior rather than an in-process mock.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use shadowcat::auth::password::hash_password;
use shadowcat::auth::role::ServerRole;
use shadowcat::config::Config;
use shadowcat::data::sqlite::SqliteRepository;
use shadowcat::http::{self, AppState};
use shadowcat::ws::WsState;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

/// Build a live-DB-backed `AppState` + world + logged-in cookie, mirroring the shared pattern in
/// `tests/ws_convergence.rs` (this file predates the shared harness's WS-connect helper and does
/// not need its asset/upload fixtures, so it stays self-contained rather than pulling that in).
async fn seeded_state() -> (AppState, String, String) {
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let hash = hash_password("pw").unwrap();
    let uid = repo
        .create_user("u", Some(&hash), ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("test", uid, 0).await.unwrap();

    let state = AppState {
        repo: repo.clone(),
        config: Arc::new(Config::default()),
        setup_token: None,
        initialized: Arc::new(AtomicBool::new(true)),
        ws: WsState::new(),
        upload_rate: Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
        uploads: Arc::new(shadowcat::http::assets::uploads::UploadSessions::new()),
        auth_throttle: Arc::new(shadowcat::http::throttle::AuthThrottle::new()),
        write_barrier: Arc::new(tokio::sync::RwLock::new(())),
        preview_fetch_locks: Arc::new(dashmap::DashMap::new()),
    };
    (state, "u".to_string(), world.id.to_string())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn graceful_shutdown_delivers_a_close_frame_to_a_live_websocket_before_serve_returns() {
    let (state, username, world) = seeded_state().await;
    let ws_state = state.ws.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    // Mirrors `main::shutdown_signal`'s own shape: the future `axum::serve(..)
    // .with_graceful_shutdown` is given must itself call `WsState::trigger_shutdown()` before
    // resolving -- `http::serve` does not do this on the caller's behalf, since a caller with no
    // WebSocket sessions to notify (or a caller composing several shutdown triggers) may not
    // want it invoked automatically.
    let trigger_ws = ws_state.clone();
    let shutdown_fut = async move {
        let _ = shutdown_rx.await;
        trigger_ws.trigger_shutdown();
    };
    let serve_task = tokio::spawn(http::serve(listener, state, shutdown_fut));

    // Log in over HTTP, then open a real WS connection carrying that session.
    let client = reqwest::Client::builder()
        .cookie_store(true)
        .build()
        .unwrap();
    let res = client
        .post(format!("http://{addr}/api/login"))
        .json(&serde_json::json!({ "username": username, "password": "pw" }))
        .send()
        .await
        .unwrap();
    let cookie = res
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    let url = format!("ws://{addr}/ws?world={world}");
    let mut req = url.into_client_request().unwrap();
    req.headers_mut().insert("cookie", cookie.parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    // Drain the Welcome frame so the connection is fully established before shutdown fires.
    let welcome = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("welcome frame timed out")
        .expect("stream ended before welcome")
        .unwrap();
    assert!(matches!(welcome, Message::Text(_)));

    // The connection is registered against `WsState::live_connections` by now (the guard is
    // acquired synchronously inside `ws_handler`, before the 101 upgrade response is even
    // returned — see the sibling test below for the narrower window this closes).
    assert_eq!(
        ws_state.live_connection_count(),
        1,
        "connection guard should be live once Welcome has been delivered"
    );

    // Trigger shutdown, then IMMEDIATELY await `serve_task` -- no intervening wait for the
    // close frame on the client side. This ordering is the crux of the reproduction: before the
    // `WsState::wait_for_drain` fix, `axum::serve(..).with_graceful_shutdown(..)` resolves as
    // soon as ordinary HTTP connections drain, which races independently of (and, empirically,
    // resolves before) `handle_socket`'s spawned task noticing the shutdown signal and sending
    // its Close frame -- so `serve_task` returning is NOT evidence the close was ever sent. The
    // fix's contract is that `serve()` itself does not return until that task has exited, which
    // this checks by reading `live_connection_count()` in the same instant `serve_task`
    // resolves, with no delay in between that could let the race finish on its own.
    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), serve_task)
        .await
        .expect("serve() did not return within the drain timeout")
        .expect("serve task panicked")
        .expect("serve() returned an error");

    assert_eq!(
        ws_state.live_connection_count(),
        0,
        "the websocket connection task must have exited (dropped its guard) by the time serve() returns"
    );

    // Now that `serve()` has returned, the close frame must already be sitting in the client's
    // read buffer -- a short timeout here (rather than none at all) only absorbs the network
    // round trip of a frame that was sent before this point, not a wait for one sent after.
    let close_observed = tokio::time::timeout(Duration::from_millis(500), async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Close(_))) => return true,
                Some(Ok(_)) => continue,
                _ => return false,
            }
        }
    })
    .await
    .expect("close frame was not already available on the client after serve() returned");
    assert!(close_observed, "client did not observe a Close frame");
}

/// Pins the drain-accounting boundary itself, independent of the sibling test above: the
/// connection guard must be live as soon as the WS handshake completes, BEFORE the connection's
/// own task (`ws::conn::handle_socket`, spawned internally by `WebSocketUpgrade::on_upgrade`
/// detached from the request future that produced the 101) has necessarily run at all — the
/// sibling test only checks this AFTER draining a Welcome frame, which requires that task to
/// have already run substantially far regardless of where the guard is acquired, so it cannot
/// distinguish "acquired in `ws_handler`" from "acquired at the top of `handle_socket`".
///
/// Forcing the actual race deterministically (observing a `live_connection_count()` of 0 on the
/// OLD code, where the guard was acquired inside the spawned task) is not possible from outside
/// axum: `WebSocketUpgrade::on_upgrade`'s internal `tokio::spawn` exposes no hook to delay the
/// spawned task's first poll, and the window between "101 response returned" and "spawned task
/// first polled" is typically sub-microsecond on an idle multi-threaded runtime, so a timing
/// probe would be flaky in both directions (occasionally "catching" the gap on old code, more
/// often not) rather than a reliable regression guard. This test instead pins the STRUCTURAL
/// guarantee the fix provides: because `ws_handler` now acquires the guard before calling
/// `on_upgrade` at all, `live_connection_count()` is provably 1 by the time the caller's
/// `connect_async` future resolves, with no dependency on when the spawned task is scheduled —
/// asserted with NO intervening await (not even draining Welcome) between the handshake
/// completing and the read.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_connection_guard_is_live_before_the_upgrade_response_completes() {
    let (state, username, world) = seeded_state().await;
    let ws_state = state.ws.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let trigger_ws = ws_state.clone();
    let shutdown_fut = async move {
        let _ = shutdown_rx.await;
        trigger_ws.trigger_shutdown();
    };
    let serve_task = tokio::spawn(http::serve(listener, state, shutdown_fut));

    let client = reqwest::Client::builder()
        .cookie_store(true)
        .build()
        .unwrap();
    let res = client
        .post(format!("http://{addr}/api/login"))
        .json(&serde_json::json!({ "username": username, "password": "pw" }))
        .send()
        .await
        .unwrap();
    let cookie = res
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    let url = format!("ws://{addr}/ws?world={world}");
    let mut req = url.into_client_request().unwrap();
    req.headers_mut().insert("cookie", cookie.parse().unwrap());
    // No Welcome drain here, deliberately -- see the doc comment above.
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    assert_eq!(
        ws_state.live_connection_count(),
        1,
        "the guard must be live the instant the upgrade handshake completes, before this \
         connection's own task has necessarily run at all"
    );

    // Tear down cleanly rather than leaving the connection dangling for the rest of the test
    // binary's run.
    let _ = ws.close(None).await;
    drop(ws);
    shutdown_tx.send(()).unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(10), serve_task).await;
}
