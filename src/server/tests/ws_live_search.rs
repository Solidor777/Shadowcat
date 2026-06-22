//! Live search subscriptions over real WS clients against an ephemeral in-process
//! server. A subscriber receives `search_update` when a matching readable doc
//! appears, and never receives a document it cannot read (no-op suppressed).
//!
//! The harness prelude mirrors `ws_convergence.rs` (each `tests/*.rs` is its own
//! crate, so the helpers are replicated rather than imported).

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use shadowcat::auth::password::hash_password;
use shadowcat::auth::role::ServerRole;
use shadowcat::config::Config;
use shadowcat::data::document::WorldRole;
use shadowcat::data::sqlite::SqliteRepository;
use shadowcat::http::{self, AppState};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

struct Harness {
    addr: String,
    cookie: String, // the seeded GM/owner
    world: Uuid,
    repo: Arc<SqliteRepository>,
}

async fn spawn() -> Harness {
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let hash = hash_password("pw").unwrap();
    let uid = repo
        .create_user("gm", Some(&hash), ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("test", uid, 0).await.unwrap();

    let state = AppState {
        repo: repo.clone(),
        config: Arc::new(Config::default()),
        setup_token: None,
        initialized: Arc::new(AtomicBool::new(true)),
        ws: shadowcat::ws::WsState::new(),
        upload_rate: Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
    };
    let app = http::router(state).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let cookie = login(&addr, "gm", "pw").await;
    Harness {
        addr,
        cookie,
        world: world.id,
        repo,
    }
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn login(addr: &str, username: &str, password: &str) -> String {
    let client = reqwest::Client::builder()
        .cookie_store(true)
        .build()
        .unwrap();
    let res = client
        .post(format!("http://{addr}/api/login"))
        .json(&serde_json::json!({ "username": username, "password": password }))
        .send()
        .await
        .unwrap();
    res.headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

impl Harness {
    async fn connect_with(&self, cookie: &str) -> Ws {
        let url = format!("ws://{}/ws?world={}", self.addr, self.world);
        let mut req = url.into_client_request().unwrap();
        req.headers_mut().insert("cookie", cookie.parse().unwrap());
        let (ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
        ws
    }

    /// Create a world member with `role` and return their session cookie.
    async fn add_member(&self, username: &str, role: WorldRole) -> String {
        let hash = hash_password("pw").unwrap();
        let id = self
            .repo
            .create_user(username, Some(&hash), ServerRole::User, 0)
            .await
            .unwrap();
        self.repo.add_member(self.world, id, role).await.unwrap();
        login(&self.addr, username, "pw").await
    }
}

async fn send(ws: &mut Ws, v: serde_json::Value) {
    ws.send(Message::Text(v.to_string())).await.unwrap();
}

/// An intent creating one doc with the given `name` and `permissions.default`.
fn create_intent(world: Uuid, n: u128, name: &str, default_role: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "intent",
        "intent_id": Uuid::from_u128(n),
        "ops": [{
            "op": "create",
            "doc": {
                "id": Uuid::from_u128(1000 + n),
                "scope": { "kind": "world", "world_id": world },
                "doc_type": "actor",
                "schema_version": 1,
                "permissions": { "default": default_role, "users": {}, "property_overrides": {},
                                 "capabilities": { "by_role": {}, "by_user": {} } },
                "system": { "name": name },
                "created_at": 0,
                "updated_at": 0,
            }
        }],
    })
}

/// Read frames, skipping unrelated ones, until one of `wanted` type arrives or
/// the budget elapses.
async fn next_of(ws: &mut Ws, wanted: &str, budget: Duration) -> Option<serde_json::Value> {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let Ok(Some(Ok(m))) = tokio::time::timeout(remaining, ws.next()).await else {
            return None;
        };
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(m.to_text().unwrap_or("")) {
            if v["type"].as_str() == Some(wanted) {
                return Some(v);
            }
        }
    }
}

#[tokio::test]
async fn live_subscription_pushes_update_on_matching_create() {
    let h = spawn().await;
    let pl_cookie = h.add_member("pl", WorldRole::Player).await;

    // Player subscribes to "dragon".
    let mut sub = h.connect_with(&pl_cookie).await;
    send(
        &mut sub,
        serde_json::json!({
            "type": "search", "request_id": Uuid::from_u128(1),
            "query": "dragon", "limit": 20, "cursor": null, "subscribe": true
        }),
    )
    .await;
    let initial = next_of(&mut sub, "search_result", Duration::from_secs(5))
        .await
        .expect("initial search_result");
    assert_eq!(initial["hits"].as_array().unwrap().len(), 0);

    // GM creates a readable (default observer) doc matching "dragon".
    let mut gm = h.connect_with(&h.cookie.clone()).await;
    send(&mut gm, create_intent(h.world, 1, "Red Dragon", "observer")).await;

    // Player receives a SearchUpdate containing it (after the debounce window).
    let upd = next_of(&mut sub, "search_update", Duration::from_secs(5))
        .await
        .expect("search_update");
    assert_eq!(upd["hits"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn duplicate_subscription_id_is_rejected() {
    let h = spawn().await;
    let pl_cookie = h.add_member("pl", WorldRole::Player).await;
    let mut sub = h.connect_with(&pl_cookie).await;

    let frame = serde_json::json!({
        "type": "search", "request_id": Uuid::from_u128(7),
        "query": "dragon", "limit": 20, "cursor": null, "subscribe": true
    });
    send(&mut sub, frame.clone()).await;
    next_of(&mut sub, "search_result", Duration::from_secs(5))
        .await
        .expect("initial search_result");

    // A second subscribe with the same request_id must be rejected, not silently
    // orphan the first subscription.
    send(&mut sub, frame).await;
    let err = next_of(&mut sub, "search_error", Duration::from_secs(5))
        .await
        .expect("search_error for duplicate id");
    assert!(err["message"].as_str().unwrap().contains("duplicate"));
}

#[tokio::test]
async fn burst_of_events_coalesces_without_starving() {
    let h = spawn().await;
    let pl_cookie = h.add_member("pl", WorldRole::Player).await;

    let mut sub = h.connect_with(&pl_cookie).await;
    send(
        &mut sub,
        serde_json::json!({
            "type": "search", "request_id": Uuid::from_u128(8),
            "query": "dragon", "limit": 20, "cursor": null, "subscribe": true
        }),
    )
    .await;
    next_of(&mut sub, "search_result", Duration::from_secs(5))
        .await
        .expect("initial search_result");

    // Fire a rapid burst of readable creates; leading-edge debounce must still
    // fire and reflect the final state (no starvation under a sustained stream).
    let mut gm = h.connect_with(&h.cookie.clone()).await;
    for n in 1..=3u128 {
        send(
            &mut gm,
            create_intent(h.world, n, "Bronze Dragon", "observer"),
        )
        .await;
    }

    // Drain updates until one reflects all three (or the budget elapses).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut last = 0usize;
    while tokio::time::Instant::now() < deadline {
        match next_of(&mut sub, "search_update", Duration::from_secs(2)).await {
            Some(u) => {
                last = u["hits"].as_array().unwrap().len();
                if last == 3 {
                    break;
                }
            }
            None => break,
        }
    }
    assert_eq!(
        last, 3,
        "live subscription must converge to the full burst result"
    );
}

#[tokio::test]
async fn live_subscription_never_pushes_unreadable_docs() {
    let h = spawn().await;
    let pl_cookie = h.add_member("pl", WorldRole::Player).await;

    let mut sub = h.connect_with(&pl_cookie).await;
    send(
        &mut sub,
        serde_json::json!({
            "type": "search", "request_id": Uuid::from_u128(2),
            "query": "secret", "limit": 20, "cursor": null, "subscribe": true
        }),
    )
    .await;
    next_of(&mut sub, "search_result", Duration::from_secs(5))
        .await
        .expect("initial search_result");

    // GM creates a GM-only (default none) doc matching "secret".
    let mut gm = h.connect_with(&h.cookie.clone()).await;
    send(&mut gm, create_intent(h.world, 2, "Secret Item", "none")).await;

    // No search_update: the player's top-N stays empty (no-op suppressed).
    assert!(
        next_of(&mut sub, "search_update", Duration::from_millis(800))
            .await
            .is_none(),
        "a GM-only doc must not trigger a live update for the player"
    );
}
