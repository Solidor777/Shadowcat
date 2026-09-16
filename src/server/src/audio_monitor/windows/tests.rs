//! Windows-only enumeration smoke test (run on the Windows matrix leg):
//! `platform_monitor` never panics on a headless runner, even one with
//! no active render session (`enumerate_sessions` returning an empty `Vec` is a valid,
//! non-error result, and a missing endpoint surfaces as a runtime `Backend` error from
//! `poll`, not a construction failure).

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::super::platform_monitor;

#[test]
fn platform_monitor_never_panics_and_reports_a_sane_result() {
    let mut m = platform_monitor(Arc::new(AtomicBool::new(false)))
        .expect("Windows always has a working WASAPI backend");
    let _ = m.poll(); // Ok(_) (possibly empty) or MonitorError::Backend — never a panic
}
