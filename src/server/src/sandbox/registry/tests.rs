use super::*;

#[test]
fn scan_of_a_missing_modules_dir_yields_an_empty_registry() {
    let registry =
        ValidatorRegistry::scan(std::path::Path::new("no-such-modules-dir"), Arc::default());
    assert!(registry.validator_for("anything", "actor").is_none());
}

#[test]
fn cache_returns_the_same_registry_on_a_second_unchanged_scan() {
    let dir = std::env::temp_dir().join(format!(
        "shadowcat-sandbox-registry-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let cache = ValidatorRegistryCache::default();
    let first = cache.get_or_scan(&dir);
    let second = cache.get_or_scan(&dir);
    assert!(Arc::ptr_eq(&first, &second));
    std::fs::remove_dir_all(&dir).ok();
}

/// Writes a minimal installed-module folder under `dir/<id>/` declaring one validator for
/// `doc_type`, compiled from `wat_src`.
fn write_installed_module(dir: &std::path::Path, id: &str, doc_type: &str, wat_src: &str) {
    let module_dir = dir.join(id);
    std::fs::create_dir_all(&module_dir).unwrap();
    std::fs::write(
        module_dir.join("module.json"),
        serde_json::json!({
            "id": id,
            "version": "1.0.0",
            "validators": [{ "docType": doc_type, "wasm": "v.wasm" }],
        })
        .to_string(),
    )
    .unwrap();
    let bytes = wat::parse_str(wat_src).expect("valid WAT fixture");
    std::fs::write(module_dir.join("v.wasm"), bytes).unwrap();
}

/// No `validate` export at all — every call faults with `FaultKind::BadAbi`.
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

fn doc(doc_type: &str, system: serde_json::Value) -> crate::data::document::Document {
    crate::data::document::Document {
        id: uuid::Uuid::new_v4(),
        scope: crate::data::document::Scope::World {
            world_id: uuid::Uuid::nil(),
        },
        doc_type: doc_type.into(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner: None,
        permissions: crate::data::document::PermissionSet::default(),
        embedded: std::collections::BTreeMap::new(),
        parent_id: None,
        engine: None,
        system,
        created_at: 0,
        updated_at: 0,
    }
}

#[test]
fn scan_assigns_each_compiled_validator_the_installed_modules_own_id() {
    // Both modules declare a validator for the SAME doc_type — the only way to prove
    // `compile_one` stamps `module_id` from the INSTALLED module, never from `decl.doc_type`.
    let dir = std::env::temp_dir().join(format!(
        "shadowcat-sandbox-scan-ids-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    write_installed_module(&dir, "module-a", "actor", FAULTING_WAT);
    write_installed_module(&dir, "module-b", "actor", ACCEPTING_WAT);

    let registry = ValidatorRegistry::scan(&dir, Arc::default());
    let a = registry
        .validator_for("module-a", "actor")
        .expect("module-a's validator compiled");
    let b = registry
        .validator_for("module-b", "actor")
        .expect("module-b's validator compiled");
    assert_eq!(
        a.module_id, "module-a",
        "module_id must be the INSTALLED MODULE id, never the doc_type it validates"
    );
    assert_eq!(b.module_id, "module-b");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_validator_that_fails_to_compile_records_a_load_error_and_keeps_its_siblings() {
    let dir = std::env::temp_dir().join(format!(
        "shadowcat-sandbox-scan-load-error-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    write_installed_module(&dir, "module-a", "actor", ACCEPTING_WAT);
    // A declared `.wasm` that is not valid WASM at all: the entry fails, the module still
    // scans, and the diagnostic is recorded for `load_error_for`.
    let bad_dir = dir.join("module-bad");
    std::fs::create_dir_all(&bad_dir).unwrap();
    std::fs::write(
        bad_dir.join("module.json"),
        serde_json::json!({
            "id": "module-bad",
            "version": "1.0.0",
            "validators": [{ "docType": "actor", "wasm": "v.wasm" }],
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(bad_dir.join("v.wasm"), b"not wasm".as_slice()).unwrap();

    let registry = ValidatorRegistry::scan(&dir, Arc::default());
    assert!(registry.validator_for("module-a", "actor").is_some());
    assert!(registry.validator_for("module-bad", "actor").is_none());
    assert!(
        registry.load_error_for("module-bad").is_some(),
        "a failed compile must be recorded, fail-open on discovery"
    );
    assert!(registry.load_error_for("module-a").is_none());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_wasm_path_escaping_the_module_folder_is_refused() {
    let dir = std::env::temp_dir().join(format!(
        "shadowcat-sandbox-scan-traversal-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    write_installed_module(&dir, "module-a", "actor", ACCEPTING_WAT);
    // `wasm` names a path ABOVE the module's own folder (the sibling module's compiled
    // validator): refused by `compile_one`'s `is_strictly_within` boundary, recorded as this
    // module's load error.
    let escape_dir = dir.join("module-escape");
    std::fs::create_dir_all(&escape_dir).unwrap();
    std::fs::write(
        escape_dir.join("module.json"),
        serde_json::json!({
            "id": "module-escape",
            "version": "1.0.0",
            "validators": [{ "docType": "actor", "wasm": "../module-a/v.wasm" }],
        })
        .to_string(),
    )
    .unwrap();

    let registry = ValidatorRegistry::scan(&dir, Arc::default());
    assert!(registry.validator_for("module-escape", "actor").is_none());
    assert_eq!(
        registry.load_error_for("module-escape"),
        Some("wasm path escapes the module's own folder")
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn validate_document_over_a_scanned_registry_isolates_each_modules_fault_streak() {
    let dir = std::env::temp_dir().join(format!(
        "shadowcat-sandbox-scan-streak-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    write_installed_module(&dir, "module-a", "item", FAULTING_WAT);
    write_installed_module(&dir, "module-b", "item", ACCEPTING_WAT);

    let registry = ValidatorRegistry::scan(&dir, Arc::default());
    let world = uuid::Uuid::from_u128(42);
    let a_only = vec!["module-a".to_string()];
    let b_only = vec!["module-b".to_string()];
    let mut d = doc("item", serde_json::json!({}));

    for expected in 1..=3u32 {
        let verdict =
            crate::sandbox::validate_document(&registry, &a_only, &mut d, None, true, world, &[])
                .await
                .expect("structurally valid document");
        let crate::sandbox::ValidatorVerdict::Fault(fault) = verdict else {
            panic!("expected Fault from module-a, got {verdict:?}");
        };
        assert_eq!(fault.module, "module-a");
        assert_eq!(fault.consecutive, expected);
    }

    // module-b's own accept, resolved through the SAME scanned registry but keyed on a
    // DIFFERENT installed-module id (both modules here declare the SAME doc_type, "item"),
    // must never touch module-a's streak.
    let verdict =
        crate::sandbox::validate_document(&registry, &b_only, &mut d, None, true, world, &[])
            .await
            .expect("structurally valid document");
    assert_eq!(verdict, crate::sandbox::ValidatorVerdict::Accept);

    let verdict =
        crate::sandbox::validate_document(&registry, &a_only, &mut d, None, true, world, &[])
            .await
            .expect("structurally valid document");
    let crate::sandbox::ValidatorVerdict::Fault(fault) = verdict else {
        panic!("expected Fault from module-a, got {verdict:?}");
    };
    assert_eq!(
        fault.consecutive, 4,
        "module-b's accept must not have reset or incremented module-a's streak"
    );

    std::fs::remove_dir_all(&dir).ok();
}
