//! The `shadowcat` executable: serves the embedded web client + API from one
//! process, or runs a one-shot backup/restore and exits.
//!
//! # Examples
//!
//! ```text
//! shadowcat                                     # serve on 127.0.0.1:30000
//! shadowcat --bind 0.0.0.0:30000 --db /srv/shadowcat.db
//! shadowcat --backup-to backups/2026-07-30      # snapshot, then exit
//! shadowcat --restore-from backups/2026-07-30 --force
//! shadowcat audio-monitor --port 31998          # localhost audio-session levels, then serve
//! ```

// Ratchet: every item in this bin crate must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use shadowcat::config::{Cli, Config};
use shadowcat::data::sqlite::SqliteRepository;
use shadowcat::http::{self, AppState};

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut cli = Cli::parse();

    if cli.command.is_some() && (cli.backup_to.is_some() || cli.restore_from.is_some()) {
        anyhow::bail!("a subcommand and --backup-to/--restore-from are mutually exclusive");
    }
    if let Some(shadowcat::config::CliCommand::AudioMonitor(args)) = cli.command.take() {
        // No `Config` is resolved yet on this early-return path (this subcommand never
        // reaches `Config::load` below), so there is no `log_dir` to derive a rotation
        // root from -- stdout-only. A short-lived diagnostic CLI tool gains little from
        // rotated files anyway.
        let _log_guard = init_tracing(None)?;
        return shadowcat::audio_monitor::server::run(args).await;
    }

    if cli.backup_to.is_some() && cli.restore_from.is_some() {
        anyhow::bail!("--backup-to and --restore-from are mutually exclusive");
    }
    let backup_to = cli.backup_to.clone();
    let restore_from = cli.restore_from.clone();
    let force = cli.force;

    let config = Config::load(cli)?;

    if let Some(dir) = backup_to {
        let _log_guard = init_tracing(config.logs_path().as_deref())?;
        run_backup(&config, std::path::Path::new(&dir), force).await?;
        return Ok(());
    }
    if let Some(dir) = restore_from {
        let _log_guard = init_tracing(config.logs_path().as_deref())?;
        run_restore(&config, std::path::Path::new(&dir), force).await?;
        return Ok(());
    }

    // Held for the rest of `main`'s lifetime: `tracing_appender::non_blocking`'s worker
    // thread flushes buffered log lines only while this guard is alive, and drops them
    // on an early return otherwise.
    let _log_guard = init_tracing(config.logs_path().as_deref())?;

    let repo = SqliteRepository::connect(&config.db)
        .await?
        .with_modules_dir(config.modules_path());
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
        uploads: Arc::new(shadowcat::http::assets::uploads::UploadSessions::new()),
        auth_throttle: Arc::new(shadowcat::http::throttle::AuthThrottle::new()),
        write_barrier: Arc::new(tokio::sync::RwLock::new(())),
        preview_fetch_locks: Arc::new(dashmap::DashMap::new()),
    };

    shadowcat::auth::session::spawn_session_sweep(&state.repo);
    shadowcat::http::assets::uploads::spawn_sweeper(
        state.uploads.clone(),
        state.upload_rate.clone(),
    );

    let ws_state = state.ws.clone();
    let listener = tokio::net::TcpListener::bind(&config.bind).await?;
    tracing::info!(bind = %config.bind, "shadowcat listening");
    // `http::serve` wraps `axum::serve(..).with_graceful_shutdown(..)` (which is what gives the
    // connect-info service so `throttle::ClientIp` resolves a real address — production only;
    // axum-test's mock transport has none, degrading the IP throttle to identity-only there
    // without a 500) with a post-shutdown drain of any still-live WebSocket connection task —
    // see `http::serve`'s own doc for why that drain step is necessary at all.
    http::serve(listener, state, shutdown_signal(ws_state)).await?;
    Ok(())
}

/// Resolves once Ctrl+C or (on Unix) SIGTERM arrives, having already told every live
/// WebSocket connection's egress loop to close (`WsState::trigger_shutdown`) — this is the
/// future `http::serve` passes to `axum::serve(..).with_graceful_shutdown`. That alone does
/// NOT wait for a WebSocket connection's task to actually finish exiting (see
/// `ws::conn::egress_loop`'s `shutdown` select arm for where the signal is received, and
/// `http::serve`'s own doc for the separate drain step that waits for the task to exit).
///
/// # Examples
///
/// ```text
/// // Passed directly to `axum::serve(..).with_graceful_shutdown(shutdown_signal(ws_state))`;
/// // not callable in a doctest (it blocks on a real OS signal).
/// ```
async fn shutdown_signal(ws: shadowcat::ws::WsState) {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    // Windows has no SIGTERM; `pending()` never resolves there, so the `select!` below
    // reduces to Ctrl+C alone on that platform without a second `#[cfg]` branch at the call
    // site. macOS and Linux are both `unix` and take the branch above.
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    tracing::info!("shutdown signal received; closing live connections");
    ws.trigger_shutdown();
}

/// One-shot `--backup-to` mode: snapshot the resolved db + assets into
/// `out_dir`, print a one-line summary, and return — the caller exits before
/// any server startup runs. Refuses a non-empty `out_dir` unless `force`.
/// Rejects an in-memory `config.db` up front: `shadowcat::backup::create_backup`
/// needs a real file to `VACUUM INTO` from, and resolving that file through
/// `shadowcat::db::database_file` (rather than treating `config.db` as a bare
/// path) is what makes a `sqlite:`-URL-form db back up correctly instead of
/// silently targeting a nonexistent literal path.
async fn run_backup(config: &Config, out_dir: &std::path::Path, force: bool) -> anyhow::Result<()> {
    let db_path = shadowcat::db::database_file(&config.db)?.ok_or_else(|| {
        anyhow::anyhow!(
            "backup requires a file-backed database; {} is in-memory",
            config.db
        )
    })?;
    if !force && !shadowcat::backup::dir_is_empty_or_absent(out_dir)? {
        anyhow::bail!(
            "refusing to write into non-empty directory {} without --force",
            out_dir.display()
        );
    }
    let manifest =
        shadowcat::backup::create_backup(&db_path, &config.assets_path(), out_dir).await?;
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
/// starts the server. Rejects an in-memory `config.db` up front, resolving
/// the destination file through `shadowcat::db::database_file` rather than
/// treating `config.db` as a bare path — see `run_backup`'s doc for why.
async fn run_restore(
    config: &Config,
    backup_dir: &std::path::Path,
    force: bool,
) -> anyhow::Result<()> {
    let db_path = shadowcat::db::database_file(&config.db)?.ok_or_else(|| {
        anyhow::anyhow!(
            "restore requires a file-backed database; {} is in-memory",
            config.db
        )
    })?;
    shadowcat::backup::restore_backup(backup_dir, &db_path, &config.assets_path(), force).await?;
    println!(
        "restored {} into db={} assets={}",
        backup_dir.display(),
        db_path.display(),
        config.assets_path().display(),
    );
    Ok(())
}

/// Retention bound on daily-rotated log files (`init_tracing`'s file layer): the oldest file
/// beyond this count is deleted on rotation. Two weeks of daily files is enough to diagnose an
/// issue reported days after it occurred without letting the log directory grow unbounded on a
/// long-lived host.
const MAX_LOG_FILES: usize = 14;

/// Builds the daily-rotated rolling file appender `init_tracing`'s file layer writes through,
/// creating `dir` first. `Inner::new` (the vendored `tracing-appender` rolling-file internals,
/// reached through `Builder::build` below) prunes old log files -- a `read_dir` over `dir` --
/// BEFORE it creates `dir` itself, so a fresh install's first-ever start would otherwise hit a
/// missing directory and the crate `eprintln!`s a read error, unconditionally, with no way to
/// silence it from this side of the API. Creating `dir` here first, before `Builder::build` ever
/// runs, is what keeps that first prune from having anything to fail on.
///
/// # Examples
///
/// ```text
/// // Not runnable as a doctest: rustdoc runs no doctests for bin targets, and this item is
/// // private to the `shadowcat` binary crate besides. Exercised for real in `tests::
/// // build_rolling_appender_creates_a_fresh_log_directory_before_writing`.
/// let appender = build_rolling_appender(std::path::Path::new("logs")).unwrap();
/// ```
fn build_rolling_appender(
    dir: &std::path::Path,
) -> anyhow::Result<tracing_appender::rolling::RollingFileAppender> {
    std::fs::create_dir_all(dir)?;
    Ok(tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("shadowcat")
        .filename_suffix("log")
        .max_log_files(MAX_LOG_FILES)
        .build(dir)?)
}

/// Structured logging filtered by SHADOWCAT_LOG (falling back to RUST_LOG, then "info").
/// Always writes to stdout; ADDITIONALLY writes daily-rotated files under `log_dir` when
/// `Some` — `None` means stdout-only, whether because the one early one-shot invocation that
/// runs before `Config` resolves (the `audio-monitor` subcommand — see this function's callers
/// in `main`) has no `Config` to derive a directory from at all, or because every OTHER caller
/// passes `Config::logs_path()` through unchanged, which is itself `None` for an in-memory/
/// non-filesystem `db` with no explicit `log_dir` override (see `logs_path`'s own doc for the
/// exact rule). The file layer is
/// non-blocking (`tracing_appender::
/// non_blocking`): its `WorkerGuard` must be held by the caller for as long as file logging
/// should keep flushing, so this returns it rather than dropping it internally — a dropped
/// guard stops the background writer thread.
///
/// # Examples
///
/// ```text
/// // Not callable in a doctest: `fmt().init()`/`registry().init()` install a
/// // process-global subscriber, which can only happen once per test binary.
/// let _guard = init_tracing(Some(std::path::Path::new("logs")));
/// ```
fn init_tracing(
    log_dir: Option<&std::path::Path>,
) -> anyhow::Result<Option<tracing_appender::non_blocking::WorkerGuard>> {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = std::env::var("SHADOWCAT_LOG")
        .ok()
        .and_then(|s| EnvFilter::try_new(s).ok())
        .or_else(|| EnvFilter::try_from_default_env().ok())
        .unwrap_or_else(|| EnvFilter::new("info"));
    let stdout_layer = fmt::layer();
    match log_dir {
        Some(dir) => {
            let appender = build_rolling_appender(dir)?;
            let (non_blocking, guard) = tracing_appender::non_blocking(appender);
            // ANSI color codes are meaningless (and noisy) in a plain log file.
            let file_layer = fmt::layer().with_writer(non_blocking).with_ansi(false);
            tracing_subscriber::registry()
                .with(filter)
                .with(stdout_layer)
                .with(file_layer)
                .init();
            Ok(Some(guard))
        }
        None => {
            tracing_subscriber::registry()
                .with(filter)
                .with(stdout_layer)
                .init();
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests;
