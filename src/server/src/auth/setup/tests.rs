use super::*;

#[tokio::test]
async fn bootstrap_seeds_admin_once_then_is_idempotent() {
    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let cfg = Config {
        admin_user: Some("ops".into()),
        admin_password: Some("pw-bootstrap".into()),
        ..Config::default()
    };

    assert!(bootstrap_admin(&repo, &cfg).await.unwrap());
    assert!(repo.admin_exists().await.unwrap());
    // Second call: admin already exists → no-op.
    assert!(!bootstrap_admin(&repo, &cfg).await.unwrap());
}

#[tokio::test]
async fn bootstrap_noop_without_config_creds() {
    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let cfg = Config::default();
    assert!(!bootstrap_admin(&repo, &cfg).await.unwrap());
    assert!(!repo.admin_exists().await.unwrap());
}
