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
fn logs_path_defaults_to_db_sibling() {
    let mut cfg = Config {
        db: "/data/shadowcat.db".into(),
        ..Config::default()
    };
    assert_eq!(
        cfg.logs_path(),
        Some(std::path::PathBuf::from("/data").join("logs"))
    );
    cfg.log_dir = Some("/custom/logs".into());
    assert_eq!(
        cfg.logs_path(),
        Some(std::path::PathBuf::from("/custom/logs"))
    );
}

#[test]
fn log_dir_cli_flag_overrides_default() {
    let _guard = config_env_test_lock();
    let cli = Cli {
        log_dir: Some("/cli-logs".into()),
        config: Some("/nonexistent/shadowcat.toml".into()),
        ..Cli::default()
    };
    let cfg = Config::load(cli).expect("load");
    assert_eq!(cfg.log_dir.as_deref(), Some("/cli-logs"));
    assert_eq!(cfg.logs_path(), Some(std::path::PathBuf::from("/cli-logs")));
}

/// An in-memory/non-filesystem `db` has no on-disk sibling to place a `logs/` directory
/// beside, so `logs_path` must return `None` (file logging off) rather than inventing a
/// cwd-relative directory nobody asked for (see `main::build_rolling_appender`'s own doc for
/// the consequence this invariant prevents).
#[test]
fn logs_path_is_none_for_an_in_memory_database() {
    let cfg = Config {
        db: "sqlite::memory:".into(),
        ..Config::default()
    };
    assert_eq!(cfg.logs_path(), None);
}

/// A real on-disk db path is unaffected by the in-memory carve-out — this is the ordinary
/// sibling-of-the-db-file case, restated here alongside the in-memory case for contrast.
#[test]
fn logs_path_resolves_a_sibling_for_a_real_db_file() {
    let cfg = Config {
        db: "data/shadowcat.db".into(),
        ..Config::default()
    };
    assert_eq!(
        cfg.logs_path(),
        Some(std::path::PathBuf::from("data").join("logs"))
    );
}

/// An explicit `log_dir` wins even when `db` is in-memory — an operator who names a directory
/// explicitly clearly wants file logging regardless of what the database itself is.
#[test]
fn logs_path_explicit_log_dir_wins_over_an_in_memory_database() {
    let cfg = Config {
        db: "sqlite::memory:".into(),
        log_dir: Some("/explicit-logs".into()),
        ..Config::default()
    };
    assert_eq!(
        cfg.logs_path(),
        Some(std::path::PathBuf::from("/explicit-logs"))
    );
}

#[test]
fn backups_path_for_defaults_to_the_db_files_sibling() {
    let mut cfg = Config::default();
    let db_file = std::path::PathBuf::from("/data").join("shadowcat.db");
    assert_eq!(
        cfg.backups_path_for(&db_file),
        std::path::PathBuf::from("/data").join("backups")
    );
    cfg.backups_dir = Some("/custom/backups".into());
    assert_eq!(
        cfg.backups_path_for(&db_file),
        std::path::PathBuf::from("/custom/backups")
    );
}

/// A bare database file name has `Path::parent() == Some("")`; the sibling
/// resolves against the current directory rather than an empty-rooted path.
#[test]
fn backups_path_for_a_bare_db_file_name_resolves_against_the_current_directory() {
    let cfg = Config::default();
    assert_eq!(
        cfg.backups_path_for(std::path::Path::new("shadowcat.db")),
        std::path::Path::new(".").join("backups")
    );
}

/// `Config.db` accepts a `sqlite:`-scheme URL, not only a bare path: the file
/// `crate::db::database_file` resolves from it is what `backups_path_for`
/// places the sibling beside — the composition `http::routes::admin_backup`
/// performs.
#[test]
fn backups_path_for_resolves_a_sibling_for_an_absolute_sqlite_url() {
    let cfg = Config {
        db: "sqlite:///data/shadowcat.db".into(),
        ..Config::default()
    };
    let db_file = crate::db::database_file(&cfg.db).unwrap().unwrap();
    assert_eq!(
        cfg.backups_path_for(&db_file),
        std::path::PathBuf::from("/data").join("backups")
    );
}

/// `Config.db` may also be a relative `sqlite:` URL (no leading `//`).
#[test]
fn logs_path_resolves_a_sibling_for_a_relative_sqlite_url() {
    let cfg = Config {
        db: "sqlite:data/shadowcat.db".into(),
        ..Config::default()
    };
    assert_eq!(
        cfg.logs_path(),
        Some(std::path::PathBuf::from("data").join("logs"))
    );
}

/// `mode=memory` is a second in-memory spelling `SqliteConnectOptions` reads
/// on any URL, independent of the `sqlite::memory:`/`:memory:` forms —
/// `logs_path` must treat it the same way.
#[test]
fn logs_path_is_none_for_mode_equals_memory_query_param() {
    let cfg = Config {
        db: "sqlite://a.db?mode=memory".into(),
        ..Config::default()
    };
    assert_eq!(cfg.logs_path(), None);
}

/// A db path containing the substring "memory" (Windows-style separators, so
/// this runs identically on every OS via `Path::join` rather than a literal
/// backslash) is still a real on-disk file — `logs_path` resolves it via
/// `SqliteConnectOptions`, never a `contains("memory")` check on the raw string.
#[test]
fn logs_path_resolves_a_sibling_for_a_windows_style_path_containing_memory() {
    let dir = std::path::Path::new("C:\\").join("memory-lane");
    let cfg = Config {
        db: dir.join("shadowcat.db").to_string_lossy().into_owned(),
        ..Config::default()
    };
    assert_eq!(cfg.logs_path(), Some(dir.join("logs")));
}

/// Same invariant, `/`-style separators.
#[test]
fn logs_path_resolves_a_sibling_for_a_path_containing_memory() {
    let dir = std::path::Path::new("memory-lane");
    let cfg = Config {
        db: dir.join("shadowcat.db").to_string_lossy().into_owned(),
        ..Config::default()
    };
    assert_eq!(cfg.logs_path(), Some(dir.join("logs")));
}

/// `assets_path` falls back to the current directory (never a bogus
/// substring-derived path) when the db is in-memory and no `assets_dir`
/// override is given — asset storage is required regardless of the db.
#[test]
fn assets_path_falls_back_to_cwd_for_an_in_memory_database() {
    let cfg = Config {
        db: "sqlite::memory:".into(),
        ..Config::default()
    };
    assert_eq!(
        cfg.assets_path(),
        std::path::PathBuf::from(".").join("assets")
    );
}

/// Same fallback rule for `modules_path`.
#[test]
fn modules_path_falls_back_to_cwd_for_an_in_memory_database() {
    let cfg = Config {
        db: "sqlite::memory:".into(),
        ..Config::default()
    };
    assert_eq!(
        cfg.modules_path(),
        std::path::PathBuf::from(".").join("modules")
    );
}

#[test]
fn auth_throttle_budgets_default_to_none_and_env_overrides() {
    let cfg = Config::default();
    assert_eq!(cfg.login_per_min_per_identity, None);
    assert_eq!(cfg.login_per_min_per_ip, None);
    assert_eq!(cfg.invite_per_min_per_account, None);
    assert_eq!(cfg.invite_per_min_per_ip, None);
    assert_eq!(cfg.world_create_per_min_per_account, None);

    // Layering: SHADOWCAT_* env overrides the built-in default, matching
    // every other optional Config field's precedence.
    let _guard = config_env_test_lock();
    // SAFETY: single-threaded test-only env mutation, restored below.
    unsafe {
        std::env::set_var("SHADOWCAT_LOGIN_PER_MIN_PER_IDENTITY", "10000");
        std::env::set_var("SHADOWCAT_WORLD_CREATE_PER_MIN_PER_ACCOUNT", "10000");
    }
    let cli = Cli {
        config: Some("/nonexistent/shadowcat.toml".into()),
        ..Default::default()
    };
    let cfg = Config::load(cli).expect("load");
    // SAFETY: matches the set_var above.
    unsafe {
        std::env::remove_var("SHADOWCAT_LOGIN_PER_MIN_PER_IDENTITY");
        std::env::remove_var("SHADOWCAT_WORLD_CREATE_PER_MIN_PER_ACCOUNT");
    }
    assert_eq!(cfg.login_per_min_per_identity, Some(10_000));
    assert_eq!(cfg.world_create_per_min_per_account, Some(10_000));
}

#[test]
fn non_numeric_world_create_throttle_budget_env_var_fails_to_load() {
    // A malformed `SHADOWCAT_WORLD_CREATE_PER_MIN_PER_ACCOUNT` value fails
    // `Config::load` outright rather than silently falling back to `None`/the
    // built-in default — the same layering `auth_throttle_budgets_default_to_none_and_env_overrides`
    // pins for a well-formed value.
    let _guard = config_env_test_lock();
    // SAFETY: single-threaded test-only env mutation, restored below.
    unsafe {
        std::env::set_var("SHADOWCAT_WORLD_CREATE_PER_MIN_PER_ACCOUNT", "not-a-number");
    }
    let cli = Cli {
        config: Some("/nonexistent/shadowcat.toml".into()),
        ..Default::default()
    };
    let result = Config::load(cli);
    // SAFETY: matches the set_var above.
    unsafe {
        std::env::remove_var("SHADOWCAT_WORLD_CREATE_PER_MIN_PER_ACCOUNT");
    }
    assert!(result.is_err());
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
        log_dir: None,
        backups_dir: None,
        backup_to: None,
        restore_from: None,
        force: false,
        retain_originals: None,
        behind_tls_proxy: None,
        command: None,
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
fn behind_tls_proxy_defaults_false_and_env_overrides() {
    let cfg = Config::default();
    assert!(!cfg.behind_tls_proxy);

    let _guard = config_env_test_lock();
    // SAFETY: single-threaded test-only env mutation, restored below.
    unsafe {
        std::env::set_var("SHADOWCAT_BEHIND_TLS_PROXY", "true");
    }
    let cli = Cli {
        config: Some("/nonexistent/shadowcat.toml".into()),
        ..Default::default()
    };
    let cfg = Config::load(cli).expect("load");
    // SAFETY: matches the set_var above.
    unsafe {
        std::env::remove_var("SHADOWCAT_BEHIND_TLS_PROXY");
    }
    assert!(cfg.behind_tls_proxy);
}

#[test]
fn behind_tls_proxy_cli_flag_overrides_env_and_default() {
    let _guard = config_env_test_lock();
    let cli = Cli {
        behind_tls_proxy: Some(true),
        ..Cli::default()
    };
    let cfg = Config::load(cli).expect("load");
    assert!(cfg.behind_tls_proxy);
}

/// A loopback bind is `is_exposed() == false` on its own, but a TLS-terminating
/// reverse proxy on the same host makes it internet-reachable — the flag alone
/// must flip the derived predicate.
#[test]
fn is_exposed_true_when_loopback_bind_is_behind_a_tls_proxy() {
    let cfg = Config::default(); // loopback bind, behind_tls_proxy = false
    assert!(cfg.is_loopback_bind());
    assert!(!cfg.is_exposed());

    let cfg = Config {
        behind_tls_proxy: true,
        ..Config::default()
    };
    assert!(cfg.is_loopback_bind());
    assert!(cfg.is_exposed());
}

#[test]
fn is_exposed_true_for_non_loopback_bind_regardless_of_proxy_flag() {
    let cfg = Config {
        bind: "0.0.0.0:30000".into(),
        ..Config::default()
    };
    assert!(cfg.is_exposed());
}

/// Anti-drift: the setup-token policy reads `is_exposed()` and flips with `behind_tls_proxy` on a
/// loopback bind; the TLS-presuming decisions (session `Secure`, HSTS) read `is_tls_fronted()`,
/// which a plain non-loopback bind does NOT satisfy.
#[test]
fn behind_tls_proxy_flips_setup_token_policy_and_tls_fronted_together() {
    let loopback_cfg = Config::default();
    assert!(matches!(
        loopback_cfg.setup_token_policy(),
        SetupTokenPolicy::Open
    ));
    assert!(!loopback_cfg.is_exposed());
    assert!(!loopback_cfg.is_tls_fronted());

    let proxied_cfg = Config {
        behind_tls_proxy: true,
        ..Config::default()
    };
    assert!(matches!(
        proxied_cfg.setup_token_policy(),
        SetupTokenPolicy::Required(None)
    ));
    assert!(proxied_cfg.is_exposed());
    assert!(proxied_cfg.is_tls_fronted());
}

/// A LAN bind (plain HTTP, no proxy) is exposed but not TLS-fronted.
#[test]
fn a_plain_non_loopback_bind_is_exposed_but_not_tls_fronted() {
    let cfg = Config {
        bind: "0.0.0.0:30000".into(),
        ..Config::default()
    };
    assert!(cfg.is_exposed());
    assert!(!cfg.is_tls_fronted());
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
