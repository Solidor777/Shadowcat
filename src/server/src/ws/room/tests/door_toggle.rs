//! `Room::toggle_door` — the positional reach + visibility gate behind
//! `ClientMsg::ToggleDoor`, its GM bypass, its `Locked` state machine, and the
//! role-conditioned refusal-wording discipline (`door_toggle_wire_message`).
//!
//! Geometry conventions (square grid, cell size 100): the token stands at
//! (50,50) beside a light at (50,50) (bright 1.5 / dim 3.0 cells, environment
//! intensity 0 — `movement_scene("visible", true)`). The NEAR door is the
//! segment x=150, y∈[100,200] (interior to grid column 1 at cell size 100), its
//! nearest point to the token is (150,100) at distance √12500 ≈ 111.8 ≤
//! `DOOR_REACH_CELLS` × 100, and its cells are lit and in line of sight — so
//! reach and geometric visibility both hold for it. The FAR door is the segment
//! (300,100)-(400,100), whose nearest point sits ≈ 254.9 world units from the
//! token — beyond reach. The SIGHT wall (y=75, x∈[0,300], blocks sight only)
//! cuts the token's line of sight to the near door's cells without touching
//! reach.

use super::*;
use crate::auth::role::ServerRole;
use crate::data::document::{DocRole, Visibility};
use crate::data::engine::DoorState;
use serde_json::json;

/// The near door's segment (see the module doc).
const NEAR_SEG: (f64, f64, f64, f64) = (150.0, 100.0, 150.0, 200.0);
/// The far door's segment (see the module doc).
const FAR_SEG: (f64, f64, f64, f64) = (300.0, 100.0, 400.0, 100.0);

/// Publish a door-carrying wall into `h`'s scene. `door: None` authors no `door`
/// key at all (an ordinary wall). `gm_only` hides the `/engine` band from
/// non-GMs via the property-override tier.
async fn place_door_wall(
    h: &MovementHandle,
    id: u128,
    seg: (f64, f64, f64, f64),
    door: Option<&str>,
    gm_only: bool,
) -> Uuid {
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let wall_id = crate::test_ids::test_uuid(id);
    let mut wall = wdoc(h.world_id, wall_id, "wall");
    wall.parent_id = Some(h.scene_id);
    wall.owner = Some(h.gm.user_id);
    // World-readable envelope: the door affordance exists client-side only for
    // a wall the player's document stream delivers.
    wall.permissions.default = DocRole::Observer;
    let mut engine = json!({
        "seg": { "x1": seg.0, "y1": seg.1, "x2": seg.2, "y2": seg.3 },
        "blocksMove": true,
        "blocksSight": true,
        "blocksLight": true,
    });
    if let Some(door) = door {
        engine["door"] = json!(door);
    }
    wall.engine = Some(engine);
    if gm_only {
        wall.permissions
            .property_overrides
            .insert("/engine".into(), Visibility::GmOnly);
    }
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: wall }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    wall_id
}

/// Publish the sight-only wall that cuts the token's line of sight to the near
/// door's cells (see the module doc).
async fn place_sight_wall(h: &MovementHandle, id: u128) {
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let wall_id = crate::test_ids::test_uuid(id);
    let mut wall = wdoc(h.world_id, wall_id, "wall");
    wall.parent_id = Some(h.scene_id);
    wall.owner = Some(h.gm.user_id);
    wall.permissions.default = DocRole::Observer;
    wall.engine = Some(json!({
        "seg": { "x1": 0.0, "y1": 75.0, "x2": 300.0, "y2": 75.0 },
        "blocksMove": false,
        "blocksSight": true,
        "blocksLight": false,
    }));
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: wall }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
}

/// A `ToggleDoor` request against `h`'s scene/wall.
fn toggle_req(
    h: &MovementHandle,
    wall_id: Uuid,
    to: DoorState,
) -> crate::ws::room::ToggleDoorInputs {
    crate::ws::room::ToggleDoorInputs {
        request_id: crate::test_ids::test_uuid(0xD001),
        scene_id: h.scene_id,
        wall_id,
        to,
        ts: 0,
    }
}

/// The wall's currently-stored `door` state, read from the hydrated ECS mirror.
async fn stored_door(h: &MovementHandle, wall_id: Uuid) -> Option<String> {
    h.room
        .scene()
        .read()
        .await
        .scene_entity_doc(wall_id)
        .and_then(|d| d.engine)
        .and_then(|e| e.get("door")?.as_str().map(|s| s.to_string()))
}

#[tokio::test]
async fn toggle_door_refused_when_the_actor_controls_no_token_in_the_scene() {
    let h = movement_scene("visible", true).await;
    let wall_id = place_door_wall(&h, 0xD100, NEAR_SEG, Some("closed"), false).await;
    // A second world member with NO token in the scene: ownership (clause 1) fails
    // before reach or visibility are consulted.
    let p2 = h
        .repo
        .create_user("player2", None, ServerRole::User, 0)
        .await
        .unwrap();
    h.repo
        .add_member(h.world_id, p2, WorldRole::Player)
        .await
        .unwrap();
    let player2 = PermissionContext {
        user_id: p2,
        world_role: WorldRole::Player,
    };
    let res = h
        .room
        .toggle_door(&h.repo, &player2, toggle_req(&h, wall_id, DoorState::Open))
        .await;
    assert!(
        matches!(res, Err(DoorToggleError::NoOwnedToken)),
        "expected NoOwnedToken, got {res:?}"
    );
    assert_eq!(stored_door(&h, wall_id).await.as_deref(), Some("closed"));
}

#[tokio::test]
async fn toggle_door_refused_when_the_nearest_owned_token_is_out_of_door_reach_cells() {
    let h = movement_scene("visible", true).await;
    // The far door's nearest point is ≈ 254.9 world units from the token —
    // beyond `DOOR_REACH_CELLS` × 100 = 150 (clause 2).
    let wall_id = place_door_wall(&h, 0xD101, FAR_SEG, Some("closed"), false).await;
    let res = h
        .room
        .toggle_door(&h.repo, &h.player, toggle_req(&h, wall_id, DoorState::Open))
        .await;
    assert!(
        matches!(res, Err(DoorToggleError::OutOfReach)),
        "expected OutOfReach, got {res:?}"
    );
    assert_eq!(stored_door(&h, wall_id).await.as_deref(), Some("closed"));
}

#[tokio::test]
async fn toggle_door_refused_when_in_reach_but_the_doors_cells_are_outside_the_actors_visible_cells(
) {
    let h = movement_scene("visible", true).await;
    let wall_id = place_door_wall(&h, 0xD102, NEAR_SEG, Some("closed"), false).await;
    // The sight wall cuts line of sight to the door's cells while reach still
    // holds (≈ 111.8 ≤ 150): clause 3 alone refuses.
    place_sight_wall(&h, 0xD103).await;
    let res = h
        .room
        .toggle_door(&h.repo, &h.player, toggle_req(&h, wall_id, DoorState::Open))
        .await;
    assert!(
        matches!(res, Err(DoorToggleError::NotVisible)),
        "expected NotVisible, got {res:?}"
    );
    assert_eq!(stored_door(&h, wall_id).await.as_deref(), Some("closed"));
}

/// The clause-(4) pin: clauses 1–3 ALL pass (the token is adjacent, and the
/// door's cells are inside its visible-cell set — the visibility raycast is
/// deliberately permission-blind), and the toggle is STILL refused because the
/// wall's `/engine` tier is `gm_only`. Deleting the `engine_tier_visible` check
/// alone, with every other clause unchanged, flips this test to a pass-through.
#[tokio::test]
async fn toggle_door_refused_for_a_gm_only_door_even_when_adjacent_and_in_line_of_sight() {
    let h = movement_scene("visible", true).await;
    let wall_id = place_door_wall(&h, 0xD104, NEAR_SEG, Some("closed"), true).await;
    let res = h
        .room
        .toggle_door(&h.repo, &h.player, toggle_req(&h, wall_id, DoorState::Open))
        .await;
    assert!(
        matches!(res, Err(DoorToggleError::WallNotVisible)),
        "expected WallNotVisible, got {res:?}"
    );
    assert_eq!(stored_door(&h, wall_id).await.as_deref(), Some("closed"));
}

#[tokio::test]
async fn toggle_door_succeeds_when_in_reach_visible_and_wall_visible() {
    let h = movement_scene("visible", true).await;
    let wall_id = place_door_wall(&h, 0xD105, NEAR_SEG, Some("closed"), false).await;

    // closed → open
    h.room
        .toggle_door(&h.repo, &h.player, toggle_req(&h, wall_id, DoorState::Open))
        .await
        .expect("closed → open in reach and in sight must succeed");
    assert_eq!(stored_door(&h, wall_id).await.as_deref(), Some("open"));
    // The write persisted, not just the ECS mirror.
    let persisted = h.repo.get_document(wall_id).await.unwrap().unwrap();
    assert_eq!(
        persisted
            .engine
            .as_ref()
            .and_then(|e| e.get("door"))
            .and_then(|d| d.as_str()),
        Some("open")
    );
    // An open door is a hole: it leaves the routing wall set even though every
    // authored flag is still set.
    assert!(
        !h.room
            .scene()
            .read()
            .await
            .move_wall_entries(h.scene_id, None)
            .iter()
            .any(|(s, _)| *s
                == crate::scene::vision::Seg {
                    a: (NEAR_SEG.0, NEAR_SEG.1),
                    b: (NEAR_SEG.2, NEAR_SEG.3),
                }),
        "an open door must drop out of the movement wall set"
    );

    // open → closed
    h.room
        .toggle_door(
            &h.repo,
            &h.player,
            toggle_req(&h, wall_id, DoorState::Closed),
        )
        .await
        .expect("open → closed in reach and in sight must succeed");
    assert_eq!(stored_door(&h, wall_id).await.as_deref(), Some("closed"));
}

#[tokio::test]
async fn toggle_door_never_flips_locked_for_a_non_gm() {
    let h = movement_scene("visible", true).await;

    // A wall currently Locked refuses a non-GM's request regardless of `to`.
    let locked_id = place_door_wall(&h, 0xD106, NEAR_SEG, Some("locked"), false).await;
    for to in [DoorState::Open, DoorState::Closed] {
        let res = h
            .room
            .toggle_door(&h.repo, &h.player, toggle_req(&h, locked_id, to))
            .await;
        assert!(
            matches!(res, Err(DoorToggleError::Locked)),
            "a non-GM must never flip off Locked (to {to:?}), got {res:?}"
        );
    }
    assert_eq!(stored_door(&h, locked_id).await.as_deref(), Some("locked"));

    // A non-GM proposing `to: Locked` is refused even on a flippable door.
    let closed_id = place_door_wall(&h, 0xD107, NEAR_SEG, Some("closed"), false).await;
    let res = h
        .room
        .toggle_door(
            &h.repo,
            &h.player,
            toggle_req(&h, closed_id, DoorState::Locked),
        )
        .await;
    assert!(
        matches!(res, Err(DoorToggleError::Locked)),
        "a non-GM must never flip onto Locked, got {res:?}"
    );
    assert_eq!(stored_door(&h, closed_id).await.as_deref(), Some("closed"));

    // A doorless wall has nothing to flip (the never-absent half of the rule).
    let plain_id = place_door_wall(&h, 0xD108, NEAR_SEG, None, false).await;
    let res = h
        .room
        .toggle_door(
            &h.repo,
            &h.player,
            toggle_req(&h, plain_id, DoorState::Open),
        )
        .await;
    assert!(
        matches!(res, Err(DoorToggleError::Locked)),
        "a doorless wall must refuse a non-GM toggle, got {res:?}"
    );
    assert_eq!(stored_door(&h, plain_id).await, None);
}

#[tokio::test]
async fn toggle_door_gm_flips_from_anywhere_in_the_world() {
    // No GM token, no reach, no geometric visibility, no engine-tier visibility
    // requirement: the GM bypasses clauses 1–4 entirely.
    let h = movement_scene("visible", true).await;
    let far_hidden = place_door_wall(&h, 0xD109, FAR_SEG, Some("closed"), true).await;
    h.room
        .toggle_door(&h.repo, &h.gm, toggle_req(&h, far_hidden, DoorState::Open))
        .await
        .expect("a GM flips a far, gm_only door");
    assert_eq!(stored_door(&h, far_hidden).await.as_deref(), Some("open"));

    // The GM may also set and clear Locked (the full wall editor's authority,
    // reachable through this intent too).
    h.room
        .toggle_door(
            &h.repo,
            &h.gm,
            toggle_req(&h, far_hidden, DoorState::Locked),
        )
        .await
        .expect("a GM may lock");
    assert_eq!(stored_door(&h, far_hidden).await.as_deref(), Some("locked"));
    h.room
        .toggle_door(
            &h.repo,
            &h.gm,
            toggle_req(&h, far_hidden, DoorState::Closed),
        )
        .await
        .expect("a GM may unlock");
    assert_eq!(stored_door(&h, far_hidden).await.as_deref(), Some("closed"));
}

/// The refusal-wording uniformity pin (a non-GM learns NOTHING from the text):
/// every refusal scenario reachable by a non-GM produces the identical wire
/// message. Driven end-to-end through `Room::toggle_door` so the proof covers
/// the handler's own classification, not just the error type's `Display`.
#[tokio::test]
async fn toggle_door_non_gm_refusal_message_is_byte_identical_across_every_refusal_reason() {
    let mut messages: Vec<String> = Vec::new();

    // UnknownWall: the id names nothing in the ECS.
    {
        let h = movement_scene("visible", true).await;
        let res = h
            .room
            .toggle_door(
                &h.repo,
                &h.player,
                toggle_req(&h, crate::test_ids::test_uuid(0xD10A), DoorState::Open),
            )
            .await;
        messages.push(refusal_wire_text(&h, res));
    }
    // SceneMismatch: the wall lives in the scene, but the request names a
    // different scene id.
    {
        let h = movement_scene("visible", true).await;
        let wall_id = place_door_wall(&h, 0xD10B, NEAR_SEG, Some("closed"), false).await;
        let mut req = toggle_req(&h, wall_id, DoorState::Open);
        req.scene_id = crate::test_ids::test_uuid(0xD10C);
        let res = h.room.toggle_door(&h.repo, &h.player, req).await;
        messages.push(refusal_wire_text(&h, res));
    }
    // NoOwnedToken: a member with no token in the scene.
    {
        let h = movement_scene("visible", true).await;
        let wall_id = place_door_wall(&h, 0xD10D, NEAR_SEG, Some("closed"), false).await;
        let p2 = h
            .repo
            .create_user("player2", None, ServerRole::User, 0)
            .await
            .unwrap();
        h.repo
            .add_member(h.world_id, p2, WorldRole::Player)
            .await
            .unwrap();
        let player2 = PermissionContext {
            user_id: p2,
            world_role: WorldRole::Player,
        };
        let res = h
            .room
            .toggle_door(&h.repo, &player2, toggle_req(&h, wall_id, DoorState::Open))
            .await;
        messages.push(refusal_wire_text(&h, res));
    }
    // OutOfReach: the far door.
    {
        let h = movement_scene("visible", true).await;
        let wall_id = place_door_wall(&h, 0xD10E, FAR_SEG, Some("closed"), false).await;
        let res = h
            .room
            .toggle_door(&h.repo, &h.player, toggle_req(&h, wall_id, DoorState::Open))
            .await;
        messages.push(refusal_wire_text(&h, res));
    }
    // NotVisible: the near door behind the sight wall.
    {
        let h = movement_scene("visible", true).await;
        let wall_id = place_door_wall(&h, 0xD10F, NEAR_SEG, Some("closed"), false).await;
        place_sight_wall(&h, 0xD110).await;
        let res = h
            .room
            .toggle_door(&h.repo, &h.player, toggle_req(&h, wall_id, DoorState::Open))
            .await;
        messages.push(refusal_wire_text(&h, res));
    }
    // WallNotVisible: the gm_only door, adjacent and in line of sight.
    {
        let h = movement_scene("visible", true).await;
        let wall_id = place_door_wall(&h, 0xD111, NEAR_SEG, Some("closed"), true).await;
        let res = h
            .room
            .toggle_door(&h.repo, &h.player, toggle_req(&h, wall_id, DoorState::Open))
            .await;
        messages.push(refusal_wire_text(&h, res));
    }
    // Locked: a non-GM proposing `to: Locked` on a flippable door.
    {
        let h = movement_scene("visible", true).await;
        let wall_id = place_door_wall(&h, 0xD112, NEAR_SEG, Some("closed"), false).await;
        let res = h
            .room
            .toggle_door(
                &h.repo,
                &h.player,
                toggle_req(&h, wall_id, DoorState::Locked),
            )
            .await;
        messages.push(refusal_wire_text(&h, res));
    }

    assert_eq!(
        messages.len(),
        7,
        "every refusal scenario must actually refuse"
    );
    for m in &messages {
        assert_eq!(m, "door rejected", "the non-GM wording is uniform");
    }
    assert!(
        messages.windows(2).all(|w| w[0] == w[1]),
        "byte-identical across every refusal reason: {messages:?}"
    );
}

/// The refusal's wire text exactly as `ws::conn`'s handler wrapper computes it
/// (`door_toggle_wire_message` against the caller's role).
fn refusal_wire_text(h: &MovementHandle, res: Result<(), DoorToggleError>) -> String {
    crate::ws::room::door_toggle_wire_message(
        &res.expect_err("this scenario must refuse"),
        h.player.world_role,
    )
}

/// The GM half of the discipline: a GM caller MAY receive variant-specific
/// diagnostics (the same GM-only-reachable distinction `CombatRewind`'s wording
/// precedent establishes) — the split is deliberate and role-conditioned, not
/// an oversight in either direction.
#[tokio::test]
async fn toggle_door_gm_refusal_message_may_be_distinct_per_reason() {
    use crate::data::document::WorldRole;
    let gm = WorldRole::Gm;
    let player = WorldRole::Player;

    // The mapping itself: distinct diagnostics per variant for a GM...
    let variants = [
        DoorToggleError::UnknownWall,
        DoorToggleError::SceneMismatch,
        DoorToggleError::NoOwnedToken,
        DoorToggleError::OutOfReach,
        DoorToggleError::NotVisible,
        DoorToggleError::WallNotVisible,
        DoorToggleError::Locked,
    ];
    let gm_texts: std::collections::HashSet<String> = variants
        .iter()
        .map(|v| crate::ws::room::door_toggle_wire_message(v, gm))
        .collect();
    assert_eq!(
        gm_texts.len(),
        variants.len(),
        "the GM diagnostics distinguish every refusal reason"
    );
    // ...while the same variants all read identically for a non-GM.
    let player_texts: std::collections::HashSet<String> = variants
        .iter()
        .map(|v| crate::ws::room::door_toggle_wire_message(v, player))
        .collect();
    assert_eq!(
        player_texts.len(),
        1,
        "the non-GM wording is one uniform string"
    );

    // Driven end-to-end as a GM: UnknownWall vs SceneMismatch read differently.
    let h = movement_scene("visible", true).await;
    let wall_id = place_door_wall(&h, 0xD113, NEAR_SEG, Some("closed"), false).await;
    let unknown = h
        .room
        .toggle_door(
            &h.repo,
            &h.gm,
            toggle_req(&h, crate::test_ids::test_uuid(0xD114), DoorState::Open),
        )
        .await;
    let mut mismatch_req = toggle_req(&h, wall_id, DoorState::Open);
    mismatch_req.scene_id = crate::test_ids::test_uuid(0xD115);
    let mismatch = h.room.toggle_door(&h.repo, &h.gm, mismatch_req).await;
    let unknown_text =
        crate::ws::room::door_toggle_wire_message(&unknown.expect_err("unknown wall refuses"), gm);
    let mismatch_text = crate::ws::room::door_toggle_wire_message(
        &mismatch.expect_err("scene mismatch refuses"),
        gm,
    );
    assert_ne!(
        unknown_text, mismatch_text,
        "the GM path is not held to the non-GM uniformity"
    );
}
