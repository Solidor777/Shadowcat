use super::*;
use crate::data::command::Operation;

#[test]
fn start_initializes_round_one_and_runs_turn_start_recovery() {
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0xA, None, false, (0.0, 30.0));
    let b = actor_combatant(11, combat, 0xB, None, false, (0.0, 30.0));
    let snap = snapshot(
        combat_engine(vec![a.doc.id, b.doc.id], None, 0, false),
        vec![a, b],
        vec![],
    );
    let ops = start(&snap, 0, WORLD, Uuid::from_u128(0x6E)).unwrap();
    let docs = apply(&snap, &ops);
    let c: CombatEngine = engine_of(&docs, combat);
    assert!(c.active && c.round == 1 && c.turn == Some(Uuid::from_u128(10)));
    let first: CombatantEngine = engine_of(&docs, Uuid::from_u128(10));
    assert_eq!(
        first.resources["movement"].current, 30.0,
        "turn_start recovery applied and clamped to max"
    );
    let second: CombatantEngine = engine_of(&docs, Uuid::from_u128(11));
    assert_eq!(second.resources["movement"].current, 0.0);
    assert!(
        ops.iter()
            .any(|o| matches!(o, Operation::Create { doc } if doc.doc_type == "combat-history")),
        "first history record"
    );
}

#[test]
fn start_with_a_turn_resumes_without_resnapshot_or_recovery() {
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0xA, None, false, (5.0, 30.0));
    let mut engine = combat_engine(vec![a.doc.id], Some(a.doc.id), 3, false);
    engine.movement.enforcement = Enforcement::Warn;
    let snap = CombatSnapshot {
        chain: (
            None,
            Some(CombatDefaults {
                enforcement: Some(Enforcement::None),
                ..Default::default()
            }),
            None,
        ),
        ..snapshot(engine, vec![a], vec![])
    };
    let ops = start(&snap, 0, WORLD, Uuid::nil()).unwrap();
    let docs = apply(&snap, &ops);
    let c: CombatEngine = engine_of(&docs, combat);
    assert!(c.active && c.round == 3 && c.movement.enforcement == Enforcement::Warn);
    let a: CombatantEngine = engine_of(&docs, Uuid::from_u128(10));
    assert_eq!(a.resources["movement"].current, 5.0);
}

#[test]
fn start_pauses_the_other_active_combat_on_the_scene() {
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0xA, None, false, (0.0, 30.0));
    let other = doc(
        2,
        "combat",
        None,
        serde_json::to_value(combat_engine(vec![], None, 2, true)).unwrap(),
    );
    let snap = CombatSnapshot {
        other_active: vec![other.clone()],
        ..snapshot(
            combat_engine(vec![a.doc.id], None, 0, false),
            vec![a],
            vec![],
        )
    };
    let ops = start(&snap, 0, WORLD, Uuid::nil()).unwrap();
    let deactivate = ops.iter().find(|o| matches!(o, Operation::Update { doc_id, changes } if *doc_id == other.id && changes.iter().any(|c| c.path == "/engine/active" && c.new == serde_json::json!(false))));
    assert!(deactivate.is_some());
    let idx_off = ops
        .iter()
        .position(|o| matches!(o, Operation::Update { doc_id, .. } if *doc_id == other.id))
        .unwrap();
    let idx_on = ops
        .iter()
        .position(|o| matches!(o, Operation::Update { doc_id, .. } if *doc_id == combat))
        .unwrap();
    assert!(
        idx_off < idx_on,
        "release before claim so the batch gate sees the release first"
    );
}

#[test]
fn advance_moves_to_the_next_combatant_and_wraps_the_round_with_round_recoveries() {
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0xA, None, false, (10.0, 30.0));
    let b = actor_combatant(11, combat, 0xB, None, false, (0.0, 30.0));
    let snap = snapshot(
        combat_engine(vec![a.doc.id, b.doc.id], Some(a.doc.id), 1, true),
        vec![a, b],
        vec![],
    );
    let docs = apply(&snap, &advance(&snap, WORLD, Uuid::nil(), 0).unwrap());
    let c: CombatEngine = engine_of(&docs, combat);
    assert_eq!((c.round, c.turn), (1, Some(Uuid::from_u128(11))));
    let b: CombatantEngine = engine_of(&docs, Uuid::from_u128(11));
    assert_eq!(
        b.resources["movement"].current, 30.0,
        "turn_start for the new turn owner"
    );
    // Second advance wraps.
    let snap2 = CombatSnapshot {
        engine: c.clone(),
        combat: docs[&combat].clone(),
        combatants: snap
            .combatants
            .iter()
            .map(|x| Combatant {
                doc: docs[&x.doc.id].clone(),
                engine: engine_of(&docs, x.doc.id),
            })
            .collect(),
        ..snap
    };
    let docs2 = apply(&snap2, &advance(&snap2, WORLD, Uuid::nil(), 0).unwrap());
    let c2: CombatEngine = engine_of(&docs2, combat);
    assert_eq!((c2.round, c2.turn), (2, Some(Uuid::from_u128(10))));
}

#[test]
fn text_recoveries_apply_nothing_server_side() {
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0xA, None, false, (0.0, 30.0));
    let mut snap = snapshot(
        combat_engine(vec![a.doc.id], None, 0, false),
        vec![a],
        vec![],
    );
    snap.registry = Some(registry_with_movement(Formula::Text("speed".into())));
    let docs = apply(&snap, &start(&snap, 0, WORLD, Uuid::nil()).unwrap());
    let a: CombatantEngine = engine_of(&docs, Uuid::from_u128(10));
    assert_eq!(a.resources["movement"].current, 0.0);
}

#[test]
fn hidden_combatant_auto_resolves_under_owner_may_end_and_holds_under_gm_only() {
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0xA, None, false, (0.0, 30.0));
    let h = actor_combatant(11, combat, 0xB, None, true, (0.0, 30.0));
    let c = actor_combatant(12, combat, 0xC, None, false, (0.0, 30.0));
    let snap = snapshot(
        combat_engine(vec![a.doc.id, h.doc.id, c.doc.id], Some(a.doc.id), 1, true),
        vec![a.clone(), h.clone(), c.clone()],
        vec![],
    );
    let docs = apply(&snap, &advance(&snap, WORLD, Uuid::nil(), 0).unwrap());
    let e: CombatEngine = engine_of(&docs, combat);
    assert_eq!(
        e.turn,
        Some(Uuid::from_u128(12)),
        "the hidden turn started and ended inside the command"
    );
    let hidden: CombatantEngine = engine_of(&docs, Uuid::from_u128(11));
    assert_eq!(
        hidden.resources["movement"].current, 30.0,
        "its turn_start recovery still ran"
    );
    let mut gm_only = combat_engine(vec![a.doc.id, h.doc.id, c.doc.id], Some(a.doc.id), 1, true);
    gm_only.turn_control = TurnControl::GmOnly;
    let snap = snapshot(gm_only, vec![a, h, c], vec![]);
    let docs = apply(&snap, &advance(&snap, WORLD, Uuid::nil(), 0).unwrap());
    let e: CombatEngine = engine_of(&docs, combat);
    assert_eq!(e.turn, Some(Uuid::from_u128(11)));
}

#[test]
fn event_posts_its_message_decrements_lifespan_and_is_deleted_at_zero() {
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0xA, None, false, (0.0, 30.0));
    let ev = event_combatant(11, combat, Some(1), Some("Lair action"));
    let snap = snapshot(
        combat_engine(vec![a.doc.id, ev.doc.id], Some(a.doc.id), 1, true),
        vec![a.clone(), ev],
        vec![],
    );
    let ops = advance(&snap, WORLD, Uuid::from_u128(0x6E), 0).unwrap();
    assert!(ops
        .iter()
        .any(|o| matches!(o, Operation::Create { doc } if doc.doc_type == "message")));
    assert!(ops
        .iter()
        .any(|o| matches!(o, Operation::Delete { doc } if doc.id == Uuid::from_u128(11))));
    let docs = apply(&snap, &ops);
    let e: CombatEngine = engine_of(&docs, combat);
    assert_eq!(
        e.turn,
        Some(Uuid::from_u128(10)),
        "wrapped back to a; the event is gone"
    );
    assert_eq!(e.order, vec![Uuid::from_u128(10)]);
    assert_eq!(e.round, 2);
}

#[test]
fn all_events_order_terminates_by_the_loop_guard() {
    let combat = Uuid::from_u128(1);
    let e1 = event_combatant(10, combat, None, None);
    let e2 = event_combatant(11, combat, None, None);
    let snap = snapshot(
        combat_engine(vec![e1.doc.id, e2.doc.id], Some(e1.doc.id), 1, true),
        vec![e1, e2],
        vec![],
    );
    let docs = apply(&snap, &advance(&snap, WORLD, Uuid::nil(), 0).unwrap());
    let e: CombatEngine = engine_of(&docs, combat);
    assert_eq!(e.round, 2);
    assert!(e.turn.is_some());
}

#[test]
fn roll_writes_initiative_rebuilds_order_and_posts_whispers_for_hidden() {
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0xA, None, false, (0.0, 30.0));
    let h = actor_combatant(11, combat, 0xB, None, true, (0.0, 30.0));
    let snap = snapshot(
        combat_engine(vec![a.doc.id, h.doc.id], None, 0, false),
        vec![a, h],
        vec![],
    );
    let post = |total: i64| RollPost::test_with_total("1d20", total);
    let ops = roll(
        &snap,
        &[
            (Uuid::from_u128(10), post(5)),
            (Uuid::from_u128(11), post(17)),
        ],
        WORLD,
        Uuid::from_u128(0x6E),
        "table",
        0,
    )
    .unwrap();
    let msgs: Vec<&Document> = ops
        .iter()
        .filter_map(|o| match o {
            Operation::Create { doc } if doc.doc_type == "message" => Some(doc),
            _ => None,
        })
        .collect();
    assert_eq!(msgs.len(), 2);
    let hidden_msg = msgs
        .iter()
        .find(|m| m.permissions.default == DocRole::None)
        .expect("the hidden combatant's roll is GM-only");
    assert_eq!(hidden_msg.permissions.gm_role, Some(DocRole::Observer));
    let docs = apply(&snap, &ops);
    let e: CombatEngine = engine_of(&docs, combat);
    assert_eq!(e.order, vec![Uuid::from_u128(11), Uuid::from_u128(10)]);
}

#[test]
fn resource_delta_and_set_clamp_to_zero_and_max() {
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0xA, None, false, (10.0, 30.0));
    let snap = snapshot(
        combat_engine(vec![a.doc.id], Some(a.doc.id), 1, true),
        vec![a],
        vec![],
    );
    let docs = apply(
        &snap,
        &resource(
            &snap,
            Uuid::from_u128(10),
            "movement",
            ResourceOp::Delta { amount: -50.0 },
        )
        .unwrap(),
    );
    assert_eq!(
        engine_of::<CombatantEngine>(&docs, Uuid::from_u128(10)).resources["movement"].current,
        0.0
    );
    let docs = apply(
        &snap,
        &resource(
            &snap,
            Uuid::from_u128(10),
            "movement",
            ResourceOp::Set { value: 99.0 },
        )
        .unwrap(),
    );
    assert_eq!(
        engine_of::<CombatantEngine>(&docs, Uuid::from_u128(10)).resources["movement"].current,
        30.0
    );
    assert!(matches!(
        resource(
            &snap,
            Uuid::from_u128(10),
            "actions",
            ResourceOp::Set { value: 1.0 }
        ),
        Err(CombatError::NotFound)
    ));
    assert!(matches!(
        resource(
            &snap,
            Uuid::from_u128(10),
            "movement",
            ResourceOp::Set { value: f64::NAN }
        ),
        Err(CombatError::Forbidden)
    ));
}

#[test]
fn sort_orders_by_initiative_then_tiebreak_then_existing_index() {
    let combat = Uuid::from_u128(1);
    let mut a = actor_combatant(10, combat, 0xA, None, false, (0.0, 30.0));
    a.engine.initiative = Some(12.0);
    let mut b = actor_combatant(11, combat, 0xB, None, false, (0.0, 30.0));
    b.engine.initiative = Some(12.0);
    b.engine.tiebreak = 1.0;
    let mut c = actor_combatant(12, combat, 0xC, None, false, (0.0, 30.0));
    c.engine.initiative = None;
    assert_eq!(
        rebuild_order(
            &[a, b, c],
            &[
                Uuid::from_u128(10),
                Uuid::from_u128(11),
                Uuid::from_u128(12)
            ]
        ),
        vec![
            Uuid::from_u128(11),
            Uuid::from_u128(10),
            Uuid::from_u128(12)
        ]
    );
}

#[test]
fn end_expires_cleanup_effects_then_deletes_the_combat() {
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0xA, None, false, (0.0, 30.0));
    let host = actor_with_effect(0xA, None, 3, ExpiryPoint::TurnEnd, DurationUnit::Rounds);
    let snap = snapshot(
        combat_engine(vec![a.doc.id], Some(a.doc.id), 2, true),
        vec![a],
        vec![host],
    );
    let ops = end(&snap).unwrap();
    assert!(ops.iter().any(|o| matches!(o, Operation::Update { doc_id, changes } if *doc_id == Uuid::from_u128(0xA) && changes[0].path == "/embedded/effect/0/engine/active" && changes[0].new == serde_json::json!(false))));
    assert!(matches!(ops.last(), Some(Operation::Delete { doc }) if doc.id == combat));
    let mut off = snap.engine.clone();
    off.effect_cleanup = false;
    let snap_off = CombatSnapshot {
        engine: off.clone(),
        combat: doc(1, "combat", None, serde_json::to_value(&off).unwrap()),
        ..snap
    };
    let ops = end(&snap_off).unwrap();
    assert!(
        !ops.iter().any(
            |o| matches!(o, Operation::Update { doc_id, .. } if *doc_id == Uuid::from_u128(0xA))
        ),
        "master switch off ⇒ no cleanup"
    );
}

#[test]
fn pause_only_clears_active() {
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0xA, None, false, (0.0, 30.0));
    let snap = snapshot(
        combat_engine(vec![a.doc.id], Some(a.doc.id), 2, true),
        vec![a],
        vec![],
    );
    let ops = pause(&snap).unwrap();
    assert_eq!(ops.len(), 1);
    assert!(
        matches!(&ops[0], Operation::Update { changes, .. } if changes.len() == 1 && changes[0].path == "/engine/active")
    );
}
