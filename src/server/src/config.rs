// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::net::{SocketAddr, ToSocketAddrs};

use clap::Parser;
use figment::providers::{Env, Format, Serialized, Toml};
use figment::Figment;
use serde::{Deserialize, Serialize};

/// CLI flags. Every field is optional so it only overrides lower layers when
/// explicitly provided.
///
/// # Examples
///
/// ```
/// use shadowcat::config::{Cli, Config};
///
/// let cli = Cli { bind: Some("0.0.0.0:9000".into()), ..Default::default() };
/// let cfg = Config::load(cli).expect("config layering");
/// assert_eq!(cfg.bind, "0.0.0.0:9000");
/// ```
#[derive(Parser, Debug, Default)]
#[command(name = "shadowcat")]
pub struct Cli {
    /// Listen address override (`host:port`); wins over every other layer.
    #[arg(long)]
    pub bind: Option<String>,
    /// SQLite database file path override.
    #[arg(long)]
    pub db: Option<String>,
    /// TOML config file path (default `shadowcat.toml`; a missing file is ignored).
    #[arg(long)]
    pub config: Option<String>,
    /// Headless first-admin bootstrap: username override (pairs with `--admin-password`).
    #[arg(long)]
    pub admin_user: Option<String>,
    /// Headless first-admin bootstrap: password override (pairs with `--admin-user`).
    #[arg(long)]
    pub admin_password: Option<String>,
    /// Setup-window policy override: `auto` | `off` | `required` | an explicit token.
    #[arg(long)]
    pub setup_token: Option<String>,
    /// Session-cookie signing-key override (base64, >= 64 bytes decoded).
    #[arg(long)]
    pub session_key: Option<String>,
    /// Asset storage root override (default: sibling `assets/` beside the db file).
    #[arg(long)]
    pub assets_dir: Option<String>,
    /// Installed-module discovery root override (default: sibling `modules/`).
    #[arg(long)]
    pub modules_dir: Option<String>,
    /// Rolling log-file output root override (default: sibling `logs/`).
    #[arg(long)]
    pub log_dir: Option<String>,
    /// In-server backup output root override (default: sibling `backups/`).
    #[arg(long)]
    pub backups_dir: Option<String>,
    /// One-shot: snapshot the resolved db + assets into this directory, print
    /// a summary, and exit before the server would otherwise start.
    #[arg(long)]
    pub backup_to: Option<String>,
    /// One-shot: restore a prior `--backup-to` directory over the resolved db
    /// + assets, print a summary, and exit. Never starts the server.
    #[arg(long)]
    pub restore_from: Option<String>,
    /// Required to let `--backup-to` overwrite a non-empty output directory,
    /// or `--restore-from` overwrite an existing destination db/assets dir.
    #[arg(long)]
    pub force: bool,
    /// Keep the uploaded original beside the converted canonical file
    /// (`Config.retain_originals`); `--retain-originals false` discards it.
    #[arg(long)]
    pub retain_originals: Option<bool>,
    /// Declares that a TLS-terminating reverse proxy fronts this bind on the same host
    /// (`Config.behind_tls_proxy`). A loopback bind fronted this way is internet-reachable
    /// even though the bind address itself is not — set this so the setup-token policy,
    /// session-cookie `Secure` flag, and HSTS response header all treat the deployment as
    /// exposed. Never inferred automatically.
    #[arg(long)]
    pub behind_tls_proxy: Option<bool>,
    /// Subcommand; `None` (the default — no subcommand given) is ordinary serve mode. Every
    /// root flag above parses unconditionally regardless of which variant, if any, this holds.
    #[command(subcommand)]
    pub command: Option<CliCommand>,
}

/// Top-level subcommands of the `shadowcat` binary.
///
/// # Examples
///
/// ```
/// use clap::Parser;
/// use shadowcat::config::{Cli, CliCommand};
///
/// let cli = Cli::parse_from(["shadowcat", "audio-monitor", "--port", "0"]);
/// assert!(matches!(cli.command, Some(CliCommand::AudioMonitor(_))));
/// ```
#[derive(clap::Subcommand, Debug)]
pub enum CliCommand {
    /// Serve the localhost audio-session-level WebSocket the ducking module's
    /// `OsMonitorSource` connects to (`crate::audio_monitor::server::run`). Never starts the
    /// main world server; mutually exclusive with `--backup-to`/`--restore-from`.
    AudioMonitor(AudioMonitorArgs),
}

/// Arguments for `shadowcat audio-monitor`.
///
/// # Examples
///
/// ```
/// use clap::Parser;
/// use shadowcat::config::{Cli, CliCommand};
///
/// let cli = Cli::parse_from(["shadowcat", "audio-monitor", "--watch", "discord", "--watch", "teams"]);
/// let Some(CliCommand::AudioMonitor(args)) = cli.command else {
///     unreachable!()
/// };
/// assert_eq!(args.port, 31998); // the built-in default
/// assert_eq!(args.watch, vec!["discord".to_string(), "teams".to_string()]);
/// ```
#[derive(clap::Args, Debug)]
pub struct AudioMonitorArgs {
    /// Localhost port to bind. `0` binds an ephemeral port (printed on start).
    #[arg(long, default_value_t = 31998)]
    pub port: u16,
    /// Additional allowed `Origin` header values, beside the built-in
    /// `http://localhost:30000`/`http://127.0.0.1:30000` defaults.
    #[arg(long)]
    pub allow_origin: Vec<String>,
    /// Initial watched-process substrings (case-insensitive); default `discord` when empty.
    /// The client's live `watch` frame replaces this list at runtime without a restart.
    #[arg(long)]
    pub watch: Vec<String>,
}

/// Effective server configuration after layering. Precedence (high→low):
/// CLI flag > SHADOWCAT_* env > TOML file > built-in default.
///
/// # Examples
///
/// ```
/// use shadowcat::config::Config;
///
/// let cfg = Config { bind: "0.0.0.0:9000".into(), ..Default::default() };
/// assert_eq!(cfg.bind, "0.0.0.0:9000");
/// assert_eq!(cfg.db, "./shadowcat.db"); // untouched fields keep the default
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Listen address (`host:port`). Default `127.0.0.1:30000`; non-loopback
    /// binds flip the `auto` setup policy to token-required.
    pub bind: String,
    /// SQLite database file path. Default `./shadowcat.db`; sibling dirs
    /// (`assets/`, `modules/`, `backups/`) resolve relative to it.
    pub db: String,
    /// Headless first-admin bootstrap username; `None` = interactive setup form.
    pub admin_user: Option<String>,
    /// Headless first-admin bootstrap password; `None` = interactive setup form.
    pub admin_password: Option<String>,
    /// `"auto"` | `"off"` | `"required"` | `<explicit token>`.
    pub setup_token: String,
    /// Session-cookie signing key (base64). `None` = generated once and
    /// persisted to the DB `settings` table, then loaded on every later boot —
    /// sessions already survive an ordinary restart unpinned. Set it explicitly
    /// for replicas sharing one key or controlled rotation
    /// (`auth::session::load_or_create_key`).
    pub session_key: Option<String>,
    /// Asset storage root. `None` → sibling `assets/` beside the db file.
    pub assets_dir: Option<String>,
    /// Installed-module discovery root. `None` → sibling `modules/` dir beside the db file.
    pub modules_dir: Option<String>,
    /// Rolling log-file output root (`main::init_tracing`). `None` → sibling `logs/`
    /// dir beside the db file, mirroring `assets_dir`/`modules_dir`'s own convention.
    pub log_dir: Option<String>,
    /// In-server backup output root (`POST /api/admin/backup`). `None` → sibling
    /// `backups/` dir beside the db file.
    pub backups_dir: Option<String>,
    /// Regular-uploader size cap (bytes). Default 25 MiB.
    pub upload_max_bytes: u64,
    /// Regular-uploader uploads per minute. Default 20.
    pub upload_rate_per_min: u32,
    /// GM/owner size cap; `None` → 2× `upload_max_bytes`.
    pub upload_max_bytes_gm: Option<u64>,
    /// GM/owner uploads per minute; `None` → 2× `upload_rate_per_min`.
    pub upload_rate_per_min_gm: Option<u32>,
    /// Whether a converted upload keeps its original bytes on disk as
    /// `<uuid>.orig` (reconvert + "download original" need it). Default
    /// `true`; `false` trades those for disk. Host-level: the host pays for
    /// the disk, so this is not a per-world setting.
    pub retain_originals: bool,
    /// Per-identity `/api/login` budget (trailing 60s); `None` →
    /// `throttle::LOGIN_PER_MIN_PER_IDENTITY`. Self-hosting operators behind a
    /// NAT/shared-proxy IP, or an automated test harness that logs in as one
    /// identity repeatedly, may need to raise this — never lower it below a
    /// value that still bounds credential stuffing.
    pub login_per_min_per_identity: Option<usize>,
    /// Per-IP `/api/login` budget (trailing 60s); `None` →
    /// `throttle::LOGIN_PER_MIN_PER_IP`.
    pub login_per_min_per_ip: Option<usize>,
    /// Per-account `/api/invites/accept` budget (trailing 60s); `None` →
    /// `throttle::INVITE_PER_MIN_PER_ACCOUNT`.
    pub invite_per_min_per_account: Option<usize>,
    /// Per-IP `/api/invites/accept` budget (trailing 60s); `None` →
    /// `throttle::INVITE_PER_MIN_PER_IP`.
    pub invite_per_min_per_ip: Option<usize>,
    /// Per-account `POST /api/worlds` budget (trailing 60s); `None` →
    /// `throttle::WORLD_CREATE_PER_MIN_PER_ACCOUNT`. Same rationale as
    /// `login_per_min_per_identity`: an automated test harness that creates many
    /// worlds under one seeded identity across a long-running session needs to raise
    /// this — never lower it below a value that still bounds a scripted flood.
    pub world_create_per_min_per_account: Option<usize>,
    /// Exact peer IP addresses trusted to set `X-Forwarded-For` (reverse-proxy deployments only).
    /// Default empty — `ClientIp` never consults the header and resolves solely from the TCP peer
    /// address. Matched by EXACT address only, not CIDR range
    /// (self-hosting operators list every trusted proxy's address explicitly; a typical single
    /// reverse-proxy-on-the-same-host deployment needs one entry, e.g. `"127.0.0.1"`). A request
    /// whose immediate TCP peer is NOT in this list gets its `X-Forwarded-For` header ignored
    /// entirely, regardless of content — this is what prevents an arbitrary internet client from
    /// spoofing its own address by sending the header directly.
    pub trusted_proxies: Vec<String>,
    /// Declares a TLS-terminating reverse proxy fronts `bind` on the same host. Default
    /// `false`. Two present-tense readers, at two different decision granularities:
    /// `is_exposed()` folds this together with `is_loopback_bind()` into the single derived
    /// predicate `setup_token_policy`'s `"auto"` arm, `AppState::resolve_setup_token`'s
    /// warning, and the session cookie's `Secure` flag (`auth::session::session_layer`) all
    /// read; the HSTS response header (`http::router`) reads this field DIRECTLY instead,
    /// never through `is_exposed()` — see `is_exposed()`'s own doc for why HSTS's decision is
    /// narrower.
    pub behind_tls_proxy: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:30000".into(),
            db: "./shadowcat.db".into(),
            admin_user: None,
            admin_password: None,
            setup_token: "auto".into(),
            session_key: None,
            assets_dir: None,
            modules_dir: None,
            log_dir: None,
            backups_dir: None,
            upload_max_bytes: 25 * 1024 * 1024,
            upload_rate_per_min: 20,
            upload_max_bytes_gm: None,
            upload_rate_per_min_gm: None,
            retain_originals: true,
            login_per_min_per_identity: None,
            login_per_min_per_ip: None,
            invite_per_min_per_account: None,
            invite_per_min_per_ip: None,
            world_create_per_min_per_account: None,
            trusted_proxies: Vec::new(),
            behind_tls_proxy: false,
        }
    }
}

/// Resolved setup-window policy. `Required(None)` means a token is required but
/// none was supplied — the server generates one at boot.
///
/// # Examples
///
/// ```
/// use shadowcat::config::SetupTokenPolicy;
///
/// let policy = SetupTokenPolicy::Required(Some("secret".to_string()));
/// assert!(matches!(policy, SetupTokenPolicy::Required(Some(_))));
/// assert!(matches!(SetupTokenPolicy::Open, SetupTokenPolicy::Open));
/// ```
#[derive(Debug, Clone)]
pub enum SetupTokenPolicy {
    /// `/api/setup` accepts the first admin without a token.
    Open,
    /// A token gates `/api/setup`; `None` = generate-and-log at boot.
    Required(Option<String>),
}

impl Config {
    /// Layer file + env over defaults via figment, then apply CLI overrides in
    /// code so CLI strictly wins (figment cannot easily skip `None` CLI fields).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use shadowcat::config::{Cli, Config};
    ///
    /// // Reads shadowcat.toml (if present) + SHADOWCAT_* env, then applies CLI wins.
    /// let cfg = Config::load(Cli::default()).expect("config layering");
    /// assert!(!cfg.bind.is_empty());
    /// ```
    // Boxed: figment::Error is third-party and large by value, so returning it inline
    // would widen every Result in the boot path to that error's size.
    pub fn load(cli: Cli) -> Result<Self, Box<figment::Error>> {
        let config_path = cli
            .config
            .clone()
            .unwrap_or_else(|| "shadowcat.toml".into());
        let mut cfg: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::file(&config_path)) // missing file is ignored
            .merge(Env::prefixed("SHADOWCAT_"))
            .extract()?;

        if let Some(v) = cli.bind {
            cfg.bind = v;
        }
        if let Some(v) = cli.db {
            cfg.db = v;
        }
        if let Some(v) = cli.admin_user {
            cfg.admin_user = Some(v);
        }
        if let Some(v) = cli.admin_password {
            cfg.admin_password = Some(v);
        }
        if let Some(v) = cli.setup_token {
            cfg.setup_token = v;
        }
        if let Some(v) = cli.session_key {
            cfg.session_key = Some(v);
        }
        if let Some(v) = cli.retain_originals {
            cfg.retain_originals = v;
        }
        if let Some(v) = cli.assets_dir {
            cfg.assets_dir = Some(v);
        }
        if let Some(v) = cli.modules_dir {
            cfg.modules_dir = Some(v);
        }
        if let Some(v) = cli.log_dir {
            cfg.log_dir = Some(v);
        }
        if let Some(v) = cli.backups_dir {
            cfg.backups_dir = Some(v);
        }
        if let Some(v) = cli.behind_tls_proxy {
            cfg.behind_tls_proxy = v;
        }
        Ok(cfg)
    }

    /// Resolves a directory named `name` as a sibling of the on-disk database
    /// file `crate::db::database_file` (the single decider for "which file on
    /// disk is `self.db`" -- see its own doc) resolves `self.db` to, or `None`
    /// when `self.db` names an in-memory database or fails to parse. An
    /// unparseable `self.db` yields the same `None` fallback as an in-memory
    /// one here, because `SqliteRepository::connect` already rejects such a
    /// string at server startup, before any caller of this resolver ever
    /// runs. Shared by every sibling-directory resolver below so the "what
    /// counts as a filesystem-backed db" decision is made in exactly one
    /// place rather than re-derived per resolver.
    ///
    /// # Examples
    ///
    /// ```text
    /// db_sibling_dir("backups") == None  // in-memory or malformed self.db
    /// ```
    fn db_sibling_dir(&self, name: &str) -> Option<std::path::PathBuf> {
        let file = crate::db::database_file(&self.db).ok().flatten()?;
        Some(Self::sibling_of(&file, name))
    }

    /// The directory named `name` beside the database file `db_file`. A bare
    /// file name (no parent component) resolves against the current
    /// directory: `Path::parent` returns `Some("")` for it, which is filtered
    /// to `.` so the result is never an empty-rooted path. The one place the
    /// sibling layout is spelled — `db_sibling_dir` and `backups_path_for`
    /// both derive through it.
    ///
    /// # Examples
    ///
    /// ```text
    /// sibling_of("/data/shadowcat.db", "logs") == "/data/logs"
    /// sibling_of("shadowcat.db", "logs")       == "./logs"
    /// ```
    fn sibling_of(db_file: &std::path::Path, name: &str) -> std::path::PathBuf {
        db_file
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(name)
    }

    /// Resolve the asset storage root: explicit `assets_dir`, else a sibling
    /// `assets/` directory beside the db file (built via std::path, #2), via
    /// `db_sibling_dir`. Falls back to the current directory when the db has
    /// no on-disk location (in-memory, or unparseable) and no `assets_dir`
    /// override was given -- asset storage is required infrastructure
    /// regardless of where (or whether) the database lives on disk.
    ///
    /// # Examples
    ///
    /// ```
    /// let cfg = shadowcat::config::Config {
    ///     db: "data/shadowcat.db".into(),
    ///     ..Default::default()
    /// };
    /// assert_eq!(cfg.assets_path(), std::path::Path::new("data").join("assets"));
    /// ```
    pub fn assets_path(&self) -> std::path::PathBuf {
        if let Some(dir) = &self.assets_dir {
            return std::path::PathBuf::from(dir);
        }
        self.db_sibling_dir("assets")
            .unwrap_or_else(|| std::path::PathBuf::from(".").join("assets"))
    }

    /// Resolve the installed-module discovery root: explicit `modules_dir`, else a
    /// sibling `modules/` directory beside the db file (built via std::path, #2),
    /// via `db_sibling_dir`, falling back to the current directory on the same terms
    /// as `assets_path`. Unlike `assets_path`, nothing writes here server-side
    /// (install is a manual filesystem extract — no server-side installer exists) —
    /// the directory need not exist; a missing dir scans as "no modules installed"
    /// (see `modules::scan_installed_modules`).
    ///
    /// # Examples
    ///
    /// ```
    /// let cfg = shadowcat::config::Config {
    ///     modules_dir: Some("staged-modules".into()),
    ///     ..Default::default()
    /// };
    /// assert_eq!(cfg.modules_path(), std::path::PathBuf::from("staged-modules"));
    /// ```
    pub fn modules_path(&self) -> std::path::PathBuf {
        if let Some(dir) = &self.modules_dir {
            return std::path::PathBuf::from(dir);
        }
        self.db_sibling_dir("modules")
            .unwrap_or_else(|| std::path::PathBuf::from(".").join("modules"))
    }

    /// Resolve the rolling log-file output root: explicit `log_dir` always wins (even over an
    /// in-memory `db`, since an operator who names a directory explicitly clearly wants file
    /// logging); otherwise `None` when `db_sibling_dir` cannot place a directory beside the db
    /// file — the db is in-memory (either `:memory:` spelling, or a `mode=memory` query
    /// parameter — see `crate::db::database_file`) or its connection string fails to parse;
    /// otherwise a sibling `logs/` directory beside the db file (built via std::path, #2), the
    /// same sibling-of-the-db-file convention `assets_path`/`modules_path` use. `None` means file
    /// logging is off entirely (`init_tracing` falls back to stdout-only) — never a directory
    /// this function invents on the caller's behalf for a database it cannot locate on disk.
    ///
    /// # Examples
    ///
    /// ```
    /// let cfg = shadowcat::config::Config {
    ///     db: "data/shadowcat.db".into(),
    ///     ..Default::default()
    /// };
    /// assert_eq!(cfg.logs_path(), Some(std::path::Path::new("data").join("logs")));
    ///
    /// let memory_cfg = shadowcat::config::Config {
    ///     db: "sqlite::memory:".into(),
    ///     ..Default::default()
    /// };
    /// assert_eq!(memory_cfg.logs_path(), None);
    /// ```
    pub fn logs_path(&self) -> Option<std::path::PathBuf> {
        if let Some(dir) = &self.log_dir {
            return Some(std::path::PathBuf::from(dir));
        }
        self.db_sibling_dir("logs")
    }

    /// Resolve the backup output root for the on-disk database file
    /// `db_file`: explicit `backups_dir`, else a sibling `backups/` directory
    /// beside `db_file` (built via std::path, #2). Takes the already-resolved
    /// file rather than re-deriving it from `self.db`, so the caller decides
    /// once — through `crate::db::database_file` — whether a backup is
    /// possible at all (an in-memory `db` has no file to back up) and this
    /// resolver only places the output.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut cfg = shadowcat::config::Config::default();
    /// let db_file = std::path::Path::new("data").join("shadowcat.db");
    /// assert_eq!(cfg.backups_path_for(&db_file), std::path::Path::new("data").join("backups"));
    /// cfg.backups_dir = Some("elsewhere".into());
    /// assert_eq!(cfg.backups_path_for(&db_file), std::path::PathBuf::from("elsewhere"));
    /// ```
    pub fn backups_path_for(&self, db_file: &std::path::Path) -> std::path::PathBuf {
        match &self.backups_dir {
            Some(dir) => std::path::PathBuf::from(dir),
            None => Self::sibling_of(db_file, "backups"),
        }
    }

    /// Role-tiered upload size cap (GM defaults to 2× the regular value).
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::document::WorldRole;
    ///
    /// let cfg = shadowcat::config::Config::default();
    /// assert_eq!(cfg.effective_max_bytes(WorldRole::Player), 25 * 1024 * 1024);
    /// assert_eq!(cfg.effective_max_bytes(WorldRole::Gm), 2 * cfg.effective_max_bytes(WorldRole::Player));
    /// ```
    pub fn effective_max_bytes(&self, role: crate::data::document::WorldRole) -> u64 {
        match role {
            crate::data::document::WorldRole::Gm => self
                .upload_max_bytes_gm
                .unwrap_or(self.upload_max_bytes.saturating_mul(2)),
            _ => self.upload_max_bytes,
        }
    }

    /// Role-tiered uploads-per-minute (GM defaults to 2× the regular value).
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::document::WorldRole;
    ///
    /// let cfg = shadowcat::config::Config::default(); // 20/min player rate
    /// assert_eq!(cfg.effective_rate_per_min(WorldRole::Gm), 40);
    /// ```
    pub fn effective_rate_per_min(&self, role: crate::data::document::WorldRole) -> u32 {
        match role {
            crate::data::document::WorldRole::Gm => self
                .upload_rate_per_min_gm
                .unwrap_or(self.upload_rate_per_min.saturating_mul(2)),
            _ => self.upload_rate_per_min,
        }
    }

    /// True when the bind host resolves to a loopback address. `0.0.0.0` /
    /// non-loopback hosts are treated as exposed. On parse failure, default to
    /// the safe answer (not loopback) so the token is required.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut cfg = shadowcat::config::Config::default(); // 127.0.0.1:30000
    /// assert!(cfg.is_loopback_bind());
    /// cfg.bind = "0.0.0.0:30000".into();
    /// assert!(!cfg.is_loopback_bind());
    /// ```
    pub fn is_loopback_bind(&self) -> bool {
        self.bind
            .to_socket_addrs()
            .ok()
            .and_then(|mut a| a.next())
            .map(|addr: SocketAddr| addr.ip().is_loopback())
            .unwrap_or(false)
    }

    /// True when this deployment is reachable from beyond the local host: a non-loopback
    /// bind, OR a loopback bind explicitly declared `behind_tls_proxy` (a TLS-terminating
    /// reverse proxy on the same host forwards internet traffic to it). This is the predicate
    /// every "is this deployment exposed" security decision reads — `setup_token_policy`'s
    /// `"auto"` arm, `AppState::resolve_setup_token`'s warning, and the session cookie's
    /// `Secure` flag (`auth::session::session_layer`) all call this rather than
    /// `is_loopback_bind()` directly, so those cannot drift out of agreement with each other
    /// on this input. The HSTS response header (`http::router`) is a NARROWER decision and
    /// reads `behind_tls_proxy` directly, never through this predicate: HSTS instructs a
    /// browser to upgrade every future request to HTTPS, which is correct only when a TLS
    /// proxy is actually declared in front of this bind — a plain non-loopback bind with no
    /// TLS anywhere in the path is "exposed" by this predicate but must never send HSTS.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut cfg = shadowcat::config::Config::default(); // loopback, no proxy
    /// assert!(!cfg.is_exposed());
    /// cfg.behind_tls_proxy = true;
    /// assert!(cfg.is_exposed());
    /// ```
    pub fn is_exposed(&self) -> bool {
        !self.is_loopback_bind() || self.behind_tls_proxy
    }

    /// Resolve the `setup_token` string into the effective policy: `off` →
    /// open, `required` → token generated at boot, `auto` → derived from
    /// `is_exposed()` (not exposed = open, exposed = required), anything else = that
    /// literal token.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::config::{Config, SetupTokenPolicy};
    ///
    /// let cfg = Config::default(); // setup_token "auto" + loopback bind
    /// assert!(matches!(cfg.setup_token_policy(), SetupTokenPolicy::Open));
    /// ```
    pub fn setup_token_policy(&self) -> SetupTokenPolicy {
        match self.setup_token.as_str() {
            "off" => SetupTokenPolicy::Open,
            "required" => SetupTokenPolicy::Required(None),
            "auto" => {
                if self.is_exposed() {
                    SetupTokenPolicy::Required(None)
                } else {
                    SetupTokenPolicy::Open
                }
            }
            explicit => SetupTokenPolicy::Required(Some(explicit.to_string())),
        }
    }

    /// Whether `ip` is a configured trusted proxy (`trusted_proxies`), matched by exact address.
    /// An unparseable entry in `trusted_proxies` is skipped (warned), not fatal.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::net::IpAddr;
    /// let cfg = shadowcat::config::Config {
    ///     trusted_proxies: vec!["127.0.0.1".into()],
    ///     ..Default::default()
    /// };
    /// assert!(cfg.is_trusted_proxy("127.0.0.1".parse::<IpAddr>().unwrap()));
    /// assert!(!cfg.is_trusted_proxy("8.8.8.8".parse::<IpAddr>().unwrap()));
    /// ```
    pub fn is_trusted_proxy(&self, ip: std::net::IpAddr) -> bool {
        self.trusted_proxies
            .iter()
            .any(|s| match s.parse::<std::net::IpAddr>() {
                Ok(configured) => configured == ip,
                Err(_) => {
                    tracing::warn!(entry = %s, "trusted_proxies: unparseable IP address, skipped");
                    false
                }
            })
    }
}

#[cfg(test)]
mod tests;
