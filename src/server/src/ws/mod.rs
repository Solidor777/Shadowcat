//! Realtime WebSocket event bus: per-world rooms, sequenced broadcasts,
//! ring-buffer + log-backed resync, a server time source, and telemetry.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use uuid::Uuid;

pub mod conn;
pub mod protocol;
pub mod room;
#[cfg(test)]
pub(crate) mod test_support;
pub mod time;

pub use room::RoomRegistry;

/// Per-user sliding-window ping budget on shared state. Unlike the per-connection
/// window it replaces, a user's N concurrent sockets share one budget — a stronger
/// abuse backstop. 60 s window; over-budget pings drop silently at the call site.
#[derive(Default)]
pub struct PingRateLimiter {
    /// Per-user hit timestamps within the sliding window.
    hits: Mutex<HashMap<Uuid, Vec<i64>>>,
}

impl PingRateLimiter {
    /// An empty limiter (one per `WsState`; budget shared per user).
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::ws::PingRateLimiter;
    ///
    /// let limiter = PingRateLimiter::new();
    /// let user = uuid::Uuid::new_v4();
    /// assert!(limiter.check(user, 0, 5)); // first hit is within budget
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a ping for `user` at `now_ms`, returning whether it is within the
    /// `per_min` budget over the trailing 60 s window.
    pub fn check(&self, user: Uuid, now_ms: i64, per_min: usize) -> bool {
        let mut g = self.hits.lock().expect("ping rate-limiter mutex poisoned");
        let v = g.entry(user).or_default();
        v.retain(|&t| t > now_ms - 60_000);
        if v.len() >= per_min {
            return false;
        }
        v.push(now_ms);
        true
    }
}

/// Realtime state shared in `AppState`. A thin handle today; the seam for future
/// bus internals (actor pool / external broker) without touching callers.
#[derive(Clone)]
pub struct WsState {
    /// The world -> room fan-out registry.
    pub rooms: Arc<RoomRegistry>,
    /// Per-user ping budget (shared across a user's connections).
    pub ping_rate: Arc<PingRateLimiter>,
    /// Per-user chat flood budget (shared across a user's connections).
    pub message_rate: Arc<PingRateLimiter>,
    /// The link-preview SSRF-guarded fetch client, built ONCE via
    /// `chat::build_link_preview_client()` (the no-flag production
    /// constructor — never the test-only loopback-permitting one) and
    /// shared across every connection/world; a preview's target is
    /// world-independent.
    pub link_preview_client: Arc<reqwest::Client>,
    /// In-memory per-URL preview cache, shared across every
    /// connection/world (a URL's preview is world-independent).
    pub link_preview_cache: Arc<crate::chat::LinkPreviewCache>,
    /// Per-user outbound-fetch budget for the link-preview fetcher (shared
    /// across a user's connections, same shape as `message_rate`).
    pub preview_rate: Arc<crate::chat::PreviewRateLimiter>,
    /// In-memory installed-module scan cache, shared across every
    /// connection/world (the installed-module set is server-wide, not
    /// per-world). See `crate::modules::ModuleScanCache`'s own doc for the
    /// invalidation rule.
    pub module_scan_cache: Arc<crate::modules::ModuleScanCache>,
}

impl WsState {
    /// Fresh realtime state (empty registry + limiters).
    ///
    /// # Examples
    ///
    /// ```
    /// let ws = shadowcat::ws::WsState::new();
    /// assert!(ws.rooms.get(uuid::Uuid::nil()).is_none());
    /// ```
    pub fn new() -> Self {
        Self {
            rooms: Arc::new(RoomRegistry::new()),
            ping_rate: Arc::new(PingRateLimiter::new()),
            message_rate: Arc::new(PingRateLimiter::new()),
            link_preview_client: Arc::new(crate::chat::build_link_preview_client()),
            link_preview_cache: Arc::new(crate::chat::LinkPreviewCache::new()),
            preview_rate: Arc::new(crate::chat::PreviewRateLimiter::new()),
            module_scan_cache: Arc::new(crate::modules::ModuleScanCache::new()),
        }
    }

    /// A `WsState` whose rooms use a custom broadcast ring capacity. Test-only:
    /// shrinking the ring forces the lag-driven resync path deterministically.
    pub fn with_broadcast_capacity(capacity: usize) -> Self {
        Self {
            rooms: Arc::new(RoomRegistry::with_capacity(capacity)),
            ping_rate: Arc::new(PingRateLimiter::new()),
            message_rate: Arc::new(PingRateLimiter::new()),
            link_preview_client: Arc::new(crate::chat::build_link_preview_client()),
            link_preview_cache: Arc::new(crate::chat::LinkPreviewCache::new()),
            preview_rate: Arc::new(crate::chat::PreviewRateLimiter::new()),
            module_scan_cache: Arc::new(crate::modules::ModuleScanCache::new()),
        }
    }
}

impl Default for WsState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn ping_limit_is_shared_across_connections_per_user() {
        let lim = PingRateLimiter::new();
        let u = Uuid::from_u128(1);
        for i in 0..30 {
            assert!(lim.check(u, 1_000 + i, 30), "first 30 allowed");
        }
        assert!(!lim.check(u, 1_031, 30), "31st in window denied (per-user)");
        // A different user has an independent budget.
        assert!(lim.check(Uuid::from_u128(2), 1_032, 30));
    }
}
