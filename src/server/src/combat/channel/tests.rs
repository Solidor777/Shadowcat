//! Unit-level serde/tag checks for the combat channel wire types, independent of ECS behavior
//! (see scene::tests::combat_index::resolved_combats for the derivation tests).
use super::*;

#[test]
fn resource_binding_kind_serializes_snake_case() {
    assert_eq!(
        serde_json::to_value(ResourceBindingKind::Mirror).unwrap(),
        serde_json::json!("mirror")
    );
    assert_eq!(
        serde_json::to_value(ResourceBindingKind::Tracked).unwrap(),
        serde_json::json!("tracked")
    );
}

#[test]
fn combats_payload_round_trips_through_json() {
    let payload = CombatsPayload {
        combats: vec![CombatView {
            id: uuid::Uuid::nil(),
            scene_id: uuid::Uuid::nil(),
            combatants: vec![CombatantView {
                id: uuid::Uuid::nil(),
                resources: Some(std::collections::BTreeMap::from([(
                    "movement".to_string(),
                    ResolvedResourceView {
                        binding: ResourceBindingKind::Tracked,
                        current: Some(1.0),
                        max: Some(2.0),
                        error: None,
                    },
                )])),
                movement_cells: Some(0.5),
            }],
        }],
    };
    let v = serde_json::to_value(&payload).unwrap();
    let back: CombatsPayload = serde_json::from_value(v).unwrap();
    assert_eq!(back, payload);
}
