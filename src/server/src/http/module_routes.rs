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

use axum::extract::Path;
use axum::http::header;
use axum::response::{IntoResponse, Response};

use crate::http::error::AppError;

/// `GET /modules/{id}/{*path}` — static file serving from an installed
/// module's OWN folder only. Auth: any authenticated user (browsers `import()`
/// the entry + fetch its relative assets under session cookies). The server
/// never reads/executes this JS (ARCHITECTURE invariant 2) — this is
/// byte-serving with a MANDATORY two-stage path-traversal guard:
///   1. `id` alone (a single URL segment, but percent-encoded `..`/`/` can
///      still smuggle a traversal into it) must canonicalize to a path still
///      inside the modules root.
///   2. `rel_path` joined onto that module's own canonicalized root must
///      still canonicalize to a path inside THAT root.
///
/// Both canonicalize calls resolve symlinks too, closing that escape route in
/// the same check. Any failure (missing file, either escape) is a uniform 404
/// — never distinguishing "traversal rejected" from "not found".
pub async fn serve_module_file(
    _user: AuthUser,
    State(state): State<AppState>,
    Path((id, rel_path)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let modules_root = state.config.modules_path();
    let modules_root_canon = tokio::fs::canonicalize(&modules_root)
        .await
        .map_err(|_| AppError::NotFound)?;
    let module_dir = modules_root.join(&id);
    let module_dir_canon = tokio::fs::canonicalize(&module_dir)
        .await
        .map_err(|_| AppError::NotFound)?;
    if !module_dir_canon.starts_with(&modules_root_canon) {
        return Err(AppError::NotFound);
    }
    let candidate = module_dir.join(&rel_path);
    let candidate_canon = tokio::fs::canonicalize(&candidate)
        .await
        .map_err(|_| AppError::NotFound)?;
    if !candidate_canon.starts_with(&module_dir_canon) {
        return Err(AppError::NotFound);
    }
    let bytes = tokio::fs::read(&candidate_canon)
        .await
        .map_err(|_| AppError::NotFound)?;
    // `.js`/`.mjs` must be exactly `text/javascript` — load-bearing for ESM
    // `import()`; mime_guess alone is not trusted to pick the exact MIME the
    // browser's module loader requires.
    let content_type = match candidate_canon.extension().and_then(|e| e.to_str()) {
        Some("js") | Some("mjs") => "text/javascript".to_string(),
        _ => mime_guess::from_path(&candidate_canon)
            .first_or_octet_stream()
            .to_string(),
    };
    Ok(([(header::CONTENT_TYPE, content_type)], bytes).into_response())
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

    #[tokio::test]
    async fn serve_module_file_requires_auth() {
        let server = axum_test::TestServer::new(router(initialized_state().await).await).unwrap();
        server
            .get("/modules/whatever/index.js")
            .await
            .assert_status(StatusCode::UNAUTHORIZED);
    }

    async fn logged_in_server_with_modules_dir(
        dir: &std::path::Path,
    ) -> axum_test::TestServer {
        let mut state = initialized_state().await;
        state.config = std::sync::Arc::new(crate::config::Config {
            modules_dir: Some(dir.to_string_lossy().to_string()),
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
        server
    }

    #[tokio::test]
    async fn serve_module_file_serves_the_entry_with_js_content_type() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("mod-a")).unwrap();
        std::fs::write(dir.path().join("mod-a").join("index.js"), b"export const x = 1;").unwrap();
        let server = logged_in_server_with_modules_dir(dir.path()).await;

        let res = server.get("/modules/mod-a/index.js").await;
        res.assert_status_ok();
        assert_eq!(res.text(), "export const x = 1;");
        let ct = res.header("content-type");
        assert_eq!(ct, "text/javascript");
    }

    #[tokio::test]
    async fn serve_module_file_serves_a_nested_asset_with_a_generic_content_type() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("mod-a").join("assets")).unwrap();
        std::fs::write(dir.path().join("mod-a").join("assets").join("icon.png"), b"\x89PNG").unwrap();
        let server = logged_in_server_with_modules_dir(dir.path()).await;

        let res = server.get("/modules/mod-a/assets/icon.png").await;
        res.assert_status_ok();
        assert_eq!(res.header("content-type"), "image/png");
    }

    #[tokio::test]
    async fn serve_module_file_404s_a_nonexistent_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("mod-a")).unwrap();
        let server = logged_in_server_with_modules_dir(dir.path()).await;
        server
            .get("/modules/mod-a/index.js")
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn serve_module_file_rejects_a_rel_path_traversal_out_of_the_module_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("mod-a")).unwrap();
        std::fs::write(dir.path().join("secret.txt"), b"outside mod-a, inside modules_dir").unwrap();
        let server = logged_in_server_with_modules_dir(dir.path()).await;

        // Percent-encoded so the traversal segment reaches the server unresolved
        // (a client-side fetch would otherwise normalize a literal `..` away
        // before the request is even sent, defeating the point of this test).
        let res = server.get("/modules/mod-a/%2e%2e%2fsecret.txt").await;
        res.assert_status(StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn serve_module_file_rejects_an_id_segment_that_escapes_the_modules_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("mod-a")).unwrap();
        std::fs::write(dir.path().join("outside.txt"), b"parent of modules_dir").unwrap();
        let server = logged_in_server_with_modules_dir(dir.path()).await;

        // id="..%2f.." resolves (via the `id` capture alone) above modules_dir
        // before `rel_path` is even considered — the two-stage guard must catch
        // this at the FIRST canonicalize, not rely on the second.
        let res = server.get("/modules/%2e%2e/outside.txt").await;
        res.assert_status(StatusCode::NOT_FOUND);
    }
}
