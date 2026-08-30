//! `combat::handle_combat_intent`'s wire-dispatch layer: authz per variant, dice-context
//! resolution for `CombatRoll`, and the one-server-authored-command commit via
//! `Room::commit_combat`. Fixture pattern mirrors
//! `handle_move_request_broadcasts_move_stream_no_etx_on_success`'s harness construction.

use super::*;
use crate::auth::role::ServerRole;
use crate::combat::handle_combat_intent;
use crate::data::document::{DocRole, WorldRole};
use crate::data::engine::combat::{
    CombatEngine, CombatantEngine, CombatantKind, CombatantResource, EffectLifecycleDefaults,
    Enforcement, Formula, Interpretation, MovementRules, Recovery, Resource, ResourceBinding,
    ResourceRegistryEngine, TurnControl,
};
use crate::data::membership::PermissionContext;
use crate::ws::protocol::{CombatRollEntry, ResourceOp};
use crate::ws::room::{Room, RoomRegistry};

/// A GM + player, a scene, a player-owned actor-linked token, a `resource-registry` defining
/// `movement`, and a combat with two combatants in order `[player_combatant, hidden_npc]`: the
/// player's own (visible) and the GM's NPC (`permissions.default: none`). Neither combatant is
/// started/active — tests that need a running combat call `CombatStart` themselves.
struct Harness {
    /// The backing repository.
    repo: Arc<SqliteRepository>,
    /// The combat's room.
    room: Arc<Room>,
    /// GM permission context.
    gm: PermissionContext,
    /// Player permission context.
    player: PermissionContext,
    /// The world the fixture lives in.
    world_id: Uuid,
    /// The combat document.
    combat: Uuid,
    /// The player's own combatant.
    player_combatant: Uuid,
    /// The GM's hidden NPC combatant.
    hidden_npc: Uuid,
    /// Cached `WorldCapDefaults`, for `filter_command`.
    world_defaults: crate::data::document::WorldCapDefaults,
    /// A fresh per-test combat-intent flood budget — every `handle_combat_intent`
    /// call in a test shares this one instance, same as a real connection's
    /// `message_rate`.
    rate: crate::ws::PingRateLimiter,
}

impl Harness {
    /// Reads the combat document's current parsed engine.
    async fn combat_engine(&self) -> CombatEngine {
        let doc = self.repo.get_document(self.combat).await.unwrap().unwrap();
        serde_json::from_value(doc.engine.unwrap()).unwrap()
    }

    /// Overwrites the combat document's whole `/engine` band with `engine`, via an ordinary
    /// GM-authored `Operation::Update` (`combat::ops::whole_engine_replace` — the same helper
    /// every real transition uses — so this reads the SAME OCC pre-image convention as
    /// production writes).
    async fn set_combat_engine(&self, engine: CombatEngine) {
        let doc = self.repo.get_document(self.combat).await.unwrap().unwrap();
        let change =
            crate::combat::ops::whole_engine_replace(&doc, serde_json::to_value(&engine).unwrap());
        self.room
            .publish(
                self.repo.as_ref(),
                &self.gm,
                vec![crate::data::command::Operation::Update {
                    doc_id: self.combat,
                    changes: vec![change],
                }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();
    }
}

/// Builds a `Harness` with an inactive combat (`round == 0`, `turn == None`).
async fn combat_harness() -> Harness {
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let gm_id = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", gm_id, 0).await.unwrap();
    let gm = PermissionContext {
        user_id: gm_id,
        world_role: WorldRole::Gm,
    };

    let player_id = repo
        .create_user("player", None, ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world.id, player_id, WorldRole::Player)
        .await
        .unwrap();
    let player = PermissionContext {
        user_id: player_id,
        world_role: WorldRole::Player,
    };

    let reg = RoomRegistry::new();
    let room = reg
        .get_or_create(repo.as_ref(), world.id)
        .await
        .unwrap()
        .unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;

    let (scene_id, actor_id, token_id, registry_id, combat_id, pc_id, npc_id, npc_actor_id) = (
        Uuid::from_u128(0xCA01),
        Uuid::from_u128(0xCA02),
        Uuid::from_u128(0xCA03),
        Uuid::from_u128(0xCA04),
        Uuid::from_u128(0xCA05),
        Uuid::from_u128(0xCA06),
        Uuid::from_u128(0xCA07),
        Uuid::from_u128(0xCA08),
    );

    // Scene the combat runs on.
    let mut scene = wdoc(world.id, scene_id, "scene");
    scene.owner = Some(gm_id);
    scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
    room.publish(
        repo.as_ref(),
        &gm,
        vec![crate::data::command::Operation::Create { doc: scene }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // Player-owned actor + token.
    let mut actor = wdoc(world.id, actor_id, "actor");
    actor.owner = Some(player_id);
    actor.engine = Some(json!({
        "displayName": "Player Fixture",
        "visual": { "kind": "image", "asset": "a.png" },
        "size": { "w": 100.0, "h": 100.0 },
        "shape": "square",
        "conditions": [],
        "prototype": false,
    }));
    room.publish(
        repo.as_ref(),
        &gm,
        vec![crate::data::command::Operation::Create { doc: actor }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut token = wdoc(world.id, token_id, "token");
    token.parent_id = Some(scene_id);
    token.owner = Some(player_id);
    token.permissions.users.insert(player_id, DocRole::Owner);
    token.engine = Some(token_engine(50.0, 50.0));
    room.publish(
        repo.as_ref(),
        &gm,
        vec![crate::data::command::Operation::Create { doc: token }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // Resource registry: both combatants track a `movement` resource.
    let mut registry = wdoc(world.id, registry_id, "resource-registry");
    registry.owner = Some(gm_id);
    let registry_engine = ResourceRegistryEngine {
        resources: [(
            "movement".to_string(),
            Resource {
                name: "Movement".into(),
                order: 0,
                binding: ResourceBinding::Tracked {
                    max: Formula::Number(10.0),
                    recover: Recovery::default(),
                },
            },
        )]
        .into_iter()
        .collect(),
    };
    registry.engine = Some(serde_json::to_value(&registry_engine).unwrap());
    room.publish(
        repo.as_ref(),
        &gm,
        vec![crate::data::command::Operation::Create { doc: registry }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // The combat itself: inactive, order = [player_combatant, hidden_npc].
    let mut combat = wdoc(world.id, combat_id, "combat");
    combat.owner = Some(gm_id);
    let combat_engine = CombatEngine {
        scene_id,
        active: false,
        round: 0,
        turn: None,
        turn_control: TurnControl::OwnerMayEnd,
        order: vec![pc_id, npc_id],
        movement: MovementRules {
            resource: None,
            interpretation: Interpretation::PerCell,
            enforcement: Enforcement::None,
        },
        effect_cleanup: true,
        rewind_restore: true,
        forward_restore: false,
        effect_lifecycle: EffectLifecycleDefaults::default(),
    };
    combat.engine = Some(serde_json::to_value(&combat_engine).unwrap());
    room.publish(
        repo.as_ref(),
        &gm,
        vec![crate::data::command::Operation::Create { doc: combat }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // Player's own (visible) combatant.
    let mut pc = wdoc(world.id, pc_id, "combatant");
    pc.parent_id = Some(combat_id);
    pc.owner = Some(player_id);
    // Visible (not `is_hidden`) — `default: none` is what marks a combatant hidden; the
    // player's own combatant is readable by everyone, writable via `owner`.
    pc.permissions.default = DocRole::Observer;
    pc.permissions.users.insert(player_id, DocRole::Owner);
    let pc_engine = CombatantEngine {
        kind: CombatantKind::Actor {
            token_id: Some(token_id),
            actor_id: Some(actor_id),
        },
        initiative: None,
        tiebreak: 0.0,
        resources: [(
            "movement".to_string(),
            CombatantResource {
                current: 5.0,
                max: 10.0,
            },
        )]
        .into_iter()
        .collect(),
    };
    pc.engine = Some(serde_json::to_value(&pc_engine).unwrap());
    room.publish(
        repo.as_ref(),
        &gm,
        vec![crate::data::command::Operation::Create { doc: pc }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // GM's hidden NPC combatant — `permissions.default: none`; no `users` entry for the player.
    let mut npc = wdoc(world.id, npc_id, "combatant");
    npc.parent_id = Some(combat_id);
    npc.owner = Some(gm_id);
    npc.permissions.default = DocRole::None;
    let npc_engine = CombatantEngine {
        kind: CombatantKind::Actor {
            token_id: None,
            actor_id: Some(npc_actor_id),
        },
        initiative: None,
        tiebreak: 0.0,
        resources: [(
            "movement".to_string(),
            CombatantResource {
                current: 5.0,
                max: 10.0,
            },
        )]
        .into_iter()
        .collect(),
    };
    npc.engine = Some(serde_json::to_value(&npc_engine).unwrap());
    room.publish(
        repo.as_ref(),
        &gm,
        vec![crate::data::command::Operation::Create { doc: npc }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let world_defaults = repo.world_cap_defaults(world.id).await.unwrap();

    Harness {
        repo,
        room,
        gm,
        player,
        world_id: world.id,
        combat: combat_id,
        player_combatant: pc_id,
        hidden_npc: npc_id,
        world_defaults,
        rate: crate::ws::PingRateLimiter::new(),
    }
}

/// Drains `rx` until the next `RoomEvent::Event`, returning its `StoredCommand`. Skips any
/// `RoomEvent::Other` (out-of-band aux frames) in between.
async fn next_event(
    rx: &mut tokio::sync::broadcast::Receiver<crate::ws::room::RoomEvent>,
) -> crate::data::snapshot::StoredCommand {
    loop {
        match rx.recv().await.unwrap() {
            crate::ws::room::RoomEvent::Event(ev) => return (*ev).clone(),
            crate::ws::room::RoomEvent::Other(_) => continue,
        }
    }
}

/// `load_current_docs` against `h`'s repository, for a `filter_command` call under test.
async fn current_docs(
    h: &Harness,
    cmd: &crate::data::command::Command,
) -> std::collections::HashMap<Uuid, crate::data::permission::CurrentDoc> {
    crate::data::permission::load_current_docs(h.repo.as_ref(), cmd).await
}

/// Full GM round trip (`CombatStart` → `CombatAdvance` → `CombatPause` → `CombatEnd`), each call
/// confirmed by `None` (the broadcast `Event` is the notification). `CombatStart` creates a
/// GM-only `combat-history` record; a player's filtered view of that same broadcast carries no
/// `combat-history` op, the first WS-layer exercise of the document-level secrecy the combat
/// history engine enforces. `CombatEnd` deletes the combat; its children cascade.
#[tokio::test]
async fn gm_start_advance_pause_end_round_trip_and_players_get_no_history() {
    let h = combat_harness().await;
    let (mut gm_rx, _) = h.room.subscribe();

    assert!(handle_combat_intent(
        &h.room,
        h.repo.as_ref(),
        &h.gm,
        ClientMsg::CombatStart {
            request_id: Uuid::from_u128(1),
            combat_id: h.combat,
        },
        0,
        &h.rate,
    )
    .await
    .is_none());

    let combat = h.repo.get_document(h.combat).await.unwrap().unwrap();
    let e: CombatEngine = serde_json::from_value(combat.engine.unwrap()).unwrap();
    assert!(e.active && e.round == 1);

    let history = h
        .repo
        .query_children(h.combat)
        .await
        .unwrap()
        .into_iter()
        .find(|d| d.doc_type == "combat-history")
        .unwrap();
    assert_eq!(history.permissions.default, DocRole::None);

    // The player's filtered view of the CombatStart broadcast carries no combat-history op.
    let ev = next_event(&mut gm_rx).await;
    let current = current_docs(&h, &ev.command).await;
    let filtered = crate::data::permission::filter_command(
        &ev.command,
        &ev.snapshot,
        &h.player,
        &h.world_defaults,
        &current,
        |_| None,
    );
    assert!(!filtered.ops.iter().any(|o| matches!(
        o,
        crate::data::command::Operation::Create { doc } if doc.doc_type == "combat-history"
    )));

    assert!(handle_combat_intent(
        &h.room,
        h.repo.as_ref(),
        &h.gm,
        ClientMsg::CombatAdvance {
            request_id: Uuid::from_u128(2),
            combat_id: h.combat,
        },
        0,
        &h.rate,
    )
    .await
    .is_none());
    assert!(handle_combat_intent(
        &h.room,
        h.repo.as_ref(),
        &h.gm,
        ClientMsg::CombatPause {
            request_id: Uuid::from_u128(3),
            combat_id: h.combat,
        },
        0,
        &h.rate,
    )
    .await
    .is_none());
    assert!(handle_combat_intent(
        &h.room,
        h.repo.as_ref(),
        &h.gm,
        ClientMsg::CombatEnd {
            request_id: Uuid::from_u128(4),
            combat_id: h.combat,
        },
        0,
        &h.rate,
    )
    .await
    .is_none());
    assert!(h.repo.get_document(h.combat).await.unwrap().is_none());
    assert!(
        h.repo.query_children(h.combat).await.unwrap().is_empty(),
        "cascade"
    );
}

/// A non-GM may `CombatAdvance` only their own current turn under
/// `TurnControl::OwnerMayEnd`; once `GmOnly` holds a hidden turn, the same player is refused with
/// the SAME wording an unknown `combat_id` produces — the information-leak-prevention property
/// `CombatError`'s `Display` exists for.
#[tokio::test]
async fn owner_may_end_only_their_own_turn_and_errors_share_one_wording() {
    let h = combat_harness().await; // order: [player's combatant, hidden NPC]
    handle_combat_intent(
        &h.room,
        h.repo.as_ref(),
        &h.gm,
        ClientMsg::CombatStart {
            request_id: Uuid::nil(),
            combat_id: h.combat,
        },
        0,
        &h.rate,
    )
    .await;

    let ok = handle_combat_intent(
        &h.room,
        h.repo.as_ref(),
        &h.player,
        ClientMsg::CombatAdvance {
            request_id: Uuid::from_u128(7),
            combat_id: h.combat,
        },
        0,
        &h.rate,
    )
    .await;
    assert!(ok.is_none(), "own turn");

    // The hidden NPC auto-resolved (OwnerMayEnd) and it's the player's turn again; switch to
    // GmOnly and let the GM park the turn on the hidden NPC to hold it.
    let mut gm_only = h.combat_engine().await;
    gm_only.turn_control = TurnControl::GmOnly;
    h.set_combat_engine(gm_only).await;
    handle_combat_intent(
        &h.room,
        h.repo.as_ref(),
        &h.gm,
        ClientMsg::CombatAdvance {
            request_id: Uuid::nil(),
            combat_id: h.combat,
        },
        0,
        &h.rate,
    )
    .await;

    let denied = handle_combat_intent(
        &h.room,
        h.repo.as_ref(),
        &h.player,
        ClientMsg::CombatAdvance {
            request_id: Uuid::from_u128(8),
            combat_id: h.combat,
        },
        0,
        &h.rate,
    )
    .await;
    let unknown = handle_combat_intent(
        &h.room,
        h.repo.as_ref(),
        &h.player,
        ClientMsg::CombatAdvance {
            request_id: Uuid::from_u128(9),
            combat_id: Uuid::from_u128(0xDEAD),
        },
        0,
        &h.rate,
    )
    .await;
    let (
        Some(ServerMsg::CombatError { message: m1, .. }),
        Some(ServerMsg::CombatError { message: m2, .. }),
    ) = (denied, unknown)
    else {
        panic!("expected two CombatError replies");
    };
    assert_eq!(
        m1, m2,
        "hidden-turn refusal and unknown-combat refusal are indistinguishable"
    );
}

/// `CombatRoll` resolves the channel's dice context, posts a `GmOnly` message for a hidden
/// combatant, propagates a roll-cap failure as a `CombatError`, and refuses a non-owner rolling
/// for a combatant they don't own.
#[tokio::test]
async fn roll_uses_the_channel_dice_context_and_posts_a_gm_only_message_for_hidden() {
    let h = combat_harness().await;

    let ok = handle_combat_intent(
        &h.room,
        h.repo.as_ref(),
        &h.gm,
        ClientMsg::CombatRoll {
            request_id: Uuid::nil(),
            combat_id: h.combat,
            channel: "table".into(),
            rolls: vec![CombatRollEntry {
                combatant_id: h.hidden_npc,
                notation: "1d20".into(),
            }],
        },
        0,
        &h.rate,
    )
    .await;
    assert!(ok.is_none());

    let msgs = h.repo.query_documents(h.world_id, "message").await.unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].permissions.default, DocRole::None);

    let bad = handle_combat_intent(
        &h.room,
        h.repo.as_ref(),
        &h.gm,
        ClientMsg::CombatRoll {
            request_id: Uuid::nil(),
            combat_id: h.combat,
            channel: "table".into(),
            rolls: vec![CombatRollEntry {
                combatant_id: h.hidden_npc,
                notation: "101d6".into(),
            }],
        },
        0,
        &h.rate,
    )
    .await;
    assert!(
        matches!(bad, Some(ServerMsg::CombatError { .. })),
        "caps apply"
    );

    let player_on_npc = handle_combat_intent(
        &h.room,
        h.repo.as_ref(),
        &h.player,
        ClientMsg::CombatRoll {
            request_id: Uuid::nil(),
            combat_id: h.combat,
            channel: "table".into(),
            rolls: vec![CombatRollEntry {
                combatant_id: h.hidden_npc,
                notation: "1d20".into(),
            }],
        },
        0,
        &h.rate,
    )
    .await;
    assert!(matches!(player_on_npc, Some(ServerMsg::CombatError { .. })));
}

/// A non-GM sending `CombatRoll` with an EMPTY `rolls` list gets refused, not a committed write —
/// an empty list has no entry for `authorize` to check ownership against, so the loop over
/// `rolls` would otherwise vacuously succeed for ANY non-GM world member regardless of any
/// relationship to this combat, and `transition::roll` unconditionally rewrites `/engine/order`
/// even when nothing changed (unlike `sort`, which no-ops a genuine non-change).
#[tokio::test]
async fn combat_roll_with_empty_rolls_is_refused_not_a_vacuous_success() {
    let h = combat_harness().await;
    let order_before = h.combat_engine().await.order;

    let refused = handle_combat_intent(
        &h.room,
        h.repo.as_ref(),
        &h.player,
        ClientMsg::CombatRoll {
            request_id: Uuid::nil(),
            combat_id: h.combat,
            channel: "table".into(),
            rolls: vec![],
        },
        0,
        &h.rate,
    )
    .await;
    assert!(matches!(refused, Some(ServerMsg::CombatError { .. })));
    assert_eq!(
        h.combat_engine().await.order,
        order_before,
        "no write committed for an empty-rolls request"
    );
}

/// `CombatResource` admits the GM or the combatant's own owner; a player naming a combatant they
/// don't own (the hidden NPC) is refused.
#[tokio::test]
async fn resource_authz_is_gm_or_owner() {
    let h = combat_harness().await;

    let own = ClientMsg::CombatResource {
        request_id: Uuid::nil(),
        combat_id: h.combat,
        combatant_id: h.player_combatant,
        resource: "movement".into(),
        op: ResourceOp::Set { value: 3.0 },
    };
    assert!(
        handle_combat_intent(&h.room, h.repo.as_ref(), &h.player, own, 0, &h.rate)
            .await
            .is_none()
    );

    let other = ClientMsg::CombatResource {
        request_id: Uuid::nil(),
        combat_id: h.combat,
        combatant_id: h.hidden_npc,
        resource: "movement".into(),
        op: ResourceOp::Set { value: 3.0 },
    };
    assert!(matches!(
        handle_combat_intent(&h.room, h.repo.as_ref(), &h.player, other, 0, &h.rate).await,
        Some(ServerMsg::CombatError { .. })
    ));
}

/// A caller over the per-minute combat-intent flood budget is refused before any doc access —
/// exhausting the budget with cheap GM `CombatSort` calls, the NEXT call is refused with a
/// `CombatError` and never reaches the repository (the combat's `order` is untouched).
#[tokio::test]
async fn combat_dispatch_is_rate_limited() {
    let h = combat_harness().await;
    let rate = crate::ws::PingRateLimiter::new();
    // Exhausts the budget (mirrors `combat::handle_combat_intent`'s 30/min figure) with
    // pre-authorized GM calls so the LAST call below is the one under test.
    for i in 0u128..30 {
        let _ = handle_combat_intent(
            &h.room,
            h.repo.as_ref(),
            &h.gm,
            ClientMsg::CombatSort {
                request_id: Uuid::from_u128(i),
                combat_id: h.combat,
            },
            0,
            &rate,
        )
        .await;
    }
    let order_before = h.combat_engine().await.order;
    let refused = handle_combat_intent(
        &h.room,
        h.repo.as_ref(),
        &h.gm,
        ClientMsg::CombatSort {
            request_id: Uuid::nil(),
            combat_id: h.combat,
        },
        0,
        &rate,
    )
    .await;
    assert!(
        matches!(refused, Some(ServerMsg::CombatError { .. })),
        "over budget"
    );
    assert_eq!(
        h.combat_engine().await.order,
        order_before,
        "no doc access after budget refusal"
    );
}

/// Every one of the eight `Combat*` `ClientMsg` variants parses from its wire tag, round-tripping
/// through `serde_json`. This proves the WIRE TAG round-trip only — it says nothing about
/// `conn.rs`'s actual dispatch routing. Dispatch-arm coverage is guaranteed independently, by the
/// compiler: the inner `match serde_json::from_str::<ClientMsg>(...)` in `conn.rs` enumerates
/// `Ok(...)` per `ClientMsg` variant with no catch-all over the enum, so removing a `Combat*`
/// variant from the combined dispatch arm makes that match non-exhaustive and fails the build,
/// rather than silently dropping the frame.
#[test]
fn all_eight_combat_frames_round_trip_their_wire_tag() {
    for v in [
        json!({ "type": "combat_start", "request_id": Uuid::nil(), "combat_id": Uuid::nil() }),
        json!({ "type": "combat_pause", "request_id": Uuid::nil(), "combat_id": Uuid::nil() }),
        json!({ "type": "combat_end", "request_id": Uuid::nil(), "combat_id": Uuid::nil() }),
        json!({ "type": "combat_advance", "request_id": Uuid::nil(), "combat_id": Uuid::nil() }),
        json!({ "type": "combat_rewind", "request_id": Uuid::nil(), "combat_id": Uuid::nil() }),
        json!({ "type": "combat_sort", "request_id": Uuid::nil(), "combat_id": Uuid::nil() }),
        json!({
            "type": "combat_roll",
            "request_id": Uuid::nil(),
            "combat_id": Uuid::nil(),
            "channel": "table",
            "rolls": [{ "combatant_id": Uuid::nil(), "notation": "1d20" }],
        }),
        json!({
            "type": "combat_resource",
            "request_id": Uuid::nil(),
            "combat_id": Uuid::nil(),
            "combatant_id": Uuid::nil(),
            "resource": "movement",
            "op": { "kind": "set", "value": 3.0 },
        }),
    ] {
        let ty = v["type"].as_str().unwrap().to_string();
        assert!(serde_json::from_value::<ClientMsg>(v).is_ok(), "{ty}");
    }
}

/// A concurrent write to a combat racing `Room::commit_combat` surfaces as a clean
/// `DataError::Conflict`, never a lost update: two `CombatResource` ops are built from the SAME
/// snapshot (simulating two clients racing off one stale read); the first commit succeeds, and the
/// second — still carrying the pre-race OCC pre-image — is refused rather than silently
/// overwriting the first commit's write.
#[tokio::test]
async fn concurrent_combat_resource_writes_produce_a_clean_conflict_not_a_lost_update() {
    let h = combat_harness().await;

    let snap = crate::combat::load_snapshot(h.repo.as_ref(), h.world_id, h.combat)
        .await
        .unwrap();
    let ops_a = crate::combat::resource(
        &snap,
        h.player_combatant,
        "movement",
        crate::combat::ResourceOp::Set { value: 1.0 },
    )
    .unwrap();
    let ops_b = crate::combat::resource(
        &snap,
        h.player_combatant,
        "movement",
        crate::combat::ResourceOp::Set { value: 2.0 },
    )
    .unwrap();

    h.room
        .commit_combat(h.repo.as_ref(), &h.player, ops_a, 0)
        .await
        .expect("first commit off the shared snapshot succeeds");

    let second = h
        .room
        .commit_combat(h.repo.as_ref(), &h.player, ops_b, 0)
        .await;
    assert!(
        matches!(second, Err(crate::data::DataError::Conflict(_))),
        "a second write off the SAME stale snapshot must conflict cleanly, not silently overwrite \
         the first: got {second:?}"
    );

    // The first write's value survived; the second never applied.
    let combatant = h
        .repo
        .get_document(h.player_combatant)
        .await
        .unwrap()
        .unwrap();
    let engine: CombatantEngine = serde_json::from_value(combatant.engine.unwrap()).unwrap();
    assert_eq!(engine.resources["movement"].current, 1.0);
}

/// `CombatRewind` end to end through the WS handler — the only intent whose ops include an
/// `Operation::Create` under `WriteOrigin::CombatTransition` (the server re-`Create`ing a
/// combatant an earlier auto-resolution deleted). Also exercises the per-boundary history
/// records: the walk crosses the event's own boundary, so a rewind can land on it.
#[tokio::test]
async fn gm_rewind_recreates_a_deleted_event_combatant_and_rebuilds_the_order() {
    let h = combat_harness().await;
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let event_id = Uuid::from_u128(0xCA10);

    // A one-shot Event between the player's combatant and the hidden NPC: it resolves and
    // deletes itself on the advance that reaches it.
    let mut ev = wdoc(h.world_id, event_id, "combatant");
    ev.parent_id = Some(h.combat);
    ev.owner = Some(h.gm.user_id);
    ev.name = Some("Lair action".to_string());
    ev.permissions.default = DocRole::Observer;
    ev.engine = Some(
        serde_json::to_value(&CombatantEngine {
            kind: CombatantKind::Event {
                lifespan: Some(1),
                message: None,
            },
            initiative: None,
            tiebreak: 0.0,
            resources: Default::default(),
        })
        .unwrap(),
    );
    h.room
        .publish(
            h.repo.as_ref(),
            &h.gm,
            vec![crate::data::command::Operation::Create { doc: ev }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    let mut engine = h.combat_engine().await;
    engine.order = vec![h.player_combatant, event_id, h.hidden_npc];
    h.set_combat_engine(engine).await;

    for (i, msg) in [
        ClientMsg::CombatStart {
            request_id: Uuid::from_u128(1),
            combat_id: h.combat,
        },
        ClientMsg::CombatAdvance {
            request_id: Uuid::from_u128(2),
            combat_id: h.combat,
        },
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            handle_combat_intent(&h.room, h.repo.as_ref(), &h.gm, msg, 0, &h.rate)
                .await
                .is_none(),
            "step {i} refused"
        );
    }
    assert!(
        h.repo.get_document(event_id).await.unwrap().is_none(),
        "the advance resolved and deleted the one-shot event"
    );

    // Two rewinds: the first lands on the hidden NPC's own boundary, the second on the event's
    // — the one captured while it was still alive, so restoring it re-`Create`s the document.
    for i in 0..2u32 {
        assert!(
            handle_combat_intent(
                &h.room,
                h.repo.as_ref(),
                &h.gm,
                ClientMsg::CombatRewind {
                    request_id: Uuid::from_u128(10 + u128::from(i)),
                    combat_id: h.combat,
                },
                0,
                &h.rate,
            )
            .await
            .is_none(),
            "rewind {i} refused"
        );
    }

    let restored = h
        .repo
        .get_document(event_id)
        .await
        .unwrap()
        .expect("the rewind re-Created the deleted event combatant");
    assert_eq!(restored.doc_type, "combatant");
    assert_eq!(restored.parent_id, Some(h.combat));
    assert_eq!(restored.name.as_deref(), Some("Lair action"));
    let e: CombatantEngine = serde_json::from_value(restored.engine.unwrap()).unwrap();
    assert!(matches!(
        e.kind,
        CombatantKind::Event {
            lifespan: Some(1),
            ..
        }
    ));

    let engine = h.combat_engine().await;
    assert!(
        engine.order.contains(&event_id),
        "/engine/order was rebuilt so the re-Created event is reachable by a future walk"
    );
    assert_eq!(
        engine.turn,
        Some(event_id),
        "the clock sits on the boundary the record described"
    );
}

/// `CombatSort` end to end through the WS handler: GM-only, and it rewrites `/engine/order` from
/// the combatants' current initiatives.
#[tokio::test]
async fn gm_sort_rewrites_the_order_from_current_initiatives_and_players_are_refused() {
    let h = combat_harness().await;
    // The hidden NPC out-rolls the player's combatant, so a correct sort must swap the pair.
    for (id, initiative) in [(h.player_combatant, 3.0f64), (h.hidden_npc, 19.0f64)] {
        let doc = h.repo.get_document(id).await.unwrap().unwrap();
        h.room
            .publish(
                h.repo.as_ref(),
                &h.gm,
                vec![crate::data::command::Operation::Update {
                    doc_id: id,
                    changes: vec![crate::combat::ops::set_engine(
                        &doc,
                        "/engine/initiative",
                        json!(initiative),
                    )
                    .unwrap()],
                }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();
    }

    let refusal = handle_combat_intent(
        &h.room,
        h.repo.as_ref(),
        &h.player,
        ClientMsg::CombatSort {
            request_id: Uuid::from_u128(1),
            combat_id: h.combat,
        },
        0,
        &h.rate,
    )
    .await;
    assert!(
        matches!(refusal, Some(ServerMsg::CombatError { .. })),
        "CombatSort is GM-only"
    );
    assert_eq!(
        h.combat_engine().await.order,
        vec![h.player_combatant, h.hidden_npc],
        "the refused player intent left the order untouched"
    );

    assert!(handle_combat_intent(
        &h.room,
        h.repo.as_ref(),
        &h.gm,
        ClientMsg::CombatSort {
            request_id: Uuid::from_u128(2),
            combat_id: h.combat,
        },
        0,
        &h.rate,
    )
    .await
    .is_none());
    assert_eq!(
        h.combat_engine().await.order,
        vec![h.hidden_npc, h.player_combatant],
        "sorted by initiative descending"
    );
}
