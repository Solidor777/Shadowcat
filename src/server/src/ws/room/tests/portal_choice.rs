//! The portal offer/answer exchange as `Room` runs it: a token that stops in a `portal_choice`
//! region is offered its destinations without moving, an answer applies the chosen target through
//! the same teleport a fixed portal uses, and every refusal reads the same to a non-GM.

use super::region_triggers::{
    move_token, notice_text, place_region, region_notices, trigger_region_engine,
};
use super::*;
use crate::data::document::DocRole;
use crate::ws::protocol::PortalOfferOption;
use serde_json::json;

/// One `PortalOffer` frame's fields, as a test reads them.
struct Offer {
    /// The id an answer names.
    offer_id: Uuid,
    /// The token the offer is for.
    token: Uuid,
    /// The options as sent.
    options: Vec<PortalOfferOption>,
    /// Whether only GM connections receive the frame.
    gm_only: bool,
    /// The token's effective owner as stamped on the frame.
    owner: Option<Uuid>,
}

/// Drains every buffered broadcast off `rx` and returns each `PortalOffer` frame, in order.
fn offers(rx: &mut tokio::sync::broadcast::Receiver<RoomEvent>) -> Vec<Offer> {
    use tokio::sync::broadcast::error::TryRecvError;
    let mut out = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(RoomEvent::Other(msg)) => {
                if let ServerMsg::PortalOffer {
                    offer_id,
                    token,
                    options,
                    gm_only,
                    owner,
                    ..
                } = &*msg
                {
                    out.push(Offer {
                        offer_id: *offer_id,
                        token: *token,
                        options: options.clone(),
                        gm_only: *gm_only,
                        owner: *owner,
                    });
                }
            }
            Ok(RoomEvent::Event(_)) | Err(TryRecvError::Lagged(_)) => {}
            Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => break,
        }
    }
    out
}

/// Creates a world-readable scene named `name`, for a cross-scene destination.
async fn place_named_scene(h: &MovementHandle, id: u128, name: &str) -> Uuid {
    let scene_id = crate::test_ids::test_uuid(id);
    let mut scene = crate::data::document::tests::world_scoped_doc(h.world_id, scene_id, "scene");
    scene.owner = Some(h.gm.user_id);
    scene.name = Some(name.to_string());
    scene.permissions.default = DocRole::Observer;
    scene.engine = Some(json!({ "grid": { "kind": "square", "size": 100.0 }, "background": null }));
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: scene }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    scene_id
}

/// Creates a region on `cell` that no non-GM may read (`permissions.default = None`), so its
/// `visible_to_all` is false.
async fn place_secret_region(
    h: &MovementHandle,
    id: u128,
    cell: (i32, i32),
    triggers: serde_json::Value,
) {
    let region_id = crate::test_ids::test_uuid(id);
    let mut region =
        crate::data::document::tests::world_scoped_doc(h.world_id, region_id, "region");
    region.parent_id = Some(h.scene_id);
    region.owner = Some(h.gm.user_id);
    region.permissions.default = DocRole::None;
    region.engine = Some(trigger_region_engine(cell, "terrain", triggers));
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: region }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
}

/// A `portal_choice` trigger list with `options` (each already a JSON option).
fn choice_triggers(options: serde_json::Value) -> serde_json::Value {
    json!([{ "on": "enter", "effect": {
        "type": "portal_choice", "prompt": "Where to?", "options": options } }])
}

/// A same-scene option with no revealed scene name.
fn plain_option(label: &str, x: f64, y: f64) -> serde_json::Value {
    json!({ "label": label, "target": { "x": x, "y": y } })
}

/// A second player, who owns nothing.
fn stranger() -> PermissionContext {
    PermissionContext {
        user_id: Uuid::from_u128(0xBEEF),
        world_role: WorldRole::Player,
    }
}

/// The token's committed `(x, y)`.
async fn token_xy(h: &MovementHandle) -> (f64, f64) {
    let eng = h
        .repo
        .get_document(h.token_id)
        .await
        .unwrap()
        .unwrap()
        .engine
        .unwrap();
    (eng["x"].as_f64().unwrap(), eng["y"].as_f64().unwrap())
}

/// The current commit time, the clock `move_token` stamps its offers with.
fn now() -> i64 {
    crate::ws::time::now_millis()
}

/// A scene with one portal region on cell (1,0) offering two same-scene options, the player's
/// token walked into it. Returns the handle, the room's offer receiver and the one offer.
async fn offered() -> (
    MovementHandle,
    tokio::sync::broadcast::Receiver<RoomEvent>,
    Offer,
) {
    let h = movement_scene("unrestricted", false).await;
    place_region(
        &h,
        0x8A0,
        (1, 0),
        "terrain",
        choice_triggers(json!([
            plain_option("North room", 450.0, 450.0),
            plain_option("South room", 450.0, 650.0)
        ])),
    )
    .await;
    let (mut rx, _) = h.room.subscribe();
    move_token(&h, vec![h.start, h.adj]).await;
    let mut got = offers(&mut rx);
    assert_eq!(got.len(), 1, "one offer frame for one portal entry");
    let offer = got.remove(0);
    (h, rx, offer)
}

#[tokio::test]
async fn entering_a_portal_choice_region_offers_without_moving_the_token() {
    let (h, _rx, offer) = offered().await;
    assert_eq!(offer.token, h.token_id);
    assert_eq!(
        (offer.gm_only, offer.owner),
        (false, Some(h.player.user_id)),
        "a public portal is addressed to the token's owner"
    );
    let labels: Vec<_> = offer.options.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(labels, ["North room", "South room"]);
    assert_eq!(
        token_xy(&h).await,
        h.adj,
        "the offer is made after the move; nothing teleports yet"
    );
}

#[tokio::test]
async fn an_accepted_choice_commits_the_same_ops_as_a_fixed_teleport_to_that_target() {
    // The fixed run: a `teleport` region to (450, 650).
    let fixed = movement_scene("unrestricted", false).await;
    place_region(
        &fixed,
        0x8A1,
        (1, 0),
        "terrain",
        json!([{ "on": "enter", "effect": { "type": "teleport",
            "target": { "scene": null, "x": 450.0, "y": 650.0, "elevation": 3.0, "vfx": null } } }]),
    )
    .await;
    move_token(&fixed, vec![fixed.start, fixed.adj]).await;

    // The chosen run: the same target as option 1 of a `portal_choice` region.
    let (h, _rx, offer) = {
        let h = movement_scene("unrestricted", false).await;
        place_region(
            &h,
            0x8A1,
            (1, 0),
            "terrain",
            choice_triggers(json!([
                plain_option("Decoy", 50.0, 650.0),
                { "label": "Target", "target": { "x": 450.0, "y": 650.0, "elevation": 3.0 } }
            ])),
        )
        .await;
        let (mut rx, _) = h.room.subscribe();
        move_token(&h, vec![h.start, h.adj]).await;
        let offer = offers(&mut rx).remove(0);
        (h, rx, offer)
    };
    h.room
        .handle_choose_portal(&h.repo, &h.player, offer.offer_id, Some(1), now())
        .await
        .unwrap();

    /// The ops of the command that writes the token's `/engine/x`, as JSON.
    async fn teleport_ops(h: &MovementHandle) -> serde_json::Value {
        let cmd = super::region_triggers::committed_events(h)
            .await
            .into_iter()
            .find(|e| {
                e.command.ops.iter().any(|op| {
                    matches!(op, Operation::Update { doc_id, changes }
                    if *doc_id == h.token_id && changes.iter().any(|c| c.path == "/engine/elevation"))
                })
            })
            .expect("the teleport committed");
        serde_json::to_value(&cmd.command.ops).unwrap()
    }
    assert_eq!(
        teleport_ops(&h).await,
        teleport_ops(&fixed).await,
        "a chosen hop and a fixed hop commit identical ops for the same target"
    );
    assert_eq!(token_xy(&h).await, (450.0, 650.0));
}

#[tokio::test]
async fn declining_commits_nothing_and_uses_the_offer_up() {
    let (h, _rx, offer) = offered().await;
    h.room
        .handle_choose_portal(&h.repo, &h.player, offer.offer_id, None, now())
        .await
        .unwrap();
    assert_eq!(token_xy(&h).await, h.adj);
    let again = h
        .room
        .handle_choose_portal(&h.repo, &h.player, offer.offer_id, Some(0), now())
        .await;
    assert!(
        matches!(again, Err(portal::PortalChoiceError::UnknownOffer)),
        "a declined offer cannot be answered again"
    );
}

/// Asserts `result` is a refusal whose wire text is the one uniform non-GM wording, and returns
/// that text.
fn uniform_refusal(result: Result<(), portal::PortalChoiceError>) -> String {
    let err = result.expect_err("the answer must be refused");
    portal::portal_choice_wire_message(&err, WorldRole::Player)
}

#[tokio::test]
async fn every_refusal_reads_the_same_to_a_player_and_differs_for_a_gm() {
    let (h, _rx, offer) = offered().await;
    let room = &h.room;
    let ttl_past = now() + portal::PORTAL_OFFER_TTL_MS + 10_000;

    // Unknown offer id.
    let unknown = uniform_refusal(
        room.handle_choose_portal(&h.repo, &h.player, Uuid::from_u128(1), Some(0), now())
            .await,
    );
    // A user who controls nothing: refused, and the offer stands for its owner.
    let wrong_user = uniform_refusal(
        room.handle_choose_portal(&h.repo, &stranger(), offer.offer_id, Some(0), now())
            .await,
    );
    // Index out of range (consumes the offer).
    let bad_index = uniform_refusal(
        room.handle_choose_portal(&h.repo, &h.player, offer.offer_id, Some(9), now())
            .await,
    );
    // A second answer to the consumed offer.
    let second = uniform_refusal(
        room.handle_choose_portal(&h.repo, &h.player, offer.offer_id, Some(0), now())
            .await,
    );
    assert_eq!(
        [&wrong_user, &bad_index, &second],
        [&unknown, &unknown, &unknown]
    );

    // An expired offer.
    let (mut rx2, _) = h.room.subscribe();
    h.room
        .handle_open_portal(&h.repo, &h.player, h.token_id, now())
        .await
        .unwrap();
    let fresh = offers(&mut rx2).remove(0);
    let expired = uniform_refusal(
        room.handle_choose_portal(&h.repo, &h.player, fresh.offer_id, Some(0), ttl_past)
            .await,
    );
    assert_eq!(expired, unknown);

    // A GM sees which refusal it was.
    let gm_text = portal::portal_choice_wire_message(
        &room
            .handle_choose_portal(&h.repo, &h.gm, Uuid::from_u128(1), Some(0), now())
            .await
            .unwrap_err(),
        WorldRole::Gm,
    );
    assert_ne!(gm_text, unknown, "a GM gets the specific diagnostic");
}

#[tokio::test]
async fn a_stranger_cannot_burn_the_offer_before_its_owner_answers() {
    let (h, _rx, offer) = offered().await;
    assert!(h
        .room
        .handle_choose_portal(&h.repo, &stranger(), offer.offer_id, None, now())
        .await
        .is_err());
    h.room
        .handle_choose_portal(&h.repo, &h.player, offer.offer_id, Some(0), now())
        .await
        .expect("the owner's answer still works");
    assert_eq!(token_xy(&h).await, (450.0, 450.0));
}

#[tokio::test]
async fn a_gm_may_choose_on_a_players_behalf() {
    let (h, _rx, offer) = offered().await;
    h.room
        .handle_choose_portal(&h.repo, &h.gm, offer.offer_id, Some(1), now())
        .await
        .unwrap();
    assert_eq!(token_xy(&h).await, (450.0, 650.0));
}

/// The GM drags the token from the portal's cell to (50, 250), two cells clear of the region, a
/// placement that updates its stored membership as a walk does.
async fn gm_place_out_of_the_region(h: &MovementHandle) {
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Update {
                doc_id: h.token_id,
                changes: vec![
                    FieldChange {
                        remove: false,
                        path: "/engine/x".into(),
                        old: json!(150.0),
                        new: json!(50.0),
                    },
                    FieldChange {
                        remove: false,
                        path: "/engine/y".into(),
                        old: json!(50.0),
                        new: json!(250.0),
                    },
                ],
            }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn an_offer_dies_when_the_token_is_moved_out_of_the_region() {
    let (h, _rx, offer) = offered().await;
    gm_place_out_of_the_region(&h).await;
    let result = h
        .room
        .handle_choose_portal(&h.repo, &h.player, offer.offer_id, Some(0), now())
        .await;
    assert!(matches!(result, Err(portal::PortalChoiceError::TokenMoved)));
    assert_eq!(token_xy(&h).await, (50.0, 250.0), "the token stays put");
}

#[tokio::test]
async fn a_new_offer_replaces_the_tokens_previous_one() {
    let (h, _rx, first) = offered().await;
    let (mut rx, _) = h.room.subscribe();
    h.room
        .handle_open_portal(&h.repo, &h.player, h.token_id, now())
        .await
        .unwrap();
    let second = offers(&mut rx).remove(0);
    assert_ne!(first.offer_id, second.offer_id);
    let stale = h
        .room
        .handle_choose_portal(&h.repo, &h.player, first.offer_id, Some(0), now())
        .await;
    assert!(matches!(
        stale,
        Err(portal::PortalChoiceError::UnknownOffer)
    ));
    h.room
        .handle_choose_portal(&h.repo, &h.player, second.offer_id, Some(0), now())
        .await
        .unwrap();
}

#[tokio::test]
async fn open_portal_re_offers_after_a_decline_and_refuses_a_stranger_or_a_token_outside_any_portal(
) {
    let (h, _rx, offer) = offered().await;
    h.room
        .handle_choose_portal(&h.repo, &h.player, offer.offer_id, None, now())
        .await
        .unwrap();

    let (mut rx, _) = h.room.subscribe();
    h.room
        .handle_open_portal(&h.repo, &h.player, h.token_id, now())
        .await
        .unwrap();
    let again = offers(&mut rx);
    assert_eq!(again.len(), 1, "a declined player can ask again");
    assert_eq!(again[0].options.len(), 2);

    assert!(h
        .room
        .handle_open_portal(&h.repo, &stranger(), h.token_id, now())
        .await
        .is_err());

    gm_place_out_of_the_region(&h).await;
    let outside = h
        .room
        .handle_open_portal(&h.repo, &h.player, h.token_id, now())
        .await;
    assert!(
        matches!(outside, Err(portal::PortalChoiceError::NoPortal)),
        "a token standing in no portal has nothing to open"
    );
}

#[tokio::test]
async fn a_secret_regions_offer_is_gm_only_and_a_player_cannot_open_it() {
    let h = movement_scene("unrestricted", false).await;
    place_secret_region(
        &h,
        0x8A2,
        (1, 0),
        choice_triggers(json!([plain_option("Hidden stair", 450.0, 450.0)])),
    )
    .await;
    let (mut rx, _) = h.room.subscribe();
    move_token(&h, vec![h.start, h.adj]).await;
    let offer = offers(&mut rx).remove(0);
    assert!(offer.gm_only, "a secret portal addresses GMs only");

    let player_open = uniform_refusal(
        h.room
            .handle_open_portal(&h.repo, &h.player, h.token_id, now())
            .await,
    );
    let absent_open = uniform_refusal(
        h.room
            .handle_open_portal(&h.repo, &stranger(), Uuid::from_u128(7), now())
            .await,
    );
    assert_eq!(
        player_open, absent_open,
        "a secret portal is indistinguishable from none"
    );
    h.room
        .handle_open_portal(&h.repo, &h.gm, h.token_id, now())
        .await
        .expect("a GM can open it");
}

#[tokio::test]
async fn the_scene_name_is_sent_only_when_the_option_reveals_it() {
    let h = movement_scene("unrestricted", false).await;
    let dest = place_named_scene(&h, 0x8B0, "The Vault").await;
    let gone = crate::test_ids::test_uuid(0x8B1);
    place_region(
        &h,
        0x8A3,
        (1, 0),
        "terrain",
        choice_triggers(json!([
            { "label": "Revealed", "reveal_scene": true,
              "target": { "scene": dest.to_string(), "x": 250.0, "y": 250.0 } },
            { "label": "Concealed",
              "target": { "scene": dest.to_string(), "x": 250.0, "y": 250.0 } },
            { "label": "Missing", "reveal_scene": true,
              "target": { "scene": gone.to_string(), "x": 250.0, "y": 250.0 } }
        ])),
    )
    .await;
    let (mut rx, _) = h.room.subscribe();
    move_token(&h, vec![h.start, h.adj]).await;
    let offer = offers(&mut rx).remove(0);
    let names: Vec<_> = offer
        .options
        .iter()
        .map(|o| (o.destination_name.as_deref(), o.available))
        .collect();
    assert_eq!(
        names,
        [(Some("The Vault"), true), (None, true), (None, false)]
    );
    let wire = serde_json::to_string(&offer.options).unwrap();
    assert!(
        !wire.contains("250"),
        "coordinates never travel on the frame"
    );
}

#[tokio::test]
async fn choosing_a_missing_destination_notices_the_gm_and_moves_nothing() {
    let h = movement_scene("unrestricted", false).await;
    let gone = crate::test_ids::test_uuid(0x8B2);
    place_region(
        &h,
        0x8A4,
        (1, 0),
        "terrain",
        choice_triggers(json!([
            { "label": "Gone", "target": { "scene": gone.to_string(), "x": 250.0, "y": 250.0 } }
        ])),
    )
    .await;
    let (mut rx, _) = h.room.subscribe();
    move_token(&h, vec![h.start, h.adj]).await;
    let offer = offers(&mut rx).remove(0);
    h.room
        .handle_choose_portal(&h.repo, &h.player, offer.offer_id, Some(0), now())
        .await
        .unwrap();
    assert_eq!(token_xy(&h).await, h.adj);
    let texts: Vec<String> = region_notices(&h).await.iter().map(notice_text).collect();
    assert!(
        texts.iter().any(|t| t.contains("does not exist")),
        "the refused hop surfaces as a GM notice, got {texts:?}"
    );
}

#[tokio::test]
async fn a_chosen_destination_that_offers_another_portal_is_refused_as_chained() {
    let h = movement_scene("unrestricted", false).await;
    place_region(
        &h,
        0x8A5,
        (1, 0),
        "terrain",
        choice_triggers(json!([plain_option("Onward", 250.0, 50.0)])),
    )
    .await;
    place_region(
        &h,
        0x8A6,
        (2, 0),
        "terrain",
        choice_triggers(json!([plain_option("Beyond", 450.0, 50.0)])),
    )
    .await;
    let (mut rx, _) = h.room.subscribe();
    move_token(&h, vec![h.start, h.adj]).await;
    let offer = offers(&mut rx).remove(0);
    h.room
        .handle_choose_portal(&h.repo, &h.player, offer.offer_id, Some(0), now())
        .await
        .unwrap();
    assert_eq!(token_xy(&h).await, (250.0, 50.0));
    assert!(
        offers(&mut rx).is_empty(),
        "the destination's portal does not offer: one hop per move"
    );
    let texts: Vec<String> = region_notices(&h).await.iter().map(notice_text).collect();
    assert!(texts.iter().any(|t| t.contains("chained portal refused")));
}

/// Moves the player token one cell down from where a teleport left it, through the real
/// `execute_move` moving-lock check.
async fn move_from_teleport_target(
    h: &MovementHandle,
) -> Result<crate::ws::room::MoveExecution, DataError> {
    h.room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path: vec![(450.0, 450.0), (450.0, 550.0)],
                ts: now(),
                request_id: Uuid::nil(),
            },
        )
        .await
}

/// Fails when a fixed teleport trigger leaves the walk's moving lock in place. The 0.001
/// cells/sec animation speed makes the lock window far longer than the test.
#[tokio::test]
async fn a_move_right_after_a_fixed_teleport_is_accepted() {
    let h = movement_scene_with_speed("unrestricted", false, 0.001).await;
    place_region(
        &h,
        0x8B0,
        (1, 0),
        "terrain",
        json!([{ "on": "enter", "effect": { "type": "teleport",
            "target": { "scene": null, "x": 450.0, "y": 450.0, "elevation": null, "vfx": null } } }]),
    )
    .await;
    move_token(&h, vec![h.start, h.adj]).await;
    assert_eq!(token_xy(&h).await, (450.0, 450.0));
    move_from_teleport_target(&h)
        .await
        .expect("the teleport ended the token's active stream");
    assert_eq!(token_xy(&h).await, (450.0, 550.0));
}

/// Fails when answering a portal offer leaves the walk's moving lock in place.
#[tokio::test]
async fn a_move_right_after_a_chosen_portal_hop_is_accepted() {
    let h = movement_scene_with_speed("unrestricted", false, 0.001).await;
    place_region(
        &h,
        0x8B1,
        (1, 0),
        "terrain",
        choice_triggers(json!([plain_option("North room", 450.0, 450.0)])),
    )
    .await;
    let (mut rx, _) = h.room.subscribe();
    move_token(&h, vec![h.start, h.adj]).await;
    let offer = offers(&mut rx).remove(0);
    h.room
        .handle_choose_portal(&h.repo, &h.player, offer.offer_id, Some(0), now())
        .await
        .unwrap();
    assert_eq!(token_xy(&h).await, (450.0, 450.0));
    move_from_teleport_target(&h)
        .await
        .expect("the chosen hop ended the token's active stream");
}
