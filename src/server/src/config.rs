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
    #[arg(long)]
    pub bind: Option<String>,
    #[arg(long)]
    pub db: Option<String>,
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub admin_user: Option<String>,
    #[arg(long)]
    pub admin_password: Option<String>,
    #[arg(long)]
    pub setup_token: Option<String>,
    #[arg(long)]
    pub session_key: Option<String>,
    #[arg(long)]
    pub assets_dir: Option<String>,
}

/// Effective server configuration after layering. Precedence (high→low):
/// CLI flag > SHADOWCAT_* env > TOML file > built-in default.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub bind: String,
    pub db: String,
    pub admin_user: Option<String>,
    pub admin_password: Option<String>,
    /// "auto" | "off" | "required" | <explicit token>.
    pub setup_token: String,
    pub session_key: Option<String>,
    /// Asset storage root. `None` → sibling `assets/` beside the db file.
    pub assets_dir: Option<String>,
    /// Regular-uploader size cap (bytes). Default 25 MiB.
    pub upload_max_bytes: u64,
    /// Regular-uploader uploads per minute. Default 20.
    pub upload_rate_per_min: u32,
    /// GM/owner size cap; `None` → 2× `upload_max_bytes`.
    pub upload_max_bytes_gm: Option<u64>,
    /// GM/owner uploads per minute; `None` → 2× `upload_rate_per_min`.
    pub upload_rate_per_min_gm: Option<u32>,
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
            upload_max_bytes: 25 * 1024 * 1024,
            upload_rate_per_min: 20,
            upload_max_bytes_gm: None,
            upload_rate_per_min_gm: None,
        }
    }
}

/// Resolved setup-window policy. `Required(None)` means a token is required but
/// none was supplied — the server generates one at boot.
#[derive(Debug, Clone)]
pub enum SetupTokenPolicy {
    Open,
    Required(Option<String>),
}

impl Config {
    /// Layer file + env over defaults via figment, then apply CLI overrides in
    /// code so CLI strictly wins (figment cannot easily skip `None` CLI fields).
    // Boot-only call; figment::Error is third-party and large by value, so the
    // large-Result cost is irrelevant here.
    #[allow(clippy::result_large_err)]
    pub fn load(cli: Cli) -> Result<Self, figment::Error> {
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
        Ok(cfg)
    }

    /// Resolve the asset storage root: explicit `assets_dir`, else a sibling
    /// `assets/` directory beside the db file (built via std::path, #2).
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

    /// Role-tiered upload size cap (GM defaults to 2× the regular value).
    pub fn effective_max_bytes(&self, role: crate::data::document::WorldRole) -> u64 {
        match role {
            crate::data::document::WorldRole::Gm => self
                .upload_max_bytes_gm
                .unwrap_or(self.upload_max_bytes.saturating_mul(2)),
            _ => self.upload_max_bytes,
        }
    }

    /// Role-tiered uploads-per-minute (GM defaults to 2× the regular value).
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
    pub fn is_loopback_bind(&self) -> bool {
        self.bind
            .to_socket_addrs()
            .ok()
            .and_then(|mut a| a.next())
            .map(|addr: SocketAddr| addr.ip().is_loopback())
            .unwrap_or(false)
    }

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
