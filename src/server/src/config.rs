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
}

/// Effective server configuration after layering. Precedence (high→low):
/// CLI flag > SHADOWCAT_* env > TOML file > built-in default.
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
    /// Exact peer IP addresses trusted to set `X-Forwarded-For` (reverse-proxy deployments only).
    /// Default empty — `ClientIp` never consults the header and resolves solely from the TCP peer
    /// address, exactly as before this field existed. Matched by EXACT address only, not CIDR range
    /// (self-hosting operators list every trusted proxy's address explicitly; a typical single
    /// reverse-proxy-on-the-same-host deployment needs one entry, e.g. `"127.0.0.1"`). A request
    /// whose immediate TCP peer is NOT in this list gets its `X-Forwarded-For` header ignored
    /// entirely, regardless of content — this is what prevents an arbitrary internet client from
    /// spoofing its own address by sending the header directly.
    pub trusted_proxies: Vec<String>,
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
            trusted_proxies: Vec::new(),
        }
    }
}

/// Resolved setup-window policy. `Required(None)` means a token is required but
/// none was supplied — the server generates one at boot.
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
        if let Some(v) = cli.backups_dir {
            cfg.backups_dir = Some(v);
        }
        Ok(cfg)
    }

    /// Resolve the asset storage root: explicit `assets_dir`, else a sibling
    /// `assets/` directory beside the db file (built via std::path, #2).
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
        std::path::Path::new(&self.db)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("assets")
    }

    /// Resolve the installed-module discovery root: explicit `modules_dir`, else a
    /// sibling `modules/` directory beside the db file (built via std::path, #2).
    /// Unlike `assets_path`, nothing writes here server-side (install is a
    /// manual filesystem extract — no server-side installer exists) — the
    /// directory need not exist; a missing dir scans as "no modules installed"
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
        std::path::Path::new(&self.db)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("modules")
    }

    /// Resolve the in-server backup output root: explicit `backups_dir`, else a
    /// sibling `backups/` directory beside the db file (built via std::path, #2).
    ///
    /// # Examples
    ///
    /// ```
    /// let cfg = shadowcat::config::Config::default(); // db = ./shadowcat.db
    /// assert_eq!(cfg.backups_path(), std::path::Path::new(".").join("backups"));
    /// ```
    pub fn backups_path(&self) -> std::path::PathBuf {
        if let Some(dir) = &self.backups_dir {
            return std::path::PathBuf::from(dir);
        }
        std::path::Path::new(&self.db)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("backups")
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

    /// Resolve the `setup_token` string into the effective policy: `off` →
    /// open, `required` → token generated at boot, `auto` → derived from the
    /// bind (loopback = open, exposed = required), anything else = that
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
                if self.is_loopback_bind() {
                    SetupTokenPolicy::Open
                } else {
                    SetupTokenPolicy::Required(None)
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
