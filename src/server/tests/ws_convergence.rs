//! Desync-convergence harness: real WS clients against an ephemeral in-process server.
//!
//! Faults are induced by client behavior (stop reading -> Lagged, ignore frames,
//! disconnect+reconnect). Convergence is asserted against the authoritative `world_events` log.

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
        uploads: Arc::new(shadowcat::http::assets::uploads::UploadSessions::new()),
        auth_throttle: Arc::new(shadowcat::http::throttle::AuthThrottle::new()),
        write_barrier: Arc::new(tokio::sync::RwLock::new(())),
        preview_fetch_locks: Arc::new(dashmap::DashMap::new()),
        file_permits: Arc::new(shadowcat::http::assets::permits::StreamedFilePermits::production()),
        tls: shadowcat::tls::TlsRuntime::disabled(),
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
        self.connect_world(self.world, cookie).await
    }

    /// Connect to `world` (which need not be the harness's own) with `cookie`.
    async fn connect_world(&self, world: Uuid, cookie: &str) -> Ws {
        let url = format!("ws://{}/ws?world={}", self.addr, world);
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
            .map(|c| c.command.seq)
            .collect()
    }
}

/// Deterministic v4-shaped document id for test fixtures: same low-order bytes as
/// `Uuid::from_u128(n)`, with the version/variant nibbles forced to v4/RFC4122 so it satisfies
/// `shadowcat::data::validation::validate_document_id`. Mirrors
/// `shadowcat_test_support::test_doc_id`, restated locally since this file builds its own
/// harness rather than depending on that crate.
fn test_doc_id(n: u128) -> Uuid {
    let mut bytes = n.to_be_bytes();
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
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
            // "actor" is engine-defined; a minimal valid body clears the
            // ingress gate. `system` above is `create_op`'s own parameter, the field callers vary.
            "engine": {
                "displayName": "Test",
                "visual": { "kind": "image", "asset": "11111111-1111-1111-1111-111111111111" },
                "size": { "w": 1.0, "h": 1.0 },
                "shape": "square",
                "faction": null,
                "conditions": [],
                "prototype": true
            },
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
    let doc_id = test_doc_id(1000 + n as u128);
    intent_msg(
        n,
        serde_json::json!([create_op(world, doc_id, serde_json::json!({}))]),
    )
}

/// An explicit gap-recovery request from `from_seq`.
fn resync_request(from_seq: i64) -> Message {
    Message::Text(serde_json::json!({ "type": "resync_request", "from_seq": from_seq }).to_string())
}

/// A cold-start `Hello { last_seq: None }` — establishes this connection's resync
/// floor at the room's current seq when the room enforces it.
fn hello_cold_start(world: Uuid) -> Message {
    Message::Text(
        serde_json::json!({ "type": "hello", "world": world, "last_seq": null }).to_string(),
    )
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
    // The join-time config seed committed (seq 1) before this Welcome.
    assert_eq!(welcome["current_seq"], 1);

    // Emit one create intent; expect an Event with seq 2.
    ws.send(create_intent(h.world, 1)).await.unwrap();
    let evt = loop {
        let m = ws.next().await.unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_str(m.to_text().unwrap()).unwrap();
        if v["type"] == "event" {
            break v;
        }
    };
    assert_eq!(evt["command"]["seq"], 2);
    assert_eq!(h.authoritative_seqs().await, vec![1, 2]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn all_clients_converge_after_reconnect() {
    let h = spawn().await;

    // Client A stays connected the whole time. A cold-start Hello (sent before any
    // event is published) establishes this user's resync floor at seq 0 — mirroring
    // the real client's unconditional first-open Hello — so a LATER connection for
    // the same user (client B below; this harness's connections all share one
    // authenticated user) inherits that floor and is not clamped when it later
    // requests the full backlog (the floor is keyed by user_id, not by connection).
    let mut a = h.connect().await;
    let _ = a.next().await; // Welcome
    a.send(hello_cold_start(h.world)).await.unwrap();
    // A time_ping/time_pong round trip on this SAME connection is a synchronization
    // barrier: per-connection ingress frames are processed in send order, so the pong
    // proves the Hello above was already applied (floor established) before the
    // publisher below opens its own connection — no cross-connection race.
    a.send(Message::Text(
        serde_json::json!({ "type": "time_ping", "client_t0": 0 }).to_string(),
    ))
    .await
    .unwrap();
    let _ = drain_until_type(&mut a, "time_pong").await;

    // Emit 5 events from a publisher client.
    let mut pubc = h.connect().await;
    let _ = pubc.next().await; // Welcome
    for n in 0..5 {
        pubc.send(create_intent(h.world, n)).await.unwrap();
    }

    // Client A receives all 5 live (seq 1 is A's own join-time config seed,
    // published before A subscribed, so its stream starts at 2).
    let a_seqs = drain_event_seqs(&mut a, 5).await;
    assert_eq!(a_seqs, vec![2, 3, 4, 5, 6]);

    // Client B joins late and explicitly resyncs from seq 1.
    let mut b = h.connect().await;
    let _ = b.next().await; // Welcome (current_seq = 5)
    b.send(Message::Text(
        serde_json::json!({ "type": "resync_request", "from_seq": 1 }).to_string(),
    ))
    .await
    .unwrap();
    // B's replay is clamped to A's floor (established at the post-seed
    // watermark, seq 1), so it starts at 2 like A's live stream did.
    let b_seqs = drain_event_seqs(&mut b, 5).await;
    assert_eq!(b_seqs, vec![2, 3, 4, 5, 6]);

    // Authoritative log is the ground truth both converged to.
    assert_eq!(h.authoritative_seqs().await, vec![1, 2, 3, 4, 5, 6]);
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
        let doc_id = test_doc_id(1000 + n as u128);
        let op = create_op(h.world, doc_id, serde_json::json!({ "n": n }));
        pubc.send(intent_msg(n, serde_json::json!([op])))
            .await
            .unwrap();
    }

    // Wait until all 30 are durably applied (and thus broadcast). Bounded so a genuine
    // stall fails loudly rather than hanging.
    let mut waited = 0;
    // 31 = the join-time config seed + the 30 published intents.
    while h.authoritative_seqs().await.len() < 31 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        waited += 1;
        assert!(
            waited < 300,
            "server did not apply seed + 30 intents within 30s"
        );
    }

    // Resume reading `slow`: the final delivered seq reaches the authoritative tail
    // (31), strictly increasing, no dups — via live frames or a resync.
    let seqs = drain_until_seq(&mut slow, 31).await;
    assert_eq!(*seqs.last().unwrap(), 31);
    let mut sorted = seqs.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(seqs, sorted, "no duplicates or reordering after resync");
    assert_eq!(*h.authoritative_seqs().await.last().unwrap(), 31);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn converges_with_publishing_during_resync() {
    // Two end-to-end convergence guarantees under concurrent publishing:
    //   (1) a client reading concurrently with an active publisher never sees a
    //       DROPPED or REORDERED event — its delivered stream is a contiguous,
    //       gap-free prefix from seq 1;
    //   (2) the full history is recoverable by a FRESH client via `ResyncRequest`
    //       — the deterministic convergence path real clients use on staleness.
    // The deterministic broadcast-`Lagged` → resync handling itself (where a real
    // socket cannot reliably force the lag — OS buffering absorbs a non-reading
    // client, the same non-portability as the Lagged test) is unit-tested against
    // `egress_loop` directly in `ws::conn::tests::egress_lag_triggers_resync_and_converges`.
    //
    // Volume is kept modest (`TOTAL`): the load-bearing invariants above hold at any
    // count, while a large backlog only burdens the single-writer ingress, whose
    // apply throughput collapses under CI-runner saturation (a 300-intent backlog
    // takes >60s to drain on a saturated ubuntu-latest runner) — a runner-capacity
    // artifact, not a correctness property.
    const TOTAL: i64 = 100;

    let h = spawn().await;
    let mut reader = h.connect().await;
    let _ = reader.next().await; // Welcome
                                 // A cold-start Hello (before any event is published) establishes this user's
                                 // resync floor at seq 0 — mirroring the real client's unconditional first-open
                                 // Hello — so the LATER `late` connection below (same authenticated user; the
                                 // floor is keyed by user_id, not by connection) inherits it and its full-history
                                 // resync isn't clamped. The time_ping/time_pong round trip on this SAME
                                 // connection is a synchronization barrier proving the Hello was applied before
                                 // the publisher below opens its own connection — no cross-connection race.
    reader.send(hello_cold_start(h.world)).await.unwrap();
    reader
        .send(Message::Text(
            serde_json::json!({ "type": "time_ping", "client_t0": 0 }).to_string(),
        ))
        .await
        .unwrap();
    let _ = drain_until_type(&mut reader, "time_pong").await;

    let mut pubc = h.connect().await;
    let _ = pubc.next().await;
    let world = h.world;
    let publisher = tokio::spawn(async move {
        for n in 0..TOTAL as u64 {
            pubc.send(create_intent(world, n)).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        // Return the socket so it is NOT dropped here. Dropping a tungstenite
        // stream tears the TCP connection down abruptly, which can RST away
        // frames the server has not read yet — silently losing the tail of the
        // publish. The caller keeps it open until the server has applied all.
        pubc
    });

    // Begin reading partway through the publish, so delivery overlaps live emission.
    tokio::time::sleep(std::time::Duration::from_millis(40)).await;
    let seqs = drain_until_seq(&mut reader, TOTAL + 1).await;

    // Keep the publisher socket open past its last send, then wait until the server
    // has durably applied every published intent (the single-writer pool may still
    // be draining the ingress backlog on a slow runner). 600×100ms = 60s headroom.
    let _pubc = publisher.await.unwrap();
    for _ in 0..600 {
        if h.authoritative_seqs().await.last().copied() == Some(TOTAL + 1) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // (1) No drop/reorder under concurrent publishing: a contiguous prefix from
    // seq 2 (seq 1 is the reader's own join-time config seed, published before
    // it subscribed).
    assert_eq!(seqs.first().copied(), Some(2));
    assert!(
        seqs.windows(2).all(|w| w[1] == w[0] + 1),
        "events dropped or duplicated under concurrent publishing: {seqs:?}"
    );

    // All published intents are durably sequenced...
    assert_eq!(
        h.authoritative_seqs().await.last().copied(),
        Some(TOTAL + 1)
    );

    // (2) ...and the full history is recoverable: a fresh client resyncs from
    // seq 1 and receives every event its floor admits (the shared user's floor
    // sits at the post-seed watermark, seq 1) contiguously through the tail.
    let mut late = h.connect().await;
    let _ = late.next().await; // Welcome (current_seq = TOTAL + 1)
    late.send(resync_request(1)).await.unwrap();
    let recovered = drain_until_seq(&mut late, TOTAL + 1).await;
    assert_eq!(recovered.first().copied(), Some(2));
    assert_eq!(*recovered.last().unwrap(), TOTAL + 1);
    assert!(
        recovered.windows(2).all(|w| w[1] == w[0] + 1),
        "gap in full resync: {recovered:?}"
    );
}

/// Sends nothing; reads frames from a `resync_begin` through the matching `resync_end`,
/// returning the Event seqs replayed in between plus the `resync_end` frame itself. Panics
/// if either bracket frame does not arrive within budget.
async fn drain_resync(ws: &mut Ws) -> (Vec<i64>, serde_json::Value) {
    let _begin = drain_until_type(ws, "resync_begin").await;
    let mut seqs = vec![];
    loop {
        let Ok(Some(Ok(m))) =
            tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await
        else {
            panic!("timed out waiting for resync_end");
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(m.to_text().unwrap_or("")) else {
            continue;
        };
        match v["type"].as_str() {
            Some("event") => seqs.push(v["command"]["seq"].as_i64().unwrap()),
            Some("resync_end") => return (seqs, v),
            _ => {}
        }
    }
}

/// Read frames until one of type `ty` arrives (5s budget), returning it.
async fn drain_until_type(ws: &mut Ws, ty: &str) -> serde_json::Value {
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resync_floor_bounds_a_late_clients_reach_when_enforced() {
    // A connection sends a cold-start Hello (establishing its floor at the room's current
    // seq) only AFTER several events have already been committed, then asks for the entire
    // history via an unbounded ResyncRequest. With enforcement on, the reply is bounded to
    // its own floor forward — none of the events committed before the Hello are replayed.
    let h = spawn().await;

    let mut pubc = h.connect().await;
    let _ = pubc.next().await; // Welcome
    for n in 0..3 {
        pubc.send(create_intent(h.world, n)).await.unwrap();
    }
    let mut waited = 0;
    // 4 = the join-time config seed + the 3 published intents.
    while h.authoritative_seqs().await.len() < 4 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        waited += 1;
        assert!(waited < 100, "seed + 3 events did not commit in time");
    }

    let mut late = h.connect().await;
    let _ = late.next().await; // Welcome (current_seq = 4)
    late.send(hello_cold_start(h.world)).await.unwrap();
    late.send(resync_request(1)).await.unwrap();

    let (seqs, end) = drain_resync(&mut late).await;
    assert!(
        seqs.is_empty(),
        "resync must not replay events committed before the client's own cold-start floor: {seqs:?}"
    );
    assert_eq!(
        end["current_seq"], 4,
        "resync_end still reports the true tail"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resync_without_a_prior_hello_fails_closed_when_enforced() {
    // No floor was ever established for this connection (it never sent Hello). With
    // enforcement on, the resync floor defaults to `current_seq() + 1` — an explicit
    // ResyncRequest for the whole history returns nothing, forcing a proper bootstrap.
    let h = spawn().await;

    let mut pubc = h.connect().await;
    let _ = pubc.next().await;
    for n in 0..3 {
        pubc.send(create_intent(h.world, n)).await.unwrap();
    }
    let mut waited = 0;
    // 4 = the join-time config seed + the 3 published intents.
    while h.authoritative_seqs().await.len() < 4 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        waited += 1;
        assert!(waited < 100, "seed + 3 events did not commit in time");
    }

    let mut late = h.connect().await;
    let _ = late.next().await; // Welcome — deliberately no Hello sent.
    late.send(resync_request(1)).await.unwrap();

    let (seqs, _end) = drain_resync(&mut late).await;
    assert!(
        seqs.is_empty(),
        "no floor established ⇒ empty resync, not the unbounded default: {seqs:?}"
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

    let doc_id = test_doc_id(5000);
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
    assert_eq!(event_seqs, vec![2, 3], "create + first update commit");
    let rejects: Vec<&serde_json::Value> =
        frames.iter().filter(|f| f["type"] == "reject").collect();
    assert_eq!(rejects.len(), 1);
    assert_eq!(rejects[0]["reason"], "conflict");
    // The committed value is the first writer's (seq 1 is the join-time
    // config seed).
    assert_eq!(h.authoritative_seqs().await, vec![1, 2, 3]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn player_write_to_gm_owned_doc_is_forbidden() {
    let h = spawn().await;
    let mut gm = h.connect().await;
    let _ = gm.next().await; // Welcome

    // GM creates a doc with default permissions (only GM may write).
    let doc_id = test_doc_id(6000);
    gm.send(intent_msg(
        1,
        serde_json::json!([create_op(h.world, doc_id, serde_json::json!({ "hp": 1 }))]),
    ))
    .await
    .unwrap();
    let created = drain_frames(&mut gm, 1).await;
    assert_eq!(created[0]["command"]["seq"], 2);

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

/// Reads the join refusal off a refused socket: the `error` frame, then the stream ending.
async fn read_join_refusal(ws: &mut Ws) -> serde_json::Value {
    let first = tokio::time::timeout(std::time::Duration::from_secs(10), ws.next())
        .await
        .expect("a refusal frame arrives")
        .expect("the stream is not empty")
        .expect("the frame reads");
    let Message::Text(text) = first else {
        panic!("expected a text error frame, got {first:?}");
    };
    let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
    let rest = tokio::time::timeout(std::time::Duration::from_secs(10), ws.next())
        .await
        .expect("the socket closes after the refusal");
    assert!(
        matches!(rest, None | Some(Ok(Message::Close(_))) | Some(Err(_))),
        "nothing but a close follows the refusal, got {rest:?}"
    );
    frame
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_non_member_is_refused_with_a_forbidden_error_then_closed() {
    let h = spawn().await;
    let hash = hash_password("pw").unwrap();
    h.repo
        .create_user("stranger", Some(&hash), ServerRole::User, 0)
        .await
        .unwrap();
    let cookie = h.login("stranger", "pw").await;

    let mut ws = h.connect_with(&cookie).await;
    let frame = read_join_refusal(&mut ws).await;
    assert_eq!(frame["type"], "error");
    assert_eq!(frame["code"], "forbidden");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_nonexistent_world_is_refused_with_world_not_found_then_closed() {
    let h = spawn().await;
    let mut ws = h.connect_world(Uuid::from_u128(0xdead), &h.cookie).await;
    let frame = read_join_refusal(&mut ws).await;
    assert_eq!(frame["type"], "error");
    assert_eq!(frame["code"], "world_not_found");
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
    let doc_id = test_doc_id(7000);
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
        // "actor" is engine-defined; a minimal valid body clears the
        // ingress gate. `system` below is what this test exercises.
        "engine": {
            "displayName": "Test",
            "visual": { "kind": "image", "asset": "11111111-1111-1111-1111-111111111111" },
            "size": { "w": 1.0, "h": 1.0 },
            "shape": "square",
            "faction": null,
            "conditions": [],
            "prototype": true
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
    assert_eq!(frames[0]["command"]["seq"], 2);
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

/// Reads frames until one of type `ty` and returns it with the `type` of every frame skipped on
/// the way, so a test can assert that no frame of some type arrived before it (the socket
/// delivers a connection's frames in order).
async fn recv_until_collecting(ws: &mut Ws, ty: &str) -> (serde_json::Value, Vec<String>) {
    let mut skipped = Vec::new();
    loop {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .expect("timeout")
            .expect("stream ended")
            .expect("ws error");
        if let Message::Text(t) = msg {
            let v: serde_json::Value = serde_json::from_str(&t).unwrap();
            if v["type"] == ty {
                return (v, skipped);
            }
            skipped.push(v["type"].as_str().unwrap_or_default().to_owned());
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

    // ScenePing is only relayed for a scene that exists in this world (M-security-7's
    // guard); the GM creates one first.
    let scene_id = test_doc_id(9001);
    let scene_doc = serde_json::json!({
        "id": scene_id,
        "scope": { "kind": "world", "world_id": h.world },
        "doc_type": "scene",
        "schema_version": 1,
        "engine": { "grid": { "kind": "square", "size": 100.0 }, "background": null },
        "system": {},
        "created_at": 0,
        "updated_at": 0,
    });
    a.send(intent_msg(
        1,
        serde_json::json!([{ "op": "create", "doc": scene_doc }]),
    ))
    .await
    .unwrap();
    recv_until(&mut a, "event").await;
    recv_until(&mut b, "event").await;

    a.send(Message::Text(
        serde_json::json!({ "type": "scene_ping", "scene": scene_id, "x": 12.0, "y": 34.0 })
            .to_string(),
    ))
    .await
    .unwrap();

    // The other member receives the relayed ping with the sender stamped.
    let p = recv_until(&mut b, "scene_ping").await;
    assert_eq!(p["x"], 12.0);
    assert_eq!(p["y"], 34.0);
    assert!(p["user"].is_string());

    // It is out-of-band: the ping itself must not have created an authoritative
    // event (seq 1 is the join-time config seed; the scene-creation intent
    // above committed at seq 2).
    assert_eq!(h.authoritative_seqs().await, vec![1, 2]);
}

#[tokio::test]
async fn emote_relays_out_of_band_with_sender_stamped_and_drops_unauthorized() {
    let h = spawn().await;
    let cookie_b = h.add_member("b", WorldRole::Player).await;
    let b_id = h
        .repo
        .member_id_by_username(h.world, "b")
        .await
        .unwrap()
        .unwrap();
    let mut a = h.connect().await; // owner/GM
    let mut b = h.connect_with(&cookie_b).await;
    // Both must be joined (welcome received → subscribed) before the lossy broadcast.
    recv_until(&mut a, "welcome").await;
    recv_until(&mut b, "welcome").await;

    // The GM creates the scene plus two tokens: one b effectively owns, one owned by
    // no one (a GM-only token). The emote relay authorizes on the TOKEN — effective
    // ownership plus membership in the named scene — so both fixtures are needed.
    let scene_id = test_doc_id(9002);
    let b_token = test_doc_id(9003);
    let gm_token = test_doc_id(9004);
    let scene_doc = serde_json::json!({
        "id": scene_id,
        "scope": { "kind": "world", "world_id": h.world },
        "doc_type": "scene",
        "schema_version": 1,
        "engine": { "grid": { "kind": "square", "size": 100.0 }, "background": null },
        "system": {},
        "created_at": 0,
        "updated_at": 0,
    });
    let token_doc = |id: Uuid, owner: serde_json::Value| {
        serde_json::json!({
            "id": id,
            "scope": { "kind": "world", "world_id": h.world },
            "doc_type": "token",
            "schema_version": 1,
            "owner": owner,
            "parent_id": scene_id,
            "engine": { "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 },
            "system": {},
            "created_at": 0,
            "updated_at": 0,
        })
    };
    a.send(intent_msg(
        1,
        serde_json::json!([
            { "op": "create", "doc": scene_doc },
            { "op": "create", "doc": token_doc(b_token, serde_json::to_value(b_id).unwrap()) },
            { "op": "create", "doc": token_doc(gm_token, serde_json::Value::Null) },
        ]),
    ))
    .await
    .unwrap();
    recv_until(&mut a, "event").await;
    recv_until(&mut b, "event").await;

    // Drops, in send order (the broadcast channel preserves order, so any of these
    // reaching `a` would arrive BEFORE the final legitimate emote):
    // 1. a token b does not effectively own;
    // 2. a scene id the token is not parented to;
    // 3. an empty payload;
    // 4. an over-long payload (17 bytes > the 16-byte cap).
    for frame in [
        serde_json::json!({ "type": "emote", "scene": scene_id, "token": gm_token, "emote": "😀" }),
        serde_json::json!({ "type": "emote", "scene": Uuid::from_u128(9999), "token": b_token, "emote": "😀" }),
        serde_json::json!({ "type": "emote", "scene": scene_id, "token": b_token, "emote": "" }),
        serde_json::json!({ "type": "emote", "scene": scene_id, "token": b_token, "emote": "aaaaaaaaaaaaaaaaa" }),
    ] {
        b.send(Message::Text(frame.to_string())).await.unwrap();
    }
    b.send(Message::Text(
        serde_json::json!({ "type": "emote", "scene": scene_id, "token": b_token, "emote": "😀" })
            .to_string(),
    ))
    .await
    .unwrap();

    // The FIRST emote `a` receives is the legitimate one, proving every drop above held.
    let e = recv_until(&mut a, "emote").await;
    assert_eq!(e["scene"], serde_json::to_value(scene_id).unwrap());
    assert_eq!(e["token"], serde_json::to_value(b_token).unwrap());
    assert_eq!(e["user"], serde_json::to_value(b_id).unwrap());
    assert_eq!(e["emote"], "😀");

    // The sender receives their own echo.
    let echo = recv_until(&mut b, "emote").await;
    assert_eq!(echo["token"], serde_json::to_value(b_token).unwrap());

    // Out-of-band: no emote created an authoritative event (seq 1 is the join-time
    // config seed; the scene+token intent committed at seq 2).
    assert_eq!(h.authoritative_seqs().await, vec![1, 2]);
}

/// A world-scoped scene document with an explicit `permissions.default` role.
fn ruler_scene_doc(world: Uuid, id: Uuid, default_role: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "scope": { "kind": "world", "world_id": world },
        "doc_type": "scene",
        "schema_version": 1,
        "permissions": { "default": default_role, "users": {}, "property_overrides": {} },
        "engine": {
            "grid": { "kind": "square", "size": 100.0 }, "background": null
        },
        "system": {},
        "created_at": 0,
        "updated_at": 0,
    })
}

fn ruler_update(scene: Uuid, points: serde_json::Value, active: bool) -> Message {
    ruler_update_from(scene, points, active, "")
}

/// A `ruler_update` frame carrying the sending connection's `origin` token.
fn ruler_update_from(
    scene: Uuid,
    points: serde_json::Value,
    active: bool,
    origin: &str,
) -> Message {
    Message::Text(
        serde_json::json!({
            "type": "ruler_update", "scene": scene, "points": points, "active": active,
            "origin": origin
        })
        .to_string(),
    )
}

/// GM `a` (the world owner) and player `b` joined to the harness world, with `scene_docs`
/// created by the GM and observed by both.
async fn ruler_room(h: &Harness, scene_docs: Vec<serde_json::Value>) -> (Ws, Ws, Uuid) {
    let cookie_b = h.add_member("b", WorldRole::Player).await;
    let b_id = h
        .repo
        .member_id_by_username(h.world, "b")
        .await
        .unwrap()
        .unwrap();
    let mut a = h.connect().await;
    let mut b = h.connect_with(&cookie_b).await;
    recv_until(&mut a, "welcome").await;
    recv_until(&mut b, "welcome").await;
    let ops: Vec<serde_json::Value> = scene_docs
        .into_iter()
        .map(|d| serde_json::json!({ "op": "create", "doc": d }))
        .collect();
    a.send(intent_msg(1, serde_json::Value::Array(ops)))
        .await
        .unwrap();
    recv_until(&mut a, "event").await;
    recv_until(&mut b, "event").await;
    (a, b, b_id)
}

#[tokio::test]
async fn ruler_broadcast_relayed_to_the_room() {
    let h = spawn().await;
    let scene = test_doc_id(9101);
    let (mut a, mut b, b_id) =
        ruler_room(&h, vec![ruler_scene_doc(h.world, scene, "observer")]).await;

    b.send(ruler_update(
        scene,
        serde_json::json!([[12.0, 34.0], [56.0, 78.0]]),
        true,
    ))
    .await
    .unwrap();

    // The GM receives the sender-stamped frame whole.
    let r = recv_until(&mut a, "ruler").await;
    assert_eq!(r["from"], serde_json::to_value(b_id).unwrap());
    assert_eq!(r["scene"], serde_json::to_value(scene).unwrap());
    assert_eq!(r["points"], serde_json::json!([[12.0, 34.0], [56.0, 78.0]]));
    assert_eq!(r["active"], true);

    // Stopping is delivered with no points.
    b.send(ruler_update(scene, serde_json::json!([]), false))
        .await
        .unwrap();
    let stop = recv_until(&mut a, "ruler").await;
    assert_eq!(stop["active"], false);
    assert_eq!(stop["points"], serde_json::json!([]));
    // `b` receives its own two frames too; they are consumed here so the origin check below
    // reads the frame it is about.
    assert_eq!(recv_until(&mut b, "ruler").await["active"], true);
    assert_eq!(recv_until(&mut b, "ruler").await["active"], false);

    // The relay is `ScenePing`'s: the sender's own connection receives the frame too, with the
    // origin token it sent, so the originating client can recognize and drop it.
    a.send(ruler_update_from(
        scene,
        serde_json::json!([[5.0, 5.0]]),
        true,
        "conn-a",
    ))
    .await
    .unwrap();
    for (who, w) in [("the sender", &mut a), ("the other member", &mut b)] {
        let r = recv_until(w, "ruler").await;
        assert_eq!(r["points"], serde_json::json!([[5.0, 5.0]]), "{who}");
        assert_eq!(r["origin"], "conn-a", "{who} receives the origin unchanged");
    }
    // Out-of-band: no authoritative event beyond the join seed and the scene create.
    assert_eq!(h.authoritative_seqs().await, vec![1, 2]);
}

#[tokio::test]
async fn ruler_update_silently_dropped_when_sender_lacks_read() {
    let h = spawn().await;
    let hidden = test_doc_id(9102);
    let visible = test_doc_id(9103);
    let (mut a, mut b, _) = ruler_room(
        &h,
        vec![
            ruler_scene_doc(h.world, hidden, "none"),
            ruler_scene_doc(h.world, visible, "observer"),
        ],
    )
    .await;

    // Send order is preserved, so a hidden-scene frame that slipped through would reach `a`
    // BEFORE the legitimate one.
    b.send(ruler_update(hidden, serde_json::json!([[1.0, 1.0]]), true))
        .await
        .unwrap();
    b.send(ruler_update(visible, serde_json::json!([[2.0, 2.0]]), true))
        .await
        .unwrap();

    let r = recv_until(&mut a, "ruler").await;
    assert_eq!(r["scene"], serde_json::to_value(visible).unwrap());
    assert_eq!(r["points"], serde_json::json!([[2.0, 2.0]]));
}

#[tokio::test]
async fn ruler_update_rejected_past_max_points() {
    let h = spawn().await;
    let scene = test_doc_id(9104);
    let (mut a, mut b, _) = ruler_room(&h, vec![ruler_scene_doc(h.world, scene, "observer")]).await;

    let pts = |n: usize| {
        serde_json::Value::Array((0..n).map(|i| serde_json::json!([i as f64, 0.0])).collect())
    };
    // One past the cap fails to parse and is answered to the sender alone; a coordinate past the
    // magnitude bound is dropped silently.
    b.send(ruler_update(scene, pts(257), true)).await.unwrap();
    assert_eq!(
        recv_until(&mut b, "error").await["code"],
        "bad_message",
        "an over-cap frame is a malformed frame, answered like any other"
    );
    b.send(ruler_update(
        scene,
        serde_json::json!([[1.0e12, 0.0]]),
        true,
    ))
    .await
    .unwrap();
    // Exactly the cap is admitted, and is the FIRST ruler `a` receives.
    b.send(ruler_update(scene, pts(256), true)).await.unwrap();

    let r = recv_until(&mut a, "ruler").await;
    assert_eq!(r["points"].as_array().unwrap().len(), 256);
    // The sender is echoed its own admitted frame; the out-of-range frame sent before it drew no
    // reply, so no `error` arrives ahead of that echo.
    let (echo, skipped) = recv_until_collecting(&mut b, "ruler").await;
    assert_eq!(echo["points"].as_array().unwrap().len(), 256);
    assert!(
        !skipped.iter().any(|ty| ty == "error"),
        "an out-of-range coordinate is dropped silently, got frames {skipped:?}"
    );
}

/// `JSON.stringify` turns `NaN` and `Infinity` into `null`, which `f64` deserialization rejects:
/// a non-finite coordinate reaches the server as a malformed frame and the sender alone receives
/// the generic `bad_message`; only a FINITE coordinate past the magnitude bound is dropped
/// silently. Both are measured through the real socket, then a valid frame proves the
/// connection survives and the first ruler `a` sees is that valid one.
#[tokio::test]
async fn ruler_update_null_coordinate_is_a_bad_message_and_out_of_range_is_dropped_silently() {
    let h = spawn().await;
    let scene = test_doc_id(9114);
    let (mut a, mut b, _) = ruler_room(&h, vec![ruler_scene_doc(h.world, scene, "observer")]).await;

    b.send(ruler_update(scene, serde_json::json!([[null, 0.0]]), true))
        .await
        .unwrap();
    assert_eq!(
        recv_until(&mut b, "error").await["code"],
        "bad_message",
        "a null coordinate (a serialized NaN or Infinity) is a malformed frame"
    );
    b.send(ruler_update(
        scene,
        serde_json::json!([[1.0e12, 0.0]]),
        true,
    ))
    .await
    .unwrap();
    b.send(ruler_update(scene, serde_json::json!([[3.0, 3.0]]), true))
        .await
        .unwrap();

    let r = recv_until(&mut a, "ruler").await;
    assert_eq!(
        r["points"],
        serde_json::json!([[3.0, 3.0]]),
        "the out-of-range frame relayed nothing and the connection survived the malformed one"
    );
    // The sender is echoed the valid frame; the out-of-range frame sent before it drew no reply,
    // so no `error` arrives between the `bad_message` above and that echo.
    let (echo, skipped) = recv_until_collecting(&mut b, "ruler").await;
    assert_eq!(echo["points"], serde_json::json!([[3.0, 3.0]]));
    assert!(
        !skipped.iter().any(|ty| ty == "error"),
        "an out-of-range coordinate is dropped silently, got frames {skipped:?}"
    );
}

#[tokio::test]
async fn ruler_update_refused_when_scene_belongs_to_a_different_world() {
    let h = spawn().await;
    let scene = test_doc_id(9105);
    let (mut a, mut b, _) = ruler_room(&h, vec![ruler_scene_doc(h.world, scene, "observer")]).await;

    // The seeded owner also owns a second world holding its own scene.
    let owner = h
        .repo
        .member_id_by_username(h.world, "u")
        .await
        .unwrap()
        .unwrap();
    let other = h.repo.create_world_owned("other", owner, 0).await.unwrap();
    let foreign = test_doc_id(9106);
    let mut o = h.connect_world(other.id, &h.cookie).await;
    recv_until(&mut o, "welcome").await;
    o.send(intent_msg(
        1,
        serde_json::json!([{ "op": "create", "doc": ruler_scene_doc(other.id, foreign, "observer") }]),
    ))
    .await
    .unwrap();
    recv_until(&mut o, "event").await;

    b.send(ruler_update(foreign, serde_json::json!([[1.0, 1.0]]), true))
        .await
        .unwrap();
    b.send(ruler_update(scene, serde_json::json!([[2.0, 2.0]]), true))
        .await
        .unwrap();

    let r = recv_until(&mut a, "ruler").await;
    assert_eq!(
        r["scene"],
        serde_json::to_value(scene).unwrap(),
        "the foreign-world scene is refused"
    );
}

/// A ruler is relayed unclipped: one GM ruler, three recipients with different sight (a GM, a
/// player whose sight is walled off from most of the route, a player with no token on the
/// scene). Every recipient receives the full route and the stop. The payloads are compared,
/// not repository state.
#[tokio::test]
async fn ruler_is_relayed_unclipped_to_every_recipient_whatever_their_sight() {
    let h = spawn().await;
    let scene = test_doc_id(9107);
    let cookie_gm2 = h.add_member("gm2", WorldRole::Gm).await;
    let cookie_p1 = h.add_member("p1", WorldRole::Player).await;
    let cookie_p2 = h.add_member("p2", WorldRole::Player).await;
    let p1 = h
        .repo
        .member_id_by_username(h.world, "p1")
        .await
        .unwrap()
        .unwrap();
    let mut sender = h.connect().await; // world owner (GM)
    let mut gm2 = h.connect_with(&cookie_gm2).await;
    let mut p1_ws = h.connect_with(&cookie_p1).await;
    let mut p2_ws = h.connect_with(&cookie_p2).await;
    for w in [&mut sender, &mut gm2, &mut p1_ws, &mut p2_ws] {
        recv_until(w, "welcome").await;
    }

    // p1 owns a token at (50,50) with a sight wall at x=100; p2 owns nothing.
    let token = serde_json::json!({
        "id": test_doc_id(9108),
        "scope": { "kind": "world", "world_id": h.world },
        "doc_type": "token",
        "schema_version": 1,
        "owner": serde_json::to_value(p1).unwrap(),
        "parent_id": scene,
        "engine": { "x": 50.0, "y": 50.0, "w": 1.0, "h": 1.0, "rotation": 0.0 },
        "system": {},
        "created_at": 0,
        "updated_at": 0,
    });
    let wall = serde_json::json!({
        "id": test_doc_id(9109),
        "scope": { "kind": "world", "world_id": h.world },
        "doc_type": "wall",
        "schema_version": 1,
        "parent_id": scene,
        "engine": { "seg": { "x1": 100, "y1": -500, "x2": 100, "y2": 500 }, "blocksSight": true },
        "system": {},
        "created_at": 0,
        "updated_at": 0,
    });
    sender
        .send(intent_msg(
            1,
            serde_json::json!([
                { "op": "create", "doc": ruler_scene_doc(h.world, scene, "observer") },
                { "op": "create", "doc": token },
                { "op": "create", "doc": wall },
            ]),
        ))
        .await
        .unwrap();
    for w in [&mut sender, &mut gm2, &mut p1_ws, &mut p2_ws] {
        recv_until(w, "event").await;
    }

    // Only (60,50) and (70,50) lie inside p1's sight; p2 sees nothing at all.
    let full = serde_json::json!([[250.0, 50.0], [60.0, 50.0], [70.0, 50.0], [260.0, 50.0]]);
    sender
        .send(ruler_update(scene, full.clone(), true))
        .await
        .unwrap();

    for (who, w) in [
        ("a GM", &mut gm2),
        ("a player with walled-off sight", &mut p1_ws),
        ("a player with no token", &mut p2_ws),
    ] {
        let r = recv_until(w, "ruler").await;
        assert_eq!(r["points"], full, "{who} receives the full route");
        assert_eq!(r["active"], true);
    }

    sender
        .send(ruler_update(scene, serde_json::json!([]), false))
        .await
        .unwrap();
    for (who, w) in [
        ("a GM", &mut gm2),
        ("a player with walled-off sight", &mut p1_ws),
        ("a player with no token", &mut p2_ws),
    ] {
        let stop = recv_until(w, "ruler").await;
        assert_eq!(stop["active"], false, "{who} receives the stop");
        assert_eq!(stop["points"], serde_json::json!([]));
    }
}

/// The server is the definition of a frame: an update flagged inactive reaches recipients with
/// no points, and an update flagged active with no points reaches them as the stop.
#[tokio::test]
async fn ruler_frames_are_canonicalised_by_the_server() {
    let h = spawn().await;
    let scene = test_doc_id(9113);
    let (mut a, mut b, _) = ruler_room(&h, vec![ruler_scene_doc(h.world, scene, "observer")]).await;

    b.send(ruler_update(
        scene,
        serde_json::json!([[0.0, 0.0], [1.0, 1.0]]),
        false,
    ))
    .await
    .unwrap();
    let stop = recv_until(&mut a, "ruler").await;
    assert_eq!(stop["active"], false);
    assert_eq!(stop["points"], serde_json::json!([]));

    b.send(ruler_update(scene, serde_json::json!([]), true))
        .await
        .unwrap();
    let empty_active = recv_until(&mut a, "ruler").await;
    assert_eq!(empty_active["active"], false);
    assert_eq!(empty_active["points"], serde_json::json!([]));

    b.send(ruler_update(scene, serde_json::json!([[2.0, 2.0]]), true))
        .await
        .unwrap();
    let live = recv_until(&mut a, "ruler").await;
    assert_eq!(live["active"], true);
    assert_eq!(live["points"], serde_json::json!([[2.0, 2.0]]));
}

/// Two connections of the SAME user both receive the frame, each carrying the origin of the
/// connection that sent it, so a client can drop only its own connection's frame: the second
/// device has no local preview to stand in for the ruler.
#[tokio::test]
async fn ruler_reaches_every_connection_of_the_senders_user_with_the_origin_unchanged() {
    let h = spawn().await;
    let scene = test_doc_id(9111);
    let (mut a1, mut b, _) =
        ruler_room(&h, vec![ruler_scene_doc(h.world, scene, "observer")]).await;
    let mut a2 = h.connect().await; // the same user as `a1`, a second device
    recv_until(&mut a2, "welcome").await;

    a1.send(ruler_update_from(
        scene,
        serde_json::json!([[1.0, 2.0], [3.0, 4.0]]),
        true,
        "device-1",
    ))
    .await
    .unwrap();

    let owner = h
        .repo
        .member_id_by_username(h.world, "u")
        .await
        .unwrap()
        .unwrap();
    for (who, w) in [
        ("the originating connection", &mut a1),
        ("the same user's second connection", &mut a2),
        ("another member", &mut b),
    ] {
        let r = recv_until(w, "ruler").await;
        assert_eq!(
            r["points"],
            serde_json::json!([[1.0, 2.0], [3.0, 4.0]]),
            "{who}"
        );
        assert_eq!(r["origin"], "device-1", "{who}");
        assert_eq!(
            r["from"],
            serde_json::to_value(owner).unwrap(),
            "{who}: the frame names the sending user"
        );
    }
}

/// The origin token is bounded like the other payload fields: past `MAX_RULER_ORIGIN_BYTES`
/// the frame is dropped silently, at the bound it is relayed.
#[tokio::test]
async fn ruler_update_rejected_past_max_origin_bytes() {
    let h = spawn().await;
    let scene = test_doc_id(9112);
    let (mut a, mut b, _) = ruler_room(&h, vec![ruler_scene_doc(h.world, scene, "observer")]).await;

    let pts = serde_json::json!([[1.0, 1.0]]);
    b.send(ruler_update_from(scene, pts.clone(), true, &"x".repeat(65)))
        .await
        .unwrap();
    b.send(ruler_update_from(scene, pts, true, &"y".repeat(64)))
        .await
        .unwrap();
    let r = recv_until(&mut a, "ruler").await;
    assert_eq!(
        r["origin"],
        "y".repeat(64),
        "the oversize origin was dropped; the first frame relayed is the one at the bound"
    );
}

/// The ingress arm spends the sender's ruler budget: exactly `ratePerMin` frames (the shared
/// fixture's number, read on both sides) are relayed, and the next ones never reach the room.
/// Sender and receiver drain in lockstep, so no broadcast receiver can fall behind and the
/// count does not depend on egress timing.
#[tokio::test]
async fn ruler_updates_are_rate_limited_per_user() {
    let budget = serde_json::from_str::<serde_json::Value>(include_str!(
        "../../client/core/src/__fixtures__/ruler-limits.json"
    ))
    .unwrap()["ratePerMin"]
        .as_u64()
        .unwrap() as usize;
    let h = spawn().await;
    let scene = test_doc_id(9110);
    let (mut a, mut b, _) = ruler_room(&h, vec![ruler_scene_doc(h.world, scene, "observer")]).await;

    for i in 0..budget {
        b.send(ruler_update(
            scene,
            serde_json::json!([[i as f64, 0.0]]),
            true,
        ))
        .await
        .unwrap();
        let relayed = recv_until(&mut a, "ruler").await;
        assert_eq!(relayed["points"][0][0].as_f64().unwrap(), i as f64);
        recv_until(&mut b, "ruler").await; // the sender's own copy, drained in lockstep
    }
    for _ in 0..3 {
        b.send(ruler_update(
            scene,
            serde_json::json!([[9999.0, 0.0]]),
            true,
        ))
        .await
        .unwrap();
    }
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(500),
            recv_until(&mut a, "ruler")
        )
        .await
        .is_err(),
        "frames past the budget are dropped, never relayed"
    );
}
