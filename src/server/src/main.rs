use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use shadowcat::config::{Cli, Config};
use shadowcat::data::sqlite::SqliteRepository;
use shadowcat::http::{self, AppState};

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = Config::load(cli)?;
    init_tracing();

    let repo = SqliteRepository::connect(&config.db).await?;
    std::fs::create_dir_all(config.assets_path())?;

    // Headless bootstrap (remote hosting): seed admin from config if present.
    let seeded = shadowcat::auth::setup::bootstrap_admin(&repo, &config).await?;
    let initialized = seeded || repo.admin_exists().await?;
    let setup_token = AppState::resolve_setup_token(&config);

    let state = AppState {
        repo: Arc::new(repo),
        config: Arc::new(config.clone()),
        setup_token,
        initialized: Arc::new(AtomicBool::new(initialized)),
        ws: shadowcat::ws::WsState::new(),
        upload_rate: Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
    };

    shadowcat::auth::session::spawn_session_sweep(&state.repo);

    let app = http::router(state).await;
    let listener = tokio::net::TcpListener::bind(&config.bind).await?;
    tracing::info!(bind = %config.bind, "shadowcat listening");
    axum::serve(listener, app).await?;
    Ok(())
}

/// Structured logging filtered by SHADOWCAT_LOG (falling back to RUST_LOG, then "info").
fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = std::env::var("SHADOWCAT_LOG")
        .ok()
        .and_then(|s| EnvFilter::try_new(s).ok())
        .or_else(|| EnvFilter::try_from_default_env().ok())
        .unwrap_or_else(|| EnvFilter::new("info"));
    fmt().with_env_filter(filter).init();
}
