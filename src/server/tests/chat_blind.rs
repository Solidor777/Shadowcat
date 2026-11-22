//! Integration proof: a `/blindroll` keeps the roll's formula and result from every reader but
//! the GM, on every egress path: broadcast, HTTP resync/load, and full-text search. Harness
//! mirrors `chat_audience`'s `spawn`/`add_member`/`connect_with`/`recv_until`.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

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

/// A running server over one in-memory world.
struct Harness {
    /// The listener's address.
    addr: String,
    /// The world every member belongs to.
    world: Uuid,
    /// The backing repository.
    repo: Arc<SqliteRepository>,
}

/// Spawns the server; the world owner `u` is the GM.
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
        file_permits: Arc::new(shadowcat::http::assets::permits::StreamedFilePermits::production()),
        tls: shadowcat::tls::TlsRuntime::disabled(),
        portmap: shadowcat::portmap::PortMapRuntime::disabled(),
        trash: Arc::new(shadowcat::trash::Trash::new(
            std::env::temp_dir(),
            shadowcat::trash::TrashRetention::Disabled,
        )),
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

/// A websocket client stream.
type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

impl Harness {
    /// Opens a socket authenticated by `cookie`.
    async fn connect_with(&self, cookie: &str) -> Ws {
        let url = format!("ws://{}/ws?world={}", self.addr, self.world);
        let mut req = url.into_client_request().unwrap();
        req.headers_mut().insert("cookie", cookie.parse().unwrap());
        let (ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
        ws
    }

    /// Logs `username` in and returns its session cookie.
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

    /// Creates a world member and returns its session cookie.
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

    /// `GET /api/worlds/{world}/documents?type=message` as `cookie`.
    async fn list_messages(&self, cookie: &str) -> Vec<serde_json::Value> {
        let res = reqwest::Client::new()
            .get(format!(
                "http://{}/api/worlds/{}/documents?type=message",
                self.addr, self.world
            ))
            .header("cookie", cookie)
            .send()
            .await
            .unwrap();
        res.json().await.unwrap()
    }
}

/// Drains frames until one of `ty` arrives.
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

/// Drains frames until the next `message` Create and returns its document.
async fn recv_next_message_create(ws: &mut Ws) -> serde_json::Value {
    loop {
        let evt = recv_until(ws, "event").await;
        let op = &evt["command"]["ops"][0];
        if op["op"] == "create" && op["doc"]["doc_type"] == "message" {
            return op["doc"].clone();
        }
    }
}

/// A `send_message` frame on the public frame audience.
fn send_message_frame(content: &str) -> Message {
    Message::Text(
        serde_json::json!({
            "type": "send_message",
            "request_id": Uuid::new_v4(),
            "channel": "general",
            "content": content,
            "actor_owner": null,
            "audience": { "kind": "public" },
        })
        .to_string(),
    )
}

/// A `search` frame for `query`.
fn search_frame(query: &str) -> Message {
    Message::Text(
        serde_json::json!({
            "type": "search",
            "request_id": Uuid::new_v4(),
            "query": query,
            "limit": 10,
            "cursor": null,
        })
        .to_string(),
    )
}

/// Asserts a delivered or listed roll message body carries no formula, result or source.
fn assert_blinded(doc: &serde_json::Value, path: &str) {
    let engine = &doc["engine"];
    let seg = &engine["content"][0];
    assert_eq!(seg["kind"], "roll_embed", "{path}");
    assert!(seg.get("formula").is_none(), "{path}: formula leaked");
    assert!(seg.get("outcome").is_none(), "{path}: outcome leaked");
    assert!(engine.get("source").is_none(), "{path}: source leaked");
    assert!(
        !engine.to_string().contains("7d11"),
        "{path}: formula text in the body"
    );
}

/// The roller, another player and the GM each read the blind roll through broadcast, resync and
/// search; only the GM gets the formula and result.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_blind_roll_is_hidden_from_the_roller_and_others_on_every_egress_path() {
    let h = spawn().await;
    let gm_cookie = h.login("u", "pw").await;
    let roller_cookie = h.add_member("roller", WorldRole::Player).await;
    let other_cookie = h.add_member("other", WorldRole::Player).await;

    let mut ws_gm = h.connect_with(&gm_cookie).await;
    let mut ws_roller = h.connect_with(&roller_cookie).await;
    let mut ws_other = h.connect_with(&other_cookie).await;
    recv_until(&mut ws_gm, "welcome").await;
    recv_until(&mut ws_roller, "welcome").await;
    recv_until(&mut ws_other, "welcome").await;

    ws_roller
        .send(send_message_frame("/blindroll 7d11 | xylophone"))
        .await
        .unwrap();

    // Broadcast.
    let gm_doc = recv_next_message_create(&mut ws_gm).await;
    assert_eq!(gm_doc["engine"]["content"][0]["formula"], "7d11");
    assert!(gm_doc["engine"]["content"][0]["outcome"]["total"].is_number());
    let roller_doc = recv_next_message_create(&mut ws_roller).await;
    assert_blinded(&roller_doc, "roller broadcast");
    let other_doc = recv_next_message_create(&mut ws_other).await;
    assert_blinded(&other_doc, "other broadcast");
    assert_eq!(roller_doc["engine"]["audience"]["kind"], "blind");

    // Resync / load.
    let gm_list = h.list_messages(&gm_cookie).await;
    assert_eq!(gm_list.len(), 1);
    assert_eq!(gm_list[0]["engine"]["content"][0]["formula"], "7d11");
    for (label, cookie) in [("roller", &roller_cookie), ("other", &other_cookie)] {
        let list = h.list_messages(cookie).await;
        assert_eq!(list.len(), 1, "{label} still receives the blind card");
        assert_blinded(&list[0], &format!("{label} resync"));
    }

    // Search: the formula is found by the GM alone; the public flavor is found by everyone.
    ws_gm.send(search_frame("7d11")).await.unwrap();
    let hits = recv_until(&mut ws_gm, "search_result").await;
    assert_eq!(
        hits["hits"].as_array().unwrap().len(),
        1,
        "GM finds the formula"
    );
    for (label, ws) in [("roller", &mut ws_roller), ("other", &mut ws_other)] {
        ws.send(search_frame("7d11")).await.unwrap();
        let hits = recv_until(ws, "search_result").await;
        assert_eq!(
            hits["hits"].as_array().unwrap().len(),
            0,
            "{label} must not find the formula"
        );
        ws.send(search_frame("xylophone")).await.unwrap();
        let hits = recv_until(ws, "search_result").await;
        assert_eq!(
            hits["hits"].as_array().unwrap().len(),
            1,
            "{label} finds the roll's public flavor"
        );
    }
}
