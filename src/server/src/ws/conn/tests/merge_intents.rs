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
use crate::merge::{MergeBase, ParentKind};
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

    /// Whole-map `/permissions/property_overrides` rewrite of `id` as the GM.
    async fn set_overrides(&self, id: Uuid, overrides: &[(&str, Visibility)]) {
        let cur = self.get(id).await;
        let new: std::collections::BTreeMap<String, Visibility> = overrides
            .iter()
            .map(|(p, v)| ((*p).to_string(), *v))
            .collect();
        self.room
            .publish(
                self.repo.as_ref(),
                &self.gm,
                vec![Operation::Update {
                    doc_id: id,
                    changes: vec![FieldChange {
                        path: "/permissions/property_overrides".into(),
                        old: serde_json::to_value(&cur.permissions.property_overrides).unwrap(),
                        new: serde_json::to_value(&new).unwrap(),
                        remove: false,
                    }],
                }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();
    }

    /// Whole-collection `/embedded/items` rewrite of `id` as the GM.
    async fn set_items(&self, id: Uuid, items: Vec<Document>) {
        let cur = self.get(id).await;
        let old = cur
            .embedded
            .get("items")
            .map_or(serde_json::Value::Null, |kids| {
                serde_json::to_value(kids).unwrap()
            });
        self.room
            .publish(
                self.repo.as_ref(),
                &self.gm,
                vec![Operation::Update {
                    doc_id: id,
                    changes: vec![FieldChange {
                        path: "/embedded/items".into(),
                        old,
                        new: serde_json::to_value(&items).unwrap(),
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

/// An embedded `actor` child `id` with `system`, optionally stamped from
/// template child `from`.
fn embedded_child(
    world: Uuid,
    id: Uuid,
    from: Option<Uuid>,
    system: serde_json::Value,
) -> Document {
    let mut d = crate::data::document::tests::world_scoped_doc(world, id, "actor");
    d.source = from.map(|id| Source {
        id,
        pack: None,
        version: 1,
    });
    d.system = system;
    d
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

/// Hidden-conflict filtering addresses embedded children by IDENTITY, not by
/// index: the instance's first child is template-deleted and unchanged (so
/// the merge drops it, shifting every later child's OUTPUT index down by
/// one), and the second child's `gm_only` `/system/secret` conflicts. The
/// conflict is withheld from a non-GM owner and auto-resolves child-wins,
/// even though the child's live index and its conflict-path index disagree.
#[tokio::test]
async fn pull_withholds_a_hidden_embedded_conflict_behind_a_dropped_sibling() {
    let h = merge_harness().await;
    let (template, child) = (Uuid::from_u128(0xE601), Uuid::from_u128(0xE602));
    let (t_a, t_b) = (Uuid::from_u128(0xE603), Uuid::from_u128(0xE604));
    let (i_a, i_b) = (Uuid::from_u128(0xE605), Uuid::from_u128(0xE606));

    let mut tmpl = template_doc(
        h.world_id,
        template,
        h.gm.user_id,
        DocRole::Observer,
        json!({}),
    );
    tmpl.embedded.insert(
        "items".into(),
        vec![
            embedded_child(h.world_id, t_a, None, json!({ "hp": 1 })),
            embedded_child(h.world_id, t_b, None, json!({ "hp": 1, "secret": "S1" })),
        ],
    );
    h.create(tmpl).await;
    let mut inst = instance_doc(
        h.world_id,
        child,
        template,
        h.player.user_id,
        DocRole::Observer,
        json!({}),
    );
    inst.permissions
        .users
        .insert(h.player.user_id, DocRole::Owner);
    inst.permissions
        .capabilities
        .by_user
        .entry(h.player.user_id)
        .or_default()
        .insert(crate::data::permission::cap::MANAGE_EMBEDDED.to_string());
    let mut i_b_doc = embedded_child(
        h.world_id,
        i_b,
        Some(t_b),
        json!({ "hp": 1, "secret": "S1" }),
    );
    i_b_doc
        .permissions
        .property_overrides
        .insert("/system/secret".into(), Visibility::GmOnly);
    inst.embedded.insert(
        "items".into(),
        vec![
            embedded_child(h.world_id, i_a, Some(t_a), json!({ "hp": 1 })),
            i_b_doc,
        ],
    );
    h.create(inst).await;

    // The template deletes T_a and edits T_b's secret; the instance edits
    // I_b's secret. I_a is unchanged, so the merge drops it.
    h.set_items(
        template,
        vec![embedded_child(
            h.world_id,
            t_b,
            None,
            json!({ "hp": 1, "secret": "S2" }),
        )],
    )
    .await;
    let live = h.get(child).await;
    let mut edited = live.embedded["items"].clone();
    edited[1].system = json!({ "hp": 1, "secret": "S3" });
    h.set_items(child, edited).await;

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
        "the withheld conflict leaves the visible set empty, so the pull applies"
    );
    let items = &h.get(child).await.embedded["items"];
    assert_eq!(
        items.len(),
        1,
        "the unchanged template-deleted child was dropped"
    );
    assert_eq!(items[0].id, i_b);
    assert_eq!(
        items[0].system,
        json!({ "hp": 1, "secret": "S3" }),
        "the hidden conflict auto-resolved child-wins"
    );
}

/// Same identity rule on the TEMPLATE side: the template reorders its
/// children so the child whose `/system/secret` is `gm_only` on the template
/// sits at a template index different from its correlated instance child's
/// output index. The hidden template value never reaches the instance or the
/// wire.
#[tokio::test]
async fn pull_withholds_a_hidden_template_side_conflict_behind_a_template_reorder() {
    let h = merge_harness().await;
    let (template, child) = (Uuid::from_u128(0xE611), Uuid::from_u128(0xE612));
    let (t_a, t_b) = (Uuid::from_u128(0xE613), Uuid::from_u128(0xE614));
    let (i_a, i_b) = (Uuid::from_u128(0xE615), Uuid::from_u128(0xE616));

    let mut t_a_doc = embedded_child(h.world_id, t_a, None, json!({ "hp": 1, "secret": "S1" }));
    t_a_doc
        .permissions
        .property_overrides
        .insert("/system/secret".into(), Visibility::GmOnly);
    let mut tmpl = template_doc(
        h.world_id,
        template,
        h.gm.user_id,
        DocRole::Observer,
        json!({}),
    );
    tmpl.embedded.insert(
        "items".into(),
        vec![
            t_a_doc.clone(),
            embedded_child(h.world_id, t_b, None, json!({ "hp": 1 })),
        ],
    );
    h.create(tmpl).await;
    let mut inst = instance_doc(
        h.world_id,
        child,
        template,
        h.player.user_id,
        DocRole::Observer,
        json!({}),
    );
    inst.permissions
        .users
        .insert(h.player.user_id, DocRole::Owner);
    inst.permissions
        .capabilities
        .by_user
        .entry(h.player.user_id)
        .or_default()
        .insert(crate::data::permission::cap::MANAGE_EMBEDDED.to_string());
    inst.embedded.insert(
        "items".into(),
        vec![
            embedded_child(
                h.world_id,
                i_a,
                Some(t_a),
                json!({ "hp": 1, "secret": "S1" }),
            ),
            embedded_child(h.world_id, i_b, Some(t_b), json!({ "hp": 1 })),
        ],
    );
    h.create(inst).await;

    // Reorder the template to [T_b, T_a] and edit T_a's secret; the instance
    // edits I_a's secret too, so the pair conflicts on a template-hidden path.
    t_a_doc.system = json!({ "hp": 1, "secret": "S2" });
    h.set_items(
        template,
        vec![
            embedded_child(h.world_id, t_b, None, json!({ "hp": 1 })),
            t_a_doc,
        ],
    )
    .await;
    let live = h.get(child).await;
    let mut edited = live.embedded["items"].clone();
    edited[0].system = json!({ "hp": 1, "secret": "S3" });
    h.set_items(child, edited).await;

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
        !wire.contains("S2"),
        "the template's hidden value never appears in the frame: {wire}"
    );
    assert!(matches!(pull_status(reply), MergePullStatus::Applied));
    let items = &h.get(child).await.embedded["items"];
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].id, i_a, "instance order is preserved");
    assert_eq!(
        items[0].system,
        json!({ "hp": 1, "secret": "S3" }),
        "the instance keeps its own value on the template-hidden path"
    );
}

/// The hidden-field fixture for the parent-side rule: a GM-owned template
/// carrying a `gm_only` `/system/gm_secret` and an `owner_or_gm`
/// `/system/owner_note` (both hidden from the player, who is neither the
/// template's owner nor a GM) plus a visible `/system/hp`; a player-owned
/// instance stamped from it with no overrides of its own. Returns
/// (template_id, child_id).
async fn template_hidden_pair(h: &Harness, template_id: Uuid, child_id: Uuid) -> (Uuid, Uuid) {
    let mut template = template_doc(
        h.world_id,
        template_id,
        h.gm.user_id,
        DocRole::Observer,
        json!({ "hp": 10, "gm_secret": "S1", "owner_note": "N1" }),
    );
    template
        .permissions
        .property_overrides
        .insert("/system/gm_secret".into(), Visibility::GmOnly);
    template
        .permissions
        .property_overrides
        .insert("/system/owner_note".into(), Visibility::OwnerOrGm);
    h.create(template).await;
    let mut child = instance_doc(
        h.world_id,
        child_id,
        template_id,
        h.player.user_id,
        DocRole::Observer,
        json!({ "hp": 10, "gm_secret": "S1", "owner_note": "N1" }),
    );
    child
        .permissions
        .users
        .insert(h.player.user_id, DocRole::Owner);
    child
        .permissions
        .capabilities
        .by_user
        .entry(h.player.user_id)
        .or_default()
        .insert(crate::data::permission::cap::MANAGE_EMBEDDED.to_string());
    h.create(child).await;
    (template_id, child_id)
}

/// The stored `base` is ONE canonical value: the FULL, unredacted template
/// snapshot with its recorded policy re-expressed for this instance
/// (`snapshot_for_instance` under the two documents' effective-owner
/// relation), whichever seat ran the merge that wrote it. Proves the write
/// path stores the full snapshot (not the writer's view of it): under a
/// requester-relative snapshot this fails for any non-GM writer.
async fn assert_base_is_the_full_snapshot(h: &Harness, template: Uuid, child: Uuid) {
    let t = h.get(template).await;
    let c = h.get(child).await;
    let same_owner = h.repo.effective_owner_of(&c).await.unwrap()
        == h.repo.effective_owner_of(&t).await.unwrap();
    let expected =
        serde_json::to_value(crate::merge::bands::snapshot_for_instance(&t, same_owner)).unwrap();
    let stored = h
        .get(child)
        .await
        .base
        .expect("a merge write refreshes base");
    assert!(
        crate::merge::tree::structural_diff(&stored, &expected).is_empty(),
        "stored base {stored} is the full template snapshot {expected}"
    );
}

/// `filter_properties` of the stored document `id` under `ctx`'s resolved
/// access — the view egress delivers to that seat.
async fn seat_view(h: &Harness, ctx: &PermissionContext, id: Uuid) -> Document {
    let d = h.get(id).await;
    let owner = h.repo.effective_owner_of(&d).await.unwrap();
    let access = crate::data::permission::resolve_access_world(
        ctx.user_id,
        ctx.world_role,
        &d,
        &h.world_defaults.grants_for(&d.doc_type),
        owner,
    );
    crate::data::permission::filter_properties(&d, &access).unwrap()
}

/// One seat's `syncState` parity, computed the way the client computes it:
/// that seat's egress view of the stored base, read through `MergeBase`'s
/// own defaults (the client's `normalizeBase`), structurally equals
/// `snapshot_base` of that seat's egress view of the template on CONTENT
/// (the recorded policy maps are excluded, as the client's `syncState`
/// excludes them — the snapshot's is re-expressed for the instance, the
/// template's is verbatim) — so the client's badge reads up-to-date for this
/// seat. Pinned per seat because the two views are cut by different policies
/// (the snapshot's recorded one and the template's current one) and must
/// still agree.
async fn assert_sync_state_parity(
    h: &Harness,
    ctx: &PermissionContext,
    template: Uuid,
    child: Uuid,
) {
    let template_view = seat_view(h, ctx, template).await;
    let expected = serde_json::to_value(crate::merge::bands::content_only(
        &crate::merge::snapshot_base(&template_view),
    ))
    .unwrap();
    let base_view = seat_view(h, ctx, child)
        .await
        .base
        .expect("an owner-or-GM seat receives base");
    let normalized: MergeBase = serde_json::from_value(base_view).expect("a redacted base parses");
    let normalized = serde_json::to_value(crate::merge::bands::content_only(&normalized)).unwrap();
    assert!(
        crate::merge::tree::structural_diff(&normalized, &expected).is_empty(),
        "this seat's view of the stored base {normalized} equals its view of the template {expected}"
    );
}

/// The parent side of a pull is the template as the REQUESTER sees it: the
/// template's hidden edits (`gm_only`, `owner_or_gm`) never move into the
/// instance's content and never appear in the reply. The refreshed `/base` is
/// the FULL template's snapshot, and the requester's egress view of it is
/// cut by the policy the snapshot records — so the client's sync badge, which
/// diffs its view of `base` against its redacted store view of the template,
/// reads up-to-date for every seat.
#[tokio::test]
async fn pull_never_moves_template_hidden_values_and_stores_the_full_snapshot() {
    let h = merge_harness().await;
    let (template, child) =
        template_hidden_pair(&h, Uuid::from_u128(0xE701), Uuid::from_u128(0xE702)).await;
    h.set_system(
        template,
        json!({ "hp": 11, "gm_secret": "S2", "owner_note": "N2" }),
    )
    .await;

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
        !wire.contains("S2") && !wire.contains("N2"),
        "the hidden template values never appear in the frame: {wire}"
    );
    assert!(matches!(pull_status(reply), MergePullStatus::Applied));
    let stored = h.get(child).await;
    assert_eq!(
        stored.system,
        json!({ "hp": 11, "gm_secret": "S1", "owner_note": "N1" }),
        "only the visible edit moved; the instance's own hidden-path values stay"
    );
    assert_base_is_the_full_snapshot(&h, template, child).await;
    let base_view = seat_view(&h, &h.player, child)
        .await
        .base
        .expect("the instance owner receives base");
    let base_wire = serde_json::to_string(&base_view).unwrap();
    assert!(
        !base_wire.contains("S2")
            && !base_wire.contains("N2")
            && base_view["system"].get("gm_secret").is_none()
            && base_view["system"].get("owner_note").is_none(),
        "the requester's egress view of the snapshot carries nothing they cannot see: {base_wire}"
    );
    assert_sync_state_parity(&h, &h.player, template, child).await;
    assert_sync_state_parity(&h, &h.gm, template, child).await;

    // A GM's pull of the same pair is unchanged: the GM sees everything.
    let (template2, child2) =
        template_hidden_pair(&h, Uuid::from_u128(0xE703), Uuid::from_u128(0xE704)).await;
    h.set_system(
        template2,
        json!({ "hp": 11, "gm_secret": "S2", "owner_note": "N2" }),
    )
    .await;
    let reply = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.gm,
        ClientMsg::MergePull {
            request_id: Uuid::from_u128(2),
            child_id: child2,
            resolutions: None,
        },
        0,
    )
    .await
    .expect("a reply");
    assert!(matches!(pull_status(reply), MergePullStatus::Applied));
    assert_eq!(
        h.get(child2).await.system,
        json!({ "hp": 11, "gm_secret": "S2", "owner_note": "N2" })
    );
    assert_base_is_the_full_snapshot(&h, template2, child2).await;
    assert_sync_state_parity(&h, &h.gm, template2, child2).await;
    assert_sync_state_parity(&h, &h.player, template2, child2).await;
}

/// Revert resets only what the requester can see of the template: the
/// instance's values on template-hidden paths survive the reset, and the
/// refreshed `/base` is the full snapshot, cut per seat at egress.
#[tokio::test]
async fn revert_keeps_the_instances_values_on_template_hidden_paths() {
    let h = merge_harness().await;
    let (template, child) =
        template_hidden_pair(&h, Uuid::from_u128(0xE711), Uuid::from_u128(0xE712)).await;
    h.set_system(
        template,
        json!({ "hp": 11, "gm_secret": "S2", "owner_note": "N2" }),
    )
    .await;
    h.set_system(
        child,
        json!({ "hp": 5, "gm_secret": "S3", "owner_note": "N3", "extra": true }),
    )
    .await;

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
    let wire = serde_json::to_string(&reply).unwrap();
    assert!(!wire.contains("S2") && !wire.contains("N2"), "{wire}");
    assert!(matches!(
        reply,
        ServerMsg::MergeResult {
            outcome: MergeOutcome::Revert { .. },
            ..
        }
    ));
    assert_eq!(
        h.get(child).await.system,
        json!({ "hp": 11, "gm_secret": "S3", "owner_note": "N3" }),
        "visible paths reset to the template; hidden-path values are the instance's own"
    );
    assert_base_is_the_full_snapshot(&h, template, child).await;
    assert_sync_state_parity(&h, &h.player, template, child).await;
    assert_sync_state_parity(&h, &h.gm, template, child).await;
}

/// Push, parent-side rule: the pusher owns the template but a `gm_only`
/// field on it is still hidden from them; the instance (owned by someone
/// else) hides an `owner_or_gm` field from the pusher. Neither hidden value
/// moves or appears on the wire; the instance's `/base` snapshots the FULL
/// template, and the instance owner's egress view of it omits the template's
/// hidden field.
#[tokio::test]
async fn push_never_moves_template_hidden_values_into_an_instance_the_pusher_cannot_fully_see() {
    let h = merge_harness().await;
    let (template, instance) = (Uuid::from_u128(0xE721), Uuid::from_u128(0xE722));
    let mut tmpl = template_doc(
        h.world_id,
        template,
        h.player.user_id,
        DocRole::Observer,
        json!({ "hp": 10, "gm_secret": "S1", "mine": "M1" }),
    );
    tmpl.permissions
        .users
        .insert(h.player.user_id, DocRole::Owner);
    tmpl.permissions
        .property_overrides
        .insert("/system/gm_secret".into(), Visibility::GmOnly);
    tmpl.permissions
        .capabilities
        .by_user
        .entry(h.player.user_id)
        .or_default()
        .insert(crate::data::permission::cap::MANAGE_EMBEDDED.to_string());
    h.create(tmpl).await;
    let mut inst = instance_doc(
        h.world_id,
        instance,
        template,
        h.bystander.user_id,
        DocRole::Observer,
        json!({ "hp": 10, "gm_secret": "S1", "mine": "M1" }),
    );
    inst.permissions
        .property_overrides
        .insert("/system/mine".into(), Visibility::OwnerOrGm);
    let caps = inst
        .permissions
        .capabilities
        .by_user
        .entry(h.player.user_id)
        .or_default();
    caps.insert(crate::data::permission::cap::WRITE_FIELDS.to_string());
    caps.insert(crate::data::permission::cap::MANAGE_EMBEDDED.to_string());
    h.create(inst).await;
    // The GM edits the template's hidden field; the pusher's own visible
    // edits are `hp` and `mine`; the instance's owner diverged on `mine`.
    h.set_system(
        template,
        json!({ "hp": 11, "gm_secret": "S2", "mine": "M2" }),
    )
    .await;
    h.set_system(
        instance,
        json!({ "hp": 10, "gm_secret": "S1", "mine": "M3" }),
    )
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
        !wire.contains("S2") && !wire.contains("M2") && !wire.contains("M3"),
        "no hidden value on either side appears in the frame: {wire}"
    );
    let ServerMsg::MergeResult {
        outcome: MergeOutcome::Push { instances, .. },
        ..
    } = reply
    else {
        panic!("expected a MergeResult::Push");
    };
    assert_eq!(instances.len(), 1);
    assert!(matches!(instances[0].status, PushInstanceStatus::Applied));
    assert_eq!(
        h.get(instance).await.system,
        json!({ "hp": 11, "gm_secret": "S1", "mine": "M3" }),
        "hp moved; the template-hidden gm_secret did not; the child-hidden conflict stayed child-wins"
    );
    assert_base_is_the_full_snapshot(&h, template, instance).await;
    let owner_base = seat_view(&h, &h.bystander, instance)
        .await
        .base
        .expect("the instance owner receives base");
    assert!(
        owner_base["system"].get("gm_secret").is_none()
            && !serde_json::to_string(&owner_base).unwrap().contains("S2"),
        "the instance owner's view of the snapshot omits the template's gm_only value: {owner_base}"
    );
    assert!(
        seat_view(&h, &h.player, instance).await.base.is_none(),
        "the pusher, neither owner nor GM of the instance, receives no base at all"
    );
    assert_sync_state_parity(&h, &h.bystander, template, instance).await;
    assert_sync_state_parity(&h, &h.gm, template, instance).await;
}

/// A resolution whose "take template" cannot be applied to the current merged
/// shape — the instance replaced `/system/obj` with a scalar while the
/// template edited `/system/obj/x`, so the conflict sits at the template's
/// path with nowhere to write — is rejected as `Unresolvable` carrying the
/// fresh conflict set. The connection survives (the release profile aborts on
/// panic, so this path may never panic) and the instance is untouched.
#[tokio::test]
async fn pull_resolution_under_an_ancestor_descendant_conflict_is_rejected_not_fatal() {
    let h = merge_harness().await;
    let (template, child) = (Uuid::from_u128(0xE801), Uuid::from_u128(0xE802));
    h.create(template_doc(
        h.world_id,
        template,
        h.gm.user_id,
        DocRole::Observer,
        json!({ "obj": { "x": 1 } }),
    ))
    .await;
    let mut inst = instance_doc(
        h.world_id,
        child,
        template,
        h.player.user_id,
        DocRole::Observer,
        json!({ "obj": { "x": 1 } }),
    );
    inst.permissions
        .users
        .insert(h.player.user_id, DocRole::Owner);
    h.create(inst).await;
    h.set_system(template, json!({ "obj": { "x": 2 } })).await;
    h.set_system(child, json!({ "obj": 5 })).await;

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
        panic!("the overlap is reported as a conflict at the template's path");
    };
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].path, "/system/obj/x");

    let second = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.player,
        ClientMsg::MergePull {
            request_id: Uuid::from_u128(2),
            child_id: child,
            resolutions: Some(vec!["/system/obj/x".into()]),
        },
        0,
    )
    .await
    .expect("a reply, not an abort");
    let MergeErrorKind::Unresolvable(fresh) = error_reason(second) else {
        panic!("taking the template under a scalar is rejected as Unresolvable");
    };
    assert!(
        matches!(fresh, MergeOutcome::Pull { status: MergePullStatus::Conflicts(ref c), .. } if c.len() == 1),
        "the rejection carries the fresh conflict set: {fresh:?}"
    );
    assert_eq!(
        h.get(child).await.system,
        json!({ "obj": 5 }),
        "nothing was written"
    );

    // The same connection keeps working: choosing the instance's side applies.
    let third = handle_merge_intent(
        &h.room,
        h.repo.as_ref(),
        &h.player,
        ClientMsg::MergePull {
            request_id: Uuid::from_u128(3),
            child_id: child,
            resolutions: Some(vec![]),
        },
        0,
    )
    .await
    .expect("a reply");
    assert!(matches!(pull_status(third), MergePullStatus::Applied));
}

/// A template the requester cannot READ is reported exactly like a missing one
/// (`NotFound`, never `Forbidden`): the instance's `source` id must not confirm
/// that a document the requester cannot see exists.
#[tokio::test]
async fn pull_against_an_unreadable_template_is_not_found() {
    let h = merge_harness().await;
    let (template, child) = (Uuid::from_u128(0xE811), Uuid::from_u128(0xE812));
    h.create(template_doc(
        h.world_id,
        template,
        h.gm.user_id,
        DocRole::None,
        json!({ "hp": 10 }),
    ))
    .await;
    let mut inst = instance_doc(
        h.world_id,
        child,
        template,
        h.player.user_id,
        DocRole::Observer,
        json!({ "hp": 10 }),
    );
    inst.permissions
        .users
        .insert(h.player.user_id, DocRole::Owner);
    h.create(inst).await;

    for msg in [
        ClientMsg::MergePull {
            request_id: Uuid::from_u128(1),
            child_id: child,
            resolutions: None,
        },
        ClientMsg::MergeRevert {
            request_id: Uuid::from_u128(2),
            child_id: child,
        },
    ] {
        let reply = handle_merge_intent(&h.room, h.repo.as_ref(), &h.player, msg, 0)
            .await
            .expect("a reply");
        assert!(
            matches!(error_reason(reply), MergeErrorKind::NotFound),
            "an unreadable template is indistinguishable from a missing one"
        );
    }
}

/// An instance already in sync with its template yields a merge with no
/// changes: the reply is `Applied` and NOTHING is published — the world's
/// sequence does not advance, so a clean instance costs no `Event` per
/// pull (or per push resolution round).
#[tokio::test]
async fn pull_on_an_in_sync_instance_publishes_nothing() {
    let h = merge_harness().await;
    let (template, child) =
        player_pullable(&h, Uuid::from_u128(0xE821), Uuid::from_u128(0xE822)).await;
    h.set_system(template, json!({ "hp": 12 })).await;
    let pull = |request_id: u128| ClientMsg::MergePull {
        request_id: Uuid::from_u128(request_id),
        child_id: child,
        resolutions: None,
    };

    let first = handle_merge_intent(&h.room, h.repo.as_ref(), &h.player, pull(1), 0)
        .await
        .expect("a reply");
    assert!(matches!(pull_status(first), MergePullStatus::Applied));
    let seq_after_first = h.room.current_seq();
    assert_eq!(h.get(child).await.system, json!({ "hp": 12 }));

    let second = handle_merge_intent(&h.room, h.repo.as_ref(), &h.player, pull(2), 0)
        .await
        .expect("a reply");
    assert!(matches!(pull_status(second), MergePullStatus::Applied));
    assert_eq!(
        h.room.current_seq(),
        seq_after_first,
        "an in-sync pull publishes no Event"
    );
}

/// Ping-pong: the stored `/base` is one canonical value, so consecutive
/// pulls by seats with different views of the template never rewrite each
/// other's snapshot. After a GM's pull lands the template's edit, the
/// player's pull and the GM's second pull each publish NOTHING (the room
/// sequence stands still), and both seats' `syncState` parity holds
/// throughout. Under a requester-relative snapshot the player's pull would
/// write its narrower view out and the GM's badge would flip back.
#[tokio::test]
async fn gm_pull_then_player_pull_on_an_in_sync_instance_publishes_nothing() {
    let h = merge_harness().await;
    let (template, child) =
        template_hidden_pair(&h, Uuid::from_u128(0xE901), Uuid::from_u128(0xE902)).await;
    h.set_system(
        template,
        json!({ "hp": 11, "gm_secret": "S2", "owner_note": "N2" }),
    )
    .await;
    let pull = |ctx: &'static str, request: u128| {
        let ctx = match ctx {
            "gm" => &h.gm,
            _ => &h.player,
        };
        handle_merge_intent(
            &h.room,
            h.repo.as_ref(),
            ctx,
            ClientMsg::MergePull {
                request_id: Uuid::from_u128(request),
                child_id: child,
                resolutions: None,
            },
            0,
        )
    };

    let before = h.room.current_seq();
    assert!(matches!(
        pull("gm", 1).await.map(pull_status),
        Some(MergePullStatus::Applied)
    ));
    let after_gm = h.room.current_seq();
    assert_eq!(
        after_gm,
        before + 1,
        "the GM's pull commits the template's edit"
    );
    assert_base_is_the_full_snapshot(&h, template, child).await;
    assert_sync_state_parity(&h, &h.gm, template, child).await;
    assert_sync_state_parity(&h, &h.player, template, child).await;

    assert!(matches!(
        pull("player", 2).await.map(pull_status),
        Some(MergePullStatus::Applied)
    ));
    assert_eq!(
        h.room.current_seq(),
        after_gm,
        "the player's pull on the in-sync instance publishes nothing"
    );
    assert_base_is_the_full_snapshot(&h, template, child).await;
    assert_sync_state_parity(&h, &h.gm, template, child).await;
    assert_sync_state_parity(&h, &h.player, template, child).await;

    assert!(matches!(
        pull("gm", 3).await.map(pull_status),
        Some(MergePullStatus::Applied)
    ));
    assert_eq!(
        h.room.current_seq(),
        after_gm,
        "the GM's second pull publishes nothing either"
    );
    assert_sync_state_parity(&h, &h.gm, template, child).await;
    assert_sync_state_parity(&h, &h.player, template, child).await;
}

/// Recipient secrecy: a GM's push of a `gm_only` template value lands on a
/// player-owned instance HIDDEN — the template's policy propagates onto the
/// instance WITH THE MERGE WRITE (the template hid the field only after the
/// instance was stamped, so the stamp carried no such policy) — so the
/// instance owner's egress of the `Event` (`/system` and `/base` deltas), of
/// the stored document (`/system`, `/base`) and of any `MergeResult` never
/// carries the value, while the GM's does.
#[tokio::test]
async fn gm_push_of_a_gm_only_template_value_never_reaches_the_instance_owner() {
    let h = merge_harness().await;
    let (template, child) = (Uuid::from_u128(0xE911), Uuid::from_u128(0xE912));
    h.create(template_doc(
        h.world_id,
        template,
        h.gm.user_id,
        DocRole::Observer,
        json!({ "hp": 10, "gm_secret": "S1" }),
    ))
    .await;
    let mut inst = instance_doc(
        h.world_id,
        child,
        template,
        h.player.user_id,
        DocRole::Observer,
        json!({ "hp": 10, "gm_secret": "S1" }),
    );
    inst.permissions
        .users
        .insert(h.player.user_id, DocRole::Owner);
    h.create(inst).await;
    assert!(
        h.get(child).await.permissions.property_overrides.is_empty(),
        "the stamp carried no policy: the template had none yet"
    );
    // The GM now hides the field on the template and edits it.
    h.set_overrides(template, &[("/system/gm_secret", Visibility::GmOnly)])
        .await;
    h.set_system(template, json!({ "hp": 10, "gm_secret": "S2" }))
        .await;

    let (mut rx, _) = h.room.subscribe();
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
    let ServerMsg::MergeResult {
        outcome: MergeOutcome::Push { instances, .. },
        ..
    } = &reply
    else {
        panic!("expected a MergeResult::Push, got {reply:?}");
    };
    assert_eq!(instances.len(), 1);
    assert!(matches!(instances[0].status, PushInstanceStatus::Applied));
    // The reply is addressed to the pusher alone; with the merge applied it
    // carries no conflict payload for anyone to read the value from.
    let wire = serde_json::to_string(&reply).unwrap();
    assert!(
        !wire.contains("S2"),
        "an applied push discloses no values: {wire}"
    );

    let ev = loop {
        match rx.recv().await.unwrap() {
            crate::ws::room::RoomEvent::Event(ev) => break (*ev).clone(),
            crate::ws::room::RoomEvent::Other(_) => continue,
        }
    };
    let current = crate::data::permission::load_current_docs(h.repo.as_ref(), &ev.command).await;
    let owner_event = serde_json::to_string(&filter_command(
        &ev.command,
        &ev.snapshot,
        &h.player,
        &h.world_defaults,
        &current,
        |_| None,
    ))
    .unwrap();
    assert!(
        !owner_event.contains("S2"),
        "the instance owner's Event egress never carries the value: {owner_event}"
    );
    let gm_event = serde_json::to_string(&filter_command(
        &ev.command,
        &ev.snapshot,
        &h.gm,
        &h.world_defaults,
        &current,
        |_| None,
    ))
    .unwrap();
    assert!(
        gm_event.contains("S2"),
        "the GM's Event egress carries it: {gm_event}"
    );

    let stored = h.get(child).await;
    assert_eq!(stored.system["gm_secret"], json!("S2"), "the value landed");
    assert_eq!(
        stored
            .permissions
            .property_overrides
            .get("/system/gm_secret"),
        Some(&Visibility::GmOnly),
        "and landed hidden: the template's policy propagated onto the instance"
    );
    let owner_doc = serde_json::to_string(&seat_view(&h, &h.player, child).await).unwrap();
    assert!(
        !owner_doc.contains("S2"),
        "the instance owner's document egress (system and base) omits it: {owner_doc}"
    );
    let gm_doc = seat_view(&h, &h.gm, child).await;
    assert_eq!(gm_doc.system["gm_secret"], json!("S2"));
    assert_eq!(gm_doc.base.unwrap()["system"]["gm_secret"], json!("S2"));
    assert_sync_state_parity(&h, &h.player, template, child).await;
    assert_sync_state_parity(&h, &h.gm, template, child).await;
}

/// A template owner who is NOT the instance's owner pushes: every non-owner
/// requester's child-side hidden set carries the synthetic `/base` entry, so
/// a withhold rule of "any hidden pointer at all" would silently swallow
/// every template-deleted-but-changed child conflict for every such push.
/// With no overrides anywhere, the conflict is reported.
#[tokio::test]
async fn non_owner_template_owner_push_reports_a_template_deleted_child_conflict() {
    let h = merge_harness().await;
    let (template, instance) = (Uuid::from_u128(0xEA01), Uuid::from_u128(0xEA02));
    let (t_a, i_a) = (Uuid::from_u128(0xEA03), Uuid::from_u128(0xEA04));
    let mut tmpl = template_doc(
        h.world_id,
        template,
        h.player.user_id,
        DocRole::Observer,
        json!({}),
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
    tmpl.embedded.insert(
        "items".into(),
        vec![embedded_child(h.world_id, t_a, None, json!({ "hp": 1 }))],
    );
    h.create(tmpl).await;
    let mut inst = instance_doc(
        h.world_id,
        instance,
        template,
        h.bystander.user_id,
        DocRole::Observer,
        json!({}),
    );
    let caps = inst
        .permissions
        .capabilities
        .by_user
        .entry(h.player.user_id)
        .or_default();
    caps.insert(crate::data::permission::cap::WRITE_FIELDS.to_string());
    caps.insert(crate::data::permission::cap::MANAGE_EMBEDDED.to_string());
    inst.embedded.insert(
        "items".into(),
        vec![embedded_child(
            h.world_id,
            i_a,
            Some(t_a),
            json!({ "hp": 1 }),
        )],
    );
    h.create(inst).await;
    // The template deletes T_a; the instance edits I_a.
    h.set_items(template, vec![]).await;
    h.set_items(
        instance,
        vec![embedded_child(
            h.world_id,
            i_a,
            Some(t_a),
            json!({ "hp": 5 }),
        )],
    )
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
    let ServerMsg::MergeResult {
        outcome: MergeOutcome::Push { instances, .. },
        ..
    } = reply
    else {
        panic!("expected a MergeResult::Push, got {reply:?}");
    };
    assert_eq!(instances.len(), 1);
    let PushInstanceStatus::Conflicts(conflicts) = &instances[0].status else {
        panic!(
            "the template-deleted, instance-changed child is a reported conflict, got {:?}",
            instances[0].status
        );
    };
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].path, "/embedded/items/0");
    assert!(matches!(conflicts[0].parent_kind, ParentKind::Delete));
}
