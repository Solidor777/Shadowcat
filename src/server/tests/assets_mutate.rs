//! GM placement mutation over the live HTTP surface: `PATCH` rename / move /
//! retag with the `moved` notice, the bulk route's single transaction, the
//! `created` notice on upload, folder delete with the reparent default and
//! the purge option, and the folder-rename → derived-tag refresh.

use common::{drain_until_type, spawn, Harness, PNG_1X1};
use futures_util::StreamExt;
use shadowcat::data::repository::Repository;
use shadowcat_test_support as common;
use uuid::Uuid;

/// Creates an `asset_folder` document named `name` under `parent`.
async fn create_folder(h: &Harness, name: &str, parent: Option<Uuid>) -> Uuid {
    create_folder_in(h, h.world, name, parent).await
}

/// `create_folder` in an arbitrary world the harness GM owns.
async fn create_folder_in(h: &Harness, world: Uuid, name: &str, parent: Option<Uuid>) -> Uuid {
    let id = Uuid::new_v4();
    let res = h
        .client
        .post(format!("http://{}/api/worlds/{}/documents", h.addr, world))
        .json(&serde_json::json!({
            "id": id,
            "scope": { "kind": "world", "world_id": world },
            "doc_type": "asset_folder",
            "schema_version": 1,
            "name": name,
            "parent_id": parent,
            "engine": { "sort": 0 },
            "system": {},
            "permissions": { "default": "observer", "users": {}, "property_overrides": {} },
            "created_at": 0,
            "updated_at": 0,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "folder create: {:?}", res.text().await);
    id
}

async fn upload_png(h: &Harness, name: &str) -> String {
    let asset: serde_json::Value = h
        .upload(name, "image/png", PNG_1X1.to_vec())
        .await
        .json()
        .await
        .unwrap();
    asset["id"].as_str().unwrap().to_string()
}

async fn get_asset(h: &Harness, id: &str) -> shadowcat::data::asset::Asset {
    h.repo
        .get_asset(Uuid::parse_str(id).unwrap())
        .await
        .unwrap()
        .expect("asset exists")
}

fn has_tag(tags: &[String], t: &str) -> bool {
    tags.iter().any(|x| x == t)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upload_broadcasts_created() {
    let h = spawn().await;
    let mut ws = h.connect().await;
    let _ = ws.next().await; // Welcome
    let id = upload_png(&h, "m.png").await;
    let frame = drain_until_type(&mut ws, "asset_changed").await;
    assert_eq!(frame["uuid"], id);
    assert_eq!(frame["op"], "created");
    assert_eq!(frame["version"], 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn patch_renames_moves_and_retags_and_broadcasts_moved() {
    let h = spawn().await;
    let maps = create_folder(&h, "Maps", None).await;
    let id = upload_png(&h, "m.png").await;
    let mut ws = h.connect().await;
    let _ = ws.next().await; // Welcome

    let res = h
        .client
        .patch(format!("http://{}/api/assets/{id}", h.addr))
        .json(&serde_json::json!({
            "name": " crypt.png ",
            "folder_id": maps,
            "tags": ["hero", " dungeon "],
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "{:?}", res.text().await);
    let updated: serde_json::Value = res.json().await.unwrap();
    assert_eq!(updated["original_name"], "crypt.png");
    assert_eq!(updated["folder_id"], maps.to_string());
    assert_eq!(updated["tags"], serde_json::json!(["dungeon", "hero"]));
    assert_eq!(updated["version"], 1, "placement never bumps the version");
    let derived: Vec<String> = serde_json::from_value(updated["derived_tags"].clone()).unwrap();
    assert!(has_tag(&derived, "Maps"), "{derived:?}");
    assert!(has_tag(&derived, "webp"), "{derived:?}");

    let frame = drain_until_type(&mut ws, "asset_changed").await;
    assert_eq!(frame["uuid"], id);
    assert_eq!(frame["op"], "moved");
    assert_eq!(frame["version"], 1);

    // `folder_id: null` moves back to root; an absent field is unchanged.
    let res = h
        .client
        .patch(format!("http://{}/api/assets/{id}", h.addr))
        .json(&serde_json::json!({ "folder_id": null }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let updated: serde_json::Value = res.json().await.unwrap();
    assert_eq!(updated["folder_id"], serde_json::Value::Null);
    assert_eq!(updated["original_name"], "crypt.png");
    assert_eq!(updated["tags"], serde_json::json!(["dungeon", "hero"]));
    let derived: Vec<String> = serde_json::from_value(updated["derived_tags"].clone()).unwrap();
    assert!(!has_tag(&derived, "Maps"), "{derived:?}");

    // Validation: empty name, bad tag, unknown folder.
    for body in [
        serde_json::json!({ "name": "  " }),
        serde_json::json!({ "tags": [""] }),
        serde_json::json!({ "folder_id": Uuid::new_v4() }),
    ] {
        let res = h
            .client
            .patch(format!("http://{}/api/assets/{id}", h.addr))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 422, "{body}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn patch_is_gm_only_and_rejects_a_cross_world_folder() {
    let h = spawn().await;
    let id = upload_png(&h, "m.png").await;
    let (_pid, cookie) = h.add_player("viewer").await;
    let res = reqwest::Client::new()
        .patch(format!("http://{}/api/assets/{id}", h.addr))
        .header("cookie", &cookie)
        .json(&serde_json::json!({ "name": "hijack.png" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 403);

    // A folder that lives in another world is not a valid destination.
    let other = h.repo.create_world_owned("B", h.user, 0).await.unwrap();
    let foreign = create_folder_in(&h, other.id, "Elsewhere", None).await;
    let res = h
        .client
        .patch(format!("http://{}/api/assets/{id}", h.addr))
        .json(&serde_json::json!({ "folder_id": foreign }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 422);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_moves_and_retags_in_one_transaction() {
    let h = spawn().await;
    let maps = create_folder(&h, "Maps", None).await;
    let a = upload_png(&h, "a.png").await;
    let b = upload_png(&h, "b.png").await;
    let mut ws = h.connect().await;
    let _ = ws.next().await; // Welcome

    // One foreign id poisons the whole batch: nothing applies.
    let res = h
        .client
        .post(format!(
            "http://{}/api/worlds/{}/assets/bulk",
            h.addr, h.world
        ))
        .json(&serde_json::json!({ "ids": [a, b, Uuid::new_v4()], "folder_id": maps }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
    assert_eq!(get_asset(&h, &a).await.folder_id, None);

    let res = h
        .client
        .post(format!(
            "http://{}/api/worlds/{}/assets/bulk",
            h.addr, h.world
        ))
        .json(&serde_json::json!({
            "ids": [a, b],
            "folder_id": maps,
            "add_tags": ["hero"],
            "remove_tags": ["webp"],
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "{:?}", res.text().await);
    let updated: Vec<serde_json::Value> = res.json().await.unwrap();
    assert_eq!(updated.len(), 2);
    for u in &updated {
        assert_eq!(u["folder_id"], maps.to_string());
        assert_eq!(u["tags"], serde_json::json!(["hero"]));
        let derived: Vec<String> = serde_json::from_value(u["derived_tags"].clone()).unwrap();
        assert!(has_tag(&derived, "Maps"));
        // A derived tag cannot be removed: it comes straight back.
        assert!(has_tag(&derived, "webp"));
    }
    let mut moved = Vec::new();
    for _ in 0..2 {
        let frame = drain_until_type(&mut ws, "asset_changed").await;
        assert_eq!(frame["op"], "moved");
        moved.push(frame["uuid"].as_str().unwrap().to_string());
    }
    moved.sort();
    let mut expected = vec![a.clone(), b.clone()];
    expected.sort();
    assert_eq!(moved, expected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn folder_delete_default_reparents_and_purge_deletes_assets() {
    let h = spawn().await;
    // A → B, one asset in each.
    let a = create_folder(&h, "A", None).await;
    let b = create_folder(&h, "B", Some(a)).await;
    let x = upload_png(&h, "x.png").await;
    let y = upload_png(&h, "y.png").await;
    for (id, folder) in [(&x, a), (&y, b)] {
        let res = h
            .client
            .patch(format!("http://{}/api/assets/{id}", h.addr))
            .json(&serde_json::json!({ "folder_id": folder }))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
    }

    // Default: reparent. Deleting A cascades B; x and y land at root.
    let res = h
        .client
        .delete(format!("http://{}/api/asset-folders/{a}", h.addr))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "{:?}", res.text().await);
    assert!(h.repo.get_document(a).await.unwrap().is_none());
    assert!(h.repo.get_document(b).await.unwrap().is_none());
    let x_doc = get_asset(&h, &x).await;
    let y_doc = get_asset(&h, &y).await;
    assert_eq!(x_doc.folder_id, None);
    assert_eq!(y_doc.folder_id, None);
    assert!(!has_tag(&x_doc.derived_tags, "A"));
    assert!(!has_tag(&y_doc.derived_tags, "B"));

    // Purge: assets in the subtree are deleted first (files + notices).
    let c = create_folder(&h, "C", None).await;
    let d = create_folder(&h, "D", Some(c)).await;
    for (id, folder) in [(&x, c), (&y, d)] {
        let res = h
            .client
            .patch(format!("http://{}/api/assets/{id}", h.addr))
            .json(&serde_json::json!({ "folder_id": folder }))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
    }
    let mut ws = h.connect().await;
    let _ = ws.next().await; // Welcome
    let res = h
        .client
        .delete(format!(
            "http://{}/api/asset-folders/{c}?assets=delete",
            h.addr
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "{:?}", res.text().await);
    assert!(h.repo.get_document(c).await.unwrap().is_none());
    assert!(h.repo.get_document(d).await.unwrap().is_none());
    for id in [&x, &y] {
        assert!(h
            .repo
            .get_asset(Uuid::parse_str(id).unwrap())
            .await
            .unwrap()
            .is_none());
        let dir = h.assets_dir.join(h.world.to_string());
        for suffix in ["", ".orig", ".thumb.webp", ".preview.webp"] {
            assert!(!dir.join(format!("{id}{suffix}")).exists(), "{id}{suffix}");
        }
        let res = h
            .client
            .get(format!("http://{}/api/assets/{id}", h.addr))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 404);
    }
    let mut deleted = Vec::new();
    for _ in 0..2 {
        let frame = drain_until_type(&mut ws, "asset_changed").await;
        assert_eq!(frame["op"], "deleted");
        deleted.push(frame["uuid"].as_str().unwrap().to_string());
    }
    deleted.sort();
    let mut expected = vec![x.clone(), y.clone()];
    expected.sort();
    assert_eq!(deleted, expected);

    // Unknown mode → 400; an unknown / non-folder id → 404.
    let res = h
        .client
        .delete(format!(
            "http://{}/api/asset-folders/{}?assets=maybe",
            h.addr,
            Uuid::new_v4()
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);
    let res = h
        .client
        .delete(format!(
            "http://{}/api/asset-folders/{}",
            h.addr,
            Uuid::new_v4()
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn folder_rename_refreshes_contained_assets_derived_tags() {
    let h = spawn().await;
    let a = create_folder(&h, "A", None).await;
    let b = create_folder(&h, "B", Some(a)).await;
    let id = upload_png(&h, "deep.png").await;
    let res = h
        .client
        .patch(format!("http://{}/api/assets/{id}", h.addr))
        .json(&serde_json::json!({ "folder_id": b }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let before = get_asset(&h, &id).await;
    assert!(has_tag(&before.derived_tags, "A") && has_tag(&before.derived_tags, "B"));

    // Rename the ANCESTOR folder through the ordinary document path.
    let res = h
        .client
        .patch(format!("http://{}/api/documents/{a}", h.addr))
        .json(&serde_json::json!({
            "changes": [{ "path": "/name", "old": "A", "new": "Archive" }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "{:?}", res.text().await);
    let after = get_asset(&h, &id).await;
    assert!(
        has_tag(&after.derived_tags, "Archive"),
        "{:?}",
        after.derived_tags
    );
    assert!(
        !has_tag(&after.derived_tags, "A"),
        "{:?}",
        after.derived_tags
    );
    assert!(has_tag(&after.derived_tags, "B"));
}
