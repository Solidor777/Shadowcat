pub mod assets;
pub mod embed;
pub mod error;
pub mod routes;

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
        .route("/api/me", get(routes::me))
        .route(
            "/api/me/ui-state",
            get(routes::get_ui_state).put(routes::put_ui_state),
        )
        .route("/api/login", post(routes::login))
        .route("/api/logout", post(routes::logout))
        .route("/api/setup", post(routes::setup))
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
    async fn list_members_is_gm_only_and_returns_usernames() {
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

        // GM sees members with usernames.
        let members: serde_json::Value =
            gm.get(&format!("/api/worlds/{world_id}/members")).await.json();
        let arr = members.as_array().unwrap();
        assert!(arr.iter().any(|m| m["username"] == "gm"));
        assert!(arr.iter().any(|m| m["username"] == "pl"));

        // A non-GM member is forbidden from listing members.
        pl.get(&format!("/api/worlds/{world_id}/members"))
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
              "provides": [{ "contract": "shadowcat.surface:sidebar", "cardinality": "singleton" }],
              "requires": [] },
            { "module_id": "combat", "version": "1.0.0",
              "provides": [], "requires": ["shadowcat.surface:sidebar"] }
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
        assert_eq!(
            got[0]["provides"][0]["contract"],
            "shadowcat.surface:sidebar"
        );

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
              "provides": [{ "contract": "shadowcat.surface:sidebar", "cardinality": "singleton" }], "requires": [] },
            { "module_id": "b", "version": "1.0.0",
              "provides": [{ "contract": "shadowcat.surface:sidebar", "cardinality": "singleton" }], "requires": [] }
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
              "provides": [{ "contract": "shadowcat.surface:sidebar", "cardinality": "singleton" }], "requires": [] },
            { "module_id": "b", "version": "1.0.0",
              "provides": [{ "contract": "shadowcat.surface:sidebar", "cardinality": "multi" }], "requires": [] }
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
}
