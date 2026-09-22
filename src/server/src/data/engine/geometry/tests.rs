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
        }
    );

    // The authored flags being ABSENT changes nothing either: open wins over
    // both authored extremes.
    let mut sparse = wall(Some(DoorState::Open));
    sparse.blocks_sight = None;
    sparse.blocks_light = None;
    sparse.blocks_move = None;
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
        }
    );

    let mut partial = wall(None);
    partial.blocks_move = Some(false);
    let flags = partial.effective_flags();
    assert!(!flags.blocks_move);
    assert!(flags.blocks_sight);
    assert!(flags.blocks_light);
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
