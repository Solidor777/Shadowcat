//! `ServerMsg::Evicted` delivery: per-user targeting and terminal close.
mod common;

use common::drain_until_type;
use futures_util::StreamExt;
use shadowcat::ws::protocol::ServerMsg;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

#[tokio::test]
async fn evicted_frame_targets_and_closes() {
    let h = common::spawn().await;
    let mut ws = h.connect().await;
    drain_until_type(&mut ws, "welcome").await;

    let room = h.ws.rooms.get(h.world).expect("room exists after join");

    // Targeted at a DIFFERENT user: must not be delivered and must not close
    // this connection.
    room.broadcast_aux(ServerMsg::Evicted {
        user: Some(Uuid::new_v4()),
    });
    // Targeted at nobody (world deletion): delivered to everyone, then closed.
    room.broadcast_aux(ServerMsg::Evicted { user: None });

    // The next evicted frame this connection sees must be the world-wide one —
    // a delivered targeted-at-other frame would surface here with a non-null
    // user and fail the assert.
    let evicted = drain_until_type(&mut ws, "evicted").await;
    assert!(
        evicted["user"].is_null(),
        "targeted frame leaked to a non-target: {evicted}"
    );

    // The connection terminates right after: a Close frame or stream end.
    let next = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
        .await
        .expect("socket should close promptly after the evicted frame");
    match next {
        None | Some(Ok(Message::Close(_))) | Some(Err(_)) => {}
        Some(Ok(other)) => panic!("expected close after evicted frame, got {other:?}"),
    }
}
