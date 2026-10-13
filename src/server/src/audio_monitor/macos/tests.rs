//! Tests for the macOS process-tap backend, run on the macOS matrix leg.
//!
//! The smoke test calls `platform_monitor` and accepts a monitor (polled once, result ignored) or
//! `MonitorError::Unsupported` naming the 14.2 version requirement, and fails on
//! `MonitorError::Backend`. The pure-helper tests cover `departed_pids`, `untapped_processes` and
//! `aggregate_description`. The selector tests pin each hand-written Core Audio selector to its
//! four ASCII characters. One test reads a built `CATapDescription` back through the
//! Objective-C runtime.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};

use super::super::{platform_monitor, MonitorError};
use super::{
    aggregate_description, build_stereo_mixdown_tap_description, departed_pids,
    macos_at_least_14_2, new_tap_uuid, objc_msg_send_0, registered_selector, release_objc_instance,
    untapped_processes, K_AUDIO_DEVICE_PROPERTY_DEVICE_UID,
    K_AUDIO_HARDWARE_PROPERTY_DEFAULT_SYSTEM_OUTPUT_DEVICE,
    K_AUDIO_HARDWARE_PROPERTY_PROCESS_OBJECT_LIST, K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
    K_AUDIO_PROCESS_PROPERTY_PID,
};

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

/// A pid present in the tap map but absent from the current live set is "departed" and must be
/// torn down. `departed_pids` takes no real `TapHandle` or Core Audio resource, so the diff
/// `teardown_departed_taps` runs is exercised in isolation.
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

/// Each hand-written Core Audio selector equals the big-endian value of its four ASCII
/// characters, so a transposed or mistyped code fails here.
#[test]
fn selector_constants_equal_their_four_char_codes() {
    assert_eq!(
        K_AUDIO_HARDWARE_PROPERTY_PROCESS_OBJECT_LIST,
        u32::from_be_bytes(*b"prs#")
    );
    assert_eq!(K_AUDIO_PROCESS_PROPERTY_PID, u32::from_be_bytes(*b"ppid"));
    assert_eq!(
        K_AUDIO_HARDWARE_PROPERTY_DEFAULT_SYSTEM_OUTPUT_DEVICE,
        u32::from_be_bytes(*b"sOut")
    );
    assert_eq!(
        K_AUDIO_DEVICE_PROPERTY_DEVICE_UID,
        u32::from_be_bytes(*b"uid ")
    );
    assert_eq!(
        K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
        u32::from_be_bytes(*b"glob")
    );
}

/// The (pid, object id) pairs returned are exactly the live processes without a tap, ordered by
/// pid, each keeping its own process object id rather than the pid.
#[test]
fn untapped_processes_pairs_each_untapped_pid_with_its_process_object_id() {
    let live: HashMap<i32, u32> = [(300, 71), (100, 52), (200, 63)].into_iter().collect();
    let pairs = untapped_processes(&live, [200].into_iter());
    assert_eq!(pairs, vec![(100, 52), (300, 71)]);
}

/// A tapped pid that is no longer live is not reported, and nothing is reported when every live
/// pid already has a tap.
#[test]
fn untapped_processes_ignores_tapped_pids_and_reports_nothing_when_all_are_tapped() {
    let live: HashMap<i32, u32> = [(1, 10), (2, 20)].into_iter().collect();
    assert!(untapped_processes(&live, [1, 2, 9].into_iter()).is_empty());
}

/// Every minted tap uuid parses as a UUID, is spelled in upper case (the spelling `NSUUID`'s
/// `UUIDString` returns) and differs between calls.
#[test]
fn new_tap_uuid_is_an_upper_case_uuid_that_differs_per_call() {
    let first = new_tap_uuid();
    let second = new_tap_uuid();
    assert!(uuid::Uuid::parse_str(&first).is_ok());
    assert_eq!(first, first.to_uppercase());
    assert_ne!(first, second);
}

fn string_value(dict: &CFDictionary<CFType, CFType>, key: &str) -> String {
    dict.find(CFString::new(key).as_CFType())
        .expect("key present")
        .downcast::<CFString>()
        .expect("string value")
        .to_string()
}

fn number_value(dict: &CFDictionary<CFType, CFType>, key: &str) -> i32 {
    dict.find(CFString::new(key).as_CFType())
        .expect("key present")
        .downcast::<CFNumber>()
        .expect("number value")
        .to_i32()
        .expect("i32 value")
}

fn single_entry_dictionary(
    dict: &CFDictionary<CFType, CFType>,
    key: &str,
) -> CFDictionary<CFType, CFType> {
    let item = dict
        .find(CFString::new(key).as_CFType())
        .expect("key present");
    // SAFETY: the value under `key` is a `CFArray` of `CFDictionary`, per `aggregate_description`.
    let array =
        unsafe { CFArray::<CFType>::wrap_under_get_rule(item.as_CFTypeRef() as CFArrayRef) };
    assert_eq!(array.len(), 1);
    let entry = array.get(0).expect("one entry");
    // SAFETY: the array element is a `CFDictionary`, per `aggregate_description`.
    unsafe { CFDictionary::wrap_under_get_rule(entry.as_CFTypeRef() as CFDictionaryRef) }
}

/// The aggregate description carries each key the Core Audio header names, the header's string
/// spelling for it, the output device as time source and sub-device, and a sub-tap whose `uid` is
/// the tap description's uuid with drift compensation on. The key strings are written out here
/// as literals so a mistyped constant in the module fails.
#[test]
fn aggregate_description_carries_the_keys_core_audio_reads() {
    let dict = aggregate_description(77, "BuiltInSpeakerDevice", "TAP-UUID-0001");

    assert_eq!(string_value(&dict, "name"), "shadowcat-tap-77");
    assert_eq!(string_value(&dict, "uid"), "shadowcat-tap-77");
    assert_eq!(string_value(&dict, "master"), "BuiltInSpeakerDevice");
    assert_eq!(number_value(&dict, "private"), 1);
    assert_eq!(number_value(&dict, "stacked"), 0);
    assert_eq!(number_value(&dict, "tapautostart"), 1);

    let sub_device = single_entry_dictionary(&dict, "subdevices");
    assert_eq!(string_value(&sub_device, "uid"), "BuiltInSpeakerDevice");

    let sub_tap = single_entry_dictionary(&dict, "taps");
    assert_eq!(string_value(&sub_tap, "uid"), "TAP-UUID-0001");
    assert_eq!(number_value(&sub_tap, "drift"), 1);
}

/// A built `CATapDescription` holds the process object id it was given (not a pid) in its
/// `processes` array and the uuid it was given as its `UUID`, read back through the Objective-C
/// runtime.
#[test]
fn tap_description_holds_the_process_object_id_and_the_given_uuid() {
    if !macos_at_least_14_2() {
        return;
    }
    let tap_uuid = new_tap_uuid();
    let description =
        build_stereo_mixdown_tap_description(42, &tap_uuid).expect("tap description builds");
    let uuid_sel = registered_selector("UUID").expect("selector");
    let uuid_string_sel = registered_selector("UUIDString").expect("selector");
    let processes_sel = registered_selector("processes").expect("selector");
    // SAFETY: `description` is a live +1-owned `CATapDescription`; `UUID`, `UUIDString` and
    // `processes` are zero-argument getters returning an object, and the returned objects are
    // retained by `wrap_under_get_rule` before `description` is released.
    let (read_uuid, process_ids) = unsafe {
        let ns_uuid = objc_msg_send_0(description, uuid_sel);
        let uuid_string = objc_msg_send_0(ns_uuid, uuid_string_sel);
        let read_uuid = CFString::wrap_under_get_rule(uuid_string as CFStringRef).to_string();
        let processes = objc_msg_send_0(description, processes_sel);
        let processes = CFArray::<CFType>::wrap_under_get_rule(processes as CFArrayRef);
        release_objc_instance(description);
        (read_uuid, processes)
    };
    assert_eq!(read_uuid, tap_uuid);
    assert_eq!(process_ids.len(), 1);
    let first = process_ids.get(0).expect("one process");
    assert_eq!(
        first.downcast::<CFNumber>().and_then(|n| n.to_i64()),
        Some(42)
    );
}
