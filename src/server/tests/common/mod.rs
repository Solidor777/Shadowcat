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
    pub client: reqwest::Client, // cookie-jar client, already logged in
    pub user: Uuid,              // the seeded user's id
    pub world: Uuid,
    pub repo: Arc<SqliteRepository>,
}

pub async fn spawn() -> Harness {
    spawn_with(|_| {}).await
}

/// Like `spawn`, but `mutate` can tweak the `Config` before the server starts
/// (e.g. set a tiny upload cap). Uses a per-run tempdir for asset storage.
pub async fn spawn_with(mutate: impl FnOnce(&mut Config)) -> Harness {
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let hash = hash_password("pw").unwrap();
    let uid = repo
        .create_user("u", Some(&hash), ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("test", uid, 0).await.unwrap();

    let assets_dir = std::env::temp_dir().join(format!("shadowcat-assets-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&assets_dir).unwrap();
    let mut cfg = Config {
        assets_dir: Some(assets_dir.to_string_lossy().into_owned()),
        ..Config::default()
    };
    mutate(&mut cfg);

    let state = AppState {
        repo: repo.clone(),
        config: Arc::new(cfg),
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
        client,
        user: uid,
        world: world.id,
        repo,
    }
}

pub type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Minimal valid PNG (1×1) — passes magic-byte detection.
pub const PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

impl Harness {
    /// Upload `bytes` as `name` to this world; returns the raw response.
    pub async fn upload(
        &self,
        name: &str,
        content_type: &str,
        bytes: Vec<u8>,
    ) -> reqwest::Response {
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(name.to_string())
            .mime_str(content_type)
            .unwrap();
        let form = reqwest::multipart::Form::new().part("file", part);
        self.client
            .post(format!(
                "http://{}/api/worlds/{}/assets",
                self.addr, self.world
            ))
            .multipart(form)
            .send()
            .await
            .unwrap()
    }

    pub async fn connect(&self) -> Ws {
        self.connect_as(&self.cookie).await
    }

    /// Connect a WS to this world using an arbitrary session `cookie` (e.g. a player's).
    pub async fn connect_as(&self, cookie: &str) -> Ws {
        let url = format!("ws://{}/ws?world={}", self.addr, self.world);
        let mut req = url.into_client_request().unwrap();
        req.headers_mut().insert("cookie", cookie.parse().unwrap());
        let (ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
        ws
    }

    /// Create a second user, add them to this world as a Player, log them in, and return
    /// `(user_id, session-cookie)`. Exercises the per-player (non-GM) vision/fog path.
    pub async fn add_player(&self, username: &str) -> (Uuid, String) {
        let hash = hash_password("pw").unwrap();
        let uid = self
            .repo
            .create_user(username, Some(&hash), ServerRole::User, 0)
            .await
            .unwrap();
        self.repo
            .add_member(
                self.world,
                uid,
                shadowcat::data::document::WorldRole::Player,
            )
            .await
            .unwrap();
        let client = reqwest::Client::builder()
            .cookie_store(true)
            .build()
            .unwrap();
        let res = client
            .post(format!("http://{}/api/login", self.addr))
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
        (uid, cookie)
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
pub fn create_doc_op(
    world: Uuid,
    id: u128,
    parent: Option<u128>,
    doc_type: &str,
) -> serde_json::Value {
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

/// A `create` op for a token owned by `owner` at scene position `(x, y)`. Used to drive a
/// player's server-side vision (which selects tokens by `owner == user_id`).
pub fn create_owned_token_op(
    world: Uuid,
    id: u128,
    scene: u128,
    owner: Uuid,
    x: f64,
    y: f64,
) -> serde_json::Value {
    serde_json::json!({
        "op": "create",
        "doc": {
            "id": Uuid::from_u128(id),
            "scope": { "kind": "world", "world_id": world },
            "doc_type": "token",
            "schema_version": 1,
            "owner": owner,
            "parent_id": Uuid::from_u128(scene),
            "system": { "x": x, "y": y, "w": 100, "h": 100 },
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
