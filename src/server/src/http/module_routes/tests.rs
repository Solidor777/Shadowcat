use crate::http::router;
use crate::http::tests::initialized_state;
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
    assert_eq!(arr[0]["id"], "actors-plus");
}

/// The wire `id` MUST be the install folder name, not the manifest's
/// author-declared `id` — the server's enabled-module set is keyed on
/// the folder, so a client keying on `manifest.id` instead would show
/// wrong toggle state and send ids the server rejects whenever the two
/// diverge (a community author's declared id colliding with, or simply
/// differing from, the folder it's installed under).
#[tokio::test]
async fn list_installed_modules_returns_the_folder_id_distinct_from_manifest_id() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("folder-name")).unwrap();
    std::fs::write(
        dir.path().join("folder-name").join("module.json"),
        r#"{"id":"declared-manifest-id","version":"1.0.0"}"#,
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
    assert_eq!(
        arr[0]["id"], "folder-name",
        "wire id must be the folder name"
    );
    assert_eq!(
        arr[0]["manifest"]["id"], "declared-manifest-id",
        "manifest.id stays the opaque author-declared value, distinct from the wire id"
    );
}

#[tokio::test]
async fn serve_module_file_requires_auth() {
    let server = axum_test::TestServer::new(router(initialized_state().await).await).unwrap();
    server
        .get("/modules/whatever/index.js")
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
}

async fn logged_in_server_with_modules_dir(dir: &std::path::Path) -> axum_test::TestServer {
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
    std::fs::write(
        dir.path().join("mod-a").join("index.js"),
        b"export const x = 1;",
    )
    .unwrap();
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
    std::fs::write(
        dir.path().join("mod-a").join("assets").join("icon.png"),
        b"\x89PNG",
    )
    .unwrap();
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
    std::fs::write(
        dir.path().join("secret.txt"),
        b"outside mod-a, inside modules_dir",
    )
    .unwrap();
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

// Pure unit coverage of the containment predicate itself, run on ALL
// three CI OSes (unlike the symlink-based HTTP reproduction below, which
// is unix-only). `Path::starts_with` alone is
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

// HTTP-reachable reproduction. A module "folder"
// that is itself a symlink resolving to `modules_root` (rather than to
// some unrelated external directory, see the "escaping" test below)
// canonicalizes `module_dir` to exactly `modules_root` — the equality
// case `starts_with` alone would wrongly permit. Left unfixed, this lets
// ANY module's `id` collapse the boundary and read a SIBLING module's own
// files (mod-b's), a direct cross-module isolation violation, not just
// access to loose files at the modules root.
#[cfg(unix)]
#[tokio::test]
async fn serve_module_file_rejects_a_module_folder_symlink_that_collapses_to_the_modules_root() {
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

// Pins the documented symlink-resolution defense
// ("Both canonicalize calls resolve symlinks too, closing that escape
// route"). A module "folder" that is itself a
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

// Regression lock for `Path::join` replacing the base
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

// The Windows-specific counterpart of the
// absolute-rel_path family above. On Windows a DRIVE-LETTER-absolute second
// argument (`C:\...`) makes `Path::join` discard the base entirely, exactly
// as a leading `/` does on Unix — a hazard the Unix-rooted tests can't
// exercise. Pure `Path`-logic (no filesystem / `canonicalize` / elevated
// symlink privileges), so it locks the regression reliably on the Windows
// CI runner where a symlink-based HTTP test would flake.
#[cfg(windows)]
#[test]
fn is_strictly_within_rejects_a_drive_letter_absolute_that_replaces_the_base() {
    use std::path::Path;
    let module_dir = Path::new(r"C:\data\modules\mod-a");
    // A drive-absolute `rel_path` — `join` discards `module_dir` entirely.
    let joined = module_dir.join(r"C:\windows\win.ini");
    assert_eq!(
        joined,
        Path::new(r"C:\windows\win.ini"),
        "a drive-absolute argument must replace the base (std invariant this test locks)"
    );
    assert!(
        !super::is_strictly_within(&joined, module_dir),
        "a drive-absolute rel_path escaping the module folder must be rejected"
    );
}

// Double-percent-encoding must fail closed (single
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
    state
        .repo
        .create_user("gm", Some(&hash), crate::auth::role::ServerRole::User, 0)
        .await
        .unwrap();
    let player_id = state
        .repo
        .create_user("pl", Some(&hash), crate::auth::role::ServerRole::User, 0)
        .await
        .unwrap();

    let gm = axum_test::TestServer::builder()
        .save_cookies()
        .build(router(state.clone()).await)
        .unwrap();
    gm.post("/api/login")
        .json(&serde_json::json!({"username":"gm","password":"pw"}))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    let world: serde_json::Value = gm
        .post("/api/worlds")
        .json(&serde_json::json!({"name":"W"}))
        .await
        .json();
    let world_id = world["id"].as_str().unwrap().to_string();
    gm.post(&format!("/api/worlds/{world_id}/members"))
        .json(&serde_json::json!({"user": player_id, "role": "player"}))
        .await
        .assert_status(StatusCode::NO_CONTENT);

    let pl = axum_test::TestServer::builder()
        .save_cookies()
        .build(router(state).await)
        .unwrap();
    pl.post("/api/login")
        .json(&serde_json::json!({"username":"pl","password":"pw"}))
        .await
        .assert_status(StatusCode::NO_CONTENT);

    (gm, pl, world_id)
}

#[tokio::test]
async fn enabled_modules_gm_crud_and_member_read() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("actors-plus")).unwrap();
    std::fs::write(
        dir.path().join("actors-plus").join("module.json"),
        format!(
            r#"{{"id":"actors-plus","version":"1.0.0","engines":{{"shadowcat":"^{}"}}}}"#,
            env!("CARGO_PKG_VERSION")
        ),
    )
    .unwrap();
    let (gm, pl, world_id) = logged_in_gm_and_player_with_modules_dir(dir.path()).await;

    // Empty by default.
    let got: serde_json::Value = gm
        .get(&format!("/api/worlds/{world_id}/enabled-modules"))
        .await
        .json();
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
    let got: serde_json::Value = pl
        .get(&format!("/api/worlds/{world_id}/enabled-modules"))
        .await
        .json();
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
    let got: serde_json::Value = gm
        .get(&format!("/api/worlds/{world_id}/enabled-modules"))
        .await
        .json();
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
async fn enabled_modules_rejects_two_system_providers() {
    let dir = tempfile::tempdir().unwrap();
    for id in ["sys-a", "sys-b"] {
        std::fs::create_dir_all(dir.path().join(id)).unwrap();
        std::fs::write(
            dir.path().join(id).join("module.json"),
            format!(
                r#"{{"id":"{id}","version":"1.0.0","engines":{{"shadowcat":"*"}},"provides":[{{"contract":"shadowcat.system","cardinality":"singleton"}}]}}"#
            ),
        )
        .unwrap();
    }
    let (gm, _pl, world_id) = logged_in_gm_and_player_with_modules_dir(dir.path()).await;
    // Two enabled system providers would let the server's system-defaults
    // pick and the client's singleton-contract winner diverge: rejected.
    gm.put(&format!("/api/worlds/{world_id}/enabled-modules"))
        .json(&serde_json::json!(["sys-a", "sys-b"]))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    // Exactly one system provider is fine.
    gm.put(&format!("/api/worlds/{world_id}/enabled-modules"))
        .json(&serde_json::json!(["sys-a"]))
        .await
        .assert_status(StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn enabling_a_system_module_refreshes_the_system_defaults_singleton() {
    /// The world's stored `system-defaults` engine body, read straight from
    /// the repo (the stored copy is what must track the manifest).
    async fn stored_sd(state: &crate::http::AppState, world_id: uuid::Uuid) -> serde_json::Value {
        use crate::data::repository::Repository;
        let docs = state
            .repo
            .query_documents_by_types(world_id, &["system-defaults"])
            .await
            .unwrap();
        assert_eq!(docs.len(), 1, "the singleton exists");
        docs[0].engine.clone().unwrap()
    }

    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("sys")).unwrap();
    std::fs::write(
        dir.path().join("sys").join("module.json"),
        r#"{"id":"sys","version":"1.0.0","engines":{"shadowcat":"*"},"provides":[{"contract":"shadowcat.system","cardinality":"singleton"}],"systemDefaults":{"scene":{"fog":false}}}"#,
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
        .create_user("gm", Some(&hash), crate::auth::role::ServerRole::User, 0)
        .await
        .unwrap();
    let server = axum_test::TestServer::builder()
        .save_cookies()
        .build(router(state.clone()).await)
        .unwrap();
    server
        .post("/api/login")
        .json(&serde_json::json!({ "username": "gm", "password": "pw" }))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    // Creating the world seeds the singleton with the empty default (no
    // module enabled yet).
    let world: serde_json::Value = server
        .post("/api/worlds")
        .json(&serde_json::json!({ "name": "W" }))
        .await
        .json();
    let world_id: uuid::Uuid = world["id"].as_str().unwrap().parse().unwrap();
    let empty = serde_json::to_value(crate::data::engine::SystemDefaultsEngine::default()).unwrap();
    assert_eq!(stored_sd(&state, world_id).await, empty);
    // Enabling the system refreshes the stored singleton to its declaration.
    server
        .put(&format!("/api/worlds/{world_id}/enabled-modules"))
        .json(&serde_json::json!(["sys"]))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    let after = stored_sd(&state, world_id).await;
    assert_eq!(after.pointer("/scene/fog"), Some(&serde_json::json!(false)));
    // Disabling refreshes back to the empty default.
    server
        .put(&format!("/api/worlds/{world_id}/enabled-modules"))
        .json(&serde_json::json!([]))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert_eq!(stored_sd(&state, world_id).await, empty);
}

#[tokio::test]
async fn enabled_modules_dedups_a_duplicate_id_preserving_first_occurrence_order() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("actors-plus")).unwrap();
    std::fs::write(
        dir.path().join("actors-plus").join("module.json"),
        format!(
            r#"{{"id":"actors-plus","version":"1.0.0","engines":{{"shadowcat":"^{}"}}}}"#,
            env!("CARGO_PKG_VERSION")
        ),
    )
    .unwrap();
    let (gm, _pl, world_id) = logged_in_gm_and_player_with_modules_dir(dir.path()).await;
    gm.put(&format!("/api/worlds/{world_id}/enabled-modules"))
        .json(&serde_json::json!(["actors-plus", "actors-plus"]))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    let got: serde_json::Value = gm
        .get(&format!("/api/worlds/{world_id}/enabled-modules"))
        .await
        .json();
    assert_eq!(got, serde_json::json!(["actors-plus"]));
}

#[tokio::test]
async fn enabled_modules_rejects_a_batch_exceeding_the_max_cap() {
    let dir = tempfile::tempdir().unwrap();
    let (gm, _pl, world_id) = logged_in_gm_and_player_with_modules_dir(dir.path()).await;
    let ids: Vec<String> = (0..(super::MAX_ENABLED_MODULES + 1))
        .map(|i| format!("mod-{i}"))
        .collect();
    gm.put(&format!("/api/worlds/{world_id}/enabled-modules"))
        .json(&serde_json::json!(ids))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    let got: serde_json::Value = gm
        .get(&format!("/api/worlds/{world_id}/enabled-modules"))
        .await
        .json();
    assert_eq!(
        got,
        serde_json::json!([]),
        "an over-cap batch must not persist"
    );
}

#[tokio::test]
async fn enabled_modules_a_batch_with_one_bad_id_rejects_the_whole_batch() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("actors-plus")).unwrap();
    std::fs::write(
        dir.path().join("actors-plus").join("module.json"),
        format!(
            r#"{{"id":"actors-plus","version":"1.0.0","engines":{{"shadowcat":"^{}"}}}}"#,
            env!("CARGO_PKG_VERSION")
        ),
    )
    .unwrap();
    let (gm, _pl, world_id) = logged_in_gm_and_player_with_modules_dir(dir.path()).await;
    gm.put(&format!("/api/worlds/{world_id}/enabled-modules"))
        .json(&serde_json::json!(["actors-plus", "ghost"]))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    let got: serde_json::Value = gm
        .get(&format!("/api/worlds/{world_id}/enabled-modules"))
        .await
        .json();
    assert_eq!(
        got,
        serde_json::json!([]),
        "a valid id in a rejected batch must not partially apply"
    );
}
