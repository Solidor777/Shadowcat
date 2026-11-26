use super::*;

fn write_module(dir: &Path, folder: &str, json: &str) {
    let module_dir = dir.join(folder);
    std::fs::create_dir_all(&module_dir).unwrap();
    std::fs::write(module_dir.join("module.json"), json).unwrap();
}

#[test]
fn missing_modules_dir_yields_empty() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("does-not-exist");
    assert!(scan_installed_modules(&missing).is_empty());
}

#[test]
fn discovers_a_valid_module_with_default_entry() {
    let dir = tempfile::tempdir().unwrap();
    write_module(
        dir.path(),
        "actors-plus",
        r#"{"id":"actors-plus","version":"1.0.0","engines":{"shadowcat":"^0.1.0"}}"#,
    );
    let found = scan_installed_modules(dir.path());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, "actors-plus");
    assert_eq!(found[0].entry_url, "/modules/actors-plus/index.js");
    assert_eq!(found[0].engines_shadowcat.as_deref(), Some("^0.1.0"));
    assert_eq!(found[0].manifest_json["id"], "actors-plus");
}

#[test]
fn a_declared_id_that_is_not_the_folder_name_still_loads_under_the_folder_name() {
    let dir = tempfile::tempdir().unwrap();
    write_module(
        dir.path(),
        "on-disk-name",
        r#"{"id":"declared-name","version":"1.0.0"}"#,
    );
    let found = scan_installed_modules(dir.path());
    // The folder name wins everywhere the id is used as a key, and the mismatch does not
    // reject the module — it is reported, so a module whose declared id differs from its
    // folder keeps loading.
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, "on-disk-name");
    assert_eq!(found[0].entry_url, "/modules/on-disk-name/index.js");
    // The raw manifest is preserved verbatim, declared id included, so a consumer can still
    // see what the author wrote.
    assert_eq!(found[0].manifest_json["id"], "declared-name");
}

#[test]
fn respects_an_entry_override() {
    let dir = tempfile::tempdir().unwrap();
    write_module(
        dir.path(),
        "custom-entry",
        r#"{"id":"custom-entry","version":"1.0.0","entry":"bundle/main.js"}"#,
    );
    let found = scan_installed_modules(dir.path());
    assert_eq!(found[0].entry_url, "/modules/custom-entry/bundle/main.js");
}

#[test]
fn an_invalid_manifest_is_skipped_without_hiding_valid_siblings() {
    let dir = tempfile::tempdir().unwrap();
    write_module(dir.path(), "broken", r#"{"not-json"#); // malformed JSON
    write_module(
        dir.path(),
        "missing-fields",
        r#"{"name":"no id or version"}"#,
    );
    write_module(dir.path(), "good", r#"{"id":"good","version":"1.0.0"}"#);
    let found = scan_installed_modules(dir.path());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, "good");
}

#[test]
fn manifest_system_defaults_are_extracted_and_validated() {
    let dir = tempfile::tempdir().unwrap();
    write_module(
        dir.path(),
        "sys",
        r#"{"id":"sys","version":"1.0.0","engines":{"shadowcat":"*"},"provides":[{"contract":"shadowcat.system","cardinality":"singleton"}],"systemDefaults":{"scene":{"fog":false}}}"#,
    );
    let found = scan_installed_modules(dir.path());
    assert_eq!(found.len(), 1);
    assert!(found[0].provides_system);
    let sd = found[0]
        .system_defaults
        .as_ref()
        .expect("valid declaration extracted");
    assert_eq!(sd.scene.as_ref().unwrap().fog, Some(false));
}

#[test]
fn manifest_invalid_system_defaults_are_dropped_but_the_module_still_loads() {
    let dir = tempfile::tempdir().unwrap();
    // Unknown field: fails SystemDefaultsEngine deserialization.
    write_module(
        dir.path(),
        "bad-shape",
        r#"{"id":"bad-shape","version":"1.0.0","systemDefaults":{"bogus":1}}"#,
    );
    // Well-shaped but semantically invalid: fails validate() (speed must be > 0).
    write_module(
        dir.path(),
        "bad-value",
        r#"{"id":"bad-value","version":"1.0.0","systemDefaults":{"animation":{"speedCellsPerSec":-1.0}}}"#,
    );
    let found = scan_installed_modules(dir.path());
    assert_eq!(
        found.len(),
        2,
        "fail-open discovery: the modules still load"
    );
    assert!(found.iter().all(|m| m.system_defaults.is_none()));
}

const PLACEMENT_TRIGGER: &str =
    r#"[{"on":"enter","effect":{"type":"condition_add","condition":"burning"}}]"#;

#[test]
fn manifest_placements_are_extracted_with_defaults() {
    let dir = tempfile::tempdir().unwrap();
    write_module(
        dir.path(),
        "spells",
        &format!(
            r#"{{"id":"spells","version":"1.0.0","placements":[{{"id":"ball","who":"caster_owner","shapes":["circle"],"maxExtent":12,"triggers":{PLACEMENT_TRIGGER}}}]}}"#
        ),
    );
    let found = scan_installed_modules(dir.path());
    assert_eq!(found[0].placements.len(), 1);
    let p = &found[0].placements[0];
    assert_eq!(p.id, "ball");
    assert_eq!(p.who, PlacementWho::CasterOwner);
    assert_eq!(p.behavior, "terrain");
    assert_eq!(p.cost, 1.0);
    assert_eq!(p.triggers.len(), 1);
}

#[test]
fn manifest_invalid_and_duplicate_placements_are_dropped_but_the_module_still_loads() {
    let dir = tempfile::tempdir().unwrap();
    write_module(
        dir.path(),
        "spells",
        &format!(
            r#"{{"id":"spells","version":"1.0.0","placements":[
              {{"id":"ok","who":"player","shapes":["circle"],"maxExtent":5,"triggers":{PLACEMENT_TRIGGER}}},
              {{"id":"ok","who":"gm","shapes":["circle"],"maxExtent":5}},
              {{"id":"unknown-field","who":"gm","shapes":["circle"],"maxExtent":5,"bogus":1}},
              {{"id":"no-shapes","who":"gm","shapes":[],"maxExtent":5}},
              {{"id":"bad-shape","who":"gm","shapes":["blob"],"maxExtent":5}},
              {{"id":"zero-extent","who":"gm","shapes":["circle"],"maxExtent":0}},
              {{"id":"bad-who","who":"everyone","shapes":["circle"],"maxExtent":5}},
              "not an object"
            ]}}"#
        ),
    );
    let found = scan_installed_modules(dir.path());
    assert_eq!(
        found.len(),
        1,
        "fail-open discovery: the module still loads"
    );
    let ids: Vec<&str> = found[0].placements.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["ok"], "only the first valid declaration survives");
    assert_eq!(found[0].placements[0].who, PlacementWho::Player);
}

#[test]
fn a_placement_admits_only_its_declared_kinds_within_its_extent() {
    let decl: PlacementDecl = serde_json::from_str(
        r#"{"id":"z","who":"player","shapes":["circle","rect"],"maxExtent":10}"#,
    )
    .unwrap();
    let circle = |r: f64| crate::data::engine::RegionShape {
        kind: "circle".into(),
        points: vec![0.0, 0.0, r],
    };
    assert!(decl.admits(&circle(5.0)), "diameter 10 is at the limit");
    assert!(!decl.admits(&circle(5.1)), "diameter 10.2 exceeds it");
    let rect = crate::data::engine::RegionShape {
        kind: "rect".into(),
        points: vec![0.0, 0.0, 10.0, 4.0],
    };
    assert!(decl.admits(&rect));
    let polygon = crate::data::engine::RegionShape {
        kind: "polygon".into(),
        points: vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0],
    };
    assert!(!decl.admits(&polygon), "polygon is not declared");
}

#[test]
fn placement_duration_and_range_are_validated_at_scan() {
    let dir = tempfile::tempdir().unwrap();
    write_module(
        dir.path(),
        "spells",
        r#"{"id":"spells","version":"1.0.0","placements":[
          {"id":"ok","who":"player","shapes":["circle"],"maxExtent":5,"durationSecs":30,"maxRangeCells":6},
          {"id":"zero-duration","who":"gm","shapes":["circle"],"maxExtent":5,"durationSecs":0},
          {"id":"huge-duration","who":"gm","shapes":["circle"],"maxExtent":5,"durationSecs":99999999},
          {"id":"zero-range","who":"gm","shapes":["circle"],"maxExtent":5,"maxRangeCells":0},
          {"id":"negative-range","who":"gm","shapes":["circle"],"maxExtent":5,"maxRangeCells":-2}
        ]}"#,
    );
    let found = scan_installed_modules(dir.path());
    let ids: Vec<&str> = found[0].placements.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["ok"]);
    assert_eq!(found[0].placements[0].duration_secs, Some(30));
    assert_eq!(found[0].placements[0].max_range_cells, Some(6.0));
}

#[test]
fn a_placement_expires_duration_after_placement_and_never_without_a_duration() {
    let timed: PlacementDecl = serde_json::from_str(
        r#"{"id":"z","who":"player","shapes":["circle"],"maxExtent":10,"durationSecs":90}"#,
    )
    .unwrap();
    let forever: PlacementDecl =
        serde_json::from_str(r#"{"id":"z","who":"player","shapes":["circle"],"maxExtent":10}"#)
            .unwrap();
    assert_eq!(timed.expires_at(10_000), Some(100_000));
    assert_eq!(forever.expires_at(10_000), None);
}

#[test]
fn range_is_measured_to_the_area_centre_in_cells_and_a_malformed_area_is_out_of_range() {
    let ranged: PlacementDecl = serde_json::from_str(
        r#"{"id":"z","who":"player","shapes":["circle","rect","polygon"],"maxExtent":50,"maxRangeCells":2}"#,
    )
    .unwrap();
    let unlimited: PlacementDecl =
        serde_json::from_str(r#"{"id":"z","who":"player","shapes":["circle"],"maxExtent":50}"#)
            .unwrap();
    let at = |x: f64| crate::data::engine::RegionShape {
        kind: "circle".into(),
        points: vec![x, 0.0, 1.0],
    };
    // Two cells of 100 units: exactly at the limit is in range, a hair past it is not.
    assert!(ranged.within_range((0.0, 0.0), &at(200.0), 100.0));
    assert!(!ranged.within_range((0.0, 0.0), &at(200.5), 100.0));
    // The measure is cells: the same span at a smaller cell size is farther.
    assert!(!ranged.within_range((0.0, 0.0), &at(200.0), 50.0));
    // A rect and a polygon are measured to their bounding-box centre.
    let rect = crate::data::engine::RegionShape {
        kind: "rect".into(),
        points: vec![100.0, -50.0, 300.0, 50.0],
    };
    assert!(ranged.within_range((0.0, 0.0), &rect, 100.0));
    let polygon = crate::data::engine::RegionShape {
        kind: "polygon".into(),
        points: vec![300.0, 0.0, 500.0, 0.0, 500.0, 100.0],
    };
    assert!(!ranged.within_range((0.0, 0.0), &polygon, 100.0));
    let malformed = crate::data::engine::RegionShape {
        kind: "circle".into(),
        points: vec![0.0],
    };
    assert!(!ranged.within_range((0.0, 0.0), &malformed, 100.0));
    assert!(
        unlimited.within_range((0.0, 0.0), &at(1e9), 100.0),
        "no range declared"
    );
}

#[test]
fn a_module_without_the_system_contract_does_not_provide_system() {
    let dir = tempfile::tempdir().unwrap();
    write_module(
        dir.path(),
        "plain",
        r#"{"id":"plain","version":"1.0.0","provides":[{"contract":"x:y","cardinality":"multi"}]}"#,
    );
    let found = scan_installed_modules(dir.path());
    assert!(!found[0].provides_system);
    assert!(found[0].system_defaults.is_none());
}

#[test]
fn a_folder_with_no_module_json_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("not-a-module")).unwrap();
    assert!(scan_installed_modules(dir.path()).is_empty());
}

#[test]
fn unknown_manifest_fields_are_tolerated_forward_compatibly() {
    let dir = tempfile::tempdir().unwrap();
    write_module(
        dir.path(),
        "future",
        r#"{"id":"future","version":"1.0.0","someFutureField":{"nested":true}}"#,
    );
    let found = scan_installed_modules(dir.path());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, "future");
}

#[test]
fn discovery_order_is_deterministic() {
    let dir = tempfile::tempdir().unwrap();
    write_module(dir.path(), "zzz", r#"{"id":"zzz","version":"1.0.0"}"#);
    write_module(dir.path(), "aaa", r#"{"id":"aaa","version":"1.0.0"}"#);
    let found = scan_installed_modules(dir.path());
    assert_eq!(
        found.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["aaa", "zzz"]
    );
}

#[test]
fn manifest_validators_are_extracted() {
    let dir = tempfile::tempdir().unwrap();
    write_module(
        dir.path(),
        "validated",
        r#"{"id":"validated","version":"1.0.0","validators":[{"docType":"actor","wasm":"v.wasm"}]}"#,
    );
    let found = scan_installed_modules(dir.path());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].validators.len(), 1);
    assert_eq!(found[0].validators[0].doc_type, "actor");
    assert_eq!(found[0].validators[0].wasm, std::path::Path::new("v.wasm"));
}

#[test]
fn a_module_without_validators_scans_with_an_empty_declaration_list() {
    let dir = tempfile::tempdir().unwrap();
    write_module(dir.path(), "plain", r#"{"id":"plain","version":"1.0.0"}"#);
    let found = scan_installed_modules(dir.path());
    assert_eq!(found.len(), 1);
    assert!(found[0].validators.is_empty());
}

#[test]
fn semver_wildcard_matches_anything() {
    assert!(semver_satisfies("9.9.9", "*"));
}

#[test]
fn semver_exact_match() {
    assert!(semver_satisfies("1.2.3", "1.2.3"));
    assert!(!semver_satisfies("1.2.4", "1.2.3"));
}

#[test]
fn semver_caret_allows_same_major_gte_patch_minor() {
    assert!(semver_satisfies("1.4.0", "^1.2.3"));
    assert!(!semver_satisfies("1.2.2", "^1.2.3"));
    assert!(!semver_satisfies("2.0.0", "^1.2.3"));
}

#[test]
fn semver_caret_0_x_y_boundary_is_minor_when_major_is_zero() {
    assert!(semver_satisfies("0.2.5", "^0.2.3"));
    assert!(!semver_satisfies("0.3.0", "^0.2.3"));
}

#[test]
fn semver_caret_0_0_y_boundary_is_patch_when_major_and_minor_are_zero() {
    assert!(semver_satisfies("0.0.3", "^0.0.3"));
    assert!(!semver_satisfies("0.0.4", "^0.0.3"));
}

#[test]
fn semver_tilde_allows_same_major_minor_gte_patch() {
    assert!(semver_satisfies("1.2.9", "~1.2.3"));
    assert!(!semver_satisfies("1.3.0", "~1.2.3"));
}

#[test]
fn semver_invalid_version_fails_closed() {
    assert!(!semver_satisfies("not-a-version", "*"));
}

#[test]
fn engine_compat_ok_requires_the_engines_field() {
    let dir = tempfile::tempdir().unwrap();
    write_module(
        dir.path(),
        "no-engines",
        r#"{"id":"no-engines","version":"1.0.0"}"#,
    );
    let m = &scan_installed_modules(dir.path())[0];
    // A module with no declared compat range never enables (mandatory
    // going forward for the modules-folder pipeline).
    assert!(!engine_compat_ok(m));
}

#[test]
fn engine_compat_ok_checks_the_running_server_version() {
    let dir = tempfile::tempdir().unwrap();
    write_module(
        dir.path(),
        "compatible",
        &format!(
            r#"{{"id":"compatible","version":"1.0.0","engines":{{"shadowcat":"^{}"}}}}"#,
            env!("CARGO_PKG_VERSION")
        ),
    );
    write_module(
        dir.path(),
        "incompatible",
        r#"{"id":"incompatible","version":"1.0.0","engines":{"shadowcat":"^99.0.0"}}"#,
    );
    let found = scan_installed_modules(dir.path());
    let compatible = found.iter().find(|m| m.id == "compatible").unwrap();
    let incompatible = found.iter().find(|m| m.id == "incompatible").unwrap();
    assert!(engine_compat_ok(compatible));
    assert!(!engine_compat_ok(incompatible));
}

#[test]
fn module_scan_cache_reuses_an_equal_result_when_nothing_on_disk_changed() {
    let dir = tempfile::tempdir().unwrap();
    write_module(
        dir.path(),
        "cached-mod",
        r#"{"id":"cached-mod","version":"1.0.0"}"#,
    );
    let cache = ModuleScanCache::new();
    let first = cache.get_or_scan(dir.path());
    let second = cache.get_or_scan(dir.path());
    assert_eq!(
        first.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        second.iter().map(|m| m.id.as_str()).collect::<Vec<_>>()
    );
}

#[test]
fn module_scan_cache_detects_an_added_module_via_the_directory_mtime() {
    let dir = tempfile::tempdir().unwrap();
    write_module(
        dir.path(),
        "first-mod",
        r#"{"id":"first-mod","version":"1.0.0"}"#,
    );
    let cache = ModuleScanCache::new();
    let before = cache.get_or_scan(dir.path());
    assert_eq!(before.len(), 1);

    // Adding a new folder directly under `dir` bumps `dir`'s own mtime on
    // every platform this project targets — the install/uninstall signal.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    write_module(
        dir.path(),
        "second-mod",
        r#"{"id":"second-mod","version":"1.0.0"}"#,
    );

    let after = cache.get_or_scan(dir.path());
    assert_eq!(
        after.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["first-mod", "second-mod"]
    );
}

/// Discriminating case: with the directory otherwise unchanged (no
/// add/remove of any entry directly under `dir`), an IN-PLACE edit to an
/// already-cached module's `module.json` does NOT bump `dir`'s own mtime
/// — only the manifest file's own mtime changes. A cache keyed solely on
/// the top-level directory mtime would return the STALE pre-edit result
/// here; `ModuleScanCache` must also track each cached module's own
/// manifest mtime to catch this, matching the guarantee
/// `welcome_capability_requirements`'s own doc comment already makes
/// (re-checking `engine_compat_ok` on every Welcome, not just at enable
/// time, so an on-disk manifest edit is visible without a restart).
#[test]
fn module_scan_cache_detects_an_in_place_manifest_edit() {
    let dir = tempfile::tempdir().unwrap();
    write_module(
        dir.path(),
        "editable",
        r#"{"id":"editable","version":"1.0.0","engines":{"shadowcat":"^0.1.0"}}"#,
    );
    let cache = ModuleScanCache::new();
    let before = cache.get_or_scan(dir.path());
    assert_eq!(
        before[0].engines_shadowcat.as_deref(),
        Some("^0.1.0"),
        "precondition: original manifest content observed"
    );

    // Overwrite the SAME module's manifest in place; `dir` itself gains no
    // new/removed entry, so only the manifest file's own mtime advances.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    write_module(
        dir.path(),
        "editable",
        r#"{"id":"editable","version":"1.0.0","engines":{"shadowcat":"^99.0.0"}}"#,
    );

    let after = cache.get_or_scan(dir.path());
    assert_eq!(
        after[0].engines_shadowcat.as_deref(),
        Some("^99.0.0"),
        "an in-place manifest edit must invalidate the cache even though the \
         parent directory's own mtime never changed"
    );
}
