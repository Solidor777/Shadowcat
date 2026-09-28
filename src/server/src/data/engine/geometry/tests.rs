//! Sibling tests for the geometry engine bands — the door-state resolver battery.

use super::*;

/// A wall with every authored flag set, carrying `door`.
fn wall(door: Option<DoorState>) -> WallEngine {
    WallEngine {
        seg: Seg {
            x1: 0.0,
            y1: 0.0,
            x2: 10.0,
            y2: 0.0,
        },
        blocks_sight: Some(true),
        blocks_light: Some(true),
        blocks_move: Some(true),
        blocks_sound: None,
        elevation: None,
        door,
        direction: None,
        sight_tier: None,
    }
}

#[test]
fn door_open_resolves_all_flags_false_regardless_of_authored_flags() {
    // An open door is a HOLE: every effective flag is false even though every
    // authored flag is true.
    let flags = wall(Some(DoorState::Open)).effective_flags();
    assert_eq!(
        flags,
        EffectiveWallFlags {
            blocks_move: false,
            blocks_sight: false,
            blocks_light: false,
            blocks_sound: false,
        }
    );

    // The authored flags being ABSENT changes nothing either: open wins over
    // both authored extremes — including an explicit `blocks_sound: true`.
    let mut sparse = wall(Some(DoorState::Open));
    sparse.blocks_sight = None;
    sparse.blocks_light = None;
    sparse.blocks_move = None;
    sparse.blocks_sound = Some(true);
    assert_eq!(sparse.effective_flags(), flags);
}

#[test]
fn door_closed_and_locked_resolve_authored_flags_unchanged() {
    // Closed and Locked differ only in who may flip the door, never in what it
    // blocks: both apply the authored flags unchanged.
    for state in [DoorState::Closed, DoorState::Locked] {
        let flags = wall(Some(state)).effective_flags();
        assert_eq!(
            flags,
            EffectiveWallFlags {
                blocks_move: true,
                blocks_sight: true,
                blocks_light: true,
                // `blocks_sound` absent in the fixture: inherits the resolved
                // `blocks_sight`.
                blocks_sound: true,
            },
            "{state:?} must apply the authored flags unchanged"
        );

        // A closed door can still be see-through: the authored flags stay
        // independent per axis (a barred gate you can see through).
        let mut gate = wall(Some(state));
        gate.blocks_sight = Some(false);
        gate.blocks_move = Some(true);
        gate.blocks_light = None;
        let flags = gate.effective_flags();
        assert!(flags.blocks_move);
        assert!(!flags.blocks_sight);
        assert!(!flags.blocks_light);
        assert!(
            !flags.blocks_sound,
            "an unauthored blocks_sound inherits the resolved blocks_sight (false here)"
        );
    }
}

#[test]
fn door_absent_resolves_authored_flags_unchanged() {
    // The pre-existing-behavior pin: a wall carrying no door resolves exactly
    // its authored flags. Also pinned at the serde boundary: a stored document
    // written before the field existed carries no `door` key and must
    // deserialize to `None` (the migration-freedom contract).
    let body = serde_json::json!({
        "seg": { "x1": 0.0, "y1": 0.0, "x2": 10.0, "y2": 0.0 },
        "blocksSight": true,
        "blocksMove": true,
        "blocksLight": true,
    });
    let parsed: WallEngine =
        serde_json::from_value(body).expect("a doorless stored wall deserializes unchanged");
    assert_eq!(parsed.door, None);
    assert_eq!(
        parsed.effective_flags(),
        EffectiveWallFlags {
            blocks_move: true,
            blocks_sight: true,
            blocks_light: true,
            blocks_sound: true,
        }
    );

    let mut partial = wall(None);
    partial.blocks_move = Some(false);
    let flags = partial.effective_flags();
    assert!(!flags.blocks_move);
    assert!(flags.blocks_sight);
    assert!(flags.blocks_light);
    assert!(flags.blocks_sound);
}

#[test]
fn direction_and_sight_tier_deserialize_absent_as_none() {
    // The migration-freedom contract for both fields: a stored document written
    // before they existed carries neither key and must deserialize with both `None`.
    let body = serde_json::json!({
        "seg": { "x1": 0.0, "y1": 0.0, "x2": 10.0, "y2": 0.0 },
        "blocksSight": true,
    });
    let parsed: WallEngine =
        serde_json::from_value(body).expect("a pre-direction stored wall deserializes unchanged");
    assert_eq!(parsed.direction, None);
    assert_eq!(parsed.sight_tier, None);
    assert!(parsed.validate().is_ok());
}

#[test]
fn direction_and_sight_tier_round_trip_their_wire_names() {
    // The wire spellings are camelCase (`blockingSide`, `sightTier`) and the tier is the
    // snake_case variant name — pin both, since the client authors these payloads by key.
    let mut w = wall(None);
    w.direction = Some(WallDirection {
        blocking_side: -1.0,
    });
    w.sight_tier = Some(WallSightTier::Limited);
    let json = serde_json::to_value(&w).unwrap();
    assert_eq!(
        json["direction"],
        serde_json::json!({ "blockingSide": -1.0 })
    );
    assert_eq!(json["sightTier"], serde_json::json!("limited"));
    let back: WallEngine = serde_json::from_value(json).unwrap();
    assert_eq!(
        back.direction,
        Some(WallDirection {
            blocking_side: -1.0
        })
    );
    assert_eq!(back.sight_tier, Some(WallSightTier::Limited));
    assert!(back.validate().is_ok());
}

#[test]
fn wall_validate_accepts_absent_direction_and_unit_signs_only() {
    assert!(
        wall(None).validate().is_ok(),
        "no direction: nothing to validate"
    );
    for sign in [1.0, -1.0] {
        let mut w = wall(None);
        w.direction = Some(WallDirection {
            blocking_side: sign,
        });
        assert!(w.validate().is_ok(), "blocking_side {sign} must pass");
    }
    for bad in [0.0, 2.0, -0.5, f64::NAN, f64::INFINITY] {
        let mut w = wall(None);
        w.direction = Some(WallDirection { blocking_side: bad });
        assert!(
            w.validate().is_err(),
            "blocking_side {bad} must be rejected: the side test reads it as a sign"
        );
    }
}

#[test]
fn wall_validate_rejects_a_non_unit_blocking_side_at_ingress() {
    // The `normalize_engine` wiring is the real ingress chokepoint: a `blockingSide`
    // outside {-1.0, 1.0} must fail the whole write, not just the typed validate.
    let body = serde_json::json!({
        "seg": { "x1": 0.0, "y1": 0.0, "x2": 10.0, "y2": 0.0 },
        "direction": { "blockingSide": 0.0 },
    });
    assert!(
        crate::data::engine::normalize_engine_opt("wall", Some(&body)).is_err(),
        "a zero blocking_side must be rejected at ingress, never normalized"
    );
    let good = serde_json::json!({
        "seg": { "x1": 0.0, "y1": 0.0, "x2": 10.0, "y2": 0.0 },
        "direction": { "blockingSide": 1.0 },
        "sightTier": "limited",
    });
    assert!(
        crate::data::engine::normalize_engine_opt("wall", Some(&good)).is_ok(),
        "a unit sign and a known tier pass ingress"
    );
}

#[test]
fn blocks_sound_inherits_the_resolved_blocks_sight_when_absent() {
    // The inheritance default: an unauthored `blocks_sound` resolves to the
    // door-resolved `blocks_sight`, so a wall that never authors the field
    // occludes sound exactly as it occludes sight.
    let mut w = wall(None);
    assert!(
        w.effective_flags().blocks_sound,
        "blocks_sight: Some(true) inherits true"
    );
    w.blocks_sight = None;
    assert!(
        !w.effective_flags().blocks_sound,
        "blocks_sight absent resolves false and inherits as false"
    );
    w.blocks_sight = Some(false);
    assert!(
        !w.effective_flags().blocks_sound,
        "blocks_sight: Some(false) inherits false"
    );
}

#[test]
fn blocks_sound_explicit_wins_over_inheritance_in_both_directions() {
    // Sound-transparent but sight-blocking (a heavy curtain): blocks_sound wins.
    let mut curtain = wall(None);
    curtain.blocks_sound = Some(false);
    let flags = curtain.effective_flags();
    assert!(flags.blocks_sight);
    assert!(!flags.blocks_sound);

    // Sight-transparent but sound-blocking (a window): blocks_sound wins.
    let mut window = wall(None);
    window.blocks_sight = Some(false);
    window.blocks_sound = Some(true);
    let flags = window.effective_flags();
    assert!(!flags.blocks_sight);
    assert!(flags.blocks_sound);
}

#[test]
fn blocks_sound_round_trips_its_wire_name_and_defaults_absent() {
    // The wire spelling is camelCase `blocksSound`; pin it, since the client
    // authors the payload by key.
    let mut w = wall(None);
    w.blocks_sound = Some(false);
    let json = serde_json::to_value(&w).unwrap();
    assert_eq!(json["blocksSound"], serde_json::json!(false));
    let back: WallEngine = serde_json::from_value(json).unwrap();
    assert_eq!(back.blocks_sound, Some(false));

    // A stored document carrying no `blocksSound` key deserializes with `None`
    // (the inherit default) and passes ingress unchanged.
    let body = serde_json::json!({
        "seg": { "x1": 0.0, "y1": 0.0, "x2": 10.0, "y2": 0.0 },
        "blocksSight": true,
    });
    let parsed: WallEngine = serde_json::from_value(body.clone()).unwrap();
    assert_eq!(parsed.blocks_sound, None);
    assert!(
        parsed.effective_flags().blocks_sound,
        "absent blocks_sound inherits the authored blocks_sight"
    );
    assert!(
        crate::data::engine::normalize_engine_opt("wall", Some(&body)).is_ok(),
        "a wall without a blocksSound key passes ingress"
    );
}

// --- Region ambient-light override validation ---

/// A minimal valid `ambient_light` region engine body, overridden per test.
fn ambient_region(mode: AmbientLightMode, level: f64) -> RegionEngine {
    RegionEngine {
        shape: RegionGeometry::Single(RegionShape {
            kind: "rect".to_string(),
            points: vec![0.0, 0.0, 5.0, 5.0],
        }),
        behavior: "ambient_light".to_string(),
        cost: 1.0,
        cost_by_tag: std::collections::BTreeMap::new(),
        enabled: true,
        triggers: Vec::new(),
        elevation: None,
        ambient_override: Some(AmbientLightOverride { mode, level }),
    }
}

#[test]
fn ambient_override_requires_the_ambient_light_behavior() {
    let mut region = ambient_region(AmbientLightMode::Darken, 0.5);
    region.behavior = "terrain".to_string();
    assert!(
        region.validate().is_err(),
        "an override on a non-ambient behavior is rejected"
    );
}

#[test]
fn ambient_override_level_must_be_finite() {
    let region = ambient_region(AmbientLightMode::Darken, f64::NAN);
    assert!(region.validate().is_err(), "a NaN level is rejected");
    let region = ambient_region(AmbientLightMode::Brighten, f64::INFINITY);
    assert!(region.validate().is_err(), "an infinite level is rejected");
}

#[test]
fn ambient_override_override_level_is_bounded_to_the_illumination_scale() {
    assert!(ambient_region(AmbientLightMode::Override, 0.0)
        .validate()
        .is_ok());
    assert!(ambient_region(AmbientLightMode::Override, 1.0)
        .validate()
        .is_ok());
    assert!(
        ambient_region(AmbientLightMode::Override, 1.5)
            .validate()
            .is_err(),
        "an override above 1 is rejected"
    );
    assert!(
        ambient_region(AmbientLightMode::Override, -0.1)
            .validate()
            .is_err(),
        "an override below 0 is rejected"
    );
}

#[test]
fn ambient_override_delta_levels_are_non_negative() {
    assert!(ambient_region(AmbientLightMode::Darken, 0.0)
        .validate()
        .is_ok());
    assert!(ambient_region(AmbientLightMode::Brighten, 2.5)
        .validate()
        .is_ok());
    assert!(
        ambient_region(AmbientLightMode::Darken, -0.1)
            .validate()
            .is_err(),
        "a negative darken is a confusing spelling of brighten: rejected, not normalized"
    );
    assert!(
        ambient_region(AmbientLightMode::Brighten, -0.1)
            .validate()
            .is_err(),
        "a negative brighten is rejected the same way"
    );
}

#[test]
fn ambient_light_behavior_without_an_override_is_a_valid_no_op() {
    let mut region = ambient_region(AmbientLightMode::Darken, 0.5);
    region.ambient_override = None;
    assert!(
        region.validate().is_ok(),
        "an ambient_light region with no override authored yet is a harmless no-op"
    );
    // And the wire name is the verbatim field name (`RegionEngine` declares no
    // `rename_all`; the client authors the key exactly as the field reads).
    let region = ambient_region(AmbientLightMode::Override, 0.0);
    let json = serde_json::to_value(&region).unwrap();
    assert_eq!(
        json["ambient_override"],
        serde_json::json!({ "mode": "override", "level": 0.0 })
    );
    // A stored document carrying no `ambient_override` key deserializes to `None`
    // (zero migration).
    let mut absent = serde_json::to_value(&region).unwrap();
    absent.as_object_mut().unwrap().remove("ambient_override");
    let parsed: RegionEngine = serde_json::from_value(absent).unwrap();
    assert_eq!(parsed.ambient_override, None);
}

/// A template with a valid color and no `angle`/`width` authored.
fn template(angle: Option<f64>, width: Option<f64>) -> TemplateEngine {
    TemplateEngine {
        shape: TemplateShape {
            kind: "cone".to_string(),
            x: 0.0,
            y: 0.0,
            size: 15.0,
            direction: 90.0,
            angle,
            width,
        },
        color: "#ffaa00".to_string(),
        elevation: None,
    }
}

#[test]
fn template_engine_validate_bounds_angle_and_width() {
    // Absent angle/width is the pre-existing-behavior pin: always valid.
    assert!(template(None, None).validate().is_ok());

    // A valid angle in (0.0, 180.0) and a valid non-negative width both pass.
    assert!(template(Some(60.0), Some(5.0)).validate().is_ok());
    assert!(template(Some(0.1), Some(0.0)).validate().is_ok());

    // The upper bound is pinned at its edge: just inside 180.0 is Ok, 180.0
    // itself (the collinear, zero-area wedge) and anything above are Err —
    // `conePoints`' three-point wedge cannot represent either.
    assert!(template(Some(179.9), None).validate().is_ok());
    assert!(template(Some(180.0), None).validate().is_err());
    assert!(template(Some(180.1), None).validate().is_err());
    assert!(template(Some(270.0), None).validate().is_err());

    // Angle must be finite and strictly greater than 0.0.
    assert!(template(Some(0.0), None).validate().is_err());
    assert!(template(Some(-10.0), None).validate().is_err());
    assert!(template(Some(f64::NAN), None).validate().is_err());
    assert!(template(Some(f64::INFINITY), None).validate().is_err());

    // Width must be finite and non-negative.
    assert!(template(None, Some(-0.1)).validate().is_err());
    assert!(template(None, Some(f64::NAN)).validate().is_err());
    assert!(template(None, Some(f64::INFINITY)).validate().is_err());
}
