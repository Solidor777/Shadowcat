//! Server time source (wall-clock unix millis) + NTP-style offset calibration.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::time::{SystemTime, UNIX_EPOCH};

/// Wall-clock unix milliseconds. Used for the server time source and event ts.
///
/// # Examples
///
/// ```
/// use shadowcat::ws::time::now_millis;
///
/// // Unix epoch milliseconds for any date after 1970 is strictly positive.
/// assert!(now_millis() > 0);
/// ```
pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// NTP-style calibration from a single ping/pong round trip.
/// `offset` = server_t - midpoint(client send, client recv); `rtt` = recv - send.
/// A positive offset means the server clock leads the client clock.
///
/// # Examples
///
/// ```
/// use shadowcat::ws::time::calibrate;
///
/// let (offset, rtt) = calibrate(1_000, 1_100, 1_050);
/// assert_eq!(rtt, 100);
/// assert_eq!(offset, 0); // server_t equals the midpoint of client send/recv
/// ```
pub fn calibrate(client_t0: i64, client_t1: i64, server_t: i64) -> (i64, i64) {
    let rtt = client_t1 - client_t0;
    let offset = server_t - (client_t0 + client_t1) / 2;
    (offset, rtt)
}

#[cfg(test)]
mod time_tests;
