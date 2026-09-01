use super::*;
use serde_json::json;
use std::collections::BTreeMap;

// --- (a) minimal valid bodies deserialize for every doc_type ---

#[test]
fn token_minimal_body_is_valid() {
    let v = json!({ "x": 1.0, "y": 2.0, "w": 100.0, "h": 100.0, "rotation": 0.0 });
    assert!(validate_engine("token", Some(&v)).is_ok());
}

#[test]
fn scene_minimal_body_is_valid() {
    let v = json!({ "grid": { "kind": "square", "size": 100.0 }, "background": null });
    assert!(validate_engine("scene", Some(&v)).is_ok());
}

#[test]
fn wall_minimal_body_is_valid() {
    let v = json!({ "seg": { "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 1.0 } });
    assert!(validate_engine("wall", Some(&v)).is_ok());
}

#[test]
fn region_minimal_body_is_valid() {
    let v = json!({
        "shape": { "kind": "rect", "points": [0.0, 0.0, 1.0, 1.0] },
        "behavior": "terrain", "cost": 1.0, "enabled": true
    });
    assert!(validate_engine("region", Some(&v)).is_ok());
}

#[test]
fn light_minimal_body_is_valid() {
    let v = json!({
        "x": 0.0, "y": 0.0, "emission": { "color": "#fff", "intensity": 1.0, "brightRadius": 5.0, "dimRadius": 10.0, "enabled": true }
    });
    assert!(validate_engine("light", Some(&v)).is_ok());
}

#[test]
fn drawing_minimal_body_is_valid() {
    let v = json!({
        "shape": { "kind": "rect", "points": [0.0, 0.0, 1.0, 1.0] },
        "stroke": null, "fill": null
    });
    assert!(validate_engine("drawing", Some(&v)).is_ok());
}

#[test]
fn template_minimal_body_is_valid() {
    let v = json!({
        "shape": { "kind": "cone", "x": 0.0, "y": 0.0, "size": 5.0, "direction": 0.0 },
        "color": "#f00"
    });
    assert!(validate_engine("template", Some(&v)).is_ok());
}

#[test]
fn actor_minimal_body_is_valid() {
    let v = json!({
        "displayName": "Goblin", "visual": { "kind": "image", "asset": "a" },
        "size": { "w": 1.0, "h": 1.0 }, "shape": "square",
        "faction": null, "conditions": [], "prototype": true
    });
    assert!(validate_engine("actor", Some(&v)).is_ok());
}

#[test]
fn message_minimal_body_is_valid() {
    let v = json!({
        "channel": "ic", "user_owner": "00000000-0000-0000-0000-000000000000",
        "kind": "normal", "content": []
    });
    assert!(validate_engine("message", Some(&v)).is_ok());
}

#[test]
fn world_settings_minimal_body_is_valid() {
    let v = serde_json::to_value(WorldSettingsEngine::default()).unwrap();
    assert!(validate_engine("world-settings", Some(&v)).is_ok());
}

#[test]
fn world_settings_partial_overlay_body_is_valid() {
    // The world layer is an optional overlay: a single authored leaf is a
    // complete, valid body — absent leaves fall through the resolution chain.
    let v = json!({ "scene": { "fog": false } });
    assert!(validate_engine("world-settings", Some(&v)).is_ok());
    // The old full-required shape remains valid (full ⊃ optional).
    let full = json!({
        "scene": {
            "losRestriction": true, "fog": true, "lightingEnabled": true,
            "lightMode": "environmentLight",
            "environment": { "color": "#0a0e1a", "intensity": 0.0 },
            "observerVision": false, "movementRestriction": "visible",
            "movementModel": "grid-stepped", "partialCellLeniency": true
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6.0, "easing": "easeInOut" }
    });
    assert!(validate_engine("world-settings", Some(&full)).is_ok());
}

#[test]
fn vision_modes_minimal_body_is_valid() {
    let v = json!({ "modes": {} });
    assert!(validate_engine("vision-modes", Some(&v)).is_ok());
}

#[test]
fn light_gradation_minimal_body_is_valid() {
    let v = json!({ "bands": [] });
    assert!(validate_engine("light-gradation", Some(&v)).is_ok());
}

#[test]
fn chat_settings_minimal_body_is_valid() {
    assert!(validate_engine("chat-settings", Some(&json!({}))).is_ok());
}

#[test]
fn dice_settings_minimal_body_is_valid() {
    assert!(validate_engine("dice-settings", Some(&json!({}))).is_ok());
}

#[test]
fn channel_registry_minimal_body_is_valid() {
    assert!(validate_engine("channel-registry", Some(&json!({ "channels": {} }))).is_ok());
}

#[test]
fn faction_registry_minimal_body_is_valid() {
    assert!(validate_engine("faction-registry", Some(&json!({ "factions": {} }))).is_ok());
}

#[test]
fn condition_registry_minimal_body_is_valid() {
    assert!(validate_engine("condition-registry", Some(&json!({ "conditions": {} }))).is_ok());
}

// --- (b) unknown fields rejected (struct-level; skip tagged-enum-only bodies) ---

#[test]
fn token_unknown_field_is_rejected() {
    let v = json!({ "x": 1.0, "y": 2.0, "w": 1.0, "h": 1.0, "rotation": 0.0, "bogus": 1 });
    assert!(validate_engine("token", Some(&v)).is_err());
}

#[test]
fn scene_unknown_field_is_rejected() {
    let v = json!({ "grid": { "kind": "square", "size": 100.0 }, "background": null, "bogus": 1 });
    assert!(validate_engine("scene", Some(&v)).is_err());
}

#[test]
fn wall_unknown_field_is_rejected() {
    let v = json!({ "seg": { "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 1.0 }, "bogus": 1 });
    assert!(validate_engine("wall", Some(&v)).is_err());
}

#[test]
fn wall_elevation_absent_defaults_to_none() {
    // No `elevation` key: the wall occludes every elevation (see `WallEngine::elevation`).
    let v = json!({ "seg": { "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 1.0 } });
    let w: WallEngine = serde_json::from_value(v).unwrap();
    assert_eq!(w.elevation, None);
}

#[test]
fn wall_elevation_partial_band_parses_with_open_end() {
    // An absent end is unbounded: `{"bottom": 2}` occludes elevation >= 2 only.
    let v = json!({
        "seg": { "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 1.0 },
        "elevation": { "bottom": 2.0 }
    });
    let w: WallEngine = serde_json::from_value(v).unwrap();
    assert_eq!(
        w.elevation,
        Some(WallElevation {
            bottom: Some(2.0),
            top: None
        })
    );
}

#[test]
fn wall_elevation_unknown_field_is_rejected() {
    let v = json!({
        "seg": { "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 1.0 },
        "elevation": { "bottom": 2.0, "bogus": 1 }
    });
    assert!(serde_json::from_value::<WallEngine>(v).is_err());
}

#[test]
fn token_and_light_elevation_absent_default_to_none() {
    // No `elevation` key reads as grounded (None = 0) on both carriers.
    let t: TokenEngine = serde_json::from_value(json!({
        "x": 1.0, "y": 2.0, "w": 100.0, "h": 100.0, "rotation": 0.0
    }))
    .unwrap();
    assert_eq!(t.elevation, None);
    let l: LightEngine = serde_json::from_value(json!({
        "x": 0.0, "y": 0.0,
        "emission": { "color": "#fff", "intensity": 1.0, "brightRadius": 5.0, "dimRadius": 10.0, "enabled": true }
    }))
    .unwrap();
    assert_eq!(l.elevation, None);
}

#[test]
fn region_unknown_field_is_rejected() {
    let v = json!({
        "shape": { "kind": "rect", "points": [] },
        "behavior": "terrain", "cost": 1.0, "enabled": true, "bogus": 1
    });
    assert!(validate_engine("region", Some(&v)).is_err());
}

#[test]
fn light_unknown_field_is_rejected() {
    let v = json!({
        "x": 0.0, "y": 0.0, "emission": { "color": "#fff", "intensity": 1.0, "brightRadius": 5.0, "dimRadius": 10.0, "enabled": true }, "bogus": 1
    });
    assert!(validate_engine("light", Some(&v)).is_err());
}

#[test]
fn drawing_unknown_field_is_rejected() {
    let v = json!({
        "shape": { "kind": "rect", "points": [] }, "stroke": null, "fill": null, "bogus": 1
    });
    assert!(validate_engine("drawing", Some(&v)).is_err());
}

#[test]
fn template_unknown_field_is_rejected() {
    let v = json!({
        "shape": { "kind": "cone", "x": 0.0, "y": 0.0, "size": 5.0, "direction": 0.0 },
        "color": "#f00", "bogus": 1
    });
    assert!(validate_engine("template", Some(&v)).is_err());
}

#[test]
fn actor_unknown_field_is_rejected() {
    let v = json!({
        "displayName": "Goblin", "visual": { "kind": "image", "asset": "a" },
        "size": { "w": 1.0, "h": 1.0 }, "shape": "square",
        "faction": null, "conditions": [], "prototype": true, "bogus": 1
    });
    assert!(validate_engine("actor", Some(&v)).is_err());
}

#[test]
fn actor_missing_display_name_is_rejected() {
    let v = json!({
        "visual": { "kind": "image", "asset": "a" },
        "size": { "w": 1.0, "h": 1.0 }, "shape": "square",
        "faction": null, "conditions": [], "prototype": true
    });
    assert!(validate_engine("actor", Some(&v)).is_err());
}

#[test]
fn actor_missing_conditions_is_rejected() {
    let v = json!({
        "displayName": "Goblin", "visual": { "kind": "image", "asset": "a" },
        "size": { "w": 1.0, "h": 1.0 }, "shape": "square",
        "faction": null, "prototype": true
    });
    assert!(validate_engine("actor", Some(&v)).is_err());
}

#[test]
fn actor_missing_faction_key_accepted_as_none() {
    // INVARIANT: `Option<T>` accepts an absent key as `None` regardless
    // of `#[serde(default)]` — pins the true ingress contract on
    // `ActorEngine.faction` (see its doc comment).
    let v = json!({
        "displayName": "Goblin", "visual": { "kind": "image", "asset": "a" },
        "size": { "w": 1.0, "h": 1.0 }, "shape": "square",
        "conditions": [], "prototype": true
    });
    assert!(validate_engine("actor", Some(&v)).is_ok());
    let engine: ActorEngine = serde_json::from_value(v).unwrap();
    assert_eq!(engine.faction, None);
}

#[test]
fn world_settings_unknown_field_is_rejected() {
    let mut v = serde_json::to_value(WorldSettingsEngine::default()).unwrap();
    v.as_object_mut().unwrap().insert("bogus".into(), json!(1));
    assert!(validate_engine("world-settings", Some(&v)).is_err());
}

#[test]
fn vision_modes_unknown_field_is_rejected() {
    assert!(validate_engine("vision-modes", Some(&json!({ "modes": {}, "bogus": 1 }))).is_err());
}

#[test]
fn light_gradation_unknown_field_is_rejected() {
    assert!(validate_engine("light-gradation", Some(&json!({ "bands": [], "bogus": 1 }))).is_err());
}

#[test]
fn chat_settings_unknown_field_is_rejected() {
    assert!(validate_engine("chat-settings", Some(&json!({ "bogus": 1 }))).is_err());
}

#[test]
fn dice_settings_unknown_field_is_rejected() {
    assert!(validate_engine("dice-settings", Some(&json!({ "bogus": 1 }))).is_err());
}

#[test]
fn channel_registry_unknown_field_is_rejected() {
    assert!(validate_engine(
        "channel-registry",
        Some(&json!({ "channels": {}, "bogus": 1 }))
    )
    .is_err());
}

#[test]
fn faction_registry_unknown_field_is_rejected() {
    assert!(validate_engine(
        "faction-registry",
        Some(&json!({ "factions": {}, "bogus": 1 }))
    )
    .is_err());
}

#[test]
fn condition_registry_unknown_field_is_rejected() {
    assert!(validate_engine(
        "condition-registry",
        Some(&json!({ "conditions": {}, "bogus": 1 }))
    )
    .is_err());
}

#[test]
fn message_unknown_field_is_rejected() {
    assert!(validate_engine(
        "message",
        Some(&json!({
            "channel": "ic", "user_owner": "00000000-0000-0000-0000-000000000000",
            "kind": "normal", "content": [], "bogus": 1
        }))
    )
    .is_err());
}

// --- (c) wrong-typed field rejected (all 23 registered doc_types) ---

#[test]
fn token_wrong_typed_field_is_rejected() {
    let v = json!({ "x": "12", "y": 2.0, "w": 1.0, "h": 1.0, "rotation": 0.0 });
    assert!(validate_engine("token", Some(&v)).is_err());
}

#[test]
fn scene_wrong_typed_grid_size_is_rejected() {
    let v = json!({ "grid": { "kind": "square", "size": "100" }, "background": null });
    assert!(validate_engine("scene", Some(&v)).is_err());
}

#[test]
fn wall_wrong_typed_field_is_rejected() {
    let v = json!({ "seg": { "x1": "0", "y1": 0.0, "x2": 1.0, "y2": 1.0 } });
    assert!(validate_engine("wall", Some(&v)).is_err());
}

#[test]
fn region_wrong_typed_field_is_rejected() {
    let v = json!({
        "shape": { "kind": "rect", "points": [0.0, 0.0, 1.0, 1.0] },
        "behavior": "terrain", "cost": "1.0", "enabled": true
    });
    assert!(validate_engine("region", Some(&v)).is_err());
}

#[test]
fn light_wrong_typed_intensity_is_rejected() {
    let v = json!({
        "x": 0.0, "y": 0.0, "emission": { "color": "#fff", "intensity": "1", "brightRadius": 5.0, "dimRadius": 10.0, "enabled": true }
    });
    assert!(validate_engine("light", Some(&v)).is_err());
}

#[test]
fn light_falloff_curve_is_a_closed_enum() {
    for curve in ["linear", "quadratic", "none"] {
        let v = json!({
            "x": 0.0, "y": 0.0,
            "emission": { "color": "#fff", "intensity": 1.0, "brightRadius": 5.0, "dimRadius": 10.0,
                "falloff": { "curve": curve }, "enabled": true }
        });
        assert!(validate_engine("light", Some(&v)).is_ok(), "curve {curve}");
    }
    let v = json!({
        "x": 0.0, "y": 0.0,
        "emission": { "color": "#fff", "intensity": 1.0, "brightRadius": 5.0, "dimRadius": 10.0,
            "falloff": { "curve": "cubic" }, "enabled": true }
    });
    assert!(validate_engine("light", Some(&v)).is_err());
}

#[test]
fn actor_and_token_override_carried_light_bodies_are_valid() {
    let emission = json!({
        "color": "#ffeeaa", "intensity": 0.8, "brightRadius": 2.0, "dimRadius": 6.0,
        "enabled": true
    });
    let actor = json!({
        "displayName": "Torchbearer", "visual": { "kind": "image", "asset": "a" },
        "size": { "w": 1.0, "h": 1.0 }, "shape": "square",
        "faction": null, "conditions": [], "prototype": true,
        "light": emission
    });
    assert!(validate_engine("actor", Some(&actor)).is_ok());
    let token = json!({
        "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0,
        "overrides": { "light": { "color": "#ffeeaa", "intensity": 0.0, "brightRadius": 0.0,
            "dimRadius": 0.0, "enabled": false } }
    });
    assert!(validate_engine("token", Some(&token)).is_ok());
}

#[test]
fn drawing_wrong_typed_field_is_rejected() {
    let v = json!({
        "shape": { "kind": "rect", "points": [0.0, 0.0, 1.0, 1.0] },
        "stroke": null, "fill": { "color": 1, "alpha": null }
    });
    assert!(validate_engine("drawing", Some(&v)).is_err());
}

#[test]
fn template_wrong_typed_field_is_rejected() {
    let v = json!({
        "shape": { "kind": "cone", "x": 0.0, "y": 0.0, "size": 5.0, "direction": 0.0 },
        "color": 1
    });
    assert!(validate_engine("template", Some(&v)).is_err());
}

#[test]
fn actor_wrong_typed_field_is_rejected() {
    let v = json!({
        "displayName": "Goblin", "visual": { "kind": "image", "asset": "a" },
        "size": { "w": 1.0, "h": 1.0 }, "shape": "square",
        "faction": null, "conditions": [], "prototype": "true"
    });
    assert!(validate_engine("actor", Some(&v)).is_err());
}

#[test]
fn message_wrong_typed_field_is_rejected() {
    let v = json!({
        "channel": 1, "user_owner": "00000000-0000-0000-0000-000000000000",
        "kind": "normal", "content": []
    });
    assert!(validate_engine("message", Some(&v)).is_err());
}

#[test]
fn world_settings_wrong_typed_field_is_rejected() {
    let mut v = serde_json::to_value(WorldSettingsEngine::default()).unwrap();
    v["animation"]["speedCellsPerSec"] = json!("6");
    assert!(validate_engine("world-settings", Some(&v)).is_err());
}

#[test]
fn vision_modes_wrong_typed_field_is_rejected() {
    let v = json!({
        "modes": {
            "darkvision": {
                "id": "darkvision", "name": "Darkvision",
                "illuminationFloor": "dark", "defaultRange": "60"
            }
        }
    });
    assert!(validate_engine("vision-modes", Some(&v)).is_err());
}

#[test]
fn light_gradation_wrong_typed_field_is_rejected() {
    let v = json!({ "bands": [{ "name": "dim", "minIllumination": "0.5" }] });
    assert!(validate_engine("light-gradation", Some(&v)).is_err());
}

#[test]
fn chat_settings_wrong_typed_field_is_rejected() {
    let v = json!({ "markdown": "true" });
    assert!(validate_engine("chat-settings", Some(&v)).is_err());
}

#[test]
fn dice_settings_wrong_typed_field_is_rejected() {
    let v = json!({ "mode": 1 });
    assert!(validate_engine("dice-settings", Some(&v)).is_err());
}

#[test]
fn channel_registry_wrong_typed_field_is_rejected() {
    let v = json!({ "channels": { "ic": { "name": 1 } } });
    assert!(validate_engine("channel-registry", Some(&v)).is_err());
}

#[test]
fn faction_registry_wrong_typed_field_is_rejected() {
    let v = json!({
        "factions": {
            "goblins": { "name": "Goblins", "color": "#fff", "stance": 1 }
        }
    });
    assert!(validate_engine("faction-registry", Some(&v)).is_err());
}

#[test]
fn condition_registry_wrong_typed_field_is_rejected() {
    let v = json!({ "conditions": { "prone": { "name": "Prone", "icon": 1 } } });
    assert!(validate_engine("condition-registry", Some(&v)).is_err());
}

// --- (d)/(e)/(f): registry membership + None handling ---

#[test]
fn non_engine_doc_type_with_engine_body_is_rejected() {
    assert!(validate_engine("item", Some(&json!({}))).is_err());
    assert!(validate_engine("custom-thing", Some(&json!({}))).is_err());
}

#[test]
fn non_engine_doc_type_without_engine_is_ok() {
    assert!(validate_engine("item", None).is_ok());
    assert!(validate_engine("custom-thing", None).is_ok());
}

#[test]
fn engine_doc_type_missing_engine_is_rejected() {
    assert!(validate_engine("token", None).is_err());
}

// --- token/actor visual union round-trips ---

#[test]
fn token_visual_image_round_trips() {
    let v: TokenVisual =
        serde_json::from_value(json!({ "kind": "image", "asset": "a.png" })).unwrap();
    assert!(matches!(v, TokenVisual::Image { .. }));
    assert_eq!(serde_json::to_value(&v).unwrap()["kind"], "image");
}

#[test]
fn token_visual_animated_round_trips() {
    let src = json!({ "kind": "animated", "source": { "type": "frames", "frames": ["a"] }, "fps": 12.0, "loop": true });
    let v: TokenVisual = serde_json::from_value(src.clone()).unwrap();
    assert!(matches!(v, TokenVisual::Animated { .. }));
    assert_eq!(serde_json::to_value(&v).unwrap(), src);
}

#[test]
fn token_visual_faces_round_trips() {
    let mut faces = BTreeMap::new();
    faces.insert(
        "default".to_string(),
        RenderVisual::Image {
            asset: "a.png".into(),
        },
    );
    let v = TokenVisual::Faces {
        faces,
        default: "default".into(),
        face_map: None,
    };
    let json = serde_json::to_value(&v).unwrap();
    let back: TokenVisual = serde_json::from_value(json).unwrap();
    assert_eq!(v, back);
}

#[test]
fn animated_source_frames_round_trips() {
    let src: AnimatedSource =
        serde_json::from_value(json!({ "type": "frames", "frames": ["a", "b"] })).unwrap();
    assert!(matches!(src, AnimatedSource::Frames { .. }));
}

#[test]
fn animated_source_sheet_round_trips() {
    let src: AnimatedSource =
        serde_json::from_value(json!({ "type": "sheet", "asset": "s.png", "rows": 2, "cols": 3 }))
            .unwrap();
    assert!(matches!(src, AnimatedSource::Sheet { .. }));
}

// --- literal-set assertions (client writers emit these strings today) ---

#[test]
fn shape_literal_set_deserializes() {
    for shape in ["square", "circle"] {
        let v = json!({
            "displayName": "Goblin", "visual": { "kind": "image", "asset": "a" },
            "size": { "w": 1.0, "h": 1.0 }, "shape": shape,
            "faction": null, "conditions": [], "prototype": true
        });
        assert!(
            validate_engine("actor", Some(&v)).is_ok(),
            "shape '{shape}' must be accepted"
        );
    }
}

#[test]
fn region_shape_kind_literal_set_deserializes() {
    for (kind, points) in [
        ("rect", json!([0.0, 0.0, 1.0, 1.0])),
        ("circle", json!([0.0, 0.0, 1.0])),
        ("polygon", json!([0.0, 0.0, 1.0, 0.0, 1.0, 1.0])),
    ] {
        let v = json!({
            "shape": { "kind": kind, "points": points },
            "behavior": "terrain", "cost": 1.0, "enabled": true
        });
        assert!(
            validate_engine("region", Some(&v)).is_ok(),
            "region shape kind '{kind}' must be accepted"
        );
    }
}

#[test]
fn region_behavior_literal_set_deserializes() {
    for behavior in ["terrain", "impassable", "arrest"] {
        let v = json!({
            "shape": { "kind": "rect", "points": [0.0, 0.0, 1.0, 1.0] },
            "behavior": behavior, "cost": 1.0, "enabled": true
        });
        assert!(
            validate_engine("region", Some(&v)).is_ok(),
            "region behavior '{behavior}' must be accepted"
        );
    }
}

/// Drift guard: the engine-literal fallbacks (`WorldSceneDefaults::default`,
/// `Pathfinding::default`, `AnimationSettings::default`) must serialize to
/// the SAME values as the client's `DEFAULT_WORLD_SETTINGS`, field-by-field.
/// `WorldSettingsEngine::default()` itself is the EMPTY overlay (what the
/// world-config seed authors) — every member serializes as null.
#[test]
fn engine_literal_defaults_match_client_default() {
    assert_eq!(
        serde_json::to_value(WorldSceneDefaults::default()).unwrap(),
        json!({
            "losRestriction": true,
            "fog": true,
            "lightingEnabled": true,
            "lightMode": "environmentLight",
            "environment": { "color": "#0a0e1a", "intensity": 0.0 },
            "observerVision": false,
            "movementRestriction": "visible",
            "movementModel": "grid-stepped",
            "partialCellLeniency": true,
        })
    );
    assert_eq!(
        serde_json::to_value(Pathfinding::default()).unwrap(),
        json!({ "diagonalRule": "chebyshev" })
    );
    assert_eq!(
        serde_json::to_value(AnimationSettings::default()).unwrap(),
        json!({ "speedCellsPerSec": 6.0, "easing": "easeInOut" })
    );
    assert_eq!(
        serde_json::to_value(WorldSettingsEngine::default()).unwrap(),
        json!({
            "scene": null,
            "pathfinding": null,
            "animation": null,
            "activeScene": null,
            "combat": null,
        })
    );
}

#[test]
fn scene_and_world_settings_accept_combat_overrides() {
    let scene = json!({
        "grid": { "kind": "square", "size": 100.0 }, "background": null,
        "combat": { "movementResource": "ship", "enforcement": "hard" }
    });
    let n = normalize_engine_opt("scene", Some(&scene))
        .unwrap()
        .unwrap();
    assert_eq!(n["combat"]["movementResource"], json!("ship"));
    assert_eq!(n["combat"]["enforcement"], json!("hard"));
    assert!(n["combat"]
        .get("interpretation")
        .is_some_and(|v| v.is_null()));

    let mut ws = serde_json::to_value(WorldSettingsEngine::default()).unwrap();
    ws["combat"] = json!({ "turnControl": "gm_only" });
    let n = normalize_engine_opt("world-settings", Some(&ws))
        .unwrap()
        .unwrap();
    assert_eq!(n["combat"]["turnControl"], json!("gm_only"));
}

#[test]
fn scene_combat_override_can_clear_the_movement_resource() {
    let scene = json!({
        "grid": { "kind": "square", "size": 100.0 }, "background": null,
        "combat": { "movementResource": null }
    });
    let n = normalize_engine_opt("scene", Some(&scene))
        .unwrap()
        .unwrap();
    // An explicit null survives re-serialization (it means CLEAR, not unset).
    assert!(n["combat"]
        .as_object()
        .unwrap()
        .contains_key("movementResource"));
    assert!(n["combat"]["movementResource"].is_null());
    let typed: SceneEngine = serde_json::from_value(n).unwrap();
    assert_eq!(typed.combat.unwrap().movement_resource, Some(None));
}

#[test]
fn scene_without_combat_key_reads_as_unset() {
    let scene = json!({ "grid": { "kind": "square", "size": 100.0 }, "background": null });
    let typed: SceneEngine = serde_json::from_value(scene).unwrap();
    assert_eq!(typed.combat, None);
}

#[test]
fn engine_of_defaults_on_absent_or_malformed() {
    let doc = crate::data::document::tests::world_scoped_doc(
        uuid::Uuid::from_u128(1),
        uuid::Uuid::from_u128(2),
        "world-settings",
    );
    let ws: WorldSettingsEngine = engine_of(&doc);
    assert_eq!(ws, WorldSettingsEngine::default());
}

// --- combat / combatant / resource-registry / effect ---

#[test]
fn combat_minimal_body_is_valid() {
    let v = json!({
        "scene_id": "00000000-0000-0000-0000-000000000001",
        "active": false, "round": 0, "turn": null,
        "turn_control": "owner_may_end", "order": [],
        "movement": { "resource": null, "interpretation": "per_cell", "enforcement": "none" },
        "effect_cleanup": true, "rewind_restore": true, "forward_restore": false,
        "effect_lifecycle": { "onCombatEnd": null, "onTurnEnd": null, "onAdvance": null }
    });
    assert!(validate_engine("combat", Some(&v)).is_ok());
}

#[test]
fn combatant_actor_minimal_body_is_valid() {
    let v = json!({
        "kind": { "type": "actor", "token_id": "00000000-0000-0000-0000-000000000002", "actor_id": null },
        "initiative": null, "tiebreak": 0.0, "resources": {}
    });
    assert!(validate_engine("combatant", Some(&v)).is_ok());
}

#[test]
fn combatant_event_minimal_body_is_valid() {
    let v = json!({
        "kind": { "type": "event", "lifespan": null, "message": "The lair action triggers" },
        "initiative": 20.0, "tiebreak": 0.0, "resources": {}
    });
    assert!(validate_engine("combatant", Some(&v)).is_ok());
}

#[test]
fn resource_registry_minimal_body_is_valid() {
    assert!(validate_engine("resource-registry", Some(&json!({ "resources": {} }))).is_ok());
}

#[test]
fn resource_registry_tracked_and_mirror_bindings_are_valid() {
    let v = json!({ "resources": {
        "movement": { "name": "Movement", "order": 0, "binding": { "kind": "tracked",
            "max": "speed", "recover": { "turn_start": "speed", "turn_end": 0, "round_start": 0, "round_end": 0 } } },
        "hp": { "name": "HP", "order": 1, "binding": { "kind": "mirror", "value": "hp" } }
    }});
    assert!(validate_engine("resource-registry", Some(&v)).is_ok());
}

#[test]
fn effect_minimal_body_is_valid() {
    assert!(validate_engine(
        "effect",
        Some(&json!({ "active": true, "transfer": false, "duration": null }))
    )
    .is_ok());
}

#[test]
fn effect_with_formula_duration_and_lifecycle_is_valid() {
    let v = json!({ "active": true, "transfer": false,
        "duration": { "amount": "rounds + 1", "remaining": 3, "unit": "rounds", "anchor": null, "expires": "turn_end" },
        "lifecycle": { "on_combat_end": 1, "on_turn_end": null, "on_advance": "1 - persistent" } });
    assert!(validate_engine("effect", Some(&v)).is_ok());
}

#[test]
fn effect_duration_amount_formula_is_bounded_and_remaining_optional() {
    let long = "x".repeat(crate::formula::MAX_FORMULA_LENGTH + 1);
    let v = json!({ "active": true, "duration": { "amount": long, "unit": "rounds", "anchor": null, "expires": "turn_end" } });
    assert!(validate_engine("effect", Some(&v)).is_err());
    let v = json!({ "active": true, "duration": { "amount": 2, "unit": "turns", "anchor": null, "expires": "turn_start" } });
    let n = normalize_engine_opt("effect", Some(&v)).unwrap().unwrap();
    assert_eq!(n["duration"]["remaining"], json!(null));
    assert_eq!(n["lifecycle"], json!(null));
}

#[test]
fn combat_snapshot_carries_cleanup_and_restore_flags() {
    let v = json!({
        "scene_id": "00000000-0000-0000-0000-000000000001",
        "active": false, "round": 0, "turn": null, "turn_control": "owner_may_end", "order": [],
        "movement": { "resource": null, "interpretation": "per_cell", "enforcement": "none" },
        "effect_cleanup": true, "rewind_restore": true, "forward_restore": false,
        "effect_lifecycle": { "onCombatEnd": null, "onTurnEnd": null, "onAdvance": null }
    });
    assert!(validate_engine("combat", Some(&v)).is_ok());
    let missing = json!({ "scene_id": "00000000-0000-0000-0000-000000000001", "active": false, "round": 0, "turn": null,
        "turn_control": "owner_may_end", "order": [], "movement": { "resource": null, "interpretation": "per_cell", "enforcement": "none" } });
    assert!(
        validate_engine("combat", Some(&missing)).is_err(),
        "snapshot flags are required"
    );
}

#[test]
fn combat_history_minimal_body_is_valid_and_cursor_is_bounded() {
    assert!(validate_engine(
        "combat-history",
        Some(&json!({ "records": [], "cursor": 0 }))
    )
    .is_ok());
    assert!(validate_engine(
        "combat-history",
        Some(&json!({ "records": [], "cursor": 1 }))
    )
    .is_err());
}

#[test]
fn combat_defaults_accept_cleanup_and_restore_overrides() {
    let v = json!({ "effectCleanup": false, "rewindRestore": false, "forwardRestore": true,
        "effectLifecycle": { "onCombatEnd": 0, "onTurnEnd": "fleeting", "onAdvance": null } });
    let d: CombatDefaults = serde_json::from_value(v).unwrap();
    assert_eq!(d.effect_cleanup, Some(false));
    assert_eq!(d.forward_restore, Some(true));
}

/// A `combat` body whose snapshotted lifecycle carries `formula` in `on_advance`.
fn combat_with_on_advance(formula: serde_json::Value) -> serde_json::Value {
    json!({
        "scene_id": "00000000-0000-0000-0000-000000000001",
        "active": false, "round": 0, "turn": null, "turn_control": "owner_may_end", "order": [],
        "movement": { "resource": null, "interpretation": "per_cell", "enforcement": "none" },
        "effect_cleanup": true, "rewind_restore": true, "forward_restore": false,
        "effect_lifecycle": { "onCombatEnd": null, "onTurnEnd": null, "onAdvance": formula }
    })
}

#[test]
fn combat_snapshot_lifecycle_formula_must_parse() {
    assert!(validate_engine(
        "combat",
        Some(&combat_with_on_advance(json!("persistent - 1")))
    )
    .is_ok());
    let err = validate_engine("combat", Some(&combat_with_on_advance(json!("1 +")))).unwrap_err();
    assert!(
        err.to_string().contains("effect_lifecycle.on_advance"),
        "{err}"
    );
}

#[test]
fn combat_defaults_lifecycle_formula_must_parse_under_every_container() {
    let bad =
        json!({ "effectLifecycle": { "onCombatEnd": null, "onTurnEnd": "(", "onAdvance": null } });
    let good = json!({ "effectLifecycle": { "onCombatEnd": null, "onTurnEnd": "fleeting", "onAdvance": null } });

    let sd = |c: &serde_json::Value| json!({ "combat": c });
    assert!(validate_engine("system-defaults", Some(&sd(&good))).is_ok());
    assert!(validate_engine("system-defaults", Some(&sd(&bad))).is_err());

    let ws = |c: &serde_json::Value| {
        let mut v = serde_json::to_value(WorldSettingsEngine::default()).unwrap();
        v["combat"] = c.clone();
        v
    };
    assert!(validate_engine("world-settings", Some(&ws(&good))).is_ok());
    assert!(validate_engine("world-settings", Some(&ws(&bad))).is_err());

    let sc = |c: &serde_json::Value| json!({ "grid": { "kind": "square", "size": 100.0 }, "background": null, "combat": c });
    assert!(validate_engine("scene", Some(&sc(&good))).is_ok());
    assert!(validate_engine("scene", Some(&sc(&bad))).is_err());
}

#[test]
fn combat_unknown_field_is_rejected() {
    let v = json!({
        "scene_id": "00000000-0000-0000-0000-000000000001",
        "active": false, "round": 0, "turn": null, "turn_control": "owner_may_end", "order": [],
        "movement": { "resource": null, "interpretation": "per_cell", "enforcement": "none" },
        "effect_cleanup": true, "rewind_restore": true, "forward_restore": false,
        "effect_lifecycle": { "onCombatEnd": null, "onTurnEnd": null, "onAdvance": null },
        "bogus": 1
    });
    assert!(validate_engine("combat", Some(&v)).is_err());
}

#[test]
fn combatant_unknown_field_is_rejected() {
    let v = json!({
        "kind": { "type": "event", "lifespan": null, "message": null },
        "initiative": null, "tiebreak": 0.0, "resources": {}, "bogus": 1
    });
    assert!(validate_engine("combatant", Some(&v)).is_err());
}

#[test]
fn resource_registry_unknown_field_is_rejected() {
    assert!(validate_engine(
        "resource-registry",
        Some(&json!({ "resources": {}, "bogus": 1 }))
    )
    .is_err());
}

#[test]
fn effect_unknown_field_is_rejected() {
    assert!(validate_engine(
        "effect",
        Some(&json!({ "active": true, "transfer": false, "bogus": 1 }))
    )
    .is_err());
}

#[test]
fn combat_wrong_typed_round_is_rejected() {
    let v = json!({
        "scene_id": "00000000-0000-0000-0000-000000000001",
        "active": false, "round": "1", "turn": null, "turn_control": "owner_may_end", "order": [],
        "movement": { "resource": null, "interpretation": "per_cell", "enforcement": "none" },
        "effect_cleanup": true, "rewind_restore": true, "forward_restore": false,
        "effect_lifecycle": { "onCombatEnd": null, "onTurnEnd": null, "onAdvance": null }
    });
    assert!(validate_engine("combat", Some(&v)).is_err());
}

#[test]
fn combatant_wrong_typed_tiebreak_is_rejected() {
    let v = json!({
        "kind": { "type": "event", "lifespan": null, "message": null },
        "initiative": null, "tiebreak": "0", "resources": {}
    });
    assert!(validate_engine("combatant", Some(&v)).is_err());
}

#[test]
fn resource_registry_wrong_typed_order_is_rejected() {
    let v = json!({ "resources": { "x": { "name": "X", "order": "0", "binding": { "kind": "mirror", "value": 1 } } } });
    assert!(validate_engine("resource-registry", Some(&v)).is_err());
}

#[test]
fn effect_wrong_typed_active_is_rejected() {
    assert!(validate_engine(
        "effect",
        Some(&json!({ "active": "yes", "transfer": false }))
    )
    .is_err());
}

#[test]
fn effect_absent_transfer_defaults_false_and_reserializes_explicitly() {
    let n = normalize_engine_opt("effect", Some(&json!({ "active": true })))
        .unwrap()
        .unwrap();
    assert_eq!(n["transfer"], json!(false));
    assert!(n.get("duration").is_some_and(|d| d.is_null()));
}

// --- system-defaults ---

#[test]
fn system_defaults_empty_body_is_valid() {
    assert!(validate_engine("system-defaults", Some(&json!({}))).is_ok());
}

#[test]
fn system_defaults_partial_overlay_is_valid_and_reserializes_absent_as_null() {
    let v = json!({
        "scene": { "fog": false },
        "pathfinding": { "diagonalRule": "euclidean" },
        "combat": { "enforcement": "hard" }
    });
    let n = normalize_engine_opt("system-defaults", Some(&v))
        .unwrap()
        .unwrap();
    assert_eq!(n["scene"]["fog"], json!(false));
    assert_eq!(n["scene"]["losRestriction"], json!(null));
    assert_eq!(n["animation"], json!(null));
    assert_eq!(n["combat"]["enforcement"], json!("hard"));
}

#[test]
fn system_defaults_unknown_field_is_rejected() {
    assert!(validate_engine("system-defaults", Some(&json!({ "activeScene": null }))).is_err());
    assert!(validate_engine("system-defaults", Some(&json!({ "scene": { "bogus": 1 } }))).is_err());
}

#[test]
fn system_defaults_wrong_type_is_rejected() {
    assert!(validate_engine(
        "system-defaults",
        Some(&json!({ "scene": { "fog": "yes" } }))
    )
    .is_err());
    assert!(validate_engine(
        "system-defaults",
        Some(&json!({ "animation": { "speedCellsPerSec": -1.0 } }))
    )
    .is_err());
}

// --- world-config seed bodies (the engine definition; client constants mirror these) ---

#[test]
fn faction_registry_seed_content() {
    let s = FactionRegistryEngine::seed();
    assert_eq!(s.factions.len(), 3);
    let f = &s.factions["friendly"];
    assert_eq!(
        (f.name.as_str(), f.color.as_str(), f.stance),
        ("Friendly", "#3fb950", FactionStance::Friendly)
    );
    let n = &s.factions["neutral"];
    assert_eq!(
        (n.name.as_str(), n.color.as_str(), n.stance),
        ("Neutral", "#9e9e9e", FactionStance::Neutral)
    );
    let h = &s.factions["hostile"];
    assert_eq!(
        (h.name.as_str(), h.color.as_str(), h.stance),
        ("Hostile", "#f85149", FactionStance::Hostile)
    );
    let v = serde_json::to_value(&s).unwrap();
    assert!(validate_engine("faction-registry", Some(&v)).is_ok());
}

#[test]
fn condition_registry_seed_content() {
    let s = ConditionRegistryEngine::seed();
    let expect: &[(&str, &str, &str)] = &[
        ("dead", "Dead", "💀"),
        ("unconscious", "Unconscious", "😵"),
        ("prone", "Prone", "🛌"),
        ("stunned", "Stunned", "💫"),
        ("poisoned", "Poisoned", "🤢"),
        ("blinded", "Blinded", "🙈"),
        ("invisible", "Invisible", "👻"),
        ("hasted", "Hasted", "⚡"),
        ("slowed", "Slowed", "🐌"),
    ];
    assert_eq!(s.conditions.len(), expect.len());
    for (id, name, icon) in expect {
        let c = &s.conditions[*id];
        assert_eq!((c.name.as_str(), c.icon.as_str()), (*name, *icon));
    }
    let v = serde_json::to_value(&s).unwrap();
    assert!(validate_engine("condition-registry", Some(&v)).is_ok());
}

#[test]
fn channel_registry_seed_content() {
    let s = ChannelRegistryEngine::seed();
    assert_eq!(s.channels.len(), 1);
    assert_eq!(s.channels["general"].name, "General");
    let v = serde_json::to_value(&s).unwrap();
    assert!(validate_engine("channel-registry", Some(&v)).is_ok());
}

#[test]
fn light_gradation_seed_content() {
    let s = LightGradationEngine::seed();
    let bands: Vec<(&str, f64)> = s
        .bands
        .iter()
        .map(|b| (b.name.as_str(), b.min_illumination))
        .collect();
    assert_eq!(bands, vec![("bright", 0.67), ("dim", 0.34), ("dark", 0.0)]);
    let v = serde_json::to_value(&s).unwrap();
    assert!(validate_engine("light-gradation", Some(&v)).is_ok());
}

#[test]
fn vision_modes_seed_content() {
    let s = VisionModesEngine::seed();
    assert_eq!(s.modes.len(), 3);
    let n = &s.modes["normal"];
    assert_eq!(
        (
            n.id.as_str(),
            n.name.as_str(),
            n.illumination_floor.as_str(),
            n.default_range,
            n.render_hint.as_deref(),
            n.perceives,
            n.requires_los,
        ),
        (
            "normal",
            "Normal",
            "dim",
            0.0,
            None,
            Perception::Terrain,
            true
        )
    );
    let d = &s.modes["darkvision"];
    assert_eq!(
        (
            d.id.as_str(),
            d.name.as_str(),
            d.illumination_floor.as_str(),
            d.default_range,
            d.render_hint.as_deref(),
            d.perceives,
            d.requires_los,
        ),
        (
            "darkvision",
            "Darkvision",
            "dark",
            12.0,
            Some("desaturate"),
            Perception::Terrain,
            true
        )
    );
    let t = &s.modes["tremorsense"];
    assert_eq!(
        (
            t.id.as_str(),
            t.name.as_str(),
            t.default_range,
            t.render_hint.as_deref(),
            t.perceives,
            t.requires_los,
        ),
        (
            "tremorsense",
            "Tremorsense",
            12.0,
            None,
            Perception::Creatures,
            false
        )
    );
    let v = serde_json::to_value(&s).unwrap();
    assert!(validate_engine("vision-modes", Some(&v)).is_ok());
}

#[test]
fn vision_mode_absent_sense_fields_default_to_terrain_los() {
    // A mode authored before `perceives`/`requiresLos` existed (no keys at all)
    // must deserialize unchanged: terrain perception, LOS-gated.
    let v = json!({
        "id": "normal", "name": "Normal",
        "illuminationFloor": "dim", "defaultRange": 0.0
    });
    let m: VisionMode = serde_json::from_value(v).unwrap();
    assert_eq!(m.perceives, Perception::Terrain);
    assert!(m.requires_los);
    // Serde wire shape: camelCase field names, lowercase perception values.
    let w = serde_json::to_value(&m).unwrap();
    assert_eq!(w["perceives"], json!("terrain"));
    assert_eq!(w["requiresLos"], json!(true));
}

#[test]
fn vision_mode_unknown_field_is_rejected() {
    let v = json!({
        "id": "normal", "name": "Normal",
        "illuminationFloor": "dim", "defaultRange": 0.0, "bogus": 1
    });
    assert!(serde_json::from_value::<VisionMode>(v).is_err());
}

#[test]
fn empty_default_config_bodies_are_valid() {
    let chat = serde_json::to_value(ChatSettingsEngine::default()).unwrap();
    assert!(validate_engine("chat-settings", Some(&chat)).is_ok());
    let dice_default = DiceSettingsEngine::default();
    assert_eq!(
        (
            dice_default.mode,
            dice_default.direction,
            dice_default.channel_overrides.len()
        ),
        (DiceModeSetting::Total, DiceDirectionSetting::HighWins, 0)
    );
    let dice = serde_json::to_value(&dice_default).unwrap();
    assert!(validate_engine("dice-settings", Some(&dice)).is_ok());
    let resources = serde_json::to_value(ResourceRegistryEngine::default()).unwrap();
    assert!(validate_engine("resource-registry", Some(&resources)).is_ok());
    let sd = serde_json::to_value(SystemDefaultsEngine::default()).unwrap();
    assert!(validate_engine("system-defaults", Some(&sd)).is_ok());
}

#[test]
fn asset_folder_is_engine_type_with_sort_only() {
    assert!(is_engine_doc_type("asset_folder"));
    assert!(validate_engine("asset_folder", Some(&json!({ "sort": 3 }))).is_ok());
    assert!(validate_engine("asset_folder", Some(&json!({ "sort": 3, "name": "x" }))).is_err());
    assert!(validate_engine("asset_folder", None).is_err());
}
