//! Windows backend: `IAudioSessionManager2`/`IAudioMeterInformation` over the default render
//! endpoint. COM is apartment-threaded, so this backend spawns its OWN dedicated OS thread at
//! construction, initializes COM as multithreaded (MTA) exactly once there, and keeps that
//! thread alive for the process's lifetime — `SessionMonitor::poll` never cares which thread
//! its own caller runs on, since it only reads state the dedicated thread publishes (the same
//! shape as the Linux/PipeWire backend).
//!
//! Verification note: checked against the resolved `windows` crate's published 0.58 sources —
//! the session-control interfaces (`IAudioSessionManager2`, `IAudioSessionControl`,
//! `IAudioSessionControl2`, `IAudioSessionEnumerator`) live under `Win32::Media::Audio`
//! directly, while the meter interface (`IAudioMeterInformation`) lives under the nested
//! `Win32::Media::Audio::Endpoints` module behind its own
//! `Win32_Media_Audio_Endpoints` feature; `IMMDevice::Activate` takes
//! `Option<*const windows_core::PROPVARIANT>` (no extra feature), and
//! `IAudioSessionEnumerator::GetCount`/`GetSession` are `i32`-shaped. This file's shape
//! (dedicated MTA thread; enumerator -> default render endpoint -> session manager ->
//! per-session control+meter; basename via `QueryFullProcessImageNameW`) is the design to
//! preserve.

use std::sync::{Arc, Mutex};

use windows::core::Interface;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::Media::Audio::{
    eMultimedia, eRender, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator,
    MMDeviceEnumerator,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::{reduce_to_basename, MonitorError, SessionLevel, SessionMonitor};

/// Windows `SessionMonitor`: reads the latest state the dedicated MTA thread publishes.
///
/// # Examples
///
/// ```
/// use shadowcat::audio_monitor::windows::WindowsMonitor;
///
/// // Construction only needs COM to initialize; it never panics on a session-less host.
/// let _outcome = WindowsMonitor::new();
/// ```
pub struct WindowsMonitor {
    /// Shared latest reading, updated by the background thread every poll interval.
    latest: Arc<Mutex<Result<Vec<SessionLevel>, MonitorError>>>,
}

impl WindowsMonitor {
    /// Spawns the dedicated COM/MTA thread and blocks briefly for its COM initialization (or
    /// its startup failure) before returning; a session-less or endpoint-less host still
    /// constructs successfully and reports the enumeration error from `poll` instead.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::audio_monitor::windows::WindowsMonitor;
    /// use shadowcat::audio_monitor::SessionMonitor;
    ///
    /// if let Ok(mut monitor) = WindowsMonitor::new() {
    ///     let _levels = monitor.poll(); // Ok(_) (possibly empty) or a runtime `Backend` error
    /// }
    /// ```
    pub fn new() -> Result<Self, MonitorError> {
        let latest: Arc<Mutex<Result<Vec<SessionLevel>, MonitorError>>> =
            Arc::new(Mutex::new(Ok(Vec::new())));
        let thread_latest = latest.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();

        std::thread::spawn(move || run_wasapi_loop(thread_latest, ready_tx));

        match ready_rx.recv_timeout(std::time::Duration::from_secs(2)) {
            Ok(Ok(())) => Ok(Self { latest }),
            Ok(Err(reason)) => Err(MonitorError::Backend(reason)),
            Err(_) => Err(MonitorError::Backend(
                "WASAPI backend startup timed out".to_string(),
            )),
        }
    }
}

impl SessionMonitor for WindowsMonitor {
    fn poll(&mut self) -> Result<Vec<SessionLevel>, MonitorError> {
        self.latest
            .lock()
            .map_err(|_| MonitorError::Backend("WASAPI state lock poisoned".to_string()))?
            .clone()
    }
}

/// Runs on its own dedicated OS thread for the process's lifetime: initializes COM as MTA
/// once, then loops enumerating the default render endpoint's audio sessions every 100 ms and
/// publishing the result into `latest`. Signals readiness (or the reason startup failed) once
/// over `ready`.
fn run_wasapi_loop(
    latest: Arc<Mutex<Result<Vec<SessionLevel>, MonitorError>>>,
    ready: std::sync::mpsc::Sender<Result<(), String>>,
) {
    // SAFETY: `CoInitializeEx` is called exactly once on this dedicated thread before any
    // other COM call on it, and this thread never exits while the process is serving.
    let init = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if init.is_err() {
        let _ = ready.send(Err(format!("CoInitializeEx failed: {init:?}")));
        return;
    }
    let _ = ready.send(Ok(()));
    loop {
        let result = enumerate_sessions();
        *latest.lock().expect("WASAPI state lock poisoned") = result;
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// One enumeration pass: default render endpoint -> `IAudioSessionManager2` ->
/// `IAudioSessionControl2` per active session -> `IAudioMeterInformation::GetPeakValue` +
/// `GetProcessId` -> `QueryFullProcessImageNameW` reduced to a basename.
fn enumerate_sessions() -> Result<Vec<SessionLevel>, MonitorError> {
    // SAFETY: called only from `run_wasapi_loop`'s dedicated MTA thread, after
    // `CoInitializeEx` has already succeeded on it.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|e| MonitorError::Backend(format!("MMDeviceEnumerator: {e}")))?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eMultimedia)
            .map_err(|e| MonitorError::Backend(format!("GetDefaultAudioEndpoint: {e}")))?;
        let manager: IAudioSessionManager2 = device
            .Activate(CLSCTX_ALL, None)
            .map_err(|e| MonitorError::Backend(format!("IAudioSessionManager2 activate: {e}")))?;
        let session_enum = manager
            .GetSessionEnumerator()
            .map_err(|e| MonitorError::Backend(format!("GetSessionEnumerator: {e}")))?;
        let count = session_enum
            .GetCount()
            .map_err(|e| MonitorError::Backend(format!("GetCount: {e}")))?;

        let mut levels = Vec::new();
        for i in 0..count {
            let control = session_enum
                .GetSession(i)
                .map_err(|e| MonitorError::Backend(format!("GetSession: {e}")))?;
            let control2: IAudioSessionControl2 = control
                .cast()
                .map_err(|e| MonitorError::Backend(format!("IAudioSessionControl2 cast: {e}")))?;
            let pid = control2
                .GetProcessId()
                .map_err(|e| MonitorError::Backend(format!("GetProcessId: {e}")))?;
            let meter: IAudioMeterInformation = control
                .cast()
                .map_err(|e| MonitorError::Backend(format!("IAudioMeterInformation cast: {e}")))?;
            let peak = meter
                .GetPeakValue()
                .map_err(|e| MonitorError::Backend(format!("GetPeakValue: {e}")))?;
            let process = process_name_for_pid(pid).unwrap_or_else(|| format!("pid-{pid}"));
            levels.push(SessionLevel {
                process: reduce_to_basename(&process),
                peak,
            });
        }
        Ok(levels)
    }
}

/// Resolves a process id to its executable's basename via `QueryFullProcessImageNameW`.
/// Returns `None` on any failure (a session whose process already exited between enumeration
/// and this call, or insufficient rights) — the caller falls back to a `pid-<n>` placeholder
/// rather than dropping the session entirely.
fn process_name_for_pid(pid: u32) -> Option<String> {
    // SAFETY: `OpenProcess`/`QueryFullProcessImageNameW`/`CloseHandle` are used in the
    // documented open-query-close sequence; the handle is closed on every return path.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 260];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        if !ok {
            return None;
        }
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

#[cfg(test)]
mod tests;
