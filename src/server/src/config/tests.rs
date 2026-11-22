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
        command: None,
        ..Default::default()
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
        trusted_proxies: vec!["127.0.0.1".parse().unwrap(), "10.0.0.5".parse().unwrap()],
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
fn trusted_proxy_matches_across_ipv4_mapped_forms() {
    let v4 = Config {
        trusted_proxies: vec!["192.0.2.1".parse().unwrap()],
        ..Config::default()
    };
    assert!(v4.is_trusted_proxy("::ffff:192.0.2.1".parse().unwrap()));
    let mapped = Config {
        trusted_proxies: vec!["::ffff:192.0.2.1".parse().unwrap()],
        ..Config::default()
    };
    assert!(mapped.is_trusted_proxy("192.0.2.1".parse().unwrap()));
    assert!(!mapped.is_trusted_proxy("192.0.2.2".parse().unwrap()));
}

#[test]
fn an_unparseable_trusted_proxy_entry_fails_boot_naming_key_and_value() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shadowcat.toml");
    std::fs::write(
        &path,
        "trusted_proxies = [\"127.0.0.1\", \"not-an-ip\"]
",
    )
    .unwrap();
    let _guard = config_env_test_lock();
    let err = Config::load(Cli {
        config: Some(path.to_string_lossy().into_owned()),
        ..Default::default()
    })
    .expect_err("an entry that is not an IP address must fail boot");
    let text = err.to_string();
    assert!(text.contains("trusted_proxies"), "names the key: {text}");
    assert!(text.contains("not-an-ip"), "names the value: {text}");
}

#[test]
fn ipv6_literal_trusted_proxies_load_from_toml_and_from_the_environment() {
    let expected: Vec<std::net::IpAddr> = vec![
        "::1".parse().unwrap(),
        "2001:db8::1".parse().unwrap(),
        "::ffff:192.0.2.1".parse().unwrap(),
    ];
    let _guard = config_env_test_lock();

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shadowcat.toml");
    std::fs::write(
        &path,
        "trusted_proxies = [\"::1\", \"2001:db8::1\", \"::ffff:192.0.2.1\"]
",
    )
    .unwrap();
    let from_toml = Config::load(Cli {
        config: Some(path.to_string_lossy().into_owned()),
        ..Default::default()
    })
    .expect("IPv6 literals load from TOML");
    assert_eq!(from_toml.trusted_proxies, expected);

    // SAFETY: single-threaded test-only env mutation (serialised by the lock), restored below.
    unsafe {
        std::env::set_var(
            "SHADOWCAT_TRUSTED_PROXIES",
            "[::1,2001:db8::1,::ffff:192.0.2.1]",
        );
    }
    let from_env = Config::load(Cli {
        config: Some("/nonexistent/shadowcat.toml".into()),
        ..Default::default()
    });
    // SAFETY: matches the set_var above.
    unsafe {
        std::env::remove_var("SHADOWCAT_TRUSTED_PROXIES");
    }
    assert_eq!(
        from_env
            .expect("IPv6 literals load from the environment")
            .trusted_proxies,
        expected
    );
}

#[test]
fn trusted_proxies_env_var_needs_bracket_syntax_for_a_list() {
    // A bare scalar value for a Vec field fails to parse through figment's Env
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
        "a bare scalar env value for a Vec field was expected to fail to parse"
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
    assert_eq!(
        cfg.trusted_proxies,
        vec!["127.0.0.1".parse::<std::net::IpAddr>().unwrap()]
    );
}

#[test]
fn behind_proxy_defaults_false_and_env_overrides() {
    let cfg = Config::default();
    assert!(!cfg.behind_proxy);

    let _guard = config_env_test_lock();
    // SAFETY: single-threaded test-only env mutation, restored below.
    unsafe {
        std::env::set_var("SHADOWCAT_BEHIND_PROXY", "true");
    }
    let cli = Cli {
        config: Some("/nonexistent/shadowcat.toml".into()),
        ..Default::default()
    };
    let cfg = Config::load(cli).expect("load");
    // SAFETY: matches the set_var above.
    unsafe {
        std::env::remove_var("SHADOWCAT_BEHIND_PROXY");
    }
    assert!(cfg.behind_proxy);
}

#[test]
fn behind_proxy_cli_flag_overrides_env_and_default() {
    let _guard = config_env_test_lock();
    let cli = Cli {
        behind_proxy: Some(true),
        ..Cli::default()
    };
    let cfg = Config::load(cli).expect("load");
    assert!(cfg.behind_proxy);
}

/// The retired key is a boot error from the environment layer, naming the replacement.
#[test]
fn retired_behind_tls_proxy_key_fails_boot_from_env_even_when_false() {
    let _guard = config_env_test_lock();
    // SAFETY: single-threaded test-only env mutation, restored below.
    unsafe {
        std::env::set_var("SHADOWCAT_BEHIND_TLS_PROXY", "false");
    }
    let cli = Cli {
        config: Some("/nonexistent/shadowcat.toml".into()),
        ..Default::default()
    };
    let result = Config::load(cli);
    // SAFETY: matches the set_var above.
    unsafe {
        std::env::remove_var("SHADOWCAT_BEHIND_TLS_PROXY");
    }
    let err = result.expect_err("a retired key must not be silently ignored");
    let text = err.to_string();
    assert!(text.contains("behind_proxy"), "{text}");
    assert!(text.contains("behind_tls_proxy"), "{text}");
}

/// The retired key is a boot error from the TOML layer too.
#[test]
fn retired_behind_tls_proxy_key_fails_boot_from_toml() {
    let _guard = config_env_test_lock();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shadowcat.toml");
    std::fs::write(&path, "behind_tls_proxy = true\n").unwrap();
    let cli = Cli {
        config: Some(path.to_string_lossy().into_owned()),
        ..Default::default()
    };
    let err = Config::load(cli).expect_err("retired key in TOML");
    assert!(err.to_string().contains("behind_proxy"));
}

/// `--behind-tls-proxy` is not a CLI flag, so clap rejects it before `Config::load` runs;
/// `--behind-proxy` parses.
#[test]
fn retired_behind_tls_proxy_cli_flag_is_rejected_by_clap() {
    use clap::Parser;
    assert!(Cli::try_parse_from(["shadowcat", "--behind-tls-proxy", "true"]).is_err());
    assert!(Cli::try_parse_from(["shadowcat", "--behind-proxy", "true"]).is_ok());
}

/// A loopback bind is `is_exposed() == false` on its own, but a reverse proxy on the same host
/// makes it internet-reachable — the flag alone must flip the derived predicate.
#[test]
fn is_exposed_true_when_loopback_bind_is_behind_a_proxy() {
    let cfg = Config::default(); // loopback bind, behind_proxy = false
    assert!(cfg.is_loopback_bind());
    assert!(!cfg.is_exposed());

    let cfg = Config {
        behind_proxy: true,
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

/// Anti-drift: the setup-token policy and the exposure warning read the one `is_exposed()`
/// predicate and flip together with `behind_proxy` on a loopback bind.
#[test]
fn behind_proxy_flips_setup_token_policy_and_exposure_together() {
    let loopback_cfg = Config::default();
    assert!(matches!(
        loopback_cfg.setup_token_policy(),
        SetupTokenPolicy::Open
    ));
    assert!(!loopback_cfg.is_exposed());

    let proxied_cfg = Config {
        behind_proxy: true,
        ..Config::default()
    };
    assert!(matches!(
        proxied_cfg.setup_token_policy(),
        SetupTokenPolicy::Required(None)
    ));
    assert!(proxied_cfg.is_exposed());
}

#[test]
fn behind_proxy_without_trusted_proxies_warns_once() {
    let bare = Config {
        behind_proxy: true,
        ..Config::default()
    };
    let warnings = bare.startup_warnings();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("trusted_proxies"));

    let listed = Config {
        behind_proxy: true,
        trusted_proxies: vec!["127.0.0.1".parse().unwrap()],
        ..Config::default()
    };
    assert!(listed.startup_warnings().is_empty());
    assert!(Config::default().startup_warnings().is_empty());
}

/// Every TLS/ACME key layers CLI > env > TOML > default.
#[test]
fn tls_keys_layer_default_toml_env_cli() {
    let _guard = config_env_test_lock();
    let d = Config::default();
    assert_eq!(d.tls, TlsMode::Auto);
    assert_eq!(d.acme_challenge, AcmeChallenge::TlsAlpn01);
    assert_eq!(d.acme_directory, DEFAULT_ACME_DIRECTORY);
    assert!(!d.acme_terms_agreed);
    assert!(d.tls_hostnames.is_empty() && d.acme_domains.is_empty());
    assert!(d.hsts_max_age_secs.is_none() && d.public_url.is_none());

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shadowcat.toml");
    std::fs::write(
        &path,
        "tls = \"off\"\ntls_hostnames = [\"toml.example\"]\nhsts_max_age_secs = 100\npublic_url = \"https://toml.example\"\ntls_dir = \"toml-tls\"\n",
    )
    .unwrap();
    let toml_only = Config::load(Cli {
        config: Some(path.to_string_lossy().into_owned()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(toml_only.tls, TlsMode::Off);
    assert_eq!(toml_only.tls_hostnames, vec!["toml.example".to_string()]);
    assert_eq!(toml_only.hsts_max_age_secs, Some(100));
    assert_eq!(
        toml_only.public_url.as_deref(),
        Some("https://toml.example")
    );
    assert_eq!(toml_only.tls_dir.as_deref(), Some("toml-tls"));

    // SAFETY: single-threaded (env-lock guarded) test-only env mutation, restored below.
    unsafe {
        std::env::set_var("SHADOWCAT_HSTS_MAX_AGE_SECS", "200");
        std::env::set_var("SHADOWCAT_TLS_HOSTNAMES", "[env.example]");
    }
    let layered = Config::load(Cli {
        config: Some(path.to_string_lossy().into_owned()),
        ..Default::default()
    });
    // SAFETY: matches the set_var above.
    unsafe {
        std::env::remove_var("SHADOWCAT_HSTS_MAX_AGE_SECS");
        std::env::remove_var("SHADOWCAT_TLS_HOSTNAMES");
    }
    let layered = layered.unwrap();
    assert_eq!(layered.hsts_max_age_secs, Some(200));
    assert_eq!(layered.tls_hostnames, vec!["env.example".to_string()]);

    let cli = Cli {
        config: Some(path.to_string_lossy().into_owned()),
        tls: Some(TlsMode::Auto),
        tls_hostnames: Some(vec!["cli.example".into()]),
        hsts_max_age_secs: Some(300),
        public_url: Some("https://cli.example".into()),
        tls_dir: Some("cli-tls".into()),
        acme_challenge: Some(AcmeChallenge::Http01),
        acme_directory: Some("https://ca.example/dir".into()),
        acme_terms_agreed: Some(true),
        acme_email: Some("ops@example.com".into()),
        ..Default::default()
    };
    let cfg = Config::load(cli).unwrap();
    assert_eq!(cfg.tls, TlsMode::Auto);
    assert_eq!(cfg.tls_hostnames, vec!["cli.example".to_string()]);
    assert_eq!(cfg.hsts_max_age_secs, Some(300));
    assert_eq!(cfg.public_url.as_deref(), Some("https://cli.example"));
    assert_eq!(cfg.tls_dir.as_deref(), Some("cli-tls"));
    assert_eq!(cfg.acme_challenge, AcmeChallenge::Http01);
    assert_eq!(cfg.acme_directory, "https://ca.example/dir");
    assert!(cfg.acme_terms_agreed);
    assert_eq!(cfg.acme_email.as_deref(), Some("ops@example.com"));
}

/// The CLI parses every new flag, including comma-separated list values.
#[test]
fn tls_cli_flags_parse() {
    use clap::Parser;
    let cli = Cli::parse_from([
        "shadowcat",
        "--tls",
        "off",
        "--tls-cert",
        "c.pem",
        "--tls-key",
        "k.pem",
        "--tls-hostnames",
        "a.example,b.example",
        "--acme-domains",
        "g.example",
        "--acme-challenge",
        "http-01",
        "--hsts-max-age-secs",
        "60",
        "--tls-regenerate",
    ]);
    assert_eq!(cli.tls, Some(TlsMode::Off));
    assert_eq!(cli.tls_hostnames.unwrap().len(), 2);
    assert_eq!(cli.acme_challenge, Some(AcmeChallenge::Http01));
    assert_eq!(cli.hsts_max_age_secs, Some(60));
    assert!(cli.tls_regenerate);
}

/// Every contradiction `validate_tls` reports, each decidable from values alone.
#[test]
fn validate_tls_reports_every_contradiction() {
    let base = || Config {
        db: "data/shadowcat.db".into(),
        ..Config::default()
    };
    assert!(base().validate_tls().is_ok());

    let pem = || Config {
        tls_cert: Some("c.pem".into()),
        tls_key: Some("k.pem".into()),
        ..base()
    };
    assert!(pem().validate_tls().is_ok());

    let acme = || Config {
        acme_domains: vec!["game.example".into()],
        acme_terms_agreed: true,
        ..base()
    };
    assert!(acme().validate_tls().is_ok());

    // PEM + ACME.
    let both = Config {
        acme_domains: vec!["game.example".into()],
        acme_terms_agreed: true,
        ..pem()
    };
    assert_eq!(both.validate_tls(), Err(TlsConfigError::PemAndAcme));
    // tls = off with PEM / ACME.
    let off_pem = Config {
        tls: TlsMode::Off,
        ..pem()
    };
    assert!(matches!(
        off_pem.validate_tls(),
        Err(TlsConfigError::OffWithMaterial(_))
    ));
    let off_acme = Config {
        tls: TlsMode::Off,
        ..acme()
    };
    assert!(matches!(
        off_acme.validate_tls(),
        Err(TlsConfigError::OffWithMaterial(_))
    ));
    // Half a PEM pair.
    let only_cert = Config {
        tls_cert: Some("c.pem".into()),
        ..base()
    };
    assert_eq!(
        only_cert.validate_tls(),
        Err(TlsConfigError::HalfPemPair("tls_key"))
    );
    let only_key = Config {
        tls_key: Some("k.pem".into()),
        ..base()
    };
    assert_eq!(
        only_key.validate_tls(),
        Err(TlsConfigError::HalfPemPair("tls_cert"))
    );
    // ACME without the terms flag.
    let no_terms = Config {
        acme_terms_agreed: false,
        ..acme()
    };
    assert_eq!(
        no_terms.validate_tls(),
        Err(TlsConfigError::AcmeWithoutTerms)
    );
    // ACME with an IP literal, and with a wildcard.
    for bad in ["192.0.2.1", "*.example.com", "https://x.example", "a b"] {
        let c = Config {
            acme_domains: vec![bad.into()],
            ..acme()
        };
        assert!(
            matches!(c.validate_tls(), Err(TlsConfigError::AcmeDomainInvalid(_))),
            "{bad}"
        );
    }
    // ACME with an in-memory db and no tls_dir; a tls_dir cures it.
    let memory = Config {
        db: "sqlite::memory:".into(),
        ..acme()
    };
    assert_eq!(memory.validate_tls(), Err(TlsConfigError::AcmeNeedsTlsDir));
    let memory_with_dir = Config {
        tls_dir: Some("tls-state".into()),
        ..memory
    };
    assert!(memory_with_dir.validate_tls().is_ok());
    // Malformed tls_hostnames entries; valid wildcard and IP entries.
    for bad in [
        "https://x.example",
        "x.example:443",
        "x.example/path",
        "",
        "-x.example",
    ] {
        let c = Config {
            tls_hostnames: vec![bad.into()],
            ..base()
        };
        assert!(
            matches!(c.validate_tls(), Err(TlsConfigError::HostnameInvalid(_))),
            "{bad:?}"
        );
    }
    let ok_hosts = Config {
        tls_hostnames: vec!["*.game.example".into(), "192.0.2.9".into(), "::1".into()],
        ..base()
    };
    assert!(ok_hosts.validate_tls().is_ok());
    // public_url must be an https origin: no user information, path, query or fragment.
    for bad in [
        "http://x.example",
        "x.example",
        "https://",
        "ftp://x.example",
        "https://user:secret@x.example",
        "https://user@x.example",
        "https://x.example/lobby",
        "https://x.example/?token=1",
        "https://x.example/#top",
    ] {
        let c = Config {
            public_url: Some(bad.into()),
            ..base()
        };
        assert!(
            matches!(c.validate_tls(), Err(TlsConfigError::PublicUrlInvalid(_))),
            "{bad}"
        );
    }
    for ok in [
        "https://x.example",
        "https://x.example/",
        "https://x.example:8443",
    ] {
        let c = Config {
            public_url: Some(ok.into()),
            ..base()
        };
        assert!(c.validate_tls().is_ok(), "{ok}");
    }
    // acme_directory must be https://.
    let plain_dir = Config {
        acme_directory: "http://ca.example/dir".into(),
        ..base()
    };
    assert!(matches!(
        plain_dir.validate_tls(),
        Err(TlsConfigError::AcmeDirectoryNotHttps(_))
    ));
    // acme_ca_root without acme_domains.
    let orphan_root = Config {
        acme_ca_root: Some("root.pem".into()),
        ..base()
    };
    assert_eq!(
        orphan_root.validate_tls(),
        Err(TlsConfigError::AcmeCaRootWithoutDomains)
    );
    // acme_ca_root with acme_domains is fine at validation time: the FILE is read later and
    // degrades rather than aborting boot.
    let with_root = Config {
        acme_ca_root: Some("does-not-exist.pem".into()),
        ..acme()
    };
    assert!(with_root.validate_tls().is_ok());
}

/// `Config::load` does not run `validate_tls`: the entry points that use TLS settings do, so a
/// backup or restore is not blocked by an unrelated TLS mistake.
#[test]
fn load_leaves_tls_validation_to_the_entry_points() {
    let _guard = config_env_test_lock();
    let cli = Cli {
        config: Some("/nonexistent/shadowcat.toml".into()),
        tls: Some(TlsMode::Off),
        tls_cert: Some("c.pem".into()),
        tls_key: Some("k.pem".into()),
        ..Default::default()
    };
    let cfg = Config::load(cli).expect("load only reads configuration");
    let err = cfg.validate_tls().expect_err("off + PEM");
    assert!(err.to_string().contains("off"));
}

#[test]
fn the_retired_key_message_gives_the_full_proxy_migration() {
    for needle in [
        "behind_proxy",
        "trusted_proxies",
        "X-Forwarded-Proto",
        "OVERWRITE",
    ] {
        assert!(
            super::RETIRED_BEHIND_TLS_PROXY_MESSAGE.contains(needle),
            "the migration text must mention {needle}"
        );
    }
}

#[test]
fn tls_path_defaults_to_db_sibling_and_explicit_wins() {
    let mut cfg = Config {
        db: "/data/shadowcat.db".into(),
        ..Config::default()
    };
    assert_eq!(
        cfg.tls_path(),
        Some(std::path::PathBuf::from("/data").join("tls"))
    );
    cfg.tls_dir = Some("/custom/tls".into());
    assert_eq!(
        cfg.tls_path(),
        Some(std::path::PathBuf::from("/custom/tls"))
    );
    let memory = Config {
        db: "sqlite::memory:".into(),
        ..Config::default()
    };
    assert_eq!(memory.tls_path(), None);
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

#[test]
fn cap_exemption_covers_loopback_and_configured_proxies_and_nothing_else() {
    let cfg = Config {
        trusted_proxies: vec!["192.0.2.50".parse().unwrap()],
        ..Config::default()
    };
    for exempt in [
        "127.0.0.1",
        "127.0.0.2",
        "::1",
        "::ffff:127.0.0.1",
        "192.0.2.50",
    ] {
        assert!(cfg.is_cap_exempt(exempt.parse().unwrap()), "{exempt}");
    }
    for capped in ["192.0.2.1", "8.8.8.8", "2001:db8::1", "::ffff:192.0.2.1"] {
        assert!(!cfg.is_cap_exempt(capped.parse().unwrap()), "{capped}");
    }
}

#[test]
fn loopback_is_cap_exempt_without_becoming_a_forwarded_header_authority() {
    let cfg = Config::default();
    assert!(cfg.trusted_proxies.is_empty());
    for loopback in ["127.0.0.1", "127.0.0.2", "::1", "::ffff:127.0.0.1"] {
        let ip = loopback.parse().unwrap();
        assert!(cfg.is_cap_exempt(ip), "{loopback}");
        assert!(!cfg.is_trusted_proxy(ip), "{loopback}");
    }
}

#[test]
fn egress_window_defaults_to_the_peer_stall_timeout() {
    let cfg = Config::default();
    assert_eq!(
        cfg.egress_window(),
        crate::http::conn_serve::PEER_STALL_TIMEOUT
    );
    assert_eq!(cfg.egress_window_secs, 60);
}

/// Loads a configuration from the environment's `SHADOWCAT_EGRESS_WINDOW_SECS` and `cli`.
fn load_with_egress_env(value: Option<&str>, cli: Cli) -> Result<Config, Box<figment::Error>> {
    let _guard = config_env_test_lock();
    // SAFETY: single-threaded test-only env mutation under `config_env_test_lock`, restored below.
    unsafe {
        match value {
            Some(v) => std::env::set_var("SHADOWCAT_EGRESS_WINDOW_SECS", v),
            None => std::env::remove_var("SHADOWCAT_EGRESS_WINDOW_SECS"),
        }
    }
    let loaded = Config::load(cli);
    // SAFETY: as above.
    unsafe { std::env::remove_var("SHADOWCAT_EGRESS_WINDOW_SECS") };
    loaded
}

#[test]
fn egress_window_layers_cli_over_env_over_default() {
    let cfg = load_with_egress_env(None, Cli::default()).unwrap();
    assert_eq!(cfg.egress_window_secs, 60);
    let cfg = load_with_egress_env(Some("120"), Cli::default()).unwrap();
    assert_eq!(cfg.egress_window_secs, 120);
    assert_eq!(cfg.egress_window(), std::time::Duration::from_secs(120));
    let cli = Cli {
        egress_window_secs: Some(30),
        ..Cli::default()
    };
    let cfg = load_with_egress_env(Some("120"), cli).unwrap();
    assert_eq!(cfg.egress_window_secs, 30);
}

/// Loads a config with `SHADOWCAT_PORT_FORWARD` set to `value` (or removed) for the call.
fn load_with_port_forward_env(value: Option<&str>, cli: Cli) -> Config {
    let _guard = config_env_test_lock();
    // SAFETY: single-threaded test-only env mutation under `config_env_test_lock`, restored below.
    unsafe {
        match value {
            Some(v) => std::env::set_var("SHADOWCAT_PORT_FORWARD", v),
            None => std::env::remove_var("SHADOWCAT_PORT_FORWARD"),
        }
    }
    let loaded = Config::load(cli).unwrap();
    // SAFETY: as above.
    unsafe { std::env::remove_var("SHADOWCAT_PORT_FORWARD") };
    loaded
}

#[test]
fn port_forward_is_unset_by_default_and_layers_cli_over_env() {
    assert_eq!(
        load_with_port_forward_env(None, Cli::default()).port_forward,
        None
    );
    assert_eq!(
        load_with_port_forward_env(Some("true"), Cli::default()).port_forward,
        Some(true)
    );
    let cli = Cli {
        port_forward: Some(false),
        ..Cli::default()
    };
    assert_eq!(
        load_with_port_forward_env(Some("true"), cli).port_forward,
        Some(false)
    );
}

#[test]
fn backup_retention_defaults_to_unlimited_and_takes_cli_values() {
    let _guard = config_env_test_lock();
    let cfg = Config::load(Cli::default()).unwrap();
    assert_eq!((cfg.backup_keep_last, cfg.backup_keep_days), (None, None));
    let cli = Cli {
        backup_keep_last: Some(4),
        backup_keep_days: Some(30),
        ..Cli::default()
    };
    let cfg = Config::load(cli).unwrap();
    assert_eq!(cfg.backup_keep_last, Some(4));
    assert_eq!(cfg.backup_keep_days, Some(30));
}

#[test]
fn egress_window_accepts_exactly_one_to_thirty_six_hundred_seconds() {
    for ok in [EGRESS_WINDOW_SECS_MIN, EGRESS_WINDOW_SECS_MAX] {
        let cfg = load_with_egress_env(Some(&ok.to_string()), Cli::default()).unwrap();
        assert_eq!(cfg.egress_window_secs, ok);
    }
    for bad in [EGRESS_WINDOW_SECS_MIN - 1, EGRESS_WINDOW_SECS_MAX + 1] {
        let from_env = load_with_egress_env(Some(&bad.to_string()), Cli::default());
        assert!(from_env.is_err(), "env value {bad} was accepted");
        let cli = Cli {
            egress_window_secs: Some(bad),
            ..Cli::default()
        };
        assert!(
            load_with_egress_env(None, cli).is_err(),
            "CLI value {bad} was accepted"
        );
    }
}

#[test]
fn one_configured_window_reaches_the_response_bounds_and_the_websocket_send_bound() {
    let cfg = Config {
        egress_window_secs: 7,
        ..Config::default()
    };
    let (_tx, rx) = tokio::sync::watch::channel(false);
    let sink = crate::ws::bounded_sink::BoundedSink::for_config(
        futures_util::sink::drain::<u8>(),
        rx,
        &cfg,
    );
    assert_eq!(
        crate::http::conn_serve::ConnLimits::for_config(&cfg).egress_window,
        std::time::Duration::from_secs(7)
    );
    assert_eq!(sink.stall_timeout(), std::time::Duration::from_secs(7));
}

/// Loads a TOML body through the real layering with the env lock held.
fn load_toml(body: &str) -> Result<Config, Box<figment::Error>> {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("shadowcat.toml");
    std::fs::write(&file, body).unwrap();
    let _guard = config_env_test_lock();
    Config::load(Cli {
        config: Some(file.to_string_lossy().into_owned()),
        ..Default::default()
    })
}

/// `n` days as a retention.
fn retention_days(n: u32) -> crate::trash::TrashRetention {
    crate::trash::TrashRetention::Days(std::num::NonZeroU32::new(n).unwrap())
}

/// A config rooted at `base` with the db beside it, so the default sibling layout applies.
fn toml_for(base: &std::path::Path, extra: &str) -> String {
    let db = base.join("shadowcat.db");
    format!("db = {:?}\n{extra}", db.to_string_lossy())
}

#[test]
fn retention_config_parses_int_never_and_disabled_and_rejects_zero_and_garbage() {
    use crate::trash::TrashRetention;
    let tmp = tempfile::tempdir().unwrap();
    let load = |line: &str| load_toml(&toml_for(tmp.path(), line));

    assert_eq!(load("").unwrap().trash_retention, TrashRetention::default());
    assert_eq!(
        load("trash_retention = 7").unwrap().trash_retention,
        retention_days(7)
    );
    assert_eq!(
        load("trash_retention = \"never\"").unwrap().trash_retention,
        TrashRetention::Never
    );
    assert_eq!(
        load("trash_retention = \"disabled\"")
            .unwrap()
            .trash_retention,
        TrashRetention::Disabled
    );
    assert_eq!(
        load("trash_retention = \"14\"").unwrap().trash_retention,
        retention_days(14)
    );
    for bad in [
        "trash_retention = 0",
        "trash_retention = -5",
        "trash_retention = \"sometimes\"",
    ] {
        assert!(load(bad).is_err(), "{bad} must fail to load");
    }
    let zero = load("trash_retention = 0").unwrap_err().to_string();
    assert!(
        zero.contains("disabled"),
        "the zero error points at `disabled`: {zero}"
    );
}

#[test]
fn retention_cli_flag_overrides_the_file_and_env_supplies_an_int() {
    use crate::trash::TrashRetention;
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("shadowcat.toml");
    std::fs::write(&file, toml_for(tmp.path(), "trash_retention = 7")).unwrap();
    let _guard = config_env_test_lock();
    let cfg = Config::load(Cli {
        config: Some(file.to_string_lossy().into_owned()),
        trash_retention: Some("never".into()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(cfg.trash_retention, TrashRetention::Never);

    // SAFETY: serialised by the env lock, restored below.
    unsafe { std::env::set_var("SHADOWCAT_TRASH_RETENTION", "21") };
    let from_env = Config::load(Cli {
        config: Some(file.to_string_lossy().into_owned()),
        ..Default::default()
    });
    // SAFETY: matches the set_var above.
    unsafe { std::env::remove_var("SHADOWCAT_TRASH_RETENTION") };
    assert_eq!(from_env.unwrap().trash_retention, retention_days(21));
}

#[test]
fn trash_path_defaults_to_the_db_sibling_and_honours_an_explicit_dir() {
    let mut cfg = Config {
        db: "/data/shadowcat.db".into(),
        ..Config::default()
    };
    assert_eq!(
        cfg.trash_path(),
        std::path::PathBuf::from("/data").join("trash")
    );
    cfg.trash_dir = Some("/elsewhere/bin".into());
    assert_eq!(cfg.trash_path(), std::path::PathBuf::from("/elsewhere/bin"));
}

#[test]
fn trash_path_falls_back_to_assets_path_sibling_for_in_memory_db_with_explicit_assets_dir() {
    let cfg = Config {
        db: "sqlite::memory:".into(),
        assets_dir: Some("/vol/assets".into()),
        ..Config::default()
    };
    assert_eq!(
        cfg.trash_path(),
        std::path::PathBuf::from("/vol").join("trash")
    );
}

#[test]
fn trash_path_normalizes_a_single_component_assets_dir_parent_to_current_dir() {
    let cfg = Config {
        db: "sqlite::memory:".into(),
        assets_dir: Some("assets".into()),
        ..Config::default()
    };
    assert_eq!(
        cfg.trash_path(),
        std::path::PathBuf::from(".").join("trash")
    );
}

#[test]
fn config_load_accepts_the_default_sibling_layout() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(load_toml(&toml_for(tmp.path(), "")).is_ok());
}

#[test]
fn config_load_rejects_trash_dir_nested_inside_each_sibling_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    for key in ["assets_dir", "modules_dir", "backups_dir", "log_dir"] {
        let outer = base.join(format!("{key}-root"));
        let inner = outer.join("trash");
        let body = toml_for(
            base,
            &format!(
                "{key} = {:?}\ntrash_dir = {:?}",
                outer.to_string_lossy(),
                inner.to_string_lossy()
            ),
        );
        let err = load_toml(&body).expect_err(key).to_string();
        assert!(
            err.contains(key),
            "{key}: the error names the sibling: {err}"
        );
        assert!(
            err.contains(&*inner.to_string_lossy()) && err.contains(&*outer.to_string_lossy()),
            "{key}: the error names both paths: {err}"
        );
    }
}

#[test]
fn config_load_rejects_a_sibling_dir_nested_inside_trash_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let trash = tmp.path().join("bin");
    let body = toml_for(
        tmp.path(),
        &format!(
            "trash_dir = {:?}\nassets_dir = {:?}",
            trash.to_string_lossy(),
            trash.join("assets").to_string_lossy()
        ),
    );
    assert!(load_toml(&body).is_err());
}

#[test]
fn config_load_rejects_trash_dir_equal_to_a_sibling_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let same = tmp.path().join("shared");
    let body = toml_for(
        tmp.path(),
        &format!(
            "trash_dir = {:?}\nmodules_dir = {:?}",
            same.to_string_lossy(),
            same.to_string_lossy()
        ),
    );
    assert!(load_toml(&body).is_err());
}

#[test]
fn trash_dir_sharing_only_a_name_prefix_with_a_sibling_is_accepted() {
    let tmp = tempfile::tempdir().unwrap();
    let body = toml_for(
        tmp.path(),
        &format!(
            "assets_dir = {:?}\ntrash_dir = {:?}",
            tmp.path().join("assets").to_string_lossy(),
            tmp.path().join("assets-trash").to_string_lossy()
        ),
    );
    assert!(
        load_toml(&body).is_ok(),
        "component-wise comparison, not string prefix"
    );
}
