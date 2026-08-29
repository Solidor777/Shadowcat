use super::*;

#[test]
fn collect_effects_finds_anchored_effects_on_actor_token_copy_and_transferring_items() {
    let combat = Uuid::from_u128(1);
    let actor = actor_with_effect(0xA, None, 2, ExpiryPoint::TurnEnd, DurationUnit::Rounds);
    let mut token = doc(
        0x70,
        "token",
        Some(SCENE),
        serde_json::json!({ "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0, "rotation": 0.0 }),
    );
    token.embedded.insert(
        "actor".into(),
        vec![actor_with_effect(
            0xE,
            None,
            1,
            ExpiryPoint::TurnStart,
            DurationUnit::Turns,
        )],
    );
    let mut item = doc(0x1, "item", None, serde_json::json!({}));
    let mut transferring =
        actor_with_effect(0xF, None, 1, ExpiryPoint::RoundEnd, DurationUnit::Rounds)
            .embedded
            .remove("effect")
            .unwrap();
    let mut eng: EffectEngine =
        serde_json::from_value(transferring[0].engine.clone().unwrap()).unwrap();
    eng.transfer = true;
    transferring[0].engine = Some(serde_json::to_value(&eng).unwrap());
    item.embedded.insert("effect".into(), transferring);
    let mut actor = actor;
    actor.embedded.insert("item".into(), vec![item]);
    let mut c = actor_combatant(10, combat, 0xA, None, false, (0.0, 30.0));
    c.engine.kind = CombatantKind::Actor {
        token_id: Some(Uuid::from_u128(0x70)),
        actor_id: Some(Uuid::from_u128(0xA)),
    };
    let snap = snapshot(
        combat_engine(vec![c.doc.id], Some(c.doc.id), 1, true),
        vec![c.clone()],
        vec![actor, token],
    );
    let refs = collect_effects(&snap, &c);
    let paths: Vec<(Uuid, String)> = refs.iter().map(|r| (r.host, r.path.clone())).collect();
    assert!(paths.contains(&(Uuid::from_u128(0xA), "/embedded/effect/0".into())));
    assert!(paths.contains(&(
        Uuid::from_u128(0xA),
        "/embedded/item/0/embedded/effect/0".into()
    )));
    assert!(paths.contains(&(
        Uuid::from_u128(0x70),
        "/embedded/actor/0/embedded/effect/0".into()
    )));
    assert_eq!(refs.len(), 3);
}

#[test]
fn boundary_tick_decrements_and_expires_at_zero_and_skips_unresolved() {
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0xA, None, false, (0.0, 30.0));
    let b = actor_combatant(11, combat, 0xB, None, false, (0.0, 30.0));
    let host_a = actor_with_effect(0xA, None, 1, ExpiryPoint::TurnEnd, DurationUnit::Turns);
    let mut host_b = actor_with_effect(0xB, None, 1, ExpiryPoint::TurnEnd, DurationUnit::Turns);
    let mut e: EffectEngine =
        serde_json::from_value(host_b.embedded["effect"][0].engine.clone().unwrap()).unwrap();
    e.duration.as_mut().unwrap().remaining = None;
    host_b.embedded.get_mut("effect").unwrap()[0].engine = Some(serde_json::to_value(&e).unwrap());
    let snap = snapshot(
        combat_engine(vec![a.doc.id, b.doc.id], Some(a.doc.id), 1, true),
        vec![a, b],
        vec![host_a, host_b],
    );
    let ops = advance(&snap, WORLD, Uuid::nil(), 0).unwrap();
    let docs = apply(&snap, &ops);
    let ea: EffectEngine = serde_json::from_value(
        docs[&Uuid::from_u128(0xA)].embedded["effect"][0]
            .engine
            .clone()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(ea.duration.unwrap().remaining, Some(0));
    assert!(!ea.active);
    let eb: EffectEngine = serde_json::from_value(
        docs[&Uuid::from_u128(0xB)].embedded["effect"][0]
            .engine
            .clone()
            .unwrap(),
    )
    .unwrap();
    assert!(
        eb.active && eb.duration.unwrap().remaining.is_none(),
        "unresolved effects are never touched"
    );
}

#[test]
fn rounds_unit_ticks_only_on_round_boundaries_and_turn_end_policy_expires_at_host_turn_end() {
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0xA, None, false, (0.0, 30.0));
    let b = actor_combatant(11, combat, 0xB, None, false, (0.0, 30.0));
    let host_a = actor_with_effect(0xA, None, 2, ExpiryPoint::RoundEnd, DurationUnit::Rounds);
    let mut host_b = actor_with_effect(0xB, None, 5, ExpiryPoint::RoundEnd, DurationUnit::Rounds);
    let mut e: EffectEngine =
        serde_json::from_value(host_b.embedded["effect"][0].engine.clone().unwrap()).unwrap();
    e.lifecycle.as_mut().unwrap().resolved = Some(ResolvedLifecycle {
        on_combat_end: true,
        on_turn_end: true,
        on_advance: true,
    });
    host_b.embedded.get_mut("effect").unwrap()[0].engine = Some(serde_json::to_value(&e).unwrap());
    let snap = snapshot(
        combat_engine(vec![a.doc.id, b.doc.id], Some(b.doc.id), 1, true),
        vec![a, b],
        vec![host_a, host_b],
    );
    let docs = apply(&snap, &advance(&snap, WORLD, Uuid::nil(), 0).unwrap()); // b's turn ends, round wraps
    let ea: EffectEngine = serde_json::from_value(
        docs[&Uuid::from_u128(0xA)].embedded["effect"][0]
            .engine
            .clone()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        ea.duration.unwrap().remaining,
        Some(1),
        "one round boundary passed"
    );
    let eb: EffectEngine = serde_json::from_value(
        docs[&Uuid::from_u128(0xB)].embedded["effect"][0]
            .engine
            .clone()
            .unwrap(),
    )
    .unwrap();
    assert!(!eb.active, "on_turn_end policy expired it at b's turn end");
}
