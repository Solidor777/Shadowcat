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
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::data::repository::Repository;
use crate::http::error::AppError;

/// PROPER-descendant containment: `candidate` must be strictly inside `root`,
/// never equal to it. `Path::starts_with` alone is satisfied by equality, so
/// an `id` segment that canonicalizes to exactly `modules_root` (a `.`
/// segment, or a module "folder" that is a symlink back to the root itself)
/// would otherwise collapse the per-module boundary onto its parent, letting
/// stage 2 read ANY file under `modules_root` — including another module's
/// own files, not just loose root files.
fn is_strictly_within(candidate: &std::path::Path, root: &std::path::Path) -> bool {
    candidate != root && candidate.starts_with(root)
}

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
///
/// Windows case-insensitive/case-preserving filesystem semantics (a request
/// path differing only in ASCII case still resolving to the same file) are a
/// known, accepted gap: not portably testable across the three-OS CI matrix,
/// and `canonicalize` returns the on-disk casing regardless, so containment
/// still holds — only content-addressing BY case would be affected, which
/// this handler never does.
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
    if !is_strictly_within(&module_dir_canon, &modules_root_canon) {
        return Err(AppError::NotFound);
    }
    let candidate = module_dir.join(&rel_path);
    let candidate_canon = tokio::fs::canonicalize(&candidate)
        .await
        .map_err(|_| AppError::NotFound)?;
    if !candidate_canon.starts_with(&module_dir_canon) {
        return Err(AppError::NotFound);
    }
    // TOCTOU: `candidate_canon` is re-resolved above but the file could still
    // be replaced/removed between this canonicalize and the read below.
    // Accepted: exploiting that window requires filesystem write access to
    // `modules_dir`, which already grants strictly broader capability
    // (arbitrary file planting) than the race itself would add.
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

use uuid::Uuid;

use crate::http::routes::require_gm;

/// Upper bound on a world's enabled-module set. Parsed on every read/write and
/// broadcast (via the `Welcome`-time merge) — far above any realistic install.
const MAX_ENABLED_MODULES: usize = 256;

/// A world's enabled installed-module ids. Any member (needed at join to load
/// the enabled set) — mirrors `list_members`'s any-member-may-read stance.
pub async fn get_world_enabled_modules(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
) -> Result<Json<Vec<String>>, AppError> {
    state
        .repo
        .permission_context(world, user.id, user.role)
        .await?;
    Ok(Json(state.repo.world_enabled_modules(world).await?))
}

/// Replace a world's enabled installed-module set. GM/admin only. Every id
/// must name a currently-installed, validly-manifested module whose
/// `engines.shadowcat` range is satisfied by the running server version (T6) —
/// enabling a version-incompatible or unknown module is rejected outright,
/// atomically (never partially applied).
pub async fn set_world_enabled_modules(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
    Json(ids): Json<Vec<String>>,
) -> Result<StatusCode, AppError> {
    require_gm(&state, &user, world).await?;
    if ids.len() > MAX_ENABLED_MODULES {
        return Err(AppError::Unprocessable(format!(
            "too many enabled modules (max {MAX_ENABLED_MODULES})"
        )));
    }
    let installed = crate::modules::scan_installed_modules(&state.config.modules_path());
    for id in &ids {
        let Some(m) = installed.iter().find(|m| &m.id == id) else {
            return Err(AppError::Unprocessable(format!(
                "module '{id}' is not installed"
            )));
        };
        if !crate::modules::engine_compat_ok(m) {
            return Err(AppError::Unprocessable(format!(
                "module '{id}' is incompatible with this server version (requires shadowcat {})",
                m.engines_shadowcat.as_deref().unwrap_or("(missing engines.shadowcat)")
            )));
        }
    }
    state.repo.set_world_enabled_modules(world, &ids).await?;
    Ok(StatusCode::NO_CONTENT)
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

    // Does not exercise the traversal guard: axum-test builds the request
    // through the `url` crate, which applies WHATWG dot-segment normalization
    // to `%2e%2e` client-side, collapsing it before the request is sent. The
    // 404 below comes from route non-match on the normalized path, not from
    // `is_strictly_within` rejecting an escaping `id`. Real coverage for this
    // escape class lives in `is_strictly_within_rejects_equality_but_accepts_a_proper_descendant`
    // and `serve_module_file_rejects_a_module_folder_symlink_that_collapses_to_the_modules_root`.
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

    // Buddy-check Critical: pure unit coverage of the containment predicate
    // itself, run on ALL three CI OSes (unlike the symlink-based HTTP
    // reproduction below, which is unix-only). `Path::starts_with` alone is
    // satisfied by EQUALITY — an `id`/`rel_path` that canonicalizes to exactly
    // `root` must NOT be treated as "within" it, or stage 2 permits reading
    // ANY file under `modules_root`, including another module's own files.
    #[test]
    fn is_strictly_within_rejects_equality_but_accepts_a_proper_descendant() {
        use std::path::Path;
        let root = Path::new("/modules");
        assert!(
            !super::is_strictly_within(root, root),
            "root must not be considered strictly within itself"
        );
        assert!(
            !super::is_strictly_within(Path::new("/other"), root),
            "an unrelated path must not be considered within root"
        );
        assert!(
            super::is_strictly_within(Path::new("/modules/mod-a"), root),
            "a proper descendant must still be accepted"
        );
    }

    // Buddy-check Critical: HTTP-reachable reproduction. A module "folder"
    // that is itself a symlink resolving to `modules_root` (rather than to
    // some unrelated external directory, see the "escaping" test below)
    // canonicalizes `module_dir` to exactly `modules_root` — the equality
    // case `starts_with` alone would wrongly permit. Left unfixed, this lets
    // ANY module's `id` collapse the boundary and read a SIBLING module's own
    // files (mod-b's), a direct cross-module isolation violation, not just
    // access to loose files at the modules root.
    #[cfg(unix)]
    #[tokio::test]
    async fn serve_module_file_rejects_a_module_folder_symlink_that_collapses_to_the_modules_root()
    {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("mod-a")).unwrap();
        std::fs::create_dir_all(dir.path().join("mod-b")).unwrap();
        std::fs::write(
            dir.path().join("mod-b").join("secret.js"),
            b"export const secret = 1;",
        )
        .unwrap();
        // "self-link" is a normal module id (no encoding tricks needed to
        // reach the handler); it resolves to `modules_root` itself.
        symlink(dir.path(), dir.path().join("self-link")).unwrap();
        let server = logged_in_server_with_modules_dir(dir.path()).await;

        let res = server.get("/modules/self-link/mod-b/secret.js").await;
        res.assert_status(StatusCode::NOT_FOUND);
    }

    // Buddy-check Important: the documented symlink-resolution defense
    // ("Both canonicalize calls resolve symlinks too, closing that escape
    // route") had zero regression coverage. A module "folder" that is itself a
    // symlink pointing outside modules_root must still 404 once containment
    // requires a PROPER descendant (not just any path starting with the root
    // prefix, which a symlink target sharing a prefix could otherwise satisfy).
    #[cfg(unix)]
    #[tokio::test]
    async fn serve_module_file_rejects_a_module_folder_that_is_a_symlink_escaping_the_root() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("payload.js"), b"export const x = 1;").unwrap();
        symlink(outside.path(), dir.path().join("mod-evil")).unwrap();
        let server = logged_in_server_with_modules_dir(dir.path()).await;

        let res = server.get("/modules/mod-evil/payload.js").await;
        res.assert_status(StatusCode::NOT_FOUND);
    }

    // Windows symlink creation (`std::os::windows::fs::symlink_dir`) requires
    // elevated privileges / Developer Mode in most CI runners, making a
    // Windows-side regression test unreliable rather than a real gap in
    // coverage of the Unix-only test above (the canonicalize-based guard
    // itself is platform-neutral).

    // Buddy-check Minor: regression lock for `Path::join` replacing the base
    // entirely when given an absolute second argument. Already defended by the
    // post-join canonicalize + `starts_with` re-check; this test guards against
    // a future "simplification" that trusts the join without re-canonicalizing.
    #[tokio::test]
    async fn serve_module_file_rejects_a_rel_path_that_looks_absolute_after_decoding() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("mod-a")).unwrap();
        std::fs::write(dir.path().join("secret.txt"), b"outside mod-a").unwrap();
        let server = logged_in_server_with_modules_dir(dir.path()).await;

        // %2f decodes to a literal `/`; if naively joined as an absolute path
        // component this would replace `module_dir` entirely instead of
        // appending under it.
        let res = server.get("/modules/mod-a/%2f%2e%2e/secret.txt").await;
        res.assert_status(StatusCode::NOT_FOUND);
    }

    // Buddy-check Minor: double-percent-encoding must fail closed (single
    // decode pass yields a literal, non-path string like `%2e%2e%2fsecret.txt`
    // that simply doesn't exist), not panic or 500.
    #[tokio::test]
    async fn serve_module_file_rejects_double_percent_encoded_traversal() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("mod-a")).unwrap();
        std::fs::write(dir.path().join("secret.txt"), b"outside mod-a").unwrap();
        let server = logged_in_server_with_modules_dir(dir.path()).await;

        let res = server.get("/modules/mod-a/%252e%252e%252fsecret.txt").await;
        res.assert_status(StatusCode::NOT_FOUND);
    }

    async fn logged_in_gm_and_player_with_modules_dir(
        dir: &std::path::Path,
    ) -> (axum_test::TestServer, axum_test::TestServer, String) {
        let mut state = initialized_state().await;
        state.config = std::sync::Arc::new(crate::config::Config {
            modules_dir: Some(dir.to_string_lossy().to_string()),
            ..crate::config::Config::default()
        });
        let hash = crate::auth::password::hash_password("pw").unwrap();
        state.repo.create_user("gm", Some(&hash), crate::auth::role::ServerRole::User, 0).await.unwrap();
        let player_id = state
            .repo
            .create_user("pl", Some(&hash), crate::auth::role::ServerRole::User, 0)
            .await
            .unwrap();

        let gm = axum_test::TestServer::builder().save_cookies().build(router(state.clone()).await).unwrap();
        gm.post("/api/login").json(&serde_json::json!({"username":"gm","password":"pw"})).await.assert_status(StatusCode::NO_CONTENT);
        let world: serde_json::Value = gm.post("/api/worlds").json(&serde_json::json!({"name":"W"})).await.json();
        let world_id = world["id"].as_str().unwrap().to_string();
        gm.post(&format!("/api/worlds/{world_id}/members"))
            .json(&serde_json::json!({"user": player_id, "role": "player"}))
            .await
            .assert_status(StatusCode::NO_CONTENT);

        let pl = axum_test::TestServer::builder().save_cookies().build(router(state).await).unwrap();
        pl.post("/api/login").json(&serde_json::json!({"username":"pl","password":"pw"})).await.assert_status(StatusCode::NO_CONTENT);

        (gm, pl, world_id)
    }

    #[tokio::test]
    async fn enabled_modules_gm_crud_and_member_read() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("actors-plus")).unwrap();
        std::fs::write(
            dir.path().join("actors-plus").join("module.json"),
            format!(r#"{{"id":"actors-plus","version":"1.0.0","engines":{{"shadowcat":"^{}"}}}}"#, env!("CARGO_PKG_VERSION")),
        )
        .unwrap();
        let (gm, pl, world_id) = logged_in_gm_and_player_with_modules_dir(dir.path()).await;

        // Empty by default.
        let got: serde_json::Value = gm.get(&format!("/api/worlds/{world_id}/enabled-modules")).await.json();
        assert_eq!(got, serde_json::json!([]));

        // A non-GM cannot enable.
        pl.put(&format!("/api/worlds/{world_id}/enabled-modules"))
            .json(&serde_json::json!(["actors-plus"]))
            .await
            .assert_status(StatusCode::FORBIDDEN);

        // The GM enables it.
        gm.put(&format!("/api/worlds/{world_id}/enabled-modules"))
            .json(&serde_json::json!(["actors-plus"]))
            .await
            .assert_status(StatusCode::NO_CONTENT);

        // Any member (not just the GM) can read the enabled set.
        let got: serde_json::Value = pl.get(&format!("/api/worlds/{world_id}/enabled-modules")).await.json();
        assert_eq!(got, serde_json::json!(["actors-plus"]));
    }

    #[tokio::test]
    async fn enabled_modules_rejects_an_uninstalled_id() {
        let dir = tempfile::tempdir().unwrap();
        let (gm, _pl, world_id) = logged_in_gm_and_player_with_modules_dir(dir.path()).await;
        gm.put(&format!("/api/worlds/{world_id}/enabled-modules"))
            .json(&serde_json::json!(["not-installed"]))
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        // Rejected atomically: nothing is persisted from the bad batch.
        let got: serde_json::Value = gm.get(&format!("/api/worlds/{world_id}/enabled-modules")).await.json();
        assert_eq!(got, serde_json::json!([]));
    }

    #[tokio::test]
    async fn enabled_modules_rejects_an_engine_incompatible_module() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("too-new")).unwrap();
        std::fs::write(
            dir.path().join("too-new").join("module.json"),
            r#"{"id":"too-new","version":"1.0.0","engines":{"shadowcat":"^99.0.0"}}"#,
        )
        .unwrap();
        let (gm, _pl, world_id) = logged_in_gm_and_player_with_modules_dir(dir.path()).await;
        gm.put(&format!("/api/worlds/{world_id}/enabled-modules"))
            .json(&serde_json::json!(["too-new"]))
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn enabled_modules_rejects_a_module_with_no_engines_field() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("no-engines")).unwrap();
        std::fs::write(
            dir.path().join("no-engines").join("module.json"),
            r#"{"id":"no-engines","version":"1.0.0"}"#,
        )
        .unwrap();
        let (gm, _pl, world_id) = logged_in_gm_and_player_with_modules_dir(dir.path()).await;
        gm.put(&format!("/api/worlds/{world_id}/enabled-modules"))
            .json(&serde_json::json!(["no-engines"]))
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn enabled_modules_a_batch_with_one_bad_id_rejects_the_whole_batch() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("actors-plus")).unwrap();
        std::fs::write(
            dir.path().join("actors-plus").join("module.json"),
            format!(r#"{{"id":"actors-plus","version":"1.0.0","engines":{{"shadowcat":"^{}"}}}}"#, env!("CARGO_PKG_VERSION")),
        )
        .unwrap();
        let (gm, _pl, world_id) = logged_in_gm_and_player_with_modules_dir(dir.path()).await;
        gm.put(&format!("/api/worlds/{world_id}/enabled-modules"))
            .json(&serde_json::json!(["actors-plus", "ghost"]))
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        let got: serde_json::Value = gm.get(&format!("/api/worlds/{world_id}/enabled-modules")).await.json();
        assert_eq!(got, serde_json::json!([]), "a valid id in a rejected batch must not partially apply");
    }
}
