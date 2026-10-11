//! Fixtures for opening a WebSocket inside an HTTP/2 connection.
//!
//! Fixtures for tests that open a WebSocket inside an HTTP/2 connection: an application state with
//! one user owning one world, and the login that yields the session cookie the handshake needs.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use axum::body::Bytes;
use axum::http::Request;
use shadowcat::auth::password::hash_password;
use shadowcat::auth::role::ServerRole;
use shadowcat::config::Config;
use shadowcat::data::sqlite::SqliteRepository;
use shadowcat::http::AppState;
use shadowcat::ws::WsState;

use crate::tls::H2Client;

/// An `AppState` over an in-memory database with one user (`u` / `pw`) owning one world, and that
/// world's id.
pub async fn seeded_state() -> (AppState, uuid::Uuid) {
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let hash = hash_password("pw").unwrap();
    let uid = repo
        .create_user("u", Some(&hash), ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("test", uid, 0).await.unwrap();
    let state = AppState {
        repo,
        config: Arc::new(Config::default()),
        setup_token: None,
        initialized: Arc::new(AtomicBool::new(true)),
        ws: WsState::new(),
        upload_rate: Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
        uploads: Arc::new(shadowcat::http::assets::uploads::UploadSessions::new()),
        auth_throttle: Arc::new(shadowcat::http::throttle::AuthThrottle::new()),
        write_barrier: Arc::new(tokio::sync::RwLock::new(())),
        preview_fetch_locks: Arc::new(dashmap::DashMap::new()),
        file_permits: Arc::new(shadowcat::http::assets::permits::StreamedFilePermits::production()),
        tls: shadowcat::tls::TlsRuntime::disabled(),
    };
    (state, world.id)
}

/// Logs in as `u` over `client` and returns the session cookie (`name=value`).
pub async fn login(client: &mut H2Client) -> String {
    client.sender = client.sender.clone().ready().await.unwrap();
    let req = Request::post("http://test/api/login")
        .header("content-type", "application/json")
        .body(())
        .unwrap();
    let (response, mut body) = client.sender.send_request(req, false).unwrap();
    body.send_data(
        Bytes::from_static(br#"{"username":"u","password":"pw"}"#),
        true,
    )
    .unwrap();
    let response = response.await.unwrap();
    assert!(
        response.status().is_success(),
        "login: {}",
        response.status()
    );
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
