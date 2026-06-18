use std::sync::atomic::Ordering;
use std::sync::OnceLock;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tower_sessions::Session;
use uuid::Uuid;

use crate::auth::password::{hash_password, verify_password};
use crate::auth::role::ServerRole;
use crate::auth::session::{AdminUser, AuthUser, SessionUser};
use crate::auth::setup::{create_admin, now_millis};
use crate::data::command::{Command, FieldChange, Operation};
use crate::data::document::{Document, Scope, World, WorldRole};
use crate::data::membership::PermissionContext;
use crate::data::permission::{filter_command, filter_properties, resolve_access};
use crate::data::repository::Repository;
use crate::health::HealthStatus;
use crate::http::error::AppError;
use crate::http::AppState;
use crate::ws::room::RoomStatsSnapshot;

/// Liveness + DB connectivity probe.
pub async fn health(State(state): State<AppState>) -> Json<HealthStatus> {
    let connected = sqlx::query("SELECT 1")
        .fetch_one(state.repo.pool())
        .await
        .is_ok();
    Json(HealthStatus::ok(connected))
}

/// Admin-only snapshot of live room telemetry.
pub async fn debug_rooms(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> Json<Vec<RoomStatsSnapshot>> {
    Json(state.ws.rooms.snapshot())
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct MeResponse {
    pub id: uuid::Uuid,
    pub username: String,
    pub server_role: ServerRole,
}

/// Current session identity, or 401.
pub async fn me(user: AuthUser) -> Json<MeResponse> {
    Json(MeResponse {
        id: user.id,
        username: user.username,
        server_role: user.role,
    })
}

/// A real Argon2id hash of a throwaway password, computed once. The unknown-user
/// login path verifies against it so it costs the same as a wrong-password path,
/// removing a timing oracle that would otherwise reveal which usernames exist.
fn anti_enumeration_phc() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY
        .get_or_init(|| hash_password("anti-enumeration-unused").expect("hash dummy"))
        .as_str()
}

/// Verify credentials and establish a session. Uniform 401 on unknown user or
/// wrong password — no enumeration. Always runs a verify to keep timing flat.
pub async fn login(
    State(state): State<AppState>,
    session: Session,
    Json(body): Json<LoginRequest>,
) -> Result<axum::http::StatusCode, AppError> {
    let record = state
        .repo
        .user_by_username(&body.username)
        .await
        .map_err(|_| AppError::Internal)?;

    // Exactly one Argon2 verify on every path — against the stored hash when
    // present, else a throwaway hash — so unknown users, credential-less users,
    // and wrong passwords all cost the same and cannot be told apart by timing.
    let verify_target = record
        .as_ref()
        .and_then(|u| u.password_hash.as_deref())
        .map(str::to_owned)
        .unwrap_or_else(|| anti_enumeration_phc().to_owned());
    let verified = verify_password(&body.password, &verify_target);

    // Only a user that actually has a stored credential may authenticate.
    let authed = record.filter(|u| u.password_hash.is_some());
    let (true, Some(u)) = (verified, authed) else {
        return Err(AppError::Unauthorized);
    };

    // Rotate the session id on privilege change to defeat session fixation.
    session.cycle_id().await.map_err(|_| AppError::Internal)?;
    session
        .insert(
            "user",
            SessionUser {
                id: u.id,
                username: u.username,
                role: u.server_role,
            },
        )
        .await
        .map_err(|_| AppError::Internal)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Destroy the session. Propagates store errors so a failed flush is not
/// reported as a successful logout — the cookie would otherwise still authenticate.
pub async fn logout(session: Session) -> Result<axum::http::StatusCode, AppError> {
    session.flush().await.map_err(|_| AppError::Internal)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct SetupRequest {
    pub username: String,
    pub password: String,
    pub token: Option<String>,
}

/// First-run admin creation. Gated: 409 once initialized; 403 on token mismatch
/// when a token is required. Flips `initialized` so the gate opens.
pub async fn setup(
    State(state): State<AppState>,
    Json(body): Json<SetupRequest>,
) -> Result<axum::http::StatusCode, AppError> {
    // Fast reject once initialized to avoid an Argon2 hash on a closed window;
    // the guarded insert below is the authoritative race-free gate.
    if state.initialized.load(Ordering::Relaxed) {
        return Err(AppError::Conflict("server already initialized".into()));
    }
    if let Some(expected) = &state.setup_token {
        let provided = body.token.as_deref().unwrap_or("");
        // Constant-time compare: the token guards the internet-exposed first-admin
        // window and that window stays open across failed attempts.
        if !bool::from(provided.as_bytes().ct_eq(expected.as_bytes())) {
            return Err(AppError::Forbidden);
        }
    }
    match create_admin(&state.repo, &body.username, &body.password, now_millis()).await? {
        Some(_) => {
            state.initialized.store(true, Ordering::Relaxed);
            Ok(axum::http::StatusCode::NO_CONTENT)
        }
        None => Err(AppError::Conflict("server already initialized".into())),
    }
}

// --- Worlds, membership, and documents (M5) ---

/// Run `ops` through the one authoritative write path for `world`, broadcasting
/// to live WS subscribers, and return the author's filtered view of the command.
async fn write_ops(
    state: &AppState,
    user: &AuthUser,
    world: Uuid,
    ops: Vec<Operation>,
) -> Result<Json<Command>, AppError> {
    let ctx = state
        .repo
        .permission_context(world, user.id, user.role)
        .await?;
    let room = state
        .ws
        .rooms
        .get_or_create(state.repo.as_ref(), world)
        .await?
        .ok_or(AppError::NotFound)?;
    let cmd = room
        .publish(state.repo.as_ref(), &ctx, ops, now_millis())
        .await?;
    let filtered = filter_command(state.repo.as_ref(), &cmd, &ctx).await;
    Ok(Json(filtered))
}

/// Resolve the caller's context and require world-GM authority (server admins
/// resolve to GM). Used to gate membership management.
async fn require_gm(
    state: &AppState,
    user: &AuthUser,
    world: Uuid,
) -> Result<PermissionContext, AppError> {
    let ctx = state
        .repo
        .permission_context(world, user.id, user.role)
        .await?;
    if ctx.world_role != WorldRole::Gm {
        return Err(AppError::Forbidden);
    }
    Ok(ctx)
}

/// The world_id of a world-scoped document, or 404 for a compendium document
/// (compendium CRUD is out of M5 scope).
fn world_of(doc: &Document) -> Result<Uuid, AppError> {
    match doc.scope {
        Scope::World { world_id } => Ok(world_id),
        Scope::Compendium { .. } => Err(AppError::NotFound),
    }
}

#[derive(Deserialize)]
pub struct CreateWorldRequest {
    pub name: String,
}

/// Any authenticated user may create a world; the creator is seated as its GM.
pub async fn create_world(
    user: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateWorldRequest>,
) -> Result<Json<World>, AppError> {
    let world = state
        .repo
        .create_world_owned(&body.name, user.id, now_millis())
        .await?;
    Ok(Json(world))
}

#[derive(Serialize)]
pub struct MemberEntry {
    pub user: Uuid,
    pub role: WorldRole,
}

pub async fn list_members(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
) -> Result<Json<Vec<MemberEntry>>, AppError> {
    require_gm(&state, &user, world).await?;
    let members = state.repo.list_members(world).await?;
    Ok(Json(
        members
            .into_iter()
            .map(|(user, role)| MemberEntry { user, role })
            .collect(),
    ))
}

#[derive(Deserialize)]
pub struct AddMemberRequest {
    pub user: Uuid,
    pub role: WorldRole,
}

/// Add a member or change an existing member's role (idempotent upsert).
pub async fn add_member(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
    Json(body): Json<AddMemberRequest>,
) -> Result<StatusCode, AppError> {
    require_gm(&state, &user, world).await?;
    if state.repo.member_role(world, body.user).await?.is_some() {
        state.repo.set_role(world, body.user, body.role).await?;
    } else {
        state.repo.add_member(world, body.user, body.role).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_member(
    user: AuthUser,
    State(state): State<AppState>,
    Path((world, target)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    require_gm(&state, &user, world).await?;
    state.repo.remove_member(world, target).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn create_document(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
    Json(doc): Json<Document>,
) -> Result<Json<Command>, AppError> {
    write_ops(&state, &user, world, vec![Operation::Create { doc }]).await
}

#[derive(Deserialize)]
pub struct DocQuery {
    pub r#type: String,
}

pub async fn list_documents(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
    Query(q): Query<DocQuery>,
) -> Result<Json<Vec<Document>>, AppError> {
    let ctx = state
        .repo
        .permission_context(world, user.id, user.role)
        .await?;
    let docs = state.repo.query_documents(world, &q.r#type).await?;
    let visible = docs
        .into_iter()
        .filter_map(|d| {
            let access = resolve_access(ctx.user_id, ctx.world_role, &d);
            access.can_read.then(|| filter_properties(&d, access))
        })
        .collect();
    Ok(Json(visible))
}

pub async fn get_document(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Document>, AppError> {
    let doc = state
        .repo
        .get_document(id)
        .await?
        .ok_or(AppError::NotFound)?;
    let world = world_of(&doc)?;
    let ctx = state
        .repo
        .permission_context(world, user.id, user.role)
        .await?;
    let access = resolve_access(ctx.user_id, ctx.world_role, &doc);
    if !access.can_read {
        return Err(AppError::NotFound);
    }
    Ok(Json(filter_properties(&doc, access)))
}

#[derive(Deserialize)]
pub struct PatchRequest {
    pub changes: Vec<FieldChange>,
}

pub async fn patch_document(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchRequest>,
) -> Result<Json<Command>, AppError> {
    let doc = state
        .repo
        .get_document(id)
        .await?
        .ok_or(AppError::NotFound)?;
    let world = world_of(&doc)?;
    write_ops(
        &state,
        &user,
        world,
        vec![Operation::Update {
            doc_id: id,
            changes: body.changes,
        }],
    )
    .await
}

pub async fn delete_document(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Command>, AppError> {
    let doc = state
        .repo
        .get_document(id)
        .await?
        .ok_or(AppError::NotFound)?;
    let world = world_of(&doc)?;
    write_ops(&state, &user, world, vec![Operation::Delete { doc }]).await
}
