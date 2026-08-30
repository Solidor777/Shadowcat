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
