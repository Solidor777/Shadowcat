#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::sync::atomic::Ordering;
use std::sync::OnceLock;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tower_sessions::Session;
use ts_rs::TS;
use uuid::Uuid;

use crate::auth::password::{hash_password, verify_password_async};
use crate::auth::role::ServerRole;
use crate::auth::session::{AdminUser, AuthUser, SessionUser};
use crate::auth::setup::{create_admin, now_millis};
use crate::data::command::{Command, FieldChange, Operation, WriteOrigin};
use crate::data::document::{
    AdditionalProperties, CapabilityRequirement, Cardinality, ContractDeclaration, Document,
    Schema, SchemaDeclaration, SchemaType, World, WorldCapDefaults, WorldRole,
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

/// `POST /api/admin/backup` — in-server whole-server backup (admin only). Holds
/// the write barrier in write mode across the snapshot, so no asset
/// commit+rename interleaves with the `VACUUM INTO` + assets copy — the
/// backup's DB metadata and file bytes are mutually consistent. DB writers
/// need no gating: `VACUUM INTO` is transactionally consistent against a live
/// writer by itself.
pub async fn admin_backup(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<crate::backup::BackupManifest>, AppError> {
    let _quiesce = state.write_barrier.write().await;
    let out = state
        .config
        .backups_path()
        .join(format!("backup-{}", now_millis()));
    // `create_backup` opens its own single-connection pool against the same
    // SQLite file; a `VACUUM INTO` "database is locked" from racing the live
    // pool surfaces as an ordinary Internal error for the admin to retry —
    // no busy-loop.
    let manifest = crate::backup::create_backup(
        std::path::Path::new(&state.config.db),
        &state.config.assets_path(),
        &out,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "in-server backup failed");
        AppError::Internal
    })?;
    Ok(Json(manifest))
}

/// `POST /api/login` body.
#[derive(Deserialize)]
pub struct LoginRequest {
    /// Account username.
    pub username: String,
    /// Plaintext password (verified against the Argon2 hash; never stored).
    pub password: String,
}

/// `GET /api/me` response: the session's identity.
#[derive(Serialize)]
pub struct MeResponse {
    /// Account id.
    pub id: uuid::Uuid,
    /// Account username.
    pub username: String,
    /// Server tier (admin/user) — orthogonal to any world role.
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

/// Upper bound on a stored UI-state blob, applied to the MERGED result of a
/// patch (only the store sees that; see `SqliteRepository::merge_ui_state`).
/// Small UI session state; far above any realistic payload.
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

/// Merge a partial UI-state patch into the caller's stored state; the merge
/// rule itself is stated once, on `SqliteRepository::merge_ui_state`. This
/// route only validates the wire shape (`body` and any `worlds` value must be
/// JSON objects) before delegating. Sending only changed slices/keys is the
/// concurrency control: concurrent sessions of one account contend only on
/// the individual keys both write, instead of last-writer-wins on the whole
/// blob. The size cap applies to the merged result (422 via
/// `DataError::TooLarge`).
pub async fn put_ui_state(
    user: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Result<StatusCode, AppError> {
    if !body.is_object() {
        return Err(AppError::Unprocessable(
            "ui_state must be a JSON object".into(),
        ));
    }
    if let Some(worlds) = body.get("worlds") {
        if !worlds.is_object() {
            return Err(AppError::Unprocessable(
                "ui_state `worlds` must be a JSON object".into(),
            ));
        }
    }
    state
        .repo
        .merge_ui_state(user.id, &body, MAX_UI_STATE_BYTES)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Public server bootstrap info for the SPA's first-load routing (setup vs
/// login). Exposes nothing beyond the `initialized` bit the setup-409 already
/// reveals.
#[derive(Serialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct ServerConfig {
    /// Whether a first admin exists (routes the SPA to setup vs login).
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
    ip: crate::http::throttle::ClientIp,
    Json(body): Json<LoginRequest>,
) -> Result<axum::http::StatusCode, AppError> {
    use crate::http::throttle::{LOGIN_PER_MIN_PER_IDENTITY, LOGIN_PER_MIN_PER_IP};
    let now = now_millis();
    let per_identity = state
        .config
        .login_per_min_per_identity
        .unwrap_or(LOGIN_PER_MIN_PER_IDENTITY);
    let per_ip = state
        .config
        .login_per_min_per_ip
        .unwrap_or(LOGIN_PER_MIN_PER_IP);
    // Identity key counts unknown usernames identically to known ones — the
    // throttle must not become the enumeration oracle the constant-verify
    // design below exists to prevent. Uniform 429 for both key kinds.
    let ident_key = format!("login:u:{}", body.username.to_lowercase());
    let ip_ok = match ip.0 {
        Some(addr) => state
            .auth_throttle
            .check(&format!("login:ip:{addr}"), now, per_ip),
        None => true,
    };
    if !ip_ok || !state.auth_throttle.check(&ident_key, now, per_identity) {
        return Err(AppError::TooManyRequests("try again later".into()));
    }

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

/// `POST /api/setup` body: first-admin creation.
#[derive(Deserialize)]
pub struct SetupRequest {
    /// First admin's username.
    pub username: String,
    /// First admin's password (hashed with Argon2 before storage).
    pub password: String,
    /// Setup token, when the policy requires one (`Config::setup_token_policy`).
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

// --- Account administration ---

/// Username policy for admin-created accounts: 3–32 characters drawn from
/// `[A-Za-z0-9._-]`. Constraining the identity space to ASCII is load-bearing,
/// not cosmetic: it makes SQLite's ASCII-only `NOCASE` uniqueness guard in
/// `create_user_unique` a complete case-fold, and it removes the whitespace and
/// Unicode-homoglyph variants that would otherwise let one account impersonate
/// another in a member roster or chat attribution.
const MIN_USERNAME_LEN: usize = 3;
/// Username length ceiling (roster/attribution display bound).
const MAX_USERNAME_LEN: usize = 32;
/// Floor on an admin-issued password. The server never stores plaintext, so
/// this only guards against trivially guessable seeded accounts.
const MIN_PASSWORD_LEN: usize = 8;
/// Ceiling on the plaintext handed to Argon2. Hash cost grows with input past
/// the block size, so an unbounded password is a CPU amplification vector.
const MAX_PASSWORD_BYTES: usize = 256;

/// Normalize (trim) and validate a username against the account policy.
/// Applied at EVERY insertion path — `create_user` here, and `create_admin`
/// (which `/api/setup` and the headless `bootstrap_admin` both funnel through)
/// — so `create_user_unique`'s documented ASCII invariant holds for every row
/// in `users`. A non-ASCII first admin would not be case-folded by SQLite's
/// ASCII-only `NOCASE`, leaving a homoglyph account able to impersonate it.
pub(crate) fn validate_username(raw: &str) -> Result<String, AppError> {
    let name = raw.trim();
    if name.len() < MIN_USERNAME_LEN || name.len() > MAX_USERNAME_LEN {
        return Err(AppError::Unprocessable(format!(
            "username must be {MIN_USERNAME_LEN}-{MAX_USERNAME_LEN} characters"
        )));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(AppError::Unprocessable(
            "username may contain only letters, digits, '.', '_' and '-'".into(),
        ));
    }
    Ok(name.to_owned())
}

/// `POST /api/users` body (admin-only route).
#[derive(Deserialize)]
pub struct CreateUserRequest {
    /// New account's username (validated length/uniqueness).
    pub username: String,
    /// New account's password (Argon2-hashed before storage).
    pub password: String,
    /// Server tier of the new account; omitted means `User`. Only an admin can
    /// reach this field at all (the `AdminUser` extractor runs before the body
    /// is deserialized), so admin-minting stays an admin-only capability.
    #[serde(default)]
    pub server_role: Option<ServerRole>,
}

/// An account as exposed to the admin surface. Carries no credential material:
/// the password hash is neither a field here nor selected by `list_users`.
#[derive(Serialize)]
pub struct UserEntry {
    /// Account id.
    pub id: Uuid,
    /// Account username.
    pub username: String,
    /// Server tier.
    pub server_role: ServerRole,
}

/// Create an account. Admin-only via the `AdminUser` extractor, which gates on
/// `ServerRole::Admin` alone — never on world role, which `permission_context`
/// would resolve to GM for an admin and which no world-tier grant can confer.
pub async fn create_user(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(body): Json<CreateUserRequest>,
) -> Result<Json<UserEntry>, AppError> {
    let username = validate_username(&body.username)?;
    if body.password.len() < MIN_PASSWORD_LEN {
        return Err(AppError::Unprocessable(format!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        )));
    }
    if body.password.len() > MAX_PASSWORD_BYTES {
        return Err(AppError::Unprocessable(format!(
            "password must be at most {MAX_PASSWORD_BYTES} bytes"
        )));
    }
    let server_role = body.server_role.unwrap_or(ServerRole::User);
    let hash = crate::auth::password::hash_password_async(body.password)
        .await
        .map_err(|_| AppError::Internal)?;
    // Guarded single-statement insert: a case-insensitive collision yields
    // `None` rather than a unique-constraint error surfacing as a 500.
    let id = state
        .repo
        .create_user_unique(&username, &hash, server_role, now_millis())
        .await?
        .ok_or_else(|| AppError::Conflict("username already taken".into()))?;
    Ok(Json(UserEntry {
        id,
        username,
        server_role,
    }))
}

/// Every account. Admin-only: a world GM manages world membership by username
/// and is deliberately never handed a server-wide user directory.
pub async fn list_users(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<UserEntry>>, AppError> {
    let users = state.repo.list_users().await?;
    Ok(Json(
        users
            .into_iter()
            .map(|(id, username, server_role)| UserEntry {
                id,
                username,
                server_role,
            })
            .collect(),
    ))
}

/// DELETE /api/users/{id} — server-admin only (`AdminUser`: server tier,
/// never a world-role check). Self-deletion is refused so the operation
/// never has to answer "who revokes the caller's own live session
/// mid-request" — another admin performs it; the in-tx last-admin guard
/// (repo) remains the structural backstop. After commit, live connections
/// are kicked across every room; the account's cookies died inside the same
/// transaction, so a reconnect fails authentication.
pub async fn delete_user(
    admin: AdminUser,
    State(state): State<AppState>,
    Path(target): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    if admin.0.id == target {
        return Err(AppError::Conflict("cannot delete your own account".into()));
    }
    state.repo.delete_user(target).await?;
    state.ws.rooms.evict_user(target);
    Ok(StatusCode::NO_CONTENT)
}

// --- Worlds, membership, and documents ---

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
    // Messages are server-authored via `SendMessage` only; reject
    // any client-authored message op before it reaches apply_intent.
    if crate::chat::ops_target_message(&ops) {
        return Err(AppError::Forbidden);
    }
    let room = state
        .ws
        .rooms
        .get_or_create(state.repo.as_ref(), world)
        .await?
        .ok_or(AppError::NotFound)?;
    let cmd = room
        .publish(
            state.repo.as_ref(),
            &ctx,
            ops,
            now_millis(),
            WriteOrigin::Client,
        )
        .await?;
    let world_defaults = state.repo.world_cap_defaults(world).await?;
    let current = crate::data::permission::load_update_docs(state.repo.as_ref(), &cmd).await;
    let filtered = {
        let ecs = room.scene().read().await;
        filter_command(&cmd, &ctx, &world_defaults, &current, |id| ecs.actor(id))
    };
    Ok(Json(filtered))
}

/// Resolve the caller's context and require world-GM authority (server admins
/// resolve to GM). Used to gate membership management and asset mutation.
pub(crate) async fn require_gm(
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

/// `POST /api/worlds` body.
#[derive(Deserialize)]
pub struct CreateWorldRequest {
    /// Display name for the new world.
    pub name: String,
}

/// A world the caller can access, with their effective role. The client's
/// world-select list item.
#[derive(Serialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct WorldEntry {
    /// World id.
    pub id: Uuid,
    /// World display name.
    pub name: String,
    /// The caller's effective role in it (admin sees all worlds as GM).
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

/// DELETE /api/worlds/{id} — server admin or that world's GM (`require_gm`
/// resolves admins to GM; one symbol, no authz fork). Ordering: tombstone +
/// evict the live room FIRST (no new joins, existing connections get a
/// terminal frame), then one DB transaction, then the asset directory —
/// delete convention: rows first, files second, so a crash orphans files on
/// disk rather than leaving a live world missing them. The barrier read side
/// spans the commit + dir removal so a backup never snapshots half a delete.
pub async fn delete_world(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    require_gm(&state, &user, world).await?;
    let room = state.ws.rooms.begin_delete(world);
    if let Some(room) = &room {
        room.broadcast_aux(crate::ws::protocol::ServerMsg::Evicted { user: None });
    }
    // Everything below must lift the tombstone on the way out.
    let result = async {
        let _read_permit = state.write_barrier.read().await;
        state.repo.delete_world(world).await?;
        let dir = state.config.assets_path().join(world.to_string());
        match tokio::fs::remove_dir_all(&dir).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                tracing::warn!(?e, %world, "world asset dir removal failed after row delete")
            }
        }
        Ok::<(), AppError>(())
    }
    .await;
    state.ws.rooms.finish_delete(world);
    result?;
    Ok(StatusCode::NO_CONTENT)
}

/// A roster row of `GET /api/worlds/{id}/members`.
#[derive(Serialize)]
pub struct MemberEntry {
    /// Member's user id.
    pub user: Uuid,
    /// Member's username (chat/GM UI name resolution).
    pub username: String,
    /// Member's world role.
    pub role: WorldRole,
}

/// List the world's roster. Any member may call it (chat resolves user ids to
/// names); non-members get 403 Forbidden — the world id is caller-supplied, so
/// a membership denial leaks nothing (contrast `by_id_not_found`'s by-id
/// routes, where 403-vs-404 would confirm existence).
pub async fn list_members(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
) -> Result<Json<Vec<MemberEntry>>, AppError> {
    // Any world member may list the roster: the chat card resolves user ids
    // to usernames for every viewer (author names, whisper recipient
    // labels), and a table's roster is inherently visible in shared play.
    // `permission_context` rejects non-members (Forbidden) — server admins
    // resolve to GM.
    state
        .repo
        .permission_context(world, user.id, user.role)
        .await?;
    let members = state.repo.list_members(world).await?;
    Ok(Json(
        members
            .into_iter()
            .map(|(user, username, role)| MemberEntry {
                user,
                username,
                role,
            })
            .collect(),
    ))
}

/// Identifies the target of a membership write.
///
/// The target is a user id the caller already holds — there is deliberately no
/// by-name form. Naming an arbitrary account would make this route a
/// username-existence oracle (seating on a hit is observable through
/// `list_members`), and `create_world` is open to every authenticated account,
/// so "GM-only" would bound nothing. Seating a stranger goes through
/// `create_invite`/`accept_invite`, where the invited user redeems from their
/// own session.
///
/// `role` is a [`WorldRole`] — a closed enum of `gm`/`player`/`spectator`. No
/// server-tier value is representable here, so this GM-reachable route cannot
/// express, let alone grant, a `ServerRole`.
#[derive(Deserialize)]
pub struct AddMemberRequest {
    /// The account to seat.
    pub user: Uuid,
    /// Their world role (a `WorldRole` — no server tier is representable).
    pub role: WorldRole,
}

/// Add a member or change an existing member's role (idempotent upsert).
/// `world_members.user_id` is a foreign key, so an unknown target must reject
/// as a 404 rather than surfacing as a constraint violation (a 500) —
/// `upsert_member` proves existence atomically with the write, so a user
/// deleted mid-request cannot resurface the FK path.
pub async fn add_member(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
    Json(body): Json<AddMemberRequest>,
) -> Result<StatusCode, AppError> {
    require_gm(&state, &user, world).await?;
    state
        .repo
        .upsert_member(world, body.user, body.role)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// --- World invites ---

/// Lifetime of a minted invite. An unredeemed code stops working after this
/// even if the GM forgets it exists — a bearer credential must not be
/// perpetual.
const INVITE_TTL_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// Live (unconsumed, unrevoked, unexpired) invites a single world may hold.
/// Bounds table growth from a GM minting in a loop; far above any real table.
const MAX_ACTIVE_INVITES_PER_WORLD: i64 = 64;

/// `POST /api/worlds/{id}/invites` body (GM-only route).
#[derive(Deserialize)]
pub struct CreateInviteRequest {
    /// The world role the redeemer is seated at. A [`WorldRole`], so no
    /// server tier is representable — an invite can never confer a `ServerRole`.
    pub role: WorldRole,
}

/// A freshly minted invite. `code` is the bearer credential and appears here
/// once and nowhere else: the server stores only a hash of its secret half, so
/// it cannot be re-read from the listing or recovered from the database.
#[derive(Serialize)]
pub struct MintedInvite {
    /// Invite id (the code's selector half).
    pub id: Uuid,
    /// The bearer code — shown once here, stored only as a hash.
    pub code: String,
    /// Role the redeemer is seated at.
    pub role: WorldRole,
    /// Expiry, Unix epoch milliseconds.
    pub expires_at: i64,
}

/// An invite as the minting GM's listing sees it. Carries no credential
/// material — neither the code nor its hash is a field here.
#[derive(Serialize)]
pub struct InviteEntry {
    /// Invite id.
    pub id: Uuid,
    /// Role it seats at.
    pub role: WorldRole,
    /// Mint time, Unix epoch milliseconds.
    pub created_at: i64,
    /// Expiry, Unix epoch milliseconds.
    pub expires_at: i64,
    /// Set when revoked (listing context only; the redemption gate lives in
    /// `consume_invite`'s guarded UPDATE).
    pub revoked_at: Option<i64>,
    /// Set when redeemed (listing context only).
    pub consumed_at: Option<i64>,
}

impl From<crate::data::sqlite::InviteRecord> for InviteEntry {
    fn from(r: crate::data::sqlite::InviteRecord) -> Self {
        InviteEntry {
            id: r.id,
            role: r.role,
            created_at: r.created_at,
            expires_at: r.expires_at,
            revoked_at: r.revoked_at,
            consumed_at: r.consumed_at,
        }
    }
}

/// Mint a single-use invite for this world. GM of THIS world only.
pub async fn create_invite(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
    Json(body): Json<CreateInviteRequest>,
) -> Result<Json<MintedInvite>, AppError> {
    require_gm(&state, &user, world).await?;
    let minted = crate::auth::invite::mint().map_err(|_| AppError::Internal)?;
    let now = now_millis();
    let expires_at = now.saturating_add(INVITE_TTL_MS);
    let stored = state
        .repo
        .create_invite(
            crate::data::sqlite::NewInvite {
                id: minted.id,
                world,
                secret_hash: &minted.secret_hash,
                role: body.role,
                created_by: user.id,
                now,
                expires_at,
            },
            MAX_ACTIVE_INVITES_PER_WORLD,
        )
        .await?;
    if !stored {
        return Err(AppError::Conflict(format!(
            "too many active invites (max {MAX_ACTIVE_INVITES_PER_WORLD}); revoke one first"
        )));
    }
    Ok(Json(MintedInvite {
        id: minted.id,
        code: minted.code,
        role: body.role,
        expires_at,
    }))
}

/// This world's invites. GM of THIS world only; codes are not recoverable here.
pub async fn list_invites(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
) -> Result<Json<Vec<InviteEntry>>, AppError> {
    require_gm(&state, &user, world).await?;
    let invites = state.repo.list_invites(world).await?;
    Ok(Json(invites.into_iter().map(InviteEntry::from).collect()))
}

/// Revoke an invite, effective immediately (redemption re-checks revocation in
/// its consume statement). GM of THIS world only, and the revoke is scoped to
/// the world in SQL — a GM of another world gets the same 404 for an id that
/// exists elsewhere as for one that exists nowhere.
pub async fn revoke_invite(
    user: AuthUser,
    State(state): State<AppState>,
    Path((world, code_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    require_gm(&state, &user, world).await?;
    if !state
        .repo
        .revoke_invite(world, code_id, now_millis())
        .await?
    {
        return Err(AppError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/invites/accept` body.
#[derive(Deserialize)]
pub struct AcceptInviteRequest {
    /// The bearer code. Carried in the BODY, never the path: a request line is
    /// recorded by `TraceLayer`'s span (`uri`, at DEBUG), browser history,
    /// `Referer`, and proxy access logs, none of which this credential may
    /// reach in plaintext — it is hashed at rest for the same reason.
    pub code: String,
}

/// Redeem an invite: seat the CALLER into the invite's world at the invite's
/// role. Any authenticated user; the code is the only authorization.
///
/// Every rejection — malformed, unknown, wrong secret, expired, revoked,
/// already consumed — returns exactly `AppError::NotFound` after exactly one
/// Argon2 verify, so the failures are indistinguishable by status, body, or
/// timing. Nothing about a world the caller holds no valid code for is
/// disclosed: the world's identity is read inside the consume transaction and
/// returned only on success.
pub async fn accept_invite(
    user: AuthUser,
    State(state): State<AppState>,
    ip: crate::http::throttle::ClientIp,
    Json(body): Json<AcceptInviteRequest>,
) -> Result<Json<WorldEntry>, AppError> {
    use crate::auth::invite;
    use crate::http::throttle::{INVITE_PER_MIN_PER_ACCOUNT, INVITE_PER_MIN_PER_IP};
    let now = now_millis();
    let per_account = state
        .config
        .invite_per_min_per_account
        .unwrap_or(INVITE_PER_MIN_PER_ACCOUNT);
    let per_ip = state
        .config
        .invite_per_min_per_ip
        .unwrap_or(INVITE_PER_MIN_PER_IP);
    let ip_ok = match ip.0 {
        Some(addr) => state
            .auth_throttle
            .check(&format!("invite:ip:{addr}"), now, per_ip),
        None => true,
    };
    if !ip_ok
        || !state
            .auth_throttle
            .check(&format!("invite:u:{}", user.id), now, per_account)
    {
        return Err(AppError::TooManyRequests("try again later".into()));
    }

    let code = body.code;
    let parsed = invite::parse(&code);
    let record = match parsed {
        Some((id, _)) => state.repo.invite_by_id(id).await?,
        None => None,
    };
    // Exactly one verify on every path — against the stored hash when a row
    // was found, else a throwaway hash — mirroring `anti_enumeration_phc` on
    // the login path.
    let (secret, target) = match (&parsed, &record) {
        (Some((_, secret)), Some(rec)) => (secret.clone(), rec.secret_hash.clone()),
        _ => (
            invite::DUMMY_SECRET.to_owned(),
            invite::dummy_phc().to_owned(),
        ),
    };
    let verified = verify_password_async(secret, target).await;

    let Some(rec) = record.filter(|_| verified) else {
        return Err(AppError::NotFound);
    };
    // Expiry, revocation, and single-use are decided HERE, by one guarded
    // statement — never by a preceding read of `rec` (that would be both a
    // TOCTOU double-seat and a second, distinguishable failure shape). The
    // world's name comes back from inside the same transaction: reading it
    // afterwards could 404 on an already-burned invite, reporting the uniform
    // failure for a redemption that in fact succeeded.
    let Some(seated) = state
        .repo
        .consume_invite(rec.id, user.id, now_millis())
        .await?
    else {
        return Err(AppError::NotFound);
    };
    Ok(Json(WorldEntry {
        id: seated.world,
        name: seated.world_name,
        role: seated.role,
    }))
}

/// Unseat a member. GM-only (`require_gm`); refuses to remove the last GM
/// (surfaces the repository's Conflict).
pub async fn remove_member(
    user: AuthUser,
    State(state): State<AppState>,
    Path((world, target)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    require_gm(&state, &user, world).await?;
    state.repo.remove_member(world, target).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Create a document over HTTP: same authoritative write path as a WS intent
/// (`write_ops` -> `apply_intent`), same authz and validation.
pub async fn create_document(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
    Json(doc): Json<Document>,
) -> Result<Json<Command>, AppError> {
    write_ops(&state, &user, world, vec![Operation::Create { doc }]).await
}

/// Query string of `GET /api/worlds/{id}/documents`.
#[derive(Deserialize)]
pub struct DocQuery {
    /// The doc_type to list.
    pub r#type: String,
}

/// List documents of one type, per-recipient filtered (READ-gated per doc,
/// properties redacted via `filter_properties`; tokens join effective owners).
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
    // Every result shares `q.type`, so resolve the type-scoped grants once.
    let world_grants = world_defaults.grants_for(&q.r#type);
    let docs = state.repo.query_documents(world, &q.r#type).await?;
    // One batched actor fetch when listing tokens; the per-doc join is then
    // in-memory (token_actor_link returns None for every other doc_type, so the
    // map is simply unused there).
    let actors: std::collections::HashMap<Uuid, Document> = if q.r#type == "token" {
        state
            .repo
            .query_documents(world, "actor")
            .await?
            .into_iter()
            .map(|a| (a.id, a))
            .collect()
    } else {
        std::collections::HashMap::new()
    };
    let visible = docs
        .into_iter()
        .filter_map(|d| {
            let owner = crate::data::permission::effective_owner_via(&d, &|id| actors.get(id));
            let access =
                resolve_access_world(ctx.user_id, ctx.world_role, &d, &world_grants, owner);
            access
                .has(cap::READ)
                .then(|| filter_properties(&d, &access))
        })
        .collect();
    Ok(Json(visible))
}

/// On by-id document routes a non-member's `Forbidden` is remapped to `NotFound`:
/// a 403-vs-404 split would let a non-member confirm a document id exists.
/// World-scoped routes keep `Forbidden` — the world id is supplied by the caller,
/// so a membership denial there leaks nothing.
fn by_id_not_found(e: crate::data::DataError) -> AppError {
    match e {
        crate::data::DataError::Forbidden => AppError::NotFound,
        other => other.into(),
    }
}

/// Fetch one document by id: world derived from the doc (`world_of`),
/// READ-gated, redacted via `filter_properties`; absent and forbidden are both
/// 404 (existence-hiding).
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
    // 404 for a compendium document (`world_of` returns `None` for one) —
    // same NotFound this route already returns for a missing doc, existence
    // hiding via a uniform not-found rather than a distinct error shape.
    let world = crate::data::document::world_of(&doc).ok_or(AppError::NotFound)?;
    let ctx = state
        .repo
        .permission_context(world, user.id, user.role)
        .await
        .map_err(by_id_not_found)?;
    let world_defaults = state.repo.world_cap_defaults(world).await?;
    let owner = state.repo.effective_owner_of(&doc).await?;
    let access = resolve_access_world(
        ctx.user_id,
        ctx.world_role,
        &doc,
        &world_defaults.grants_for(&doc.doc_type),
        owner,
    );
    if !access.has(cap::READ) {
        return Err(AppError::NotFound);
    }
    Ok(Json(filter_properties(&doc, &access)))
}

/// `PATCH /api/documents/{id}` body.
#[derive(Deserialize)]
pub struct PatchRequest {
    /// Field-level changes, each with its OCC pre-image.
    pub changes: Vec<FieldChange>,
}

/// Field-level update by id: same authoritative write path as a WS intent;
/// world derived from the doc; refusals surface as 404/409/422 per `AppError`.
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
    // 404 for a compendium document (`world_of` returns `None` for one) —
    // same NotFound this route already returns for a missing doc, existence
    // hiding via a uniform not-found rather than a distinct error shape.
    let world = crate::data::document::world_of(&doc).ok_or(AppError::NotFound)?;
    // by-id route: a non-member is 404, not 403 (existence hiding). Members fall
    // through to write_ops, which may still 403 on a capability denial.
    state
        .repo
        .permission_context(world, user.id, user.role)
        .await
        .map_err(by_id_not_found)?;
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

/// Delete by id through the authoritative write path (carries the full
/// pre-image for invertibility; cascades handled by `delete_document_tx`).
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
    // 404 for a compendium document (`world_of` returns `None` for one) —
    // same NotFound this route already returns for a missing doc, existence
    // hiding via a uniform not-found rather than a distinct error shape.
    let world = crate::data::document::world_of(&doc).ok_or(AppError::NotFound)?;
    // by-id route: a non-member is 404, not 403 (existence hiding).
    state
        .repo
        .permission_context(world, user.id, user.role)
        .await
        .map_err(by_id_not_found)?;
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

/// Structural validation of a capability-defaults payload: every grant token
/// must be a namespaced `<ns>:<verb>` string within length bounds.
///
/// # Examples
///
/// ```text
/// validate_grants(&defaults)?; // rejects "not-namespaced" tokens with 422
/// ```
fn validate_grants(defaults: &WorldCapDefaults) -> Result<(), AppError> {
    // Per-document grants (all + every doc-type override).
    for g in std::iter::once(&defaults.all).chain(defaults.by_type.values()) {
        for set in g.by_role.values().chain(g.by_user.values()) {
            for token in set {
                validate_capability(token)?;
            }
        }
    }
    // World-level role_caps (all + every doc-type override).
    let role_sets = defaults
        .role_caps
        .all
        .values()
        .chain(defaults.role_caps.by_type.values().flat_map(|m| m.values()));
    for set in role_sets {
        for token in set {
            validate_capability(token)?;
        }
    }
    Ok(())
}

/// A world's capability configuration. GM/admin only.
pub async fn get_world_capability_defaults(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
) -> Result<Json<WorldCapDefaults>, AppError> {
    require_gm(&state, &user, world).await?;
    Ok(Json(state.repo.world_cap_defaults(world).await?))
}

/// Replace a world's capability configuration. GM/admin only.
pub async fn set_world_capability_defaults(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
    Json(defaults): Json<WorldCapDefaults>,
) -> Result<StatusCode, AppError> {
    require_gm(&state, &user, world).await?;
    validate_grants(&defaults)?;
    state.repo.set_world_cap_defaults(world, &defaults).await?;
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
    let mut seen_modules: HashSet<&str> = HashSet::new();
    let mut cardinality_of: HashMap<&str, Cardinality> = HashMap::new();
    let mut provider_count: HashMap<&str, usize> = HashMap::new();
    for d in decls {
        if d.module_id.is_empty() || d.version.is_empty() {
            return Err(AppError::Unprocessable(
                "declaration module_id and version must be non-empty".into(),
            ));
        }
        // A module appears once: two declarations for one module_id is an
        // ambiguous topology (which version/provides wins).
        if !seen_modules.insert(d.module_id.as_str()) {
            return Err(AppError::Unprocessable(format!(
                "duplicate module_id '{}'",
                d.module_id
            )));
        }
        for p in &d.provides {
            validate_contract_token(&p.contract)?;
            provided.insert(p.contract.as_str());
            // A contract's cardinality must be consistent across all providers:
            // one module asserting singleton while another asserts multi is a
            // contradiction the consistency authority rejects.
            match cardinality_of.get(p.contract.as_str()) {
                Some(prev) if *prev != p.cardinality => {
                    return Err(AppError::Unprocessable(format!(
                        "contract '{}' declared with conflicting cardinalities",
                        p.contract
                    )));
                }
                _ => {
                    cardinality_of.insert(p.contract.as_str(), p.cardinality);
                }
            }
            *provider_count.entry(p.contract.as_str()).or_insert(0) += 1;
        }
    }
    // A singleton contract must have exactly one provider (catches two singletons
    // and the mixed singleton/multi case via the cardinality check above).
    for (contract, card) in &cardinality_of {
        if *card == Cardinality::Singleton && provider_count[contract] > 1 {
            return Err(AppError::Unprocessable(format!(
                "contract '{contract}' is singleton but provided more than once"
            )));
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

/// Upper bound on schema declarations stored per world (parsed on every write,
/// broadcast in `Welcome`). Sized like `MAX_CONTRACT_DECLARATIONS`.
const MAX_SCHEMA_DECLARATIONS: usize = 256;
/// Upper bound on nodes in a single schema type-tree (backstops a pathological
/// declaration; the size cap already bounds the DATA, this bounds the SCHEMA).
const MAX_SCHEMA_NODES: usize = 512;
/// Upper bound on schema type-tree nesting depth.
const MAX_SCHEMA_DEPTH: usize = 16;
/// The single schema-format vocabulary version this server understands (F7). A
/// future vocabulary bump increments this and the set endpoint rejects formats
/// it does not know, so a format bump can never be silently half-enforced.
const SCHEMA_FORMAT_V1: u32 = 1;

/// Structurally validate one schema type-tree node, fail-closed:
/// bounded depth/node-count and cross-field legality (a node's non-`type` keys
/// must match its `type`). serde's `deny_unknown_fields` already rejected unknown
/// keys and bad `type` values at deserialize; this rejects a well-typed-but-
/// nonsensical node (e.g. `items` on an object, `properties` on an array).
fn validate_schema(s: &Schema, depth: usize, budget: &mut usize) -> Result<(), AppError> {
    if depth > MAX_SCHEMA_DEPTH {
        return Err(AppError::Unprocessable(format!(
            "schema nesting exceeds max depth {MAX_SCHEMA_DEPTH}"
        )));
    }
    if *budget == 0 {
        return Err(AppError::Unprocessable(format!(
            "schema exceeds max node count {MAX_SCHEMA_NODES}"
        )));
    }
    *budget -= 1;

    let has_object_keys =
        s.properties.is_some() || s.required.is_some() || s.additional_properties.is_some();
    let has_array_keys = s.items.is_some();

    match s.ty {
        None => {
            // `{}` (any): must carry no other constraint keys.
            if has_object_keys || has_array_keys || s.nullable.is_some() {
                return Err(AppError::Unprocessable(
                    "a typeless schema node ({}) must have no other keys".into(),
                ));
            }
        }
        Some(SchemaType::Object) => {
            if has_array_keys {
                return Err(AppError::Unprocessable(
                    "'items' is only valid on an array schema".into(),
                ));
            }
            if let Some(props) = &s.properties {
                for sub in props.values() {
                    validate_schema(sub, depth + 1, budget)?;
                }
            }
            if let Some(AdditionalProperties::Schema(sub)) = &s.additional_properties {
                validate_schema(sub, depth + 1, budget)?;
            }
        }
        Some(SchemaType::Array) => {
            if has_object_keys {
                return Err(AppError::Unprocessable(
                    "'properties'/'required'/'additionalProperties' are only valid on an object schema"
                        .into(),
                ));
            }
            if let Some(items) = &s.items {
                validate_schema(items, depth + 1, budget)?;
            }
        }
        Some(SchemaType::String | SchemaType::Number | SchemaType::Boolean | SchemaType::Null) => {
            if has_object_keys || has_array_keys {
                return Err(AppError::Unprocessable(
                    "a scalar schema node accepts only 'type' and 'nullable'".into(),
                ));
            }
        }
    }
    Ok(())
}

/// Validate a world's schema declaration set at set-time,
/// fail-closed — the server is the consistency authority. Mirrors
/// `validate_contract_declarations` on bounded count and non-empty
/// module_id/version, but a `SchemaDeclaration` is NOT a per-module bundle
/// (unlike `ContractDeclaration`, which carries `provides`/`requires` and so
/// is naturally one-per-module) — it is a single `(doc_type, subtree_pointer,
/// schema)` triple, and one module legitimately needs several of them (e.g.
/// separate declarations for `/system/stats` and `/system/mechanics` under
/// the same `module_id`). So `module_id` is intentionally NOT required to be
/// unique across the batch; ambiguity is instead prevented by the
/// per-doc_type pointer-overlap check below, understood schema_format, strict
/// `/system/…` pointers, and each schema a well-formed type-tree.
fn validate_schema_declarations(decls: &[SchemaDeclaration]) -> Result<(), AppError> {
    if decls.len() > MAX_SCHEMA_DECLARATIONS {
        return Err(AppError::Unprocessable(format!(
            "too many schema declarations (max {MAX_SCHEMA_DECLARATIONS})"
        )));
    }
    for d in decls {
        if d.module_id.is_empty() || d.version.is_empty() {
            return Err(AppError::Unprocessable(
                "schema declaration module_id and version must be non-empty".into(),
            ));
        }
        if d.schema_format != SCHEMA_FORMAT_V1 {
            return Err(AppError::Unprocessable(format!(
                "unsupported schema_format {} (server understands {SCHEMA_FORMAT_V1})",
                d.schema_format
            )));
        }
        // Pointer must be a strict `/system/…` descendant: guards content only,
        // never the whole `system` body (bare `/system` re-introduces the
        // deny_unknown_fields-on-system problem the band split avoids), and never
        // `/engine`, `/permissions`, `/name`, or the envelope. Mirrors the
        // `set_world_cap_requirements` writable-namespace check, narrowed.
        let p = &d.subtree_pointer;
        if !p.starts_with("/system/") || p.ends_with('/') || p.contains("//") {
            return Err(AppError::Unprocessable(format!(
                "subtree_pointer '{p}' must be a strict /system/… descendant"
            )));
        }
        let mut budget = MAX_SCHEMA_NODES;
        validate_schema(&d.schema, 0, &mut budget)?;
    }
    // No two entries for one doc_type whose pointers are equal or where one is a
    // prefix of the other (ambiguous authority: which schema governs the nested
    // value?). Prefix test: `a` covers `b` iff `b == a` or `b` starts with
    // `a` + "/".
    for (i, a) in decls.iter().enumerate() {
        for b in &decls[i + 1..] {
            if a.doc_type != b.doc_type {
                continue;
            }
            let (x, y) = (&a.subtree_pointer, &b.subtree_pointer);
            let overlaps =
                x == y || y.starts_with(&format!("{x}/")) || x.starts_with(&format!("{y}/"));
            if overlaps {
                return Err(AppError::Unprocessable(format!(
                    "overlapping schema pointers '{x}' and '{y}' for doc_type '{}'",
                    a.doc_type
                )));
            }
        }
    }
    Ok(())
}

/// A world's structural schema declarations. GM/admin only.
pub async fn get_world_schema_declarations(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
) -> Result<Json<Vec<SchemaDeclaration>>, AppError> {
    require_gm(&state, &user, world).await?;
    Ok(Json(state.repo.world_schema_declarations(world).await?))
}

/// Replace a world's structural schema declarations. GM/admin only; validated.
pub async fn set_world_schema_declarations(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
    Json(decls): Json<Vec<SchemaDeclaration>>,
) -> Result<StatusCode, AppError> {
    require_gm(&state, &user, world).await?;
    validate_schema_declarations(&decls)?;
    state
        .repo
        .set_world_schema_declarations(world, &decls)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
