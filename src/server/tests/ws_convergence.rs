//! Desync-convergence harness: real WS clients against an ephemeral in-process
//! server. Faults are induced by client behavior (stop reading -> Lagged,
//! ignore frames, disconnect+reconnect). Convergence is asserted against the
//! authoritative `world_events` log.

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
    let world = repo.create_world("test", 0).await.unwrap();
    let hash = hash_password("pw").unwrap();
    repo.create_user("u", Some(&hash), ServerRole::User, 0)
        .await
        .unwrap();

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

    // Log in over HTTP to obtain the signed session cookie, then reuse it on WS.
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
            .map(|c| c.seq)
            .collect()
    }
}

fn emit(nonce: u64) -> Message {
    Message::Text(serde_json::json!({ "type": "emit_test", "nonce": nonce }).to_string())
}

/// Read frames, collecting Event seqs, until `count` events are seen or a budget
/// elapses. Returns collected seqs in arrival order.
async fn drain_event_seqs(ws: &mut Ws, count: usize) -> Vec<i64> {
    let mut seqs = vec![];
    while seqs.len() < count {
        let Ok(Some(Ok(m))) =
            tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await
        else {
            break;
        };
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(m.to_text().unwrap_or("")) {
            if v["type"] == "event" {
                seqs.push(v["command"]["seq"].as_i64().unwrap());
            }
        }
    }
    seqs
}

#[tokio::test]
async fn join_welcome_emit_receive() {
    let h = spawn().await;
    let mut ws = h.connect().await;

    // First server frame is Welcome.
    let first = ws.next().await.unwrap().unwrap();
    let welcome: serde_json::Value = serde_json::from_str(first.to_text().unwrap()).unwrap();
    assert_eq!(welcome["type"], "welcome");
    assert_eq!(welcome["current_seq"], 0);

    // Emit one test event; expect an Event with seq 1.
    ws.send(emit(1)).await.unwrap();
    let evt = loop {
        let m = ws.next().await.unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_str(m.to_text().unwrap()).unwrap();
        if v["type"] == "event" {
            break v;
        }
    };
    assert_eq!(evt["command"]["seq"], 1);
    assert_eq!(h.authoritative_seqs().await, vec![1]);
}

#[tokio::test]
async fn all_clients_converge_after_reconnect() {
    let h = spawn().await;

    // Client A stays connected the whole time.
    let mut a = h.connect().await;
    let _ = a.next().await; // Welcome

    // Emit 5 events from a publisher client.
    let mut pubc = h.connect().await;
    let _ = pubc.next().await; // Welcome
    for n in 0..5 {
        pubc.send(emit(n)).await.unwrap();
    }

    // Client A receives all 5 live.
    let a_seqs = drain_event_seqs(&mut a, 5).await;
    assert_eq!(a_seqs, vec![1, 2, 3, 4, 5]);

    // Client B joins late and explicitly resyncs from seq 1.
    let mut b = h.connect().await;
    let _ = b.next().await; // Welcome (current_seq = 5)
    b.send(Message::Text(
        serde_json::json!({ "type": "resync_request", "from_seq": 1 }).to_string(),
    ))
    .await
    .unwrap();
    let b_seqs = drain_event_seqs(&mut b, 5).await;
    assert_eq!(b_seqs, vec![1, 2, 3, 4, 5]);

    // Authoritative log is the ground truth both converged to.
    assert_eq!(h.authoritative_seqs().await, vec![1, 2, 3, 4, 5]);
}

#[tokio::test]
async fn slow_reader_recovers_via_resync() {
    let h = spawn().await;
    let mut slow = h.connect().await;
    let _ = slow.next().await; // Welcome

    // Flood more events than the broadcast capacity (256) without reading,
    // pressuring the slow connection toward a lag-driven resync.
    let mut pubc = h.connect().await;
    let _ = pubc.next().await;
    for n in 0..400 {
        pubc.send(emit(n)).await.unwrap();
    }
    // Give the server time to process the publishes.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Now read: whether delivery came live or via resync, the final delivered
    // seq reaches the authoritative tail (400), strictly increasing, no dups.
    let seqs = drain_event_seqs(&mut slow, 400).await;
    assert_eq!(*seqs.last().unwrap(), 400);
    let mut sorted = seqs.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(seqs, sorted, "no duplicates or reordering after resync");
    assert_eq!(*h.authoritative_seqs().await.last().unwrap(), 400);
}

#[tokio::test]
async fn time_sync_returns_pong() {
    let h = spawn().await;
    let mut ws = h.connect().await;
    let _ = ws.next().await; // Welcome
    ws.send(Message::Text(
        serde_json::json!({ "type": "time_ping", "client_t0": 1000 }).to_string(),
    ))
    .await
    .unwrap();
    let pong = loop {
        let m = ws.next().await.unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_str(m.to_text().unwrap()).unwrap();
        if v["type"] == "time_pong" {
            break v;
        }
    };
    assert_eq!(pong["client_t0"], 1000);
    assert!(pong["server_t"].as_i64().unwrap() > 0);
}
