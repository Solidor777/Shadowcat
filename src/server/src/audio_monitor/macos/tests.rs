//! macOS-only enumeration smoke test (run on the macOS matrix leg):
//! `platform_monitor` never panics — either it returns a working monitor whose
//! first `poll()` succeeds, or (on a macOS version below 14.2, if the runner image is ever
//! older) `MonitorError::Unsupported` naming the version requirement.

use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::super::{platform_monitor, MonitorError};
use super::departed_pids;

#[test]
fn platform_monitor_never_panics_and_reports_a_sane_result() {
    match platform_monitor(Arc::new(AtomicBool::new(false))) {
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

/// The pid diff `teardown_departed_taps` runs before every teardown call: a pid present in the
/// tap map but absent from the current live set is "departed" and must be torn down. Exercises
/// the LOGIC in isolation (`departed_pids` takes no real `TapHandle`/Core Audio resource), which
/// is what `run_process_tap_loop`'s leak fix depends on.
#[test]
fn departed_pids_reports_exactly_the_pids_missing_from_live() {
    let live: HashSet<i32> = [2].into_iter().collect();
    let mut departed = departed_pids([1, 2, 3].into_iter(), &live);
    departed.sort_unstable();
    assert_eq!(departed, vec![1, 3]);
}

/// Every present pid is "departed" against an empty live set — the shape
/// `run_process_tap_loop` uses on its shutdown path to tear down every remaining tap.
#[test]
fn departed_pids_against_an_empty_live_set_is_everything_present() {
    let live: HashSet<i32> = HashSet::new();
    let mut departed = departed_pids([4, 5].into_iter(), &live);
    departed.sort_unstable();
    assert_eq!(departed, vec![4, 5]);
}

/// No pid departs when every present pid is still live.
#[test]
fn departed_pids_is_empty_when_everything_present_is_still_live() {
    let live: HashSet<i32> = [1, 2].into_iter().collect();
    let departed = departed_pids([1, 2].into_iter(), &live);
    assert!(departed.is_empty());
}
