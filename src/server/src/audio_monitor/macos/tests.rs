//! Tests for the macOS process-tap backend, run on the macOS matrix leg.
//!
//! The smoke test calls `platform_monitor` and accepts a monitor (polled once, result ignored) or
//! `MonitorError::Unsupported` naming the 14.2 version requirement, and fails on
//! `MonitorError::Backend`. The pure-helper tests cover `departed_pids`, `untapped_processes`,
//! `TapRetryBackoff`, `aggregate_description`, `buffer_list_peak` and `verify_tap_format`. The
//! selector tests pin each hand-written Core Audio selector to its four ASCII characters, and the
//! layout tests pin the `AudioBuffer` size, the `AudioBufferList` field offset that
//! `buffer_list_peak` indexes with, and the `AudioStreamBasicDescription` field offsets.
//! One test reads a built `CATapDescription` back through the Objective-C runtime on macOS 14.2
//! or newer, and asserts the `Unsupported` result of `MacosMonitor::new` below it.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};

use super::super::{platform_monitor, MonitorError};
use super::{
    aggregate_description, buffer_list_peak, build_stereo_mixdown_tap_description, departed_pids,
    macos_at_least_14_2, new_tap_uuid, objc_msg_send_0, registered_selector, release_objc_instance,
    untapped_processes, verify_tap_format, AudioBuffer, AudioBufferList,
    AudioStreamBasicDescription, MacosMonitor, TapRetryBackoff, K_AUDIO_DEVICE_PROPERTY_DEVICE_UID,
    K_AUDIO_FORMAT_FLAG_IS_BIG_ENDIAN, K_AUDIO_FORMAT_FLAG_IS_FLOAT,
    K_AUDIO_FORMAT_FLAG_IS_NON_INTERLEAVED, K_AUDIO_FORMAT_FLAG_IS_PACKED,
    K_AUDIO_FORMAT_LINEAR_PCM, K_AUDIO_HARDWARE_PROPERTY_DEFAULT_SYSTEM_OUTPUT_DEVICE,
    K_AUDIO_HARDWARE_PROPERTY_PROCESS_OBJECT_LIST, K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
    K_AUDIO_PROCESS_PROPERTY_PID, K_AUDIO_TAP_PROPERTY_FORMAT, TAP_RETRY_DELAY,
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

/// A tap whose pid is absent from the live map is "departed" and must be torn down.
/// `departed_pids` takes no real `TapHandle` or Core Audio resource, so the diff
/// `teardown_departed_taps` runs is exercised in isolation.
#[test]
fn departed_pids_reports_exactly_the_taps_missing_from_live() {
    let live: HashMap<i32, u32> = [(2, 20)].into_iter().collect();
    let mut departed = departed_pids([(1, 10), (2, 20), (3, 30)].into_iter(), &live);
    departed.sort_unstable();
    assert_eq!(departed, vec![1, 3]);
}

/// A pid that stays live but now enumerates under a different process object id is "departed":
/// a tap created from an object id that no longer names that pid is torn down.
#[test]
fn departed_pids_reports_a_live_pid_whose_process_object_id_changed() {
    let live: HashMap<i32, u32> = [(7, 99), (8, 80)].into_iter().collect();
    let departed = departed_pids([(7, 70), (8, 80)].into_iter(), &live);
    assert_eq!(departed, vec![7]);
}

/// Every tap is "departed" against an empty live map, the shape `run_process_tap_loop` uses on
/// its shutdown path to tear down every remaining tap.
#[test]
fn departed_pids_against_an_empty_live_map_is_everything_present() {
    let live: HashMap<i32, u32> = HashMap::new();
    let mut departed = departed_pids([(4, 40), (5, 50)].into_iter(), &live);
    departed.sort_unstable();
    assert_eq!(departed, vec![4, 5]);
}

/// No tap departs when every present pid is still live under the same process object id.
#[test]
fn departed_pids_is_empty_when_everything_present_still_matches_live() {
    let live: HashMap<i32, u32> = [(1, 10), (2, 20)].into_iter().collect();
    let departed = departed_pids([(1, 10), (2, 20)].into_iter(), &live);
    assert!(departed.is_empty());
}

/// Each hand-written Core Audio selector, and the linear PCM format id, equals the big-endian
/// value of its four ASCII characters, so a transposed or mistyped code fails here.
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
    assert_eq!(K_AUDIO_TAP_PROPERTY_FORMAT, u32::from_be_bytes(*b"tfmt"));
    assert_eq!(K_AUDIO_FORMAT_LINEAR_PCM, u32::from_be_bytes(*b"lpcm"));
}

/// The Core Audio headers declare `AudioBuffer` as two `UInt32`s and a pointer and
/// `AudioBufferList` as a `UInt32` count followed by the buffer array, so on a 64-bit target the
/// mirrors must measure 16 bytes and place `buffers` at offset 8. `buffer_list_peak` finds the
/// first entry at that offset and steps by that size.
#[test]
fn audio_buffer_layout_matches_the_core_audio_headers() {
    assert_eq!(std::mem::size_of::<AudioBuffer>(), 16);
    assert_eq!(std::mem::offset_of!(AudioBufferList, buffers), 8);
}

/// The (pid, object id) pairs returned are exactly the live processes without a tap, ordered by
/// pid, each keeping its own process object id rather than the pid.
#[test]
fn untapped_processes_pairs_each_untapped_pid_with_its_process_object_id() {
    let live: HashMap<i32, u32> = [(300, 71), (100, 52), (200, 63)].into_iter().collect();
    let pairs = untapped_processes(&live, [(200, 63)].into_iter());
    assert_eq!(pairs, vec![(100, 52), (300, 71)]);
}

/// A tapped pair that is no longer live is not reported, and nothing is reported when every live
/// pair already has a tap.
#[test]
fn untapped_processes_ignores_tapped_pairs_and_reports_nothing_when_all_are_tapped() {
    let live: HashMap<i32, u32> = [(1, 10), (2, 20)].into_iter().collect();
    assert!(untapped_processes(&live, [(1, 10), (2, 20), (9, 90)].into_iter()).is_empty());
}

/// A live pid whose tap was built from a different process object id is reported again, paired
/// with its current object id.
#[test]
fn untapped_processes_reports_a_pid_tapped_under_a_stale_process_object_id() {
    let live: HashMap<i32, u32> = [(5, 55)].into_iter().collect();
    assert_eq!(
        untapped_processes(&live, [(5, 50)].into_iter()),
        vec![(5, 55)]
    );
}

/// A process whose tap failed is retried only after `TAP_RETRY_DELAY`, only the first recorded
/// failure per (pid, object id) reports as new, and a success or a departed process clears the
/// record.
#[test]
fn tap_retry_backoff_delays_a_failed_process_until_the_retry_delay_elapses() {
    let start = Instant::now();
    let mut backoff = TapRetryBackoff::default();
    let key = (11, 110);
    assert!(backoff.should_attempt(key, start));
    assert!(backoff.record_failure(key, start));
    assert!(!backoff.should_attempt(key, start));
    assert!(!backoff.should_attempt(key, start + TAP_RETRY_DELAY - Duration::from_millis(1)));
    assert!(backoff.should_attempt(key, start + TAP_RETRY_DELAY));
    assert!(
        !backoff.record_failure(key, start + TAP_RETRY_DELAY),
        "a repeat failure of the same key is not the first"
    );

    backoff.record_success(key);
    assert!(backoff.should_attempt(key, start));

    assert!(backoff.record_failure(key, start));
    let live: HashMap<i32, u32> = [(11, 111)].into_iter().collect();
    backoff.retain_live(&live);
    assert!(
        backoff.should_attempt(key, start),
        "a pid now enumerating under another object id drops the old key's record"
    );

    assert!(backoff.record_failure(key, start));
    let live_same: HashMap<i32, u32> = [(11, 110)].into_iter().collect();
    backoff.retain_live(&live_same);
    assert!(
        !backoff.should_attempt(key, start),
        "a pair still live under the same object id keeps its delay"
    );

    let empty: HashMap<i32, u32> = HashMap::new();
    backoff.retain_live(&empty);
    assert!(
        backoff.should_attempt(key, start),
        "a pid absent from the live map drops its record"
    );
}

/// Every minted tap uuid is the upper-case hyphenated spelling of a parsed UUID (the spelling
/// `NSUUID`'s `UUIDString` returns) and differs between calls.
#[test]
fn new_tap_uuid_is_an_upper_case_hyphenated_uuid_that_differs_per_call() {
    let first = new_tap_uuid();
    let second = new_tap_uuid();
    let parsed = uuid::Uuid::parse_str(&first).expect("parses as a UUID");
    assert_eq!(first, parsed.hyphenated().to_string().to_uppercase());
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
    assert_eq!(string_value(&dict, "uid"), "shadowcat-tap-TAP-UUID-0001");
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

/// Two aggregate descriptions for the same pid but different tap uuids carry different UIDs.
#[test]
fn aggregate_description_uid_differs_per_tap_uuid() {
    let first = aggregate_description(77, "Out", "TAP-A");
    let second = aggregate_description(77, "Out", "TAP-B");
    assert_ne!(string_value(&first, "uid"), string_value(&second, "uid"));
}

/// A built `CATapDescription` holds the process object id it was given (not a pid) in its
/// `processes` array and the uuid it was given as its `UUID`, read back through the Objective-C
/// runtime. Below macOS 14.2 the test asserts that `MacosMonitor::new` reports `Unsupported`
/// instead.
#[test]
fn tap_description_holds_the_process_object_id_and_the_given_uuid() {
    if !macos_at_least_14_2() {
        match MacosMonitor::new(Arc::new(AtomicBool::new(false))) {
            Err(MonitorError::Unsupported(reason)) => assert!(reason.contains("14.2")),
            Err(MonitorError::Backend(reason)) => panic!("unexpected backend error: {reason}"),
            Ok(_) => panic!("a monitor was built below macOS 14.2"),
        }
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

/// An `AudioBufferList` over-allocated for `N` entries: the count header followed by `N` entries.
#[repr(C)]
struct ListOf<const N: usize> {
    /// The header count `buffer_list_peak` loops over.
    number_buffers: u32,
    /// The `N` entries the header counts.
    buffers: [AudioBuffer; N],
}

/// An `AudioBuffer` over `samples`, which must outlive every read of the returned entry.
fn buffer_over(samples: &mut [f32]) -> AudioBuffer {
    AudioBuffer {
        number_channels: 1,
        data_byte_size: std::mem::size_of_val(samples) as u32,
        data: samples.as_mut_ptr().cast(),
    }
}

/// The peak is the maximum over every buffer in the list, not the first buffer's maximum, and a
/// negative sample counts by magnitude.
#[test]
fn buffer_list_peak_takes_the_maximum_over_every_buffer() {
    let mut first = [0.1f32, -0.2];
    let mut second = [0.3f32, -0.9];
    let list = ListOf::<2> {
        number_buffers: 2,
        buffers: [buffer_over(&mut first), buffer_over(&mut second)],
    };
    // SAFETY: `list` is a live two-entry list whose entries point at live sample arrays.
    let peak = unsafe { buffer_list_peak((&list as *const ListOf<2>).cast::<AudioBufferList>()) };
    assert_eq!(peak, 0.9);
}

/// A buffer with a null data pointer or zero length is skipped without ending the scan of the
/// buffers after it.
#[test]
fn buffer_list_peak_skips_empty_buffers_and_keeps_scanning() {
    let mut samples = [0.25f32, -0.5];
    let empty = AudioBuffer {
        number_channels: 1,
        data_byte_size: 8,
        data: std::ptr::null_mut(),
    };
    let zero_length = AudioBuffer {
        number_channels: 1,
        data_byte_size: 0,
        data: samples.as_mut_ptr().cast(),
    };
    let list = ListOf::<3> {
        number_buffers: 3,
        buffers: [empty, zero_length, buffer_over(&mut samples)],
    };
    // SAFETY: `list` is a live three-entry list; the null and zero-length entries are never read
    // through, and the third points at a live sample array.
    let peak = unsafe { buffer_list_peak((&list as *const ListOf<3>).cast::<AudioBufferList>()) };
    assert_eq!(peak, 0.5);
}

/// A list that declares no buffers reads as silence even though its declared first entry holds
/// samples.
#[test]
fn buffer_list_peak_reads_a_list_with_no_buffers_as_zero() {
    let mut samples = [0.7f32];
    let list = ListOf::<1> {
        number_buffers: 0,
        buffers: [buffer_over(&mut samples)],
    };
    // SAFETY: `list` is a live list; `number_buffers` is 0, so no entry is read.
    let peak = unsafe { buffer_list_peak((&list as *const ListOf<1>).cast::<AudioBufferList>()) };
    assert_eq!(peak, 0.0);
}

/// The Core Audio headers declare `AudioStreamBasicDescription` as a `Float64` followed by eight
/// `UInt32`s, so the mirror measures 40 bytes with `mFormatID` at offset 8, `mFormatFlags` at 12
/// and `mBitsPerChannel` at 32. `kAudioTapPropertyFormat` is read into this struct.
#[test]
fn stream_description_layout_matches_the_core_audio_headers() {
    assert_eq!(std::mem::size_of::<AudioStreamBasicDescription>(), 40);
    assert_eq!(
        std::mem::offset_of!(AudioStreamBasicDescription, format_id),
        8
    );
    assert_eq!(
        std::mem::offset_of!(AudioStreamBasicDescription, format_flags),
        12
    );
    assert_eq!(
        std::mem::offset_of!(AudioStreamBasicDescription, bits_per_channel),
        32
    );
}

/// `kAudioFormatFlagIsSignedInteger`, used only to build refused descriptions.
const K_AUDIO_FORMAT_FLAG_IS_SIGNED_INTEGER: u32 = 1 << 2;

/// A stereo 48 kHz interleaved description with the given id, flags and sample width.
fn description(
    format_id: u32,
    format_flags: u32,
    bits_per_channel: u32,
) -> AudioStreamBasicDescription {
    AudioStreamBasicDescription {
        sample_rate: 48_000.0,
        format_id,
        format_flags,
        bytes_per_packet: 2 * bits_per_channel / 8,
        frames_per_packet: 1,
        bytes_per_frame: 2 * bits_per_channel / 8,
        channels_per_frame: 2,
        bits_per_channel,
        reserved: 0,
    }
}

/// The packed little-endian 32-bit float description `buffer_list_peak` decodes is accepted.
#[test]
fn verify_tap_format_accepts_packed_little_endian_float32() {
    let format = description(
        K_AUDIO_FORMAT_LINEAR_PCM,
        K_AUDIO_FORMAT_FLAG_IS_FLOAT | K_AUDIO_FORMAT_FLAG_IS_PACKED,
        32,
    );
    assert_eq!(verify_tap_format(&format), Ok(()));
}

/// A non-interleaved layout (one mono buffer per channel) is accepted: the peak is a maximum over
/// every buffer's samples whatever the channel layout.
#[test]
fn verify_tap_format_accepts_non_interleaved_float32() {
    let format = description(
        K_AUDIO_FORMAT_LINEAR_PCM,
        K_AUDIO_FORMAT_FLAG_IS_FLOAT
            | K_AUDIO_FORMAT_FLAG_IS_PACKED
            | K_AUDIO_FORMAT_FLAG_IS_NON_INTERLEAVED,
        32,
    );
    assert_eq!(verify_tap_format(&format), Ok(()));
}

/// 16-bit signed integer PCM is refused, and the message names the observed flags and width.
#[test]
fn verify_tap_format_rejects_sixteen_bit_integer_pcm() {
    let format = description(
        K_AUDIO_FORMAT_LINEAR_PCM,
        K_AUDIO_FORMAT_FLAG_IS_SIGNED_INTEGER | K_AUDIO_FORMAT_FLAG_IS_PACKED,
        16,
    );
    let refusal = verify_tap_format(&format).expect_err("16-bit integer PCM must be refused");
    assert!(refusal.contains("flags 0xc"), "{refusal}");
    assert!(refusal.contains("16 bits"), "{refusal}");
}

/// A format id other than linear PCM is refused even when its flags and width look like float32.
#[test]
fn verify_tap_format_rejects_a_non_linear_pcm_format_id() {
    let aac = u32::from_be_bytes(*b"aac ");
    let format = description(
        aac,
        K_AUDIO_FORMAT_FLAG_IS_FLOAT | K_AUDIO_FORMAT_FLAG_IS_PACKED,
        32,
    );
    assert!(verify_tap_format(&format).is_err());
}

/// Big-endian float32 is refused: `buffer_list_peak` decodes little-endian.
#[test]
fn verify_tap_format_rejects_big_endian_float32() {
    let format = description(
        K_AUDIO_FORMAT_LINEAR_PCM,
        K_AUDIO_FORMAT_FLAG_IS_FLOAT
            | K_AUDIO_FORMAT_FLAG_IS_PACKED
            | K_AUDIO_FORMAT_FLAG_IS_BIG_ENDIAN,
        32,
    );
    assert!(verify_tap_format(&format).is_err());
}

/// Float PCM of another width is refused.
#[test]
fn verify_tap_format_rejects_float64() {
    let float64 = description(
        K_AUDIO_FORMAT_LINEAR_PCM,
        K_AUDIO_FORMAT_FLAG_IS_FLOAT | K_AUDIO_FORMAT_FLAG_IS_PACKED,
        64,
    );
    assert!(verify_tap_format(&float64).is_err());
}

/// The packed flag may be clear when the frame layout implies it, as the header's description of
/// `kAudioFormatFlagIsPacked` states: stereo interleaved 32-bit samples in 8-byte frames, or
/// non-interleaved ones in 4-byte frames.
#[test]
fn verify_tap_format_accepts_float32_whose_frame_layout_implies_packed() {
    let interleaved = description(K_AUDIO_FORMAT_LINEAR_PCM, K_AUDIO_FORMAT_FLAG_IS_FLOAT, 32);
    assert_eq!(interleaved.bytes_per_frame, 8);
    assert_eq!(verify_tap_format(&interleaved), Ok(()));
    let non_interleaved = AudioStreamBasicDescription {
        format_flags: K_AUDIO_FORMAT_FLAG_IS_FLOAT | K_AUDIO_FORMAT_FLAG_IS_NON_INTERLEAVED,
        bytes_per_frame: 4,
        ..interleaved
    };
    assert_eq!(verify_tap_format(&non_interleaved), Ok(()));
}

/// Float32 samples held in a wider container (16-byte stereo frames) are unpacked and refused.
#[test]
fn verify_tap_format_rejects_float32_in_a_wider_container() {
    let wide = AudioStreamBasicDescription {
        bytes_per_frame: 16,
        ..description(K_AUDIO_FORMAT_LINEAR_PCM, K_AUDIO_FORMAT_FLAG_IS_FLOAT, 32)
    };
    assert!(verify_tap_format(&wide).is_err());
}
