use axum::extract::State;
use axum::Json;
use serde::Serialize;
use ts_rs::TS;

use crate::auth::session::AuthUser;
use crate::http::AppState;

/// `GET /api/modules` response element: the raw manifest (opaque to the
/// server beyond structural discovery, ARCHITECTURE invariant 2 — the client's
/// own Zod schema re-validates it) plus the URL the client dynamic-imports.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct InstalledModuleInfo {
    #[ts(type = "unknown")]
    pub manifest: serde_json::Value,
    pub entry_url: String,
}

impl From<&crate::modules::InstalledModule> for InstalledModuleInfo {
    fn from(m: &crate::modules::InstalledModule) -> Self {
        InstalledModuleInfo {
            manifest: m.manifest_json.clone(),
            entry_url: m.entry_url.clone(),
        }
    }
}

/// `GET /api/modules` — every validly installed module. Any authenticated user
/// (a client needs this to resolve entry URLs for its world's enabled set).
/// Freshly re-scanned per request (see the plan's "module discovery caching"
/// decision) — a manual filesystem install is visible without a restart.
pub async fn list_installed_modules(
    _user: AuthUser,
    State(state): State<AppState>,
) -> Json<Vec<InstalledModuleInfo>> {
    let installed = crate::modules::scan_installed_modules(&state.config.modules_path());
    Json(installed.iter().map(Into::into).collect())
}

#[cfg(test)]
mod tests {
    use crate::http::tests::initialized_state;
    use crate::http::router;
    use axum::http::StatusCode;

    #[tokio::test]
    async fn list_installed_modules_requires_auth() {
        let server = axum_test::TestServer::new(router(initialized_state().await).await).unwrap();
        server
            .get("/api/modules")
            .await
            .assert_status(StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn list_installed_modules_returns_the_scanned_set() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("actors-plus")).unwrap();
        std::fs::write(
            dir.path().join("actors-plus").join("module.json"),
            r#"{"id":"actors-plus","version":"1.0.0","provides":[{"contract":"x:y","cardinality":"multi"}]}"#,
        )
        .unwrap();

        let mut state = initialized_state().await;
        state.config = std::sync::Arc::new(crate::config::Config {
            modules_dir: Some(dir.path().to_string_lossy().to_string()),
            ..crate::config::Config::default()
        });
        let hash = crate::auth::password::hash_password("pw").unwrap();
        state
            .repo
            .create_user("u", Some(&hash), crate::auth::role::ServerRole::User, 0)
            .await
            .unwrap();
        let server = axum_test::TestServer::builder()
            .save_cookies()
            .build(router(state).await)
            .unwrap();
        server
            .post("/api/login")
            .json(&serde_json::json!({ "username": "u", "password": "pw" }))
            .await
            .assert_status(StatusCode::NO_CONTENT);

        let got: serde_json::Value = server.get("/api/modules").await.json();
        let arr = got.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["manifest"]["id"], "actors-plus");
        assert_eq!(arr[0]["manifest"]["provides"][0]["contract"], "x:y");
        assert_eq!(arr[0]["entry_url"], "/modules/actors-plus/index.js");
    }
}
