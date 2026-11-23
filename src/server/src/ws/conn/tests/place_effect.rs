//! These tests check `handle_place_effect` and the module-placement seam behind it.
//!
//! The reply is `None` on success (the broadcast `Event` of the region Create is the
//! confirmation) and a correlated `PlaceEffectError` otherwise, with the role-conditioned wording
//! applied at this layer. The seam's invariant: the effect set comes from the module manifest, the
//! sender is vetted against the declaration's `who`, and a player client cannot create an
//! effectful region any other way.

use super::*;

/// The shared per-user write budget the handler spends; a fresh limiter per call so no test
/// consumes another's allowance.
fn fresh_rate() -> crate::ws::PingRateLimiter {
    crate::ws::PingRateLimiter::new()
}

/// A world with a GM, a token-owning player, a second player who owns nothing, a scene, the
/// owner's token on it, and an installed + enabled module `spells` declaring three placements:
/// `gm_zone` (who `gm`), `open_zone` (who `player`) and `cast_zone` (who `caster_owner`), each
/// a circle of at most 10 units carrying one `enter` condition trigger.
struct PlaceFixture {
    /// The repository.
    repo: std::sync::Arc<SqliteRepository>,
    /// The world's room.
    room: Arc<crate::ws::room::Room>,
    /// The GM's context.
    gm: PermissionContext,
    /// The token owner's context.
    owner: PermissionContext,
    /// A second player's context (owns no token).
    other: PermissionContext,
    /// The world.
    world_id: Uuid,
    /// The scene.
    scene_id: Uuid,
    /// The owner's token.
    token_id: Uuid,
    /// The installed modules directory (kept alive for the scan).
    dir: tempfile::TempDir,
    /// The scan cache the handler shares.
    cache: Arc<crate::modules::ModuleScanCache>,
}

/// Build a `PlaceFixture`. `enable` controls whether the world enables module `spells`.
async fn place_fixture(enable: bool) -> PlaceFixture {
    use crate::data::document::{DocRole, WorldRole};
    use crate::ws::room::RoomRegistry;

    let dir = tempfile::tempdir().unwrap();
    let module = dir.path().join("spells");
    std::fs::create_dir(&module).unwrap();
    let trigger =
        json!([{ "on": "enter", "effect": { "type": "condition_add", "condition": "burning" } }]);
    let manifest = json!({
        "id": "spells",
        "version": "1.0.0",
        "engines": { "shadowcat": format!("^{}", env!("CARGO_PKG_VERSION")) },
        "placements": [
            { "id": "gm_zone", "who": "gm", "shapes": ["circle"], "maxExtent": 10.0, "triggers": trigger },
            { "id": "open_zone", "who": "player", "shapes": ["circle", "rect"], "maxExtent": 10.0, "triggers": trigger, "label": "Open zone" },
            { "id": "cast_zone", "who": "caster_owner", "shapes": ["circle"], "maxExtent": 10.0, "triggers": trigger },
        ]
    });
    std::fs::write(module.join("module.json"), manifest.to_string()).unwrap();

    let repo = std::sync::Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let author = repo
        .create_user("gm", None, crate::auth::role::ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", author, 0).await.unwrap();
    let gm = PermissionContext {
        user_id: author,
        world_role: WorldRole::Gm,
    };
    let mut players = Vec::new();
    for name in ["owner", "other"] {
        let id = repo
            .create_user(name, None, crate::auth::role::ServerRole::User, 0)
            .await
            .unwrap();
        repo.add_member(world.id, id, WorldRole::Player)
            .await
            .unwrap();
        players.push(PermissionContext {
            user_id: id,
            world_role: WorldRole::Player,
        });
    }
    let (other, owner) = (players.pop().unwrap(), players.pop().unwrap());
    if enable {
        repo.set_world_enabled_modules(
            world.id,
            &[crate::modules::WorldModuleEntry {
                id: "spells".into(),
                validators_enabled: false,
            }],
        )
        .await
        .unwrap();
    }

    let room = RoomRegistry::new()
        .get_or_create(repo.as_ref(), world.id)
        .await
        .unwrap()
        .unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let (scene_id, token_id) = (
        crate::test_ids::test_uuid(0xE201),
        crate::test_ids::test_uuid(0xE202),
    );
    let mut scene = wdoc(world.id, scene_id, "scene");
    scene.owner = Some(author);
    scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
    let mut token = wdoc(world.id, token_id, "token");
    token.parent_id = Some(scene_id);
    token.owner = Some(owner.user_id);
    token
        .permissions
        .users
        .insert(owner.user_id, DocRole::Owner);
    token.engine = Some(token_engine(50.0, 50.0));
    room.publish(
        repo.as_ref(),
        &gm,
        vec![
            crate::data::command::Operation::Create { doc: scene },
            crate::data::command::Operation::Create { doc: token },
        ],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    PlaceFixture {
        repo,
        room,
        gm,
        owner,
        other,
        world_id: world.id,
        scene_id,
        token_id,
        dir,
        cache: Arc::new(crate::modules::ModuleScanCache::new()),
    }
}

impl PlaceFixture {
    /// Send a `PlaceEffect` for `placement` as `ctx` with the given area and caster.
    async fn place(
        &self,
        ctx: &PermissionContext,
        placement: &str,
        shape: crate::data::engine::RegionShape,
        caster: Option<Uuid>,
    ) -> Option<ServerMsg> {
        handle_place_effect(
            &self.room,
            self.repo.as_ref(),
            ctx,
            self.dir.path(),
            &self.cache,
            &fresh_rate(),
            crate::ws::room::PlaceEffectInputs {
                request_id: crate::test_ids::test_uuid(0xE210),
                module: "spells".into(),
                placement: placement.into(),
                scene_id: self.scene_id,
                shape,
                caster,
                ts: 0,
            },
        )
        .await
    }

    /// The regions currently parented to the scene.
    async fn regions(&self) -> Vec<crate::data::document::Document> {
        self.repo
            .query_children(self.scene_id)
            .await
            .unwrap()
            .into_iter()
            .filter(|d| d.doc_type == "region")
            .collect()
    }
}

/// A circle of radius `r` at the origin.
fn circle(r: f64) -> crate::data::engine::RegionShape {
    crate::data::engine::RegionShape {
        kind: "circle".into(),
        points: vec![0.0, 0.0, r],
    }
}

/// The error text of a refusal frame, panicking on any other reply.
fn refusal(res: Option<ServerMsg>) -> String {
    match res {
        Some(ServerMsg::PlaceEffectError { message, .. }) => message,
        other => panic!("expected a PlaceEffectError, got {other:?}"),
    }
}

#[tokio::test]
async fn a_caster_owner_places_the_manifest_declared_effect_set() {
    let f = place_fixture(true).await;
    let res = f
        .place(&f.owner, "cast_zone", circle(3.0), Some(f.token_id))
        .await;
    assert!(res.is_none(), "success replies nothing; got {res:?}");
    let regions = f.regions().await;
    assert_eq!(regions.len(), 1, "exactly one region is created");
    let engine = regions[0].engine.as_ref().unwrap();
    assert_eq!(
        engine["triggers"][0]["effect"]["condition"], "burning",
        "the effect set is the manifest's"
    );
    assert_eq!(engine["shape"]["points"], json!([0.0, 0.0, 3.0]));
    assert_eq!(regions[0].system["placement"]["id"], "cast_zone");
}

#[tokio::test]
async fn a_player_who_does_not_own_the_caster_is_refused_with_the_uniform_wording() {
    let f = place_fixture(true).await;
    // The other player names the owner's token; ownership, not the named id, decides.
    let res = f
        .place(&f.other, "cast_zone", circle(3.0), Some(f.token_id))
        .await;
    assert_eq!(refusal(res), "placement rejected");
    let none = f.place(&f.other, "cast_zone", circle(3.0), None).await;
    assert_eq!(refusal(none), "placement rejected");
    assert!(f.regions().await.is_empty(), "nothing was created");
}

#[tokio::test]
async fn who_gm_admits_a_gm_and_refuses_every_player() {
    let f = place_fixture(true).await;
    let res = f
        .place(&f.owner, "gm_zone", circle(3.0), Some(f.token_id))
        .await;
    assert_eq!(refusal(res), "placement rejected");
    assert!(f.regions().await.is_empty());
    assert!(f.place(&f.gm, "gm_zone", circle(3.0), None).await.is_none());
    assert_eq!(f.regions().await.len(), 1);
}

#[tokio::test]
async fn who_player_admits_any_player_without_a_caster_and_carries_the_label() {
    let f = place_fixture(true).await;
    assert!(f
        .place(&f.other, "open_zone", circle(3.0), None)
        .await
        .is_none());
    let regions = f.regions().await;
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].name.as_deref(), Some("Open zone"));
}

#[tokio::test]
async fn the_area_is_clamped_to_the_declared_kinds_and_extent() {
    let f = place_fixture(true).await;
    // Diameter 12 exceeds maxExtent 10.
    let big = f.place(&f.other, "open_zone", circle(6.0), None).await;
    assert_eq!(refusal(big), "placement rejected");
    // `cast_zone` allows circles only.
    let rect = crate::data::engine::RegionShape {
        kind: "rect".into(),
        points: vec![0.0, 0.0, 2.0, 2.0],
    };
    let wrong = f.place(&f.owner, "cast_zone", rect, Some(f.token_id)).await;
    assert_eq!(refusal(wrong), "placement rejected");
    // A malformed point list never reaches a region.
    let bad = crate::data::engine::RegionShape {
        kind: "circle".into(),
        points: vec![0.0, 0.0],
    };
    assert_eq!(
        refusal(f.place(&f.other, "open_zone", bad, None).await),
        "placement rejected"
    );
    assert!(f.regions().await.is_empty());
}

#[tokio::test]
async fn a_module_the_world_has_not_enabled_places_nothing() {
    let f = place_fixture(false).await;
    let res = f.place(&f.gm, "gm_zone", circle(3.0), None).await;
    assert_eq!(
        refusal(res),
        "no such enabled placement",
        "even a GM cannot place from a module the world has not enabled"
    );
    assert!(f.regions().await.is_empty());
}

#[tokio::test]
async fn a_gm_receives_the_specific_diagnostic() {
    let f = place_fixture(true).await;
    let res = f.place(&f.gm, "no_such_zone", circle(3.0), None).await;
    assert_eq!(refusal(res), "no such enabled placement");
}

#[tokio::test]
async fn a_player_client_alone_cannot_create_an_effectful_region() {
    let f = place_fixture(true).await;
    // The same region a placement would build, submitted as an ordinary client write.
    let decl: crate::modules::PlacementDecl = serde_json::from_value(json!({
        "id": "x", "who": "player", "shapes": ["circle"], "maxExtent": 10.0,
        "triggers": [{ "on": "enter", "effect": { "type": "condition_add", "condition": "burning" } }]
    }))
    .unwrap();
    let region = decl.region_engine(circle(3.0));
    let mut doc = crate::data::document::tests::world_scoped_doc(
        f.world_id,
        crate::test_ids::test_uuid(0xE2FF),
        "region",
    );
    doc.parent_id = Some(f.scene_id);
    doc.engine = Some(serde_json::to_value(&region).unwrap());
    let res = f
        .room
        .publish(
            f.repo.as_ref(),
            &f.owner,
            vec![crate::data::command::Operation::Create { doc }],
            0,
            WriteOrigin::Client,
        )
        .await;
    assert!(
        res.is_err(),
        "a player's own Create of a region is refused: {res:?}"
    );
    assert!(f.regions().await.is_empty());
}

#[tokio::test]
async fn the_shared_write_budget_limits_placement_requests() {
    let f = place_fixture(true).await;
    let rate = fresh_rate();
    let mut refused = 0;
    for _ in 0..(crate::ws::MESSAGE_RATE_PER_MIN + 5) {
        let res = handle_place_effect(
            &f.room,
            f.repo.as_ref(),
            &f.gm,
            f.dir.path(),
            &f.cache,
            &rate,
            crate::ws::room::PlaceEffectInputs {
                request_id: crate::test_ids::test_uuid(0xE211),
                module: "spells".into(),
                placement: "no_such_zone".into(),
                scene_id: f.scene_id,
                shape: circle(1.0),
                caster: None,
                ts: 0,
            },
        )
        .await;
        if matches!(res, Some(ServerMsg::PlaceEffectError { ref message, .. }) if message == "rate limited")
        {
            refused += 1;
        }
    }
    assert!(refused >= 1, "the budget must eventually refuse");
}
