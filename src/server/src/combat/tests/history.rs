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
    let a = actor_combatant(10, combat, 0x1A, None, false, 0.0);
    let b = actor_combatant(11, combat, 0x1B, None, false, 0.0);
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

/// A rewind steps back ONE turn boundary at a time, and an auto-resolved
/// `Event`'s own boundary is one of them: the first rewind lands on the
/// event's turn (restoring the event document the advance deleted), and only
/// the second reaches the actor's turn that preceded it.
#[test]
fn rewind_restores_spent_resources_expired_effects_and_deleted_events() {
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0x1A, None, false, 0.0);
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
    // First rewind: back to the EVENT's own auto-resolved boundary.
    let s3 = rebuild(&s2, apply(&s2, &rewind(&s2, 0).unwrap()));
    assert_eq!(
        (s3.engine.round, s3.engine.turn),
        (1, Some(Uuid::from_u128(11))),
        "the intermediate record captured at the event's own turn boundary"
    );
    assert!(
        s3.combatants
            .iter()
            .any(|x| x.doc.id == Uuid::from_u128(11)),
        "event re-created"
    );
    assert!(
        s3.engine.order.contains(&Uuid::from_u128(11)),
        "the re-created event is reachable from order again"
    );
    assert_eq!(
        s3.combatants
            .iter()
            .find(|x| x.doc.id == Uuid::from_u128(10))
            .unwrap()
            .engine
            .resources["movement"]
            .current,
        10.0,
        "the spend stood at the event's boundary, so it is restored as it stood"
    );

    // Second rewind: back to the actor's own turn, before the advance ran.
    let docs = apply(&s3, &rewind(&s3, 0).unwrap());
    let a: CombatantEngine = engine_of(&docs, Uuid::from_u128(10));
    assert_eq!(
        a.resources["movement"].current, 30.0,
        "boundary value restored, not the spent one"
    );
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
    let h: CombatHistoryEngine = engine_of(&docs, s3.history.as_ref().unwrap().0.id);
    assert_eq!(
        (h.records.len(), h.cursor),
        (1, 0),
        "forward_restore off ⇒ the future is truncated on rewind"
    );
}

/// With `rewind_restore` off nothing brings back a combatant the clock deleted, so a boundary
/// whose `turn` names an exhausted `Event` can never be re-entered: writing `/engine/turn` back
/// to it would leave `turn` naming an id absent from `/engine/order`, which
/// `CombatEngine::validate` refuses. The refusal must be the specific `RewindUnreachable`, made
/// before any op is built — not a generic engine rejection of an already-assembled batch, and
/// never a silent clamp of the clock to some other turn.
#[test]
fn rewind_restore_off_refuses_a_boundary_whose_turn_was_deleted() {
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0x1A, None, false, 0.0);
    let ev = event_combatant(11, combat, Some(1), None);
    let host = actor_with_effect(0x1A, None, 1, ExpiryPoint::TurnEnd, DurationUnit::Turns);
    let snap = snapshot(
        combat_engine(vec![a.doc.id, ev.doc.id], None, 0, false),
        vec![a, ev],
        vec![host],
    );

    // `start` snapshots the resolved rules chain onto the combat, so the flag is cleared AFTER
    // it rather than on the pre-start engine, which `start` would overwrite.
    let mut s1 = rebuild(
        &snap,
        apply(&snap, &start(&snap, 0, WORLD, Uuid::nil()).unwrap()),
    );
    s1.engine.rewind_restore = false;
    s1.combat.engine = Some(serde_json::to_value(&s1.engine).unwrap());

    // The advance walks past the event, which fires and deletes itself; the boundary record
    // captured at the event's own turn survives in history with `turn` naming the deleted id.
    let mut s2 = rebuild(
        &s1,
        apply(&s1, &advance(&s1, WORLD, Uuid::nil(), 0).unwrap()),
    );
    assert!(
        !s2.engine.order.contains(&Uuid::from_u128(11)),
        "the exhausted event left the order"
    );
    {
        let (_, h) = s2.history.as_ref().unwrap();
        assert_eq!(
            h.records[h.cursor as usize - 1].turn,
            Uuid::from_u128(11),
            "the boundary one step back is the deleted event's own turn"
        );
    }

    assert!(matches!(
        rewind(&s2, 0),
        Err(CombatError::RewindUnreachable)
    ));

    // Positive control: the SAME boundary rewinds cleanly with `rewind_restore` on, so the
    // refusal above is specific to "nothing will restore the combatant", not to the boundary.
    // Safe to mutate `s2` in place — the refused rewind produced no ops and changed nothing.
    s2.engine.rewind_restore = true;
    s2.combat.engine = Some(serde_json::to_value(&s2.engine).unwrap());
    let docs = apply(&s2, &rewind(&s2, 0).unwrap());
    let c: CombatEngine = engine_of(&docs, combat);
    assert_eq!(c.turn, Some(Uuid::from_u128(11)));
    assert!(c.order.contains(&Uuid::from_u128(11)));
}

#[test]
fn rewind_at_the_first_record_is_refused() {
    let (snap, _) = running_with_history();
    assert!(matches!(rewind(&snap, 0), Err(CombatError::Unrewindable)));
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
    let docs = apply(&s2, &rewind(&s2, 0).unwrap());
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
    let s1 = rebuild(&s2, apply(&s2, &rewind(&s2, 0).unwrap()));
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
    let s1 = rebuild(&s2, apply(&s2, &rewind(&s2, 0).unwrap()));
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

/// The COUNT bound, exercised on its own: pre-filled with content-free
/// records so the independent serialized-byte bound (see the test below)
/// never binds first and the assertion is about `MAX_TURN_HISTORY` alone.
#[test]
fn history_is_capped_and_the_cursor_shifts_with_the_drop() {
    let (mut snap, _) = running_with_history();
    let (doc, h) = snap.history.as_mut().unwrap();
    let rec = TurnRecord {
        round: h.records[0].round,
        turn: h.records[0].turn,
        combatants: Vec::new(),
        effects: Vec::new(),
    };
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

/// The BYTE bound, which the count bound does not imply: a log pre-filled to
/// `MAX_TURN_HISTORY` records of REAL captured content serializes past
/// `MAX_SYSTEM_BYTES`, so an advance that only enforced the count bound
/// would emit an `/engine` band the repository refuses — rolling the whole
/// transition back and wedging the clock permanently. `apply`'s own
/// `validate_system_size` check is what makes the failure visible here.
#[test]
fn history_is_capped_by_serialized_bytes_before_the_engine_band_overflows() {
    let (mut snap, _) = running_with_history();
    let (doc, h) = snap.history.as_mut().unwrap();
    let mut rec = h.records[0].clone();
    // Pad one captured combatant's opaque `system` band — content a real combat authors freely
    // — until `MAX_TURN_HISTORY + 1` such records genuinely breach the cap. Derived from the cap
    // rather than assumed, so this test keeps measuring the byte bound no matter how many bytes
    // the capture shape itself happens to cost.
    let need = crate::data::validation::MAX_SYSTEM_BYTES / (MAX_TURN_HISTORY + 1) + 1;
    let pad = need.saturating_sub(serde_json::to_vec(&rec).unwrap().len());
    rec.combatants[0].system = serde_json::json!({ "pad": "x".repeat(pad) });
    h.records = (0..MAX_TURN_HISTORY).map(|_| rec.clone()).collect();
    h.cursor = (MAX_TURN_HISTORY - 1) as u32;
    doc.engine = Some(serde_json::to_value(&*h).unwrap());
    // Precondition: without the byte bound, retaining all 200 records plus a
    // fresh one is what breaches the cap — the state this test drives from.
    let unbounded = CombatHistoryEngine {
        records: (0..MAX_TURN_HISTORY + 1).map(|_| rec.clone()).collect(),
        cursor: MAX_TURN_HISTORY as u32,
    };
    assert!(
        serde_json::to_vec(&unbounded).unwrap().len() > crate::data::validation::MAX_SYSTEM_BYTES,
        "fixture must actually be able to breach the cap, or this test proves nothing"
    );

    let history_id = snap.history.as_ref().unwrap().0.id;
    let docs = apply(&snap, &advance(&snap, WORLD, Uuid::nil(), 0).unwrap());
    let h: CombatHistoryEngine = engine_of(&docs, history_id);
    assert!(
        h.records.len() < MAX_TURN_HISTORY,
        "the byte bound evicted beyond what the count bound alone would have dropped"
    );
    assert_eq!(
        h.cursor as usize,
        h.records.len() - 1,
        "the cursor follows the newest record after eviction"
    );
    let band = serde_json::to_vec(docs[&history_id].engine.as_ref().unwrap()).unwrap();
    assert!(
        band.len() <= crate::data::validation::MAX_SYSTEM_BYTES,
        "the committed engine band stays within the ingress cap"
    );
}

/// The envelope constant `evict_to_fit` charges for a serialized
/// `CombatHistoryEngine` must never UNDER-count, or the eviction it drives
/// leaves a band that still breaches the cap.
#[test]
fn the_history_envelope_estimate_is_never_an_undercount() {
    let (snap, _) = running_with_history();
    let rec = snap.history.as_ref().unwrap().1.records[0].clone();
    for count in [1usize, 2, 7, 64] {
        let records: Vec<TurnRecord> = (0..count).map(|_| rec.clone()).collect();
        let engine = CombatHistoryEngine {
            records: records.clone(),
            cursor: (count - 1) as u32,
        };
        let actual = serde_json::to_vec(&engine).unwrap().len();
        let estimated: usize = 34
            + records
                .iter()
                .map(|r| serde_json::to_vec(r).unwrap().len())
                .sum::<usize>()
            + (count - 1);
        assert!(
            estimated >= actual,
            "envelope estimate {estimated} under-counts the real {actual} at {count} records"
        );
    }
}
