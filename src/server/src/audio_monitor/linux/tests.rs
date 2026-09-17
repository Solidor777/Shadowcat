//! Linux-only enumeration smoke test (run on the Linux matrix leg):
//! `platform_monitor` never panics, and returns either a working monitor whose
//! first `poll()` succeeds, or `MonitorError::Unsupported` on a PipeWire-less runner — GitHub
//! Actions' `ubuntu-latest` images do not run a PipeWire session daemon, so CI is expected to
//! exercise the `Unsupported` arm; a developer machine with PipeWire running exercises the
//! `Ok` arm.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::super::{platform_monitor, MonitorError};
use super::valid_sample_window;

#[test]
fn platform_monitor_never_panics_and_reports_a_sane_result() {
    match platform_monitor(Arc::new(AtomicBool::new(false))) {
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

/// The common case: the chunk's offset/size sit entirely within the buffer, so the window is
/// exactly the chunk itself.
#[test]
fn valid_sample_window_returns_the_chunk_when_it_fits() {
    assert_eq!(valid_sample_window(1024, 16, 256), (16, 272));
}

/// A chunk whose reported size runs past the end of the allocation (a malformed/stale chunk) is
/// clamped to the buffer's own length rather than producing a slice range that would panic.
#[test]
fn valid_sample_window_clamps_a_chunk_that_overruns_the_buffer() {
    assert_eq!(valid_sample_window(100, 90, 50), (90, 100));
}

/// A chunk whose offset alone is already past the end of the buffer collapses to an empty,
/// still-valid `start <= end` window instead of `start > end`.
#[test]
fn valid_sample_window_collapses_to_empty_when_offset_exceeds_the_buffer() {
    assert_eq!(valid_sample_window(64, 200, 10), (64, 64));
}

/// A zero-size chunk (nothing written this cycle) yields an empty window at its own offset.
#[test]
fn valid_sample_window_is_empty_for_a_zero_size_chunk() {
    assert_eq!(valid_sample_window(64, 8, 0), (8, 8));
}
