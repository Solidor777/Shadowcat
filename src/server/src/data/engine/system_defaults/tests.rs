use super::*;
use serde_json::json;

#[test]
fn overlay_absent_and_null_both_read_as_unset() {
    let a: SceneDefaultsOverlay = serde_json::from_value(json!({})).unwrap();
    let b: SceneDefaultsOverlay = serde_json::from_value(json!({ "fog": null })).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.fog, None);
}

#[test]
fn animation_speed_must_be_finite_and_positive() {
    let bad = SystemDefaultsEngine {
        animation: Some(AnimationOverlay {
            speed_cells_per_sec: Some(0.0),
            easing: None,
        }),
        ..Default::default()
    };
    assert!(bad.validate().is_err());
    let ok = SystemDefaultsEngine {
        animation: Some(AnimationOverlay {
            speed_cells_per_sec: Some(4.0),
            easing: None,
        }),
        ..Default::default()
    };
    assert!(ok.validate().is_ok());
}

/// Field-set parity between `WorldSceneDefaults` and its overlay: serialize a
/// fully-populated world struct, then deserialize it as the overlay with
/// `deny_unknown_fields` — every world key must be an overlay key — and check
/// the overlay has no extra `Some` slots the world struct lacks.
#[test]
fn world_scene_defaults_and_overlay_share_a_field_set() {
    let world =
        serde_json::to_value(crate::data::engine::WorldSettingsEngine::default().scene).unwrap();
    let overlay: SceneDefaultsOverlay = serde_json::from_value(world.clone()).unwrap();
    let back = serde_json::to_value(&overlay).unwrap();
    assert_eq!(
        back.as_object().unwrap().keys().collect::<Vec<_>>(),
        world.as_object().unwrap().keys().collect::<Vec<_>>()
    );
    assert!(back.as_object().unwrap().values().all(|v| !v.is_null()));
}
