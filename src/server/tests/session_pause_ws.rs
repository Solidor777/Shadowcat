//! Proof: the session pause's refusals reach a real WebSocket. While `session.paused` is set on the
//! world-settings document, a player's `MoveRequest`, combat control and token position intent each
//! get the typed `paused` refusal, the GM's same requests succeed, and a player's chat still goes
//! through. This pins the `ws::conn` wiring (the typed-request check ahead of dispatch and the
//! `Intent` arm's check ahead of publish) that the `ws::session_pause` unit tests do not reach.

use shadowcat_test_support as common;

use common::*;
use futures_util::SinkExt;
use shadowcat::data::repository::Repository;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

/// Scene, player-owned token, and combat document ids (v4-shaped via `test_doc_id`).
const SCENE: u128 = 10;
/// The player-owned token's id.
const TOKEN: u128 = 11;
/// The combat document's id.
const COMBAT: u128 = 12;

/// The combatant document's id.
const COMBATANT: u128 = 13;

/// A `combatant` document create op under `COMBAT`: a GM-owned actor combatant with no token, so a
/// started combat has a turn to run.
fn create_combatant_op(world: Uuid) -> serde_json::Value {
    serde_json::json!({
        "op": "create",
        "doc": {
            "id": test_doc_id(COMBATANT),
            "scope": { "kind": "world", "world_id": world },
            "doc_type": "combatant",
            "schema_version": 1,
            "parent_id": test_doc_id(COMBAT),
            "engine": {
                "kind": { "type": "event", "lifespan": null, "message": null },
                "initiative": null, "tiebreak": 0.0, "resources": {}
            },
            "system": {},
            "created_at": 0,
            "updated_at": 0,
        }
    })
}

/// A `combat` document create op bound to `SCENE`, inactive, ordering `COMBATANT`.
fn create_combat_op(world: Uuid) -> serde_json::Value {
    serde_json::json!({
        "op": "create",
        "doc": {
            "id": test_doc_id(COMBAT),
            "scope": { "kind": "world", "world_id": world },
            "doc_type": "combat",
            "schema_version": 1,
            "parent_id": null,
            "engine": {
                "scene_id": test_doc_id(SCENE).to_string(), "active": false, "round": 0, "turn": null,
                "turn_control": "owner_may_end", "order": [test_doc_id(COMBATANT).to_string()],
                "movement": { "resource": null, "interpretation": "per_cell", "enforcement": "none" },
                "effect_cleanup": true, "rewind_restore": true, "forward_restore": false,
                "effect_lifecycle": { "onCombatEnd": null, "onTurnEnd": null, "onAdvance": null }
            },
            "system": {},
            "created_at": 0,
            "updated_at": 0,
        }
    })
}

/// An `Update` op setting `path` on `doc_id` from `old` to `new`.
fn update_op(
    doc_id: Uuid,
    path: &str,
    old: serde_json::Value,
    new: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "op": "update",
        "doc_id": doc_id,
        "changes": [{ "path": path, "old": old, "new": new, "remove": false }],
    })
}

/// A text frame carrying `value`.
fn text(value: serde_json::Value) -> Message {
    Message::Text(value.to_string())
}

/// Reads `event` frames until one whose ops name `doc_id`, so a later assertion never races the
/// commit it depends on.
async fn event_touching(ws: &mut Ws, doc_id: Uuid) -> serde_json::Value {
    loop {
        let ev = drain_until_event(ws).await;
        if ev.to_string().contains(&doc_id.to_string()) {
            return ev;
        }
    }
}

/// While paused: a player's `MoveRequest`, combat control and token position intent are each
/// refused with `paused`, the GM's same requests succeed, and a player's chat still goes through.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_paused_session_refuses_a_players_move_combat_and_token_intent_but_not_the_gm_or_chat() {
    let h = spawn().await;
    let (player, cookie) = h.add_player("player1").await;
    let mut gm = h.connect().await;
    let mut pws = h.connect_as(&cookie).await;
    drain_until_type(&mut gm, "welcome").await;
    drain_until_type(&mut pws, "welcome").await;

    // Scene, a player-owned token on cell centre (50, 50), and an inactive combat on the scene.
    gm.send(intent_msg(
        1,
        serde_json::json!([
            create_doc_op(h.world, SCENE, None, "scene"),
            create_owned_token_op(h.world, TOKEN, SCENE, player, 50.0, 50.0),
            create_combat_op(h.world),
            create_combatant_op(h.world),
        ]),
    ))
    .await
    .unwrap();
    event_touching(&mut gm, test_doc_id(COMBAT)).await;

    // The GM pauses the session.
    let settings = h
        .repo
        .query_documents(h.world, "world-settings")
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("the join-time seed created the world-settings document")
        .id;
    gm.send(intent_msg(
        2,
        serde_json::json!([update_op(
            settings,
            "/engine/session",
            serde_json::Value::Null,
            serde_json::json!({ "paused": true }),
        )]),
    ))
    .await
    .unwrap();
    event_touching(&mut gm, settings).await;

    // The player's move is refused with the typed `paused` reason.
    let move_req = |request_n: u128, goal_x: f64| {
        text(serde_json::json!({
            "type": "move_request",
            "request_id": Uuid::from_u128(request_n),
            "scene": test_doc_id(SCENE),
            "token_id": test_doc_id(TOKEN),
            "path": [[50.0, 50.0], [goal_x, 50.0]],
        }))
    };
    pws.send(move_req(100, 150.0)).await.unwrap();
    let refused = drain_until_type(&mut pws, "move_error").await;
    assert_eq!(refused["request_id"], Uuid::from_u128(100).to_string());
    assert_eq!(refused["reason"], "paused");

    // A player's combat control is refused with the same reason, for a control that would
    // otherwise name an existing combat.
    pws.send(text(serde_json::json!({
        "type": "combat_advance",
        "request_id": Uuid::from_u128(101),
        "combat_id": test_doc_id(COMBAT),
    })))
    .await
    .unwrap();
    let refused = drain_until_type(&mut pws, "combat_error").await;
    assert_eq!(refused["request_id"], Uuid::from_u128(101).to_string());
    assert_eq!(refused["reason"], "paused");

    // A player's token position intent is refused with `paused`.
    pws.send(intent_msg(
        3,
        serde_json::json!([update_op(
            test_doc_id(TOKEN),
            "/engine/x",
            serde_json::json!(50.0),
            serde_json::json!(150.0),
        )]),
    ))
    .await
    .unwrap();
    let refused = drain_until_type(&mut pws, "reject").await;
    assert_eq!(refused["intent_id"], Uuid::from_u128(3).to_string());
    assert_eq!(refused["reason"], "paused");

    // A player's chat still goes through: the broadcast `message` create reaches the player.
    pws.send(text(serde_json::json!({
        "type": "send_message",
        "request_id": Uuid::from_u128(102),
        "channel": "general",
        "content": "still talking",
        "actor_owner": null,
    })))
    .await
    .unwrap();
    let chat = drain_until_event(&mut pws).await;
    assert_eq!(chat["command"]["ops"][0]["doc"]["doc_type"], "message");

    // The GM's same combat control and move succeed while paused.
    gm.send(text(serde_json::json!({
        "type": "combat_start",
        "request_id": Uuid::from_u128(103),
        "combat_id": test_doc_id(COMBAT),
    })))
    .await
    .unwrap();
    let ok = drain_until_type(&mut gm, "combat_result").await;
    assert_eq!(ok["request_id"], Uuid::from_u128(103).to_string());

    gm.send(move_req(104, 150.0)).await.unwrap();
    let moved = event_touching(&mut gm, test_doc_id(TOKEN)).await;
    assert_eq!(
        moved["command"]["ops"][0]["doc_id"],
        test_doc_id(TOKEN).to_string()
    );
}
