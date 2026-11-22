//! Tests for `handle_revise_audience`, `permissions_for_audience` and the
//! per-recipient broadcast a revision produces.

use super::*;
use crate::auth::role::ServerRole;
use crate::data::command::Operation;
use crate::data::document::{DocRole, WorldCapDefaults, WorldRole};
use crate::data::membership::PermissionContext;
use crate::data::permission::{filter_command, load_current_docs};
use crate::data::snapshot::StoredCommand;
use crate::data::sqlite::SqliteRepository;
use crate::ws::room::{Room, RoomEvent, RoomRegistry};
use crate::ws::PingRateLimiter;
use std::sync::Arc;

/// A world with a GM, a sender player, a second player and a bystander.
pub(super) struct Harness {
    /// The backing repository.
    pub(super) repo: SqliteRepository,
    /// The world's room.
    pub(super) room: Arc<Room>,
    /// The GM's context.
    pub(super) gm: PermissionContext,
    /// The message author's context.
    pub(super) sender: PermissionContext,
    /// A player a whisper can name.
    pub(super) friend: PermissionContext,
    /// A player no audience names.
    pub(super) bystander: PermissionContext,
}

/// Builds the four-user world.
pub(super) async fn harness() -> Harness {
    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    crate::data::world_seed::seed_test_channel_registry(&repo, w.id, &[]).await;
    let mut players = Vec::new();
    for name in ["sender", "friend", "bystander"] {
        let id = repo
            .create_user(name, None, ServerRole::User, 0)
            .await
            .unwrap();
        repo.add_member(w.id, id, WorldRole::Player).await.unwrap();
        players.push(PermissionContext {
            user_id: id,
            world_role: WorldRole::Player,
        });
    }
    let room = RoomRegistry::new()
        .get_or_create(&repo, w.id)
        .await
        .unwrap()
        .unwrap();
    let bystander = players.pop().unwrap();
    let friend = players.pop().unwrap();
    let sender = players.pop().unwrap();
    Harness {
        repo,
        room,
        gm: PermissionContext {
            user_id: gm,
            world_role: WorldRole::Gm,
        },
        sender,
        friend,
        bystander,
    }
}

/// Sends `content` as `who` to `audience` and returns the stored message id.
pub(super) async fn send(
    h: &Harness,
    who: &PermissionContext,
    content: &str,
    audience: Audience,
) -> Uuid {
    let rate = PingRateLimiter::new();
    let (cmd, _) = handle_send_message(
        MessageRequestCtx {
            room: &h.room,
            repo: &h.repo,
            ctx: who,
            rate: &rate,
            vfx_rate: &rate,
            preview: LinkPreviewDeps {
                client: &link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "general".into(),
        content.into(),
        None,
        audience,
        None,
    )
    .await
    .unwrap()
    .unwrap();
    match &cmd.ops[0] {
        Operation::Create { doc } => doc.id,
        other => panic!("expected Create, got {other:?}"),
    }
}

/// The request context for a revision by `who`.
fn req<'a>(
    h: &'a Harness,
    who: &'a PermissionContext,
    rate: &'a PingRateLimiter,
) -> RecalcRollRequestCtx<'a> {
    RecalcRollRequestCtx {
        room: &h.room,
        repo: &h.repo,
        ctx: who,
        rate,
        now: 200,
        budget_per_min: 30,
    }
}

/// Receives the next committed command off the room's broadcast.
pub(super) async fn next_event(
    rx: &mut tokio::sync::broadcast::Receiver<RoomEvent>,
) -> StoredCommand {
    loop {
        match rx.recv().await.unwrap() {
            RoomEvent::Event(ev) => return (*ev).clone(),
            RoomEvent::Other(_) => continue,
        }
    }
}

/// What `who` receives on the wire for `ev`: the per-recipient filtered ops.
pub(super) async fn wire_ops(
    h: &Harness,
    who: &PermissionContext,
    ev: &StoredCommand,
) -> Vec<Operation> {
    let current = load_current_docs(&h.repo, &ev.command).await;
    filter_command(
        &ev.command,
        &ev.snapshot,
        who,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    )
    .ops
}

/// True when `ops` deliver the message's content to the recipient (a Create or
/// an Update that carries the engine body).
fn delivers_content(ops: &[Operation], id: Uuid) -> bool {
    ops.iter().any(|op| match op {
        Operation::Create { doc } => doc.id == id && doc.engine.is_some(),
        Operation::Update { doc_id, .. } => *doc_id == id,
        _ => false,
    })
}

#[test]
fn permissions_for_audience_matches_each_audience_and_keeps_the_author_owner() {
    let author = Uuid::new_v4();
    let other = Uuid::new_v4();
    let none = BlindReaders::default();
    let (d, g, u) = permissions_for_audience(author, &Audience::Public, &none);
    assert_eq!((d, g), (DocRole::Observer, None));
    assert_eq!(u.get(&author), Some(&DocRole::Owner));

    let (d, g, u) = permissions_for_audience(
        author,
        &Audience::Whisper {
            recipients: vec![other, author],
        },
        &none,
    );
    assert_eq!((d, g), (DocRole::None, Some(DocRole::None)));
    assert_eq!(u.get(&other), Some(&DocRole::Observer));
    assert_eq!(
        u.get(&author),
        Some(&DocRole::Owner),
        "naming the author as a recipient must not demote them"
    );

    let (d, g, u) = permissions_for_audience(author, &Audience::GmOnly, &none);
    assert_eq!((d, g), (DocRole::None, Some(DocRole::Observer)));
    assert_eq!(u.len(), 1);
}

#[tokio::test]
async fn reveal_delivers_the_message_to_a_previously_excluded_player_in_the_same_event() {
    let h = harness().await;
    let id = send(&h, &h.gm, "secret", Audience::GmOnly).await;
    let (mut rx, _) = h.room.subscribe();
    let rate = PingRateLimiter::new();
    handle_revise_audience(req(&h, &h.gm, &rate), id, Audience::Public)
        .await
        .unwrap();
    let ev = next_event(&mut rx).await;

    let seen = wire_ops(&h, &h.bystander, &ev).await;
    let created = seen.iter().find_map(|op| match op {
        Operation::Create { doc } if doc.id == id => Some(doc),
        _ => None,
    });
    let doc = created.expect("the revealed message arrives as a Create in this very event");
    let engine: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
    assert_eq!(engine.audience, Audience::Public);

    let stored = h.repo.get_document(id).await.unwrap().unwrap();
    assert_eq!(stored.permissions.default, DocRole::Observer);
    assert_eq!(stored.permissions.gm_role, None);
}

#[tokio::test]
async fn conceal_hides_the_message_from_everyone_outside_the_whisper_on_the_wire() {
    let h = harness().await;
    let id = send(&h, &h.sender, "visible to all", Audience::Public).await;
    let (mut rx, _) = h.room.subscribe();
    let rate = PingRateLimiter::new();
    handle_revise_audience(
        req(&h, &h.gm, &rate),
        id,
        Audience::Whisper {
            recipients: vec![h.friend.user_id],
        },
    )
    .await
    .unwrap();
    let ev = next_event(&mut rx).await;

    let bystander_ops = wire_ops(&h, &h.bystander, &ev).await;
    assert!(
        !delivers_content(&bystander_ops, id),
        "the excluded player's copy of the event must not carry the message: {bystander_ops:?}"
    );
    for op in &bystander_ops {
        if let Operation::Delete { doc } = op {
            assert!(
                doc.engine.is_none(),
                "the stub Delete carries no engine body"
            );
        }
    }
    // The GM is excluded by `gm_role: Some(None)` the same as any non-recipient:
    // a whisper hides from the GM unless they are listed.
    assert!(!delivers_content(&wire_ops(&h, &h.gm, &ev).await, id));
    // The named recipient and the author still read it.
    assert!(
        delivers_content(&wire_ops(&h, &h.friend, &ev).await, id),
        "the named recipient keeps reading the message"
    );
    let sender_ops = wire_ops(&h, &h.sender, &ev).await;
    assert!(
        sender_ops
            .iter()
            .all(|op| !matches!(op, Operation::Delete { .. })),
        "the author never loses sight of their own message: {sender_ops:?}"
    );

    let stored = h.repo.get_document(id).await.unwrap().unwrap();
    assert_eq!(
        stored.permissions.users.get(&h.sender.user_id),
        Some(&DocRole::Owner),
        "a conceal can never demote the author below Owner"
    );
    assert_eq!(stored.owner, Some(h.sender.user_id));
}

#[tokio::test]
async fn a_conceal_to_gm_only_keeps_the_players_own_authorship() {
    let h = harness().await;
    let id = send(&h, &h.sender, "oops", Audience::Public).await;
    let rate = PingRateLimiter::new();
    handle_revise_audience(req(&h, &h.gm, &rate), id, Audience::GmOnly)
        .await
        .unwrap();
    let stored = h.repo.get_document(id).await.unwrap().unwrap();
    assert_eq!(stored.permissions.default, DocRole::None);
    assert_eq!(stored.permissions.gm_role, Some(DocRole::Observer));
    assert_eq!(
        stored.permissions.users.get(&h.sender.user_id),
        Some(&DocRole::Owner)
    );
}

#[tokio::test]
async fn only_a_gm_may_revise_not_even_the_messages_owner() {
    let h = harness().await;
    let id = send(&h, &h.sender, "mine", Audience::Public).await;
    let rate = PingRateLimiter::new();
    let err = handle_revise_audience(req(&h, &h.sender, &rate), id, Audience::GmOnly)
        .await
        .unwrap_err();
    assert!(matches!(err, SendMessageError::Forbidden));
    let stored = h.repo.get_document(id).await.unwrap().unwrap();
    assert_eq!(stored.permissions.default, DocRole::Observer);
}

#[tokio::test]
async fn revising_a_tombstoned_or_missing_or_non_message_target_is_not_found() {
    let h = harness().await;
    let id = send(&h, &h.sender, "bye", Audience::Public).await;
    let rate = PingRateLimiter::new();
    handle_delete_message(&h.room, &h.repo, &h.gm, &rate, id, 150, 30)
        .await
        .unwrap();
    let err = handle_revise_audience(req(&h, &h.gm, &rate), id, Audience::GmOnly)
        .await
        .unwrap_err();
    assert!(
        matches!(err, SendMessageError::NotFound),
        "tombstone: {err:?}"
    );

    let err = handle_revise_audience(req(&h, &h.gm, &rate), Uuid::new_v4(), Audience::GmOnly)
        .await
        .unwrap_err();
    assert!(
        matches!(err, SendMessageError::NotFound),
        "missing: {err:?}"
    );
}

#[tokio::test]
async fn a_whisper_to_a_non_member_is_refused_and_changes_nothing() {
    let h = harness().await;
    let id = send(&h, &h.sender, "hi", Audience::Public).await;
    let rate = PingRateLimiter::new();
    let err = handle_revise_audience(
        req(&h, &h.gm, &rate),
        id,
        Audience::Whisper {
            recipients: vec![Uuid::new_v4()],
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SendMessageError::UnknownRecipient));
    let stored = h.repo.get_document(id).await.unwrap().unwrap();
    assert_eq!(stored.permissions.default, DocRole::Observer);
}

#[tokio::test]
async fn a_revision_keeps_the_rolls_gm_only_property_overrides() {
    let h = harness().await;
    let id = send(&h, &h.gm, "/roll 1d6", Audience::Public).await;
    let before = h.repo.get_document(id).await.unwrap().unwrap();
    assert!(!before.permissions.property_overrides.is_empty());
    let rate = PingRateLimiter::new();
    handle_revise_audience(req(&h, &h.gm, &rate), id, Audience::GmOnly)
        .await
        .unwrap();
    let after = h.repo.get_document(id).await.unwrap().unwrap();
    assert_eq!(
        after.permissions.property_overrides,
        before.permissions.property_overrides
    );
}

#[tokio::test]
async fn a_world_requirement_on_an_unrelated_permissions_path_does_not_block_a_reveal() {
    use crate::data::document::CapabilityRequirement;
    let h = harness().await;
    let id = send(&h, &h.gm, "secret", Audience::GmOnly).await;
    let world_id = match h.repo.get_document(id).await.unwrap().unwrap().scope {
        crate::data::document::Scope::World { world_id } => world_id,
        _ => unreachable!(),
    };
    h.repo
        .set_world_cap_requirements(
            world_id,
            &[CapabilityRequirement {
                path_prefix: "/permissions/users".into(),
                caps: ["sys:gate".to_string()].into_iter().collect(),
            }],
        )
        .await
        .unwrap();
    let rate = PingRateLimiter::new();
    handle_revise_audience(req(&h, &h.gm, &rate), id, Audience::Public)
        .await
        .expect("the vetted GM reveal skips the additive declared requirement");
}
