//! Creature senses in the per-recipient `MoveStream` clip (`clip_move_stream` →
//! `InstantSight::sees_token`): a tremorsense observer is streamed a grounded token it cannot
//! see, through the SAME reach decision `player_perceived_tokens` makes for that token at rest.

use super::*;
use crate::ws::protocol::PosSample;

/// The observer's-side sight wall at x=100 (`setup_clip_room`'s wall body).
fn wall() -> serde_json::Value {
    json!({ "seg": { "x1": 100, "y1": -500, "x2": 100, "y2": 500 }, "blocksSight": true })
}

/// The stranger's token document the walk below moves, readable by every player.
const STRANGER_TOKEN: Uuid = Uuid::from_u128(0x5E);

/// Publishes the stranger's token at (250,50) — behind the wall — at `elevation`.
async fn publish_stranger_token(
    room: &crate::ws::room::Room,
    repo: &SqliteRepository,
    gm_ctx: &PermissionContext,
    scene: Uuid,
    elevation: Option<f64>,
) {
    use crate::data::command::Operation;
    use crate::data::document::DocRole;
    let mut tok =
        crate::data::document::tests::world_scoped_doc(room.world_id, STRANGER_TOKEN, "token");
    tok.parent_id = Some(scene);
    tok.owner = Some(gm_ctx.user_id);
    tok.permissions.default = DocRole::Observer;
    let mut engine = token_engine(250.0, 50.0);
    if let Some(e) = elevation {
        engine["elevation"] = json!(e);
    }
    tok.engine = Some(engine);
    room.publish(
        repo,
        gm_ctx,
        vec![Operation::Create { doc: tok }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
}

/// The stranger's unlit two-sample walk behind the wall, moving `STRANGER_TOKEN`.
fn dark_walk(scene: Uuid, now: i64) -> ServerMsg {
    ServerMsg::MoveStream {
        request_id: Uuid::from_u128(0x5E1),
        token_id: STRANGER_TOKEN,
        mover: Uuid::from_u128(0xAABB),
        scene,
        start_server_ms: now as f64,
        duration_ms: 1_000.0,
        stop: [350.0, 50.0],
        samples: vec![
            PosSample {
                t_ms: 0.0,
                pos: [250.0, 50.0],
            },
            PosSample {
                t_ms: 1_000.0,
                pos: [350.0, 50.0],
            },
        ],
        mover_vision: None,
        mover_light: None,
        cost: Some(1.0),
        truncated: Some(false),
    }
}

/// The observer's clip of `frame`: the position sample count, or `None` when suppressed.
async fn clipped_count(
    frame: &ServerMsg,
    ctx: &PermissionContext,
    room: &crate::ws::room::Room,
) -> Option<usize> {
    let out = clip_move_stream(
        frame,
        ctx,
        None,
        room,
        &crate::data::document::WorldCapDefaults::default(),
    )
    .await?;
    let ServerMsg::MoveStream { samples, .. } = out else {
        panic!("clip returns a MoveStream")
    };
    Some(samples.len())
}

#[tokio::test]
async fn a_tremorsense_observer_is_streamed_a_grounded_token_it_cannot_see() {
    // Normal vision: the walk is behind the wall and unlit — suppressed.
    let (room, gm_ctx, obs_ctx, scene_id, repo) =
        setup_dark_clip_room(Some((50.0, 50.0)), Some(wall())).await;
    publish_stranger_token(&room, repo.as_ref(), &gm_ctx, scene_id, None).await;
    let now = crate::ws::time::now_millis();
    assert!(clipped_count(&dark_walk(scene_id, now), &obs_ctx, &room)
        .await
        .is_none());

    // Tremorsense within its 12-cell default range: the whole walk, walls and darkness
    // notwithstanding.
    let (room, gm_ctx, obs_ctx, scene_id, repo) = setup_dark_clip_room_with_vision(
        (50.0, 50.0),
        Some(wall()),
        json!([{ "mode": "tremorsense" }]),
    )
    .await;
    publish_stranger_token(&room, repo.as_ref(), &gm_ctx, scene_id, None).await;
    assert_eq!(
        clipped_count(&dark_walk(scene_id, now), &obs_ctx, &room).await,
        Some(2)
    );

    // A flying stranger is not felt through the ground.
    let (room, gm_ctx, obs_ctx, scene_id, repo) = setup_dark_clip_room_with_vision(
        (50.0, 50.0),
        Some(wall()),
        json!([{ "mode": "tremorsense" }]),
    )
    .await;
    publish_stranger_token(&room, repo.as_ref(), &gm_ctx, scene_id, Some(5.0)).await;
    assert!(clipped_count(&dark_walk(scene_id, now), &obs_ctx, &room)
        .await
        .is_none());
}
