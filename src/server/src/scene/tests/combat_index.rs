//! `SceneEcs::active_combat_for_scene` / `SceneEcs::combatant_for_token`: the combat-family
//! side-table hydration and the token→combatant lookup the movement-budget gate reads.
use super::*;
use crate::data::document::{DocRole, Visibility, WorldRole};
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
        "initiative": null, "tiebreak": 0.0, "resources": { "movement": { "current": 30.0 } } }),
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

/// SceneEcs::resolved_combats: GM sees every combatant with resolved numbers; a player sees
/// only their own combatant numbers and the hidden NPC is absent entirely; an unparseable
/// formula surfaces as error: Some(..) with current/max both None; a materialized stored
/// value is honored; Spaces yields movement_cells == current; a missing grid.distance under
/// PerCell yields movement_cells: None; a paused combat is still reported; two computations
/// over the same ECS state are equal (fingerprint stability).
mod resolved_combats {
    use super::*;

    /// A registry with a Tracked movement resource (max: "speed", PerCell scale) and a
    /// Mirror hp resource (value: "hp"), and one combat (active) on scene with three
    /// combatants: a GM-owned NPC (npc), a player-owned PC (pc, actor system.speed = 30,
    /// system.hp = 12), and a GM-only hidden NPC (hidden). scene grid.distance.per_cell is 5.
    struct Fixture {
        ecs: SceneEcs,
        scene: Uuid,
        pc_combatant: Uuid,
        npc_combatant: Uuid,
        hidden_combatant: Uuid,
        player_ctx: PermissionContext,
    }

    fn build() -> Fixture {
        let scene = Uuid::from_u128(1);
        let player = Uuid::from_u128(0x77);
        let mut ecs = SceneEcs::new();
        ecs.insert_scene_for_test(
            scene,
            json!({ "grid": { "kind": "square", "size": 100, "distance": { "perCell": 5.0, "unit": "ft" } }, "background": null }),
        );
        ecs.apply_op(&Operation::Create {
            doc: entity_doc_top_eng(
                0x50,
                "resource-registry",
                json!({ "resources": {
                    "movement": { "name": "Movement", "order": 0, "binding": {
                        "kind": "tracked", "max": "speed",
                        "recover": { "turn_start": 0, "turn_end": 0, "round_start": 0, "round_end": 0 } } },
                    "hp": { "name": "HP", "order": 1, "binding": { "kind": "mirror", "value": "hp" } }
                } }),
            ),
        });

        let mut pc_actor = entity_doc_top_eng(0xA0, "actor", actor_body(json!([])));
        pc_actor.system = json!({ "speed": 30.0, "hp": 12.0 });
        ecs.apply_op(&Operation::Create { doc: pc_actor });

        let pc_token = entity_doc_eng(
            0xB0,
            1,
            "token",
            json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0, "actor_id": Uuid::from_u128(0xA0) }),
        );
        ecs.apply_op(&Operation::Create { doc: pc_token });

        let mut combat_doc = entity_doc_top_eng(
            3,
            "combat",
            combat_body(
                scene,
                true,
                vec![Uuid::from_u128(4), Uuid::from_u128(5), Uuid::from_u128(6)],
            ),
        );
        combat_doc.permissions.default = DocRole::Observer;
        ecs.apply_op(&Operation::Create { doc: combat_doc });

        // pc: owned by the player, readable, resources visible.
        let mut pc = entity_doc_eng(
            4,
            3,
            "combatant",
            json!({ "kind": { "type": "actor", "token_id": Uuid::from_u128(0xB0), "actor_id": Uuid::from_u128(0xA0) },
                "initiative": null, "tiebreak": 0.0, "resources": {} }),
        );
        pc.owner = Some(player);
        pc.permissions.default = DocRole::Observer;
        pc.permissions.users.insert(player, DocRole::Owner);
        // Mirrors the owner_or_gm stamp `apply_intent` ingress applies to a real Create with no
        // explicit `/engine/resources` override — this raw ECS fixture bypasses that ingress
        // stamp, so the property override is set explicitly here to model a real persisted doc.
        pc.permissions
            .property_overrides
            .insert("/engine/resources".to_string(), Visibility::OwnerOrGm);
        ecs.apply_op(&Operation::Create { doc: pc });

        // npc: GM-owned, but readable by everyone (observer default) -- no actor host, so its
        // resources evaluate against a NoHostResolver (an unparseable/unresolvable formula).
        let mut npc = entity_doc_eng(
            5,
            3,
            "combatant",
            json!({ "kind": { "type": "actor", "token_id": null, "actor_id": null },
                "initiative": null, "tiebreak": 0.0, "resources": {} }),
        );
        npc.owner = None;
        npc.permissions.default = DocRole::Observer;
        npc.permissions
            .property_overrides
            .insert("/engine/resources".to_string(), Visibility::OwnerOrGm);
        ecs.apply_op(&Operation::Create { doc: npc });

        // hidden: GM-only.
        let mut hidden = entity_doc_eng(
            6,
            3,
            "combatant",
            json!({ "kind": { "type": "actor", "token_id": null, "actor_id": null },
                "initiative": null, "tiebreak": 0.0, "resources": {} }),
        );
        hidden.owner = None;
        hidden.permissions.default = DocRole::None;
        hidden
            .permissions
            .property_overrides
            .insert("/engine/resources".to_string(), Visibility::OwnerOrGm);
        ecs.apply_op(&Operation::Create { doc: hidden });

        Fixture {
            ecs,
            scene,
            pc_combatant: Uuid::from_u128(4),
            npc_combatant: Uuid::from_u128(5),
            hidden_combatant: Uuid::from_u128(6),
            player_ctx: PermissionContext {
                user_id: player,
                world_role: WorldRole::Player,
            },
        }
    }

    #[test]
    fn gm_sees_every_combatant_with_resolved_numbers() {
        let f = build();
        let payload = f.ecs.resolved_combats(&gm_ctx(), &WorldCapDefaults::default());
        assert_eq!(payload.combats.len(), 1);
        let combat = &payload.combats[0];
        assert_eq!(combat.scene_id, f.scene);
        assert_eq!(combat.combatants.len(), 3, "the GM sees pc, npc, and the hidden combatant");

        let pc = combat
            .combatants
            .iter()
            .find(|c| c.id == f.pc_combatant)
            .unwrap();
        let resources = pc.resources.as_ref().expect("pc resources visible to the GM");
        let movement = &resources["movement"];
        assert_eq!(movement.current, Some(30.0));
        assert_eq!(movement.max, Some(30.0));
        assert!(movement.error.is_none());
        let hp = &resources["hp"];
        assert_eq!(hp.current, Some(12.0));
        assert_eq!(hp.max, Some(12.0));
        assert_eq!(pc.movement_cells, Some(6.0), "30 movement divided by 5 per_cell is 6 cells");

        let hidden = combat
            .combatants
            .iter()
            .find(|c| c.id == f.hidden_combatant)
            .expect("the GM sees the hidden combatant");
        assert!(hidden.resources.is_some());
    }

    #[test]
    fn player_sees_own_numbers_and_the_hidden_combatant_is_absent() {
        let f = build();
        let payload = f
            .ecs
            .resolved_combats(&f.player_ctx, &WorldCapDefaults::default());
        let combat = &payload.combats[0];
        assert_eq!(
            combat.combatants.len(),
            2,
            "the hidden combatant is absent for a player"
        );
        assert!(!combat
            .combatants
            .iter()
            .any(|c| c.id == f.hidden_combatant));

        let pc = combat
            .combatants
            .iter()
            .find(|c| c.id == f.pc_combatant)
            .unwrap();
        assert_eq!(pc.resources.as_ref().unwrap()["movement"].current, Some(30.0));

        let npc = combat
            .combatants
            .iter()
            .find(|c| c.id == f.npc_combatant)
            .unwrap();
        assert!(
            npc.resources.is_none(),
            "the NPC is observer-visible but its /engine/resources band is owner_or_gm by default"
        );
        assert!(npc.movement_cells.is_none());
    }

    #[test]
    fn an_unresolvable_formula_yields_an_error_with_no_numbers() {
        let f = build();
        // The npc combatant has no host document, so its Tracked movement.max: "speed"
        // formula evaluates against NoHostResolver and fails with an unknown-reference error.
        let payload = f.ecs.resolved_combats(&gm_ctx(), &WorldCapDefaults::default());
        let combat = &payload.combats[0];
        let npc = combat
            .combatants
            .iter()
            .find(|c| c.id == f.npc_combatant)
            .unwrap();
        let movement = &npc.resources.as_ref().unwrap()["movement"];
        assert!(movement.current.is_none());
        assert!(movement.max.is_none());
        assert!(movement.error.is_some());
    }

    #[test]
    fn a_materialized_stored_value_is_honored() {
        let mut f = build();
        f.ecs.apply_op(&Operation::Update {
            doc_id: f.pc_combatant,
            changes: vec![fc("/engine/resources/movement", json!({ "current": 10.0 }))],
        });
        let payload = f.ecs.resolved_combats(&gm_ctx(), &WorldCapDefaults::default());
        let pc = payload.combats[0]
            .combatants
            .iter()
            .find(|c| c.id == f.pc_combatant)
            .unwrap();
        let movement = &pc.resources.as_ref().unwrap()["movement"];
        assert_eq!(movement.current, Some(10.0));
        assert_eq!(movement.max, Some(30.0));
        assert_eq!(pc.movement_cells, Some(2.0), "10 divided by 5 per_cell is 2 cells");
    }

    #[test]
    fn spaces_interpretation_yields_movement_cells_equal_to_current() {
        let mut f = build();
        let mut engine: eng::CombatEngine = serde_json::from_value(
            f.ecs
                .active_combat_for_scene(f.scene)
                .map(|(_, e)| serde_json::to_value(e).unwrap())
                .unwrap(),
        )
        .unwrap();
        engine.movement.interpretation = eng::Interpretation::Spaces;
        f.ecs.apply_op(&Operation::Update {
            doc_id: Uuid::from_u128(3),
            changes: vec![fc("/engine", serde_json::to_value(&engine).unwrap())],
        });
        let payload = f.ecs.resolved_combats(&gm_ctx(), &WorldCapDefaults::default());
        let pc = payload.combats[0]
            .combatants
            .iter()
            .find(|c| c.id == f.pc_combatant)
            .unwrap();
        assert_eq!(pc.movement_cells, Some(30.0));
    }

    #[test]
    fn no_grid_distance_yields_movement_cells_none_under_per_cell() {
        let mut f = build();
        f.ecs.insert_scene_for_test(
            f.scene,
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null }),
        );
        let payload = f.ecs.resolved_combats(&gm_ctx(), &WorldCapDefaults::default());
        let pc = payload.combats[0]
            .combatants
            .iter()
            .find(|c| c.id == f.pc_combatant)
            .unwrap();
        assert!(pc.movement_cells.is_none());
    }

    #[test]
    fn a_paused_combat_is_still_reported() {
        let mut f = build();
        f.ecs.apply_op(&Operation::Update {
            doc_id: Uuid::from_u128(3),
            changes: vec![fc("/engine/active", json!(false))],
        });
        let payload = f.ecs.resolved_combats(&gm_ctx(), &WorldCapDefaults::default());
        assert_eq!(payload.combats.len(), 1, "a paused combat is still readable");
    }

    #[test]
    fn the_payload_is_stable_across_two_computations() {
        let f = build();
        let a = f.ecs.resolved_combats(&gm_ctx(), &WorldCapDefaults::default());
        let b = f.ecs.resolved_combats(&gm_ctx(), &WorldCapDefaults::default());
        assert_eq!(a, b);
    }

    /// Anti-drift parity: movement_cells for the turn owner equals the Hard budget-gate
    /// ceiling ws::room::resolve_budget computes for the same token -- both derive through the
    /// shared ws::room::resource_cells helper.
    #[test]
    fn movement_cells_matches_the_hard_budget_gate_ceiling_for_the_turn_owner() {
        let f = build();
        let payload = f.ecs.resolved_combats(&gm_ctx(), &WorldCapDefaults::default());
        let pc = payload.combats[0]
            .combatants
            .iter()
            .find(|c| c.id == f.pc_combatant)
            .unwrap();

        let bg = crate::ws::room::budget_gate_for_token(
            &f.ecs,
            f.scene,
            Uuid::from_u128(0xB0),
            &f.player_ctx,
            &WorldCapDefaults::default(),
        )
        .expect("the gate resolves for the pc token");
        let crate::ws::room::BudgetResolution::Resolved { budget_cells, .. } =
            crate::ws::room::resolve_budget(&bg, false)
        else {
            panic!("expected a resolved budget");
        };
        assert_eq!(
            pc.movement_cells, budget_cells,
            "the channel movement_cells must equal the Hard gate ceiling for the turn owner"
        );
    }
}
