//! macOS-only enumeration smoke test (run on the macOS matrix leg):
//! `platform_monitor` never panics — either it returns a working monitor whose
//! first `poll()` succeeds, or (on a macOS version below 14.2, if the runner image is ever
//! older) `MonitorError::Unsupported` naming the version requirement.

use super::super::{platform_monitor, MonitorError};

#[test]
fn platform_monitor_never_panics_and_reports_a_sane_result() {
    match platform_monitor() {
        Ok(mut m) => {
            let _ = m.poll();
        }
        Err(MonitorError::Unsupported(reason)) => {
            assert!(reason.contains("14.2"));
        }
        Err(MonitorError::Backend(reason)) => {
            panic!("unexpected backend error on a supported macOS runner: {reason}")
        }
    }
}
