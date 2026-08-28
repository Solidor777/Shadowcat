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
        "x": 0.0, "y": 0.0, "color": "#fff", "intensity": 1.0,
        "brightRadius": 5.0, "dimRadius": 10.0, "enabled": true
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
        "x": 0.0, "y": 0.0, "color": "#fff", "intensity": 1.0,
        "brightRadius": 5.0, "dimRadius": 10.0, "enabled": true, "bogus": 1
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

// --- (c) wrong-typed field rejected (all 17 registered doc_types) ---

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
        "x": 0.0, "y": 0.0, "color": "#fff", "intensity": "1",
        "brightRadius": 5.0, "dimRadius": 10.0, "enabled": true
    });
    assert!(validate_engine("light", Some(&v)).is_err());
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

/// Drift guard: `WorldSettingsEngine::default()` must serialize to the
/// SAME values as the client's `DEFAULT_WORLD_SETTINGS`, field-by-field.
#[test]
fn world_settings_default_matches_client_default() {
    let v = serde_json::to_value(WorldSettingsEngine::default()).unwrap();
    assert_eq!(
        v,
        json!({
            "scene": {
                "losRestriction": true,
                "fog": true,
                "lightingEnabled": true,
                "lightMode": "environmentLight",
                "environment": { "color": "#0a0e1a", "intensity": 0.0 },
                "observerVision": false,
                "movementRestriction": "visible",
                "movementModel": "grid-stepped",
                "partialCellLeniency": true,
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6.0, "easing": "easeInOut" },
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
        "movement": { "resource": null, "interpretation": "per_cell", "enforcement": "none" }
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
fn effect_with_duration_is_valid() {
    let v = json!({ "active": true, "transfer": false, "duration": {
        "amount": 3, "unit": "rounds", "anchor": null, "expires": "turn_end",
        "started": { "round": 1, "turn_index": 0 } } });
    assert!(validate_engine("effect", Some(&v)).is_ok());
}

#[test]
fn combat_unknown_field_is_rejected() {
    let v = json!({
        "scene_id": "00000000-0000-0000-0000-000000000001",
        "active": false, "round": 0, "turn": null, "turn_control": "owner_may_end", "order": [],
        "movement": { "resource": null, "interpretation": "per_cell", "enforcement": "none" },
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
        "movement": { "resource": null, "interpretation": "per_cell", "enforcement": "none" }
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
