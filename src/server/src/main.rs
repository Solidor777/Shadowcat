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
    if cli.tls_regenerate
        && (cli.command.is_some() || cli.backup_to.is_some() || cli.restore_from.is_some())
    {
        anyhow::bail!(
            "--tls-regenerate is mutually exclusive with --backup-to, --restore-from and subcommands"
        );
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
    let tls_regenerate = cli.tls_regenerate;

    let config = Config::load(cli)?;

    if tls_regenerate {
        let _log_guard = init_tracing(config.logs_path().as_deref())?;
        config.validate_tls()?;
        run_tls_regenerate(&config)?;
        return Ok(());
    }

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

    // Serving is the path that uses the TLS settings; `Config::load` leaves them unchecked so the
    // backup/restore modes above are not blocked by them.
    config.validate_tls()?;

    apply_staged_restore(&config).await?;

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

    for warning in config.startup_warnings() {
        tracing::warn!("{warning}");
    }
    shadowcat::tls::install_default_provider();
    let tls = shadowcat::tls::TlsRuntime::start(&config);
    shadowcat::tls::TlsRuntime::spawn_maintenance(&tls);
    if config.hsts_max_age_secs.is_some()
        && config.trusted_proxies.is_empty()
        && tls.status().source == Some(shadowcat::tls::TlsSource::SelfSigned)
    {
        tracing::warn!(
            "hsts_max_age_secs is set but the active certificate is self-signed and no trusted proxy is configured: HSTS is only ever sent on CA-trusted native TLS or trusted-proxy HTTPS requests, so it will not be sent"
        );
    }

    let trash = Arc::new(shadowcat::trash::Trash::from_config(&config));
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
        file_permits: Arc::new(crate::http::assets::permits::StreamedFilePermits::production()),
        tls: tls.clone(),
        portmap: shadowcat::portmap::PortMapRuntime::new(Arc::new(config.clone())),
        trash: trash.clone(),
    };
    let portmap = state.portmap.clone();
    let stored_port_forward = shadowcat::portmap::runtime::read_stored_toggle(&state.repo).await;

    shadowcat::auth::session::spawn_housekeeping_sweep(&state.repo);
    shadowcat::http::assets::uploads::spawn_sweeper(
        state.uploads.clone(),
        state.upload_rate.clone(),
        state.trash.clone(),
    );
    // The first pass runs at once, so entries that expired while the server was down are removed
    // at startup; a `disabled` trash has nothing to sweep and starts no task.
    shadowcat::trash::spawn_retention_sweep(trash);

    // Both connection caps are fitted to one descriptor budget, derived from the soft limit as it
    // is AFTER the raise attempt, which asks for exactly `descriptors_needed` (`fd_budget`).
    // The raise-then-fit is `fd_budget::raise_and_fit`, the same call the `test_server` binary makes.
    let (sniff_limits, conn_limits) = shadowcat::fd_budget::raise_and_fit(
        shadowcat::tls::listener::Limits::default(),
        shadowcat::http::conn_serve::ConnLimits::for_config(&config),
    );

    let tcp = tokio::net::TcpListener::bind(&config.bind).await?;
    let bound = tcp.local_addr()?;
    // One port serves plain HTTP and TLS; the first byte of each connection decides
    // (`tls::listener::SniffListener`).
    let listener = shadowcat::tls::listener::SniffListener::new(
        tcp,
        tls.clone(),
        Arc::new(config.clone()),
        sniff_limits,
    );
    tracing::info!(bind = %config.bind, "shadowcat listening");
    // Process tests read the bound port from these `reachable at http://<addr>:<port>` lines, so
    // that form is a stable contract.
    for line in tls.startup_lines(bound) {
        tracing::info!("reachable at {line}");
    }
    // Router port forwarding starts only now that the listener is bound, so accepting connections
    // never waits on it, and no router answer (or silence) can fail startup: the task reports
    // failures as status (`portmap::runtime`).
    portmap.activate(bound, stored_port_forward).await;
    // Upgrades stored Opus derivatives written by an older encoder or muxer, one asset at a time
    // in the background. Started after the listener is bound so it never delays accepting; it
    // writes no database row and bumps no version (`data::asset::audio_upgrade`).
    shadowcat::data::asset::audio_upgrade::spawn_audio_upgrade(
        state.repo.clone(),
        config.assets_path(),
        state.write_barrier.clone(),
        state.trash.clone(),
    );
    // `http::serve_with` runs `http::conn_serve::serve_router` (per-connection `ConnMeta` connect-info,
    // so `throttle::ClientIp` and the scheme decider see a real peer; header-read, HTTP/2 idle,
    // keep-alive and live-connection bounds), tells live WebSocket sessions to close when the
    // drain begins, and then waits for their tasks; see `http::serve_with`.
    let served = http::serve_with(listener, state, shutdown_signal(), conn_limits).await;
    // After the HTTP drain, whether it ended cleanly or not: deletes the router mapping within its
    // deadline.
    portmap.shutdown().await;
    served?;
    Ok(())
}

/// Resolves once Ctrl+C or (on Unix) SIGTERM arrives. This is the future `http::serve_with` passes to
/// `http::conn_serve::serve_router` as its shutdown trigger; telling WebSocket sessions to close
/// and waiting for them is `http::serve_with`'s drain step, shared with the source-ended path.
///
/// # Examples
///
/// ```text
/// // Passed directly to `http::serve_with(listener, state, shutdown_signal(), limits)`;
/// // not callable in a doctest (it blocks on a real OS signal).
/// ```
async fn shutdown_signal() {
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
    let trash = shadowcat::trash::Trash::from_config(config);
    let manifest = shadowcat::backup::create_backup(
        &db_path,
        &config.assets_path(),
        out_dir,
        &trash,
        shadowcat::backup::BackupKind::Manual,
        None,
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

/// Applies a restore the admin route staged before shutting the server down
/// (`shadowcat::backup::apply_pending_restore`), then lets startup continue. Runs before the
/// database pool opens, because the restore swaps the database file. A failed restore is logged
/// and recorded in `restore-failed.json`; the server starts on the data it had. An in-memory
/// database has no marker and is skipped.
///
/// # Examples
///
/// ```text
/// // Called once from `main`, ahead of `SqliteRepository::connect`; the staged-restore behavior
/// // is exercised by `shadowcat::backup::apply_pending_restore`'s tests.
/// apply_staged_restore(&config).await?;
/// ```
async fn apply_staged_restore(config: &Config) -> anyhow::Result<()> {
    let Some(db_path) = shadowcat::db::database_file(&config.db)? else {
        return Ok(());
    };
    let trash = shadowcat::trash::Trash::from_config(config);
    match shadowcat::backup::apply_pending_restore(
        &db_path,
        &config.assets_path(),
        &config.backups_path_for(&db_path),
        &trash,
    )
    .await?
    {
        shadowcat::backup::PendingRestore::Absent => {}
        shadowcat::backup::PendingRestore::Applied(id) => {
            tracing::info!(backup = %id, "staged restore applied");
        }
        shadowcat::backup::PendingRestore::Failed(error) => {
            tracing::error!(%error, "staged restore failed; starting on the existing data");
        }
    }
    Ok(())
}

/// One-shot `--tls-regenerate` mode: replaces the persisted self-signed certificate with a fresh
/// one advertising the current local addresses and configured `tls_hostnames`, prints its
/// SHA-256 fingerprint, and returns — the caller exits before any server startup runs. Requires
/// an on-disk TLS directory (`Config::tls_path`); an in-memory database with no `tls_dir` has
/// nothing to regenerate because its certificate lives only in process memory.
fn run_tls_regenerate(config: &Config) -> anyhow::Result<()> {
    let dir = config.tls_path().ok_or_else(|| {
        anyhow::anyhow!(
            "--tls-regenerate requires a persistent TLS directory: the database is in-memory and tls_dir is unset"
        )
    })?;
    let cert = shadowcat::tls::selfsigned::force_regenerate(
        &dir,
        &config.tls_hostnames,
        &shadowcat::tls::selfsigned::SanSources::detect(config.tls_advertise_hostname),
        &shadowcat::tls::selfsigned::RcgenGenerator,
        time::OffsetDateTime::now_utc(),
        &shadowcat::trash::Trash::from_config(config),
    )?;
    println!(
        "regenerated {}: sha256 {}",
        shadowcat::tls::selfsigned::cert_path(&dir).display(),
        cert.fingerprint()
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
    let trash = shadowcat::trash::Trash::from_config(config);
    shadowcat::backup::restore_backup(backup_dir, &db_path, &config.assets_path(), force, &trash)
        .await?;
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
