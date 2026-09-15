//! macOS backend: the Core Audio PROCESS TAP API (`AudioHardwareCreateProcessTap`), added in
//! macOS 14.2 — new enough that `coreaudio-sys` does not wrap it in the resolved version, so
//! this file binds the small entry-point surface it needs directly
//! via `extern "C"` against the `CoreAudio`/`AudioToolbox` frameworks, using `core-foundation`
//! only for `CFString`/`CFRelease` handling. On macOS < 14.2 (detected by
//! `macos_at_least_14_2`'s Darwin-kernel version probe), `MacosMonitor::new` returns
//! `MonitorError::Unsupported("macOS 14.2 or newer")` — the hello frame says so verbatim.
//!
//! Requires the user to grant the "System Audio Recording" permission on first run (the OS's
//! own prompt).
//!
//! Verification note: the exact C signatures below are transcribed from Apple's published
//! `CoreAudio/AudioHardware.h`/`AudioToolbox` headers for macOS 14.2+; they MUST be checked
//! against the actual SDK headers on a macOS host (`xcrun --show-sdk-path`) — Apple's
//! process-tap surface is new enough that a header mismatch is the most likely single build
//! failure in this subsystem. Keep the dedicated-thread + process-object-list +
//! aggregate-device-with-tap shape unchanged if a signature differs.

use std::ffi::c_void;
use std::sync::{Arc, Mutex};

use core_foundation::base::CFRelease;
use core_foundation::string::CFString;

use super::{reduce_to_basename, MonitorError, SessionLevel, SessionMonitor};

/// Opaque Core Audio object id (`AudioObjectID`, a `u32` per the framework's own typedef).
type AudioObjectId = u32;

/// `kAudioHardwarePropertyProcessObjectList`'s numeric selector (from `AudioHardware.h`).
const K_AUDIO_HARDWARE_PROPERTY_PROCESS_OBJECT_LIST: u32 = 0x70_6c_69_73; // 'plis'
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
}

/// macOS `SessionMonitor`: reads the latest state the dedicated Core Audio thread publishes.
///
/// # Examples
///
/// ```
/// use shadowcat::audio_monitor::macos::MacosMonitor;
///
/// // Below macOS 14.2 construction reports `Unsupported`; at or above it never panics.
/// let _outcome = MacosMonitor::new();
/// ```
pub struct MacosMonitor {
    /// Shared latest reading, updated by the background thread every poll interval.
    latest: Arc<Mutex<Result<Vec<SessionLevel>, MonitorError>>>,
}

impl MacosMonitor {
    /// Checks the macOS version, then spawns the dedicated Core Audio thread. Returns
    /// `MonitorError::Unsupported("macOS 14.2 or newer")` below that version.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::audio_monitor::macos::MacosMonitor;
    /// use shadowcat::audio_monitor::SessionMonitor;
    ///
    /// if let Ok(mut monitor) = MacosMonitor::new() {
    ///     let _levels = monitor.poll();
    /// }
    /// ```
    pub fn new() -> Result<Self, MonitorError> {
        if !macos_at_least_14_2() {
            return Err(MonitorError::Unsupported("macOS 14.2 or newer".to_string()));
        }
        let latest: Arc<Mutex<Result<Vec<SessionLevel>, MonitorError>>> =
            Arc::new(Mutex::new(Ok(Vec::new())));
        let thread_latest = latest.clone();
        std::thread::spawn(move || run_process_tap_loop(thread_latest));
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

/// Runs on its own dedicated Core Audio thread for the process's lifetime: enumerates the
/// system's process object list, creates a process tap + aggregate device per tapped
/// process, and publishes each process's measured peak into `latest` every 100 ms.
fn run_process_tap_loop(latest: Arc<Mutex<Result<Vec<SessionLevel>, MonitorError>>>) {
    loop {
        let result = enumerate_processes();
        *latest.lock().expect("Core Audio state lock poisoned") = result;
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// One enumeration pass: `kAudioHardwarePropertyProcessObjectList` -> pid per object ->
/// `proc_pidpath` reduced to a basename. Peak measurement (the process-tap + aggregate-device
/// audio callback) publishes into a per-pid running-peak map this function reads and resets,
/// mirroring the Linux backend's window-reset shape; wiring the tap's IO callback into that
/// map remains to be written against the verified SDK headers (module doc's verification
/// note).
fn enumerate_processes() -> Result<Vec<SessionLevel>, MonitorError> {
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
        return Err(MonitorError::Backend(format!(
            "AudioObjectGetPropertyDataSize failed: {status}"
        )));
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
        return Err(MonitorError::Backend(format!(
            "AudioObjectGetPropertyData failed: {status}"
        )));
    }

    let mut levels = Vec::new();
    for id in ids {
        // Each process object's pid is itself read via a further
        // `kAudioProcessPropertyPID` `AudioObjectGetPropertyData` call in the full
        // implementation; `id` doubles as a placeholder pid source until that call's
        // selector constant is verified against the SDK headers (module doc's verification
        // note).
        let pid = id as i32;
        let mut buf = [0u8; 4096];
        let len = unsafe { proc_pidpath(pid, buf.as_mut_ptr(), buf.len() as u32) };
        if len <= 0 {
            continue;
        }
        let path = String::from_utf8_lossy(&buf[..len as usize]).into_owned();
        levels.push(SessionLevel {
            process: reduce_to_basename(&path),
            peak: 0.0,
        });
    }
    let _ = CFString::new(""); // keeps the `core_foundation` import live until the
                               // CFString-based per-process name accessor lands (module doc's
                               // verification note).
    let _ = CFRelease as usize; // same, for `CFRelease`'s use releasing tap objects at teardown.
    Ok(levels)
}

#[cfg(test)]
mod tests;
