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
    pub fn load(cli: Cli) -> Result<Self, figment::Error> {
        let config_path = cli.config.clone().unwrap_or_else(|| "shadowcat.toml".into());
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
        Ok(cfg)
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
        assert!(matches!(cfg.setup_token_policy(), SetupTokenPolicy::Required(None)));
        cfg.setup_token = "off".into();
        assert!(matches!(cfg.setup_token_policy(), SetupTokenPolicy::Open));
        cfg.setup_token = "required".into();
        assert!(matches!(cfg.setup_token_policy(), SetupTokenPolicy::Required(None)));
        cfg.setup_token = "s3cret".into();
        assert!(matches!(cfg.setup_token_policy(), SetupTokenPolicy::Required(Some(ref v)) if v == "s3cret"));
    }
}
