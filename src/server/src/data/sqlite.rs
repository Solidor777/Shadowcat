// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use async_trait::async_trait;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::auth::role::ServerRole;
use crate::data::command::{
    apply_field_change, Command, FieldChange, Operation, UnsequencedCommand, WriteOrigin,
};
use crate::data::document::{
    world_of, CapabilityRequirement, ContractDeclaration, Document, SchemaDeclaration, Scope,
    World, WorldCapDefaults, WorldRole,
};
use crate::data::engine::{
    CombatEngine, COMBATANT_DOC_TYPE, COMBAT_DOC_TYPE, COMBAT_HISTORY_DOC_TYPE,
    CONDITION_REGISTRY_DOC_TYPE, FACTION_REGISTRY_DOC_TYPE, RESOURCE_REGISTRY_DOC_TYPE,
    SYSTEM_DEFAULTS_DOC_TYPE, WORLD_SETTINGS_DOC_TYPE,
};
use crate::data::membership::PermissionContext;
use crate::data::permission::{
    cap, carried_light_in_body, carried_light_touched, declared_caps_for_document,
    declared_caps_for_path, required_cap_for_path, resolve_access_world, Access,
};
use crate::data::repository::Repository;
use crate::data::snapshot::{CommandSnapshot, StoredCommand};
use crate::data::validation;
use crate::data::world_bundle::{
    BundleManifest, ExportedAssetRow, ExportedDocumentRow, ExportedEventRow, ExportedFogRow,
    ExportedInviteRow, ExportedMemberRow, ExportedSettingRow, ImportSummary, WorldExportData,
    WorldImportData, BUNDLE_SCHEMA_VERSION,
};
use crate::data::DataError;

/// Doc_types capped at one document per world. Checked (transactionally,
/// alongside the existing-id conflict check) at the `apply_intent` Create
/// chokepoint — a stray second singleton doc would otherwise resolve
/// nondeterministically-but-safely via lowest-UUID ordering (see
/// `chat::settings::resolve_content_policy`'s doc comment); this closes that
/// gap at construction time rather than leaving it to read-side tolerance.
const SINGLETON_DOC_TYPES: &[&str] = &[
    WORLD_SETTINGS_DOC_TYPE,
    FACTION_REGISTRY_DOC_TYPE,
    CONDITION_REGISTRY_DOC_TYPE,
    RESOURCE_REGISTRY_DOC_TYPE,
    SYSTEM_DEFAULTS_DOC_TYPE,
    crate::chat::CHAT_SETTINGS_DOC_TYPE,
    crate::chat::DICE_SETTINGS_DOC_TYPE,
    crate::data::engine::CHANNEL_REGISTRY_DOC_TYPE,
    crate::data::engine::LIGHT_GRADATION_DOC_TYPE,
    crate::data::engine::VISION_MODES_DOC_TYPE,
];

/// Auth-facing projection of a user row.
///
/// # Examples
///
/// ```
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::data::sqlite::UserRecord;
///
/// let record = UserRecord {
///     id: uuid::Uuid::nil(),
///     username: "MOCK_USER".into(),
///     password_hash: None,
///     server_role: ServerRole::User,
/// };
/// assert_eq!(record.username, "MOCK_USER");
/// ```
#[derive(Debug, Clone)]
pub struct UserRecord {
    /// Account id.
    pub id: Uuid,
    /// Unique login name.
    pub username: String,
    /// Argon2 PHC string; `None` = login disabled (e.g. seeded fixture accounts).
    pub password_hash: Option<String>,
    /// Server tier (admin/user), orthogonal to any per-world role.
    pub server_role: ServerRole,
}

/// A world invite as stored. `secret_hash` is an Argon2 PHC string over the
/// code's verifier half; the code itself is never stored. The lifecycle
/// columns are read-only context for the GM's listing — they are NOT the
/// redemption gate, which lives entirely in `consume_invite`'s single guarded
/// UPDATE (see [[two-query-guard-needs-tx]]).
///
/// # Examples
///
/// ```
/// use shadowcat::data::document::WorldRole;
/// use shadowcat::data::sqlite::InviteRecord;
///
/// let invite = InviteRecord {
///     id: uuid::Uuid::nil(),
///     world_id: uuid::Uuid::nil(),
///     secret_hash: "mock-hash".into(),
///     role: WorldRole::Player,
///     created_at: 0,
///     expires_at: 1_000,
///     revoked_at: None,
///     consumed_at: None,
/// };
/// assert!(invite.consumed_at.is_none());
/// ```
#[derive(Debug, Clone)]
pub struct InviteRecord {
    /// Selector half of the invite code (also the row id).
    pub id: Uuid,
    /// World the invite seats into.
    pub world_id: Uuid,
    /// Argon2 PHC string over the code's verifier half; the code is never stored.
    pub secret_hash: String,
    /// Role granted on redemption (for a NEW member; standing is never changed).
    pub role: WorldRole,
    /// Mint time, Unix epoch milliseconds.
    pub created_at: i64,
    /// Expiry, Unix epoch milliseconds.
    pub expires_at: i64,
    /// Set when a GM revokes the invite (listing context only).
    pub revoked_at: Option<i64>,
    /// Set when redeemed (listing context only).
    pub consumed_at: Option<i64>,
}

/// The outcome of a successful redemption: the world the caller is now a member
/// of and the role they actually hold there (which is their PRE-EXISTING role
/// when they were already a member — redemption grants access, never changes
/// standing). Every field is read inside `consume_invite`'s transaction.
///
/// # Examples
///
/// ```
/// use shadowcat::data::document::WorldRole;
/// use shadowcat::data::sqlite::SeatedByInvite;
///
/// let seated = SeatedByInvite {
///     world: uuid::Uuid::nil(),
///     world_name: "MOCK_WORLD".into(),
///     role: WorldRole::Player,
/// };
/// assert_eq!(seated.role, WorldRole::Player);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatedByInvite {
    /// The world the caller is now a member of.
    pub world: Uuid,
    /// Its display name (for the redemption response).
    pub world_name: String,
    /// The role they hold there (pre-existing role if already a member).
    pub role: WorldRole,
}

/// The fields of an invite row at mint time.
///
/// # Examples
///
/// ```
/// use shadowcat::data::document::WorldRole;
/// use shadowcat::data::sqlite::NewInvite;
///
/// let invite = NewInvite {
///     id: uuid::Uuid::nil(),
///     world: uuid::Uuid::nil(),
///     secret_hash: "mock-hash",
///     role: WorldRole::Player,
///     created_by: uuid::Uuid::nil(),
///     now: 0,
///     expires_at: 1_000,
/// };
/// assert_eq!(invite.secret_hash, "mock-hash");
/// ```
pub struct NewInvite<'a> {
    /// Selector half of the minted code — the row id and the code must agree.
    pub id: Uuid,
    /// World the invite is for.
    pub world: Uuid,
    /// Argon2 PHC string over the code's verifier half.
    pub secret_hash: &'a str,
    /// Role a new member is seated with.
    pub role: WorldRole,
    /// Minting GM's user id.
    pub created_by: Uuid,
    /// Mint time, Unix epoch milliseconds.
    pub now: i64,
    /// Expiry, Unix epoch milliseconds.
    pub expires_at: i64,
}

/// The `documents` row's scope/source column tuple `document_row_columns`
/// derives from a `Document` envelope: `(scope_kind, world_id, pack,
/// source_id, source_pack, source_version)`.
type DocumentRowColumns = (
    &'static str,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
);

/// SQLite-backed storage. Holds a connection pool; migrations are embedded
/// from `migrations/` and run at connect time.
///
/// # Examples
///
/// ```
/// # #[tokio::main]
/// # async fn main() -> Result<(), shadowcat::data::DataError> {
/// use shadowcat::data::repository::Repository;
/// use shadowcat::data::sqlite::SqliteRepository;
/// let repo = SqliteRepository::connect("sqlite::memory:").await?;
/// assert!(repo.get_world(uuid::Uuid::nil()).await?.is_none());
/// # Ok(())
/// # }
/// ```
pub struct SqliteRepository {
    /// Single-connection pool: the one writer serializing every transaction.
    pool: SqlitePool,
    /// The connect options `pool` was opened from — cloned to open a second
    /// pool against the identical database (see `open_read_pool`); never
    /// re-derive this by re-parsing a URL string (see
    /// `crate::db::parse_connect_options`'s doc for why).
    connect_options: sqlx::sqlite::SqliteConnectOptions,
    /// Installed-modules discovery root, `None` when this repository was never wired to one
    /// (every existing test construction via `connect()` alone) — validators never run
    /// without it, fail-open by absence exactly like `scan_installed_modules`'s own missing-
    /// dir handling.
    modules_dir: Option<std::path::PathBuf>,
    /// Compiled validator cache, mirroring `crate::modules::ModuleScanCache`'s own
    /// invalidation. Always present (cheap to construct; does no I/O until first scan).
    validator_registry_cache: std::sync::Arc<crate::sandbox::registry::ValidatorRegistryCache>,
}

impl SqliteRepository {
    /// Connect to `url` (e.g. "sqlite::memory:" or "sqlite:///path/to.db")
    /// and run migrations. `url` is parsed once via
    /// [`crate::db::parse_connect_options`] and the resulting options open
    /// the pool through [`crate::db::connect_pool_with_options`] — never
    /// restated here.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// // Migrations already ran: the `worlds` table exists and is empty.
    /// assert!(repo.get_world(uuid::Uuid::nil()).await?.is_none());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn connect(url: &str) -> Result<Self, DataError> {
        let connect_options = crate::db::parse_connect_options(url)?;
        let pool = crate::db::connect_pool_with_options(connect_options.clone()).await?;
        sqlx::migrate!()
            .run(&pool)
            .await
            .map_err(sqlx::Error::from)?;
        Ok(Self {
            pool,
            connect_options,
            modules_dir: None,
            validator_registry_cache: Default::default(),
        })
    }

    /// The underlying pool, for callers that run their own queries (tests,
    /// one-shot admin paths).
    ///
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let row: (i64,) = sqlx::query_as("SELECT 1").fetch_one(repo.pool()).await?;
    /// assert_eq!(row.0, 1);
    /// # Ok(())
    /// # }
    /// ```
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Opens a second, read-only pool against the same database `pool`
    /// writes through — see [`crate::db::open_read_only_pool`]. For a
    /// `sqlite::memory:`-backed repository this shares the SAME generated
    /// in-memory database [`Self::connect`] opened, never a fresh, empty one.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let read_pool = repo.open_read_pool().await?;
    /// let row: (i64,) = sqlx::query_as("SELECT 1").fetch_one(&read_pool).await?;
    /// assert_eq!(row.0, 1);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn open_read_pool(&self) -> Result<SqlitePool, sqlx::Error> {
        crate::db::open_read_only_pool(self.connect_options.clone()).await
    }

    /// Attaches an installed-modules discovery root, enabling sandboxed validator support
    /// on this repository. Every existing `connect()` caller that never calls this keeps
    /// `modules_dir: None` — validators never run, exactly as if none were installed.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:")
    ///     .await?
    ///     .with_modules_dir("no-such-modules-dir");
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_modules_dir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.modules_dir = Some(dir.into());
        self
    }

    /// The compiled validator registry for `modules_dir`, or an empty registry when the
    /// scan finds nothing. Off the async worker via `spawn_blocking`, matching every other
    /// blocking module-scan call site in this crate; used by
    /// `http::module_routes::list_installed_modules`'s validator-status projection.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let registry = repo
    ///     .validator_registry(std::path::Path::new("no-such-modules-dir"))
    ///     .await;
    /// assert!(registry.validator_for("example-module", "actor").is_none());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn validator_registry(
        &self,
        modules_dir: &std::path::Path,
    ) -> std::sync::Arc<crate::sandbox::registry::ValidatorRegistry> {
        let cache = self.validator_registry_cache.clone();
        let dir = modules_dir.to_path_buf();
        tokio::task::spawn_blocking(move || cache.get_or_scan(&dir))
            .await
            .unwrap_or_default()
    }

    /// See `Repository::get_link_preview_cache`.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// assert!(repo.get_link_preview_cache("https://example.com").await?.is_none());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_link_preview_cache(
        &self,
        url: &str,
    ) -> Result<Option<crate::data::repository::LinkPreviewCacheRow>, DataError> {
        let row = sqlx::query(
            "SELECT title, description, image_asset_id, fetched_at FROM link_preview_cache WHERE url = ?",
        )
        .bind(url)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else { return Ok(None) };
        let fetched_at_raw: String = row.get("fetched_at");
        let fetched_at_ms = fetched_at_raw
            .parse::<i64>()
            .map_err(|e| DataError::OpFailed(e.to_string()))?;
        let image_asset_id = row
            .get::<Option<String>, _>("image_asset_id")
            .map(|s| Uuid::parse_str(&s).map_err(|e| DataError::OpFailed(e.to_string())))
            .transpose()?;
        Ok(Some(crate::data::repository::LinkPreviewCacheRow {
            title: row.get("title"),
            description: row.get("description"),
            image_asset_id,
            fetched_at_ms,
        }))
    }

    /// See `Repository::upsert_link_preview_cache`.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// repo.upsert_link_preview_cache("https://example.com", Some("MOCK_TITLE"), None, 0).await?;
    /// let row = repo.get_link_preview_cache("https://example.com").await?.unwrap();
    /// assert_eq!(row.title.as_deref(), Some("MOCK_TITLE"));
    /// # Ok(())
    /// # }
    /// ```
    pub async fn upsert_link_preview_cache(
        &self,
        url: &str,
        title: Option<&str>,
        description: Option<&str>,
        fetched_at_ms: i64,
    ) -> Result<(), DataError> {
        sqlx::query(
            "INSERT INTO link_preview_cache (url, title, description, fetched_at) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT(url) DO UPDATE SET \
               title = excluded.title, description = excluded.description, fetched_at = excluded.fetched_at",
        )
        .bind(url)
        .bind(title)
        .bind(description)
        .bind(fetched_at_ms.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// See `Repository::set_link_preview_cache_image`.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::asset::{Asset, AssetMeta};
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let world = repo.create_world("w", 0).await?;
    /// let asset = uuid::Uuid::new_v4();
    /// repo.insert_asset(&Asset {
    ///     id: asset,
    ///     world_id: world.id,
    ///     storage_key: format!("{}/{asset}", world.id),
    ///     original_name: "preview.png".into(),
    ///     content_type: "image/webp".into(),
    ///     byte_size: 10,
    ///     created_by: None,
    ///     created_at: 0,
    ///     version: 1,
    ///     folder_id: None,
    ///     tags: vec![],
    ///     derived_tags: vec![],
    ///     meta: AssetMeta::unprocessed("image/png", 10),
    /// })
    /// .await?;
    /// repo.upsert_link_preview_cache("https://example.com", None, None, 0).await?;
    /// repo.set_link_preview_cache_image("https://example.com", asset).await?;
    /// let row = repo.get_link_preview_cache("https://example.com").await?.unwrap();
    /// assert_eq!(row.image_asset_id, Some(asset));
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_link_preview_cache_image(
        &self,
        url: &str,
        image_asset_id: Uuid,
    ) -> Result<(), DataError> {
        sqlx::query("UPDATE link_preview_cache SET image_asset_id = ? WHERE url = ?")
            .bind(image_asset_id.to_string())
            .bind(url)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

/// A world-sequenced command may only carry documents scoped to its own world.
/// A foreign scope would file the row outside this world's seq stream, making it
/// unreachable by `events_since` for either world and breaking replay scoping.
fn check_command_scope(doc: &Document, world_id: Uuid) -> Result<(), DataError> {
    match &doc.scope {
        Scope::World { world_id: w } if *w == world_id => Ok(()),
        _ => Err(DataError::OpFailed(
            "document scope does not match the command's world".into(),
        )),
    }
}

/// Phase 1's Create authorization, resolved per op through ONE function both the
/// in-transaction Phase-1 arm and `apply_intent`'s pre-transaction validator screen
/// call — authz is the codebase's never-fork class, so the screen never re-spells
/// any of these checks. Returns the resolved `Access` (the screen additionally reads
/// it for the validator `prior` band's READ gate). `executor` is the write
/// transaction for Phase 1, a read-only pool connection for the screen — both only
/// ever feed `load_effective_owner`.
async fn authorize_create_intent<'e, E>(
    executor: E,
    ctx: &PermissionContext,
    doc: &Document,
    origin: WriteOrigin,
    world_defaults: &crate::data::document::WorldCapDefaults,
    world_reqs: &[crate::data::document::CapabilityRequirement],
) -> Result<Access, DataError>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    // `system-defaults` is server-authored: its content mirrors the installed
    // system package's declaration, so every client-reachable origin is rejected
    // outright — `WriteOrigin::ConfigSeed` (the world-config seed/refresh path) is
    // the ONLY origin that may author it.
    if doc.doc_type == SYSTEM_DEFAULTS_DOC_TYPE && origin != WriteOrigin::ConfigSeed {
        return Err(DataError::Forbidden);
    }
    let create_owner = SqliteRepository::load_effective_owner(executor, doc).await?;
    let access = resolve_access_world(
        ctx.user_id,
        ctx.world_role,
        doc,
        &world_defaults.grants_for(&doc.doc_type),
        create_owner,
    );
    // A capability-skipping server-authored origin (`WriteOrigin::
    // skips_capability_gates`) has already been vetted by its own trusted caller,
    // so the ordinary per-op capability floor and the world-level create gate
    // below are skipped for those origins ONLY — every other check (the
    // `system-defaults` one above included) still runs unconditionally.
    if !origin.skips_capability_gates() && !access.has(cap::WRITE_FIELDS) {
        return Err(DataError::Forbidden);
    }
    // Baseline chat-posting right: a Player may author a `message`, exempt from
    // the otherwise-GM-only core:create gate. The WRITE_FIELDS floor above still
    // applies, and the extra `doc.owner == Some(ctx.user_id)` clause ties the
    // message to its poster. REQUIRED PRECONDITION for soundness: the WS/HTTP
    // client-intent ingress MUST reject any client-authored `message` op (it
    // does — see `chat::ops_target_message`), so that a `message` Create reaches
    // `apply_intent` only from the server-side message-send handler.
    let is_baseline_message = doc.doc_type == crate::chat::MESSAGE_DOC_TYPE
        && ctx.world_role == WorldRole::Player
        && doc.owner == Some(ctx.user_id);
    if !origin.skips_capability_gates()
        && ctx.world_role != WorldRole::Gm
        && !is_baseline_message
        && !world_defaults.role_has(ctx.world_role, &doc.doc_type, cap::CREATE)
    {
        tracing::debug!(
            user = %ctx.user_id, doc_type = %doc.doc_type,
            "create denied: missing core:create"
        );
        return Err(DataError::Forbidden);
    }
    // Create writes the whole body at once, so any declared requirement whose
    // protected path is populated must be authorized — otherwise Create is a
    // wholesale bypass of the declarative gate that Update enforces field-by-field.
    let doc_json = serde_json::to_value(doc)?;
    for extra in declared_caps_for_document(&doc_json, world_reqs) {
        if !access.has(extra) {
            tracing::debug!(
                user = %ctx.user_id, doc = %doc.id, capability = extra,
                "create denied: missing declared capability"
            );
            return Err(DataError::Forbidden);
        }
    }
    // Create carries no field paths, so the carried-light GM gate (see
    // `authorize_update_change`) checks the body directly: a non-GM may not
    // create a token/actor already carrying an emission, even in a world whose
    // `core:create` grant otherwise admits the Create itself.
    if !origin.skips_capability_gates()
        && ctx.world_role != WorldRole::Gm
        && carried_light_in_body(&doc.doc_type, &doc_json)
    {
        tracing::debug!(
            user = %ctx.user_id, doc_type = %doc.doc_type,
            "create denied: carried-light authoring is GM-only"
        );
        return Err(DataError::Forbidden);
    }
    Ok(access)
}

/// Phase 1's stored-type rejections and access resolution for an Update, shared by
/// the in-transaction arm and the pre-transaction validator screen (see
/// `authorize_create_intent` for why this is one function). Returns the resolved
/// `Access` plus whether the op is a `ServerMessageRevision` write to a message
/// doc — `authorize_update_change` needs that exact scope for its two exact-path
/// exemptions.
async fn authorize_update_access<'e, E>(
    executor: E,
    ctx: &PermissionContext,
    cur: &Document,
    origin: WriteOrigin,
    world_defaults: &crate::data::document::WorldCapDefaults,
) -> Result<(Access, bool), DataError>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    // Message docs are server-authored and immutable to clients: `Update` carries
    // no `doc_type` for `chat::ops_target_message` to classify, so the rejection
    // reads the authoritative STORED doc_type. `WriteOrigin::ServerMessageRevision`
    // — set ONLY by the server edit/delete handlers or the post-publish enrichment
    // republish, never derivable from any wire frame — re-opens this path for
    // their sanitized authoritative revision; every other origin is rejected, so a
    // combat clock batch may `Create` a `message` doc but can never `Update` one.
    if cur.doc_type == crate::chat::MESSAGE_DOC_TYPE && origin != WriteOrigin::ServerMessageRevision
    {
        return Err(DataError::Forbidden);
    }
    // `system-defaults` is server-authored (see `authorize_create_intent`'s
    // matching rejection): rejected against the authoritative STORED doc_type for
    // every origin but the world-config seed/refresh path's `ConfigSeed`.
    if cur.doc_type == SYSTEM_DEFAULTS_DOC_TYPE && origin != WriteOrigin::ConfigSeed {
        return Err(DataError::Forbidden);
    }
    // A `ServerMessageRevision` handler has ALREADY vetted owner-or-GM authority
    // before ever reaching here, so re-deriving capability from the document's
    // own permission fields for THIS origin+doc_type pair would incorrectly
    // re-restrict a GM's moderation edit/delete of a restricted-audience message.
    // Grant only READ + WRITE_FIELDS (never `all: true`): both existing handlers
    // construct a single `/engine` FieldChange and never touch `/permissions` or
    // `/embedded`, so the exemption is scoped to exactly what it is used for.
    let is_server_message_revision = cur.doc_type == crate::chat::MESSAGE_DOC_TYPE
        && origin == WriteOrigin::ServerMessageRevision;
    let access = if is_server_message_revision {
        Access {
            caps: [cap::READ.to_string(), cap::WRITE_FIELDS.to_string()]
                .into_iter()
                .collect(),
            all: false,
            see_gm_only: true,
            is_owner: true,
        }
    } else {
        // Effective owner joined from the LIVE linked actor — a linked token's
        // owner is never stored on the token.
        let upd_owner = SqliteRepository::load_effective_owner(executor, cur).await?;
        resolve_access_world(
            ctx.user_id,
            ctx.world_role,
            cur,
            &world_defaults.grants_for(&cur.doc_type),
            upd_owner,
        )
    };
    Ok((access, is_server_message_revision))
}

/// The per-op context `authorize_update_change` shares across a batch's changes:
/// everything that is not the individual `FieldChange` being judged or the
/// pre-image serialization it is judged against.
struct UpdateAuthzContext<'a> {
    /// The intent's author and world role.
    ctx: &'a PermissionContext,
    /// The stored pre-image document the Update applies to.
    cur: &'a Document,
    /// The write's origin (capability-skipping origins are exempted inside).
    origin: WriteOrigin,
    /// The world's GM-authored declarative capability requirements.
    world_reqs: &'a [crate::data::document::CapabilityRequirement],
    /// The resolved per-op `Access` (from `authorize_update_access`).
    access: &'a Access,
    /// Whether this op is a `ServerMessageRevision` write to a message doc.
    is_server_message_revision: bool,
}

/// Phase 1's per-change authorization for an Update — the structural capability
/// mapping (`required_cap_for_path`), the additive declared-requirement check
/// (`declared_caps_for_path`), and the carried-light GM gate
/// (`carried_light_touched`) for one `FieldChange`. Pure: every input was resolved
/// by `authorize_update_access` (or Phase 1's own load), so the in-transaction
/// arm and the pre-transaction validator screen apply the identical decision.
fn authorize_update_change(
    op: &UpdateAuthzContext<'_>,
    ch: &FieldChange,
    whole: &serde_json::Value,
) -> Result<(), DataError> {
    let UpdateAuthzContext {
        ctx,
        cur,
        origin,
        world_reqs,
        access,
        is_server_message_revision,
    } = *op;
    // Each field path requires its capability (`permission::required_cap_for_path`):
    // an immutable envelope field (id, scope, source, ...) maps to no capability
    // and is rejected for everyone. `/base` maps to no capability too: it is
    // server-owned (`permission::WRITABLE_BANDS`), derived at Create and refreshed
    // by server merge writes only.
    let need = required_cap_for_path(&ch.path);
    // The one server-owned field write through this gate: a merge handler's
    // whole-band `/base` refresh under `WriteOrigin::TemplateMerge`. The handler
    // already derived authorization against the computed Update, so the capability
    // mapping does not apply to it — every other check still runs. `/base/...`
    // sub-paths stay rejected for every origin.
    let merge_base_refresh =
        need.is_none() && origin == WriteOrigin::TemplateMerge && ch.path == "/base";
    // A capability-skipping origin (`WriteOrigin::skips_capability_gates`) skips
    // only the actor-holds-`need` test, never `required_cap_for_path`'s mapping:
    // an immutable envelope path (`None`, the merge refresh excepted) is still
    // rejected for every origin, those included.
    if need.is_none() && !merge_base_refresh {
        return Err(DataError::Forbidden);
    }
    if let Some(need) = need {
        if !origin.skips_capability_gates() && !access.has(need) {
            // A `ServerMessageRevision` write to a message doc may ALSO write
            // exactly `/permissions/property_overrides` (never any other
            // `/permissions` subpath) without holding `cap::EDIT_PERMISSIONS` —
            // `handle_recalc_roll` needs this to register a freshly-appended
            // recalc entry's gm_only override pointer. This exact-path admission
            // widens nothing for any other doc_type/origin/path.
            let is_recalc_override_write =
                is_server_message_revision && ch.path == "/permissions/property_overrides";
            if !is_recalc_override_write {
                tracing::debug!(
                    user = %ctx.user_id, path = %ch.path, capability = need,
                    "intent denied: missing capability"
                );
                return Err(DataError::Forbidden);
            }
        }
    }
    // Declarative requirements are additive: a module/world may demand extra
    // capabilities for a sub-path on top of the structural base above. SKIPPED
    // only for a `ServerMessageRevision` write to exactly `/engine` or
    // `/permissions/property_overrides` — a world's `CapabilityRequirement`
    // carries no `doc_type`, so an ancestor write to `/engine` would otherwise
    // inherit a requirement declared for a wholly unrelated doc_type's field,
    // blocking a GM's already-vetted moderation write. Any OTHER path under this
    // origin still goes through this check.
    let is_scoped_smr_write = is_server_message_revision
        && matches!(
            ch.path.as_str(),
            "/engine" | "/permissions/property_overrides"
        );
    if !origin.skips_capability_gates() && !is_scoped_smr_write {
        for extra in declared_caps_for_path(&ch.path, world_reqs) {
            if !access.has(extra) {
                tracing::debug!(
                    user = %ctx.user_id, path = %ch.path, capability = extra,
                    "intent denied: missing declared capability"
                );
                return Err(DataError::Forbidden);
            }
        }
    }
    // Carried-light authoring is GM-only, value-aware
    // (`permission::carried_light_touched`): an emission joins the SHARED
    // illumination field every viewer's lit mask and movement gate read, so unlike
    // an owner's other writable fields, writing one edits other players' secrecy
    // masks. An ancestor write is refused only when the emission subtree actually
    // changes, so a whole-`/engine/overrides` write that leaves `light` untouched
    // stays legal.
    if !origin.skips_capability_gates()
        && ctx.world_role != WorldRole::Gm
        && carried_light_touched(&cur.doc_type, &ch.path, ch.remove, whole, &ch.new)
    {
        tracing::debug!(
            user = %ctx.user_id, path = %ch.path,
            "intent denied: carried-light authoring is GM-only"
        );
        return Err(DataError::Forbidden);
    }
    Ok(())
}

/// Reconstructs the MERGED post-image document a `changes` Update would produce against
/// `doc_id`'s CURRENT stored row, read through `executor` — shared by Phase 2's authoritative
/// merge (`&mut *tx`, inside the write transaction) and the pre-transaction validator
/// pre-image build (a read-only pool connection, before the transaction opens): both merges
/// must reach the IDENTICAL document, or the validated post-image and the committed one could
/// silently diverge. Returns the PRE-image and the merged POST-image. `DataError::NotFound` if
/// the row is absent; `DataError::OpFailed` if `changes` would change the document id.
async fn merge_update_document<'e, E>(
    executor: E,
    doc_id: Uuid,
    changes: &[FieldChange],
) -> Result<(Document, Document), DataError>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let row = sqlx::query("SELECT json FROM documents WHERE id = ?")
        .bind(doc_id.to_string())
        .fetch_optional(executor)
        .await?
        .ok_or(DataError::NotFound)?;
    let pre_value: serde_json::Value = serde_json::from_str(row.get::<String, _>("json").as_str())?;
    let pre_doc: Document = serde_json::from_value(pre_value.clone())?;
    let mut post_value = pre_value;
    for ch in changes {
        // THE `apply_field_change` mutation rule — the same call the authoritative
        // merge makes; never a re-spelled remove/set branch.
        apply_field_change(&mut post_value, ch)?;
    }
    let post_doc: Document = serde_json::from_value(post_value)?;
    if post_doc.id != doc_id {
        return Err(DataError::OpFailed(
            "update must not change the document id".into(),
        ));
    }
    Ok((pre_doc, post_doc))
}

/// `doc`'s `CombatEngine`, or `None` if `doc` is not a `combat` document.
/// A stored `combat` document's engine body is always valid by construction
/// (validated at every write), so a parse failure here is treated the same
/// as a non-combat `doc_type` -- the caller's own decision this feeds is
/// advisory-only (see `apply_intent`'s one-active-per-scene tracking), never
/// the authoritative engine validation `validate_engine_tree` performs.
fn combat_engine_of(doc: &Document) -> Option<CombatEngine> {
    if doc.doc_type != COMBAT_DOC_TYPE {
        return None;
    }
    doc.engine
        .clone()
        .and_then(|e| serde_json::from_value(e).ok())
}

/// The `CombatEngine` `cur` would carry after `changes` apply, computed
/// entirely in memory -- no storage touched. Used ONLY to drive the
/// one-active-combat-per-scene decision ahead of the authoritative merge
/// (`validate_engine_tree` on the real post-image); a change this cannot
/// merge or re-parse into a `Document` yields `None` rather than an error,
/// since the real validation elsewhere in `apply_intent` surfaces any such
/// failure through its own path.
fn merged_combat_engine(cur: &Document, changes: &[FieldChange]) -> Option<CombatEngine> {
    if cur.doc_type != COMBAT_DOC_TYPE {
        return None;
    }
    let mut value = serde_json::to_value(cur).ok()?;
    changes
        .iter()
        .try_for_each(|ch| apply_field_change(&mut value, ch))
        .ok()?;
    let doc: Document = serde_json::from_value(value).ok()?;
    doc.engine.and_then(|e| serde_json::from_value(e).ok())
}

#[async_trait]
impl Repository for SqliteRepository {
    async fn apply_command(&self, cmd: UnsequencedCommand) -> Result<StoredCommand, DataError> {
        let mut tx = self.pool.begin().await?;

        // Allocate the next per-world seq from the single durable source.
        // Unlike `apply_intent` (seq allocated AFTER Phase-1 validation, so a
        // rejected intent never consumes one), this bump happens BEFORE any
        // op is validated -- safe only because the whole transaction rolls
        // back on any early `?` return below, so a rejected write never
        // commits the bumped seq either. A future error-handling refactor
        // that starts returning `Ok` on a partially-applied/rejected op
        // (instead of aborting via `?`) would silently start consuming seqs
        // on rejected writes; keep this ordering paired with whole-tx
        // rollback semantics.
        let seq: i64 = sqlx::query("UPDATE worlds SET seq = seq + 1 WHERE id = ? RETURNING seq")
            .bind(cmd.world_id.to_string())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(DataError::NotFound)?
            .get("seq");

        // Expand each Delete into its descendants (children-first) so a parent
        // delete removes children through explicit, logged ops rather than the
        // silent SQL FK cascade (#2/#8). apply_command is the trusted substrate
        // (undo/replay), so unlike apply_intent the descendants are not
        // capability-checked.
        let mut ops = Vec::with_capacity(cmd.ops.len());
        for op in cmd.ops {
            match op {
                Operation::Delete { doc } => {
                    for desc in Self::descendants_first(&mut tx, doc.id).await? {
                        let cur = Self::load_document(&mut *tx, desc).await?.ok_or_else(|| {
                            DataError::Conflict(format!("descendant {desc} missing"))
                        })?;
                        ops.push(Operation::Delete { doc: cur });
                    }
                    ops.push(Operation::Delete { doc });
                }
                other => ops.push(other),
            }
        }

        let mut sequenced = Command {
            seq,
            world_id: cmd.world_id,
            author: cmd.author,
            ts: cmd.ts,
            ops,
        };

        // Apply each operation. `normalized_ops` mirrors apply_intent's
        // rebuild: identical to `sequenced.ops` except an Update's
        // `FieldChange.new` under `/engine`(/*) is renormalized to the
        // validated post-image, so the returned `Command`, the
        // `world_events` log entry, and any future `events_since` replay
        // all carry the identical normalized value the row was stored
        // with. apply_command is the trusted substrate (undo/replay) and
        // skips capability/schema/size checks by design (zero production
        // callers), but the engine band's normalize-then-store invariant
        // is data integrity, not authz -- it applies regardless of trust
        // level.
        let mut post_images: std::collections::HashMap<Uuid, Document> =
            std::collections::HashMap::new();
        let mut deleted_created_seqs: std::collections::HashMap<Uuid, i64> =
            std::collections::HashMap::new();
        // Batch-start permissions for each Update target, captured the FIRST time its
        // pre-image is loaded — before any op applies — so a second same-batch Update to
        // the same doc still snapshots the true batch-start permissions, not an
        // intermediate value from an earlier op in this same command.
        let mut pre_permissions: std::collections::HashMap<
            Uuid,
            crate::data::document::PermissionSet,
        > = std::collections::HashMap::new();
        // Batch-start EFFECTIVE OWNER for each Update target, captured in lockstep with
        // `pre_permissions` at the same pre-image load point (first Update of a batch id
        // wins). `Option<Uuid>` inside the map value: an entry's ABSENCE means "not yet
        // captured", its `None` value means "captured, no owner".
        let mut pre_owners: std::collections::HashMap<Uuid, Option<Uuid>> =
            std::collections::HashMap::new();
        let mut normalized_ops = Vec::with_capacity(sequenced.ops.len());
        for op in &sequenced.ops {
            match op {
                Operation::Create { doc } => {
                    check_command_scope(doc, sequenced.world_id)?;
                    let mut doc = doc.clone();
                    // Same band-classification gate as apply_intent: no stored override
                    // can name a pointer redaction cannot classify (data integrity, not
                    // authz — see the /engine gate below for the same rationale).
                    crate::data::validation::validate_property_overrides(&doc)?;
                    crate::data::validation::validate_engine_tree(&mut doc)?;
                    crate::data::validation::validate_containment(&doc)?;
                    Self::upsert_document(&mut tx, &doc, seq).await?;
                    post_images.insert(doc.id, doc.clone());
                    normalized_ops.push(Operation::Create { doc });
                }
                Operation::Delete { doc } => {
                    check_command_scope(doc, sequenced.world_id)?;
                    if let Some(cs) = Self::document_created_seq(&mut *tx, doc.id).await? {
                        deleted_created_seqs.insert(doc.id, cs);
                    }
                    Self::delete_document_tx(&mut tx, doc.id).await?;
                    normalized_ops.push(op.clone());
                }
                Operation::Move {
                    doc_id, parent_id, ..
                } => {
                    let cur = Self::load_document(&mut *tx, *doc_id)
                        .await?
                        .ok_or_else(|| DataError::Conflict(format!("document {doc_id} missing")))?;
                    check_command_scope(&cur, sequenced.world_id)?;
                    // Batch-start snapshot capture, in lockstep with the
                    // Update arm's below.
                    if !pre_permissions.contains_key(doc_id) {
                        pre_permissions.insert(*doc_id, cur.permissions.clone());
                        let pre_owner = Self::load_effective_owner(&mut *tx, &cur).await?;
                        pre_owners.insert(*doc_id, pre_owner);
                    }
                    if cur.parent_id == *parent_id {
                        // No-op: carried in the log for invertibility;
                        // nothing written, nothing bumped, no hooks run.
                        post_images.insert(*doc_id, cur);
                        normalized_ops.push(op.clone());
                    } else {
                        // Trusted substrate: no capability/OCC gate, but the
                        // structural placement rules are data integrity and
                        // trust does not exempt them (same rationale as
                        // `validate_property_overrides` running here). Earlier
                        // ops in this command are already applied, so the
                        // batch bookkeeping maps are empty by construction.
                        let mut doc = cur;
                        doc.parent_id = *parent_id;
                        validation::validate_containment(&doc)?;
                        Self::check_parent_placement(
                            &mut tx,
                            sequenced.world_id,
                            &doc,
                            &Default::default(),
                            &Default::default(),
                        )
                        .await?;
                        Self::check_move_acyclic(
                            &mut tx,
                            *doc_id,
                            *parent_id,
                            &Default::default(),
                            &Default::default(),
                        )
                        .await?;
                        doc.updated_at = sequenced.ts;
                        Self::upsert_document(&mut tx, &doc, seq).await?;
                        if doc.doc_type == crate::data::engine::ASSET_FOLDER_DOC_TYPE {
                            Self::refresh_derived_tags_for_folder_subtree(&mut tx, doc.id).await?;
                        }
                        post_images.insert(*doc_id, doc);
                        normalized_ops.push(op.clone());
                    }
                }
                Operation::Update { doc_id, changes } => {
                    let row = sqlx::query("SELECT json FROM documents WHERE id = ?")
                        .bind(doc_id.to_string())
                        .fetch_optional(&mut *tx)
                        .await?
                        .ok_or(DataError::NotFound)?;
                    let mut value: serde_json::Value =
                        serde_json::from_str(row.get::<String, _>("json").as_str())?;
                    // Captured BEFORE this op applies, and only for the FIRST Update of
                    // this id in the batch — see `pre_permissions`'s own comment. Owner
                    // capture rides the same guard so the two maps stay in lockstep.
                    if !pre_permissions.contains_key(doc_id) {
                        let pre_doc: Document = serde_json::from_value(value.clone())?;
                        pre_permissions.insert(*doc_id, pre_doc.permissions.clone());
                        let pre_owner = Self::load_effective_owner(&mut *tx, &pre_doc).await?;
                        pre_owners.insert(*doc_id, pre_owner);
                    }
                    // Captured before this op's own `changes` apply — the
                    // TRUE stored pre-image `derive_engine_side_effects`
                    // diffs against below, to surface a normalize-time side
                    // effect on an engine key none of this op's own `changes`
                    // named (e.g. `NoteEngine::derive_body`). Capturing after
                    // the `apply_field_change` loop would compare the
                    // post-`changes`, pre-normalize value against itself,
                    // reporting the wrong `old` for a nested request this
                    // op's own change already applied.
                    let pre_engine = value.get("engine").cloned();
                    for ch in changes {
                        // THE `apply_field_change` mutation rule. Never
                        // re-derive the remove/set branch here: the derived scene ECS
                        // mirrors these same changes and must land the same value.
                        apply_field_change(&mut value, ch)?;
                    }
                    let mut doc: Document = serde_json::from_value(value)?;
                    // Identity and world scope are immutable through an update:
                    // changing id forks a duplicate row (load key != upsert key);
                    // changing world files the row outside this world's seq stream.
                    if doc.id != *doc_id {
                        return Err(DataError::OpFailed(
                            "update must not change the document id".into(),
                        ));
                    }
                    check_command_scope(&doc, sequenced.world_id)?;
                    // Same band-classification gate as apply_intent: no
                    // stored override can name a pointer redaction cannot
                    // classify (data integrity, not authz -- see below).
                    crate::data::validation::validate_property_overrides(&doc)?;
                    // Same /engine gate as apply_intent (the trusted
                    // substrate skips capability/schema/size checks by
                    // design, but the engine band's normalize-then-store
                    // invariant is data integrity, not authz -- the row,
                    // the log, and any future replay must carry the
                    // identical normalized value).
                    crate::data::validation::validate_engine_tree(&mut doc)?;
                    // updated_at tracks last mutation; the command ts is authoritative.
                    doc.updated_at = sequenced.ts;
                    Self::upsert_document(&mut tx, &doc, seq).await?;
                    post_images.insert(*doc_id, doc.clone());
                    // A folder's name is a derived tag on every asset beneath
                    // it; any Update to an `asset_folder` (rename being the
                    // one that matters) recomputes that subtree in this tx.
                    if doc.doc_type == crate::data::engine::ASSET_FOLDER_DOC_TYPE {
                        Self::refresh_derived_tags_for_folder_subtree(&mut tx, doc.id).await?;
                    }

                    // Re-derive each `/engine`(/*) `FieldChange.new` from
                    // the SAME validated post-image so the returned
                    // Command and the world_events log entry carry the
                    // identical normalized value the row was stored with
                    // -- never the raw submitted JSON.
                    let normalized_doc_json = serde_json::to_value(&doc)?;
                    let requested_paths: std::collections::HashSet<String> =
                        changes.iter().map(|ch| ch.path.clone()).collect();
                    let mut normalized_changes: Vec<FieldChange> = changes
                        .iter()
                        .map(|ch| {
                            if ch.path == "/engine" || ch.path.starts_with("/engine/") {
                                if let Some(v) = normalized_doc_json.pointer(&ch.path) {
                                    return FieldChange {
                                        remove: false,
                                        path: ch.path.clone(),
                                        old: ch.old.clone(),
                                        new: v.clone(),
                                    };
                                }
                            }
                            ch.clone()
                        })
                        .collect();
                    // A normalize-time derivation (e.g. `NoteEngine::derive_body`) can change
                    // an engine key none of this op's own `changes` named — surface those too,
                    // or the broadcast/log/author's own optimistic store never see them.
                    // apply_command is the trusted substrate (undo/replay) and skips
                    // capability/schema/size checks by design (zero production callers), so a
                    // derived path's declared capability requirements are not re-checked here
                    // either -- the same trust-level rationale as every other check this arm
                    // skips.
                    normalized_changes.extend(crate::data::validation::derive_engine_side_effects(
                        &doc.doc_type,
                        pre_engine.as_ref(),
                        doc.engine.as_ref(),
                        &requested_paths,
                    ));
                    normalized_ops.push(Operation::Update {
                        doc_id: *doc_id,
                        changes: normalized_changes,
                    });
                }
            }
        }
        sequenced.ops = normalized_ops;

        let world_gm_at_commit: std::collections::HashMap<Uuid, bool> =
            Self::world_member_roles(&mut *tx, sequenced.world_id)
                .await?
                .into_iter()
                .map(|(uid, role)| (uid, role == WorldRole::Gm))
                .collect();
        let mut per_op = Vec::with_capacity(sequenced.ops.len());
        for op in &sequenced.ops {
            per_op.push(Some(
                Self::build_op_snapshot(
                    &mut tx,
                    op,
                    &post_images,
                    &deleted_created_seqs,
                    &pre_permissions,
                    &pre_owners,
                )
                .await?,
            ));
        }
        let stored = StoredCommand {
            command: sequenced,
            snapshot: CommandSnapshot {
                per_op,
                world_gm_at_commit,
            },
        };

        // Append to the log.
        sqlx::query("INSERT INTO world_events (world_id, seq, author_id, ts, command_json) VALUES (?, ?, ?, ?, ?)")
            .bind(stored.command.world_id.to_string())
            .bind(seq)
            .bind(stored.command.author.to_string())
            .bind(stored.command.ts)
            .bind(serde_json::to_string(&stored)?)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(stored)
    }

    async fn apply_intent(
        &self,
        ctx: &crate::data::membership::PermissionContext,
        world_id: Uuid,
        mut ops: Vec<Operation>,
        ts: i64,
        origin: WriteOrigin,
    ) -> Result<StoredCommand, DataError> {
        // Load world default grants before opening the transaction: the
        // single-writer pool holds one connection, so a settings query mid-tx
        // would deadlock.
        let world_defaults = self.world_cap_defaults(world_id).await?;
        // Write-enforcement input is ONLY the GM-authored `world_cap_requirements`
        // record. Module-declared `requirements` (published to clients via the
        // Welcome union, see `ws::conn::welcome_capability_requirements`) are
        // advisory client-side UX only and are intentionally NOT consulted here —
        // server authority over write policy stays with the GM/operator, never
        // community module code.
        let world_reqs = self.world_cap_requirements(world_id).await?;
        // Loaded before the transaction (like `world_cap_requirements` above):
        // the single-writer pool would deadlock on a mid-tx settings query.
        // This is the GM-controlled tier-2 structural schema registry; the
        // writer never supplies its own judging schema.
        let world_schemas = self.world_schema_declarations(world_id).await?;
        // Sandboxed validators run HERE, entirely before the write transaction opens: the
        // single-writer pool (`max_connections(1)`) serializes every `apply_intent` server-wide,
        // so a validator held inside the transaction would throttle every hosted world. A stale
        // pre-image read here is safe — see `validated_pre_images` below for the in-transaction
        // re-validation that closes the gap the per-pointer OCC check leaves.
        let mut validated_pre_images: std::collections::HashMap<Uuid, (Document, bool)> =
            std::collections::HashMap::new();
        let mut validator_pass: Option<(
            std::sync::Arc<crate::sandbox::registry::ValidatorRegistry>,
            Vec<String>,
        )> = None;
        if let Some(modules_dir) = self.modules_dir.clone() {
            let enabled = match self.world_enabled_modules(world_id).await {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(world = %world_id, error = %e, "enabled-module read failed; validator pass skipped for this intent");
                    Vec::new()
                }
            };
            let enabled_module_ids: Vec<String> = enabled
                .iter()
                .filter(|e| e.validators_enabled)
                .map(|e| e.id.clone())
                .collect();
            if !enabled_module_ids.is_empty() {
                let registry = {
                    let cache = self.validator_registry_cache.clone();
                    tokio::task::spawn_blocking(move || cache.get_or_scan(&modules_dir))
                        .await
                        .unwrap_or_default()
                };
                let read_pool = self.open_read_pool().await?;
                for op in &ops {
                    let (mut doc, prior): (Document, Option<Document>) = match op {
                        Operation::Create { doc } => (doc.clone(), None),
                        Operation::Update { doc_id, changes } => {
                            let touches_system = changes
                                .iter()
                                .any(|c| crate::data::permission::targets_system_band(&c.path));
                            if !touches_system {
                                continue;
                            }
                            match merge_update_document(&read_pool, *doc_id, changes).await {
                                Ok((pre, post)) => (post, Some(pre)),
                                // A missing/malformed pre-image here is not this pass's problem
                                // to report — the real, authoritative Phase 1 load inside the
                                // transaction below surfaces the SAME failure properly.
                                Err(_) => continue,
                            }
                        }
                        Operation::Move { .. } => continue,
                        Operation::Delete { .. } => continue,
                    };
                    // Authorization screen, consulted BEFORE any validator: the SAME shared
                    // functions Phase 1's own arms call inside the transaction below
                    // (`authorize_create_intent`/`authorize_update_access`/
                    // `authorize_update_change`), never a re-spelled copy — an op Phase 1
                    // would refuse with `Forbidden` must not reach a validator's error text
                    // (which can disclose the validator's rules), must not burn a faulting
                    // module's streak toward auto-disable, and must not burn fuel CPU.
                    // Capability-skipping origins are exempt inside the shared functions
                    // exactly as they are in Phase 1. The screen's `Forbidden` also short-
                    // circuits the structural pre-pass below — a deliberately stricter
                    // precedence than Phase 1's (less disclosure), never a weaker one:
                    // anything the screen admits still faces Phase 1 unchanged.
                    let prior_permitted = match op {
                        Operation::Create { doc: create_doc } => {
                            authorize_create_intent(
                                &read_pool,
                                ctx,
                                create_doc,
                                origin,
                                &world_defaults,
                                &world_reqs,
                            )
                            .await?;
                            // A Create carries no prior band, so there is nothing the
                            // READ gate could withhold.
                            true
                        }
                        Operation::Update { changes, .. } => {
                            let pre_doc = prior
                                .as_ref()
                                .expect("an Update reaching this point always merged a pre-image");
                            let (access, is_smr) = authorize_update_access(
                                &read_pool,
                                ctx,
                                pre_doc,
                                origin,
                                &world_defaults,
                            )
                            .await?;
                            let whole = serde_json::to_value(pre_doc)?;
                            let authz_op = UpdateAuthzContext {
                                ctx,
                                cur: pre_doc,
                                origin,
                                world_reqs: &world_reqs,
                                access: &access,
                                is_server_message_revision: is_smr,
                            };
                            for ch in changes {
                                authorize_update_change(&authz_op, ch, &whole)?;
                            }
                            // The validator's `prior` band is stored content: a writer
                            // WITHOUT whole-document READ must not receive it (directly,
                            // or reflected through a crafted refusal reason).
                            access.has(cap::READ)
                        }
                        Operation::Move { .. } | Operation::Delete { .. } => {
                            unreachable!("Move/Delete ops continue above the screen")
                        }
                    };
                    match crate::sandbox::validate_document(
                        &registry,
                        &enabled_module_ids,
                        &mut doc,
                        prior.as_ref(),
                        prior_permitted,
                        world_id,
                        &world_schemas,
                    )
                    .await
                    {
                        // `validate_document`'s own structural pre-pass rejected `doc` before
                        // any validator ran — Phase 1 below would reject it identically, so
                        // its error is returned untouched here.
                        Err(structural_err) => return Err(structural_err),
                        Ok(crate::sandbox::ValidatorVerdict::Accept) => {}
                        Ok(crate::sandbox::ValidatorVerdict::Refuse { module, reason }) => {
                            return Err(DataError::OpFailed(format!(
                                "validator {module}: {reason}"
                            )));
                        }
                        Ok(crate::sandbox::ValidatorVerdict::Fault(fault)) => {
                            return Err(DataError::Validator(fault));
                        }
                    }
                    // Captured for the in-transaction re-validation below: Phase 1's OCC
                    // covers only each change's own pointer, so a concurrent write to any
                    // OTHER path of the same document would pass OCC and commit a post-image
                    // no validator ever saw. The Update arm compares its in-transaction
                    // pre-image against this capture and re-validates when they differ.
                    if let (Operation::Update { doc_id, .. }, Some(pre)) = (op, prior) {
                        validated_pre_images.insert(*doc_id, (pre, prior_permitted));
                    }
                }
                validator_pass = Some((registry, enabled_module_ids));
            }
        }
        let mut tx = self.pool.begin().await?;

        // Phase 1 — authorize, structurally validate, and check pre-images.
        // No row is mutated; any failure here drops the transaction, so the
        // per-world seq is never consumed by a rejected intent. `Create`'s
        // `doc` is mutated in place (`&mut ops`) so `validate_engine_tree`
        // can normalize the engine band here and have that normalization
        // survive into Phase 2 storage AND the returned `Command` (broadcast).
        //
        // `claimed_singletons` tracks singleton doc_types already passed by an
        // EARLIER Create in this SAME batch. Phase 2 (the actual inserts)
        // only runs after every op in the batch clears Phase 1, so a batch
        // containing two Creates of the same singleton doc_type would have
        // both ops' `singleton_doc_exists` reads see nothing (neither has
        // been inserted yet) and both pass the DB check alone — this set
        // closes that intra-batch gap the DB check cannot see.
        let mut claimed_singletons: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        // `batch_combats` tracks the ids of `combat` documents Created earlier
        // in this same batch, so a same-batch `combat` + `combatant` pair
        // (a scene+combat+combatants import/setup in one Intent) can satisfy
        // the combatant-parentage check without a DB round trip that would
        // see nothing yet inserted.
        let mut batch_combats: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
        // `batch_folders` plays the same role for `asset_folder` AND `note`
        // documents (a mixed-doc_type map keyed by id, since ids are unique
        // regardless of type): a folder/note Created earlier in this batch is
        // a valid parent for a later one, and `check_asset_folder_parent`/
        // `check_note_parent` — plus `check_move_acyclic`'s cycle walk, shared
        // across every doc_type that supports a parent tree — resolve through
        // it before falling back to the database.
        let mut batch_folders: std::collections::HashMap<Uuid, Document> =
            std::collections::HashMap::new();
        // `batch_moves` records the PROSPECTIVE parent of each Move already
        // validated in this batch. Phase 2 applies nothing until every op
        // clears Phase 1, so a cycle walk that read only the stored tree
        // would validate each Move against a tree no op has rewritten yet —
        // two Moves that swap a pair of subtrees into a cycle would each
        // pass alone. `check_move_acyclic` consults this map first, so it
        // sees the tree the batch will actually leave.
        let mut batch_moves: std::collections::HashMap<Uuid, Option<Uuid>> =
            std::collections::HashMap::new();
        // `scene_owner` maps a scene id to the id of the `combat` document
        // that holds its active slot, AS OF THIS POINT in a single simulated
        // walk of `ops` in their actual batch order. The one-active-combat-
        // per-scene decision -- for both the Create arm and the Update
        // arm -- is made exactly ONCE, entirely inside Phase 1 (Phase 2
        // performs no independent recomputation of it), consulting and
        // mutating this ONE map: a claim (a Create or Update that would make
        // some combat `active: true` on scene `S`) succeeds when
        // `scene_owner.get(S)` is absent or already equals that combat's own
        // id, and inserts `S -> that combat's id`; a release (an Update that
        // would make its own combat `active: false`) removes `S` only when
        // `scene_owner.get(S)` equals that SAME combat's id, and otherwise
        // does nothing -- an unrelated combat's deactivation can never touch
        // a DIFFERENT combat's claim on the same scene. Seeded lazily by
        // `Self::ensure_scene_owner_seeded`, and consulted only through
        // this map, so the whole batch shares one running fact rather than
        // re-deriving it at each op.
        let mut scene_owner: std::collections::HashMap<Uuid, Uuid> =
            std::collections::HashMap::new();
        // Scenes `Self::ensure_scene_owner_seeded` has already resolved
        // (seeded from the DB, or deliberately left unseeded) for this
        // batch -- a scene is seeded at most once regardless of how many
        // ops in the batch touch it.
        let mut seeded_scenes: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
        // The set of scenes a genuine same-batch active-true-to-false
        // transition will free, computed by a pre-scan over the WHOLE batch
        // before the Phase-1 per-op loop runs (and before `scene_owner` is
        // seeded at all) -- not derived incrementally in op order. A single
        // forward walk of `ops` cannot see a LATER op's deactivation when
        // validating an EARLIER op that claims the same scene (e.g.
        // `[activate combat A, deactivate combat B]` on one scene: A's claim
        // is checked before B's deactivation has run), so which scenes this
        // batch will free must be known before the walk begins.
        // `Self::ensure_scene_owner_seeded` consults this set to decide
        // whether to seed a scene's owner from the DB at all: a scene this
        // batch will free is left unseeded (no owner) from the start, so an
        // earlier same-batch claim on it is validated against the state the
        // batch will actually leave, not a DB row a later op is about to
        // invalidate.
        //
        // Only an op whose PRE-image was already `active: true` qualifies: an
        // op touching a combat that was ALREADY inactive must never mark its
        // scene as freed this batch, or an unrelated activation elsewhere in
        // the batch would wrongly skip its DB conflict check. Two op shapes
        // free a scene:
        // - `Delete` of an `active: true` combat -- the combat, and its claim,
        //   cease to exist outright.
        // - `Update` whose post-merge `active` is `false` (a genuine
        //   true->false transition, via `merged_combat_engine`), OR whose
        //   post-merge `active` STAYS `true` but `scene_id` changed -- moving
        //   away from a scene while remaining active vacates that scene just
        //   as genuinely as deactivating does. These two `Update` cases are
        //   mutually exclusive (`merged_engine.active` cannot be both `true`
        //   and `false` for the same op) and file under different scenes (the
        //   PRE-merge scene in both cases -- the scene the combat is LEAVING,
        //   never the one it is moving to), so neither double-frees nor
        //   conflicts with the other.
        // An op this cannot merge or parse is left alone here -- Phase 1's
        // real validation below surfaces the failure through the ordinary
        // path.
        let mut deactivations_this_batch: std::collections::HashSet<Uuid> =
            std::collections::HashSet::new();
        for op in &ops {
            match op {
                Operation::Update { doc_id, changes } => {
                    if let Some(cur) = Self::load_document(&mut *tx, *doc_id).await? {
                        // Scoped the same way every other load site in this
                        // function is: a foreign-world document must never
                        // influence this batch's validation, even indirectly
                        // through the pre-scan's bookkeeping.
                        check_command_scope(&cur, world_id)?;
                        if let Some(pre_engine) = combat_engine_of(&cur) {
                            if pre_engine.active {
                                if let Some(merged_engine) = merged_combat_engine(&cur, changes) {
                                    if !merged_engine.active
                                        || merged_engine.scene_id != pre_engine.scene_id
                                    {
                                        deactivations_this_batch.insert(pre_engine.scene_id);
                                    }
                                }
                            }
                        }
                    }
                }
                Operation::Delete { doc } => {
                    if let Some(cur) = Self::load_document(&mut *tx, doc.id).await? {
                        check_command_scope(&cur, world_id)?;
                        if let Some(pre_engine) = combat_engine_of(&cur) {
                            if pre_engine.active {
                                deactivations_this_batch.insert(pre_engine.scene_id);
                            }
                        }
                    }
                }
                Operation::Create { .. } => {}
                // A combat document refuses any parent at `validate_containment`,
                // so a Move can never alter a combat engine's active/scene state.
                Operation::Move { .. } => {}
            }
        }
        // Batch-start permissions for each Update target, captured the FIRST time its
        // pre-image is loaded in Phase 1 — before any op applies — so a second same-batch
        // Update to the same doc still snapshots the true batch-start permissions, not an
        // intermediate value written by an earlier op in this same command.
        let mut pre_permissions: std::collections::HashMap<
            Uuid,
            crate::data::document::PermissionSet,
        > = std::collections::HashMap::new();
        // Batch-start EFFECTIVE OWNER for each Update target, captured in lockstep with
        // `pre_permissions` at the same pre-image load point (first Update of a batch id
        // wins). `Option<Uuid>` inside the map value: an entry's ABSENCE means "not yet
        // captured", its `None` value means "captured, no owner".
        let mut pre_owners: std::collections::HashMap<Uuid, Option<Uuid>> =
            std::collections::HashMap::new();
        // Phase 1's resolved `Access` for each `Operation::Update`, ONE entry
        // per Update op, pushed in the same relative order those ops appear
        // in `ops` (an id may recur across several Update ops; each gets its
        // own entry). Phase 2 threads this through to gate a normalize-time
        // derived engine change (`derive_engine_side_effects`) against the
        // world's declared capability requirements — reusing the capability
        // set Phase 1 already resolved for this exact actor/document/origin,
        // never re-resolving access in Phase 2.
        let mut update_access: Vec<Access> = Vec::new();
        // A stamp whose template is CREATED in this same batch cannot load it
        // from the store yet (nothing is written before the loop below), so
        // the batch's own Creates are consulted first, by id — only the ones
        // some other Create in the batch names as its `source`.
        let batch_sources: std::collections::HashSet<Uuid> = ops
            .iter()
            .filter_map(|op| match op {
                Operation::Create { doc } => doc.source.as_ref().map(|s| s.id),
                _ => None,
            })
            .collect();
        let batch_templates: std::collections::HashMap<Uuid, Document> = ops
            .iter()
            .filter_map(|op| match op {
                Operation::Create { doc } if batch_sources.contains(&doc.id) => {
                    Some((doc.id, doc.clone()))
                }
                _ => None,
            })
            .collect();
        for op in &mut ops {
            match op {
                Operation::Move {
                    doc_id,
                    parent_id,
                    old_parent_id,
                } => {
                    let cur = Self::load_document(&mut *tx, *doc_id)
                        .await?
                        .ok_or_else(|| DataError::Conflict(format!("document {doc_id} missing")))?;
                    check_command_scope(&cur, world_id)?;
                    // GM-only, and only where the GM's unconditional
                    // short-circuit holds: a `gm_role`-capped GM floor-resolves
                    // through `resolve_access_world` like any other actor and
                    // is refused. `CombatTransition` skips this capability
                    // gate — see the Create arm's matching comment above.
                    if origin != WriteOrigin::CombatTransition {
                        let owner = Self::load_effective_owner(&mut *tx, &cur).await?;
                        let access = resolve_access_world(
                            ctx.user_id,
                            ctx.world_role,
                            &cur,
                            &world_defaults.grants_for(&cur.doc_type),
                            owner,
                        );
                        if ctx.world_role != WorldRole::Gm || !access.all {
                            return Err(DataError::Forbidden);
                        }
                    }
                    // Stored `message` docs refuse every ordinary mutation
                    // path — the same stored-doc_type classification the
                    // Update arm applies below.
                    if cur.doc_type == crate::chat::MESSAGE_DOC_TYPE
                        && origin != WriteOrigin::ServerMessageRevision
                    {
                        return Err(DataError::OpFailed(
                            "message documents cannot be moved".into(),
                        ));
                    }
                    if *old_parent_id != cur.parent_id {
                        return Err(DataError::Conflict(format!(
                            "parent pre-image mismatch for {doc_id}"
                        )));
                    }
                    // Batch-start snapshot capture, in lockstep with the
                    // Update arm's (see `pre_permissions`'s own comment).
                    if !pre_permissions.contains_key(doc_id) {
                        pre_permissions.insert(*doc_id, cur.permissions.clone());
                        let pre_owner = Self::load_effective_owner(&mut *tx, &cur).await?;
                        pre_owners.insert(*doc_id, pre_owner);
                    }
                    if *parent_id != cur.parent_id {
                        // Create-validity on the post-image: a Move is legal
                        // exactly where a Create with this parent would be.
                        let mut post = cur.clone();
                        post.parent_id = *parent_id;
                        validation::validate_containment(&post)?;
                        Self::check_parent_placement(
                            &mut tx,
                            world_id,
                            &post,
                            &batch_folders,
                            &batch_combats,
                        )
                        .await?;
                        Self::check_move_acyclic(
                            &mut tx,
                            *doc_id,
                            *parent_id,
                            &batch_folders,
                            &batch_moves,
                        )
                        .await?;
                        batch_moves.insert(*doc_id, *parent_id);
                    }
                }
                Operation::Create { doc } => {
                    check_command_scope(doc, world_id)?;
                    // `base` is server-owned: derive it from the document's
                    // OWN bands BEFORE any validation runs, so the derived
                    // value is what gets validated, normalized, stored,
                    // broadcast and logged — a stamped instance (`source`
                    // set) snapshots itself, any other document stores no
                    // base, and embedded children never carry one. Any
                    // client-supplied `base` is discarded here. The
                    // template is loaded so its content-band policy lands
                    // on the new instance first (`propagate_overrides`,
                    // inside `derive_create_base`); a template in another
                    // world is treated exactly as a missing one, the same
                    // reading `merge_intents::load_pull_docs` gives it.
                    // `apply_command` (the trusted undo/replay substrate)
                    // deliberately does NOT re-derive: it applies the
                    // already-derived logged op verbatim, and every
                    // production Create reaches storage through this arm.
                    let template = match &doc.source {
                        Some(source) => match batch_templates.get(&source.id) {
                            Some(t) => Some(t.clone()),
                            None => Self::load_document(&mut *tx, source.id)
                                .await?
                                .filter(|t| world_of(t).is_none_or(|w| w == world_id)),
                        },
                        None => None,
                    };
                    // The instance owner's standing on the template, under
                    // which egress evaluates the derived snapshot's recorded
                    // policy: both effective owners through the one
                    // linked-actor join (`load_effective_owner`) and the
                    // owner's membership role, resolved by the one
                    // `permission::owner_standing` the merge writers use.
                    let owner_standing = match &template {
                        Some(t) => {
                            let owner = match Self::load_effective_owner(&mut *tx, doc).await? {
                                Some(user) => Self::load_member_role(&mut *tx, world_id, user)
                                    .await?
                                    .map(|role| (user, role)),
                                None => None,
                            };
                            let template_owner = Self::load_effective_owner(&mut *tx, t).await?;
                            crate::data::permission::owner_standing(
                                owner,
                                t,
                                &world_defaults.grants_for(&t.doc_type),
                                template_owner,
                            )
                        }
                        None => crate::data::document::OwnerStanding::Stranger,
                    };
                    crate::merge::bands::derive_create_base(doc, template.as_ref(), owner_standing);
                    // A combatant's stored resource numbers derive from actor
                    // formulas that may read hidden leaves, so their egress
                    // defaults to the trusted tier: stamp the override when
                    // the Create carries none. An explicit entry — any tier,
                    // `Visibility::All` included — is the author's deliberate
                    // widening and is left untouched, as are Updates. Stamped
                    // BEFORE `validate_property_overrides` so the inserted
                    // entry is validated like an authored one.
                    if doc.doc_type == COMBATANT_DOC_TYPE {
                        doc.permissions
                            .property_overrides
                            .entry("/engine/resources".to_string())
                            .or_insert(crate::data::document::Visibility::OwnerOrGm);
                    }
                    validation::validate_system_size(doc)?;
                    validation::validate_property_overrides(doc)?;
                    validation::validate_engine_tree(doc)?;
                    // Re-checked AFTER `validate_engine_tree`: for a `note`
                    // document that call replaces `doc.engine` with the
                    // SERVER-DERIVED body (`NoteEngine::derive_body`), and the
                    // cap above ran against the client's pre-derivation
                    // payload (an empty `body: []`), not the value that is
                    // actually about to be stored, written to `world_events`,
                    // and broadcast. Reusing `validate_system_size` (rather
                    // than a second size rule) keeps ONE cap statement that
                    // now covers both the submitted and the derived shape.
                    validation::validate_system_size(doc)?;
                    validation::validate_containment(doc)?;
                    Self::check_parent_placement(
                        &mut tx,
                        world_id,
                        doc,
                        &batch_folders,
                        &batch_combats,
                    )
                    .await?;
                    if doc.doc_type == crate::data::engine::ASSET_FOLDER_DOC_TYPE
                        || doc.doc_type == crate::data::engine::NOTE_DOC_TYPE
                    {
                        batch_folders.insert(doc.id, doc.clone());
                    }
                    if doc.doc_type == COMBAT_DOC_TYPE {
                        batch_combats.insert(doc.id);
                        // `validate_engine_tree` above already validated and
                        // normalized this body against `CombatEngine`, so the
                        // re-deserialize here cannot fail; `.expect` on the
                        // parse itself (rather than discarding the error via
                        // `.ok()`) surfaces the real `serde_json::Error` text
                        // if that invariant is ever broken by a future schema
                        // change.
                        let combat_engine: CombatEngine = serde_json::from_value(
                            doc.engine
                                .clone()
                                .expect("a combat doc_type always carries an engine body"),
                        )
                        .expect("validate_engine_tree already validated the combat engine body");
                        if combat_engine.active {
                            Self::ensure_scene_owner_seeded(
                                &mut *tx,
                                world_id,
                                combat_engine.scene_id,
                                &mut scene_owner,
                                &mut seeded_scenes,
                                &deactivations_this_batch,
                            )
                            .await?;
                            match scene_owner.get(&combat_engine.scene_id) {
                                Some(&owner) if owner != doc.id => {
                                    return Err(DataError::Conflict(
                                        "an active combat already exists on this scene".into(),
                                    ));
                                }
                                _ => {
                                    scene_owner.insert(combat_engine.scene_id, doc.id);
                                }
                            }
                        }
                    }
                    validation::validate_system_schema_tree(doc, &world_schemas)?;
                    // A self-referential parent_id satisfies the self-FK and
                    // commits, then poisons the doc's deletion (the descendant
                    // walk would loop). Reject it. A stored parent's world
                    // scope is `check_parent_placement`'s check above; an
                    // unborn same-command parent is left to the FK at apply
                    // time, so batched scene+children creates still pass.
                    if doc.parent_id == Some(doc.id) {
                        return Err(Self::self_parent_error());
                    }
                    // Authorization: the ONE shared statement of the Create
                    // capability floor (`authorize_create_intent`) — the
                    // pre-transaction validator screen above consults the same
                    // function, so the two can never disagree. A capability-
                    // skipping server-authored origin (`WriteOrigin::
                    // skips_capability_gates`: the combat clock's
                    // `CombatTransition`, the world-config seed's `ConfigSeed`)
                    // is exempt from the per-op capability floor inside it —
                    // every other check in this arm (scope, size, engine,
                    // containment, singleton, one-active-per-scene, schema)
                    // still runs unconditionally.
                    authorize_create_intent(
                        &mut *tx,
                        ctx,
                        doc,
                        origin,
                        &world_defaults,
                        &world_reqs,
                    )
                    .await?;
                    // Create is non-clobbering: an existing id is a conflict,
                    // not a silent overwrite (unlike upsert in apply_command).
                    if Self::load_document(&mut *tx, doc.id).await?.is_some() {
                        return Err(DataError::Conflict(format!(
                            "document {} already exists",
                            doc.id
                        )));
                    }
                    // Singleton doc_type create-gate: check-then-insert runs
                    // inside THIS transaction (same `tx` the existing-id check
                    // and the eventual insert use), so a concurrent Create
                    // racing this check cannot both pass it — the single-
                    // writer pool (`max_connections(1)`) serializes competing
                    // `apply_intent` transactions at connection-acquisition,
                    // and this query never touches `&self.pool` (which would
                    // deadlock mid-transaction, not race). `claimed_singletons`
                    // additionally covers a second same-batch Create of the
                    // same singleton doc_type, which the DB read alone cannot
                    // see (see the comment above the Phase-1 loop).
                    if SINGLETON_DOC_TYPES.contains(&doc.doc_type.as_str()) {
                        if claimed_singletons.contains(doc.doc_type.as_str())
                            || Self::singleton_doc_exists(&mut *tx, world_id, &doc.doc_type).await?
                        {
                            return Err(DataError::Conflict(format!(
                                "a '{}' document already exists in this world",
                                doc.doc_type
                            )));
                        }
                        claimed_singletons.insert(doc.doc_type.clone());
                    }
                }
                Operation::Delete { doc } => {
                    let cur = Self::load_document(&mut *tx, doc.id)
                        .await?
                        .ok_or_else(|| {
                            DataError::Conflict(format!("document {} missing", doc.id))
                        })?;
                    // Authorize against the stored doc, scoped to this world, so
                    // a GM of one world cannot delete another world's document.
                    check_command_scope(&cur, world_id)?;
                    // `system-defaults` deletion is reserved to the server-side
                    // config-seed path — same rejection the Create arm applies,
                    // against the authoritative STORED doc_type.
                    if cur.doc_type == SYSTEM_DEFAULTS_DOC_TYPE && origin != WriteOrigin::ConfigSeed
                    {
                        return Err(DataError::Forbidden);
                    }
                    let del_owner = Self::load_effective_owner(&mut *tx, &cur).await?;
                    // Capability-skipping origins (`WriteOrigin::
                    // skips_capability_gates`) skip this gate — see the Create
                    // arm's matching comment above.
                    if !origin.skips_capability_gates()
                        && !resolve_access_world(
                            ctx.user_id,
                            ctx.world_role,
                            &cur,
                            &world_defaults.grants_for(&cur.doc_type),
                            del_owner,
                        )
                        .has(cap::DELETE)
                    {
                        return Err(DataError::Forbidden);
                    }
                }
                Operation::Update { doc_id, changes } => {
                    let cur = Self::load_document(&mut *tx, *doc_id)
                        .await?
                        .ok_or_else(|| DataError::Conflict(format!("document {doc_id} missing")))?;
                    // Captured BEFORE this op applies, and only for the FIRST Update of
                    // this id in the batch — see `pre_permissions`'s own comment. Owner
                    // capture rides the same guard so the two maps stay in lockstep.
                    if !pre_permissions.contains_key(doc_id) {
                        pre_permissions.insert(*doc_id, cur.permissions.clone());
                        let pre_owner = Self::load_effective_owner(&mut *tx, &cur).await?;
                        pre_owners.insert(*doc_id, pre_owner);
                    }
                    check_command_scope(&cur, world_id)?;
                    // Stored-type rejections and access resolution: the ONE shared
                    // statement (`authorize_update_access`) the pre-transaction
                    // validator screen also consults — see `authorize_create_intent`
                    // for the never-fork rationale. The `ServerMessageRevision`
                    // branch trusts the calling handler to have already vetted
                    // owner-or-GM authority; the storage layer only authorizes the
                    // write's SHAPE (a scoped READ + WRITE_FIELDS grant, never
                    // `all: true`).
                    let (access, is_server_message_revision) =
                        authorize_update_access(&mut *tx, ctx, &cur, origin, &world_defaults)
                            .await?;
                    // Recorded for Phase 2's derived-path capability check
                    // before any per-change validation below can reject this
                    // op -- an error return here never reaches Phase 2, so
                    // pushing early does not desync the two phases' op order.
                    update_access.push(access.clone());
                    // Field-level OCC: every change's pre-image must equal the
                    // current value at its pointer (absent reads as Null).
                    // INVARIANT: `cur` is this op's OWN `load_document` read of
                    // the not-yet-written transaction, never a simulation of an
                    // earlier same-batch op's changes — a second `Update` to the
                    // same document whose `old` names an earlier op's `new` in
                    // this batch reads the pre-batch stored value and Conflicts.
                    // The client's `buildUpdate` is the intended single-Update
                    // shape for same-document edits: it coalesces every field
                    // into one `FieldEdit[]` batch rather than issuing several
                    // Updates to the same doc in one command.
                    let whole = serde_json::to_value(&cur)?;
                    let authz_op = UpdateAuthzContext {
                        ctx,
                        cur: &cur,
                        origin,
                        world_reqs: &world_reqs,
                        access: &access,
                        is_server_message_revision,
                    };
                    for ch in &*changes {
                        validation::validate_field_change(ch)?;
                        // Per-change authorization: the ONE shared statement
                        // (`authorize_update_change`) — the structural
                        // capability mapping (`required_cap_for_path`), the
                        // additive declared-requirement check, and the
                        // carried-light GM gate — also consulted by the
                        // pre-transaction validator screen, so the two can
                        // never disagree.
                        authorize_update_change(&authz_op, ch, &whole)?;
                        let actual = whole
                            .pointer(&ch.path)
                            .cloned()
                            .unwrap_or(serde_json::Value::Null);
                        // Numeric-aware: a whole-number-valued engine `f64` round-tripped
                        // through a JS client loses its Float-ness (PosInt/Float variant
                        // split), so raw `!=` here would spuriously Conflict an otherwise
                        // up-to-date write. See `values_semantically_eq` doc comment.
                        // Shape-aware for the engine band: the stored value is the
                        // normalizer's output (absent `Option` fields as explicit null),
                        // so a raw mismatch is re-tried against the pre-image read through
                        // that same normalizer (`normalized_engine_pre_image`); a `system`
                        // pre-image gets no second reading. A pre-image that omits or
                        // disagrees on a REAL stored value still differs after
                        // normalization and still conflicts.
                        let pre_image_matches =
                            crate::data::command::values_semantically_eq(&actual, &ch.old)
                                || validation::normalized_engine_pre_image(
                                    &whole, &ch.path, &ch.old,
                                )
                                .is_some_and(|normalized| {
                                    crate::data::command::values_semantically_eq(
                                        &actual,
                                        &normalized,
                                    )
                                });
                        if !pre_image_matches {
                            return Err(DataError::Conflict(format!(
                                "stale pre-image at {}",
                                ch.path
                            )));
                        }
                    }
                    // One-active-combat-per-scene enforcement for an Update,
                    // run entirely HERE in Phase 1 -- never re-derived by
                    // Phase 2, which performs no independent recomputation of
                    // this invariant (see `scene_owner`'s doc comment above).
                    // Uses the same tolerant merge-simulation the pre-scan
                    // above uses; a merge/parse failure here is left to the
                    // authoritative `validate_engine_tree` pass in Phase 2 to
                    // surface.
                    if let Some(pre_engine) = combat_engine_of(&cur) {
                        if let Some(merged_engine) = merged_combat_engine(&cur, changes) {
                            if merged_engine.active {
                                let scene = merged_engine.scene_id;
                                Self::ensure_scene_owner_seeded(
                                    &mut *tx,
                                    world_id,
                                    scene,
                                    &mut scene_owner,
                                    &mut seeded_scenes,
                                    &deactivations_this_batch,
                                )
                                .await?;
                                match scene_owner.get(&scene) {
                                    Some(&owner) if owner != *doc_id => {
                                        return Err(DataError::Conflict(
                                            "an active combat already exists on this scene".into(),
                                        ));
                                    }
                                    _ => {
                                        scene_owner.insert(scene, *doc_id);
                                    }
                                }
                            } else if pre_engine.active {
                                // A genuine active-true -> false transition:
                                // free the PRE-merge scene (never the
                                // post-merge one) -- an Update that
                                // simultaneously moves a combat to a
                                // different scene AND deactivates it must
                                // free the scene it was actually active on,
                                // never the scene it is moving to, which may
                                // already hold an unrelated genuinely-active
                                // combat this batch never touches.
                                let scene = pre_engine.scene_id;
                                Self::ensure_scene_owner_seeded(
                                    &mut *tx,
                                    world_id,
                                    scene,
                                    &mut scene_owner,
                                    &mut seeded_scenes,
                                    &deactivations_this_batch,
                                )
                                .await?;
                                // Release only when THIS combat is the
                                // current owner in the simulation -- an
                                // unrelated combat's deactivation must never
                                // remove a DIFFERENT combat's claim on this
                                // scene, even if that different combat is the
                                // scene's real, currently-active occupant.
                                if scene_owner.get(&scene) == Some(&*doc_id) {
                                    scene_owner.remove(&scene);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Substitute the authoritative stored document into each Delete op: the
        // client supplies only the id to delete, so the broadcast and the
        // world_events log must carry server state, never the client body
        // (whose forged permissions would otherwise drive per-recipient
        // redaction and persist into the authoritative event log).
        let mut authoritative_ops = Vec::with_capacity(ops.len());
        for op in ops {
            match op {
                Operation::Delete { doc } => {
                    // A scene/parent delete expands to explicit Delete ops for
                    // every descendant (children before parents) so each removal
                    // is an individually reversible op (#8) and broadcasts to
                    // clients (#2) — never a silent FK cascade. Descendants are
                    // discovered here in Phase 2, so each is authorized against
                    // its stored doc with the same DELETE gate Phase 1 applies to
                    // the submitted op.
                    for desc in Self::descendants_first(&mut tx, doc.id).await? {
                        let cur = Self::load_document(&mut *tx, desc).await?.ok_or_else(|| {
                            DataError::Conflict(format!("descendant {desc} missing"))
                        })?;
                        check_command_scope(&cur, world_id)?;
                        let desc_owner = Self::load_effective_owner(&mut *tx, &cur).await?;
                        // Capability-skipping origins (`WriteOrigin::
                        // skips_capability_gates`) skip this gate — see the
                        // Create arm's matching comment above.
                        if !origin.skips_capability_gates()
                            && !resolve_access_world(
                                ctx.user_id,
                                ctx.world_role,
                                &cur,
                                &world_defaults.grants_for(&cur.doc_type),
                                desc_owner,
                            )
                            .has(cap::DELETE)
                        {
                            return Err(DataError::Forbidden);
                        }
                        authoritative_ops.push(Operation::Delete { doc: cur });
                    }
                    let cur = Self::load_document(&mut *tx, doc.id)
                        .await?
                        .ok_or_else(|| {
                            DataError::Conflict(format!("document {} missing", doc.id))
                        })?;
                    authoritative_ops.push(Operation::Delete { doc: cur });
                }
                other => authoritative_ops.push(other),
            }
        }

        // Phase 2 — allocate seq, apply, log. Identical machinery to
        // apply_command; authorization above has already cleared every op.
        // Consumes `update_access` in order: `authoritative_ops` preserves
        // the relative order of every `Operation::Update` from `ops`
        // (Delete expansion is the only reordering, and it never touches
        // Update), so the Nth Update encountered here is always the Nth
        // entry Phase 1 pushed.
        let mut update_access_iter = update_access.into_iter();
        let seq: i64 = sqlx::query("UPDATE worlds SET seq = seq + 1 WHERE id = ? RETURNING seq")
            .bind(world_id.to_string())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(DataError::NotFound)?
            .get("seq");

        let mut sequenced = Command {
            seq,
            world_id,
            author: ctx.user_id,
            ts,
            ops: authoritative_ops,
        };

        // Rebuilt in place of `sequenced.ops`: identical to the input ops
        // except an Update's `FieldChange.new` under `/engine`(/*) is
        // renormalized to the validated post-image (see below). Since
        // `sequenced` is what gets broadcast AND logged to `world_events`
        // (INSERT further down) AND replayed by `events_since`, this is the
        // single chokepoint that keeps all three in sync with the persisted
        // row.
        let mut post_images: std::collections::HashMap<Uuid, Document> =
            std::collections::HashMap::new();
        let mut deleted_created_seqs: std::collections::HashMap<Uuid, i64> =
            std::collections::HashMap::new();
        let mut normalized_ops = Vec::with_capacity(sequenced.ops.len());
        for op in &sequenced.ops {
            match op {
                Operation::Create { doc } => {
                    Self::upsert_document(&mut tx, doc, seq).await?;
                    post_images.insert(doc.id, doc.clone());
                    normalized_ops.push(op.clone());
                }
                Operation::Delete { doc } => {
                    if let Some(cs) = Self::document_created_seq(&mut *tx, doc.id).await? {
                        deleted_created_seqs.insert(doc.id, cs);
                    }
                    Self::delete_document_tx(&mut tx, doc.id).await?;
                    normalized_ops.push(op.clone());
                }
                Operation::Move {
                    doc_id, parent_id, ..
                } => {
                    let cur = Self::load_document(&mut *tx, *doc_id)
                        .await?
                        .ok_or(DataError::NotFound)?;
                    if cur.parent_id == *parent_id {
                        // No-op: carried in the log for invertibility;
                        // nothing written, nothing bumped, no hooks run.
                        post_images.insert(*doc_id, cur);
                        normalized_ops.push(op.clone());
                    } else {
                        let mut doc = cur;
                        doc.parent_id = *parent_id;
                        doc.updated_at = ts;
                        Self::upsert_document(&mut tx, &doc, seq).await?;
                        // A folder's ancestor names are derived tags on every
                        // asset beneath it; re-parenting recomputes the whole
                        // subtree in this tx, same as the rename hook below.
                        if doc.doc_type == crate::data::engine::ASSET_FOLDER_DOC_TYPE {
                            Self::refresh_derived_tags_for_folder_subtree(&mut tx, doc.id).await?;
                        }
                        post_images.insert(*doc_id, doc);
                        normalized_ops.push(op.clone());
                    }
                }
                Operation::Update { doc_id, changes } => {
                    let (pre_doc, mut doc) =
                        merge_update_document(&mut *tx, *doc_id, changes).await?;
                    // Captured before this op's own `changes` apply — the
                    // TRUE stored pre-image `derive_engine_side_effects`
                    // diffs against below, to surface a normalize-time side
                    // effect on an engine key none of this op's own `changes`
                    // named (e.g. `NoteEngine::derive_body`). Capturing after
                    // the merge would compare the post-`changes`,
                    // pre-normalize value against itself,
                    // reporting the wrong `old` for a nested request this
                    // op's own change already applied.
                    let pre_engine = pre_doc.engine.clone();
                    check_command_scope(&doc, world_id)?;
                    // Embedded children NEVER carry `base` (the Create arm's
                    // `derive_create_base` strips it recursively), but a
                    // client-origin Update can still land one on the
                    // post-image — a `/embedded/<coll>/<i>/base` leaf write or
                    // a whole-collection replacement carrying base-bearing
                    // children — so the merged post-image is checked here,
                    // fail-closed, BEFORE the shape/engine walks below (the
                    // invariant is simpler than anything they check). Server-
                    // authored origins skip it: `WriteOrigin::TemplateMerge`'s
                    // whole-collection rewrites carry restamped/merged
                    // children, which never carry `base` by construction
                    // (`merge::plan::plan_to_update`), as do the other trusted
                    // origins' embedded writes.
                    if !origin.is_server_authored()
                        && crate::merge::bands::embedded_carries_base(&doc)
                    {
                        return Err(DataError::Forbidden);
                    }
                    // Body cap re-checked post-merge: the merged result, not the
                    // pre-image, is what gets stored.
                    validation::validate_system_size(&doc)?;
                    validation::validate_property_overrides(&doc)?;
                    // Engine band re-validated + normalized post-merge (mutates
                    // `doc.engine` in place to the re-serialized validated
                    // struct — see `validate_engine_tree`'s doc comment).
                    validation::validate_engine_tree(&mut doc)?;
                    // Re-checked AFTER `validate_engine_tree`: for a `note`
                    // document that call replaces `doc.engine` with the
                    // SERVER-DERIVED body (`NoteEngine::derive_body`), so the
                    // cap above ran against the merged pre-derivation
                    // payload, not the value about to be stored, written to
                    // `world_events`, and broadcast. Reusing
                    // `validate_system_size` keeps ONE cap statement that now
                    // covers both the merged and the derived shape.
                    validation::validate_system_size(&doc)?;
                    validation::validate_containment(&doc)?;
                    // The doc is already fully normalized at this point (the
                    // second `validate_system_size` above ran against the
                    // SAME post-`validate_engine_tree` value), so the true
                    // derived side effects can be computed and capability-
                    // gated here, BEFORE anything is written -- fail closed,
                    // nothing stored, on a denial.
                    let requested_paths_for_derived: std::collections::HashSet<String> =
                        changes.iter().map(|ch| ch.path.clone()).collect();
                    let derived_changes_preview =
                        crate::data::validation::derive_engine_side_effects(
                            &doc.doc_type,
                            pre_engine.as_ref(),
                            doc.engine.as_ref(),
                            &requested_paths_for_derived,
                        );
                    // Phase 1's resolved `Access` for this exact Update op —
                    // never re-resolved here (see `update_access`'s doc
                    // comment). Absent only for a capability-skipping origin
                    // (`CombatTransition`/`TemplateMerge`/`ConfigSeed`), which
                    // this check exempts the same way `declared_caps_for_path`
                    // is exempted for every other write path in Phase 1 above
                    // — a capability-skipping origin's authorization is
                    // vetted entirely elsewhere (see the Phase-1 doc comments
                    // beside `origin.skips_capability_gates()`), and a
                    // derived engine path is no exception to that trust.
                    if !origin.skips_capability_gates() {
                        let access_for_derived = update_access_iter
                            .next()
                            .expect("one Access recorded per Update op in Phase 1");
                        for extra in &derived_changes_preview {
                            for cap_needed in declared_caps_for_path(&extra.path, &world_reqs) {
                                if !access_for_derived.has(cap_needed) {
                                    tracing::debug!(
                                        user = %ctx.user_id, path = %extra.path, capability = cap_needed,
                                        "intent denied: missing declared capability on a derived engine path"
                                    );
                                    return Err(DataError::Forbidden);
                                }
                            }
                        }
                    } else {
                        // Keep the iterator in lockstep with Phase 1's push
                        // order even when this op's check is skipped — Phase
                        // 1 still pushed an entry for it.
                        update_access_iter.next();
                    }
                    // One-active-combat-per-scene is validated ONLY in Phase
                    // 1 (see `apply_intent`'s Update arm there, and the
                    // `scene_owner` doc comment) -- this phase trusts that
                    // decision and performs no recomputation of it, the same
                    // way it trusts Phase 1's OCC/capability/containment/
                    // singleton decisions for every other check.
                    // Tier-2 structural schema gate on the MERGED post-image
                    // (existing row + applied `FieldChange`s), matching
                    // `validate_engine_tree` above: never the pre-image.
                    validation::validate_system_schema_tree(&doc, &world_schemas)?;
                    // In-transaction validator re-validation (the TOCTOU half of
                    // the pre-transaction pass): the pass validated a post-image
                    // merged from a pre-transaction read, and Phase 1's OCC covers
                    // only each change's own pointer — a concurrent write to any
                    // OTHER path of this document would pass OCC and commit a
                    // post-image no validator ever saw. When the in-transaction
                    // pre-image differs from the pass's capture, the SAME
                    // `sandbox::validate_document` re-runs here against the final,
                    // fully-normalized post-image that actually commits. Re-
                    // validation, never `Conflict`: refusing on any concurrent
                    // write would let two writers livelock a protected document
                    // (each one's write invalidates the other's validation).
                    if let (
                        Some((registry, enabled_module_ids)),
                        Some((captured_pre, prior_permitted)),
                    ) = (&validator_pass, validated_pre_images.get(doc_id))
                    {
                        let pre_doc_json = serde_json::to_value(&pre_doc)?;
                        let captured_json = serde_json::to_value(captured_pre)?;
                        if !crate::data::command::values_semantically_eq(
                            &pre_doc_json,
                            &captured_json,
                        ) {
                            match crate::sandbox::validate_document(
                                registry,
                                enabled_module_ids,
                                &mut doc,
                                Some(&pre_doc),
                                *prior_permitted,
                                world_id,
                                &world_schemas,
                            )
                            .await
                            {
                                Err(structural_err) => return Err(structural_err),
                                Ok(crate::sandbox::ValidatorVerdict::Accept) => {}
                                Ok(crate::sandbox::ValidatorVerdict::Refuse { module, reason }) => {
                                    return Err(DataError::OpFailed(format!(
                                        "validator {module}: {reason}"
                                    )));
                                }
                                Ok(crate::sandbox::ValidatorVerdict::Fault(fault)) => {
                                    return Err(DataError::Validator(fault));
                                }
                            }
                        }
                    }
                    doc.updated_at = ts;
                    Self::upsert_document(&mut tx, &doc, seq).await?;
                    post_images.insert(*doc_id, doc.clone());
                    // A folder's name is a derived tag on every asset beneath
                    // it; any Update to an `asset_folder` (rename being the
                    // one that matters) recomputes that subtree in this tx.
                    if doc.doc_type == crate::data::engine::ASSET_FOLDER_DOC_TYPE {
                        Self::refresh_derived_tags_for_folder_subtree(&mut tx, doc.id).await?;
                    }

                    // `validate_engine_tree` above normalizes `doc.engine` (a
                    // JSON-number literal coerced to its typed f64
                    // representation; an unknown key smuggled into a
                    // tagged-enum sub-object dropped by the
                    // deserialize-then-reserialize round trip), and that
                    // normalization reaches `doc` alone — the caller's own
                    // `FieldChange.new` values are untouched by it.
                    // Re-derive each `/engine`(/*) `FieldChange.new` from the
                    // SAME validated post-image so the broadcast delta and the
                    // `world_events` log entry (and therefore every future
                    // `events_since` replay) carry the identical normalized
                    // value the row was stored with — never the raw
                    // client-submitted JSON. `/system`-prefixed changes are
                    // untouched: only the structurally-typed engine band goes
                    // through `validate_engine_tree`.
                    let normalized_doc_json = serde_json::to_value(&doc)?;
                    let mut normalized_changes: Vec<FieldChange> = changes
                        .iter()
                        .map(|ch| {
                            if ch.path == "/engine" || ch.path.starts_with("/engine/") {
                                if let Some(v) = normalized_doc_json.pointer(&ch.path) {
                                    return FieldChange {
                                        remove: false,
                                        path: ch.path.clone(),
                                        old: ch.old.clone(),
                                        new: v.clone(),
                                    };
                                }
                            }
                            ch.clone()
                        })
                        .collect();
                    // A normalize-time derivation (e.g. `NoteEngine::derive_body`) can change
                    // an engine key none of this op's own `changes` named — surface those too,
                    // or the broadcast/log/author's own optimistic store never see them.
                    // Reuses `derived_changes_preview`, already computed and
                    // capability-gated above — never recomputed here.
                    normalized_changes.extend(derived_changes_preview);
                    normalized_ops.push(Operation::Update {
                        doc_id: *doc_id,
                        changes: normalized_changes,
                    });
                }
            }
        }
        sequenced.ops = normalized_ops;

        let world_gm_at_commit: std::collections::HashMap<Uuid, bool> =
            Self::world_member_roles(&mut *tx, world_id)
                .await?
                .into_iter()
                .map(|(uid, role)| (uid, role == WorldRole::Gm))
                .collect();
        let mut per_op = Vec::with_capacity(sequenced.ops.len());
        for op in &sequenced.ops {
            per_op.push(Some(
                Self::build_op_snapshot(
                    &mut tx,
                    op,
                    &post_images,
                    &deleted_created_seqs,
                    &pre_permissions,
                    &pre_owners,
                )
                .await?,
            ));
        }
        let stored = StoredCommand {
            command: sequenced,
            snapshot: CommandSnapshot {
                per_op,
                world_gm_at_commit,
            },
        };

        sqlx::query("INSERT INTO world_events (world_id, seq, author_id, ts, command_json) VALUES (?, ?, ?, ?, ?)")
            .bind(stored.command.world_id.to_string())
            .bind(seq)
            .bind(stored.command.author.to_string())
            .bind(stored.command.ts)
            .bind(serde_json::to_string(&stored)?)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(stored)
    }

    async fn get_document(&self, id: Uuid) -> Result<Option<Document>, DataError> {
        let row = sqlx::query("SELECT json FROM documents WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(r) => Ok(Some(serde_json::from_str(
                r.get::<String, _>("json").as_str(),
            )?)),
            None => Ok(None),
        }
    }

    async fn get_document_with_created_seq(
        &self,
        id: Uuid,
    ) -> Result<Option<(Document, i64)>, DataError> {
        let row = sqlx::query("SELECT json, created_seq FROM documents WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(r) => {
                let doc: Document = serde_json::from_str(r.get::<String, _>("json").as_str())?;
                let created_seq: i64 = r.get("created_seq");
                Ok(Some((doc, created_seq)))
            }
            None => Ok(None),
        }
    }

    async fn effective_owner_of(&self, doc: &Document) -> Result<Option<Uuid>, DataError> {
        Self::load_effective_owner(&self.pool, doc).await
    }

    async fn query_documents(
        &self,
        world_id: Uuid,
        doc_type: &str,
    ) -> Result<Vec<Document>, DataError> {
        let rows = sqlx::query(
            "SELECT json FROM documents WHERE world_id = ? AND doc_type = ? ORDER BY id",
        )
        .bind(world_id.to_string())
        .bind(doc_type)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| Ok(serde_json::from_str(r.get::<String, _>("json").as_str())?))
            .collect()
    }

    async fn query_documents_by_types(
        &self,
        world_id: Uuid,
        doc_types: &[&str],
    ) -> Result<Vec<Document>, DataError> {
        if doc_types.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = std::iter::repeat_n("?", doc_types.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT json FROM documents WHERE world_id = ? AND doc_type IN ({placeholders}) ORDER BY id"
        );
        // The interpolated segment is only a fixed-count `?, ?, ...` placeholder list built
        // from `doc_types.len()`, never caller-supplied string content — every actual value
        // (world_id, each doc_type) is bound as a parameter below, so this is not injectable.
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(world_id.to_string());
        for doc_type in doc_types {
            query = query.bind(*doc_type);
        }
        let rows = query.fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|r| Ok(serde_json::from_str(r.get::<String, _>("json").as_str())?))
            .collect()
    }

    async fn query_all_documents(&self, world_id: Uuid) -> Result<Vec<Document>, DataError> {
        let rows = sqlx::query("SELECT json FROM documents WHERE world_id = ? ORDER BY id")
            .bind(world_id.to_string())
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|r| Ok(serde_json::from_str(r.get::<String, _>("json").as_str())?))
            .collect()
    }

    async fn query_children(&self, parent: Uuid) -> Result<Vec<Document>, DataError> {
        let rows = sqlx::query("SELECT json FROM documents WHERE parent_id = ? ORDER BY id")
            .bind(parent.to_string())
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|r| Ok(serde_json::from_str(r.get::<String, _>("json").as_str())?))
            .collect()
    }

    async fn query_scene_entities(&self, world: Uuid) -> Result<Vec<Document>, DataError> {
        let rows = sqlx::query(
            "SELECT json FROM documents WHERE world_id = ? \
             AND (doc_type = 'scene' OR parent_id IS NOT NULL) ORDER BY id",
        )
        .bind(world.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| Ok(serde_json::from_str(r.get::<String, _>("json").as_str())?))
            .collect()
    }

    async fn documents_by_source(
        &self,
        pack: Option<&str>,
        source_id: Uuid,
    ) -> Result<Vec<Document>, DataError> {
        let rows = match pack {
            Some(p) => {
                sqlx::query(
                    "SELECT json FROM documents WHERE source_pack = ? AND source_id = ? ORDER BY id",
                )
                .bind(p)
                .bind(source_id.to_string())
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query(
                    "SELECT json FROM documents WHERE source_pack IS NULL AND source_id = ? ORDER BY id",
                )
                .bind(source_id.to_string())
                .fetch_all(&self.pool)
                .await?
            }
        };
        rows.into_iter()
            .map(|r| Ok(serde_json::from_str(r.get::<String, _>("json").as_str())?))
            .collect()
    }

    async fn instances_of(
        &self,
        world_id: Uuid,
        template_id: Uuid,
    ) -> Result<Vec<Document>, DataError> {
        let rows = sqlx::query(
            "SELECT json FROM documents WHERE world_id = ? \
             AND source_pack IS NULL AND source_id = ? ORDER BY id",
        )
        .bind(world_id.to_string())
        .bind(template_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| Ok(serde_json::from_str(r.get::<String, _>("json").as_str())?))
            .collect()
    }

    async fn events_since(
        &self,
        world_id: Uuid,
        seq: i64,
    ) -> Result<Vec<StoredCommand>, DataError> {
        let rows = sqlx::query(
            "SELECT command_json FROM world_events WHERE world_id = ? AND seq > ? ORDER BY seq",
        )
        .bind(world_id.to_string())
        .bind(seq)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| {
                StoredCommand::from_stored_json(r.get::<String, _>("command_json").as_str())
                    .map_err(DataError::from)
            })
            .collect()
    }

    async fn get_world(&self, id: Uuid) -> Result<Option<World>, DataError> {
        let row =
            sqlx::query("SELECT id, name, seq, created_at, updated_at FROM worlds WHERE id = ?")
                .bind(id.to_string())
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|r| World {
            id: Uuid::parse_str(r.get::<String, _>("id").as_str()).unwrap(),
            name: r.get("name"),
            seq: r.get("seq"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        }))
    }

    async fn member_role(&self, world: Uuid, user: Uuid) -> Result<Option<WorldRole>, DataError> {
        // Delegates to `SqliteRepository::member_role`, the inherent method of
        // the same name; method resolution on a concrete `SqliteRepository`
        // self prefers the inherent impl, so this is not infinite recursion.
        SqliteRepository::member_role(self, world, user).await
    }

    async fn member_id_by_username(
        &self,
        world: Uuid,
        username: &str,
    ) -> Result<Option<Uuid>, DataError> {
        // Delegates to the inherent method of the same name (see `member_role`
        // above for why this is not infinite recursion).
        SqliteRepository::member_id_by_username(self, world, username).await
    }

    async fn world_cap_defaults(&self, world: Uuid) -> Result<WorldCapDefaults, DataError> {
        match self.get_setting(&world_caps_key(world)).await? {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Ok(WorldCapDefaults::default()),
        }
    }

    async fn world_cap_requirements(
        &self,
        world: Uuid,
    ) -> Result<Vec<CapabilityRequirement>, DataError> {
        match self.get_setting(&world_caps_req_key(world)).await? {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Ok(Vec::new()),
        }
    }

    async fn world_contract_declarations(
        &self,
        world: Uuid,
    ) -> Result<Vec<ContractDeclaration>, DataError> {
        match self.get_setting(&world_contracts_key(world)).await? {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Ok(Vec::new()),
        }
    }

    async fn world_schema_declarations(
        &self,
        world: Uuid,
    ) -> Result<Vec<SchemaDeclaration>, DataError> {
        match self.get_setting(&world_schemas_key(world)).await? {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Ok(Vec::new()),
        }
    }

    async fn world_enabled_modules(
        &self,
        world: Uuid,
    ) -> Result<Vec<crate::modules::WorldModuleEntry>, DataError> {
        match self.get_setting(&world_modules_key(world)).await? {
            Some(json) => Ok(crate::modules::WorldModuleEntry::parse_legacy_tolerant(
                &json,
            )?),
            None => Ok(Vec::new()),
        }
    }

    async fn set_world_enabled_modules(
        &self,
        world: Uuid,
        entries: &[crate::modules::WorldModuleEntry],
    ) -> Result<(), DataError> {
        let json = serde_json::to_string(entries)?;
        self.set_setting(&world_modules_key(world), &json).await
    }

    async fn reset_validator_fault_streak(&self, world: Uuid, module: &str) {
        self.validator_registry_cache.reset_faults(world, module);
    }

    async fn search(
        &self,
        ctx: &crate::data::membership::PermissionContext,
        world_id: Uuid,
        query: &str,
        limit: u32,
        cursor: Option<i64>,
        doc_types: &[String],
    ) -> Result<crate::data::search::SearchPage, DataError> {
        use crate::data::search::{build_match, SearchHit, SearchPage, MAX_SEARCH_DOC_TYPES};

        // Bound the candidates examined per request: a query matching many docs
        // the actor cannot read would otherwise page to exhaustion, one
        // get_document per candidate, on the single-writer pool. On hitting the
        // budget before `limit`, return a partial page + cursor to resume.
        const MAX_SCAN: i64 = 500;

        if doc_types.len() > MAX_SEARCH_DOC_TYPES {
            return Err(DataError::OpFailed("too many doc types".into()));
        }

        let limit = limit.clamp(1, 100) as usize;
        let Some(match_expr) = build_match(query) else {
            return Ok(SearchPage {
                hits: Vec::new(),
                next_cursor: None,
            });
        };
        let world_defaults = self.world_cap_defaults(world_id).await?;

        // Visibility-split index: a non-GM matches, scores, and snippets only
        // against `documents_fts_public` (GM-only properties stripped at
        // index time), so neither the MATCH (oracle), the bm25 score, nor the
        // snippet can reveal GM-only text. A GM/admin searches the separate
        // `documents_fts_gm` table. (Server admin resolves to the Gm world
        // role in `permission_context`.)
        //
        // TWO SEPARATE single-column tables, not two columns of one table:
        // SQLite FTS5's bm25() computes each row's document-length
        // normalization term from the token count of
        // the WHOLE ROW (every declared column combined), not just the
        // matched/weighted column — a documented FTS5 characteristic. In a
        // shared two-column table, per-column bm25() weight arguments zero a
        // column's term-frequency*IDF CONTRIBUTION but cannot remove its
        // tokens from that shared row-length denominator, so a non-GM
        // searcher's score still shifts by the sheer LENGTH of GM-only text
        // on the same row — even text that never matches the query. Separate
        // tables make each tier's row length genuinely isolated: a non-GM
        // query's table contains no GM-only text in any column of any row.
        let is_gm = ctx.world_role == WorldRole::Gm;
        let table = if is_gm {
            "documents_fts_gm"
        } else {
            "documents_fts_public"
        };

        // Iterate the BM25-ranked candidates from `cursor`, reading each doc and
        // keeping only those the actor may read, until `limit` readable hits are
        // collected, the candidates are exhausted, or the scan budget is spent.
        // Over-iteration here is what prevents redaction from producing a short
        // page. A negative client cursor is clamped to the start.
        let start: i64 = cursor.unwrap_or(0).max(0);
        let mut offset: i64 = start;
        let mut hits: Vec<SearchHit> = Vec::with_capacity(limit);
        let batch: i64 = (limit as i64).clamp(16, MAX_SCAN);
        let mut next_cursor: Option<i64> = None;

        'outer: loop {
            // A `sqlx::QueryBuilder`, not a numbered-placeholder `&'static str`:
            // the optional `doc_type IN (...)` clause needs a variable-length
            // bind list, and mixing explicit `?1..?N` with trailing bare `?`
            // placeholders in the same statement is not a pattern used
            // elsewhere in this crate — the builder pushes every bind in
            // textual order instead, so bind order is always the push order.
            let mut qb: sqlx::QueryBuilder<sqlx::Sqlite> = sqlx::QueryBuilder::new(format!(
                "SELECT doc_id, bm25({table}) AS score, \
                 snippet({table}, 0, '<mark>', '</mark>', '…', 16) AS snippet \
                 FROM {table} WHERE {table} MATCH "
            ));
            qb.push_bind(match_expr.clone());
            qb.push(" AND world_id = ");
            qb.push_bind(world_id.to_string());
            if !doc_types.is_empty() {
                qb.push(" AND doc_type IN (");
                for (i, dt) in doc_types.iter().enumerate() {
                    if i > 0 {
                        qb.push(", ");
                    }
                    qb.push_bind(dt.clone());
                }
                qb.push(")");
            }
            qb.push(" ORDER BY score LIMIT ");
            qb.push_bind(batch);
            qb.push(" OFFSET ");
            qb.push_bind(offset);

            let rows = qb.build().fetch_all(&self.pool).await?;

            if rows.is_empty() {
                break; // exhausted; next_cursor stays None
            }

            for row in &rows {
                offset += 1;
                let doc_id: String = row.get("doc_id");
                let doc_id =
                    Uuid::parse_str(&doc_id).map_err(|e| DataError::OpFailed(e.to_string()))?;
                let Some(doc) = self.get_document(doc_id).await? else {
                    continue;
                };
                // One extra pool read per linked-token candidate, bounded by
                // `MAX_SCAN`; the ws hot path never enters here.
                let owner = Self::load_effective_owner(&self.pool, &doc).await?;
                let access = resolve_access_world(
                    ctx.user_id,
                    ctx.world_role,
                    &doc,
                    &world_defaults.grants_for(&doc.doc_type),
                    owner,
                );
                if !access.has(cap::READ) {
                    continue;
                }
                let document = match crate::data::permission::filter_properties(&doc, &access) {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::warn!(doc_id = %doc.id, error = %e, "omitting search hit");
                        continue;
                    }
                };
                hits.push(SearchHit {
                    document,
                    score: row.get("score"),
                    snippet: row.get("snippet"),
                });
                if hits.len() == limit {
                    // More candidates may remain; hand back the rank offset.
                    next_cursor = Some(offset);
                    break 'outer;
                }
            }

            if offset - start >= MAX_SCAN {
                // Scan budget spent before `limit`; resume from here next page.
                next_cursor = Some(offset);
                break;
            }
            if (rows.len() as i64) < batch {
                break; // last batch was partial → no more candidates
            }
        }

        Ok(SearchPage { hits, next_cursor })
    }

    async fn get_explored(&self, scene: Uuid, user: Uuid) -> Result<Option<Vec<u8>>, DataError> {
        // Delegate to the concrete method on SqliteRepository (same query, exposed
        // on the trait so Room::publish can call it through &dyn Repository).
        SqliteRepository::get_explored(self, scene, user).await
    }

    async fn get_link_preview_cache(
        &self,
        url: &str,
    ) -> Result<Option<crate::data::repository::LinkPreviewCacheRow>, DataError> {
        SqliteRepository::get_link_preview_cache(self, url).await
    }

    async fn upsert_link_preview_cache(
        &self,
        url: &str,
        title: Option<&str>,
        description: Option<&str>,
        fetched_at_ms: i64,
    ) -> Result<(), DataError> {
        SqliteRepository::upsert_link_preview_cache(self, url, title, description, fetched_at_ms)
            .await
    }

    async fn set_link_preview_cache_image(
        &self,
        url: &str,
        image_asset_id: Uuid,
    ) -> Result<(), DataError> {
        SqliteRepository::set_link_preview_cache_image(self, url, image_asset_id).await
    }

    async fn get_asset(&self, id: Uuid) -> Result<Option<crate::data::asset::Asset>, DataError> {
        SqliteRepository::get_asset(self, id).await
    }
}

/// Settings key holding a world's default capability grants (JSON).
fn world_caps_key(world: Uuid) -> String {
    format!("world_caps:{world}")
}

/// Settings key holding a world's declarative capability requirements (JSON).
fn world_caps_req_key(world: Uuid) -> String {
    format!("world_caps_req:{world}")
}

/// Settings key holding a world's UI contract declarations (JSON).
fn world_contracts_key(world: Uuid) -> String {
    format!("world_contracts:{world}")
}

/// Settings key holding a world's structural schema declarations (JSON).
fn world_schemas_key(world: Uuid) -> String {
    format!("world_schemas:{world}")
}

/// Settings key holding a world's enabled installed-module ids (JSON).
fn world_modules_key(world: Uuid) -> String {
    format!("world_modules:{world}")
}

/// The per-world `settings` keys. SINGLE SOURCE for "what world-scoped
/// settings blobs exist": `delete_world`'s purge iterates this array, so a
/// new per-world blob added here is purged automatically (never-fork; adding
/// a sixth key fn without extending this array is the drift this prevents).
fn world_settings_keys(world: Uuid) -> [String; 5] {
    [
        world_caps_key(world),
        world_caps_req_key(world),
        world_contracts_key(world),
        world_schemas_key(world),
        world_modules_key(world),
    ]
}

mod assets;
mod documents;
mod export_import;
mod membership;
mod notes;
mod ui_state;
mod worlds;

#[cfg(test)]
mod tests;
