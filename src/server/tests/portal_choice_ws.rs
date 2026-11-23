//! Proof: the portal offer and its answer reach real WebSockets. A token placed inside a
//! `portal_choice` region is offered to its owner and to the GM but never to another player; the
//! owner's answer teleports the token; a stranger's answer is refused with the uniform wording; and
//! a paused session refuses a player's accepting answer while the GM, a decline and `open_portal`
//! stay allowed. This pins the `ws::conn` wiring (addressed egress, dispatch, the pause check) that
//! the room-level unit tests do not reach.

use shadowcat_test_support as common;

use common::*;
use futures_util::SinkExt;
use shadowcat::data::repository::Repository;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

/// The portal's scene.
const SCENE: u128 = 30;
/// The player-owned token.
const TOKEN: u128 = 31;
/// The portal region, covering cell (1, 0) of the size-100 grid.
const REGION: u128 = 32;
/// The second scene option 1 leads to.
const DEST: u128 = 33;

/// A text frame carrying `value`.
fn text(value: serde_json::Value) -> Message {
    Message::Text(value.to_string())
}

/// A world-readable `portal_choice` region on `SCENE`'s cell (1, 0): option 0 stays on the scene,
/// option 1 leads to `DEST` and reveals its name.
fn create_portal_region_op(world: Uuid) -> serde_json::Value {
    serde_json::json!({
        "op": "create",
        "doc": {
            "id": test_doc_id(REGION),
            "scope": { "kind": "world", "world_id": world },
            "doc_type": "region",
            "schema_version": 1,
            "parent_id": test_doc_id(SCENE),
            "permissions": { "default": "observer", "users": {}, "property_overrides": {} },
            "engine": {
                "shape": { "kind": "rect", "points": [100.0, 0.0, 200.0, 100.0] },
                "behavior": "terrain", "cost": 1.0, "enabled": true,
                "triggers": [{ "on": "enter", "effect": {
                    "type": "portal_choice", "prompt": "Where to?",
                    "options": [
                        { "label": "Back room", "target": { "x": 450.0, "y": 450.0 } },
                        { "label": "Down the stairs", "reveal_scene": true,
                          "target": { "scene": test_doc_id(DEST), "x": 250.0, "y": 250.0 } }
                    ] } }]
            },
            "system": {},
            "created_at": 0,
            "updated_at": 0,
        }
    })
}

/// The destination scene, named so a revealed name is observable.
fn create_named_dest_op(world: Uuid) -> serde_json::Value {
    let mut op = create_doc_op(world, DEST, None, "scene");
    op["doc"]["name"] = serde_json::json!("The Vault");
    op
}

/// An `Update` op writing the token's `/engine/x` from 50 to 150, placing it inside the region.
fn place_in_region_op() -> serde_json::Value {
    serde_json::json!({
        "op": "update",
        "doc_id": test_doc_id(TOKEN),
        "changes": [{ "path": "/engine/x", "old": 50.0, "new": 150.0, "remove": false }],
    })
}

/// Reads frames until one of type `ty`, panicking if a `portal_offer` arrives first when
/// `forbid_offer` is set: the frames before `ty` are exactly what the connection was sent.
async fn until_without_offer(ws: &mut Ws, ty: &str) -> serde_json::Value {
    loop {
        let frame = next_non_presence_frame(ws).await;
        assert_ne!(
            frame["type"], "portal_offer",
            "this connection must never be sent the offer"
        );
        if frame["type"] == ty {
            return frame;
        }
    }
}

/// The token's committed `/engine/x`.
async fn token_x(h: &Harness) -> f64 {
    h.repo
        .get_document(test_doc_id(TOKEN))
        .await
        .unwrap()
        .unwrap()
        .engine
        .unwrap()["x"]
        .as_f64()
        .unwrap()
}

/// Seeds the scene, destination, owned token and portal region, and places the token inside the
/// region. Returns the owner's id and the GM, owner and stranger sockets, the owner's and GM's
/// already holding the offer's frame.
async fn offered_world(h: &Harness) -> (Uuid, Ws, Ws, Ws, serde_json::Value, serde_json::Value) {
    let (owner, owner_cookie) = h.add_player("owner").await;
    let (_stranger, stranger_cookie) = h.add_player("stranger").await;
    let mut gm = h.connect().await;
    let mut ows = h.connect_as(&owner_cookie).await;
    let mut sws = h.connect_as(&stranger_cookie).await;
    for ws in [&mut gm, &mut ows, &mut sws] {
        drain_until_type(ws, "welcome").await;
    }
    gm.send(intent_msg(
        1,
        serde_json::json!([
            create_doc_op(h.world, SCENE, None, "scene"),
            create_named_dest_op(h.world),
            create_owned_token_op(h.world, TOKEN, SCENE, owner, 50.0, 50.0),
            create_portal_region_op(h.world),
        ]),
    ))
    .await
    .unwrap();
    loop {
        let ev = drain_until_event(&mut gm).await;
        if ev.to_string().contains(&test_doc_id(REGION).to_string()) {
            break;
        }
    }
    gm.send(intent_msg(2, serde_json::json!([place_in_region_op()])))
        .await
        .unwrap();
    let to_owner = drain_until_type(&mut ows, "portal_offer").await;
    let to_gm = drain_until_type(&mut gm, "portal_offer").await;
    (owner, gm, ows, sws, to_owner, to_gm)
}

/// The owner and the GM receive the offer, a stranger does not; the owner's answer teleports the
/// token and a stranger's answer is refused with the uniform wording.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_offer_reaches_only_the_owner_and_gm_and_the_owners_answer_teleports() {
    let h = spawn().await;
    let (owner, mut gm, mut ows, mut sws, to_owner, to_gm) = offered_world(&h).await;

    assert_eq!(to_owner["offer_id"], to_gm["offer_id"]);
    assert_eq!(to_owner["owner"], owner.to_string());
    assert_eq!(to_owner["gm_only"], false);
    assert_eq!(to_owner["options"][0]["label"], "Back room");
    assert_eq!(
        to_owner["options"][0]["destination_name"],
        serde_json::Value::Null
    );
    assert_eq!(to_owner["options"][1]["destination_name"], "The Vault");
    assert_eq!(token_x(&h).await, 150.0, "the offer alone moves nothing");

    // The stranger's own refusal is the sentinel: any offer frame sent to it would arrive first.
    sws.send(text(serde_json::json!({
        "type": "choose_portal",
        "request_id": Uuid::from_u128(40),
        "offer_id": to_owner["offer_id"],
        "choice": 0,
    })))
    .await
    .unwrap();
    let refused = until_without_offer(&mut sws, "portal_choice_error").await;
    assert_eq!(refused["request_id"], Uuid::from_u128(40).to_string());
    assert_eq!(refused["message"], "portal rejected");
    assert_eq!(
        token_x(&h).await,
        150.0,
        "a stranger's answer moves nothing"
    );

    // The owner's answer teleports the token to option 0.
    ows.send(text(serde_json::json!({
        "type": "choose_portal",
        "request_id": Uuid::from_u128(41),
        "offer_id": to_owner["offer_id"],
        "choice": 0,
    })))
    .await
    .unwrap();
    loop {
        let ev = drain_until_event(&mut gm).await;
        if ev.to_string().contains("/engine/y") {
            break;
        }
    }
    assert_eq!(token_x(&h).await, 450.0);
}

/// A paused session refuses a player's accepting answer with the typed `paused` reason while a
/// decline and `open_portal` stay allowed and the GM's answer goes through.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_paused_session_refuses_a_players_accepting_answer_but_not_a_decline_open_or_the_gm() {
    let h = spawn().await;
    let (_owner, mut gm, mut ows, _sws, to_owner, _to_gm) = offered_world(&h).await;

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
        3,
        serde_json::json!([{
            "op": "update",
            "doc_id": settings,
            "changes": [{ "path": "/engine/session", "old": null,
                          "new": { "paused": true }, "remove": false }],
        }]),
    ))
    .await
    .unwrap();
    loop {
        let ev = drain_until_event(&mut gm).await;
        if ev.to_string().contains(&settings.to_string()) {
            break;
        }
    }

    ows.send(text(serde_json::json!({
        "type": "choose_portal",
        "request_id": Uuid::from_u128(50),
        "offer_id": to_owner["offer_id"],
        "choice": 1,
    })))
    .await
    .unwrap();
    let refused = drain_until_type(&mut ows, "portal_choice_error").await;
    assert_eq!(refused["request_id"], Uuid::from_u128(50).to_string());
    assert_eq!(refused["reason"], "paused");
    assert_eq!(token_x(&h).await, 150.0, "the refused answer moves nothing");

    // A decline is allowed while paused, and consumes the offer; `open_portal` brings it back.
    ows.send(text(serde_json::json!({
        "type": "choose_portal",
        "request_id": Uuid::from_u128(51),
        "offer_id": to_owner["offer_id"],
        "choice": null,
    })))
    .await
    .unwrap();
    ows.send(text(serde_json::json!({
        "type": "open_portal",
        "request_id": Uuid::from_u128(52),
        "token": test_doc_id(TOKEN),
    })))
    .await
    .unwrap();
    let again = drain_until_type(&mut ows, "portal_offer").await;
    assert_ne!(again["offer_id"], to_owner["offer_id"]);

    // The GM answers for the player while paused.
    gm.send(text(serde_json::json!({
        "type": "choose_portal",
        "request_id": Uuid::from_u128(53),
        "offer_id": again["offer_id"],
        "choice": 0,
    })))
    .await
    .unwrap();
    loop {
        let ev = drain_until_event(&mut gm).await;
        if ev.to_string().contains("/engine/y") {
            break;
        }
    }
    assert_eq!(token_x(&h).await, 450.0);
}
