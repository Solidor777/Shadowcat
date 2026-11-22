use super::*;
use serde_json::json;
use uuid::Uuid;

fn combat(order: Vec<Uuid>) -> CombatEngine {
    CombatEngine {
        scene_id: crate::test_ids::test_uuid(1),
        active: false,
        round: 0,
        turn: None,
        turn_control: TurnControl::OwnerMayEnd,
        order,
        movement: MovementRules {
            resource: None,
            interpretation: Interpretation::PerCell,
            enforcement: Enforcement::None,
        },
        effect_cleanup: true,
        rewind_restore: true,
        forward_restore: false,
        effect_lifecycle: EffectLifecycleDefaults::default(),
    }
}

#[test]
fn combat_order_must_be_unique() {
    let id = crate::test_ids::test_uuid(7);
    assert!(combat(vec![id, id]).validate().is_err());
    assert!(combat(vec![id, crate::test_ids::test_uuid(8)])
        .validate()
        .is_ok());
}

#[test]
fn combat_turn_must_be_in_order_when_set() {
    let id = crate::test_ids::test_uuid(7);
    let mut c = combat(vec![id]);
    c.turn = Some(crate::test_ids::test_uuid(9));
    assert!(c.validate().is_err());
    c.turn = Some(id);
    assert!(c.validate().is_ok());
}

#[test]
fn combatant_actor_kind_needs_a_token_or_an_actor() {
    let none = CombatantEngine {
        kind: CombatantKind::Actor {
            token_id: None,
            actor_id: None,
        },
        initiative: None,
        tiebreak: 0.0,
        defeated: false,
        group: None,
        resources: Default::default(),
    };
    assert!(none.validate().is_err());
    let some = CombatantEngine {
        kind: CombatantKind::Actor {
            token_id: None,
            actor_id: Some(crate::test_ids::test_uuid(3)),
        },
        ..none
    };
    assert!(some.validate().is_ok());
}

#[test]
fn combatant_numbers_must_be_finite() {
    let mut c = CombatantEngine {
        kind: CombatantKind::Event {
            lifespan: None,
            message: None,
        },
        initiative: Some(f64::NAN),
        tiebreak: 0.0,
        defeated: false,
        group: None,
        resources: Default::default(),
    };
    assert!(c.validate().is_err());
    c.initiative = Some(12.0);
    c.resources.insert(
        "movement".into(),
        CombatantResource {
            current: f64::INFINITY,
        },
    );
    assert!(c.validate().is_err());
    c.resources
        .insert("movement".into(), CombatantResource { current: 30.0 });
    assert!(c.validate().is_ok());
}

#[test]
fn formula_text_must_parse_and_fit_the_length_cap() {
    fn reg_with(value: Formula) -> ResourceRegistryEngine {
        let mut reg = ResourceRegistryEngine {
            resources: Default::default(),
        };
        reg.resources.insert(
            "m".into(),
            Resource {
                name: "M".into(),
                order: 0,
                binding: ResourceBinding::Mirror { value },
            },
        );
        reg
    }
    let rejected = |src: &str| reg_with(Formula::Text(src.into())).validate().unwrap_err();
    assert!(rejected("").contains("unexpected end of formula"));
    assert!(rejected("1 +").contains("unexpected end of formula"));
    assert!(rejected("(").contains("unexpected end of formula"));
    assert!(rejected("dex ? 2").contains("unexpected '?' at position 4"));
    assert!(rejected(&"x".repeat(513)).contains("formula exceeds 512 characters"));
    assert!(rejected("dex \u{1F600} 2").contains("unexpected '\u{1F600}'"));
    assert!(reg_with(Formula::Text("hp".into())).validate().is_ok());
    assert!(reg_with(Formula::Text("speed * 2".into()))
        .validate()
        .is_ok());
    assert!(reg_with(Formula::Text("max(stats.hp.final, 1)".into()))
        .validate()
        .is_ok());
    assert!(reg_with(Formula::Number(f64::INFINITY)).validate().is_err());
}

#[test]
fn formula_number_must_be_finite() {
    let mut reg = ResourceRegistryEngine {
        resources: Default::default(),
    };
    reg.resources.insert(
        "m".into(),
        Resource {
            name: "M".into(),
            order: 0,
            binding: ResourceBinding::Tracked {
                max: Formula::Number(f64::INFINITY),
                recover: Recovery {
                    turn_start: Formula::Number(0.0),
                    turn_end: Formula::Number(0.0),
                    round_start: Formula::Number(0.0),
                    round_end: Formula::Number(0.0),
                },
            },
        },
    );
    assert!(reg.validate().is_err());
}

#[test]
fn recovery_fields_default_to_zero() {
    let r: Recovery = serde_json::from_value(json!({})).unwrap();
    assert_eq!(r.turn_start, Formula::Number(0.0));
    assert_eq!(r.round_end, Formula::Number(0.0));
}

#[test]
fn formula_serializes_untagged() {
    assert_eq!(
        serde_json::to_value(Formula::Number(2.5)).unwrap(),
        json!(2.5)
    );
    assert_eq!(
        serde_json::to_value(Formula::Text("speed".into())).unwrap(),
        json!("speed")
    );
    assert_eq!(
        serde_json::from_value::<Formula>(json!(3)).unwrap(),
        Formula::Number(3.0)
    );
}

#[test]
fn effect_duration_amount_must_be_positive() {
    let e = EffectEngine {
        active: true,
        transfer: false,
        duration: Some(Duration {
            amount: Formula::Number(0.0),
            remaining: None,
            unit: DurationUnit::Rounds,
            anchor: None,
            expires: ExpiryPoint::TurnEnd,
        }),
        lifecycle: None,
        origin: None,
    };
    assert!(e.validate().is_err());
}

#[test]
fn effect_duration_amount_text_must_parse() {
    let with_amount = |amount: Formula| EffectEngine {
        active: true,
        transfer: false,
        duration: Some(Duration {
            amount,
            remaining: None,
            unit: DurationUnit::Rounds,
            anchor: None,
            expires: ExpiryPoint::TurnEnd,
        }),
        lifecycle: None,
        origin: None,
    };
    assert!(with_amount(Formula::Text("1 +".into())).validate().is_err());
    assert!(with_amount(Formula::Text("rounds".into()))
        .validate()
        .is_ok());
}

#[test]
fn combat_history_validates_every_captured_band() {
    let effect = |amount: Formula| EffectEngine {
        active: true,
        transfer: false,
        duration: Some(Duration {
            amount,
            remaining: Some(2),
            unit: DurationUnit::Rounds,
            anchor: None,
            expires: ExpiryPoint::TurnEnd,
        }),
        lifecycle: None,
        origin: None,
    };
    let history = |engine: EffectEngine, combatant: CombatantEngine| CombatHistoryEngine {
        records: vec![TurnRecord {
            round: 1,
            turn: crate::test_ids::test_uuid(7),
            combatants: vec![CapturedCombatant {
                id: crate::test_ids::test_uuid(7),
                name: None,
                permissions: Default::default(),
                owner: None,
                engine: combatant,
                system: json!({}),
            }],
            effects: vec![EffectSnapshot {
                host: crate::test_ids::test_uuid(9),
                path: "/embedded/effect/0".into(),
                engine,
            }],
        }],
        cursor: 0,
    };
    let combatant = |initiative: Option<f64>| CombatantEngine {
        kind: CombatantKind::Event {
            lifespan: None,
            message: None,
        },
        initiative,
        tiebreak: 0.0,
        defeated: false,
        group: None,
        resources: Default::default(),
    };
    assert!(
        history(effect(Formula::Text("rounds".into())), combatant(Some(1.0)))
            .validate()
            .is_ok()
    );
    let bad_effect = history(effect(Formula::Text("1 +".into())), combatant(Some(1.0)))
        .validate()
        .unwrap_err();
    assert!(bad_effect.contains("records[0].effects[0]"), "{bad_effect}");
    let bad_combatant = history(effect(Formula::Number(2.0)), combatant(Some(f64::NAN)))
        .validate()
        .unwrap_err();
    assert!(
        bad_combatant.contains("records[0].combatants[0]"),
        "{bad_combatant}"
    );
}

#[test]
fn resolve_combat_rules_scene_overrides_world_overrides_engine() {
    let engine_only = resolve_combat_rules(None, None, None);
    assert_eq!(
        engine_only,
        ResolvedCombatRules {
            movement: MovementRules {
                resource: None,
                interpretation: Interpretation::PerCell,
                enforcement: Enforcement::None
            },
            turn_control: TurnControl::OwnerMayEnd,
            effect_cleanup: true,
            rewind_restore: true,
            forward_restore: false,
            effect_lifecycle: EffectLifecycleDefaults::default(),
            expiry_notice: ExpiryNotice::Visible,
        }
    );
    let world = CombatDefaults {
        movement_resource: Some(Some("movement".into())),
        interpretation: Some(Interpretation::Spaces),
        enforcement: Some(Enforcement::Hard),
        turn_control: Some(TurnControl::GmOnly),
        ..Default::default()
    };
    let scene = CombatDefaults {
        movement_resource: Some(Some("ship".into())),
        interpretation: None,
        enforcement: Some(Enforcement::Warn),
        turn_control: None,
        ..Default::default()
    };
    let r = resolve_combat_rules(None, Some(&world), Some(&scene));
    assert_eq!(r.movement.resource.as_deref(), Some("ship"));
    assert_eq!(r.movement.interpretation, Interpretation::Spaces);
    assert_eq!(r.movement.enforcement, Enforcement::Warn);
    assert_eq!(r.turn_control, TurnControl::GmOnly);
    // A scene may clear the world's resource explicitly (Some(None)) — distinct from "unset".
    let clear = CombatDefaults {
        movement_resource: Some(None),
        ..Default::default()
    };
    assert_eq!(
        resolve_combat_rules(None, Some(&world), Some(&clear))
            .movement
            .resource,
        None
    );
}

#[test]
fn resolve_combat_rules_system_layer_sits_under_world_and_scene() {
    let system = CombatDefaults {
        enforcement: Some(Enforcement::Hard),
        turn_control: Some(TurnControl::GmOnly),
        ..Default::default()
    };
    let world = CombatDefaults {
        enforcement: Some(Enforcement::Warn),
        ..Default::default()
    };
    let r = resolve_combat_rules(Some(&system), Some(&world), None);
    assert_eq!(r.movement.enforcement, Enforcement::Warn);
    assert_eq!(r.turn_control, TurnControl::GmOnly);
    let scene = CombatDefaults {
        movement_resource: Some(None),
        ..Default::default()
    };
    let system = CombatDefaults {
        movement_resource: Some(Some("movement".into())),
        ..Default::default()
    };
    let r = resolve_combat_rules(Some(&system), None, Some(&scene));
    assert_eq!(
        r.movement.resource, None,
        "a scene clear beats a system-supplied resource"
    );
}

#[test]
fn combat_defaults_absent_and_null_fields_both_read_as_unset() {
    let d: CombatDefaults = serde_json::from_value(json!({ "interpretation": null })).unwrap();
    assert_eq!(d, CombatDefaults::default());
}

#[test]
fn resolve_combat_rules_folds_cleanup_and_restore_flags() {
    let r = resolve_combat_rules(None, None, None);
    assert!(r.effect_cleanup && r.rewind_restore && !r.forward_restore);
    let world = CombatDefaults {
        effect_cleanup: Some(false),
        forward_restore: Some(true),
        ..Default::default()
    };
    let r = resolve_combat_rules(None, Some(&world), None);
    assert!(!r.effect_cleanup && r.forward_restore);
}

#[test]
fn expiry_notice_resolves_scene_over_world_over_system_over_visible() {
    assert_eq!(
        resolve_combat_rules(None, None, None).expiry_notice,
        ExpiryNotice::Visible
    );
    let gm_only = CombatDefaults {
        expiry_notice: Some(ExpiryNotice::GmOnly),
        ..Default::default()
    };
    let visible = CombatDefaults {
        expiry_notice: Some(ExpiryNotice::Visible),
        ..Default::default()
    };
    assert_eq!(
        resolve_combat_rules(Some(&gm_only), None, None).expiry_notice,
        ExpiryNotice::GmOnly,
        "the system layer applies when nothing above sets it"
    );
    assert_eq!(
        resolve_combat_rules(Some(&gm_only), Some(&visible), None).expiry_notice,
        ExpiryNotice::Visible,
        "the world layer beats the system layer"
    );
    assert_eq!(
        resolve_combat_rules(Some(&visible), Some(&visible), Some(&gm_only)).expiry_notice,
        ExpiryNotice::GmOnly,
        "the scene layer beats both"
    );
}

#[test]
fn expiry_notice_reads_from_its_camel_case_wire_key() {
    let d: CombatDefaults = serde_json::from_value(json!({ "expiryNotice": "gm_only" })).unwrap();
    assert_eq!(d.expiry_notice, Some(ExpiryNotice::GmOnly));
}

/// An effect body with a given `origin` and nothing else set.
fn effect_with_origin(origin: Option<EffectOrigin>) -> EffectEngine {
    EffectEngine {
        active: true,
        transfer: false,
        duration: None,
        lifecycle: None,
        origin,
    }
}

#[test]
fn an_effect_body_without_origin_still_deserializes_as_none() {
    let e: EffectEngine =
        serde_json::from_value(json!({ "active": true, "transfer": false, "duration": null }))
            .unwrap();
    assert_eq!(e.origin, None);
    assert!(e.validate().is_ok());
}

#[test]
fn an_effect_origin_round_trips_and_an_unknown_origin_key_is_refused() {
    let origin = EffectOrigin {
        source: Some(crate::test_ids::test_uuid(5)),
        label: Some("Aldric".into()),
    };
    let e = effect_with_origin(Some(origin.clone()));
    let back: EffectEngine = serde_json::from_value(serde_json::to_value(&e).unwrap()).unwrap();
    assert_eq!(back.origin, Some(origin));
    let bad = serde_json::from_value::<EffectEngine>(json!({
        "active": true, "origin": { "label": "x", "extra": 1 }
    }));
    assert!(
        bad.is_err(),
        "deny_unknown_fields reaches the nested origin"
    );
}

#[test]
fn a_turn_history_effect_snapshot_round_trips_its_origin() {
    let snapshot = EffectSnapshot {
        host: crate::test_ids::test_uuid(9),
        path: "/embedded/effect/0".into(),
        engine: effect_with_origin(Some(EffectOrigin {
            source: None,
            label: Some("Aldric".into()),
        })),
    };
    let back: EffectSnapshot =
        serde_json::from_value(serde_json::to_value(&snapshot).unwrap()).unwrap();
    assert_eq!(back, snapshot);
}

#[test]
fn an_effect_origin_needs_a_source_or_a_label_and_bounds_the_label() {
    let with_label = |label: &str| {
        effect_with_origin(Some(EffectOrigin {
            source: None,
            label: Some(label.into()),
        }))
    };
    assert!(effect_with_origin(Some(EffectOrigin {
        source: None,
        label: None
    }))
    .validate()
    .is_err());
    assert!(with_label("").validate().is_err());
    assert!(with_label(&"x".repeat(MAX_ORIGIN_LABEL_CHARS))
        .validate()
        .is_ok());
    assert!(with_label(&"x".repeat(MAX_ORIGIN_LABEL_CHARS + 1))
        .validate()
        .is_err());
    assert!(with_label("two\nlines").validate().is_err());
    let dangling = effect_with_origin(Some(EffectOrigin {
        source: Some(crate::test_ids::test_uuid(404)),
        label: None,
    }));
    assert!(dangling.validate().is_ok(), "a source need not resolve");
}

#[test]
fn combat_history_records_are_capped_and_cursor_in_range() {
    let rec = |round| TurnRecord {
        round,
        turn: crate::test_ids::test_uuid(1),
        combatants: vec![],
        effects: vec![],
    };
    let h = CombatHistoryEngine {
        records: (0..MAX_TURN_HISTORY as u32 + 1).map(rec).collect(),
        cursor: 0,
    };
    assert!(h.validate().is_err());
    let h = CombatHistoryEngine {
        records: vec![rec(1), rec(1)],
        cursor: 2,
    };
    assert!(h.validate().is_err());
    let h = CombatHistoryEngine {
        records: vec![rec(1)],
        cursor: 0,
    };
    assert!(h.validate().is_ok());
}

/// The engine-fallback `ResolvedCombatRules` (`resolve_combat_rules(None, None, None)`),
/// serialized into the client camelCase `CombatDefaults` spelling, must byte-equal the JSON
/// fixture the client Vitest suite reads as `ENGINE_COMBAT_DEFAULTS`. Either side drifting
/// fails one of the two tests reading this single fixture.
#[test]
fn engine_combat_defaults_matches_the_shared_fixture() {
    let r = resolve_combat_rules(None, None, None);
    let client_spelling = json!({
        "movementResource": r.movement.resource,
        "interpretation": r.movement.interpretation,
        "enforcement": r.movement.enforcement,
        "turnControl": r.turn_control,
        "effectCleanup": r.effect_cleanup,
        "effectLifecycle": {
            "onCombatEnd": r.effect_lifecycle.on_combat_end,
            "onTurnEnd": r.effect_lifecycle.on_turn_end,
            "onAdvance": r.effect_lifecycle.on_advance,
        },
        "rewindRestore": r.rewind_restore,
        "forwardRestore": r.forward_restore,
        "expiryNotice": r.expiry_notice,
    });
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../client/core/src/__fixtures__/engine-combat-defaults.json"
    ))
    .expect("engine-combat-defaults.json parses");
    assert_eq!(client_spelling, fixture);
}

/// One case of the shared combat-provenance fixture. `source` is deliberately absent —
/// which resolution layer supplied a value is not observable server-side, so this side
/// pins only the VALUE `resolve_combat_rules` produces; the Vitest side
/// (`resolveSettingProvenance matches the shared cross-language fixture for every new
/// combat leaf`) asserts both value and source over the same case list.
#[derive(Deserialize)]
struct ProvenanceCase {
    #[serde(default)]
    system: Option<CombatDefaults>,
    #[serde(default)]
    world: Option<CombatDefaults>,
    #[serde(default)]
    scene: Option<CombatDefaults>,
    path: String,
    expect: ProvenanceExpect,
}

#[derive(Deserialize)]
struct ProvenanceExpect {
    value: serde_json::Value,
}

/// Pins `resolve_combat_rules`'s resolved VALUE for every case in the shared
/// combat-provenance fixture against the client's `resolveSettingProvenance`
/// mirror — the same never-fork discipline as `engine_combat_defaults_matches_the_shared_fixture`.
#[test]
fn resolve_combat_rules_matches_the_shared_provenance_fixture() {
    let cases: Vec<ProvenanceCase> = serde_json::from_str(include_str!(
        "../../../../../client/core/src/__fixtures__/combat-provenance-cases.json"
    ))
    .expect("combat-provenance-cases.json parses");
    for c in cases {
        let r = resolve_combat_rules(c.system.as_ref(), c.world.as_ref(), c.scene.as_ref());
        let got = match c.path.as_str() {
            "combat.effectCleanup" => json!(r.effect_cleanup),
            "combat.rewindRestore" => json!(r.rewind_restore),
            "combat.forwardRestore" => json!(r.forward_restore),
            "combat.expiryNotice" => json!(r.expiry_notice),
            "combat.effectLifecycle.onCombatEnd" => json!(r.effect_lifecycle.on_combat_end),
            "combat.effectLifecycle.onTurnEnd" => json!(r.effect_lifecycle.on_turn_end),
            "combat.effectLifecycle.onAdvance" => json!(r.effect_lifecycle.on_advance),
            other => panic!("unhandled fixture path: {other}"),
        };
        assert!(
            provenance_values_eq(&got, &c.expect.value),
            "case path {}: got {got:?}, expected {:?}",
            c.path,
            c.expect.value
        );
    }
}

/// Loose numeric equality for a provenance fixture value: a JSON integer literal (`0`) and a
/// Rust-serialized `f64` (`0.0`) parse into different `serde_json::Number` variants, so plain
/// `Value` equality would fail on an otherwise-matching case whose expected value is a whole
/// number — `Formula::Number` is always `f64`, but the fixture is shared with the client side,
/// which writes whole numbers as plain JSON integers.
fn provenance_values_eq(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    match (a, b) {
        (serde_json::Value::Number(x), serde_json::Value::Number(y)) => x.as_f64() == y.as_f64(),
        _ => a == b,
    }
}
