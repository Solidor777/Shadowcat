#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use axum::extract::State;
use axum::Json;
use serde::Serialize;
use ts_rs::TS;

use crate::auth::session::AuthUser;
use crate::http::AppState;

/// `GET /api/modules` response element: `id` is the canonical install folder
/// name (`crate::modules::InstalledModule::id`) — the SAME key
/// `set_world_enabled_modules`/`world_enabled_modules` validate and store
/// against. `manifest` is the raw, author-declared manifest (opaque to the
/// server beyond structural discovery — the client's own Zod schema
/// re-validates it) and may declare a DIFFERENT `id`
/// than the folder it's installed under; callers must key enabled-set
/// membership on this `id` field, never `manifest.id`, or toggle state and
/// save requests silently diverge from the server's authoritative key space.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct InstalledModuleInfo {
    /// The install FOLDER name — the authoritative enablement key.
    pub id: String,
    /// The raw module.json, byte-for-byte (the client's Zod schema reads it).
    #[ts(type = "unknown")]
    pub manifest: serde_json::Value,
    /// Served entry URL: `/modules/<folder-id>/<entry>`.
    pub entry_url: String,
}

impl From<&crate::modules::InstalledModule> for InstalledModuleInfo {
    fn from(m: &crate::modules::InstalledModule) -> Self {
        InstalledModuleInfo {
            id: m.id.clone(),
            manifest: m.manifest_json.clone(),
            entry_url: m.entry_url.clone(),
        }
    }
}

/// `GET /api/modules` — every validly installed module. Any authenticated user
/// (a client needs this to resolve entry URLs for its world's enabled set).
/// Freshly re-scanned per request, with no cache — a manual filesystem install is visible
/// without a restart, at the cost of a directory walk per call.
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
/// never reads or executes this JS — this is byte-serving with a MANDATORY
/// two-stage path-traversal guard:
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
/// `engines.shadowcat` range is satisfied by the running server version —
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
    // Order-preserving dedup: a duplicate id in the request body is otherwise
    // inert (stored/validated redundantly) but inflates the persisted set and
    // the client's echoed response; first occurrence wins.
    let mut seen = std::collections::HashSet::with_capacity(ids.len());
    let ids: Vec<String> = ids
        .into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect();
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
                m.engines_shadowcat
                    .as_deref()
                    .unwrap_or("(missing engines.shadowcat)")
            )));
        }
    }
    // At most one enabled module may provide the system contract: the
    // server's system-defaults derivation and the client's singleton-contract
    // winner must never diverge on which system is active.
    let systems: Vec<&str> = ids
        .iter()
        .filter(|id| installed.iter().any(|m| &m.id == *id && m.provides_system))
        .map(String::as_str)
        .collect();
    if systems.len() > 1 {
        return Err(AppError::Unprocessable(format!(
            "at most one enabled module may provide {} (got: {})",
            crate::modules::SYSTEM_CONTRACT,
            systems.join(", ")
        )));
    }
    state.repo.set_world_enabled_modules(world, &ids).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests;
