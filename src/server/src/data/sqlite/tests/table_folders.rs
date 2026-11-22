//! Table-folder placement (`check_table_folder_parent`) and the folder-delete reparent step
//! (`table_folder_reparent_moves`) through both authoritative write paths.

use super::*;

/// Creates `docs` through `apply_intent` as the GM at `ts`.
async fn create_all(
    r: &SqliteRepository,
    ctx: &crate::data::membership::PermissionContext,
    w: Uuid,
    docs: &[Document],
    ts: i64,
) {
    r.apply_intent(
        ctx,
        w,
        docs.iter()
            .map(|d| Operation::Create { doc: d.clone() })
            .collect(),
        ts,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
}

/// Creates one `Create` intent and returns its error, which must be an `OpFailed`.
async fn create_err(
    r: &SqliteRepository,
    ctx: &crate::data::membership::PermissionContext,
    w: Uuid,
    doc: Document,
) -> String {
    let err = r
        .apply_intent(
            ctx,
            w,
            vec![Operation::Create { doc }],
            5,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    match err {
        DataError::OpFailed(m) => m,
        other => panic!("expected OpFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn a_table_and_a_folder_may_be_created_under_a_table_folder_in_the_same_world() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let top = table_folder_doc(1, w, "top", None);
    create_all(&r, &ctx, w, &[top.clone()], 1).await;
    let sub = table_folder_doc(2, w, "sub", Some(top.id));
    let table = table_doc(3, w, Some(top.id));
    create_all(&r, &ctx, w, &[sub.clone(), table.clone()], 2).await;
    assert_eq!(
        r.get_document(table.id).await.unwrap().unwrap().parent_id,
        Some(top.id)
    );
    assert_eq!(
        r.get_document(sub.id).await.unwrap().unwrap().parent_id,
        Some(top.id)
    );
}

#[tokio::test]
async fn a_folder_and_its_table_created_in_one_batch_resolve_through_the_batch() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let top = table_folder_doc(1, w, "top", None);
    let table = table_doc(2, w, Some(top.id));
    create_all(&r, &ctx, w, &[top.clone(), table.clone()], 1).await;
    assert_eq!(
        r.get_document(table.id).await.unwrap().unwrap().parent_id,
        Some(top.id)
    );
}

#[tokio::test]
async fn a_table_created_under_a_non_table_folder_parent_is_rejected() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let note = note_doc(1, w, "n", None);
    let asset_folder = folder_doc(2, w, "a", None);
    create_all(&r, &ctx, w, &[note.clone(), asset_folder.clone()], 1).await;
    for parent in [note.id, asset_folder.id] {
        let msg = create_err(&r, &ctx, w, table_doc(3, w, Some(parent))).await;
        assert_eq!(msg, "table parent must be a table_folder in the same world");
    }
    let msg = create_err(&r, &ctx, w, table_folder_doc(4, w, "f", Some(note.id))).await;
    assert_eq!(
        msg,
        "table_folder parent must be a table_folder in the same world"
    );
}

#[tokio::test]
async fn a_table_folder_cannot_sit_under_a_table() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let table = table_doc(1, w, None);
    create_all(&r, &ctx, w, &[table.clone()], 1).await;
    let msg = create_err(&r, &ctx, w, table_folder_doc(2, w, "f", Some(table.id))).await;
    assert_eq!(
        msg,
        "table_folder parent must be a table_folder in the same world"
    );
}

/// `Move` has no generic parent-world check, so this fails when
/// `check_table_folder_parent` is stubbed to `Ok(())`.
#[tokio::test]
async fn moving_a_table_under_a_table_folder_in_another_world_is_rejected() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let other_gm = r
        .create_user("gm-other", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w2 = r.create_world_owned("W2", other_gm, 0).await.unwrap();
    let ctx2 = crate::data::membership::PermissionContext {
        user_id: other_gm,
        world_role: WorldRole::Gm,
    };
    let foreign = table_folder_doc(9, w2.id, "foreign", None);
    create_all(&r, &ctx2, w2.id, &[foreign.clone()], 1).await;
    let table = table_doc(1, w, None);
    create_all(&r, &ctx, w, &[table.clone()], 2).await;
    let err = r
        .apply_intent(
            &ctx,
            w,
            vec![Operation::Move {
                doc_id: table.id,
                parent_id: Some(foreign.id),
                old_parent_id: None,
            }],
            3,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::OpFailed(_)));
}

/// Builds `top` (child of `outer`, or root) holding `n` tables directly plus `sub` holding `m`
/// tables; returns `(outer, top, sub, direct_ids, nested_ids)` after creating all of them.
async fn folder_tree(
    r: &SqliteRepository,
    ctx: &crate::data::membership::PermissionContext,
    w: Uuid,
    n: u128,
    m: u128,
) -> (Document, Document, Document, Vec<Uuid>, Vec<Uuid>) {
    let outer = table_folder_doc(1, w, "outer", None);
    let top = table_folder_doc(2, w, "top", Some(outer.id));
    let sub = table_folder_doc(3, w, "sub", Some(top.id));
    create_all(r, ctx, w, &[outer.clone(), top.clone(), sub.clone()], 1).await;
    let direct: Vec<Document> = (0..n)
        .map(|i| table_doc(100 + i, w, Some(top.id)))
        .collect();
    let nested: Vec<Document> = (0..m)
        .map(|i| table_doc(200 + i, w, Some(sub.id)))
        .collect();
    let all: Vec<Document> = direct.iter().chain(nested.iter()).cloned().collect();
    create_all(r, ctx, w, &all, 2).await;
    (
        outer,
        top,
        sub,
        direct.iter().map(|d| d.id).collect(),
        nested.iter().map(|d| d.id).collect(),
    )
}

/// Asserts every id in `ids` is a live table whose parent is `parent`.
async fn assert_tables_under(r: &SqliteRepository, ids: &[Uuid], parent: Option<Uuid>) {
    for id in ids {
        let d = r
            .get_document(*id)
            .await
            .unwrap()
            .expect("table survives its folder's deletion");
        assert_eq!(d.doc_type, "table");
        assert_eq!(d.parent_id, parent);
    }
}

#[tokio::test]
async fn deleting_a_folder_with_n_tables_leaves_n_tables_reparented_zero_deleted_via_apply_intent()
{
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let (outer, top, sub, direct, nested) = folder_tree(&r, &ctx, w, 3, 2).await;

    let stored = r.get_document(top.id).await.unwrap().unwrap();
    let stored_cmd = r
        .apply_intent(
            &ctx,
            w,
            vec![Operation::Delete { doc: stored }],
            3,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    assert!(r.get_document(top.id).await.unwrap().is_none());
    assert!(r.get_document(sub.id).await.unwrap().is_none());
    assert!(r.get_document(outer.id).await.unwrap().is_some());
    assert_tables_under(&r, &direct, Some(outer.id)).await;
    assert_tables_under(&r, &nested, Some(outer.id)).await;
    // The lift is logged as explicit Moves, one per table, ahead of the folder deletes.
    let moves = stored_cmd
        .command
        .ops
        .iter()
        .filter(|op| matches!(op, Operation::Move { .. }))
        .count();
    assert_eq!(moves, direct.len() + nested.len());
    let deletes: Vec<Uuid> = stored_cmd
        .command
        .ops
        .iter()
        .filter_map(|op| match op {
            Operation::Delete { doc } => Some(doc.id),
            _ => None,
        })
        .collect();
    assert_eq!(deletes.len(), 2, "only the two folders are deleted");
}

#[tokio::test]
async fn deleting_a_folder_with_n_tables_leaves_n_tables_reparented_zero_deleted_via_apply_command()
{
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let (outer, top, sub, direct, nested) = folder_tree(&r, &ctx, w, 3, 2).await;

    let stored = r.get_document(top.id).await.unwrap().unwrap();
    r.apply_command(UnsequencedCommand {
        world_id: w,
        author: ctx.user_id,
        ts: 3,
        ops: vec![Operation::Delete { doc: stored }],
    })
    .await
    .unwrap();

    assert!(r.get_document(top.id).await.unwrap().is_none());
    assert!(r.get_document(sub.id).await.unwrap().is_none());
    assert_tables_under(&r, &direct, Some(outer.id)).await;
    assert_tables_under(&r, &nested, Some(outer.id)).await;
}

#[tokio::test]
async fn deleting_a_root_level_folder_lands_its_tables_at_the_world_root() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let top = table_folder_doc(1, w, "top", None);
    let table = table_doc(2, w, Some(top.id));
    create_all(&r, &ctx, w, &[top.clone(), table.clone()], 1).await;
    let stored = r.get_document(top.id).await.unwrap().unwrap();
    r.apply_intent(
        &ctx,
        w,
        vec![Operation::Delete { doc: stored }],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    assert_tables_under(&r, &[table.id], None).await;
}

#[tokio::test]
async fn deleting_a_folder_ignores_the_client_supplied_parent_in_the_delete_body() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let (outer, top, _sub, direct, _nested) = folder_tree(&r, &ctx, w, 1, 0).await;
    // The client body claims the folder is top-level; the stored parent (`outer`) wins.
    let mut forged = r.get_document(top.id).await.unwrap().unwrap();
    forged.parent_id = None;
    r.apply_intent(
        &ctx,
        w,
        vec![Operation::Delete { doc: forged }],
        3,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    assert_tables_under(&r, &direct, Some(outer.id)).await;
}

#[tokio::test]
async fn the_reparent_step_emits_no_moves_for_an_empty_folder_or_a_non_folder() {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let folder = table_folder_doc(1, w, "f", None);
    let note = note_doc(2, w, "n", None);
    create_all(&r, &ctx, w, &[folder.clone(), note.clone()], 1).await;
    let mut tx = r.pool.begin().await.unwrap();
    let none = std::collections::HashSet::new();
    assert!(
        SqliteRepository::table_folder_reparent_moves(&mut tx, folder.id, &none)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        SqliteRepository::table_folder_reparent_moves(&mut tx, note.id, &none)
            .await
            .unwrap()
            .is_empty()
    );
}

/// Builds `outer > top > sub` with one table under each, deletes `outer` and `top` in one
/// command in the order given, and asserts every table survives at the world root (the nearest
/// ancestor the command does not delete).
async fn delete_outer_and_nested_in_one_command(outer_first: bool, via_intent: bool) {
    let r = repo().await;
    let (w, ctx) = gm_world(&r).await;
    let outer = table_folder_doc(1, w, "outer", None);
    let top = table_folder_doc(2, w, "top", Some(outer.id));
    let sub = table_folder_doc(3, w, "sub", Some(top.id));
    create_all(&r, &ctx, w, &[outer.clone(), top.clone(), sub.clone()], 1).await;
    let tables = [
        table_doc(10, w, Some(outer.id)),
        table_doc(11, w, Some(top.id)),
        table_doc(12, w, Some(sub.id)),
    ];
    create_all(&r, &ctx, w, &tables, 2).await;
    let outer_stored = r.get_document(outer.id).await.unwrap().unwrap();
    let top_stored = r.get_document(top.id).await.unwrap().unwrap();
    let mut ops = vec![
        Operation::Delete { doc: outer_stored },
        Operation::Delete { doc: top_stored },
    ];
    if !outer_first {
        ops.reverse();
    }
    if via_intent {
        r.apply_intent(&ctx, w, ops, 3, WriteOrigin::Client)
            .await
            .unwrap();
    } else {
        r.apply_command(UnsequencedCommand {
            world_id: w,
            author: ctx.user_id,
            ts: 3,
            ops,
        })
        .await
        .unwrap();
    }
    assert!(r.get_document(outer.id).await.unwrap().is_none());
    assert!(r.get_document(top.id).await.unwrap().is_none());
    assert!(r.get_document(sub.id).await.unwrap().is_none());
    let ids: Vec<Uuid> = tables.iter().map(|t| t.id).collect();
    assert_tables_under(&r, &ids, None).await;
}

#[tokio::test]
async fn deleting_outer_then_nested_folder_in_one_intent_keeps_every_table() {
    delete_outer_and_nested_in_one_command(true, true).await;
}

#[tokio::test]
async fn deleting_nested_then_outer_folder_in_one_intent_keeps_every_table() {
    delete_outer_and_nested_in_one_command(false, true).await;
}

#[tokio::test]
async fn deleting_outer_then_nested_folder_in_one_command_keeps_every_table() {
    delete_outer_and_nested_in_one_command(true, false).await;
}

#[tokio::test]
async fn deleting_nested_then_outer_folder_in_one_command_keeps_every_table() {
    delete_outer_and_nested_in_one_command(false, false).await;
}
