//! Ingress-guard integration test: a client `Intent` that authors a `message`
//! doc directly (bypassing `SendMessage`) is rejected, never reaching
//! `apply_intent` — the security half of server-authoritative chat ingest.
//! Harness mirrors `ws_convergence`'s `spawn`/`connect`/`intent_msg` pattern.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use shadowcat::auth::password::hash_password;
use shadowcat::auth::role::ServerRole;
use shadowcat::config::Config;
use shadowcat::data::repository::Repository;
use shadowcat::data::sqlite::SqliteRepository;
use shadowcat::http::{self, AppState};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

struct Harness {
    addr: String,
    cookie: String,
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
        uploads: Arc::new(shadowcat::http::assets::uploads::UploadSessions::new()),
        auth_throttle: Arc::new(shadowcat::http::throttle::AuthThrottle::new()),
        write_barrier: Arc::new(tokio::sync::RwLock::new(())),
        preview_fetch_locks: Arc::new(dashmap::DashMap::new()),
    };
    let app = http::router(state).await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::builder()
        .cookie_store(true)
        .build()
        .unwrap();
    let res = client
        .post(format!("http://{addr}/api/login"))
        .json(&serde_json::json!({ "username": "u", "password": "pw" }))
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

    Harness {
        addr,
        cookie,
        world: world.id,
        repo,
    }
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

impl Harness {
    async fn connect(&self) -> Ws {
        let url = format!("ws://{}/ws?world={}", self.addr, self.world);
        let mut req = url.into_client_request().unwrap();
        req.headers_mut()
            .insert("cookie", self.cookie.parse().unwrap());
        let (ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
        ws
    }

    async fn authoritative_seqs(&self) -> Vec<i64> {
        self.repo
            .events_since(self.world, 0)
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.command.seq)
            .collect()
    }
}

/// An `Intent` frame: correlation id from `intent_n`, carrying `ops`.
fn intent_msg(intent_n: u64, ops: serde_json::Value) -> Message {
    Message::Text(
        serde_json::json!({
            "type": "intent",
            "intent_id": Uuid::from_u128(intent_n as u128),
            "ops": ops,
        })
        .to_string(),
    )
}

/// A `create` op authoring a `message` doc directly — the client path this
/// guard must reject (only `handle_send_message` may author these).
fn create_message_op(world: Uuid, doc_id: Uuid) -> serde_json::Value {
    serde_json::json!({
        "op": "create",
        "doc": {
            "id": doc_id,
            "scope": { "kind": "world", "world_id": world },
            "doc_type": "message",
            "schema_version": 1,
            "system": {
                "channel": "all",
                "user_owner": Uuid::from_u128(1),
                "kind": "normal",
                "content": [],
            },
            "created_at": 0,
            "updated_at": 0,
        }
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn client_authored_message_create_is_rejected() {
    let h = spawn().await;
    let mut ws = h.connect().await;
    let _ = ws.next().await; // Welcome

    let doc_id = Uuid::from_u128(9000);
    ws.send(intent_msg(
        1,
        serde_json::json!([create_message_op(h.world, doc_id)]),
    ))
    .await
    .unwrap();

    // Expect exactly a reject frame, never an authored event.
    let Ok(Some(Ok(m))) = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await
    else {
        panic!("no frame received");
    };
    let frame: serde_json::Value = serde_json::from_str(m.to_text().unwrap()).unwrap();
    assert_eq!(frame["type"], "reject");
    assert_eq!(frame["reason"], "forbidden");

    // No event was broadcast; the log holds only the join-time config seed.
    assert_eq!(h.authoritative_seqs().await, vec![1]);
}
