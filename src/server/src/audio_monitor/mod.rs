//! `shadowcat audio-monitor`: a localhost-only subcommand serving the peak audio level of
//! watched OS processes (Discord, any voice app) to the ducking module's `OsMonitorSource`
//! over a WebSocket. Never a second executable — a `CliCommand::AudioMonitor` branch of the
//! single `shadowcat` binary.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

/// A scripted in-memory backend for tests — never compiled into the release binary.
#[cfg(test)]
pub mod fake;
/// Linux backend (PipeWire).
#[cfg(target_os = "linux")]
pub mod linux;
/// macOS backend (Core Audio process tap, macOS 14.2+).
#[cfg(target_os = "macos")]
pub mod macos;
/// The localhost WebSocket server: origin allowlist, hello/levels/watch frames, the 10 Hz loop.
pub mod server;
/// Windows backend (WASAPI `IAudioSessionManager2`/`IAudioMeterInformation`).
#[cfg(target_os = "windows")]
pub mod windows;

/// One watched process's current peak output level, already basename-reduced and clamped.
///
/// # Examples
///
/// ```
/// use shadowcat::audio_monitor::SessionLevel;
///
/// let level = SessionLevel { process: "discord".to_string(), peak: 0.5 };
/// assert_eq!(level.process, "discord");
/// assert!(level.peak <= 1.0);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct SessionLevel {
    /// The process's executable basename (never a full path — see `reduce_to_basename`),
    /// truncated to `PROCESS_NAME_MAX_CHARS`.
    pub process: String,
    /// Peak output level, clamped to `[0, 1]`.
    pub peak: f32,
}

/// Why a platform backend could not be constructed, or could not read the current sessions.
///
/// # Examples
///
/// ```
/// use shadowcat::audio_monitor::MonitorError;
///
/// let err = MonitorError::Unsupported("PipeWire not running".to_string());
/// assert!(matches!(err, MonitorError::Unsupported(_)));
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum MonitorError {
    /// This platform/OS version has no working backend (e.g. macOS < 14.2, no PipeWire
    /// socket). The string is the player-presentable reason surfaced on the `hello` frame.
    Unsupported(String),
    /// A backend call failed at runtime (device enumeration, OS API error). The string is a
    /// diagnostic-only message (logged, never sent to a client).
    Backend(String),
}

/// Polled at 10 Hz by `server::run`'s loop. Every platform backend implements this the same
/// poll-shape way: each backend runs its own dedicated background thread internally
/// (COM-apartment-safe on Windows, event-loop-driven on Linux/PipeWire) and publishes its
/// latest reading into shared state; `poll` just reads the latest published value, so the
/// caller never branches on OS and never blocks waiting on a platform API call.
///
/// # Examples
///
/// ```
/// use shadowcat::audio_monitor::{MonitorError, SessionLevel, SessionMonitor};
///
/// struct Silent;
/// impl SessionMonitor for Silent {
///     fn poll(&mut self) -> Result<Vec<SessionLevel>, MonitorError> {
///         Ok(Vec::new()) // no watched process is making sound right now
///     }
/// }
///
/// let mut monitor = Silent;
/// assert!(monitor.poll().unwrap().is_empty());
/// ```
pub trait SessionMonitor: Send {
    /// Returns the current per-process peak levels, or a runtime `MonitorError::Backend`.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::audio_monitor::{MonitorError, SessionLevel, SessionMonitor};
    ///
    /// struct Loud;
    /// impl SessionMonitor for Loud {
    ///     fn poll(&mut self) -> Result<Vec<SessionLevel>, MonitorError> {
    ///         Ok(vec![SessionLevel { process: "discord".to_string(), peak: 0.9 }])
    ///     }
    /// }
    ///
    /// let mut monitor = Loud;
    /// assert_eq!(monitor.poll().unwrap()[0].peak, 0.9);
    /// ```
    fn poll(&mut self) -> Result<Vec<SessionLevel>, MonitorError>;
}

/// Constructs the platform-appropriate `SessionMonitor`, or `MonitorError::Unsupported` when
/// this OS/OS-version has none (macOS < 14.2, Linux with no PipeWire socket running, or a
/// build for any other target).
///
/// # Examples
///
/// ```
/// use shadowcat::audio_monitor::platform_monitor;
///
/// // Never panics: a host without a working backend reports `Unsupported` instead.
/// let _outcome = platform_monitor();
/// ```
pub fn platform_monitor() -> Result<Box<dyn SessionMonitor>, MonitorError> {
    #[cfg(target_os = "windows")]
    {
        windows::WindowsMonitor::new().map(|m| Box::new(m) as Box<dyn SessionMonitor>)
    }
    #[cfg(target_os = "macos")]
    {
        macos::MacosMonitor::new().map(|m| Box::new(m) as Box<dyn SessionMonitor>)
    }
    #[cfg(target_os = "linux")]
    {
        linux::LinuxMonitor::new().map(|m| Box::new(m) as Box<dyn SessionMonitor>)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        Err(MonitorError::Unsupported(
            "this operating system".to_string(),
        ))
    }
}

/// Basename length cap before serialization; names longer than this are truncated.
///
/// # Examples
///
/// ```
/// use shadowcat::audio_monitor::PROCESS_NAME_MAX_CHARS;
///
/// assert_eq!(PROCESS_NAME_MAX_CHARS, 128);
/// ```
pub const PROCESS_NAME_MAX_CHARS: usize = 128;

/// Reduces a full process path/name to its basename — the full path never leaves a backend
/// (Windows' `QueryFullProcessImageNameW`, macOS' `proc_pidpath`). Both `/` and `\` are
/// treated as separators: `\` is an ordinary character to `std::path::Path` on a Unix host,
/// so a separator-agnostic split is what guarantees a Windows-style path can never leak
/// whole out of a non-Windows monitor.
///
/// # Examples
///
/// ```
/// use shadowcat::audio_monitor::reduce_to_basename;
///
/// assert_eq!(reduce_to_basename("/usr/bin/discord"), "discord");
/// assert_eq!(reduce_to_basename("C:\\Program Files\\Discord\\Discord.exe"), "Discord.exe");
/// assert_eq!(reduce_to_basename("discord"), "discord");
/// ```
pub fn reduce_to_basename(full: &str) -> String {
    full.rsplit(['/', '\\'])
        .find(|segment| !segment.is_empty())
        .unwrap_or(full)
        .to_string()
}

/// Applies `PROCESS_NAME_MAX_CHARS` by Unicode scalar count (never byte length, so a
/// multi-byte name is never truncated mid-codepoint).
///
/// # Examples
///
/// ```
/// use shadowcat::audio_monitor::truncate_process_name;
///
/// let long = "x".repeat(200);
/// assert_eq!(truncate_process_name(&long).chars().count(), 128);
/// ```
pub fn truncate_process_name(name: &str) -> String {
    name.chars().take(PROCESS_NAME_MAX_CHARS).collect()
}

/// Clamps a peak reading to the wire-safe `[0, 1]` range (a backend may read a raw meter
/// value outside it transiently).
///
/// # Examples
///
/// ```
/// use shadowcat::audio_monitor::clamp_peak;
///
/// assert_eq!(clamp_peak(1.5), 1.0);
/// assert_eq!(clamp_peak(0.42), 0.42);
/// ```
pub fn clamp_peak(peak: f32) -> f32 {
    peak.clamp(0.0, 1.0)
}

/// Case-insensitive substring match against the watch list. An empty `watch` matches nothing
/// (never "match all").
///
/// # Examples
///
/// ```
/// use shadowcat::audio_monitor::matches_watch_list;
///
/// assert!(matches_watch_list("Discord.exe", &["discord".to_string()]));
/// assert!(!matches_watch_list("firefox", &["discord".to_string()]));
/// ```
pub fn matches_watch_list(process_basename: &str, watch: &[String]) -> bool {
    let lower = process_basename.to_lowercase();
    watch.iter().any(|w| lower.contains(&w.to_lowercase()))
}

/// Filters + basename-reduces + clamps + truncates raw backend output into the sessions that
/// are actually allowed to leave the process — the ONE place this happens; filtering happens
/// in the monitor process, before anything is sent over the wire. Every backend's own
/// `SessionLevel` construction already reduces to a basename at its own call site; this
/// function re-derives it defensively too, so a future backend that forgets cannot leak a
/// full path.
///
/// # Examples
///
/// ```
/// use shadowcat::audio_monitor::{filter_for_watch_list, SessionLevel};
///
/// let raw = vec![
///     SessionLevel { process: "/usr/bin/discord".to_string(), peak: 1.5 },
///     SessionLevel { process: "/usr/bin/firefox".to_string(), peak: 0.3 },
/// ];
/// let filtered = filter_for_watch_list(raw, &["discord".to_string()]);
/// assert_eq!(filtered, vec![SessionLevel { process: "discord".to_string(), peak: 1.0 }]);
/// ```
pub fn filter_for_watch_list(raw: Vec<SessionLevel>, watch: &[String]) -> Vec<SessionLevel> {
    raw.into_iter()
        .map(|s| SessionLevel {
            process: truncate_process_name(&reduce_to_basename(&s.process)),
            peak: clamp_peak(s.peak),
        })
        .filter(|s| matches_watch_list(&s.process, watch))
        .collect()
}

#[cfg(test)]
mod tests;
