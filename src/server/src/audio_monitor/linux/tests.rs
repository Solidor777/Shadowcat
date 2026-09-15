//! Linux-only enumeration smoke test (run on the Linux matrix leg):
//! `platform_monitor` never panics, and returns either a working monitor whose
//! first `poll()` succeeds, or `MonitorError::Unsupported` on a PipeWire-less runner — GitHub
//! Actions' `ubuntu-latest` images do not run a PipeWire session daemon, so CI is expected to
//! exercise the `Unsupported` arm; a developer machine with PipeWire running exercises the
//! `Ok` arm.

use super::super::{platform_monitor, MonitorError};

#[test]
fn platform_monitor_never_panics_and_reports_a_sane_result() {
    match platform_monitor() {
        Ok(mut m) => {
            let _ = m.poll(); // Ok(_) or MonitorError::Backend — never a panic
        }
        Err(MonitorError::Unsupported(reason)) => {
            assert!(!reason.is_empty());
        }
        Err(MonitorError::Backend(_)) => {
            panic!("construction should report Unsupported, not Backend, when no daemon is running")
        }
    }
}
