//! Throwaway WS server for manual/external clients and the Node<->Rust e2e
//! harness. In-memory DB seeded with a GM (gm/pw) and a player (pl/pw), one
//! world, a player-owned document, and a declarative capability requirement on
//! `/engine/vision`. Prints the bind address and a machine-readable
//! `e2e-fixture:` JSON line the harness parses.
//!
//! The fixture credentials are fixed and public, so `--bind` accepts only a specific interface
//! address (loopback or one assigned to this machine); a wildcard address (`0.0.0.0`, `::`) is
//! refused because it would expose those credentials on every network the machine is attached to.
//!
//! # Examples
//!
//! ```text
//! test_server                                   # in-memory fixture world
//! test_server --modules-dir staged-modules/     # + installed-module discovery
//! ```

// Ratchet: every item in this bin crate must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use shadowcat::auth::password::hash_password;
use shadowcat::auth::role::ServerRole;
use shadowcat::config::Config;
use shadowcat::data::command::{Operation, WriteOrigin};
use shadowcat::data::document::{
    CapabilityRequirement, DocRole, Document, PermissionSet, Scope, WorldRole,
};
use shadowcat::data::membership::PermissionContext;
use shadowcat::data::repository::Repository;
use shadowcat::data::sqlite::SqliteRepository;
use shadowcat::http::{self, AppState};
use uuid::Uuid;

use clap::Parser;

/// Command-line arguments. `--modules-dir <path>` overrides the modules folder the embedded
/// router scans/serves from (default: none installed), letting the Node<->Rust e2e harness — and
/// an external module repo's own smoke script — point a fresh `test_server` at a
/// fixture-populated temp folder without touching the hardcoded in-memory fixture data below.
/// The remaining flags select the listen address and the built-in HTTPS behaviour, mirroring the
/// production `Config` keys of the same names, so the browser suite can drive the server over
/// both schemes on a real (non-loopback) interface address.
#[derive(Parser, Debug, Default)]
struct Args {
    /// Modules folder the embedded router scans/serves; `None` = none installed.
    #[arg(long)]
    modules_dir: Option<String>,
    /// Listen address (`host:port`); default an ephemeral loopback port.
    #[arg(long)]
    bind: Option<String>,
    /// Built-in HTTPS mode (`auto` | `off`).
    #[arg(long, value_enum)]
    tls: Option<shadowcat::config::TlsMode>,
    /// Opt-in HSTS `max-age` seconds.
    #[arg(long)]
    hsts_max_age_secs: Option<u64>,
    /// Operator certificate chain (PEM), pairs with `--tls-key`.
    #[arg(long)]
    tls_cert: Option<String>,
    /// Operator private key (PEM), pairs with `--tls-cert`.
    #[arg(long)]
    tls_key: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let args = Args::parse();
    let mut config = Config::default();
    if let Some(dir) = args.modules_dir {
        config.modules_dir = Some(dir);
    }
    config.bind = args.bind.unwrap_or_else(|| "127.0.0.1:0".into());
    let requested: std::net::SocketAddr = config.bind.parse().map_err(|e| {
        anyhow::anyhow!(
            "--bind {:?} must be an IP:port socket address: {e}",
            config.bind
        )
    })?;
    anyhow::ensure!(
        !requested.ip().to_canonical().is_unspecified(),
        "--bind {}: refused. test_server serves the fixed credentials gm/pw and pl/pw, so it must not listen on every interface; bind a specific interface address (127.0.0.1 or this machine's LAN address)",
        config.bind
    );
    config.db = "sqlite::memory:".into();
    if let Some(mode) = args.tls {
        config.tls = mode;
    }
    config.hsts_max_age_secs = args.hsts_max_age_secs;
    config.tls_cert = args.tls_cert;
    config.tls_key = args.tls_key;
    config.validate_tls()?;
    shadowcat::tls::install_default_provider();
    let tls = shadowcat::tls::TlsRuntime::start(&config);
    let repo = Arc::new(
        SqliteRepository::connect("sqlite::memory:")
            .await?
            .with_modules_dir(config.modules_path()),
    );
    let hash = hash_password("pw")?;

    // GM owns the world; player is a member.
    let gm = repo
        .create_user("gm", Some(&hash), ServerRole::User, 0)
        .await?;
    let world = repo.create_world_owned("test", gm, 0).await?;
    let player = repo
        .create_user("pl", Some(&hash), ServerRole::User, 0)
        .await?;
    repo.add_member(world.id, player, WorldRole::Player).await?;

    // A player-owned actor (engine-defined) carrying a populated
    // /engine/vision subtree; `name` lives on the envelope, `hp` stays
    // opaque game-system data.
    let mut perms = PermissionSet::default();
    perms.users.insert(player, DocRole::Owner);
    let doc = Document {
        id: Uuid::new_v4(),
        scope: Scope::World { world_id: world.id },
        doc_type: "actor".into(),
        schema_version: 1,
        name: Some("Player Dragon".into()),
        source: None,
        base: None,
        owner: Some(player),
        permissions: perms,
        embedded: Default::default(),
        parent_id: None,
        engine: Some(serde_json::json!({
            "displayName": "Player Dragon",
            "visual": { "kind": "image", "asset": "77777777-7777-7777-7777-777777777777" },
            "size": { "w": 1.0, "h": 1.0 },
            "shape": "square",
            "faction": null,
            "conditions": [],
            "prototype": true,
            // A whole-number range: `apply_intent`'s OCC pre-image comparison
            // is numeric-aware (`data::command::values_semantically_eq`)
            // across the serde_json PosInt/Float variant split, so a JS
            // client's whole-number pre-image round-trips correctly here.
            "vision": [{ "mode": "darkvision", "range": 30 }]
        })),
        system: serde_json::json!({ "hp": 10 }),
        created_at: 0,
        updated_at: 0,
    };
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    repo.apply_intent(
        &gm_ctx,
        world.id,
        vec![Operation::Create { doc: doc.clone() }],
        0,
        WriteOrigin::Client,
    )
    .await?;

    // A GM-only document (default None → the player cannot read it) for the e2e
    // search-leak assertion: it also matches "dragon" but must never reach a
    // player's results.
    let secret = Document {
        id: Uuid::new_v4(),
        scope: Scope::World { world_id: world.id },
        doc_type: "actor".into(),
        schema_version: 1,
        name: Some("Secret Dragon".into()),
        source: None,
        base: None,
        owner: Some(gm),
        permissions: PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
        embedded: Default::default(),
        parent_id: None,
        engine: Some(serde_json::json!({
            "displayName": "Secret Dragon",
            "visual": { "kind": "image", "asset": "77777777-7777-7777-7777-777777777777" },
            "size": { "w": 1.0, "h": 1.0 },
            "shape": "square",
            "faction": null,
            "conditions": [],
            "prototype": true
        })),
        system: serde_json::json!({}),
        created_at: 0,
        updated_at: 0,
    };
    repo.apply_intent(
        &gm_ctx,
        world.id,
        vec![Operation::Create { doc: secret }],
        0,
        WriteOrigin::Client,
    )
    .await?;

    // Writing /engine/vision requires dnd5e:gm_vision (which the player lacks).
    repo.set_world_cap_requirements(
        world.id,
        &[CapabilityRequirement {
            path_prefix: "/engine/vision".into(),
            caps: ["dnd5e:gm_vision".to_string()].into_iter().collect(),
        }],
    )
    .await?;

    let state = AppState {
        repo,
        config: Arc::new(config),
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
    shadowcat::http::assets::uploads::spawn_sweeper(
        state.uploads.clone(),
        state.upload_rate.clone(),
    );
    let config = state.config.clone();
    let app = http::router(state).await;
    let tcp = tokio::net::TcpListener::bind(&config.bind).await?;
    let addr = tcp.local_addr()?;
    // The same raise-then-fit `main` runs, so the browser-suite server never serves under a
    // descriptor budget the production binary would not accept (`fd_budget::raise_and_fit`).
    let (sniff_limits, conn_limits) = shadowcat::fd_budget::raise_and_fit(
        shadowcat::tls::listener::Limits::default(),
        http::conn_serve::ConnLimits::for_config(&config),
    );
    let listener = shadowcat::tls::listener::SniffListener::new(
        tcp,
        tls.clone(),
        config.clone(),
        sniff_limits,
    );
    tracing::info!(%addr, world = %world.id, "test_server listening (gm/pw, pl/pw)");
    println!(
        "test_server: http://{addr}  world={}  login gm/pw or pl/pw",
        world.id
    );
    // Machine-readable fixture line for the e2e harness.
    println!(
        "e2e-fixture: {}",
        serde_json::json!({
            "world": world.id, "doc": doc.id, "gm": gm, "player": player
        })
    );
    println!(
        "test_server-tls: {}",
        serde_json::json!({ "enabled": tls.status().enabled, "url": format!("https://{addr}") })
    );
    let end =
        http::conn_serve::serve_router(listener, app, std::future::pending(), conn_limits, || {})
            .await;
    anyhow::bail!("test_server stopped serving: {end:?}")
}
