//! `Room`'s region-trigger fire sites: move execution (`Room::execute_move`'s post-commit
//! call into `fire_region_triggers`) and placement/teleport (`Room::publish`'s candidate scan
//! into `fire_placement_triggers`), the per-effect-kind application semantics, and the secrecy
//! forcing rule for regions not visible to every world member.

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
    let region_id = Uuid::from_u128(id);
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
    let actor_id = Uuid::from_u128(actor_id);
    let mut actor = wdoc(h.world_id, actor_id, "actor");
    actor.owner = Some(h.gm.user_id);
    actor.system = system;
    actor.engine = Some(json!({
        "displayName": "Goblin", "visual": { "kind": "image", "asset": "a" },
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
        Uuid::from_u128(0x7A7),
        Uuid::from_u128(0x7A8),
        Uuid::from_u128(0x7A9),
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
    let region_id = Uuid::from_u128(0x7AD);
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
    let mut placed = wdoc(h.world_id, Uuid::from_u128(0x7AF), "token");
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
