use super::*;

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
        "https://a.example/".to_string(),
        Some(preview("https://a.example/")),
        t0,
    );

    let just_under = t0 + POSITIVE_TTL - Duration::from_secs(1);
    assert_eq!(
        cache.get("https://a.example/", just_under),
        Some(Some(preview("https://a.example/")))
    );
}

#[test]
fn misses_after_positive_ttl_expiry() {
    let cache = LinkPreviewCache::new();
    let t0 = Instant::now();
    cache.insert(
        "https://a.example/".to_string(),
        Some(preview("https://a.example/")),
        t0,
    );

    let after = t0 + POSITIVE_TTL;
    assert_eq!(cache.get("https://a.example/", after), None);
}

#[test]
fn negative_hit_within_negative_ttl_then_miss_after() {
    let cache = LinkPreviewCache::new();
    let t0 = Instant::now();
    cache.insert("https://bad.example/".to_string(), None, t0);

    let just_under = t0 + NEGATIVE_TTL - Duration::from_secs(1);
    assert_eq!(cache.get("https://bad.example/", just_under), Some(None));

    let after = t0 + NEGATIVE_TTL;
    assert_eq!(cache.get("https://bad.example/", after), None);
}

#[test]
fn unknown_url_is_a_miss() {
    let cache = LinkPreviewCache::new();
    assert_eq!(
        cache.get("https://never-inserted.example/", Instant::now()),
        None
    );
}

#[test]
fn eviction_drops_the_oldest_entry_past_the_cap() {
    let cache = LinkPreviewCache::new();
    let base = Instant::now();

    for i in 0..MAX_CACHE_ENTRIES {
        let url = format!("https://u{i}.example/");
        cache.insert(url, None, base + Duration::from_millis(i as u64));
    }
    // The oldest entry (u0) is still present at exactly the cap.
    assert!(cache
        .get("https://u0.example/", base + Duration::from_millis(1))
        .is_some());

    // One more insert must evict u0 (the oldest by stamped time) to stay
    // opportunistically bounded.
    let newest_stamp = base + Duration::from_millis(MAX_CACHE_ENTRIES as u64);
    cache.insert("https://uNEW.example/".to_string(), None, newest_stamp);

    assert_eq!(
        cache.get("https://u0.example/", newest_stamp),
        None,
        "oldest entry should have been evicted"
    );
    assert!(cache.get("https://uNEW.example/", newest_stamp).is_some());
}

#[test]
fn rate_limiter_allows_up_to_per_min_then_rejects_and_recovers() {
    let lim = PreviewRateLimiter::new();
    let u = uuid::Uuid::from_u128(42);
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
