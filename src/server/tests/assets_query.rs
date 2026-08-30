//! The asset query endpoint over the live HTTP surface: folder scope
//! (root / direct / recursive), all-of tags, kind, case-insensitive name
//! substring, size-capped regex, sorting, keyset pagination followed to the
//! end — and the bare listing contract (no parameters ⇒ a plain array).

use common::{spawn, Harness};
use shadowcat::data::asset::{Asset, AssetMeta};
use shadowcat_test_support as common;
use uuid::Uuid;

/// Creates an `asset_folder` document named `name` under `parent`.
async fn create_folder(h: &Harness, name: &str, parent: Option<Uuid>) -> Uuid {
    let id = Uuid::new_v4();
    let res = h
        .client
        .post(format!(
            "http://{}/api/worlds/{}/documents",
            h.addr, h.world
        ))
        .json(&serde_json::json!({
            "id": id,
            "scope": { "kind": "world", "world_id": h.world },
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

/// Seeds one asset row directly (no bytes on disk — the query never reads them).
async fn seed(
    h: &Harness,
    name: &str,
    content_type: &str,
    size: i64,
    created_at: i64,
    folder: Option<Uuid>,
    explicit: &[&str],
    derived: &[&str],
) -> Uuid {
    let id = Uuid::new_v4();
    h.repo
        .insert_asset(&Asset {
            id,
            world_id: h.world,
            storage_key: format!("{}/{}", h.world, id),
            original_name: name.into(),
            content_type: content_type.into(),
            byte_size: size,
            created_by: Some(h.user),
            created_at,
            version: 1,
            folder_id: folder,
            tags: vec![],
            derived_tags: vec![],
            meta: AssetMeta::unprocessed(content_type, size),
        })
        .await
        .unwrap();
    let explicit: Vec<String> = explicit.iter().map(|s| s.to_string()).collect();
    let derived: Vec<String> = derived.iter().map(|s| s.to_string()).collect();
    h.repo
        .set_asset_tags(id, &explicit, &derived)
        .await
        .unwrap();
    id
}

async fn page(h: &Harness, query: &str) -> serde_json::Value {
    let res = h
        .client
        .get(format!(
            "http://{}/api/worlds/{}/assets?{}",
            h.addr, h.world, query
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "{query}: {:?}", res.text().await);
    res.json().await.unwrap()
}

fn names(page: &serde_json::Value) -> Vec<String> {
    page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["original_name"].as_str().unwrap().to_string())
        .collect()
}

/// Five assets: two at root, two in folder A, one in A/B.
struct Seeded {
    a: Uuid,
    b: Uuid,
}

async fn seed_world(h: &Harness) -> Seeded {
    let a = create_folder(h, "A", None).await;
    let b = create_folder(h, "B", Some(a)).await;
    seed(
        h,
        "Map of Crypt.png",
        "image/webp",
        10,
        1,
        None,
        &["hero"],
        &["image", "webp"],
    )
    .await;
    seed(
        h,
        "notes.pdf",
        "application/pdf",
        40,
        2,
        None,
        &["hero"],
        &["other"],
    )
    .await;
    seed(
        h,
        "crypt",
        "image/webp",
        30,
        3,
        Some(a),
        &[],
        &["image", "webp", "A"],
    )
    .await;
    seed(
        h,
        "big map.jpg",
        "image/jpeg",
        50,
        4,
        Some(a),
        &[],
        &["image", "jpeg", "A"],
    )
    .await;
    seed(
        h,
        "goblin.png",
        "image/png",
        20,
        5,
        Some(b),
        &["hero"],
        &["image", "png", "A", "B"],
    )
    .await;
    Seeded { a, b }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bare_listing_is_still_a_plain_array() {
    let h = spawn().await;
    seed_world(&h).await;
    let res = h
        .client
        .get(format!("http://{}/api/worlds/{}/assets", h.addr, h.world))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    let arr = body
        .as_array()
        .expect("bare listing is an array, not a page");
    assert_eq!(arr.len(), 5);
    assert!(arr.iter().all(|a| a["derived_tags"].is_array()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn folder_tag_kind_and_name_filters() {
    let h = spawn().await;
    let s = seed_world(&h).await;

    assert_eq!(
        names(&page(&h, "folder=root").await),
        vec!["Map of Crypt.png", "notes.pdf"]
    );
    assert_eq!(
        names(&page(&h, &format!("folder={}", s.a)).await),
        vec!["crypt", "big map.jpg"]
    );
    assert_eq!(
        names(&page(&h, &format!("folder={}&recursive=true", s.a)).await),
        vec!["crypt", "big map.jpg", "goblin.png"]
    );
    assert_eq!(
        names(&page(&h, &format!("folder={}", s.b)).await),
        vec!["goblin.png"]
    );
    // All-of tags: explicit + derived mixed.
    assert_eq!(
        names(&page(&h, "tags=hero,image").await),
        vec!["Map of Crypt.png", "goblin.png"]
    );
    assert_eq!(names(&page(&h, "kind=other").await), vec!["notes.pdf"]);
    assert_eq!(
        names(&page(&h, "kind=image&folder=root").await),
        vec!["Map of Crypt.png"]
    );
    // Case-insensitive substring; `%` in the needle is literal, not a wildcard.
    assert_eq!(
        names(&page(&h, "name=MAP").await),
        vec!["Map of Crypt.png", "big map.jpg"]
    );
    assert!(names(&page(&h, "name=%25").await).is_empty());
    // Regex, over the SQL-narrowed rows.
    assert_eq!(
        names(&page(&h, "name_regex=%5Ecr.pt%24").await),
        vec!["crypt"]
    );
    assert_eq!(
        names(&page(&h, "name_regex=(%3Fi)crypt&kind=image").await),
        vec!["Map of Crypt.png", "crypt"]
    );
    // A malformed regex is a 400, never a 500.
    let res = h
        .client
        .get(format!(
            "http://{}/api/worlds/{}/assets?name_regex=(",
            h.addr, h.world
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sort_and_keyset_pagination_walk_to_the_end() {
    let h = spawn().await;
    seed_world(&h).await;

    let mut seen = Vec::new();
    let mut query = "sort=size&limit=2".to_string();
    let mut pages = 0;
    loop {
        let p = page(&h, &query).await;
        let items = names(&p);
        assert!(items.len() <= 2);
        seen.extend(items);
        pages += 1;
        match p["next_cursor"].as_str() {
            Some(c) => query = format!("sort=size&limit=2&cursor={c}"),
            None => break,
        }
    }
    assert_eq!(pages, 3);
    assert_eq!(
        seen,
        vec![
            "Map of Crypt.png",
            "goblin.png",
            "crypt",
            "notes.pdf",
            "big map.jpg"
        ]
    );

    // Name sort is case-insensitive; a regex page still carries a cursor
    // over the rows it examined, so nothing is skipped when it is followed.
    assert_eq!(
        names(&page(&h, "sort=name").await),
        vec![
            "big map.jpg",
            "crypt",
            "goblin.png",
            "Map of Crypt.png",
            "notes.pdf"
        ]
    );
    let first = page(&h, "sort=name&limit=1&name_regex=p").await;
    assert_eq!(names(&first), vec!["big map.jpg"]);
    let cursor = first["next_cursor"].as_str().unwrap().to_string();
    let second = page(
        &h,
        &format!("sort=name&limit=1&name_regex=p&cursor={cursor}"),
    )
    .await;
    assert_eq!(names(&second), vec!["crypt"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn query_is_membership_gated() {
    let h = spawn().await;
    seed_world(&h).await;
    let other = h.repo.create_world("elsewhere", 0).await.unwrap();
    let res = h
        .client
        .get(format!(
            "http://{}/api/worlds/{}/assets?folder=root",
            h.addr, other.id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 403);
}
