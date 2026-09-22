use super::*;
use crate::data::document::{world_of, PermissionSet, Scope};
use crate::data::snapshot::CommandSnapshot;

// -------------------------------------------------------------------

/// `immediate_snapshot` with every Update op's pre-image permissions set to `before`.
fn snapshot_with_before<'a>(
    cmd: &Command,
    current: &HashMap<Uuid, CurrentDoc>,
    gm_at_commit: &[Uuid],
    actor_lookup: &impl Fn(&Uuid) -> Option<&'a Document>,
    before: PermissionSet,
) -> CommandSnapshot {
    let mut s = immediate_snapshot(cmd, current, gm_at_commit, actor_lookup);
    for op in s.per_op.iter_mut().flatten() {
        op.permissions_before_commit = Some(before.clone());
    }
    s
}

/// Sets every Update op's `owner_before_commit` in `snap` to `owner`, in place — for tests
/// exercising a same-op or standalone `/owner` reassignment's own READ transition (the
/// `TOKEN_DOC_TYPE` ownership floor case `owner_at_commit` cannot stand in for).
fn set_owner_before(snap: &mut CommandSnapshot, owner: Option<Uuid>) {
    for op in snap.per_op.iter_mut().flatten() {
        op.owner_before_commit = owner;
    }
}

#[tokio::test]
async fn reveal_by_permissions_synthesizes_a_create_for_the_newly_readable_recipient() {
    let player = crate::test_ids::test_uuid(1);
    let mut d = doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        serde_json::json!({ "hp": 3 }),
    );
    d.doc_type = "combatant".into();
    d.engine = crate::data::document::tests::default_test_engine("combatant");
    let cmd = Command {
        seq: 9,
        world_id: world_of(&d).unwrap(),
        author: crate::test_ids::test_uuid(99),
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: d.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/permissions/default".into(),
                old: serde_json::json!("none"),
                new: serde_json::json!("observer"),
            }],
        }],
    };
    let current = HashMap::from([(
        d.id,
        CurrentDoc {
            doc: d.clone(),
            created_seq: 1,
        },
    )]);
    let lookup = |_: &Uuid| None;
    let snap = snapshot_with_before(
        &cmd,
        &current,
        &[],
        &lookup,
        PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
    );
    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snap,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert_eq!(out.seq, 9);
    assert_eq!(out.ops.len(), 1);
    match &out.ops[0] {
        Operation::Create { doc } => {
            assert_eq!(doc.id, d.id);
            assert_eq!(doc.doc_type, "combatant");
            assert_eq!(doc.system["hp"], 3);
        }
        other => panic!("expected a synthesized Create, got {other:?}"),
    }
}

#[tokio::test]
async fn hide_by_permissions_synthesizes_a_stub_delete_for_the_recipient_losing_read() {
    let player = crate::test_ids::test_uuid(1);
    let mut d = doc(
        PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
        serde_json::json!({ "hp": 3 }),
    );
    d.doc_type = "combatant".into();
    d.engine = crate::data::document::tests::default_test_engine("combatant");
    d.name = Some("MOCK_NAME_A".into());
    let cmd = Command {
        seq: 10,
        world_id: world_of(&d).unwrap(),
        author: crate::test_ids::test_uuid(99),
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: d.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/permissions/default".into(),
                old: serde_json::json!("observer"),
                new: serde_json::json!("none"),
            }],
        }],
    };
    let current = HashMap::from([(
        d.id,
        CurrentDoc {
            doc: d.clone(),
            created_seq: 1,
        },
    )]);
    let lookup = |_: &Uuid| None;
    let snap = snapshot_with_before(
        &cmd,
        &current,
        &[],
        &lookup,
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
    );
    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snap,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert_eq!(out.ops.len(), 1);
    match &out.ops[0] {
        Operation::Delete { doc } => {
            assert_eq!(doc.id, d.id);
            assert_eq!(doc.doc_type, "combatant");
            // Stub: nothing the recipient may no longer see rides the Delete.
            assert!(doc.name.is_none());
            assert!(doc.engine.is_none());
            assert_eq!(doc.system, serde_json::json!({}));
            assert!(doc.embedded.is_empty());
            assert_eq!(doc.permissions, PermissionSet::default());
        }
        other => panic!("expected a synthesized Delete, got {other:?}"),
    }
}

#[tokio::test]
async fn a_gm_recipient_sees_the_plain_update_on_hide_and_reveal() {
    let gm = crate::test_ids::test_uuid(2);
    let mut d = doc(
        PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
        serde_json::json!({}),
    );
    d.doc_type = "combatant".into();
    d.engine = crate::data::document::tests::default_test_engine("combatant");
    let cmd = Command {
        seq: 11,
        world_id: world_of(&d).unwrap(),
        author: gm,
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: d.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/permissions/default".into(),
                old: serde_json::json!("observer"),
                new: serde_json::json!("none"),
            }],
        }],
    };
    let current = HashMap::from([(
        d.id,
        CurrentDoc {
            doc: d.clone(),
            created_seq: 1,
        },
    )]);
    let lookup = |_: &Uuid| None;
    let snap = snapshot_with_before(
        &cmd,
        &current,
        &[gm],
        &lookup,
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
    );
    let ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let out = filter_command(
        &cmd,
        &snap,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert!(matches!(&out.ops[0], Operation::Update { .. }));
}

#[tokio::test]
async fn a_reveal_is_not_synthesized_when_current_access_denies_read() {
    // Revealed at commit, hidden again later: the replayed earlier command must not
    // hand the recipient a document they may not currently see.
    let player = crate::test_ids::test_uuid(1);
    let mut d = doc(
        PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
        serde_json::json!({}),
    );
    d.doc_type = "combatant".into();
    d.engine = crate::data::document::tests::default_test_engine("combatant");
    let cmd = Command {
        seq: 12,
        world_id: world_of(&d).unwrap(),
        author: crate::test_ids::test_uuid(99),
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: d.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/permissions/default".into(),
                old: serde_json::json!("none"),
                new: serde_json::json!("observer"),
            }],
        }],
    };
    let current = HashMap::from([(
        d.id,
        CurrentDoc {
            doc: d.clone(),
            created_seq: 1,
        },
    )]);
    let lookup = |_: &Uuid| None;
    let mut snap = snapshot_with_before(
        &cmd,
        &current,
        &[],
        &lookup,
        PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
    );
    // Commit-time permissions were the revealing post-image (observer), current is none.
    for op in snap.per_op.iter_mut().flatten() {
        op.permissions_at_commit = Some(PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        });
    }
    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snap,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert!(out.ops.is_empty());
}

#[tokio::test]
async fn a_legacy_snapshot_without_before_permissions_keeps_the_old_behaviour() {
    let player = crate::test_ids::test_uuid(1);
    let mut d = doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        serde_json::json!({}),
    );
    d.doc_type = "combatant".into();
    d.engine = crate::data::document::tests::default_test_engine("combatant");
    let cmd = Command {
        seq: 13,
        world_id: world_of(&d).unwrap(),
        author: crate::test_ids::test_uuid(99),
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: d.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/permissions/default".into(),
                old: serde_json::json!("none"),
                new: serde_json::json!("observer"),
            }],
        }],
    };
    let current = HashMap::from([(
        d.id,
        CurrentDoc {
            doc: d.clone(),
            created_seq: 1,
        },
    )]);
    let lookup = |_: &Uuid| None;
    let snap = immediate_snapshot(&cmd, &current, &[], &lookup); // permissions_before_commit: None
    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snap,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert!(matches!(&out.ops[0], Operation::Update { .. }));
}

#[tokio::test]
async fn a_same_op_owner_and_permissions_change_still_reveals_to_the_new_owner() {
    // `doc_type: "combatant"` — NOT `TOKEN_DOC_TYPE`, so `effective_role`'s ownership floor
    // never engages and this transition is driven entirely by the `permissions.default`
    // change. Confirms the same-op `/owner` write is harmless for a doc_type where ownership
    // carries no capability weight; it does NOT exercise the ownership-floor transition
    // itself — see `a_token_reassigned_away_synthesizes_a_delete_for_the_old_owner`/
    // `a_token_reassigned_to_a_new_owner_synthesizes_a_create_from_ownership_alone` for that.
    let new_owner = crate::test_ids::test_uuid(1);
    // `d` carries the POST-image permissions (matching this Update's `new` values), the
    // same convention `reveal_by_permissions_synthesizes_a_create_for_the_newly_readable_recipient`
    // uses: `immediate_snapshot` derives `permissions_at_commit` from `d` itself.
    let mut d = doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        serde_json::json!({ "hp": 3 }),
    );
    d.doc_type = "combatant".into();
    d.engine = crate::data::document::tests::default_test_engine("combatant");
    d.owner = Some(new_owner);
    let cmd = Command {
        seq: 14,
        world_id: world_of(&d).unwrap(),
        author: crate::test_ids::test_uuid(99),
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: d.id,
            changes: vec![
                FieldChange {
                    remove: false,
                    path: "/owner".into(),
                    old: serde_json::Value::Null,
                    new: serde_json::json!(new_owner),
                },
                FieldChange {
                    remove: false,
                    path: "/permissions/default".into(),
                    old: serde_json::json!("none"),
                    new: serde_json::json!("observer"),
                },
            ],
        }],
    };
    let current = HashMap::from([(
        d.id,
        CurrentDoc {
            doc: d.clone(),
            created_seq: 1,
        },
    )]);
    let lookup = |_: &Uuid| None;
    let mut snap = snapshot_with_before(
        &cmd,
        &current,
        &[],
        &lookup,
        PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
    );
    for op in snap.per_op.iter_mut().flatten() {
        op.owner_at_commit = Some(new_owner);
    }
    let ctx = PermissionContext {
        user_id: new_owner,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snap,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert_eq!(out.ops.len(), 1);
    assert!(matches!(&out.ops[0], Operation::Create { .. }));
}

#[tokio::test]
async fn a_token_reassigned_away_synthesizes_a_delete_for_the_old_owner() {
    // `TOKEN_DOC_TYPE`'s ownership floor grants whole-document READ purely from
    // `effective_owner`, independent of `permissions.default` — which never changes here.
    // The old owner must receive a synthesized `Delete`, driven by ownership ALONE.
    let old_owner = crate::test_ids::test_uuid(1);
    let new_owner = crate::test_ids::test_uuid(2);
    let mut d = doc(
        PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
        serde_json::json!({}),
    );
    d.doc_type = TOKEN_DOC_TYPE.into();
    d.engine = crate::data::document::tests::default_test_engine(TOKEN_DOC_TYPE);
    // `d` (and `current`) carry the POST-image: the token now belongs to `new_owner`.
    d.owner = Some(new_owner);
    let cmd = Command {
        seq: 15,
        world_id: world_of(&d).unwrap(),
        author: crate::test_ids::test_uuid(99),
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: d.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/owner".into(),
                old: serde_json::json!(old_owner),
                new: serde_json::json!(new_owner),
            }],
        }],
    };
    let current = HashMap::from([(
        d.id,
        CurrentDoc {
            doc: d.clone(),
            created_seq: 1,
        },
    )]);
    let lookup = |_: &Uuid| None;
    // `permissions.default` is identical before and after (`None` throughout) — the
    // transition below comes from ownership alone.
    let mut snap = snapshot_with_before(
        &cmd,
        &current,
        &[],
        &lookup,
        PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
    );
    set_owner_before(&mut snap, Some(old_owner));
    let ctx = PermissionContext {
        user_id: old_owner,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snap,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert_eq!(out.ops.len(), 1);
    match &out.ops[0] {
        Operation::Delete { doc } => assert_eq!(doc.id, d.id),
        other => panic!("expected a synthesized Delete for the old owner, got {other:?}"),
    }
}

#[tokio::test]
async fn a_token_reassigned_to_a_new_owner_synthesizes_a_create_from_ownership_alone() {
    // Mirror of the Delete case above: `permissions.default` never changes, so the READ
    // transition is driven entirely by ownership, and the NEW owner must receive a
    // synthesized `Create`.
    let old_owner = crate::test_ids::test_uuid(1);
    let new_owner = crate::test_ids::test_uuid(2);
    let mut d = doc(
        PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
        serde_json::json!({ "hp": 4 }),
    );
    d.doc_type = TOKEN_DOC_TYPE.into();
    d.engine = crate::data::document::tests::default_test_engine(TOKEN_DOC_TYPE);
    d.owner = Some(new_owner);
    let cmd = Command {
        seq: 16,
        world_id: world_of(&d).unwrap(),
        author: crate::test_ids::test_uuid(99),
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: d.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/owner".into(),
                old: serde_json::json!(old_owner),
                new: serde_json::json!(new_owner),
            }],
        }],
    };
    let current = HashMap::from([(
        d.id,
        CurrentDoc {
            doc: d.clone(),
            created_seq: 1,
        },
    )]);
    let lookup = |_: &Uuid| None;
    let mut snap = snapshot_with_before(
        &cmd,
        &current,
        &[],
        &lookup,
        PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
    );
    set_owner_before(&mut snap, Some(old_owner));
    let ctx = PermissionContext {
        user_id: new_owner,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snap,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert_eq!(out.ops.len(), 1);
    match &out.ops[0] {
        Operation::Create { doc } => {
            assert_eq!(doc.id, d.id);
            assert_eq!(doc.system["hp"], 4);
        }
        other => panic!("expected a synthesized Create for the new owner, got {other:?}"),
    }
}

#[tokio::test]
async fn two_updates_to_the_same_doc_in_one_command_synthesize_exactly_one_transition() {
    // Two `Operation::Update`s targeting the SAME `doc_id` in one `Command`. Both share the
    // identical aggregate `permissions_before_commit`/`permissions_at_commit` (batch-start/
    // batch-end, captured once per doc_id — see `last_update_idx_for_doc`'s own comment), so
    // both resolve the SAME READ transition; only the LAST op may synthesize.
    let player = crate::test_ids::test_uuid(1);
    let mut d = doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        serde_json::json!({ "hp": 5 }),
    );
    d.doc_type = "combatant".into();
    d.engine = crate::data::document::tests::default_test_engine("combatant");
    let cmd = Command {
        seq: 17,
        world_id: world_of(&d).unwrap(),
        author: crate::test_ids::test_uuid(99),
        ts: 0,
        ops: vec![
            Operation::Update {
                doc_id: d.id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/system/hp".into(),
                    old: serde_json::json!(3),
                    new: serde_json::json!(5),
                }],
            },
            Operation::Update {
                doc_id: d.id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/permissions/default".into(),
                    old: serde_json::json!("none"),
                    new: serde_json::json!("observer"),
                }],
            },
        ],
    };
    let current = HashMap::from([(
        d.id,
        CurrentDoc {
            doc: d.clone(),
            created_seq: 1,
        },
    )]);
    let lookup = |_: &Uuid| None;
    let snap = snapshot_with_before(
        &cmd,
        &current,
        &[],
        &lookup,
        PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
    );
    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snap,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert_eq!(
        out.ops.len(),
        1,
        "exactly one synthesized entry, not one per same-doc_id Update op: {:?}",
        out.ops
    );
    assert!(matches!(&out.ops[0], Operation::Create { .. }));
}

#[test]
fn base_egress_is_cut_by_the_policy_the_snapshot_records() {
    // The stored base is the FULL template snapshot; the owner receives it
    // minus every pointer the snapshot's own recorded policy hides from them
    // (root `property_overrides`, record `propertyOverrides` at the record's
    // snapshot position), a GM receives it whole, and a non-owner receives no
    // base at all. The instance's OWN content overrides are not what cuts it.
    let owner = crate::test_ids::test_uuid(1);
    let other = crate::test_ids::test_uuid(2);
    let gm = crate::test_ids::test_uuid(3);
    let mut inst = doc(PermissionSet::default(), serde_json::json!({ "hp": 1 }));
    inst.owner = Some(owner);
    inst.base = Some(serde_json::json!({
        "name": "Tmpl",
        "engine": null,
        "system": { "hp": 1, "secret": "S", "note": "N", "public": "P" },
        "embedded": {
            "items": [{
                "sourceId": "tc",
                "name": "Kid",
                "engine": { "hp": 5 },
                "system": { "k": "K" },
                "embedded": {},
                "propertyOverrides": { "/engine/hp": "gm_only" }
            }]
        },
        "property_overrides": {
            "/system/secret": "gm_only",
            "/system/note": "owner_or_gm",
            "/name": "gm_only"
        },
        "owner_standing": "owner"
    }));

    let a_owner = resolve_access(owner, WorldRole::Player, &inst, inst.owner);
    let v = filter_properties(&inst, &a_owner).unwrap();
    let b = v.base.expect("the owner receives base");
    assert!(b["system"].get("secret").is_none(), "gm_only cut: {b}");
    assert_eq!(b["system"]["note"], "N", "owner_or_gm kept for the owner");
    assert_eq!(b["system"]["public"], "P");
    assert!(
        b.get("name").is_none(),
        "a hidden band key is stripped from the snapshot"
    );
    assert!(
        b["embedded"]["items"][0]["engine"].get("hp").is_none(),
        "a record's recorded policy cuts the record: {b}"
    );
    assert_eq!(b["embedded"]["items"][0]["system"]["k"], "K");

    let a_gm = resolve_access(gm, WorldRole::Gm, &inst, inst.owner);
    let vg = filter_properties(&inst, &a_gm).unwrap();
    assert_eq!(vg.base.unwrap()["system"]["secret"], "S");

    let a_other = resolve_access(other, WorldRole::Player, &inst, inst.owner);
    let vo = filter_properties(&inst, &a_other).unwrap();
    assert!(vo.base.is_none(), "a non-owner receives no base");
}

#[test]
fn base_egress_fails_closed_on_a_recorded_policy_it_cannot_act_on() {
    let owner = crate::test_ids::test_uuid(1);
    let mut inst = doc(PermissionSet::default(), serde_json::json!({}));
    inst.owner = Some(owner);
    let a_owner = resolve_access(owner, WorldRole::Player, &inst, inst.owner);
    for policy in [
        serde_json::json!({ "/system/secret": "not-a-tier" }),
        serde_json::json!({ "/permissions/default": "gm_only" }),
    ] {
        inst.base = Some(serde_json::json!({
            "name": null, "engine": null, "system": {}, "embedded": {},
            "property_overrides": policy
        }));
        assert!(filter_properties(&inst, &a_owner).is_err());
    }
}

/// A coalescing variant of `own_overrides`'s `/base` half: it treats an absent
/// `property_overrides` key as empty ("nothing hidden") instead of the real
/// function's fail-closed `serde_json::from_value::<StoredBase>` error. Exists ONLY
/// so a test can observe the exact disclosure the real fail-closed check prevents —
/// never called from production, and deliberately duplicated rather than
/// parameterized, since a shared toggle would risk the bug leaking back into the
/// real function.
fn legacy_coalescing_base_overrides(doc: &Document) -> Vec<(String, Visibility)> {
    let mut out = vec![("/base".to_string(), Visibility::OwnerOrGm)];
    let base = doc.base.as_ref().expect("test base is present");
    let obj = base.as_object().expect("test base is an object");
    let owner_standing: OwnerStanding = obj
        .get("owner_standing")
        .cloned()
        .map(|v| serde_json::from_value(v).expect("test owner_standing parses"))
        .unwrap_or(OwnerStanding::Owner);
    if owner_standing == OwnerStanding::Stranger {
        out.push(("/base".to_string(), Visibility::GmOnly));
    }
    let snapshot = MergeBase {
        name: obj.get("name").and_then(|v| v.as_str()).map(str::to_string),
        engine: obj
            .get("engine")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        system: obj
            .get("system")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        embedded: Default::default(),
        // THE BUG under test: a missing `property_overrides` key coalesces to "no
        // policy recorded" instead of failing the read closed.
        property_overrides: obj
            .get("property_overrides")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
    };
    base_policy(&snapshot, "/base", owner_standing, &mut out)
        .expect("test snapshot is well-formed");
    out
}

#[test]
fn base_egress_fails_closed_when_the_recorded_policy_map_is_absent_rather_than_leaking_the_field_it_would_have_hidden(
) {
    // A template hides `/system/secret` `gm_only`; that tier is recorded onto the
    // instance's OWN stored `base` snapshot (`merge::bands::recorded_overrides`/
    // `snapshot_base`) as its `property_overrides` entry. If that map is ever ABSENT
    // from the stored value — corruption, not a legitimate redacted view, since
    // `check_base_node_shape` requires the key present at every node — the instance's
    // non-GM OWNER (who passes `/base`'s hardcoded `OwnerOrGm` gate but is not the GM the
    // recorded tier hides the field from) must still never see `secret` through `/base`.
    // A `base_policy` that coalesced the absent map into "nothing hidden" would let
    // `/base/system/secret` reach this owner unredacted.
    let owner = crate::test_ids::test_uuid(1);
    let mut inst = doc(PermissionSet::default(), serde_json::json!({}));
    inst.owner = Some(owner);
    let a_owner = resolve_access(owner, WorldRole::Player, &inst, inst.owner);
    inst.base = Some(serde_json::json!({
        "name": null,
        "engine": null,
        "system": { "secret": "S" },
        "embedded": {},
        "owner_standing": "owner"
        // `property_overrides` deliberately absent: this is the shape a corrupted or
        // pre-validation row could carry despite ingest requiring the key.
    }));
    let result = filter_properties(&inst, &a_owner);
    assert!(
        result.is_err(),
        "an absent recorded-policy map must fail the whole read closed, not disclose \
         `secret` unredacted: {result:?}"
    );

    // Demonstrate the disclosure this closes, rather than asserting only that the
    // real path errors: drive the SAME document through the coalescing read path
    // and confirm `secret` actually reaches the non-GM owner unredacted there.
    let legacy_overrides = legacy_coalescing_base_overrides(&inst);
    let legacy_hidden = hidden_from_overrides(&legacy_overrides, &a_owner);
    let mut whole = serde_json::to_value(&inst).expect("test document serializes");
    redact_pointers(&mut whole, &legacy_hidden).expect("legacy hidden set classifies");
    assert_eq!(
        whole["base"]["system"]["secret"], "S",
        "the coalescing read path discloses `secret` to the non-GM owner: {whole}"
    );
}

#[test]
fn base_egress_fails_closed_when_a_records_embedded_map_is_absent() {
    // Same class one level down: an `EmbeddedBaseChild` record missing its own required
    // `embedded` key must fail the read rather than silently skipping recursion into
    // whatever the record's own recorded policy would otherwise have hidden.
    let owner = crate::test_ids::test_uuid(1);
    let mut inst = doc(PermissionSet::default(), serde_json::json!({}));
    inst.owner = Some(owner);
    let a_owner = resolve_access(owner, WorldRole::Player, &inst, inst.owner);
    inst.base = Some(serde_json::json!({
        "name": null, "engine": null, "system": {},
        "property_overrides": {}, "owner_standing": "owner",
        "embedded": {
            "items": [{
                "sourceId": "tc",
                "name": null,
                "engine": null,
                "system": { "secret": "S" },
                "propertyOverrides": { "/system/secret": "gm_only" }
                // `embedded` deliberately absent on this record.
            }]
        }
    }));
    assert!(filter_properties(&inst, &a_owner).is_err());
}

#[tokio::test]
async fn base_egress_redacts_the_update_delta_by_the_recorded_policy() {
    // The same `own_overrides` source feeds `filter_command`: a whole-band
    // `/base` change is stripped of the recorded hidden pointers for the
    // owner, exactly like a `/system` change is stripped of the document's
    // own hidden pointers. The instance is stamped from a template that
    // hides `/system/secret`, so its derived base records that policy.
    use crate::auth::role::ServerRole;
    use crate::data::command::WriteOrigin;
    use crate::data::sqlite::SqliteRepository;

    let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let owner = r
        .create_user("owner", None, ServerRole::User, 0)
        .await
        .unwrap();
    r.add_member(w.id, owner, WorldRole::Player).await.unwrap();
    let mut template_perms = perms_with(&[("/system/secret", Visibility::GmOnly)]);
    // `owner` needs whole-document READ on the template for its derived
    // `owner_standing` to be anything but `Stranger` — a `PermissionSet`'s
    // default role is `DocRole::None`.
    template_perms.default = DocRole::Observer;
    let mut template = doc(
        template_perms,
        serde_json::json!({ "hp": 1, "secret": "S1" }),
    );
    template.id = crate::test_ids::test_uuid(55);
    template.scope = Scope::World { world_id: w.id };
    template.owner = Some(gm);
    let mut d = doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        serde_json::json!({ "hp": 1 }),
    );
    d.scope = Scope::World { world_id: w.id };
    d.owner = Some(owner);
    d.source = Some(crate::data::document::Source {
        id: template.id,
        pack: None,
        version: 1,
    });
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![
            Operation::Create { doc: template },
            Operation::Create { doc: d.clone() },
        ],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let stored = r.get_document(d.id).await.unwrap().unwrap();
    let old_base = stored.base.clone().expect("derived base");
    assert_eq!(old_base["property_overrides"]["/system/secret"], "gm_only");
    let mut new_base = old_base.clone();
    new_base["system"] = serde_json::json!({ "hp": 2, "secret": "S2" });

    let cmd = Command {
        seq: 2,
        world_id: w.id,
        author: gm,
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: d.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/base".into(),
                old: old_base,
                new: new_base,
            }],
        }],
    };
    let current = load_current_docs(&r, &cmd).await;
    let snapshot = immediate_snapshot(&cmd, &current, &[gm], &|_| None);
    let ctx = PermissionContext {
        user_id: owner,
        world_role: WorldRole::Player,
    };
    let view = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &view.ops[0] else {
        panic!("the owner keeps the Update");
    };
    let base = &changes
        .iter()
        .find(|c| c.path == "/base")
        .expect("owner receives /base")
        .new;
    assert!(base["system"].get("secret").is_none(), "{base}");
    assert_eq!(base["system"]["hp"], 2);
}

#[test]
fn writes_a_content_band_agrees_with_the_clients_ismergeablebandpointer_on_every_corpus_case() {
    // Shared conformance fixture, read on both sides: this file
    // (`include_str!`) and the client's `isMergeableBandPointer` test
    // (`readFileSync`). One corpus, so the two classifiers cannot silently
    // drift apart on which pointers name a mergeable band.
    const CORPUS: &str = include_str!(
        "../../../../../client/core/src/__fixtures__/mergeable-band-pointer-conformance.json"
    );
    #[derive(serde::Deserialize)]
    struct Case {
        pointer: String,
        mergeable: bool,
    }
    #[derive(serde::Deserialize)]
    struct Corpus {
        cases: Vec<Case>,
    }
    let corpus: Corpus =
        serde_json::from_str(CORPUS).expect("mergeable-band-pointer-conformance.json parses");
    for case in corpus.cases {
        assert_eq!(
            writes_a_content_band(&case.pointer),
            case.mergeable,
            "{}",
            case.pointer
        );
    }
}

#[test]
fn an_empty_trailing_segment_is_refused_at_both_the_write_and_egress_classifiers() {
    // `writes_a_content_band` names no segment past a bare trailing separator, so
    // `/system/` is refused there directly; the sibling top-level classifier
    // `redaction_target` refuses `/system/` itself via its own `!tail.is_empty()`
    // guard, and — because `is_base_content_residual` is derived from
    // `writes_a_content_band` — a `/base/system/` pointer now agrees with it
    // rather than disagreeing on the same malformed shape.
    assert!(!writes_a_content_band("/system/"));
    assert!(!writes_a_content_band("/engine/"));
    assert_eq!(redaction_target("/system/"), None);
    assert_eq!(redaction_target("/base/system/"), None);
}

#[test]
fn carried_light_ancestor_echo_with_shifted_number_variant_is_not_a_touch() {
    // Stored emission carries whole-number `f64` radii (Float variant); a JS client's
    // faithful echo comes back PosInt. The ancestor-write comparison must treat the
    // echo as unchanged, or a non-GM whole-`/engine/overrides` write touching only a
    // SIBLING field is refused on a pure representation mismatch.
    let whole = serde_json::json!({
        "engine": { "x": 0.0, "overrides": { "light": {
            "color": "#ffaa00", "intensity": 1.0, "brightRadius": 4.0,
            "dimRadius": 8.0, "falloff": null, "enabled": true
        }, "vision": null } }
    });
    let echoed = serde_json::json!({
        "light": {
            "color": "#ffaa00", "intensity": 1, "brightRadius": 4,
            "dimRadius": 8, "falloff": null, "enabled": true
        },
        "vision": null
    });
    assert!(!carried_light_touched(
        "token",
        "/engine/overrides",
        false,
        &whole,
        &echoed
    ));
    // Control: a genuine change under the same ancestor write still counts.
    let changed = serde_json::json!({
        "light": {
            "color": "#ffaa00", "intensity": 1, "brightRadius": 5,
            "dimRadius": 8, "falloff": null, "enabled": true
        },
        "vision": null
    });
    assert!(carried_light_touched(
        "token",
        "/engine/overrides",
        false,
        &whole,
        &changed
    ));
}

/// One Move op over a doc whose COMMIT-time and CURRENT permission sets are
/// supplied independently, filtered for `recipient`. `current_created_seq`
/// lets a test diverge the id's generation from the snapshot's.
fn filter_move_with(
    commit_perms: PermissionSet,
    current_perms: PermissionSet,
    recipient: &PermissionContext,
    current_created_seq: i64,
) -> Command {
    let world = crate::test_ids::test_uuid(9);
    let target = crate::test_ids::test_uuid(41);
    let mut at_commit = doc(commit_perms, serde_json::json!({}));
    at_commit.id = target;
    at_commit.scope = Scope::World { world_id: world };
    let mut now = doc(current_perms, serde_json::json!({}));
    now.id = target;
    now.scope = Scope::World { world_id: world };
    now.parent_id = Some(crate::test_ids::test_uuid(42));

    let cmd = Command {
        seq: 5,
        world_id: world,
        author: crate::test_ids::test_uuid(1),
        ts: 0,
        ops: vec![Operation::Move {
            doc_id: target,
            parent_id: Some(crate::test_ids::test_uuid(42)),
            old_parent_id: None,
        }],
    };
    let commit_current: HashMap<Uuid, CurrentDoc> =
        [(target, current_doc(at_commit, 1))].into_iter().collect();
    let mut snapshot = immediate_snapshot(&cmd, &commit_current, &[], &|_| None);
    // `immediate_snapshot` records no generation; stamp the commit-time one so
    // the recreated-id guard is exercisable.
    if let Some(Some(s)) = snapshot.per_op.get_mut(0) {
        s.created_seq_at_commit = Some(1);
    }
    let live_current: HashMap<Uuid, CurrentDoc> = [(target, current_doc(now, current_created_seq))]
        .into_iter()
        .collect();
    filter_command(
        &cmd,
        &snapshot,
        recipient,
        &WorldCapDefaults::default(),
        &live_current,
        |_| None,
    )
}

#[test]
fn move_is_delivered_verbatim_iff_read_holds_at_commit_and_now() {
    let player = PermissionContext {
        user_id: crate::test_ids::test_uuid(77),
        world_role: WorldRole::Player,
    };
    let readable = PermissionSet {
        default: DocRole::Observer,
        ..Default::default()
    };
    let out = filter_move_with(readable.clone(), readable, &player, 1);
    // Exactly the Move op, unmodified — and NOTHING synthesized: READ never
    // consults `parent_id` (`resolve_access_world`), so a Move alone can
    // produce no READ transition for any recipient.
    assert_eq!(out.ops.len(), 1);
    assert!(matches!(
        &out.ops[0],
        Operation::Move { doc_id, parent_id: Some(p), old_parent_id: None }
            if *doc_id == crate::test_ids::test_uuid(41) && *p == crate::test_ids::test_uuid(42)
    ));
}

#[test]
fn move_is_dropped_without_commit_time_read() {
    let player = PermissionContext {
        user_id: crate::test_ids::test_uuid(77),
        world_role: WorldRole::Player,
    };
    let denied = PermissionSet {
        default: DocRole::None,
        ..Default::default()
    };
    let readable = PermissionSet {
        default: DocRole::Observer,
        ..Default::default()
    };
    let out = filter_move_with(denied, readable, &player, 1);
    assert!(out.ops.is_empty());
}

#[test]
fn move_is_dropped_without_current_read() {
    let player = PermissionContext {
        user_id: crate::test_ids::test_uuid(77),
        world_role: WorldRole::Player,
    };
    let denied = PermissionSet {
        default: DocRole::None,
        ..Default::default()
    };
    let readable = PermissionSet {
        default: DocRole::Observer,
        ..Default::default()
    };
    let out = filter_move_with(readable, denied, &player, 1);
    assert!(out.ops.is_empty());
}

#[test]
fn move_for_a_recreated_id_generation_is_dropped() {
    let player = PermissionContext {
        user_id: crate::test_ids::test_uuid(77),
        world_role: WorldRole::Player,
    };
    let readable = PermissionSet {
        default: DocRole::Observer,
        ..Default::default()
    };
    // The current row's generation differs from the snapshot's: the id was
    // hard-deleted and recreated since commit.
    let out = filter_move_with(readable.clone(), readable, &player, 2);
    assert!(out.ops.is_empty());
}

#[test]
fn move_with_a_legacy_recorded_command_is_dropped_on_replay() {
    let player = PermissionContext {
        user_id: crate::test_ids::test_uuid(77),
        world_role: WorldRole::Player,
    };
    let world = crate::test_ids::test_uuid(9);
    let target = crate::test_ids::test_uuid(41);
    let mut now = doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        serde_json::json!({}),
    );
    now.id = target;
    now.scope = Scope::World { world_id: world };
    let cmd = Command {
        seq: 5,
        world_id: world,
        author: crate::test_ids::test_uuid(1),
        ts: 0,
        ops: vec![Operation::Move {
            doc_id: target,
            parent_id: None,
            old_parent_id: Some(crate::test_ids::test_uuid(42)),
        }],
    };
    // A legacy `world_events` row: no recorded per-op snapshot at all.
    let snapshot = CommandSnapshot {
        per_op: vec![None],
        world_gm_at_commit: Default::default(),
    };
    let live: HashMap<Uuid, CurrentDoc> = [(target, current_doc(now, 1))].into_iter().collect();
    let out = filter_command(
        &cmd,
        &snapshot,
        &player,
        &WorldCapDefaults::default(),
        &live,
        |_| None,
    );
    assert!(out.ops.is_empty());
}
