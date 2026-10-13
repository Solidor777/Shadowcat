//! The macOS backend uses Core Audio's process tap API, available from macOS 14.2.
//!
//! The `coreaudio-sys` bindings do not cover the process tap entry points, so this file declares
//! the small entry-point surface it needs itself in `extern "C"` blocks. It uses `core-foundation`
//! for `CFString`/`CFDictionary`/`CFArray`/`CFNumber` handling.
//!
//! The first `extern "C"` block carries the framework link directive
//! `#[link(name = "CoreAudio", kind = "framework")]`, which links only `CoreAudio`. The other
//! symbols declared in that block (`proc_pidpath` and the Objective-C runtime functions
//! `objc_getClass`, `sel_registerName`, `class_respondsToSelector` and `objc_msgSend`) resolve
//! through the system libraries every macOS process links.
//!
//! On macOS < 14.2 (detected by `macos_at_least_14_2`'s Darwin-kernel version probe),
//! `MacosMonitor::new` returns `MonitorError::Unsupported("macOS 14.2 or newer")`, and the hello
//! frame says so verbatim.
//!
//! The first run requires the user to grant the "System Audio Recording" permission (the OS's own
//! prompt).
//!
//! The signatures of `AudioObjectGetPropertyData`, `AudioObjectGetPropertyDataSize`,
//! `AudioHardwareCreateProcessTap`, `AudioHardwareDestroyProcessTap`,
//! `AudioHardwareCreateAggregateDevice`, `AudioHardwareDestroyAggregateDevice`,
//! `AudioDeviceCreateIOProcID`, `AudioDeviceStart`, `AudioDeviceStop` and
//! `AudioDeviceDestroyIOProcID` are declared by hand in the CoreAudio `extern "C"` block.
//! `CATapDescription` is an Objective-C class with no C-only constructor, so
//! `build_stereo_mixdown_tap_description` builds it through `objc_msgSend` rather than an
//! `objc`/`objc2` crate (neither is a workspace dependency). Before it sends `alloc`, it checks
//! with `class_respondsToSelector` that the class implements `initStereoMixdownOfProcesses:` and
//! `setUUID:`, and `new_ns_uuid` checks `initWithUUIDString:` the same way, so an absent selector
//! fails closed. If a signature changes, the dedicated-thread, process-object-list and
//! aggregate-device-with-tap shape must be preserved.

use std::collections::HashMap;
use std::ffi::{c_void, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use core_foundation::array::CFArray;
use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};

use super::{reduce_to_basename, MonitorError, SessionLevel, SessionMonitor};

/// Opaque Core Audio object id (`AudioObjectID`, a `u32` per the framework's own typedef).
type AudioObjectId = u32;

/// `kAudioHardwarePropertyProcessObjectList`'s numeric selector, the four-char code `'prs#'`.
const K_AUDIO_HARDWARE_PROPERTY_PROCESS_OBJECT_LIST: u32 = 0x7072_7323; // 'prs#'
/// `kAudioProcessPropertyPID`'s numeric selector, the four-char code `'ppid'`. The object id
/// Core Audio enumerates is its own opaque `AudioObjectID`, never a POSIX pid, so the pid is read
/// back through this property before `proc_pidpath` can resolve it.
const K_AUDIO_PROCESS_PROPERTY_PID: u32 = 0x7070_6964; // 'ppid'
/// `kAudioHardwarePropertyDefaultSystemOutputDevice`'s numeric selector, the four-char code
/// `'sOut'`: the `AudioObjectID` of the output device the system uses for system sounds.
const K_AUDIO_HARDWARE_PROPERTY_DEFAULT_SYSTEM_OUTPUT_DEVICE: u32 = 0x734f_7574; // 'sOut'
/// `kAudioDevicePropertyDeviceUID`'s numeric selector, the four-char code `'uid '` (trailing
/// space): the device's persistent UID as a `CFString` the caller owns.
const K_AUDIO_DEVICE_PROPERTY_DEVICE_UID: u32 = 0x7569_6420; // 'uid '
/// The global audio-hardware object id every `AudioObjectGetPropertyData` call against a
/// hardware-scoped selector targets.
const K_AUDIO_OBJECT_SYSTEM_OBJECT: AudioObjectId = 1;

/// A Core Audio property address: which property, on which scope/element.
#[repr(C)]
#[derive(Clone, Copy)]
struct AudioObjectPropertyAddress {
    /// The property's numeric selector (e.g. `K_AUDIO_HARDWARE_PROPERTY_PROCESS_OBJECT_LIST`).
    selector: u32,
    /// The property's scope (this file only ever uses the global scope).
    scope: u32,
    /// The property's element (this file only ever uses the main element).
    element: u32,
}

/// Core Audio's global property scope selector ('glob').
const K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL: u32 = 0x676c_6f62;
/// Core Audio's main (non-channel-specific) property element.
const K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN: u32 = 0;

/// `kAudioAggregateDeviceUIDKey`, `kAudioSubDeviceUIDKey` and `kAudioSubTapUIDKey`: all three
/// Core Audio constants spell the string `"uid"`.
const KEY_UID: &str = "uid";
/// `kAudioAggregateDeviceNameKey`: the aggregate device's human-readable name.
const KEY_NAME: &str = "name";
/// `kAudioAggregateDeviceMainSubDeviceKey`: the UID of the sub-device that is the aggregate's
/// time source.
const KEY_MAIN_SUB_DEVICE: &str = "master";
/// `kAudioAggregateDeviceIsPrivateKey`: `1` keeps the aggregate private to the creating process.
const KEY_PRIVATE: &str = "private";
/// `kAudioAggregateDeviceIsStackedKey`: `0` feeds every sub-device output stream the same data.
const KEY_STACKED: &str = "stacked";
/// `kAudioAggregateDeviceTapAutoStartKey`: non-zero makes `AudioDeviceStart` wait for the first
/// tapped audio; the Core Audio header requires `KEY_PRIVATE` to accompany it.
const KEY_TAP_AUTO_START: &str = "tapautostart";
/// `kAudioAggregateDeviceSubDeviceListKey`: the array of sub-device description dictionaries.
const KEY_SUB_DEVICE_LIST: &str = "subdevices";
/// `kAudioAggregateDeviceTapListKey`: the array of sub-tap description dictionaries.
const KEY_TAP_LIST: &str = "taps";
/// `kAudioSubTapDriftCompensationKey`: non-zero enables drift compensation for the sub-tap.
const KEY_DRIFT_COMPENSATION: &str = "drift";

#[link(name = "CoreAudio", kind = "framework")]
extern "C" {
    /// Reads a Core Audio object property's data into `out_data`, `out_data_size` in/out.
    fn AudioObjectGetPropertyData(
        object_id: AudioObjectId,
        address: *const AudioObjectPropertyAddress,
        qualifier_data_size: u32,
        qualifier_data: *const c_void,
        out_data_size: *mut u32,
        out_data: *mut c_void,
    ) -> i32;
    /// Reads a Core Audio object property's data SIZE (a required precursor call so the
    /// caller can allocate the right buffer for a variable-length property like a process
    /// list).
    fn AudioObjectGetPropertyDataSize(
        object_id: AudioObjectId,
        address: *const AudioObjectPropertyAddress,
        qualifier_data_size: u32,
        qualifier_data: *const c_void,
        out_data_size: *mut u32,
    ) -> i32;
    /// POSIX `proc_pidpath(3)` (libsystem): resolves a pid to its executable's full path;
    /// used here to name each tapped process before `reduce_to_basename` strips the path down.
    fn proc_pidpath(pid: i32, buffer: *mut u8, buffersize: u32) -> i32;

    /// Creates a process tap from a `CATapDescription` (an Objective-C object, hence the
    /// `*mut c_void` receiver — see `build_stereo_mixdown_tap_description`), publishing the new
    /// tap's own `AudioObjectID` into `out_tap_id`.
    fn AudioHardwareCreateProcessTap(
        description: *mut c_void,
        out_tap_id: *mut AudioObjectId,
    ) -> i32;
    /// Releases the Core Audio resources a tap id (`AudioHardwareCreateProcessTap`'s result)
    /// holds. Idempotent teardown counterpart of that call.
    fn AudioHardwareDestroyProcessTap(tap_id: AudioObjectId) -> i32;
    /// Creates a private aggregate device from a `CFDictionaryRef` description (this file
    /// always includes exactly one tap in `kAudioAggregateDeviceTapListKey`), publishing its
    /// `AudioObjectID` into `out_device_id`.
    fn AudioHardwareCreateAggregateDevice(
        description: *const c_void,
        out_device_id: *mut AudioObjectId,
    ) -> i32;
    /// Releases the Core Audio resources an aggregate device id
    /// (`AudioHardwareCreateAggregateDevice`'s result) holds. Teardown counterpart of that
    /// call.
    fn AudioHardwareDestroyAggregateDevice(device_id: AudioObjectId) -> i32;
    /// Registers `proc_` as the aggregate device's IO callback, publishing an opaque proc id
    /// into `out_proc_id` (needed by `AudioDeviceStart`/`AudioDeviceDestroyIOProcID`).
    fn AudioDeviceCreateIOProcID(
        device_id: AudioObjectId,
        proc_: AudioDeviceIoProc,
        client_data: *mut c_void,
        out_proc_id: *mut *mut c_void,
    ) -> i32;
    /// Starts IO callbacks flowing on `device_id` through `proc_id` (an
    /// `AudioDeviceCreateIOProcID` result).
    fn AudioDeviceStart(device_id: AudioObjectId, proc_id: *mut c_void) -> i32;
    /// Stops IO callbacks on `device_id`/`proc_id`. Teardown counterpart of `AudioDeviceStart`.
    fn AudioDeviceStop(device_id: AudioObjectId, proc_id: *mut c_void) -> i32;
    /// Releases an IO proc id (an `AudioDeviceCreateIOProcID` result).
    fn AudioDeviceDestroyIOProcID(device_id: AudioObjectId, proc_id: *mut c_void) -> i32;

    /// The Objective-C runtime's class lookup, used to resolve `CATapDescription` (no C-only
    /// constructor exists for a process tap description) and `NSUUID`.
    fn objc_getClass(name: *const i8) -> *mut c_void;
    /// The Objective-C runtime's selector registration.
    fn sel_registerName(name: *const i8) -> *mut c_void;
    /// Reports whether `cls` implements `sel`. `build_stereo_mixdown_tap_description` and
    /// `new_ns_uuid` check each selector they send before the `objc_msgSend` that sends it, so an
    /// absent selector returns `None` instead of raising `doesNotRecognizeSelector:`.
    fn class_respondsToSelector(cls: *mut c_void, sel: *mut c_void) -> i8;
    /// `objc_msgSend`, the Objective-C runtime's message dispatch. Declared once at its minimal
    /// (0-argument-selector) arity: Objective-C calls this same C symbol with a different
    /// effective signature per invocation depending on the target method's real argument count,
    /// and a second `extern "C"` declaration of the same link name with a different signature
    /// trips rustc's `clashing_extern_declarations` lint (warn by default, an error under
    /// `cargo clippy -D warnings`). A wider-arity call site instead takes this declaration's
    /// function pointer and `transmute`s it to the `unsafe extern "C" fn(...)` type that call
    /// needs (`objc_msg_send_1`), which avoids the `objc`/`objc2` crates.
    fn objc_msgSend(receiver: *mut c_void, selector: *mut c_void) -> *mut c_void;
}

/// `objc_msgSend` called with a bare selector (no arguments) — the declared arity, so no
/// transmute is needed.
unsafe fn objc_msg_send_0(receiver: *mut c_void, selector: *mut c_void) -> *mut c_void {
    objc_msgSend(receiver, selector)
}

/// `objc_msgSend` called with one object argument (`initStereoMixdownOfProcesses:`, `setUUID:` and
/// `initWithUUIDString:`) — reinterprets `objc_msgSend`'s function pointer at the 3-argument arity
/// those calls need. See `objc_msgSend`'s doc comment for why this file cannot declare that arity
/// as its own `extern` item.
unsafe fn objc_msg_send_1(
    receiver: *mut c_void,
    selector: *mut c_void,
    arg1: *mut c_void,
) -> *mut c_void {
    let send: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut c_void =
        std::mem::transmute(
            objc_msgSend as unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void,
        );
    send(receiver, selector, arg1)
}

/// A Core Audio device IO callback (`AudioDeviceIOProc`): fires once per IO cycle carrying the
/// aggregate device's input buffer list. Only `in_input_data`/`client_data` are read here.
type AudioDeviceIoProc = extern "C" fn(
    device_id: AudioObjectId,
    now: *const c_void,
    in_input_data: *const AudioBufferList,
    in_input_time: *const c_void,
    out_output_data: *mut c_void,
    in_output_time: *const c_void,
    client_data: *mut c_void,
) -> i32;

/// Mirrors the Core Audio `AudioBufferList` struct: a single-element view is enough here since
/// this file always taps a single stereo-or-mono process mixdown.
#[repr(C)]
struct AudioBufferList {
    /// Number of `AudioBuffer` entries in `buffers`.
    number_buffers: u32,
    /// The first (and, for this file's use, only) buffer.
    buffers: [AudioBuffer; 1],
}

/// Mirrors the Core Audio `AudioBuffer` struct: one channel-interleaved audio buffer.
#[repr(C)]
struct AudioBuffer {
    /// Channel count in `data`.
    number_channels: u32,
    /// `data`'s length in bytes.
    data_byte_size: u32,
    /// Interleaved `Float32` sample data.
    data: *mut c_void,
}

/// Shared per-pid running-peak map, published into `TAP_PEAKS` once the dedicated Core Audio
/// thread starts.
type TapPeaks = Arc<Mutex<HashMap<i32, f32>>>;

/// Per-process-tap running peak, shared with `enumerate_processes` and reset on each read —
/// mirrors the Linux backend's window-reset shape (`LinuxMonitor`'s `NodeState.peak`).
static TAP_PEAKS: Mutex<Option<TapPeaks>> = Mutex::new(None);

/// The IO callback registered on every tapped aggregate device: computes this window's
/// max-abs-sample over the buffer's interleaved `Float32` data and folds it into `TAP_PEAKS`
/// under the tap id `client_data` carries.
extern "C" fn tap_io_proc(
    _device_id: AudioObjectId,
    _now: *const c_void,
    in_input_data: *const AudioBufferList,
    _in_input_time: *const c_void,
    _out_output_data: *mut c_void,
    _in_output_time: *const c_void,
    client_data: *mut c_void,
) -> i32 {
    if in_input_data.is_null() {
        return 0;
    }
    // SAFETY: Core Audio guarantees a valid `AudioBufferList` for the lifetime of this call.
    let list = unsafe { &*in_input_data };
    // `number_buffers` counts the valid entries of the variable-length `buffers` array, so
    // `buffers[0]` is readable only when it is at least 1.
    if list.number_buffers == 0 {
        return 0;
    }
    let buffer = &list.buffers[0];
    if buffer.data.is_null() || buffer.data_byte_size == 0 {
        return 0;
    }
    // SAFETY: `data`/`data_byte_size` together describe a valid, live `Float32` buffer for the
    // duration of this callback, per the `AudioDeviceIOProc` contract.
    let samples = unsafe {
        std::slice::from_raw_parts(buffer.data as *const u8, buffer.data_byte_size as usize)
    };
    let peak = samples
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b).abs())
        .fold(0f32, f32::max);
    let pid = client_data as usize as i32;
    if let Ok(guard) = TAP_PEAKS.lock() {
        if let Some(peaks) = guard.as_ref() {
            if let Ok(mut peaks) = peaks.lock() {
                let entry = peaks.entry(pid).or_insert(0.0);
                *entry = entry.max(peak);
            }
        }
    }
    0
}

/// macOS `SessionMonitor`: reads the latest state the dedicated Core Audio thread publishes.
///
/// # Examples
///
/// ```
/// use std::sync::atomic::AtomicBool;
/// use std::sync::Arc;
/// use shadowcat::audio_monitor::macos::MacosMonitor;
///
/// // Below macOS 14.2 construction reports `Unsupported`; at or above it never panics.
/// let _outcome = MacosMonitor::new(Arc::new(AtomicBool::new(false)));
/// ```
pub struct MacosMonitor {
    /// Shared latest reading, updated by the background thread every poll interval.
    latest: Arc<Mutex<Result<Vec<SessionLevel>, MonitorError>>>,
}

impl MacosMonitor {
    /// Checks the macOS version, then spawns the dedicated Core Audio thread. Returns
    /// `MonitorError::Unsupported("macOS 14.2 or newer")` below that version. `shutdown` is
    /// polled by the dedicated thread's loop; when the caller sets it, the thread tears down
    /// every live tap (`teardown_tap`) and exits rather than leaking Core Audio resources for
    /// the rest of the process's lifetime.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::atomic::AtomicBool;
    /// use std::sync::Arc;
    /// use shadowcat::audio_monitor::macos::MacosMonitor;
    /// use shadowcat::audio_monitor::SessionMonitor;
    ///
    /// if let Ok(mut monitor) = MacosMonitor::new(Arc::new(AtomicBool::new(false))) {
    ///     let _levels = monitor.poll();
    /// }
    /// ```
    pub fn new(shutdown: Arc<AtomicBool>) -> Result<Self, MonitorError> {
        if !macos_at_least_14_2() {
            return Err(MonitorError::Unsupported("macOS 14.2 or newer".to_string()));
        }
        let latest: Arc<Mutex<Result<Vec<SessionLevel>, MonitorError>>> =
            Arc::new(Mutex::new(Ok(Vec::new())));
        let thread_latest = latest.clone();
        std::thread::spawn(move || run_process_tap_loop(thread_latest, shutdown));
        Ok(Self { latest })
    }
}

impl SessionMonitor for MacosMonitor {
    fn poll(&mut self) -> Result<Vec<SessionLevel>, MonitorError> {
        self.latest
            .lock()
            .map_err(|_| MonitorError::Backend("Core Audio state lock poisoned".to_string()))?
            .clone()
    }
}

/// Reads the OS version through POSIX `uname(2)`'s kernel release string, parsed as a Darwin
/// version: Darwin 23.2 corresponds to macOS 14.2 (Darwin 24 to macOS 15, and so on). Fails
/// closed — any unreadable or unparseable version reports unsupported rather than guessing.
fn macos_at_least_14_2() -> bool {
    let mut uts: Utsname = unsafe { std::mem::zeroed() };
    if unsafe { uname(&mut uts) } != 0 {
        return false;
    }
    let release = unsafe { std::ffi::CStr::from_ptr(uts.release.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    let mut parts = release.split('.');
    let major: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    major > 23 || (major == 23 && minor >= 2)
}

/// Mirrors the POSIX `struct utsname` that `uname` fills — only `release` is read.
#[repr(C)]
struct Utsname {
    /// Operating system name.
    sysname: [i8; 256],
    /// Network node hostname.
    nodename: [i8; 256],
    /// OS release (Darwin kernel version, e.g. `"23.2.0"`) — the field this module reads.
    release: [i8; 256],
    /// OS version string.
    version: [i8; 256],
    /// Hardware identifier.
    machine: [i8; 256],
}
extern "C" {
    /// POSIX `uname(2)`: fills `buf` with the running kernel's identification.
    fn uname(buf: *mut Utsname) -> i32;
}

/// One Core Audio process tap's live resources: the tap object, the private aggregate device
/// wrapping it, and the registered IO proc id — released together by `teardown_tap`.
struct TapHandle {
    /// The Core Audio process object id the tap was created from. A tap whose process now
    /// enumerates under a different object id is stale (`departed_pids`).
    process_object_id: AudioObjectId,
    /// The `AudioHardwareCreateProcessTap` result.
    tap_id: AudioObjectId,
    /// The `AudioHardwareCreateAggregateDevice` result — the actual object `AudioDeviceStart`
    /// pulls samples from (a bare tap id is not itself a startable IO device).
    aggregate_id: AudioObjectId,
    /// The `AudioDeviceCreateIOProcID` result.
    proc_id: *mut c_void,
}

/// Releases every resource a `TapHandle` holds, in creation order reversed: stop IO, destroy
/// the proc id, destroy the aggregate device, destroy the tap.
fn teardown_tap(handle: TapHandle) {
    // SAFETY: each id/proc_id was returned by its matching `AudioHardware*`/`AudioDevice*`
    // creation call above and is torn down at most once (removed from the caller's map first).
    unsafe {
        AudioDeviceStop(handle.aggregate_id, handle.proc_id);
        AudioDeviceDestroyIOProcID(handle.aggregate_id, handle.proc_id);
        AudioHardwareDestroyAggregateDevice(handle.aggregate_id);
        AudioHardwareDestroyProcessTap(handle.tap_id);
    }
}

/// Delay before a process whose tap creation failed is tried again.
const TAP_RETRY_DELAY: Duration = Duration::from_secs(5);

/// Tap-creation failures per (pid, process object id), each with the earliest instant the next
/// attempt may run. Without it a process Core Audio refuses to tap would repeat the whole
/// create/destroy sequence every 100 ms poll cycle.
#[derive(Default)]
struct TapRetryBackoff {
    /// Earliest next attempt per (pid, process object id).
    next_attempt: HashMap<(i32, AudioObjectId), Instant>,
}

impl TapRetryBackoff {
    /// Whether `key` has no recorded failure or its retry delay has elapsed at `now`.
    fn should_attempt(&self, key: (i32, AudioObjectId), now: Instant) -> bool {
        self.next_attempt.get(&key).is_none_or(|next| now >= *next)
    }

    /// Records a failed attempt for `key`, delaying the next one by `TAP_RETRY_DELAY`. Returns
    /// `true` when `key` had no earlier recorded failure, so the caller logs the first failure
    /// at a higher level than the repeats.
    fn record_failure(&mut self, key: (i32, AudioObjectId), now: Instant) -> bool {
        self.next_attempt
            .insert(key, now + TAP_RETRY_DELAY)
            .is_none()
    }

    /// Drops the failure record of `key` after a successful attempt.
    fn record_success(&mut self, key: (i32, AudioObjectId)) {
        self.next_attempt.remove(&key);
    }

    /// Drops every record whose (pid, process object id) pair is absent from `live`.
    fn retain_live(&mut self, live: &HashMap<i32, AudioObjectId>) {
        self.next_attempt
            .retain(|(pid, object_id), _| live.get(pid) == Some(object_id));
    }
}

/// Runs on its own dedicated Core Audio thread for the process's lifetime: enumerates the
/// system's process object list, creates a process tap + aggregate device per tapped process
/// (tearing down a tap whose process has exited or now enumerates under a different process
/// object id via `teardown_tap`, never merely dropping the `TapHandle` — it holds no `Drop` impl
/// of its own, so a dropped-without-teardown handle would leak its
/// `AudioDeviceIOProcID`/aggregate device/process tap for the rest of the process's lifetime),
/// and publishes each process's measured peak into `latest` every 100 ms. A failed tap creation
/// is logged and retried after `TAP_RETRY_DELAY`. Exits (also tearing down every still-live tap
/// first) once `shutdown` is observed set.
fn run_process_tap_loop(
    latest: Arc<Mutex<Result<Vec<SessionLevel>, MonitorError>>>,
    shutdown: Arc<AtomicBool>,
) {
    let peaks: Arc<Mutex<HashMap<i32, f32>>> = Arc::new(Mutex::new(HashMap::new()));
    *TAP_PEAKS.lock().expect("tap peak map lock poisoned") = Some(peaks.clone());
    let mut taps: HashMap<i32, TapHandle> = HashMap::new();
    let mut backoff = TapRetryBackoff::default();
    let mut output_uid_failure_logged = false;

    while !shutdown.load(Ordering::Relaxed) {
        let (result, processes) = enumerate_processes(&peaks);
        let live: HashMap<i32, AudioObjectId> = processes.into_iter().collect();
        teardown_departed_taps(&mut taps, &live);
        backoff.retain_live(&live);
        let now = Instant::now();
        let candidates: Vec<(i32, AudioObjectId)> = untapped_processes(
            &live,
            taps.iter()
                .map(|(pid, handle)| (*pid, handle.process_object_id)),
        )
        .into_iter()
        .filter(|key| backoff.should_attempt(*key, now))
        .collect();
        if !candidates.is_empty() {
            match read_default_system_output_uid() {
                Ok(output_uid) => {
                    output_uid_failure_logged = false;
                    for (pid, process_object_id) in candidates {
                        match create_tap_for_pid(pid, process_object_id, &output_uid) {
                            Ok(handle) => {
                                backoff.record_success((pid, process_object_id));
                                taps.insert(pid, handle);
                            }
                            Err(failure) => {
                                if backoff.record_failure((pid, process_object_id), now) {
                                    tracing::warn!(pid, %failure, "process tap creation failed");
                                } else {
                                    tracing::debug!(pid, %failure, "process tap creation failed");
                                }
                            }
                        }
                    }
                }
                Err(failure) => {
                    if !output_uid_failure_logged {
                        tracing::warn!(%failure, "default system output device unreadable");
                    }
                    output_uid_failure_logged = true;
                }
            }
        }
        peaks
            .lock()
            .expect("tap peak map lock poisoned")
            .retain(|pid, _| live.contains_key(pid));
        *latest.lock().expect("Core Audio state lock poisoned") = result;
        std::thread::sleep(Duration::from_millis(100));
    }
    // Shutdown requested: every remaining tap is "departed" relative to an empty live map, so
    // the same teardown path used every poll cycle also drains the thread's exit.
    teardown_departed_taps(&mut taps, &HashMap::new());
}

/// Removes and tears down (`teardown_tap`) every tap that `departed_pids` reports against `live`,
/// in place. The sole path that ever discards a `TapHandle`: `run_process_tap_loop` calls it both
/// every poll cycle (departed processes) and once more on exit (`live` empty, so every tap
/// departs). The diff itself lives in `departed_pids` so it can be exercised without invoking real
/// Core Audio teardown calls.
fn teardown_departed_taps(taps: &mut HashMap<i32, TapHandle>, live: &HashMap<i32, AudioObjectId>) {
    let present: Vec<(i32, AudioObjectId)> = taps
        .iter()
        .map(|(pid, handle)| (*pid, handle.process_object_id))
        .collect();
    for pid in departed_pids(present.into_iter(), live) {
        if let Some(handle) = taps.remove(&pid) {
            teardown_tap(handle);
        }
    }
}

/// Pure diff: which of the `present` (pid, process object id) taps no longer match `live`, either
/// because the pid is absent or because it now enumerates under a different process object id (a
/// tap built from the old id points at a different object than the live process). Split out from
/// `teardown_departed_taps` so the "which taps get torn down" decision is unit-testable without a
/// real Core Audio device/aggregate/tap to release.
fn departed_pids(
    present: impl Iterator<Item = (i32, AudioObjectId)>,
    live: &HashMap<i32, AudioObjectId>,
) -> Vec<i32> {
    present
        .filter(|(pid, object_id)| live.get(pid) != Some(object_id))
        .map(|(pid, _)| pid)
        .collect()
}

/// Pure selection: the live processes, as (pid, process object id) pairs ordered by pid, that no
/// `tapped` pair matches. Split out from `run_process_tap_loop` so the pid-to-object-id pairing
/// handed to `create_tap_for_pid` is unit-testable without Core Audio.
fn untapped_processes(
    live: &HashMap<i32, AudioObjectId>,
    tapped: impl Iterator<Item = (i32, AudioObjectId)>,
) -> Vec<(i32, AudioObjectId)> {
    let tapped: std::collections::HashSet<(i32, AudioObjectId)> = tapped.collect();
    let mut pairs: Vec<(i32, AudioObjectId)> = live
        .iter()
        .map(|(pid, object_id)| (*pid, *object_id))
        .filter(|pair| !tapped.contains(pair))
        .collect();
    pairs.sort_unstable();
    pairs
}

/// Reads a Core Audio process object's `kAudioProcessPropertyPID` property. The object id Core
/// Audio enumerates is its own opaque id, never a pid, so this call is required before
/// `proc_pidpath` can resolve anything real.
fn read_process_pid(process_object_id: AudioObjectId) -> Option<i32> {
    let address = AudioObjectPropertyAddress {
        selector: K_AUDIO_PROCESS_PROPERTY_PID,
        scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
        element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    };
    let mut pid: i32 = 0;
    let mut size: u32 = std::mem::size_of::<i32>() as u32;
    // SAFETY: `pid` is a valid `i32` out-param sized exactly to `size`.
    let status = unsafe {
        AudioObjectGetPropertyData(
            process_object_id,
            &address,
            0,
            std::ptr::null(),
            &mut size,
            &mut pid as *mut i32 as *mut c_void,
        )
    };
    (status == 0).then_some(pid)
}

/// Sends an Objective-C `release` message to `instance` — manual (ARC-free) reference counting
/// for the `alloc`/`init`-owned `CATapDescription` and `NSUUID` instances this file builds.
/// `CATapDescription` is a plain Objective-C class with no documented CoreFoundation toll-free
/// bridging, so releasing it via `CFRelease` (which relies on retain-count layout compatibility
/// that is undocumented for an arbitrary non-bridged class) is avoided in favor of the real
/// message send, which this file applies to both classes.
///
/// # Safety
/// `instance` must be a live, +1-owned Objective-C object pointer (or null, which this function
/// treats as a no-op).
unsafe fn release_objc_instance(instance: *mut c_void) {
    if instance.is_null() {
        return;
    }
    if let Ok(release_sel_name) = CString::new("release") {
        objc_msg_send_0(instance, sel_registerName(release_sel_name.as_ptr()));
    }
}

/// Returns the Objective-C selector registered under `name`, or `None` if `name` holds a NUL.
fn registered_selector(name: &str) -> Option<*mut c_void> {
    let name = CString::new(name).ok()?;
    // SAFETY: `name` is NUL-terminated and alive across the call.
    Some(unsafe { sel_registerName(name.as_ptr()) })
}

/// Returns a fresh upper-case hyphenated UUID string, the spelling `NSUUID`'s `UUIDString`
/// produces. It is the tap description's `UUID` and the aggregate sub-tap's `uid`.
fn new_tap_uuid() -> String {
    uuid::Uuid::new_v4().to_string().to_uppercase()
}

/// Builds an `NSUUID` holding `uuid_string` through `initWithUUIDString:`. Returns the +1-owned
/// instance, to be released with `release_objc_instance`, or `None` when the class or selector is
/// absent or the string does not parse as a UUID.
fn new_ns_uuid(uuid_string: &str) -> Option<*mut c_void> {
    let class_name = CString::new("NSUUID").ok()?;
    let init_sel = registered_selector("initWithUUIDString:")?;
    let alloc_sel = registered_selector("alloc")?;
    let uuid_string = CFString::new(uuid_string);
    // SAFETY: `class` is null-checked before use as a receiver, `class_name` is NUL-terminated and
    // alive across the lookup, and `uuid_string` stays alive until after the send.
    // `class_respondsToSelector` confirms `initWithUUIDString:` exists; `objc_msg_send_1`
    // reinterprets `objc_msgSend` as (receiver, selector, one object argument) returning an
    // object, the shape of that initializer.
    unsafe {
        let class = objc_getClass(class_name.as_ptr());
        if class.is_null() || class_respondsToSelector(class, init_sel) == 0 {
            return None;
        }
        let instance = objc_msg_send_0(class, alloc_sel);
        if instance.is_null() {
            return None;
        }
        let initialized = objc_msg_send_1(
            instance,
            init_sel,
            uuid_string.as_CFTypeRef() as *mut c_void,
        );
        (!initialized.is_null()).then_some(initialized)
    }
}

/// Builds a `CATapDescription` mixing down the audio of the process whose Core Audio process
/// object id is `process_object_id`, with `tap_uuid` as the description's `UUID`, via
/// hand-transcribed `objc_msgSend` calls (the module doc states why no Objective-C crate is
/// used). `initStereoMixdownOfProcesses:` takes process `AudioObjectID`s, not pids. Every
/// selector it sends is checked before `alloc`, and the `NSUUID` is built before `alloc`, so a
/// failure on either leaves no description to release. Returns the owned Objective-C instance
/// pointer, to be released with `release_objc_instance`.
fn build_stereo_mixdown_tap_description(
    process_object_id: AudioObjectId,
    tap_uuid: &str,
) -> Option<*mut c_void> {
    let class_name = CString::new("CATapDescription").ok()?;
    let alloc_sel = registered_selector("alloc")?;
    let init_sel = registered_selector("initStereoMixdownOfProcesses:")?;
    let set_uuid_sel = registered_selector("setUUID:")?;
    // SAFETY: `class` is null-checked before it is used as a receiver, and `class_name` is
    // NUL-terminated and alive across the lookup. `ids`, the `CFArray` passed to
    // `initStereoMixdownOfProcesses:`, stays alive until after that send. `objc_msg_send_1`
    // reinterprets `objc_msgSend` as (receiver, selector, one object argument) returning an
    // object; that is the shape this code assumes for the initializer and for `setUUID:` (whose
    // return value is ignored), and `class_respondsToSelector` confirms each selector exists,
    // not its signature.
    unsafe {
        let class = objc_getClass(class_name.as_ptr());
        if class.is_null()
            || class_respondsToSelector(class, init_sel) == 0
            || class_respondsToSelector(class, set_uuid_sel) == 0
        {
            // An absent class or selector fails closed rather than risk
            // `doesNotRecognizeSelector:` aborting the process.
            return None;
        }
        let ns_uuid = new_ns_uuid(tap_uuid)?;
        let instance = objc_msg_send_0(class, alloc_sel);
        if instance.is_null() {
            release_objc_instance(ns_uuid);
            return None;
        }
        let ids = CFArray::from_CFTypes(&[CFNumber::from(i64::from(process_object_id))]);
        let described = objc_msg_send_1(instance, init_sel, ids.as_CFTypeRef() as *mut c_void);
        if described.is_null() {
            release_objc_instance(ns_uuid);
            return None;
        }
        // `UUID` is a `copy` property: the description keeps its own copy, so `ns_uuid` is
        // released right after the send.
        objc_msg_send_1(described, set_uuid_sel, ns_uuid);
        release_objc_instance(ns_uuid);
        Some(described)
    }
}

/// Reads the persistent UID string of the system's default system-output device, the sub-device
/// the aggregate takes its clock from. The error names the property read that failed and its
/// `OSStatus`, or the empty result.
fn read_default_system_output_uid() -> Result<String, String> {
    let device_address = AudioObjectPropertyAddress {
        selector: K_AUDIO_HARDWARE_PROPERTY_DEFAULT_SYSTEM_OUTPUT_DEVICE,
        scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
        element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    };
    let mut device_id: AudioObjectId = 0;
    let mut size = std::mem::size_of::<AudioObjectId>() as u32;
    // SAFETY: `device_id` is a valid `AudioObjectId` out-param sized exactly to `size`.
    let status = unsafe {
        AudioObjectGetPropertyData(
            K_AUDIO_OBJECT_SYSTEM_OBJECT,
            &device_address,
            0,
            std::ptr::null(),
            &mut size,
            &mut device_id as *mut AudioObjectId as *mut c_void,
        )
    };
    if status != 0 {
        return Err(format!(
            "reading the default system output device failed: {status}"
        ));
    }
    if device_id == 0 {
        return Err("the default system output device id is 0".to_string());
    }

    let uid_address = AudioObjectPropertyAddress {
        selector: K_AUDIO_DEVICE_PROPERTY_DEVICE_UID,
        scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
        element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    };
    let mut uid_ref: CFStringRef = std::ptr::null();
    let mut size = std::mem::size_of::<CFStringRef>() as u32;
    // SAFETY: `uid_ref` is a valid pointer-sized out-param sized exactly to `size`.
    let status = unsafe {
        AudioObjectGetPropertyData(
            device_id,
            &uid_address,
            0,
            std::ptr::null(),
            &mut size,
            &mut uid_ref as *mut CFStringRef as *mut c_void,
        )
    };
    if status != 0 {
        return Err(format!(
            "reading the default system output device UID failed: {status}"
        ));
    }
    if uid_ref.is_null() {
        return Err("the default system output device UID is null".to_string());
    }
    // SAFETY: a successful read of `kAudioDevicePropertyDeviceUID` hands the caller one owned
    // reference to the `CFString`, which the create rule adopts.
    Ok(unsafe { CFString::wrap_under_create_rule(uid_ref) }.to_string())
}

/// Builds the `AudioHardwareCreateAggregateDevice` description of a private aggregate device
/// named `shadowcat-tap-{pid}` with the UID `shadowcat-tap-{tap_uuid}`, so two taps never share a
/// UID. Its time source is the sub-device `output_uid`, and its one sub-tap has `uid` `tap_uuid`
/// (the tap description's `UUID`) with drift compensation enabled.
fn aggregate_description(
    pid: i32,
    output_uid: &str,
    tap_uuid: &str,
) -> CFDictionary<CFType, CFType> {
    let aggregate_name = format!("shadowcat-tap-{pid}");
    let aggregate_uid = format!("shadowcat-tap-{tap_uuid}");
    let sub_device = CFDictionary::from_CFType_pairs(&[(
        CFString::new(KEY_UID).as_CFType(),
        CFString::new(output_uid).as_CFType(),
    )]);
    let sub_tap = CFDictionary::from_CFType_pairs(&[
        (
            CFString::new(KEY_UID).as_CFType(),
            CFString::new(tap_uuid).as_CFType(),
        ),
        (
            CFString::new(KEY_DRIFT_COMPENSATION).as_CFType(),
            CFNumber::from(1i32).as_CFType(),
        ),
    ]);
    CFDictionary::from_CFType_pairs(&[
        (
            CFString::new(KEY_NAME).as_CFType(),
            CFString::new(&aggregate_name).as_CFType(),
        ),
        (
            CFString::new(KEY_UID).as_CFType(),
            CFString::new(&aggregate_uid).as_CFType(),
        ),
        (
            CFString::new(KEY_MAIN_SUB_DEVICE).as_CFType(),
            CFString::new(output_uid).as_CFType(),
        ),
        (
            CFString::new(KEY_PRIVATE).as_CFType(),
            CFNumber::from(1i32).as_CFType(),
        ),
        (
            CFString::new(KEY_STACKED).as_CFType(),
            CFNumber::from(0i32).as_CFType(),
        ),
        (
            CFString::new(KEY_TAP_AUTO_START).as_CFType(),
            CFNumber::from(1i32).as_CFType(),
        ),
        (
            CFString::new(KEY_SUB_DEVICE_LIST).as_CFType(),
            CFArray::from_CFTypes(&[sub_device]).as_CFType(),
        ),
        (
            CFString::new(KEY_TAP_LIST).as_CFType(),
            CFArray::from_CFTypes(&[sub_tap]).as_CFType(),
        ),
    ])
}

/// Attempts to create a full tap → aggregate-device → running-IO-proc chain for the process with
/// POSIX pid `pid` and Core Audio process object id `process_object_id`, with `output_uid` (see
/// `read_default_system_output_uid`) as the aggregate's time source. The error names the step that
/// failed and its `OSStatus`. A single process's tap failing never takes down the whole
/// enumeration loop, mirroring the Linux backend's per-node `attach_monitor_stream` failure
/// shape.
fn create_tap_for_pid(
    pid: i32,
    process_object_id: AudioObjectId,
    output_uid: &str,
) -> Result<TapHandle, String> {
    let tap_uuid = new_tap_uuid();
    let description = build_stereo_mixdown_tap_description(process_object_id, &tap_uuid)
        .ok_or_else(|| "building the CATapDescription failed".to_string())?;
    let mut tap_id: AudioObjectId = 0;
    // SAFETY: `description` is a live, owned `CATapDescription*` from the call above;
    // `tap_id` is a valid `AudioObjectID` out-param.
    let status = unsafe { AudioHardwareCreateProcessTap(description, &mut tap_id) };
    // SAFETY: `description` is a live Objective-C object pointer this function owns a +1
    // reference to (an `alloc`/`init` pair); `release_objc_instance` matches that ownership
    // regardless of whether the tap call above succeeded. Uses the real Objective-C message
    // send rather than `CFRelease` — see `release_objc_instance`'s doc comment.
    unsafe { release_objc_instance(description) };
    if status != 0 {
        return Err(format!("AudioHardwareCreateProcessTap failed: {status}"));
    }

    let description_dict = aggregate_description(pid, output_uid, &tap_uuid);

    let mut aggregate_id: AudioObjectId = 0;
    // SAFETY: `description_dict` is a live `CFDictionaryRef` for the duration of this call;
    // `aggregate_id` is a valid `AudioObjectID` out-param.
    let status = unsafe {
        AudioHardwareCreateAggregateDevice(description_dict.as_CFTypeRef(), &mut aggregate_id)
    };
    if status != 0 {
        // SAFETY: `tap_id` was returned by the successful `AudioHardwareCreateProcessTap`
        // call above and has not yet been torn down.
        unsafe {
            AudioHardwareDestroyProcessTap(tap_id);
        }
        return Err(format!(
            "AudioHardwareCreateAggregateDevice failed: {status}"
        ));
    }

    let mut proc_id: *mut c_void = std::ptr::null_mut();
    // SAFETY: `aggregate_id` is the just-created device; `tap_io_proc` matches
    // `AudioDeviceIoProc`'s ABI; `client_data` round-trips `pid` through the callback.
    let status = unsafe {
        AudioDeviceCreateIOProcID(
            aggregate_id,
            tap_io_proc,
            pid as usize as *mut c_void,
            &mut proc_id,
        )
    };
    if status != 0 || proc_id.is_null() {
        // SAFETY: both ids were returned by the successful calls above and not yet torn down.
        unsafe {
            AudioHardwareDestroyAggregateDevice(aggregate_id);
            AudioHardwareDestroyProcessTap(tap_id);
        }
        return Err(format!("AudioDeviceCreateIOProcID failed: {status}"));
    }

    // SAFETY: `aggregate_id`/`proc_id` were returned by the successful calls immediately above.
    let start_status = unsafe { AudioDeviceStart(aggregate_id, proc_id) };
    let handle = TapHandle {
        process_object_id,
        tap_id,
        aggregate_id,
        proc_id,
    };
    if start_status != 0 {
        teardown_tap(handle);
        return Err(format!("AudioDeviceStart failed: {start_status}"));
    }

    Ok(handle)
}

/// The result of one `enumerate_processes` pass: the per-process levels, and every resolved
/// (pid, process object id) pair.
type ProcessScan = (
    Result<Vec<SessionLevel>, MonitorError>,
    Vec<(i32, AudioObjectId)>,
);

/// One enumeration pass: `kAudioHardwarePropertyProcessObjectList` -> real pid per object (via
/// `read_process_pid`) -> `proc_pidpath` reduced to a basename, with `peak` read from `peaks`
/// (populated by each tap's `tap_io_proc` callback) and reset for the next window, mirroring
/// the Linux backend's window-reset shape. Also returns every resolved (pid, process object id)
/// pair, so `run_process_tap_loop` can diff live taps against the current process set and hand
/// each new tap its process object id.
fn enumerate_processes(peaks: &Arc<Mutex<HashMap<i32, f32>>>) -> ProcessScan {
    let address = AudioObjectPropertyAddress {
        selector: K_AUDIO_HARDWARE_PROPERTY_PROCESS_OBJECT_LIST,
        scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
        element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    };
    let mut size: u32 = 0;
    // SAFETY: `AudioObjectGetPropertyDataSize` writes only into `size`; no buffer is passed.
    let status = unsafe {
        AudioObjectGetPropertyDataSize(
            K_AUDIO_OBJECT_SYSTEM_OBJECT,
            &address,
            0,
            std::ptr::null(),
            &mut size,
        )
    };
    if status != 0 {
        return (
            Err(MonitorError::Backend(format!(
                "AudioObjectGetPropertyDataSize failed: {status}"
            ))),
            Vec::new(),
        );
    }
    let count = size as usize / std::mem::size_of::<AudioObjectId>();
    let mut ids = vec![0 as AudioObjectId; count];
    // SAFETY: `ids` is sized exactly to `size` bytes, matching what the prior call reported.
    let status = unsafe {
        AudioObjectGetPropertyData(
            K_AUDIO_OBJECT_SYSTEM_OBJECT,
            &address,
            0,
            std::ptr::null(),
            &mut size,
            ids.as_mut_ptr() as *mut c_void,
        )
    };
    if status != 0 {
        return (
            Err(MonitorError::Backend(format!(
                "AudioObjectGetPropertyData failed: {status}"
            ))),
            Vec::new(),
        );
    }

    let mut levels = Vec::new();
    let mut processes = Vec::new();
    for id in ids {
        let Some(pid) = read_process_pid(id) else {
            continue;
        };
        let mut buf = [0u8; 4096];
        let len = unsafe { proc_pidpath(pid, buf.as_mut_ptr(), buf.len() as u32) };
        if len <= 0 {
            continue;
        }
        let path = String::from_utf8_lossy(&buf[..len as usize]).into_owned();
        let peak = peaks
            .lock()
            .ok()
            .and_then(|mut guard| guard.insert(pid, 0.0))
            .unwrap_or(0.0);
        processes.push((pid, id));
        levels.push(SessionLevel {
            process: reduce_to_basename(&path),
            peak,
        });
    }
    (Ok(levels), processes)
}

#[cfg(test)]
mod tests;
