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
fn combatant_numbers_must_be_finite_and_max_non_negative() {
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
            current: 30.0,
            max: -1.0,
        },
    );
    assert!(c.validate().is_err());
    c.resources.insert(
        "movement".into(),
        CombatantResource {
            current: 30.0,
            max: 30.0,
        },
    );
    assert!(c.validate().is_ok());
}

#[test]
fn formula_text_is_bounded_and_non_empty() {
    let mut reg = ResourceRegistryEngine {
        resources: Default::default(),
    };
    reg.resources.insert(
        "m".into(),
        Resource {
            name: "M".into(),
            order: 0,
            binding: ResourceBinding::Mirror {
                value: Formula::Text(String::new()),
            },
        },
    );
    assert!(reg.validate().is_err());
    reg.resources.insert(
        "m".into(),
        Resource {
            name: "M".into(),
            order: 0,
            binding: ResourceBinding::Mirror {
                value: Formula::Text("x".repeat(MAX_FORMULA_CHARS + 1)),
            },
        },
    );
    assert!(reg.validate().is_err());
    reg.resources.insert(
        "m".into(),
        Resource {
            name: "M".into(),
            order: 0,
            binding: ResourceBinding::Mirror {
                value: Formula::Text("hp".into()),
            },
        },
    );
    assert!(reg.validate().is_ok());
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
            amount: 0,
            unit: DurationUnit::Rounds,
            anchor: None,
            expires: ExpiryPoint::TurnEnd,
            started: ClockStamp {
                round: 1,
                turn_index: 0,
            },
        }),
    };
    assert!(e.validate().is_err());
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
        }
    );
    let world = CombatDefaults {
        movement_resource: Some(Some("movement".into())),
        interpretation: Some(Interpretation::Spaces),
        enforcement: Some(Enforcement::Hard),
        turn_control: Some(TurnControl::GmOnly),
    };
    let scene = CombatDefaults {
        movement_resource: Some(Some("ship".into())),
        interpretation: None,
        enforcement: Some(Enforcement::Warn),
        turn_control: None,
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
