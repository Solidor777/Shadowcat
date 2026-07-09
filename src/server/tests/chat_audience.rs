//! Integration proof (M11c-2): a restricted-audience message (`Audience::Whisper`
//! / `Audience::GmOnly`) is invisible to anyone outside its audience on every
//! egress path — broadcast, HTTP resync/load, and full-text search — and the
//! GM's usual unconditional access is itself gated by that audience. Harness
//! mirrors `chat_delivery.rs`'s `spawn`/`add_member`/`connect_with`/`recv_until`.

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

/// Spawns the harness and returns the world-owning user's id alongside it —
/// that user is GM (per `create_world_owned`), and several tests below need
/// to address the GM by uuid (e.g. to name them in a whisper's recipients).
async fn spawn() -> (Harness, Uuid) {
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
    };
    let app = http::router(state).await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (
        Harness {
            addr,
            world: world.id,
            repo,
        },
        uid,
    )
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

    async fn add_member(&self, username: &str, role: WorldRole) -> (Uuid, String) {
        let hash = hash_password("pw").unwrap();
        let id = self
            .repo
            .create_user(username, Some(&hash), ServerRole::User, 0)
            .await
            .unwrap();
        self.repo.add_member(self.world, id, role).await.unwrap();
        (id, self.login(username, "pw").await)
    }

    /// `GET /api/worlds/{world}/documents?type=message` as `cookie` — the
    /// resync/load path a fresh connection or page load uses.
    async fn list_messages(&self, cookie: &str) -> Vec<serde_json::Value> {
        let client = reqwest::Client::new();
        let res = client
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

/// Drain frames until one of `type` arrives (skips welcome/ping/etc.).
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

/// Drain frames until the next `event` whose command carries a `message`
/// Create op, returning that op's `doc`. Used as a timeout-free absence
/// proof: send a distinguishable follow-up public message after the message
/// under test, then assert THIS returns the follow-up's doc, not the one
/// under test — proving the one under test was never delivered at all.
async fn recv_next_message_create(ws: &mut Ws) -> serde_json::Value {
    loop {
        let evt = recv_until(ws, "event").await;
        let op = &evt["command"]["ops"][0];
        if op["op"] == "create" && op["doc"]["doc_type"] == "message" {
            return op["doc"].clone();
        }
    }
}

fn send_message_frame(channel: &str, content: &str, audience: serde_json::Value) -> Message {
    Message::Text(
        serde_json::json!({
            "type": "send_message",
            "channel": channel,
            "content": content,
            "actor_owner": null,
            "audience": audience,
        })
        .to_string(),
    )
}

fn whisper_audience(recipients: &[Uuid]) -> serde_json::Value {
    serde_json::json!({ "kind": "whisper", "recipients": recipients })
}

/// A whisper reaches its named recipient but never a third, unnamed member —
/// proven by having the bystander's next observed message-create be a
/// distinguishable follow-up public message, not the whisper.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn whisper_reaches_only_the_named_recipient() {
    let (h, _gm_id) = spawn().await;
    let (_sender_id, cookie_sender) = h.add_member("sender", WorldRole::Player).await;
    let (recipient_id, cookie_recipient) = h.add_member("recipient", WorldRole::Player).await;
    let (_bystander_id, cookie_bystander) = h.add_member("bystander", WorldRole::Player).await;

    let mut ws_sender = h.connect_with(&cookie_sender).await;
    let mut ws_recipient = h.connect_with(&cookie_recipient).await;
    let mut ws_bystander = h.connect_with(&cookie_bystander).await;
    recv_until(&mut ws_sender, "welcome").await;
    recv_until(&mut ws_recipient, "welcome").await;
    recv_until(&mut ws_bystander, "welcome").await;

    ws_sender
        .send(send_message_frame(
            "whispers",
            "secret",
            whisper_audience(&[recipient_id]),
        ))
        .await
        .unwrap();
    let recipient_doc = recv_next_message_create(&mut ws_recipient).await;
    assert_eq!(recipient_doc["system"]["content"][0]["text"], "secret");

    ws_sender
        .send(send_message_frame("all", "marker", serde_json::json!({ "kind": "public" })))
        .await
        .unwrap();
    let bystander_doc = recv_next_message_create(&mut ws_bystander).await;
    assert_eq!(
        bystander_doc["system"]["content"][0]["text"], "marker",
        "the bystander's first observed message-create is the PUBLIC marker, \
         proving the whisper was never delivered to them at all"
    );
}

/// The GM does NOT see a whisper that doesn't name them — the same
/// absence-proof pattern as above, applied to the world's GM.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn whisper_excludes_the_gm_unless_named() {
    let (h, _gm_id) = spawn().await;
    let gm_cookie = h.login("u", "pw").await;
    let (_a_id, cookie_a) = h.add_member("a", WorldRole::Player).await;
    let (b_id, _cookie_b) = h.add_member("b", WorldRole::Player).await;

    let mut ws_gm = h.connect_with(&gm_cookie).await;
    let mut ws_a = h.connect_with(&cookie_a).await;
    recv_until(&mut ws_gm, "welcome").await;
    recv_until(&mut ws_a, "welcome").await;

    ws_a.send(send_message_frame(
        "whispers",
        "player-to-player secret",
        whisper_audience(&[b_id]),
    ))
    .await
    .unwrap();

    ws_a.send(send_message_frame("all", "marker", serde_json::json!({ "kind": "public" })))
        .await
        .unwrap();
    let gm_doc = recv_next_message_create(&mut ws_gm).await;
    assert_eq!(
        gm_doc["system"]["content"][0]["text"], "marker",
        "the GM's first observed message-create is the PUBLIC marker — the \
         unnamed whisper never reached them"
    );
}

/// A whisper that DOES name the GM reaches them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn whisper_reaches_the_gm_when_named() {
    let (h, gm_id) = spawn().await;
    let gm_cookie = h.login("u", "pw").await;
    let (_a_id, cookie_a) = h.add_member("a", WorldRole::Player).await;

    let mut ws_gm = h.connect_with(&gm_cookie).await;
    let mut ws_a = h.connect_with(&cookie_a).await;
    recv_until(&mut ws_gm, "welcome").await;
    recv_until(&mut ws_a, "welcome").await;

    ws_a.send(send_message_frame(
        "whispers",
        "for the GM's eyes too",
        whisper_audience(&[gm_id]),
    ))
    .await
    .unwrap();

    let gm_doc = recv_next_message_create(&mut ws_gm).await;
    assert_eq!(gm_doc["system"]["content"][0]["text"], "for the GM's eyes too");
}

/// A whisper naming a uuid that is not a world member is rejected wholesale
/// — fail-closed — and nothing is persisted (proven by a subsequent public
/// message landing at seq 1, not seq 2).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn whisper_to_unknown_recipient_is_rejected_and_nothing_persists() {
    let (h, _gm_id) = spawn().await;
    let (_sender_id, cookie_sender) = h.add_member("sender", WorldRole::Player).await;

    let mut ws_sender = h.connect_with(&cookie_sender).await;
    recv_until(&mut ws_sender, "welcome").await;

    let foreign = Uuid::from_u128(999_999);
    ws_sender
        .send(send_message_frame(
            "whispers",
            "should never persist",
            whisper_audience(&[foreign]),
        ))
        .await
        .unwrap();

    ws_sender
        .send(send_message_frame("all", "marker", serde_json::json!({ "kind": "public" })))
        .await
        .unwrap();
    let doc = recv_next_message_create(&mut ws_sender).await;
    assert_eq!(doc["system"]["content"][0]["text"], "marker");

    let seqs = h.repo.events_since(h.world, 0).await.unwrap();
    assert_eq!(
        seqs.len(),
        1,
        "only the marker was persisted — the rejected whisper consumed no seq"
    );
}

/// The recipient's own HTTP resync/load (`GET .../documents?type=message`)
/// surfaces their whisper; a non-recipient's resync/load never surfaces it —
/// the third egress path (alongside broadcast and search) the module doc
/// claims coverage for.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn whisper_is_hidden_from_a_non_recipients_resync_but_visible_to_the_recipient() {
    let (h, _gm_id) = spawn().await;
    let (_sender_id, cookie_sender) = h.add_member("sender", WorldRole::Player).await;
    let (recipient_id, cookie_recipient) = h.add_member("recipient", WorldRole::Player).await;
    let (_bystander_id, cookie_bystander) = h.add_member("bystander", WorldRole::Player).await;

    let mut ws_sender = h.connect_with(&cookie_sender).await;
    let mut ws_recipient = h.connect_with(&cookie_recipient).await;
    recv_until(&mut ws_sender, "welcome").await;
    recv_until(&mut ws_recipient, "welcome").await;

    ws_sender
        .send(send_message_frame(
            "whispers",
            "resync-only secret",
            whisper_audience(&[recipient_id]),
        ))
        .await
        .unwrap();
    recv_next_message_create(&mut ws_recipient).await; // wait for delivery before resyncing

    let recipient_docs = h.list_messages(&cookie_recipient).await;
    let matching: Vec<_> = recipient_docs
        .iter()
        .filter(|d| d["system"]["content"][0]["text"] == "resync-only secret")
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "the recipient's own resync/load surfaces exactly their whisper"
    );

    let bystander_docs = h.list_messages(&cookie_bystander).await;
    assert!(
        bystander_docs.is_empty(),
        "a non-recipient's resync/load must never surface the whisper"
    );
}

/// Search: the whisper's own recipient finds it; a non-recipient's search
/// for the same unique term returns nothing, even though the FTS index
/// contains the text (the per-hit READ-gate filter, not the index split,
/// enforces this — see design doc §1).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn whisper_content_is_hidden_from_a_non_recipient_search() {
    let (h, _gm_id) = spawn().await;
    let (_sender_id, cookie_sender) = h.add_member("sender", WorldRole::Player).await;
    let (recipient_id, cookie_recipient) = h.add_member("recipient", WorldRole::Player).await;
    let (_bystander_id, cookie_bystander) = h.add_member("bystander", WorldRole::Player).await;

    let mut ws_sender = h.connect_with(&cookie_sender).await;
    let mut ws_recipient = h.connect_with(&cookie_recipient).await;
    let mut ws_bystander = h.connect_with(&cookie_bystander).await;
    recv_until(&mut ws_sender, "welcome").await;
    recv_until(&mut ws_recipient, "welcome").await;
    recv_until(&mut ws_bystander, "welcome").await;

    ws_sender
        .send(send_message_frame(
            "whispers",
            "xylophone galaxy secret",
            whisper_audience(&[recipient_id]),
        ))
        .await
        .unwrap();
    recv_next_message_create(&mut ws_recipient).await; // wait for delivery before searching

    let search = |request_id: Uuid| {
        Message::Text(
            serde_json::json!({
                "type": "search",
                "request_id": request_id,
                "query": "xylophone",
                "limit": 10,
                "cursor": null,
            })
            .to_string(),
        )
    };

    let req_recipient = Uuid::from_u128(1);
    ws_recipient
        .send(search(req_recipient))
        .await
        .unwrap();
    let result = recv_until(&mut ws_recipient, "search_result").await;
    assert_eq!(
        result["hits"].as_array().unwrap().len(),
        1,
        "the recipient finds their own whisper"
    );

    let req_bystander = Uuid::from_u128(2);
    ws_bystander
        .send(search(req_bystander))
        .await
        .unwrap();
    let result = recv_until(&mut ws_bystander, "search_result").await;
    assert_eq!(
        result["hits"].as_array().unwrap().len(),
        0,
        "a non-recipient's search must not surface the whisper"
    );
}
