use super::*;

fn bare_scene() -> SceneEngine {
    SceneEngine {
        grid: Grid {
            kind: "square".to_string(),
            size: 50.0,
            distance: None,
        },
        background: None,
        bounds: None,
        padding: None,
        snap_to_grid: None,
        vision: None,
        lighting: None,
        combat: None,
        levels: Vec::new(),
        ambience: None,
    }
}

#[test]
fn scene_engine_round_trips_with_no_ambience() {
    let scene = bare_scene();
    let v = serde_json::to_value(&scene).unwrap();
    let back: SceneEngine = serde_json::from_value(v).unwrap();
    assert_eq!(back, scene);
}

#[test]
fn scene_engine_round_trips_with_an_ambience_override() {
    let mut scene = bare_scene();
    scene.ambience = Some(SceneAmbience {
        playlist: uuid::Uuid::new_v4(),
        gain: 0.6,
    });
    let v = serde_json::to_value(&scene).unwrap();
    let back: SceneEngine = serde_json::from_value(v).unwrap();
    assert_eq!(back, scene);
}

#[test]
fn scene_engine_rejects_nonfinite_ambience_gain() {
    let mut scene = bare_scene();
    scene.ambience = Some(SceneAmbience {
        playlist: uuid::Uuid::new_v4(),
        gain: f64::NAN,
    });
    assert!(scene.validate().is_err());
}

#[test]
fn scene_engine_round_trips_a_padding() {
    let mut scene = bare_scene();
    scene.padding = Some(3.0);
    let v = serde_json::to_value(&scene).unwrap();
    assert_eq!(v["padding"], serde_json::json!(3.0));
    let back: SceneEngine = serde_json::from_value(v).unwrap();
    assert_eq!(back, scene);
}

#[test]
fn scene_engine_admits_padding_from_zero_to_the_cap_inclusive() {
    // Discrimination: fails if either end of the range is exclusive.
    for ok in [0.0, 1.5, MAX_SCENE_PADDING_CELLS] {
        let mut scene = bare_scene();
        scene.padding = Some(ok);
        assert!(scene.validate().is_ok(), "{ok} must be admitted");
    }
}

#[test]
fn scene_engine_rejects_padding_outside_the_range_instead_of_clamping() {
    // Discrimination: fails if an over-cap, negative or non-finite margin is accepted or clamped.
    for bad in [-0.5, MAX_SCENE_PADDING_CELLS + 0.5, f64::NAN, f64::INFINITY] {
        let mut scene = bare_scene();
        scene.padding = Some(bad);
        assert!(scene.validate().is_err(), "{bad} must be rejected");
    }
}

#[test]
fn world_settings_default_has_no_audio_overlay() {
    assert!(WorldSettingsEngine::default().audio.is_none());
}

#[test]
fn world_settings_rejects_nonfinite_through_wall_gain() {
    let settings = WorldSettingsEngine {
        audio: Some(AudioOverlay {
            through_wall_gain: Some(f64::INFINITY),
            ..AudioOverlay::default()
        }),
        ..WorldSettingsEngine::default()
    };
    assert!(settings.validate().is_err());
}

#[test]
fn audio_overlay_default_is_all_absent() {
    let overlay = AudioOverlay::default();
    assert!(overlay.spatial.is_none());
    assert!(overlay.occlusion.is_none());
    assert!(overlay.through_wall_gain.is_none());
}
