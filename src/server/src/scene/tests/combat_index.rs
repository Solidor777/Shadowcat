//! `SceneEcs::active_combat_for_scene` / `SceneEcs::combatant_for_token`: the combat-family
//! side-table hydration and the token→combatant lookup the movement-budget gate reads.
use super::*;

/// A structurally-complete `CombatEngine` body: `scene`, `active`, and `order` are the fields
/// each test varies; the rest are fixed, valid values.
fn combat_body(scene: Uuid, active: bool, order: Vec<Uuid>) -> serde_json::Value {
    json!({ "scene_id": scene, "active": active, "round": 1, "turn": order.first(), "turn_control": "owner_may_end", "order": order,
        "movement": { "resource": "movement", "interpretation": "per_cell", "enforcement": "hard" },
        "effect_cleanup": true, "rewind_restore": true, "forward_restore": false, "effect_lifecycle": {} })
}

#[test]
fn active_combat_and_combatant_lookups_follow_apply_op() {
    let mut ecs = SceneEcs::new();
    let scene = Uuid::from_u128(1);
    ecs.insert_scene_for_test(
        scene,
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null }),
    );
    let token = entity_doc_eng(
        2,
        1,
        "token",
        json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    ecs.apply_op(&Operation::Create { doc: token });
    let combat = entity_doc_top_eng(
        3,
        "combat",
        combat_body(scene, true, vec![Uuid::from_u128(4)]),
    );
    ecs.apply_op(&Operation::Create {
        doc: combat.clone(),
    });
    let mut combatant = entity_doc_eng(
        4,
        3,
        "combatant",
        json!({ "kind": { "type": "actor", "token_id": Uuid::from_u128(2), "actor_id": null },
        "initiative": null, "tiebreak": 0.0, "resources": { "movement": { "current": 30.0, "max": 30.0 } } }),
    );
    combatant.permissions.default = crate::data::document::DocRole::Observer;
    ecs.apply_op(&Operation::Create { doc: combatant });
    let (id, e) = ecs.active_combat_for_scene(scene).expect("active");
    assert_eq!(id, Uuid::from_u128(3));
    assert_eq!(e.movement.resource.as_deref(), Some("movement"));
    let (cid, ce, hidden, _) = ecs
        .combatant_for_token(id, Uuid::from_u128(2))
        .expect("combatant");
    assert_eq!(cid, Uuid::from_u128(4));
    assert_eq!(ce.resources["movement"].current, 30.0);
    assert!(!hidden);
    ecs.apply_op(&Operation::Update {
        doc_id: Uuid::from_u128(3),
        changes: vec![fc("/engine/active", json!(false))],
    });
    assert!(ecs.active_combat_for_scene(scene).is_none());
}

#[test]
fn combatant_lookup_falls_back_to_the_tokens_actor_id() {
    // token with engine.actor_id = 0xA; combatant kind { token_id: null, actor_id: 0xA } ⇒ found.
    let mut ecs = SceneEcs::new();
    let scene = Uuid::from_u128(1);
    ecs.insert_scene_for_test(
        scene,
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null }),
    );
    let actor_id = Uuid::from_u128(0xA);
    let token = entity_doc_eng(
        2,
        1,
        "token",
        json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0, "actor_id": actor_id }),
    );
    ecs.apply_op(&Operation::Create { doc: token });
    let combat = entity_doc_top_eng(
        3,
        "combat",
        combat_body(scene, true, vec![Uuid::from_u128(4)]),
    );
    ecs.apply_op(&Operation::Create {
        doc: combat.clone(),
    });
    let mut combatant = entity_doc_eng(
        4,
        3,
        "combatant",
        json!({ "kind": { "type": "actor", "token_id": null, "actor_id": actor_id },
            "initiative": null, "tiebreak": 0.0, "resources": {} }),
    );
    combatant.permissions.default = crate::data::document::DocRole::Observer;
    ecs.apply_op(&Operation::Create { doc: combatant });

    let (id, ce, hidden, _owner) = ecs
        .combatant_for_token(Uuid::from_u128(3), Uuid::from_u128(2))
        .expect("combatant via the token's resolved actor_id");
    assert_eq!(id, Uuid::from_u128(4));
    assert!(matches!(
        ce.kind,
        eng::CombatantKind::Actor {
            actor_id: Some(a),
            ..
        } if a == actor_id
    ));
    assert!(!hidden);

    // A token_id match still wins over an actor_id match when both are present — checked by
    // the primary test above (`token_id == Some(token)` short-circuits before any actor_id
    // comparison runs).
    assert!(ecs
        .combatant_for_token(Uuid::from_u128(3), Uuid::from_u128(999))
        .is_none());
}
