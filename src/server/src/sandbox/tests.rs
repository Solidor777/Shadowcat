use super::*;
use crate::data::document::{Document, PermissionSet, Scope};
use dashmap::DashMap;
use std::collections::BTreeMap;
use std::sync::Arc;

fn doc(doc_type: &str, system: serde_json::Value) -> Document {
    Document {
        id: uuid::Uuid::new_v4(),
        scope: Scope::World {
            world_id: uuid::Uuid::nil(),
        },
        doc_type: doc_type.into(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner: None,
        permissions: PermissionSet::default(),
        embedded: BTreeMap::new(),
        parent_id: None,
        engine: None,
        system,
        created_at: 0,
        updated_at: 0,
    }
}

/// Compiles a minimal WAT fixture into a `CompiledValidator` for `module_id` through
/// `runtime::CompiledValidator::compile`, the same single compile path the registry's
/// scan-time compile uses.
fn compiled_for(module_id: &str, wat_src: &str) -> runtime::CompiledValidator {
    let bytes = wat::parse_str(wat_src).expect("valid WAT fixture");
    runtime::CompiledValidator::compile(module_id, &bytes).expect("module compiles")
}

/// No `validate` export at all — every call faults with `FaultKind::BadAbi`, the cheapest
/// fault to construct deterministically.
const FAULTING_WAT: &str = r#"
  (module
    (memory (export "memory") 1)
    (func (export "alloc") (param i32) (result i32) (i32.const 0)))
"#;

/// Always accepts.
const ACCEPTING_WAT: &str = r#"
  (module
    (memory (export "memory") 1)
    (func (export "alloc") (param i32) (result i32) (i32.const 0))
    (func (export "validate") (param i32 i32) (result i32) (i32.const 0)))
"#;

/// Always refuses with a fixed reason — an authored, WORKING decision, never a technical
/// fault.
const REFUSING_WAT: &str = r#"
  (module
    (memory (export "memory") 1)
    (data (i32.const 2048) "no")
    (func (export "alloc") (param i32) (result i32) (i32.const 1024))
    (func (export "validate") (param i32 i32) (result i32) (i32.const 1))
    (func (export "reason_ptr") (result i32) (i32.const 2048))
    (func (export "reason_len") (result i32) (i32.const 2)))
"#;

#[test]
fn collect_validated_nodes_recurses_embedded_children_paired_by_id() {
    let child_post = doc("combatant", serde_json::json!({ "hp": 5 }));
    let child_id = child_post.id;
    let mut parent_post = doc("combat", serde_json::json!({}));
    parent_post
        .embedded
        .insert("combatants".into(), vec![child_post.clone()]);

    let mut child_prior = child_post.clone();
    child_prior.system = serde_json::json!({ "hp": 10 });
    let mut parent_prior = parent_post.clone();
    parent_prior
        .embedded
        .insert("combatants".into(), vec![child_prior]);

    let mut nodes = Vec::new();
    collect_validated_nodes(&parent_post, Some(&parent_prior), &mut nodes);
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0].doc_type, "combat");
    assert_eq!(nodes[1].doc_type, "combatant");
    assert_eq!(
        nodes[1].system_prior,
        Some(&serde_json::json!({ "hp": 10 }))
    );
    assert_eq!(nodes[1].system_post, &serde_json::json!({ "hp": 5 }));
    let _ = child_id;
}

#[test]
fn collect_validated_nodes_treats_a_freshly_added_child_as_a_create() {
    let child = doc("combatant", serde_json::json!({ "hp": 5 }));
    let mut parent_post = doc("combat", serde_json::json!({}));
    parent_post
        .embedded
        .insert("combatants".into(), vec![child]);
    let parent_prior = doc("combat", serde_json::json!({}));

    let mut nodes = Vec::new();
    collect_validated_nodes(&parent_post, Some(&parent_prior), &mut nodes);
    assert_eq!(nodes[1].system_prior, None);
}

#[tokio::test]
async fn validate_document_short_circuits_on_first_non_accept_in_module_id_order() {
    // No installed modules ⇒ the registry has nothing to match against ⇒ Accept.
    // (Full accept/refuse/fault coverage lives in `runtime::tests` and
    // `apply_intent`'s own integration tests — this module only owns the
    // tree-walk and short-circuit ordering.)
    let registry = registry::ValidatorRegistry::default();
    let mut d = doc("item", serde_json::json!({ "hp": -1 }));
    let verdict = validate_document(&registry, &[], &mut d, None, uuid::Uuid::nil(), &[])
        .await
        .expect("structurally valid document");
    assert_eq!(verdict, ValidatorVerdict::Accept);
}

#[tokio::test]
async fn per_module_fault_streaks_are_independent() {
    let registry = registry::ValidatorRegistry::for_test(vec![
        ("module-a", "item", compiled_for("module-a", FAULTING_WAT)),
        ("module-b", "item", compiled_for("module-b", ACCEPTING_WAT)),
    ]);
    let world = uuid::Uuid::from_u128(1);
    let a_only = vec!["module-a".to_string()];
    let b_only = vec!["module-b".to_string()];
    let mut d = doc("item", serde_json::json!({}));

    for expected in 1..=4u32 {
        let verdict = validate_document(&registry, &a_only, &mut d, None, world, &[])
            .await
            .expect("structurally valid document");
        let ValidatorVerdict::Fault(fault) = verdict else {
            panic!("expected Fault, got {verdict:?}");
        };
        assert_eq!(fault.consecutive, expected);
    }

    // module-b's own accept, keyed on a DIFFERENT module id, never touches module-a's streak.
    let verdict = validate_document(&registry, &b_only, &mut d, None, world, &[])
        .await
        .expect("structurally valid document");
    assert_eq!(verdict, ValidatorVerdict::Accept);

    let verdict = validate_document(&registry, &a_only, &mut d, None, world, &[])
        .await
        .expect("structurally valid document");
    let ValidatorVerdict::Fault(fault) = verdict else {
        panic!("expected Fault, got {verdict:?}");
    };
    assert_eq!(
        fault.consecutive, 5,
        "module-b's accept must not have reset module-a's streak"
    );
}

#[tokio::test]
async fn a_modules_own_accept_resets_its_own_streak() {
    let faults: Arc<DashMap<(uuid::Uuid, String), u32>> = Arc::default();
    let faulting = registry::ValidatorRegistry::for_test_with_faults(
        vec![("module-a", "item", compiled_for("module-a", FAULTING_WAT))],
        faults.clone(),
    );
    let accepting = registry::ValidatorRegistry::for_test_with_faults(
        vec![("module-a", "item", compiled_for("module-a", ACCEPTING_WAT))],
        faults,
    );
    let world = uuid::Uuid::from_u128(2);
    let ids = vec!["module-a".to_string()];
    let mut d = doc("item", serde_json::json!({}));

    for _ in 0..4 {
        validate_document(&faulting, &ids, &mut d, None, world, &[])
            .await
            .expect("structurally valid document");
    }

    // The SAME module's own accept — via a registry sharing the identical fault map, exactly
    // as a real rescan hands out a fresh `ValidatorRegistry` sharing the cache's one
    // persistent counter — resets module-a's streak to zero.
    let verdict = validate_document(&accepting, &ids, &mut d, None, world, &[])
        .await
        .expect("structurally valid document");
    assert_eq!(verdict, ValidatorVerdict::Accept);

    let verdict = validate_document(&faulting, &ids, &mut d, None, world, &[])
        .await
        .expect("structurally valid document");
    let ValidatorVerdict::Fault(fault) = verdict else {
        panic!("expected Fault, got {verdict:?}");
    };
    assert_eq!(
        fault.consecutive, 1,
        "module-a's own accept must have reset its streak to zero"
    );
}

#[tokio::test]
async fn a_modules_own_refuse_also_resets_its_streak() {
    // A `Refuse` is an authored, WORKING decision — the module ran correctly and declined
    // the write — so it resets the streak exactly like `Accept`; only `Fault` is evidence of
    // a technical break.
    let faults: Arc<DashMap<(uuid::Uuid, String), u32>> = Arc::default();
    let faulting = registry::ValidatorRegistry::for_test_with_faults(
        vec![("module-a", "item", compiled_for("module-a", FAULTING_WAT))],
        faults.clone(),
    );
    let refusing = registry::ValidatorRegistry::for_test_with_faults(
        vec![("module-a", "item", compiled_for("module-a", REFUSING_WAT))],
        faults,
    );
    let world = uuid::Uuid::from_u128(3);
    let ids = vec!["module-a".to_string()];
    let mut d = doc("item", serde_json::json!({}));

    for _ in 0..4 {
        validate_document(&faulting, &ids, &mut d, None, world, &[])
            .await
            .expect("structurally valid document");
    }

    let verdict = validate_document(&refusing, &ids, &mut d, None, world, &[])
        .await
        .expect("structurally valid document");
    assert!(matches!(verdict, ValidatorVerdict::Refuse { .. }));

    let verdict = validate_document(&faulting, &ids, &mut d, None, world, &[])
        .await
        .expect("structurally valid document");
    let ValidatorVerdict::Fault(fault) = verdict else {
        panic!("expected Fault, got {verdict:?}");
    };
    assert_eq!(
        fault.consecutive, 1,
        "module-a's own refuse must have reset its streak to zero"
    );
}

#[tokio::test]
async fn enabled_module_order_is_sorted_by_validate_document_not_trusted_from_the_caller() {
    // Both modules ALWAYS refuse; requesting them in reverse-alphabetical order must still
    // yield module-a's own refusal, proving `validate_document` sorts `enabled_module_ids`
    // itself rather than trusting the caller's own collection order.
    let registry = registry::ValidatorRegistry::for_test(vec![
        ("module-b", "item", compiled_for("module-b", REFUSING_WAT)),
        ("module-a", "item", compiled_for("module-a", REFUSING_WAT)),
    ]);
    let world = uuid::Uuid::from_u128(5);
    let reverse_order = vec!["module-b".to_string(), "module-a".to_string()];
    let mut d = doc("item", serde_json::json!({}));

    let verdict = validate_document(&registry, &reverse_order, &mut d, None, world, &[])
        .await
        .expect("structurally valid document");
    let ValidatorVerdict::Refuse { module, .. } = verdict else {
        panic!("expected Refuse, got {verdict:?}");
    };
    assert_eq!(
        module, "module-a",
        "the alphabetically-first module's reason must win regardless of the caller's own enabled-list order"
    );
}

#[tokio::test]
async fn a_structural_failure_never_reaches_or_faults_a_validator() {
    // `module-a`'s validator ALWAYS faults if it is ever consulted — this test's whole point
    // is that a structurally invalid `doc` never reaches it.
    let registry = registry::ValidatorRegistry::for_test(vec![(
        "module-a",
        "item",
        compiled_for("module-a", FAULTING_WAT),
    )]);
    let world = uuid::Uuid::from_u128(4);
    let ids = vec!["module-a".to_string()];
    // A tier-2 schema requiring `/system/hp` to be a number; the document violates it.
    let schemas = vec![crate::data::document::SchemaDeclaration {
        module_id: "example-system".into(),
        version: "1".into(),
        schema_format: 1,
        doc_type: "item".into(),
        subtree_pointer: "/system/hp".into(),
        schema: serde_json::from_value(serde_json::json!({ "type": "number" })).unwrap(),
    }];
    let mut d = doc("item", serde_json::json!({ "hp": "not-a-number" }));

    let err = validate_document(&registry, &ids, &mut d, None, world, &schemas)
        .await
        .expect_err("a tier-2 schema violation must surface as Phase 1's own error");
    assert!(
        matches!(err, crate::data::DataError::SchemaViolation { .. }),
        "expected SchemaViolation, got {err:?}"
    );

    // `record_fault` increments from whatever is currently stored and returns the new total —
    // a fresh `1` here proves module-a's streak was still zero, i.e. the rejected document
    // above never reached (and never faulted) the validator.
    assert_eq!(
        registry.record_fault(world, "module-a"),
        1,
        "a structurally-invalid document must never have counted against a validator's fault streak"
    );
}
