use super::*;

/// A room over an in-memory repository, for the presence counter tests.
async fn room() -> Arc<Room> {
    let (repo, world, _ctx) = repo_with_world().await;
    RoomRegistry::new()
        .get_or_create(&repo, world)
        .await
        .unwrap()
        .unwrap()
}

/// Only the 0->1 connect reports a transition; a second concurrent connect does not. Fails if
/// `presence_connect` reports `true` on every call (re-announcing an already-online user).
#[tokio::test]
async fn presence_connect_reports_only_the_first_connection() {
    let room = room().await;
    let user = Uuid::new_v4();
    assert!(room.presence_connect(user));
    assert!(!room.presence_connect(user));
    assert!(!room.presence_connect(user));
}

/// Only the disconnect that reaches zero reports a transition. Fails if `presence_disconnect`
/// reports `true` while other connections remain.
#[tokio::test]
async fn presence_disconnect_reports_only_the_last_connection() {
    let room = room().await;
    let user = Uuid::new_v4();
    room.presence_connect(user);
    room.presence_connect(user);
    assert!(!room.presence_disconnect(user));
    assert!(room.presence_disconnect(user));
}

/// A disconnect with no prior connect is a no-op and a later connect still counts as the first.
/// Fails if the count underflows or an entry lingers after reaching zero.
#[tokio::test]
async fn presence_disconnect_without_connect_does_not_underflow() {
    let room = room().await;
    let user = Uuid::new_v4();
    assert!(!room.presence_disconnect(user));
    assert!(room.presence_connect(user));
    assert!(room.presence_disconnect(user));
    assert!(room.presence_connect(user));
}

/// Counts are per user: one user's connections never mask another's transitions.
#[tokio::test]
async fn presence_counts_are_per_user() {
    let room = room().await;
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    assert!(room.presence_connect(a));
    assert!(room.presence_connect(b));
    assert!(room.presence_disconnect(a));
    assert!(room.presence_disconnect(b));
}
