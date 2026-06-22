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
