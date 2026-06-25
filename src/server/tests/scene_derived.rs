//! E2E: the SceneDerived channel — initial push, coalesced re-eval on scene
//! change, and unknown-channel error — over the real WS server.

mod common;

use common::*;
use futures_util::{SinkExt, StreamExt};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn identity_channel_pushes_on_scene_change() {
    let h = spawn().await;
    let mut ws = h.connect().await;
    let _ = ws.next().await; // Welcome

    // Subscribe to the debug identity channel → initial SceneDerived (count 0).
    ws.send(scene_subscribe(1, "identity")).await.unwrap();
    let first = drain_until_type(&mut ws, "scene_derived").await;
    assert_eq!(first["payload"]["entity_count"], 0);

    // Create a scene + child; after coalescing, expect a SceneDerived with the
    // new count and a computed_at_seq at or past the create's seq.
    ws.send(create_scene_with_children(h.world, 10, &[11]))
        .await
        .unwrap();
    let upd = drain_until_type(&mut ws, "scene_derived").await;
    assert_eq!(upd["payload"]["entity_count"], 2); // scene + child
    assert!(upd["computed_at_seq"].as_i64().unwrap() >= 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn vision_channel_works_over_the_wire_for_gm() {
    // The `vision` channel (unlike the debug `identity` one) is a real release-build channel.
    // The seeded user owns the world (GM) → mode "all" (no fog). Per-recipient masking + the
    // empty-fog path are covered by the `compute_derived` unit tests; the egress recompute-on-
    // change trigger is the same path the identity test exercises.
    let h = spawn().await;
    let mut ws = h.connect().await;
    let _ = ws.next().await; // Welcome

    ws.send(scene_subscribe(3, "vision")).await.unwrap();
    let first = drain_until_type(&mut ws, "scene_derived").await;
    assert_eq!(first["channel"], "vision");
    assert_eq!(first["payload"]["mode"], "all");
    // The GM has no fog → the dispatch-layer explored accumulation is a no-op (no `explored`).
    assert!(first["payload"].get("explored").is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn vision_emits_persistent_explored_for_a_player_across_reconnect() {
    // The player path (M9c): a token-owning player's masked vision payload carries a non-empty,
    // scene-tagged `explored` set that persists across a reconnect. Exercises the egress dispatch
    // wiring (enrich_vision_explored at the real socket sites with the recipient's own user/world).
    let h = spawn().await;
    let (player, cookie) = h.add_player("player1").await;

    // The GM creates a scene + a token owned by the player at the origin.
    let mut gm = h.connect().await;
    let _ = gm.next().await; // Welcome
    gm.send(intent_msg(
        1,
        serde_json::json!([
            create_doc_op(h.world, 10, None, "scene"),
            create_owned_token_op(h.world, 11, 10, player, 0.0, 0.0),
        ]),
    ))
    .await
    .unwrap();
    let _ = drain_until_event(&mut gm).await; // the create committed (ECS hydrated before the event)

    // The player subscribes → a masked payload with a non-empty explored set for the scene.
    let mut pws = h.connect_as(&cookie).await;
    let _ = pws.next().await; // Welcome
    pws.send(scene_subscribe(5, "vision")).await.unwrap();
    let first = drain_until_type(&mut pws, "scene_derived").await;
    assert_eq!(first["payload"]["mode"], "masked");
    let explored = first["payload"]["explored"].as_array().unwrap();
    assert_eq!(explored.len(), 1);
    assert_eq!(explored[0]["scene"], json_uuid(10));
    assert!(
        !explored[0]["cells"].as_array().unwrap().is_empty(),
        "the player's vision marked explored cells"
    );

    // Reconnect → the persisted explored is re-emitted (cross-device/-reconnect persistence).
    drop(pws);
    let mut again = h.connect_as(&cookie).await;
    let _ = again.next().await; // Welcome
    again.send(scene_subscribe(6, "vision")).await.unwrap();
    let second = drain_until_type(&mut again, "scene_derived").await;
    assert!(
        !second["payload"]["explored"][0]["cells"]
            .as_array()
            .unwrap()
            .is_empty(),
        "explored persisted across reconnect"
    );
}

fn json_uuid(n: u128) -> serde_json::Value {
    serde_json::Value::String(uuid::Uuid::from_u128(n).to_string())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn vision_frame_includes_lit_mask_after_room_hydration() {
    // Exercises the apply_op LIVE-UPDATE path: the GM connects first (creating the room via
    // get_or_create), then publishes world-settings + scene + player token + bright light
    // through a single intent — those ops flow through apply_op, which keeps the room's
    // side-tables current. The player then subscribes and receives a SceneDerived frame.
    //
    // What this test proves: the lit mask flows end-to-end through the SceneDerived WS frame
    // (apply_op → SceneEcs side-tables → compute_derived → wire payload). It does NOT prove
    // get_or_create cold-start hydration — the room already exists when the GM publishes,
    // so the config-docs arrive via apply_op, not via the DB query_documents path.
    // Cold-start hydration (get_or_create reading from a pre-populated DB) is covered by
    // the `get_or_create_hydrates_config_and_actors_from_db` unit test in ws/room.rs.
    let h = spawn().await;
    let (player, player_cookie) = h.add_player("litplayer").await;

    // The GM publishes: world-settings (full structural guard so resolve_scene reads through),
    // a scene, a player-owned token at (50,50) — cell (0,0) with the default 100-unit grid —
    // and a bright light at (50,50) covering that cell.
    let mut gm = h.connect().await;
    let _ = gm.next().await; // Welcome

    gm.send(intent_msg(
        1,
        serde_json::json!([
            // world-settings: the structural guard requires scene+pathfinding+animation objects.
            // lightingEnabled defaults to true and lightMode to "environmentLight"; with
            // env_intensity 0.0 (default) + a bright point light the token's cell is lit.
            {
                "op": "create",
                "doc": {
                    "id": json_uuid(50),
                    "scope": { "kind": "world", "world_id": h.world },
                    "doc_type": "world-settings",
                    "schema_version": 1,
                    "system": {
                        "scene": {
                            "lightingEnabled": true,
                            "lightMode": "environmentLight",
                            "losRestriction": true,
                            "fog": true,
                            "observerVision": false,
                            "environment": { "color": "#0a0e1a", "intensity": 0.0 }
                        },
                        "pathfinding": { "diagonalRule": "chebyshev" },
                        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
                    },
                    "created_at": 0,
                    "updated_at": 0,
                }
            },
            create_doc_op(h.world, 10, None, "scene"),
            create_owned_token_op(h.world, 11, 10, player, 50.0, 50.0),
            // Bright light at the token's cell: brightRadius 3 covers cell (0,0) fully.
            {
                "op": "create",
                "doc": {
                    "id": json_uuid(20),
                    "scope": { "kind": "world", "world_id": h.world },
                    "doc_type": "light",
                    "schema_version": 1,
                    "parent_id": json_uuid(10),
                    "system": {
                        "x": 50.0, "y": 50.0,
                        "color": "#ffffff",
                        "intensity": 1.0,
                        "brightRadius": 3.0,
                        "dimRadius": 6.0,
                        "enabled": true
                    },
                    "created_at": 0,
                    "updated_at": 0,
                }
            },
        ]),
    ))
    .await
    .unwrap();
    let _ = drain_until_event(&mut gm).await; // wait for the commit (ECS hydrated before the event)

    // The player opens a `vision` scene subscription. The room was created when the GM
    // connected; the config-docs were hydrated in `get_or_create` so the first frame
    // already carries a non-empty lit mask.
    let mut pws = h.connect_as(&player_cookie).await;
    let _ = pws.next().await; // Welcome
    pws.send(scene_subscribe(5, "vision")).await.unwrap();
    let first = drain_until_type(&mut pws, "scene_derived").await;

    assert_eq!(
        first["payload"]["mode"], "masked",
        "player gets the masked payload"
    );
    let lit = first["payload"]["lit"]
        .as_array()
        .expect("lit array present in masked payload");
    assert!(
        !lit.is_empty(),
        "lit mask is non-empty — at least one scene has lit cells"
    );
    // cells is a flat integer array packed 4 ints/cell (i, j, band_index, tint);
    // len >= 4 means >= 1 cell; len % 4 == 0 proves the packing invariant is intact.
    let cells = lit[0]["cells"].as_array().unwrap();
    assert!(
        cells.len() >= 4,
        "at least one lit cell in the first scene entry"
    );
    assert_eq!(
        cells.len() % 4,
        0,
        "cells is a flat array packed 4 ints/cell (i,j,band,tint)"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gm_can_see_as_player_but_a_player_cannot_see_as_another() {
    // M9c-2 see-as-player: a GM subscribing `vision` with `as_user = player` receives EXACTLY that
    // player's masked view (their polygons + explored). A non-GM `as_user` is rejected — the
    // player-to-player access boundary.
    let h = spawn().await;
    let (player, player_cookie) = h.add_player("seen").await;
    let (_other, other_cookie) = h.add_player("snoop").await;

    // The GM creates a scene + a token owned by `player`.
    let mut gm = h.connect().await;
    let _ = gm.next().await; // Welcome
    gm.send(intent_msg(
        1,
        serde_json::json!([
            create_doc_op(h.world, 10, None, "scene"),
            create_owned_token_op(h.world, 11, 10, player, 0.0, 0.0),
        ]),
    ))
    .await
    .unwrap();
    let _ = drain_until_event(&mut gm).await;

    // The player connects + subscribes first, accumulating their own explored memory.
    let mut pre = h.connect_as(&player_cookie).await;
    let _ = pre.next().await; // Welcome
    pre.send(scene_subscribe(6, "vision")).await.unwrap();
    let _ = drain_until_type(&mut pre, "scene_derived").await; // persisted the player's explored
    drop(pre);

    // GM sees-as `player` → masked payload reflecting the player's view (their live polygons + the
    // explored memory they accumulated), NOT the GM's own mode:"all".
    gm.send(scene_subscribe_as(7, "vision", player))
        .await
        .unwrap();
    let seen = drain_until_type(&mut gm, "scene_derived").await;
    assert_eq!(seen["payload"]["mode"], "masked");
    assert!(
        !seen["payload"]["polygons"].as_array().unwrap().is_empty(),
        "the GM sees the target player's live vision polygons"
    );
    assert!(
        !seen["payload"]["explored"][0]["cells"]
            .as_array()
            .unwrap()
            .is_empty(),
        "the GM sees the target player's accumulated explored memory"
    );

    // A non-GM player attempting `as_user` (the other player) is REJECTED.
    let mut snoop = h.connect_as(&other_cookie).await;
    let _ = snoop.next().await; // Welcome
    snoop
        .send(scene_subscribe_as(8, "vision", player))
        .await
        .unwrap();
    let err = drain_until_type(&mut snoop, "scene_error").await;
    assert!(err["message"]
        .as_str()
        .unwrap()
        .contains("not authorized to view as another user"));

    // The same player subscribing normally (their own vision) still works.
    let mut p = h.connect_as(&player_cookie).await;
    let _ = p.next().await; // Welcome
    p.send(scene_subscribe(9, "vision")).await.unwrap();
    let own = drain_until_type(&mut p, "scene_derived").await;
    assert_eq!(own["payload"]["mode"], "masked");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn see_as_a_non_member_is_rejected() {
    // A GM may only see-as an actual member; a stranger UUID is rejected (not silently honored).
    let h = spawn().await;
    let mut gm = h.connect().await;
    let _ = gm.next().await; // Welcome
    gm.send(scene_subscribe_as(7, "vision", uuid::Uuid::from_u128(999)))
        .await
        .unwrap();
    let err = drain_until_type(&mut gm, "scene_error").await;
    assert!(err["message"].as_str().unwrap().contains("not a member"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn duplicate_scene_subscribe_id_is_rejected() {
    // A reused request_id would silently orphan the prior scene sub (mirrors the search path guard).
    let h = spawn().await;
    let mut ws = h.connect().await;
    let _ = ws.next().await; // Welcome
    ws.send(scene_subscribe(5, "vision")).await.unwrap();
    let _ = drain_until_type(&mut ws, "scene_derived").await; // the first registers
    ws.send(scene_subscribe(5, "vision")).await.unwrap(); // same request_id
    let err = drain_until_type(&mut ws, "scene_error").await;
    assert!(err["message"]
        .as_str()
        .unwrap()
        .contains("duplicate subscription id"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unknown_channel_errors() {
    let h = spawn().await;
    let mut ws = h.connect().await;
    let _ = ws.next().await; // Welcome

    ws.send(scene_subscribe(2, "no_such_channel"))
        .await
        .unwrap();
    let err = drain_until_type(&mut ws, "scene_error").await;
    assert!(err["message"].as_str().unwrap().contains("unknown channel"));
}
