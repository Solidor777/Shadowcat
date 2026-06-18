pub mod embed;
pub mod error;
pub mod middleware;
pub mod routes;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use axum::routing::get;
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
}

impl AppState {
    /// Resolve the token `/api/setup` will require. `None` = open window.
    pub fn resolve_setup_token(config: &Config) -> Option<String> {
        use crate::config::SetupTokenPolicy;
        match config.setup_token_policy() {
            SetupTokenPolicy::Open => None,
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
        .route("/api/me", get(routes::me))
        .route("/api/login", axum::routing::post(routes::login))
        .route("/api/logout", axum::routing::post(routes::logout))
        .route("/api/setup", axum::routing::post(routes::setup))
        .fallback(embed::static_handler)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::init_gate,
        ))
        .layer(
            // Outermost→innermost: stamp a request id, trace the span, then
            // propagate the id onto the response.
            ServiceBuilder::new()
                .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
                .layer(TraceLayer::new_for_http())
                .layer(PropagateRequestIdLayer::x_request_id()),
        )
        .layer(sessions)
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
        }
    }

    /// A `test_state` with the init gate already open — for exercising
    /// normal (post-setup) routes without walking the first-run flow.
    pub(crate) async fn initialized_state() -> AppState {
        let state = test_state().await;
        state.initialized.store(true, std::sync::atomic::Ordering::Relaxed);
        state
    }

    use crate::auth::password::hash_password;
    use crate::auth::role::ServerRole;

    async fn server_with_user(username: &str, password: &str, role: ServerRole) -> axum_test::TestServer {
        let state = initialized_state().await;
        let hash = hash_password(password).unwrap();
        state.repo.create_user(username, Some(&hash), role, 0).await.unwrap();
        axum_test::TestServer::builder()
            .save_cookies()
            .build(router(state).await)
            .unwrap()
    }

    async fn fresh_server() -> axum_test::TestServer {
        // Uninitialized state, open token window (loopback default).
        let state = test_state().await;
        axum_test::TestServer::builder().save_cookies().build(router(state).await).unwrap()
    }

    #[tokio::test]
    async fn setup_creates_admin_then_closes() {
        let server = fresh_server().await;

        // Uninitialized: a normal page redirects to setup.
        let redirect = server.get("/").await;
        redirect.assert_status(axum::http::StatusCode::SEE_OTHER);

        let setup = server.post("/api/setup").json(&serde_json::json!({
            "username": "admin", "password": "pw-admin"
        })).await;
        setup.assert_status(axum::http::StatusCode::NO_CONTENT);

        // Now initialized: second setup is a conflict, and "/" serves index.
        server.post("/api/setup").json(&serde_json::json!({
            "username": "x", "password": "y"
        })).await.assert_status(axum::http::StatusCode::CONFLICT);
        server.get("/").await.assert_status_ok();

        // The created admin can log in.
        server.post("/api/login").json(&serde_json::json!({
            "username": "admin", "password": "pw-admin"
        })).await.assert_status(axum::http::StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn setup_requires_token_when_policy_demands_it() {
        let mut state = test_state().await;
        // Force a required token regardless of bind.
        let mut cfg = crate::config::Config::default();
        cfg.setup_token = "the-token".into();
        state.config = std::sync::Arc::new(cfg.clone());
        state.setup_token = AppState::resolve_setup_token(&cfg);
        let server = axum_test::TestServer::builder().save_cookies().build(router(state).await).unwrap();

        server.post("/api/setup").json(&serde_json::json!({
            "username": "admin", "password": "pw"
        })).await.assert_status(axum::http::StatusCode::FORBIDDEN);

        server.post("/api/setup").json(&serde_json::json!({
            "username": "admin", "password": "pw", "token": "the-token"
        })).await.assert_status(axum::http::StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn login_success_then_me_then_logout() {
        let server = server_with_user("gm-1", "pw-correct", ServerRole::User).await;

        server.get("/api/me").await.assert_status(axum::http::StatusCode::UNAUTHORIZED);

        let login = server.post("/api/login").json(&serde_json::json!({
            "username": "gm-1", "password": "pw-correct"
        })).await;
        login.assert_status(axum::http::StatusCode::NO_CONTENT);

        let me = server.get("/api/me").await;
        me.assert_status_ok();
        assert!(me.text().contains("gm-1"));

        server.post("/api/logout").await.assert_status(axum::http::StatusCode::NO_CONTENT);
        server.get("/api/me").await.assert_status(axum::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn login_rejects_wrong_password_and_unknown_user_identically() {
        let server = server_with_user("gm-1", "pw-correct", ServerRole::User).await;

        let bad_pw = server.post("/api/login").json(&serde_json::json!({
            "username": "gm-1", "password": "pw-wrong"
        })).await;
        let unknown = server.post("/api/login").json(&serde_json::json!({
            "username": "ghost", "password": "whatever"
        })).await;

        bad_pw.assert_status(axum::http::StatusCode::UNAUTHORIZED);
        unknown.assert_status(axum::http::StatusCode::UNAUTHORIZED);
        assert_eq!(bad_pw.text(), unknown.text(), "no user enumeration via body");
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
}
