//! Shared WS integration-test harness for the scene tests: a real WS client
//! against an ephemeral in-process server whose single seeded user owns the
//! world (GM), so its intents are authorized.
//!
//! A library crate rather than a shared module included by each integration-test binary: such a
//! module compiles whole while each binary uses only part of it, so every unused
//! helper is dead code there and the module needs a blanket suppression. A `pub` item in a
//! library is reachable by definition, so no such diagnostic fires and nothing is silenced.
//! Reaching this crate only through `shadowcat`'s [dev-dependencies] keeps it out of release
//! builds.
#![deny(missing_docs)]

/// The HTTP/2 implementation the client helpers use, re-exported so tests name one version.
pub use h2;
/// A serve loop with routes that stall in controlled ways, and a paced HTTP/2 client.
pub mod egress;
/// Fixtures for opening a WebSocket inside an HTTP/2 connection.
pub mod h2_session;
/// HTTP/2 with prior knowledge over plain TCP: a serve-loop source and a client.
pub mod h2c;
/// Log capture for tests that assert on what the server logs.
pub mod logs;
/// A serve-loop source that records when the server drops each connection.
pub mod observed;
/// TLS client helpers.
pub mod tls;

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

/// A live in-process server plus everything needed to talk to it: an HTTP client already
/// authenticated as the seeded user, that user's world, and direct handles to the storage and
/// WebSocket registry behind the socket.
///
/// The seeded user owns `world`, so its intents authorize as GM. A test needing a non-GM
/// perspective must create and log in a second user rather than reusing `cookie`.
pub struct Harness {
    /// `host:port` the ephemeral server bound to. The port is OS-assigned, so concurrent test
    /// binaries never collide.
    pub addr: String,
    /// Session cookie for the seeded GM user, ready to attach to a request or a WS handshake.
    pub cookie: String,
    /// HTTP client with a cookie jar, already logged in as the seeded user.
    pub client: reqwest::Client,
    /// The seeded user's id.
    pub user: Uuid,
    /// The world the seeded user owns.
    pub world: Uuid,
    /// The same repository the running server writes through, for asserting persisted state
    /// directly instead of reading it back over HTTP.
    pub repo: Arc<SqliteRepository>,
    /// Registry handle for driving rooms directly, bypassing the socket.
    pub ws: shadowcat::ws::WsState,
    /// Per-run asset root; each world's directory lives beneath it. Distinct per harness, so
    /// asset writes from concurrent tests cannot overwrite each other.
    pub assets_dir: std::path::PathBuf,
    /// The TLS runtime the server was started with (`TlsRuntime::disabled()` for [`spawn`] /
    /// [`spawn_with`]; the real ladder for [`spawn_tls`] / [`spawn_tls_with`]).
    pub tls: Arc<shadowcat::tls::TlsRuntime>,
    /// The effective configuration the server was started with.
    pub config: Arc<Config>,
}

/// Start a harness with the default configuration.
///
/// Equivalent to [`spawn_with`] with a no-op mutator; use that instead when a test needs to
/// change the config the server starts with.
pub async fn spawn() -> Harness {
    spawn_with(|_| {}).await
}

/// Like `spawn`, but `mutate` can tweak the `Config` before the server starts
/// (e.g. set a tiny upload cap). Uses a per-run tempdir for asset storage. The server answers
/// plain HTTP only (TLS off), served through `http::into_service` so `ConnectInfo<ConnMeta>` has
/// the production shape.
pub async fn spawn_with(mutate: impl FnOnce(&mut Config)) -> Harness {
    spawn_inner(mutate, TlsSetup::Off).await
}

/// Start a harness whose ONE port answers both plain HTTP and TLS (the production listener),
/// with an in-memory self-signed certificate.
pub async fn spawn_tls() -> Harness {
    spawn_tls_with(|_| {}).await
}

/// Like [`spawn_tls`], with a `Config` mutator applied before the TLS ladder runs.
pub async fn spawn_tls_with(mutate: impl FnOnce(&mut Config)) -> Harness {
    spawn_inner(mutate, TlsSetup::Start).await
}

/// Like [`spawn_tls_with`], but the TLS runtime is built by `make_runtime` from the mutated
/// `Config` (for example with an injected certificate generator), while the SAME production
/// sniffing listener serves the port.
pub async fn spawn_with_runtime(
    mutate: impl FnOnce(&mut Config),
    make_runtime: impl FnOnce(&Config) -> Arc<shadowcat::tls::TlsRuntime> + 'static,
) -> Harness {
    spawn_inner(mutate, TlsSetup::With(Box::new(make_runtime))).await
}

/// Builds a caller-supplied TLS runtime from the harness `Config`.
type RuntimeFactory = Box<dyn FnOnce(&Config) -> Arc<shadowcat::tls::TlsRuntime>>;

/// How a harness builds its TLS runtime.
enum TlsSetup {
    /// TLS off: the plain `TcpListener` serves the router.
    Off,
    /// The production ladder over the configuration.
    Start,
    /// A caller-built runtime.
    With(RuntimeFactory),
}

/// Shared body of every `spawn*` function.
async fn spawn_inner(mutate: impl FnOnce(&mut Config), setup: TlsSetup) -> Harness {
    let assets_dir = std::env::temp_dir().join(format!("shadowcat-assets-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&assets_dir).unwrap();
    let mut cfg = Config {
        assets_dir: Some(assets_dir.to_string_lossy().into_owned()),
        db: "sqlite::memory:".into(),
        ..Config::default()
    };
    mutate(&mut cfg);
    let cfg = Arc::new(cfg);

    let repo = Arc::new(
        SqliteRepository::connect("sqlite::memory:")
            .await
            .unwrap()
            .with_modules_dir(cfg.modules_path()),
    );
    let hash = hash_password("pw").unwrap();
    let uid = repo
        .create_user("u", Some(&hash), ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("test", uid, 0).await.unwrap();

    shadowcat::tls::install_default_provider();
    let tls_on = !matches!(setup, TlsSetup::Off);
    let tls = match setup {
        TlsSetup::Off => shadowcat::tls::TlsRuntime::disabled(),
        TlsSetup::Start => shadowcat::tls::TlsRuntime::start(&cfg),
        TlsSetup::With(make) => make(&cfg),
    };
    let state = AppState {
        repo: repo.clone(),
        config: cfg.clone(),
        setup_token: None,
        initialized: Arc::new(AtomicBool::new(true)),
        ws: shadowcat::ws::WsState::new(),
        upload_rate: Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
        uploads: Arc::new(shadowcat::http::assets::uploads::UploadSessions::new()),
        auth_throttle: Arc::new(shadowcat::http::throttle::AuthThrottle::new()),
        write_barrier: Arc::new(tokio::sync::RwLock::new(())),
        preview_fetch_locks: Arc::new(dashmap::DashMap::new()),
        tls: tls.clone(),
    };
    let ws_state = state.ws.clone();
    let app = http::router(state).await;

    let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = tcp.local_addr().unwrap().to_string();
    if tls_on {
        let listener = shadowcat::tls::listener::SniffListener::new(
            tcp,
            tls.clone(),
            cfg.clone(),
            shadowcat::tls::listener::Limits::default(),
        );
        tokio::spawn(http::conn_serve::serve_router(
            listener,
            app,
            std::future::pending(),
            http::conn_serve::ConnLimits::default(),
            || {},
        ));
    } else {
        tokio::spawn(http::conn_serve::serve_router(
            tcp,
            app,
            std::future::pending(),
            http::conn_serve::ConnLimits::default(),
            || {},
        ));
    }

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
        ws: ws_state,
        assets_dir,
        tls,
        config: cfg,
    }
}

/// A connected client WebSocket to the harness server. Named because the underlying
/// `tokio_tungstenite` type is long enough to obscure every signature that returns one.
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

    /// Replace asset `id`'s bytes with `bytes` as `name`; returns the raw response.
    pub async fn replace(
        &self,
        id: &str,
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
            .post(format!("http://{}/api/assets/{}/replace", self.addr, id))
            .multipart(form)
            .send()
            .await
            .unwrap()
    }

    /// DER of the certificate an IP-literal / SNI-less connection receives (the self-signed
    /// fallback).
    pub fn fallback_cert_der(&self) -> rustls::pki_types::CertificateDer<'static> {
        let snapshot = self
            .tls
            .resolver()
            .expect("TLS is enabled for this harness")
            .snapshot();
        snapshot.fallback().certified_key().cert[0].clone()
    }

    /// The socket address the server listens on.
    pub fn socket_addr(&self) -> std::net::SocketAddr {
        self.addr.parse().unwrap()
    }

    /// A `reqwest` client (cookie jar on) that trusts exactly this server's self-signed
    /// certificate, so the handshake validates the SAN, key usage and validity for real.
    /// `add_root_certificate` alone ADDS to reqwest's built-in root store (its
    /// `tls_built_in_root_certs` defaults to on), so the built-in roots are switched off
    /// to make "exactly" true.
    pub fn https_client(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .cookie_store(true)
            .tls_built_in_root_certs(false)
            .add_root_certificate(
                reqwest::Certificate::from_der(self.fallback_cert_der().as_ref()).unwrap(),
            )
            .build()
            .unwrap()
    }

    /// Open a `wss://` WebSocket as the given session `cookie` over HTTP/1.1, trusting this
    /// server's certificate.
    pub async fn connect_tls_as(&self, cookie: &str) -> Ws {
        let url = format!("wss://{}/ws?world={}", self.addr, self.world);
        let mut req = url.into_client_request().unwrap();
        req.headers_mut().insert("cookie", cookie.parse().unwrap());
        let cfg = tls::trusting_config(&[self.fallback_cert_der()], &["http/1.1"]);
        let (ws, _) = tokio_tungstenite::connect_async_tls_with_config(
            req,
            None,
            false,
            Some(tokio_tungstenite::Connector::Rustls(cfg)),
        )
        .await
        .unwrap();
        ws
    }

    /// Open a WebSocket as the seeded GM user. Use [`Harness::connect_as`] to connect as a
    /// different user.
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

/// Deterministic v4-shaped document id for test fixtures: same low-order bytes as
/// `Uuid::from_u128(n)`, with the version/variant nibbles forced to v4/RFC4122 so it satisfies
/// `shadowcat::data::validation::validate_document_id`. Two distinct small `n` differ only in
/// their low-order bytes, so this stays injective for every id this suite constructs. Not a real
/// random id: only for document-id fixtures (correlation ids like `request_id`/`intent_id` are
/// never validated this way and stay on plain `Uuid::from_u128`).
pub fn test_doc_id(n: u128) -> Uuid {
    let mut bytes = n.to_be_bytes();
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
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

/// A minimal valid `engine` body for `doc_type` (mirrors
/// `data::engine::validate_engine`'s battery), `None` for a non-engine
/// doc type. `system` above stays the opaque placeholder these fixtures
/// pre-date the engine band with — the read-path re-root onto `engine` is
/// later checkpoint work; this only satisfies the ingress gate.
fn default_engine_for(doc_type: &str) -> Option<serde_json::Value> {
    match doc_type {
        "token" => Some(serde_json::json!({
            "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0
        })),
        "scene" => Some(serde_json::json!({
            "grid": { "kind": "square", "size": 100.0 }, "background": null
        })),
        "wall" => {
            Some(serde_json::json!({ "seg": { "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 1.0 } }))
        }
        "region" => Some(serde_json::json!({
            "shape": { "kind": "rect", "points": [0.0, 0.0, 1.0, 1.0] },
            "behavior": "terrain", "cost": 1.0, "enabled": true
        })),
        "light" => Some(serde_json::json!({
            "x": 0.0, "y": 0.0, "emission": { "color": "#fff", "intensity": 1.0, "brightRadius": 5.0, "dimRadius": 10.0, "enabled": true }
        })),
        "drawing" => Some(serde_json::json!({
            "shape": { "kind": "rect", "points": [0.0, 0.0, 1.0, 1.0] },
            "stroke": null, "fill": null
        })),
        "template" => Some(serde_json::json!({
            "shape": { "kind": "cone", "x": 0.0, "y": 0.0, "size": 5.0, "direction": 0.0 },
            "color": "#f00"
        })),
        "actor" => Some(serde_json::json!({
            "displayName": "Test", "visual": { "kind": "image", "asset": "11111111-1111-1111-1111-111111111111" },
            "size": { "w": 1.0, "h": 1.0 }, "shape": "square",
            "faction": null, "conditions": [], "prototype": true
        })),
        "world-settings" | "vision-modes" | "light-gradation" | "chat-settings"
        | "dice-settings" | "channel-registry" | "faction-registry" | "condition-registry" => {
            // These registry/settings singletons aren't constructed through
            // this generic helper in the current suite; add a case here if
            // a future fixture needs one.
            None
        }
        _ => None,
    }
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
            "id": test_doc_id(id),
            "scope": { "kind": "world", "world_id": world },
            "doc_type": doc_type,
            "schema_version": 1,
            "parent_id": parent.map(test_doc_id),
            "engine": default_engine_for(doc_type),
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
            "id": test_doc_id(id),
            "scope": { "kind": "world", "world_id": world },
            "doc_type": "token",
            "schema_version": 1,
            "owner": owner,
            "parent_id": test_doc_id(scene),
            "engine": { "x": x, "y": y, "w": 100.0, "h": 100.0, "rotation": 0.0 },
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
                "id": test_doc_id(id),
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

/// A `scene_subscribe` frame carrying `as_user` (see-as-player; GM-only, server-authorized).
pub fn scene_subscribe_as(request_n: u64, channel: &str, as_user: Uuid) -> Message {
    Message::Text(
        serde_json::json!({
            "type": "scene_subscribe",
            "request_id": Uuid::from_u128(request_n as u128),
            "channel": channel,
            "as_user": as_user,
        })
        .to_string(),
    )
}

/// Read frames until one of type `ty` arrives (5s budget), returning it.
pub async fn drain_until_type<S>(ws: &mut S, ty: &str) -> serde_json::Value
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
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
pub async fn drain_until_event<S>(ws: &mut S) -> serde_json::Value
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    drain_until_type(ws, "event").await
}
