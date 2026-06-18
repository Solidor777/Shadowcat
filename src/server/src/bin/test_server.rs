//! Throwaway WS server for manual/external clients. In-memory DB, one seeded
//! user (u/pw) and one world; prints the bind address and world id.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use shadowcat::auth::password::hash_password;
use shadowcat::auth::role::ServerRole;
use shadowcat::config::Config;
use shadowcat::data::sqlite::SqliteRepository;
use shadowcat::http::{self, AppState};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await?);
    let world = repo.create_world("test", 0).await?;
    let hash = hash_password("pw")?;
    repo.create_user("u", Some(&hash), ServerRole::User, 0)
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
    tracing::info!(%addr, world = %world.id, "test_server listening (user: u / pw)");
    println!("test_server: http://{addr}  world={}  login u/pw", world.id);
    axum::serve(listener, app).await?;
    Ok(())
}
