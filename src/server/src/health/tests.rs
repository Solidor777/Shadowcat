use super::*;

#[test]
fn ok_reports_ok_status_and_passes_through_db_flag() {
    let s = HealthStatus::ok(true);
    assert_eq!(s.status, "ok");
    assert!(s.db_connected);
}
