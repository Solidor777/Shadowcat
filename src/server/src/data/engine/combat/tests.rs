use super::*;
use serde_json::json;
use uuid::Uuid;

fn combat(order: Vec<Uuid>) -> CombatEngine {
    CombatEngine {
        scene_id: Uuid::from_u128(1),
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
    let id = Uuid::from_u128(7);
    assert!(combat(vec![id, id]).validate().is_err());
    assert!(combat(vec![id, Uuid::from_u128(8)]).validate().is_ok());
}

#[test]
fn combat_turn_must_be_in_order_when_set() {
    let id = Uuid::from_u128(7);
    let mut c = combat(vec![id]);
    c.turn = Some(Uuid::from_u128(9));
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
        resources: Default::default(),
    };
    assert!(none.validate().is_err());
    let some = CombatantEngine {
        kind: CombatantKind::Actor {
            token_id: None,
            actor_id: Some(Uuid::from_u128(3)),
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
    };
    let history = |engine: EffectEngine, combatant: CombatantEngine| CombatHistoryEngine {
        records: vec![TurnRecord {
            round: 1,
            turn: Uuid::from_u128(7),
            combatants: vec![CapturedCombatant {
                id: Uuid::from_u128(7),
                name: None,
                permissions: Default::default(),
                owner: None,
                engine: combatant,
                system: json!({}),
            }],
            effects: vec![EffectSnapshot {
                host: Uuid::from_u128(9),
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
fn combat_history_records_are_capped_and_cursor_in_range() {
    let rec = |round| TurnRecord {
        round,
        turn: Uuid::from_u128(1),
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
    });
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../client/core/src/__fixtures__/engine-combat-defaults.json"
    ))
    .expect("engine-combat-defaults.json parses");
    assert_eq!(client_spelling, fixture);
}
