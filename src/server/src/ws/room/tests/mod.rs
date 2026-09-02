use super::*;
use crate::auth::role::ServerRole;
use crate::data::document::{WorldCapDefaults, WorldRole};
use crate::data::membership::PermissionContext;
use crate::data::sqlite::SqliteRepository;
use std::sync::atomic::Ordering;
use uuid::Uuid;

// Dual-write fixture helpers (`ws_engine`/`token_engine`) live in `ws::test_support`,
// shared with `ws::conn`'s test module.
use crate::ws::test_support::{token_engine, ws_engine};

async fn repo_with_world() -> (SqliteRepository, Uuid, PermissionContext) {
    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let author = repo
        .create_user("a", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", author, 0).await.unwrap();
    let ctx = PermissionContext {
        user_id: author,
        world_role: WorldRole::Gm,
    };
    (repo, world.id, ctx)
}

#[tokio::test]
async fn begin_delete_tombstones_and_removes() {
    let (repo, world_id, _ctx) = repo_with_world().await;
    let reg = RoomRegistry::new();
    reg.get_or_create(&repo, world_id).await.unwrap().unwrap();

    let room = reg.begin_delete(world_id);
    assert!(room.is_some(), "live room returned for eviction broadcast");
    // The world row still exists — the refusal below is the tombstone's.
    assert!(reg.get_or_create(&repo, world_id).await.unwrap().is_none());

    reg.finish_delete(world_id);
    assert!(reg.get_or_create(&repo, world_id).await.unwrap().is_some());
}

/// Delegating repo whose `query_documents_by_types` — the LAST hydration
/// read in `get_or_create` — performs a COMPLETE world deletion
/// (begin_delete → delete_world → finish_delete) on its first call: a
/// delete that starts and finishes entirely inside the hydration window,
/// after the caller's tombstone check and `get_world` read but before its
/// registry insert. At re-check time the tombstone is already lifted, so
/// only a world-existence re-verify can refuse the ghost room.
struct DeleteMidHydration<'a> {
    inner: &'a SqliteRepository,
    registry: &'a RoomRegistry,
    world: Uuid,
    fired: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl Repository for DeleteMidHydration<'_> {
    async fn apply_command(
        &self,
        cmd: crate::data::command::UnsequencedCommand,
    ) -> Result<crate::data::snapshot::StoredCommand, DataError> {
        self.inner.apply_command(cmd).await
    }
    async fn apply_intent(
        &self,
        ctx: &crate::data::membership::PermissionContext,
        world_id: Uuid,
        ops: Vec<Operation>,
        ts: i64,
        origin: WriteOrigin,
    ) -> Result<crate::data::snapshot::StoredCommand, DataError> {
        self.inner
            .apply_intent(ctx, world_id, ops, ts, origin)
            .await
    }
    async fn get_document(&self, id: Uuid) -> Result<Option<Document>, DataError> {
        self.inner.get_document(id).await
    }
    async fn get_document_with_created_seq(
        &self,
        id: Uuid,
    ) -> Result<Option<(Document, i64)>, DataError> {
        self.inner.get_document_with_created_seq(id).await
    }
    async fn effective_owner_of(&self, doc: &Document) -> Result<Option<Uuid>, DataError> {
        self.inner.effective_owner_of(doc).await
    }
    async fn query_documents(
        &self,
        world_id: Uuid,
        doc_type: &str,
    ) -> Result<Vec<Document>, DataError> {
        self.inner.query_documents(world_id, doc_type).await
    }
    async fn query_documents_by_types(
        &self,
        world_id: Uuid,
        doc_types: &[&str],
    ) -> Result<Vec<Document>, DataError> {
        if !self.fired.swap(true, Ordering::SeqCst) {
            self.registry.begin_delete(self.world);
            self.inner.delete_world(self.world).await?;
            self.registry.finish_delete(self.world);
        }
        self.inner
            .query_documents_by_types(world_id, doc_types)
            .await
    }
    async fn query_all_documents(&self, world_id: Uuid) -> Result<Vec<Document>, DataError> {
        self.inner.query_all_documents(world_id).await
    }
    async fn query_children(&self, parent: Uuid) -> Result<Vec<Document>, DataError> {
        self.inner.query_children(parent).await
    }
    async fn query_scene_entities(&self, world: Uuid) -> Result<Vec<Document>, DataError> {
        self.inner.query_scene_entities(world).await
    }
    async fn documents_by_source(
        &self,
        pack: Option<&str>,
        source_id: Uuid,
    ) -> Result<Vec<Document>, DataError> {
        self.inner.documents_by_source(pack, source_id).await
    }
    async fn events_since(
        &self,
        world_id: Uuid,
        seq: i64,
    ) -> Result<Vec<crate::data::snapshot::StoredCommand>, DataError> {
        self.inner.events_since(world_id, seq).await
    }
    async fn get_world(&self, id: Uuid) -> Result<Option<crate::data::document::World>, DataError> {
        self.inner.get_world(id).await
    }
    async fn member_role(&self, world: Uuid, user: Uuid) -> Result<Option<WorldRole>, DataError> {
        self.inner.member_role(world, user).await
    }
    async fn member_id_by_username(
        &self,
        world: Uuid,
        username: &str,
    ) -> Result<Option<Uuid>, DataError> {
        self.inner.member_id_by_username(world, username).await
    }
    async fn world_cap_defaults(
        &self,
        world: Uuid,
    ) -> Result<crate::data::document::WorldCapDefaults, DataError> {
        self.inner.world_cap_defaults(world).await
    }
    async fn world_cap_requirements(
        &self,
        world: Uuid,
    ) -> Result<Vec<crate::data::document::CapabilityRequirement>, DataError> {
        self.inner.world_cap_requirements(world).await
    }
    async fn world_contract_declarations(
        &self,
        world: Uuid,
    ) -> Result<Vec<crate::data::document::ContractDeclaration>, DataError> {
        self.inner.world_contract_declarations(world).await
    }
    async fn world_schema_declarations(
        &self,
        world: Uuid,
    ) -> Result<Vec<crate::data::document::SchemaDeclaration>, DataError> {
        self.inner.world_schema_declarations(world).await
    }
    async fn world_enabled_modules(&self, world: Uuid) -> Result<Vec<String>, DataError> {
        self.inner.world_enabled_modules(world).await
    }
    async fn search(
        &self,
        ctx: &crate::data::membership::PermissionContext,
        world_id: Uuid,
        query: &str,
        limit: u32,
        cursor: Option<i64>,
    ) -> Result<crate::data::search::SearchPage, DataError> {
        self.inner.search(ctx, world_id, query, limit, cursor).await
    }
    async fn get_explored(&self, scene: Uuid, user: Uuid) -> Result<Option<Vec<u8>>, DataError> {
        self.inner.get_explored(scene, user).await
    }
    async fn get_link_preview_cache(
        &self,
        url: &str,
    ) -> Result<Option<crate::data::repository::LinkPreviewCacheRow>, DataError> {
        self.inner.get_link_preview_cache(url).await
    }
    async fn upsert_link_preview_cache(
        &self,
        url: &str,
        title: Option<&str>,
        description: Option<&str>,
        fetched_at_ms: i64,
    ) -> Result<(), DataError> {
        self.inner
            .upsert_link_preview_cache(url, title, description, fetched_at_ms)
            .await
    }
    async fn set_link_preview_cache_image(
        &self,
        url: &str,
        image_asset_id: Uuid,
    ) -> Result<(), DataError> {
        self.inner
            .set_link_preview_cache_image(url, image_asset_id)
            .await
    }
}

#[tokio::test]
async fn get_or_create_refuses_when_delete_completes_mid_hydration() {
    let (repo, world_id, _ctx) = repo_with_world().await;
    let reg = RoomRegistry::new();
    let wrapper = DeleteMidHydration {
        inner: &repo,
        registry: &reg,
        world: world_id,
        fired: std::sync::atomic::AtomicBool::new(false),
    };
    // The deletion completes (tombstone lifted, row gone) while hydration
    // is still in flight; the lifted flag alone proves nothing at re-check
    // time — only the world row's absence can.
    let room = reg.get_or_create(&wrapper, world_id).await.unwrap();
    assert!(room.is_none(), "ghost room registered for a deleted world");
    assert!(reg.get(world_id).is_none());
}

#[tokio::test]
async fn evict_user_reaches_every_room() {
    let (repo, w1, _ctx) = repo_with_world().await;
    let author2 = repo
        .create_user("b", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w2 = repo.create_world_owned("W2", author2, 0).await.unwrap().id;

    let reg = RoomRegistry::new();
    let r1 = reg.get_or_create(&repo, w1).await.unwrap().unwrap();
    let r2 = reg.get_or_create(&repo, w2).await.unwrap().unwrap();
    let (mut rx1, _) = r1.subscribe();
    let (mut rx2, _) = r2.subscribe();

    let target = Uuid::new_v4();
    reg.evict_user(target);

    for rx in [&mut rx1, &mut rx2] {
        match rx.recv().await.unwrap() {
            RoomEvent::Other(msg) => match msg.as_ref() {
                ServerMsg::Evicted { user } => assert_eq!(*user, Some(target)),
                other => panic!("expected Evicted, got {other:?}"),
            },
            RoomEvent::Event(_) => panic!("expected a non-Event broadcast (Evicted)"),
        }
    }
}

#[tokio::test]
async fn publish_hydrates_scene_ecs() {
    let (repo, world_id, ctx) = repo_with_world().await;
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
    assert_eq!(room.scene().read().await.entity_count(), 0);

    // Publish a scene doc (a scene entity by doc_type, no parent FK needed).
    let mut scene =
        crate::data::document::tests::world_scoped_doc(world_id, Uuid::from_u128(20), "scene");
    scene.owner = Some(ctx.user_id);
    room.publish(
        &repo,
        &ctx,
        vec![Operation::Create { doc: scene }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    assert_eq!(room.scene().read().await.entity_count(), 1);
}

/// A `/system/x` write on a token is game-system data — it must not be treated as a move
/// by `Room::publish`'s movement gate (which reads `/engine` exclusively), and the
/// write must not desync the ECS's committed `/engine` position. This is the integration-
/// level counterpart of `scene::mod::tests::token_move_uses_post_image_resisting_forged_
/// bypasses`'s `/system/x` decoy assertion (same naming-collision decoy: `/system/x` vs.
/// `/engine/x`), proved end-to-end through `Room::publish` rather than the bare ECS method.
#[tokio::test]
async fn system_field_write_bypasses_the_move_gate_and_does_not_desync_the_engine_band() {
    let h = movement_scene_with_wall().await;

    // A `/system/x` + `/system/y` decoy pair targeting `(200,150)`. If the gate mistakenly
    // read these `/system/*` paths as `/engine/x,y` (the naming-collision decoy this test
    // targets), the resulting straight-line move from the committed start `(50,50)` to
    // `(200,150)` crosses `movement_scene_with_wall`'s horizontal wall (y=100, x∈[100,200])
    // at x=125 — well clear of both wall endpoints, no corner-touch ambiguity — and would be
    // rejected. The gate must not even see this write, since it targets `/system`, not
    // `/engine`.
    let write = Operation::Update {
        doc_id: h.token_id,
        changes: vec![
            FieldChange {
                remove: false,
                path: "/system/x".into(),
                old: serde_json::Value::Null, // absent key reads as Null (no `system.x` default)
                new: serde_json::json!(200.0),
            },
            FieldChange {
                remove: false,
                path: "/system/y".into(),
                old: serde_json::Value::Null,
                new: serde_json::json!(150.0),
            },
        ],
    };
    h.room
        .publish(
            &h.repo,
            &h.player,
            vec![write],
            now_millis(),
            WriteOrigin::Client,
        )
        .await
        .expect("a /system write must not be rejected by the movement gate");

    // The engine-band position is untouched by the /system write.
    let pos = h.committed_pos(h.token_id).await;
    assert_eq!(
        pos, h.start,
        "/system write must not move the token's /engine position"
    );
}

/// Defense-in-depth: a single `Update`'s FieldChange list combining a wholesale `/engine`
/// replace AND a leaf `/engine/x` change must produce the SAME post-image whether the
/// refusal predicate's replay (`SceneEcs::token_move`) or the commit path's replay
/// (`apply_intent`'s sequential `command::apply_field_change` application) computes it —
/// in BOTH possible orderings of the two changes. Both replay implementations apply
/// `changes` via `command::apply_field_change` in array order independently; this pins them
/// against silently diverging (which would let the predicate judge one post-image while a
/// different one actually lands). Actor is the GM: this is a real position CHANGE, which the
/// movement gate refuses for a non-GM outright — the property under test (replay agreement)
/// is orthogonal to who is writing, and the GM path exercises the identical
/// `token_move`/commit replay.
#[tokio::test]
async fn mixed_wholesale_and_leaf_engine_changes_agree_between_gate_and_commit_in_both_orderings() {
    use crate::data::command::FieldChange;
    use serde_json::json;

    // The wholesale `old` pre-image must equal the ACTUAL stored `/engine` value, which
    // includes `TokenEngine`'s `#[serde(default)]` `null` fields (visual/actor_id/
    // overrides/face) beyond the `token_engine(50.0, 50.0)` fixture's x/y/w/h/rotation —
    // read it back rather than hand-constructing it, mirroring `mv_to`'s convention.
    async fn stored_engine(h: &MovementHandle) -> serde_json::Value {
        h.repo
            .get_document(h.token_id)
            .await
            .unwrap()
            .unwrap()
            .engine
            .unwrap()
    }
    let wholesale_new = json!({
        "x": 10.0, "y": 10.0, "w": 1.0, "h": 1.0, "rotation": 0.0,
        "visual": null, "actor_id": null, "overrides": null, "face": null
    });

    // Ordering A: wholesale replace, then a leaf x-overwrite. Expected final: (20,10).
    {
        let h = movement_scene_with_wall().await;
        let start_engine = stored_engine(&h).await;
        let changes = vec![
            FieldChange {
                remove: false,
                path: "/engine".into(),
                old: start_engine.clone(),
                new: wholesale_new.clone(),
            },
            FieldChange {
                remove: false,
                path: "/engine/x".into(),
                old: json!(50.0),
                new: json!(20.0),
            },
        ];
        let gate_post = {
            let scene = h.room.scene().read().await;
            scene.token_move(h.token_id, &changes).unwrap().2
        };
        h.room
            .publish(
                &h.repo,
                &h.gm,
                vec![Operation::Update {
                    doc_id: h.token_id,
                    changes,
                }],
                now_millis(),
                WriteOrigin::Client,
            )
            .await
            .unwrap();
        let committed = h.committed_pos(h.token_id).await;
        assert_eq!(gate_post, (20.0, 10.0), "ordering A gate post-image");
        assert_eq!(committed, (20.0, 10.0), "ordering A committed post-image");
        assert_eq!(
            gate_post, committed,
            "ordering A: gate and commit post-images must agree"
        );
    }

    // Ordering B: leaf x-overwrite, then wholesale replace. Expected final: (10,10).
    {
        let h = movement_scene_with_wall().await;
        let start_engine = stored_engine(&h).await;
        let changes = vec![
            FieldChange {
                remove: false,
                path: "/engine/x".into(),
                old: json!(50.0),
                new: json!(20.0),
            },
            FieldChange {
                remove: false,
                path: "/engine".into(),
                old: start_engine.clone(),
                new: wholesale_new.clone(),
            },
        ];
        let gate_post = {
            let scene = h.room.scene().read().await;
            scene.token_move(h.token_id, &changes).unwrap().2
        };
        h.room
            .publish(
                &h.repo,
                &h.gm,
                vec![Operation::Update {
                    doc_id: h.token_id,
                    changes,
                }],
                now_millis(),
                WriteOrigin::Client,
            )
            .await
            .unwrap();
        let committed = h.committed_pos(h.token_id).await;
        assert_eq!(gate_post, (10.0, 10.0), "ordering B gate post-image");
        assert_eq!(committed, (10.0, 10.0), "ordering B committed post-image");
        assert_eq!(
            gate_post, committed,
            "ordering B: gate and commit post-images must agree"
        );
    }
}

// -----------------------------------------------------------------------
// Non-GM position writes refused; Create placement gated on the same mask
// -----------------------------------------------------------------------

/// Shared fixture handle for the non-GM-refusal/Create-gate tests: a room with a scene, an
/// optional token, and both a GM and a player `PermissionContext`.
struct PlaceHandle {
    room: Arc<Room>,
    repo: SqliteRepository,
    gm_ctx: PermissionContext,
    player_ctx: PermissionContext,
    token: Uuid,
    world: Uuid,
    scene: Uuid,
}

/// A scene with a player-owned token at (50,50). No movement-restriction machinery: the
/// movement gate refuses a non-GM position CHANGE unconditionally, so these fixtures need
/// no lighting.
async fn room_with_player_owned_token() -> PlaceHandle {
    use crate::data::document::DocRole;

    let (repo, world_id, gm_ctx) = repo_with_world().await;
    let p = repo
        .create_user("player", None, ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world_id, p, WorldRole::Player)
        .await
        .unwrap();
    let player_ctx = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };

    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let scene_id = Uuid::from_u128(0xD901);
    let token_id = Uuid::from_u128(0xD902);

    let mut scene = wdoc(world_id, scene_id, "scene");
    scene.owner = Some(gm_ctx.user_id);
    room.publish(
        &repo,
        &gm_ctx,
        vec![Operation::Create { doc: scene }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut token = wdoc(world_id, token_id, "token");
    token.parent_id = Some(scene_id);
    token.owner = Some(p);
    token.permissions.users.insert(p, DocRole::Owner);
    token.engine = Some(token_engine(50.0, 50.0));
    room.publish(
        &repo,
        &gm_ctx,
        vec![Operation::Create { doc: token }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    PlaceHandle {
        room,
        repo,
        gm_ctx,
        player_ctx,
        token: token_id,
        world: world_id,
        scene: scene_id,
    }
}

/// A scene with a `blocksMove` wall crossing the GM's test move (50,50)->(250,50), and a
/// token at (50,50). Demonstrates the "GM ignores walls" exemption narratively — the
/// non-GM-refusal never reaches a GM regardless, so no gate actually consults this wall.
async fn room_with_gm_and_blocking_wall() -> PlaceHandle {
    use serde_json::json;

    let (repo, world_id, gm_ctx) = repo_with_world().await;
    let p = repo
        .create_user("player", None, ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world_id, p, WorldRole::Player)
        .await
        .unwrap();
    let player_ctx = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };

    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let scene_id = Uuid::from_u128(0xD910);
    let token_id = Uuid::from_u128(0xD911);
    let wall_id = Uuid::from_u128(0xD912);

    let mut scene = wdoc(world_id, scene_id, "scene");
    scene.owner = Some(gm_ctx.user_id);
    room.publish(
        &repo,
        &gm_ctx,
        vec![Operation::Create { doc: scene }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut token = wdoc(world_id, token_id, "token");
    token.parent_id = Some(scene_id);
    token.owner = Some(gm_ctx.user_id);
    token.engine = Some(token_engine(50.0, 50.0));
    room.publish(
        &repo,
        &gm_ctx,
        vec![Operation::Create { doc: token }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // Vertical wall at x=150 spanning y∈[-50,50]: the horizontal step (50,50)->(250,50)
    // crosses it.
    let mut wall = wdoc(world_id, wall_id, "wall");
    wall.parent_id = Some(scene_id);
    wall.owner = Some(gm_ctx.user_id);
    wall.engine = Some(
        json!({ "seg": { "x1": 150.0, "y1": -50.0, "x2": 150.0, "y2": 50.0 }, "blocksMove": true }),
    );
    room.publish(
        &repo,
        &gm_ctx,
        vec![Operation::Create { doc: wall }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    PlaceHandle {
        room,
        repo,
        gm_ctx,
        player_ctx,
        token: token_id,
        world: world_id,
        scene: scene_id,
    }
}

/// A scene lit only near (50,50) (movementRestriction="visible"), with `core:create` granted
/// to Player so a player-authored `Create` reaches the placement gate at all. Asserts its own
/// mask is non-empty so the two Create tests built on it fail for the right reason.
async fn room_with_player_create_capability_and_lit_corner() -> PlaceHandle {
    use serde_json::json;

    let (repo, world_id, gm_ctx) = repo_with_world().await;
    let p = repo
        .create_user("player", None, ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world_id, p, WorldRole::Player)
        .await
        .unwrap();
    let player_ctx = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };

    let mut caps = crate::data::document::WorldCapDefaults::default();
    caps.role_caps
        .all
        .entry(WorldRole::Player)
        .or_default()
        .insert("core:create".into());
    repo.set_world_cap_defaults(world_id, &caps).await.unwrap();

    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let scene_id = Uuid::from_u128(0xD920);
    let ws_id = Uuid::from_u128(0xD921);
    let light_id = Uuid::from_u128(0xD922);
    let vision_token_id = Uuid::from_u128(0xD923);

    let mut ws = wdoc(world_id, ws_id, "world-settings");
    ws.owner = Some(gm_ctx.user_id);
    ws.system = json!({
        "scene": {
            "losRestriction": true, "fog": true,
            "lightingEnabled": true, "lightMode": "environmentLight",
            "environment": { "color": "#000000", "intensity": 0.0 },
            "observerVision": false,
            "movementRestriction": "visible",
            "partialCellLeniency": true
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    });
    ws.engine = Some(ws_engine(ws.system.clone()));
    room.publish(
        &repo,
        &gm_ctx,
        vec![Operation::Create { doc: ws }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut scene = wdoc(world_id, scene_id, "scene");
    scene.owner = Some(gm_ctx.user_id);
    room.publish(
        &repo,
        &gm_ctx,
        vec![Operation::Create { doc: scene }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // brightRadius=1.0 cell (100 wu) around (50,50): lights cell (0,0) only.
    let mut light = wdoc(world_id, light_id, "light");
    light.parent_id = Some(scene_id);
    light.owner = Some(gm_ctx.user_id);
    light.system = json!({
        "x": 50.0, "y": 50.0, "emission": { "color": "#ffffff", "intensity": 1.0, "brightRadius": 1.0, "dimRadius": 1.0, "enabled": true }
    });
    light.engine = Some(light.system.clone());
    room.publish(
        &repo,
        &gm_ctx,
        vec![Operation::Create { doc: light }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // A vision source: `visible_cells` requires an owned (or, under `observerVision`,
    // whole-document-READ) token in the
    // scene — without one, `sources` is empty and the mask is unconditionally empty
    // regardless of lighting.
    let mut vision_token = wdoc(world_id, vision_token_id, "token");
    vision_token.parent_id = Some(scene_id);
    vision_token.owner = Some(p);
    vision_token
        .permissions
        .users
        .insert(p, crate::data::document::DocRole::Owner);
    vision_token.engine = Some(token_engine(50.0, 50.0));
    room.publish(
        &repo,
        &gm_ctx,
        vec![Operation::Create { doc: vision_token }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // Non-vacuity: an empty mask would make the "inside the mask" Create test pass for the
    // wrong reason (Forbidden regardless of placement).
    {
        let scene_ecs = room.scene().read().await;
        let mask = scene_ecs.visible_cells(
            player_ctx.user_id,
            player_ctx.world_role,
            &WorldCapDefaults::default(),
            scene_id,
            true,
        );
        assert!(
            !mask.is_empty(),
            "fixture's lit corner must produce a non-empty visible mask"
        );
    }

    PlaceHandle {
        room,
        repo,
        gm_ctx,
        player_ctx,
        token: Uuid::nil(),
        world: world_id,
        scene: scene_id,
    }
}

/// Same scene/lighting as `room_with_player_create_capability_and_lit_corner`, without the
/// `core:create` grant — exercises the GM's unconditional Create placement instead.
async fn room_with_gm_and_lit_corner() -> PlaceHandle {
    use serde_json::json;

    let (repo, world_id, gm_ctx) = repo_with_world().await;
    let p = repo
        .create_user("player", None, ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world_id, p, WorldRole::Player)
        .await
        .unwrap();
    let player_ctx = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };

    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let scene_id = Uuid::from_u128(0xD930);
    let ws_id = Uuid::from_u128(0xD931);
    let light_id = Uuid::from_u128(0xD932);

    let mut ws = wdoc(world_id, ws_id, "world-settings");
    ws.owner = Some(gm_ctx.user_id);
    ws.system = json!({
        "scene": {
            "losRestriction": true, "fog": true,
            "lightingEnabled": true, "lightMode": "environmentLight",
            "environment": { "color": "#000000", "intensity": 0.0 },
            "observerVision": false,
            "movementRestriction": "visible",
            "partialCellLeniency": true
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    });
    ws.engine = Some(ws_engine(ws.system.clone()));
    room.publish(
        &repo,
        &gm_ctx,
        vec![Operation::Create { doc: ws }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut scene = wdoc(world_id, scene_id, "scene");
    scene.owner = Some(gm_ctx.user_id);
    room.publish(
        &repo,
        &gm_ctx,
        vec![Operation::Create { doc: scene }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut light = wdoc(world_id, light_id, "light");
    light.parent_id = Some(scene_id);
    light.owner = Some(gm_ctx.user_id);
    light.system = json!({
        "x": 50.0, "y": 50.0, "emission": { "color": "#ffffff", "intensity": 1.0, "brightRadius": 1.0, "dimRadius": 1.0, "enabled": true }
    });
    light.engine = Some(light.system.clone());
    room.publish(
        &repo,
        &gm_ctx,
        vec![Operation::Create { doc: light }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    PlaceHandle {
        room,
        repo,
        gm_ctx,
        player_ctx,
        token: Uuid::nil(),
        world: world_id,
        scene: scene_id,
    }
}

/// Same as `room_with_player_create_capability_and_lit_corner`, but `movementRestriction:
/// unrestricted` — Create is authorized everywhere regardless of the mask.
async fn room_with_player_create_and_unrestricted_scene() -> PlaceHandle {
    use serde_json::json;

    let (repo, world_id, gm_ctx) = repo_with_world().await;
    let p = repo
        .create_user("player", None, ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world_id, p, WorldRole::Player)
        .await
        .unwrap();
    let player_ctx = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };

    let mut caps = crate::data::document::WorldCapDefaults::default();
    caps.role_caps
        .all
        .entry(WorldRole::Player)
        .or_default()
        .insert("core:create".into());
    repo.set_world_cap_defaults(world_id, &caps).await.unwrap();

    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let scene_id = Uuid::from_u128(0xD940);
    let ws_id = Uuid::from_u128(0xD941);

    let mut ws = wdoc(world_id, ws_id, "world-settings");
    ws.owner = Some(gm_ctx.user_id);
    ws.system = json!({
        "scene": {
            "losRestriction": true, "fog": true,
            "lightingEnabled": false, "lightMode": "environmentLight",
            "environment": { "color": "#000000", "intensity": 0.0 },
            "observerVision": false,
            "movementRestriction": "unrestricted",
            "partialCellLeniency": true
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    });
    ws.engine = Some(ws_engine(ws.system.clone()));
    room.publish(
        &repo,
        &gm_ctx,
        vec![Operation::Create { doc: ws }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut scene = wdoc(world_id, scene_id, "scene");
    scene.owner = Some(gm_ctx.user_id);
    room.publish(
        &repo,
        &gm_ctx,
        vec![Operation::Create { doc: scene }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    PlaceHandle {
        room,
        repo,
        gm_ctx,
        player_ctx,
        token: Uuid::nil(),
        world: world_id,
        scene: scene_id,
    }
}

/// Same lighting/vision setup as `room_with_player_create_capability_and_lit_corner`, but
/// `movementRestriction: revealed` — Create is authorized against `visible ∪ explored`,
/// exercising the Create gate's Revealed branch (`revealed_pending`/`get_explored`), which
/// is otherwise unreachable through `Operation::Create` in this test module.
async fn room_with_player_create_capability_and_revealed_corner() -> PlaceHandle {
    use serde_json::json;

    let (repo, world_id, gm_ctx) = repo_with_world().await;
    let p = repo
        .create_user("player", None, ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world_id, p, WorldRole::Player)
        .await
        .unwrap();
    let player_ctx = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };

    let mut caps = crate::data::document::WorldCapDefaults::default();
    caps.role_caps
        .all
        .entry(WorldRole::Player)
        .or_default()
        .insert("core:create".into());
    repo.set_world_cap_defaults(world_id, &caps).await.unwrap();

    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let scene_id = Uuid::from_u128(0xD950);
    let ws_id = Uuid::from_u128(0xD951);
    let light_id = Uuid::from_u128(0xD952);
    let vision_token_id = Uuid::from_u128(0xD953);

    let mut ws = wdoc(world_id, ws_id, "world-settings");
    ws.owner = Some(gm_ctx.user_id);
    ws.system = json!({
        "scene": {
            "losRestriction": true, "fog": true,
            "lightingEnabled": true, "lightMode": "environmentLight",
            "environment": { "color": "#000000", "intensity": 0.0 },
            "observerVision": false,
            "movementRestriction": "revealed",
            "partialCellLeniency": true
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    });
    ws.engine = Some(ws_engine(ws.system.clone()));
    room.publish(
        &repo,
        &gm_ctx,
        vec![Operation::Create { doc: ws }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut scene = wdoc(world_id, scene_id, "scene");
    scene.owner = Some(gm_ctx.user_id);
    room.publish(
        &repo,
        &gm_ctx,
        vec![Operation::Create { doc: scene }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // brightRadius=1.0 cell (100 wu) around (50,50): lights cell (0,0) only.
    let mut light = wdoc(world_id, light_id, "light");
    light.parent_id = Some(scene_id);
    light.owner = Some(gm_ctx.user_id);
    light.system = json!({
        "x": 50.0, "y": 50.0, "emission": { "color": "#ffffff", "intensity": 1.0, "brightRadius": 1.0, "dimRadius": 1.0, "enabled": true }
    });
    light.engine = Some(light.system.clone());
    room.publish(
        &repo,
        &gm_ctx,
        vec![Operation::Create { doc: light }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // A vision source: `visible_cells` requires an owned (or, under `observerVision`,
    // whole-document-READ) token in the
    // scene — without one, `sources` is empty and the mask is unconditionally empty
    // regardless of lighting.
    let mut vision_token = wdoc(world_id, vision_token_id, "token");
    vision_token.parent_id = Some(scene_id);
    vision_token.owner = Some(p);
    vision_token
        .permissions
        .users
        .insert(p, crate::data::document::DocRole::Owner);
    vision_token.engine = Some(token_engine(50.0, 50.0));
    room.publish(
        &repo,
        &gm_ctx,
        vec![Operation::Create { doc: vision_token }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // Non-vacuity: an empty visible mask leaves it ambiguous whether the Revealed branch's
    // explored union is doing any work in the tests built on this fixture.
    {
        let scene_ecs = room.scene().read().await;
        let mask = scene_ecs.visible_cells(
            player_ctx.user_id,
            player_ctx.world_role,
            &WorldCapDefaults::default(),
            scene_id,
            true,
        );
        assert!(
            !mask.is_empty(),
            "fixture's lit corner must produce a non-empty visible mask"
        );
    }

    PlaceHandle {
        room,
        repo,
        gm_ctx,
        player_ctx,
        token: Uuid::nil(),
        world: world_id,
        scene: scene_id,
    }
}

/// A token `Document` at `(x, y)` in `world`, parented to `scene`, ready for
/// `Operation::Create`. `permissions.default = Owner` carries the WRITE_FIELDS floor for
/// WHICHEVER user creates it (player or GM) — this fixture exercises the placement mask, not
/// per-user document ownership, so `core:create` (from the fixture's world-cap grant, or
/// the GM's unconditional access) is the only authorization axis in play here.
fn token_doc_at(world: Uuid, scene: Uuid, x: f64, y: f64) -> Document {
    use crate::data::document::DocRole;
    let mut doc = crate::data::document::tests::world_scoped_doc(world, Uuid::new_v4(), "token");
    doc.parent_id = Some(scene);
    doc.permissions.default = DocRole::Owner;
    doc.engine = Some(token_engine(x, y));
    doc
}

#[tokio::test]
async fn non_gm_token_position_update_is_refused() {
    use serde_json::json;
    let h = room_with_player_owned_token().await;
    let ops = vec![Operation::Update {
        doc_id: h.token,
        changes: vec![
            FieldChange {
                path: "/engine/x".into(),
                old: json!(50.0),
                new: json!(150.0),
                remove: false,
            },
            FieldChange {
                path: "/engine/y".into(),
                old: json!(50.0),
                new: json!(50.0),
                remove: false,
            },
        ],
    }];
    let err = h
        .room
        .publish(&h.repo, &h.player_ctx, ops, 0, WriteOrigin::Client)
        .await
        .expect_err("a player may not write a token position");
    assert!(
        matches!(err, DataError::Forbidden),
        "refused as Forbidden, got {err:?}"
    );
}

#[tokio::test]
async fn gm_token_position_update_still_succeeds_through_a_wall() {
    use serde_json::json;
    // A GM places a token where they like, walls included.
    let h = room_with_gm_and_blocking_wall().await;
    let ops = vec![Operation::Update {
        doc_id: h.token,
        changes: vec![FieldChange {
            path: "/engine/x".into(),
            old: json!(50.0),
            new: json!(250.0),
            remove: false,
        }],
    }];
    h.room
        .publish(&h.repo, &h.gm_ctx, ops, 0, WriteOrigin::Client)
        .await
        .expect("a GM position write is unconditional");
}

#[tokio::test]
async fn non_gm_wholesale_engine_write_that_moves_a_token_is_refused() {
    // Post-image detection: `token_move` applies all changes in array order over the
    // committed /engine band, so replacing the whole band cannot smuggle a position change
    // past a per-path check.
    use serde_json::json;
    let h = room_with_player_owned_token().await;
    let ops = vec![Operation::Update {
        doc_id: h.token,
        changes: vec![FieldChange {
            path: "/engine".into(),
            old: json!({"x": 50.0, "y": 50.0}),
            new: json!({"x": 150.0, "y": 50.0}),
            remove: false,
        }],
    }];
    let err = h
        .room
        .publish(&h.repo, &h.player_ctx, ops, 0, WriteOrigin::Client)
        .await
        .expect_err("a wholesale engine write is caught");
    assert!(matches!(err, DataError::Forbidden));
}

#[tokio::test]
async fn non_gm_non_position_token_update_still_succeeds() {
    // The refusal is scoped to POSITION CHANGE, not to token writes generally. `token_move`
    // returns Some for any token with readable x/y, so an `.is_some()` predicate would refuse
    // this — the pre/post comparison is what makes this test pass.
    use serde_json::json;
    let h = room_with_player_owned_token().await;
    let ops = vec![Operation::Update {
        doc_id: h.token,
        changes: vec![FieldChange {
            path: "/engine/rotation".into(),
            old: json!(0.0),
            new: json!(90.0),
            remove: false,
        }],
    }];
    h.room
        .publish(&h.repo, &h.player_ctx, ops, 0, WriteOrigin::Client)
        .await
        .expect("a player may still rotate a token they own");
}

#[tokio::test]
async fn non_gm_engine_write_leaving_position_unchanged_succeeds() {
    // The boundary of the pre/post comparison: an /engine write that re-states the SAME x,y
    // is not a move and must be allowed.
    use serde_json::json;
    let h = room_with_player_owned_token().await;
    let ops = vec![Operation::Update {
        doc_id: h.token,
        changes: vec![FieldChange {
            path: "/engine/x".into(),
            old: json!(50.0),
            new: json!(50.0),
            remove: false,
        }],
    }];
    h.room
        .publish(&h.repo, &h.player_ctx, ops, 0, WriteOrigin::Client)
        .await
        .expect("a no-op position write is not a move");
}

#[tokio::test]
async fn non_gm_token_create_outside_the_mask_is_refused() {
    let h = room_with_player_create_capability_and_lit_corner().await;
    let ops = vec![Operation::Create {
        doc: token_doc_at(h.world, h.scene, 500.0, 500.0),
    }];
    let err = h
        .room
        .publish(&h.repo, &h.player_ctx, ops, 0, WriteOrigin::Client)
        .await
        .expect_err("placement in fog is refused");
    assert!(matches!(err, DataError::Forbidden));
}

#[tokio::test]
async fn non_gm_token_create_inside_the_mask_succeeds() {
    let h = room_with_player_create_capability_and_lit_corner().await;
    let ops = vec![Operation::Create {
        doc: token_doc_at(h.world, h.scene, 50.0, 50.0),
    }];
    h.room
        .publish(&h.repo, &h.player_ctx, ops, 0, WriteOrigin::Client)
        .await
        .expect("placement in a visible cell is allowed");
}

#[tokio::test]
async fn non_gm_token_create_in_explored_but_unlit_cell_succeeds() {
    // Guards the Create gate's Revealed branch (the `revealed_pending` consumption
    // loop): a cell that is explored-but-not-currently-visible must still admit a player
    // Create, mirroring `execute_move_revealed_union_allows_explored_cell`'s movement-side
    // assertion of the same `visible ∪ explored` contract.
    let h = room_with_player_create_capability_and_revealed_corner().await;

    // Target (550,550) = cell (5,5): outside the fixture's lit corner (only (0,0) is lit).
    let mut seed = crate::scene::explored::ExploredSet::new();
    seed.mark_cells((0..6).flat_map(|i| (0..6).map(move |j| (i, j))));
    h.repo
        .set_explored(
            h.world,
            h.scene,
            h.player_ctx.user_id,
            &seed.to_bytes(crate::scene::GridKind::Square),
        )
        .await
        .unwrap();

    let ops = vec![Operation::Create {
        doc: token_doc_at(h.world, h.scene, 550.0, 550.0),
    }];
    h.room
        .publish(&h.repo, &h.player_ctx, ops, 0, WriteOrigin::Client)
        .await
        .expect("placement in an explored-but-unlit cell is allowed under Revealed");
}

#[tokio::test]
async fn non_gm_token_create_outside_visible_and_explored_is_refused() {
    let h = room_with_player_create_capability_and_revealed_corner().await;
    // No explored blob seeded: the target cell is neither visible nor explored.
    let ops = vec![Operation::Create {
        doc: token_doc_at(h.world, h.scene, 900.0, 900.0),
    }];
    let err = h
        .room
        .publish(&h.repo, &h.player_ctx, ops, 0, WriteOrigin::Client)
        .await
        .expect_err("placement outside visible ∪ explored is refused");
    assert!(matches!(err, DataError::Forbidden));
}

#[tokio::test]
async fn gm_token_create_anywhere_succeeds() {
    let h = room_with_gm_and_lit_corner().await;
    let ops = vec![Operation::Create {
        doc: token_doc_at(h.world, h.scene, 500.0, 500.0),
    }];
    h.room
        .publish(&h.repo, &h.gm_ctx, ops, 0, WriteOrigin::Client)
        .await
        .expect("a GM places a token anywhere");
}

#[tokio::test]
async fn unrestricted_scene_ungates_non_gm_token_create() {
    let h = room_with_player_create_and_unrestricted_scene().await;
    let ops = vec![Operation::Create {
        doc: token_doc_at(h.world, h.scene, 500.0, 500.0),
    }];
    h.room
        .publish(&h.repo, &h.player_ctx, ops, 0, WriteOrigin::Client)
        .await
        .expect("Unrestricted ungates placement, as it ungates movement");
}

#[tokio::test]
async fn get_or_create_hydrates_config_and_actors_from_db() {
    use crate::data::document::DocRole;
    use serde_json::json;
    let (repo, world_id, gm) = repo_with_world().await;
    let p = repo
        .create_user("p", None, ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world_id, p, WorldRole::Player)
        .await
        .unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let (scene_id, token_id, light_id, ws_id) = (
        Uuid::from_u128(10),
        Uuid::from_u128(11),
        Uuid::from_u128(12),
        Uuid::from_u128(13),
    );

    // First registry: publish (→ DB) world-settings + scene + player-owned token + an enabled
    // light at the token cell. These writes go through apply_op on reg1's room, committing to
    // the DB. The second registry never sees any of these live publishes.
    let reg1 = RoomRegistry::new();
    let room1 = reg1.get_or_create(&repo, world_id).await.unwrap().unwrap();

    let mut ws = wdoc(world_id, ws_id, "world-settings");
    ws.owner = Some(gm.user_id);
    ws.system = json!({
        "scene": { "losRestriction": true, "fog": true, "lightingEnabled": true,
                   "lightMode": "environmentLight", "environment": {"color":"#0a0e1a","intensity":0.0},
                   "observerVision": false, "movementRestriction": "visible", "partialCellLeniency": true },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" } });
    ws.engine = Some(ws_engine(ws.system.clone()));
    room1
        .publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: ws }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    let mut scene = wdoc(world_id, scene_id, "scene");
    scene.owner = Some(gm.user_id);
    scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
    room1
        .publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: scene }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    let mut token = wdoc(world_id, token_id, "token");
    token.parent_id = Some(scene_id);
    token.owner = Some(p);
    token.permissions.users.insert(p, DocRole::Owner);
    token.engine = Some(token_engine(50.0, 50.0));
    room1
        .publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: token }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    let mut light = wdoc(world_id, light_id, "light");
    light.parent_id = Some(scene_id);
    light.owner = Some(gm.user_id);
    light.system = json!({
        "x": 50.0, "y": 50.0, "emission": { "color": "#ffffff", "intensity": 1.0, "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true }
    });
    light.engine = Some(light.system.clone());
    room1
        .publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: light }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    // A FRESH registry never saw the live publishes: a non-empty mask here proves
    // get_or_create hydrated the config-docs + scene/token/light from the DB (NOT the
    // apply_op live path). If the four query_documents hydration calls are removed from
    // get_or_create, world_settings_doc() returns None and the player_lit_mask uses
    // fail-closed defaults with env_intensity 0.0 + no world-settings structural guard,
    // meaning resolve_scene has no world-settings layer — but the light is still a scene
    // entity so it IS hydrated via from_documents. What the hydration calls specifically
    // prove is that the world-settings doc is present on the cold-start room, confirming
    // the config-doc queries ran. The mask non-emptiness proves the full chain end-to-end
    // (world-settings resolved + scene entity light + player token all loaded from DB).
    let reg2 = RoomRegistry::new();
    let room2 = reg2.get_or_create(&repo, world_id).await.unwrap().unwrap();
    let ecs = room2.scene().read().await;
    assert!(
        ecs.world_settings_doc().is_some(),
        "world-settings hydrated from DB by get_or_create"
    );
    let mask = ecs.player_lit_mask(
        p,
        WorldRole::Player,
        &WorldCapDefaults::default(),
        &ecs.resolved_bands(),
    );
    assert!(
        mask.iter().any(|s| !s.cells.is_empty()),
        "player lit mask non-empty after cold-start hydration (config + token + light from DB)"
    );
}

#[tokio::test]
async fn get_or_create_batched_query_handles_partial_doc_type_presence() {
    use serde_json::json;
    let (repo, world_id, gm) = repo_with_world().await;
    let wdoc = crate::data::document::tests::world_scoped_doc;

    let reg1 = RoomRegistry::new();
    let room1 = reg1.get_or_create(&repo, world_id).await.unwrap().unwrap();

    // Only actor + world-settings exist; light-gradation/vision-modes absent.
    let actor_id = Uuid::from_u128(30);
    let mut actor = wdoc(world_id, actor_id, "actor");
    actor.owner = Some(gm.user_id);
    actor.engine = Some(json!({
        "displayName": "Fixture Actor",
        "visual": { "kind": "image", "asset": "a.png" },
        "size": { "w": 1.0, "h": 1.0 },
        "shape": "square",
        "conditions": [],
        "prototype": true,
        "vision": [],
    }));
    room1
        .publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: actor }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    let ws_id = Uuid::from_u128(31);
    let mut ws = wdoc(world_id, ws_id, "world-settings");
    ws.owner = Some(gm.user_id);
    ws.system = json!({
        "scene": { "losRestriction": true, "fog": true, "lightingEnabled": true,
                   "lightMode": "environmentLight", "environment": {"color":"#0a0e1a","intensity":0.0},
                   "observerVision": false, "movementRestriction": "visible", "partialCellLeniency": true },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" } });
    ws.engine = Some(ws_engine(ws.system.clone()));
    room1
        .publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: ws }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    // A FRESH registry never saw the live publishes: get_or_create must hydrate
    // world-settings + actor from the DB even though light-gradation/vision-modes
    // are absent for this world — proving the batched query resolves each doc_type
    // independently rather than requiring all four to be present.
    let reg2 = RoomRegistry::new();
    let room2 = reg2.get_or_create(&repo, world_id).await.unwrap().unwrap();
    let ecs = room2.scene().read().await;

    assert!(
        ecs.world_settings_doc().is_some(),
        "world-settings must hydrate independently"
    );
    assert!(
        ecs.actor(&actor_id).is_some(),
        "actor must hydrate independently"
    );
    assert!(
        ecs.gradation_doc().is_none(),
        "absent light-gradation must not error or block others"
    );
    assert!(
        ecs.vision_modes_doc().is_none(),
        "absent vision-modes must not error or block others"
    );
}

#[tokio::test]
async fn publish_allocates_seq_buffers_and_broadcasts() {
    let (repo, world_id, ctx) = repo_with_world().await;
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
    let (mut rx, current) = room.subscribe();
    assert_eq!(current, 0);

    let cmd = room
        .publish(&repo, &ctx, vec![], 10, WriteOrigin::Client)
        .await
        .unwrap();
    assert_eq!(cmd.seq, 1);
    assert_eq!(room.current_seq(), 1);

    let got = rx.recv().await.unwrap();
    assert_eq!(got.event_seq(), Some(1));
    assert_eq!(room.stats.events_published.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn get_or_create_returns_none_for_missing_world() {
    let (repo, _world_id, _ctx) = repo_with_world().await;
    let reg = RoomRegistry::new();
    assert!(reg
        .get_or_create(&repo, Uuid::from_u128(999))
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn resync_hot_then_cold_tiers() {
    let (repo, world_id, ctx) = repo_with_world().await;
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
    for _ in 0..3 {
        room.publish(&repo, &ctx, vec![], 0, WriteOrigin::Client)
            .await
            .unwrap();
    } // seq 1,2,3

    // hot: from_seq 2 resident in buffer
    let (hot, src) = room.resync_range(&repo, 2).await.unwrap();
    assert_eq!(src, ResyncSource::Buffer);
    assert_eq!(
        hot.iter()
            .map(|m| m.event_seq().unwrap())
            .collect::<Vec<_>>(),
        vec![2, 3]
    );
}

#[tokio::test]
async fn publish_is_ordered_under_concurrency() {
    let (repo, world_id, ctx) = repo_with_world().await;
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
    let (mut rx, _) = room.subscribe();

    let repo = std::sync::Arc::new(repo);
    let mut handles = vec![];
    for _ in 0..50 {
        let room = room.clone();
        let repo = repo.clone();
        handles.push(tokio::spawn(async move {
            room.publish(repo.as_ref(), &ctx, vec![], 0, WriteOrigin::Client)
                .await
                .unwrap();
        }));
    }
    for h in handles {
        h.await.unwrap();
    }

    let mut seqs = vec![];
    for _ in 0..50 {
        seqs.push(rx.recv().await.unwrap().event_seq().unwrap());
    }
    let mut sorted = seqs.clone();
    sorted.sort();
    assert_eq!(
        seqs, sorted,
        "broadcast delivery order must equal seq order"
    );
    assert_eq!(seqs, (1..=50).collect::<Vec<_>>());
}

// -----------------------------------------------------------------------
// Movement-restriction gate
// -----------------------------------------------------------------------

struct MovementHandle {
    room: Arc<Room>,
    repo: SqliteRepository,
    gm: PermissionContext,
    player: PermissionContext,
    world_id: Uuid,
    scene_id: Uuid,
    token_id: Uuid,
    /// Committed start position of the primary token (scene-unit coords).
    start: (f64, f64),
    /// A lit cell reachable from `start` without crossing any wall.
    lit_goal: (f64, f64),
    /// An adjacent (king-step) cell reachable from `start` (unrestricted/visible scenes).
    adj: (f64, f64),
    /// A cell adjacent to `adj`, used as the second leg in moving-lock tests.
    adj2: (f64, f64),
}

impl MovementHandle {
    /// Read the committed position of `token` from the authoritative ECS.
    async fn committed_pos(&self, token: Uuid) -> (f64, f64) {
        self.room
            .scene()
            .read()
            .await
            .token_position(token)
            .expect("token not found in ECS")
    }
}

/// Publish world-settings with `movementRestriction`, a scene (grid 100), a
/// player-owned token at (50,50), and optionally a white point light at (50,50)
/// with brightRadius=1.5, dimRadius=3.0. Env intensity=0 so only the placed
/// light illuminates (cells beyond ~1.5 cell-radii are dark).
async fn movement_scene(restriction: &str, with_light: bool) -> MovementHandle {
    movement_scene_with_speed(restriction, with_light, 6.0).await
}

/// `movement_scene`, with the world's animation speed (cells/sec) under test control.
///
/// The per-token moving lock's end epoch is derived from the travelled distance in GRID STEPS
/// divided by the speed (`MoveExecution::duration_ms` states the conversion), and
/// `Room::execute_move` checks it against its OWN internal `ws::time::now_millis()` — not the
/// `now` argument — so a test cannot hold the lock open by pinning the clock. At the default 6
/// cells/sec a one-cell move locks for only ~167 ms, which a loaded machine can outrun between
/// two awaits. A test asserting lock-held behavior must therefore choose a speed slow enough
/// that the window cannot close under any plausible scheduling delay.
async fn movement_scene_with_speed(
    restriction: &str,
    with_light: bool,
    speed_cells_per_sec: f64,
) -> MovementHandle {
    use crate::data::document::DocRole;
    use serde_json::json;

    let (repo, world_id, gm) = repo_with_world().await;
    let p = repo
        .create_user("player", None, crate::auth::role::ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world_id, p, WorldRole::Player)
        .await
        .unwrap();
    let player = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };

    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let (scene_id, token_id, ws_id, light_id) = (
        Uuid::from_u128(0x5CE0),
        Uuid::from_u128(0x5CE1),
        Uuid::from_u128(0x5CE2),
        Uuid::from_u128(0x5CE3),
    );

    let mut ws = wdoc(world_id, ws_id, "world-settings");
    ws.owner = Some(gm.user_id);
    ws.system = json!({
        "scene": {
            "losRestriction": true, "fog": true,
            "lightingEnabled": true, "lightMode": "environmentLight",
            "environment": { "color": "#000000", "intensity": 0.0 },
            "observerVision": false,
            "movementRestriction": restriction,
            "partialCellLeniency": true
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": speed_cells_per_sec, "easing": "easeInOut" }
    });
    ws.engine = Some(ws_engine(ws.system.clone()));
    room.publish(
        &repo,
        &gm,
        vec![Operation::Create { doc: ws }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut scene = wdoc(world_id, scene_id, "scene");
    scene.owner = Some(gm.user_id);
    scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
    room.publish(
        &repo,
        &gm,
        vec![Operation::Create { doc: scene }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut token = wdoc(world_id, token_id, "token");
    token.parent_id = Some(scene_id);
    token.owner = Some(p);
    token.permissions.users.insert(p, DocRole::Owner);
    token.engine = Some(token_engine(50.0, 50.0));
    room.publish(
        &repo,
        &gm,
        vec![Operation::Create { doc: token }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    if with_light {
        // Bright boundary = 1.5 * 100 = 150 world units from (50,50).
        // Cell (0,0) center=(50,50): dist=0 → lit. Cell (20,20) center=(2050,2050): dark.
        let mut light = wdoc(world_id, light_id, "light");
        light.parent_id = Some(scene_id);
        light.owner = Some(gm.user_id);
        light.system = json!({
            "x": 50.0, "y": 50.0, "emission": { "color": "#ffffff", "intensity": 1.0, "brightRadius": 1.5, "dimRadius": 3.0, "enabled": true }
        });
        light.engine = Some(light.system.clone());
        room.publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: light }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    }

    MovementHandle {
        room,
        repo,
        gm,
        player,
        world_id,
        scene_id,
        token_id,
        // Token starts at (50,50) — center of cell (0,0) with grid size 100.
        start: (50.0, 50.0),
        // Cell (0,0) is illuminated by the light at (50,50); (0,0) center=(50,50) → lit.
        // For unrestricted/no-light scenes this field is still a reachable adjacent cell.
        lit_goal: (50.0, 150.0),
        // Adjacent cell: one king-step from (50,50).
        adj: (150.0, 50.0),
        // Two king-steps from start: used as the second leg in moving-lock tests.
        adj2: (250.0, 50.0),
    }
}

// -----------------------------------------------------------------------
// commit_ops_locked direct test — gate-free authoritative write path
// -----------------------------------------------------------------------

#[tokio::test]
async fn commit_ops_writes_and_broadcasts_without_gating() {
    let (repo, world_id, ctx) = repo_with_world().await;
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
    let (mut rx, current) = room.subscribe();
    assert_eq!(current, 0);

    // Build a real create op — mirrors publish_hydrates_scene_ecs exactly so this
    // test exercises the ECS apply_op write path and commits a real document row,
    // not just the seq-bump + broadcast path.
    let mut scene =
        crate::data::document::tests::world_scoped_doc(world_id, Uuid::from_u128(20), "scene");
    scene.owner = Some(ctx.user_id);
    let op = Operation::Create { doc: scene };

    // Acquire the guard here, mirroring the single-acquisition discipline: the caller
    // (publish or execute_move) holds the guard, then calls commit_ops_locked.
    // Invariant: commit_ops_locked MUST NOT re-acquire publish_guard (deadlock).
    let _guard = room.publish_guard.lock().await;
    let cmd = room
        .commit_ops_locked(&repo, &ctx, vec![op], 10, WriteOrigin::Client)
        .await
        .unwrap();
    drop(_guard);

    assert_eq!(cmd.seq, 1);
    assert_eq!(room.current_seq(), cmd.seq);
    assert_eq!(room.stats.events_published.load(Ordering::Relaxed), 1);
    assert!(matches!(rx.recv().await.unwrap(), RoomEvent::Event(_)));
    // Verify the create op landed: cmd carries the committed op and the ECS reflects it.
    assert!(
        !cmd.ops.is_empty(),
        "committed command must carry the create op"
    );
    assert_eq!(
        room.scene().read().await.entity_count(),
        1,
        "ECS must reflect the committed scene entity"
    );
}

// -----------------------------------------------------------------------
// Room::execute_move — server-authoritative atomic move + moving lock
// -----------------------------------------------------------------------

/// Scene with token at (50,50), a wall that blocks the step from `corner` to
/// `beyond_wall`, and movementRestriction="unrestricted" so only the wall gate fires.
///
/// Geometry (grid size=100):
///   - start       = (50,50)  — token committed position (center of cell 0,0)
///   - corner      = (150,50) — one king-step right; clear (no wall on this path)
///   - beyond_wall = (150,150) — one king-step down from corner; a horizontal wall
///     at y=100 (x ∈ [100,200]) blocks the step corner→beyond_wall.
///
/// Wall: x1=100,y1=100,x2=200,y2=100. Step (150,50)→(150,150): vertical at x=150
/// crosses y=100 — blocked.
async fn movement_scene_with_wall() -> MovementHandle {
    use crate::data::document::DocRole;
    use serde_json::json;

    let (repo, world_id, gm) = repo_with_world().await;
    let p = repo
        .create_user("player_wall", None, crate::auth::role::ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world_id, p, WorldRole::Player)
        .await
        .unwrap();
    let player = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };

    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let (scene_id, token_id, ws_id, wall_id) = (
        Uuid::from_u128(0xFA11_0001),
        Uuid::from_u128(0xFA11_0002),
        Uuid::from_u128(0xFA11_0003),
        Uuid::from_u128(0xFA11_0004),
    );

    // Unrestricted: only the wall gate applies, no lighting or mask required.
    let mut ws = wdoc(world_id, ws_id, "world-settings");
    ws.owner = Some(gm.user_id);
    ws.system = json!({
        "scene": {
            "losRestriction": false, "fog": false,
            "lightingEnabled": false, "lightMode": "environmentLight",
            "environment": { "color": "#ffffff", "intensity": 1.0 },
            "observerVision": false,
            "movementRestriction": "unrestricted",
            "partialCellLeniency": true
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    });
    ws.engine = Some(ws_engine(ws.system.clone()));
    room.publish(
        &repo,
        &gm,
        vec![Operation::Create { doc: ws }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut scene = wdoc(world_id, scene_id, "scene");
    scene.owner = Some(gm.user_id);
    scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
    room.publish(
        &repo,
        &gm,
        vec![Operation::Create { doc: scene }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut token = wdoc(world_id, token_id, "token");
    token.parent_id = Some(scene_id);
    token.owner = Some(p);
    token.permissions.users.insert(p, DocRole::Owner);
    token.engine = Some(token_engine(50.0, 50.0));
    room.publish(
        &repo,
        &gm,
        vec![Operation::Create { doc: token }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // Horizontal wall at y=100, x ∈ [100,200]. Blocks vertical step (150,50)→(150,150).
    let mut wall = wdoc(world_id, wall_id, "wall");
    wall.parent_id = Some(scene_id);
    wall.owner = Some(gm.user_id);
    wall.engine =
        Some(json!({ "seg": { "x1": 100, "y1": 100, "x2": 200, "y2": 100 }, "blocksMove": true }));
    room.publish(
        &repo,
        &gm,
        vec![Operation::Create { doc: wall }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    MovementHandle {
        room,
        repo,
        gm,
        player,
        world_id,
        scene_id,
        token_id,
        start: (50.0, 50.0),
        // clear one-step right; used as `lit_goal` and `adj` (corner)
        lit_goal: (150.0, 50.0),
        adj: (150.0, 50.0),
        // wall blocks the step adj→adj2 (beyond wall)
        adj2: (150.0, 150.0),
    }
}

/// Current epoch milliseconds for test timestamps.
fn now_millis() -> i64 {
    crate::ws::time::now_millis()
}

#[tokio::test]
async fn execute_move_commits_the_stop_it_returns() {
    // "visible" restriction with a light: start (50,50) and the adjacent cell (50,150)
    // are both within the bright radius (1.5 cells), so the player move is allowed.
    // The committed ECS position must equal the returned stop.
    let h = movement_scene("visible", /*with_light=*/ true).await;
    let res = h
        .room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path: vec![h.start, h.lit_goal],
                ts: now_millis(),
                request_id: Uuid::from_u128(0xF00D_0001),
            },
        )
        .await
        .unwrap();
    // Committed ECS position must equal stop (atomic write invariant).
    assert_eq!(h.committed_pos(h.token_id).await, res.stop);
}

/// `movement_scene("visible", true)` — one player token in scene A, lit only near the
/// origin — plus a SECOND scene B in the same world, for exercising a `MoveRequest` that
/// names a scene the moved token does not live in.
///
/// `b_unrestricted`: B carries a per-scene `movementRestriction: "unrestricted"` override,
/// so gating against B skips the visibility mask entirely.
/// `b_lit_token`: the player also owns a token in B under a wide light, so B's mask
/// authorizes scene-local coordinates that are dark (and therefore unauthorized) in A.
///
/// Returns the handle for scene A and B's id.
async fn movement_scene_with_second_scene(
    b_unrestricted: bool,
    b_lit_token: bool,
) -> (MovementHandle, Uuid) {
    use crate::data::document::DocRole;
    use serde_json::json;

    let h = movement_scene("visible", true).await;
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let (scene_b, token_b, light_b) = (
        Uuid::from_u128(0x5CEB_0001),
        Uuid::from_u128(0x5CEB_0002),
        Uuid::from_u128(0x5CEB_0003),
    );

    let mut scene = wdoc(h.world_id, scene_b, "scene");
    scene.owner = Some(h.gm.user_id);
    scene.engine = Some(if b_unrestricted {
        json!({
            "grid": { "kind": "square", "size": 100 },
            "vision": { "movementRestriction": "unrestricted" }
        })
    } else {
        json!({ "grid": { "kind": "square", "size": 100 } })
    });
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: scene }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    if b_lit_token {
        // Vision source in B so the player has an LOS polygon there at all.
        let mut token = wdoc(h.world_id, token_b, "token");
        token.parent_id = Some(scene_b);
        token.owner = Some(h.player.user_id);
        token
            .permissions
            .users
            .insert(h.player.user_id, DocRole::Owner);
        token.engine = Some(token_engine(250.0, 50.0));
        h.room
            .publish(
                &h.repo,
                &h.gm,
                vec![Operation::Create { doc: token }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();

        // Dim boundary = 6 cells = 600 units from (250,50): cells (0,0)..(8,0) are lit in B,
        // whereas A's light (bright 1.5 / dim 3.0 from (50,50)) leaves cell (4,0) dark.
        let mut light = wdoc(h.world_id, light_b, "light");
        light.parent_id = Some(scene_b);
        light.owner = Some(h.gm.user_id);
        light.system = json!({
            "x": 250.0, "y": 50.0, "emission": { "color": "#ffffff", "intensity": 1.0, "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true }
        });
        light.engine = Some(light.system.clone());
        h.room
            .publish(
                &h.repo,
                &h.gm,
                vec![Operation::Create { doc: light }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();
    }

    (h, scene_b)
}

#[tokio::test]
async fn execute_move_refuses_a_scene_id_the_token_does_not_live_in_unrestricted() {
    // Cross-scene gate substitution: the token lives in A (movementRestriction "visible",
    // lit only near the origin) but the request names B, which is "unrestricted". Gating
    // against B would skip the mask entirely and teleport the token 20 cells across A's fog.
    let (h, scene_b) = movement_scene_with_second_scene(true, false).await;
    let far_dark = (2050.0, 2050.0);
    let res = h
        .room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: scene_b,
                token: h.token_id,
                path: vec![h.start, far_dark],
                ts: now_millis(),
                request_id: Uuid::from_u128(0xF00D_0002),
            },
        )
        .await;
    assert!(
        matches!(res, Err(DataError::Forbidden)),
        "a MoveRequest naming a scene the token does not live in must be refused by the gate — not incidentally by the moving lock or a downstream write"
    );
    assert_eq!(
        h.committed_pos(h.token_id).await,
        h.start,
        "the refused move must not have committed a position"
    );
}

#[tokio::test]
async fn execute_move_refuses_a_scene_id_the_token_does_not_live_in_visible() {
    // Same substitution with every scene "visible": B's mask (a wide light around the
    // player's own token in B) would authorize scene-local coordinates that are dark in A.
    let (h, scene_b) = movement_scene_with_second_scene(false, true).await;
    // Cell (4,0): inside B's dim radius, outside A's.
    let dark_in_a = (450.0, 50.0);

    let res = h
        .room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: scene_b,
                token: h.token_id,
                path: vec![h.start, dark_in_a],
                ts: now_millis(),
                request_id: Uuid::from_u128(0xF00D_0003),
            },
        )
        .await;
    assert!(
        matches!(res, Err(DataError::Forbidden)),
        "B's mask must never authorize movement of a token that lives in A, and the refusal must come from the gate — not incidentally from the moving lock"
    );
    assert_eq!(
        h.committed_pos(h.token_id).await,
        h.start,
        "the refused move must not have committed a position"
    );

    // Control (runs second: the refused request above committed nothing and took no moving
    // lock): the same request named against A truncates short of the destination, proving
    // A's own mask genuinely does not authorize it.
    let control = h
        .room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path: vec![h.start, dark_in_a],
                ts: now_millis(),
                request_id: Uuid::from_u128(0xF00D_0004),
            },
        )
        .await
        .expect("same-scene request is executed, then gated per cell");
    assert_ne!(
        control.stop, dark_in_a,
        "control: A's own mask must not authorize this destination"
    );
}

#[tokio::test]
async fn both_movement_gates_refuse_a_token_whose_parent_scene_has_no_document() {
    // Anti-drift: `Room::publish` (drag) and `Room::execute_move` (MoveRequest) must agree on
    // which scenes are ADMISSIBLE AT ALL, not merely on which cells are visible — the same
    // parity axis as the shared `MAX_GATE_WALK_COORD` bound. A silent 100-unit cell-size
    // default in either gate would index the mask, the region field, and the traversal walk
    // in a grid no scene declared, and would do so in only one of the two gates.
    //
    // The world here is `unrestricted`, so neither gate can refuse for an unrelated
    // mask reason: with the default restored, `publish` reaches its `Unrestricted` continue
    // and `execute_move` walks the path unmasked, and both then fail — if at all — with
    // something other than `Forbidden`.
    use crate::data::command::FieldChange;
    use crate::data::document::DocRole;
    let h = movement_scene_with_wall().await;
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let dangling_id = Uuid::from_u128(0xDA46_3000);
    let ghost_scene = Uuid::from_u128(0xDA46_4000);
    let mut dangling = wdoc(h.world_id, dangling_id, "token");
    dangling.parent_id = Some(ghost_scene);
    dangling.owner = Some(h.player.user_id);
    dangling
        .permissions
        .users
        .insert(h.player.user_id, DocRole::Owner);
    dangling.engine = Some(token_engine(50.0, 50.0));
    // Injected straight into the derived read-model: storage's foreign key (and its
    // descendant-expanding delete) makes this state unreachable through `publish`, and
    // neither gate may depend on that storage guarantee.
    h.room
        .scene()
        .write()
        .await
        .apply_op(&Operation::Create { doc: dangling });

    let moved = h
        .room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: ghost_scene,
                token: dangling_id,
                path: vec![(50.0, 50.0), (150.0, 50.0)],
                ts: now_millis(),
                request_id: Uuid::from_u128(0xF00D_0005),
            },
        )
        .await;
    assert!(
        matches!(moved, Err(DataError::Forbidden)),
        "execute_move must refuse a token whose parent scene has no document"
    );

    let dragged = h
        .room
        .publish(
            &h.repo,
            &h.player,
            vec![Operation::Update {
                doc_id: dangling_id,
                changes: vec![
                    FieldChange {
                        remove: false,
                        path: "/engine/x".into(),
                        old: serde_json::json!(50.0),
                        new: serde_json::json!(150.0),
                    },
                    FieldChange {
                        remove: false,
                        path: "/engine/y".into(),
                        old: serde_json::json!(50.0),
                        new: serde_json::json!(50.0),
                    },
                ],
            }],
            now_millis(),
            WriteOrigin::Client,
        )
        .await;
    assert!(
        matches!(dragged, Err(DataError::Forbidden)),
        "publish's drag gate must refuse the same input execute_move refuses"
    );
}

#[tokio::test]
async fn execute_move_gate_inputs_come_from_the_tokens_own_scene() {
    // Pins the DERIVATION, independently of the rejection. Whatever the outcome shape, a
    // request naming an `unrestricted` scene the token does not live in must not move the
    // token, because the restriction, mask, walls, and regions the walk is gated against
    // come from the token's own scene. This holds both when the mismatch is refused outright
    // and when it is merely executed against the derived scene (a zero-progress stop), so
    // dropping the equality check leaves it green while dropping the derivation breaks it —
    // unlike the two tests above, which assert `is_err()` and therefore pin only the
    // redundant rejection.
    let (h, scene_b) = movement_scene_with_second_scene(true, false).await;
    let far_dark = (2050.0, 2050.0);
    let res = h
        .room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: scene_b,
                token: h.token_id,
                path: vec![h.start, far_dark],
                ts: now_millis(),
                request_id: Uuid::from_u128(0xF00D_0006),
            },
        )
        .await;
    if let Ok(exec) = &res {
        assert_eq!(
            exec.scene, h.scene_id,
            "the executed scene is the token's own, never the one the request named"
        );
    }
    // Wherever the token ended up, that cell must be one A's OWN mask authorizes — the
    // property that fails the instant any gate input is keyed on the requested scene, and
    // that holds equally whether the request was refused (stop == start) or walked under
    // A's gate. `far_dark` is outside A's mask, so a mask-skipped walk lands outside it.
    let (cx, cy) = h.committed_pos(h.token_id).await;
    let committed_cell = ((cx / 100.0).floor() as i32, (cy / 100.0).floor() as i32);
    let mask = h.room.scene().read().await.visible_cells(
        h.player.user_id,
        h.player.world_role,
        &WorldCapDefaults::default(),
        h.scene_id,
        true,
    );
    assert!(
        mask.contains(&committed_cell),
        "committed cell {committed_cell:?} is not in scene A's visibility mask"
    );
}

#[tokio::test]
async fn execute_move_still_executes_a_request_naming_the_tokens_own_scene() {
    // Guard cannot silently break play: the legitimate same-scene move still commits.
    let (h, _scene_b) = movement_scene_with_second_scene(true, true).await;
    let res = h
        .room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path: vec![h.start, h.lit_goal],
                ts: now_millis(),
                request_id: Uuid::from_u128(0xF00D_0007),
            },
        )
        .await
        .expect("same-scene move must still succeed");
    assert_eq!(res.stop, h.lit_goal);
    assert_eq!(
        res.scene, h.scene_id,
        "the executed scene is the token's own parent scene"
    );
    assert_eq!(h.committed_pos(h.token_id).await, h.lit_goal);
}

#[tokio::test]
async fn execute_move_refuses_a_token_with_no_parent_scene() {
    // Fail closed: a token with no resolvable scene has no gate inputs of its own, so the
    // client's `scene_id` must never be used as a fallback.
    use crate::data::document::DocRole;
    let h = movement_scene("visible", true).await;
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let orphan_id = Uuid::from_u128(0x0FFA_1000);
    let mut orphan = wdoc(h.world_id, orphan_id, "token");
    orphan.parent_id = None;
    orphan.owner = Some(h.player.user_id);
    orphan
        .permissions
        .users
        .insert(h.player.user_id, DocRole::Owner);
    orphan.engine = Some(token_engine(50.0, 50.0));
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: orphan }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    let res = h
        .room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: orphan_id,
                path: vec![(50.0, 50.0), (50.0, 150.0)],
                ts: now_millis(),
                request_id: Uuid::from_u128(0xF00D_0008),
            },
        )
        .await;
    assert!(
        matches!(res, Err(DataError::Forbidden)),
        "a parentless token must be refused by the gate, not by a downstream write"
    );
}

#[tokio::test]
async fn execute_move_refuses_a_token_whose_parent_scene_does_not_exist() {
    // Fail closed: a dangling `parent_id` resolves to no scene document, so no cell size,
    // restriction, mask, or wall set can be derived — the move is refused, never gated
    // against the client's `scene_id` or a default cell size.
    //
    // The state is injected straight into the derived read-model: storage enforces the
    // `parent_id` foreign key (and cascades on scene delete), so a dangling parent cannot be
    // reached through `publish`. The gate must not depend on that storage guarantee.
    // `DataError::Forbidden` (not a storage error) is asserted so the test cannot pass on the
    // downstream write failing instead of the gate refusing.
    use crate::data::document::DocRole;
    let h = movement_scene("visible", true).await;
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let dangling_id = Uuid::from_u128(0xDA46_1000);
    let ghost_scene = Uuid::from_u128(0xDA46_2000);
    let mut dangling = wdoc(h.world_id, dangling_id, "token");
    dangling.parent_id = Some(ghost_scene);
    dangling.owner = Some(h.player.user_id);
    dangling
        .permissions
        .users
        .insert(h.player.user_id, DocRole::Owner);
    dangling.engine = Some(token_engine(50.0, 50.0));
    h.room
        .scene()
        .write()
        .await
        .apply_op(&Operation::Create { doc: dangling });

    let res = h
        .room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: ghost_scene,
                token: dangling_id,
                path: vec![(50.0, 50.0), (50.0, 150.0)],
                ts: now_millis(),
                request_id: Uuid::from_u128(0xF00D_0009),
            },
        )
        .await;
    assert!(
        matches!(res, Err(DataError::Forbidden)),
        "a token whose parent scene does not exist must be refused by the gate"
    );
}

#[tokio::test]
async fn client_update_with_posint_pre_image_after_execute_move_is_accepted() {
    // Reproduces the OCC PosInt/Float variant-mismatch bug end-to-end:
    // `execute_move` commits a whole-number-valued token position, which
    // stores as a serde_json `Float` (`json!(f64)` always serializes to the
    // Float variant, even for a whole number). A subsequent client-authored
    // `Update` -- like an ordinary `sendMoves` token drag -- echoes the
    // JS-side whole number back as a `PosInt` pre-image (`JSON.parse` cannot
    // preserve "this was a float" for a whole-number value). The OCC check
    // in `apply_intent` must accept this pre-image, not spuriously Conflict.
    use crate::data::command::FieldChange;

    let h = movement_scene("unrestricted", false).await;
    h.room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path: vec![h.start, h.adj],
                ts: now_millis(),
                request_id: Uuid::from_u128(0xF00D_0010),
            },
        )
        .await
        .unwrap();
    assert_eq!(h.committed_pos(h.token_id).await, h.adj);

    // Sanity: the stored /engine/x is the Float variant serialization.
    let stored = h.repo.get_document(h.token_id).await.unwrap().unwrap();
    let stored_x = stored.engine.unwrap()["x"].clone();
    assert_eq!(
        serde_json::to_string(&stored_x).unwrap(),
        "150.0",
        "execute_move must commit the whole-number position as a Float"
    );

    // Client echoes the JS whole number 150 as a PosInt pre-image, not a
    // Float, for the OCC comparison -- exactly what `sendMoves` does.
    let ops = vec![Operation::Update {
        doc_id: h.token_id,
        changes: vec![FieldChange {
            remove: false,
            path: "/engine/x".into(),
            old: serde_json::Value::Number(serde_json::Number::from(150u64)),
            new: serde_json::json!(160.0),
        }],
    }];
    let result = h
        .repo
        .apply_intent(
            &h.player,
            h.world_id,
            ops,
            now_millis(),
            WriteOrigin::Client,
        )
        .await;
    assert!(
        result.is_ok(),
        "a PosInt pre-image numerically equal to the stored Float must be accepted, got: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn execute_move_rejects_a_moving_token() {
    // First execute_move succeeds and stamps the moving lock (end epoch in the future).
    // An immediate second call on the same token must be Forbidden while the lock is held.
    //
    // Speed 0.001 cells/sec (the floor `resolved_animation_speed` clamps to) makes the lock
    // window ~1.4e6 seconds for this one-cell move. The lock is checked against
    // `execute_move`'s own internal clock, not a test-supplied `now`, so the window must be
    // wide enough that no scheduling delay between the two awaits can close it — at the
    // default 6 cells/sec it is only ~167 ms, which a loaded machine outruns intermittently.
    let h = movement_scene_with_speed("unrestricted", false, 0.001).await;
    let _ = h
        .room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path: vec![h.start, h.adj],
                ts: now_millis(),
                request_id: Uuid::from_u128(0xF00D_0011),
            },
        )
        .await
        .unwrap();
    // Immediately request again — moving lock end is still in the future.
    let again = h
        .room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path: vec![h.adj, h.adj2],
                ts: now_millis(),
                request_id: Uuid::from_u128(0xF00D_0012),
            },
        )
        .await;
    assert!(
        matches!(again, Err(DataError::Forbidden)),
        "second execute_move on a moving token must be Forbidden"
    );
}

#[tokio::test]
async fn non_gm_mover_gets_progressive_sweep_in_unrestricted_scene() {
    // A non-GM mover in an Unrestricted-mode scene must get a progressive vision
    // sweep gated on ROLE, not on the Unrestricted restriction mode itself.
    let h = movement_scene("unrestricted", false).await;
    let res = h
        .room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path: vec![h.start, h.adj],
                ts: now_millis(),
                request_id: Uuid::from_u128(0xF00D_0013),
            },
        )
        .await
        .unwrap();
    let ServerMsg::MoveStream { mover_vision, .. } = res.frame.as_ref() else {
        panic!("frame must be a MoveStream");
    };
    assert!(
        mover_vision.is_some(),
        "a non-GM mover in an Unrestricted scene must get a progressive vision sweep, not a static-fog snap"
    );
}

#[tokio::test]
async fn gm_mover_still_gets_no_sweep_in_unrestricted_scene() {
    // GM movers must never get a sweep, regardless of restriction mode (unchanged).
    let h = movement_scene("unrestricted", false).await;
    let res = h
        .room
        .execute_move(
            &h.repo,
            &h.gm,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path: vec![h.start, h.adj],
                ts: now_millis(),
                request_id: Uuid::from_u128(0xF00D_0014),
            },
        )
        .await
        .unwrap();
    let ServerMsg::MoveStream { mover_vision, .. } = res.frame.as_ref() else {
        panic!("frame must be a MoveStream");
    };
    assert!(
        mover_vision.is_none(),
        "GM movers must not get a sweep, regardless of restriction mode (unchanged behavior)"
    );
}

#[tokio::test]
async fn execute_move_truncates_at_a_wall_atomically() {
    // Path: start → corner → beyond_wall. Wall blocks the second step; executor
    // truncates at corner and commits atomically at that stop.
    let h = movement_scene_with_wall().await;
    let corner = h.adj;
    let beyond_wall = h.adj2;
    let res = h
        .room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path: vec![h.start, corner, beyond_wall],
                ts: now_millis(),
                request_id: Uuid::from_u128(0xF00D_0015),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        res.stop, corner,
        "executor must stop at the last clear cell"
    );
    assert_eq!(
        h.committed_pos(h.token_id).await,
        corner,
        "committed position must equal the truncation stop"
    );
}

/// `movement_scene_with_wall`'s geometry, but the wall crosses the FIRST step
/// (`start` → `adj`) instead of the second, so the very first dense sample of the walk is
/// blocked and `execute_move` reaches its zero-progress branch (`outcome.stop == start`).
/// Mirrors `move_exec::tests::scene_with_wall_across_the_path`'s wall placement at the
/// `SceneEcs` level, one layer up through `Room::execute_move`.
async fn movement_scene_with_wall_across_the_first_step() -> MovementHandle {
    use crate::data::document::DocRole;
    use serde_json::json;

    let (repo, world_id, gm) = repo_with_world().await;
    let p = repo
        .create_user("player_wall0", None, crate::auth::role::ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world_id, p, WorldRole::Player)
        .await
        .unwrap();
    let player = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };

    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let (scene_id, token_id, ws_id, wall_id) = (
        Uuid::from_u128(0xFA10_0001),
        Uuid::from_u128(0xFA10_0002),
        Uuid::from_u128(0xFA10_0003),
        Uuid::from_u128(0xFA10_0004),
    );

    // Unrestricted: only the wall gate applies.
    let mut ws = wdoc(world_id, ws_id, "world-settings");
    ws.owner = Some(gm.user_id);
    ws.system = json!({
        "scene": {
            "losRestriction": false, "fog": false,
            "lightingEnabled": false, "lightMode": "environmentLight",
            "environment": { "color": "#ffffff", "intensity": 1.0 },
            "observerVision": false,
            "movementRestriction": "unrestricted",
            "partialCellLeniency": true
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    });
    ws.engine = Some(ws_engine(ws.system.clone()));
    room.publish(
        &repo,
        &gm,
        vec![Operation::Create { doc: ws }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut scene = wdoc(world_id, scene_id, "scene");
    scene.owner = Some(gm.user_id);
    scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
    room.publish(
        &repo,
        &gm,
        vec![Operation::Create { doc: scene }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut token = wdoc(world_id, token_id, "token");
    token.parent_id = Some(scene_id);
    token.owner = Some(p);
    token.permissions.users.insert(p, DocRole::Owner);
    token.engine = Some(token_engine(50.0, 50.0));
    room.publish(
        &repo,
        &gm,
        vec![Operation::Create { doc: token }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // Vertical wall at x=100, y ∈ [0,100]: crosses the FIRST step (50,50)->(150,50).
    let mut wall = wdoc(world_id, wall_id, "wall");
    wall.parent_id = Some(scene_id);
    wall.owner = Some(gm.user_id);
    wall.engine =
        Some(json!({ "seg": { "x1": 100, "y1": 0, "x2": 100, "y2": 100 }, "blocksMove": true }));
    room.publish(
        &repo,
        &gm,
        vec![Operation::Create { doc: wall }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    MovementHandle {
        room,
        repo,
        gm,
        player,
        world_id,
        scene_id,
        token_id,
        start: (50.0, 50.0),
        lit_goal: (150.0, 50.0),
        adj: (150.0, 50.0),
        adj2: (150.0, 150.0),
    }
}

#[tokio::test]
async fn execute_move_returns_a_zero_progress_frame_when_the_first_step_is_blocked() {
    let h = movement_scene_with_wall_across_the_first_step().await;
    let now = now_millis();
    let exec = h
        .room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path: vec![h.start, h.adj],
                ts: now,
                request_id: Uuid::from_u128(0xF00D_0021),
            },
        )
        .await
        .unwrap();
    assert_eq!(exec.stop, h.start, "the very first step was blocked");
    let ServerMsg::MoveStream { duration_ms, .. } = exec.frame.as_ref() else {
        panic!("frame must be a MoveStream");
    };
    assert_eq!(*duration_ms, 0.0);
    assert!(
        h.room.scene_streams(h.scene_id, now).await.is_empty(),
        "a zero-progress move is never registered in the in-flight registry"
    );
}

#[tokio::test]
async fn execute_move_registers_the_full_frame_and_accessors_filter_by_mover_scene_and_expiry() {
    // Slow speed so the stream stays unexpired for the duration of the assertions.
    let h = movement_scene_with_speed("unrestricted", false, 0.5).await;
    let req = Uuid::from_u128(0x5EED);
    let exec = h
        .room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path: vec![h.start, h.adj],
                ts: now_millis(),
                request_id: req,
            },
        )
        .await
        .unwrap();
    let ServerMsg::MoveStream {
        request_id,
        token_id,
        mover,
        scene,
        mover_vision,
        cost,
        ..
    } = exec.frame.as_ref()
    else {
        panic!("frame must be a MoveStream");
    };
    assert_eq!(*request_id, req);
    assert_eq!(*token_id, h.token_id);
    assert_eq!(*mover, h.player.user_id);
    assert_eq!(*scene, h.scene_id);
    assert!(
        cost.is_some(),
        "the registered frame is the full in-process frame"
    );
    // A player mover on a non-GM path carries a vision timeline (None only for GM movers).
    assert!(mover_vision.is_some());

    let now = now_millis();
    let in_scene = h.room.scene_streams(h.scene_id, now).await;
    assert_eq!(in_scene.len(), 1);
    assert_eq!(in_scene[0].0, h.token_id, "keyed by the moving token");
    assert!(Arc::ptr_eq(&in_scene[0].1, &exec.frame));
    assert!(h
        .room
        .scene_streams(Uuid::from_u128(0xBAD), now)
        .await
        .is_empty());
    // concurrent_streams excludes every stream of the named MOVER, not just one token.
    assert!(h
        .room
        .concurrent_streams(h.scene_id, h.player.user_id, now)
        .await
        .is_empty());
    assert_eq!(
        h.room
            .concurrent_streams(h.scene_id, h.gm.user_id, now)
            .await
            .len(),
        1
    );
    // A different scene id has no registered stream at all.
    assert!(h
        .room
        .concurrent_streams(Uuid::from_u128(0xBAD), h.gm.user_id, now)
        .await
        .is_empty());
    // Expiry: a `now` past end_ms hides it.
    assert!(h
        .room
        .scene_streams(h.scene_id, now + 3_600_000)
        .await
        .is_empty());
}

#[tokio::test]
async fn execute_move_authoritative_field_arrests_a_region_the_players_route_never_saw() {
    use crate::data::document::Visibility;
    use serde_json::json;

    let (repo, world_id, gm) = repo_with_world().await;
    let p = repo
        .create_user("p", None, ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world_id, p, WorldRole::Player)
        .await
        .unwrap();
    let player = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };

    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    let region_id = Uuid::from_u128(12);
    let ws_id = Uuid::from_u128(13);

    let mut ws = wdoc(world_id, ws_id, "world-settings");
    ws.owner = Some(gm.user_id);
    ws.system = json!({
        "scene": {
            "losRestriction": true, "fog": true,
            "lightingEnabled": false, "lightMode": "environmentLight",
            "environment": { "color": "#000000", "intensity": 0.0 },
            "observerVision": false,
            "movementRestriction": "unrestricted",
            "partialCellLeniency": true,
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" },
    });
    ws.engine = Some(ws_engine(ws.system.clone()));
    room.publish(
        &repo,
        &gm,
        vec![Operation::Create { doc: ws }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut scene = wdoc(world_id, scene_id, "scene");
    scene.owner = Some(gm.user_id);
    scene.system = json!({ "grid": { "size": 100 } });
    room.publish(
        &repo,
        &gm,
        vec![Operation::Create { doc: scene }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut token = wdoc(world_id, token_id, "token");
    token.parent_id = Some(scene_id);
    token.owner = Some(player.user_id);
    token.engine = Some(json!({ "x": 0.0, "y": 0.0, "w": 100, "h": 100, "rotation": 0 }));
    room.publish(
        &repo,
        &gm,
        vec![Operation::Create { doc: token }],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut region = wdoc(world_id, region_id, "region");
    region.parent_id = Some(scene_id);
    region.owner = Some(gm.user_id);
    region.engine = Some(json!({
        "shape": { "kind": "rect", "points": [50.0, 0.0, 150.0, 100.0] },
        "behavior": "impassable", "cost": 1.0, "enabled": true,
    }));
    region
        .permissions
        .property_overrides
        .insert("/engine".into(), Visibility::GmOnly);
    room.publish(
        &repo,
        &gm,
        vec![Operation::Create { doc: region }],
        3,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // The player's own pathfind field never sees this secret region — the route request
    // itself is out of scope here (the router is covered elsewhere); this test proves
    // execute_move enforces it regardless.
    let exec = room
        .execute_move(
            &repo,
            &player,
            crate::ws::room::MoveRequestInputs {
                scene_id,
                token: token_id,
                path: vec![(0.0, 0.0), (100.0, 0.0)],
                ts: 100,
                request_id: Uuid::from_u128(0xF00D_0016),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        exec.stop,
        (0.0, 0.0),
        "authoritative field blocks the secret impassable region"
    );
}

#[tokio::test]
async fn execute_move_revealed_union_allows_explored_cell() {
    // Guards the Revealed-union contract: visible_cells ∪ explored must be passed to
    // the pure executor, not visible_cells alone. A cell that is explored-but-not-
    // currently-visible must be reachable under Revealed restriction.
    //
    // "revealed" scene, light at (50,50) radius 1.5 cells. Target (550,550) = cell (5,5)
    // is outside the light radius (not in visible_cells). The explored set is seeded to
    // cover cells (0,0)–(5,5) so visible ∪ explored includes the entire path.
    let h = movement_scene("revealed", /*with_light=*/ true).await;

    let mut seed = crate::scene::explored::ExploredSet::new();
    seed.mark_cells((0..6).flat_map(|i| (0..6).map(move |j| (i, j))));
    h.repo
        .set_explored(
            h.world_id,
            h.scene_id,
            h.player.user_id,
            &seed.to_bytes(crate::scene::GridKind::Square),
        )
        .await
        .unwrap();

    // Diagonal king-steps from (50,50) to (550,550) — 5 steps, all in the explored zone.
    let path: Vec<(f64, f64)> = (0..=5)
        .map(|i| (50.0 + i as f64 * 100.0, 50.0 + i as f64 * 100.0))
        .collect();

    let res = h
        .room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path: path.clone(),
                ts: now_millis(),
                request_id: Uuid::from_u128(0xF00D_0017),
            },
        )
        .await
        .unwrap();

    // If the union was correctly applied the token reaches the explored-but-dark goal.
    assert_eq!(
        res.stop,
        *path.last().unwrap(),
        "revealed union must allow move into explored-but-not-visible cell"
    );
    assert_eq!(h.committed_pos(h.token_id).await, res.stop);
}

/// Identical to `movement_scene`, but the scene doc's `engine.vision.movementModel` is
/// explicitly `"continuous"`: proves `execute_move` gates an any-angle route
/// from a scene genuinely marked continuous, not just incidentally sent a diagonal path.
/// Functionally inert on the server today — `execute_move` has no `movementModel` branch,
/// being engine-agnostic; this mirrors `movement_scene`'s body (the established
/// per-scenario-helper convention) with one added JSON key.
async fn movement_scene_continuous(restriction: &str, with_light: bool) -> MovementHandle {
    use serde_json::json;

    let (repo, world_id, gm) = repo_with_world().await;
    let p = repo
        .create_user(
            "player_continuous",
            None,
            crate::auth::role::ServerRole::User,
            0,
        )
        .await
        .unwrap();
    repo.add_member(world_id, p, WorldRole::Player)
        .await
        .unwrap();
    let player = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };

    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let (scene_id, token_id, ws_id, light_id) = (
        Uuid::from_u128(0xC047_0000),
        Uuid::from_u128(0xC047_0001),
        Uuid::from_u128(0xC047_0002),
        Uuid::from_u128(0xC047_0003),
    );

    let mut ws = wdoc(world_id, ws_id, "world-settings");
    ws.owner = Some(gm.user_id);
    ws.system = json!({
        "scene": {
            "losRestriction": true, "fog": true,
            "lightingEnabled": true, "lightMode": "environmentLight",
            "environment": { "color": "#000000", "intensity": 0.0 },
            "observerVision": false,
            "movementRestriction": restriction,
            "partialCellLeniency": true
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    });
    ws.engine = Some(ws_engine(ws.system.clone()));
    room.publish(
        &repo,
        &gm,
        vec![Operation::Create { doc: ws }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // Only structural difference from `movement_scene`: declares `vision.movementModel` on
    // the scene doc. Inert server-side today — execute_move has no movementModel branch.
    let mut scene = wdoc(world_id, scene_id, "scene");
    scene.owner = Some(gm.user_id);
    scene.system = json!({
        "grid": { "kind": "square", "size": 100 },
        "vision": { "movementModel": "continuous" }
    });
    scene.engine = Some(scene.system.clone());
    room.publish(
        &repo,
        &gm,
        vec![Operation::Create { doc: scene }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut token = wdoc(world_id, token_id, "token");
    token.parent_id = Some(scene_id);
    token.owner = Some(p);
    // Required for the player to have write permission on the token's /engine/x,y fields
    // at commit time (mirrors every sibling helper — movement_scene et al.); `owner` alone
    // does not grant the per-doc write permission apply_intent checks.
    token
        .permissions
        .users
        .insert(p, crate::data::document::DocRole::Owner);
    token.engine = Some(token_engine(50.0, 50.0));
    room.publish(
        &repo,
        &gm,
        vec![Operation::Create { doc: token }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    if with_light {
        // Bright boundary = 1.5 * 100 = 150 world units; dim boundary = 3.0 * 100 = 300.
        let mut light = wdoc(world_id, light_id, "light");
        light.parent_id = Some(scene_id);
        light.owner = Some(gm.user_id);
        light.system = json!({
            "x": 50.0, "y": 50.0, "emission": { "color": "#ffffff", "intensity": 1.0, "brightRadius": 1.5, "dimRadius": 3.0, "enabled": true }
        });
        light.engine = Some(light.system.clone());
        room.publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: light }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    }

    MovementHandle {
        room,
        repo,
        gm,
        player,
        world_id,
        scene_id,
        token_id,
        start: (50.0, 50.0),
        lit_goal: (50.0, 150.0),
        adj: (150.0, 50.0),
        adj2: (250.0, 50.0),
    }
}

#[tokio::test]
async fn execute_move_continuous_any_angle_route_commits_atomically() {
    // Proves the unified sampled executor gates a genuinely any-angle
    // (non-grid-aligned) polyline exactly like a grid path — no movementModel branch
    // anywhere on this path. Goal (110,130) is a 3-4-5 triangle scaled ×20
    // from start (50,50): distance = sqrt(60²+80²) = 100 wu, safely inside the light's
    // 150 wu bright radius (50 wu margin) and not a grid cell-center (cell centers sit
    // at 50 + 100k on each axis).
    let h = movement_scene_continuous("visible", /*with_light=*/ true).await;
    let goal = (110.0, 130.0);
    let res = h
        .room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path: vec![h.start, goal],
                ts: now_millis(),
                request_id: Uuid::from_u128(0xF00D_0018),
            },
        )
        .await
        .unwrap();
    assert_eq!(res.stop, goal, "any-angle move commits at the exact goal");
    assert_eq!(h.committed_pos(h.token_id).await, res.stop);
}

#[tokio::test]
async fn execute_move_continuous_truncates_before_entering_unseen_space() {
    // `execute_move`'s per-cell gate TRUNCATES a route at the last visible sample rather
    // than rejecting the whole request outright (`DataError::Forbidden` is reserved for
    // structural failures — unknown token / TooLong / Degenerate — and the moving-lock
    // check; a genuine cell-gate stop is `Ok` with a partial `stop`, exactly like the
    // sibling wall-truncation test `execute_move_truncates_at_a_wall_atomically`). This
    // proves the cell-sampled gate applies to any-angle paths, not just grid ones, and
    // still commits atomically at the truncation point rather than silently reaching a
    // goal in unseen territory.
    //
    // Goal (650,850) is a 3-4-5 triangle scaled ×200 from start (50,50): distance =
    // sqrt(600²+800²) = 1000 wu. `gate_walk` subdivides this into 8 dense ≤1-cell samples
    // (cheby = max(600,800) = 800 wu ⇒ k = ceil(800/100) = 8), at (50+75k, 50+100k) for
    // k=0..8. What decides a sample's cell is the light's own dim radius (3.0 cells = 300 wu
    // at this scene's cell size 100), not a fixed scan-box margin — a lamp's occlusion
    // polygon grows to cover its authored reach, so the reach itself, not
    // `VISION_BOUND_MARGIN`, is what a cell's distance from the colocated token/light
    // viewpoint (50,50) is checked against. Sample 2, (200,250), lands in cell (2,2)
    // (center (250,250), 282.8 wu from the lamp) — inside the 300wu dim radius, lit. Sample
    // 3, (275,350), lands in cell (2,3) (center (250,350), 360.6 wu from the lamp) — past the
    // dim radius, dark — so the walk truncates entering that cell, leaving the token at
    // sample 2's exact position.
    let h = movement_scene_continuous("visible", /*with_light=*/ true).await;
    let goal = (650.0, 850.0);
    let res = h
        .room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path: vec![h.start, goal],
                ts: now_millis(),
                request_id: Uuid::from_u128(0xF00D_0019),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        res.stop,
        (200.0, 250.0),
        "cell-gate truncates the route at the last visible sample, short of the goal"
    );
    assert_ne!(
        res.stop, goal,
        "must not silently reach a goal in unseen space"
    );
    assert_eq!(h.committed_pos(h.token_id).await, res.stop);
}

/// Hex OUTER radius (circumradius) `hex_move_scene`'s scene declares, in scene units. One grid
/// step on a pointy-top hex is `√3` times this, which is what makes the two scalars distinct.
const HEX_MOVE_SIZE: f64 = 50.0;
/// Animation speed `hex_move_scene`'s world authors, in GRID CELLS per second.
const HEX_MOVE_SPEED_CELLS_PER_SEC: f64 = 6.0;

/// A wall-less pointy-top hex scene at `HEX_MOVE_SIZE` with movement unrestricted and lighting
/// off — so neither the visibility mask nor a light gates the step and the returned duration is
/// the only thing a move can be measured by — one player-owned token at hex (0,0) = pixel
/// (0,0), and the world's animation speed at `HEX_MOVE_SPEED_CELLS_PER_SEC`.
///
/// `start`/`lit_goal`/`adj`/`adj2` are pixel coordinates the caller derives from the resolved
/// shape, so they are filled with the token's own start and left for the test to replace;
/// nothing in this fixture's own tests reads the goal fields.
async fn hex_move_scene() -> MovementHandle {
    use serde_json::json;

    let (repo, world_id, gm) = repo_with_world().await;
    let p = repo
        .create_user("player_hex", None, crate::auth::role::ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world_id, p, WorldRole::Player)
        .await
        .unwrap();
    let player = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };

    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let (scene_id, token_id, ws_id) = (
        Uuid::from_u128(0x4E60_0000),
        Uuid::from_u128(0x4E60_0001),
        Uuid::from_u128(0x4E60_0002),
    );

    let mut ws = wdoc(world_id, ws_id, "world-settings");
    ws.owner = Some(gm.user_id);
    ws.system = json!({
        "scene": {
            "losRestriction": false, "fog": true,
            "lightingEnabled": false, "lightMode": "environmentLight",
            "environment": { "color": "#000000", "intensity": 0.0 },
            "observerVision": false,
            "movementRestriction": "unrestricted",
            "partialCellLeniency": true
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": HEX_MOVE_SPEED_CELLS_PER_SEC,
                       "easing": "easeInOut" }
    });
    ws.engine = Some(ws_engine(ws.system.clone()));
    room.publish(
        &repo,
        &gm,
        vec![Operation::Create { doc: ws }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut scene = wdoc(world_id, scene_id, "scene");
    scene.owner = Some(gm.user_id);
    scene.system = json!({ "grid": { "kind": "hex", "size": HEX_MOVE_SIZE } });
    scene.engine = Some(json!({
        "grid": { "kind": "hex", "size": HEX_MOVE_SIZE },
        "background": null
    }));
    room.publish(
        &repo,
        &gm,
        vec![Operation::Create { doc: scene }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut token = wdoc(world_id, token_id, "token");
    token.parent_id = Some(scene_id);
    token.owner = Some(p);
    token
        .permissions
        .users
        .insert(p, crate::data::document::DocRole::Owner);
    token.engine = Some(token_engine(0.0, 0.0));
    room.publish(
        &repo,
        &gm,
        vec![Operation::Create { doc: token }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    MovementHandle {
        room,
        repo,
        gm,
        player,
        world_id,
        scene_id,
        token_id,
        start: (0.0, 0.0),
        lit_goal: (0.0, 0.0),
        adj: (0.0, 0.0),
        adj2: (0.0, 0.0),
    }
}

#[tokio::test]
async fn a_hex_move_animates_at_the_grid_step_rate() {
    // Animation speed is authored in cells per second, so one grid step at six cells per
    // second lasts 1000/6 ms whatever the grid kind. On a pointy-top hex a step is √3·size
    // scene units, not `size`.
    //
    // Discrimination: dividing the travelled distance by the indexing scale reports
    // (√3·size/size)/6·1000 ≈ 288.7 ms for the same step, which the 1 ms tolerance rejects by
    // two orders of magnitude. The expectation is derived from the authored SPEED and the
    // step count, never from the distance the executor returns; the destination is derived
    // from the scene's own resolved shape so the move really is one axial step.
    let h = hex_move_scene().await;
    let dest = {
        let scene = h.room.scene().read().await;
        scene
            .resolve_grid_shape(h.scene_id, HEX_MOVE_SIZE)
            .cell_center((1, 0))
    };
    let out = h
        .room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path: vec![h.start, dest],
                ts: now_millis(),
                request_id: Uuid::from_u128(0xF00D_0020),
            },
        )
        .await
        .unwrap();
    assert_eq!(out.stop, dest, "the single axial step completes");
    let expected_ms = 1000.0 / HEX_MOVE_SPEED_CELLS_PER_SEC;
    assert!(
        (out.duration_ms - expected_ms).abs() < 1.0,
        "one grid step at {HEX_MOVE_SPEED_CELLS_PER_SEC} cells per second lasts {expected_ms} ms, got {}",
        out.duration_ms
    );
}

#[tokio::test]
async fn resync_floor_is_established_seq_plus_one() {
    let (repo, world_id, ctx) = repo_with_world().await;
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();

    room.establish_resync_floor(ctx.user_id).await;
    assert_eq!(
        room.resync_floor(ctx.user_id).await,
        room.current_seq() + 1,
        "floor established at seq 0 permits resync starting at seq 1"
    );
}

/// Discriminating test for the whole feature: a user who never called
/// `establish_resync_floor` fails closed to `current_seq() + 1` (empty resync), NOT to
/// `1` (today's effectively-unbounded reach). Verified by hand: temporarily changing
/// `resync_floor`'s `None` branch to `1` makes this assertion fail (`1 != current_seq()
/// + 1` once at least one event has committed), confirming the test actually catches a
/// regression back to the unbounded default rather than passing vacuously.
#[tokio::test]
async fn resync_floor_fails_closed_for_a_user_who_never_established_one() {
    let (repo, world_id, ctx) = repo_with_world().await;
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();

    // Advance current_seq so `1` (the old unbounded default) and
    // `current_seq() + 1` (the fail-closed default) are distinguishable.
    let mut scene =
        crate::data::document::tests::world_scoped_doc(world_id, Uuid::from_u128(30), "scene");
    scene.owner = Some(ctx.user_id);
    room.publish(
        &repo,
        &ctx,
        vec![Operation::Create { doc: scene }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    assert!(room.current_seq() > 0, "precondition: seq has advanced");

    let stranger = Uuid::new_v4();
    assert_eq!(
        room.resync_floor(stranger).await,
        room.current_seq() + 1,
        "no floor recorded ⇒ fail-closed to current_seq()+1, not the unbounded default of 1"
    );
}

#[tokio::test]
async fn resync_floor_only_ever_advances_across_repeated_cold_starts() {
    let (repo, world_id, ctx) = repo_with_world().await;
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();

    // First cold start at seq 0.
    room.establish_resync_floor(ctx.user_id).await;
    let first_floor = room.resync_floor(ctx.user_id).await;

    // Advance current_seq, then a second cold start (e.g. a reload / a second tab).
    let mut scene =
        crate::data::document::tests::world_scoped_doc(world_id, Uuid::from_u128(31), "scene");
    scene.owner = Some(ctx.user_id);
    room.publish(
        &repo,
        &ctx,
        vec![Operation::Create { doc: scene }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    room.establish_resync_floor(ctx.user_id).await;
    let second_floor = room.resync_floor(ctx.user_id).await;

    assert!(
        second_floor > first_floor,
        "a later cold start moves the floor forward, never backward: {first_floor} -> {second_floor}"
    );
}

#[tokio::test]
async fn resync_floors_are_independent_per_user() {
    let (repo, world_id, ctx) = repo_with_world().await;
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();

    let other_user = Uuid::new_v4();
    room.establish_resync_floor(ctx.user_id).await;

    // The other user never established a floor: still fails closed, unaffected by the
    // first user's established floor.
    assert_eq!(
        room.resync_floor(other_user).await,
        room.current_seq() + 1,
        "establishing one user's floor must not affect another user's"
    );
}

mod movement_budget;

/// `MoveStream.mover_light` computation: presence, sampling, suppression.
mod mover_light;
