//! `WriteOrigin::CombatTransition`'s capability exemption and the one-active-
//! combat-per-scene batch fix (same-batch deactivate+activate swap, in either
//! op ordering, still rejecting a genuine two-activation batch).

use super::*;
use crate::data::command::{FieldChange, Operation, WriteOrigin};
use crate::data::document::WorldRole;
use crate::data::membership::PermissionContext;
use crate::data::DataError;

fn activate(doc_id: Uuid, active: bool) -> Operation {
    Operation::Update {
        doc_id,
        changes: vec![FieldChange {
            remove: false,
            path: "/engine/active".into(),
            old: serde_json::json!(!active),
            new: serde_json::json!(active),
        }],
    }
}

/// An unrelated combat's same-batch deactivation must never let a THIRD
/// combat's Create claim a scene that a different combat already legitimately
/// activated earlier in the same batch. `a` activates `s`, `b` (an unrelated
/// combat already active on `s`) deactivates, then `c` tries to Create active
/// on `s` -- `a`'s earlier claim must still hold, so the whole batch rejects.
#[tokio::test]
async fn an_unrelated_deactivation_cannot_let_a_third_combat_steal_an_already_claimed_scene() {
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
    let scene = Uuid::from_u128(0x5CE);
    // `a` starts inactive; `b` starts active on the same scene.
    let a = combat_doc(1, w.id, scene, false);
    let b = combat_doc(2, w.id, scene, true);
    r.apply_intent(
        &ctx,
        w.id,
        vec![
            Operation::Create { doc: a.clone() },
            Operation::Create { doc: b.clone() },
        ],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let c = combat_doc(3, w.id, scene, true);
    let res = r
        .apply_intent(
            &ctx,
            w.id,
            vec![
                activate(a.id, true),
                activate(b.id, false),
                Operation::Create { doc: c.clone() },
            ],
            2,
            WriteOrigin::Client,
        )
        .await;
    assert!(matches!(res, Err(DataError::Conflict(_))));
    // Nothing in the rejected batch landed: `c` never got created, and `a`
    // stayed exactly as it was created (inactive).
    assert!(r.get_document(c.id).await.unwrap().is_none());
    let stored_a = r.get_document(a.id).await.unwrap().unwrap();
    assert_eq!(stored_a.engine.unwrap()["active"], serde_json::json!(false));
}

#[tokio::test]
async fn a_swap_batch_deactivating_then_activating_on_one_scene_passes_in_either_order() {
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
    let scene = Uuid::from_u128(0x5CE);
    let a = combat_doc(1, w.id, scene, true);
    let b = combat_doc(2, w.id, scene, false);
    r.apply_intent(
        &ctx,
        w.id,
        vec![
            Operation::Create { doc: a.clone() },
            Operation::Create { doc: b.clone() },
        ],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    r.apply_intent(
        &ctx,
        w.id,
        vec![activate(a.id, false), activate(b.id, true)],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    r.apply_intent(
        &ctx,
        w.id,
        vec![activate(a.id, true), activate(b.id, false)],
        3,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn activating_two_different_combats_on_one_scene_in_one_batch_still_rejects() {
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
    let scene = Uuid::from_u128(0x5CE);
    let a = combat_doc(1, w.id, scene, false);
    let b = combat_doc(2, w.id, scene, false);
    r.apply_intent(
        &ctx,
        w.id,
        vec![
            Operation::Create { doc: a.clone() },
            Operation::Create { doc: b.clone() },
        ],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let res = r
        .apply_intent(
            &ctx,
            w.id,
            vec![activate(a.id, true), activate(b.id, true)],
            2,
            WriteOrigin::Client,
        )
        .await;
    assert!(matches!(res, Err(DataError::Conflict(_))));
}

/// Same rejection as above, with the op order reversed — the one-active-per-
/// scene fix must not accidentally become order-dependent in the OTHER
/// direction (i.e. it must reject regardless of which of the two activating
/// ops the batch lists first), since neither combat is ever deactivated in
/// this batch.
#[tokio::test]
async fn activating_two_different_combats_on_one_scene_in_one_batch_still_rejects_reverse_order() {
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
    let scene = Uuid::from_u128(0x5CE);
    let a = combat_doc(1, w.id, scene, false);
    let b = combat_doc(2, w.id, scene, false);
    r.apply_intent(
        &ctx,
        w.id,
        vec![
            Operation::Create { doc: a.clone() },
            Operation::Create { doc: b.clone() },
        ],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let res = r
        .apply_intent(
            &ctx,
            w.id,
            vec![activate(b.id, true), activate(a.id, true)],
            2,
            WriteOrigin::Client,
        )
        .await;
    assert!(matches!(res, Err(DataError::Conflict(_))));
}

/// A batch Creating a second active combat on a scene that already has one
/// active, alongside an UNRELATED update to a THIRD, already-inactive combat
/// on the SAME scene, must still reject — the pre-scan that seeds
/// `apply_intent`'s `deactivations_this_batch` set must key off a genuine
/// active-true -> false TRANSITION, never merely "ends up inactive", or an
/// unrelated no-op update would wrongly suppress the DB conflict check for
/// the real double-activation.
#[tokio::test]
async fn an_unrelated_update_to_an_already_inactive_combat_does_not_mask_a_real_conflict() {
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
    let scene = Uuid::from_u128(0x5CE);
    // `a` starts (and stays) active; `c` starts (and stays) inactive.
    let a = combat_doc(1, w.id, scene, true);
    let c = combat_doc(2, w.id, scene, false);
    let d = combat_doc(3, w.id, scene, true);
    r.apply_intent(
        &ctx,
        w.id,
        vec![
            Operation::Create { doc: a.clone() },
            Operation::Create { doc: c.clone() },
        ],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    // `c`'s update never touches `/engine/active` (it targets `/engine/round`
    // instead), so `c` remains inactive before and after — no real release.
    let touch_round = Operation::Update {
        doc_id: c.id,
        changes: vec![FieldChange {
            remove: false,
            path: "/engine/round".into(),
            old: serde_json::json!(0),
            new: serde_json::json!(1),
        }],
    };
    // `d` is a genuinely new second active combat on `a`'s already-active
    // scene — this must reject on that basis, not on a doc-id collision.
    let res = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Create { doc: d.clone() }, touch_round],
            2,
            WriteOrigin::Client,
        )
        .await;
    assert!(matches!(res, Err(DataError::Conflict(_))));
    // Confirm the rejection was the active-combat check, not an existing-id
    // collision: `d` never landed.
    assert!(r.get_document(d.id).await.unwrap().is_none());
}

#[tokio::test]
async fn combat_transition_origin_skips_capability_gates_but_keeps_occ_and_validation() {
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = r.create_user("p", None, ServerRole::User, 0).await.unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    r.add_member(w.id, player, WorldRole::Player).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let p_ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let combat = combat_doc(1, w.id, Uuid::from_u128(0x5CE), false);
    let c = combatant_doc(2, w.id, combat.id);
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![
            Operation::Create {
                doc: combat.clone(),
            },
            Operation::Create { doc: c.clone() },
        ],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let bump = |old: f64, new: f64| Operation::Update {
        doc_id: c.id,
        changes: vec![FieldChange {
            remove: false,
            path: "/engine/tiebreak".into(),
            old: serde_json::json!(old),
            new: serde_json::json!(new),
        }],
    };
    // A Player holds no WRITE_FIELDS grant on this un-owned combatant, so an
    // ordinary Client-origin write is still refused — the capability floor
    // this origin exists to skip is genuinely load-bearing for `Client`.
    assert!(matches!(
        r.apply_intent(&p_ctx, w.id, vec![bump(0.0, 1.0)], 2, WriteOrigin::Client)
            .await,
        Err(DataError::Forbidden)
    ));
    r.apply_intent(
        &p_ctx,
        w.id,
        vec![bump(0.0, 1.0)],
        3,
        WriteOrigin::CombatTransition,
    )
    .await
    .unwrap();
    assert!(
        matches!(
            r.apply_intent(
                &p_ctx,
                w.id,
                vec![bump(0.0, 2.0)],
                4,
                WriteOrigin::CombatTransition
            )
            .await,
            Err(DataError::Conflict(_))
        ),
        "OCC still binds"
    );
    let bad = Operation::Update {
        doc_id: c.id,
        changes: vec![FieldChange {
            remove: false,
            path: "/engine/tiebreak".into(),
            old: serde_json::json!(1.0),
            new: serde_json::json!("nan"),
        }],
    };
    assert!(
        matches!(
            r.apply_intent(&p_ctx, w.id, vec![bad], 5, WriteOrigin::CombatTransition)
                .await,
            Err(DataError::BadEngine(_))
        ),
        "engine validation still binds"
    );
}

/// A `CombatTransition` batch may `Create` a `message` document (posting a
/// roll result or event message) but is blanket-rejected from `Update`-ing
/// one — same as `Client`, and unaffected by this origin's capability
/// exemption (the message-doc reject runs before any capability check).
#[tokio::test]
async fn combat_transition_may_create_but_never_update_a_message_document() {
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
    let msg = Document {
        id: Uuid::from_u128(1),
        scope: Scope::World { world_id: w.id },
        doc_type: crate::chat::MESSAGE_DOC_TYPE.to_string(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner: Some(gm),
        permissions: Default::default(),
        embedded: Default::default(),
        parent_id: None,
        engine: Some(serde_json::json!({
            "channel": "all", "user_owner": gm, "kind": "normal",
            "audience": {"kind": "public"}, "content": []
        })),
        system: serde_json::json!({}),
        created_at: 0,
        updated_at: 0,
    };
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Create { doc: msg.clone() }],
        1,
        WriteOrigin::CombatTransition,
    )
    .await
    .unwrap();
    let update = Operation::Update {
        doc_id: msg.id,
        changes: vec![FieldChange {
            remove: false,
            path: "/name".into(),
            old: serde_json::json!(null),
            new: serde_json::json!("renamed"),
        }],
    };
    let res = r
        .apply_intent(&ctx, w.id, vec![update], 2, WriteOrigin::CombatTransition)
        .await;
    assert!(matches!(res, Err(DataError::Forbidden)));
}

/// A batch that deactivates `a`, lets a same-batch `Create` of `b` claim the
/// now-freed scene, and then re-declares `a`'s ORIGINAL pre-batch `active:
/// true` as a second Update's OCC pre-image (Phase 1 performs no writes, so
/// both Updates to `a` independently pass OCC against the same unwritten
/// pristine row) must reject the whole batch -- `b`'s claim already consumed
/// the one release credit this scene can grant, and `a`'s own reassertion is
/// a SECOND activation on an already-claimed scene, not a legitimate revert.
#[tokio::test]
async fn a_same_doc_double_update_cannot_reclaim_a_release_already_spent_by_another_combat() {
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
    let scene = Uuid::from_u128(0x5CE);
    let a = combat_doc(1, w.id, scene, true);
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Create { doc: a.clone() }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let b = combat_doc(2, w.id, scene, true);
    let deactivate_a = activate(a.id, false);
    let create_b = Operation::Create { doc: b.clone() };
    // Re-declares the PRISTINE pre-batch value (`true`) as this op's own OCC
    // pre-image, rather than the value `deactivate_a` would have left behind
    // had it committed first — the exploit this pins.
    let reassert_a_active = Operation::Update {
        doc_id: a.id,
        changes: vec![FieldChange {
            remove: false,
            path: "/engine/active".into(),
            old: serde_json::json!(true),
            new: serde_json::json!(true),
        }],
    };
    let res = r
        .apply_intent(
            &ctx,
            w.id,
            vec![deactivate_a, create_b, reassert_a_active],
            2,
            WriteOrigin::Client,
        )
        .await;
    assert!(matches!(res, Err(DataError::Conflict(_))));
    // Nothing in the rejected batch landed: `b` never got created, and `a`
    // stayed exactly as it was created.
    assert!(r.get_document(b.id).await.unwrap().is_none());
    let stored_a = r.get_document(a.id).await.unwrap().unwrap();
    assert_eq!(stored_a.engine.unwrap()["active"], serde_json::json!(true));
}

/// A single Update that both moves a combat to a DIFFERENT scene and
/// deactivates it must free the scene it was actually active on (the
/// PRE-merge scene), never the scene it is moving to. A same-batch `Create`
/// activating a different combat on the DESTINATION scene must still reject
/// against that scene's own genuinely-active, batch-untouched combat.
#[tokio::test]
async fn a_scene_rebind_combined_with_deactivate_frees_the_old_scene_not_the_new_one() {
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
    let s1 = Uuid::from_u128(0x51);
    let s2 = Uuid::from_u128(0x52);
    // `z` is genuinely active on `s2` and is never touched by the batch
    // under test.
    let z = combat_doc(1, w.id, s2, true);
    // `x` starts active on `s1` and will be moved to `s2` while deactivating.
    let x = combat_doc(2, w.id, s1, true);
    r.apply_intent(
        &ctx,
        w.id,
        vec![
            Operation::Create { doc: z.clone() },
            Operation::Create { doc: x.clone() },
        ],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let move_and_deactivate_x = Operation::Update {
        doc_id: x.id,
        changes: vec![
            FieldChange {
                remove: false,
                path: "/engine/scene_id".into(),
                old: serde_json::json!(s1.to_string()),
                new: serde_json::json!(s2.to_string()),
            },
            FieldChange {
                remove: false,
                path: "/engine/active".into(),
                old: serde_json::json!(true),
                new: serde_json::json!(false),
            },
        ],
    };
    let w_combat = combat_doc(3, w.id, s2, true);
    let res = r
        .apply_intent(
            &ctx,
            w.id,
            vec![
                move_and_deactivate_x,
                Operation::Create {
                    doc: w_combat.clone(),
                },
            ],
            2,
            WriteOrigin::Client,
        )
        .await;
    assert!(matches!(res, Err(DataError::Conflict(_))));
    assert!(r.get_document(w_combat.id).await.unwrap().is_none());
}

/// A `CombatTransition` batch may `Delete` a document (and its cascade-
/// deleted descendants) without holding `cap::DELETE`; the identical Delete
/// under `Client` origin by the same low-privilege actor is still refused.
#[tokio::test]
async fn combat_transition_may_delete_a_cascading_combat_without_delete_capability() {
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = r.create_user("p", None, ServerRole::User, 0).await.unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    r.add_member(w.id, player, WorldRole::Player).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let p_ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let combat = combat_doc(1, w.id, Uuid::from_u128(0x5CE), false);
    let c = combatant_doc(2, w.id, combat.id);
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![
            Operation::Create {
                doc: combat.clone(),
            },
            Operation::Create { doc: c.clone() },
        ],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    // The player holds no `cap::DELETE` grant on either document (both carry
    // the shipping default `permissions: PermissionSet { default:
    // DocRole::None, .. }`), so an ordinary `Client`-origin Delete refuses.
    let denied = r
        .apply_intent(
            &p_ctx,
            w.id,
            vec![Operation::Delete {
                doc: combat.clone(),
            }],
            2,
            WriteOrigin::Client,
        )
        .await;
    assert!(matches!(denied, Err(DataError::Forbidden)));
    r.apply_intent(
        &p_ctx,
        w.id,
        vec![Operation::Delete {
            doc: combat.clone(),
        }],
        3,
        WriteOrigin::CombatTransition,
    )
    .await
    .unwrap();
    assert!(r.get_document(combat.id).await.unwrap().is_none());
    assert!(
        r.get_document(c.id).await.unwrap().is_none(),
        "cascade-deleted combatant child"
    );
}

/// A `CombatTransition` batch attempting to `Update` an immutable envelope
/// field is rejected with `DataError::Forbidden` -- `required_cap_for_path`'s
/// `None`-branch rejection stays unconditional for every origin, this one
/// included.
#[tokio::test]
async fn combat_transition_update_of_an_immutable_envelope_field_is_forbidden() {
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
    let combat = combat_doc(1, w.id, Uuid::from_u128(0x5CE), false);
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Create {
            doc: combat.clone(),
        }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let update = Operation::Update {
        doc_id: combat.id,
        changes: vec![FieldChange {
            remove: false,
            path: "/id".into(),
            old: serde_json::json!(combat.id),
            new: serde_json::json!(Uuid::from_u128(0xBAD)),
        }],
    };
    let res = r
        .apply_intent(&ctx, w.id, vec![update], 2, WriteOrigin::CombatTransition)
        .await;
    assert!(matches!(res, Err(DataError::Forbidden)));
}

/// Deleting an active combat and Creating a different active combat on its
/// scene, in the SAME batch, must succeed -- the pre-scan that seeds
/// `apply_intent`'s `deactivations_this_batch` set must also recognize a
/// `Delete` of an `active: true` combat as freeing its scene, not only an
/// `Update` transitioning `active` from `true` to `false`. Runs under
/// `Client` origin (not `CombatTransition`) to confirm the fix is unrelated
/// to `cap::DELETE`'s own origin-gated skip.
#[tokio::test]
async fn deleting_an_active_combat_frees_its_scene_for_a_same_batch_create() {
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
    let scene = Uuid::from_u128(0x5CE);
    let a = combat_doc(1, w.id, scene, true);
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Create { doc: a.clone() }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let b = combat_doc(2, w.id, scene, true);
    r.apply_intent(
        &ctx,
        w.id,
        vec![
            Operation::Delete { doc: a.clone() },
            Operation::Create { doc: b.clone() },
        ],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    assert!(r.get_document(a.id).await.unwrap().is_none());
    let stored_b = r.get_document(b.id).await.unwrap().unwrap();
    assert_eq!(stored_b.engine.unwrap()["active"], serde_json::json!(true));
}

/// An `Update` that changes a combat's `scene_id` while `active` stays `true`
/// throughout must free the OLD scene for a same-batch `Create` claiming it --
/// the pre-scan's freeing condition must not require `active` to transition
/// to `false`; moving away from a scene while remaining active vacates that
/// scene just as genuinely as deactivating does. Both op orderings.
#[tokio::test]
async fn a_scene_rebind_that_stays_active_frees_the_old_scene_for_a_same_batch_create() {
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
    let s1 = Uuid::from_u128(0x51);
    let s2 = Uuid::from_u128(0x52);
    let a = combat_doc(1, w.id, s1, true);
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Create { doc: a.clone() }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let move_a_stay_active = Operation::Update {
        doc_id: a.id,
        changes: vec![FieldChange {
            remove: false,
            path: "/engine/scene_id".into(),
            old: serde_json::json!(s1.to_string()),
            new: serde_json::json!(s2.to_string()),
        }],
    };
    let c = combat_doc(2, w.id, s1, true);
    r.apply_intent(
        &ctx,
        w.id,
        vec![
            move_a_stay_active.clone(),
            Operation::Create { doc: c.clone() },
        ],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let stored_a = r.get_document(a.id).await.unwrap().unwrap();
    assert_eq!(
        stored_a.engine.as_ref().unwrap()["scene_id"],
        serde_json::json!(s2.to_string())
    );
    assert_eq!(stored_a.engine.unwrap()["active"], serde_json::json!(true));
    let stored_c = r.get_document(c.id).await.unwrap().unwrap();
    assert_eq!(stored_c.engine.unwrap()["active"], serde_json::json!(true));

    // Reverse ordering: Create the new claimant on `s1b` FIRST, then move `x`
    // away from `s1b` -- reuse fresh scenes/docs so this half is independent
    // of the state left by the first half above.
    let s1b = Uuid::from_u128(0x53);
    let s2b = Uuid::from_u128(0x54);
    let x = combat_doc(3, w.id, s1b, true);
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Create { doc: x.clone() }],
        3,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let move_x_stay_active = Operation::Update {
        doc_id: x.id,
        changes: vec![FieldChange {
            remove: false,
            path: "/engine/scene_id".into(),
            old: serde_json::json!(s1b.to_string()),
            new: serde_json::json!(s2b.to_string()),
        }],
    };
    let y = combat_doc(4, w.id, s1b, true);
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Create { doc: y.clone() }, move_x_stay_active],
        4,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let stored_x = r.get_document(x.id).await.unwrap().unwrap();
    assert_eq!(
        stored_x.engine.as_ref().unwrap()["scene_id"],
        serde_json::json!(s2b.to_string())
    );
    let stored_y = r.get_document(y.id).await.unwrap().unwrap();
    assert_eq!(stored_y.engine.unwrap()["active"], serde_json::json!(true));
}

/// A scene-rebind Update that stays active must not accidentally free the
/// DESTINATION scene -- only the scene the combat is LEAVING. A same-batch
/// move onto an already-occupied scene must still conflict against that
/// scene's real, batch-untouched occupant. Mirrors
/// `a_scene_rebind_combined_with_deactivate_frees_the_old_scene_not_the_new_one`,
/// which pins the same non-double-free property for the merged-`active:
/// false` branch; this pins it for the new merged-`active: true` branch.
#[tokio::test]
async fn scene_rebind_stays_active_does_not_free_the_destination_scenes_real_occupant() {
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
    let s1 = Uuid::from_u128(0x51);
    let s2 = Uuid::from_u128(0x52);
    // `z` is genuinely active on `s2` and is never touched by the batch.
    let z = combat_doc(1, w.id, s2, true);
    let a = combat_doc(2, w.id, s1, true);
    r.apply_intent(
        &ctx,
        w.id,
        vec![
            Operation::Create { doc: z.clone() },
            Operation::Create { doc: a.clone() },
        ],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    // `a` moves onto `z`'s scene while staying active -- must conflict
    // against `z`'s real, batch-untouched claim on `s2`.
    let move_a_onto_s2 = Operation::Update {
        doc_id: a.id,
        changes: vec![FieldChange {
            remove: false,
            path: "/engine/scene_id".into(),
            old: serde_json::json!(s1.to_string()),
            new: serde_json::json!(s2.to_string()),
        }],
    };
    let res = r
        .apply_intent(&ctx, w.id, vec![move_a_onto_s2], 2, WriteOrigin::Client)
        .await;
    assert!(matches!(res, Err(DataError::Conflict(_))));
    // `a` is untouched: still on `s1`, still active.
    let stored_a = r.get_document(a.id).await.unwrap().unwrap();
    assert_eq!(
        stored_a.engine.as_ref().unwrap()["scene_id"],
        serde_json::json!(s1.to_string())
    );
    assert_eq!(stored_a.engine.unwrap()["active"], serde_json::json!(true));
}
