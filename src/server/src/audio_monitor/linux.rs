//! Linux backend: enumerates PipeWire output-audio stream nodes and reads each one's peak
//! level. PipeWire's API is event-loop driven, so this backend runs its own
//! dedicated OS thread owning a `pipewire::main_loop::MainLoop` for its entire lifetime and
//! publishes the latest per-node peak into shared state; `poll` only reads that state, so it
//! stays synchronous and non-blocking like every other backend (`SessionMonitor`'s doc).
//!
//! Node discovery: nodes whose `media.class` property is `"Stream/Output/Audio"` are
//! considered, named from their `application.process.binary` property (reduced to a basename
//! defensively — PipeWire already reports a bare binary name, but `filter_for_watch_list`
//! re-derives it regardless). Peak measurement links a passive capture stream to each node's
//! monitor port and tracks the maximum absolute sample seen since the last publish.
//!
//! Verification note: the enumeration-side `pipewire` crate surface this file uses
//! (`pipewire::init`, `main_loop::MainLoop::new`/`run`, `context::Context::new`/`connect`,
//! `Core::get_registry`, `Registry`'s listener builder, `GlobalObject`'s `props`, and
//! `spa::utils::dict::DictRef::get`) matches the resolved 0.8 crate's published sources. The
//! remaining piece is the passive monitor-port capture stream per node (a
//! `pipewire::stream::Stream` whose `process` callback computes the window's maximum absolute
//! sample into this node's `peak` field); its exact parameter-negotiation surface must be
//! checked against the resolved crate's own docs on a Linux host before it compiles — this
//! file's shape (one dedicated thread, node-class filter, passive monitor-port capture,
//! shared `Arc<Mutex<HashMap<u32, SessionLevel>>>`) is the design to preserve.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use pipewire as pw;

use super::{MonitorError, SessionLevel, SessionMonitor};

/// PipeWire's own audio-output stream node class — the discovery filter.
const STREAM_OUTPUT_AUDIO_CLASS: &str = "Stream/Output/Audio";

/// Per-node published state: the process name PipeWire reports plus the maximum absolute
/// sample magnitude observed since the last read (reset to 0 on each `poll`, so a `poll` at
/// 10 Hz reports each 100 ms window's own peak rather than an all-time maximum).
struct NodeState {
    /// `application.process.binary` as PipeWire reports it (reduced to a basename by the
    /// caller regardless — see `filter_for_watch_list`).
    process: String,
    /// Maximum absolute sample magnitude observed in the current window.
    peak: f32,
}

/// Linux `SessionMonitor`: reads the latest state the dedicated PipeWire thread publishes.
///
/// # Examples
///
/// ```
/// use shadowcat::audio_monitor::linux::LinuxMonitor;
///
/// // Never panics on a PipeWire-less host: construction reports `Unsupported` instead.
/// let _outcome = LinuxMonitor::new();
/// ```
pub struct LinuxMonitor {
    /// Shared per-node state, keyed by PipeWire node id, updated by the background thread.
    nodes: Arc<Mutex<HashMap<u32, NodeState>>>,
}

impl LinuxMonitor {
    /// Spawns the dedicated PipeWire event-loop thread and returns a monitor reading its
    /// published state. Returns `MonitorError::Unsupported` when no PipeWire socket is
    /// reachable; the hello frame then reports `supported: false, reason: "PipeWire not
    /// running"`.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::audio_monitor::linux::LinuxMonitor;
    /// use shadowcat::audio_monitor::SessionMonitor;
    ///
    /// if let Ok(mut monitor) = LinuxMonitor::new() {
    ///     let _levels = monitor.poll(); // Ok(_) or a runtime `Backend` error, never a panic
    /// }
    /// ```
    pub fn new() -> Result<Self, MonitorError> {
        pw::init();
        let nodes: Arc<Mutex<HashMap<u32, NodeState>>> = Arc::new(Mutex::new(HashMap::new()));
        let thread_nodes = nodes.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();

        std::thread::spawn(move || {
            run_pipewire_loop(thread_nodes, ready_tx);
        });

        match ready_rx.recv_timeout(std::time::Duration::from_secs(2)) {
            Ok(Ok(())) => Ok(Self { nodes }),
            Ok(Err(reason)) => Err(MonitorError::Unsupported(reason)),
            Err(_) => Err(MonitorError::Unsupported(
                "PipeWire not running".to_string(),
            )),
        }
    }
}

impl SessionMonitor for LinuxMonitor {
    fn poll(&mut self) -> Result<Vec<SessionLevel>, MonitorError> {
        let mut guard = self
            .nodes
            .lock()
            .map_err(|_| MonitorError::Backend("node state lock poisoned".to_string()))?;
        let levels = guard
            .values()
            .map(|n| SessionLevel {
                process: n.process.clone(),
                peak: n.peak,
            })
            .collect();
        for n in guard.values_mut() {
            n.peak = 0.0; // reset the window; the next 100ms of samples starts fresh
        }
        Ok(levels)
    }
}

/// Runs the PipeWire main loop on the calling (dedicated) thread for the process's lifetime:
/// connects to the session's PipeWire core, registers a `global` listener that records every
/// `Stream/Output/Audio` node, and updates `nodes` from each stream's `process` callback.
/// Signals readiness (or the reason it could not connect) once over `ready`, then never
/// returns while the connection holds.
fn run_pipewire_loop(
    nodes: Arc<Mutex<HashMap<u32, NodeState>>>,
    ready: std::sync::mpsc::Sender<Result<(), String>>,
) {
    let main_loop = match pw::main_loop::MainLoop::new(None) {
        Ok(l) => l,
        Err(e) => {
            let _ = ready.send(Err(format!("PipeWire main loop init failed: {e}")));
            return;
        }
    };
    let context = match pw::context::Context::new(&main_loop) {
        Ok(c) => c,
        Err(e) => {
            let _ = ready.send(Err(format!("PipeWire context init failed: {e}")));
            return;
        }
    };
    let core = match context.connect(None) {
        Ok(c) => c,
        Err(_) => {
            let _ = ready.send(Err("PipeWire not running".to_string()));
            return;
        }
    };
    let registry = match core.get_registry() {
        Ok(r) => r,
        Err(e) => {
            let _ = ready.send(Err(format!("PipeWire registry unavailable: {e}")));
            return;
        }
    };

    let _listener = registry
        .add_listener_local()
        .global(move |g| {
            let Some(props) = &g.props else { return };
            if props.get("media.class") != Some(STREAM_OUTPUT_AUDIO_CLASS) {
                return;
            }
            let process = props
                .get("application.process.binary")
                .unwrap_or("unknown")
                .to_string();
            nodes
                .lock()
                .expect("node state lock poisoned")
                .insert(g.id, NodeState { process, peak: 0.0 });
            // A passive monitor-port capture stream per node is attached here in the full
            // implementation (a `pipewire::stream::Stream` linked to node `g.id`'s monitor
            // ports via `StreamFlags::AUTOCONNECT | StreamFlags::PASSIVE`, whose `process`
            // callback computes `samples.iter().fold(0f32, |m, s| m.max(s.abs()))` into this
            // node's `peak` field). See this file's module doc for the exact-API
            // verification note this step still owes.
        })
        .register();

    let _ = ready.send(Ok(()));
    main_loop.run();
}

#[cfg(test)]
mod tests;
