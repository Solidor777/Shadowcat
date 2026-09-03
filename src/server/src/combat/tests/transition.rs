use super::*;
use crate::data::command::Operation;

#[test]
fn start_initializes_round_one_and_runs_turn_start_recovery() {
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0xA, None, false, 0.0);
    let b = actor_combatant(11, combat, 0xB, None, false, 0.0);
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
    let a = actor_combatant(10, combat, 0xA, None, false, 5.0);
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
    let a = actor_combatant(10, combat, 0xA, None, false, 0.0);
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
    let a = actor_combatant(10, combat, 0xA, None, false, 10.0);
    let b = actor_combatant(11, combat, 0xB, None, false, 0.0);
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
fn advance_skips_a_combatant_moved_out_of_the_active_combat() {
    // `order` still names the moved-out combatant's id; `combatants` (a
    // fresh query of the combat's current children) does not — exactly the
    // shape an ordinary `Move` of the `combatant` document produces.
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0xA, None, false, 0.0);
    let departed = Uuid::from_u128(11);
    let c = actor_combatant(12, combat, 0xC, None, false, 0.0);
    let snap = snapshot(
        combat_engine(vec![a.doc.id, departed, c.doc.id], Some(a.doc.id), 1, true),
        vec![a, c],
        vec![],
    );
    let docs = apply(&snap, &advance(&snap, WORLD, Uuid::nil(), 0).unwrap());
    let engine: CombatEngine = engine_of(&docs, combat);
    assert_eq!(
        engine.turn,
        Some(Uuid::from_u128(12)),
        "advance succeeded and landed on the next real combatant, skipping the departed slot"
    );
    assert_eq!(
        engine.order,
        vec![Uuid::from_u128(10), Uuid::from_u128(12)],
        "the departed combatant's id was dropped from the persisted order"
    );
}

#[test]
fn advance_never_auto_admits_an_arrival_but_an_explicit_sort_does() {
    // `c` is a queryable child of the combat (present in `combatants`) that
    // `order` never named — exactly the shape an ordinary `Move` of the
    // `combatant` document into this combat produces. Whether such a
    // combatant ever takes a turn is a GM decision (`CombatSort`/
    // `CombatRoll`), never something `advance` decides on its own: a
    // combatant created without an `order` entry from the start is a
    // genuine, permanent design state (a staged/hidden combatant), and
    // `order` alone cannot distinguish that from a fresh arrival —
    // `heal_order` must not guess.
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0xA, None, false, 0.0);
    let b = actor_combatant(11, combat, 0xB, None, false, 0.0);
    let c = actor_combatant(12, combat, 0xC, None, false, 0.0);
    let snap = snapshot(
        combat_engine(vec![a.doc.id, b.doc.id], Some(a.doc.id), 1, true),
        vec![a, b, c],
        vec![],
    );
    let docs = apply(&snap, &advance(&snap, WORLD, Uuid::nil(), 0).unwrap());
    let engine: CombatEngine = engine_of(&docs, combat);
    assert_eq!(
        engine.order,
        vec![Uuid::from_u128(10), Uuid::from_u128(11)],
        "advance leaves the arrival out of order — it is not a departed entry to prune"
    );
    assert_eq!(engine.turn, Some(Uuid::from_u128(11)));

    // The arrival is not invisible forever: an explicit `CombatSort` folds
    // it in deterministically, proving the closure is available on demand
    // rather than automatic.
    let sort_docs = apply(&snap, &sort(&snap).unwrap());
    let sorted: CombatEngine = engine_of(&sort_docs, combat);
    assert!(
        sorted.order.contains(&Uuid::from_u128(12)),
        "an explicit sort admits the arrival"
    );
}

#[test]
fn a_stale_order_entry_heals_on_the_next_transition_even_when_the_walk_never_reaches_it() {
    // The stale entry sits between the current turn and the next real
    // combatant, so this advance's own walk stops one step short of ever
    // reaching its slot — proving the correction is structural (applied at
    // snapshot load) rather than a side effect of the walk passing over it.
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0xA, None, false, 0.0);
    let stale = Uuid::from_u128(99);
    let b = actor_combatant(11, combat, 0xB, None, false, 0.0);
    let snap = snapshot(
        combat_engine(vec![a.doc.id, stale, b.doc.id], Some(a.doc.id), 1, true),
        vec![a, b],
        vec![],
    );
    let docs = apply(&snap, &advance(&snap, WORLD, Uuid::nil(), 0).unwrap());
    let engine: CombatEngine = engine_of(&docs, combat);
    assert_eq!(
        engine.order,
        vec![Uuid::from_u128(10), Uuid::from_u128(11)],
        "the already-inconsistent row healed on this transition's own commit"
    );
}

#[test]
fn text_recoveries_evaluate_server_side_over_the_formula_host() {
    let combat = Uuid::from_u128(1);
    // 0x9A, not 0xA — a host id must never numerically collide with a
    // combatant doc id (`actor_combatant(10, ..)` is id 10 == 0xA).
    let a = actor_combatant(10, combat, 0x9A, None, false, 0.0);
    let mut host = doc(0x9A, "actor", None, actor_body());
    host.system = serde_json::json!({ "speed": 12.0 });
    let mut snap = snapshot(
        combat_engine(vec![a.doc.id], None, 0, false),
        vec![a],
        vec![host],
    );
    snap.registry = Some(registry_with_movement(Formula::Text("speed / 2".into())));
    let docs = apply(&snap, &start(&snap, 0, WORLD, Uuid::nil()).unwrap());
    let a: CombatantEngine = engine_of(&docs, Uuid::from_u128(10));
    assert_eq!(
        a.resources["movement"].current, 6.0,
        "the turn_start recovery evaluated over the host's system band"
    );
}

#[test]
fn a_failing_recovery_formula_applies_nothing_and_posts_one_gm_only_notice() {
    let combat = Uuid::from_u128(1);
    // No host document at all: the text recovery's reference cannot resolve.
    let a = actor_combatant(10, combat, 0xA, None, false, 5.0);
    let mut snap = snapshot(
        combat_engine(vec![a.doc.id], None, 0, false),
        vec![a],
        vec![],
    );
    snap.registry = Some(registry_with_movement(Formula::Text("speed".into())));
    let ops = start(&snap, 0, WORLD, Uuid::nil()).unwrap();
    let notices: Vec<_> = ops
        .iter()
        .filter_map(|o| match o {
            Operation::Create { doc } if doc.doc_type == "message" => Some(doc),
            _ => None,
        })
        .collect();
    assert_eq!(
        notices.len(),
        1,
        "one deduped notice for the whole transition"
    );
    let text = serde_json::to_string(notices[0].engine.as_ref().unwrap()).unwrap();
    assert!(
        text.contains("movement turn_start"),
        "the notice names the failing resource and boundary: {text}"
    );
    let docs = apply(&snap, &ops);
    let a: CombatantEngine = engine_of(&docs, Uuid::from_u128(10));
    assert_eq!(
        a.resources["movement"].current, 5.0,
        "the failing recovery applied nothing"
    );
}

#[test]
fn an_absent_tracked_entry_reads_full_and_materializes_on_first_change() {
    let combat = Uuid::from_u128(1);
    let mut a = actor_combatant(10, combat, 0x9A, None, false, 0.0);
    a.engine.resources.clear();
    a.doc.engine = Some(serde_json::to_value(&a.engine).unwrap());
    let mut registry = registry_with_movement(Formula::Number(0.0));
    if let ResourceBinding::Tracked { recover, .. } =
        &mut registry.resources.get_mut("movement").unwrap().binding
    {
        recover.turn_end = Formula::Number(-10.0);
    }
    let mut snap = snapshot(
        combat_engine(vec![a.doc.id], Some(a.doc.id), 1, true),
        vec![a],
        vec![],
    );
    snap.registry = Some(registry);
    let docs = apply(&snap, &advance(&snap, WORLD, Uuid::nil(), 0).unwrap());
    let e: CombatantEngine = engine_of(&docs, Uuid::from_u128(10));
    assert_eq!(
        e.resources["movement"].current, 20.0,
        "full (the evaluated max, 30) minus the turn_end recovery, materializing the absent entry"
    );
}

#[test]
fn an_event_with_no_stored_entry_recovers_uniformly_like_any_combatant() {
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0x9A, None, false, 0.0);
    // Survives its own resolution (no lifespan) and carries NO resource entry.
    let ev = event_combatant(11, combat, None, None);
    let mut registry = registry_with_movement(Formula::Number(0.0));
    if let ResourceBinding::Tracked { recover, .. } =
        &mut registry.resources.get_mut("movement").unwrap().binding
    {
        recover.turn_end = Formula::Number(-10.0);
    }
    let mut snap = snapshot(
        combat_engine(vec![a.doc.id, ev.doc.id], Some(a.doc.id), 1, true),
        vec![a, ev],
        vec![],
    );
    snap.registry = Some(registry);
    let docs = apply(&snap, &advance(&snap, WORLD, Uuid::nil(), 0).unwrap());
    let e: CombatantEngine = engine_of(&docs, Uuid::from_u128(11));
    assert_eq!(
        e.resources["movement"].current, 20.0,
        "the drain materialized the event's absent entry from full (30) exactly as it would an actor's"
    );
}

#[test]
fn resource_intent_refuses_a_mirror_binding_and_clamps_against_an_evaluated_text_max() {
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0x9A, None, false, 5.0);
    let mut host = doc(0x9A, "actor", None, actor_body());
    host.system = serde_json::json!({ "hp": 27.0, "mv": 8.0 });
    let mut snap = snapshot(
        combat_engine(vec![a.doc.id], Some(a.doc.id), 1, true),
        vec![a],
        vec![host],
    );
    let mut registry = registry_with_movement(Formula::Number(0.0));
    if let Some(r) = registry.resources.get_mut("movement") {
        r.binding = ResourceBinding::Tracked {
            max: Formula::Text("mv".into()),
            recover: Recovery::default(),
        };
    }
    registry.resources.insert(
        "hp".into(),
        Resource {
            name: "HP".into(),
            order: 1,
            binding: ResourceBinding::Mirror {
                value: Formula::Text("hp".into()),
            },
        },
    );
    snap.registry = Some(registry);
    assert!(
        matches!(
            resource(
                &snap,
                Uuid::from_u128(10),
                "hp",
                ResourceOp::Delta { amount: -3.0 }
            ),
            Err(CombatError::Forbidden)
        ),
        "a Mirror number lives on the actor; the clock refuses to write it"
    );
    let ops = resource(
        &snap,
        Uuid::from_u128(10),
        "movement",
        ResourceOp::Set { value: 50.0 },
    )
    .unwrap();
    let docs = apply(&snap, &ops);
    let e: CombatantEngine = engine_of(&docs, Uuid::from_u128(10));
    assert_eq!(
        e.resources["movement"].current, 8.0,
        "clamped to the evaluated text max"
    );
}

#[test]
fn hidden_combatant_auto_resolves_under_owner_may_end_and_holds_under_gm_only() {
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0xA, None, false, 0.0);
    let h = actor_combatant(11, combat, 0xB, None, true, 0.0);
    let c = actor_combatant(12, combat, 0xC, None, false, 0.0);
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
    let a = actor_combatant(10, combat, 0xA, None, false, 0.0);
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
    let a = actor_combatant(10, combat, 0xA, None, false, 0.0);
    let h = actor_combatant(11, combat, 0xB, None, true, 0.0);
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
    let a = actor_combatant(10, combat, 0xA, None, false, 10.0);
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
    let mut a = actor_combatant(10, combat, 0xA, None, false, 0.0);
    a.engine.initiative = Some(12.0);
    let mut b = actor_combatant(11, combat, 0xB, None, false, 0.0);
    b.engine.initiative = Some(12.0);
    b.engine.tiebreak = 1.0;
    let mut c = actor_combatant(12, combat, 0xC, None, false, 0.0);
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
    let a = actor_combatant(10, combat, 0xA, None, false, 0.0);
    let host = actor_with_effect(0xA, None, 3, ExpiryPoint::TurnEnd, DurationUnit::Rounds);
    let snap = snapshot(
        combat_engine(vec![a.doc.id], Some(a.doc.id), 2, true),
        vec![a],
        vec![host],
    );
    let ops = end(&snap, WORLD, Uuid::nil(), 0).unwrap();
    assert!(ops.iter().any(|o| matches!(o, Operation::Update { doc_id, changes } if *doc_id == Uuid::from_u128(0xA) && changes[0].path == "/embedded/effect/0/engine/active" && changes[0].new == serde_json::json!(false))));
    assert!(matches!(ops.last(), Some(Operation::Delete { doc }) if doc.id == combat));
    let mut off = snap.engine.clone();
    off.effect_cleanup = false;
    let snap_off = CombatSnapshot {
        engine: off.clone(),
        combat: doc(1, "combat", None, serde_json::to_value(&off).unwrap()),
        ..snap
    };
    let ops = end(&snap_off, WORLD, Uuid::nil(), 0).unwrap();
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
    let a = actor_combatant(10, combat, 0xA, None, false, 0.0);
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

#[test]
fn round_wrap_ticks_a_shared_unanchored_effect_at_most_once() {
    // a and b share the SAME host (both link to actor 0x10). Its unanchored
    // effect is collected once per combatant, but a round-wrap sweep must
    // still tick it exactly once, not once per combatant that shares it.
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0x10, None, false, 0.0);
    let b = actor_combatant(11, combat, 0x10, None, false, 0.0);
    let host = actor_with_effect(0x10, None, 2, ExpiryPoint::RoundStart, DurationUnit::Rounds);
    // b is the current turn; ending it wraps the round straight into round_wrap.
    let snap = snapshot(
        combat_engine(vec![a.doc.id, b.doc.id], Some(b.doc.id), 1, true),
        vec![a, b],
        vec![host],
    );
    let docs = apply(&snap, &advance(&snap, WORLD, Uuid::nil(), 0).unwrap());
    let c: CombatEngine = engine_of(&docs, combat);
    assert_eq!(c.round, 2, "the round wrapped exactly once");
    let effect: EffectEngine = serde_json::from_value(
        docs[&Uuid::from_u128(0x10)].embedded["effect"][0]
            .engine
            .clone()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        effect.duration.unwrap().remaining,
        Some(1),
        "decremented by exactly one round boundary, not once per sharing combatant"
    );
}

/// An `Event` gets the SAME start-and-end boundary pair every other entry
/// the walk touches gets — symmetric with the hidden-actor branch, and what
/// lets an `Event` carry a tracked resource that recovers at either edge of
/// its own turn.
#[test]
fn a_surviving_event_runs_its_own_turn_end_boundary_not_just_turn_start() {
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0xA, None, false, 0.0);
    // lifespan None: the event survives its own resolution, so there IS a
    // document left for the turn_end boundary to write.
    let mut ev = event_combatant(11, combat, None, None);
    ev.engine
        .resources
        .insert("movement".to_string(), CombatantResource { current: 20.0 });
    ev.doc.engine = Some(serde_json::to_value(&ev.engine).unwrap());
    let mut registry = registry_with_movement(Formula::Number(30.0));
    if let ResourceBinding::Tracked { recover, .. } =
        &mut registry.resources.get_mut("movement").unwrap().binding
    {
        recover.turn_end = Formula::Number(-10.0);
    }
    let mut snap = snapshot(
        combat_engine(vec![a.doc.id, ev.doc.id], Some(a.doc.id), 1, true),
        vec![a, ev],
        vec![],
    );
    snap.registry = Some(registry);
    let docs = apply(&snap, &advance(&snap, WORLD, Uuid::nil(), 0).unwrap());
    let e: CombatantEngine = engine_of(&docs, Uuid::from_u128(11));
    assert_eq!(
        e.resources["movement"].current, 20.0,
        "turn_start's +30 (clamped to max 30) THEN turn_end's -10 — 30 would mean turn_end never ran"
    );
}

/// Every turn boundary the walk crosses is recorded, not just the one it
/// settles on: an auto-resolving hidden entry gets its own record, so a
/// rewind can land on it.
#[test]
fn an_auto_resolved_hidden_entry_gets_its_own_history_record() {
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0xA, None, false, 0.0);
    let h = actor_combatant(11, combat, 0xB, None, true, 0.0);
    let c = actor_combatant(12, combat, 0xC, None, false, 0.0);
    let snap = snapshot(
        combat_engine(vec![a.doc.id, h.doc.id, c.doc.id], Some(a.doc.id), 1, true),
        vec![a, h, c],
        vec![],
    );
    let docs = apply(&snap, &advance(&snap, WORLD, Uuid::nil(), 0).unwrap());
    let history = docs
        .values()
        .find(|d| d.doc_type == "combat-history")
        .expect("the advance created the history document");
    let engine: CombatHistoryEngine =
        serde_json::from_value(history.engine.clone().unwrap()).unwrap();
    assert_eq!(
        engine.records.iter().map(|r| r.turn).collect::<Vec<Uuid>>(),
        vec![Uuid::from_u128(11), Uuid::from_u128(12)],
        "the hidden entry the walk auto-resolved past has its own record before the settled one"
    );
    assert_eq!(engine.cursor as usize, engine.records.len() - 1);
    // One write to the history document for the whole transition, however
    // many boundaries it crossed — the OCC contract, not an accident.
    let ops = advance(&snap, WORLD, Uuid::nil(), 0).unwrap();
    assert_eq!(
        ops.iter()
            .filter(|o| matches!(o, Operation::Create { doc } if doc.doc_type == "combat-history"))
            .count(),
        1,
        "exactly one history write per transition"
    );
}

#[test]
fn guard_exhaustion_still_fully_resolves_the_final_auto_resolving_entry() {
    // order = [a, b], both hidden actors under OwnerMayEnd. Starting
    // advance() from a: the walk visits b (fully resolves it, wraps the
    // round) then lands back on a at the guard's last iteration — a must
    // ALSO be fully resolved (its own turn_start AND turn_end both run),
    // never just parked mid-turn.
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0xA, None, true, 0.0);
    let b = actor_combatant(11, combat, 0xB, None, true, 0.0);
    let mut registry = registry_with_movement(Formula::Number(30.0));
    if let ResourceBinding::Tracked { recover, .. } =
        &mut registry.resources.get_mut("movement").unwrap().binding
    {
        recover.turn_end = Formula::Number(-10.0);
    }
    let mut snap = snapshot(
        combat_engine(vec![a.doc.id, b.doc.id], Some(a.doc.id), 1, true),
        vec![a, b],
        vec![],
    );
    snap.registry = Some(registry);
    let docs = apply(&snap, &advance(&snap, WORLD, Uuid::nil(), 0).unwrap());
    let e: CombatEngine = engine_of(&docs, combat);
    assert_eq!(e.round, 2, "the walk wrapped exactly once");
    assert_eq!(
        e.turn,
        Some(Uuid::from_u128(10)),
        "parked on the last-visited entry, per the all-auto-resolving termination case"
    );
    // Both entries must show turn_start's +30 THEN turn_end's -10: 20, not
    // 30 (turn_end skipped) and not 0 (turn_start skipped).
    let a: CombatantEngine = engine_of(&docs, Uuid::from_u128(10));
    assert_eq!(
        a.resources["movement"].current, 20.0,
        "a is the guard-exhaustion entry — its own turn_end must still have run"
    );
    let b: CombatantEngine = engine_of(&docs, Uuid::from_u128(11));
    assert_eq!(
        b.resources["movement"].current, 20.0,
        "b was fully resolved mid-walk"
    );
}

#[test]
fn auto_resolved_hidden_turn_coalesces_host_updates_into_one_operation() {
    // a is the current (visible) turn; h is a hidden OwnerMayEnd actor
    // encountered next, bound to a host with two effects — one ticking at
    // TurnStart, one at TurnEnd — so h's auto-resolved turn (its `enter_turn`
    // and `run_turn_end` back to back) writes the SAME host document twice.
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0xA, None, false, 0.0);
    let h = actor_combatant(11, combat, 0x50, None, true, 0.0);
    let mut host = actor_with_effect(0x50, None, 1, ExpiryPoint::TurnStart, DurationUnit::Turns);
    let turn_end_effect =
        actor_with_effect(0x51, None, 1, ExpiryPoint::TurnEnd, DurationUnit::Turns);
    host.embedded
        .get_mut("effect")
        .unwrap()
        .push(turn_end_effect.embedded["effect"][0].clone());
    let snap = snapshot(
        combat_engine(vec![a.doc.id, h.doc.id], Some(a.doc.id), 1, true),
        vec![a, h],
        vec![host],
    );
    let ops = advance(&snap, WORLD, Uuid::nil(), 0).unwrap();
    let host_updates: Vec<&Operation> = ops
        .iter()
        .filter(
            |o| matches!(o, Operation::Update { doc_id, .. } if *doc_id == Uuid::from_u128(0x50)),
        )
        .collect();
    assert_eq!(
        host_updates.len(),
        1,
        "both boundary passes' writes to the SAME host must coalesce into one Operation::Update"
    );
    let Operation::Update { changes, .. } = host_updates[0] else {
        unreachable!()
    };
    assert!(
        changes.len() >= 2,
        "both effects' field changes are present in the single coalesced Update"
    );
}

#[test]
fn auto_resolved_hidden_turn_coalesces_combatant_updates_into_one_operation() {
    // h is a hidden OwnerMayEnd actor encountered right after a (visible,
    // current turn) ends. h's OWN resource recovers at BOTH boundaries of
    // its auto-resolved turn (turn_start +30, turn_end -10, the exact same
    // configuration `guard_exhaustion_still_fully_resolves_...` uses) --
    // two separate `run_boundary` calls write the SAME path on h's own
    // combatant document. `Working::coalesce_updates` covers combatant
    // documents identically to host documents, not just hosts.
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0xA, None, false, 0.0);
    let h = actor_combatant(11, combat, 0xB, None, true, 0.0);
    let mut registry = registry_with_movement(Formula::Number(30.0));
    if let ResourceBinding::Tracked { recover, .. } =
        &mut registry.resources.get_mut("movement").unwrap().binding
    {
        recover.turn_end = Formula::Number(-10.0);
    }
    let mut snap = snapshot(
        combat_engine(vec![a.doc.id, h.doc.id], Some(a.doc.id), 1, true),
        vec![a, h],
        vec![],
    );
    snap.registry = Some(registry);
    let ops = advance(&snap, WORLD, Uuid::nil(), 0).unwrap();
    let combatant_updates: Vec<&Operation> = ops
        .iter()
        .filter(|o| matches!(o, Operation::Update { doc_id, .. } if *doc_id == Uuid::from_u128(11)))
        .collect();
    assert_eq!(
        combatant_updates.len(),
        1,
        "both boundary passes' writes to h's own document must coalesce into one Operation::Update"
    );
    let Operation::Update { changes, .. } = combatant_updates[0] else {
        unreachable!()
    };
    let movement_change = changes
        .iter()
        .find(|c| c.path == "/engine/resources/movement/current")
        .expect("the merged FieldChange for h's movement resource is present");
    assert_eq!(
        movement_change.old,
        serde_json::json!(0.0),
        "old is the TRUE pre-transition value, not turn_start's intermediate 30.0"
    );
    assert_eq!(
        movement_change.new,
        serde_json::json!(20.0),
        "new is the cumulative result of turn_start(+30) then turn_end(-10)"
    );
    // `apply`'s own strict OCC check is itself proof this batch is
    // well-formed relative to the batch-start document.
    let docs = apply(&snap, &ops);
    let h_engine: CombatantEngine = engine_of(&docs, Uuid::from_u128(11));
    assert_eq!(h_engine.resources["movement"].current, 20.0);
}

#[test]
fn settle_turn_step_budget_stays_linear_even_with_many_sequential_event_removals() {
    // `advance()`'s own real entry point never starts `settle_turn` at
    // idx = 0: `advance_impl` runs the CURRENT turn's own `run_turn_end`
    // OUTSIDE `settle_turn` first, so the walk always begins at idx = 1
    // (one past `order[0]`, the entry already holding `turn`) -- only
    // `start()` enters at idx = 0. Lifespans are the descending-lifespan
    // construction ROTATED to start at position 1: E_0's lifespan is `1`
    // (it is visited LAST in the walk's first lap, once it wraps back
    // around to position 0), each of E_2..E_{N-1} descends by one further
    // position, and E_1 -- which would otherwise need `N` decrements and
    // be the very last entry removed -- carries an unbounded (`None`)
    // lifespan instead, so the whole order never fully empties (which
    // would surface as `CombatError::Empty` rather than a step count).
    // Every full lap over the CURRENT order decrements every remaining
    // finite-lifespan entry by exactly one, so the entry visited LAST in
    // that lap is always the one (other than E_1) that reaches zero,
    // right on the lap's own final step -- the shape a full-length budget
    // reset on every removal drives to O(N^2) total steps: N + (N-1) +
    // ... + 2, plus one final step once E_1 alone remains and stops the
    // walk. `settle_turn`'s budget instead grows by only ONE step per
    // removal, bounding the whole walk to a small multiple of N.
    const N: u32 = 30;
    let combat = Uuid::from_u128(1);
    let mut combatants = Vec::new();
    let mut order = Vec::new();
    for i in 0..N {
        let lifespan = if i == 0 {
            Some(1)
        } else if i == 1 {
            None
        } else {
            Some(N - i + 1)
        };
        let ev = event_combatant(100 + i as u128, combat, lifespan, None);
        order.push(ev.doc.id);
        combatants.push(ev);
    }
    let first = order[0];
    let snap = snapshot(
        combat_engine(order.clone(), Some(first), 1, true),
        combatants,
        vec![],
    );
    let (ops, steps) = advance_with_step_count(&snap, WORLD, Uuid::nil(), 0).unwrap();
    assert!(
        steps <= 3 * N as usize,
        "settle_turn took {steps} steps for N={N} entries -- expected a LINEAR bound (<= 3N), \
         not the O(N^2) a full-budget reset on every removal would produce"
    );
    assert!(
        !ops.is_empty(),
        "the walk did real work (removed at least one exhausted Event), not an early no-op"
    );
}
