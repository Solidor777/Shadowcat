use std::sync::atomic::Ordering;
use std::sync::OnceLock;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use ts_rs::TS;
use tower_sessions::Session;
use uuid::Uuid;

use crate::auth::password::{hash_password, verify_password_async};
use crate::auth::role::ServerRole;
use crate::auth::session::{AdminUser, AuthUser, SessionUser};
use crate::auth::setup::{create_admin, now_millis};
use crate::data::command::{Command, FieldChange, Operation};
use crate::data::document::{
    CapabilityGrants, CapabilityRequirement, Cardinality, ContractDeclaration, Document, Scope,
    World, WorldRole,
};
use crate::data::membership::PermissionContext;
use crate::data::permission::{
    cap, filter_command, filter_properties, required_cap_for_path, resolve_access_world,
};
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

/// Upper bound on a stored UI-state blob. It is read/written whole per user and
/// is small UI session state; far above any realistic payload.
const MAX_UI_STATE_BYTES: usize = 64 * 1024;

/// The caller's opaque UI-state object, or `{}` when unset.
pub async fn get_ui_state(
    user: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    let val = match state.repo.get_ui_state(user.id).await? {
        // Stored only after passing the object-shape check below, so a parse
        // failure here is server-side corruption, not client-actionable.
        Some(s) => serde_json::from_str(&s).map_err(|_| AppError::Internal)?,
        None => serde_json::json!({}),
    };
    Ok(Json(val))
}

/// Replace the caller's UI-state. Validates object-shape + size only; the body
/// is otherwise opaque (the client owns its structure).
pub async fn put_ui_state(
    user: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Result<StatusCode, AppError> {
    if !body.is_object() {
        return Err(AppError::Unprocessable("ui_state must be a JSON object".into()));
    }
    // Cap the canonical compact serialization (what is actually persisted), not
    // the raw request bytes — deterministic regardless of client whitespace.
    let s = serde_json::to_string(&body).map_err(|_| AppError::Internal)?;
    if s.len() > MAX_UI_STATE_BYTES {
        return Err(AppError::Unprocessable(format!(
            "ui_state too large (max {MAX_UI_STATE_BYTES} bytes)"
        )));
    }
    state.repo.set_ui_state(user.id, &s).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Public server bootstrap info for the SPA's first-load routing (setup vs
/// login). Exposes nothing beyond the `initialized` bit the setup-409 already
/// reveals.
#[derive(Serialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct ServerConfig {
    pub initialized: bool,
}

/// Whether a first admin exists. Unauthenticated; reachable before init.
pub async fn config(State(state): State<AppState>) -> Json<ServerConfig> {
    Json(ServerConfig {
        initialized: state.initialized.load(Ordering::Relaxed),
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
    let verified = verify_password_async(body.password, verify_target).await;

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
    let world_defaults = state.repo.world_cap_defaults(world).await?;
    let filtered = filter_command(state.repo.as_ref(), &cmd, &ctx, &world_defaults).await;
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

/// A world the caller can access, with their effective role. The client's
/// world-select list item.
#[derive(Serialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct WorldEntry {
    pub id: Uuid,
    pub name: String,
    pub role: WorldRole,
}

/// Worlds the authenticated caller may access.
pub async fn list_worlds(
    user: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<WorldEntry>>, AppError> {
    let worlds = state.repo.worlds_for_user(user.id, user.role).await?;
    Ok(Json(
        worlds
            .into_iter()
            .map(|(w, role)| WorldEntry {
                id: w.id,
                name: w.name,
                role,
            })
            .collect(),
    ))
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
    let world_defaults = state.repo.world_cap_defaults(world).await?;
    let docs = state.repo.query_documents(world, &q.r#type).await?;
    let visible = docs
        .into_iter()
        .filter_map(|d| {
            let access = resolve_access_world(ctx.user_id, ctx.world_role, &d, &world_defaults);
            access
                .has(cap::READ)
                .then(|| filter_properties(&d, &access))
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
    let world_defaults = state.repo.world_cap_defaults(world).await?;
    let access = resolve_access_world(ctx.user_id, ctx.world_role, &doc, &world_defaults);
    if !access.has(cap::READ) {
        return Err(AppError::NotFound);
    }
    Ok(Json(filter_properties(&doc, &access)))
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

/// Structural validation of a capability token: `<namespace>:<verb>`, both parts
/// non-empty. The server never interprets the verb's meaning.
fn validate_capability(token: &str) -> Result<(), AppError> {
    match token.split_once(':') {
        Some((ns, verb)) if !ns.is_empty() && !verb.is_empty() => Ok(()),
        _ => Err(AppError::Unprocessable(format!(
            "malformed capability '{token}' (expected <namespace>:<verb>)"
        ))),
    }
}

fn validate_grants(grants: &CapabilityGrants) -> Result<(), AppError> {
    for set in grants.by_role.values().chain(grants.by_user.values()) {
        for token in set {
            validate_capability(token)?;
        }
    }
    Ok(())
}

/// A world's default capability grants. GM/admin only.
pub async fn get_world_capability_defaults(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
) -> Result<Json<CapabilityGrants>, AppError> {
    require_gm(&state, &user, world).await?;
    Ok(Json(state.repo.world_cap_defaults(world).await?))
}

/// Replace a world's default capability grants. GM/admin only.
pub async fn set_world_capability_defaults(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
    Json(grants): Json<CapabilityGrants>,
) -> Result<StatusCode, AppError> {
    require_gm(&state, &user, world).await?;
    validate_grants(&grants)?;
    state.repo.set_world_cap_defaults(world, &grants).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Upper bound on the number of declarative requirements stored per world. The
/// policy is parsed on every write and broadcast in the `Welcome` frame, so it
/// is kept small; far above any realistic hand-authored ruleset.
const MAX_CAPABILITY_REQUIREMENTS: usize = 256;

/// A world's declarative capability requirements. GM/admin only.
pub async fn get_world_capability_requirements(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
) -> Result<Json<Vec<CapabilityRequirement>>, AppError> {
    require_gm(&state, &user, world).await?;
    Ok(Json(state.repo.world_cap_requirements(world).await?))
}

/// Replace a world's declarative capability requirements. GM/admin only.
pub async fn set_world_capability_requirements(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
    Json(reqs): Json<Vec<CapabilityRequirement>>,
) -> Result<StatusCode, AppError> {
    require_gm(&state, &user, world).await?;
    // Bound the stored policy: it is parsed on every write and broadcast in full
    // to every client on connect, so an unbounded list amplifies across joiners.
    if reqs.len() > MAX_CAPABILITY_REQUIREMENTS {
        return Err(AppError::Unprocessable(format!(
            "too many requirements (max {MAX_CAPABILITY_REQUIREMENTS})"
        )));
    }
    for req in &reqs {
        // A trailing slash makes the prefix unmatchable (the matcher appends its
        // own `/`), so it would silently enforce nothing.
        if req.path_prefix.ends_with('/') {
            return Err(AppError::Unprocessable(format!(
                "path_prefix '{}' must not end with /",
                req.path_prefix
            )));
        }
        // Reject prefixes outside the writable namespaces: they can never match a
        // real field write, so the rule would be a silent no-op (false confidence
        // that a path is protected when it is not).
        if required_cap_for_path(&req.path_prefix).is_none() {
            return Err(AppError::Unprocessable(format!(
                "path_prefix '{}' is not within a writable namespace \
                 (/system, /embedded, /permissions)",
                req.path_prefix
            )));
        }
        // A requirement with no capabilities enforces nothing — reject it rather
        // than store a fail-open rule a GM believes is protecting a path.
        if req.caps.is_empty() {
            return Err(AppError::Unprocessable(format!(
                "requirement for '{}' must list at least one capability",
                req.path_prefix
            )));
        }
        for token in &req.caps {
            validate_capability(token)?;
        }
    }
    state.repo.set_world_cap_requirements(world, &reqs).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Upper bound on the number of contract declarations stored per world. Parsed
/// on every write and broadcast in `Welcome`; far above any realistic module set.
const MAX_CONTRACT_DECLARATIONS: usize = 256;

/// Structural validation of a contract id: `<namespace>:<name>`, both non-empty.
fn validate_contract_token(token: &str) -> Result<(), AppError> {
    match token.split_once(':') {
        Some((ns, name)) if !ns.is_empty() && !name.is_empty() => Ok(()),
        _ => Err(AppError::Unprocessable(format!(
            "malformed contract '{token}' (expected <namespace>:<name>)"
        ))),
    }
}

/// Validate a world's contract declaration set: bounded count, well-formed
/// non-empty fields, no duplicate `singleton` provider, and every `requires`
/// satisfied by some `provides` in the set. Fail-closed — the server is the
/// consistency authority.
fn validate_contract_declarations(decls: &[ContractDeclaration]) -> Result<(), AppError> {
    use std::collections::{HashMap, HashSet};
    if decls.len() > MAX_CONTRACT_DECLARATIONS {
        return Err(AppError::Unprocessable(format!(
            "too many declarations (max {MAX_CONTRACT_DECLARATIONS})"
        )));
    }
    let mut provided: HashSet<&str> = HashSet::new();
    let mut singleton_count: HashMap<&str, usize> = HashMap::new();
    for d in decls {
        if d.module_id.is_empty() || d.version.is_empty() {
            return Err(AppError::Unprocessable(
                "declaration module_id and version must be non-empty".into(),
            ));
        }
        for p in &d.provides {
            validate_contract_token(&p.contract)?;
            provided.insert(p.contract.as_str());
            if p.cardinality == Cardinality::Singleton {
                let n = singleton_count.entry(p.contract.as_str()).or_insert(0);
                *n += 1;
                if *n > 1 {
                    return Err(AppError::Unprocessable(format!(
                        "contract '{}' is singleton but provided more than once",
                        p.contract
                    )));
                }
            }
        }
    }
    for d in decls {
        for req in &d.requires {
            validate_contract_token(req)?;
            if !provided.contains(req.as_str()) {
                return Err(AppError::Unprocessable(format!(
                    "required contract '{req}' has no provider in the declared set"
                )));
            }
        }
    }
    Ok(())
}

/// A world's UI contract declarations. GM/admin only.
pub async fn get_world_contract_declarations(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
) -> Result<Json<Vec<ContractDeclaration>>, AppError> {
    require_gm(&state, &user, world).await?;
    Ok(Json(state.repo.world_contract_declarations(world).await?))
}

/// Replace a world's UI contract declarations. GM/admin only; validated.
pub async fn set_world_contract_declarations(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
    Json(decls): Json<Vec<ContractDeclaration>>,
) -> Result<StatusCode, AppError> {
    require_gm(&state, &user, world).await?;
    validate_contract_declarations(&decls)?;
    state
        .repo
        .set_world_contract_declarations(world, &decls)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
