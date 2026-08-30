use super::*;

/// Serializes every test that calls `Config::load` — its `figment::providers::Env` layer
/// reads real process env vars, which are shared process-global state; a test that
/// temporarily sets one (even to an intentionally-invalid value, to prove a parse-failure
/// case) would otherwise race any OTHER `Config::load` call running concurrently in this
/// same test binary and observe the pollution. Recovers from a poisoned lock (a prior test
/// panicking mid-guard) rather than propagating the poison to every later test.
fn config_env_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

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
    let _guard = config_env_test_lock();
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
    // Vulnerable to the same env-var-pollution race `config_env_test_lock` guards against —
    // this test doesn't mutate env vars itself, but `Config::load`'s env layer would still
    // observe another concurrently-running test's temporarily-set (and possibly
    // intentionally-invalid) value without this guard.
    let _guard = config_env_test_lock();
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
        retain_originals: None,
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

#[test]
fn trusted_proxy_matches_exact_configured_ip_only() {
    let cfg = Config {
        trusted_proxies: vec!["127.0.0.1".into(), "10.0.0.5".into()],
        ..Config::default()
    };
    assert!(cfg.is_trusted_proxy("127.0.0.1".parse().unwrap()));
    assert!(cfg.is_trusted_proxy("10.0.0.5".parse().unwrap()));
    assert!(!cfg.is_trusted_proxy("10.0.0.6".parse().unwrap()));
    assert!(!cfg.is_trusted_proxy("8.8.8.8".parse().unwrap()));
}

#[test]
fn trusted_proxy_default_is_empty_and_trusts_nothing() {
    let cfg = Config::default();
    assert!(cfg.trusted_proxies.is_empty());
    assert!(!cfg.is_trusted_proxy("127.0.0.1".parse().unwrap()));
}

#[test]
fn trusted_proxy_skips_unparseable_entries_without_panicking() {
    let cfg = Config {
        trusted_proxies: vec!["not-an-ip".into(), "127.0.0.1".into()],
        ..Config::default()
    };
    assert!(cfg.is_trusted_proxy("127.0.0.1".parse().unwrap()));
    assert!(!cfg.is_trusted_proxy("not-an-ip".parse().unwrap_or("0.0.0.0".parse().unwrap())));
}

#[test]
fn trusted_proxies_env_var_needs_bracket_syntax_for_a_list() {
    // A bare scalar value for a Vec<String> field fails to parse through figment's Env
    // provider, even for a single entry — the bracketed form is required.
    let _guard = config_env_test_lock();
    // SAFETY: single-threaded test-only env mutation, restored below.
    unsafe {
        std::env::set_var("SHADOWCAT_TRUSTED_PROXIES", "127.0.0.1");
    }
    let cli = Cli {
        config: Some("/nonexistent/shadowcat.toml".into()),
        ..Default::default()
    };
    let bare_result = Config::load(cli);
    unsafe {
        std::env::remove_var("SHADOWCAT_TRUSTED_PROXIES");
    }
    assert!(
        bare_result.is_err(),
        "a bare scalar env value for a Vec<String> field was expected to fail to parse"
    );

    // SAFETY: single-threaded test-only env mutation, restored below.
    unsafe {
        std::env::set_var("SHADOWCAT_TRUSTED_PROXIES", "[127.0.0.1]");
    }
    let cli = Cli {
        config: Some("/nonexistent/shadowcat.toml".into()),
        ..Default::default()
    };
    let cfg = Config::load(cli).expect("bracketed single-entry list should load");
    // SAFETY: matches the set_var above.
    unsafe {
        std::env::remove_var("SHADOWCAT_TRUSTED_PROXIES");
    }
    assert_eq!(cfg.trusted_proxies, vec!["127.0.0.1".to_string()]);
}

#[test]
fn retain_originals_defaults_true_and_cli_overrides() {
    let _guard = config_env_test_lock();
    let cfg = Config::load(Cli::default()).unwrap();
    assert!(cfg.retain_originals);
    let cli = Cli {
        retain_originals: Some(false),
        ..Cli::default()
    };
    let cfg = Config::load(cli).unwrap();
    assert!(!cfg.retain_originals);
}
