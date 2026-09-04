//! Asset row + tag persistence (`data::sqlite::assets`).

use super::*;
use crate::data::asset::{Asset, AssetMeta};

fn sample(world: Uuid) -> Asset {
    let id = Uuid::new_v4();
    Asset {
        id,
        world_id: world,
        storage_key: format!("{world}/{id}"),
        original_name: "map.png".into(),
        content_type: "image/webp".into(),
        byte_size: 10,
        created_by: None,
        created_at: 1,
        version: 1,
        folder_id: None,
        tags: vec![],
        derived_tags: vec![],
        meta: AssetMeta {
            width: Some(4),
            height: Some(4),
            has_alpha: true,
            animated: false,
            original_content_type: "image/png".into(),
            original_byte_size: 20,
            original_retained: true,
            conversion_note: None,
        },
    }
}

#[tokio::test]
async fn asset_round_trips_meta_and_tags() {
    let repo = repo().await;
    let world = repo.create_world("w", 1).await.unwrap();
    let a = sample(world.id);
    repo.insert_asset(&a).await.unwrap();
    repo.set_asset_tags(a.id, &["hero".into()], &["image".into(), "square".into()])
        .await
        .unwrap();
    let got = repo.get_asset(a.id).await.unwrap().unwrap();
    assert_eq!(got.meta, a.meta);
    assert_eq!(got.tags, vec!["hero".to_string()]);
    assert_eq!(
        got.derived_tags,
        vec!["image".to_string(), "square".to_string()]
    );
    // set replaces, never accumulates
    repo.set_asset_tags(a.id, &[], &["image".into()])
        .await
        .unwrap();
    let got = repo.get_asset(a.id).await.unwrap().unwrap();
    assert!(got.tags.is_empty());
    assert_eq!(got.derived_tags, vec!["image".to_string()]);
    // listing carries the same tags
    let listed = repo.list_assets_by_world(world.id).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].derived_tags, vec!["image".to_string()]);
}

#[tokio::test]
async fn folder_delete_reparents_assets_and_cascades_subfolders() {
    let repo = repo().await;
    let (world, ctx) = gm_world(&repo).await;
    let a = folder_doc(1, world, "A", None);
    let b = folder_doc(2, world, "B", Some(a.id));
    repo.apply_intent(
        &ctx,
        world,
        vec![
            Operation::Create { doc: a.clone() },
            Operation::Create { doc: b.clone() },
        ],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let mut x = sample(world);
    x.folder_id = Some(a.id);
    let mut y = sample(world);
    y.folder_id = Some(b.id);
    repo.insert_asset(&x).await.unwrap();
    repo.insert_asset(&y).await.unwrap();

    let stored_a = repo.get_document(a.id).await.unwrap().unwrap();
    repo.apply_intent(
        &ctx,
        world,
        vec![Operation::Delete { doc: stored_a }],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    assert!(repo.get_document(a.id).await.unwrap().is_none());
    assert!(
        repo.get_document(b.id).await.unwrap().is_none(),
        "sub-folder cascades"
    );
    let x = repo.get_asset(x.id).await.unwrap().unwrap();
    let y = repo.get_asset(y.id).await.unwrap().unwrap();
    assert_eq!(
        x.folder_id, None,
        "asset in the deleted folder lands in its parent (root)"
    );
    assert_eq!(
        y.folder_id, None,
        "asset in the cascaded sub-folder lands at root too"
    );
}

#[tokio::test]
async fn folder_parent_must_be_folder_and_acyclic() {
    let repo = repo().await;
    let (world, ctx) = gm_world(&repo).await;
    let actor = world_doc(10, world, serde_json::json!({}));
    let a = folder_doc(1, world, "A", None);
    let b = folder_doc(2, world, "B", Some(a.id));
    repo.apply_intent(
        &ctx,
        world,
        vec![
            Operation::Create { doc: actor.clone() },
            Operation::Create { doc: a.clone() },
            Operation::Create { doc: b.clone() },
        ],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // Parent that is not a folder.
    let bad = folder_doc(3, world, "C", Some(actor.id));
    let err = repo
        .apply_intent(
            &ctx,
            world,
            vec![Operation::Create { doc: bad }],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::OpFailed(_)), "{err:?}");

    // `parent_id` is an immutable envelope path: no Update can re-parent a
    // folder, which is what makes the tree acyclic by construction (see
    // `check_asset_folder_parent`). Pinned here because the invariant above
    // rests on it.
    let err = repo
        .apply_intent(
            &ctx,
            world,
            vec![Operation::Update {
                doc_id: a.id,
                changes: vec![FieldChange {
                    path: "/parent_id".into(),
                    old: serde_json::Value::Null,
                    new: serde_json::json!(b.id.to_string()),
                    remove: false,
                }],
            }],
            3,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::Forbidden), "{err:?}");
}

#[tokio::test]
async fn folder_delete_refreshes_moved_assets_folder_tags() {
    let repo = repo().await;
    let (world, ctx) = gm_world(&repo).await;
    let a = folder_doc(1, world, "A", None);
    let b = folder_doc(2, world, "B", Some(a.id));
    repo.apply_intent(
        &ctx,
        world,
        vec![
            Operation::Create { doc: a.clone() },
            Operation::Create { doc: b.clone() },
        ],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let mut x = sample(world);
    x.folder_id = Some(b.id);
    repo.insert_asset(&x).await.unwrap();
    repo.set_asset_tags(
        x.id,
        &["hero".into()],
        &[
            "A".into(),
            "B".into(),
            "image".into(),
            "link-preview".into(),
        ],
    )
    .await
    .unwrap();

    // Ancestor walk is root-first.
    let mut tx = repo.pool.begin().await.unwrap();
    let names = SqliteRepository::folder_ancestor_names(&mut tx, Some(b.id))
        .await
        .unwrap();
    assert_eq!(names, vec!["A".to_string(), "B".to_string()]);
    assert!(SqliteRepository::folder_ancestor_names(&mut tx, None)
        .await
        .unwrap()
        .is_empty());
    drop(tx);

    let stored_b = repo.get_document(b.id).await.unwrap().unwrap();
    repo.apply_intent(
        &ctx,
        world,
        vec![Operation::Delete { doc: stored_b }],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let x = repo.get_asset(x.id).await.unwrap().unwrap();
    assert_eq!(x.folder_id, Some(a.id));
    assert_eq!(x.tags, vec!["hero".to_string()], "explicit tags untouched");
    // "B" gone, "A" kept, provenance recovered from the old derived set,
    // dimension tags recomputed from the row.
    assert_eq!(
        x.derived_tags,
        vec![
            "A".to_string(),
            "image".to_string(),
            "link-preview".to_string(),
            "square".to_string(),
            "transparent".to_string(),
            "webp".to_string(),
        ]
    );
}
