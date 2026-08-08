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
            login_per_min_per_identity: None,
            login_per_min_per_ip: None,
            invite_per_min_per_account: None,
            invite_per_min_per_ip: None,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_defaults_and_tiering() {
        use crate::data::document::WorldRole;
        let cfg = Config::default();
        // Default size cap 25 MiB, rate 20/min; GM = 2x when unset.
        assert_eq!(cfg.upload_max_bytes, 25 * 1024 * 1024);
        assert_eq!(cfg.effective_max_bytes(WorldRole::Player), 25 * 1024 * 1024);
        assert_eq!(cfg.effective_max_bytes(WorldRole::Gm), 50 * 1024 * 1024);
        assert_eq!(cfg.effective_rate_per_min(WorldRole::Player), 20);
        assert_eq!(cfg.effective_rate_per_min(WorldRole::Gm), 40);
    }

    #[test]
    fn assets_path_defaults_to_db_sibling() {
        let mut cfg = Config {
            db: "/data/shadowcat.db".into(),
            ..Config::default()
        };
        assert_eq!(
            cfg.assets_path(),
            std::path::PathBuf::from("/data").join("assets")
        );
        cfg.assets_dir = Some("/custom/assets".into());
        assert_eq!(
            cfg.assets_path(),
            std::path::PathBuf::from("/custom/assets")
        );
    }

    #[test]
    fn modules_path_defaults_to_db_sibling() {
        let mut cfg = Config {
            db: "/data/shadowcat.db".into(),
            ..Config::default()
        };
        assert_eq!(
            cfg.modules_path(),
            std::path::PathBuf::from("/data").join("modules")
        );
        cfg.modules_dir = Some("/custom/modules".into());
        assert_eq!(
            cfg.modules_path(),
            std::path::PathBuf::from("/custom/modules")
        );
    }

    #[test]
    fn backups_path_defaults_to_db_sibling() {
        let mut cfg = Config {
            db: "/data/shadowcat.db".into(),
            ..Config::default()
        };
        assert_eq!(
            cfg.backups_path(),
            std::path::PathBuf::from("/data").join("backups")
        );
        cfg.backups_dir = Some("/custom/backups".into());
        assert_eq!(
            cfg.backups_path(),
            std::path::PathBuf::from("/custom/backups")
        );
    }

    #[test]
    fn auth_throttle_budgets_default_to_none_and_env_overrides() {
        let cfg = Config::default();
        assert_eq!(cfg.login_per_min_per_identity, None);
        assert_eq!(cfg.login_per_min_per_ip, None);
        assert_eq!(cfg.invite_per_min_per_account, None);
        assert_eq!(cfg.invite_per_min_per_ip, None);

        // Layering: SHADOWCAT_* env overrides the built-in default, matching
        // every other optional Config field's precedence.
        // SAFETY: single-threaded test-only env mutation, restored below.
        unsafe {
            std::env::set_var("SHADOWCAT_LOGIN_PER_MIN_PER_IDENTITY", "10000");
        }
        let cli = Cli {
            config: Some("/nonexistent/shadowcat.toml".into()),
            ..Default::default()
        };
        let cfg = Config::load(cli).expect("load");
        // SAFETY: matches the set_var above.
        unsafe {
            std::env::remove_var("SHADOWCAT_LOGIN_PER_MIN_PER_IDENTITY");
        }
        assert_eq!(cfg.login_per_min_per_identity, Some(10_000));
    }

    #[test]
    fn defaults_apply_when_nothing_set() {
        let cfg = Config::default();
        assert_eq!(cfg.bind, "127.0.0.1:30000");
        assert_eq!(cfg.db, "./shadowcat.db");
        assert_eq!(cfg.setup_token, "auto");
        assert!(cfg.admin_user.is_none());
    }

    #[test]
    fn cli_overrides_take_precedence_over_defaults() {
        let cli = Cli {
            bind: Some("0.0.0.0:8080".into()),
            db: None,
            config: Some("/nonexistent/shadowcat.toml".into()),
            admin_user: Some("ops".into()),
            admin_password: None,
            setup_token: None,
            session_key: None,
            assets_dir: None,
            modules_dir: None,
            backups_dir: None,
            backup_to: None,
            restore_from: None,
            force: false,
        };
        let cfg = Config::load(cli).expect("load");
        assert_eq!(cfg.bind, "0.0.0.0:8080");
        assert_eq!(cfg.db, "./shadowcat.db"); // untouched default
        assert_eq!(cfg.admin_user.as_deref(), Some("ops"));
    }

    #[test]
    fn loopback_detection() {
        let mut cfg = Config::default();
        assert!(cfg.is_loopback_bind());
        cfg.bind = "0.0.0.0:30000".into();
        assert!(!cfg.is_loopback_bind());
        cfg.bind = "[::1]:30000".into();
        assert!(cfg.is_loopback_bind());
    }

    #[test]
    fn setup_token_policy_auto_derives_from_bind() {
        let mut cfg = Config::default(); // auto + loopback
        assert!(matches!(cfg.setup_token_policy(), SetupTokenPolicy::Open));
        cfg.bind = "0.0.0.0:30000".into();
        assert!(matches!(
            cfg.setup_token_policy(),
            SetupTokenPolicy::Required(None)
        ));
        cfg.setup_token = "off".into();
        assert!(matches!(cfg.setup_token_policy(), SetupTokenPolicy::Open));
        cfg.setup_token = "required".into();
        assert!(matches!(
            cfg.setup_token_policy(),
            SetupTokenPolicy::Required(None)
        ));
        cfg.setup_token = "s3cret".into();
        assert!(
            matches!(cfg.setup_token_policy(), SetupTokenPolicy::Required(Some(ref v)) if v == "s3cret")
        );
    }
}
