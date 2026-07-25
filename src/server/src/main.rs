use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use shadowcat::config::{Cli, Config};
use shadowcat::data::sqlite::SqliteRepository;
use shadowcat::http::{self, AppState};

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if cli.backup_to.is_some() && cli.restore_from.is_some() {
        anyhow::bail!("--backup-to and --restore-from are mutually exclusive");
    }
    let backup_to = cli.backup_to.clone();
    let restore_from = cli.restore_from.clone();
    let force = cli.force;

    let config = Config::load(cli)?;

    if let Some(dir) = backup_to {
        init_tracing();
        run_backup(&config, std::path::Path::new(&dir), force).await?;
        return Ok(());
    }
    if let Some(dir) = restore_from {
        init_tracing();
        run_restore(&config, std::path::Path::new(&dir), force).await?;
        return Ok(());
    }

    init_tracing();

    let repo = SqliteRepository::connect(&config.db).await?;
    std::fs::create_dir_all(config.assets_path())?;

    // Runs once at boot purely to surface a summary in the log; every actual
    // read (GET /api/modules, enable-time validation) re-scans fresh, so this
    // never goes stale.
    let discovered = shadowcat::modules::scan_installed_modules(&config.modules_path());
    tracing::info!(count = discovered.len(), "installed modules discovered");

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
        auth_throttle: Arc::new(shadowcat::http::throttle::AuthThrottle::new()),
        write_barrier: Arc::new(tokio::sync::RwLock::new(())),
    };

    shadowcat::auth::session::spawn_session_sweep(&state.repo);

    let app = http::router(state).await;
    let listener = tokio::net::TcpListener::bind(&config.bind).await?;
    tracing::info!(bind = %config.bind, "shadowcat listening");
    // connect-info service so `throttle::ClientIp` resolves a real address
    // (production only — axum-test's mock transport has none, degrading the
    // IP throttle to identity-only there without a 500).
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
    Ok(())
}

/// One-shot `--backup-to` mode: snapshot the resolved db + assets into
/// `out_dir`, print a one-line summary, and return — the caller exits before
/// any server startup runs. Refuses a non-empty `out_dir` unless `force`.
async fn run_backup(config: &Config, out_dir: &std::path::Path, force: bool) -> anyhow::Result<()> {
    if !force && !shadowcat::backup::dir_is_empty_or_absent(out_dir)? {
        anyhow::bail!(
            "refusing to write into non-empty directory {} without --force",
            out_dir.display()
        );
    }
    let manifest = shadowcat::backup::create_backup(
        std::path::Path::new(&config.db),
        &config.assets_path(),
        out_dir,
    )
    .await?;
    println!(
        "backup written to {}: {} asset file(s), {} db byte(s), shadowcat {}",
        out_dir.display(),
        manifest.asset_file_count,
        manifest.db_bytes,
        manifest.shadowcat_version,
    );
    Ok(())
}

/// One-shot `--restore-from` mode: copy a prior `--backup-to` directory over
/// the resolved db + assets, print a one-line summary, and return — never
/// starts the server.
async fn run_restore(
    config: &Config,
    backup_dir: &std::path::Path,
    force: bool,
) -> anyhow::Result<()> {
    shadowcat::backup::restore_backup(
        backup_dir,
        std::path::Path::new(&config.db),
        &config.assets_path(),
        force,
    )
    .await?;
    println!(
        "restored {} into db={} assets={}",
        backup_dir.display(),
        config.db,
        config.assets_path().display(),
    );
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
