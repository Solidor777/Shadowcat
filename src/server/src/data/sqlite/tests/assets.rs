//! Asset row + tag persistence (`data::sqlite::assets`).

use super::*;
use crate::data::asset::query::{AssetFilter, AssetKind};
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
            duration_ms: None,
            sample_rate: None,
            sheet: None,
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

/// One-asset query helper: builds `query`, defers folder/kind/sort/cursor
/// to their defaults, and returns the matched ids.
async fn query_ids(repo: &SqliteRepository, world: Uuid, query: &str) -> Vec<Uuid> {
    repo.query_assets(
        world,
        &AssetFilter {
            query: Some(query.to_string()),
            ..Default::default()
        },
        Default::default(),
        None,
        10,
    )
    .await
    .unwrap()
    .into_iter()
    .map(|a| a.id)
    .collect()
}

#[tokio::test]
async fn query_assets_full_text_matches_a_name_word() {
    let repo = repo().await;
    let world = repo.create_world("w", 1).await.unwrap();
    let a = sample(world.id);
    repo.insert_asset(&a).await.unwrap();
    assert_eq!(query_ids(&repo, world.id, "map").await, vec![a.id]);
    assert!(query_ids(&repo, world.id, "dungeon").await.is_empty());
}

#[tokio::test]
async fn query_assets_full_text_matches_an_explicit_tag() {
    let repo = repo().await;
    let world = repo.create_world("w", 1).await.unwrap();
    let a = sample(world.id);
    repo.insert_asset(&a).await.unwrap();
    repo.set_asset_tags(a.id, &["heroic".into()], &[])
        .await
        .unwrap();
    assert_eq!(query_ids(&repo, world.id, "heroic").await, vec![a.id]);
}

#[tokio::test]
async fn query_assets_full_text_matches_a_derived_tag() {
    let repo = repo().await;
    let world = repo.create_world("w", 1).await.unwrap();
    let a = sample(world.id);
    repo.insert_asset(&a).await.unwrap();
    repo.set_asset_tags(a.id, &[], &["square".into(), "webp".into()])
        .await
        .unwrap();
    assert_eq!(query_ids(&repo, world.id, "square").await, vec![a.id]);
}

#[tokio::test]
async fn query_assets_full_text_refreshes_on_rename() {
    let repo = repo().await;
    let world = repo.create_world("w", 1).await.unwrap();
    let a = sample(world.id);
    repo.insert_asset(&a).await.unwrap();
    assert_eq!(query_ids(&repo, world.id, "map").await, vec![a.id]);
    repo.update_asset_placement(a.id, Some("dungeon.png"), None, None)
        .await
        .unwrap();
    assert!(query_ids(&repo, world.id, "map").await.is_empty());
    assert_eq!(query_ids(&repo, world.id, "dungeon").await, vec![a.id]);
}

#[tokio::test]
async fn query_assets_full_text_stops_matching_after_tag_removal() {
    let repo = repo().await;
    let world = repo.create_world("w", 1).await.unwrap();
    let a = sample(world.id);
    repo.insert_asset(&a).await.unwrap();
    repo.set_asset_tags(a.id, &["heroic".into()], &[])
        .await
        .unwrap();
    assert_eq!(query_ids(&repo, world.id, "heroic").await, vec![a.id]);
    repo.set_asset_tags(a.id, &[], &[]).await.unwrap();
    assert!(query_ids(&repo, world.id, "heroic").await.is_empty());
}

#[tokio::test]
async fn query_assets_full_text_composes_with_folder_kind_tags_and_regex() {
    use crate::data::asset::query::{AssetKind, FolderFilter};

    let repo = repo().await;
    let (world, ctx) = gm_world(&repo).await;
    let folder = folder_doc(1, world, "Heroes", None);
    repo.apply_intent(
        &ctx,
        world,
        vec![Operation::Create {
            doc: folder.clone(),
        }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let mut a = sample(world);
    a.folder_id = Some(folder.id);
    repo.insert_asset(&a).await.unwrap();
    repo.set_asset_tags(a.id, &["heroic".into()], &[])
        .await
        .unwrap();
    let mut b = sample(world);
    b.original_name = "heroic_map2.png".into();
    repo.insert_asset(&b).await.unwrap();

    // Folder scope narrows to `a` alone, even though `b` also matches the text.
    let page = repo
        .query_assets(
            world,
            &AssetFilter {
                folder: Some(FolderFilter::In {
                    folder: folder.id,
                    recursive: false,
                }),
                query: Some("heroic".to_string()),
                ..Default::default()
            },
            Default::default(),
            None,
            10,
        )
        .await
        .unwrap();
    assert_eq!(page.iter().map(|x| x.id).collect::<Vec<_>>(), vec![a.id]);

    // Kind + tags + query compose (an AND of every dimension): only the
    // folder-scoped asset carries the "heroic" tag, so the untagged one
    // is excluded even though its name also matches "map".
    let page = repo
        .query_assets(
            world,
            &AssetFilter {
                kind: Some(AssetKind::Image),
                tags: vec!["heroic".to_string()],
                query: Some("map".to_string()),
                ..Default::default()
            },
            Default::default(),
            None,
            10,
        )
        .await
        .unwrap();
    assert_eq!(page.iter().map(|x| x.id).collect::<Vec<_>>(), vec![a.id]);
}

#[tokio::test]
async fn query_assets_empty_or_punctuation_query_is_empty_page() {
    let repo = repo().await;
    let world = repo.create_world("w", 1).await.unwrap();
    let a = sample(world.id);
    repo.insert_asset(&a).await.unwrap();
    assert!(query_ids(&repo, world.id, "").await.is_empty());
    assert!(query_ids(&repo, world.id, "---").await.is_empty());
}

#[tokio::test]
async fn assets_fts_row_removed_on_asset_delete() {
    let repo = repo().await;
    let world = repo.create_world("w", 1).await.unwrap();
    let a = sample(world.id);
    repo.insert_asset(&a).await.unwrap();
    assert_eq!(query_ids(&repo, world.id, "map").await, vec![a.id]);
    repo.delete_asset(a.id).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM assets_fts WHERE asset_id = ?")
        .bind(a.id.to_string())
        .fetch_one(repo.pool())
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn assets_fts_rows_removed_on_world_delete() {
    let repo = repo().await;
    let world = repo.create_world("w", 1).await.unwrap().id;
    let a = sample(world);
    repo.insert_asset(&a).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM assets_fts WHERE world_id = ?")
        .bind(world.to_string())
        .fetch_one(repo.pool())
        .await
        .unwrap();
    assert_eq!(n, 1);
    repo.delete_world(world).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM assets_fts WHERE world_id = ?")
        .bind(world.to_string())
        .fetch_one(repo.pool())
        .await
        .unwrap();
    assert_eq!(n, 0);
}

/// An audio-kind asset row (WAV label, audio metadata populated).
fn audio_sample(world: Uuid) -> Asset {
    let id = Uuid::new_v4();
    Asset {
        id,
        world_id: world,
        storage_key: format!("{world}/{id}"),
        original_name: "loop.wav".into(),
        content_type: "audio/wav".into(),
        byte_size: 100,
        created_by: None,
        created_at: 1,
        version: 1,
        folder_id: None,
        tags: vec![],
        derived_tags: vec![],
        meta: AssetMeta {
            duration_ms: Some(2_000),
            sample_rate: Some(44_100),
            ..AssetMeta::unprocessed("audio/wav", 100)
        },
    }
}

#[tokio::test]
async fn query_assets_kind_audio_filters_the_content_type_prefix() {
    let repo = repo().await;
    let world = repo.create_world("w", 1).await.unwrap();
    let image = sample(world.id);
    let audio = audio_sample(world.id);
    repo.insert_asset(&image).await.unwrap();
    repo.insert_asset(&audio).await.unwrap();

    let filter = |kind| AssetFilter {
        kind,
        ..Default::default()
    };
    let audio_only = repo
        .query_assets(
            world.id,
            &filter(Some(AssetKind::Audio)),
            Default::default(),
            None,
            10,
        )
        .await
        .unwrap();
    assert_eq!(
        audio_only.iter().map(|x| x.id).collect::<Vec<_>>(),
        vec![audio.id]
    );
    let other = repo
        .query_assets(
            world.id,
            &filter(Some(AssetKind::Other)),
            Default::default(),
            None,
            10,
        )
        .await
        .unwrap();
    assert!(
        other.is_empty(),
        "audio/ is neither image/ nor other/ once the audio kind exists"
    );
}

#[tokio::test]
async fn asset_sheet_meta_round_trips_through_flat_columns() {
    use crate::data::asset::process::SheetMeta;
    let repo = repo().await;
    let world = repo.create_world("w", 1).await.unwrap();
    let mut a = sample(world.id);
    a.meta.sheet = Some(SheetMeta {
        rows: 2,
        cols: 2,
        count: 3,
        frame_ms: vec![100, 100, 100],
        width: 8,
        height: 8,
    });
    repo.insert_asset(&a).await.unwrap();
    let got = repo.get_asset(a.id).await.unwrap().unwrap();
    assert_eq!(got.meta.sheet, a.meta.sheet);

    // replace_asset_bytes persists the same flat columns (and can clear them).
    repo.replace_asset_bytes(
        a.id,
        &a.storage_key,
        "image/webp",
        10,
        &AssetMeta {
            sheet: None,
            ..a.meta.clone()
        },
    )
    .await
    .unwrap();
    let got = repo.get_asset(a.id).await.unwrap().unwrap();
    assert_eq!(got.meta.sheet, None);
}

#[tokio::test]
async fn asset_without_sheet_round_trips_to_none_not_zeroed_fields() {
    let repo = repo().await;
    let world = repo.create_world("w", 1).await.unwrap();
    let a = sample(world.id);
    assert!(a.meta.sheet.is_none());
    repo.insert_asset(&a).await.unwrap();
    let got = repo.get_asset(a.id).await.unwrap().unwrap();
    assert_eq!(got.meta.sheet, None);
}
