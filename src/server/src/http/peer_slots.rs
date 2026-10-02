//! Per-peer and global connection slots: the ONE definition of how a peer is keyed, how a cap is
//! counted and how a trusted proxy is exempted. The pre-hand-off admission caps
//! (`crate::tls::listener::SniffListener`) and the post-hand-off live-connection caps
//! (`crate::http::conn_serve::serve_router`) each own a [`PeerSlots`] with their own limits and
//! share everything else, so the two phases cannot disagree on who counts as one peer.
//!
//! INVARIANT: a refused admission is logged by [`PeerSlots::admit`] itself, through one bounded
//! summary ([`REFUSAL_WARN_INTERVAL`]): a counter and the most recent refused key, never a map
//! keyed by peer, because the peer is attacker-chosen and a map of refused peers is unbounded
//! memory. Both phases get the same visibility because both call `admit`.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::net::{IpAddr, Ipv6Addr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dashmap::DashMap;

/// The cap key of a peer address: the canonical IPv4 address, or an IPv6 address with its low 64
/// bits cleared. Source: RFC 4291 section 2.5.4 (a /64 is the smallest standard subnet, so every
/// address a single subscriber can hold shares one prefix) and RFC 4291 section 2.5.5.2 (an
/// IPv4-mapped address IS the IPv4 host, so a dual-stack socket must not give one host two keys).
///
/// # Examples
///
/// ```
/// use shadowcat::http::peer_slots::peer_key;
///
/// assert_eq!(peer_key("::ffff:192.0.2.1".parse().unwrap()), peer_key("192.0.2.1".parse().unwrap()));
/// assert_eq!(
///     peer_key("2001:db8:1:2:aaaa::1".parse().unwrap()),
///     peer_key("2001:db8:1:2:bbbb::2".parse().unwrap()),
/// );
/// assert_ne!(
///     peer_key("2001:db8:1:2::1".parse().unwrap()),
///     peer_key("2001:db8:1:3::1".parse().unwrap()),
/// );
/// ```
pub fn peer_key(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => {
            let masked = u128::from(v6) & !u128::from(u64::MAX);
            IpAddr::V6(Ipv6Addr::from(masked))
        }
        v4 => v4,
    }
}

/// Longest interval between two refusal warnings of one [`PeerSlots`]. Refusals in between are
/// counted and reported by the next warning.
pub const REFUSAL_WARN_INTERVAL: Duration = Duration::from_secs(10);

/// Appended to the refusal warning when the per-peer cap refused: many players behind one reverse
/// proxy share the proxy's address, and `Config::trusted_proxies` is what exempts it from the
/// per-peer cap. The global cap's refusal gets no such advice, since no listing relieves it.
const PER_PEER_REMEDY: &str =
    "; if this address is a reverse proxy in front of the server, list it in `trusted_proxies`";

/// The bounded state behind the refusal warning: a count and the time of the last warning.
#[derive(Debug, Default)]
struct Refusals {
    /// Refusals since the last warning.
    since_warn: u64,
    /// When the last warning was issued; `None` before the first refusal.
    last_warn: Option<Instant>,
}

impl Refusals {
    /// Records one refusal at `now`; returns the number of refusals to report when a warning is
    /// due (the first refusal, then at most one per [`REFUSAL_WARN_INTERVAL`]), resetting the
    /// count.
    fn record(&mut self, now: Instant) -> Option<u64> {
        self.since_warn = self.since_warn.saturating_add(1);
        let due = self
            .last_warn
            .is_none_or(|last| now.saturating_duration_since(last) >= REFUSAL_WARN_INTERVAL);
        if !due {
            return None;
        }
        self.last_warn = Some(now);
        Some(std::mem::take(&mut self.since_warn))
    }
}

/// Which cap refused an admission.
#[derive(Debug, Clone, Copy)]
enum Cause {
    /// The global cap was reached.
    Global,
    /// The per-peer cap was reached.
    PerPeer,
}

/// A global cap and a per-[`peer_key`] cap over the slots currently held.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use shadowcat::http::peer_slots::PeerSlots;
///
/// let slots = Arc::new(PeerSlots::new(10, 1));
/// let ip = "192.0.2.1".parse().unwrap();
/// let held = slots.admit(ip, false).unwrap();
/// assert!(slots.admit(ip, false).is_none(), "the per-peer cap is 1");
/// drop(held);
/// assert!(slots.admit(ip, false).is_some(), "dropping a slot releases it");
/// ```
#[derive(Debug)]
pub struct PeerSlots {
    /// Slots currently held across all peers.
    global: AtomicUsize,
    /// The same, per [`peer_key`].
    per_peer: DashMap<IpAddr, u32>,
    /// Maximum slots across all peers.
    global_cap: usize,
    /// Maximum slots for one [`peer_key`].
    per_peer_cap: u32,
    /// What the slots count, for the refusal warning.
    name: &'static str,
    /// The refusal summary.
    refusals: Mutex<Refusals>,
}

/// One held slot; dropping it releases the global and per-peer counts.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use shadowcat::http::peer_slots::PeerSlots;
///
/// let slots = Arc::new(PeerSlots::new(4, 4));
/// let slot = slots.admit("192.0.2.1".parse().unwrap(), false).unwrap();
/// assert_eq!(slots.held(), 1);
/// drop(slot);
/// assert_eq!(slots.held(), 0);
/// ```
#[derive(Debug)]
pub struct Slot {
    /// Counters to release into.
    slots: Arc<PeerSlots>,
    /// The [`peer_key`] whose count was incremented; `None` for a cap-exempt peer.
    peer: Option<IpAddr>,
}

impl PeerSlots {
    /// Counters with the given caps.
    ///
    /// # Examples
    ///
    /// ```
    /// let slots = shadowcat::http::peer_slots::PeerSlots::new(1024, 32);
    /// assert_eq!(slots.held(), 0);
    /// ```
    pub fn new(global_cap: usize, per_peer_cap: u32) -> Self {
        Self {
            global: AtomicUsize::new(0),
            per_peer: DashMap::new(),
            global_cap,
            per_peer_cap,
            name: "connections",
            refusals: Mutex::new(Refusals::default()),
        }
    }

    /// Names what the slots count in the refusal warning (for example "pre-hand-off
    /// connections").
    ///
    /// # Examples
    ///
    /// ```
    /// let slots = shadowcat::http::peer_slots::PeerSlots::new(1, 1).named("live connections");
    /// assert_eq!(slots.held(), 0);
    /// ```
    pub fn named(mut self, name: &'static str) -> Self {
        self.name = name;
        self
    }

    /// Counts a refusal and, when due, logs one summary warning.
    fn refused(&self, key: IpAddr, cause: Cause) {
        let report = self
            .refusals
            .lock()
            .ok()
            .and_then(|mut r| r.record(Instant::now()));
        if let Some(count) = report {
            let remedy = match cause {
                Cause::PerPeer => PER_PEER_REMEDY,
                Cause::Global => "",
            };
            tracing::warn!(
                what = self.name,
                refused = count,
                peer = %key,
                cap = ?cause,
                "connection cap reached: refusing new {}; the peer is an IPv4 address or an IPv6 /64{}",
                self.name,
                remedy
            );
        }
    }

    /// Slots currently held across all peers.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use shadowcat::http::peer_slots::PeerSlots;
    ///
    /// let slots = Arc::new(PeerSlots::new(2, 2));
    /// let _held = slots.admit("192.0.2.1".parse().unwrap(), false).unwrap();
    /// assert_eq!(slots.held(), 1);
    /// ```
    pub fn held(&self) -> usize {
        self.global.load(Ordering::Acquire)
    }

    /// Reserves a slot for `ip`, or `None` when a cap is reached. An `exempt` peer (a trusted
    /// proxy, behind which many players share one address) skips the per-peer cap and stays
    /// bound by the global cap.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use shadowcat::http::peer_slots::PeerSlots;
    ///
    /// let slots = Arc::new(PeerSlots::new(2, 1));
    /// let ip = "192.0.2.1".parse().unwrap();
    /// let _a = slots.admit(ip, true).unwrap();
    /// let _b = slots.admit(ip, true).unwrap();
    /// assert!(slots.admit(ip, true).is_none(), "the global cap still binds an exempt peer");
    /// ```
    pub fn admit(self: &Arc<Self>, ip: IpAddr, exempt: bool) -> Option<Slot> {
        let key = peer_key(ip);
        let reserved = self
            .global
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < self.global_cap).then_some(n + 1)
            })
            .is_ok();
        if !reserved {
            self.refused(key, Cause::Global);
            return None;
        }
        if exempt {
            return Some(Slot {
                slots: self.clone(),
                peer: None,
            });
        }
        let mut allowed = true;
        self.per_peer
            .entry(key)
            .and_modify(|c| {
                if *c < self.per_peer_cap {
                    *c += 1;
                } else {
                    allowed = false;
                }
            })
            .or_insert(1);
        if !allowed {
            self.global.fetch_sub(1, Ordering::AcqRel);
            self.refused(key, Cause::PerPeer);
            return None;
        }
        Some(Slot {
            slots: self.clone(),
            peer: Some(key),
        })
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.slots.global.fetch_sub(1, Ordering::AcqRel);
        if let Some(key) = self.peer {
            let empty = {
                let Some(mut c) = self.slots.per_peer.get_mut(&key) else {
                    return;
                };
                *c = c.saturating_sub(1);
                *c == 0
            };
            if empty {
                self.slots.per_peer.remove_if(&key, |_, c| *c == 0);
            }
        }
    }
}

#[cfg(test)]
mod tests;
