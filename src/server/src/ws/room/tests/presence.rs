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

/// The online set is exactly the users with a live connection: a second connection adds no
/// entry, the last disconnect removes it. Fails if `presence_online` kept a separate list that
/// the connect/disconnect transitions do not maintain.
#[tokio::test]
async fn presence_online_tracks_the_connection_counts() {
    let room = room().await;
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    assert!(room.presence_online().is_empty());
    room.presence_connect(a);
    room.presence_connect(a);
    room.presence_connect(b);
    let mut expected = vec![a, b];
    expected.sort_unstable();
    assert_eq!(room.presence_online(), expected);
    room.presence_disconnect(a);
    assert_eq!(room.presence_online().len(), 2);
    room.presence_disconnect(a);
    assert_eq!(room.presence_online(), vec![b]);
}

/// A profile broadcast reaches the live room of a world the user is seated in, carrying the
/// stored values, and reaches no other room. Fails if the broadcast ignored seating (sending
/// to every live room) or sent stale values.
#[tokio::test]
async fn broadcast_member_profile_goes_only_to_rooms_the_user_is_seated_in() {
    let (repo, world, ctx) = repo_with_world().await;
    let registry = RoomRegistry::new();
    let seated = registry.get_or_create(&repo, world).await.unwrap().unwrap();
    let stranger = repo
        .create_user("stranger", None, crate::auth::role::ServerRole::User, 0)
        .await
        .unwrap();
    let other_world = repo.create_world_owned("other", stranger, 0).await.unwrap();
    let other = registry
        .get_or_create(&repo, other_world.id)
        .await
        .unwrap()
        .unwrap();
    let (mut seated_rx, _) = seated.subscribe();
    let (mut other_rx, _) = other.subscribe();

    repo.set_profile(ctx.user_id, Some("#336699"), Some("they/them"))
        .await
        .unwrap();
    registry
        .broadcast_member_profile(&repo, ctx.user_id)
        .await
        .unwrap();

    let RoomEvent::Other(frame) = seated_rx.recv().await.unwrap() else {
        panic!("expected an out-of-band frame");
    };
    assert!(matches!(
        frame.as_ref(),
        ServerMsg::MemberProfile { user, colour, pronouns, has_avatar }
            if *user == ctx.user_id
                && colour.as_deref() == Some("#336699")
                && pronouns.as_deref() == Some("they/them")
                && !*has_avatar
    ));
    assert!(other_rx.try_recv().is_err());
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
