use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Server health snapshot shared with the client via the ts-rs type pipeline.
/// INVARIANT: the TS mirror in src/types/generated must be regenerated whenever
/// this struct changes (CI enforces sync).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct HealthStatus {
    /// Overall status string; `"ok"` is the only healthy value.
    pub status: String,
    /// Whether the SQLite pool answered the probe query.
    pub db_connected: bool,
}

impl HealthStatus {
    /// An `"ok"` snapshot with the given DB-probe result.
    ///
    /// # Examples
    ///
    /// ```
    /// let h = shadowcat::health::HealthStatus::ok(true);
    /// assert_eq!(h.status, "ok");
    /// assert!(h.db_connected);
    /// ```
    pub fn ok(db_connected: bool) -> Self {
        Self {
            status: "ok".to_string(),
            db_connected,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_reports_ok_status_and_passes_through_db_flag() {
        let s = HealthStatus::ok(true);
        assert_eq!(s.status, "ok");
        assert!(s.db_connected);
    }
}
