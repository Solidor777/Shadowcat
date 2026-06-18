//! Server time source (wall-clock unix millis) + NTP-style offset calibration.

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
mod time_tests {
    use super::*;

    #[test]
    fn calibrate_computes_offset_and_rtt() {
        // client sends at 1000, receives at 1100 (rtt 100); server stamped 2060.
        // midpoint = 1050; offset = 2060 - 1050 = 1010.
        let (offset, rtt) = calibrate(1000, 1100, 2060);
        assert_eq!(rtt, 100);
        assert_eq!(offset, 1010);
    }

    #[test]
    fn now_millis_is_positive_and_monotone_enough() {
        let a = now_millis();
        let b = now_millis();
        assert!(a > 0 && b >= a);
    }
}
