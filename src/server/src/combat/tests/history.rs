use super::*;
use crate::data::command::Operation;

/// Rebuilds a `CombatSnapshot` from an applied document map — the same
/// shape `load_snapshot` reads from a repository, but from `apply`'s
/// in-memory result: the combat doc by id, every `"combatant"`/
/// `"combat-history"` child parented to it, and every host an `Actor`
/// combatant names.
fn rebuild(prev: &CombatSnapshot, docs: HashMap<Uuid, Document>) -> CombatSnapshot {
    let combat = docs
        .get(&prev.combat.id)
        .cloned()
        .expect("combat document present after apply");
    let engine: CombatEngine = serde_json::from_value(
        combat
            .engine
            .clone()
            .expect("combat document always carries an engine body"),
    )
    .unwrap();

    let mut combatants = Vec::new();
    let mut history = None;
    for doc in docs.values() {
        if doc.parent_id != Some(combat.id) {
            continue;
        }
        match doc.doc_type.as_str() {
            "combatant" => {
                if let Some(raw) = doc.engine.clone() {
                    if let Ok(e) = serde_json::from_value::<CombatantEngine>(raw) {
                        combatants.push(Combatant {
                            doc: doc.clone(),
                            engine: e,
                        });
                    }
                }
            }
            "combat-history" => {
                if let Some(raw) = doc.engine.clone() {
                    if let Ok(h) = serde_json::from_value::<CombatHistoryEngine>(raw) {
                        history = Some((doc.clone(), h));
                    }
                }
            }
            _ => {}
        }
    }

    let mut hosts = HashMap::new();
    for c in &combatants {
        if let CombatantKind::Actor { token_id, actor_id } = &c.engine.kind {
            for id in [token_id, actor_id].into_iter().flatten() {
                if let Some(doc) = docs.get(id) {
                    hosts.insert(*id, doc.clone());
                }
            }
        }
    }

    CombatSnapshot {
        combat,
        engine,
        combatants,
        hosts,
        history,
        registry: prev.registry.clone(),
        other_active: Vec::new(),
        chain: prev.chain.clone(),
    }
}

fn running_with_history() -> (CombatSnapshot, Uuid) {
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0x1A, None, false, (0.0, 30.0));
    let b = actor_combatant(11, combat, 0xB, None, false, (0.0, 30.0));
    let host = actor_with_effect(0x1A, None, 2, ExpiryPoint::TurnEnd, DurationUnit::Turns);
    let snap = snapshot(
        combat_engine(vec![a.doc.id, b.doc.id], None, 0, false),
        vec![a, b],
        vec![host],
    );
    let docs = apply(&snap, &start(&snap, 0, WORLD, Uuid::nil()).unwrap());
    (rebuild(&snap, docs), combat)
}

#[test]
fn start_and_each_advance_append_a_record_and_move_the_cursor() {
    let (snap, _) = running_with_history();
    let (_, h) = snap.history.as_ref().unwrap();
    assert_eq!((h.records.len(), h.cursor), (1, 0));
    let snap2 = rebuild(
        &snap,
        apply(&snap, &advance(&snap, WORLD, Uuid::nil(), 0).unwrap()),
    );
    let (_, h) = snap2.history.as_ref().unwrap();
    assert_eq!((h.records.len(), h.cursor), (2, 1));
    assert_eq!(h.records[1].turn, Uuid::from_u128(11));
    assert_eq!(h.records[1].effects.len(), 1);
}

#[test]
fn rewind_restores_spent_resources_expired_effects_and_deleted_events() {
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0x1A, None, false, (0.0, 30.0));
    let ev = event_combatant(11, combat, Some(1), None);
    let host = actor_with_effect(0x1A, None, 1, ExpiryPoint::TurnEnd, DurationUnit::Turns);
    let snap = snapshot(
        combat_engine(vec![a.doc.id, ev.doc.id], None, 0, false),
        vec![a, ev],
        vec![host],
    );
    let s1 = rebuild(
        &snap,
        apply(&snap, &start(&snap, 0, WORLD, Uuid::nil()).unwrap()),
    );
    // Spend movement mid-turn, then advance (a's effect expires, the event fires and is deleted).
    let s1 = rebuild(
        &s1,
        apply(
            &s1,
            &resource(
                &s1,
                Uuid::from_u128(10),
                "movement",
                ResourceOp::Delta { amount: -20.0 },
            )
            .unwrap(),
        ),
    );
    let s2 = rebuild(
        &s1,
        apply(&s1, &advance(&s1, WORLD, Uuid::nil(), 0).unwrap()),
    );
    assert!(
        s2.combatants
            .iter()
            .all(|c| c.doc.id != Uuid::from_u128(11)),
        "event deleted"
    );
    let ops = rewind(&s2).unwrap();
    let docs = apply(&s2, &ops);
    let a: CombatantEngine = engine_of(&docs, Uuid::from_u128(10));
    assert_eq!(
        a.resources["movement"].current, 30.0,
        "boundary value restored, not the spent one"
    );
    assert!(docs.contains_key(&Uuid::from_u128(11)), "event re-created");
    let e: EffectEngine = serde_json::from_value(
        docs[&Uuid::from_u128(0x1A)].embedded["effect"][0]
            .engine
            .clone()
            .unwrap(),
    )
    .unwrap();
    assert!(e.active && e.duration.unwrap().remaining == Some(1));
    let c: CombatEngine = engine_of(&docs, combat);
    assert_eq!((c.round, c.turn), (1, Some(Uuid::from_u128(10))));
    let h: CombatHistoryEngine = engine_of(&docs, s2.history.as_ref().unwrap().0.id);
    assert_eq!(
        (h.records.len(), h.cursor),
        (1, 0),
        "forward_restore off ⇒ the future is truncated on rewind"
    );
}

#[test]
fn rewind_at_the_first_record_is_refused() {
    let (snap, _) = running_with_history();
    assert!(matches!(rewind(&snap), Err(CombatError::Unrewindable)));
}

#[test]
fn rewind_restore_off_moves_only_the_clock() {
    let (mut snap, combat) = running_with_history();
    snap.engine.rewind_restore = false;
    snap.combat.engine = Some(serde_json::to_value(&snap.engine).unwrap());
    let s2 = rebuild(
        &snap,
        apply(&snap, &advance(&snap, WORLD, Uuid::nil(), 0).unwrap()),
    );
    let s2 = rebuild(
        &s2,
        apply(
            &s2,
            &resource(
                &s2,
                Uuid::from_u128(11),
                "movement",
                ResourceOp::Delta { amount: -20.0 },
            )
            .unwrap(),
        ),
    );
    let docs = apply(&s2, &rewind(&s2).unwrap());
    assert_eq!(
        engine_of::<CombatEngine>(&docs, combat).turn,
        Some(Uuid::from_u128(10))
    );
    assert_eq!(
        engine_of::<CombatantEngine>(&docs, Uuid::from_u128(11)).resources["movement"].current,
        10.0,
        "not restored"
    );
}

#[test]
fn forward_restore_keeps_the_future_and_fast_forwards_when_nothing_changed() {
    let (mut snap, combat) = running_with_history();
    snap.engine.forward_restore = true;
    snap.combat.engine = Some(serde_json::to_value(&snap.engine).unwrap());
    let s2 = rebuild(
        &snap,
        apply(&snap, &advance(&snap, WORLD, Uuid::nil(), 0).unwrap()),
    );
    let s1 = rebuild(&s2, apply(&s2, &rewind(&s2).unwrap()));
    let h: &CombatHistoryEngine = &s1.history.as_ref().unwrap().1;
    assert_eq!((h.records.len(), h.cursor), (2, 0), "future retained");
    let ops = advance(&s1, WORLD, Uuid::nil(), 0).unwrap();
    assert!(
        !ops.iter()
            .any(|o| matches!(o, Operation::Create { doc } if doc.doc_type == "message")),
        "no transition ran"
    );
    let docs = apply(&s1, &ops);
    assert_eq!(
        engine_of::<CombatEngine>(&docs, combat).turn,
        Some(Uuid::from_u128(11))
    );
    assert_eq!(
        engine_of::<CombatHistoryEngine>(&docs, s1.history.as_ref().unwrap().0.id).cursor,
        1
    );
}

#[test]
fn forward_restore_discards_the_future_when_a_combatant_changed() {
    let (mut snap, _) = running_with_history();
    snap.engine.forward_restore = true;
    snap.combat.engine = Some(serde_json::to_value(&snap.engine).unwrap());
    let s2 = rebuild(
        &snap,
        apply(&snap, &advance(&snap, WORLD, Uuid::nil(), 0).unwrap()),
    );
    let s1 = rebuild(&s2, apply(&s2, &rewind(&s2).unwrap()));
    let s1 = rebuild(
        &s1,
        apply(
            &s1,
            &resource(
                &s1,
                Uuid::from_u128(10),
                "movement",
                ResourceOp::Delta { amount: -5.0 },
            )
            .unwrap(),
        ),
    );
    let docs = apply(&s1, &advance(&s1, WORLD, Uuid::nil(), 0).unwrap());
    let h: CombatHistoryEngine = engine_of(&docs, s1.history.as_ref().unwrap().0.id);
    assert_eq!(
        (h.records.len(), h.cursor),
        (2, 1),
        "future truncated then a fresh record pushed"
    );
}

#[test]
fn history_is_capped_and_the_cursor_shifts_with_the_drop() {
    let (mut snap, _) = running_with_history();
    let (doc, h) = snap.history.as_mut().unwrap();
    let rec = h.records[0].clone();
    h.records = (0..MAX_TURN_HISTORY).map(|_| rec.clone()).collect();
    h.cursor = (MAX_TURN_HISTORY - 1) as u32;
    doc.engine = Some(serde_json::to_value(&*h).unwrap());
    let docs = apply(&snap, &advance(&snap, WORLD, Uuid::nil(), 0).unwrap());
    let h: CombatHistoryEngine = engine_of(&docs, snap.history.as_ref().unwrap().0.id);
    assert_eq!(
        (h.records.len(), h.cursor as usize),
        (MAX_TURN_HISTORY, MAX_TURN_HISTORY - 1)
    );
}
