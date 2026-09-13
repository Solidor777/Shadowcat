//! `validate_bounds` cap-by-cap truth table, and `vfx_permitted` over a stub
//! `Repository` (canned doc + defaults, every other method unreachable) — the
//! spectator short-circuit is pinned by a `get_document` call count.

use super::*;
use crate::data::command::{Operation, UnsequencedCommand, WriteOrigin};
use crate::data::document::{
    CapabilityGrants, CapabilityRequirement, ContractDeclaration, DocRole, Document, PermissionSet,
    SchemaDeclaration, Scope, World, WorldCapDefaults, WorldRole,
};
use crate::data::membership::PermissionContext;
use crate::data::repository::{LinkPreviewCacheRow, Repository};
use crate::data::snapshot::StoredCommand;
use crate::data::DataError;
use std::sync::atomic::{AtomicUsize, Ordering};

fn req() -> VfxRequest {
    VfxRequest {
        scene: Uuid::new_v4(),
        asset: "a".into(),
        x: 0.0,
        y: 0.0,
        scale: None,
        rotation: None,
        duration_ms: None,
        sound: None,
        elevation: None,
    }
}

#[test]
fn validate_bounds_accepts_a_plain_request() {
    assert!(validate_bounds(&req()));
}

#[test]
fn validate_bounds_refuses_non_finite_and_out_of_bound_coordinates() {
    let bound = crate::scene::move_exec::MAX_GATE_WALK_COORD;
    for (x, y) in [
        (f64::NAN, 0.0),
        (f64::INFINITY, 0.0),
        (0.0, f64::NEG_INFINITY),
        (bound + 1.0, 0.0),
        (0.0, -(bound + 1.0)),
    ] {
        let r = VfxRequest { x, y, ..req() };
        assert!(!validate_bounds(&r), "({x}, {y})");
    }
    let at = VfxRequest {
        x: bound,
        y: -bound,
        ..req()
    };
    assert!(validate_bounds(&at), "the bound itself is admissible");
}

#[test]
fn validate_bounds_scale_must_be_finite_and_in_the_open_closed_band() {
    // `8.0 + f64::EPSILON` is not one of the refused values: f64's ulp at 8.0 is
    // ~1.78e-15, so that sum rounds back to exactly 8.0.
    for s in [0.0, -1.0, 8.1, 9.0, f64::NAN, f64::INFINITY] {
        let r = VfxRequest {
            scale: Some(s),
            ..req()
        };
        assert!(!validate_bounds(&r), "scale {s}");
    }
    for s in [f64::MIN_POSITIVE, 1.0, 8.0] {
        let r = VfxRequest {
            scale: Some(s),
            ..req()
        };
        assert!(validate_bounds(&r), "scale {s}");
    }
}

#[test]
fn validate_bounds_duration_cap_is_inclusive() {
    let ok = VfxRequest {
        duration_ms: Some(60_000),
        ..req()
    };
    assert!(validate_bounds(&ok));
    let over = VfxRequest {
        duration_ms: Some(60_001),
        ..req()
    };
    assert!(!validate_bounds(&over));
}

#[test]
fn validate_bounds_id_lengths() {
    let empty = VfxRequest {
        asset: String::new(),
        ..req()
    };
    assert!(!validate_bounds(&empty));
    let at = VfxRequest {
        asset: "x".repeat(MAX_ID_BYTES),
        sound: Some("y".repeat(MAX_ID_BYTES)),
        ..req()
    };
    assert!(validate_bounds(&at));
    let over_asset = VfxRequest {
        asset: "x".repeat(MAX_ID_BYTES + 1),
        ..req()
    };
    assert!(!validate_bounds(&over_asset));
    let over_sound = VfxRequest {
        sound: Some("y".repeat(MAX_ID_BYTES + 1)),
        ..req()
    };
    assert!(!validate_bounds(&over_sound));
}

/// A `Repository` stub answering ONLY `get_document` (canned, call-counted) and
/// `world_cap_defaults` (canned); every other method is unreachable in these
/// tests and panics.
struct StubRepo {
    /// The document `get_document` returns (any id).
    doc: Option<Document>,
    /// The defaults `world_cap_defaults` returns.
    defaults: WorldCapDefaults,
    /// `get_document` call count — pins the spectator short-circuit.
    get_document_calls: AtomicUsize,
}

impl StubRepo {
    /// A stub returning `doc` from every `get_document`.
    fn with_doc(doc: Option<Document>) -> Self {
        StubRepo {
            doc,
            defaults: WorldCapDefaults::default(),
            get_document_calls: AtomicUsize::new(0),
        }
    }
}

/// A `scene` document in `world` with the given default role floor.
fn scene_doc(world: Uuid, default: DocRole) -> Document {
    Document {
        id: Uuid::new_v4(),
        scope: Scope::World { world_id: world },
        doc_type: "scene".into(),
        schema_version: 1,
        name: Some("Scene".into()),
        source: None,
        base: None,
        owner: None,
        permissions: PermissionSet {
            default,
            users: Default::default(),
            property_overrides: Default::default(),
            capabilities: CapabilityGrants::default(),
            gm_role: None,
        },
        embedded: Default::default(),
        parent_id: None,
        engine: None,
        system: serde_json::json!({}),
        created_at: 0,
        updated_at: 0,
    }
}

#[async_trait::async_trait]
impl Repository for StubRepo {
    async fn apply_command(&self, _cmd: UnsequencedCommand) -> Result<StoredCommand, DataError> {
        unimplemented!()
    }
    async fn apply_intent(
        &self,
        _ctx: &PermissionContext,
        _world_id: Uuid,
        _ops: Vec<Operation>,
        _ts: i64,
        _origin: WriteOrigin,
    ) -> Result<StoredCommand, DataError> {
        unimplemented!()
    }
    async fn get_document(&self, _id: Uuid) -> Result<Option<Document>, DataError> {
        self.get_document_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.doc.clone())
    }
    async fn get_document_with_created_seq(
        &self,
        _id: Uuid,
    ) -> Result<Option<(Document, i64)>, DataError> {
        unimplemented!()
    }
    async fn effective_owner_of(&self, _doc: &Document) -> Result<Option<Uuid>, DataError> {
        unimplemented!()
    }
    async fn query_documents(
        &self,
        _world_id: Uuid,
        _doc_type: &str,
    ) -> Result<Vec<Document>, DataError> {
        unimplemented!()
    }
    async fn query_documents_by_types(
        &self,
        _world_id: Uuid,
        _doc_types: &[&str],
    ) -> Result<Vec<Document>, DataError> {
        unimplemented!()
    }
    async fn query_all_documents(&self, _world_id: Uuid) -> Result<Vec<Document>, DataError> {
        unimplemented!()
    }
    async fn query_children(&self, _parent: Uuid) -> Result<Vec<Document>, DataError> {
        unimplemented!()
    }
    async fn query_scene_entities(&self, _world: Uuid) -> Result<Vec<Document>, DataError> {
        unimplemented!()
    }
    async fn documents_by_source(
        &self,
        _pack: Option<&str>,
        _source_id: Uuid,
    ) -> Result<Vec<Document>, DataError> {
        unimplemented!()
    }
    async fn instances_of(
        &self,
        _world_id: Uuid,
        _template_id: Uuid,
    ) -> Result<Vec<Document>, DataError> {
        unimplemented!()
    }
    async fn events_since(
        &self,
        _world_id: Uuid,
        _seq: i64,
    ) -> Result<Vec<StoredCommand>, DataError> {
        unimplemented!()
    }
    async fn get_world(&self, _id: Uuid) -> Result<Option<World>, DataError> {
        unimplemented!()
    }
    async fn member_role(&self, _world: Uuid, _user: Uuid) -> Result<Option<WorldRole>, DataError> {
        unimplemented!()
    }
    async fn member_id_by_username(
        &self,
        _world: Uuid,
        _username: &str,
    ) -> Result<Option<Uuid>, DataError> {
        unimplemented!()
    }
    async fn world_cap_defaults(&self, _world: Uuid) -> Result<WorldCapDefaults, DataError> {
        Ok(self.defaults.clone())
    }
    async fn world_cap_requirements(
        &self,
        _world: Uuid,
    ) -> Result<Vec<CapabilityRequirement>, DataError> {
        unimplemented!()
    }
    async fn world_contract_declarations(
        &self,
        _world: Uuid,
    ) -> Result<Vec<ContractDeclaration>, DataError> {
        unimplemented!()
    }
    async fn world_schema_declarations(
        &self,
        _world: Uuid,
    ) -> Result<Vec<SchemaDeclaration>, DataError> {
        unimplemented!()
    }
    async fn world_enabled_modules(&self, _world: Uuid) -> Result<Vec<String>, DataError> {
        unimplemented!()
    }
    async fn search(
        &self,
        _ctx: &PermissionContext,
        _world_id: Uuid,
        _query: &str,
        _limit: u32,
        _cursor: Option<i64>,
        _doc_types: &[String],
    ) -> Result<crate::data::search::SearchPage, DataError> {
        unimplemented!()
    }
    async fn get_explored(&self, _scene: Uuid, _user: Uuid) -> Result<Option<Vec<u8>>, DataError> {
        unimplemented!()
    }
    async fn get_link_preview_cache(
        &self,
        _url: &str,
    ) -> Result<Option<LinkPreviewCacheRow>, DataError> {
        unimplemented!()
    }
    async fn upsert_link_preview_cache(
        &self,
        _url: &str,
        _title: Option<&str>,
        _description: Option<&str>,
        _fetched_at_ms: i64,
    ) -> Result<(), DataError> {
        unimplemented!()
    }
    async fn set_link_preview_cache_image(
        &self,
        _url: &str,
        _image_asset_id: Uuid,
    ) -> Result<(), DataError> {
        unimplemented!()
    }
    async fn get_asset(&self, _id: Uuid) -> Result<Option<crate::data::asset::Asset>, DataError> {
        unimplemented!()
    }
}

fn ctx(role: WorldRole) -> PermissionContext {
    PermissionContext {
        user_id: Uuid::new_v4(),
        world_role: role,
    }
}

#[tokio::test]
async fn spectator_is_refused_before_any_repository_call() {
    let world = Uuid::new_v4();
    let repo = StubRepo::with_doc(Some(scene_doc(world, DocRole::Observer)));
    assert!(!vfx_permitted(Uuid::new_v4(), &ctx(WorldRole::Spectator), world, &repo).await);
    assert_eq!(repo.get_document_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn gm_and_player_with_read_are_permitted() {
    let world = Uuid::new_v4();
    let doc = scene_doc(world, DocRole::Observer);
    let scene = doc.id;
    let repo = StubRepo::with_doc(Some(doc));
    assert!(vfx_permitted(scene, &ctx(WorldRole::Gm), world, &repo).await);
    assert!(vfx_permitted(scene, &ctx(WorldRole::Player), world, &repo).await);
}

#[tokio::test]
async fn player_without_read_is_refused() {
    let world = Uuid::new_v4();
    let doc = scene_doc(world, DocRole::None);
    let scene = doc.id;
    let repo = StubRepo::with_doc(Some(doc));
    assert!(!vfx_permitted(scene, &ctx(WorldRole::Player), world, &repo).await);
    // The GM's unconditional access is unaffected by the role floor.
    assert!(vfx_permitted(scene, &ctx(WorldRole::Gm), world, &repo).await);
}

#[tokio::test]
async fn a_non_scene_doc_and_a_foreign_world_scene_are_refused() {
    let world = Uuid::new_v4();
    let mut not_scene = scene_doc(world, DocRole::Observer);
    not_scene.doc_type = "note".into();
    let repo = StubRepo::with_doc(Some(not_scene.clone()));
    assert!(!vfx_permitted(not_scene.id, &ctx(WorldRole::Gm), world, &repo).await);

    let foreign = scene_doc(Uuid::new_v4(), DocRole::Observer);
    let repo = StubRepo::with_doc(Some(foreign.clone()));
    assert!(!vfx_permitted(foreign.id, &ctx(WorldRole::Gm), world, &repo).await);

    // A missing doc fails closed too.
    let repo = StubRepo::with_doc(None);
    assert!(!vfx_permitted(Uuid::new_v4(), &ctx(WorldRole::Gm), world, &repo).await);
}
