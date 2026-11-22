use super::*;
use crate::data::document::{PermissionSet, Scope};

// ---- effective_owner: the single token-ownership rule ----

fn token_linked_to(actor_id: Option<Uuid>) -> Document {
    let mut d = doc(PermissionSet::default(), serde_json::json!({}));
    d.id = crate::test_ids::test_uuid(100);
    d.doc_type = "token".into();
    d.engine = Some(match actor_id {
        Some(a) => serde_json::json!({
            "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0, "rotation": 0.0,
            "actor_id": a.to_string()
        }),
        None => serde_json::json!({
            "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0, "rotation": 0.0
        }),
    });
    d
}

fn actor_owned_by(id: Uuid, owner: Option<Uuid>) -> Document {
    let mut d = doc(PermissionSet::default(), serde_json::json!({}));
    d.id = id;
    d.owner = owner;
    d
}

#[test]
fn token_actor_link_reads_only_a_tokens_engine_actor_id() {
    let a = crate::test_ids::test_uuid(42);
    assert_eq!(token_actor_link(&token_linked_to(Some(a))), Some(a));
    // A raw/instanced token carries no link.
    assert_eq!(token_actor_link(&token_linked_to(None)), None);
    // A non-token doc_type never links, even with a stray `actor_id` key.
    let mut impostor = token_linked_to(Some(a));
    impostor.doc_type = "actor".into();
    assert_eq!(token_actor_link(&impostor), None);
}

#[test]
fn effective_owner_prefers_the_per_token_override() {
    let actor_id = crate::test_ids::test_uuid(42);
    let inheritor = crate::test_ids::test_uuid(1);
    let override_user = crate::test_ids::test_uuid(2);
    let actor = actor_owned_by(actor_id, Some(inheritor));

    // No override: inherits the linked actor's owner.
    let plain = token_linked_to(Some(actor_id));
    assert_eq!(effective_owner(&plain, Some(&actor)), Some(inheritor));

    // Override set: it wins over the same actor, same link.
    let mut overridden = token_linked_to(Some(actor_id));
    overridden.owner = Some(override_user);
    assert_eq!(
        effective_owner(&overridden, Some(&actor)),
        Some(override_user)
    );
}

#[test]
fn effective_owner_fails_closed_on_degenerate_links() {
    let actor_id = crate::test_ids::test_uuid(42);
    let player = crate::test_ids::test_uuid(1);

    // No link, no override.
    assert_eq!(effective_owner(&token_linked_to(None), None), None);
    // Dangling link: the actor row does not exist.
    assert_eq!(
        effective_owner(&token_linked_to(Some(actor_id)), None),
        None
    );
    // Linked to an actor that nobody owns.
    assert_eq!(
        effective_owner(
            &token_linked_to(Some(actor_id)),
            Some(&actor_owned_by(actor_id, None))
        ),
        None
    );
    // A `linked_actor` that is NOT the document the link names is rejected
    // rather than trusted — a mis-joined caller under-permits, never leaks
    // write authority to the wrong actor's owner.
    assert_eq!(
        effective_owner(
            &token_linked_to(Some(actor_id)),
            Some(&actor_owned_by(
                crate::test_ids::test_uuid(999),
                Some(player)
            ))
        ),
        None
    );
    // Same, for a correctly-identified document of the wrong doc_type.
    let mut wrong_type = actor_owned_by(actor_id, Some(player));
    wrong_type.doc_type = "token".into();
    assert_eq!(
        effective_owner(&token_linked_to(Some(actor_id)), Some(&wrong_type)),
        None
    );
    // Control: the same call with the correctly-joined owned actor resolves,
    // so the rejections above are the guards, not a constant `None`.
    assert_eq!(
        effective_owner(
            &token_linked_to(Some(actor_id)),
            Some(&actor_owned_by(actor_id, Some(player)))
        ),
        Some(player)
    );
}

#[test]
fn effective_owner_rejects_a_cross_scope_actor() {
    // A candidate from another scope is an illegitimate join, same class as a
    // wrong-id or wrong-type candidate: fail closed to no owner.
    let actor_id = crate::test_ids::test_uuid(42);
    let mut token = token_linked_to(Some(actor_id));
    token.scope = Scope::World {
        world_id: crate::test_ids::test_uuid(1000),
    };
    let mut foreign = actor_owned_by(actor_id, Some(crate::test_ids::test_uuid(1)));
    foreign.scope = Scope::World {
        world_id: crate::test_ids::test_uuid(2000),
    };
    assert_eq!(effective_owner(&token, Some(&foreign)), None);

    // Same scope still resolves.
    let mut same = actor_owned_by(actor_id, Some(crate::test_ids::test_uuid(1)));
    same.scope = token.scope.clone();
    assert_eq!(
        effective_owner(&token, Some(&same)),
        Some(crate::test_ids::test_uuid(1))
    );
}

#[test]
fn a_non_token_never_inherits_ownership() {
    let actor_id = crate::test_ids::test_uuid(42);
    let player = crate::test_ids::test_uuid(1);
    let mut not_a_token = token_linked_to(Some(actor_id));
    not_a_token.doc_type = "drawing".into();
    assert_eq!(
        effective_owner(&not_a_token, Some(&actor_owned_by(actor_id, Some(player)))),
        None,
        "inheritance is token-scoped: no other doc_type joins an actor"
    );
}

#[test]
fn effective_ownership_grants_the_owner_floor_and_the_owner_or_gm_tier() {
    let actor_id = crate::test_ids::test_uuid(42);
    let player = crate::test_ids::test_uuid(1);
    let stranger = crate::test_ids::test_uuid(2);
    let mut token = token_linked_to(Some(actor_id));
    // The shipping `buildTokenDoc` default: READ-only for everyone.
    token.permissions.default = DocRole::Observer;
    let actor = actor_owned_by(actor_id, Some(player));
    let owner = effective_owner(&token, Some(&actor));

    let a_player = resolve_access(player, WorldRole::Player, &token, owner);
    assert!(
        a_player.has(cap::READ) && a_player.has(cap::WRITE_FIELDS),
        "an effective owner holds the DocRole::Owner floor"
    );
    assert!(
        !a_player.has(cap::EDIT_PERMISSIONS) && !a_player.has(cap::DELETE),
        "the BUILT-IN floor stops at WRITE_FIELDS — no re-assigning or deleting. \
         Additive `by_role[Owner]` grants can widen past it; see \
         `world_by_role_owner_grants_reach_an_inheriting_owner`"
    );
    assert!(
        a_player.is_owner && a_player.can_see(&Visibility::OwnerOrGm),
        "redaction's OwnerOrGm tier admits the same effective owner"
    );
    assert!(
        !a_player.can_see(&Visibility::GmOnly),
        "an owner is not a GM"
    );

    // Non-vacuity: same token, same call, different user.
    let a_stranger = resolve_access(stranger, WorldRole::Player, &token, owner);
    assert!(a_stranger.has(cap::READ) && !a_stranger.has(cap::WRITE_FIELDS));
    assert!(!a_stranger.is_owner);
}

#[test]
fn the_owner_floor_never_downgrades_a_stronger_document_grant() {
    // A doc that already grants a user Owner keeps it when they are NOT the
    // effective owner: the floor only ever strengthens.
    let player = crate::test_ids::test_uuid(1);
    let mut token = token_linked_to(None);
    token.permissions.users.insert(player, DocRole::Owner);
    let a = resolve_access(player, WorldRole::Player, &token, None);
    assert!(a.has(cap::WRITE_FIELDS));
    assert!(
        !a.is_owner,
        "no effective owner => not the OwnerOrGm subject"
    );
}

#[test]
fn world_by_role_owner_grants_reach_an_inheriting_owner() {
    use crate::data::document::CapabilityGrants;
    // The owner floor sets the ROLE, and that role also selects additive
    // capability grants — so an INHERITING owner receives `by_role[Owner]`
    // exactly as a stamped `permissions.users[user] = Owner` would. That
    // equivalence is the point: inherited and stamped ownership must not
    // diverge. A deployment that puts EDIT_PERMISSIONS in `by_role[Owner]`
    // is choosing to hand it to every Owner, inheriting ones included; this
    // test documents that as intended, not as an accident of the floor.
    let actor_id = crate::test_ids::test_uuid(42);
    let player = crate::test_ids::test_uuid(1);
    let stranger = crate::test_ids::test_uuid(2);
    let mut token = token_linked_to(Some(actor_id));
    token.permissions.default = DocRole::Observer;
    let actor = actor_owned_by(actor_id, Some(player));
    let owner = effective_owner(&token, Some(&actor));

    let mut world_grants = CapabilityGrants::default();
    world_grants
        .by_role
        .entry(DocRole::Owner)
        .or_default()
        .insert(cap::EDIT_PERMISSIONS.to_string());

    let inheriting = resolve_access_world(player, WorldRole::Player, &token, &world_grants, owner);
    assert!(
        inheriting.has(cap::EDIT_PERMISSIONS),
        "a world by_role[Owner] grant reaches an owner who inherits through the actor link"
    );

    // Equivalence leg: a STAMPED owner on the same doc, with no effective
    // owner at all, resolves the identical grant — the two paths agree.
    let mut stamped = token_linked_to(None);
    stamped.permissions.default = DocRole::Observer;
    stamped.permissions.users.insert(player, DocRole::Owner);
    assert!(resolve_access_world(
        player,
        WorldRole::Player,
        &stamped,
        &world_grants,
        stamped.owner
    )
    .has(cap::EDIT_PERMISSIONS));

    // Non-vacuity: the grant is role-selected, not unconditional — a
    // non-owner on the same document with the same world grants gets nothing.
    assert!(
        !resolve_access_world(stranger, WorldRole::Player, &token, &world_grants, owner)
            .has(cap::EDIT_PERMISSIONS)
    );
}

// ---- filter_command joins the effective owner (egress hot path) ----

#[tokio::test]
async fn filter_command_admits_the_inheriting_owner_of_a_linked_token() {
    // token: permissions.default = None, owner = None, linked to an actor owned
    // by P. Literal-owner egress treated P as a stranger (op dropped); the
    // effective join must now deliver Create/Update/Delete AND OwnerOrGm-tier
    // content to P, while a true stranger still receives nothing. A document
    // P can write (owner floor at apply_intent) is one P receives.
    let p = crate::test_ids::test_uuid(1);
    let stranger = crate::test_ids::test_uuid(2);
    let actor_id = crate::test_ids::test_uuid(42);
    let actor = actor_owned_by(actor_id, Some(p));
    let mut token = token_linked_to(Some(actor_id));
    token.permissions.default = DocRole::None;
    token
        .permissions
        .property_overrides
        .insert("/system/notes".into(), Visibility::OwnerOrGm);

    let cmd = Command {
        seq: 1,
        world_id: crate::test_ids::test_uuid(7),
        author: crate::test_ids::test_uuid(9),
        ts: 0,
        ops: vec![Operation::Create { doc: token.clone() }],
    };
    let lookup = |id: &Uuid| (id == &actor.id).then_some(&actor);
    let current: HashMap<Uuid, CurrentDoc> = HashMap::new();
    let snapshot = immediate_snapshot(&cmd, &current, &[], &lookup);

    let p_ctx = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &p_ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert_eq!(out.ops.len(), 1, "inheriting owner must RECEIVE the create");

    let s_ctx = PermissionContext {
        user_id: stranger,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &s_ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert!(
        out.ops.is_empty(),
        "a stranger still receives nothing (fail closed)"
    );

    // Without the actor join (dangling source) the op is withheld even from P:
    // degenerate input under-permits, never over-permits.
    let out = filter_command(
        &cmd,
        &snapshot,
        &p_ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    assert!(out.ops.is_empty());
}

#[tokio::test]
async fn filter_command_update_keeps_owner_or_gm_changes_for_the_inheriting_owner() {
    use crate::auth::role::ServerRole;
    use crate::data::command::{Command, FieldChange, Operation, WriteOrigin};
    use crate::data::membership::PermissionContext;
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
    let p = r.create_user("p", None, ServerRole::User, 0).await.unwrap();

    let mut actor = doc(PermissionSet::default(), serde_json::json!({}));
    actor.id = crate::test_ids::test_uuid(42);
    actor.scope = Scope::World { world_id: w.id };
    actor.owner = Some(p);

    let mut token = doc(
        PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
        serde_json::json!({ "notes": "secret plan" }),
    );
    token.doc_type = "token".into();
    token.id = crate::test_ids::test_uuid(100);
    token.scope = Scope::World { world_id: w.id };
    token.engine = Some(serde_json::json!({
        "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0, "rotation": 0.0,
        "actor_id": actor.id.to_string()
    }));
    token
        .permissions
        .property_overrides
        .insert("/system/notes".into(), Visibility::OwnerOrGm);

    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![
            Operation::Create { doc: actor.clone() },
            Operation::Create { doc: token.clone() },
        ],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let cmd = Command {
        seq: 2,
        world_id: w.id,
        author: gm,
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: token.id,
            changes: vec![
                FieldChange {
                    remove: false,
                    path: "/system/notes".into(),
                    old: serde_json::json!("secret plan"),
                    new: serde_json::json!("new plan"),
                },
                FieldChange {
                    remove: false,
                    path: "/base".into(),
                    old: serde_json::Value::Null,
                    new: serde_json::json!({ "system": { "notes": "template" } }),
                },
            ],
        }],
    };

    let p_ctx = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };
    let current = load_current_docs(&r, &cmd).await;
    let lookup = |id: &Uuid| (id == &actor.id).then_some(&actor);
    let snapshot = immediate_snapshot(&cmd, &current, &[], &lookup);
    let out = filter_command(
        &cmd,
        &snapshot,
        &p_ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    let Operation::Update { changes, .. } = &out.ops[0] else {
        panic!("expected Update");
    };
    assert_eq!(
        changes.len(),
        2,
        "the inheriting owner keeps both the OwnerOrGm /system/notes change and /base"
    );

    let stranger_ctx = PermissionContext {
        user_id: crate::test_ids::test_uuid(999),
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &stranger_ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert!(
        out.ops.is_empty(),
        "a stranger receives no READ on a default:none token, even via the actor join"
    );
}

// ---- write-receive parity + adversarial egress ownership ----

#[tokio::test]
async fn a_document_you_can_write_is_a_document_you_receive() {
    // A document a user can WRITE (the owner floor grants WRITE_FIELDS at
    // `apply_intent`) must also be a document that user RECEIVES at egress
    // (the same owner floor, joined through the same `effective_owner`
    // rule at `filter_command`) — write authz and read authz resolve
    // ownership through one shared join, never two. Reuses the persisted
    // actor+linked-token arrangement from
    // `filter_command_update_keeps_owner_or_gm_changes_for_the_inheriting_owner`.
    use crate::auth::role::ServerRole;
    use crate::data::command::{FieldChange, Operation, WriteOrigin};
    use crate::data::membership::PermissionContext;
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
    let p = r.create_user("p", None, ServerRole::User, 0).await.unwrap();

    let mut actor = doc(PermissionSet::default(), serde_json::json!({}));
    actor.id = crate::test_ids::test_uuid(42);
    actor.scope = Scope::World { world_id: w.id };
    actor.owner = Some(p);

    let mut token = doc(
        PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
        serde_json::json!({ "notes": "secret plan" }),
    );
    token.doc_type = "token".into();
    token.id = crate::test_ids::test_uuid(100);
    token.scope = Scope::World { world_id: w.id };
    token.engine = Some(serde_json::json!({
        "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0, "rotation": 0.0,
        "actor_id": actor.id.to_string()
    }));
    token
        .permissions
        .property_overrides
        .insert("/system/notes".into(), Visibility::OwnerOrGm);

    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![
            Operation::Create { doc: actor.clone() },
            Operation::Create { doc: token.clone() },
        ],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let p_ctx = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };

    // 1. apply_intent as P: patches /system/notes on a `default: None` token
    //    it does not literally own. Must SUCCEED — the owner floor (via the
    //    actor link) grants WRITE_FIELDS.
    let cmd = r
        .apply_intent(
            &p_ctx,
            w.id,
            vec![Operation::Update {
                doc_id: token.id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/system/notes".into(),
                    old: serde_json::json!("secret plan"),
                    new: serde_json::json!("new plan"),
                }],
            }],
            2,
            WriteOrigin::Client,
        )
        .await
        .expect("owner floor grants WRITE_FIELDS: the patch must succeed")
        .command;

    // 2. filter_command of the returned command for P, joined through the
    //    same actor link, must RETAIN the op — the owner floor also grants
    //    READ at egress through the same owner value.
    let current = load_current_docs(&r, &cmd).await;
    let lookup = |id: &Uuid| (id == &actor.id).then_some(&actor);
    let snapshot = immediate_snapshot(&cmd, &current, &[], &lookup);
    let out_p = filter_command(
        &cmd,
        &snapshot,
        &p_ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert_eq!(
        out_p.ops.len(),
        1,
        "the writer must also receive the write it just made"
    );

    // 3. A true stranger receives nothing.
    let stranger_ctx = PermissionContext {
        user_id: crate::test_ids::test_uuid(999),
        world_role: WorldRole::Player,
    };
    let out_stranger = filter_command(
        &cmd,
        &snapshot,
        &stranger_ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert!(
        out_stranger.ops.is_empty(),
        "a stranger receives nothing (fail closed)"
    );
}

#[test]
fn egress_ownership_ignores_a_cross_scope_actor() {
    // The scope check in `effective_owner`, exercised through the egress
    // join: a linked actor from a DIFFERENT scope must not be treated as
    // the token's owner at `filter_command`, even though the linked id
    // matches.
    use crate::data::command::{Command, Operation};
    use crate::data::membership::PermissionContext;

    let p = crate::test_ids::test_uuid(1);
    let actor_id = crate::test_ids::test_uuid(42);
    let mut token = token_linked_to(Some(actor_id));
    token.permissions.default = DocRole::None;
    token.scope = Scope::World {
        world_id: crate::test_ids::test_uuid(1000),
    };

    let mut foreign_actor = actor_owned_by(actor_id, Some(p));
    foreign_actor.scope = Scope::World {
        world_id: crate::test_ids::test_uuid(2000),
    };

    let cmd = Command {
        seq: 1,
        world_id: crate::test_ids::test_uuid(7),
        author: crate::test_ids::test_uuid(9),
        ts: 0,
        ops: vec![Operation::Create { doc: token.clone() }],
    };
    let lookup = |id: &Uuid| (id == &foreign_actor.id).then_some(&foreign_actor);
    let current: HashMap<Uuid, CurrentDoc> = HashMap::new();
    let snapshot = immediate_snapshot(&cmd, &current, &[], &lookup);

    let p_ctx = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &p_ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert!(
        out.ops.is_empty(),
        "a cross-scope actor join must not be treated as the owner at egress"
    );
}

#[test]
fn egress_ownership_honors_the_per_token_override() {
    // token.owner = A, linked actor owned by B: the per-token override wins
    // over the actor join, the same precedence the write path
    // (`effective_owner`) uses — A receives, B does not.
    use crate::data::command::{Command, Operation};
    use crate::data::membership::PermissionContext;

    let a = crate::test_ids::test_uuid(1);
    let b = crate::test_ids::test_uuid(2);
    let actor_id = crate::test_ids::test_uuid(42);
    let actor = actor_owned_by(actor_id, Some(b));
    let mut token = token_linked_to(Some(actor_id));
    token.permissions.default = DocRole::None;
    token.owner = Some(a);

    let cmd = Command {
        seq: 1,
        world_id: crate::test_ids::test_uuid(7),
        author: crate::test_ids::test_uuid(9),
        ts: 0,
        ops: vec![Operation::Create { doc: token.clone() }],
    };
    let lookup = |id: &Uuid| (id == &actor.id).then_some(&actor);
    let current: HashMap<Uuid, CurrentDoc> = HashMap::new();
    let snapshot = immediate_snapshot(&cmd, &current, &[], &lookup);

    let a_ctx = PermissionContext {
        user_id: a,
        world_role: WorldRole::Player,
    };
    let out_a = filter_command(
        &cmd,
        &snapshot,
        &a_ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert_eq!(
        out_a.ops.len(),
        1,
        "the per-token override wins over the actor join: A receives"
    );

    let b_ctx = PermissionContext {
        user_id: b,
        world_role: WorldRole::Player,
    };
    let out_b = filter_command(
        &cmd,
        &snapshot,
        &b_ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert!(
        out_b.ops.is_empty(),
        "the override wins over the actor: B (the linked actor's literal owner) does not receive"
    );
}

#[test]
fn egress_gm_and_gm_role_cap_are_unchanged() {
    // The owner-join plumbing through `filter_command` must not widen the
    // `gm_role` cap: a plain doc still delivers everything to
    // the GM, but a `gm_role: Some(DocRole::None)` doc (message-style,
    // e.g. a whisper) still drops the capped GM's op entirely.
    use crate::data::command::{Command, Operation};
    use crate::data::membership::PermissionContext;

    let gm = crate::test_ids::test_uuid(1);
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let current: HashMap<Uuid, CurrentDoc> = HashMap::new();

    // Plain doc: the GM still receives everything unconditionally.
    let plain = doc(PermissionSet::default(), serde_json::json!({ "hp": 10 }));
    let cmd_plain = Command {
        seq: 1,
        world_id: crate::test_ids::test_uuid(7),
        author: crate::test_ids::test_uuid(9),
        ts: 0,
        ops: vec![Operation::Create { doc: plain.clone() }],
    };
    let snapshot_plain = immediate_snapshot(&cmd_plain, &current, &[gm], &|_| None);
    let out_plain = filter_command(
        &cmd_plain,
        &snapshot_plain,
        &gm_ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    assert_eq!(
        out_plain.ops.len(),
        1,
        "an uncapped GM still receives everything"
    );

    // `gm_role: Some(DocRole::None)` doc: the capped GM's op is still
    // dropped entirely — the owner plumb must not have widened this cap.
    let mut capped = doc(PermissionSet::default(), serde_json::json!({}));
    capped.permissions.gm_role = Some(DocRole::None);
    let cmd_capped = Command {
        seq: 2,
        world_id: crate::test_ids::test_uuid(7),
        author: crate::test_ids::test_uuid(9),
        ts: 0,
        ops: vec![Operation::Create {
            doc: capped.clone(),
        }],
    };
    let snapshot_capped = immediate_snapshot(&cmd_capped, &current, &[gm], &|_| None);
    let out_capped = filter_command(
        &cmd_capped,
        &snapshot_capped,
        &gm_ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    assert!(
        out_capped.ops.is_empty(),
        "a gm_role-capped GM must still be denied — the owner plumb must not widen the cap"
    );
}

#[test]
fn redaction_target_classifies_each_whole_band() {
    // The expectation is a HARDCODED list, never `REDACTABLE_BANDS` itself. Deriving the
    // expected value from the constant under test makes the assertion definitionally true
    // for any array contents — it would stay green if a band were renamed, which is the
    // exact "both paths wrong the same way" shape this suite exists to refuse.
    for band in ["name", "engine", "system", "base"] {
        let pointer = format!("/{band}");
        assert_eq!(
            redaction_target(&pointer),
            Some(RedactionTarget::Band),
            "{pointer} must classify as a whole band"
        );
    }
    // Pins the constant's contents independently, so a band added or renamed fails HERE
    // with a message naming the obligation, rather than silently widening what egress
    // is willing to remove.
    assert_eq!(
        REDACTABLE_BANDS,
        ["name", "engine", "system", "base"],
        "the band list changed: re-audit every redaction call site and this suite"
    );
}

#[test]
fn redaction_target_classifies_within_a_band() {
    for pointer in [
        "/system/hp",
        "/system/a/b/c",
        "/engine/vision",
        "/base/system/hp",
        // An empty middle segment still lands inside the untyped body.
        "/system//hp",
        // An index segment is indistinguishable from an object key named "0" from the
        // pointer alone, so it classifies as `Within` and egress must be able to act on
        // it — narrowing the classifier to refuse index-shaped segments would hide an
        // array element from redaction instead of redacting it.
        "/system/inventory/0",
        "/system/inventory/0/secret",
    ] {
        assert_eq!(
            redaction_target(pointer),
            Some(RedactionTarget::Within),
            "{pointer} must classify as within a band"
        );
    }
}

#[test]
fn redaction_target_refuses_every_structural_envelope_field() {
    // The eleven non-content fields of `Document`. Nothing may redact these: a
    // whole-key strip either substitutes a defaulted value or leaves a shape that
    // cannot deserialize.
    for field in [
        "id",
        "scope",
        "doc_type",
        "schema_version",
        "source",
        "owner",
        "permissions",
        "parent_id",
        "embedded",
        "created_at",
        "updated_at",
    ] {
        assert_eq!(redaction_target(&format!("/{field}")), None, "/{field}");
        assert_eq!(
            redaction_target(&format!("/{field}/anything")),
            None,
            "/{field}/anything"
        );
    }
}

#[test]
fn redaction_target_refuses_permissions_subpaths_lacking_serde_default() {
    // A nested pointer into `permissions` strips a field carrying no serde default,
    // leaving a value that cannot deserialize as a `PermissionSet`.
    for pointer in [
        "/permissions",
        "/permissions/default",
        "/permissions/users",
        "/permissions/property_overrides",
    ] {
        assert_eq!(redaction_target(pointer), None, "{pointer}");
    }
}

#[test]
fn redaction_target_refuses_malformed_and_unknown_pointers() {
    for pointer in [
        "",
        "/",
        "system/hp",
        "/unknown",
        "/systemx",
        "/nameless",
        // A band name followed by a non-separator character is a collision, not a
        // match, for every band the shared prefix path handles — not just `system`.
        "/enginex",
        "/basex",
        // A band name plus a trailing separator leaves an empty residual segment,
        // which the guard refuses rather than treating as `Within`.
        "/system/",
    ] {
        assert_eq!(redaction_target(pointer), None, "{pointer:?}");
    }
}

#[test]
fn name_is_a_leaf_band_with_no_interior() {
    // `/name` is a display string, not a container — mirrors the same rule in
    // `required_cap_for_path`.
    assert_eq!(redaction_target("/name"), Some(RedactionTarget::Band));
    assert_eq!(redaction_target("/name/first"), None);
}

#[test]
fn redaction_target_refuses_a_base_pointer_naming_the_snapshots_own_structural_keys() {
    // Every one of `check_base_node_shape`'s required structural keys, at the root and at
    // a nested record: removing any of them via `RedactionTarget::Within`'s object-key-strip
    // terminal step would leave a `base` value that no longer parses as a `MergeBase`/
    // `EmbeddedBaseChild` for the recipient who receives the stripped copy.
    for pointer in [
        // The recorded policy map itself, at the root and inside a record.
        "/base/property_overrides",
        "/base/embedded/item/0/propertyOverrides",
        // The root-only standing key.
        "/base/owner_standing",
        // A whole embedded collection, or a whole record — naming either strips a required
        // key one level up (a collection is a required key of its parent node; a record is
        // a required array element with its own required keys).
        "/base/embedded",
        "/base/embedded/item",
        "/base/embedded/item/0",
        // A record's own correlation key, and a naked "embedded" segment one level down
        // that names neither a collection nor an index.
        "/base/embedded/item/0/sourceId",
        "/base/embedded/item/0/embedded",
    ] {
        assert_eq!(redaction_target(pointer), None, "{pointer}");
    }
}

#[test]
fn redaction_target_admits_base_content_at_any_embedded_depth() {
    // The three content bands remain ordinary `Within` targets under `/base`, at the root
    // and recursively through as many `/embedded/<coll>/<idx>` hops as the snapshot nests.
    for pointer in [
        "/base/name",
        "/base/engine",
        "/base/engine/vision",
        "/base/system",
        "/base/system/hp",
        "/base/embedded/item/0/name",
        "/base/embedded/item/0/system/hp",
        "/base/embedded/item/0/embedded/mod/1/system/hp",
    ] {
        assert_eq!(
            redaction_target(pointer),
            Some(RedactionTarget::Within),
            "{pointer}"
        );
    }
}

#[test]
fn validate_property_overrides_refuses_a_structural_base_pointer() {
    // The write-side chokepoint (`redaction_target`, called by
    // `validation::validate_property_overrides`) is where this must be refused: a GM with
    // `cap::EDIT_PERMISSIONS` must never be able to store an override naming a structural
    // `/base` key, because ingest is the only place this can be refused before the value
    // ever reaches egress.
    let mut d = doc(
        perms_with(&[("/base/embedded", Visibility::GmOnly)]),
        serde_json::json!({}),
    );
    d.base = Some(serde_json::json!({
        "name": null, "engine": null, "system": {}, "embedded": {},
        "property_overrides": {}, "owner_standing": "owner"
    }));
    assert!(crate::data::validation::validate_property_overrides(&d).is_err());
}
