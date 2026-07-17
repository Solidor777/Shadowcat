//! Throwaway WS server for manual/external clients and the Node<->Rust e2e
//! harness. In-memory DB seeded with a GM (gm/pw) and a player (pl/pw), one
//! world, a player-owned document, and a declarative capability requirement on
//! `/engine/vision`. Prints the bind address and a machine-readable
//! `e2e-fixture:` JSON line the harness parses.

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

/// `--modules-dir <path>`: overrides the modules folder the embedded router
/// scans/serves from (default: none installed). Lets the Node<->Rust e2e
/// harness — and an external module repo's own smoke script (see
/// `docs/design/module-authoring.md`) — point a fresh `test_server` at a
/// fixture-populated temp folder without touching the hardcoded in-memory
/// fixture data below.
#[derive(Parser, Debug, Default)]
struct Args {
    #[arg(long)]
    modules_dir: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await?);
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

    // A player-owned actor (engine-defined per M13-0 S1) carrying a populated
    // /engine/vision subtree; `name` lives on the envelope (S2), `hp` stays
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
        owner: Some(player),
        permissions: perms,
        embedded: Default::default(),
        parent_id: None,
        engine: Some(serde_json::json!({
            "displayName": "Player Dragon",
            "visual": { "kind": "image", "asset": "dragon.png" },
            "size": { "w": 1.0, "h": 1.0 },
            "shape": "square",
            "faction": null,
            "conditions": [],
            "prototype": true,
            // A whole-number range: `apply_intent`'s OCC pre-image comparison
            // is numeric-aware (`values_semantically_eq` in `data/sqlite.rs`)
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
        owner: Some(gm),
        permissions: PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
        embedded: Default::default(),
        parent_id: None,
        engine: Some(serde_json::json!({
            "displayName": "Secret Dragon",
            "visual": { "kind": "image", "asset": "dragon.png" },
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

    let args = Args::parse();
    let mut config = Config::default();
    if let Some(dir) = args.modules_dir {
        config.modules_dir = Some(dir);
    }
    let state = AppState {
        repo,
        config: Arc::new(config),
        setup_token: None,
        initialized: Arc::new(AtomicBool::new(true)),
        ws: shadowcat::ws::WsState::new(),
        upload_rate: Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
    };
    let app = http::router(state).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
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
    axum::serve(listener, app).await?;
    Ok(())
}
