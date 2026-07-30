use async_trait::async_trait;
use uuid::Uuid;

use crate::data::command::{Command, UnsequencedCommand};
use crate::data::document::{
    CapabilityRequirement, ContractDeclaration, Document, SchemaDeclaration, World,
    WorldCapDefaults, WorldRole,
};
use crate::data::DataError;

/// Storage contract. The only implementation in M2 is `SqliteRepository`;
/// the trait exists so Postgres can be added later behind the same surface.
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
    /// with `DataError::BadEngine`.
    async fn apply_command(&self, cmd: UnsequencedCommand) -> Result<Command, DataError>;

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
    async fn apply_intent(
        &self,
        ctx: &crate::data::membership::PermissionContext,
        world_id: Uuid,
        ops: Vec<crate::data::command::Operation>,
        ts: i64,
        origin: crate::data::command::WriteOrigin,
    ) -> Result<Command, DataError>;

    /// Fetch one document by id, or `None` if it does not exist. Unredacted —
    /// callers gate egress via `resolve_access` + `filter_properties`.
    async fn get_document(&self, id: Uuid) -> Result<Option<Document>, DataError>;

    /// Resolve `doc`'s effective owner against LIVE actor state — the same
    /// `permission::effective_owner` rule the write path enforces, joining the
    /// linked actor with one pool read when `doc` is a linked token. For egress
    /// read routes and search; the ws broadcast hot path joins through the room's
    /// in-memory actor table instead (zero pool reads per recipient).
    async fn effective_owner_of(&self, doc: &Document) -> Result<Option<Uuid>, DataError>;

    /// All documents of one `doc_type` in `world_id` (unredacted; egress-gated
    /// by callers).
    async fn query_documents(
        &self,
        world_id: Uuid,
        doc_type: &str,
    ) -> Result<Vec<Document>, DataError>;

    /// All documents in `world` whose `doc_type` is any of `doc_types`, in one
    /// query. Equivalent to unioning `query_documents` per type, but halves DB
    /// round-trips when a caller needs several independent doc_type singletons
    /// at once (e.g. room cold-hydration's four config/actor doc_types).
    async fn query_documents_by_types(
        &self,
        world_id: Uuid,
        doc_types: &[&str],
    ) -> Result<Vec<Document>, DataError>;

    /// All documents whose `parent_id` equals `parent` (a scene's direct
    /// children). Ordered by id for determinism.
    async fn query_children(&self, parent: Uuid) -> Result<Vec<Document>, DataError>;

    /// All scene-entity documents in `world` — scenes plus anything with a
    /// parent. Mirrors `scene::is_scene_entity` so initial ECS hydration and the
    /// live `apply_op` path share one definition of "scene entity".
    async fn query_scene_entities(&self, world: Uuid) -> Result<Vec<Document>, DataError>;

    /// Instances stamped from a given source (`source.id` + optional pack) —
    /// the template push path's audience query.
    async fn documents_by_source(
        &self,
        pack: Option<&str>,
        source_id: Uuid,
    ) -> Result<Vec<Document>, DataError>;

    /// The world's committed commands with sequence strictly greater than
    /// `seq`, in order — the reconnect/resync replay source.
    async fn events_since(&self, world_id: Uuid, seq: i64) -> Result<Vec<Command>, DataError>;

    /// Fetch a world row by id, or `None` if it does not exist.
    async fn get_world(&self, id: Uuid) -> Result<Option<World>, DataError>;

    /// A user's role within `world`, or `None` if they are not a member.
    /// Lets a `dyn Repository` caller (e.g. `chat::handle_send_message`)
    /// validate candidate uuids — a whisper's recipients — actually belong to
    /// the world before trusting them.
    async fn member_role(&self, world: Uuid, user: Uuid) -> Result<Option<WorldRole>, DataError>;

    /// The UUID of a member of `world` whose username matches exactly, or
    /// `None`. Used to resolve a `/w @name` whisper target server-side.
    async fn member_id_by_username(
        &self,
        world: Uuid,
        username: &str,
    ) -> Result<Option<Uuid>, DataError>;

    /// A world's default capability grants (additive over the per-document
    /// `DocRole` floor). Empty when unset.
    async fn world_cap_defaults(&self, world: Uuid) -> Result<WorldCapDefaults, DataError>;

    /// A world's declarative capability requirements (additive over the
    /// structural base capability for each field path). Empty when unset.
    async fn world_cap_requirements(
        &self,
        world: Uuid,
    ) -> Result<Vec<CapabilityRequirement>, DataError>;

    /// A world's UI contract declarations (GM-published). Empty when unset.
    async fn world_contract_declarations(
        &self,
        world: Uuid,
    ) -> Result<Vec<ContractDeclaration>, DataError>;

    /// A world's declarative structural schema declarations (GM-committed on
    /// module enable). Empty when unset.
    async fn world_schema_declarations(
        &self,
        world: Uuid,
    ) -> Result<Vec<SchemaDeclaration>, DataError>;

    /// A world's enabled installed-module ids (GM-set). Empty when unset.
    async fn world_enabled_modules(&self, world: Uuid) -> Result<Vec<String>, DataError>;

    /// Full-text search over a world's documents, ranked by relevance and
    /// filtered to what `ctx` may read. `cursor` is the raw-rank offset from a
    /// prior page (`None` for the first). Returns up to `limit` readable hits.
    async fn search(
        &self,
        ctx: &crate::data::membership::PermissionContext,
        world_id: Uuid,
        query: &str,
        limit: u32,
        cursor: Option<i64>,
    ) -> Result<crate::data::search::SearchPage, DataError>;

    /// The player's serialized explored-cell blob for a scene, or `None` when unexplored.
    /// Per-(scene, user) secret memory — never broadcast; used by the M10e-4 movement gate
    /// (`Revealed` mode) to union the explored set with the live visibility mask.
    async fn get_explored(&self, scene: Uuid, user: Uuid) -> Result<Option<Vec<u8>>, DataError>;
}
