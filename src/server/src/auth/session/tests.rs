use super::*;
use crate::auth::role::ServerRole;
use axum::routing::{get, post};
use axum::Router;
use tower_sessions::Session;
use uuid::Uuid;

// Test-only routes that exercise the extractors without the prod surface.
async fn login_as_admin(session: Session) -> &'static str {
    session
        .insert(
            "user",
            SessionUser {
                id: Uuid::from_u128(1),
                username: "a".into(),
                role: ServerRole::Admin,
            },
        )
        .await
        .unwrap();
    "ok"
}
async fn login_as_user(session: Session) -> &'static str {
    session
        .insert(
            "user",
            SessionUser {
                id: Uuid::from_u128(2),
                username: "u".into(),
                role: ServerRole::User,
            },
        )
        .await
        .unwrap();
    "ok"
}
async fn whoami(user: AuthUser) -> String {
    user.username
}
async fn admin_only(_admin: AdminUser) -> &'static str {
    "admin"
}

async fn harness() -> (axum_test::TestServer, ()) {
    let state = crate::http::tests::test_state().await;
    let layer = session_layer(&state.repo, &state.config).await.unwrap();
    let app = Router::new()
        .route("/t/login-admin", post(login_as_admin))
        .route("/t/login-user", post(login_as_user))
        .route("/t/me", get(whoami))
        .route("/t/admin", get(admin_only))
        .layer(layer)
        .with_state(state);
    (
        axum_test::TestServer::builder()
            .save_cookies()
            .build(app)
            .unwrap(),
        (),
    )
}

#[tokio::test]
async fn auth_user_requires_session() {
    let (server, _) = harness().await;
    server
        .get("/t/me")
        .await
        .assert_status(axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_extractor_rejects_non_admin() {
    let (server, _) = harness().await;
    server.post("/t/login-user").await.assert_status_ok();
    server.get("/t/me").await.assert_status_ok(); // any user passes AuthUser
    server
        .get("/t/admin")
        .await
        .assert_status(axum::http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_extractor_accepts_admin() {
    let (server, _) = harness().await;
    server.post("/t/login-admin").await.assert_status_ok();
    server.get("/t/admin").await.assert_status_ok();
}

#[tokio::test]
async fn delete_expired_removes_only_expired_rows() {
    let repo = crate::data::sqlite::SqliteRepository::connect("sqlite::memory:")
        .await
        .unwrap();
    let read_pool = repo.open_read_pool().await.unwrap();
    let store = SqlxSqliteStore::new(repo.pool().clone(), read_pool);
    store.migrate().await.unwrap();
    let now = OffsetDateTime::now_utc().unix_timestamp();

    // Boundary row (expiry == now) is non-loadable, so it must be swept too.
    for (id, expiry) in [
        ("expired", now - 100),
        ("boundary", now),
        ("live", now + 10_000),
    ] {
        sqlx::query("INSERT INTO tower_sessions (id, data, expiry_date) VALUES (?, '{}', ?)")
            .bind(id)
            .bind(expiry)
            .execute(repo.pool())
            .await
            .unwrap();
    }

    store.delete_expired().await.unwrap();

    let remaining: Vec<String> = sqlx::query_scalar("SELECT id FROM tower_sessions ORDER BY id")
        .fetch_all(repo.pool())
        .await
        .unwrap();
    assert_eq!(remaining, vec!["live".to_string()]);
}

#[tokio::test]
async fn session_key_is_stable_across_loads() {
    let repo = crate::data::sqlite::SqliteRepository::connect("sqlite::memory:")
        .await
        .unwrap();
    let cfg = crate::config::Config::default();
    let k1 = load_or_create_key(&repo, &cfg).await.unwrap();
    let k2 = load_or_create_key(&repo, &cfg).await.unwrap();
    assert_eq!(k1.master(), k2.master(), "persisted key must be reused");
}

#[tokio::test]
async fn sweep_deletes_only_rows_expired_past_grace() {
    use crate::data::sqlite::{NewInvite, SqliteRepository};
    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, crate::auth::role::ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", gm, 0).await.unwrap();
    let now: i64 = 100 * 24 * 60 * 60 * 1000; // day 100
    let mk = |id: u128, expires_at: i64| NewInvite {
        id: Uuid::from_u128(id),
        world: world.id,
        secret_hash: "x",
        role: crate::data::document::WorldRole::Player,
        created_by: gm,
        now: 0,
        expires_at,
    };
    // Expired 31 days ago -> swept. Expired 1 day ago -> kept (inside grace).
    repo.create_invite(mk(1, now - 31 * 24 * 60 * 60 * 1000), 64)
        .await
        .unwrap();
    repo.create_invite(mk(2, now - 24 * 60 * 60 * 1000), 64)
        .await
        .unwrap();
    let deleted = super::sweep_spent_invites(repo.pool(), now).await.unwrap();
    assert_eq!(deleted, 1);
    assert!(repo
        .invite_by_id(Uuid::from_u128(1))
        .await
        .unwrap()
        .is_none());
    assert!(repo
        .invite_by_id(Uuid::from_u128(2))
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn session_load_does_not_queue_behind_an_open_write_transaction() {
    let repo = crate::data::sqlite::SqliteRepository::connect("sqlite::memory:")
        .await
        .unwrap();
    let read_pool = repo.open_read_pool().await.unwrap();
    let store = SqlxSqliteStore::new(repo.pool().clone(), read_pool);
    store.migrate().await.unwrap();

    let mut record = Record {
        id: Id::default(),
        data: Default::default(),
        expiry_date: OffsetDateTime::now_utc() + Duration::days(1),
    };
    store.create(&mut record).await.unwrap();

    // Hold the write pool's single connection open in an uncommitted
    // transaction. Before this fix, `load()` shared this exact pool
    // (max_connections(1)), so it would have zero free connections to
    // acquire and block until this transaction ends.
    let tx = repo.pool().begin().await.unwrap();

    let loaded = tokio::time::timeout(std::time::Duration::from_secs(2), store.load(&record.id))
        .await
        .expect("load() must not queue behind an open write-pool transaction")
        .unwrap();
    assert_eq!(loaded.unwrap().id, record.id);

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn read_pool_shares_the_same_in_memory_database_as_the_write_pool() {
    let repo = crate::data::sqlite::SqliteRepository::connect("sqlite::memory:")
        .await
        .unwrap();
    let read_pool = repo.open_read_pool().await.unwrap();
    let store = SqlxSqliteStore::new(repo.pool().clone(), read_pool);
    store.migrate().await.unwrap();

    let mut record = Record {
        id: Id::default(),
        data: Default::default(),
        expiry_date: OffsetDateTime::now_utc() + Duration::days(1),
    };
    store.create(&mut record).await.unwrap();

    // If the read pool were built from a second, independent parse of
    // "sqlite::memory:" (the landmine this brief investigated), it would
    // point at a different, empty database and this would return None.
    let loaded = store.load(&record.id).await.unwrap();
    assert!(
        loaded.is_some(),
        "read pool must see data written via the write pool"
    );
}

#[tokio::test]
async fn read_pool_rejects_a_write() {
    // File-backed, not "sqlite::memory:": a named shared-cache in-memory
    // database does not enforce `SQLITE_OPEN_READONLY` against a write
    // from a connection sharing that cache (measured — a raw sqlx probe
    // with no shadowcat code in the loop still lets the write through),
    // while a real on-disk database does. Production always connects to
    // a real file, so this exercises the shape that matters; the
    // in-memory URL this repo's other tests share would pass unconditionally
    // here regardless of whether `.read_only(true)` is even applied.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("read_only_probe.db");
    let url = format!("sqlite://{}?mode=rwc", path.to_string_lossy());
    let repo = crate::data::sqlite::SqliteRepository::connect(&url)
        .await
        .unwrap();
    let read_pool = repo.open_read_pool().await.unwrap();
    let result = sqlx::query("CREATE TABLE t (x INTEGER)")
        .execute(&read_pool)
        .await;
    assert!(result.is_err(), "a read-only pool must reject a write");
}
