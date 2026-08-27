//! Server time source (wall-clock unix millis) + NTP-style offset calibration.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::time::{SystemTime, UNIX_EPOCH};

/// Wall-clock unix milliseconds. Used for the server time source and event ts.
pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// NTP-style calibration from a single ping/pong round trip.
/// `offset` = server_t - midpoint(client send, client recv); `rtt` = recv - send.
/// A positive offset means the server clock leads the client clock.
pub fn calibrate(client_t0: i64, client_t1: i64, server_t: i64) -> (i64, i64) {
    let rtt = client_t1 - client_t0;
    let offset = server_t - (client_t0 + client_t1) / 2;
    (offset, rtt)
}

#[cfg(test)]
mod time_tests;
