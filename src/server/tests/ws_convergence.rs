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
use shadowcat::data::document::WorldRole;
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
    spawn_with_ws(shadowcat::ws::WsState::new()).await
}

/// Like `spawn`, but the room broadcast ring uses a small `capacity`, so a client
/// that pauses reading is likely to overflow it and exercise the `Lagged` → resync
/// path end-to-end. The deterministic lag guard lives in the `egress_loop` unit
/// test (`ws::conn::tests::egress_lag_triggers_resync_and_converges`); this test
/// asserts only convergence, which holds whether or not the lag fires on a given OS.
async fn spawn_with_capacity(capacity: usize) -> Harness {
    spawn_with_ws(shadowcat::ws::WsState::with_broadcast_capacity(capacity)).await
}

async fn spawn_with_ws(ws: shadowcat::ws::WsState) -> Harness {
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let hash = hash_password("pw").unwrap();
    // The seeded user owns the world (GM), so its intents are authorized and
    // it passes the membership-gated WS join.
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
        ws,
        upload_rate: Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
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
        self.connect_with(&self.cookie).await
    }

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

/// A `create` op for a minimal world-scoped document.
fn create_op(world: Uuid, doc_id: Uuid, system: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "op": "create",
        "doc": {
            "id": doc_id,
            "scope": { "kind": "world", "world_id": world },
            "doc_type": "actor",
            "schema_version": 1,
            "system": system,
            "created_at": 0,
            "updated_at": 0,
        }
    })
}

/// An `update` op carrying one field change with its pre-image.
fn update_op(
    doc_id: Uuid,
    path: &str,
    old: serde_json::Value,
    new: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "op": "update",
        "doc_id": doc_id,
        "changes": [{ "path": path, "old": old, "new": new }],
    })
}

/// An intent that creates one distinct document, keyed by `n`.
fn create_intent(world: Uuid, n: u64) -> Message {
    let doc_id = Uuid::from_u128(1000 + n as u128);
    intent_msg(
        n,
        serde_json::json!([create_op(world, doc_id, serde_json::json!({}))]),
    )
}

/// An explicit gap-recovery request from `from_seq`.
fn resync_request(from_seq: i64) -> Message {
    Message::Text(serde_json::json!({ "type": "resync_request", "from_seq": from_seq }).to_string())
}

/// Drain `event` and `reject` frames (skipping welcome/ping/time_pong) until
/// `count` are collected or the budget elapses.
async fn drain_frames(ws: &mut Ws, count: usize) -> Vec<serde_json::Value> {
    let mut out = vec![];
    while out.len() < count {
        let Ok(Some(Ok(m))) =
            tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await
        else {
            break;
        };
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(m.to_text().unwrap_or("")) {
            if matches!(v["type"].as_str(), Some("event") | Some("reject")) {
                out.push(v);
            }
        }
    }
    out
}

/// Read until an Event with seq `target` (the authoritative tail) is observed,
/// returning all Event seqs in arrival order. Unlike a fixed-count drain, this
/// is robust to the publish/resync interleaving on a loaded runner: a slow
/// runner cannot truncate collection, and a genuine duplicate or gap surfaces
/// deterministically in the caller's contiguity assertion rather than as a
/// short read. Generous per-frame budget; breaks only on a real stall.
async fn drain_until_seq(ws: &mut Ws, target: i64) -> Vec<i64> {
    let mut seqs = vec![];
    loop {
        let Ok(Some(Ok(m))) =
            tokio::time::timeout(std::time::Duration::from_secs(10), ws.next()).await
        else {
            break;
        };
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(m.to_text().unwrap_or("")) {
            if v["type"] == "event" {
                let s = v["command"]["seq"].as_i64().unwrap();
                seqs.push(s);
                if s >= target {
                    break;
                }
            }
        }
    }
    seqs
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn join_welcome_emit_receive() {
    let h = spawn().await;
    let mut ws = h.connect().await;

    // First server frame is Welcome.
    let first = ws.next().await.unwrap().unwrap();
    let welcome: serde_json::Value = serde_json::from_str(first.to_text().unwrap()).unwrap();
    assert_eq!(welcome["type"], "welcome");
    assert_eq!(welcome["current_seq"], 0);

    // Emit one create intent; expect an Event with seq 1.
    ws.send(create_intent(h.world, 1)).await.unwrap();
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn all_clients_converge_after_reconnect() {
    let h = spawn().await;

    // Client A stays connected the whole time.
    let mut a = h.connect().await;
    let _ = a.next().await; // Welcome

    // Emit 5 events from a publisher client.
    let mut pubc = h.connect().await;
    let _ = pubc.next().await; // Welcome
    for n in 0..5 {
        pubc.send(create_intent(h.world, n)).await.unwrap();
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_reader_recovers_via_resync() {
    // End-to-end convergence over a real WS against a small (8-slot) broadcast ring:
    // a client that pauses reading while a publisher floods events past the ring, then
    // resumes, must still converge to the authoritative tail with no gaps or dups —
    // whether it converges from live frames or via a `Lagged`-driven resync. The lag
    // path itself is guaranteed deterministically (OS-independently) by the unit test
    // `ws::conn::tests::egress_lag_triggers_resync_and_converges`, which drives
    // `egress_loop` with a credit-gated sink; asserting the lag fired here would
    // depend on the runner's TCP buffering, which is non-portable.
    let h = spawn_with_capacity(8).await;
    let mut slow = h.connect().await;
    let _ = slow.next().await; // Welcome — then we pause reading `slow`.

    let mut pubc = h.connect().await;
    let _ = pubc.next().await;
    for n in 0..30u64 {
        let doc_id = Uuid::from_u128(1000 + n as u128);
        let op = create_op(h.world, doc_id, serde_json::json!({ "n": n }));
        pubc.send(intent_msg(n, serde_json::json!([op])))
            .await
            .unwrap();
    }

    // Wait until all 30 are durably applied (and thus broadcast). Bounded so a genuine
    // stall fails loudly rather than hanging.
    let mut waited = 0;
    while h.authoritative_seqs().await.len() < 30 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        waited += 1;
        assert!(waited < 300, "server did not apply 30 intents within 30s");
    }

    // Resume reading `slow`: the final delivered seq reaches the authoritative tail
    // (30), strictly increasing, no dups — via live frames or a resync.
    let seqs = drain_until_seq(&mut slow, 30).await;
    assert_eq!(*seqs.last().unwrap(), 30);
    let mut sorted = seqs.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(seqs, sorted, "no duplicates or reordering after resync");
    assert_eq!(*h.authoritative_seqs().await.last().unwrap(), 30);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn converges_with_publishing_during_resync() {
    // Regression guard for the resync watermark: events published while a resync
    // replay is in flight must not be dropped. The slow client accrues a backlog
    // (forcing a server-side resync) and then drains while the publisher is still
    // emitting, so live publishing overlaps the replay window.
    let h = spawn().await;
    let mut slow = h.connect().await;
    let _ = slow.next().await; // Welcome

    let mut pubc = h.connect().await;
    let _ = pubc.next().await;
    let world = h.world;
    let publisher = tokio::spawn(async move {
        for n in 0..300 {
            pubc.send(create_intent(world, n)).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        // Return the socket so it is NOT dropped here. Dropping a tungstenite
        // stream tears the TCP connection down abruptly, which can RST away
        // frames the slow server has not read yet — silently losing the tail of
        // the publish. The caller keeps it open until the server has applied all.
        pubc
    });

    // Let a backlog build on the unread `slow` connection, then drain while the
    // publisher keeps going.
    tokio::time::sleep(std::time::Duration::from_millis(40)).await;
    let seqs = drain_until_seq(&mut slow, 300).await;

    // Keep the publisher socket open past its last send, then wait until the
    // server has durably applied every published intent (the single-writer pool
    // may still be draining the ingress backlog on a slow runner).
    let _pubc = publisher.await.unwrap();
    // Wide apply-drain budget: on a saturated CI runner the single-writer ingress
    // can take well over 30s to apply 300 queued intents (documented ubuntu-latest
    // saturation). 600×100ms = 60s headroom before a genuine stall fails the test.
    for _ in 0..600 {
        if h.authoritative_seqs().await.last().copied() == Some(300) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // The watermark invariant under test: events published while a resync replay
    // is in flight are never DROPPED. A drop would punch a gap into the delivered
    // stream, so whatever the lagged client received during the overlap must be a
    // contiguous, gap-free prefix from seq 1 (no duplicates either). How far it
    // got in real time is timing-dependent (a slower runner lags more) and is
    // deliberately not asserted — auto-convergence latency on a saturated lagged
    // connection is not a correctness property; client-driven resync is.
    assert_eq!(seqs.first().copied(), Some(1));
    assert!(
        seqs.windows(2).all(|w| w[1] == w[0] + 1),
        "events dropped or duplicated across the resync window: {seqs:?}"
    );

    // All 300 are durably sequenced...
    assert_eq!(h.authoritative_seqs().await.last().copied(), Some(300));

    // ...and the full history is recoverable: a fresh client resyncs from seq 1
    // and receives every event contiguously through the tail. This is the
    // deterministic convergence path real clients use on staleness.
    let mut late = h.connect().await;
    let _ = late.next().await; // Welcome (current_seq = 300)
    late.send(resync_request(1)).await.unwrap();
    let recovered = drain_until_seq(&mut late, 300).await;
    assert_eq!(recovered.first().copied(), Some(1));
    assert_eq!(*recovered.last().unwrap(), 300);
    assert!(
        recovered.windows(2).all(|w| w[1] == w[0] + 1),
        "gap in full resync: {recovered:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn conflicting_same_field_update_is_rejected() {
    let h = spawn().await;
    let mut ws = h.connect().await;
    let _ = ws.next().await; // Welcome

    let doc_id = Uuid::from_u128(5000);
    // Create a doc with hp=10 (seq 1).
    ws.send(intent_msg(
        1,
        serde_json::json!([create_op(h.world, doc_id, serde_json::json!({ "hp": 10 }))]),
    ))
    .await
    .unwrap();
    // Two updates against the same pre-image (old=10), sent back-to-back: the
    // first wins (seq 2), the second is stale once hp is 5.
    ws.send(intent_msg(
        2,
        serde_json::json!([update_op(
            doc_id,
            "/system/hp",
            serde_json::json!(10),
            serde_json::json!(5)
        )]),
    ))
    .await
    .unwrap();
    ws.send(intent_msg(
        3,
        serde_json::json!([update_op(
            doc_id,
            "/system/hp",
            serde_json::json!(10),
            serde_json::json!(7)
        )]),
    ))
    .await
    .unwrap();

    let frames = drain_frames(&mut ws, 3).await;
    let event_seqs: Vec<i64> = frames
        .iter()
        .filter(|f| f["type"] == "event")
        .map(|f| f["command"]["seq"].as_i64().unwrap())
        .collect();
    assert_eq!(event_seqs, vec![1, 2], "create + first update commit");
    let rejects: Vec<&serde_json::Value> =
        frames.iter().filter(|f| f["type"] == "reject").collect();
    assert_eq!(rejects.len(), 1);
    assert_eq!(rejects[0]["reason"], "conflict");
    // The committed value is the first writer's.
    assert_eq!(h.authoritative_seqs().await, vec![1, 2]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn player_write_to_gm_owned_doc_is_forbidden() {
    let h = spawn().await;
    let mut gm = h.connect().await;
    let _ = gm.next().await; // Welcome

    // GM creates a doc with default permissions (only GM may write).
    let doc_id = Uuid::from_u128(6000);
    gm.send(intent_msg(
        1,
        serde_json::json!([create_op(h.world, doc_id, serde_json::json!({ "hp": 1 }))]),
    ))
    .await
    .unwrap();
    let created = drain_frames(&mut gm, 1).await;
    assert_eq!(created[0]["command"]["seq"], 1);

    // A player member tries to update it → Reject{forbidden}.
    let cookie = h.add_member("p", WorldRole::Player).await;
    let mut pc = h.connect_with(&cookie).await;
    let _ = pc.next().await; // Welcome
    pc.send(intent_msg(
        2,
        serde_json::json!([update_op(
            doc_id,
            "/system/hp",
            serde_json::json!(1),
            serde_json::json!(9)
        )]),
    ))
    .await
    .unwrap();
    let frames = drain_frames(&mut pc, 1).await;
    assert_eq!(frames[0]["type"], "reject");
    assert_eq!(frames[0]["reason"], "forbidden");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gm_only_property_hidden_from_player() {
    let h = spawn().await;

    // Player joins first so it receives the create live.
    let cookie = h.add_member("p", WorldRole::Player).await;
    let mut pc = h.connect_with(&cookie).await;
    let _ = pc.next().await; // Welcome

    let mut gm = h.connect().await;
    let _ = gm.next().await; // Welcome

    // GM creates a player-observable doc whose /system/secret is GM-only.
    let doc_id = Uuid::from_u128(7000);
    let doc = serde_json::json!({
        "id": doc_id,
        "scope": { "kind": "world", "world_id": h.world },
        "doc_type": "actor",
        "schema_version": 1,
        "permissions": {
            "default": "observer",
            "users": {},
            "property_overrides": { "/system/secret": "gm_only" }
        },
        "system": { "secret": 42, "public": 7 },
        "created_at": 0,
        "updated_at": 0,
    });
    gm.send(intent_msg(
        1,
        serde_json::json!([{ "op": "create", "doc": doc }]),
    ))
    .await
    .unwrap();

    let frames = drain_frames(&mut pc, 1).await;
    assert_eq!(frames[0]["type"], "event");
    assert_eq!(frames[0]["command"]["seq"], 1);
    let created = &frames[0]["command"]["ops"][0];
    assert_eq!(created["op"], "create");
    assert_eq!(created["doc"]["system"]["public"], 7);
    assert!(
        created["doc"]["system"].get("secret").is_none(),
        "GM-only property must be stripped for the player"
    );
}

/// Drain frames until one of `type` arrives (skips welcome/resync/etc.).
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

#[tokio::test]
async fn scene_ping_relays_out_of_band_to_world_members_with_sender_stamped() {
    let h = spawn().await;
    let cookie_b = h.add_member("b", WorldRole::Player).await;
    let mut a = h.connect().await; // owner/GM
    let mut b = h.connect_with(&cookie_b).await;
    // Both must be joined (welcome received → subscribed) before the lossy broadcast.
    recv_until(&mut a, "welcome").await;
    recv_until(&mut b, "welcome").await;

    a.send(Message::Text(
        serde_json::json!({ "type": "scene_ping", "scene": h.world, "x": 12.0, "y": 34.0 })
            .to_string(),
    ))
    .await
    .unwrap();

    // The other member receives the relayed ping with the sender stamped.
    let p = recv_until(&mut b, "scene_ping").await;
    assert_eq!(p["x"], 12.0);
    assert_eq!(p["y"], 34.0);
    assert!(p["user"].is_string());

    // It is out-of-band: it must not have created an authoritative event.
    assert!(h.authoritative_seqs().await.is_empty());
}
