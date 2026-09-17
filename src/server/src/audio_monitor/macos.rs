//! macOS backend: the Core Audio PROCESS TAP API (`AudioHardwareCreateProcessTap`), added in
//! macOS 14.2 — new enough that `coreaudio-sys` does not wrap it in the resolved version, so
//! this file binds the small entry-point surface it needs directly via `extern "C"`, linked
//! against the `CoreAudio` framework (`#[link(name = "CoreAudio", kind = "framework")]` on the
//! `extern "C"` block declaring these entry points — every symbol this file calls lives in
//! `CoreAudio.framework`; the Objective-C runtime symbols (`objc_getClass`, `sel_registerName`,
//! `class_respondsToSelector`, `objc_msgSend`) and `proc_pidpath` resolve via the platform's
//! always-linked `libSystem`/`libobjc`, needing no framework directive of their own), using
//! `core-foundation` only for `CFString`/`CFDictionary`/`CFArray`/`CFNumber` handling. On macOS
//! < 14.2 (detected by
//! `macos_at_least_14_2`'s Darwin-kernel version probe), `MacosMonitor::new` returns
//! `MonitorError::Unsupported("macOS 14.2 or newer")` — the hello frame says so verbatim.
//!
//! Requires the user to grant the "System Audio Recording" permission on first run (the OS's
//! own prompt).
//!
//! Verification note: the plain-C signatures below (`AudioObjectGetPropertyData`,
//! `AudioObjectGetPropertyDataSize`, `proc_pidpath`, `AudioDeviceCreateIOProcID`,
//! `AudioDeviceStart`, `AudioDeviceStop`, `AudioDeviceDestroyIOProcID`,
//! `AudioHardwareCreateAggregateDevice`, `AudioHardwareDestroyAggregateDevice`,
//! `AudioHardwareDestroyProcessTap`) are transcribed from Apple's published
//! `CoreAudio/AudioHardware.h`/`AudioToolbox` headers for macOS 14.2+ and MUST be checked
//! against the actual SDK headers on a macOS host (`xcrun --show-sdk-path`) before this file is
//! trusted to compile. The ONE piece with materially lower confidence is
//! `CATapDescription` construction (`build_tap_description`): it is an Objective-C class with
//! no C-only equivalent, so it is built here via hand-transcribed `objc_msgSend` calls rather
//! than the `objc`/`objc2` crate (neither is a workspace dependency, and adding one is an
//! architecture change outside this fix's scope) — this is the single most likely build/runtime
//! failure point in the file and needs a macOS host with the AudioToolbox headers to confirm.
//! Keep the dedicated-thread + process-object-list + aggregate-device-with-tap shape unchanged
//! if any signature differs.

use std::collections::HashMap;
use std::ffi::{c_void, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use core_foundation::array::CFArray;
use core_foundation::base::TCFType;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;

use super::{reduce_to_basename, MonitorError, SessionLevel, SessionMonitor};

/// Opaque Core Audio object id (`AudioObjectID`, a `u32` per the framework's own typedef).
type AudioObjectId = u32;

/// `kAudioHardwarePropertyProcessObjectList`'s numeric selector (from `AudioHardware.h`).
const K_AUDIO_HARDWARE_PROPERTY_PROCESS_OBJECT_LIST: u32 = 0x70_6c_69_73; // 'plis'
/// `kAudioProcessPropertyPID`'s numeric selector (from `AudioHardware.h`) — the SECOND
/// `AudioObjectGetPropertyData` call every process object needs: the object id Core Audio
/// enumerates is its OWN opaque `AudioObjectID`, never a POSIX pid, so the real pid must be
/// read back through this property before `proc_pidpath` can resolve anything. Encoded the
/// same four-char-code way as the already-verified `'plis'` selector above: 'p'=0x70, 'i'=0x69,
/// 'd'=0x64, ' '=0x20.
const K_AUDIO_PROCESS_PROPERTY_PID: u32 = 0x70_69_64_20; // 'pid '
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
    /// Verification note above applies.
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

    /// The Objective-C runtime's class lookup — used only to resolve `CATapDescription` (no
    /// C-only constructor exists for a process tap description).
    fn objc_getClass(name: *const i8) -> *mut c_void;
    /// The Objective-C runtime's selector registration.
    fn sel_registerName(name: *const i8) -> *mut c_void;
    /// Reports whether `cls` implements `sel` — checked before every `objc_msgSend` call this
    /// file makes against a selector name this file cannot verify against real SDK headers, so
    /// an unrecognized selector fails closed (returns `None`) instead of raising
    /// `doesNotRecognizeSelector:` and aborting the process.
    fn class_respondsToSelector(cls: *mut c_void, sel: *mut c_void) -> i8;
    /// `objc_msgSend`, the Objective-C runtime's message dispatch. Declared ONCE at its minimal
    /// (0-argument-selector) arity: Objective-C calls this same C symbol with a different
    /// effective signature per invocation depending on the target method's real argument count,
    /// which `extern "C"` cannot express as two declarations of one link name (`E0308`
    /// `clashing_extern_declarations`) — every wider-arity call site instead takes this
    /// declaration's function pointer and `transmute`s it to the specific
    /// `unsafe extern "C" fn(...)` type that call needs (`objc_msg_send_1`), the standard
    /// pattern for calling variable-arity `objc_msgSend` from Rust without the `objc`/`objc2`
    /// crates.
    fn objc_msgSend(receiver: *mut c_void, selector: *mut c_void) -> *mut c_void;
}

/// `objc_msgSend` called with a bare selector (no arguments) — the declared arity, so no
/// transmute is needed.
unsafe fn objc_msg_send_0(receiver: *mut c_void, selector: *mut c_void) -> *mut c_void {
    objc_msgSend(receiver, selector)
}

/// `objc_msgSend` called with one argument (`initStereoMixdownOfProcesses:` here) — reinterprets
/// `objc_msgSend`'s function pointer at the 3-argument arity the call actually needs. See
/// `objc_msgSend`'s doc comment for why this file cannot declare that arity as its own `extern`
/// item.
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

/// Mirrors `AudioBufferList` (`CoreAudioTypes.h`): a single-element view is enough here since
/// this file always taps a single stereo-or-mono process mixdown.
#[repr(C)]
struct AudioBufferList {
    /// Number of `AudioBuffer` entries in `buffers`.
    number_buffers: u32,
    /// The first (and, for this file's use, only) buffer.
    buffers: [AudioBuffer; 1],
}

/// Mirrors `AudioBuffer` (`CoreAudioTypes.h`): one channel-interleaved audio buffer.
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
    let buffer = unsafe { &(*in_input_data).buffers[0] };
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

/// Mirrors POSIX `struct utsname` (`sys/utsname.h`) — only `release` is read.
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

/// Runs on its own dedicated Core Audio thread for the process's lifetime: enumerates the
/// system's process object list, creates a process tap + aggregate device per tapped process
/// (tearing down a tap whose process has exited via `teardown_tap`, never merely dropping the
/// `TapHandle` — it holds no `Drop` impl of its own, so a dropped-without-teardown handle would
/// leak its `AudioDeviceIOProcID`/aggregate device/process tap for the rest of the process's
/// lifetime), and publishes each process's measured peak into `latest` every 100 ms. Exits (also
/// tearing down every still-live tap first) once `shutdown` is observed set.
fn run_process_tap_loop(
    latest: Arc<Mutex<Result<Vec<SessionLevel>, MonitorError>>>,
    shutdown: Arc<AtomicBool>,
) {
    let peaks: Arc<Mutex<HashMap<i32, f32>>> = Arc::new(Mutex::new(HashMap::new()));
    *TAP_PEAKS.lock().expect("tap peak map lock poisoned") = Some(peaks.clone());
    let mut taps: HashMap<i32, TapHandle> = HashMap::new();

    while !shutdown.load(Ordering::Relaxed) {
        let (result, pids) = enumerate_processes(&peaks);
        let live: std::collections::HashSet<i32> = pids.into_iter().collect();
        teardown_departed_taps(&mut taps, &live);
        for pid in &live {
            if !taps.contains_key(pid) {
                if let Some(handle) = create_tap_for_pid(*pid) {
                    taps.insert(*pid, handle);
                }
            }
        }
        peaks
            .lock()
            .expect("tap peak map lock poisoned")
            .retain(|pid, _| live.contains(pid));
        *latest.lock().expect("Core Audio state lock poisoned") = result;
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    // Shutdown requested: every remaining tap is "departed" relative to an empty live set, so
    // the same teardown path used every poll cycle also drains the thread's exit.
    teardown_departed_taps(&mut taps, &std::collections::HashSet::new());
}

/// Removes and tears down (`teardown_tap`) every tap whose pid is absent from `live`, in place.
/// The sole path that ever discards a `TapHandle`: `run_process_tap_loop` calls it both every
/// poll cycle (departed processes) and once more on exit (`live` empty, so every tap departs).
/// The pid diff itself lives in `departed_pids` so it can be exercised without invoking real
/// Core Audio teardown calls.
fn teardown_departed_taps(
    taps: &mut HashMap<i32, TapHandle>,
    live: &std::collections::HashSet<i32>,
) {
    for pid in departed_pids(taps.keys().copied(), live) {
        if let Some(handle) = taps.remove(&pid) {
            teardown_tap(handle);
        }
    }
}

/// Pure diff: which of `present` pids are absent from `live`. Split out from
/// `teardown_departed_taps` so the "which taps get torn down" decision is unit-testable without
/// a real Core Audio device/aggregate/tap to release.
fn departed_pids(
    present: impl Iterator<Item = i32>,
    live: &std::collections::HashSet<i32>,
) -> Vec<i32> {
    present.filter(|pid| !live.contains(pid)).collect()
}

/// Reads `pid`'s own `kAudioProcessPropertyPID` property off its Core Audio process object —
/// the object id Core Audio enumerates is its own opaque id, never a pid, so this call is
/// required before `proc_pidpath` can resolve anything real.
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
/// for the `alloc`/`init`-owned `CATapDescription` this file builds. `CATapDescription` is a
/// plain Objective-C class with no documented CoreFoundation toll-free bridging, so releasing it
/// via `CFRelease` (which relies on retain-count layout compatibility that is undocumented for
/// an arbitrary non-bridged class) is avoided in favor of the real message send.
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

/// Builds a `CATapDescription` mixing down `pid`'s own audio, via hand-transcribed
/// `objc_msgSend` calls (module doc's verification note: this is the file's highest-risk
/// surface). Returns the owned Objective-C instance pointer (release with
/// `release_objc_instance`).
fn build_stereo_mixdown_tap_description(pid: i32) -> Option<*mut c_void> {
    // SAFETY: every symbol here is a `dlsym`-resolved libobjc/CoreFoundation entry point;
    // each selector/class name is a static, NUL-terminated C string literal.
    unsafe {
        let class_name = CString::new("CATapDescription").ok()?;
        let class = objc_getClass(class_name.as_ptr());
        if class.is_null() {
            return None;
        }
        let alloc_sel = CString::new("alloc").ok()?;
        let instance = objc_msg_send_0(class, sel_registerName(alloc_sel.as_ptr()));
        if instance.is_null() {
            return None;
        }
        let init_sel_name = CString::new("initStereoMixdownOfProcesses:").ok()?;
        let init_sel = sel_registerName(init_sel_name.as_ptr());
        if class_respondsToSelector(class, init_sel) == 0 {
            // The selector this file guesses at is absent from the resolved SDK — fail
            // closed rather than risk `doesNotRecognizeSelector:` aborting the process.
            // `alloc` above yielded a +1-owned instance; release it here (manual reference
            // counting, matching every other error path in this function) rather than leaking
            // one Objective-C object per newly-discovered process.
            release_objc_instance(instance);
            return None;
        }
        let pid_number = CFNumber::from(pid);
        let pids = CFArray::from_CFTypes(&[pid_number]);
        let described = objc_msg_send_1(instance, init_sel, pids.as_CFTypeRef() as *mut c_void);
        if described.is_null() {
            return None;
        }
        Some(described)
    }
}

/// Attempts to create a full tap → aggregate-device → running-IO-proc chain for `pid`.
/// Returns `None` on any negotiation failure — a single process's tap failing must never take
/// down the whole enumeration loop, mirroring the Linux backend's per-node
/// `attach_monitor_stream` failure shape.
fn create_tap_for_pid(pid: i32) -> Option<TapHandle> {
    let description = build_stereo_mixdown_tap_description(pid)?;
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
        return None;
    }

    let uid = CFString::new(&format!("shadowcat-tap-{pid}"));
    let is_private = CFNumber::from(1i32);
    let auto_start = CFNumber::from(1i32);
    let sub_tap = CFDictionary::from_CFType_pairs(&[(
        CFString::new("uid").as_CFType(),
        uid.clone().as_CFType(),
    )]);
    let tap_list = CFArray::from_CFTypes(&[sub_tap]);
    let description_dict = CFDictionary::from_CFType_pairs(&[
        (CFString::new("uid").as_CFType(), uid.as_CFType()),
        (CFString::new("private").as_CFType(), is_private.as_CFType()),
        (
            CFString::new("tapautostart").as_CFType(),
            auto_start.as_CFType(),
        ),
        (CFString::new("taps").as_CFType(), tap_list.as_CFType()),
    ]);

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
        return None;
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
        return None;
    }

    // SAFETY: `aggregate_id`/`proc_id` were returned by the successful calls immediately above.
    let start_status = unsafe { AudioDeviceStart(aggregate_id, proc_id) };
    if start_status != 0 {
        teardown_tap(TapHandle {
            tap_id,
            aggregate_id,
            proc_id,
        });
        return None;
    }

    Some(TapHandle {
        tap_id,
        aggregate_id,
        proc_id,
    })
}

/// One enumeration pass: `kAudioHardwarePropertyProcessObjectList` -> real pid per object (via
/// `read_process_pid`) -> `proc_pidpath` reduced to a basename, with `peak` read from `peaks`
/// (populated by each tap's `tap_io_proc` callback) and reset for the next window, mirroring
/// the Linux backend's window-reset shape. Also returns every resolved pid, so
/// `run_process_tap_loop` can diff live taps against the current process set.
fn enumerate_processes(
    peaks: &Arc<Mutex<HashMap<i32, f32>>>,
) -> (Result<Vec<SessionLevel>, MonitorError>, Vec<i32>) {
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
    let mut pids = Vec::new();
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
        pids.push(pid);
        levels.push(SessionLevel {
            process: reduce_to_basename(&path),
            peak,
        });
    }
    (Ok(levels), pids)
}

#[cfg(test)]
mod tests;
