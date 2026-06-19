//! Shared WS integration-test harness for the scene tests: a real WS client
//! against an ephemeral in-process server whose single seeded user owns the
//! world (GM), so its intents are authorized.
#![allow(dead_code)]

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use futures_util::StreamExt;
use shadowcat::auth::password::hash_password;
use shadowcat::auth::role::ServerRole;
use shadowcat::config::Config;
use shadowcat::data::sqlite::SqliteRepository;
use shadowcat::http::{self, AppState};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

pub struct Harness {
    pub addr: String,
    pub cookie: String,
    pub world: Uuid,
    pub repo: Arc<SqliteRepository>,
}

pub async fn spawn() -> Harness {
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

pub type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

impl Harness {
    pub async fn connect(&self) -> Ws {
        let url = format!("ws://{}/ws?world={}", self.addr, self.world);
        let mut req = url.into_client_request().unwrap();
        req.headers_mut()
            .insert("cookie", self.cookie.parse().unwrap());
        let (ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
        ws
    }
}

/// An `Intent` frame keyed by `intent_n`, carrying `ops`.
pub fn intent_msg(intent_n: u64, ops: serde_json::Value) -> Message {
    Message::Text(
        serde_json::json!({
            "type": "intent",
            "intent_id": Uuid::from_u128(intent_n as u128),
            "ops": ops,
        })
        .to_string(),
    )
}

/// A `create` op for a minimal world-scoped scene-entity document. `parent` set
/// makes it a child; `None` + `doc_type == "scene"` makes it a scene.
pub fn create_doc_op(world: Uuid, id: u128, parent: Option<u128>, doc_type: &str) -> serde_json::Value {
    serde_json::json!({
        "op": "create",
        "doc": {
            "id": Uuid::from_u128(id),
            "scope": { "kind": "world", "world_id": world },
            "doc_type": doc_type,
            "schema_version": 1,
            "parent_id": parent.map(Uuid::from_u128),
            "system": {},
            "created_at": 0,
            "updated_at": 0,
        }
    })
}

/// One intent creating a scene plus the given child tokens. The scene op is
/// first so the children's parent_id FK is satisfied within the command.
pub fn create_scene_with_children(world: Uuid, scene: u128, children: &[u128]) -> Message {
    let mut ops = vec![create_doc_op(world, scene, None, "scene")];
    for &c in children {
        ops.push(create_doc_op(world, c, Some(scene), "token"));
    }
    intent_msg(1, serde_json::Value::Array(ops))
}

/// A `delete` op intent for `id`. The server substitutes the authoritative
/// stored document, so only the id is load-bearing in the wire envelope.
pub fn delete_doc(world: Uuid, id: u128) -> Message {
    intent_msg(
        2,
        serde_json::json!([{
            "op": "delete",
            "doc": {
                "id": Uuid::from_u128(id),
                "scope": { "kind": "world", "world_id": world },
                "doc_type": "scene",
                "schema_version": 1,
                "system": {},
                "created_at": 0,
                "updated_at": 0,
            }
        }]),
    )
}

/// A `scene_subscribe` frame for `channel`, keyed by `request_n`.
pub fn scene_subscribe(request_n: u64, channel: &str) -> Message {
    Message::Text(
        serde_json::json!({
            "type": "scene_subscribe",
            "request_id": Uuid::from_u128(request_n as u128),
            "channel": channel,
        })
        .to_string(),
    )
}

/// Read frames until one of type `ty` arrives (5s budget), returning it.
pub async fn drain_until_type(ws: &mut Ws, ty: &str) -> serde_json::Value {
    loop {
        let Ok(Some(Ok(m))) =
            tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await
        else {
            panic!("timed out waiting for frame of type {ty}");
        };
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(m.to_text().unwrap_or("")) {
            if v["type"].as_str() == Some(ty) {
                return v;
            }
        }
    }
}

/// Read until the next `event` frame.
pub async fn drain_until_event(ws: &mut Ws) -> serde_json::Value {
    drain_until_type(ws, "event").await
}
