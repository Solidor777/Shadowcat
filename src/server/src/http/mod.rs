pub mod assets;
pub mod embed;
pub mod error;
pub mod module_routes;
pub mod routes;
pub mod throttle;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post};
use axum::Router;

use crate::config::Config;
use crate::data::sqlite::SqliteRepository;

/// Shared handler state. `initialized` caches "an admin exists" so the init
/// gate avoids a DB hit per request; `setup_token`, when `Some`, is the value
/// `/api/setup` requires.
#[derive(Clone)]
pub struct AppState {
    pub repo: Arc<SqliteRepository>,
    pub config: Arc<Config>,
    pub setup_token: Option<String>,
    pub initialized: Arc<AtomicBool>,
    pub ws: crate::ws::WsState,
    pub upload_rate: Arc<assets::UploadRateLimiter>,
    pub auth_throttle: Arc<throttle::AuthThrottle>,
    /// Write-quiesce barrier for the in-server backup route: held in write mode
    /// across `create_backup`'s `VACUUM INTO` + assets copy so no asset
    /// commit+rename interleaves with the snapshot. Asset writers hold the read
    /// side (shared among themselves, blocked by the writer).
    pub write_barrier: Arc<tokio::sync::RwLock<()>>,
}

impl AppState {
    /// Resolve the token `/api/setup` will require. `None` = open window.
    pub fn resolve_setup_token(config: &Config) -> Option<String> {
        use crate::config::SetupTokenPolicy;
        match config.setup_token_policy() {
            SetupTokenPolicy::Open => {
                if !config.is_loopback_bind() {
                    tracing::warn!(
                        "setup token disabled on a non-loopback bind; /api/setup is unauthenticated until an admin exists"
                    );
                }
                None
            }
            SetupTokenPolicy::Required(Some(v)) => Some(v),
            SetupTokenPolicy::Required(None) => {
                let token = uuid::Uuid::new_v4().simple().to_string();
                tracing::info!(%token, "setup token required; enter it in the setup form the app shows on first run");
                Some(token)
            }
        }
    }
}

pub async fn router(state: AppState) -> Router {
    use tower::ServiceBuilder;
    use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
    use tower_http::trace::TraceLayer;

    let sessions = crate::auth::session::session_layer(&state.repo, &state.config)
        .await
        .expect("session layer");

    Router::new()
        .route("/health", get(routes::health))
        .route("/api/config", get(routes::config))
        .route("/ws", get(crate::ws::conn::ws_handler))
        .route("/api/debug/rooms", get(routes::debug_rooms))
        .route("/api/admin/backup", post(routes::admin_backup))
        .route("/api/me", get(routes::me))
        .route(
            "/api/me/ui-state",
            get(routes::get_ui_state).put(routes::put_ui_state),
        )
        .route("/api/login", post(routes::login))
        .route("/api/logout", post(routes::logout))
        .route("/api/setup", post(routes::setup))
        .route(
            "/api/users",
            post(routes::create_user).get(routes::list_users),
        )
        .route(
            "/api/worlds",
            post(routes::create_world).get(routes::list_worlds),
        )
        .route(
            "/api/worlds/{id}/members",
            get(routes::list_members).post(routes::add_member),
        )
        .route(
            "/api/worlds/{id}/members/{user}",
            delete(routes::remove_member),
        )
        .route(
            "/api/worlds/{id}/invites",
            post(routes::create_invite).get(routes::list_invites),
        )
        .route(
            "/api/worlds/{id}/invites/{code_id}",
            delete(routes::revoke_invite),
        )
        // The code travels in the BODY: a path segment is recorded by the trace
        // span's `uri`, browser history, `Referer`, and proxy logs.
        .route("/api/invites/accept", post(routes::accept_invite))
        .route(
            "/api/worlds/{id}/capability-defaults",
            get(routes::get_world_capability_defaults).put(routes::set_world_capability_defaults),
        )
        .route(
            "/api/worlds/{id}/capability-requirements",
            get(routes::get_world_capability_requirements)
                .put(routes::set_world_capability_requirements),
        )
        .route(
            "/api/worlds/{id}/contracts",
            get(routes::get_world_contract_declarations)
                .put(routes::set_world_contract_declarations),
        )
        .route(
            "/api/worlds/{id}/schemas",
            get(routes::get_world_schema_declarations).put(routes::set_world_schema_declarations),
        )
        .route("/api/modules", get(module_routes::list_installed_modules))
        .route(
            "/modules/{id}/{*path}",
            get(module_routes::serve_module_file),
        )
        .route(
            "/api/worlds/{id}/enabled-modules",
            get(module_routes::get_world_enabled_modules)
                .put(module_routes::set_world_enabled_modules),
        )
        .route(
            "/api/worlds/{id}/documents",
            get(routes::list_documents).post(routes::create_document),
        )
        .route(
            "/api/documents/{id}",
            get(routes::get_document)
                .patch(routes::patch_document)
                .delete(routes::delete_document),
        )
        .route(
            "/api/worlds/{world}/assets",
            post(assets::upload)
                .get(assets::list)
                .layer(DefaultBodyLimit::disable()),
        )
        .route(
            "/api/assets/{uuid}",
            get(assets::serve).delete(assets::delete),
        )
        .route(
            "/api/assets/{uuid}/replace",
            post(assets::replace).layer(DefaultBodyLimit::disable()),
        )
        .fallback(embed::static_handler)
        .layer(sessions)
        .layer(
            // Last layer = outermost. Request id is stamped first, the trace span
            // wraps everything (including sessions and the gate), then the id is
            // propagated onto the response.
            ServiceBuilder::new()
                .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
                .layer(TraceLayer::new_for_http())
                .layer(PropagateRequestIdLayer::x_request_id()),
        )
        .with_state(state)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    pub(crate) async fn test_state() -> AppState {
        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        AppState {
            repo: Arc::new(repo),
            config: Arc::new(Config::default()),
            setup_token: None,
            initialized: Arc::new(AtomicBool::new(false)),
            ws: crate::ws::WsState::new(),
            upload_rate: Arc::new(assets::UploadRateLimiter::new()),
            auth_throttle: Arc::new(throttle::AuthThrottle::new()),
            write_barrier: Arc::new(tokio::sync::RwLock::new(())),
        }
    }

    /// A `test_state` with the init gate already open — for exercising
    /// normal (post-setup) routes without walking the first-run flow.
    pub(crate) async fn initialized_state() -> AppState {
        let state = test_state().await;
        state
            .initialized
            .store(true, std::sync::atomic::Ordering::Relaxed);
        state
    }

    use crate::auth::password::hash_password;
    use crate::auth::role::ServerRole;

    async fn server_with_user(
        username: &str,
        password: &str,
        role: ServerRole,
    ) -> axum_test::TestServer {
        let state = initialized_state().await;
        let hash = hash_password(password).unwrap();
        state
            .repo
            .create_user(username, Some(&hash), role, 0)
            .await
            .unwrap();
        axum_test::TestServer::builder()
            .save_cookies()
            .build(router(state).await)
            .unwrap()
    }

    async fn fresh_server() -> axum_test::TestServer {
        // Uninitialized state, open token window (loopback default).
        let state = test_state().await;
        axum_test::TestServer::builder()
            .save_cookies()
            .build(router(state).await)
            .unwrap()
    }

    #[tokio::test]
    async fn setup_creates_admin_then_closes() {
        let server = fresh_server().await;

        let setup = server
            .post("/api/setup")
            .json(&serde_json::json!({
                "username": "admin", "password": "pw-admin"
            }))
            .await;
        setup.assert_status(axum::http::StatusCode::NO_CONTENT);

        // Now initialized: a second setup is a conflict.
        server
            .post("/api/setup")
            .json(&serde_json::json!({
                "username": "x", "password": "y"
            }))
            .await
            .assert_status(axum::http::StatusCode::CONFLICT);

        // The created admin can log in.
        server
            .post("/api/login")
            .json(&serde_json::json!({
                "username": "admin", "password": "pw-admin"
            }))
            .await
            .assert_status(axum::http::StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn setup_requires_token_when_policy_demands_it() {
        let mut state = test_state().await;
        // Force a required token regardless of bind.
        let cfg = crate::config::Config {
            setup_token: "the-token".into(),
            ..crate::config::Config::default()
        };
        state.config = std::sync::Arc::new(cfg.clone());
        state.setup_token = AppState::resolve_setup_token(&cfg);
        let server = axum_test::TestServer::builder()
            .save_cookies()
            .build(router(state).await)
            .unwrap();

        server
            .post("/api/setup")
            .json(&serde_json::json!({
                "username": "admin", "password": "pw"
            }))
            .await
            .assert_status(axum::http::StatusCode::FORBIDDEN);

        server
            .post("/api/setup")
            .json(&serde_json::json!({
                "username": "admin", "password": "pw", "token": "the-token"
            }))
            .await
            .assert_status(axum::http::StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn login_success_then_me_then_logout() {
        let server = server_with_user("gm-1", "pw-correct", ServerRole::User).await;

        server
            .get("/api/me")
            .await
            .assert_status(axum::http::StatusCode::UNAUTHORIZED);

        let login = server
            .post("/api/login")
            .json(&serde_json::json!({
                "username": "gm-1", "password": "pw-correct"
            }))
            .await;
        login.assert_status(axum::http::StatusCode::NO_CONTENT);

        let me = server.get("/api/me").await;
        me.assert_status_ok();
        assert!(me.text().contains("gm-1"));

        server
            .post("/api/logout")
            .await
            .assert_status(axum::http::StatusCode::NO_CONTENT);
        server
            .get("/api/me")
            .await
            .assert_status(axum::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn list_worlds_returns_only_callers_worlds() {
        let state = initialized_state().await;
        seed_user(&state, "a").await;
        seed_user(&state, "b").await;
        let a = login_server(&state, "a").await;
        let b = login_server(&state, "b").await;

        // a creates world1 (GM); b creates world2 (GM).
        a.post("/api/worlds")
            .json(&serde_json::json!({ "name": "world1" }))
            .await
            .assert_status_ok();
        b.post("/api/worlds")
            .json(&serde_json::json!({ "name": "world2" }))
            .await
            .assert_status_ok();

        // a sees exactly world1, as gm.
        let worlds: serde_json::Value = a.get("/api/worlds").await.json();
        let arr = worlds.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["name"], "world1");
        assert_eq!(arr[0]["role"], "gm");
        assert!(arr[0]["id"].is_string());

        // a never sees b's world.
        assert!(!worlds.to_string().contains("world2"));
    }

    #[tokio::test]
    async fn config_reports_initialized_state_and_is_public_pre_init() {
        // Uninitialized: reachable (not redirected to setup) and reports false.
        let fresh = fresh_server().await;
        let res = fresh.get("/api/config").await;
        res.assert_status_ok();
        assert_eq!(res.json::<serde_json::Value>()["initialized"], false);

        // Initialized: reports true.
        let server = axum_test::TestServer::new(router(initialized_state().await).await).unwrap();
        let res = server.get("/api/config").await;
        res.assert_status_ok();
        assert_eq!(res.json::<serde_json::Value>()["initialized"], true);
    }

    #[tokio::test]
    async fn ui_state_get_put_round_trip_and_validation() {
        let state = initialized_state().await;
        seed_user(&state, "u").await;
        let u = login_server(&state, "u").await;

        // Unauthenticated GET is rejected.
        let anon = axum_test::TestServer::builder()
            .save_cookies()
            .build(router(state.clone()).await)
            .unwrap();
        anon.get("/api/me/ui-state")
            .await
            .assert_status(StatusCode::UNAUTHORIZED);

        // Default is an empty object.
        let got: serde_json::Value = u.get("/api/me/ui-state").await.json();
        assert_eq!(got, serde_json::json!({}));

        // Store an object, read it back.
        u.put("/api/me/ui-state")
            .json(&serde_json::json!({ "global": { "locale": "en" } }))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        let got: serde_json::Value = u.get("/api/me/ui-state").await.json();
        assert_eq!(got["global"]["locale"], "en");

        // A non-object body is rejected.
        u.put("/api/me/ui-state")
            .json(&serde_json::json!([1, 2, 3]))
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

        // An over-cap body is rejected.
        let big = "x".repeat(70 * 1024);
        u.put("/api/me/ui-state")
            .json(&serde_json::json!({ "blob": big }))
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn login_rejects_wrong_password_and_unknown_user_identically() {
        let server = server_with_user("gm-1", "pw-correct", ServerRole::User).await;

        let bad_pw = server
            .post("/api/login")
            .json(&serde_json::json!({
                "username": "gm-1", "password": "pw-wrong"
            }))
            .await;
        let unknown = server
            .post("/api/login")
            .json(&serde_json::json!({
                "username": "ghost", "password": "whatever"
            }))
            .await;

        bad_pw.assert_status(axum::http::StatusCode::UNAUTHORIZED);
        unknown.assert_status(axum::http::StatusCode::UNAUTHORIZED);
        assert_eq!(
            bad_pw.text(),
            unknown.text(),
            "no user enumeration via body"
        );
    }

    #[tokio::test]
    async fn login_throttles_identity_after_budget_spending_no_argon2() {
        use crate::auth::password::verify_count;
        use crate::http::throttle::LOGIN_PER_MIN_PER_IDENTITY;
        let server = server_with_user("gm-1", "pw-correct", ServerRole::User).await;

        // Unknown identity: exhaust the budget, then assert 429 + zero verifies.
        for _ in 0..LOGIN_PER_MIN_PER_IDENTITY {
            server
                .post("/api/login")
                .json(&serde_json::json!({ "username": "ghost", "password": "x" }))
                .await
                .assert_status(axum::http::StatusCode::UNAUTHORIZED);
        }
        let before = verify_count();
        let ghost_throttled = server
            .post("/api/login")
            .json(&serde_json::json!({ "username": "ghost", "password": "x" }))
            .await;
        ghost_throttled.assert_status(axum::http::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            verify_count() - before,
            0,
            "throttled attempt must spend no Argon2"
        );

        // Known identity: identical throttle shape (status AND body) — no oracle.
        for _ in 0..LOGIN_PER_MIN_PER_IDENTITY {
            server
                .post("/api/login")
                .json(&serde_json::json!({ "username": "gm-1", "password": "wrong" }))
                .await
                .assert_status(axum::http::StatusCode::UNAUTHORIZED);
        }
        let known_throttled = server
            .post("/api/login")
            .json(&serde_json::json!({ "username": "gm-1", "password": "wrong" }))
            .await;
        known_throttled.assert_status(axum::http::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            ghost_throttled.text(),
            known_throttled.text(),
            "uniform 429 body"
        );
    }

    /// A `TestServer` served over a REAL loopback TCP connection (not the
    /// default mock transport), so `throttle::ClientIp` resolves an actual
    /// `SocketAddr` via `into_make_service_with_connect_info` — the mock
    /// transport never populates `ConnectInfo`, which would leave the
    /// per-IP throttle branches silently untested.
    async fn real_transport_server(state: AppState) -> axum_test::TestServer {
        let app = router(state)
            .await
            .into_make_service_with_connect_info::<std::net::SocketAddr>();
        axum_test::TestServer::builder()
            .http_transport()
            .save_cookies()
            .build(app)
            .unwrap()
    }

    #[tokio::test]
    async fn login_throttles_by_ip_over_real_transport() {
        use crate::auth::password::verify_count;
        use crate::http::throttle::LOGIN_PER_MIN_PER_IP;
        let server = real_transport_server(initialized_state().await).await;

        // Every request uses a distinct, never-reused unknown identity, so
        // the per-identity budget (10/min) cannot possibly be what trips —
        // only the per-IP budget (30/min) can, since all requests share one
        // real loopback address.
        for i in 0..LOGIN_PER_MIN_PER_IP {
            server
                .post("/api/login")
                .json(&serde_json::json!({ "username": format!("ghost-{i}"), "password": "x" }))
                .await
                .assert_status(axum::http::StatusCode::UNAUTHORIZED);
        }
        let before = verify_count();
        server
            .post("/api/login")
            .json(&serde_json::json!({ "username": "ghost-final", "password": "x" }))
            .await
            .assert_status(axum::http::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            verify_count() - before,
            0,
            "IP-throttled attempt must spend no Argon2"
        );
    }

    #[tokio::test]
    async fn login_rejects_user_without_password_hash() {
        let state = initialized_state().await;
        // A credential-less user (e.g. an M2-era row) must never authenticate.
        state
            .repo
            .create_user("hashless", None, ServerRole::User, 0)
            .await
            .unwrap();
        let server = axum_test::TestServer::builder()
            .save_cookies()
            .build(router(state).await)
            .unwrap();
        server
            .post("/api/login")
            .json(&serde_json::json!({ "username": "hashless", "password": "anything" }))
            .await
            .assert_status(axum::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn headless_bootstrap_closes_setup_and_allows_login() {
        // Mirror main.rs: bootstrap seeds the admin, then the gate is open.
        let state = test_state().await;
        let cfg = crate::config::Config {
            admin_user: Some("ops".into()),
            admin_password: Some("pw-boot".into()),
            ..crate::config::Config::default()
        };
        assert!(crate::auth::setup::bootstrap_admin(&state.repo, &cfg)
            .await
            .unwrap());
        state
            .initialized
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let server = axum_test::TestServer::builder()
            .save_cookies()
            .build(router(state).await)
            .unwrap();

        // Setup window is closed.
        server
            .post("/api/setup")
            .json(&serde_json::json!({ "username": "x", "password": "y" }))
            .await
            .assert_status(axum::http::StatusCode::CONFLICT);
        // The bootstrapped admin can log in.
        server
            .post("/api/login")
            .json(&serde_json::json!({ "username": "ops", "password": "pw-boot" }))
            .await
            .assert_status(axum::http::StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn health_reports_db_connected() {
        let server = axum_test::TestServer::new(router(test_state().await).await).unwrap();
        let res = server.get("/health").await;
        res.assert_status_ok();
        let body: crate::health::HealthStatus = res.json();
        assert_eq!(body.status, "ok");
        assert!(body.db_connected);
    }

    #[tokio::test]
    async fn debug_rooms_requires_admin() {
        let server = server_with_user("u", "pw", ServerRole::User).await;
        server
            .post("/api/login")
            .json(&serde_json::json!({"username":"u","password":"pw"}))
            .await;
        server
            .get("/api/debug/rooms")
            .await
            .assert_status(axum::http::StatusCode::FORBIDDEN);
    }

    // --- M5: world/document CRUD + permission HTTP surface ---

    use axum::http::StatusCode;
    use uuid::Uuid;

    /// A TestServer over `state` with a logged-in session for `username`
    /// (password "pw"). Multiple servers share the same Arc-backed state, so
    /// they act as different users against one repository.
    async fn login_server(state: &AppState, username: &str) -> axum_test::TestServer {
        let server = axum_test::TestServer::builder()
            .save_cookies()
            .build(router(state.clone()).await)
            .unwrap();
        server
            .post("/api/login")
            .json(&serde_json::json!({ "username": username, "password": "pw" }))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        server
    }

    async fn seed_user(state: &AppState, username: &str) -> Uuid {
        let hash = hash_password("pw").unwrap();
        state
            .repo
            .create_user(username, Some(&hash), ServerRole::User, 0)
            .await
            .unwrap()
    }

    async fn seed_admin(state: &AppState, username: &str) -> Uuid {
        let hash = hash_password("pw").unwrap();
        state
            .repo
            .create_user(username, Some(&hash), ServerRole::Admin, 0)
            .await
            .unwrap()
    }

    /// An unauthenticated TestServer over `state` (no login performed).
    async fn anon_server(state: &AppState) -> axum_test::TestServer {
        axum_test::TestServer::builder()
            .save_cookies()
            .build(router(state.clone()).await)
            .unwrap()
    }

    // --- Account administration (`/api/users`) ---

    /// Seats a world-GM, a plain player, and an anonymous caller against one
    /// admin, so every authz assertion below shares one fixture.
    struct UserRoutesFixture {
        state: AppState,
        admin: axum_test::TestServer,
        gm: axum_test::TestServer,
        player: axum_test::TestServer,
        anon: axum_test::TestServer,
        world_id: String,
    }

    async fn user_routes_fixture() -> UserRoutesFixture {
        let state = initialized_state().await;
        seed_admin(&state, "root-admin").await;
        seed_user(&state, "world-gm").await;
        let player_id = seed_user(&state, "plain-player").await;
        let admin = login_server(&state, "root-admin").await;
        let gm = login_server(&state, "world-gm").await;
        let player = login_server(&state, "plain-player").await;
        let anon = anon_server(&state).await;

        // `world-gm` is a GM of a real world — the point of the matrix is that
        // world-tier authority never satisfies the server-tier admin gate.
        let world: serde_json::Value = gm
            .post("/api/worlds")
            .json(&serde_json::json!({ "name": "W" }))
            .await
            .json();
        let world_id = world["id"].as_str().unwrap().to_string();
        gm.post(&format!("/api/worlds/{world_id}/members"))
            .json(&serde_json::json!({ "user": player_id, "role": "player" }))
            .await
            .assert_status(StatusCode::NO_CONTENT);

        UserRoutesFixture {
            state,
            admin,
            gm,
            player,
            anon,
            world_id,
        }
    }

    #[tokio::test]
    async fn create_user_is_admin_only_and_never_returns_a_hash() {
        let f = user_routes_fixture().await;
        let body = serde_json::json!({ "username": "new-player", "password": "pw-new-player" });

        // A world GM, a plain player, and an anonymous caller are each rejected.
        f.gm.post("/api/users")
            .json(&body)
            .await
            .assert_status(StatusCode::FORBIDDEN);
        f.player
            .post("/api/users")
            .json(&body)
            .await
            .assert_status(StatusCode::FORBIDDEN);
        f.anon
            .post("/api/users")
            .json(&body)
            .await
            .assert_status(StatusCode::UNAUTHORIZED);

        // ...and none of them created anything.
        assert!(f
            .state
            .repo
            .user_by_username("new-player")
            .await
            .unwrap()
            .is_none());

        // The admin is allowed.
        let res = f.admin.post("/api/users").json(&body).await;
        res.assert_status_ok();
        let created: serde_json::Value = res.json();
        assert_eq!(created["username"], "new-player");
        assert_eq!(created["server_role"], "user");
        assert!(created["id"].is_string());

        // No credential material anywhere in the response.
        let text = res.text();
        assert!(
            !text.contains("password"),
            "response leaks a password field"
        );
        assert!(!text.contains("hash"), "response leaks a hash field");
        assert!(!text.contains("$argon2"), "response leaks a PHC hash");

        // The account is real: it can authenticate.
        anon_server(&f.state)
            .await
            .post("/api/login")
            .json(&serde_json::json!({
                "username": "new-player", "password": "pw-new-player"
            }))
            .await
            .assert_status(StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn list_users_is_admin_only() {
        let f = user_routes_fixture().await;

        f.gm.get("/api/users")
            .await
            .assert_status(StatusCode::FORBIDDEN);
        f.player
            .get("/api/users")
            .await
            .assert_status(StatusCode::FORBIDDEN);
        f.anon
            .get("/api/users")
            .await
            .assert_status(StatusCode::UNAUTHORIZED);

        let res = f.admin.get("/api/users").await;
        res.assert_status_ok();
        let users: Vec<serde_json::Value> = res.json();
        let names: Vec<&str> = users
            .iter()
            .map(|u| u["username"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"root-admin"));
        assert!(names.contains(&"world-gm"));
        assert!(names.contains(&"plain-player"));
        // The listing projects only the non-secret columns.
        assert!(!res.text().contains("$argon2"), "listing leaks a PHC hash");
        for u in &users {
            assert!(u.get("password_hash").is_none());
        }
    }

    #[tokio::test]
    async fn create_user_can_mint_an_admin_but_only_for_an_admin_caller() {
        let f = user_routes_fixture().await;
        let body = serde_json::json!({
            "username": "second-admin", "password": "pw-second-admin", "server_role": "admin"
        });

        // A GM cannot mint an admin (rejected at the extractor, before the body).
        f.gm.post("/api/users")
            .json(&body)
            .await
            .assert_status(StatusCode::FORBIDDEN);

        let created: serde_json::Value = f.admin.post("/api/users").json(&body).await.json();
        assert_eq!(created["server_role"], "admin");
        assert_eq!(
            f.state
                .repo
                .user_by_username("second-admin")
                .await
                .unwrap()
                .unwrap()
                .server_role,
            ServerRole::Admin
        );
    }

    #[tokio::test]
    async fn create_user_rejects_duplicate_usernames_case_insensitively() {
        let f = user_routes_fixture().await;
        f.admin
            .post("/api/users")
            .json(&serde_json::json!({ "username": "dup-user", "password": "pw-dup-user" }))
            .await
            .assert_status_ok();

        // Exact duplicate, and a case variant that would otherwise be able to
        // impersonate the first account in a roster — both a clean 409, never a
        // 500 from the unique constraint.
        for name in ["dup-user", "DUP-User"] {
            f.admin
                .post("/api/users")
                .json(&serde_json::json!({ "username": name, "password": "pw-other-user" }))
                .await
                .assert_status(StatusCode::CONFLICT);
        }
        // Colliding with an account created outside this route is also rejected.
        f.admin
            .post("/api/users")
            .json(&serde_json::json!({ "username": "World-GM", "password": "pw-other-user" }))
            .await
            .assert_status(StatusCode::CONFLICT);

        let all = f.state.repo.list_users().await.unwrap();
        assert_eq!(
            all.iter()
                .filter(|(_, n, _)| n.eq_ignore_ascii_case("dup-user"))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn create_user_enforces_the_username_and_password_policy() {
        let f = user_routes_fixture().await;
        let bad_names = [
            "ab",            // too short
            &"a".repeat(33), // too long
            "has space",     // whitespace inside
            "sla/sh",        // path-ish punctuation
            "adm\u{0131}n",  // non-ASCII homoglyph
            "",              // empty
        ];
        for name in bad_names {
            f.admin
                .post("/api/users")
                .json(&serde_json::json!({ "username": name, "password": "pw-valid-1" }))
                .await
                .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        }

        // Surrounding whitespace is normalized away, not rejected.
        let created: serde_json::Value = f
            .admin
            .post("/api/users")
            .json(&serde_json::json!({ "username": "  trimmed  ", "password": "pw-valid-1" }))
            .await
            .json();
        assert_eq!(created["username"], "trimmed");

        // Password floor and ceiling.
        f.admin
            .post("/api/users")
            .json(&serde_json::json!({ "username": "short-pw", "password": "1234567" }))
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        f.admin
            .post("/api/users")
            .json(&serde_json::json!({ "username": "long-pw", "password": "x".repeat(257) }))
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }

    // --- GM member-add (by user id only) ---

    #[tokio::test]
    async fn add_member_seats_by_user_id_and_refuses_to_name_an_account() {
        let f = user_routes_fixture().await;
        let world_id = &f.world_id;
        let seated_id = seed_user(&f.state, "seated").await;

        f.gm.post(&format!("/api/worlds/{world_id}/members"))
            .json(&serde_json::json!({ "user": seated_id, "role": "player" }))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        let members: Vec<serde_json::Value> =
            f.gm.get(&format!("/api/worlds/{world_id}/members"))
                .await
                .json();
        assert_eq!(
            members
                .iter()
                .find(|m| m["username"] == "seated")
                .expect("seated by id")["role"],
            "player"
        );

        // The same call is an idempotent role change, not a duplicate seat.
        f.gm.post(&format!("/api/worlds/{world_id}/members"))
            .json(&serde_json::json!({ "user": seated_id, "role": "spectator" }))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        let members: Vec<serde_json::Value> =
            f.gm.get(&format!("/api/worlds/{world_id}/members"))
                .await
                .json();
        assert_eq!(
            members.iter().filter(|m| m["username"] == "seated").count(),
            1
        );
        assert_eq!(
            members.iter().find(|m| m["username"] == "seated").unwrap()["role"],
            "spectator"
        );

        // The by-name form is GONE: naming an account made this route a
        // username-existence oracle (a hit seats the target, observable through
        // list_members), and any authenticated user can create a world to
        // become a GM. A username body is now simply an unparseable request,
        // and it seats nobody.
        let by_name =
            f.gm.post(&format!("/api/worlds/{world_id}/members"))
                .json(&serde_json::json!({ "username": "plain-player", "role": "player" }))
                .await;
        assert_eq!(by_name.status_code(), StatusCode::UNPROCESSABLE_ENTITY);
        // An unknown name is rejected identically to a known one — no oracle.
        let unknown_name =
            f.gm.post(&format!("/api/worlds/{world_id}/members"))
                .json(&serde_json::json!({ "username": "no-such-user", "role": "player" }))
                .await;
        assert_eq!(unknown_name.status_code(), by_name.status_code());

        // A non-GM member cannot seat anyone; nor can an anonymous caller.
        f.player
            .post(&format!("/api/worlds/{world_id}/members"))
            .json(&serde_json::json!({ "user": seated_id, "role": "gm" }))
            .await
            .assert_status(StatusCode::FORBIDDEN);
        f.anon
            .post(&format!("/api/worlds/{world_id}/members"))
            .json(&serde_json::json!({ "user": seated_id, "role": "gm" }))
            .await
            .assert_status(StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn add_member_cannot_escalate_a_server_role() {
        let f = user_routes_fixture().await;
        let world_id = &f.world_id;
        let victim_id = seed_user(&f.state, "victim").await;

        // `role` deserializes as WorldRole, a closed gm/player/spectator enum:
        // no server-tier token is representable on this path.
        for escalation in [
            serde_json::json!({ "user": victim_id, "role": "admin" }),
            serde_json::json!({ "user": victim_id, "role": "user" }),
            // A stray server_role field is not part of the request shape.
            serde_json::json!({ "user": victim_id, "role": "player", "server_role": "admin" }),
        ] {
            let res =
                f.gm.post(&format!("/api/worlds/{world_id}/members"))
                    .json(&escalation)
                    .await;
            assert_ne!(
                res.status_code(),
                StatusCode::INTERNAL_SERVER_ERROR,
                "escalation attempt must not fault the server"
            );
            // Whatever the outcome, the account's SERVER role is unchanged.
            assert_eq!(
                f.state
                    .repo
                    .user_by_username("victim")
                    .await
                    .unwrap()
                    .unwrap()
                    .server_role,
                ServerRole::User,
                "world-membership write must never touch the server tier"
            );
        }

        // Even the maximal legitimate outcome — world GM — leaves the server
        // tier alone: the seated account still cannot reach an admin route.
        f.gm.post(&format!("/api/worlds/{world_id}/members"))
            .json(&serde_json::json!({ "user": victim_id, "role": "gm" }))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        assert_eq!(
            f.state
                .repo
                .user_by_username("victim")
                .await
                .unwrap()
                .unwrap()
                .server_role,
            ServerRole::User
        );
        let victim = login_server(&f.state, "victim").await;
        victim
            .get("/api/users")
            .await
            .assert_status(StatusCode::FORBIDDEN);
        victim
            .post("/api/users")
            .json(&serde_json::json!({ "username": "minted", "password": "pw-minted" }))
            .await
            .assert_status(StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn add_member_rejects_unknown_and_absent_targets() {
        let f = user_routes_fixture().await;
        let world_id = &f.world_id;

        // An unknown uuid would otherwise trip the world_members foreign key
        // and surface as a 500.
        f.gm.post(&format!("/api/worlds/{world_id}/members"))
            .json(&serde_json::json!({ "user": Uuid::from_u128(4242), "role": "player" }))
            .await
            .assert_status(StatusCode::NOT_FOUND);

        // A body with no target at all is unparseable.
        f.gm.post(&format!("/api/worlds/{world_id}/members"))
            .json(&serde_json::json!({ "role": "player" }))
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }

    // --- World invites (mint / list / revoke / redeem) ---

    /// A GM with a world, a second GM with their OWN world, an outsider with
    /// no world, and an anonymous caller.
    struct InviteFixture {
        state: AppState,
        gm: axum_test::TestServer,
        other_gm: axum_test::TestServer,
        outsider: axum_test::TestServer,
        anon: axum_test::TestServer,
        world_id: String,
        other_world_id: String,
    }

    async fn invite_fixture() -> InviteFixture {
        let state = initialized_state().await;
        seed_user(&state, "gm-alpha").await;
        seed_user(&state, "gm-beta").await;
        seed_user(&state, "outsider").await;
        let gm = login_server(&state, "gm-alpha").await;
        let other_gm = login_server(&state, "gm-beta").await;
        let outsider = login_server(&state, "outsider").await;
        let anon = anon_server(&state).await;

        let world: serde_json::Value = gm
            .post("/api/worlds")
            .json(&serde_json::json!({ "name": "Alpha" }))
            .await
            .json();
        let other: serde_json::Value = other_gm
            .post("/api/worlds")
            .json(&serde_json::json!({ "name": "Beta" }))
            .await
            .json();

        InviteFixture {
            state,
            gm,
            other_gm,
            outsider,
            anon,
            world_id: world["id"].as_str().unwrap().to_string(),
            other_world_id: other["id"].as_str().unwrap().to_string(),
        }
    }

    /// Mint an invite for `world` as `gm`, returning `(code_id, code)`.
    async fn mint_invite(gm: &axum_test::TestServer, world: &str, role: &str) -> (String, String) {
        let res = gm
            .post(&format!("/api/worlds/{world}/invites"))
            .json(&serde_json::json!({ "role": role }))
            .await;
        res.assert_status_ok();
        let body: serde_json::Value = res.json();
        (
            body["id"].as_str().unwrap().to_string(),
            body["code"].as_str().unwrap().to_string(),
        )
    }

    #[tokio::test]
    async fn accept_invite_throttles_per_account_spending_no_argon2() {
        use crate::auth::password::verify_count;
        use crate::http::throttle::INVITE_PER_MIN_PER_ACCOUNT;
        let f = invite_fixture().await;
        for _ in 0..INVITE_PER_MIN_PER_ACCOUNT {
            f.other_gm
                .post("/api/invites/accept")
                .json(&serde_json::json!({ "code": "not-a-real-code" }))
                .await
                .assert_status(axum::http::StatusCode::NOT_FOUND);
        }
        let before = verify_count();
        let throttled = f
            .other_gm
            .post("/api/invites/accept")
            .json(&serde_json::json!({ "code": "not-a-real-code" }))
            .await;
        throttled.assert_status(axum::http::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(verify_count() - before, 0);
    }

    /// Like `login_server`, but served over a REAL loopback TCP connection
    /// (`real_transport_server`) so `throttle::ClientIp` resolves an actual
    /// address instead of `None`.
    async fn real_login_server(state: &AppState, username: &str) -> axum_test::TestServer {
        let server = real_transport_server(state.clone()).await;
        server
            .post("/api/login")
            .json(&serde_json::json!({ "username": username, "password": "pw" }))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        server
    }

    #[tokio::test]
    async fn accept_invite_throttles_by_ip_over_real_transport() {
        use crate::auth::password::verify_count;
        use crate::http::throttle::INVITE_PER_MIN_PER_ACCOUNT;
        let state = initialized_state().await;
        for name in ["acct-a", "acct-b", "acct-c", "acct-d"] {
            seed_user(&state, name).await;
        }
        // Three accounts each spend their FULL per-account budget (10) over
        // one shared loopback IP, totalling exactly the per-IP budget (30) —
        // no single account's own check can explain a 429 here.
        for name in ["acct-a", "acct-b", "acct-c"] {
            let server = real_login_server(&state, name).await;
            for _ in 0..INVITE_PER_MIN_PER_ACCOUNT {
                server
                    .post("/api/invites/accept")
                    .json(&serde_json::json!({ "code": "not-a-real-code" }))
                    .await
                    .assert_status(axum::http::StatusCode::NOT_FOUND);
            }
        }
        // A brand-new account (zero prior attempts), same loopback IP — only
        // the shared per-IP key can explain the 429 below.
        let server = real_login_server(&state, "acct-d").await;
        let before = verify_count();
        server
            .post("/api/invites/accept")
            .json(&serde_json::json!({ "code": "not-a-real-code" }))
            .await
            .assert_status(axum::http::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            verify_count() - before,
            0,
            "IP-throttled attempt must spend no Argon2"
        );
    }

    #[tokio::test]
    async fn minting_listing_and_revoking_require_gm_of_that_world() {
        let f = invite_fixture().await;
        let w = &f.world_id;
        let mint = serde_json::json!({ "role": "player" });

        // The world's own GM may mint.
        let (code_id, _) = mint_invite(&f.gm, w, "player").await;

        // A GM of ANOTHER world is a non-member here: mint, list, and revoke
        // are each refused, and the refusal is the same as for any outsider.
        for caller in [&f.other_gm, &f.outsider] {
            caller
                .post(&format!("/api/worlds/{w}/invites"))
                .json(&mint)
                .await
                .assert_status(StatusCode::FORBIDDEN);
            caller
                .get(&format!("/api/worlds/{w}/invites"))
                .await
                .assert_status(StatusCode::FORBIDDEN);
            caller
                .delete(&format!("/api/worlds/{w}/invites/{code_id}"))
                .await
                .assert_status(StatusCode::FORBIDDEN);
        }
        // An anonymous caller is unauthenticated on every one.
        f.anon
            .post(&format!("/api/worlds/{w}/invites"))
            .json(&mint)
            .await
            .assert_status(StatusCode::UNAUTHORIZED);
        f.anon
            .get(&format!("/api/worlds/{w}/invites"))
            .await
            .assert_status(StatusCode::UNAUTHORIZED);
        f.anon
            .delete(&format!("/api/worlds/{w}/invites/{code_id}"))
            .await
            .assert_status(StatusCode::UNAUTHORIZED);

        // A seated PLAYER of this world is still not a GM.
        let player_id = seed_user(&f.state, "seated-player").await;
        let player = login_server(&f.state, "seated-player").await;
        f.gm.post(&format!("/api/worlds/{w}/members"))
            .json(&serde_json::json!({ "user": player_id, "role": "player" }))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        player
            .post(&format!("/api/worlds/{w}/invites"))
            .json(&mint)
            .await
            .assert_status(StatusCode::FORBIDDEN);
        player
            .get(&format!("/api/worlds/{w}/invites"))
            .await
            .assert_status(StatusCode::FORBIDDEN);
        player
            .delete(&format!("/api/worlds/{w}/invites/{code_id}"))
            .await
            .assert_status(StatusCode::FORBIDDEN);

        // The invite survived every rejected revoke: it still redeems.
        let listing: Vec<serde_json::Value> =
            f.gm.get(&format!("/api/worlds/{w}/invites")).await.json();
        assert_eq!(listing.len(), 1);
        assert!(listing[0]["revoked_at"].is_null());
    }

    #[tokio::test]
    async fn a_gm_of_one_world_cannot_revoke_another_worlds_invite() {
        let f = invite_fixture().await;
        let (code_id, code) = mint_invite(&f.gm, &f.world_id, "player").await;

        // gm-beta is the GM of their OWN world, so `require_gm` passes for that
        // world — the invite must still be untouchable, because the revoke is
        // scoped to the world in SQL.
        let other = &f.other_world_id;
        f.other_gm
            .delete(&format!("/api/worlds/{other}/invites/{code_id}"))
            .await
            .assert_status(StatusCode::NOT_FOUND);

        // Indistinguishable from an id that exists nowhere.
        let ghost = Uuid::new_v4();
        f.other_gm
            .delete(&format!("/api/worlds/{other}/invites/{ghost}"))
            .await
            .assert_status(StatusCode::NOT_FOUND);

        // ...and the invite still works.
        f.outsider
            .post("/api/invites/accept")
            .json(&serde_json::json!({ "code": code }))
            .await
            .assert_status_ok();
    }

    #[tokio::test]
    async fn redeeming_an_invite_seats_the_caller_at_the_invited_role() {
        let f = invite_fixture().await;
        let (_, code) = mint_invite(&f.gm, &f.world_id, "spectator").await;

        // Before redeeming, the outsider sees no worlds and is refused the roster.
        let worlds: Vec<serde_json::Value> = f.outsider.get("/api/worlds").await.json();
        assert!(worlds.is_empty());
        f.outsider
            .get(&format!("/api/worlds/{}/members", f.world_id))
            .await
            .assert_status(StatusCode::FORBIDDEN);

        let res = f
            .outsider
            .post("/api/invites/accept")
            .json(&serde_json::json!({ "code": code }))
            .await;
        res.assert_status_ok();
        let entry: serde_json::Value = res.json();
        assert_eq!(entry["id"], f.world_id);
        assert_eq!(entry["name"], "Alpha");
        assert_eq!(entry["role"], "spectator");

        // The seat is real and at the invited role.
        let members: Vec<serde_json::Value> =
            f.gm.get(&format!("/api/worlds/{}/members", f.world_id))
                .await
                .json();
        let seated = members
            .iter()
            .find(|m| m["username"] == "outsider")
            .expect("redeemer seated");
        assert_eq!(seated["role"], "spectator");
    }

    #[tokio::test]
    async fn every_unusable_code_fails_identically() {
        let f = invite_fixture().await;
        let w = &f.world_id;

        // Baseline: a well-formed code for an invite that never existed.
        let unknown = format!("{}.{}", Uuid::new_v4().simple(), "ab".repeat(16));

        // Already consumed.
        let (_, consumed) = mint_invite(&f.gm, w, "player").await;
        f.outsider
            .post("/api/invites/accept")
            .json(&serde_json::json!({ "code": consumed }))
            .await
            .assert_status_ok();

        // Revoked.
        let (revoked_id, revoked) = mint_invite(&f.gm, w, "player").await;
        f.gm.delete(&format!("/api/worlds/{w}/invites/{revoked_id}"))
            .await
            .assert_status(StatusCode::NO_CONTENT);

        // Expired: written directly with a past expiry (no clock to wind).
        let expired_mint = crate::auth::invite::mint().unwrap();
        let gm_id = f
            .state
            .repo
            .user_by_username("gm-alpha")
            .await
            .unwrap()
            .unwrap()
            .id;
        assert!(f
            .state
            .repo
            .create_invite(
                crate::data::sqlite::NewInvite {
                    id: expired_mint.id,
                    world: Uuid::parse_str(w).unwrap(),
                    secret_hash: &expired_mint.secret_hash,
                    role: crate::data::document::WorldRole::Player,
                    created_by: gm_id,
                    now: 1,
                    expires_at: 2,
                },
                64,
            )
            .await
            .unwrap());
        let expired = expired_mint.code.clone();

        // Right selector, wrong secret.
        let (live_id, _) = mint_invite(&f.gm, w, "player").await;
        let wrong_secret = format!("{}.{}", live_id.replace('-', ""), "cd".repeat(16));

        let malformed = "not-a-code";
        let empty_secret = format!("{}.", Uuid::new_v4().simple());

        // The response shape is only half the property. The other half is the
        // WORK done: a plain uniform 404 that skipped the verify when no row
        // matched would satisfy the (status, body) assertions below while
        // reinstating the timing oracle this whole design exists to remove. So
        // each request also asserts EXACTLY ONE Argon2 verify — a counter, not
        // wall-clock timing, which is not portable across the CI matrix.
        use crate::auth::password::verify_count;

        let mut seen: Vec<(StatusCode, String)> = Vec::new();
        for (i, code) in [
            unknown.as_str(),
            consumed.as_str(),
            revoked.as_str(),
            expired.as_str(),
            wrong_secret.as_str(),
            malformed,
            empty_secret.as_str(),
        ]
        .into_iter()
        .enumerate()
        {
            let before = verify_count();
            let res = f
                .other_gm
                .post("/api/invites/accept")
                .json(&serde_json::json!({ "code": code }))
                .await;
            assert_eq!(
                verify_count() - before,
                1,
                "failure shape {i} did not cost exactly one verify"
            );
            seen.push((res.status_code(), res.text()));
        }
        let first = seen[0].clone();
        for (i, got) in seen.iter().enumerate() {
            assert_eq!(
                *got, first,
                "redemption failure {i} is distinguishable from the baseline"
            );
        }
        assert_eq!(first.0, StatusCode::NOT_FOUND);

        // ...and the SUCCESS path costs the same one verify, so success and
        // failure are not separable by work either.
        let (_, good) = mint_invite(&f.gm, w, "player").await;
        let before = verify_count();
        f.outsider
            .post("/api/invites/accept")
            .json(&serde_json::json!({ "code": good }))
            .await
            .assert_status_ok();
        assert_eq!(
            verify_count() - before,
            1,
            "the success path did not cost exactly one verify"
        );
        // Nothing about the world behind an unusable code is disclosed, and the
        // caller is not seated anywhere by a failed redemption.
        assert!(!first.1.contains("Alpha"));
        assert!(!first.1.contains(w.as_str()));
        let worlds: Vec<serde_json::Value> = f.other_gm.get("/api/worlds").await.json();
        assert_eq!(worlds.len(), 1, "only gm-beta's own world");
        assert_eq!(worlds[0]["name"], "Beta");
    }

    #[tokio::test]
    async fn a_re_typed_code_redeems_regardless_of_hex_case() {
        let f = invite_fixture().await;
        let (_, code) = mint_invite(&f.gm, &f.world_id, "player").await;
        // Hex case carries no information, so an auto-capitalized or re-typed
        // code must not fall into the (undiagnosable) uniform-failure bucket.
        f.outsider
            .post("/api/invites/accept")
            .json(&serde_json::json!({ "code": code.to_uppercase() }))
            .await
            .assert_status_ok();
    }

    #[tokio::test]
    async fn an_invite_is_single_use_even_for_a_different_caller() {
        let f = invite_fixture().await;
        let (_, code) = mint_invite(&f.gm, &f.world_id, "player").await;

        f.outsider
            .post("/api/invites/accept")
            .json(&serde_json::json!({ "code": code }))
            .await
            .assert_status_ok();
        // The same code, presented by anyone, is spent.
        f.outsider
            .post("/api/invites/accept")
            .json(&serde_json::json!({ "code": code }))
            .await
            .assert_status(StatusCode::NOT_FOUND);
        f.other_gm
            .post("/api/invites/accept")
            .json(&serde_json::json!({ "code": code }))
            .await
            .assert_status(StatusCode::NOT_FOUND);

        // gm-beta was never seated by the refused redemption.
        let members: Vec<serde_json::Value> =
            f.gm.get(&format!("/api/worlds/{}/members", f.world_id))
                .await
                .json();
        assert!(!members.iter().any(|m| m["username"] == "gm-beta"));
    }

    #[tokio::test]
    async fn revocation_takes_effect_immediately() {
        let f = invite_fixture().await;
        let w = &f.world_id;
        let (code_id, code) = mint_invite(&f.gm, w, "player").await;

        f.gm.delete(&format!("/api/worlds/{w}/invites/{code_id}"))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        f.outsider
            .post("/api/invites/accept")
            .json(&serde_json::json!({ "code": code }))
            .await
            .assert_status(StatusCode::NOT_FOUND);
        assert!(f
            .state
            .repo
            .member_role(
                Uuid::parse_str(w).unwrap(),
                f.state
                    .repo
                    .user_by_username("outsider")
                    .await
                    .unwrap()
                    .unwrap()
                    .id
            )
            .await
            .unwrap()
            .is_none());

        // Revoking twice is not a second success.
        f.gm.delete(&format!("/api/worlds/{w}/invites/{code_id}"))
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn an_invite_cannot_carry_a_server_role() {
        let f = invite_fixture().await;
        let w = &f.world_id;

        // `role` is a WorldRole: a server tier is not expressible in the body.
        for bad in ["admin", "user"] {
            f.gm.post(&format!("/api/worlds/{w}/invites"))
                .json(&serde_json::json!({ "role": bad }))
                .await
                .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        }
        // A stray server_role field is not part of the request shape.
        let res =
            f.gm.post(&format!("/api/worlds/{w}/invites"))
                .json(&serde_json::json!({ "role": "gm", "server_role": "admin" }))
                .await;
        res.assert_status_ok();
        let code = res.json::<serde_json::Value>()["code"]
            .as_str()
            .unwrap()
            .to_string();

        // Redeeming the maximal invite (world GM) leaves the server tier alone.
        f.outsider
            .post("/api/invites/accept")
            .json(&serde_json::json!({ "code": code }))
            .await
            .assert_status_ok();
        assert_eq!(
            f.state
                .repo
                .user_by_username("outsider")
                .await
                .unwrap()
                .unwrap()
                .server_role,
            ServerRole::User
        );
        f.outsider
            .get("/api/users")
            .await
            .assert_status(StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn redeeming_never_demotes_an_existing_membership() {
        let f = invite_fixture().await;
        let w = &f.world_id;
        // The world's own GM redeems a spectator invite for their own world.
        let (_, code) = mint_invite(&f.gm, w, "spectator").await;
        f.gm.post("/api/invites/accept")
            .json(&serde_json::json!({ "code": code }))
            .await
            .assert_status_ok();

        let members: Vec<serde_json::Value> =
            f.gm.get(&format!("/api/worlds/{w}/members")).await.json();
        assert_eq!(
            members
                .iter()
                .find(|m| m["username"] == "gm-alpha")
                .unwrap()["role"],
            "gm",
            "an invite may grant access, never change a role already held"
        );
    }

    #[tokio::test]
    async fn the_listing_never_exposes_a_code_or_its_hash() {
        let f = invite_fixture().await;
        let w = &f.world_id;
        let (code_id, code) = mint_invite(&f.gm, w, "player").await;
        let secret = code.split_once('.').unwrap().1;

        let res = f.gm.get(&format!("/api/worlds/{w}/invites")).await;
        res.assert_status_ok();
        let text = res.text();
        assert!(!text.contains(secret), "listing leaks the invite secret");
        assert!(!text.contains("$argon2"), "listing leaks a PHC hash");
        assert!(!text.contains("secret"), "listing exposes a secret field");

        let entries: Vec<serde_json::Value> = res.json();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["id"], code_id);
        assert_eq!(entries[0]["role"], "player");
        assert!(entries[0]["consumed_at"].is_null());
        assert!(entries[0].get("code").is_none());

        // After redemption the listing reports it consumed, still without a code.
        f.outsider
            .post("/api/invites/accept")
            .json(&serde_json::json!({ "code": code }))
            .await
            .assert_status_ok();
        let entries: Vec<serde_json::Value> =
            f.gm.get(&format!("/api/worlds/{w}/invites")).await.json();
        assert!(entries[0]["consumed_at"].is_number());
        assert!(!f
            .gm
            .get(&format!("/api/worlds/{w}/invites"))
            .await
            .text()
            .contains(secret));
    }

    #[tokio::test]
    async fn active_invites_per_world_are_capped() {
        let f = invite_fixture().await;
        let w = &f.world_id;
        let world = Uuid::parse_str(w).unwrap();
        let gm_id = f
            .state
            .repo
            .user_by_username("gm-alpha")
            .await
            .unwrap()
            .unwrap()
            .id;
        // Seed the cap through the repository with a fixed dummy hash: minting
        // 64 codes over HTTP would run 64 real Argon2id hashes (~a minute of
        // CPU) to set up a check that is about the COUNT, not the KDF.
        for _ in 0..64 {
            assert!(f
                .state
                .repo
                .create_invite(
                    crate::data::sqlite::NewInvite {
                        id: Uuid::new_v4(),
                        world,
                        secret_hash: "phc",
                        role: crate::data::document::WorldRole::Player,
                        created_by: gm_id,
                        now: 1,
                        expires_at: i64::MAX,
                    },
                    64,
                )
                .await
                .unwrap());
        }
        let over =
            f.gm.post(&format!("/api/worlds/{w}/invites"))
                .json(&serde_json::json!({ "role": "player" }))
                .await;
        over.assert_status(StatusCode::CONFLICT);

        // Revoking one frees a slot; the cap counts LIVE invites only.
        let listing: Vec<serde_json::Value> =
            f.gm.get(&format!("/api/worlds/{w}/invites")).await.json();
        let victim = listing[0]["id"].as_str().unwrap();
        f.gm.delete(&format!("/api/worlds/{w}/invites/{victim}"))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        f.gm.post(&format!("/api/worlds/{w}/invites"))
            .json(&serde_json::json!({ "role": "player" }))
            .await
            .assert_status_ok();
    }

    #[tokio::test]
    async fn accepting_requires_authentication() {
        let f = invite_fixture().await;
        let (_, code) = mint_invite(&f.gm, &f.world_id, "player").await;
        f.anon
            .post("/api/invites/accept")
            .json(&serde_json::json!({ "code": code }))
            .await
            .assert_status(StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn setup_and_bootstrap_enforce_the_ascii_username_policy() {
        // `/api/setup` is an insertion path into `users` that bypassed
        // `validate_username`: a non-ASCII first admin is not case-folded by
        // SQLite's ASCII-only NOCASE, so a homoglyph account could not collide
        // with it and would be indistinguishable in a roster.
        for bad in ["\u{0430}dmin", "ab", "has space", &"a".repeat(33)] {
            let server = fresh_server().await;
            server
                .post("/api/setup")
                .json(&serde_json::json!({ "username": bad, "password": "pw-admin" }))
                .await
                .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        }

        // Surrounding whitespace is normalized, matching `/api/users`.
        let server = fresh_server().await;
        server
            .post("/api/setup")
            .json(&serde_json::json!({ "username": "  ops-admin  ", "password": "pw-admin" }))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        server
            .post("/api/login")
            .json(&serde_json::json!({ "username": "ops-admin", "password": "pw-admin" }))
            .await
            .assert_status(StatusCode::NO_CONTENT);

        // The headless bootstrap path is validated too, and a misconfigured
        // username fails startup rather than silently seeding no admin.
        let state = test_state().await;
        let cfg = crate::config::Config {
            admin_user: Some("\u{0430}dmin".into()),
            admin_password: Some("pw-boot".into()),
            ..crate::config::Config::default()
        };
        assert!(crate::auth::setup::bootstrap_admin(&state.repo, &cfg)
            .await
            .is_err());
        assert!(!state.repo.admin_exists().await.unwrap());
    }

    fn doc_json(
        id: Uuid,
        world: &str,
        system: serde_json::Value,
        permissions: serde_json::Value,
    ) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "scope": { "kind": "world", "world_id": world },
            "doc_type": "actor",
            "schema_version": 1,
            "permissions": permissions,
            // "actor" is engine-defined; a minimal valid body so `Create`
            // clears the ingress gate. `system` above is what these HTTP
            // tests actually exercise (untouched).
            "engine": {
                "displayName": "Test",
                "visual": { "kind": "image", "asset": "a.png" },
                "size": { "w": 1.0, "h": 1.0 },
                "shape": "square",
                "faction": null,
                "conditions": [],
                "prototype": true
            },
            "system": system,
            "created_at": 0,
            "updated_at": 0,
        })
    }

    fn gm_only_perms() -> serde_json::Value {
        serde_json::json!({ "default": "none", "users": {}, "property_overrides": {} })
    }

    #[tokio::test]
    async fn world_membership_and_document_authorization() {
        let state = initialized_state().await;
        seed_user(&state, "gm").await;
        let player_id = seed_user(&state, "pl").await;
        let stranger_id = seed_user(&state, "st").await;

        let gm = login_server(&state, "gm").await;
        let pl = login_server(&state, "pl").await;
        let st = login_server(&state, "st").await;

        // GM creates a world (becomes its GM).
        let world: serde_json::Value = gm
            .post("/api/worlds")
            .json(&serde_json::json!({ "name": "W" }))
            .await
            .json();
        let world_id = world["id"].as_str().unwrap().to_string();

        let doc_id = Uuid::from_u128(10);
        let doc = doc_json(
            doc_id,
            &world_id,
            serde_json::json!({ "hp": 1 }),
            gm_only_perms(),
        );

        // Non-member cannot create a document in the world.
        st.post(&format!("/api/worlds/{world_id}/documents"))
            .json(&doc)
            .await
            .assert_status(StatusCode::FORBIDDEN);

        // GM creates it.
        gm.post(&format!("/api/worlds/{world_id}/documents"))
            .json(&doc)
            .await
            .assert_status_ok();

        // GM adds the player as a member.
        gm.post(&format!("/api/worlds/{world_id}/members"))
            .json(&serde_json::json!({ "user": player_id, "role": "player" }))
            .await
            .assert_status(StatusCode::NO_CONTENT);

        // The player cannot write a GM-only document.
        pl.patch(&format!("/api/documents/{doc_id}"))
            .json(&serde_json::json!({ "changes": [
                { "path": "/system/hp", "old": 1, "new": 9 }
            ]}))
            .await
            .assert_status(StatusCode::FORBIDDEN);

        // A non-GM cannot manage membership.
        pl.post(&format!("/api/worlds/{world_id}/members"))
            .json(&serde_json::json!({ "user": stranger_id, "role": "player" }))
            .await
            .assert_status(StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn list_members_is_visible_to_every_member_but_not_outsiders() {
        let state = initialized_state().await;
        seed_user(&state, "gm").await;
        let player_id = seed_user(&state, "pl").await;
        let spectator_id = seed_user(&state, "sp").await;
        seed_user(&state, "outsider").await;
        let gm = login_server(&state, "gm").await;
        let pl = login_server(&state, "pl").await;
        let sp = login_server(&state, "sp").await;
        let outsider = login_server(&state, "outsider").await;

        let world: serde_json::Value = gm
            .post("/api/worlds")
            .json(&serde_json::json!({ "name": "W" }))
            .await
            .json();
        let world_id = world["id"].as_str().unwrap().to_string();
        gm.post(&format!("/api/worlds/{world_id}/members"))
            .json(&serde_json::json!({ "user": player_id, "role": "player" }))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        gm.post(&format!("/api/worlds/{world_id}/members"))
            .json(&serde_json::json!({ "user": spectator_id, "role": "spectator" }))
            .await
            .assert_status(StatusCode::NO_CONTENT);

        // GM sees members with usernames.
        let members: serde_json::Value = gm
            .get(&format!("/api/worlds/{world_id}/members"))
            .await
            .json();
        let arr = members.as_array().unwrap();
        assert!(arr.iter().any(|m| m["username"] == "gm"));
        assert!(arr.iter().any(|m| m["username"] == "pl"));

        // A non-GM member also sees the roster: the chat card resolves user
        // ids to usernames for every viewer (author names, whisper recipient
        // labels), not just the GM's see-as labels.
        let members: serde_json::Value = pl
            .get(&format!("/api/worlds/{world_id}/members"))
            .await
            .json();
        let arr = members.as_array().unwrap();
        assert!(arr.iter().any(|m| m["username"] == "gm"));
        assert!(arr.iter().any(|m| m["username"] == "pl"));

        // A spectator sees the roster too.
        sp.get(&format!("/api/worlds/{world_id}/members"))
            .await
            .assert_status(StatusCode::OK);

        // An authenticated non-member is forbidden: the world id is
        // caller-supplied, so a membership denial here leaks nothing (unlike
        // the by-id document routes, which remap to 404 for existence-hiding).
        outsider
            .get(&format!("/api/worlds/{world_id}/members"))
            .await
            .assert_status(StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn by_id_routes_hide_existence_from_non_members() {
        let state = initialized_state().await;
        seed_user(&state, "gm").await;
        seed_user(&state, "st").await;
        let gm = login_server(&state, "gm").await;
        let st = login_server(&state, "st").await;

        let world: serde_json::Value = gm
            .post("/api/worlds")
            .json(&serde_json::json!({ "name": "W" }))
            .await
            .json();
        let world_id = world["id"].as_str().unwrap().to_string();

        let doc_id = Uuid::from_u128(321);
        let doc = doc_json(
            doc_id,
            &world_id,
            serde_json::json!({ "hp": 1 }),
            gm_only_perms(),
        );
        gm.post(&format!("/api/worlds/{world_id}/documents"))
            .json(&doc)
            .await
            .assert_status_ok();

        // A non-member must not distinguish "exists but forbidden" (403) from
        // "nonexistent" (404): every by-id document route returns 404.
        st.get(&format!("/api/documents/{doc_id}"))
            .await
            .assert_status(StatusCode::NOT_FOUND);
        st.patch(&format!("/api/documents/{doc_id}"))
            .json(&serde_json::json!({ "changes": [] }))
            .await
            .assert_status(StatusCode::NOT_FOUND);
        st.delete(&format!("/api/documents/{doc_id}"))
            .await
            .assert_status(StatusCode::NOT_FOUND);

        // World-scoped routes still return 403 to a non-member: the world id is
        // supplied by the caller, so a membership denial leaks nothing.
        st.get(&format!("/api/worlds/{world_id}/documents?type=actor"))
            .await
            .assert_status(StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn conflicting_patch_returns_conflict() {
        let state = initialized_state().await;
        seed_user(&state, "gm").await;
        let gm = login_server(&state, "gm").await;
        let world: serde_json::Value = gm
            .post("/api/worlds")
            .json(&serde_json::json!({ "name": "W" }))
            .await
            .json();
        let world_id = world["id"].as_str().unwrap().to_string();

        let doc_id = Uuid::from_u128(42);
        let doc = doc_json(
            doc_id,
            &world_id,
            serde_json::json!({ "hp": 10 }),
            gm_only_perms(),
        );
        gm.post(&format!("/api/worlds/{world_id}/documents"))
            .json(&doc)
            .await
            .assert_status_ok();

        // First write commits (hp 10 -> 5).
        gm.patch(&format!("/api/documents/{doc_id}"))
            .json(&serde_json::json!({ "changes": [
                { "path": "/system/hp", "old": 10, "new": 5 }
            ]}))
            .await
            .assert_status_ok();
        // Stale pre-image (current is 5) -> 409.
        gm.patch(&format!("/api/documents/{doc_id}"))
            .json(&serde_json::json!({ "changes": [
                { "path": "/system/hp", "old": 10, "new": 7 }
            ]}))
            .await
            .assert_status(StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn get_document_strips_gm_only_for_player() {
        let state = initialized_state().await;
        seed_user(&state, "gm").await;
        let player_id = seed_user(&state, "pl").await;
        let gm = login_server(&state, "gm").await;
        let pl = login_server(&state, "pl").await;

        let world: serde_json::Value = gm
            .post("/api/worlds")
            .json(&serde_json::json!({ "name": "W" }))
            .await
            .json();
        let world_id = world["id"].as_str().unwrap().to_string();
        gm.post(&format!("/api/worlds/{world_id}/members"))
            .json(&serde_json::json!({ "user": player_id, "role": "player" }))
            .await
            .assert_status(StatusCode::NO_CONTENT);

        let doc_id = Uuid::from_u128(99);
        let perms = serde_json::json!({
            "default": "observer",
            "users": {},
            "property_overrides": { "/system/secret": "gm_only" }
        });
        let doc = doc_json(
            doc_id,
            &world_id,
            serde_json::json!({ "secret": 42, "public": 7 }),
            perms,
        );
        gm.post(&format!("/api/worlds/{world_id}/documents"))
            .json(&doc)
            .await
            .assert_status_ok();

        let got: serde_json::Value = pl.get(&format!("/api/documents/{doc_id}")).await.json();
        assert_eq!(got["system"]["public"], 7);
        assert!(
            got["system"].get("secret").is_none(),
            "GM-only property must be stripped for the player"
        );
    }

    #[tokio::test]
    async fn world_capability_defaults_enable_owner_embedded() {
        let state = initialized_state().await;
        seed_user(&state, "gm").await;
        let player_id = seed_user(&state, "pl").await;
        let gm = login_server(&state, "gm").await;
        let pl = login_server(&state, "pl").await;

        let world: serde_json::Value = gm
            .post("/api/worlds")
            .json(&serde_json::json!({ "name": "W" }))
            .await
            .json();
        let world_id = world["id"].as_str().unwrap().to_string();
        gm.post(&format!("/api/worlds/{world_id}/members"))
            .json(&serde_json::json!({ "user": player_id, "role": "player" }))
            .await
            .assert_status(StatusCode::NO_CONTENT);

        // A doc the player owns (so they hold the write_fields floor) but with no
        // per-document capability grant.
        let doc_id = Uuid::from_u128(700);
        let perms = serde_json::json!({
            "default": "none",
            "users": { player_id.to_string(): "owner" },
            "property_overrides": {}
        });
        let doc = doc_json(doc_id, &world_id, serde_json::json!({ "hp": 1 }), perms);
        gm.post(&format!("/api/worlds/{world_id}/documents"))
            .json(&doc)
            .await
            .assert_status_ok();

        let embed = serde_json::json!({ "changes": [
            { "path": "/embedded/items", "old": null, "new": [] }
        ]});

        // Without a grant the owner cannot manage embedded documents.
        pl.patch(&format!("/api/documents/{doc_id}"))
            .json(&embed)
            .await
            .assert_status(StatusCode::FORBIDDEN);

        // A non-GM cannot set world defaults.
        let defaults = serde_json::json!({
            "all": { "by_role": { "owner": ["core:manage_embedded"] }, "by_user": {} }
        });
        pl.put(&format!("/api/worlds/{world_id}/capability-defaults"))
            .json(&defaults)
            .await
            .assert_status(StatusCode::FORBIDDEN);

        // The GM sets a world default granting Owners core:manage_embedded.
        gm.put(&format!("/api/worlds/{world_id}/capability-defaults"))
            .json(&defaults)
            .await
            .assert_status(StatusCode::NO_CONTENT);

        // Now the owner may manage embedded documents.
        pl.patch(&format!("/api/documents/{doc_id}"))
            .json(&embed)
            .await
            .assert_status_ok();
    }

    #[tokio::test]
    async fn contract_declarations_gm_crud_and_validation() {
        let state = initialized_state().await;
        seed_user(&state, "gm").await;
        let player_id = seed_user(&state, "pl").await;
        let gm = login_server(&state, "gm").await;
        let pl = login_server(&state, "pl").await;

        let world: serde_json::Value = gm
            .post("/api/worlds")
            .json(&serde_json::json!({ "name": "W" }))
            .await
            .json();
        let world_id = world["id"].as_str().unwrap().to_string();
        gm.post(&format!("/api/worlds/{world_id}/members"))
            .json(&serde_json::json!({ "user": player_id, "role": "player" }))
            .await
            .assert_status(StatusCode::NO_CONTENT);

        let valid = serde_json::json!([
            { "module_id": "sidebar", "version": "1.0.0",
              "provides": [{ "contract": "example.surface:widget", "cardinality": "singleton" }],
              "requires": [] },
            { "module_id": "combat", "version": "1.0.0",
              "provides": [], "requires": ["example.surface:widget"] }
        ]);

        // A non-GM cannot read or write.
        pl.put(&format!("/api/worlds/{world_id}/contracts"))
            .json(&valid)
            .await
            .assert_status(StatusCode::FORBIDDEN);
        pl.get(&format!("/api/worlds/{world_id}/contracts"))
            .await
            .assert_status(StatusCode::FORBIDDEN);

        // The GM sets a valid set and reads it back.
        gm.put(&format!("/api/worlds/{world_id}/contracts"))
            .json(&valid)
            .await
            .assert_status(StatusCode::NO_CONTENT);
        let got: serde_json::Value = gm
            .get(&format!("/api/worlds/{world_id}/contracts"))
            .await
            .json();
        assert_eq!(got[0]["provides"][0]["contract"], "example.surface:widget");

        // Dangling requires (no provider) is rejected.
        let dangling = serde_json::json!([
            { "module_id": "combat", "version": "1.0.0", "provides": [],
              "requires": ["shadowcat.surface:nonexistent"] }
        ]);
        gm.put(&format!("/api/worlds/{world_id}/contracts"))
            .json(&dangling)
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

        // Two singleton providers of the same contract is rejected.
        let dup_singleton = serde_json::json!([
            { "module_id": "a", "version": "1.0.0",
              "provides": [{ "contract": "example.surface:widget", "cardinality": "singleton" }], "requires": [] },
            { "module_id": "b", "version": "1.0.0",
              "provides": [{ "contract": "example.surface:widget", "cardinality": "singleton" }], "requires": [] }
        ]);
        gm.put(&format!("/api/worlds/{world_id}/contracts"))
            .json(&dup_singleton)
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

        // A malformed contract string is rejected.
        let malformed = serde_json::json!([
            { "module_id": "a", "version": "1.0.0",
              "provides": [{ "contract": "no-colon", "cardinality": "multi" }], "requires": [] }
        ]);
        gm.put(&format!("/api/worlds/{world_id}/contracts"))
            .json(&malformed)
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

        // The same contract declared singleton by one module and multi by another
        // is a cardinality contradiction and is rejected.
        let mixed_cardinality = serde_json::json!([
            { "module_id": "a", "version": "1.0.0",
              "provides": [{ "contract": "example.surface:widget", "cardinality": "singleton" }], "requires": [] },
            { "module_id": "b", "version": "1.0.0",
              "provides": [{ "contract": "example.surface:widget", "cardinality": "multi" }], "requires": [] }
        ]);
        gm.put(&format!("/api/worlds/{world_id}/contracts"))
            .json(&mixed_cardinality)
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

        // Two declarations for the same module_id (ambiguous topology) is rejected.
        let dup_module = serde_json::json!([
            { "module_id": "a", "version": "1.0.0", "provides": [], "requires": [] },
            { "module_id": "a", "version": "2.0.0", "provides": [], "requires": [] }
        ]);
        gm.put(&format!("/api/worlds/{world_id}/contracts"))
            .json(&dup_module)
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn world_capability_requirements_gm_only_crud() {
        let state = initialized_state().await;
        seed_user(&state, "gm").await;
        let player_id = seed_user(&state, "pl").await;
        let gm = login_server(&state, "gm").await;
        let pl = login_server(&state, "pl").await;

        let world: serde_json::Value = gm
            .post("/api/worlds")
            .json(&serde_json::json!({ "name": "W" }))
            .await
            .json();
        let world_id = world["id"].as_str().unwrap().to_string();
        gm.post(&format!("/api/worlds/{world_id}/members"))
            .json(&serde_json::json!({ "user": player_id, "role": "player" }))
            .await
            .assert_status(StatusCode::NO_CONTENT);

        let reqs = serde_json::json!([
            { "path_prefix": "/system/vision", "caps": ["dnd5e:gm_vision"] }
        ]);

        // A non-GM cannot set requirements.
        pl.put(&format!("/api/worlds/{world_id}/capability-requirements"))
            .json(&reqs)
            .await
            .assert_status(StatusCode::FORBIDDEN);

        // The GM sets them.
        gm.put(&format!("/api/worlds/{world_id}/capability-requirements"))
            .json(&reqs)
            .await
            .assert_status(StatusCode::NO_CONTENT);

        // ...and reads them back.
        let got: serde_json::Value = gm
            .get(&format!("/api/worlds/{world_id}/capability-requirements"))
            .await
            .json();
        assert_eq!(got[0]["path_prefix"], "/system/vision");

        // A malformed path_prefix is rejected.
        let bad = serde_json::json!([{ "path_prefix": "system", "caps": ["x:y"] }]);
        gm.put(&format!("/api/worlds/{world_id}/capability-requirements"))
            .json(&bad)
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

        // An empty caps list (a fail-open no-op rule) is rejected.
        let empty = serde_json::json!([{ "path_prefix": "/system/vision", "caps": [] }]);
        gm.put(&format!("/api/worlds/{world_id}/capability-requirements"))
            .json(&empty)
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

        // A prefix outside the writable namespaces (silently inert) is rejected.
        let dead = serde_json::json!([{ "path_prefix": "/nope", "caps": ["x:y"] }]);
        gm.put(&format!("/api/worlds/{world_id}/capability-requirements"))
            .json(&dead)
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

        // A trailing-slash prefix (unmatchable, silently inert) is rejected.
        let slash = serde_json::json!([{ "path_prefix": "/system/vision/", "caps": ["x:y"] }]);
        gm.put(&format!("/api/worlds/{world_id}/capability-requirements"))
            .json(&slash)
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn schema_declarations_gm_crud_and_validation() {
        let state = initialized_state().await;
        seed_user(&state, "gm").await;
        let player_id = seed_user(&state, "pl").await;
        let gm = login_server(&state, "gm").await;
        let pl = login_server(&state, "pl").await;

        let world: serde_json::Value = gm
            .post("/api/worlds")
            .json(&serde_json::json!({ "name": "W" }))
            .await
            .json();
        let world_id = world["id"].as_str().unwrap().to_string();
        gm.post(&format!("/api/worlds/{world_id}/members"))
            .json(&serde_json::json!({ "user": player_id, "role": "player" }))
            .await
            .assert_status(StatusCode::NO_CONTENT);

        let base = format!("/api/worlds/{world_id}/schemas");

        // Non-GM cannot read or write.
        pl.get(&base).await.assert_status(StatusCode::FORBIDDEN);
        pl.put(&base)
            .json(&serde_json::json!([]))
            .await
            .assert_status(StatusCode::FORBIDDEN);

        // GM: empty set is valid; default read is empty.
        gm.put(&base)
            .json(&serde_json::json!([]))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        let got: Vec<serde_json::Value> = gm.get(&base).await.json();
        assert!(got.is_empty());

        // Valid declaration accepted.
        let ok = serde_json::json!([{
            "module_id": "nightfox", "version": "1.0.0", "schema_format": 1,
            "doc_type": "actor", "subtree_pointer": "/system/stats",
            "schema": { "type": "object", "additionalProperties": { "type": "object",
                "properties": { "kind": { "type": "string" } } } }
        }]);
        gm.put(&base)
            .json(&ok)
            .await
            .assert_status(StatusCode::NO_CONTENT);

        // Pointer not a strict /system descendant -> rejected.
        for bad_ptr in ["/engine/vision", "/permissions", "/name", "", "/system"] {
            let body = serde_json::json!([{
                "module_id": "m", "version": "1", "schema_format": 1, "doc_type": "actor",
                "subtree_pointer": bad_ptr, "schema": { "type": "object" }
            }]);
            gm.put(&base)
                .json(&body)
                .await
                .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        }

        // Overlapping pointers on one doc_type -> rejected.
        let overlap = serde_json::json!([
            { "module_id": "a", "version": "1", "schema_format": 1, "doc_type": "actor",
              "subtree_pointer": "/system/stats", "schema": { "type": "object" } },
            { "module_id": "b", "version": "1", "schema_format": 1, "doc_type": "actor",
              "subtree_pointer": "/system/stats/str", "schema": { "type": "object" } }
        ]);
        gm.put(&base)
            .json(&overlap)
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

        // Same module_id declaring two non-overlapping subtrees of the same
        // doc_type is accepted -- a SchemaDeclaration is a single
        // (doc_type, subtree_pointer, schema) triple, not a per-module bundle,
        // so one module legitimately governs several subtrees (e.g. a
        // Nightfox-style module declaring both `/system/stats` and
        // `/system/mechanics`).
        let same_module_disjoint_subtrees = serde_json::json!([
            { "module_id": "nightfox", "version": "1", "schema_format": 1, "doc_type": "actor",
              "subtree_pointer": "/system/stats", "schema": {} },
            { "module_id": "nightfox", "version": "1", "schema_format": 1, "doc_type": "actor",
              "subtree_pointer": "/system/mechanics", "schema": {} }
        ]);
        gm.put(&base)
            .json(&same_module_disjoint_subtrees)
            .await
            .assert_status(StatusCode::NO_CONTENT);

        // Same module_id declaring two DIFFERENT doc_types is also accepted.
        let same_module_different_doc_types = serde_json::json!([
            { "module_id": "nightfox", "version": "1", "schema_format": 1, "doc_type": "actor",
              "subtree_pointer": "/system/stats", "schema": {} },
            { "module_id": "nightfox", "version": "1", "schema_format": 1, "doc_type": "item",
              "subtree_pointer": "/system/stats", "schema": {} }
        ]);
        gm.put(&base)
            .json(&same_module_different_doc_types)
            .await
            .assert_status(StatusCode::NO_CONTENT);

        // Same module_id, same doc_type, OVERLAPPING subtree_pointer is still
        // rejected: ambiguity is prevented by the pointer-overlap check, not
        // by module_id uniqueness, and that check does not care whether the
        // two declarations share a module_id.
        let same_module_overlapping_subtrees = serde_json::json!([
            { "module_id": "a", "version": "1", "schema_format": 1, "doc_type": "actor",
              "subtree_pointer": "/system/x", "schema": {} },
            { "module_id": "a", "version": "2", "schema_format": 1, "doc_type": "actor",
              "subtree_pointer": "/system/x", "schema": {} }
        ]);
        gm.put(&base)
            .json(&same_module_overlapping_subtrees)
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

        // Unknown schema_format -> rejected.
        let bad_fmt = serde_json::json!([{
            "module_id": "m", "version": "1", "schema_format": 999, "doc_type": "actor",
            "subtree_pointer": "/system/x", "schema": {}
        }]);
        gm.put(&base)
            .json(&bad_fmt)
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

        // Malformed schema (unknown key) -> deserialize fails -> 4xx (not 204).
        let malformed = serde_json::json!([{
            "module_id": "m", "version": "1", "schema_format": 1, "doc_type": "actor",
            "subtree_pointer": "/system/x", "schema": { "type": "string", "enum": ["a"] }
        }]);
        assert_ne!(
            gm.put(&base).json(&malformed).await.status_code(),
            StatusCode::NO_CONTENT
        );

        // Cross-field-illegal schema (items on an object) -> rejected by validate_schema.
        let cross = serde_json::json!([{
            "module_id": "m", "version": "1", "schema_format": 1, "doc_type": "actor",
            "subtree_pointer": "/system/x",
            "schema": { "type": "object", "items": { "type": "number" } }
        }]);
        gm.put(&base)
            .json(&cross)
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }

    // --- Write-quiesce barrier + admin in-server backup ---

    /// A file-backed `AppState` (real db + assets dir under a tempdir), unlike
    /// `test_state`'s `sqlite::memory:` — `create_backup`'s `VACUUM INTO` needs
    /// an actual file on disk. The returned `TempDir` must outlive the state.
    async fn file_backed_state() -> (AppState, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("shadowcat.db");
        let assets_dir = tmp.path().join("assets");
        std::fs::create_dir_all(&assets_dir).unwrap();
        let url = format!("sqlite://{}?mode=rwc", db_path.to_string_lossy());
        let repo = SqliteRepository::connect(&url).await.unwrap();
        let cfg = crate::config::Config {
            db: db_path.to_string_lossy().into_owned(),
            assets_dir: Some(assets_dir.to_string_lossy().into_owned()),
            backups_dir: Some(tmp.path().join("backups").to_string_lossy().into_owned()),
            ..crate::config::Config::default()
        };
        let state = AppState {
            repo: Arc::new(repo),
            config: Arc::new(cfg),
            setup_token: None,
            initialized: Arc::new(AtomicBool::new(true)),
            ws: crate::ws::WsState::new(),
            upload_rate: Arc::new(assets::UploadRateLimiter::new()),
            auth_throttle: Arc::new(throttle::AuthThrottle::new()),
            write_barrier: Arc::new(tokio::sync::RwLock::new(())),
        };
        (state, tmp)
    }

    #[tokio::test]
    async fn admin_backup_is_admin_gated_and_writes_a_manifest() {
        let (state, _tmp) = file_backed_state().await;
        seed_admin(&state, "root").await;
        seed_user(&state, "pleb").await;
        let admin = login_server(&state, "root").await;
        let user = login_server(&state, "pleb").await;

        user.post("/api/admin/backup")
            .await
            .assert_status(StatusCode::FORBIDDEN);

        let res = admin.post("/api/admin/backup").await;
        res.assert_status_ok();
        let manifest: serde_json::Value = res.json();
        assert!(
            manifest
                .get("db_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                > 0
        );
    }

    #[tokio::test]
    async fn write_barrier_blocks_asset_writes_while_backup_holds_it() {
        let barrier = std::sync::Arc::new(tokio::sync::RwLock::<()>::new(()));
        let quiesce = barrier.write().await; // backup in progress
        let b2 = barrier.clone();
        let attempt = tokio::spawn(async move {
            let _w = b2.read().await; // the asset write's guard
            true
        });
        tokio::task::yield_now().await;
        assert!(
            !attempt.is_finished(),
            "asset write must wait behind the quiesce"
        );
        drop(quiesce);
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), attempt)
                .await
                .unwrap()
                .unwrap()
        );
    }
}
