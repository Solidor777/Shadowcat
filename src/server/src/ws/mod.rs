//! Realtime WebSocket event bus: per-world rooms, sequenced broadcasts,
//! ring-buffer + log-backed resync, a server time source, and telemetry.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use uuid::Uuid;

pub mod conn;
pub mod protocol;
pub mod room;
pub mod time;

pub use room::RoomRegistry;

/// Per-user sliding-window ping budget on shared state. Unlike the per-connection
/// window it replaces, a user's N concurrent sockets share one budget — a stronger
/// abuse backstop. 60 s window; over-budget pings drop silently at the call site.
#[derive(Default)]
pub struct PingRateLimiter {
    hits: Mutex<HashMap<Uuid, Vec<i64>>>,
}

impl PingRateLimiter {
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
    pub rooms: Arc<RoomRegistry>,
    /// Per-user ping budget (shared across a user's connections).
    pub ping_rate: Arc<PingRateLimiter>,
}

impl WsState {
    pub fn new() -> Self {
        Self {
            rooms: Arc::new(RoomRegistry::new()),
            ping_rate: Arc::new(PingRateLimiter::new()),
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
