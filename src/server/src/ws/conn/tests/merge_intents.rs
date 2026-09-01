//! `conn::merge_intents`'s wire-dispatch layer: the stateless two-call flow, the
//! owner-or-GM and per-path authorization derivations, push's per-instance
//! visibility/capability outcomes, and the `WriteOrigin::TemplateMerge` commit's
//! broadcast/redaction behaviour. Fixture pattern mirrors `combat_intents`'s
//! harness construction.

use super::*;
use crate::auth::role::ServerRole;
use crate::data::command::{FieldChange, Operation};
use crate::data::document::{DocRole, Document, Source, Visibility, WorldRole};
use crate::data::membership::PermissionContext;
use crate::data::permission::filter_command;
use crate::data::DataError;
use crate::merge::MergeBase;
use crate::ws::conn::merge_intents::{commit_error, handle_merge_intent};
use crate::ws::protocol::{
    MergeErrorKind, MergeOutcome, MergePullStatus, MergeRevertStatus, PushInstanceStatus,
};
use crate::ws::room::{Room, RoomRegistry};

/// A GM, a player, and an unrelated bystander in one world, with a live room.
struct Harness {
    /// The backing repository.
    repo: Arc<SqliteRepository>,
    /// The world's room.
    room: Arc<Room>,
    /// GM permission context.
    gm: PermissionContext,
    /// Player permission context.
    player: PermissionContext,
    /// Bystander permission context: a world member with no relationship to any
    /// fixture document.
    bystander: PermissionContext,
    /// The world the fixture lives in.
    world_id: Uuid,
    /// Cached `WorldCapDefaults`, for `filter_command`.
    world_defaults: crate::data::document::WorldCapDefaults,
}

impl Harness {
    /// The current stored document `id`.
    async fn get(&self, id: Uuid) -> Document {
        self.repo.get_document(id).await.unwrap().expect("document")
    }

    /// Create `doc` via a GM-authored `Operation::Create` through the one write
    /// path (which derives the instance's `base` at ingest).
    async fn create(&self, doc: Document) {
        self.room
            .publish(
                self.repo.as_ref(),
                &self.gm,
                vec![Operation::Create { doc }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();
    }

    /// Whole-band `/system` rewrite of `id` as the GM.
    async fn set_system(&self, id: Uuid, system: serde_json::Value) {
        let cur = self.get(id).await;
        self.room
            .publish(
                self.repo.as_ref(),
                &self.gm,
                vec![Operation::Update {
                    doc_id: id,
                    changes: vec![FieldChange {
                        path: "/system".into(),
                        old: cur.system.clone(),
                        new: system,
                        remove: false,
                    }],
                }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();
    }
}

/// Build the base harness: world + GM + player + bystander + room.
async fn merge_harness() -> Harness {
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let gm_id = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", gm_id, 0).await.unwrap();
    let player_id = repo
        .create_user("player", None, ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world.id, player_id, WorldRole::Player)
        .await
        .unwrap();
    let bystander_id = repo
        .create_user("bystander", None, ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world.id, bystander_id, WorldRole::Player)
        .await
        .unwrap();
    let reg = RoomRegistry::new();
    let room = reg
        .get_or_create(repo.as_ref(), world.id)
        .await
        .unwrap()
        .unwrap();
    let world_defaults = repo.world_cap_defaults(world.id).await.unwrap();
    Harness {
        repo,
        room,
        gm: PermissionContext {
            user_id: gm_id,
            world_role: WorldRole::Gm,
        },
        player: PermissionContext {
            user_id: player_id,
            world_role: WorldRole::Player,
        },
        bystander: PermissionContext {
            user_id: bystander_id,
            world_role: WorldRole::Player,
        },
        world_id: world.id,
        world_defaults,
    }
}

/// An `actor` template document: owned by `owner`, visible to the world at
/// `default`, with `system` as its game band.
fn template_doc(
    world: Uuid,
    id: Uuid,
    owner: Uuid,
    default: DocRole,
    system: serde_json::Value,
) -> Document {
    let mut d = crate::data::document::tests::world_scoped_doc(world, id, "actor");
    d.owner = Some(owner);
    d.permissions.default = default;
    d.system = system;
    d
}

/// An instance of `template_id`: `source` set, owned by `owner`, visible to the
/// world at `default`. The stored `base` is NOT set here — Create derives it.
fn instance_doc(
    world: Uuid,
    id: Uuid,
    template_id: Uuid,
    owner: Uuid,
    default: DocRole,
    system: serde_json::Value,
) -> Document {
    let mut d = crate::data::document::tests::world_scoped_doc(world, id, "actor");
    d.source = Some(Source {
        id: template_id,
        pack: None,
        version: 1,
    });
    d.owner = Some(owner);
    d.permissions.default = default;
    d.system = system;
    d
}

/// The player-pullable fixture: a template the player can read (default
/// Observer) and a player-owned instance of it, both with `system` {"hp": 10}.
/// Returns (template_id, child_id).
async fn player_pullable(h: &Harness, template_id: Uuid, child_id: Uuid) -> (Uuid, Uuid) {
    let mut template = template_doc(
        h.world_id,
        template_id,
        h.gm.user_id,
        DocRole::Observer,
        json!({ "hp": 10 }),
    );
    template.name = Some("Template".into());
    h.create(template).await;
    let mut child = instance_doc(
        h.world_id,
        child_id,
        template_id,
        h.player.user_id,
        DocRole::Observer,
        json!({ "hp": 10 }),
    );
    child.name = Some("Instance".into());
    child
        .permissions
        .users
        .insert(h.player.user_id, DocRole::Owner);
    h.create(child).await;
    (template_id, child_id)
}

/// Extract the single `Pull` outcome's status from a `MergeResult` reply.
fn pull_status(reply: ServerMsg) -> MergePullStatus {
    match reply {
        ServerMsg::MergeResult {
            outcome: MergeOutcome::Pull { status, .. },
            ..
        } => status,
        other => panic!("expected a MergeResult::Pull, got {other:?}"),
    }
}

/// Extract the `MergeErrorKind` from a `MergeError` reply.
fn error_reason(reply: ServerMsg) -> MergeErrorKind {
    match reply {
        ServerMsg::MergeError { reason, .. } => reason,
        other => panic!("expected a MergeError, got {other:?}"),
    }
}

/// A clean pull (only the template diverged) applies immediately: the child
/// takes the template's value and its `base` refreshes to the template's
/// current snapshot.
#[tokio::test]
async fn pull_clean_applies_and_refreshes_base() {
    let h = merge_harness().await;
    let (template, child) =
        player_pullable(&h, Uuid::from_u128(0xE101), Uuid::from_u128(0xE102)).await;
    h.set_system(template, json!({ "hp": 12 })).await;

    let reply = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.player,
        ClientMsg::MergePull {
            request_id: Uuid::from_u128(1),
            child_id: child,
            resolutions: None,
        },
        0,
    )
    .await
    .expect("a reply");
    assert!(
        matches!(pull_status(reply), MergePullStatus::Applied),
        "a conflict-free pull applies"
    );

    let after = h.get(child).await;
    assert_eq!(after.system, json!({ "hp": 12 }));
    let base: MergeBase =
        serde_json::from_value(after.base.clone().expect("base refreshed")).unwrap();
    assert_eq!(
        base.system,
        json!({ "hp": 12 }),
        "base is the template snapshot"
    );
}

/// A conflicted pull writes nothing and reports the conflict set; the
/// resolutions call recomputes, takes the template side of the resolved path,
/// and commits.
#[tokio::test]
async fn pull_conflicted_reports_then_resolutions_apply() {
    let h = merge_harness().await;
    let (template, child) =
        player_pullable(&h, Uuid::from_u128(0xE111), Uuid::from_u128(0xE112)).await;
    h.set_system(template, json!({ "hp": 12 })).await;
    h.set_system(child, json!({ "hp": 11 })).await;

    let first = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.player,
        ClientMsg::MergePull {
            request_id: Uuid::from_u128(1),
            child_id: child,
            resolutions: None,
        },
        0,
    )
    .await
    .expect("a reply");
    let MergePullStatus::Conflicts(conflicts) = pull_status(first) else {
        panic!("expected Conflicts");
    };
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].path, "/system/hp");
    assert_eq!(
        h.get(child).await.system,
        json!({ "hp": 11 }),
        "a conflicted first call writes nothing"
    );

    let second = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.player,
        ClientMsg::MergePull {
            request_id: Uuid::from_u128(2),
            child_id: child,
            resolutions: Some(vec!["/system/hp".to_string()]),
        },
        0,
    )
    .await
    .expect("a reply");
    assert!(matches!(pull_status(second), MergePullStatus::Applied));
    assert_eq!(
        h.get(child).await.system,
        json!({ "hp": 12 }),
        "theirs taken"
    );
}

/// A resolutions path off the mergeable surface can never have been a conflict:
/// `UnknownResolution`, carrying the fresh (still conflicted) outcome.
#[tokio::test]
async fn pull_resolutions_off_surface_path_is_unknown_resolution() {
    let h = merge_harness().await;
    let (template, child) =
        player_pullable(&h, Uuid::from_u128(0xE121), Uuid::from_u128(0xE122)).await;
    h.set_system(template, json!({ "hp": 12 })).await;
    h.set_system(child, json!({ "hp": 11 })).await;

    let reply = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.player,
        ClientMsg::MergePull {
            request_id: Uuid::from_u128(1),
            child_id: child,
            resolutions: Some(vec!["/permissions/default".to_string()]),
        },
        0,
    )
    .await
    .expect("a reply");
    let MergeErrorKind::UnknownResolution(MergeOutcome::Pull { status, .. }) = error_reason(reply)
    else {
        panic!("expected UnknownResolution carrying a Pull outcome");
    };
    assert!(
        matches!(status, MergePullStatus::Conflicts(_)),
        "the fresh outcome carries the current conflict set"
    );
    assert_eq!(
        h.get(child).await.system,
        json!({ "hp": 11 }),
        "nothing written"
    );
}

/// A resolutions call whose paths no longer match the recomputed conflict set —
/// the conflict was resolved away by an interleaving edit between the two
/// calls — is refused as `StaleResolutions` carrying the fresh outcome.
#[tokio::test]
async fn pull_resolutions_made_stale_by_an_interleaving_edit() {
    let h = merge_harness().await;
    let (template, child) =
        player_pullable(&h, Uuid::from_u128(0xE131), Uuid::from_u128(0xE132)).await;
    h.set_system(template, json!({ "hp": 12 })).await;
    h.set_system(child, json!({ "hp": 11 })).await;

    let first = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.player,
        ClientMsg::MergePull {
            request_id: Uuid::from_u128(1),
            child_id: child,
            resolutions: None,
        },
        0,
    )
    .await
    .expect("a reply");
    assert!(matches!(pull_status(first), MergePullStatus::Conflicts(_)));

    // The child moves to the template's value between calls: `/system/hp` is no
    // longer a conflict path.
    h.set_system(child, json!({ "hp": 12 })).await;

    let second = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.player,
        ClientMsg::MergePull {
            request_id: Uuid::from_u128(2),
            child_id: child,
            resolutions: Some(vec!["/system/hp".to_string()]),
        },
        0,
    )
    .await
    .expect("a reply");
    let MergeErrorKind::StaleResolutions(MergeOutcome::Pull { status, .. }) = error_reason(second)
    else {
        panic!("expected StaleResolutions carrying a Pull outcome");
    };
    assert!(
        matches!(status, MergePullStatus::Applied),
        "the fresh outcome reports the merge is now conflict-free"
    );
}

/// Pull is refused for a requester who is neither the child's effective owner
/// nor a GM — same `Forbidden` whether or not they can read the document.
#[tokio::test]
async fn pull_forbidden_for_non_owner_non_gm() {
    let h = merge_harness().await;
    let (template, _) = player_pullable(&h, Uuid::from_u128(0xE141), Uuid::from_u128(0xE142)).await;
    // A GM-owned, world-visible instance of the same template: the player can
    // READ it but is not its owner.
    let foreign = instance_doc(
        h.world_id,
        Uuid::from_u128(0xE143),
        template,
        h.gm.user_id,
        DocRole::Observer,
        json!({ "hp": 10 }),
    );
    h.create(foreign).await;

    let reply = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.player,
        ClientMsg::MergePull {
            request_id: Uuid::from_u128(1),
            child_id: Uuid::from_u128(0xE143),
            resolutions: None,
        },
        0,
    )
    .await
    .expect("a reply");
    assert!(matches!(error_reason(reply), MergeErrorKind::Forbidden));
}

/// A GM may pull into an instance owned by a player (the GM short-circuit is
/// owner-equivalent).
#[tokio::test]
async fn pull_allowed_for_gm() {
    let h = merge_harness().await;
    let (template, child) =
        player_pullable(&h, Uuid::from_u128(0xE151), Uuid::from_u128(0xE152)).await;
    h.set_system(template, json!({ "hp": 12 })).await;

    let reply = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.gm,
        ClientMsg::MergePull {
            request_id: Uuid::from_u128(1),
            child_id: child,
            resolutions: None,
        },
        0,
    )
    .await
    .expect("a reply");
    assert!(matches!(pull_status(reply), MergePullStatus::Applied));
    assert_eq!(h.get(child).await.system, json!({ "hp": 12 }));
}

/// The per-path derivation refuses a pull whose computed Update writes a path
/// the requester cannot write: the player's Owner floor grants WRITE_FIELDS but
/// not MANAGE_EMBEDDED, and the template added an embedded child since the
/// stamp, so the merge's `/embedded/items` whole-collection write exceeds the
/// player's capabilities — `Forbidden`, nothing written.
#[tokio::test]
async fn pull_forbidden_when_the_computed_update_exceeds_the_requesters_capabilities() {
    let h = merge_harness().await;
    let (template, child) =
        player_pullable(&h, Uuid::from_u128(0xE161), Uuid::from_u128(0xE162)).await;

    // Post-stamp, the template gains an embedded item.
    let item = {
        let mut d = crate::data::document::tests::world_scoped_doc(
            h.world_id,
            Uuid::from_u128(0xE163),
            "item",
        );
        d.system = json!({ "qty": 1 });
        d
    };
    h.room
        .publish(
            h.repo.as_ref(),
            &h.gm,
            vec![Operation::Update {
                doc_id: template,
                changes: vec![FieldChange {
                    path: "/embedded/items".into(),
                    old: serde_json::Value::Null,
                    new: json!([serde_json::to_value(&item).unwrap()]),
                    remove: false,
                }],
            }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    let reply = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.player,
        ClientMsg::MergePull {
            request_id: Uuid::from_u128(1),
            child_id: child,
            resolutions: None,
        },
        0,
    )
    .await
    .expect("a reply");
    assert!(matches!(error_reason(reply), MergeErrorKind::Forbidden));
    assert!(h.get(child).await.embedded.is_empty(), "nothing written");

    // Control: the same merge succeeds for the GM (every capability), and the
    // template-added child is restamped into the instance.
    let reply = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.gm,
        ClientMsg::MergePull {
            request_id: Uuid::from_u128(2),
            child_id: child,
            resolutions: None,
        },
        0,
    )
    .await
    .expect("a reply");
    assert!(matches!(pull_status(reply), MergePullStatus::Applied));
    let items = &h.get(child).await.embedded["items"];
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].system, json!({ "qty": 1 }));
}

/// `MergeRevert` never asks: on a conflicted instance it resets the mergeable
/// bands to the template outright and replies `Revert { Applied }`.
#[tokio::test]
async fn revert_applies_to_a_conflicted_instance_without_asking() {
    let h = merge_harness().await;
    let (template, child) =
        player_pullable(&h, Uuid::from_u128(0xE171), Uuid::from_u128(0xE172)).await;
    h.set_system(template, json!({ "hp": 12 })).await;
    h.set_system(child, json!({ "hp": 11 })).await;

    let reply = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.player,
        ClientMsg::MergeRevert {
            request_id: Uuid::from_u128(1),
            child_id: child,
        },
        0,
    )
    .await
    .expect("a reply");
    match reply {
        ServerMsg::MergeResult {
            outcome:
                MergeOutcome::Revert {
                    status: MergeRevertStatus::Applied,
                    ..
                },
            ..
        } => {}
        other => panic!("expected MergeResult::Revert Applied, got {other:?}"),
    }
    assert_eq!(
        h.get(child).await.system,
        json!({ "hp": 12 }),
        "the child's local diff is discarded"
    );
}

/// Revert derives authorization the same way pull does: a bystander is refused.
#[tokio::test]
async fn revert_forbidden_for_non_owner() {
    let h = merge_harness().await;
    let (template, child) =
        player_pullable(&h, Uuid::from_u128(0xE181), Uuid::from_u128(0xE182)).await;
    h.set_system(template, json!({ "hp": 12 })).await;

    let reply = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.bystander,
        ClientMsg::MergeRevert {
            request_id: Uuid::from_u128(1),
            child_id: child,
        },
        0,
    )
    .await
    .expect("a reply");
    assert!(matches!(error_reason(reply), MergeErrorKind::Forbidden));
    assert_eq!(
        h.get(child).await.system,
        json!({ "hp": 10 }),
        "nothing written"
    );
}

/// Pull distinguishes a missing child (`NotFound`) from a child that was never
/// stamped (`NotAnInstance`).
#[tokio::test]
async fn pull_not_found_and_not_an_instance() {
    let h = merge_harness().await;
    let reply = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.player,
        ClientMsg::MergePull {
            request_id: Uuid::from_u128(1),
            child_id: Uuid::from_u128(0xDEAD),
            resolutions: None,
        },
        0,
    )
    .await
    .expect("a reply");
    assert!(matches!(error_reason(reply), MergeErrorKind::NotFound));

    let plain = template_doc(
        h.world_id,
        Uuid::from_u128(0xE191),
        h.player.user_id,
        DocRole::Observer,
        json!({ "hp": 10 }),
    );
    h.create(plain).await;
    let reply = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.gm,
        ClientMsg::MergePull {
            request_id: Uuid::from_u128(2),
            child_id: Uuid::from_u128(0xE191),
            resolutions: None,
        },
        0,
    )
    .await
    .expect("a reply");
    assert!(matches!(error_reason(reply), MergeErrorKind::NotAnInstance));
}

/// The push fixture: a player-OWNED template (users entry + MANAGE_EMBEDDED
/// grant) and four instances — player-owned (writable), player-owned and
/// locally diverged (conflicted), GM-owned world-visible (not writable), and
/// GM-owned hidden (not visible). Returns (template, clean, conflicted,
/// visible_locked, hidden).
async fn push_matrix(h: &Harness) -> (Uuid, Uuid, Uuid, Uuid, Uuid) {
    let (template, clean, conflicted, visible_locked, hidden) = (
        Uuid::from_u128(0xE201),
        Uuid::from_u128(0xE202),
        Uuid::from_u128(0xE203),
        Uuid::from_u128(0xE204),
        Uuid::from_u128(0xE205),
    );
    let mut tmpl = template_doc(
        h.world_id,
        template,
        h.player.user_id,
        DocRole::Observer,
        json!({ "hp": 10 }),
    );
    tmpl.name = Some("PushTemplate".into());
    tmpl.permissions
        .users
        .insert(h.player.user_id, DocRole::Owner);
    tmpl.permissions
        .capabilities
        .by_user
        .entry(h.player.user_id)
        .or_default()
        .insert(crate::data::permission::cap::MANAGE_EMBEDDED.to_string());
    h.create(tmpl).await;

    let mut a = instance_doc(
        h.world_id,
        clean,
        template,
        h.player.user_id,
        DocRole::Observer,
        json!({ "hp": 10 }),
    );
    a.name = Some("Clean".into());
    a.permissions.users.insert(h.player.user_id, DocRole::Owner);
    h.create(a).await;

    let mut b = instance_doc(
        h.world_id,
        conflicted,
        template,
        h.player.user_id,
        DocRole::Observer,
        json!({ "hp": 10 }),
    );
    b.name = Some("Conflicted".into());
    b.permissions.users.insert(h.player.user_id, DocRole::Owner);
    h.create(b).await;

    let mut c = instance_doc(
        h.world_id,
        visible_locked,
        template,
        h.gm.user_id,
        DocRole::Observer,
        json!({ "hp": 10 }),
    );
    c.name = Some("VisibleLocked".into());
    h.create(c).await;

    let mut d = instance_doc(
        h.world_id,
        hidden,
        template,
        h.gm.user_id,
        DocRole::None,
        json!({ "hp": 10 }),
    );
    d.name = Some("Hidden".into());
    h.create(d).await;

    // Post-stamp template divergence: every instance has a pending parent-side
    // change; `conflicted` additionally diverged locally.
    h.set_system(template, json!({ "hp": 12 })).await;
    h.set_system(conflicted, json!({ "hp": 11 })).await;

    (template, clean, conflicted, visible_locked, hidden)
}

/// One player's push across the four-instance matrix: the writable clean
/// instance applies immediately, the writable conflicted one reports its
/// conflict set unwritten, the visible-but-not-writable one is `Excluded`
/// (carrying the pusher-visible name), and the not-visible one is OMITTED from
/// the outcome entirely — no entry, name, or count (true existence-hiding
/// parity with redaction: the pusher's store never contained it).
#[tokio::test]
async fn push_mixed_visibility_and_capability_outcomes() {
    let h = merge_harness().await;
    let (template, clean, conflicted, visible_locked, hidden) = push_matrix(&h).await;

    let reply = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.player,
        ClientMsg::MergePush {
            request_id: Uuid::from_u128(1),
            template_id: template,
            resolutions: None,
        },
        0,
    )
    .await
    .expect("a reply");
    let ServerMsg::MergeResult {
        outcome: MergeOutcome::Push { instances, .. },
        ..
    } = reply
    else {
        panic!("expected a MergeResult::Push");
    };
    assert_eq!(
        instances.len(),
        3,
        "the invisible instance has no entry at all"
    );
    let entry = |id: Uuid| instances.iter().find(|e| e.instance_id == id).unwrap();

    assert!(matches!(entry(clean).status, PushInstanceStatus::Applied));
    assert_eq!(
        h.get(clean).await.system,
        json!({ "hp": 12 }),
        "clean applied"
    );

    let PushInstanceStatus::Conflicts(conflicts) = &entry(conflicted).status else {
        panic!("conflicted instance must report Conflicts");
    };
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].path, "/system/hp");
    assert_eq!(
        h.get(conflicted).await.system,
        json!({ "hp": 11 }),
        "conflicted instance not written on a first call"
    );

    assert!(matches!(
        entry(visible_locked).status,
        PushInstanceStatus::Excluded
    ));
    assert_eq!(entry(visible_locked).name.as_deref(), Some("VisibleLocked"));

    assert!(
        instances.iter().all(|e| e.instance_id != hidden),
        "a hidden instance is omitted — no id, no name, no count beyond the visible set"
    );
    assert_eq!(
        h.get(hidden).await.system,
        json!({ "hp": 10 }),
        "hidden untouched"
    );
}

/// The second push call folds the per-instance resolutions in: the conflicted
/// instance commits with the template side taken; the already-clean instance
/// re-applies harmlessly; the excluded instances stay excluded.
#[tokio::test]
async fn push_resolutions_second_call_applies() {
    let h = merge_harness().await;
    let (template, clean, conflicted, _, _) = push_matrix(&h).await;

    let reply = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.player,
        ClientMsg::MergePush {
            request_id: Uuid::from_u128(1),
            template_id: template,
            resolutions: Some(
                [(conflicted, vec!["/system/hp".to_string()])]
                    .into_iter()
                    .collect(),
            ),
        },
        0,
    )
    .await
    .expect("a reply");
    let ServerMsg::MergeResult {
        outcome: MergeOutcome::Push { instances, .. },
        ..
    } = reply
    else {
        panic!("expected a MergeResult::Push");
    };
    let entry = |id: Uuid| instances.iter().find(|e| e.instance_id == id).unwrap();
    assert!(matches!(
        entry(conflicted).status,
        PushInstanceStatus::Applied
    ));
    assert!(matches!(entry(clean).status, PushInstanceStatus::Applied));
    assert_eq!(
        h.get(conflicted).await.system,
        json!({ "hp": 12 }),
        "theirs taken"
    );
}

/// A resolutions map key that names no visible push target (here: the hidden
/// instance, whose existence is never disclosed) is `UnknownResolution` — the
/// same refusal an outright-unknown id gets — and the carried fresh outcome
/// discloses nothing about it: no entry at all.
#[tokio::test]
async fn push_resolutions_for_an_invisible_instance_is_unknown_resolution() {
    let h = merge_harness().await;
    let (template, _, _, _, hidden) = push_matrix(&h).await;

    let reply = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.player,
        ClientMsg::MergePush {
            request_id: Uuid::from_u128(1),
            template_id: template,
            resolutions: Some(
                [(hidden, vec!["/system/hp".to_string()])]
                    .into_iter()
                    .collect(),
            ),
        },
        0,
    )
    .await
    .expect("a reply");
    let MergeErrorKind::UnknownResolution(MergeOutcome::Push { instances, .. }) =
        error_reason(reply)
    else {
        panic!("expected UnknownResolution carrying a Push outcome");
    };
    assert!(
        instances.iter().all(|e| e.instance_id != hidden),
        "the fresh outcome omits the hidden instance entirely"
    );
}

/// Push is refused for a requester who is not the template's effective owner
/// (and not GM), even when they can read the template.
#[tokio::test]
async fn push_forbidden_for_non_template_owner() {
    let h = merge_harness().await;
    let (template, _, _, _, _) = push_matrix(&h).await;

    let reply = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.bystander,
        ClientMsg::MergePush {
            request_id: Uuid::from_u128(1),
            template_id: template,
            resolutions: None,
        },
        0,
    )
    .await
    .expect("a reply");
    assert!(matches!(error_reason(reply), MergeErrorKind::Forbidden));
}

/// Push names a template: a missing one is `NotFound` (there is no
/// `NotAnInstance` case — the target is not an instance).
#[tokio::test]
async fn push_not_found_for_a_missing_template() {
    let h = merge_harness().await;
    let reply = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.gm,
        ClientMsg::MergePush {
            request_id: Uuid::from_u128(1),
            template_id: Uuid::from_u128(0xDEAD),
            resolutions: None,
        },
        0,
    )
    .await
    .expect("a reply");
    assert!(matches!(error_reason(reply), MergeErrorKind::NotFound));
}

/// An instance whose stored `base` does not parse as a `MergeBase` fails the
/// WHOLE push closed (`CorruptBase`) before anything is written — including
/// its clean sibling.
#[tokio::test]
async fn push_corrupt_base_fails_closed_before_any_write() {
    let h = merge_harness().await;
    let (template, clean, corrupt) = (
        Uuid::from_u128(0xE301),
        Uuid::from_u128(0xE302),
        Uuid::from_u128(0xE303),
    );
    h.create(template_doc(
        h.world_id,
        template,
        h.gm.user_id,
        DocRole::Observer,
        json!({ "hp": 10 }),
    ))
    .await;
    h.create(instance_doc(
        h.world_id,
        clean,
        template,
        h.gm.user_id,
        DocRole::Observer,
        json!({ "hp": 10 }),
    ))
    .await;
    // A legacy/corrupted row: `base` present but not a MergeBase. Seeded raw —
    // the ingest gates correctly refuse such a write, so only
    // `seed_document_unvalidated` can represent it.
    let mut bad = instance_doc(
        h.world_id,
        corrupt,
        template,
        h.gm.user_id,
        DocRole::Observer,
        json!({ "hp": 10 }),
    );
    bad.base = Some(json!("corrupt"));
    h.repo.seed_document_unvalidated(&bad).await.unwrap();
    h.set_system(template, json!({ "hp": 12 })).await;

    let reply = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.gm,
        ClientMsg::MergePush {
            request_id: Uuid::from_u128(1),
            template_id: template,
            resolutions: None,
        },
        0,
    )
    .await
    .expect("a reply");
    assert!(matches!(error_reason(reply), MergeErrorKind::CorruptBase));
    assert_eq!(
        h.get(clean).await.system,
        json!({ "hp": 10 }),
        "the corrupt sibling aborts the push before the clean instance is written"
    );
}

/// The committed merge write goes through the one write path: it lands in the
/// sequenced broadcast like any other command, and per-recipient filtering
/// treats it identically to a client-dispatched Update — a bystander's view of
/// the whole-band `/system` change has the `gm_only` subtree stripped, and the
/// `/base` change (hardcoded `OwnerOrGm`) is dropped for them entirely, while
/// the GM's view carries both.
#[tokio::test]
async fn merge_commits_broadcast_and_redact_like_a_client_update() {
    let h = merge_harness().await;
    let (template, child) = (Uuid::from_u128(0xE401), Uuid::from_u128(0xE402));
    let mut tmpl = template_doc(
        h.world_id,
        template,
        h.gm.user_id,
        DocRole::Observer,
        json!({ "hp": 10, "secret": "S1" }),
    );
    tmpl.name = Some("Template".into());
    h.create(tmpl).await;
    let mut c = instance_doc(
        h.world_id,
        child,
        template,
        h.player.user_id,
        DocRole::Observer,
        json!({ "hp": 10, "secret": "S1" }),
    );
    c.name = Some("Instance".into());
    c.permissions.users.insert(h.player.user_id, DocRole::Owner);
    c.permissions.property_overrides.insert(
        "/system/secret".into(),
        crate::data::document::Visibility::GmOnly,
    );
    h.create(c).await;
    h.set_system(template, json!({ "hp": 12, "secret": "S1" }))
        .await;

    let (mut rx, _) = h.room.subscribe();
    let reply = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.player,
        ClientMsg::MergePull {
            request_id: Uuid::from_u128(1),
            child_id: child,
            resolutions: None,
        },
        0,
    )
    .await
    .expect("a reply");
    assert!(matches!(pull_status(reply), MergePullStatus::Applied));

    // The committed command arrived on the room broadcast as an ordinary
    // sequenced event.
    let ev = loop {
        match rx.recv().await.unwrap() {
            crate::ws::room::RoomEvent::Event(ev) => break (*ev).clone(),
            crate::ws::room::RoomEvent::Other(_) => continue,
        }
    };
    assert_eq!(ev.command.ops.len(), 1);
    let Operation::Update { doc_id, changes } = &ev.command.ops[0] else {
        panic!("expected one Update op");
    };
    assert_eq!(*doc_id, child);
    let paths: Vec<&str> = changes.iter().map(|c| c.path.as_str()).collect();
    assert!(paths.contains(&"/system"), "got {paths:?}");
    assert!(paths.contains(&"/base"), "got {paths:?}");

    let current = crate::data::permission::load_current_docs(h.repo.as_ref(), &ev.command).await;

    let gm_view = filter_command(
        &ev.command,
        &ev.snapshot,
        &h.gm,
        &h.world_defaults,
        &current,
        |_| None,
    );
    let Operation::Update {
        changes: gm_changes,
        ..
    } = &gm_view.ops[0]
    else {
        panic!("GM view keeps the Update");
    };
    let gm_paths: Vec<&str> = gm_changes.iter().map(|c| c.path.as_str()).collect();
    assert!(gm_paths.contains(&"/base"));
    let gm_system = gm_changes.iter().find(|c| c.path == "/system").unwrap();
    assert_eq!(
        gm_system.new["secret"],
        json!("S1"),
        "the GM sees the hidden field"
    );

    let bystander_view = filter_command(
        &ev.command,
        &ev.snapshot,
        &h.bystander,
        &h.world_defaults,
        &current,
        |_| None,
    );
    let Operation::Update {
        changes: by_changes,
        ..
    } = &bystander_view.ops[0]
    else {
        panic!("bystander view keeps the Update");
    };
    assert!(
        !by_changes.iter().any(|c| c.path == "/base"),
        "the OwnerOrGm /base change is dropped for a bystander"
    );
    let by_system = by_changes.iter().find(|c| c.path == "/system").unwrap();
    assert!(
        by_system.new.get("secret").is_none() && by_system.old.get("secret").is_none(),
        "the gm_only subtree is stripped from the whole-band change"
    );
    assert_eq!(by_system.new["hp"], json!(12), "the visible half survives");
}

/// The hidden-conflict fixture: a template and a player-owned instance whose
/// `/system/secret` is `gm_only`-overridden on BOTH documents, both sides
/// having diverged from the snapshot (`S1` → template `S2`, child `S3`).
/// Returns (template_id, child_id).
async fn hidden_conflict_pair(h: &Harness, template_id: Uuid, child_id: Uuid) -> (Uuid, Uuid) {
    let mut template = template_doc(
        h.world_id,
        template_id,
        h.gm.user_id,
        DocRole::Observer,
        json!({ "hp": 10, "secret": "S1" }),
    );
    template
        .permissions
        .property_overrides
        .insert("/system/secret".into(), Visibility::GmOnly);
    h.create(template).await;
    let mut child = instance_doc(
        h.world_id,
        child_id,
        template_id,
        h.player.user_id,
        DocRole::Observer,
        json!({ "hp": 10, "secret": "S1" }),
    );
    child
        .permissions
        .users
        .insert(h.player.user_id, DocRole::Owner);
    child
        .permissions
        .property_overrides
        .insert("/system/secret".into(), Visibility::GmOnly);
    h.create(child).await;
    h.set_system(template_id, json!({ "hp": 10, "secret": "S2" }))
        .await;
    h.set_system(child_id, json!({ "hp": 10, "secret": "S3" }))
        .await;
    (template_id, child_id)
}

/// A conflict on a path hidden from the requester in either document is
/// removed from the replied conflict set and auto-resolves child-wins: a
/// non-GM owner pulling with a `gm_only` conflict on both documents gets an
/// Applied reply whose frame carries neither side's hidden value, and the
/// committed document keeps the child's side.
#[tokio::test]
async fn pull_hides_a_gm_only_conflict_from_a_non_gm_owner_and_applies_child_wins() {
    let h = merge_harness().await;
    let (_, child) =
        hidden_conflict_pair(&h, Uuid::from_u128(0xE501), Uuid::from_u128(0xE502)).await;

    let reply = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.player,
        ClientMsg::MergePull {
            request_id: Uuid::from_u128(1),
            child_id: child,
            resolutions: None,
        },
        0,
    )
    .await
    .expect("a reply");
    let wire = serde_json::to_string(&reply).unwrap();
    assert!(
        !wire.contains("S2") && !wire.contains("S3"),
        "the hidden values never appear in the frame: {wire}"
    );
    assert!(
        matches!(pull_status(reply), MergePullStatus::Applied),
        "the hidden conflict leaves the visible set empty, so the pull applies"
    );
    assert_eq!(
        h.get(child).await.system,
        json!({ "hp": 10, "secret": "S3" }),
        "the hidden conflict auto-resolved child-wins in the committed document"
    );
}

/// Same pair, GM requester: a GM sees every property tier, so the conflict is
/// reported normally and nothing is written on the first call.
#[tokio::test]
async fn pull_reports_the_gm_only_conflict_to_a_gm() {
    let h = merge_harness().await;
    let (_, child) =
        hidden_conflict_pair(&h, Uuid::from_u128(0xE511), Uuid::from_u128(0xE512)).await;

    let reply = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.gm,
        ClientMsg::MergePull {
            request_id: Uuid::from_u128(1),
            child_id: child,
            resolutions: None,
        },
        0,
    )
    .await
    .expect("a reply");
    let MergePullStatus::Conflicts(conflicts) = pull_status(reply) else {
        panic!("a GM sees the hidden conflict");
    };
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].path, "/system/secret");
    assert_eq!(
        h.get(child).await.system,
        json!({ "hp": 10, "secret": "S3" }),
        "a conflicted first call writes nothing"
    );
}

/// Push, hidden-conflict half: the instance's `/system/secret` is
/// `owner_or_gm`-overridden and the pusher is the TEMPLATE's owner — neither
/// the instance's owner nor a GM — so the conflict is withheld from the reply
/// and the child-wins default lands in the committed document. The GM control
/// on a fresh pair sees the conflict normally.
#[tokio::test]
async fn push_hides_an_owner_or_gm_conflict_from_a_non_owner_pusher() {
    let h = merge_harness().await;
    let (template, visible) = (Uuid::from_u128(0xE521), Uuid::from_u128(0xE522));
    let mut tmpl = template_doc(
        h.world_id,
        template,
        h.player.user_id,
        DocRole::Observer,
        json!({ "hp": 10, "secret": "S1" }),
    );
    tmpl.permissions
        .users
        .insert(h.player.user_id, DocRole::Owner);
    tmpl.permissions
        .capabilities
        .by_user
        .entry(h.player.user_id)
        .or_default()
        .insert(crate::data::permission::cap::MANAGE_EMBEDDED.to_string());
    h.create(tmpl).await;
    let mut inst = instance_doc(
        h.world_id,
        visible,
        template,
        h.gm.user_id,
        DocRole::Observer,
        json!({ "hp": 10, "secret": "S1" }),
    );
    inst.permissions
        .property_overrides
        .insert("/system/secret".into(), Visibility::OwnerOrGm);
    h.create(inst).await;
    h.set_system(template, json!({ "hp": 10, "secret": "S2" }))
        .await;
    h.set_system(visible, json!({ "hp": 10, "secret": "S3" }))
        .await;

    let reply = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.player,
        ClientMsg::MergePush {
            request_id: Uuid::from_u128(1),
            template_id: template,
            resolutions: None,
        },
        0,
    )
    .await
    .expect("a reply");
    let wire = serde_json::to_string(&reply).unwrap();
    assert!(
        !wire.contains("S2") && !wire.contains("S3"),
        "the hidden values never appear in the frame: {wire}"
    );
    let ServerMsg::MergeResult {
        outcome: MergeOutcome::Push { instances, .. },
        ..
    } = reply
    else {
        panic!("expected a MergeResult::Push");
    };
    assert_eq!(instances.len(), 1);
    assert!(
        matches!(instances[0].status, PushInstanceStatus::Applied),
        "the hidden conflict leaves the visible set empty, so the push applies"
    );
    assert_eq!(
        h.get(visible).await.system,
        json!({ "hp": 10, "secret": "S3" }),
        "the child-wins default landed in the committed document"
    );

    // GM control on a fresh pair: the same divergence is a reported conflict.
    let (template2, visible2) = (Uuid::from_u128(0xE523), Uuid::from_u128(0xE524));
    let mut tmpl2 = template_doc(
        h.world_id,
        template2,
        h.player.user_id,
        DocRole::Observer,
        json!({ "hp": 10, "secret": "S1" }),
    );
    tmpl2
        .permissions
        .users
        .insert(h.player.user_id, DocRole::Owner);
    h.create(tmpl2).await;
    let mut inst2 = instance_doc(
        h.world_id,
        visible2,
        template2,
        h.gm.user_id,
        DocRole::Observer,
        json!({ "hp": 10, "secret": "S1" }),
    );
    inst2
        .permissions
        .property_overrides
        .insert("/system/secret".into(), Visibility::OwnerOrGm);
    h.create(inst2).await;
    h.set_system(template2, json!({ "hp": 10, "secret": "S2" }))
        .await;
    h.set_system(visible2, json!({ "hp": 10, "secret": "S3" }))
        .await;

    let reply = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.gm,
        ClientMsg::MergePush {
            request_id: Uuid::from_u128(2),
            template_id: template2,
            resolutions: None,
        },
        0,
    )
    .await
    .expect("a reply");
    let ServerMsg::MergeResult {
        outcome: MergeOutcome::Push { instances, .. },
        ..
    } = reply
    else {
        panic!("expected a MergeResult::Push");
    };
    let PushInstanceStatus::Conflicts(conflicts) = &instances[0].status else {
        panic!("a GM pusher sees the owner_or_gm conflict");
    };
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].path, "/system/secret");
}

/// A server-origin merge whose plan rewrites a WHOLE embedded collection
/// commits through the write path's post-image checks: the restamped children
/// `plan_to_update` emits never carry a `base`.
#[tokio::test]
async fn template_merge_whole_collection_write_carries_no_embedded_base() {
    let h = merge_harness().await;
    let (template, child) =
        player_pullable(&h, Uuid::from_u128(0xE531), Uuid::from_u128(0xE532)).await;

    // Post-stamp, the template gains an embedded item — the pull's
    // `/embedded/items` whole-collection write restamps it into the instance.
    let item = {
        let mut d = crate::data::document::tests::world_scoped_doc(
            h.world_id,
            Uuid::from_u128(0xE533),
            "item",
        );
        d.system = json!({ "qty": 1 });
        d
    };
    h.room
        .publish(
            h.repo.as_ref(),
            &h.gm,
            vec![Operation::Update {
                doc_id: template,
                changes: vec![FieldChange {
                    path: "/embedded/items".into(),
                    old: serde_json::Value::Null,
                    new: json!([serde_json::to_value(&item).unwrap()]),
                    remove: false,
                }],
            }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    let reply = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.gm,
        ClientMsg::MergePull {
            request_id: Uuid::from_u128(1),
            child_id: child,
            resolutions: None,
        },
        0,
    )
    .await
    .expect("a reply");
    assert!(matches!(pull_status(reply), MergePullStatus::Applied));
    let items = &h.get(child).await.embedded["items"];
    assert_eq!(items.len(), 1, "the template-added child was restamped in");
    assert!(
        items[0].base.is_none(),
        "a restamped/merged embedded child never carries a base"
    );
}

/// `commit_error`'s mapping, directly: an OCC pre-image mismatch recomputes
/// the merge via `fresh` and replies `StaleResolutions` carrying that outcome
/// (or, when the recompute itself fails, ITS reason); `Forbidden` passes
/// through untouched; anything else collapses to `Internal`.
#[tokio::test]
async fn commit_error_maps_write_failures_to_the_merge_error_vocabulary() {
    let outcome = || MergeOutcome::Revert {
        child_id: Uuid::from_u128(1),
        status: MergeRevertStatus::Applied,
    };

    let reply = commit_error(
        Uuid::from_u128(9),
        DataError::Conflict("stale pre-image".into()),
        || async { Ok(outcome()) },
    )
    .await;
    let ServerMsg::MergeError {
        reason: MergeErrorKind::StaleResolutions(o),
        ..
    } = reply
    else {
        panic!("an OCC conflict maps to StaleResolutions, got {reply:?}");
    };
    assert_eq!(o, outcome(), "carries the recomputed outcome");

    let reply = commit_error(
        Uuid::from_u128(9),
        DataError::Conflict("stale pre-image".into()),
        || async { Err(MergeErrorKind::NotFound) },
    )
    .await;
    assert!(
        matches!(error_reason(reply), MergeErrorKind::NotFound),
        "a failed recompute surfaces its own reason"
    );

    let reply = commit_error(Uuid::from_u128(9), DataError::Forbidden, || async {
        Ok(outcome())
    })
    .await;
    assert!(
        matches!(error_reason(reply), MergeErrorKind::Forbidden),
        "Forbidden passes through"
    );

    let reply = commit_error(Uuid::from_u128(9), DataError::NotFound, || async {
        Ok(outcome())
    })
    .await;
    assert!(
        matches!(error_reason(reply), MergeErrorKind::Internal),
        "any other failure collapses to Internal"
    );
}
