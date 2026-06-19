//! Throwaway WS server for manual/external clients and the Node<->Rust e2e
//! harness. In-memory DB seeded with a GM (gm/pw) and a player (pl/pw), one
//! world, a player-owned document, and a declarative capability requirement on
//! `/system/vision`. Prints the bind address and a machine-readable
//! `e2e-fixture:` JSON line the harness parses.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use shadowcat::auth::password::hash_password;
use shadowcat::auth::role::ServerRole;
use shadowcat::config::Config;
use shadowcat::data::command::Operation;
use shadowcat::data::document::{
    CapabilityRequirement, DocRole, Document, PermissionSet, Scope, WorldRole,
};
use shadowcat::data::membership::PermissionContext;
use shadowcat::data::repository::Repository;
use shadowcat::data::sqlite::SqliteRepository;
use shadowcat::http::{self, AppState};
use uuid::Uuid;

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

    // A player-owned actor carrying a populated /system/vision subtree.
    let mut perms = PermissionSet::default();
    perms.users.insert(player, DocRole::Owner);
    let doc = Document {
        id: Uuid::new_v4(),
        scope: Scope::World { world_id: world.id },
        doc_type: "actor".into(),
        schema_version: 1,
        source: None,
        owner: Some(player),
        permissions: perms,
        embedded: Default::default(),
        system: serde_json::json!({ "vision": { "range": 30 }, "hp": 10 }),
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
    )
    .await?;

    // Writing /system/vision requires dnd5e:gm_vision (which the player lacks).
    repo.set_world_cap_requirements(
        world.id,
        &[CapabilityRequirement {
            path_prefix: "/system/vision".into(),
            caps: ["dnd5e:gm_vision".to_string()].into_iter().collect(),
        }],
    )
    .await?;

    let state = AppState {
        repo,
        config: Arc::new(Config::default()),
        setup_token: None,
        initialized: Arc::new(AtomicBool::new(true)),
        ws: shadowcat::ws::WsState::new(),
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
