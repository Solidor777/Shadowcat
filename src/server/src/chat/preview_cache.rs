//! Per-URL result cache and per-user fetch rate limiter for the link-preview
//! fetcher (`link_preview.rs`). Kept as a separate module because the cache
//! has NO SSRF-guard responsibility of its own — it only remembers outcomes
//! already produced by the guarded fetcher, keyed on the exact fetched URL
//! (post-redirect-resolution; the ingest stage that calls this cache decides
//! what "the fetched URL" means for a given message).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::LinkPreview;
use crate::ws::PingRateLimiter;

/// TTL for a cached SUCCESSFUL fetch. A page's title/description rarely
/// changes within an hour, and re-fetching on every re-render of the same
/// link would multiply outbound requests with no user-visible benefit.
pub const POSITIVE_TTL: Duration = Duration::from_secs(60 * 60);
/// TTL for a cached FAILED fetch (any `PreviewError`). Much shorter than
/// `POSITIVE_TTL` — a transient failure (a slow origin, a momentary 5xx)
/// should not lock a URL out of previews for an hour, but a hot loop of the
/// same bad link must not re-run the full guarded-fetch pipeline on every
/// occurrence either.
pub const NEGATIVE_TTL: Duration = Duration::from_secs(5 * 60);
/// Opportunistic cap on the number of cached URLs. `insert` evicts the
/// single oldest entry (by stamped `Instant`, ignoring TTL) once this many
/// entries are already present — bounds unbounded memory growth from a
/// world where many distinct URLs get pasted, without needing a background
/// sweep task.
pub const MAX_CACHE_ENTRIES: usize = 2048;

/// Per-user fetch budget for the link-preview fetcher. 20/min is generous
/// enough for normal chat use (a handful of links per message) while
/// bounding how many outbound guarded-fetch attempts one user can trigger
/// per minute — each attempt is a real network round-trip subject to the
/// fetcher's own multi-second timeout, so an unbounded budget could tie up
/// a meaningful number of concurrent outbound connections.
pub const PREVIEW_FETCH_PER_MIN: usize = 20;

/// Reused verbatim: `PingRateLimiter` is already a generic per-user
/// sliding-window hit budget (user, now_ms, per_min) with no ping-specific
/// logic in it. See `shadowcat-codebase-chat` / `ws/mod.rs` for the shared
/// implementation.
pub type PreviewRateLimiter = PingRateLimiter;

/// Caches `fetch_preview` outcomes keyed on the exact fetched URL string.
/// `None` stored against a URL is a NEGATIVE cache entry (a prior fetch
/// failed); `Some(preview)` is a positive one. Both are read through `get`,
/// which additionally applies the TTL appropriate to which kind is stored.
#[derive(Default)]
pub struct LinkPreviewCache {
    entries: Mutex<HashMap<String, (Instant, Option<LinkPreview>)>>,
}

impl LinkPreviewCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Looks up `url` at time `now`. Returns:
    /// - `None` — no entry, or the entry expired (TTL selected by whether
    ///   the stored value is a hit or a cached negative).
    /// - `Some(Some(preview))` — a live positive hit.
    /// - `Some(None)` — a live cached-negative hit (caller should not
    ///   re-fetch; the URL is known-bad within `NEGATIVE_TTL`).
    pub fn get(&self, url: &str, now: Instant) -> Option<Option<LinkPreview>> {
        let g = self
            .entries
            .lock()
            .expect("link-preview cache mutex poisoned");
        let (stamped, value) = g.get(url)?;
        let ttl = if value.is_some() {
            POSITIVE_TTL
        } else {
            NEGATIVE_TTL
        };
        if now.saturating_duration_since(*stamped) >= ttl {
            return None;
        }
        Some(value.clone())
    }

    /// Records the outcome of fetching `url` at time `now`. Evicts the
    /// single oldest entry first when already at `MAX_CACHE_ENTRIES` (an
    /// opportunistic bound, not a precise LRU) so an `insert` on a full
    /// cache never grows it past the cap.
    pub fn insert(&self, url: String, result: Option<LinkPreview>, now: Instant) {
        let mut g = self
            .entries
            .lock()
            .expect("link-preview cache mutex poisoned");
        if g.len() >= MAX_CACHE_ENTRIES && !g.contains_key(&url) {
            if let Some(oldest_key) = g
                .iter()
                .min_by_key(|(_, (stamped, _))| *stamped)
                .map(|(k, _)| k.clone())
            {
                g.remove(&oldest_key);
            }
        }
        g.insert(url, (now, result));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preview(url: &str) -> LinkPreview {
        LinkPreview {
            url: url.to_string(),
            title: "t".to_string(),
            description: "d".to_string(),
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

        // 60s later the sliding window has fully rolled past every prior hit.
        let recovered_at = 1_000 + 60_000;
        assert!(lim.check(u, recovered_at, PREVIEW_FETCH_PER_MIN));
    }
}
