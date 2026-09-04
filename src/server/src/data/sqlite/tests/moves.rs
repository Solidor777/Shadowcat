//! `Operation::Move` through `apply_intent`: authorization, Create-validity
//! placement (including parent world scope), folder cycle rejection
//! (including same-batch Move cycles), OCC on the parent pre-image, the
//! no-op short-circuit, derived-tag recomputation, and bundle survival.

use super::*;
use crate::data::asset::{Asset, AssetMeta};
use crate::data::membership::PermissionContext;

/// A Move op targeting `doc` toward `parent`, with the true stored pre-image.
fn move_op(doc_id: Uuid, parent_id: Option<Uuid>, old_parent_id: Option<Uuid>) -> Operation {
    Operation::Move {
        doc_id,
        parent_id,
        old_parent_id,
    }
}

/// A minimal committed asset row filed under `folder`.
fn asset_in(world: Uuid, folder: Option<Uuid>) -> Asset {
    let id = Uuid::new_v4();
    Asset {
        id,
        world_id: world,
        storage_key: format!("{world}/{id}"),
        original_name: "map.png".into(),
        content_type: "image/png".into(),
        byte_size: 10,
        created_by: None,
        created_at: 1,
        version: 1,
        folder_id: folder,
        tags: vec![],
        derived_tags: vec![],
        meta: AssetMeta::unprocessed("image/png", 1),
    }
}

/// A server-shaped `message` doc authored (and owned) by `author`.
fn message_doc(world: Uuid, author: Uuid) -> Document {
    crate::chat::build_message_doc(
        world,
        author,
        crate::chat::MessageDraft {
            channel: "all".into(),
            actor_owner: None,
            audience: crate::chat::Audience::Public,
            kind: crate::chat::MessageKind::Normal,
            content: crate::chat::plain_text_content("hi"),
            source: None,
        },
        1,
    )
}

/// Creates `docs` in one intent as the given context.
async fn create_all(r: &SqliteRepository, ctx: &PermissionContext, world: Uuid, docs: &[Document]) {
    let ops = docs
        .iter()
        .map(|d| Operation::Create { doc: d.clone() })
        .collect();
    r.apply_intent(ctx, world, ops, 1, WriteOrigin::Client)
        .await
        .unwrap();
}

#[tokio::test]
async fn gm_moves_folder_under_another_folder_updates_parent_and_updated_at() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let a = folder_doc(1, w, "alpha", None);
    let b = folder_doc(2, w, "beta", None);
    create_all(&r, &ctx, w, &[a.clone(), b.clone()]).await;

    r.apply_intent(
        &ctx,
        w,
        vec![move_op(b.id, Some(a.id), None)],
        50,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let moved = r.get_document(b.id).await.unwrap().unwrap();
    assert_eq!(moved.parent_id, Some(a.id));
    assert_eq!(moved.updated_at, 50);
}

#[tokio::test]
async fn non_gm_move_is_forbidden_and_rolls_back_the_whole_batch() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let a = folder_doc(1, w, "alpha", None);
    let b = folder_doc(2, w, "beta", None);
    create_all(&r, &ctx, w, &[a.clone(), b.clone()]).await;
    let player = r.create_user("p", None, ServerRole::User, 0).await.unwrap();
    r.add_member(w, player, WorldRole::Player).await.unwrap();
    let p_ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };

    // A `message` Create is the one doc type a plain player may author, so
    // the batch's Forbidden can only come from the Move op itself.
    let sibling = message_doc(w, player);
    let err = r
        .apply_intent(
            &p_ctx,
            w,
            vec![
                Operation::Create {
                    doc: sibling.clone(),
                },
                move_op(b.id, Some(a.id), None),
            ],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::Forbidden));
    // Whole-batch rollback: the sibling Create must not have landed.
    assert!(r.get_document(sibling.id).await.unwrap().is_none());
    let unmoved = r.get_document(b.id).await.unwrap().unwrap();
    assert_eq!(unmoved.parent_id, None);
}

#[tokio::test]
async fn gm_capped_by_gm_role_on_the_target_cannot_move_it() {
    use crate::data::document::DocRole;
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let a = folder_doc(1, w, "alpha", None);
    let mut capped = folder_doc(2, w, "beta", None);
    // `resolve_access_world`'s GM short-circuit (`Access.all`) is conditional
    // on `gm_role`; a capped GM floor-resolves and must be refused. The capped
    // doc is seeded directly (the intent path would refuse its own Create for
    // the same reason this test exists).
    capped.permissions.gm_role = Some(DocRole::None);
    create_all(&r, &ctx, w, std::slice::from_ref(&a)).await;
    let mut conn = r.pool().acquire().await.unwrap();
    SqliteRepository::upsert_document(&mut conn, &capped, 1)
        .await
        .unwrap();
    drop(conn);

    let err = r
        .apply_intent(
            &ctx,
            w,
            vec![move_op(capped.id, Some(a.id), None)],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::Forbidden));
}

#[tokio::test]
async fn move_into_own_descendant_or_self_is_rejected() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let a = folder_doc(1, w, "alpha", None);
    let b = folder_doc(2, w, "beta", Some(a.id));
    let c = folder_doc(3, w, "gamma", Some(b.id));
    create_all(&r, &ctx, w, &[a.clone(), b.clone(), c.clone()]).await;

    // Self-parent.
    for target in [a.id, b.id] {
        let err = r
            .apply_intent(
                &ctx,
                w,
                vec![move_op(
                    target,
                    Some(target),
                    if target == a.id { None } else { Some(a.id) },
                )],
                2,
                WriteOrigin::Client,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, DataError::OpFailed(_)),
            "self-parent must be refused"
        );
    }
    // Direct child and deep descendant.
    for parent in [b.id, c.id] {
        let err = r
            .apply_intent(
                &ctx,
                w,
                vec![move_op(a.id, Some(parent), None)],
                2,
                WriteOrigin::Client,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, DataError::OpFailed(_)),
            "cycle must be refused"
        );
    }
    let unmoved = r.get_document(a.id).await.unwrap().unwrap();
    assert_eq!(unmoved.parent_id, None);
}

#[tokio::test]
async fn combatant_moves_only_between_combat_parents() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let scene_a = Uuid::from_u128(100);
    let scene_b = Uuid::from_u128(101);
    let c1 = combat_doc(1, w, scene_a, false);
    let c2 = combat_doc(2, w, scene_b, false);
    let fighter = combatant_doc(3, w, c1.id);
    let bystander = world_doc(4, w, serde_json::json!({}));
    create_all(
        &r,
        &ctx,
        w,
        &[c1.clone(), c2.clone(), fighter.clone(), bystander.clone()],
    )
    .await;

    // Toward another combat: legal (Create with that parent would be).
    r.apply_intent(
        &ctx,
        w,
        vec![move_op(fighter.id, Some(c2.id), Some(c1.id))],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    assert_eq!(
        r.get_document(fighter.id).await.unwrap().unwrap().parent_id,
        Some(c2.id)
    );

    // Toward a non-combat parent: refused.
    let err = r
        .apply_intent(
            &ctx,
            w,
            vec![move_op(fighter.id, Some(bystander.id), Some(c2.id))],
            3,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::OpFailed(_)));
}

#[tokio::test]
async fn combat_document_refuses_any_parent() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let a = folder_doc(1, w, "alpha", None);
    let c = combat_doc(2, w, Uuid::from_u128(100), false);
    create_all(&r, &ctx, w, &[a.clone(), c.clone()]).await;

    let err = r
        .apply_intent(
            &ctx,
            w,
            vec![move_op(c.id, Some(a.id), None)],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::OpFailed(_)));
    assert_eq!(r.get_document(c.id).await.unwrap().unwrap().parent_id, None);
}

#[tokio::test]
async fn cross_world_parent_and_missing_parent_are_rejected() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let a = folder_doc(1, w, "alpha", None);
    create_all(&r, &ctx, w, std::slice::from_ref(&a)).await;

    // A folder in ANOTHER world cannot be the target parent.
    let other_gm = r
        .create_user("gm-other", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w2 = r.create_world_owned("W2", other_gm, 0).await.unwrap();
    let ctx2 = PermissionContext {
        user_id: other_gm,
        world_role: WorldRole::Gm,
    };
    let foreign = folder_doc(9, w2.id, "foreign", None);
    create_all(&r, &ctx2, w2.id, std::slice::from_ref(&foreign)).await;

    for bad_parent in [foreign.id, Uuid::new_v4()] {
        let err = r
            .apply_intent(
                &ctx,
                w,
                vec![move_op(a.id, Some(bad_parent), None)],
                2,
                WriteOrigin::Client,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DataError::OpFailed(_)));
    }
}

#[tokio::test]
async fn folder_moves_to_root_when_target_parent_is_none() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let a = folder_doc(1, w, "alpha", None);
    let b = folder_doc(2, w, "beta", Some(a.id));
    create_all(&r, &ctx, w, &[a.clone(), b.clone()]).await;

    r.apply_intent(
        &ctx,
        w,
        vec![move_op(b.id, None, Some(a.id))],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    assert_eq!(r.get_document(b.id).await.unwrap().unwrap().parent_id, None);
}

#[tokio::test]
async fn occ_mismatch_on_old_parent_is_a_conflict() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let a = folder_doc(1, w, "alpha", None);
    let b = folder_doc(2, w, "beta", None);
    create_all(&r, &ctx, w, &[a.clone(), b.clone()]).await;

    let err = r
        .apply_intent(
            &ctx,
            w,
            // Claims b currently sits under a; it is at the root.
            vec![move_op(b.id, None, Some(a.id))],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::Conflict(_)));
}

#[tokio::test]
async fn noop_move_succeeds_without_bumping_updated_at() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let a = folder_doc(1, w, "alpha", None);
    let b = folder_doc(2, w, "beta", Some(a.id));
    create_all(&r, &ctx, w, &[a.clone(), b.clone()]).await;
    let before = r.get_document(b.id).await.unwrap().unwrap().updated_at;

    let stored = r
        .apply_intent(
            &ctx,
            w,
            vec![move_op(b.id, Some(a.id), Some(a.id))],
            99,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    // The op is carried (invertibility of the whole command) …
    assert_eq!(stored.command.ops.len(), 1);
    // … but nothing was written.
    let after = r.get_document(b.id).await.unwrap().unwrap();
    assert_eq!(after.parent_id, Some(a.id));
    assert_eq!(after.updated_at, before);
}

#[tokio::test]
async fn folder_move_recomputes_subtree_derived_tags() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let a = folder_doc(1, w, "alpha", None);
    let b = folder_doc(2, w, "beta", Some(a.id));
    create_all(&r, &ctx, w, &[a.clone(), b.clone()]).await;
    let asset = asset_in(w, Some(b.id));
    r.insert_asset(&asset).await.unwrap();
    r.refresh_derived_tags(asset.id).await.unwrap();
    let seeded = r.get_asset(asset.id).await.unwrap().unwrap();
    assert!(seeded.derived_tags.contains(&"alpha".to_string()));
    assert!(seeded.derived_tags.contains(&"beta".to_string()));

    r.apply_intent(
        &ctx,
        w,
        vec![move_op(b.id, None, Some(a.id))],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let refreshed = r.get_asset(asset.id).await.unwrap().unwrap();
    assert!(
        !refreshed.derived_tags.contains(&"alpha".to_string()),
        "the departed ancestor's folder tag must be gone"
    );
    assert!(refreshed.derived_tags.contains(&"beta".to_string()));
}

#[tokio::test]
async fn move_of_an_embedded_only_id_is_a_conflict() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let child = world_doc(5, w, serde_json::json!({}));
    let mut parent = world_doc(6, w, serde_json::json!({}));
    parent.embedded.insert("item".into(), vec![child.clone()]);
    create_all(&r, &ctx, w, std::slice::from_ref(&parent)).await;

    let err = r
        .apply_intent(
            &ctx,
            w,
            vec![move_op(child.id, None, None)],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::Conflict(_)));
}

#[tokio::test]
async fn message_documents_refuse_move() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let msg = message_doc(w, ctx.user_id);
    create_all(&r, &ctx, w, std::slice::from_ref(&msg)).await;

    let err = r
        .apply_intent(
            &ctx,
            w,
            vec![move_op(msg.id, None, None)],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::OpFailed(_)));
}

#[tokio::test]
async fn moved_folder_tree_survives_a_bundle_round_trip() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let a = folder_doc(1, w, "alpha", None);
    let b = folder_doc(2, w, "beta", Some(a.id));
    create_all(&r, &ctx, w, &[a.clone(), b.clone()]).await;
    let asset = asset_in(w, Some(b.id));
    r.insert_asset(&asset).await.unwrap();
    r.refresh_derived_tags(asset.id).await.unwrap();

    r.apply_intent(
        &ctx,
        w,
        vec![move_op(b.id, None, Some(a.id))],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let export_tmp = tempfile::tempdir().unwrap();
    // The asset store must hold the canonical file for the bundle writer.
    let asset_dir = export_tmp.path().join(w.to_string());
    tokio::fs::create_dir_all(&asset_dir).await.unwrap();
    tokio::fs::write(asset_dir.join(asset.id.to_string()), b"BYTES")
        .await
        .unwrap();
    let export = r.export_world_rows(w).await.unwrap();
    let bytes = crate::world_bundle::write_bundle(&export, export_tmp.path(), Vec::new()).unwrap();
    let tar_path = export_tmp.path().join("bundle.tar");
    tokio::fs::write(&tar_path, &bytes).await.unwrap();

    let target = repo().await;
    target
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let import_tmp = tempfile::tempdir().unwrap();
    let data = crate::world_bundle::read_bundle(&tar_path, import_tmp.path()).unwrap();
    target.import_world(data).await.unwrap();

    let imported_b = target.get_document(b.id).await.unwrap().unwrap();
    assert_eq!(imported_b.parent_id, None, "the post-move parent survives");
    let imported_asset = target.get_asset(asset.id).await.unwrap().unwrap();
    assert!(imported_asset.derived_tags.contains(&"beta".to_string()));
    assert!(!imported_asset.derived_tags.contains(&"alpha".to_string()));
}

#[tokio::test]
async fn same_batch_create_and_move_cannot_form_a_cycle() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let a = folder_doc(1, w, "alpha", None);
    create_all(&r, &ctx, w, std::slice::from_ref(&a)).await;

    // One batch: create B under A, then move A under the not-yet-inserted B.
    let b = folder_doc(2, w, "beta", Some(a.id));
    let err = r
        .apply_intent(
            &ctx,
            w,
            vec![
                Operation::Create { doc: b.clone() },
                move_op(a.id, Some(b.id), None),
            ],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::OpFailed(_)));
    // Whole-batch rollback: B was not created either.
    assert!(r.get_document(b.id).await.unwrap().is_none());
}

#[tokio::test]
async fn generic_doc_cannot_move_to_a_cross_world_parent() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let actor = world_doc(11, w, serde_json::json!({}));
    create_all(&r, &ctx, w, std::slice::from_ref(&actor)).await;

    // A parent that exists — in ANOTHER world. No doc-type-specific placement
    // rule covers an actor, so only the generic parent-scope check can refuse.
    let other_gm = r
        .create_user("gm-other2", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w2 = r.create_world_owned("W2b", other_gm, 0).await.unwrap();
    let ctx2 = PermissionContext {
        user_id: other_gm,
        world_role: WorldRole::Gm,
    };
    let foreign = folder_doc(12, w2.id, "foreign2", None);
    create_all(&r, &ctx2, w2.id, std::slice::from_ref(&foreign)).await;

    let err = r
        .apply_intent(
            &ctx,
            w,
            vec![move_op(actor.id, Some(foreign.id), None)],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::OpFailed(_)));
    assert_eq!(
        r.get_document(actor.id).await.unwrap().unwrap().parent_id,
        None
    );
}

#[tokio::test]
async fn combatant_cannot_move_to_a_cross_world_combat() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let c1 = combat_doc(1, w, Uuid::from_u128(100), false);
    let fighter = combatant_doc(2, w, c1.id);
    create_all(&r, &ctx, w, &[c1.clone(), fighter.clone()]).await;

    let other_gm = r
        .create_user("gm-other3", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w2 = r.create_world_owned("W2c", other_gm, 0).await.unwrap();
    let ctx2 = PermissionContext {
        user_id: other_gm,
        world_role: WorldRole::Gm,
    };
    let c_foreign = combat_doc(3, w2.id, Uuid::from_u128(101), false);
    create_all(&r, &ctx2, w2.id, std::slice::from_ref(&c_foreign)).await;

    // The parent IS a combat — but in another world.
    let err = r
        .apply_intent(
            &ctx,
            w,
            vec![move_op(fighter.id, Some(c_foreign.id), Some(c1.id))],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::OpFailed(_)));
}

#[tokio::test]
async fn same_batch_two_moves_cannot_form_a_cycle() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let a = folder_doc(1, w, "alpha", None);
    let b = folder_doc(2, w, "beta", None);
    create_all(&r, &ctx, w, &[a.clone(), b.clone()]).await;

    // Each op alone is acyclic against pre-batch state; together they swap
    // into a two-cycle. The walk must see the batch's PROSPECTIVE parents.
    let err = r
        .apply_intent(
            &ctx,
            w,
            vec![
                move_op(a.id, Some(b.id), None),
                move_op(b.id, Some(a.id), None),
            ],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::OpFailed(_)));
    assert_eq!(r.get_document(a.id).await.unwrap().unwrap().parent_id, None);
    assert_eq!(r.get_document(b.id).await.unwrap().unwrap().parent_id, None);
}

#[tokio::test]
async fn replayed_two_move_batch_cannot_form_a_cycle() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let a = folder_doc(1, w, "alpha", None);
    let b = folder_doc(2, w, "beta", None);
    create_all(&r, &ctx, w, &[a.clone(), b.clone()]).await;

    // The trusted loop applies ops sequentially in one tx, so the second walk
    // reads the first write and must refuse the closing edge.
    let err = r
        .apply_command(UnsequencedCommand {
            world_id: w,
            author: ctx.user_id,
            ts: 3,
            ops: vec![
                Operation::Move {
                    doc_id: a.id,
                    parent_id: Some(b.id),
                    old_parent_id: None,
                },
                Operation::Move {
                    doc_id: b.id,
                    parent_id: Some(a.id),
                    old_parent_id: None,
                },
            ],
        })
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::OpFailed(_)));
    assert_eq!(r.get_document(a.id).await.unwrap().unwrap().parent_id, None);
}
