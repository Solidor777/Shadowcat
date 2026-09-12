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
        snap_to_grid: None,
        vision: None,
        lighting: None,
        combat: None,
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
