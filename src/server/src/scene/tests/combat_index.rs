//! `SceneEcs::active_combat_for_scene` / `SceneEcs::combatant_for_token`: the combat-family
//! side-table hydration and the token→combatant lookup the movement-budget gate reads.
use super::*;
use crate::data::document::{DocRole, WorldRole};
use crate::data::membership::PermissionContext;
use crate::data::permission::cap;

/// The world's GM. `combatant_for_token` resolves whole-document `cap::READ` through the shared
/// `resolve_access_world` authority, so the lookup tests read as a GM — whose access is
/// unconditional — and the readability rules get their own test below.
fn gm_ctx() -> PermissionContext {
    PermissionContext {
        user_id: Uuid::from_u128(0xF1),
        world_role: WorldRole::Gm,
    }
}

/// A structurally-complete `CombatEngine` body: `scene`, `active`, and `order` are the fields
/// each test varies; the rest are fixed, valid values.
fn combat_body(scene: Uuid, active: bool, order: Vec<Uuid>) -> serde_json::Value {
    json!({ "scene_id": scene, "active": active, "round": 1, "turn": order.first(), "turn_control": "owner_may_end", "order": order,
        "movement": { "resource": "movement", "interpretation": "per_cell", "enforcement": "hard" },
        "effect_cleanup": true, "rewind_restore": true, "forward_restore": false, "effect_lifecycle": {} })
}

/// The `Access` this lookup returns is the shared whole-document `cap::READ` decision, so a
/// `permissions.users` entry moves it in BOTH directions — the two cases a
/// `permissions.default`-only predicate gets wrong. A per-user grant on a `default: none`
/// combatant reads as READABLE (the caller genuinely receives that document at egress, so the
/// budget gate owes them its enforcement); a per-user `None` override on a `default: observer`
/// combatant reads as UNREADABLE (the caller never receives it, so the gate must not disclose
/// its budget through refusals or truncation).
#[test]
fn combatant_read_access_follows_per_user_permission_entries_in_both_directions() {
    let scene = Uuid::from_u128(1);
    let player = Uuid::from_u128(0x77);
    let player_ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    // `permissions.default` and the per-user entry for `player`, respectively.
    let cases = [
        (DocRole::None, Some(DocRole::Observer), true),
        (DocRole::Observer, Some(DocRole::None), false),
        (DocRole::None, None, false),
        (DocRole::Observer, None, true),
    ];
    for (default, user_entry, expected_read) in cases {
        let mut ecs = SceneEcs::new();
        ecs.insert_scene_for_test(
            scene,
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null }),
        );
        ecs.apply_op(&Operation::Create {
            doc: entity_doc_eng(
                2,
                1,
                "token",
                json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
            ),
        });
        ecs.apply_op(&Operation::Create {
            doc: entity_doc_top_eng(
                3,
                "combat",
                combat_body(scene, true, vec![Uuid::from_u128(4)]),
            ),
        });
        let mut combatant = entity_doc_eng(
            4,
            3,
            "combatant",
            json!({ "kind": { "type": "actor", "token_id": Uuid::from_u128(2), "actor_id": null },
                "initiative": null, "tiebreak": 0.0, "resources": {} }),
        );
        combatant.owner = None;
        combatant.permissions.default = default;
        if let Some(role) = user_entry {
            combatant.permissions.users.insert(player, role);
        }
        ecs.apply_op(&Operation::Create { doc: combatant });

        let (_, _, access) = ecs
            .combatant_for_token(
                Uuid::from_u128(3),
                Uuid::from_u128(2),
                &player_ctx,
                &WorldCapDefaults::default(),
            )
            .expect("combatant");
        assert_eq!(
            access.has(cap::READ),
            expected_read,
            "default {default:?} with per-user entry {user_entry:?}"
        );
    }
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
    let (cid, ce, access) = ecs
        .combatant_for_token(
            id,
            Uuid::from_u128(2),
            &gm_ctx(),
            &WorldCapDefaults::default(),
        )
        .expect("combatant");
    assert_eq!(cid, Uuid::from_u128(4));
    assert_eq!(ce.resources["movement"].current, 30.0);
    assert!(access.has(cap::READ));
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

    let (id, ce, access) = ecs
        .combatant_for_token(
            Uuid::from_u128(3),
            Uuid::from_u128(2),
            &gm_ctx(),
            &WorldCapDefaults::default(),
        )
        .expect("combatant via the token's resolved actor_id");
    assert_eq!(id, Uuid::from_u128(4));
    assert!(matches!(
        ce.kind,
        eng::CombatantKind::Actor {
            actor_id: Some(a),
            ..
        } if a == actor_id
    ));
    assert!(access.has(cap::READ));

    // A token_id match still wins over an actor_id match when both are present — checked by
    // the primary test above (`token_id == Some(token)` short-circuits before any actor_id
    // comparison runs).
    assert!(ecs
        .combatant_for_token(
            Uuid::from_u128(3),
            Uuid::from_u128(999),
            &gm_ctx(),
            &WorldCapDefaults::default()
        )
        .is_none());
}

#[test]
fn combatant_lookup_falls_back_to_an_instanced_tokens_embedded_actor_copy() {
    // INSTANCED token: `engine.actor_id` is absent (never present at all — `token_engine`'s
    // resolution reads `TokenEngine.actor_id` first and only falls back to the embedded copy's
    // id when that read is `None`), and the token carries its own `embedded.actor[0]` copy
    // instead. The combatant's `kind.actor_id` matches that embedded copy's id, not any
    // `engine.actor_id` field on the token (there isn't one).
    let mut ecs = SceneEcs::new();
    let scene = Uuid::from_u128(1);
    ecs.insert_scene_for_test(
        scene,
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null }),
    );
    let actor_id = Uuid::from_u128(0xB);
    let mut actor = entity_doc_top_eng(0xB, "actor", actor_body(json!([])));
    actor.id = actor_id;
    let mut token = entity_doc_eng(
        2,
        1,
        "token",
        json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    token.embedded.insert("actor".into(), vec![actor]);
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

    let (id, ce, access) = ecs
        .combatant_for_token(
            Uuid::from_u128(3),
            Uuid::from_u128(2),
            &gm_ctx(),
            &WorldCapDefaults::default(),
        )
        .expect("combatant via the token's embedded actor copy");
    assert_eq!(id, Uuid::from_u128(4));
    assert!(matches!(
        ce.kind,
        eng::CombatantKind::Actor {
            actor_id: Some(a),
            ..
        } if a == actor_id
    ));
    assert!(access.has(cap::READ));
}
