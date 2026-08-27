//! Per-URL result cache and per-user fetch rate limiter for the link-preview
//! fetcher (`link_preview`). Kept as a separate module because the cache
//! has NO SSRF-guard responsibility of its own — it only remembers outcomes
//! already produced by the guarded fetcher, keyed on the CANDIDATE URL the
//! ingest stage extracted from the message (`enrich` gets and inserts under
//! the same pre-fetch href; the post-redirect address the fetcher returns is
//! stored in the outcome, never used as the key).

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

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
/// logic in it. `PingRateLimiter` is the shared implementation.
pub type PreviewRateLimiter = PingRateLimiter;

/// Caches `fetch_preview` outcomes keyed on the exact fetched URL string.
/// `None` stored against a URL is a NEGATIVE cache entry (a prior fetch
/// failed); `Some(preview)` is a positive one. Both are read through `get`,
/// which additionally applies the TTL appropriate to which kind is stored.
#[derive(Default)]
pub struct LinkPreviewCache {
    /// URL -> (store time, outcome); `None` outcome = negative entry.
    entries: Mutex<HashMap<String, (Instant, Option<LinkPreview>)>>,
}

impl LinkPreviewCache {
    /// An empty cache (one per server, shared across worlds).
    ///
    /// # Examples
    ///
    /// ```
    /// let cache = shadowcat::chat::LinkPreviewCache::new();
    /// ```
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
mod tests;
