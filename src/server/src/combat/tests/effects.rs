use super::*;
use crate::combat::effects::EffectRef;
use crate::data::command::Operation;

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

/// Host and anchor are independent axes: an effect living on A's host but
/// anchored to B belongs to B's clock and to NEITHER combatant twice. An
/// effect anchored to a combatant that does not host it is reachable only
/// through the anchor's own pass, so without that pass it would be ticked,
/// expired and captured by nobody.
#[test]
fn an_effect_anchored_to_another_combatant_is_collected_by_the_anchor_not_the_host() {
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0x1A, None, false, (0.0, 30.0));
    let b = actor_combatant(11, combat, 0x1B, None, false, (0.0, 30.0));
    // Physically on A's actor, anchored to B.
    let host_a = actor_with_effect(
        0x1A,
        Some(b.doc.id),
        2,
        ExpiryPoint::TurnEnd,
        DurationUnit::Turns,
    );
    let host_b = doc(0x1B, "actor", None, actor_body());
    let snap = snapshot(
        combat_engine(vec![a.doc.id, b.doc.id], Some(a.doc.id), 1, true),
        vec![a.clone(), b.clone()],
        vec![host_a, host_b],
    );
    assert!(
        collect_effects(&snap, &a).is_empty(),
        "the HOST's combatant must not claim an effect anchored elsewhere"
    );
    let for_b: Vec<(Uuid, String)> = collect_effects(&snap, &b)
        .iter()
        .map(|r| (r.host, r.path.clone()))
        .collect();
    assert_eq!(
        for_b,
        vec![(Uuid::from_u128(0x1A), "/embedded/effect/0".to_string())],
        "the ANCHOR's combatant reaches it, exactly once, on the host it lives on"
    );
}

/// `walk_any_host`'s TOKEN branch: an effect reachable only through a token's embedded actor
/// copy is collected by its anchor exactly once, at the SAME `(host, path)` key the token's own
/// combatant's first pass produces for the same slot. The two must agree, because every
/// boundary sweep that dedupes across combatants (`transition::run_boundary`,
/// `transition::end`, `history::capture`) keys on `(host, path)` alone — a token-branch key
/// shaped differently from pass 1's would let one effect be ticked twice in one sweep.
#[test]
fn a_token_hosted_cross_host_anchored_effect_is_collected_once_at_the_pass_one_path() {
    let combat = Uuid::from_u128(1);
    let token_id = Uuid::from_u128(0x70);
    // A is hosted by a TOKEN (only its embedded actor copy carries effects); B merely anchors.
    let mut a = actor_combatant(10, combat, 0x1A, None, false, (0.0, 30.0));
    a.engine.kind = CombatantKind::Actor {
        token_id: Some(token_id),
        actor_id: None,
    };
    a.doc.engine = Some(serde_json::to_value(&a.engine).unwrap());
    let b = actor_combatant(11, combat, 0x1B, None, false, (0.0, 30.0));
    let host_b = doc(0x1B, "actor", None, actor_body());

    let token_hosting = |anchor: Option<Uuid>| {
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
                anchor,
                2,
                ExpiryPoint::TurnEnd,
                DurationUnit::Turns,
            )],
        );
        token
    };
    let keys = |refs: Vec<EffectRef>| -> Vec<(Uuid, String)> {
        refs.iter().map(|r| (r.host, r.path.clone())).collect()
    };
    let expected = vec![(token_id, "/embedded/actor/0/embedded/effect/0".to_string())];

    // Anchored to B: reachable only through B's cross-host pass, which takes the token branch.
    let snap = snapshot(
        combat_engine(vec![a.doc.id, b.doc.id], Some(a.doc.id), 1, true),
        vec![a.clone(), b.clone()],
        vec![token_hosting(Some(b.doc.id)), host_b.clone()],
    );
    assert!(
        collect_effects(&snap, &a).is_empty(),
        "the hosting token's own combatant must not claim an effect anchored elsewhere"
    );
    assert_eq!(
        keys(collect_effects(&snap, &b)),
        expected,
        "the anchor reaches the token-hosted effect exactly once"
    );

    // Unanchored: the same slot, reached instead by the token's OWN combatant through pass 1.
    let snap = snapshot(
        combat_engine(vec![a.doc.id, b.doc.id], Some(a.doc.id), 1, true),
        vec![a.clone(), b],
        vec![token_hosting(None), host_b],
    );
    assert_eq!(
        keys(collect_effects(&snap, &a)),
        expected,
        "pass 1 and the cross-host token branch key on the identical (host, path)"
    );
}

/// A cross-host anchored effect is on the ANCHOR's clock end to end, not
/// merely visible to a collector: B's own turn-end boundary ticks it, and A's
/// does not.
#[test]
fn a_cross_host_anchored_effect_ticks_on_the_anchors_own_turn_boundary() {
    let combat = Uuid::from_u128(1);
    let a = actor_combatant(10, combat, 0x1A, None, false, (0.0, 30.0));
    let b = actor_combatant(11, combat, 0x1B, None, false, (0.0, 30.0));
    let host_a = actor_with_effect(
        0x1A,
        Some(b.doc.id),
        2,
        ExpiryPoint::TurnEnd,
        DurationUnit::Turns,
    );
    let host_b = doc(0x1B, "actor", None, actor_body());
    let remaining_of = |docs: &HashMap<Uuid, Document>| -> Option<u32> {
        let e: EffectEngine = serde_json::from_value(
            docs[&Uuid::from_u128(0x1A)].embedded["effect"][0]
                .engine
                .clone()
                .unwrap(),
        )
        .unwrap();
        e.duration.unwrap().remaining
    };

    // A's turn ends: the effect is hosted on A but anchored to B, so nothing ticks.
    let snap_a = snapshot(
        combat_engine(vec![a.doc.id, b.doc.id], Some(a.doc.id), 1, true),
        vec![a.clone(), b.clone()],
        vec![host_a.clone(), host_b.clone()],
    );
    let docs = apply(&snap_a, &advance(&snap_a, WORLD, Uuid::nil(), 0).unwrap());
    assert_eq!(remaining_of(&docs), Some(2), "not A's effect to tick");

    // B's turn ends: B is the anchor, so the effect decrements.
    let snap_b = snapshot(
        combat_engine(vec![a.doc.id, b.doc.id], Some(b.doc.id), 1, true),
        vec![a, b],
        vec![host_a, host_b],
    );
    let docs = apply(&snap_b, &advance(&snap_b, WORLD, Uuid::nil(), 0).unwrap());
    assert_eq!(
        remaining_of(&docs),
        Some(1),
        "the anchor's boundary ticks it"
    );
}

#[test]
fn boundary_tick_decrements_and_expires_at_zero_and_skips_unresolved() {
    let combat = Uuid::from_u128(1);
    // Host ids (0x1A/0x1B) deliberately distinct from the combatants' own
    // doc ids (10/11) — a host id must never numerically collide with a
    // combatant doc id (see `an_effect_with_no_lifecycle_at_all...`'s own
    // note); a real, randomly-generated UUID pair never collides this way,
    // but `apply`'s strict OCC check catches it immediately if a fixture
    // does, since a collision merges the combatant's and the host's
    // documents onto one map entry.
    let a = actor_combatant(10, combat, 0x1A, None, false, (0.0, 30.0));
    let b = actor_combatant(11, combat, 0x1B, None, false, (0.0, 30.0));
    let host_a = actor_with_effect(0x1A, None, 1, ExpiryPoint::TurnEnd, DurationUnit::Turns);
    let mut host_b = actor_with_effect(0x1B, None, 1, ExpiryPoint::TurnEnd, DurationUnit::Turns);
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
        docs[&Uuid::from_u128(0x1A)].embedded["effect"][0]
            .engine
            .clone()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(ea.duration.unwrap().remaining, Some(0));
    assert!(!ea.active);
    let eb: EffectEngine = serde_json::from_value(
        docs[&Uuid::from_u128(0x1B)].embedded["effect"][0]
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
    // Host ids (0x1A/0x1B) deliberately distinct from the combatants' own
    // doc ids (10/11) — see the identical note in
    // `boundary_tick_decrements_and_expires_at_zero_and_skips_unresolved`.
    let a = actor_combatant(10, combat, 0x1A, None, false, (0.0, 30.0));
    let b = actor_combatant(11, combat, 0x1B, None, false, (0.0, 30.0));
    let host_a = actor_with_effect(0x1A, None, 2, ExpiryPoint::RoundEnd, DurationUnit::Rounds);
    let mut host_b = actor_with_effect(0x1B, None, 5, ExpiryPoint::RoundEnd, DurationUnit::Rounds);
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
        docs[&Uuid::from_u128(0x1A)].embedded["effect"][0]
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
        docs[&Uuid::from_u128(0x1B)].embedded["effect"][0]
            .engine
            .clone()
            .unwrap(),
    )
    .unwrap();
    assert!(!eb.active, "on_turn_end policy expired it at b's turn end");
}

#[test]
fn an_effect_with_no_lifecycle_at_all_is_never_touched_by_a_boundary_it_would_otherwise_match() {
    // Every other fixture sets `lifecycle: Some(EffectLifecycle { resolved:
    // Some(..), .. })`. An effect that has NOT resolved its lifecycle yet —
    // `lifecycle: None` entirely, or `Some(EffectLifecycle { resolved: None,
    // .. })` — must be left completely inert by both `tick` and
    // `expire_by_policy`, even when its `duration.expires`/`unit` exactly
    // match the boundary being processed.
    let combat = Uuid::from_u128(1);
    // 0x9A, not 0xA — a host id must never numerically collide with a
    // combatant doc id (`actor_combatant(10, ..)` == `0xA` == 10 decimal is
    // the known real-UUID-can-never-produce-this fixture gotcha).
    let a = actor_combatant(10, combat, 0x9A, None, false, (0.0, 30.0));
    let mut host = actor_with_effect(0x9A, None, 1, ExpiryPoint::TurnEnd, DurationUnit::Turns);
    let mut e: EffectEngine =
        serde_json::from_value(host.embedded["effect"][0].engine.clone().unwrap()).unwrap();
    e.lifecycle = None;
    let unresolved_before = serde_json::to_value(&e).unwrap();
    host.embedded.get_mut("effect").unwrap()[0].engine = Some(unresolved_before.clone());
    let snap = snapshot(
        combat_engine(vec![a.doc.id], Some(a.doc.id), 1, true),
        vec![a],
        vec![host],
    );
    let ops = advance(&snap, WORLD, Uuid::nil(), 0).unwrap();
    assert!(
        !ops.iter().any(
            |o| matches!(o, Operation::Update { doc_id, .. } if *doc_id == Uuid::from_u128(0x9A))
        ),
        "no host write at all — the effect's engine band is byte-identical before and after"
    );
    let docs = apply(&snap, &ops);
    assert_eq!(
        docs[&Uuid::from_u128(0x9A)].embedded["effect"][0].engine,
        Some(unresolved_before),
        "unchanged"
    );
}
