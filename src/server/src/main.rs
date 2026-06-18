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
    let initialized = repo.admin_exists().await?;

    let state = AppState {
        repo: Arc::new(repo),
        config: Arc::new(config.clone()),
        setup_token: None,
        initialized: Arc::new(AtomicBool::new(initialized)),
    };

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
