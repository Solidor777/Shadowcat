//! Shared sliding-window throttle for the two Argon2-verifying auth endpoints
//! (`/api/login`, `/api/invites/accept`). Keys are opaque strings composed by
//! the callers (`login:u:<username>`, `login:ip:<ip>`, `invite:u:<uuid>`,
//! `invite:ip:<ip>`). INVARIANT (no enumeration oracle): identity keys count
//! attempts against unknown identities exactly like known ones, and callers
//! return one uniform 429 — the throttle must never behave differently for an
//! identity that exists.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

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

/// Infallible client-IP extractor: `Some` when the server is served with connect-info (production,
/// via `into_make_service_with_connect_info`), `None` under the axum-test mock transport — IP
/// throttling degrades to identity-only there, never a 500. Behind a configured trusted reverse
/// proxy (`Config::trusted_proxies`), resolves through `X-Forwarded-For` instead of the raw TCP
/// peer — see `resolve_client_ip`'s doc for the exact trust-walk algorithm. Default configuration
/// (`trusted_proxies` empty) preserves the original TCP-peer-only behavior exactly.
pub struct ClientIp(pub Option<std::net::IpAddr>);

impl axum::extract::FromRequestParts<crate::http::AppState> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &crate::http::AppState,
    ) -> Result<Self, Self::Rejection> {
        let peer = parts
            .extensions
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|ci| ci.0.ip());
        let Some(peer) = peer else {
            return Ok(ClientIp(None));
        };
        if !state.config.is_trusted_proxy(peer) {
            return Ok(ClientIp(Some(peer)));
        }
        let forwarded = parts
            .headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok());
        Ok(ClientIp(Some(resolve_client_ip(
            peer,
            forwarded,
            &state.config,
        ))))
    }
}

/// Resolves the real client address from `peer` (the already-verified-trusted immediate TCP peer)
/// and `forwarded` (the raw `X-Forwarded-For` header value, if present), walking entries
/// right-to-left (nearest hop first) against `config.is_trusted_proxy`: each trusted-proxy entry
/// is a hop to skip past; the first non-trusted or unparseable entry is the client. Falls back to
/// `peer` if the header is absent/empty, or every entry turns out to be a trusted-proxy address.
///
/// # Examples
///
/// ```
/// use shadowcat::config::Config;
/// use shadowcat::http::throttle::resolve_client_ip;
///
/// let cfg = Config { trusted_proxies: vec!["10.0.0.1".into()], ..Default::default() };
/// let peer = "10.0.0.1".parse().unwrap();
/// let real = resolve_client_ip(peer, Some("203.0.113.9"), &cfg);
/// assert_eq!(real, "203.0.113.9".parse::<std::net::IpAddr>().unwrap());
/// ```
pub fn resolve_client_ip(
    peer: std::net::IpAddr,
    forwarded: Option<&str>,
    config: &crate::config::Config,
) -> std::net::IpAddr {
    let Some(header) = forwarded else { return peer };
    let entries: Vec<&str> = header
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    for entry in entries.iter().rev() {
        match entry.parse::<std::net::IpAddr>() {
            Ok(ip) if config.is_trusted_proxy(ip) => continue, // another trusted hop; keep walking left
            Ok(ip) => return ip,   // first non-trusted entry: the real client
            Err(_) => return peer, // malformed entry: fail safe to the verified TCP peer
        }
    }
    peer // every entry was itself a trusted proxy (or the header was empty) — fall back
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

    #[test]
    fn resolve_client_ip_trusted_peer_no_header_returns_peer() {
        let cfg = crate::config::Config {
            trusted_proxies: vec!["10.0.0.1".into()],
            ..Default::default()
        };
        let peer: std::net::IpAddr = "10.0.0.1".parse().unwrap();
        assert_eq!(resolve_client_ip(peer, None, &cfg), peer);
    }

    #[test]
    fn resolve_client_ip_single_hop_header_returns_the_client() {
        let cfg = crate::config::Config {
            trusted_proxies: vec!["10.0.0.1".into()],
            ..Default::default()
        };
        let peer: std::net::IpAddr = "10.0.0.1".parse().unwrap();
        assert_eq!(
            resolve_client_ip(peer, Some("203.0.113.9"), &cfg),
            "203.0.113.9".parse::<std::net::IpAddr>().unwrap()
        );
    }

    #[test]
    fn resolve_client_ip_walks_past_trusted_hops_to_the_untrusted_entry() {
        let cfg = crate::config::Config {
            trusted_proxies: vec!["10.0.0.1".into()],
            ..Default::default()
        };
        let peer: std::net::IpAddr = "10.0.0.1".parse().unwrap();
        assert_eq!(
            resolve_client_ip(peer, Some("203.0.113.9, 10.0.0.1"), &cfg),
            "203.0.113.9".parse::<std::net::IpAddr>().unwrap()
        );
    }

    // Discrimination check for the all-hops-trusted fallback: every header entry is itself a
    // configured trusted proxy, so the walk must exhaust and fall back to `peer`, not return the
    // last-scanned (leftmost) trusted entry. A version of `resolve_client_ip` that returned the
    // last scanned trusted entry instead of falling back to `peer` would return `10.0.0.1` here
    // (the leftmost/last-scanned entry) rather than `peer` (`10.0.0.1` too, since both proxies
    // share the same address in this fixture) — verified below with a second fixture using two
    // DISTINCT trusted addresses so the two outcomes are actually distinguishable.
    #[test]
    fn resolve_client_ip_falls_back_to_peer_when_every_hop_is_trusted() {
        let cfg = crate::config::Config {
            trusted_proxies: vec!["10.0.0.1".into(), "10.0.0.2".into()],
            ..Default::default()
        };
        let peer: std::net::IpAddr = "10.0.0.1".parse().unwrap();
        // Header's sole entry is the OTHER trusted proxy (10.0.0.2), distinct from `peer`
        // (10.0.0.1) — a last-scanned-trusted-entry implementation would return 10.0.0.2, while
        // the fall-back-to-peer implementation returns 10.0.0.1. The two are different addresses,
        // so this fixture actually discriminates between the two behaviors.
        assert_eq!(resolve_client_ip(peer, Some("10.0.0.2"), &cfg), peer);
    }

    #[test]
    fn resolve_client_ip_malformed_entry_falls_back_to_peer() {
        let cfg = crate::config::Config {
            trusted_proxies: vec!["10.0.0.1".into()],
            ..Default::default()
        };
        let peer: std::net::IpAddr = "10.0.0.1".parse().unwrap();
        assert_eq!(resolve_client_ip(peer, Some("not-an-ip"), &cfg), peer);
    }

    #[test]
    fn resolve_client_ip_empty_or_whitespace_header_falls_back_to_peer() {
        let cfg = crate::config::Config {
            trusted_proxies: vec!["10.0.0.1".into()],
            ..Default::default()
        };
        let peer: std::net::IpAddr = "10.0.0.1".parse().unwrap();
        assert_eq!(resolve_client_ip(peer, Some(""), &cfg), peer);
        assert_eq!(resolve_client_ip(peer, Some("  "), &cfg), peer);
    }
}
