//! `ServerMsg::Presence` delivery: broadcast only on a member's 0->1 and 1->0
//! connection-count transitions.
use shadowcat_test_support as common;

use common::drain_until_type;
use futures_util::StreamExt;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

/// Reads frames from `ws` for `window` and panics if any is a `presence` frame.
async fn assert_no_presence<S>(ws: &mut S, window: Duration)
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let deadline = tokio::time::Instant::now() + window;
    while let Ok(Some(Ok(frame))) = tokio::time::timeout_at(deadline, ws.next()).await {
        if let Message::Text(text) = frame {
            let v: serde_json::Value = serde_json::from_str(text.as_str()).unwrap();
            assert_ne!(v["type"], "presence", "unexpected presence frame: {v}");
        }
    }
}

/// A first connection announces online to an already-connected member, a second
/// connection of the same user announces nothing, closing one of two announces nothing, and
/// closing the last announces offline.
#[tokio::test]
async fn presence_broadcasts_only_on_connection_count_transitions() {
    let h = common::spawn().await;
    let (player, cookie) = h.add_player("presence-player").await;

    let mut gm = h.connect().await;
    drain_until_type(&mut gm, "welcome").await;
    let gm_own = drain_until_type(&mut gm, "presence").await;
    assert_eq!(gm_own["online"], true);

    let mut first = h.connect_as(&cookie).await;
    let online = drain_until_type(&mut gm, "presence").await;
    assert_eq!(online["user"].as_str(), Some(player.to_string().as_str()));
    assert_eq!(online["online"], true);

    // The opening connection receives its own online frame.
    let own = drain_until_type(&mut first, "presence").await;
    assert_eq!(own["user"].as_str(), Some(player.to_string().as_str()));

    let mut second = h.connect_as(&cookie).await;
    drain_until_type(&mut second, "welcome").await;
    assert_no_presence(&mut gm, Duration::from_millis(500)).await;

    first.close(None).await.unwrap();
    drop(first);
    assert_no_presence(&mut gm, Duration::from_millis(500)).await;

    second.close(None).await.unwrap();
    drop(second);
    let offline = drain_until_type(&mut gm, "presence").await;
    assert_eq!(offline["user"].as_str(), Some(player.to_string().as_str()));
    assert_eq!(offline["online"], false);
}
