//! The `/fx` chat command: intercepted in `handle_send_message` BEFORE `parse_command` runs
//! (`parse_command` stays a pure three-kind classifier that can never produce
//! `MessageKind::System` — see its own exhaustive test). A successful `/fx` authors no
//! message document; a failure authors a whispered `MessageKind::System` notice, the SAME
//! shape `build_system_error_notice` already establishes for `/roll`.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use uuid::Uuid;

use crate::data::document::Document;
use crate::data::membership::PermissionContext;
use crate::data::permission::{cap, effective_owner_via, resolve_access_world};
use crate::data::repository::Repository;
use crate::ws::protocol::ServerMsg;
use crate::ws::room::Room;
use crate::ws::vfx::{validate_bounds, vfx_permitted, VfxRequest};

/// Parses `body` (already stripped of a leading `/fx` and any following whitespace) into an
/// asset reference (the FIRST whitespace-separated word — never containing a space) plus an
/// optional `@`-prefixed token-name tail (everything from the first `@`-prefixed word to end
/// of line, which MAY contain spaces, since a token's `Document.name` may). Returns `None`
/// for an empty body. Text after the asset reference that does not start with `@` (e.g.
/// `/fx fireball some garbage`) carries no target and is discarded — the caller treats a
/// `None` token name as `FxError::NoTarget`.
///
/// # Examples
///
/// ```
/// use shadowcat::chat::fx::parse_fx_body;
///
/// let p = parse_fx_body("fireball @Big Red Dragon").unwrap();
/// assert_eq!(p.0, "fireball");
/// assert_eq!(p.1.as_deref(), Some("Big Red Dragon"));
/// assert!(parse_fx_body("").is_none());
/// assert_eq!(parse_fx_body("fireball").unwrap().1, None);
/// assert_eq!(parse_fx_body("fireball extra garbage").unwrap().1, None);
/// ```
pub fn parse_fx_body(body: &str) -> Option<(String, Option<String>)> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (asset_ref, remainder) = match trimmed.split_once(char::is_whitespace) {
        Some((first, rest)) => (first, rest.trim_start()),
        None => (trimmed, ""),
    };
    let token_name = remainder
        .strip_prefix('@')
        .map(|tail| tail.trim().to_string());
    Some((asset_ref.to_string(), token_name))
}

/// Why `/fx` failed — each variant's `Display` (via `to_string()` in the caller, mirroring
/// `RollError`'s player-presentable-text convention) is what the whispered notice shows.
///
/// # Examples
///
/// ```
/// use shadowcat::chat::fx::FxError;
///
/// assert_eq!(FxError::NoTarget.to_string(), "Usage: /fx <effect> @<token name>");
/// assert_eq!(FxError::UnknownToken.to_string(), "No such token.");
/// ```
#[derive(Debug)]
pub enum FxError {
    /// No `@<token-name>` was given (a coordinate-literal form is not supported).
    NoTarget,
    /// The asset reference resolved to nothing readable/nameable.
    UnknownAsset,
    /// The token reference resolved to nothing the sender may read.
    UnknownToken,
    /// The world has no active scene to play on.
    NoActiveScene,
    /// The shared `PlayVfx` bounds/authz check refused the request.
    Refused,
}

impl std::fmt::Display for FxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            FxError::NoTarget => "Usage: /fx <effect> @<token name>",
            FxError::UnknownAsset => "No such effect.",
            FxError::UnknownToken => "No such token.",
            FxError::NoActiveScene => "No active scene.",
            FxError::Refused => "That effect could not be played.",
        })
    }
}

/// Resolve `asset_ref` to an asset id: a valid UUID string is used as-is (existence is
/// re-checked implicitly by the render layer's own fail-closed resolution — `/fx` itself
/// does not require the id to already exist, matching `PlayVfx`'s own un-checked `asset`
/// field); otherwise looked up by name via `Repository::asset_id_by_name`.
async fn resolve_asset_ref(
    repo: &dyn Repository,
    world_id: Uuid,
    asset_ref: &str,
) -> Result<String, FxError> {
    if let Ok(id) = Uuid::parse_str(asset_ref) {
        return Ok(id.to_string());
    }
    repo.asset_id_by_name(world_id, asset_ref)
        .await
        .ok()
        .flatten()
        .map(|id| id.to_string())
        .ok_or(FxError::UnknownAsset)
}

/// Resolve `token_name` to its center `(x, y)` on `scene`: enumerates every `token` document
/// in `world_id` (`Repository::query_documents`, the SAME method `routes::list_documents`
/// already uses for a permission-projected listing), narrows to `scene`, and picks the FIRST
/// whose `Document.name` case-insensitively equals `token_name` AND whose resolved access
/// (the identical `resolve_access_world`/`effective_owner_via` projection `list_documents`
/// runs) grants the sender `cap::READ` — a name match the sender cannot read is treated
/// exactly like no match at all (`UnknownToken` for both), so a name probe can never oracle a
/// hidden token's existence.
async fn resolve_token_center(
    repo: &dyn Repository,
    ctx: &PermissionContext,
    world_id: Uuid,
    scene: Uuid,
    token_name: &str,
) -> Result<(f64, f64), FxError> {
    let world_defaults = repo
        .world_cap_defaults(world_id)
        .await
        .map_err(|_| FxError::UnknownToken)?;
    let grants = world_defaults.grants_for("token");
    let tokens = repo
        .query_documents(world_id, "token")
        .await
        .map_err(|_| FxError::UnknownToken)?;
    let actors: std::collections::HashMap<Uuid, Document> = repo
        .query_documents(world_id, "actor")
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|a| (a.id, a))
        .collect();
    for doc in tokens {
        if doc.parent_id != Some(scene) {
            continue;
        }
        let Some(name) = &doc.name else { continue };
        if !name.eq_ignore_ascii_case(token_name) {
            continue;
        }
        let owner = effective_owner_via(&doc, &|id| actors.get(id));
        let access = resolve_access_world(ctx.user_id, ctx.world_role, &doc, &grants, owner);
        if !access.has(cap::READ) {
            continue;
        }
        let engine = doc
            .engine
            .as_ref()
            .and_then(|v| {
                serde_json::from_value::<crate::data::engine::TokenEngine>(v.clone()).ok()
            })
            .ok_or(FxError::UnknownToken)?;
        return Ok((engine.x, engine.y));
    }
    Err(FxError::UnknownToken)
}

/// Try to handle `body` as `/fx`. Returns `None` when `body` does not start with `/fx` (the
/// caller falls through to `parse_command` as normal); `Some(Ok(()))` on a successful play
/// (the caller authors no message document); `Some(Err(FxError))` on a refusal (the caller
/// authors the whispered notice via `build_system_error_notice`).
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use shadowcat::chat::fx::try_handle_fx;
/// use shadowcat::data::document::WorldRole;
/// use shadowcat::data::membership::PermissionContext;
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::ws::room::RoomRegistry;
/// use uuid::Uuid;
///
/// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
/// let registry = RoomRegistry::new();
/// let room = registry.get_or_create(&repo, Uuid::new_v4()).await.unwrap().unwrap();
/// let ctx = PermissionContext { user_id: Uuid::new_v4(), world_role: WorldRole::Player };
/// // "hello" is not the command: falls through to `parse_command` as ordinary text.
/// assert!(try_handle_fx(&repo, &room, &ctx, Uuid::new_v4(), "hello").await.is_none());
/// # }
/// ```
pub async fn try_handle_fx(
    repo: &dyn Repository,
    room: &Room,
    ctx: &PermissionContext,
    world_id: Uuid,
    body: &str,
) -> Option<Result<(), FxError>> {
    let rest = body.strip_prefix("/fx")?;
    // "/fxsomething" (no boundary) is not the command — fall through to parse_command, which
    // will treat it as ordinary text.
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some(run_fx(repo, room, ctx, world_id, rest).await)
}

/// The command body, once `try_handle_fx` has confirmed the `/fx` prefix.
async fn run_fx(
    repo: &dyn Repository,
    room: &Room,
    ctx: &PermissionContext,
    world_id: Uuid,
    rest: &str,
) -> Result<(), FxError> {
    let (asset_ref, token_name) = parse_fx_body(rest).ok_or(FxError::NoTarget)?;
    let token_name = token_name
        .filter(|n| !n.is_empty())
        .ok_or(FxError::NoTarget)?;
    let scene = active_scene(repo, world_id)
        .await
        .ok_or(FxError::NoActiveScene)?;
    let asset = resolve_asset_ref(repo, world_id, &asset_ref).await?;
    let (x, y) = resolve_token_center(repo, ctx, world_id, scene, &token_name).await?;
    let req = VfxRequest {
        scene,
        asset: asset.clone(),
        x,
        y,
        scale: None,
        rotation: None,
        duration_ms: None,
        sound: None,
        elevation: None,
    };
    if !validate_bounds(&req) || !vfx_permitted(scene, ctx, world_id, repo).await {
        return Err(FxError::Refused);
    }
    room.broadcast_aux(ServerMsg::Vfx {
        scene,
        user: ctx.user_id,
        asset,
        x,
        y,
        scale: None,
        rotation: None,
        duration_ms: None,
        sound: None,
        elevation: None,
        id: Uuid::new_v4(),
    });
    Ok(())
}

/// The world's currently active scene id, or `None` (no scene created yet). Reads the
/// `world-settings` singleton document's `activeScene` field (the camelCase serde key of
/// `WorldSettingsEngine.active_scene`) — the same field a player's client already follows
/// for `viewedSceneId` — via a plain `query_documents` scan (a world-settings document is a
/// singleton; the first match is authoritative).
async fn active_scene(repo: &dyn Repository, world_id: Uuid) -> Option<Uuid> {
    let docs = repo
        .query_documents(world_id, crate::data::engine::WORLD_SETTINGS_DOC_TYPE)
        .await
        .ok()?;
    let doc = docs.into_iter().next()?;
    let engine = doc.engine.as_ref()?;
    engine
        .get("activeScene")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
}

#[cfg(test)]
mod tests;
