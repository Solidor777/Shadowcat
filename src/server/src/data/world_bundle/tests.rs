use super::*;

fn sample_manifest() -> BundleManifest {
    let mut row_counts = BTreeMap::new();
    row_counts.insert("documents".to_string(), 1);
    BundleManifest {
        schema_version: BUNDLE_SCHEMA_VERSION,
        world_id: Uuid::from_u128(1),
        world_name: "MOCK_WORLD_A".to_string(),
        world_seq: 42,
        world_created_at: 1000,
        world_updated_at: 2000,
        exported_at_unix_ms: 3000,
        row_counts,
    }
}

#[test]
fn manifest_round_trips_through_json() {
    let manifest = sample_manifest();
    let json = serde_json::to_string(&manifest).unwrap();
    let back: BundleManifest = serde_json::from_str(&json).unwrap();
    assert_eq!(manifest, back);
}

#[test]
fn manifest_carries_world_seq_independent_of_row_counts() {
    // `world_seq` must survive the round trip distinctly from any row
    // count, so an importer can restore `worlds.seq` without conflating
    // it with (e.g.) the document row count.
    let manifest = sample_manifest();
    assert_eq!(manifest.world_seq, 42);
    assert_ne!(
        manifest.world_seq as usize,
        *manifest.row_counts.get("documents").unwrap()
    );
}
