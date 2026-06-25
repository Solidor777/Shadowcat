use async_trait::async_trait;
use uuid::Uuid;

use crate::data::command::{Command, UnsequencedCommand};
use crate::data::document::{
    CapabilityRequirement, ContractDeclaration, Document, World, WorldCapDefaults,
};
use crate::data::DataError;

/// Storage contract. The only implementation in M2 is `SqliteRepository`;
/// the trait exists so Postgres can be added later behind the same surface.
#[async_trait]
pub trait Repository: Send + Sync {
    /// Allocate the next per-world seq, append the command to the log, and
    /// apply every operation to the document store — all in one transaction.
    async fn apply_command(&self, cmd: UnsequencedCommand) -> Result<Command, DataError>;

    /// Authorize (per `ctx`), structurally validate, and check per-op
    /// pre-images, then sequence + apply + log — all in one transaction.
    /// Field-level optimistic concurrency: an `Update` whose `FieldChange.old`
    /// does not match the current stored value yields `Conflict`. A failure in
    /// the authorize phase consumes no seq (the transaction rolls back whole).
    async fn apply_intent(
        &self,
        ctx: &crate::data::membership::PermissionContext,
        world_id: Uuid,
        ops: Vec<crate::data::command::Operation>,
        ts: i64,
    ) -> Result<Command, DataError>;

    async fn get_document(&self, id: Uuid) -> Result<Option<Document>, DataError>;

    async fn query_documents(
        &self,
        world_id: Uuid,
        doc_type: &str,
    ) -> Result<Vec<Document>, DataError>;

    /// All documents whose `parent_id` equals `parent` (a scene's direct
    /// children). Ordered by id for determinism.
    async fn query_children(&self, parent: Uuid) -> Result<Vec<Document>, DataError>;

    /// All scene-entity documents in `world` — scenes plus anything with a
    /// parent. Mirrors `scene::is_scene_entity` so initial ECS hydration and the
    /// live `apply_op` path share one definition of "scene entity".
    async fn query_scene_entities(&self, world: Uuid) -> Result<Vec<Document>, DataError>;

    async fn documents_by_source(
        &self,
        pack: Option<&str>,
        source_id: Uuid,
    ) -> Result<Vec<Document>, DataError>;

    async fn events_since(&self, world_id: Uuid, seq: i64) -> Result<Vec<Command>, DataError>;

    /// Fetch a world row by id, or `None` if it does not exist.
    async fn get_world(&self, id: Uuid) -> Result<Option<World>, DataError>;

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
