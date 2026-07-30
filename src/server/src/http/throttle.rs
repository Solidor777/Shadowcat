//! Shared sliding-window throttle for the two Argon2-verifying auth endpoints
//! (`/api/login`, `/api/invites/accept`). Keys are opaque strings composed by
//! the callers (`login:u:<username>`, `login:ip:<ip>`, `invite:u:<uuid>`,
//! `invite:ip:<ip>`). INVARIANT (no enumeration oracle): identity keys count
//! attempts against unknown identities exactly like known ones, and callers
//! return one uniform 429 — the throttle must never behave differently for an
//! identity that exists.

use std::collections::HashMap;
use std::sync::Mutex;

/// Per-identity login budget (trailing 60 s). Bounds targeted credential
/// stuffing on one account while leaving interactive retry usable.
pub const LOGIN_PER_MIN_PER_IDENTITY: usize = 10;
/// Per-IP login budget — bounds identity-rotating stuffing from one address.
pub const LOGIN_PER_MIN_PER_IP: usize = 30;
/// Per-account invite-redemption budget (the caller is authenticated).
pub const INVITE_PER_MIN_PER_ACCOUNT: usize = 10;
/// Per-IP invite-redemption budget.
pub const INVITE_PER_MIN_PER_IP: usize = 30;
/// Tracked-key capacity. Unauthenticated input mints identity keys, so the map
/// must be bounded; per-IP budgets cap the mint rate per address, and on
/// overflow new keys FAIL CLOSED (throttled) rather than evicting live state.
pub const MAX_TRACKED_KEYS: usize = 65_536;

/// The trailing sliding-window width.
const WINDOW_MS: i64 = 60_000;

/// Sliding-window budgets for login/invite abuse, keyed by opaque strings
/// (`login:user:<name>`, `login:ip:<addr>`, ...). Fails closed at capacity.
pub struct AuthThrottle {
    /// Per-key hit timestamps within the window.
    hits: Mutex<HashMap<String, Vec<i64>>>,
    /// Tracked-key bound (`MAX_TRACKED_KEYS`); new keys throttle on overflow.
    capacity: usize,
}

impl AuthThrottle {
    /// A limiter at the production capacity bound.
    ///
    /// # Examples
    ///
    /// ```
    /// let t = shadowcat::http::throttle::AuthThrottle::new();
    /// assert!(t.check("login:user:testuser-01", 0, 5)); // first hit within budget
    /// ```
    pub fn new() -> Self {
        Self {
            hits: Mutex::new(HashMap::new()),
            capacity: MAX_TRACKED_KEYS,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_capacity_for_test(capacity: usize) -> Self {
        Self {
            hits: Mutex::new(HashMap::new()),
            capacity,
        }
    }

    /// Record an attempt under `key` at `now_ms`; `true` iff within `per_min`
    /// over the trailing 60 s. At capacity, expired keys are swept first; if
    /// the map is still full a NEW key is refused (fail closed).
    pub fn check(&self, key: &str, now_ms: i64, per_min: usize) -> bool {
        let cutoff = now_ms - WINDOW_MS;
        let mut map = self.hits.lock().expect("auth-throttle mutex poisoned");
        if !map.contains_key(key) && map.len() >= self.capacity {
            map.retain(|_, v| v.iter().any(|&t| t > cutoff));
            if map.len() >= self.capacity {
                return false;
            }
        }
        let v = map.entry(key.to_owned()).or_default();
        v.retain(|&t| t > cutoff);
        if v.len() >= per_min {
            return false;
        }
        v.push(now_ms);
        true
    }
}

impl Default for AuthThrottle {
    fn default() -> Self {
        Self::new()
    }
}

/// Infallible client-IP extractor: `Some` when the server is served with
/// connect-info (production `main.rs`), `None` under the axum-test mock
/// transport — IP throttling degrades to identity-only there, never a 500.
pub struct ClientIp(pub Option<std::net::IpAddr>);

impl<S> axum::extract::FromRequestParts<S> for ClientIp
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(ClientIp(
            parts
                .extensions
                .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
                .map(|ci| ci.0.ip()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trips_after_budget_then_window_slides() {
        let t = AuthThrottle::new();
        assert!(t.check("login:u:alice", 1_000, 2));
        assert!(t.check("login:u:alice", 1_500, 2));
        assert!(!t.check("login:u:alice", 1_800, 2)); // 3rd within 60s window
        assert!(t.check("login:u:alice", 62_001, 2)); // window slid
    }

    #[test]
    fn keys_are_independent() {
        let t = AuthThrottle::new();
        assert!(t.check("login:u:alice", 1_000, 1));
        assert!(t.check("login:u:bob", 1_000, 1));
        assert!(!t.check("login:u:alice", 1_100, 1));
    }

    #[test]
    fn at_capacity_new_keys_fail_closed_until_expired_keys_are_swept() {
        let t = AuthThrottle::with_capacity_for_test(2);
        assert!(t.check("k1", 1_000, 5));
        assert!(t.check("k2", 1_000, 5));
        // Map full, new key, nothing expired → fail closed (throttled).
        assert!(!t.check("k3", 1_500, 5));
        // 61s later k1/k2's hits are expired; the sweep frees room.
        assert!(t.check("k3", 62_000, 5));
    }
}
