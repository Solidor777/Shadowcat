pub mod embed;
pub mod error;
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
        .fallback(embed::static_handler)
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

    use crate::auth::password::hash_password;
    use crate::auth::role::ServerRole;

    async fn server_with_user(username: &str, password: &str, role: ServerRole) -> axum_test::TestServer {
        let state = test_state().await;
        let hash = hash_password(password).unwrap();
        state.repo.create_user(username, Some(&hash), role, 0).await.unwrap();
        axum_test::TestServer::builder()
            .save_cookies()
            .build(router(state).await)
            .unwrap()
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
