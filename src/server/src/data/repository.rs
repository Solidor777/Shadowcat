// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use async_trait::async_trait;
use uuid::Uuid;

use crate::data::command::UnsequencedCommand;
use crate::data::document::{
    CapabilityRequirement, ContractDeclaration, Document, SchemaDeclaration, World,
    WorldCapDefaults, WorldRole,
};
use crate::data::DataError;

/// One row from the persisted `link_preview_cache` table
/// (`Repository::get_link_preview_cache`).
///
/// # Examples
///
/// ```
/// use shadowcat::data::repository::LinkPreviewCacheRow;
///
/// let row = LinkPreviewCacheRow {
///     title: Some("MOCK_TITLE".into()),
///     description: None,
///     image_asset_id: None,
///     fetched_at_ms: 0,
/// };
/// assert_eq!(row.title.as_deref(), Some("MOCK_TITLE"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkPreviewCacheRow {
    /// Server-extracted title, or `None` for a cached negative-outcome row
    /// (both `title` and `description` `None` together).
    pub title: Option<String>,
    /// Server-extracted description, or `None` for a cached negative-outcome row.
    pub description: Option<String>,
    /// The asset-ified `og:image`/oEmbed-thumbnail, once the post-publish
    /// background pipeline resolves one for this URL.
    pub image_asset_id: Option<Uuid>,
    /// When this row was last (re-)fetched, Unix epoch milliseconds.
    pub fetched_at_ms: i64,
}

/// Storage contract. The only implementation today is `SqliteRepository`;
/// the trait exists so Postgres can be added later behind the same surface.
///
/// # Examples
///
/// ```
/// # #[tokio::main]
/// # async fn main() -> Result<(), shadowcat::data::DataError> {
/// use shadowcat::data::repository::Repository;
/// use shadowcat::data::sqlite::SqliteRepository;
///
/// // `SqliteRepository` is the trait's sole implementor today.
/// let repo: Box<dyn Repository> = Box::new(SqliteRepository::connect("sqlite::memory:").await?);
/// assert!(repo.get_document(uuid::Uuid::nil()).await?.is_none());
/// # Ok(())
/// # }
/// ```
#[async_trait]
pub trait Repository: Send + Sync {
    /// Allocate the next per-world seq, append the command to the log, and
    /// apply every operation to the document store — all in one transaction.
    /// This is the trusted substrate (undo/replay): unlike `apply_intent`,
    /// it runs no capability/schema/size checks. It DOES run the same
    /// `/engine` ingress gate as `apply_intent` on Create and Update
    /// (`validate_engine_tree`, re-deriving normalized `FieldChange` values
    /// for `/engine`(/*)) — normalize-then-store is data integrity, not
    /// authz, so it applies regardless of caller trust level. A malformed
    /// or absent engine body on an engine-defined `doc_type` is rejected
    /// with `DataError::BadEngine`. Returns the commit-time redaction snapshot alongside the
    /// command — see StoredCommand.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::command::{Operation, UnsequencedCommand};
    /// use shadowcat::data::document::{Document, PermissionSet, Scope};
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    ///
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let author = repo.create_user("mock_author", None, ServerRole::User, 0).await?;
    /// let world = repo.create_world_owned("MOCK_WORLD", author, 0).await?;
    /// let doc = Document {
    ///     id: uuid::Uuid::new_v4(),
    ///     scope: Scope::World { world_id: world.id },
    ///     doc_type: "item".into(),
    ///     schema_version: 1,
    ///     name: Some("MOCK_ITEM".into()),
    ///     source: None,
    ///     base: None,
    ///     owner: Some(author),
    ///     permissions: PermissionSet::default(),
    ///     embedded: Default::default(),
    ///     parent_id: None,
    ///     engine: None,
    ///     system: serde_json::json!({}),
    ///     created_at: 0,
    ///     updated_at: 0,
    /// };
    /// let cmd = UnsequencedCommand {
    ///     world_id: world.id,
    ///     author,
    ///     ts: 0,
    ///     ops: vec![Operation::Create { doc }],
    /// };
    /// let stored = repo.apply_command(cmd).await?;
    /// assert_eq!(stored.command.ops.len(), 1);
    /// # Ok(())
    /// # }
    /// ```
    async fn apply_command(
        &self,
        cmd: UnsequencedCommand,
    ) -> Result<crate::data::snapshot::StoredCommand, DataError>;

    /// Authorize (per `ctx`), structurally validate, and check per-op
    /// pre-images, then sequence + apply + log — all in one transaction.
    /// Field-level optimistic concurrency: an `Update` whose `FieldChange.old`
    /// does not match the current stored value yields `Conflict`. A failure in
    /// the authorize phase consumes no seq (the transaction rolls back whole).
    /// `origin` gates the message-Update exemption: a stored `message` doc's
    /// `Update` is blanket-rejected for `WriteOrigin::Client` regardless of the
    /// requester's own `DocRole`; only `WriteOrigin::ServerMessageRevision` —
    /// set exclusively by the server edit/delete handlers — re-opens it, and
    /// only for that call's sanitized authoritative revision.
    /// Returns the commit-time redaction snapshot alongside the command — see StoredCommand.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::command::{Operation, WriteOrigin};
    /// use shadowcat::data::document::{Document, PermissionSet, Scope, WorldRole};
    /// use shadowcat::data::membership::PermissionContext;
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    ///
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let gm = repo.create_user("mock_gm", None, ServerRole::User, 0).await?;
    /// let world = repo.create_world_owned("MOCK_WORLD", gm, 0).await?;
    /// let doc = Document {
    ///     id: uuid::Uuid::new_v4(),
    ///     scope: Scope::World { world_id: world.id },
    ///     doc_type: "item".into(),
    ///     schema_version: 1,
    ///     name: Some("MOCK_ITEM".into()),
    ///     source: None,
    ///     base: None,
    ///     owner: Some(gm),
    ///     permissions: PermissionSet::default(),
    ///     embedded: Default::default(),
    ///     parent_id: None,
    ///     engine: None,
    ///     system: serde_json::json!({}),
    ///     created_at: 0,
    ///     updated_at: 0,
    /// };
    /// let ctx = PermissionContext { user_id: gm, world_role: WorldRole::Gm };
    /// let stored = repo
    ///     .apply_intent(&ctx, world.id, vec![Operation::Create { doc }], 0, WriteOrigin::Client)
    ///     .await?;
    /// assert_eq!(stored.command.seq, 1);
    /// # Ok(())
    /// # }
    /// ```
    async fn apply_intent(
        &self,
        ctx: &crate::data::membership::PermissionContext,
        world_id: Uuid,
        ops: Vec<crate::data::command::Operation>,
        ts: i64,
        origin: crate::data::command::WriteOrigin,
    ) -> Result<crate::data::snapshot::StoredCommand, DataError>;

    /// Fetch one document by id, or `None` if it does not exist. Unredacted —
    /// callers gate egress via `resolve_access` + `filter_properties`.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// assert!(repo.get_document(uuid::Uuid::nil()).await?.is_none());
    /// # Ok(())
    /// # }
    /// ```
    async fn get_document(&self, id: Uuid) -> Result<Option<Document>, DataError>;

    /// A document by id together with its `documents.created_seq` generation marker, or `None`
    /// if it does not exist. One round trip, not two: this is the redaction hot path's own read
    /// (`permission::load_current_docs`, called once per recipient per event), where a second
    /// separate `created_seq` query would double an already-hot per-recipient cost. Unredacted,
    /// like `get_document` — callers gate egress themselves.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// assert!(repo.get_document_with_created_seq(uuid::Uuid::nil()).await?.is_none());
    /// # Ok(())
    /// # }
    /// ```
    async fn get_document_with_created_seq(
        &self,
        id: Uuid,
    ) -> Result<Option<(Document, i64)>, DataError>;

    /// Resolve `doc`'s effective owner against LIVE actor state — the same
    /// `permission::effective_owner` rule the write path enforces, joining the
    /// linked actor with one pool read when `doc` is a linked token. For egress
    /// read routes and search; the ws broadcast hot path joins through the room's
    /// in-memory actor table instead (zero pool reads per recipient).
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::document::{Document, PermissionSet, Scope};
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    ///
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let owner = uuid::Uuid::new_v4();
    /// let doc = Document {
    ///     id: uuid::Uuid::new_v4(),
    ///     scope: Scope::World { world_id: uuid::Uuid::nil() },
    ///     doc_type: "item".into(),
    ///     schema_version: 1,
    ///     name: None,
    ///     source: None,
    ///     base: None,
    ///     owner: Some(owner),
    ///     permissions: PermissionSet::default(),
    ///     embedded: Default::default(),
    ///     parent_id: None,
    ///     engine: None,
    ///     system: serde_json::json!({}),
    ///     created_at: 0,
    ///     updated_at: 0,
    /// };
    /// // "item" is not the linked-token doc_type, so ownership is just `doc.owner`.
    /// assert_eq!(repo.effective_owner_of(&doc).await?, Some(owner));
    /// # Ok(())
    /// # }
    /// ```
    async fn effective_owner_of(&self, doc: &Document) -> Result<Option<Uuid>, DataError>;

    /// All documents of one `doc_type` in `world_id` (unredacted; egress-gated
    /// by callers).
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let actors = repo.query_documents(uuid::Uuid::nil(), "actor").await?;
    /// assert!(actors.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    async fn query_documents(
        &self,
        world_id: Uuid,
        doc_type: &str,
    ) -> Result<Vec<Document>, DataError>;

    /// All documents in `world` whose `doc_type` is any of `doc_types`, in one
    /// query. Equivalent to unioning `query_documents` per type, but halves DB
    /// round-trips when a caller needs several independent doc_type singletons
    /// at once (e.g. room cold-hydration's four config/actor doc_types).
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let docs = repo.query_documents_by_types(uuid::Uuid::nil(), &["actor", "scene"]).await?;
    /// assert!(docs.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    async fn query_documents_by_types(
        &self,
        world_id: Uuid,
        doc_types: &[&str],
    ) -> Result<Vec<Document>, DataError>;

    /// Every document in `world`, regardless of `doc_type`. Used by the current-state
    /// snapshot endpoint (`http::routes::world_snapshot`) — unlike `query_documents`/
    /// `query_documents_by_types`, which require the caller to already know which
    /// type(s) it wants, a snapshot needs every document a recipient might have
    /// access to, including arbitrary community-module-defined `system`-band doc_types
    /// this server has no fixed enum of.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let docs = repo.query_all_documents(uuid::Uuid::nil()).await?;
    /// assert!(docs.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    async fn query_all_documents(&self, world_id: Uuid) -> Result<Vec<Document>, DataError>;

    /// All documents whose `parent_id` equals `parent` (a scene's direct
    /// children). Ordered by id for determinism.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let children = repo.query_children(uuid::Uuid::nil()).await?;
    /// assert!(children.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    async fn query_children(&self, parent: Uuid) -> Result<Vec<Document>, DataError>;

    /// All scene-entity documents in `world` — scenes plus anything with a
    /// parent. Mirrors `scene::is_scene_entity` so initial ECS hydration and the
    /// live `apply_op` path share one definition of "scene entity".
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let entities = repo.query_scene_entities(uuid::Uuid::nil()).await?;
    /// assert!(entities.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    async fn query_scene_entities(&self, world: Uuid) -> Result<Vec<Document>, DataError>;

    /// Instances stamped from a given source (`source.id` + optional pack) —
    /// the template push path's audience query.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let instances = repo.documents_by_source(None, uuid::Uuid::nil()).await?;
    /// assert!(instances.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    async fn documents_by_source(
        &self,
        pack: Option<&str>,
        source_id: Uuid,
    ) -> Result<Vec<Document>, DataError>;

    /// All instances OF A WORLD TEMPLATE in one world: documents in `world_id`
    /// whose `source` names `template_id` with no compendium pack. The
    /// `ClientMsg::MergePush` audience query — same-world and pack-less by design
    /// (compendium/cross-world push is out of scope), so a template id that
    /// resolves here is one the caller's world genuinely stamped from. Ordered by
    /// id for determinism.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let instances = repo.instances_of(uuid::Uuid::nil(), uuid::Uuid::nil()).await?;
    /// assert!(instances.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    async fn instances_of(
        &self,
        world_id: Uuid,
        template_id: Uuid,
    ) -> Result<Vec<Document>, DataError>;

    /// The world's committed commands with sequence strictly greater than
    /// `seq`, in order — the reconnect/resync replay source.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let replay = repo.events_since(uuid::Uuid::nil(), 0).await?;
    /// assert!(replay.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    /// Each row's StoredCommand back-compat-parses a bare-Command row (no `command`/`snapshot`
    /// keys) via StoredCommand::from_stored_json, carrying an all-None snapshot.
    async fn events_since(
        &self,
        world_id: Uuid,
        seq: i64,
    ) -> Result<Vec<crate::data::snapshot::StoredCommand>, DataError>;

    /// Fetch a world row by id, or `None` if it does not exist.
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
    async fn get_world(&self, id: Uuid) -> Result<Option<World>, DataError>;

    /// A user's role within `world`, or `None` if they are not a member.
    /// Lets a `dyn Repository` caller (e.g. `chat::handle_send_message`)
    /// validate candidate uuids — a whisper's recipients — actually belong to
    /// the world before trusting them.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// assert!(repo.member_role(uuid::Uuid::nil(), uuid::Uuid::nil()).await?.is_none());
    /// # Ok(())
    /// # }
    /// ```
    async fn member_role(&self, world: Uuid, user: Uuid) -> Result<Option<WorldRole>, DataError>;

    /// The UUID of a member of `world` whose username matches exactly, or
    /// `None`. Used to resolve a `/w @name` whisper target server-side.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let found = repo.member_id_by_username(uuid::Uuid::nil(), "no-such-member").await?;
    /// assert!(found.is_none());
    /// # Ok(())
    /// # }
    /// ```
    async fn member_id_by_username(
        &self,
        world: Uuid,
        username: &str,
    ) -> Result<Option<Uuid>, DataError>;

    /// The first asset in `world` whose `original_name` case-insensitively equals `name`, or
    /// `None`. Ties (two assets sharing a name) resolve to the earliest-created — an
    /// under-specified but stable pick; asset names are not enforced unique.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let found = repo.asset_id_by_name(uuid::Uuid::nil(), "no-such-asset").await?;
    /// assert!(found.is_none());
    /// # Ok(())
    /// # }
    /// ```
    async fn asset_id_by_name(&self, world: Uuid, name: &str) -> Result<Option<Uuid>, DataError>;

    /// A world's default capability grants (additive over the per-document
    /// `DocRole` floor). Empty when unset.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let defaults = repo.world_cap_defaults(uuid::Uuid::nil()).await?;
    /// assert!(defaults.all.by_role.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    async fn world_cap_defaults(&self, world: Uuid) -> Result<WorldCapDefaults, DataError>;

    /// A world's declarative capability requirements (additive over the
    /// structural base capability for each field path). Empty when unset.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let reqs = repo.world_cap_requirements(uuid::Uuid::nil()).await?;
    /// assert!(reqs.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    async fn world_cap_requirements(
        &self,
        world: Uuid,
    ) -> Result<Vec<CapabilityRequirement>, DataError>;

    /// A world's UI contract declarations (GM-published). Empty when unset.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let decls = repo.world_contract_declarations(uuid::Uuid::nil()).await?;
    /// assert!(decls.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    async fn world_contract_declarations(
        &self,
        world: Uuid,
    ) -> Result<Vec<ContractDeclaration>, DataError>;

    /// A world's declarative structural schema declarations (GM-committed on
    /// module enable). Empty when unset.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let decls = repo.world_schema_declarations(uuid::Uuid::nil()).await?;
    /// assert!(decls.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    async fn world_schema_declarations(
        &self,
        world: Uuid,
    ) -> Result<Vec<SchemaDeclaration>, DataError>;

    /// A world's enabled installed-module entries (GM-set), id + per-world
    /// `validators_enabled` flag. Empty when unset. A stored legacy bare-string-array
    /// setting reads back as every id with `validators_enabled: false`
    /// (`WorldModuleEntry::parse_legacy_tolerant`).
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let entries = repo.world_enabled_modules(uuid::Uuid::nil()).await?;
    /// assert!(entries.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    async fn world_enabled_modules(
        &self,
        world: Uuid,
    ) -> Result<Vec<crate::modules::WorldModuleEntry>, DataError>;

    /// Replace a world's enabled installed-module set (GM/admin-authorized by the caller — this
    /// trait method itself performs no authorization). Stored as JSON in `settings`, beside
    /// `world_cap_requirements`/`world_contract_declarations` — enable/disable never mutates
    /// either of those.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// use shadowcat::modules::WorldModuleEntry;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let world = repo.create_world("MOCK_WORLD", 0).await?;
    /// let entries = vec![WorldModuleEntry {
    ///     id: "mock-module".into(),
    ///     validators_enabled: false,
    /// }];
    /// repo.set_world_enabled_modules(world.id, &entries).await?;
    /// assert_eq!(repo.world_enabled_modules(world.id).await?, entries);
    /// # Ok(())
    /// # }
    /// ```
    async fn set_world_enabled_modules(
        &self,
        world: Uuid,
        entries: &[crate::modules::WorldModuleEntry],
    ) -> Result<(), DataError>;

    /// A world's member list as `(user_id, username, role)` triples, ordered by
    /// username (case-insensitive) — the membership read `world_seed::seed_author`
    /// and the member-listing route share.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let members = repo.list_members(uuid::Uuid::nil()).await?;
    /// assert!(members.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    async fn list_members(&self, world: Uuid) -> Result<Vec<(Uuid, String, WorldRole)>, DataError>;

    /// Resets `module`'s consecutive sandbox-validator fault counter for `world` to zero —
    /// called by `Room::disable_faulting_validator_locked` once it finishes disabling a
    /// persistently faulting module, so a future re-enable starts the streak at zero. A cheap,
    /// synchronous, in-memory operation: a repository never wired to a `modules_dir` has no
    /// counter to reset, so this is a no-op for it.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// repo.reset_validator_fault_streak(uuid::Uuid::nil(), "mock-module").await;
    /// # Ok(())
    /// # }
    /// ```
    async fn reset_validator_fault_streak(&self, world: Uuid, module: &str);

    /// Full-text search over a world's documents, ranked by relevance and
    /// filtered to what `ctx` may read. `cursor` is the raw-rank offset from a
    /// prior page (`None` for the first). `doc_types` narrows the ranked
    /// candidates to the listed doc_types (empty = every type); a list over
    /// `data::search::MAX_SEARCH_DOC_TYPES` entries is refused. Returns up to
    /// `limit` readable hits.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::document::WorldRole;
    /// use shadowcat::data::membership::PermissionContext;
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let ctx = PermissionContext { user_id: uuid::Uuid::nil(), world_role: WorldRole::Gm };
    /// let page = repo.search(&ctx, uuid::Uuid::nil(), "dragon", 10, None, &[]).await?;
    /// assert!(page.hits.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    async fn search(
        &self,
        ctx: &crate::data::membership::PermissionContext,
        world_id: Uuid,
        query: &str,
        limit: u32,
        cursor: Option<i64>,
        doc_types: &[String],
    ) -> Result<crate::data::search::SearchPage, DataError>;

    /// The player's serialized explored-cell blob for a scene's LEVEL, or `None` when
    /// unexplored.
    /// Per-(scene, level, user) secret memory — never broadcast; used by the movement gate's
    /// `Revealed` mode to union the explored set with the live visibility mask. `level` is the
    /// level id (`""` = ground/a level-less scene) `scene::elevation::level_of` resolves.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// assert!(repo.get_explored(uuid::Uuid::nil(), "", uuid::Uuid::nil()).await?.is_none());
    /// # Ok(())
    /// # }
    /// ```
    async fn get_explored(
        &self,
        scene: Uuid,
        level: &str,
        user: Uuid,
    ) -> Result<Option<Vec<u8>>, DataError>;

    /// A persisted `link_preview_cache` row for `url`, or `None` if absent.
    /// The DB-backed tier BEHIND `chat::LinkPreviewCache`'s in-memory fast
    /// path — consulted on an in-memory miss so a cold-started process can
    /// reuse a still-fresh row rather than re-fetching every URL seen since
    /// the process last started (see `chat::link_preview::cached_or_fetch`).
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// assert!(repo.get_link_preview_cache("https://example.com").await?.is_none());
    /// # Ok(())
    /// # }
    /// ```
    async fn get_link_preview_cache(
        &self,
        url: &str,
    ) -> Result<Option<LinkPreviewCacheRow>, DataError>;

    /// Upserts `title`/`description`/`fetched_at` for `url`. Leaves any
    /// existing `image_asset_id` untouched on conflict — an already
    /// asset-ified image (set by `set_link_preview_cache_image`) must survive
    /// a later title/description refresh of the same URL.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// repo.upsert_link_preview_cache(
    ///     "https://example.com",
    ///     Some("MOCK_TITLE"),
    ///     None,
    ///     0,
    /// ).await?;
    /// let row = repo.get_link_preview_cache("https://example.com").await?.unwrap();
    /// assert_eq!(row.title.as_deref(), Some("MOCK_TITLE"));
    /// # Ok(())
    /// # }
    /// ```
    async fn upsert_link_preview_cache(
        &self,
        url: &str,
        title: Option<&str>,
        description: Option<&str>,
        fetched_at_ms: i64,
    ) -> Result<(), DataError>;

    /// Sets `image_asset_id` on an EXISTING `url` row (a no-op if the row is
    /// absent — an image is only ever attached to a URL whose
    /// title/description scrape, or the oEmbed thumbnail pipeline's own
    /// placeholder upsert, already created the row).
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::asset::{Asset, AssetMeta};
    /// use shadowcat::data::repository::Repository;
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
    async fn set_link_preview_cache_image(
        &self,
        url: &str,
        image_asset_id: Uuid,
    ) -> Result<(), DataError>;

    /// Fetch one asset row by id, or `None` if it does not exist. Unredacted:
    /// assets carry no per-recipient permission set of their own (mutation
    /// routes are GM-only; reads are member-visible with no finer-grained
    /// redaction), so a caller resolving a chat `[[asset:...]]` span must
    /// independently check `Asset.world_id` against the sending room's world.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// assert!(repo.get_asset(uuid::Uuid::nil()).await?.is_none());
    /// # Ok(())
    /// # }
    /// ```
    async fn get_asset(&self, id: Uuid) -> Result<Option<crate::data::asset::Asset>, DataError>;
}
