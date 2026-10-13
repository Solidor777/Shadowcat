use super::*;

use crate::test_ids::test_uuid;

/// The world every test below stores into unless it names another.
fn world() -> Uuid {
    test_uuid(1)
}

fn preview(url: &str) -> LinkPreview {
    LinkPreview {
        url: url.to_string(),
        title: "t".to_string(),
        description: "d".to_string(),
        image_url: None,
        image_asset_id: None,
    }
}

#[test]
fn hits_within_positive_ttl() {
    let cache = LinkPreviewCache::new();
    let t0 = Instant::now();
    cache.insert(
        world(),
        "https://a.example/".to_string(),
        Some(preview("https://a.example/")),
        t0,
    );

    let just_under = t0 + POSITIVE_TTL - Duration::from_secs(1);
    assert_eq!(
        cache.get(world(), "https://a.example/", just_under),
        Some(Some(preview("https://a.example/")))
    );
}

#[test]
fn misses_after_positive_ttl_expiry() {
    let cache = LinkPreviewCache::new();
    let t0 = Instant::now();
    cache.insert(
        world(),
        "https://a.example/".to_string(),
        Some(preview("https://a.example/")),
        t0,
    );

    let after = t0 + POSITIVE_TTL;
    assert_eq!(cache.get(world(), "https://a.example/", after), None);
}

#[test]
fn negative_hit_within_negative_ttl_then_miss_after() {
    let cache = LinkPreviewCache::new();
    let t0 = Instant::now();
    cache.insert(world(), "https://bad.example/".to_string(), None, t0);

    let just_under = t0 + NEGATIVE_TTL - Duration::from_secs(1);
    assert_eq!(
        cache.get(world(), "https://bad.example/", just_under),
        Some(None)
    );

    let after = t0 + NEGATIVE_TTL;
    assert_eq!(cache.get(world(), "https://bad.example/", after), None);
}

#[test]
fn unknown_url_is_a_miss() {
    let cache = LinkPreviewCache::new();
    assert_eq!(
        cache.get(world(), "https://never-inserted.example/", Instant::now()),
        None
    );
}

#[test]
fn eviction_drops_the_oldest_entry_past_the_cap() {
    let cache = LinkPreviewCache::new();
    let base = Instant::now();

    for i in 0..MAX_CACHE_ENTRIES {
        let url = format!("https://u{i}.example/");
        cache.insert(world(), url, None, base + Duration::from_millis(i as u64));
    }
    // The oldest entry (u0) is still present at exactly the cap.
    assert!(cache
        .get(
            world(),
            "https://u0.example/",
            base + Duration::from_millis(1)
        )
        .is_some());

    // One more insert must evict u0 (the oldest by stamped time) to stay
    // opportunistically bounded.
    let newest_stamp = base + Duration::from_millis(MAX_CACHE_ENTRIES as u64);
    cache.insert(
        world(),
        "https://uNEW.example/".to_string(),
        None,
        newest_stamp,
    );

    assert_eq!(
        cache.get(world(), "https://u0.example/", newest_stamp),
        None,
        "oldest entry should have been evicted"
    );
    assert!(cache
        .get(world(), "https://uNEW.example/", newest_stamp)
        .is_some());
}

#[test]
fn an_entry_is_a_miss_for_another_world() {
    let cache = LinkPreviewCache::new();
    let t0 = Instant::now();
    let url = "https://a.example/";
    cache.insert(world(), url.to_string(), Some(preview(url)), t0);

    assert_eq!(cache.get(test_uuid(2), url, t0), None);
    assert_eq!(cache.get(world(), url, t0), Some(Some(preview(url))));
}

#[test]
fn rate_limiter_allows_up_to_per_min_then_rejects_and_recovers() {
    let lim = PreviewRateLimiter::new();
    let u = crate::test_ids::test_uuid(42);
    for i in 0..PREVIEW_FETCH_PER_MIN {
        assert!(
            lim.check(u, 1_000 + i as i64, PREVIEW_FETCH_PER_MIN),
            "fetch {i} should be within budget"
        );
    }
    assert!(
        !lim.check(
            u,
            1_000 + PREVIEW_FETCH_PER_MIN as i64,
            PREVIEW_FETCH_PER_MIN
        ),
        "one past the budget must be rejected"
    );

    // A full 60s window past the LAST recorded hit: every prior hit has
    // aged out (`retain` threshold now exceeds the last hit's timestamp),
    // so the whole budget is free again.
    let last_hit = 1_000 + (PREVIEW_FETCH_PER_MIN as i64 - 1);
    let recovered_at = last_hit + 60_001;
    assert!(lim.check(u, recovered_at, PREVIEW_FETCH_PER_MIN));
}

/// The sweep deletes imageless rows past `POSITIVE_TTL` and keeps fresh rows and imaged rows.
#[tokio::test]
async fn sweep_deletes_only_expired_imageless_rows() {
    use crate::data::asset::{Asset, AssetMeta};
    use crate::data::sqlite::SqliteRepository;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let world = repo.create_world("w", 0).await.unwrap().id;
    let now_ms: i64 = 10 * POSITIVE_TTL.as_millis() as i64;
    let stale_ms = now_ms - POSITIVE_TTL.as_millis() as i64 - 1;
    let fresh_ms = now_ms - POSITIVE_TTL.as_millis() as i64 + 1;
    let asset = Uuid::new_v4();
    repo.insert_asset(&Asset {
        id: asset,
        world_id: world,
        storage_key: format!("{world}/{asset}"),
        original_name: "preview.png".to_string(),
        content_type: "image/png".to_string(),
        byte_size: 10,
        created_by: None,
        created_at: 0,
        version: 1,
        folder_id: None,
        tags: vec![],
        derived_tags: vec![],
        meta: AssetMeta::unprocessed("image/png", 1),
    })
    .await
    .unwrap();
    for (url, at) in [
        ("https://stale.example/", stale_ms),
        ("https://fresh.example/", fresh_ms),
        ("https://imaged.example/", stale_ms),
    ] {
        repo.upsert_link_preview_cache(world, url, Some("t"), Some("d"), at)
            .await
            .unwrap();
    }
    repo.set_link_preview_cache_image(world, "https://imaged.example/", asset)
        .await
        .unwrap();

    let deleted = sweep_expired_link_previews(repo.pool(), now_ms)
        .await
        .unwrap();

    assert_eq!(deleted, 1);
    for (url, kept) in [
        ("https://stale.example/", false),
        ("https://fresh.example/", true),
        ("https://imaged.example/", true),
    ] {
        let row = repo.get_link_preview_cache(world, url).await.unwrap();
        assert_eq!(row.is_some(), kept, "{url}");
    }
}
