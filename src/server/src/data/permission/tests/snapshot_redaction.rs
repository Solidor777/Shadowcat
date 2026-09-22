use super::*;
use crate::data::document::PermissionSet;
use crate::data::snapshot::{CommandSnapshot, OpSnapshot};

// -------------------------------------------------------------------
// Commit-time snapshot redaction — pure `filter_command` unit tests.
// Hand-built `CommandSnapshot`/`CurrentDoc` inputs; no repository round trip.
// -------------------------------------------------------------------

/// An `OpSnapshot` for an `Update` op: commit-time owner/gm/permissions plus a pruned
/// override set, no retraction, no created_seq mismatch (matches the current generation).
fn op_snapshot_update(
    owner_at_commit: Option<Uuid>,
    overrides_at_commit: Vec<(&str, Visibility)>,
    permissions_at_commit: PermissionSet,
) -> OpSnapshot {
    OpSnapshot {
        owner_at_commit,
        doc_type: "actor".into(),
        overrides_at_commit: overrides_at_commit
            .into_iter()
            .map(|(p, v)| (p.to_string(), v))
            .collect(),
        retraction_hidden_at_commit: None,
        created_seq_at_commit: None,
        permissions_at_commit: Some(permissions_at_commit),
        permissions_before_commit: None,
        owner_before_commit: None,
    }
}

fn snapshot_one_op(op: OpSnapshot, world_gm_at_commit: HashMap<Uuid, bool>) -> CommandSnapshot {
    CommandSnapshot {
        per_op: vec![Some(op)],
        world_gm_at_commit,
    }
}

fn permissions_default_observer() -> PermissionSet {
    PermissionSet {
        default: DocRole::Observer,
        ..Default::default()
    }
}

fn field_change_update_cmd(
    world: Uuid,
    author: Uuid,
    doc_id: Uuid,
    path: &str,
    old: serde_json::Value,
    new: serde_json::Value,
) -> Command {
    Command {
        seq: 1,
        world_id: world,
        author,
        ts: 0,
        ops: vec![Operation::Update {
            doc_id,
            changes: vec![FieldChange {
                remove: false,
                path: path.into(),
                old,
                new,
            }],
        }],
    }
}

#[test]
fn filter_command_drops_an_op_with_no_recorded_snapshot() {
    let world = crate::test_ids::test_uuid(9);
    let author = crate::test_ids::test_uuid(1);
    let doc_id = crate::test_ids::test_uuid(2);
    let cmd = field_change_update_cmd(
        world,
        author,
        doc_id,
        "/system/x",
        serde_json::json!(1),
        serde_json::json!(2),
    );
    let snapshot = CommandSnapshot {
        per_op: vec![None],
        world_gm_at_commit: HashMap::new(),
    };
    let cur = doc(
        permissions_default_observer(),
        serde_json::json!({ "x": 2 }),
    );
    let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
    let ctx = PermissionContext {
        user_id: crate::test_ids::test_uuid(3),
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    assert!(
        out.ops.is_empty(),
        "a None op-snapshot must drop the op on replay"
    );
}

#[test]
fn world_role_promotion_does_not_disclose_pre_promotion_gm_only_or_owner_or_gm_history() {
    // A player, hidden from a GmOnly field and a separate OwnerOrGm field while a non-GM
    // non-owner, is later promoted to GM and resyncs from before the promotion — both
    // fields must stay hidden. INVARIANT: `Access::can_see(OwnerOrGm)` is a disjunction
    // (`see_gm_only || is_owner`), so resolving the commit-time half's `see_gm_only` from
    // the recipient's CURRENT world role would defeat `owner_at_commit` for the OwnerOrGm
    // tier too, not just leak the GmOnly field — both must come from the snapshot alone.
    let world = crate::test_ids::test_uuid(9);
    let author = crate::test_ids::test_uuid(1);
    let doc_id = crate::test_ids::test_uuid(2);
    let owner = crate::test_ids::test_uuid(10);
    let recipient = crate::test_ids::test_uuid(20);
    let cmd = Command {
        seq: 1,
        world_id: world,
        author,
        ts: 0,
        ops: vec![Operation::Update {
            doc_id,
            changes: vec![
                FieldChange {
                    remove: false,
                    path: "/system/secret".into(),
                    old: serde_json::Value::Null,
                    new: serde_json::json!("gm secret"),
                },
                FieldChange {
                    remove: false,
                    path: "/system/owner_note".into(),
                    old: serde_json::Value::Null,
                    new: serde_json::json!("owner note"),
                },
            ],
        }],
    };
    let op = op_snapshot_update(
        Some(owner),
        vec![
            ("/system/secret", Visibility::GmOnly),
            ("/system/owner_note", Visibility::OwnerOrGm),
        ],
        permissions_default_observer(),
    );
    // The recipient was NOT GM at commit.
    let snapshot = snapshot_one_op(op, HashMap::from([(recipient, false)]));
    let mut cur = doc(permissions_default_observer(), serde_json::json!({}));
    cur.owner = Some(owner);
    let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
    // The recipient IS currently GM (post-promotion) — this is the defect scenario.
    let ctx = PermissionContext {
        user_id: recipient,
        world_role: WorldRole::Gm,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &out.ops[0] else {
        panic!("expected an Update op");
    };
    assert!(
        changes.is_empty(),
        "both the GmOnly and OwnerOrGm fields must stay hidden: {changes:?}"
    );
}

#[test]
fn reused_id_drops_a_stale_update_against_the_new_generation() {
    let world = crate::test_ids::test_uuid(9);
    let author = crate::test_ids::test_uuid(1);
    let doc_id = crate::test_ids::test_uuid(2);
    let cmd = field_change_update_cmd(
        world,
        author,
        doc_id,
        "/system/x",
        serde_json::json!(1),
        serde_json::json!(2),
    );
    let mut op = op_snapshot_update(None, vec![], permissions_default_observer());
    op.created_seq_at_commit = Some(5); // the OLD generation's created_seq
    let snapshot = snapshot_one_op(op, HashMap::new());
    let cur = doc(
        permissions_default_observer(),
        serde_json::json!({ "x": 2 }),
    );
    // The CURRENT document at this id is generation 9 (a later Create reused the id).
    let current = HashMap::from([(doc_id, current_doc(cur, 9))]);
    let ctx = PermissionContext {
        user_id: crate::test_ids::test_uuid(3),
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    assert!(
        out.ops.is_empty(),
        "a created_seq mismatch must drop the stale Update"
    );
}

#[test]
fn cross_op_existence_consistency_drops_an_update_denied_at_create_commit_time() {
    // A recipient denied commit-time access to a document's Create, later granted current
    // access, must ALSO have every subsequent Update to that doc_id dropped by the SAME
    // whole-document gate — not just the Create.
    let world = crate::test_ids::test_uuid(9);
    let author = crate::test_ids::test_uuid(1);
    let doc_id = crate::test_ids::test_uuid(2);
    let recipient = crate::test_ids::test_uuid(3);
    let cmd = field_change_update_cmd(
        world,
        author,
        doc_id,
        "/system/x",
        serde_json::json!(1),
        serde_json::json!(2),
    );
    // Commit-time permissions: default = None (nobody without an explicit grant may read).
    let denied_at_commit = PermissionSet {
        default: DocRole::None,
        ..Default::default()
    };
    let op = op_snapshot_update(None, vec![], denied_at_commit);
    let snapshot = snapshot_one_op(op, HashMap::new());
    // Current permissions: default = Observer (now anyone may read) — the asymmetry.
    let cur = doc(
        permissions_default_observer(),
        serde_json::json!({ "x": 2 }),
    );
    let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
    let ctx = PermissionContext {
        user_id: recipient,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    assert!(
        out.ops.is_empty(),
        "commit-time denial must drop the Update even though current access now permits it"
    );
}

#[test]
fn retraction_uses_the_commands_own_commit_moment_not_whatever_is_live() {
    // (a) A command that narrows visibility, replayed long after a LATER command has
    // narrowed it further — the retraction pass must reflect what the CHOSEN command
    // itself hid at ITS OWN commit, not whatever is live now.
    let world = crate::test_ids::test_uuid(9);
    let author = crate::test_ids::test_uuid(1);
    let doc_id = crate::test_ids::test_uuid(2);
    let recipient = crate::test_ids::test_uuid(3);
    let cmd = Command {
        seq: 1,
        world_id: world,
        author,
        ts: 0,
        ops: vec![Operation::Update {
            doc_id,
            changes: vec![FieldChange {
                remove: false,
                path: "/permissions/property_overrides/~1system~1a".into(),
                old: serde_json::Value::Null,
                new: serde_json::json!("gm_only"),
            }],
        }],
    };
    let mut op = op_snapshot_update(None, vec![], permissions_default_observer());
    // This command's OWN narrowing: only "/system/a" became hidden at ITS commit.
    op.retraction_hidden_at_commit = Some(vec![("/system/a".to_string(), Visibility::GmOnly)]);
    let snapshot = snapshot_one_op(op, HashMap::from([(recipient, false)]));
    let cur = doc(
        permissions_default_observer(),
        serde_json::json!({ "a": 1, "b": 2 }),
    );
    let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
    let ctx = PermissionContext {
        user_id: recipient,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &out.ops[0] else {
        panic!("expected an Update op");
    };
    assert!(
        changes
            .iter()
            .any(|c| c.path == "/system/a" && c.new.is_null()),
        "retraction must null the field THIS command hid: {changes:?}"
    );
    assert!(
        !changes.iter().any(|c| c.path == "/system/b"),
        "retraction must not touch a field this command never hid: {changes:?}"
    );
}

#[test]
fn retraction_does_not_null_the_owners_own_owner_or_gm_fields() {
    // (b) The SAME retracting command, replayed to the document's own OWNER — the owner's
    // legitimately-visible OwnerOrGm fields must NOT be nulled by retraction.
    let world = crate::test_ids::test_uuid(9);
    let author = crate::test_ids::test_uuid(1);
    let doc_id = crate::test_ids::test_uuid(2);
    let owner = crate::test_ids::test_uuid(3);
    let cmd = Command {
        seq: 1,
        world_id: world,
        author,
        ts: 0,
        ops: vec![Operation::Update {
            doc_id,
            changes: vec![FieldChange {
                remove: false,
                path: "/permissions/property_overrides/~1system~1name".into(),
                old: serde_json::Value::Null,
                new: serde_json::json!("owner_or_gm"),
            }],
        }],
    };
    let mut op = op_snapshot_update(Some(owner), vec![], permissions_default_observer());
    op.retraction_hidden_at_commit =
        Some(vec![("/system/name".to_string(), Visibility::OwnerOrGm)]);
    let snapshot = snapshot_one_op(op, HashMap::new());
    let mut cur = doc(
        permissions_default_observer(),
        serde_json::json!({ "name": "PC" }),
    );
    cur.owner = Some(owner);
    let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
    let ctx = PermissionContext {
        user_id: owner,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &out.ops[0] else {
        panic!("expected an Update op");
    };
    assert!(
        !changes.iter().any(|c| c.path == "/system/name"),
        "the owner's own OwnerOrGm field must not be retracted: {changes:?}"
    );
}

#[test]
fn multi_op_leak_within_one_command_is_closed_by_the_post_loop_accumulator() {
    // Within ONE command, an Update that sets a secret value followed by an Update that
    // adds a gm_only override on the SAME pointer must have BOTH ops' snapshots reflect the
    // FINAL (post-loop) override tree: BOTH ops' `overrides_at_commit` carry the gm_only
    // override here, even though only the SECOND op is the one that added it. A snapshot
    // built from each op's own per-iteration local state instead of the whole command's
    // final post-image would leave the FIRST op's `overrides_at_commit` empty, and this
    // test would then fail (the secret would leak to a non-GM/non-owner recipient on the
    // first op).
    let world = crate::test_ids::test_uuid(9);
    let author = crate::test_ids::test_uuid(1);
    let doc_id = crate::test_ids::test_uuid(2);
    let recipient = crate::test_ids::test_uuid(3);
    let cmd = Command {
        seq: 1,
        world_id: world,
        author,
        ts: 0,
        ops: vec![
            Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/system/secret".into(),
                    old: serde_json::Value::Null,
                    new: serde_json::json!("X"),
                }],
            },
            Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/permissions/property_overrides/~1system~1secret".into(),
                    old: serde_json::Value::Null,
                    new: serde_json::json!("gm_only"),
                }],
            },
        ],
    };
    let final_overrides = vec![("/system/secret", Visibility::GmOnly)];
    let op0 = op_snapshot_update(
        None,
        final_overrides.clone(),
        permissions_default_observer(),
    );
    let op1 = op_snapshot_update(None, final_overrides, permissions_default_observer());
    let snapshot = CommandSnapshot {
        per_op: vec![Some(op0), Some(op1)],
        world_gm_at_commit: HashMap::new(),
    };
    let cur = doc(
        permissions_default_observer(),
        serde_json::json!({ "secret": "X" }),
    );
    let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
    let ctx = PermissionContext {
        user_id: recipient,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &out.ops[0] else {
        panic!("expected an Update op for the FIRST op");
    };
    assert!(
        changes.is_empty(),
        "the first op's own snapshot must already reflect the LATER op's override: {changes:?}"
    );
}

#[test]
fn behavioural_mutation_current_output_unaffected_by_history_commit_output_unaffected_by_live() {
    // Mutate each live input independently — the target's overrides, its default, the
    // linked actor's owner, an embedded child's index, the recipient's world role — and
    // assert `filter_command`'s CURRENT-time output is unaffected by history and its
    // COMMIT-time output is unaffected by anything live.
    let world = crate::test_ids::test_uuid(9);
    let author = crate::test_ids::test_uuid(1);
    let doc_id = crate::test_ids::test_uuid(2);
    let recipient = crate::test_ids::test_uuid(3);
    let cmd = field_change_update_cmd(
        world,
        author,
        doc_id,
        "/system/x",
        serde_json::json!(1),
        serde_json::json!(2),
    );
    // Baseline: nothing hidden at commit, nothing hidden currently.
    let op = op_snapshot_update(None, vec![], permissions_default_observer());
    let snapshot = snapshot_one_op(op, HashMap::from([(recipient, false)]));
    let cur = doc(
        permissions_default_observer(),
        serde_json::json!({ "x": 2 }),
    );
    let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
    let ctx = PermissionContext {
        user_id: recipient,
        world_role: WorldRole::Player,
    };
    let baseline = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &baseline.ops[0] else {
        panic!("expected Update")
    };
    assert_eq!(changes.len(), 1, "baseline: field visible to everyone");

    // Mutate ONLY the live default (current-time) — commit-time snapshot untouched.
    let denied_current = PermissionSet {
        default: DocRole::None,
        ..Default::default()
    };
    let mut cur2 = doc(denied_current, serde_json::json!({ "x": 2 }));
    cur2.owner = None;
    let current2 = HashMap::from([(doc_id, current_doc(cur2, 0))]);
    let out_live_mutated = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current2,
        |_| None,
    );
    assert!(
        out_live_mutated.ops.is_empty(),
        "mutating ONLY the live default must change the CURRENT-time gate outcome"
    );

    // Mutate ONLY the commit-time permissions (recipient denied at commit) — live unchanged.
    let mut op_denied_commit = op_snapshot_update(
        None,
        vec![],
        PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
    );
    op_denied_commit.doc_type = "actor".into();
    let snapshot_denied_commit =
        snapshot_one_op(op_denied_commit, HashMap::from([(recipient, false)]));
    let out_commit_mutated = filter_command(
        &cmd,
        &snapshot_denied_commit,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    assert!(
        out_commit_mutated.ops.is_empty(),
        "mutating ONLY the commit-time permissions must change the COMMIT-time gate outcome"
    );
}

#[test]
fn embedded_child_index_in_the_commit_time_snapshot_is_independent_of_the_current_embedded_array() {
    // The commit-time override set is a flat, ALREADY-ADDRESSED pointer list
    // (`OpSnapshot::overrides_at_commit`), never re-derived from the CURRENT document's
    // embedded array. Mutating ONLY the current embedded structure (here: inserting
    // siblings so the commit-time secret child now sits at a different position) must not
    // change what the commit-time half redacts at a pointer the snapshot already names.
    let world = crate::test_ids::test_uuid(9);
    let author = crate::test_ids::test_uuid(1);
    let doc_id = crate::test_ids::test_uuid(2);
    let recipient = crate::test_ids::test_uuid(3);
    let cmd = Command {
        seq: 1,
        world_id: world,
        author,
        ts: 0,
        ops: vec![Operation::Update {
            doc_id,
            changes: vec![FieldChange {
                remove: false,
                path: "/embedded/actor/1/system/name".into(),
                old: serde_json::Value::Null,
                new: serde_json::json!("Hidden At Commit"),
            }],
        }],
    };
    let op = op_snapshot_update(
        None,
        vec![("/embedded/actor/1/system/name", Visibility::GmOnly)],
        permissions_default_observer(),
    );
    let snapshot = snapshot_one_op(op, HashMap::from([(recipient, false)]));
    // CURRENT structure: THREE children under "actor" — none carries the override (the
    // override lives only in the snapshot), so hidden_current alone would NOT redact this
    // pointer; only hidden_commit does.
    let mut cur = doc(permissions_default_observer(), serde_json::json!({}));
    cur.embedded.insert(
        "actor".into(),
        vec![
            doc(permissions_default_observer(), serde_json::json!({})),
            doc(permissions_default_observer(), serde_json::json!({})),
            doc(permissions_default_observer(), serde_json::json!({})),
        ],
    );
    let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
    let ctx = PermissionContext {
        user_id: recipient,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &out.ops[0] else {
        panic!("expected Update")
    };
    assert!(
        changes.is_empty(),
        "the commit-time snapshot's own recorded pointer must still redact, regardless of \
         what the CURRENT embedded array now holds at that index: {changes:?}"
    );
}

#[test]
fn linked_token_actor_owner_mutation_only_affects_the_current_time_half() {
    // `effective_owner_via` joins the CURRENT actor table via the caller-supplied closure;
    // mutating what it returns changes ONLY the current-time half's ownership resolution,
    // never the commit-time half's (`OpSnapshot::owner_at_commit`, frozen at commit).
    let world = crate::test_ids::test_uuid(9);
    let author = crate::test_ids::test_uuid(1);
    let token_id = crate::test_ids::test_uuid(2);
    let actor_id = crate::test_ids::test_uuid(50);
    let recipient = crate::test_ids::test_uuid(3);
    let mut perms = permissions_default_observer();
    perms
        .property_overrides
        .insert("/system/name".into(), Visibility::OwnerOrGm);
    let cmd = Command {
        seq: 1,
        world_id: world,
        author,
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: token_id,
            changes: vec![FieldChange {
                remove: false,
                path: "/system/name".into(),
                old: serde_json::Value::Null,
                new: serde_json::json!("Owner-visible name"),
            }],
        }],
    };
    // Commit-time: the recipient WAS the effective owner at commit (owner_at_commit is
    // Some(recipient)) — so the commit-time half admits this pointer regardless of what
    // the CURRENT actor_lookup later resolves. Isolates the mutation to the current-time
    // half only: union semantics mean a commit-time admission is never overridden into a
    // reveal, but a current-time denial always adds further hiding on top of it.
    let mut op = op_snapshot_update(
        Some(recipient),
        vec![("/system/name", Visibility::OwnerOrGm)],
        perms.clone(),
    );
    op.doc_type = "token".into();
    let snapshot = snapshot_one_op(op, HashMap::new());
    let mut token_doc = doc(perms, serde_json::json!({ "name": "Token PC" }));
    token_doc.doc_type = "token".into();
    token_doc.engine = Some(serde_json::json!({ "actor_id": actor_id.to_string() }));
    let current = HashMap::from([(token_id, current_doc(token_doc, 0))]);
    let ctx = PermissionContext {
        user_id: recipient,
        world_role: WorldRole::Player,
    };

    // actor_lookup resolves the recipient as the CURRENT linked actor's owner.
    let mut owning_actor = doc(PermissionSet::default(), serde_json::json!({}));
    owning_actor.id = actor_id;
    owning_actor.doc_type = "actor".into();
    owning_actor.owner = Some(recipient);
    let out_owner_now = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |id| {
            if *id == actor_id {
                Some(&owning_actor)
            } else {
                None
            }
        },
    );
    let Operation::Update { changes, .. } = &out_owner_now.ops[0] else {
        panic!("expected Update")
    };
    assert_eq!(
        changes.len(),
        1,
        "current-time ownership (via the actor_lookup closure) must admit OwnerOrGm now: {changes:?}"
    );

    // Same command, same snapshot — actor_lookup now resolves NO owner (mutate ONLY the
    // live input). The commit-time half still admits the pointer (unchanged), but the
    // current-time half now denies it, and denial from EITHER half hides a pointer.
    let out_no_owner = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &out_no_owner.ops[0] else {
        panic!("expected Update")
    };
    assert!(
        changes.is_empty(),
        "mutating ONLY the actor_lookup closure must change the CURRENT-time outcome: {changes:?}"
    );
}

#[test]
fn traversal_split_produces_byte_identical_output_for_the_same_document() {
    // The shared `(doc, prefix) -> Vec<(String, Visibility)>` traversal used by both the
    // live path (`collect_hidden`, via `hidden_from_overrides`) and snapshot construction
    // must be exactly the traversal `collect_overrides` performs — pinned here so a future
    // change to one cannot silently diverge from the other.
    let child = doc(
        perms_with(&[("/system/name", Visibility::OwnerOrGm)]),
        serde_json::json!({ "name": "Hidden" }),
    );
    let mut parent = doc(
        perms_with(&[("/system/secret", Visibility::GmOnly)]),
        serde_json::json!({ "secret": "S" }),
    );
    parent.embedded.insert("actor".into(), vec![child]);

    let mut overrides = Vec::new();
    collect_overrides(&parent, "", &mut overrides).unwrap();
    let pointers: std::collections::BTreeSet<&str> =
        overrides.iter().map(|(p, _)| p.as_str()).collect();
    assert!(pointers.contains("/system/secret"));
    assert!(pointers.contains("/base"));
    assert!(pointers.contains("/embedded/actor/0/system/name"));
    assert!(pointers.contains("/embedded/actor/0/base"));

    // hidden_from_overrides + collect_overrides together must reproduce collect_hidden's
    // own output exactly, for the same (doc, access).
    let mut via_collect_hidden = Vec::new();
    collect_hidden(&parent, &non_gm(), "", &mut via_collect_hidden).unwrap();
    let via_split = hidden_from_overrides(&overrides, &non_gm());
    let mut a: Vec<&str> = via_collect_hidden.iter().map(String::as_str).collect();
    let mut b: Vec<&str> = via_split.iter().map(String::as_str).collect();
    a.sort();
    b.sort();
    assert_eq!(
        a, b,
        "collect_hidden must equal collect_overrides + hidden_from_overrides"
    );
}

#[test]
fn world_cap_default_grant_rescues_read_at_both_halves_of_the_commit_current_conjunction() {
    // A recipient with NO document-level READ (default: DocRole::None, not owner, not
    // individually listed) but a world `WorldCapDefaults` grant of `cap::READ` for their
    // floored role must still receive the op — at BOTH the commit-time and current-time
    // halves of `filter_command`'s READ conjunction, since world capability GRANTS stay
    // current-only at both halves (never commit-snapshotted).
    let world = crate::test_ids::test_uuid(9);
    let author = crate::test_ids::test_uuid(1);
    let doc_id = crate::test_ids::test_uuid(2);
    let recipient = crate::test_ids::test_uuid(20);
    let denied = PermissionSet {
        default: DocRole::None,
        ..Default::default()
    };
    let cmd = field_change_update_cmd(
        world,
        author,
        doc_id,
        "/system/x",
        serde_json::json!(1),
        serde_json::json!(2),
    );
    let op = op_snapshot_update(None, vec![], denied.clone());
    let snapshot = snapshot_one_op(op, HashMap::new());
    let cur = doc(denied, serde_json::json!({ "x": 2 }));
    let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
    let ctx = PermissionContext {
        user_id: recipient,
        world_role: WorldRole::Player,
    };

    let mut world_defaults = WorldCapDefaults::default();
    world_defaults
        .all
        .by_role
        .entry(DocRole::None)
        .or_default()
        .insert(cap::READ.to_string());

    let out_with_grant = filter_command(&cmd, &snapshot, &ctx, &world_defaults, &current, |_| None);
    assert!(
        !out_with_grant.ops.is_empty(),
        "a world-cap-default READ grant must rescue an op the document's own \
         permissions deny, at both the commit-time and current-time halves"
    );

    // Negative control: the same recipient, same document, with NO world grant — the op
    // must still be dropped (confirms the rescue above is attributable to the grant, not
    // a change to the document/recipient setup).
    let out_no_grant = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    assert!(
        out_no_grant.ops.is_empty(),
        "with neither document access nor a world grant, the op must still be dropped"
    );
}
