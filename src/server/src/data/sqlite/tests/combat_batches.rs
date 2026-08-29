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
/// `released_active_scenes` must key off a genuine active-true -> false
/// TRANSITION, never merely "ends up inactive", or an unrelated no-op update
/// would wrongly suppress the DB conflict check for the real double-activation.
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
