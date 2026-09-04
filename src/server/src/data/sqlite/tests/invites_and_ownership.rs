//! World-invite seating/expiry/revocation, effective-token-ownership
//! resolution (linked-actor vs. per-token override, degenerate-link
//! fail-closed cases), the owner capability floor, `created_seq` generation
//! tracking, and world member-role enumeration.

use super::*;

// --- World invites ---

/// A world with a GM and two redeemers, plus one live invite.
async fn invite_fixture(role: WorldRole) -> (SqliteRepository, Uuid, Uuid, Uuid, Uuid) {
    use crate::auth::role::ServerRole;
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let a = r.create_user("a", None, ServerRole::User, 0).await.unwrap();
    let b = r.create_user("b", None, ServerRole::User, 0).await.unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let id = Uuid::new_v4();
    assert!(r
        .create_invite(
            NewInvite {
                id,
                world: w.id,
                secret_hash: "phc",
                role,
                created_by: gm,
                now: 10,
                expires_at: 1_000_000,
            },
            64
        )
        .await
        .unwrap());
    (r, w.id, id, a, b)
}

#[tokio::test]
async fn consume_invite_seats_exactly_one_redeemer() {
    let (r, world, invite, a, b) = invite_fixture(WorldRole::Player).await;

    let first = r.consume_invite(invite, a, 20).await.unwrap().unwrap();
    assert_eq!(
        first,
        SeatedByInvite {
            world,
            world_name: "W".into(),
            role: WorldRole::Player,
        }
    );
    // The guarded UPDATE is the whole gate: a second redemption of the same
    // row cannot observe it as available, so b is never seated.
    assert_eq!(r.consume_invite(invite, b, 21).await.unwrap(), None);
    assert_eq!(
        r.member_role(world, a).await.unwrap(),
        Some(WorldRole::Player)
    );
    assert_eq!(r.member_role(world, b).await.unwrap(), None);
}

#[tokio::test]
async fn consume_invite_refuses_expired_and_revoked_rows() {
    let (r, world, invite, a, _) = invite_fixture(WorldRole::Player).await;
    // `now` past the row's expiry.
    assert_eq!(r.consume_invite(invite, a, 2_000_000).await.unwrap(), None);
    assert_eq!(r.member_role(world, a).await.unwrap(), None);
    // The row was still live at a valid `now` — expiry is the only reason
    // it failed above. Revoked, it fails for a second, distinct reason.
    assert!(r.revoke_invite(world, invite, 30).await.unwrap());
    assert_eq!(r.consume_invite(invite, a, 40).await.unwrap(), None);
    assert_eq!(r.member_role(world, a).await.unwrap(), None);
    // Revoking a revoked row is not a second success.
    assert!(!r.revoke_invite(world, invite, 50).await.unwrap());
}

#[tokio::test]
async fn revoke_invite_is_scoped_to_its_world() {
    use crate::auth::role::ServerRole;
    let (r, world, invite, a, _) = invite_fixture(WorldRole::Player).await;
    let other_gm = r
        .create_user("other", None, ServerRole::User, 0)
        .await
        .unwrap();
    let other = r.create_world_owned("Other", other_gm, 0).await.unwrap();

    // Another world's id does not unlock this invite.
    assert!(!r.revoke_invite(other.id, invite, 30).await.unwrap());
    let seated = r.consume_invite(invite, a, 40).await.unwrap().unwrap();
    assert_eq!((seated.world, seated.role), (world, WorldRole::Player));
}

#[tokio::test]
async fn consume_invite_never_changes_a_role_already_held() {
    let (r, world, invite, _, _) = invite_fixture(WorldRole::Spectator).await;
    let gm = r.list_members(world).await.unwrap()[0].0;
    assert_eq!(r.member_role(world, gm).await.unwrap(), Some(WorldRole::Gm));

    let seated = r.consume_invite(invite, gm, 20).await.unwrap().unwrap();
    assert_eq!(
        (seated.world, seated.role),
        (world, WorldRole::Gm),
        "the returned role is the membership actually held"
    );
    assert_eq!(r.member_role(world, gm).await.unwrap(), Some(WorldRole::Gm));
}

#[tokio::test]
async fn list_invites_never_returns_the_stored_hash() {
    let (r, world, invite, _, _) = invite_fixture(WorldRole::Player).await;
    let listed = r.list_invites(world).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, invite);
    assert_eq!(listed[0].secret_hash, "");
    // The by-id lookup, redemption's only reader, still sees it.
    assert_eq!(
        r.invite_by_id(invite).await.unwrap().unwrap().secret_hash,
        "phc"
    );
}

#[tokio::test]
async fn create_invite_caps_live_invites_and_a_spent_one_frees_a_slot() {
    let (r, world, first, a, _) = invite_fixture(WorldRole::Player).await;
    // Cap of 1: the world already holds one live invite.
    assert!(!r
        .create_invite(
            NewInvite {
                id: Uuid::new_v4(),
                world,
                secret_hash: "phc",
                role: WorldRole::Player,
                created_by: a,
                now: 10,
                expires_at: 1_000_000,
            },
            1
        )
        .await
        .unwrap());
    r.consume_invite(first, a, 20).await.unwrap().unwrap();
    assert!(r
        .create_invite(
            NewInvite {
                id: Uuid::new_v4(),
                world,
                secret_hash: "phc",
                role: WorldRole::Player,
                created_by: a,
                now: 20,
                expires_at: 1_000_000,
            },
            1
        )
        .await
        .unwrap());
}

// ---- Token ownership: actor-inherited with a per-token override ----
//
// effective_owner(token) = the token's own `owner` override, else the LINKED
// actor's owner, resolved SERVER-SIDE at authz time against live actor state.
// Every reject leg below is paired with an accept leg that differs ONLY in the
// resolution input (which user, which override, which actor owner), so a rule
// inverted or defaulted-open flips the pair rather than passing both.

/// Attempt `/engine/x` + `/engine/y` as `user` (a Player) on `token`.
async fn try_move(
    r: &SqliteRepository,
    world: Uuid,
    user: Uuid,
    token: Uuid,
    from: (f64, f64),
    to: (f64, f64),
    ts: i64,
) -> Result<Command, DataError> {
    use crate::data::command::FieldChange;
    use crate::data::membership::PermissionContext;
    r.apply_intent(
        &PermissionContext {
            user_id: user,
            world_role: WorldRole::Player,
        },
        world,
        vec![Operation::Update {
            doc_id: token,
            changes: vec![
                FieldChange {
                    remove: false,
                    path: "/engine/x".into(),
                    old: serde_json::json!(from.0),
                    new: serde_json::json!(to.0),
                },
                FieldChange {
                    remove: false,
                    path: "/engine/y".into(),
                    old: serde_json::json!(from.1),
                    new: serde_json::json!(to.1),
                },
            ],
        }],
        ts,
        WriteOrigin::Client,
    )
    .await
    .map(|stored| stored.command)
}

/// GM, world, and two ordinary player accounts (`owner_id` is a FK, so every
/// owner must be a real user row).
async fn ownership_fixture() -> (SqliteRepository, Uuid, Uuid, Uuid, Uuid) {
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let p1 = r
        .create_user("player-one", None, ServerRole::User, 0)
        .await
        .unwrap();
    let p2 = r
        .create_user("player-two", None, ServerRole::User, 0)
        .await
        .unwrap();
    (r, gm, w.id, p1, p2)
}

async fn gm_create(r: &SqliteRepository, gm: Uuid, world: Uuid, docs: Vec<Document>, ts: i64) {
    use crate::data::membership::PermissionContext;
    r.apply_intent(
        &PermissionContext {
            user_id: gm,
            world_role: WorldRole::Gm,
        },
        world,
        docs.into_iter()
            .map(|doc| Operation::Create { doc })
            .collect(),
        ts,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn linked_token_inherits_actor_owner_for_writes() {
    let (r, gm, w, p1, p2) = ownership_fixture().await;
    let actor = actor_doc_owned_by(w, Some(p1));
    let token = owned_token_doc(w, Some(actor.id));
    gm_create(&r, gm, w, vec![actor, token.clone()], 1).await;

    // The actor's owner may move the linked token — with NO per-token `owner`
    // and NO per-token permissions entry: authority is inherited, live.
    try_move(&r, w, p1, token.id, (0.0, 0.0), (5.0, 7.0), 2)
        .await
        .expect("the linked actor's owner may move the token");

    // Non-vacuity: the SAME token, the SAME path, the SAME pre-image — only the
    // user differs. A rule defaulted open (or inverted) would let this pass too.
    let denied = try_move(&r, w, p2, token.id, (5.0, 7.0), (9.0, 9.0), 3).await;
    assert!(
        matches!(denied, Err(DataError::Forbidden)),
        "a player who owns neither the token nor its actor must not move it, got {denied:?}"
    );
}

#[tokio::test]
async fn per_token_owner_override_beats_the_linked_actors_owner() {
    let (r, gm, w, p1, p2) = ownership_fixture().await;
    let actor = actor_doc_owned_by(w, Some(p1));
    let mut token = owned_token_doc(w, Some(actor.id));
    token.owner = Some(p2); // GM override on the individual token
    gm_create(&r, gm, w, vec![actor, token.clone()], 1).await;

    // The override holder writes...
    try_move(&r, w, p2, token.id, (0.0, 0.0), (2.0, 3.0), 2)
        .await
        .expect("the per-token owner override may move the token");

    // ...and the actor's owner, who WOULD inherit but for the override, cannot.
    // Paired with the accept leg above, this pins precedence in both directions:
    // inverting it (actor owner beats override) flips both assertions.
    let denied = try_move(&r, w, p1, token.id, (2.0, 3.0), (4.0, 4.0), 3).await;
    assert!(
        matches!(denied, Err(DataError::Forbidden)),
        "the token's own override must supersede the actor's owner, got {denied:?}"
    );
}

#[tokio::test]
async fn reassigning_the_actors_owner_moves_token_authority_with_no_restamp() {
    use crate::data::command::FieldChange;
    use crate::data::membership::PermissionContext;
    let (r, gm, w, p1, p2) = ownership_fixture().await;
    let actor = actor_doc_owned_by(w, Some(p1));
    let token = owned_token_doc(w, Some(actor.id));
    gm_create(&r, gm, w, vec![actor.clone(), token.clone()], 1).await;

    try_move(&r, w, p1, token.id, (0.0, 0.0), (1.0, 1.0), 2)
        .await
        .expect("the original actor owner may move the token");

    // The GM re-assigns the ACTOR's owner. The token document is never touched.
    r.apply_intent(
        &PermissionContext {
            user_id: gm,
            world_role: WorldRole::Gm,
        },
        w,
        vec![Operation::Update {
            doc_id: actor.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/owner".into(),
                old: serde_json::json!(p1.to_string()),
                new: serde_json::json!(p2.to_string()),
            }],
        }],
        3,
        WriteOrigin::Client,
    )
    .await
    .expect("a GM may re-assign an actor's owner");

    // Authority followed the actor, with no write to the token: the token's own
    // `owner` is STILL unset — proving resolution, not a stamped copy.
    let stored = r.get_document(token.id).await.unwrap().unwrap();
    assert_eq!(
        stored.owner, None,
        "the token must carry no stamped owner — ownership is resolved, not copied"
    );

    try_move(&r, w, p2, token.id, (1.0, 1.0), (6.0, 6.0), 4)
        .await
        .expect("the actor's NEW owner may move the token");
    let denied = try_move(&r, w, p1, token.id, (6.0, 6.0), (8.0, 8.0), 5).await;
    assert!(
        matches!(denied, Err(DataError::Forbidden)),
        "the actor's ORIGINAL owner must lose the token, got {denied:?}"
    );
}

#[tokio::test]
async fn ownership_fails_closed_on_every_degenerate_link() {
    let (r, gm, w, p1, _p2) = ownership_fixture().await;

    // (a) No link at all, no override: nobody inherits.
    let unlinked = owned_token_doc(w, None);
    // (b) Dangling link: `actor_id` names a document that does not exist.
    let dangling = owned_token_doc(w, Some(Uuid::new_v4()));
    // (c) Linked to an actor with NO owner.
    let unowned_actor = actor_doc_owned_by(w, None);
    let linked_unowned = owned_token_doc(w, Some(unowned_actor.id));
    // (d) Control: identical shape, but the actor IS owned by p1.
    let owned_actor = actor_doc_owned_by(w, Some(p1));
    let linked_owned = owned_token_doc(w, Some(owned_actor.id));
    gm_create(
        &r,
        gm,
        w,
        vec![
            unlinked.clone(),
            dangling.clone(),
            unowned_actor,
            linked_unowned.clone(),
            owned_actor,
            linked_owned.clone(),
        ],
        1,
    )
    .await;

    for (label, id) in [
        ("no link", unlinked.id),
        ("dangling link", dangling.id),
        ("actor with no owner", linked_unowned.id),
    ] {
        let denied = try_move(&r, w, p1, id, (0.0, 0.0), (3.0, 3.0), 2).await;
        assert!(
            matches!(denied, Err(DataError::Forbidden)),
            "{label} must fail closed (no owner => no write), got {denied:?}"
        );
    }

    // Non-vacuity for the whole loop: the same player, the same move, on a token
    // whose only difference is a RESOLVABLE owned actor — this one succeeds, so
    // the three rejections above are the ownership rule, not a blanket denial.
    try_move(&r, w, p1, linked_owned.id, (0.0, 0.0), (3.0, 3.0), 3)
        .await
        .expect("the control leg (resolvable owned actor) must succeed");
}

#[tokio::test]
async fn an_effective_owner_cannot_reassign_or_widen_ownership() {
    use crate::data::command::FieldChange;
    use crate::data::membership::PermissionContext;
    let (r, gm, w, p1, p2) = ownership_fixture().await;
    let actor = actor_doc_owned_by(w, Some(p1));
    let token = owned_token_doc(w, Some(actor.id));
    gm_create(&r, gm, w, vec![actor, token.clone()], 1).await;

    let as_p1 = PermissionContext {
        user_id: p1,
        world_role: WorldRole::Player,
    };
    // The effective owner holds the `DocRole::Owner` floor (READ + WRITE_FIELDS)
    // and nothing more: `/owner` and `/permissions` need EDIT_PERMISSIONS, which
    // that floor does not include. Without this, an inheriting owner could pin
    // the token to themselves or hand it to anyone.
    for change in [
        FieldChange {
            remove: false,
            path: "/owner".into(),
            old: serde_json::Value::Null,
            new: serde_json::json!(p2.to_string()),
        },
        FieldChange {
            remove: false,
            path: "/permissions/default".into(),
            old: serde_json::json!("observer"),
            new: serde_json::json!("owner"),
        },
    ] {
        let path = change.path.clone();
        let denied = r
            .apply_intent(
                &as_p1,
                w,
                vec![Operation::Update {
                    doc_id: token.id,
                    changes: vec![change],
                }],
                2,
                WriteOrigin::Client,
            )
            .await;
        assert!(
            matches!(denied, Err(DataError::Forbidden)),
            "an effective owner must not write {path}, got {denied:?}"
        );
    }

    // Non-vacuity: the same user, same doc, same call shape — a WRITE_FIELDS path
    // succeeds, so the two rejections are the capability split, not a dead player.
    try_move(&r, w, p1, token.id, (0.0, 0.0), (1.0, 2.0), 3)
        .await
        .expect("the effective owner still holds WRITE_FIELDS");
}

#[tokio::test]
async fn effective_owner_of_joins_the_linked_actor_on_the_pool() {
    let (r, gm, w, p1, _p2) = ownership_fixture().await;
    let actor = actor_doc_owned_by(w, Some(p1));
    let actor_id = actor.id;
    let token = owned_token_doc(w, Some(actor_id));
    let token_id = token.id;
    gm_create(&r, gm, w, vec![actor, token], 1).await;

    let token = r.get_document(token_id).await.unwrap().unwrap();
    assert_eq!(r.effective_owner_of(&token).await.unwrap(), Some(p1));

    // Dangling link fails closed.
    let mut dangling = token.clone();
    dangling.engine = Some(serde_json::json!({
        "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0, "rotation": 0.0,
        "actor_id": Uuid::from_u128(999999).to_string()
    }));
    assert_eq!(r.effective_owner_of(&dangling).await.unwrap(), None);

    // A non-token resolves to its literal owner without any join.
    let actor = r.get_document(actor_id).await.unwrap().unwrap();
    assert_eq!(r.effective_owner_of(&actor).await.unwrap(), Some(p1));
}

#[tokio::test]
async fn the_owner_capability_floor_is_scoped_to_tokens() {
    use crate::data::command::FieldChange;
    use crate::data::membership::PermissionContext;
    let (r, gm, w, p1, _p2) = ownership_fixture().await;
    // An `actor` the player owns. `owner` carries a provenance-only
    // meaning on every non-`token` doc_type: it admits the
    // OwnerOrGm redaction tier but grants NO capability, so the player cannot
    // write the actor's body. Widening this is a separate design decision.
    let mut actor = actor_doc_owned_by(w, Some(p1));
    actor.system = serde_json::json!({ "hp": 10 });
    gm_create(&r, gm, w, vec![actor.clone()], 1).await;

    let denied = r
        .apply_intent(
            &PermissionContext {
                user_id: p1,
                world_role: WorldRole::Player,
            },
            w,
            vec![Operation::Update {
                doc_id: actor.id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/system/hp".into(),
                    old: serde_json::json!(10),
                    new: serde_json::json!(1),
                }],
            }],
            2,
            WriteOrigin::Client,
        )
        .await;
    assert!(
        matches!(denied, Err(DataError::Forbidden)),
        "the owner floor must not leak to non-token doc_types, got {denied:?}"
    );
}

#[tokio::test]
async fn a_removal_carrying_a_new_value_is_rejected_at_ingress() {
    // `remove: true` deletes the key; `new` is unused. The pairing has no
    // legitimate meaning, and `new` is checked by neither the OCC comparison
    // (which reads `old`) nor `required_cap_for_path` — so any consumer that
    // mirrors a change by unconditionally setting `new` lands an attacker-chosen
    // value where the store lands absence. Denied at ingress.
    use crate::data::command::FieldChange;
    let (r, gm, w, p1, p2) = ownership_fixture().await;
    let actor = actor_doc_owned_by(w, Some(p1));
    let token = owned_token_doc(w, Some(actor.id));
    gm_create(&r, gm, w, vec![actor, token.clone()], 1).await;

    let attempt = |remove: bool, new: serde_json::Value, ts: i64| {
        let r = &r;
        let path = "/engine/actor_id".to_string();
        async move {
            r.apply_intent(
                &crate::data::membership::PermissionContext {
                    user_id: p1,
                    world_role: WorldRole::Player,
                },
                w,
                vec![Operation::Update {
                    doc_id: token.id,
                    changes: vec![FieldChange {
                        remove,
                        path,
                        old: serde_json::json!(null),
                        new,
                    }],
                }],
                ts,
                WriteOrigin::Client,
            )
            .await
        }
    };

    // Rejected: a removal carrying a value.
    let denied = attempt(true, serde_json::json!(p2.to_string()), 2).await;
    assert!(
        matches!(denied, Err(DataError::OpFailed(_))),
        "a removal must not carry a `new` value, got {denied:?}"
    );

    // Non-vacuity: the SAME change with `new: null` clears ingress and is judged
    // on its merits (it fails the OCC pre-image check, not the shape gate) — so
    // the rejection above is the shape rule, not a blanket denial of removals.
    let occ = attempt(true, serde_json::Value::Null, 3).await;
    assert!(
        matches!(occ, Err(DataError::Conflict(_))),
        "a well-shaped removal reaches the OCC check, got {occ:?}"
    );
}

#[tokio::test]
async fn the_actor_join_does_not_cross_world_scope() {
    // `load_document` is keyed on id alone. An `actor_id` naming an actor in
    // ANOTHER world must not resolve an owner: it breaks world isolation, and
    // room hydration loads actors `WHERE world_id = ?`, so the derived vision
    // path structurally cannot see such an actor — resolving one here would be a
    // second ECS/DB ownership fork.
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let p1 = r
        .create_user("player-one", None, ServerRole::User, 0)
        .await
        .unwrap();
    let token_world = r.create_world_owned("token-world", gm, 0).await.unwrap();
    let actor_world = r.create_world_owned("actor-world", gm, 0).await.unwrap();

    // The actor is owned by p1 but lives in a different world from the token it is linked to.
    let foreign_actor = actor_doc_owned_by(actor_world.id, Some(p1));
    gm_create(&r, gm, actor_world.id, vec![foreign_actor.clone()], 1).await;
    let token = owned_token_doc(token_world.id, Some(foreign_actor.id));
    gm_create(&r, gm, token_world.id, vec![token.clone()], 2).await;

    // p1 is a member of the token's world too, so only the scope check can deny this.
    r.add_member(token_world.id, p1, WorldRole::Player)
        .await
        .unwrap();
    let denied = try_move(&r, token_world.id, p1, token.id, (0.0, 0.0), (3.0, 3.0), 3).await;
    assert!(
        matches!(denied, Err(DataError::Forbidden)),
        "a cross-world actor link must not confer ownership, got {denied:?}"
    );

    // Non-vacuity: the identical setup with the actor in the TOKEN's own world
    // succeeds — proving the denial is the scope check, not the membership or
    // the link machinery.
    let local_actor = actor_doc_owned_by(token_world.id, Some(p1));
    let local_token = owned_token_doc(token_world.id, Some(local_actor.id));
    gm_create(
        &r,
        gm,
        token_world.id,
        vec![local_actor, local_token.clone()],
        4,
    )
    .await;
    try_move(
        &r,
        token_world.id,
        p1,
        local_token.id,
        (0.0, 0.0),
        (3.0, 3.0),
        5,
    )
    .await
    .expect("a same-world actor link confers ownership");
}

#[tokio::test]
async fn created_seq_is_set_once_and_survives_updates() {
    use crate::data::command::{FieldChange, Operation};
    use crate::data::document::{DocRole, PermissionSet, Scope};
    use crate::data::membership::PermissionContext;

    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let mut perms = PermissionSet::default();
    perms.users.insert(gm, DocRole::Owner);
    let mut d = tests_doc(perms, serde_json::json!({ "hp": 1 }));
    d.scope = Scope::World { world_id: w.id };
    let doc_id = d.id;

    let stored = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Create { doc: d }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    let first_seq = stored.command.seq;

    let mut tx = r.pool.begin().await.unwrap();
    let created_after_create = SqliteRepository::document_created_seq(&mut *tx, doc_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(created_after_create, Some(first_seq));

    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Update {
            doc_id,
            changes: vec![FieldChange {
                remove: false,
                path: "/system/hp".into(),
                old: serde_json::json!(1),
                new: serde_json::json!(2),
            }],
        }],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut tx = r.pool.begin().await.unwrap();
    let created_after_update = SqliteRepository::document_created_seq(&mut *tx, doc_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        created_after_update, created_after_create,
        "created_seq must not change across an update to a still-live row"
    );
}

#[tokio::test]
async fn created_seq_is_absent_for_a_missing_document() {
    let r = repo().await;
    let mut tx = r.pool.begin().await.unwrap();
    let missing = SqliteRepository::document_created_seq(&mut *tx, Uuid::new_v4())
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(missing, None);
}

#[tokio::test]
async fn world_member_roles_reflects_every_current_member() {
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = r
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    r.add_member(w.id, player, WorldRole::Player).await.unwrap();

    let mut tx = r.pool.begin().await.unwrap();
    let roles = SqliteRepository::world_member_roles(&mut *tx, w.id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(roles.get(&gm), Some(&WorldRole::Gm));
    assert_eq!(roles.get(&player), Some(&WorldRole::Player));
}

#[tokio::test]
async fn get_document_with_created_seq_matches_a_separate_created_seq_read() {
    use crate::data::command::Operation;
    use crate::data::document::{DocRole, PermissionSet, Scope};
    use crate::data::membership::PermissionContext;

    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let mut perms = PermissionSet::default();
    perms.users.insert(gm, DocRole::Owner);
    let mut d = tests_doc(perms, serde_json::json!({ "hp": 1 }));
    d.scope = Scope::World { world_id: w.id };
    let doc_id = d.id;
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Create { doc: d }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let (doc, created_seq) = r
        .get_document_with_created_seq(doc_id)
        .await
        .unwrap()
        .expect("document must exist");
    assert_eq!(doc.id, doc_id);
    let mut tx = r.pool.begin().await.unwrap();
    let separate = SqliteRepository::document_created_seq(&mut *tx, doc_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(Some(created_seq), separate);
}

#[tokio::test]
async fn carried_light_authoring_is_gm_only() {
    use crate::data::command::FieldChange;
    use crate::data::membership::PermissionContext;
    let (r, gm, w, p1, _p2) = ownership_fixture().await;
    let actor = actor_doc_owned_by(w, Some(p1));
    let token = owned_token_doc(w, Some(actor.id));
    gm_create(&r, gm, w, vec![actor, token.clone()], 1).await;
    let player = PermissionContext {
        user_id: p1,
        world_role: WorldRole::Player,
    };
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let emission = serde_json::json!({
        "color": "#ffcc66", "intensity": 1.0, "brightRadius": 1.0, "dimRadius": 2.0,
        "enabled": true
    });
    let write = |ctx: PermissionContext,
                 path: String,
                 old: serde_json::Value,
                 new: serde_json::Value,
                 ts: i64| {
        let r = &r;
        let token = &token;
        async move {
            r.apply_intent(
                &ctx,
                w,
                vec![Operation::Update {
                    doc_id: token.id,
                    changes: vec![FieldChange {
                        remove: false,
                        path,
                        old,
                        new,
                    }],
                }],
                ts,
                WriteOrigin::Client,
            )
            .await
        }
    };

    // The token's owner holds WRITE_FIELDS on it (the Owner floor), but a carried emission
    // joins the SHARED illumination field every viewer's mask reads — GM-only.
    let denied = write(
        player,
        "/engine/overrides/light".into(),
        serde_json::json!(null),
        emission.clone(),
        2,
    )
    .await;
    assert!(
        matches!(denied, Err(DataError::Forbidden)),
        "a player's light override must be refused, got {denied:?}"
    );

    // Non-vacuity: the same owner CAN write the sibling override fields (vision is
    // self-scoped), so the refusal above is the light rule, not a blanket override ban.
    let vision = serde_json::json!([{ "mode": "darkvision", "range": 6 }]);
    write(
        player,
        "/engine/overrides/vision".into(),
        serde_json::json!(null),
        vision.clone(),
        3,
    )
    .await
    .expect("a player's vision override stays legal");

    // Ancestor writes are value-aware: a whole-overrides write that leaves `light` absent/null
    // is legal; one that introduces an emission is refused. The stored pre-image is the
    // NORMALIZED round-tripped object (every whitelist key materialized), read back raw.
    let stored = r
        .get_document(token.id)
        .await
        .unwrap()
        .expect("the token exists");
    let stored_overrides = stored
        .engine
        .as_ref()
        .unwrap()
        .pointer("/overrides")
        .cloned()
        .unwrap();
    write(
        player,
        "/engine/overrides".into(),
        stored_overrides.clone(),
        serde_json::json!({ "vision": vision, "light": null }),
        4,
    )
    .await
    .expect("an ancestor write not touching the emission stays legal");
    let stored = r
        .get_document(token.id)
        .await
        .unwrap()
        .expect("the token exists");
    let stored_overrides = stored
        .engine
        .as_ref()
        .unwrap()
        .pointer("/overrides")
        .cloned()
        .unwrap();
    let mut with_light = stored_overrides.clone();
    with_light["light"] = emission.clone();
    let denied_ancestor = write(
        player,
        "/engine/overrides".into(),
        stored_overrides,
        with_light,
        5,
    )
    .await;
    assert!(
        matches!(denied_ancestor, Err(DataError::Forbidden)),
        "an ancestor write introducing an emission must be refused, got {denied_ancestor:?}"
    );

    // The GM authors the emission.
    write(
        gm_ctx,
        "/engine/overrides/light".into(),
        serde_json::json!(null),
        emission.clone(),
        6,
    )
    .await
    .expect("the GM may author a carried light");

    // Removing the GM-authored override also changes the effective emission (restores
    // inheritance), so it is gated too.
    let denied_remove = r
        .apply_intent(
            &player,
            w,
            vec![Operation::Update {
                doc_id: token.id,
                changes: vec![FieldChange {
                    remove: true,
                    path: "/engine/overrides/light".into(),
                    old: emission.clone(),
                    new: serde_json::Value::Null,
                }],
            }],
            7,
            WriteOrigin::Client,
        )
        .await;
    assert!(
        matches!(denied_remove, Err(DataError::Forbidden)),
        "a player removing the override must be refused, got {denied_remove:?}"
    );

    // An actor's own `/engine/light` is the same shared-field rule. Give the player an explicit
    // WRITE_FIELDS grant on the actor first, so ONLY the light rule can refuse the write.
    let mut lit_actor = actor_doc_owned_by(w, Some(p1));
    lit_actor.engine = Some(serde_json::json!({
        "displayName": "Torch", "visual": { "kind": "image", "asset": "a" },
        "size": { "w": 1.0, "h": 1.0 }, "shape": "square",
        "faction": null, "conditions": [], "prototype": true
    }));
    // An explicit per-user Owner grant (the only way a player holds WRITE_FIELDS on an actor
    // doc — the `owner` field itself is provenance-only there), so ONLY the light rule can
    // refuse the write below.
    lit_actor
        .permissions
        .users
        .insert(p1, crate::data::document::DocRole::Owner);
    gm_create(&r, gm, w, vec![lit_actor.clone()], 8).await;
    let denied_actor = r
        .apply_intent(
            &player,
            w,
            vec![Operation::Update {
                doc_id: lit_actor.id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/engine/light".into(),
                    old: serde_json::json!(null),
                    new: emission,
                }],
            }],
            9,
            WriteOrigin::Client,
        )
        .await;
    assert!(
        matches!(denied_actor, Err(DataError::Forbidden)),
        "a granted player's actor light write must still be refused, got {denied_actor:?}"
    );
}
