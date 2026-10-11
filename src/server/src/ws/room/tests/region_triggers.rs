//! `Room`'s region-trigger fire sites.
//!
//! The tests cover the two sites, move execution (`Room::execute_move`'s post-commit call into
//! `fire_region_triggers`) and placement/teleport (`Room::publish`'s candidate scan into
//! `fire_placement_triggers`), the per-effect-kind application semantics, and the secrecy forcing
//! rule for regions not visible to every world member.

use super::*;
use crate::chat::{Audience, MessageEngine, MessageKind, Segment};
use crate::data::document::{DocRole, Visibility};
use serde_json::json;

/// Region engine body whose rect covers exactly `cell` of a size-100 square grid
/// (cell-center containment), carrying `triggers`.
fn trigger_region_engine(
    cell: (i32, i32),
    behavior: &str,
    triggers: serde_json::Value,
) -> serde_json::Value {
    let (x0, y0) = (cell.0 as f64 * 100.0, cell.1 as f64 * 100.0);
    json!({
        "shape": { "kind": "rect", "points": [x0, y0, x0 + 100.0, y0 + 100.0] },
        "behavior": behavior,
        "cost": 1.0,
        "enabled": true,
        "triggers": triggers,
    })
}

/// Creates a GM-owned region doc on `h`'s scene.
async fn place_region(
    h: &MovementHandle,
    id: u128,
    cell: (i32, i32),
    behavior: &str,
    triggers: serde_json::Value,
) -> Uuid {
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let region_id = crate::test_ids::test_uuid(id);
    let mut region = wdoc(h.world_id, region_id, "region");
    region.parent_id = Some(h.scene_id);
    region.owner = Some(h.gm.user_id);
    // The client envelope default: world-readable. The fixture helper default is
    // fail-closed `none`, which would force every notice GM-only.
    region.permissions.default = DocRole::Observer;
    region.engine = Some(trigger_region_engine(cell, behavior, triggers));
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: region }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    region_id
}

/// Creates an actor with the given conditions/system band and links `h`'s token to it. The
/// link Update touches no position field, so it is never a placement-trigger candidate.
async fn link_actor(
    h: &MovementHandle,
    actor_id: u128,
    conditions: Vec<&str>,
    system: serde_json::Value,
) -> Uuid {
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let actor_id = crate::test_ids::test_uuid(actor_id);
    let mut actor = wdoc(h.world_id, actor_id, "actor");
    actor.owner = Some(h.gm.user_id);
    actor.system = system;
    actor.engine = Some(json!({
        "displayName": "Goblin", "visual": { "kind": "image", "asset": "11111111-1111-1111-1111-111111111111" },
        "size": { "w": 1.0, "h": 1.0 }, "shape": "square",
        "faction": null, "conditions": conditions, "prototype": false
    }));
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: actor }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Update {
                doc_id: h.token_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/engine/actor_id".into(),
                    old: json!(null),
                    new: json!(actor_id),
                }],
            }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    actor_id
}

/// The committed `message` documents on the region channel, in commit order.
async fn region_notices(h: &MovementHandle) -> Vec<Document> {
    let mut docs = h.repo.query_documents(h.world_id, "message").await.unwrap();
    docs.retain(|d| {
        d.engine
            .as_ref()
            .and_then(|e| serde_json::from_value::<MessageEngine>(e.clone()).ok())
            .is_some_and(|m| m.channel == "region")
    });
    docs
}

/// The text body of a single-segment system notice.
fn notice_text(doc: &Document) -> String {
    let engine: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
    match &engine.content[0] {
        Segment::Text { text } => text.clone(),
        other => panic!("a region notice is one text segment, got {other:?}"),
    }
}

/// The typed engine body of a notice doc.
fn notice_engine(doc: &Document) -> MessageEngine {
    serde_json::from_value(doc.engine.clone().unwrap()).unwrap()
}

/// Moves `h`'s player token along `path`.
async fn move_token(h: &MovementHandle, path: Vec<(f64, f64)>) {
    h.room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path,
                ts: crate::ws::time::now_millis(),
                request_id: Uuid::nil(),
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn move_entering_a_trigger_region_fires_condition_and_notice_effects() {
    let h = movement_scene("unrestricted", false).await;
    // The linked actor host starts with one condition the region removes.
    let actor_id = link_actor(&h, 0x7A0, vec!["blinded"], json!({})).await;
    place_region(
        &h,
        0x7A1,
        (1, 0),
        "terrain",
        json!([
            { "on": "enter", "effect": { "type": "condition_add", "condition": "prone" } },
            { "on": "enter", "effect": { "type": "condition_remove", "condition": "blinded" } },
            { "on": "enter", "effect": { "type": "chat_notice", "text": "The mire grips you.", "audience": "public" } },
        ]),
    )
    .await;

    move_token(&h, vec![h.start, h.adj]).await;

    let actor = h.repo.get_document(actor_id).await.unwrap().unwrap();
    assert_eq!(
        actor.engine.unwrap()["conditions"],
        json!(["prone"]),
        "the add and the remove both folded onto the host's array in one write"
    );
    let notices = region_notices(&h).await;
    assert_eq!(notices.len(), 1);
    let engine = notice_engine(&notices[0]);
    assert_eq!(engine.kind, MessageKind::System);
    assert_eq!(engine.audience, Audience::Public);
    assert_eq!(notice_text(&notices[0]), "The mire grips you.");
}

#[tokio::test]
async fn reentering_a_region_does_not_duplicate_a_condition() {
    // Fast animation so the per-token moving lock expires between the awaited moves.
    let h = movement_scene_with_speed("unrestricted", false, 6000.0).await;
    let actor_id = link_actor(&h, 0x7A2, vec![], json!({})).await;
    place_region(
        &h,
        0x7A3,
        (1, 0),
        "terrain",
        json!([{ "on": "enter", "effect": { "type": "condition_add", "condition": "prone" } }]),
    )
    .await;

    move_token(&h, vec![h.start, h.adj]).await;
    move_token(&h, vec![h.adj, h.start]).await;
    move_token(&h, vec![h.start, h.adj]).await;

    let actor = h.repo.get_document(actor_id).await.unwrap().unwrap();
    assert_eq!(actor.engine.unwrap()["conditions"], json!(["prone"]));
}

#[tokio::test]
async fn arrest_effects_fire_only_when_the_walk_was_arrested_inside_the_region() {
    let h = movement_scene_with_speed("unrestricted", false, 6000.0).await;
    // Region A arrests; region B is plain terrain with an arrest-only trigger.
    place_region(
        &h,
        0x7A4,
        (1, 0),
        "arrest",
        json!([
            { "on": "enter", "effect": { "type": "chat_notice", "text": "entered-a", "audience": "public" } },
            { "on": "arrest", "effect": { "type": "chat_notice", "text": "arrested-a", "audience": "public" } },
        ]),
    )
    .await;
    place_region(
        &h,
        0x7A5,
        (0, 1),
        "terrain",
        json!([
            { "on": "enter", "effect": { "type": "chat_notice", "text": "entered-b", "audience": "public" } },
            { "on": "arrest", "effect": { "type": "chat_notice", "text": "arrested-b", "audience": "public" } },
        ]),
    )
    .await;

    // Move into the arrest region: both its enter and arrest effects fire.
    move_token(&h, vec![h.start, h.adj]).await;
    let texts: Vec<String> = region_notices(&h).await.iter().map(notice_text).collect();
    assert!(texts.contains(&"entered-a".to_string()));
    assert!(texts.contains(&"arrested-a".to_string()));

    // Walk on into the terrain region (via (150,150) so the path is axis-aligned): its enter
    // effect fires but its arrest effect does not — the walk was never arrested. Starting the
    // move inside region A does not re-fire A's enter (the token never re-enters it).
    move_token(&h, vec![h.adj, (150.0, 150.0), h.lit_goal]).await;
    let texts: Vec<String> = region_notices(&h).await.iter().map(notice_text).collect();
    assert_eq!(
        texts.iter().filter(|t| *t == "entered-a").count(),
        1,
        "region A's enter fired once, on the first move only"
    );
    assert!(texts.contains(&"entered-b".to_string()));
    assert!(
        !texts.contains(&"arrested-b".to_string()),
        "an arrest trigger on a non-arresting walk never fires"
    );
}

#[tokio::test]
async fn resource_delta_applies_tracked_amounts_against_the_entering_tokens_combatant() {
    let h = movement_scene_with_speed("unrestricted", false, 6000.0).await;
    // The actor carries the formula leaf the second region's amount reads.
    let actor_id = link_actor(&h, 0x7A6, vec![], json!({ "dmg": 4.0 })).await;

    let wdoc = crate::data::document::tests::world_scoped_doc;
    let (combat_id, combatant_id, registry_id) = (
        crate::test_ids::test_uuid(0x7A7),
        crate::test_ids::test_uuid(0x7A8),
        crate::test_ids::test_uuid(0x7A9),
    );
    let mut combat = wdoc(h.world_id, combat_id, "combat");
    combat.owner = Some(h.gm.user_id);
    combat.engine = Some(json!({
        "scene_id": h.scene_id,
        "active": true,
        "round": 1,
        "turn": combatant_id,
        "turn_control": "owner_may_end",
        "order": [combatant_id],
        "movement": { "resource": "movement", "interpretation": "spaces", "enforcement": "none" },
        "effect_cleanup": true,
        "rewind_restore": true,
        "forward_restore": false,
        "effect_lifecycle": {}
    }));
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: combat }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    let mut combatant = wdoc(h.world_id, combatant_id, "combatant");
    combatant.parent_id = Some(combat_id);
    combatant.owner = Some(h.player.user_id);
    combatant
        .permissions
        .users
        .insert(h.player.user_id, DocRole::Owner);
    combatant.engine = Some(json!({
        "kind": { "type": "actor", "token_id": h.token_id, "actor_id": actor_id },
        "initiative": null,
        "tiebreak": 0.0,
        "resources": { "hp": { "current": 10.0 } }
    }));
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: combatant }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    let mut registry = wdoc(h.world_id, registry_id, "resource-registry");
    registry.owner = Some(h.gm.user_id);
    registry.engine = Some(json!({
        "resources": {
            "hp": { "name": "HP", "order": 0,
                "binding": { "kind": "tracked", "max": 30.0,
                    "recover": { "turn_start": 0, "turn_end": 0, "round_start": 0, "round_end": 0 } } },
            "movement": { "name": "Movement", "order": 1,
                "binding": { "kind": "tracked", "max": 30.0,
                    "recover": { "turn_start": 0, "turn_end": 0, "round_start": 0, "round_end": 0 } } }
        }
    }));
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: registry }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    place_region(
        &h,
        0x7AA,
        (1, 0),
        "terrain",
        json!([{ "on": "enter", "effect": { "type": "resource_delta", "resource": "hp", "amount": -3.0 } }]),
    )
    .await;
    place_region(
        &h,
        0x7AB,
        (0, 1),
        "terrain",
        json!([{ "on": "enter", "effect": { "type": "resource_delta", "resource": "hp", "amount": "dmg" } }]),
    )
    .await;

    let hp = async |h: &MovementHandle| -> f64 {
        h.repo
            .get_document(combatant_id)
            .await
            .unwrap()
            .unwrap()
            .engine
            .unwrap()["resources"]["hp"]["current"]
            .as_f64()
            .unwrap()
    };

    move_token(&h, vec![h.start, h.adj]).await;
    assert_eq!(hp(&h).await, 7.0, "the literal amount applied");

    move_token(&h, vec![h.adj, (150.0, 150.0), h.lit_goal]).await;
    assert_eq!(
        hp(&h).await,
        11.0,
        "the formula amount evaluated against the entering token's actor host"
    );
    assert!(
        region_notices(&h).await.is_empty(),
        "successful resource effects post no failure notice"
    );
}

#[tokio::test]
async fn resource_delta_without_an_active_combat_is_a_gm_only_failure_notice() {
    let h = movement_scene("unrestricted", false).await;
    place_region(
        &h,
        0x7AC,
        (1, 0),
        "terrain",
        json!([{ "on": "enter", "effect": { "type": "resource_delta", "resource": "hp", "amount": -3.0 } }]),
    )
    .await;

    move_token(&h, vec![h.start, h.adj]).await;

    let notices = region_notices(&h).await;
    assert_eq!(
        notices.len(),
        1,
        "the skipped effect is surfaced exactly once"
    );
    let engine = notice_engine(&notices[0]);
    assert_eq!(engine.audience, Audience::GmOnly);
    assert!(
        notice_text(&notices[0]).contains("no active combat"),
        "the notice names the skip reason: {}",
        notice_text(&notices[0])
    );
}

#[tokio::test]
async fn a_secret_regions_notice_is_forced_gm_only_whatever_it_authored() {
    let h = movement_scene("unrestricted", false).await;
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let region_id = crate::test_ids::test_uuid(0x7AD);
    let mut region = wdoc(h.world_id, region_id, "region");
    region.parent_id = Some(h.scene_id);
    region.owner = Some(h.gm.user_id);
    region.engine = Some(trigger_region_engine(
        (1, 0),
        "terrain",
        json!([{ "on": "enter", "effect": { "type": "chat_notice", "text": "A trap springs.", "audience": "public" } }]),
    ));
    // World-readable at the document level (the client envelope default) — the
    // `/engine` tier override alone is what makes the region secret, so the
    // forcing below is attributable to the tier, not to whole-document denial.
    region.permissions.default = DocRole::Observer;
    region
        .permissions
        .property_overrides
        .insert("/engine".into(), Visibility::GmOnly);
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: region }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    move_token(&h, vec![h.start, h.adj]).await;

    let notices = region_notices(&h).await;
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notice_engine(&notices[0]).audience,
        Audience::GmOnly,
        "the authored public audience is forced GM-only for a region not visible to all"
    );
    assert_eq!(
        notices[0].permissions.default,
        DocRole::None,
        "no world-readable default leaks the secret region's notice"
    );
}

// --- Exit triggers (prior-tick membership) ---

#[tokio::test]
async fn a_condition_added_on_enter_is_removed_on_exit_across_two_separate_moves() {
    // Fast animation so the per-token moving lock expires between the awaited moves.
    let h = movement_scene_with_speed("unrestricted", false, 6000.0).await;
    let actor_id = link_actor(&h, 0x6A0, vec![], json!({})).await;
    place_region(
        &h,
        0x6A1,
        (1, 0),
        "terrain",
        json!([
            { "on": "enter", "effect": { "type": "condition_add", "condition": "prone" } },
            { "on": "exit", "effect": { "type": "condition_remove", "condition": "prone" } },
        ]),
    )
    .await;

    // Move 1 enters: the add fires and the token's membership records the region.
    move_token(&h, vec![h.start, h.adj]).await;
    let actor = h.repo.get_document(actor_id).await.unwrap().unwrap();
    assert_eq!(
        actor.engine.unwrap()["conditions"],
        json!(["prone"]),
        "enter adds the condition"
    );

    // Move 2 leaves: the stored membership names the region while the post-move
    // footprint no longer intersects it, so the exit fires and removes the condition —
    // the motivating scenario (`ConditionAdd` on entry gets its automatic
    // `ConditionRemove` on leaving).
    move_token(&h, vec![h.adj, h.start]).await;
    let actor = h.repo.get_document(actor_id).await.unwrap().unwrap();
    assert_eq!(
        actor.engine.unwrap()["conditions"],
        json!([]),
        "exit removes the condition across two separate moves"
    );

    // Move 3 (outside → outside) fires nothing: no stored membership, no stale exit.
    move_token(&h, vec![h.start, h.lit_goal]).await;
    let actor = h.repo.get_document(actor_id).await.unwrap().unwrap();
    assert_eq!(
        actor.engine.unwrap()["conditions"],
        json!([]),
        "a move with no membership change fires no exit"
    );
}

#[tokio::test]
async fn region_membership_is_ephemeral_and_empty_after_a_fresh_ecs_hydration() {
    let h = movement_scene("unrestricted", false).await;
    let actor_id = link_actor(&h, 0x6B0, vec![], json!({})).await;
    place_region(
        &h,
        0x6B1,
        (1, 0),
        "terrain",
        json!([
            { "on": "enter", "effect": { "type": "condition_add", "condition": "prone" } },
            { "on": "exit", "effect": { "type": "condition_remove", "condition": "prone" } },
        ]),
    )
    .await;
    move_token(&h, vec![h.start, h.adj]).await;
    // Positive control (a green gate must not be inert): the live room's ECS recorded
    // the membership, or the emptiness assertion below proves nothing.
    assert!(
        h.room
            .scene()
            .read()
            .await
            .region_membership_of(h.token_id)
            .is_some(),
        "the move recorded the token's region membership"
    );

    // A fresh hydration is what a server restart rebuilds from documents: the
    // membership table starts EMPTY — derived runtime state, never persisted.
    let reg2 = RoomRegistry::new();
    let room2 = reg2
        .get_or_create(&h.repo, h.world_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        room2
            .scene()
            .read()
            .await
            .region_membership_of(h.token_id)
            .is_none(),
        "a fresh ECS hydration carries no region membership"
    );

    // The accepted restart residual, pinned explicitly: the token walks straight out of
    // the region it occupied across the restart and NO exit fires — the fresh table has
    // no recorded membership to leave, so its enter-applied condition stays on (the same
    // residual class as an in-flight move completing across a restart).
    room2
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path: vec![h.adj, h.start],
                ts: crate::ws::time::now_millis(),
                request_id: Uuid::nil(),
            },
        )
        .await
        .unwrap();
    let actor = h.repo.get_document(actor_id).await.unwrap().unwrap();
    assert_eq!(
        actor.engine.unwrap()["conditions"],
        json!(["prone"]),
        "no exit fires from an empty post-restart membership"
    );
}

#[tokio::test]
async fn a_secret_regions_exit_notice_is_forced_gm_only_whatever_it_authored() {
    let h = movement_scene_with_speed("unrestricted", false, 6000.0).await;
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let region_id = crate::test_ids::test_uuid(0x6C0);
    let mut region = wdoc(h.world_id, region_id, "region");
    region.parent_id = Some(h.scene_id);
    region.owner = Some(h.gm.user_id);
    region.engine = Some(trigger_region_engine(
        (1, 0),
        "terrain",
        json!([
            { "on": "enter", "effect": { "type": "chat_notice", "text": "A trap springs.", "audience": "public" } },
            { "on": "exit", "effect": { "type": "chat_notice", "text": "The trap resets.", "audience": "public" } },
        ]),
    ));
    // World-readable at the document level — the `/engine` tier override alone makes the
    // region secret, so the forcing is attributable to the tier, mirroring the
    // enter-path battery's own fixture shape.
    region.permissions.default = DocRole::Observer;
    region
        .permissions
        .property_overrides
        .insert("/engine".into(), Visibility::GmOnly);
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: region }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    move_token(&h, vec![h.start, h.adj]).await;
    move_token(&h, vec![h.adj, h.start]).await;

    let notices = region_notices(&h).await;
    assert_eq!(
        notices.len(),
        2,
        "the enter AND the exit notice both committed"
    );
    for notice in &notices {
        assert_eq!(
            notice_engine(notice).audience,
            Audience::GmOnly,
            "an exit-fired notice rides the SAME secrecy forcing as enter/arrest"
        );
        assert_eq!(
            notice.permissions.default,
            DocRole::None,
            "no world-readable default leaks the secret region's exit notice"
        );
    }
    let texts: Vec<String> = notices.iter().map(notice_text).collect();
    assert!(texts.contains(&"A trap springs.".to_string()));
    assert!(texts.contains(&"The trap resets.".to_string()));
}

#[tokio::test]
async fn placement_membership_updates_so_a_teleport_out_fires_exit() {
    let h = movement_scene("unrestricted", false).await;
    let actor_id = link_actor(&h, 0x6D0, vec![], json!({})).await;
    place_region(
        &h,
        0x6D1,
        (1, 0),
        "terrain",
        json!([
            { "on": "enter", "effect": { "type": "condition_add", "condition": "prone" } },
            { "on": "exit", "effect": { "type": "condition_remove", "condition": "prone" } },
        ]),
    )
    .await;

    // A genuine position Update (a GM teleport, never a dense walk) lands the token
    // inside the region: the enter fires AND the membership records the region.
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Update {
                doc_id: h.token_id,
                changes: vec![
                    FieldChange {
                        remove: false,
                        path: "/engine/x".into(),
                        old: json!(50.0),
                        new: json!(150.0),
                    },
                    FieldChange {
                        remove: false,
                        path: "/engine/y".into(),
                        old: json!(50.0),
                        new: json!(50.0),
                    },
                ],
            }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    let actor = h.repo.get_document(actor_id).await.unwrap().unwrap();
    assert_eq!(
        actor.engine.unwrap()["conditions"],
        json!(["prone"]),
        "placement fires the enter"
    );

    // Teleporting back out fires the exit — placement updates membership exactly as a
    // move does, or no exit could ever follow a placement entry. The destination must
    // clear the region under the FOOTPRINT basis placements use: a 1x1 actor-backed
    // token's disc (radius ~0.71 cells) still touches region cell (1,0) from (50,50),
    // so the out-teleport lands two cells away.
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Update {
                doc_id: h.token_id,
                changes: vec![
                    FieldChange {
                        remove: false,
                        path: "/engine/x".into(),
                        old: json!(150.0),
                        new: json!(50.0),
                    },
                    FieldChange {
                        remove: false,
                        path: "/engine/y".into(),
                        old: json!(50.0),
                        new: json!(250.0),
                    },
                ],
            }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    let actor = h.repo.get_document(actor_id).await.unwrap().unwrap();
    assert_eq!(
        actor.engine.unwrap()["conditions"],
        json!([]),
        "a teleport out after a placement entry fires the exit"
    );
}

#[tokio::test]
async fn a_cross_scene_portal_fires_the_source_regions_exit_effects() {
    let h = movement_scene("unrestricted", false).await;
    let actor_id = link_actor(&h, 0x6E0, vec![], json!({})).await;
    let dest = place_scene(&h, 0x6E1).await;
    place_region(
        &h,
        0x6E2,
        (1, 0),
        "terrain",
        json!([
            { "on": "enter", "effect": { "type": "condition_add", "condition": "prone" } },
            { "on": "enter", "effect": { "type": "teleport",
                "target": { "scene": dest.to_string(), "x": 250.0, "y": 250.0, "elevation": null, "vfx": null } } },
            { "on": "exit", "effect": { "type": "condition_remove", "condition": "prone" } },
        ]),
    )
    .await;

    move_token(&h, vec![h.start, h.adj]).await;

    let token = h.repo.get_document(h.token_id).await.unwrap().unwrap();
    assert_eq!(token.parent_id, Some(dest), "the portal hop committed");
    // The destination scene's identity table does not carry the source region, yet the
    // exit still fires (the unfiltered by-id row): enter-add then exit-remove nets empty.
    let actor = h.repo.get_document(actor_id).await.unwrap().unwrap();
    assert_eq!(
        actor.engine.unwrap()["conditions"],
        json!([]),
        "the source region's exit fires even though the token left its scene"
    );
}

#[tokio::test]
async fn a_region_that_gains_an_exit_trigger_after_the_token_left_does_not_fire_a_stale_exit() {
    // Fast animation so the per-token moving lock expires between the awaited moves.
    let h = movement_scene_with_speed("unrestricted", false, 6000.0).await;
    let region_id = place_region(
        &h,
        0x6F0,
        (1, 0),
        "terrain",
        json!([{ "on": "enter", "effect": { "type": "chat_notice", "text": "in", "audience": "public" } }]),
    )
    .await;

    move_token(&h, vec![h.start, h.adj]).await;
    // Leaving fires nothing (the region carries no Exit trigger yet) — but the
    // UNCONDITIONAL membership update must still record the departure. This is the step
    // a "diff only when something fires" optimization skips; the rest of the test exists
    // to catch exactly that skip.
    move_token(&h, vec![h.adj, h.start]).await;
    assert_eq!(
        region_notices(&h).await.len(),
        1,
        "enter fired; leaving an exit-less region fires nothing"
    );

    // The GM now adds an Exit trigger to the region the token ALREADY left.
    let stored_triggers = h
        .repo
        .get_document(region_id)
        .await
        .unwrap()
        .unwrap()
        .engine
        .unwrap()["triggers"]
        .clone();
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Update {
                doc_id: region_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/engine/triggers".into(),
                    old: stored_triggers,
                    new: json!([
                        { "on": "enter", "effect": { "type": "chat_notice", "text": "in", "audience": "public" } },
                        { "on": "exit", "effect": { "type": "chat_notice", "text": "out", "audience": "public" } },
                    ]),
                }],
            }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    // Any later move entirely outside fires NO exit: the membership was healed when the
    // token left, so the new trigger has no stale entry to fire against.
    move_token(&h, vec![h.start, h.lit_goal]).await;
    let texts: Vec<String> = region_notices(&h).await.iter().map(notice_text).collect();
    assert_eq!(
        texts,
        vec!["in".to_string()],
        "no stale exit fires against a membership the token already left, got {texts:?}"
    );
}

#[tokio::test]
async fn an_elevation_changing_placement_fires_the_floor_banded_regions_exit() {
    let h = movement_scene("unrestricted", false).await;
    let actor_id = link_actor(&h, 0x701, vec![], json!({})).await;
    // A region banded to a non-ground floor [10, 20] — its triggers only exist for a
    // mover inside the band.
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let region_id = crate::test_ids::test_uuid(0x702);
    let mut region = wdoc(h.world_id, region_id, "region");
    region.parent_id = Some(h.scene_id);
    region.owner = Some(h.gm.user_id);
    region.permissions.default = DocRole::Observer;
    let mut engine = trigger_region_engine(
        (1, 0),
        "terrain",
        json!([
            { "on": "enter", "effect": { "type": "condition_add", "condition": "prone" } },
            { "on": "exit", "effect": { "type": "condition_remove", "condition": "prone" } },
        ]),
    );
    engine["elevation"] = json!({ "bottom": 10.0, "top": 20.0 });
    region.engine = Some(engine);
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: region }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    // Placement INTO the region AT elevation 15: the band contains the mover, so the
    // enter fires and the membership records the region.
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Update {
                doc_id: h.token_id,
                changes: vec![
                    FieldChange {
                        remove: false,
                        path: "/engine/x".into(),
                        old: json!(50.0),
                        new: json!(150.0),
                    },
                    FieldChange {
                        remove: false,
                        path: "/engine/elevation".into(),
                        old: json!(null),
                        new: json!(15.0),
                    },
                ],
            }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    let actor = h.repo.get_document(actor_id).await.unwrap().unwrap();
    assert_eq!(
        actor.engine.unwrap()["conditions"],
        json!(["prone"]),
        "the banded region's enter fires for a mover inside the band"
    );

    // Placement back to ground AND out of the region in one write: the ground-level
    // identity table excludes the banded region, yet its exit must still fire — the
    // by-id stale-row lookup (`SceneEcs::trigger_region_row`) deliberately carries no
    // mover-elevation filter, because the membership was recorded under the PRIOR floor.
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Update {
                doc_id: h.token_id,
                changes: vec![
                    FieldChange {
                        remove: false,
                        path: "/engine/x".into(),
                        old: json!(150.0),
                        new: json!(50.0),
                    },
                    FieldChange {
                        remove: false,
                        path: "/engine/y".into(),
                        old: json!(50.0),
                        new: json!(250.0),
                    },
                    FieldChange {
                        remove: false,
                        path: "/engine/elevation".into(),
                        old: json!(15.0),
                        new: json!(null),
                    },
                ],
            }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    let actor = h.repo.get_document(actor_id).await.unwrap().unwrap();
    assert_eq!(
        actor.engine.unwrap()["conditions"],
        json!([]),
        "the banded region's exit fires across an elevation change (unfiltered by-id row)"
    );
}

#[tokio::test]
async fn a_secret_regions_cross_scene_portal_exit_notice_is_forced_gm_only() {
    let h = movement_scene("unrestricted", false).await;
    let dest = place_scene(&h, 0x710).await;
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let region_id = crate::test_ids::test_uuid(0x711);
    let mut region = wdoc(h.world_id, region_id, "region");
    region.parent_id = Some(h.scene_id);
    region.owner = Some(h.gm.user_id);
    region.engine = Some(trigger_region_engine(
        (1, 0),
        "terrain",
        json!([
            { "on": "enter", "effect": { "type": "teleport",
                "target": { "scene": dest.to_string(), "x": 250.0, "y": 250.0, "elevation": null, "vfx": null } } },
            { "on": "exit", "effect": { "type": "chat_notice", "text": "The trap resets.", "audience": "public" } },
        ]),
    ));
    // The `/engine` tier override alone makes the region secret, so the forcing is
    // attributable to the tier — and the exit notice fires from the STALE-ROW plan (the
    // destination scene's table never carries this region), pinning that the secrecy
    // lookup resolves rows across BOTH sets, not just the scene table.
    region.permissions.default = DocRole::Observer;
    region
        .permissions
        .property_overrides
        .insert("/engine".into(), Visibility::GmOnly);
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: region }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    move_token(&h, vec![h.start, h.adj]).await;

    let notices = region_notices(&h).await;
    assert_eq!(notices.len(), 1, "the stale-row exit notice committed");
    let engine = notice_engine(&notices[0]);
    assert_eq!(notice_text(&notices[0]), "The trap resets.");
    assert_eq!(
        engine.audience,
        Audience::GmOnly,
        "an exit notice fired from a stale row rides the SAME secrecy forcing"
    );
    assert_eq!(
        notices[0].permissions.default,
        DocRole::None,
        "no world-readable default leaks the secret region's exit notice"
    );
}

#[tokio::test]
async fn a_public_regions_cross_scene_portal_exit_notice_stays_public() {
    let h = movement_scene("unrestricted", false).await;
    let dest = place_scene(&h, 0x712).await;
    place_region(
        &h,
        0x713,
        (1, 0),
        "terrain",
        json!([
            { "on": "enter", "effect": { "type": "teleport",
                "target": { "scene": dest.to_string(), "x": 250.0, "y": 250.0, "elevation": null, "vfx": null } } },
            { "on": "exit", "effect": { "type": "chat_notice", "text": "The gate stands open.", "audience": "public" } },
        ]),
    )
    .await;

    move_token(&h, vec![h.start, h.adj]).await;

    let notices = region_notices(&h).await;
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notice_engine(&notices[0]).audience,
        Audience::Public,
        "a public region's stale-row exit notice honors its authored audience — the \
         secrecy lookup must FIND the stale row, never default"
    );
}

#[tokio::test]
async fn placement_and_teleport_fire_enter_effects_but_region_edits_and_disabled_regions_do_not() {
    let h = movement_scene("unrestricted", false).await;
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let region_id = place_region(
        &h,
        0x7AE,
        (5, 5),
        "terrain",
        json!([{ "on": "enter", "effect": { "type": "chat_notice", "text": "trap", "audience": "gm_only" } }]),
    )
    .await;

    // Placement: a token created inside the region fires its enter effects.
    let mut placed = wdoc(h.world_id, crate::test_ids::test_uuid(0x7AF), "token");
    placed.parent_id = Some(h.scene_id);
    placed.owner = Some(h.gm.user_id);
    placed.engine = Some(token_engine(550.0, 550.0));
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: placed }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    assert_eq!(region_notices(&h).await.len(), 1, "placement fires enter");

    // Teleport: a genuine position Update landing inside the region fires.
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Update {
                doc_id: h.token_id,
                changes: vec![
                    FieldChange {
                        remove: false,
                        path: "/engine/x".into(),
                        old: json!(50.0),
                        new: json!(520.0),
                    },
                    FieldChange {
                        remove: false,
                        path: "/engine/y".into(),
                        old: json!(50.0),
                        new: json!(580.0),
                    },
                ],
            }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    assert_eq!(region_notices(&h).await.len(), 2, "teleport fires enter");

    // A region edit never re-fires its triggers on stationary tokens.
    let set_enabled = |enabled: bool| Operation::Update {
        doc_id: region_id,
        changes: vec![FieldChange {
            remove: false,
            path: "/engine/enabled".into(),
            old: json!(!enabled),
            new: json!(enabled),
        }],
    };
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![set_enabled(false)],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    assert_eq!(
        region_notices(&h).await.len(),
        2,
        "the edit itself fires nothing"
    );

    // Teleporting out and back while the region is disabled fires nothing.
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Update {
                doc_id: h.token_id,
                changes: vec![
                    FieldChange {
                        remove: false,
                        path: "/engine/x".into(),
                        old: json!(520.0),
                        new: json!(50.0),
                    },
                    FieldChange {
                        remove: false,
                        path: "/engine/y".into(),
                        old: json!(580.0),
                        new: json!(50.0),
                    },
                ],
            }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Update {
                doc_id: h.token_id,
                changes: vec![
                    FieldChange {
                        remove: false,
                        path: "/engine/x".into(),
                        old: json!(50.0),
                        new: json!(520.0),
                    },
                    FieldChange {
                        remove: false,
                        path: "/engine/y".into(),
                        old: json!(50.0),
                        new: json!(580.0),
                    },
                ],
            }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    assert_eq!(
        region_notices(&h).await.len(),
        2,
        "a disabled region contributes nothing, entry included"
    );

    // Re-enabled, the next entry fires again.
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![set_enabled(true)],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    assert_eq!(
        region_notices(&h).await.len(),
        2,
        "re-enabling is still just an edit"
    );
}

// --- Teleport (portals) ---

/// Creates a GM-owned, world-readable second scene in `h`'s world, for cross-scene teleports.
async fn place_scene(h: &MovementHandle, id: u128) -> Uuid {
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let scene_id = crate::test_ids::test_uuid(id);
    let mut scene = wdoc(h.world_id, scene_id, "scene");
    scene.owner = Some(h.gm.user_id);
    // The same envelope convention `place_region` uses: world-readable.
    scene.permissions.default = DocRole::Observer;
    scene.engine = Some(json!({
        "grid": { "kind": "square", "size": 100.0 }, "background": null }));
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
    scene_id
}

/// Every persisted command on `h`'s world, in seq order.
async fn committed_events(h: &MovementHandle) -> Vec<crate::data::snapshot::StoredCommand> {
    crate::data::repository::Repository::events_since(&h.repo, h.world_id, 0)
        .await
        .unwrap()
}

#[tokio::test]
async fn same_scene_teleport_repositions_the_token_without_a_move_op() {
    let h = movement_scene("unrestricted", false).await;
    place_region(
        &h,
        0x7B0,
        (1, 0),
        "terrain",
        json!([
            { "on": "enter", "effect": { "type": "teleport",
                "target": { "scene": null, "x": 450.0, "y": 450.0, "elevation": 15.0, "vfx": null } } }
        ]),
    )
    .await;

    move_token(&h, vec![h.start, h.adj]).await;

    let token = h.repo.get_document(h.token_id).await.unwrap().unwrap();
    let eng = token.engine.clone().unwrap();
    assert_eq!(eng["x"], json!(450.0));
    assert_eq!(eng["y"], json!(450.0));
    assert_eq!(
        eng["elevation"],
        json!(15.0),
        "a target naming an elevation writes it"
    );
    assert_eq!(
        token.parent_id,
        Some(h.scene_id),
        "a same-scene teleport never reparents"
    );
    let has_move = committed_events(&h).await.iter().any(|e| {
        e.command
            .ops
            .iter()
            .any(|op| matches!(op, Operation::Move { doc_id, .. } if *doc_id == h.token_id))
    });
    assert!(!has_move, "no Move op lands for a same-scene teleport");
}

#[tokio::test]
async fn cross_scene_teleport_reparents_and_repositions_in_one_command() {
    let h = movement_scene("unrestricted", false).await;
    let dest = place_scene(&h, 0x7C0).await;
    place_region(
        &h,
        0x7C1,
        (1, 0),
        "terrain",
        json!([
            { "on": "enter", "effect": { "type": "teleport",
                "target": { "scene": dest.to_string(), "x": 250.0, "y": 250.0, "elevation": null, "vfx": null } } }
        ]),
    )
    .await;

    move_token(&h, vec![h.start, h.adj]).await;

    // The fixture token is PLAYER-owned: with any capability-gated origin the Move arm of
    // `apply_intent` would refuse the reparenting outright — the teleport landing at all is
    // the behavioral assertion that the batch committed under a capability-skipping origin
    // (`WriteOrigin::Trigger`).
    let token = h.repo.get_document(h.token_id).await.unwrap().unwrap();
    assert_eq!(token.parent_id, Some(dest), "the token is reparented");
    let eng = token.engine.clone().unwrap();
    assert_eq!(eng["x"], json!(250.0));
    assert_eq!(eng["y"], json!(250.0));
    let teleport_cmd = committed_events(&h)
        .await
        .into_iter()
        .find(|e| {
            e.command
                .ops
                .iter()
                .any(|op| matches!(op, Operation::Move { doc_id, .. } if *doc_id == h.token_id))
        })
        .expect("the teleport's reparenting committed");
    assert!(
        teleport_cmd
            .command
            .ops
            .iter()
            .any(|op| matches!(op, Operation::Update { doc_id, .. } if *doc_id == h.token_id)),
        "the reparenting Move and the position Update commit in ONE command (one seq)"
    );
}

/// Drains every currently-buffered broadcast off `rx` (non-blocking) and returns how many were
/// `ServerMsg::Vfx` frames.
fn count_vfx_frames(
    rx: &mut tokio::sync::broadcast::Receiver<crate::ws::room::RoomEvent>,
) -> usize {
    use tokio::sync::broadcast::error::TryRecvError;
    let mut n = 0;
    loop {
        match rx.try_recv() {
            Ok(RoomEvent::Other(msg)) => {
                if matches!(&*msg, ServerMsg::Vfx { .. }) {
                    n += 1;
                }
            }
            Ok(RoomEvent::Event(_)) => {}
            // A skipped-messages notification, not a stop condition — keep draining what
            // remains resident in the ring so a Vfx frame after the gap is still counted.
            Err(TryRecvError::Lagged(_)) => {}
            Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => break,
        }
    }
    n
}

/// Drains every currently-buffered broadcast off `rx` (non-blocking) and returns every
/// `ServerMsg::ScrollingText { token, text, gm_only, .. }` frame's `(text, gm_only)` pair, in
/// order — mirrors `count_vfx_frames`'s drain shape.
fn scrolling_text_frames(
    rx: &mut tokio::sync::broadcast::Receiver<crate::ws::room::RoomEvent>,
) -> Vec<(String, bool)> {
    use tokio::sync::broadcast::error::TryRecvError;
    let mut out = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(RoomEvent::Other(msg)) => {
                if let ServerMsg::ScrollingText { text, gm_only, .. } = &*msg {
                    out.push((text.clone(), *gm_only));
                }
            }
            Ok(RoomEvent::Event(_)) => {}
            Err(TryRecvError::Lagged(_)) => {}
            Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => break,
        }
    }
    out
}

#[tokio::test]
async fn scrolling_text_trigger_broadcasts_to_the_room_when_region_is_visible_to_all() {
    let h = movement_scene("unrestricted", false).await;
    place_region(
        &h,
        0x7C0,
        (1, 0),
        "terrain",
        json!([{ "on": "enter", "effect": { "type": "scrolling_text", "text": "You feel a chill." } }]),
    )
    .await;
    let (mut rx, _) = h.room.subscribe();

    move_token(&h, vec![h.start, h.adj]).await;

    assert_eq!(
        scrolling_text_frames(&mut rx),
        vec![("You feel a chill.".to_string(), false)],
        "a visible-to-all region's overlay broadcasts room-wide (gm_only: false)"
    );
}

#[tokio::test]
async fn scrolling_text_trigger_is_gm_only_when_region_is_not_visible_to_all() {
    let h = movement_scene("unrestricted", false).await;
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let region_id = crate::test_ids::test_uuid(0x7C1);
    let mut region = wdoc(h.world_id, region_id, "region");
    region.parent_id = Some(h.scene_id);
    region.owner = Some(h.gm.user_id);
    region.engine = Some(trigger_region_engine(
        (1, 0),
        "terrain",
        json!([{ "on": "enter", "effect": { "type": "scrolling_text", "text": "A trap springs." } }]),
    ));
    region.permissions.default = DocRole::Observer;
    // Mutation witness: deleting this `/engine` GmOnly override (or the `!visible_to_all` read
    // in `Room::apply_region_trigger_effects`'s scrolling-text arm) flips `gm_only` to `false`
    // and fails this assertion.
    region
        .permissions
        .property_overrides
        .insert("/engine".into(), Visibility::GmOnly);
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: region }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    let (mut rx, _) = h.room.subscribe();

    move_token(&h, vec![h.start, h.adj]).await;

    assert_eq!(
        scrolling_text_frames(&mut rx),
        vec![("A trap springs.".to_string(), true)],
        "a secret region's overlay is forced gm_only: true, whatever the region's own visibility"
    );
}

/// A region whose `enter` trigger carries BOTH a same-scene teleport with a carried `vfx` asset
/// AND an unrelated `chat_notice` — lets a test distinguish "the whole batch was dropped" from
/// "only the VFX broadcast was throttled".
async fn place_vfx_teleport_region(h: &MovementHandle, id: u128, cell: (i32, i32)) -> Uuid {
    place_region(
        h,
        id,
        cell,
        "terrain",
        json!([
            { "on": "enter", "effect": { "type": "teleport",
                "target": { "scene": null, "x": 450.0, "y": 450.0, "elevation": null, "vfx": "11111111-1111-1111-1111-111111111111" } } },
            { "on": "enter", "effect": { "type": "chat_notice", "text": "A portal hums.", "audience": "public" } },
        ]),
    )
    .await
}

#[tokio::test]
async fn teleport_vfx_plays_at_both_ends_when_the_shared_budget_has_room() {
    // Positive control (see the "a green gate can be inert" lesson): proves the detector FIRES
    // under budget before the throttled test below proves it refuses over budget.
    let h = movement_scene("unrestricted", false).await;
    place_vfx_teleport_region(&h, 0x7B8, (1, 0)).await;
    let (mut rx, _) = h.room.subscribe();

    move_token(&h, vec![h.start, h.adj]).await;

    assert_eq!(
        count_vfx_frames(&mut rx),
        2,
        "a carried vfx plays once at the origin and once at the destination when the bucket has room"
    );
    let notices = region_notices(&h).await;
    assert_eq!(notices.len(), 1, "the unrelated chat_notice still commits");
}

#[tokio::test]
async fn teleport_vfx_is_throttled_by_the_shared_vfx_rate_bucket_like_repeated_fx_calls() {
    let h = movement_scene("unrestricted", false).await;
    place_vfx_teleport_region(&h, 0x7B9, (1, 0)).await;
    let (mut rx, _) = h.room.subscribe();

    // Exhaust the SAME bucket `ws::conn`'s `PlayVfx` arm and `chat::fx` charge, keyed on the
    // token's effective owner (the fixture token is player-owned — see `movement_scene`'s own
    // doc) — mirroring a player who already spent their 30/min budget via repeated `/fx` calls.
    // Charged at real wall-clock time (`PingRateLimiter::check`'s window is relative to the
    // `now_ms` each call passes, not absolute): `move_token` below fires the trigger moments
    // later via its own fresh `now_millis()`, which must land inside the SAME trailing-60s
    // window as these charges or `retain` prunes them all before the trigger's own check runs.
    let now = crate::ws::time::now_millis();
    for i in 0..30 {
        assert!(h.room.vfx_rate.check(h.player.user_id, now + i, 30));
    }

    move_token(&h, vec![h.start, h.adj]).await;

    assert_eq!(
        count_vfx_frames(&mut rx),
        0,
        "the portal's carried vfx is refused once the shared budget is exhausted, exactly like a \
         31st /fx call would be"
    );
    // The teleport itself and the unrelated same-batch chat_notice are NOT throttled — only the
    // vfx broadcast is refused (never-fork: the budget gates the one-shot, not the whole batch).
    let token = h.repo.get_document(h.token_id).await.unwrap().unwrap();
    let eng = token.engine.clone().unwrap();
    assert_eq!(eng["x"], json!(450.0), "the teleport still committed");
    assert_eq!(eng["y"], json!(450.0));
    let notices = region_notices(&h).await;
    assert_eq!(
        notices.len(),
        1,
        "the unrelated chat_notice still commits when only the vfx broadcast is throttled"
    );
}

#[tokio::test]
async fn teleport_to_a_missing_scene_notices_the_gm_and_moves_nothing() {
    let h = movement_scene("unrestricted", false).await;
    let missing = crate::test_ids::test_uuid(0xDEAD);
    place_region(
        &h,
        0x7D0,
        (1, 0),
        "terrain",
        json!([
            { "on": "enter", "effect": { "type": "teleport",
                "target": { "scene": missing.to_string(), "x": 0.0, "y": 0.0, "elevation": null, "vfx": null } } }
        ]),
    )
    .await;

    move_token(&h, vec![h.start, h.adj]).await;

    let token = h.repo.get_document(h.token_id).await.unwrap().unwrap();
    let eng = token.engine.clone().unwrap();
    assert_eq!(
        (eng["x"].as_f64().unwrap(), eng["y"].as_f64().unwrap()),
        h.adj,
        "the token stays where the walk ended (no move, no update)"
    );
    assert_eq!(token.parent_id, Some(h.scene_id));
    let notices = region_notices(&h).await;
    assert_eq!(notices.len(), 1, "one deduplicated failure notice");
    let engine = notice_engine(&notices[0]);
    assert_eq!(engine.audience, Audience::GmOnly);
    assert!(
        notice_text(&notices[0]).contains("does not exist"),
        "the notice names the failure"
    );
}

/// A portal target that exists (`doc_type == "scene"`) but belongs to a DIFFERENT world must
/// refuse exactly like a missing scene — not fail deep inside `commit_ops_locked`'s
/// `check_command_scope`, which would silently drop the WHOLE ops batch (every unrelated
/// effect fired in the same pass) through the generic `tracing::debug!` arm with no GM notice.
/// This region fires TWO effects on entry — the teleport AND an unrelated `chat_notice` — so a
/// batch-wide drop and a properly-isolated teleport refusal are distinguishable: the notice
/// commits either way, but only the fixed behavior ALSO posts a GM-only failure notice naming
/// the teleport.
#[tokio::test]
async fn teleport_to_a_cross_world_scene_notices_the_gm_and_moves_nothing() {
    let h = movement_scene("unrestricted", false).await;

    // A scene document that genuinely exists, in a SEPARATE world owned by the same GM.
    let other_world = h
        .repo
        .create_world_owned("other", h.gm.user_id, 0)
        .await
        .unwrap();
    let reg_other = RoomRegistry::new();
    let room_other = reg_other
        .get_or_create(&h.repo, other_world.id)
        .await
        .unwrap()
        .unwrap();
    let foreign_scene_id = crate::test_ids::test_uuid(0x7F0_5CE1);
    let mut foreign_scene =
        crate::data::document::tests::world_scoped_doc(other_world.id, foreign_scene_id, "scene");
    foreign_scene.owner = Some(h.gm.user_id);
    room_other
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: foreign_scene }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    place_region(
        &h,
        0x7F0,
        (1, 0),
        "terrain",
        json!([
            { "on": "enter", "effect": { "type": "teleport",
                "target": { "scene": foreign_scene_id.to_string(), "x": 0.0, "y": 0.0, "elevation": null, "vfx": null } } },
            { "on": "enter", "effect": { "type": "chat_notice",
                "text": "entered the cross-world portal region", "audience": "public" } }
        ]),
    )
    .await;

    move_token(&h, vec![h.start, h.adj]).await;

    let token = h.repo.get_document(h.token_id).await.unwrap().unwrap();
    let eng = token.engine.clone().unwrap();
    assert_eq!(
        (eng["x"].as_f64().unwrap(), eng["y"].as_f64().unwrap()),
        h.adj,
        "the token stays where the walk ended (no cross-world move, no update)"
    );
    assert_eq!(token.parent_id, Some(h.scene_id));

    let notices = region_notices(&h).await;
    let texts: Vec<String> = notices.iter().map(notice_text).collect();
    assert!(
        texts.iter().any(|t| t.contains("does not exist")),
        "a GM-only failure notice names the cross-world teleport refusal, got {texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|t| t.contains("entered the cross-world portal region")),
        "the batch's OTHER effect (the unrelated chat_notice) still committed, got {texts:?}"
    );
}

#[tokio::test]
async fn a_chained_portal_is_refused_after_one_hop() {
    let h = movement_scene("unrestricted", false).await;
    // The first region (cell (1,0)) teleports into the second's cell (2,0), whose own trigger
    // list carries ANOTHER teleport to (450,50).
    place_region(
        &h,
        0x7E0,
        (1, 0),
        "terrain",
        json!([
            { "on": "enter", "effect": { "type": "teleport",
                "target": { "scene": null, "x": 250.0, "y": 50.0, "elevation": null, "vfx": null } } }
        ]),
    )
    .await;
    place_region(
        &h,
        0x7E1,
        (2, 0),
        "terrain",
        json!([
            { "on": "enter", "effect": { "type": "teleport",
                "target": { "scene": null, "x": 450.0, "y": 50.0, "elevation": null, "vfx": null } } }
        ]),
    )
    .await;

    move_token(&h, vec![h.start, h.adj]).await;

    let token = h.repo.get_document(h.token_id).await.unwrap().unwrap();
    let eng = token.engine.clone().unwrap();
    assert_eq!(
        (eng["x"].as_f64().unwrap(), eng["y"].as_f64().unwrap()),
        (250.0, 50.0),
        "the token ends on the FIRST destination — one hop per move"
    );
    let texts: Vec<String> = region_notices(&h).await.iter().map(notice_text).collect();
    assert!(
        texts.iter().any(|t| t.contains("chained portal refused")),
        "the refused second hop surfaces as a GM-only notice, got {texts:?}"
    );
}

#[tokio::test]
async fn destination_enter_effects_fire_after_a_teleport() {
    let h = movement_scene("unrestricted", false).await;
    let actor_id = link_actor(&h, 0x7F0, vec![], json!({})).await;
    place_region(
        &h,
        0x7F1,
        (1, 0),
        "terrain",
        json!([
            { "on": "enter", "effect": { "type": "teleport",
                "target": { "scene": null, "x": 250.0, "y": 50.0, "elevation": null, "vfx": null } } }
        ]),
    )
    .await;
    place_region(
        &h,
        0x7F2,
        (2, 0),
        "terrain",
        json!([
            { "on": "enter", "effect": { "type": "condition_add", "condition": "prone" } }
        ]),
    )
    .await;

    move_token(&h, vec![h.start, h.adj]).await;

    let token = h.repo.get_document(h.token_id).await.unwrap().unwrap();
    let eng = token.engine.clone().unwrap();
    assert_eq!(
        (eng["x"].as_f64().unwrap(), eng["y"].as_f64().unwrap()),
        (250.0, 50.0)
    );
    let actor = h.repo.get_document(actor_id).await.unwrap().unwrap();
    assert_eq!(
        actor.engine.unwrap()["conditions"],
        json!(["prone"]),
        "the destination region's Enter effects fire after the teleport, in the same firing pass"
    );
}

#[tokio::test]
async fn teleporting_a_combatant_off_an_active_combats_scene_posts_a_notice() {
    let h = movement_scene("unrestricted", false).await;
    let actor_id = link_actor(&h, 0x800, vec![], json!({})).await;
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let (combat_id, combatant_id) = (
        crate::test_ids::test_uuid(0x801),
        crate::test_ids::test_uuid(0x802),
    );
    let mut combat = wdoc(h.world_id, combat_id, "combat");
    combat.owner = Some(h.gm.user_id);
    combat.engine = Some(json!({
        "scene_id": h.scene_id,
        "active": true,
        "round": 1,
        "turn": combatant_id,
        "turn_control": "owner_may_end",
        "order": [combatant_id],
        "movement": { "resource": null, "interpretation": "spaces", "enforcement": "none" },
        "effect_cleanup": true,
        "rewind_restore": true,
        "forward_restore": false,
        "effect_lifecycle": {}
    }));
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: combat }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    let mut combatant = wdoc(h.world_id, combatant_id, "combatant");
    combatant.parent_id = Some(combat_id);
    combatant.owner = Some(h.player.user_id);
    combatant
        .permissions
        .users
        .insert(h.player.user_id, DocRole::Owner);
    combatant.engine = Some(json!({
        "kind": { "type": "actor", "token_id": h.token_id, "actor_id": actor_id },
        "initiative": null,
        "tiebreak": 0.0,
        "resources": {}
    }));
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: combatant }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    // The baseline is the STORED (normalized) engine body, read back after the publish.
    let combatant_engine = h
        .repo
        .get_document(combatant_id)
        .await
        .unwrap()
        .unwrap()
        .engine
        .unwrap();

    let dest = place_scene(&h, 0x803).await;
    place_region(
        &h,
        0x804,
        (1, 0),
        "terrain",
        json!([
            { "on": "enter", "effect": { "type": "teleport",
                "target": { "scene": dest.to_string(), "x": 250.0, "y": 250.0, "elevation": null, "vfx": null } } }
        ]),
    )
    .await;

    move_token(&h, vec![h.start, h.adj]).await;

    let token = h.repo.get_document(h.token_id).await.unwrap().unwrap();
    assert_eq!(token.parent_id, Some(dest));
    let texts: Vec<String> = region_notices(&h).await.iter().map(notice_text).collect();
    assert!(
        texts.iter().any(|t| t.contains("teleported off scene")),
        "a GM-only notice names the combatant leaving its combat's scene, got {texts:?}"
    );
    // The combatant record is unaffected: still exists, unchanged engine body (no combat
    // document write rode the teleport batch).
    let after = h.repo.get_document(combatant_id).await.unwrap().unwrap();
    assert_eq!(
        after.engine.unwrap(),
        combatant_engine,
        "the combatant record is untouched by the teleport"
    );
}

// --- Turn/round-scoped triggers (combat-clock boundaries) ---

/// Drives one combat intent as the GM and asserts the clock accepted it.
async fn gm_combat_intent(h: &MovementHandle, msg: crate::ws::protocol::ClientMsg) {
    let rate = crate::ws::PingRateLimiter::new();
    let reply = crate::combat::handle_combat_intent(&h.room, &h.repo, &h.gm, msg, 0, &rate).await;
    assert!(
        matches!(
            reply,
            Some(crate::ws::protocol::ServerMsg::CombatResult { .. })
        ),
        "the combat intent succeeded, got {reply:?}"
    );
}

/// Publishes an inactive combat on `h`'s scene with `order` as given and no
/// recoveries, returning once committed. Combatant documents are the
/// caller's own (this only builds the combat doc itself).
async fn place_combat(h: &MovementHandle, id: u128, order: Vec<Uuid>) -> Uuid {
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let combat_id = crate::test_ids::test_uuid(id);
    let mut combat = wdoc(h.world_id, combat_id, "combat");
    combat.owner = Some(h.gm.user_id);
    combat.engine = Some(json!({
        "scene_id": h.scene_id,
        "active": false,
        "round": 0,
        "turn": null,
        "turn_control": "owner_may_end",
        "order": order,
        "movement": { "resource": null, "interpretation": "per_cell", "enforcement": "none" },
        "effect_cleanup": true,
        "rewind_restore": true,
        "forward_restore": false,
        "effect_lifecycle": {}
    }));
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: combat }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    combat_id
}

/// Publishes one combatant child document for `combat_id`. `engine` is the
/// raw `CombatantEngine` JSON; `hidden` sets the `permissions.default: none`
/// shape that marks a combatant hidden.
async fn place_combatant(
    h: &MovementHandle,
    id: u128,
    combat_id: Uuid,
    hidden: bool,
    engine: serde_json::Value,
) -> Uuid {
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let combatant_id = crate::test_ids::test_uuid(id);
    let mut combatant = wdoc(h.world_id, combatant_id, "combatant");
    combatant.parent_id = Some(combat_id);
    combatant.owner = Some(h.gm.user_id);
    if hidden {
        combatant.permissions.default = DocRole::None;
    } else {
        combatant.permissions.default = DocRole::Observer;
    }
    combatant.engine = Some(engine);
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: combatant }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    combatant_id
}

#[tokio::test]
async fn a_token_leaving_the_region_before_its_turn_start_does_not_fire_the_turn_start_trigger() {
    // Fast animation so the per-token moving lock expires between the awaited moves.
    let h = movement_scene_with_speed("unrestricted", false, 6000.0).await;
    let actor_id = link_actor(&h, 0x8A0, vec![], json!({})).await;
    place_region(
        &h,
        0x8A1,
        (1, 0),
        "terrain",
        json!([{ "on": "turn_start", "effect": { "type": "condition_add", "condition": "blessed" } }]),
    )
    .await;
    // Combat: [the token's combatant, an Event], not yet started.
    let combatant_id = place_combatant(
        &h,
        0x8A3,
        place_combat(
            &h,
            0x8A2,
            vec![
                crate::test_ids::test_uuid(0x8A3),
                crate::test_ids::test_uuid(0x8A4),
            ],
        )
        .await,
        false,
        json!({
            "kind": { "type": "actor", "token_id": h.token_id, "actor_id": actor_id },
            "initiative": null,
            "tiebreak": 0.0,
            "resources": {}
        }),
    )
    .await;
    let combat_id = crate::test_ids::test_uuid(0x8A2);
    place_combatant(
        &h,
        0x8A4,
        combat_id,
        false,
        json!({
            "kind": { "type": "event", "lifespan": null, "message": null },
            "initiative": null,
            "tiebreak": 0.0,
            "resources": {}
        }),
    )
    .await;

    // Enter the region (membership records it), then leave again (the
    // membership diff drops it) — BEFORE the clock ever reaches the token's
    // turn start.
    move_token(&h, vec![h.start, h.adj]).await;
    assert!(
        h.room
            .scene()
            .read()
            .await
            .region_membership_of(h.token_id)
            .is_some(),
        "positive control: the move recorded the token's region membership"
    );
    move_token(&h, vec![h.adj, h.start]).await;

    // CombatStart reaches the combatant's turn_start — but the token has
    // LEFT the region, so the trigger must NOT fire: the firing is
    // membership-gated, not merely phase-gated.
    gm_combat_intent(
        &h,
        crate::ws::protocol::ClientMsg::CombatStart {
            request_id: Uuid::nil(),
            combat_id,
        },
    )
    .await;
    let actor = h.repo.get_document(actor_id).await.unwrap().unwrap();
    assert_eq!(
        actor.engine.clone().unwrap()["conditions"],
        json!([]),
        "a token that left the region before its turn start fires nothing"
    );

    // Positive control: re-enter the region, then advance the clock around
    // to the combatant's NEXT turn start (its turn ends, the Event
    // auto-resolves, the round wraps, its turn begins at round 2) — now the
    // stored membership names the region and the trigger fires.
    move_token(&h, vec![h.start, h.adj]).await;
    gm_combat_intent(
        &h,
        crate::ws::protocol::ClientMsg::CombatAdvance {
            request_id: Uuid::nil(),
            combat_id,
        },
    )
    .await;
    let actor = h.repo.get_document(actor_id).await.unwrap().unwrap();
    assert_eq!(
        actor.engine.unwrap()["conditions"],
        json!(["blessed"]),
        "standing in the region at its turn start, the trigger fires"
    );
    let _ = combatant_id;
}

#[tokio::test]
async fn turn_end_region_trigger_fires_once_per_turn_across_parked_auto_resolved_turns() {
    let h = movement_scene_with_speed("unrestricted", false, 6000.0).await;
    let actor_id = link_actor(&h, 0x8B0, vec![], json!({})).await;
    // Registry: `hp` tracked, every recovery zero — only the region trigger
    // moves the value, so the count of fires is directly observable.
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let registry_id = crate::test_ids::test_uuid(0x8B1);
    let mut registry = wdoc(h.world_id, registry_id, "resource-registry");
    registry.owner = Some(h.gm.user_id);
    registry.engine = Some(json!({
        "resources": {
            "hp": { "name": "HP", "order": 0,
                "binding": { "kind": "tracked", "max": 30.0,
                    "recover": { "turn_start": 0, "turn_end": 0, "round_start": 0, "round_end": 0 } } }
        }
    }));
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: registry }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    place_region(
        &h,
        0x8B2,
        (1, 0),
        "terrain",
        json!([{ "on": "turn_end", "effect": { "type": "resource_delta", "resource": "hp", "amount": -3.0 } }]),
    )
    .await;
    // Combat [Event, hidden token-bound B]: BOTH entries auto-resolve
    // (Event always; the hidden combatant under OwnerMayEnd), so every
    // advance walks the whole order and parks back on B — the exact shape
    // whose NEXT advance re-ends the parked (round, B) turn a second time.
    let combat_id = place_combat(
        &h,
        0x8B3,
        vec![
            crate::test_ids::test_uuid(0x8B4),
            crate::test_ids::test_uuid(0x8B5),
        ],
    )
    .await;
    place_combatant(
        &h,
        0x8B4,
        combat_id,
        false,
        json!({
            "kind": { "type": "event", "lifespan": null, "message": null },
            "initiative": null,
            "tiebreak": 0.0,
            "resources": {}
        }),
    )
    .await;
    let b_id = place_combatant(
        &h,
        0x8B5,
        combat_id,
        true,
        json!({
            "kind": { "type": "actor", "token_id": h.token_id, "actor_id": actor_id },
            "initiative": null,
            "tiebreak": 0.0,
            "resources": { "hp": { "current": 10.0 } }
        }),
    )
    .await;

    let hp = async |h: &MovementHandle| -> f64 {
        h.repo
            .get_document(b_id)
            .await
            .unwrap()
            .unwrap()
            .engine
            .unwrap()["resources"]["hp"]["current"]
            .as_f64()
            .unwrap()
    };

    // The token stands in the region before the clock starts.
    move_token(&h, vec![h.start, h.adj]).await;

    // CombatStart's walk ends B's round-1 turn exactly once: 10 -> 7. (B
    // then parks: every entry auto-resolved within the step budget.)
    gm_combat_intent(
        &h,
        crate::ws::protocol::ClientMsg::CombatStart {
            request_id: Uuid::nil(),
            combat_id,
        },
    )
    .await;
    assert_eq!(
        hp(&h).await,
        7.0,
        "turn_end fired once for the round-1 turn"
    );

    // CombatAdvance FIRST re-ends the PARKED (round 1, B) turn — the
    // once-per-turn dedup suppresses that second fire — then wraps and ends
    // B's genuinely NEW round-2 turn: 7 -> 4. Without the dedup the parked
    // re-ending would apply a second -3 and hp would read 1.0.
    gm_combat_intent(
        &h,
        crate::ws::protocol::ClientMsg::CombatAdvance {
            request_id: Uuid::nil(),
            combat_id,
        },
    )
    .await;
    assert_eq!(
        hp(&h).await,
        4.0,
        "the parked turn's re-ending was deduped; only the new round-2 turn fired"
    );
}

#[tokio::test]
async fn rewind_re_arms_the_turn_round_trigger_dedup_for_the_re_crossed_boundary() {
    // Mirrors `turn_end_region_trigger_fires_once_per_turn_across_parked_auto_resolved_turns`'s
    // fixture: a turn_end resource_delta(-3) region, [Event, hidden token-bound B], both
    // auto-resolving. The point under test: a COMMITTED rewind clears the
    // combat's dedup entry, so the re-crossed boundary's trigger re-fires —
    // the same re-run recovery performs on the re-crossed boundary.
    let h = movement_scene_with_speed("unrestricted", false, 6000.0).await;
    let actor_id = link_actor(&h, 0x8C0, vec![], json!({})).await;
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let registry_id = crate::test_ids::test_uuid(0x8C1);
    let mut registry = wdoc(h.world_id, registry_id, "resource-registry");
    registry.owner = Some(h.gm.user_id);
    registry.engine = Some(json!({
        "resources": {
            "hp": { "name": "HP", "order": 0,
                "binding": { "kind": "tracked", "max": 30.0,
                    "recover": { "turn_start": 0, "turn_end": 0, "round_start": 0, "round_end": 0 } } }
        }
    }));
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: registry }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    place_region(
        &h,
        0x8C2,
        (1, 0),
        "terrain",
        json!([{ "on": "turn_end", "effect": { "type": "resource_delta", "resource": "hp", "amount": -3.0 } }]),
    )
    .await;
    let combat_id = place_combat(
        &h,
        0x8C3,
        vec![
            crate::test_ids::test_uuid(0x8C4),
            crate::test_ids::test_uuid(0x8C5),
        ],
    )
    .await;
    place_combatant(
        &h,
        0x8C4,
        combat_id,
        false,
        json!({
            "kind": { "type": "event", "lifespan": null, "message": null },
            "initiative": null,
            "tiebreak": 0.0,
            "resources": {}
        }),
    )
    .await;
    let b_id = place_combatant(
        &h,
        0x8C5,
        combat_id,
        true,
        json!({
            "kind": { "type": "actor", "token_id": h.token_id, "actor_id": actor_id },
            "initiative": null,
            "tiebreak": 0.0,
            "resources": { "hp": { "current": 10.0 } }
        }),
    )
    .await;

    let hp = async |h: &MovementHandle| -> f64 {
        h.repo
            .get_document(b_id)
            .await
            .unwrap()
            .unwrap()
            .engine
            .unwrap()["resources"]["hp"]["current"]
            .as_f64()
            .unwrap()
    };

    move_token(&h, vec![h.start, h.adj]).await;

    // CombatStart's walk ends B's round-1 turn once: 10 -> 7, recorded in
    // the dedup set.
    gm_combat_intent(
        &h,
        crate::ws::protocol::ClientMsg::CombatStart {
            request_id: Uuid::nil(),
            combat_id,
        },
    )
    .await;
    assert_eq!(hp(&h).await, 7.0, "turn_end fired once at start");

    // The rewind (rewind_restore resolves true from the engine-fallback
    // chain) restores the combatant's record-captured hp — 10, since the
    // trigger fired only post-commit — and crosses NO boundary itself, so it
    // fires nothing.
    gm_combat_intent(
        &h,
        crate::ws::protocol::ClientMsg::CombatRewind {
            request_id: Uuid::nil(),
            combat_id,
        },
    )
    .await;
    assert_eq!(
        hp(&h).await,
        10.0,
        "the rewind restored the pre-trigger record state and fired nothing itself"
    );

    // Re-advancing re-crosses B's round-1 turn_end. With the dedup entry
    // cleared by the committed rewind, the trigger re-fires: 10 -> 7.
    // WITHOUT the clear the retained key would suppress it (hp would read
    // 10) while recovery re-runs regardless.
    gm_combat_intent(
        &h,
        crate::ws::protocol::ClientMsg::CombatAdvance {
            request_id: Uuid::nil(),
            combat_id,
        },
    )
    .await;
    assert_eq!(
        hp(&h).await,
        7.0,
        "the re-crossed round-1 turn_end re-fired after the rewind cleared the dedup entry"
    );
}
