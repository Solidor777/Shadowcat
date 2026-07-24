//! Proof (M11c-1 checkpoint): a server-authored `message` document rides the
//! existing create -> sequence -> broadcast path over a real two-client WS
//! connection, with no message-specific transport code. Mirrors the
//! `ws_convergence.rs` harness (spawn/login/connect/add_member).

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use shadowcat::auth::password::hash_password;
use shadowcat::auth::role::ServerRole;
use shadowcat::config::Config;
use shadowcat::data::document::WorldRole;
use shadowcat::data::repository::Repository;
use shadowcat::data::sqlite::SqliteRepository;
use shadowcat::http::{self, AppState};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

struct Harness {
    addr: String,
    world: Uuid,
    repo: Arc<SqliteRepository>,
}

async fn spawn() -> Harness {
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
        ws: shadowcat::ws::WsState::new(),
        upload_rate: Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
        auth_throttle: Arc::new(shadowcat::http::throttle::AuthThrottle::new()),
    };
    let app = http::router(state).await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    Harness {
        addr,
        world: world.id,
        repo,
    }
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

impl Harness {
    async fn connect_with(&self, cookie: &str) -> Ws {
        let url = format!("ws://{}/ws?world={}", self.addr, self.world);
        let mut req = url.into_client_request().unwrap();
        req.headers_mut().insert("cookie", cookie.parse().unwrap());
        let (ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
        ws
    }

    /// Log in over HTTP and return the signed session cookie.
    async fn login(&self, username: &str, password: &str) -> String {
        let client = reqwest::Client::builder()
            .cookie_store(true)
            .build()
            .unwrap();
        let res = client
            .post(format!("http://{}/api/login", self.addr))
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

    /// Create a world member with `role` and return their session cookie.
    async fn add_member(&self, username: &str, role: WorldRole) -> String {
        let hash = hash_password("pw").unwrap();
        let id = self
            .repo
            .create_user(username, Some(&hash), ServerRole::User, 0)
            .await
            .unwrap();
        self.repo.add_member(self.world, id, role).await.unwrap();
        self.login(username, "pw").await
    }
}

/// Drain frames until one of `type` arrives (skips welcome/ping/time_pong/etc.).
async fn recv_until(ws: &mut Ws, ty: &str) -> serde_json::Value {
    loop {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .expect("timeout")
            .expect("stream ended")
            .expect("ws error");
        if let Message::Text(t) = msg {
            let v: serde_json::Value = serde_json::from_str(&t).unwrap();
            if v["type"] == ty {
                return v;
            }
        }
    }
}

/// A message posted by a Player over `send_message` is broadcast to a second
/// connection (another Player observer) as an authoritative `event` whose
/// created doc is a `message` carrying the posted content — proof that chat
/// ingest rides the existing document create/broadcast path end-to-end, with
/// no message-specific transport code.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn send_message_is_broadcast_as_message_document_event() {
    let h = spawn().await;

    let cookie_p = h.add_member("player", WorldRole::Player).await;
    let cookie_o = h.add_member("observer", WorldRole::Player).await;

    let mut ws_p = h.connect_with(&cookie_p).await;
    let mut ws_o = h.connect_with(&cookie_o).await;
    recv_until(&mut ws_p, "welcome").await;
    recv_until(&mut ws_o, "welcome").await;

    ws_p.send(Message::Text(
        serde_json::json!({
            "type": "send_message",
            "request_id": Uuid::new_v4(),
            "channel": "all",
            "content": "hello",
            "actor_owner": null,
        })
        .to_string(),
    ))
    .await
    .unwrap();

    let evt = recv_until(&mut ws_o, "event").await;
    let op = &evt["command"]["ops"][0];
    assert_eq!(op["op"], "create");
    assert_eq!(op["doc"]["doc_type"], "message");
    assert_eq!(op["doc"]["engine"]["channel"], "all");
    assert_eq!(
        op["doc"]["engine"]["content"][0]["kind"], "text",
        "content segment is the plain-text producer's Segment::Text"
    );
    assert_eq!(op["doc"]["engine"]["content"][0]["text"], "hello");

    // The authoritative log agrees: exactly one durable event, a message create.
    let seqs = h.repo.events_since(h.world, 0).await.unwrap();
    assert_eq!(seqs.len(), 1);
}

/// A rejected `send_message` (empty content) is surfaced to the SENDER as a
/// `chat_error` frame correlated by `request_id`, instead of vanishing silently.
/// End-to-end proof of the conn.rs dispatch + `SendMessageError` Display wiring.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rejected_send_returns_a_correlated_chat_error_to_the_sender() {
    let h = spawn().await;
    let cookie_p = h.add_member("player", WorldRole::Player).await;
    let mut ws_p = h.connect_with(&cookie_p).await;
    recv_until(&mut ws_p, "welcome").await;

    let request_id = Uuid::new_v4();
    ws_p.send(Message::Text(
        serde_json::json!({
            "type": "send_message",
            "request_id": request_id,
            "channel": "all",
            "content": "   ", // whitespace-only -> SendMessageError::Empty
            "actor_owner": null,
        })
        .to_string(),
    ))
    .await
    .unwrap();

    let err = recv_until(&mut ws_p, "chat_error").await;
    assert_eq!(err["request_id"], request_id.to_string());
    assert_eq!(err["message"], "Message cannot be empty.");

    // Nothing was persisted: the rejection never reached the authoritative log.
    let seqs = h.repo.events_since(h.world, 0).await.unwrap();
    assert!(seqs.is_empty());
}
