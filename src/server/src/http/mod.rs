pub mod embed;
pub mod error;
pub mod middleware;
pub mod routes;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

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
                tracing::info!(%token, "setup token required; provide it on /setup.html");
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
        .route("/ws", get(crate::ws::conn::ws_handler))
        .route("/api/debug/rooms", get(routes::debug_rooms))
        .route("/api/me", get(routes::me))
        .route("/api/login", post(routes::login))
        .route("/api/logout", post(routes::logout))
        .route("/api/setup", post(routes::setup))
        .route("/api/worlds", post(routes::create_world))
        .route(
            "/api/worlds/{id}/members",
            get(routes::list_members).post(routes::add_member),
        )
        .route(
            "/api/worlds/{id}/members/{user}",
            delete(routes::remove_member),
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
        .fallback(embed::static_handler)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::init_gate,
        ))
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

        // Uninitialized: a normal page redirects to setup.
        let redirect = server.get("/").await;
        redirect.assert_status(axum::http::StatusCode::SEE_OTHER);

        let setup = server
            .post("/api/setup")
            .json(&serde_json::json!({
                "username": "admin", "password": "pw-admin"
            }))
            .await;
        setup.assert_status(axum::http::StatusCode::NO_CONTENT);

        // Now initialized: second setup is a conflict, and "/" serves index.
        server
            .post("/api/setup")
            .json(&serde_json::json!({
                "username": "x", "password": "y"
            }))
            .await
            .assert_status(axum::http::StatusCode::CONFLICT);
        server.get("/").await.assert_status_ok();

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
        // Normal page served, not redirected to setup.
        server.get("/").await.assert_status_ok();
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
}
