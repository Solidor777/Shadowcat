use super::*;

#[test]
fn scan_of_a_missing_modules_dir_yields_an_empty_registry() {
    let installed =
        crate::modules::scan_installed_modules(std::path::Path::new("no-such-modules-dir"));
    let registry = ValidatorRegistry::scan(
        &installed,
        std::path::Path::new("no-such-modules-dir"),
        Arc::default(),
    );
    assert!(registry.validator_for("anything", "actor").is_none());
}

#[test]
fn cache_returns_the_same_registry_on_a_second_unchanged_scan() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ValidatorRegistryCache::default();
    let first = cache.get_or_scan(dir.path());
    let second = cache.get_or_scan(dir.path());
    assert!(Arc::ptr_eq(&first, &second));
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

/// Always refuses with a fixed reason.
const REFUSING_WAT: &str = r#"
  (module
    (memory (export "memory") 1)
    (data (i32.const 2048) "no")
    (func (export "alloc") (param i32) (result i32) (i32.const 1024))
    (func (export "validate") (param i32 i32) (result i32) (i32.const 1))
    (func (export "reason_ptr") (result i32) (i32.const 2048))
    (func (export "reason_len") (result i32) (i32.const 2)))
"#;

/// Scans `dir` once through the shared single-walk entry point the cache uses.
fn scan_dir(dir: &std::path::Path) -> ValidatorRegistry {
    let installed = crate::modules::scan_installed_modules(dir);
    ValidatorRegistry::scan(&installed, dir, Arc::default())
}

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
    let dir = tempfile::tempdir().unwrap();
    write_installed_module(dir.path(), "module-a", "actor", FAULTING_WAT);
    write_installed_module(dir.path(), "module-b", "actor", ACCEPTING_WAT);

    let registry = scan_dir(dir.path());
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
}

#[test]
fn a_validator_that_fails_to_compile_records_a_load_error_and_keeps_its_siblings() {
    let dir = tempfile::tempdir().unwrap();
    write_installed_module(dir.path(), "module-a", "actor", ACCEPTING_WAT);
    // A declared `.wasm` that is not valid WASM at all: the entry fails, the module still
    // scans, and the diagnostic is recorded for `load_error_for`.
    let bad_dir = dir.path().join("module-bad");
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

    let registry = scan_dir(dir.path());
    assert!(registry.validator_for("module-a", "actor").is_some());
    assert!(registry.validator_for("module-bad", "actor").is_none());
    assert!(
        registry.load_error_for("module-bad").is_some(),
        "a failed compile must be recorded, fail-open on discovery"
    );
    assert!(registry.load_error_for("module-a").is_none());
}

#[test]
fn a_wasm_path_escaping_the_module_folder_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    write_installed_module(dir.path(), "module-a", "actor", ACCEPTING_WAT);
    // `wasm` names a path ABOVE the module's own folder (the sibling module's compiled
    // validator): refused by `compile_one`'s `is_strictly_within` boundary, recorded as this
    // module's load error.
    let escape_dir = dir.path().join("module-escape");
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

    let registry = scan_dir(dir.path());
    assert!(registry.validator_for("module-escape", "actor").is_none());
    assert_eq!(
        registry.load_error_for("module-escape"),
        Some("wasm path escapes the module's own folder")
    );
}

#[tokio::test]
async fn validate_document_over_a_scanned_registry_isolates_each_modules_fault_streak() {
    let dir = tempfile::tempdir().unwrap();
    write_installed_module(dir.path(), "module-a", "item", FAULTING_WAT);
    write_installed_module(dir.path(), "module-b", "item", ACCEPTING_WAT);

    let registry = scan_dir(dir.path());
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
}

#[tokio::test]
async fn cache_detects_an_in_place_wasm_swap() {
    let dir = tempfile::tempdir().unwrap();
    write_installed_module(dir.path(), "module-a", "item", ACCEPTING_WAT);
    let cache = ValidatorRegistryCache::default();
    let world = uuid::Uuid::from_u128(77);
    let ids = vec!["module-a".to_string()];
    let mut d = doc("item", serde_json::json!({}));

    let first = cache.get_or_scan(dir.path());
    let verdict =
        crate::sandbox::validate_document(&first, &ids, &mut d, None, true, world, &[]).await;
    assert_eq!(verdict.unwrap(), crate::sandbox::ValidatorVerdict::Accept);

    // An IN-PLACE wasm swap: the manifest is untouched and no directory entry is
    // added or removed, so only the `.wasm` file's own mtime advances. A cache
    // blind to that signal would keep serving the stale compiled validator.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let bytes = wat::parse_str(REFUSING_WAT).expect("valid WAT fixture");
    std::fs::write(dir.path().join("module-a").join("v.wasm"), bytes).unwrap();

    let second = cache.get_or_scan(dir.path());
    assert!(
        !Arc::ptr_eq(&first, &second),
        "an in-place wasm swap must invalidate the cached registry"
    );
    let verdict =
        crate::sandbox::validate_document(&second, &ids, &mut d, None, true, world, &[]).await;
    assert!(
        matches!(
            verdict.unwrap(),
            crate::sandbox::ValidatorVerdict::Refuse { .. }
        ),
        "the swapped-in validator must be the one that runs"
    );
}

#[tokio::test]
async fn cache_detects_a_declared_wasm_dropped_in_after_a_missing_scan() {
    let dir = tempfile::tempdir().unwrap();
    // A module declaring a validator whose `.wasm` is ABSENT at scan time:
    // the registry records the load error and caches it.
    let module_dir = dir.path().join("module-a");
    std::fs::create_dir_all(&module_dir).unwrap();
    std::fs::write(
        module_dir.join("module.json"),
        serde_json::json!({
            "id": "module-a",
            "version": "1.0.0",
            "validators": [{ "docType": "item", "wasm": "v.wasm" }],
        })
        .to_string(),
    )
    .unwrap();
    let cache = ValidatorRegistryCache::default();
    let first = cache.get_or_scan(dir.path());
    assert!(first.validator_for("module-a", "item").is_none());
    assert!(first.load_error_for("module-a").is_some());

    // Dropping the file in bumps neither the modules dir's mtime nor
    // `module.json`'s — only the declared-but-missing sentinel sees it, and it
    // must invalidate the cache so the next scan compiles the new validator.
    let bytes = wat::parse_str(ACCEPTING_WAT).expect("valid WAT fixture");
    std::fs::write(module_dir.join("v.wasm"), bytes).unwrap();

    let second = cache.get_or_scan(dir.path());
    assert!(
        !Arc::ptr_eq(&first, &second),
        "a declared-but-missing wasm appearing later must invalidate the cache"
    );
    assert!(second.validator_for("module-a", "item").is_some());
    assert!(second.load_error_for("module-a").is_none());

    let world = uuid::Uuid::from_u128(78);
    let ids = vec!["module-a".to_string()];
    let mut d = doc("item", serde_json::json!({}));
    let verdict =
        crate::sandbox::validate_document(&second, &ids, &mut d, None, true, world, &[]).await;
    assert_eq!(verdict.unwrap(), crate::sandbox::ValidatorVerdict::Accept);
}
