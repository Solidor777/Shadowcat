//! The whole-session pause as `Room` and the `ws::session_pause` refusals see it: a paused
//! session refuses a non-GM's token move and combat controls, leaves everything else alone,
//! never refuses a GM, and only a GM can flip it.

use super::movement_budget::{budget_scene, BudgetHandle};
use super::*;
use crate::data::command::FieldChange;
use crate::ws::protocol::{ClientMsg, RejectReason};
use crate::ws::session_pause::{intent_refusal, msg_refusal};
use serde_json::json;

/// The `world-settings` document id `movement_scene` publishes.
fn world_settings_id() -> Uuid {
    crate::test_ids::test_uuid(0x5CE2)
}

/// Commits one `Update` of `path` (`old` -> `new`) on `doc_id` as `who`.
async fn update_as(
    h: &BudgetHandle,
    who: &PermissionContext,
    doc_id: Uuid,
    path: &str,
    old: serde_json::Value,
    new: serde_json::Value,
) -> Result<(), DataError> {
    h.room
        .publish(
            &h.repo,
            who,
            vec![Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: path.into(),
                    old,
                    new,
                }],
            }],
            0,
            WriteOrigin::Client,
        )
        .await
        .map(|_| ())
}

/// The GM sets `/engine/session` from `old` to `new`.
async fn set_session(h: &BudgetHandle, old: serde_json::Value, new: serde_json::Value) {
    update_as(h, &h.gm, world_settings_id(), "/engine/session", old, new)
        .await
        .unwrap();
}

/// The GM pauses the session.
async fn pause(h: &BudgetHandle) {
    set_session(
        h,
        json!(null),
        json!({ "paused": true, "message": "back soon" }),
    )
    .await;
}

/// A move request for the player's own token.
fn move_request(h: &BudgetHandle) -> ClientMsg {
    ClientMsg::MoveRequest {
        request_id: Uuid::from_u128(1),
        scene: h.scene_id,
        token_id: h.token_id,
        path: vec![[h.start.0, h.start.1], [h.adj.0, h.adj.1]],
    }
}

/// An advance of the harness combat's turn.
fn combat_advance() -> ClientMsg {
    ClientMsg::CombatAdvance {
        request_id: Uuid::from_u128(2),
        combat_id: crate::test_ids::test_uuid(0x5CE4),
    }
}

/// A move-position `Update` of the player's token to `x`.
fn token_x_update(h: &BudgetHandle, x: f64) -> Operation {
    Operation::Update {
        doc_id: h.token_id,
        changes: vec![FieldChange {
            remove: false,
            path: "/engine/x".into(),
            old: json!(50.0),
            new: json!(x),
        }],
    }
}

#[tokio::test]
async fn a_paused_session_refuses_a_players_move_and_combat_controls() {
    let h = budget_scene("hard", "spaces", None).await;
    // Control: an unpaused session refuses nothing.
    assert!(msg_refusal(&h.room, &h.player, &move_request(&h))
        .await
        .is_none());
    assert!(msg_refusal(&h.room, &h.player, &combat_advance())
        .await
        .is_none());

    pause(&h).await;
    let mv = msg_refusal(&h.room, &h.player, &move_request(&h)).await;
    assert!(matches!(
        mv.as_deref(),
        Some(ServerMsg::MoveError {
            reason: Some(RejectReason::Paused),
            ..
        })
    ));
    let combat = msg_refusal(&h.room, &h.player, &combat_advance()).await;
    assert!(matches!(
        combat.as_deref(),
        Some(ServerMsg::CombatError {
            reason: Some(RejectReason::Paused),
            ..
        })
    ));
}

#[tokio::test]
async fn a_paused_session_never_refuses_a_gm() {
    let h = budget_scene("hard", "spaces", None).await;
    pause(&h).await;
    assert!(msg_refusal(&h.room, &h.gm, &move_request(&h))
        .await
        .is_none());
    assert!(msg_refusal(&h.room, &h.gm, &combat_advance())
        .await
        .is_none());
    let ops = vec![token_x_update(&h, 150.0)];
    assert!(intent_refusal(&h.room, &h.repo, &h.gm, Uuid::nil(), &ops)
        .await
        .is_none());
}

#[tokio::test]
async fn resuming_lifts_every_refusal() {
    let h = budget_scene("hard", "spaces", None).await;
    pause(&h).await;
    assert!(msg_refusal(&h.room, &h.player, &move_request(&h))
        .await
        .is_some());
    set_session(
        &h,
        json!({ "paused": true, "message": "back soon" }),
        json!({ "paused": false }),
    )
    .await;
    assert!(msg_refusal(&h.room, &h.player, &move_request(&h))
        .await
        .is_none());
    let ops = vec![token_x_update(&h, 150.0)];
    assert!(
        intent_refusal(&h.room, &h.repo, &h.player, Uuid::nil(), &ops)
            .await
            .is_none()
    );
}

#[tokio::test]
async fn a_paused_session_names_a_players_token_position_write_and_not_its_other_fields() {
    let h = budget_scene("hard", "spaces", None).await;
    pause(&h).await;

    let moved = intent_refusal(
        &h.room,
        &h.repo,
        &h.player,
        Uuid::from_u128(9),
        &[token_x_update(&h, 150.0)],
    )
    .await;
    assert!(matches!(
        moved.as_deref(),
        Some(ServerMsg::Reject { reason: RejectReason::Paused, intent_id, .. })
            if *intent_id == Uuid::from_u128(9)
    ));

    // Re-stating the committed position is not a move.
    let same = intent_refusal(
        &h.room,
        &h.repo,
        &h.player,
        Uuid::nil(),
        &[token_x_update(&h, 50.0)],
    )
    .await;
    assert!(same.is_none());

    // Another token field is untouched by the pause.
    let elevation = Operation::Update {
        doc_id: h.token_id,
        changes: vec![FieldChange {
            remove: false,
            path: "/engine/elevation".into(),
            old: json!(null),
            new: json!(5.0),
        }],
    };
    assert!(
        intent_refusal(&h.room, &h.repo, &h.player, Uuid::nil(), &[elevation])
            .await
            .is_none()
    );
}

#[tokio::test]
async fn a_paused_session_refuses_a_write_to_a_combat_document() {
    let h = budget_scene("hard", "spaces", None).await;
    let combatant = Operation::Update {
        doc_id: h.combatant_id,
        changes: vec![FieldChange {
            remove: false,
            path: "/engine/resources/movement/current".into(),
            old: json!(10.0),
            new: json!(9.0),
        }],
    };
    assert!(intent_refusal(
        &h.room,
        &h.repo,
        &h.player,
        Uuid::nil(),
        &[combatant.clone()]
    )
    .await
    .is_none());
    pause(&h).await;
    assert!(matches!(
        intent_refusal(&h.room, &h.repo, &h.player, Uuid::nil(), &[combatant])
            .await
            .as_deref(),
        Some(ServerMsg::Reject {
            reason: RejectReason::Paused,
            ..
        })
    ));
}

#[tokio::test]
async fn a_paused_session_leaves_chat_doors_scene_edits_and_sheets_alone() {
    let h = budget_scene("hard", "spaces", None).await;
    pause(&h).await;

    // The typed requests the ruling keeps open.
    let door = ClientMsg::ToggleDoor {
        request_id: Uuid::nil(),
        scene_id: h.scene_id,
        wall_id: Uuid::nil(),
        to: crate::data::engine::DoorState::Open,
    };
    let chat = ClientMsg::SendMessage {
        request_id: Uuid::nil(),
        channel: "main".into(),
        content: "hello".into(),
        actor_owner: None,
        audience: Default::default(),
        system: None,
    };
    assert!(msg_refusal(&h.room, &h.player, &door).await.is_none());
    assert!(msg_refusal(&h.room, &h.player, &chat).await.is_none());

    // A scene edit and a sheet (actor) write are not frozen document types.
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let scene_edit = Operation::Create {
        doc: wdoc(h.world_id, Uuid::new_v4(), "wall"),
    };
    let sheet = Operation::Create {
        doc: wdoc(h.world_id, Uuid::new_v4(), "actor"),
    };
    assert!(intent_refusal(
        &h.room,
        &h.repo,
        &h.player,
        Uuid::nil(),
        &[scene_edit, sheet]
    )
    .await
    .is_none());
}

#[tokio::test]
async fn a_player_cannot_write_the_session_switch() {
    let h = budget_scene("hard", "spaces", None).await;
    let refused = update_as(
        &h,
        &h.player,
        world_settings_id(),
        "/engine/session",
        json!(null),
        json!({ "paused": true }),
    )
    .await;
    assert!(matches!(refused, Err(DataError::Forbidden)), "{refused:?}");
    assert!(!h.room.session_state().await.paused);
}

#[tokio::test]
async fn a_room_opened_after_the_pause_starts_paused() {
    let h = budget_scene("hard", "spaces", None).await;
    pause(&h).await;
    let reg = RoomRegistry::new();
    let late = reg
        .get_or_create(&h.repo, h.world_id)
        .await
        .unwrap()
        .unwrap();
    let state = late.session_state().await;
    assert!(state.paused);
    assert_eq!(state.message.as_deref(), Some("back soon"));
    assert!(late.session_refuses(&h.player).await);
}

#[tokio::test]
async fn an_oversized_or_control_character_message_is_refused_at_ingress() {
    let h = budget_scene("hard", "spaces", None).await;
    for message in ["x".repeat(281), "line\nbreak".to_string()] {
        let refused = update_as(
            &h,
            &h.gm,
            world_settings_id(),
            "/engine/session",
            json!(null),
            json!({ "paused": true, "message": message }),
        )
        .await;
        assert!(refused.is_err(), "{message:?} must be refused");
    }
    assert!(!h.room.session_state().await.paused);
}
