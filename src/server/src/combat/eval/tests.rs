//! `combat::eval`: formula-host resolution and every combat evaluation
//! contract — host precedence, number/text evaluation, resource derivation,
//! lifecycle-flag chains, duration amounts.

use std::collections::{BTreeMap, HashMap};

use serde_json::json;
use uuid::Uuid;

use crate::combat::tests::doc;
use crate::combat::Combatant;
use crate::data::document::Document;
use crate::data::engine::combat::{
    CombatantEngine, CombatantKind, EffectLifecycle, EffectLifecycleDefaults, Formula,
    ResourceBinding, Recovery,
};
use crate::formula::FormulaErrorKind;

use super::{
    duration_amount, eval_formula, formula_host, lifecycle_flags, resolved_resource,
};

/// An actor document whose `system` band is `system`.
fn actor_with_system(id: u128, system: serde_json::Value) -> Document {
    let mut d = doc(id, "actor", None, crate::combat::tests::actor_body());
    d.system = system;
    d
}

/// A token document embedding `actor` as its instanced copy.
fn token_embedding(id: u128, actor: Document) -> Document {
    let mut t = doc(id, "token", None, json!({}));
    t.embedded.insert("actor".into(), vec![actor]);
    t
}

/// An actor-kind combatant naming `token_id`/`actor_id`.
fn combatant(id: u128, token_id: Option<Uuid>, actor_id: Option<Uuid>) -> Combatant {
    let engine = CombatantEngine {
        kind: CombatantKind::Actor { token_id, actor_id },
        initiative: None,
        tiebreak: 0.0,
        resources: BTreeMap::new(),
    };
    Combatant {
        doc: doc(id, "combatant", None, serde_json::to_value(&engine).unwrap()),
        engine,
    }
}

/// An event combatant (no hosts by construction).
fn event_combatant(id: u128) -> Combatant {
    let engine = CombatantEngine {
        kind: CombatantKind::Event {
            lifespan: None,
            message: None,
        },
        initiative: None,
        tiebreak: 0.0,
        resources: BTreeMap::new(),
    };
    Combatant {
        doc: doc(id, "combatant", None, serde_json::to_value(&engine).unwrap()),
        engine,
    }
}

#[test]
fn host_prefers_token_embedded_actor_copy_over_linked_actor() {
    let linked = actor_with_system(0xA1, json!({"who": {"is": 1.0}}));
    let embedded = actor_with_system(0xA2, json!({"who": {"is": 2.0}}));
    let token = token_embedding(0x71, embedded);
    let c = combatant(0xC1, Some(token.id), Some(linked.id));
    let hosts: HashMap<Uuid, Document> =
        [(linked.id, linked.clone()), (token.id, token)].into();
    let host = formula_host(&hosts, &c).expect("a host resolves");
    assert_eq!(host.system, json!({"who": {"is": 2.0}}));
}

#[test]
fn host_falls_back_to_linked_actor_when_token_embeds_no_copy() {
    let linked = actor_with_system(0xA1, json!({"who": {"is": 1.0}}));
    let bare_token = doc(0x71, "token", None, json!({}));
    let c = combatant(0xC1, Some(bare_token.id), Some(linked.id));
    let hosts: HashMap<Uuid, Document> =
        [(linked.id, linked.clone()), (bare_token.id, bare_token)].into();
    let host = formula_host(&hosts, &c).expect("a host resolves");
    assert_eq!(host.system, json!({"who": {"is": 1.0}}));
}

#[test]
fn host_is_none_for_an_event_combatant() {
    let hosts: HashMap<Uuid, Document> = HashMap::new();
    assert!(formula_host(&hosts, &event_combatant(0xC1)).is_none());
}

#[test]
fn host_is_none_when_every_named_host_document_is_absent() {
    let c = combatant(0xC1, Some(Uuid::from_u128(0x71)), Some(Uuid::from_u128(0xA1)));
    let hosts: HashMap<Uuid, Document> = HashMap::new();
    assert!(formula_host(&hosts, &c).is_none());
}

#[test]
fn eval_formula_passes_numbers_through_without_a_host() {
    assert_eq!(eval_formula(&Formula::Number(4.5), None), Ok(4.5));
}

#[test]
fn eval_formula_evaluates_text_over_the_host_system_band() {
    let host = actor_with_system(0xA1, json!({"stats": {"speed": {"final": 30.0}}}));
    let got = eval_formula(&Formula::Text("stats.speed.final * 2".into()), Some(&host));
    assert_eq!(got, Ok(60.0));
}

#[test]
fn eval_formula_evaluates_reference_free_text_with_no_host() {
    assert_eq!(eval_formula(&Formula::Text("2 + 3".into()), None), Ok(5.0));
}

#[test]
fn eval_formula_reports_unknown_ref_for_referencing_text_with_no_host() {
    let got = eval_formula(&Formula::Text("stats.hp".into()), None).unwrap_err();
    assert_eq!(got.error, FormulaErrorKind::UnknownRef);
    assert!(got.detail.contains("stats.hp"), "detail names the path: {}", got.detail);
}

/// A `Tracked` binding with `max` and zeroed recoveries.
fn tracked(max: Formula) -> ResourceBinding {
    ResourceBinding::Tracked {
        max,
        recover: Recovery::default(),
    }
}

#[test]
fn resolved_resource_mirror_derives_current_and_max_from_the_value_formula() {
    let host = actor_with_system(0xA1, json!({"hp": 27.0}));
    let got = resolved_resource(
        &ResourceBinding::Mirror {
            value: Formula::Text("hp".into()),
        },
        Some(999.0),
        Some(&host),
    )
    .unwrap();
    assert_eq!((got.current, got.max), (27.0, 27.0));
}

#[test]
fn resolved_resource_tracked_uses_the_stored_current_clamped_to_the_evaluated_max() {
    let host = actor_with_system(0xA1, json!({"mv": 6.0}));
    let got = resolved_resource(&tracked(Formula::Text("mv".into())), Some(9.0), Some(&host)).unwrap();
    assert_eq!((got.current, got.max), (6.0, 6.0));
    let got = resolved_resource(&tracked(Formula::Text("mv".into())), Some(2.5), Some(&host)).unwrap();
    assert_eq!((got.current, got.max), (2.5, 6.0));
}

#[test]
fn resolved_resource_tracked_absent_entry_means_full() {
    let got = resolved_resource(&tracked(Formula::Number(30.0)), None, None).unwrap();
    assert_eq!((got.current, got.max), (30.0, 30.0));
}

#[test]
fn resolved_resource_clamps_a_negative_evaluated_max_to_zero() {
    let got = resolved_resource(&tracked(Formula::Number(-4.0)), Some(3.0), None).unwrap();
    assert_eq!((got.current, got.max), (0.0, 0.0));
}

#[test]
fn resolved_resource_propagates_an_evaluation_error() {
    let got = resolved_resource(&tracked(Formula::Text("gone".into())), None, None).unwrap_err();
    assert_eq!(got.error, FormulaErrorKind::UnknownRef);
}

#[test]
fn lifecycle_flags_engine_fallbacks_apply_when_nothing_is_authored() {
    let got = lifecycle_flags(None, &EffectLifecycleDefaults::default(), None).unwrap();
    assert!(got.on_combat_end, "fallback: expire at combat end");
    assert!(!got.on_turn_end, "fallback: keep at turn end");
    assert!(got.on_advance, "fallback: decrement");
}

#[test]
fn lifecycle_flags_chain_default_beats_the_engine_fallback() {
    let defaults = EffectLifecycleDefaults {
        on_combat_end: Some(Formula::Number(0.0)),
        on_turn_end: Some(Formula::Number(1.0)),
        on_advance: Some(Formula::Number(0.0)),
    };
    let got = lifecycle_flags(None, &defaults, None).unwrap();
    assert!(!got.on_combat_end);
    assert!(got.on_turn_end);
    assert!(!got.on_advance);
}

#[test]
fn lifecycle_flags_authored_formula_beats_the_chain_default() {
    let defaults = EffectLifecycleDefaults {
        on_turn_end: Some(Formula::Number(1.0)),
        ..Default::default()
    };
    let authored = EffectLifecycle {
        on_turn_end: Some(Formula::Number(0.0)),
        ..Default::default()
    };
    let got = lifecycle_flags(Some(&authored), &defaults, None).unwrap();
    assert!(!got.on_turn_end, "authored 0 overrides the chain's 1");
}

#[test]
fn lifecycle_flags_truthiness_is_nonzero_over_the_host() {
    let host = actor_with_system(0xA1, json!({"burning": 0.0}));
    let authored = EffectLifecycle {
        on_turn_end: Some(Formula::Text("burning".into())),
        ..Default::default()
    };
    let got = lifecycle_flags(Some(&authored), &EffectLifecycleDefaults::default(), Some(&host))
        .unwrap();
    assert!(!got.on_turn_end, "0 evaluates falsy");
}

#[test]
fn lifecycle_flags_propagates_the_first_evaluation_error() {
    let authored = EffectLifecycle {
        on_combat_end: Some(Formula::Text("missing.leaf".into())),
        ..Default::default()
    };
    let got =
        lifecycle_flags(Some(&authored), &EffectLifecycleDefaults::default(), None).unwrap_err();
    assert_eq!(got.error, FormulaErrorKind::UnknownRef);
}

#[test]
fn duration_amount_floors_the_evaluated_value() {
    let host = actor_with_system(0xA1, json!({"rounds": 3.9}));
    assert_eq!(duration_amount(&Formula::Text("rounds".into()), Some(&host)), Ok(3));
}

#[test]
fn duration_amount_rejects_a_result_below_one() {
    let got = duration_amount(&Formula::Number(0.4), None).unwrap_err();
    assert_eq!(got.error, FormulaErrorKind::Type);
    assert_eq!(got.detail, "duration amount must be >= 1");
}

#[test]
fn duration_amount_passes_an_evaluation_error_through() {
    let got = duration_amount(&Formula::Text("gone".into()), None).unwrap_err();
    assert_eq!(got.error, FormulaErrorKind::UnknownRef);
}
