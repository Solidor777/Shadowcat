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
//! Verification note: every `pipewire`/`libspa` 0.8 symbol this file uses — `pipewire::init`,
//! `main_loop::MainLoop::new`/`run`, `context::Context::new`/`connect`, `Core::get_registry`,
//! `Registry`'s listener builder (`global`/`global_remove`), `GlobalObject`'s `props`,
//! `spa::utils::dict::DictRef::get`, `stream::Stream::new`/`connect`/`add_local_listener`,
//! `StreamListener`, `StreamFlags::{AUTOCONNECT, PASSIVE}`, `Stream::dequeue_buffer`,
//! `Buffer::datas_mut`, `Data::data`, `spa::param::audio::AudioInfoRaw`,
//! `spa::pod::serialize::PodSerializer`, and `spa::pod::Pod::from_bytes` — was checked against
//! the resolved crate's own vendored source (`~/.cargo/registry/src/…/pipewire-0.8.0`) and its
//! `examples/audio-capture.rs`, not merely against documentation.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use pipewire as pw;
use pw::spa;

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

    // Capture streams must outlive the `global` callback that creates them, and this closure
    // is `Fn` (not `FnMut`), so the per-node stream/listener pairs live behind `Rc<RefCell<_>>`
    // rather than a captured mutable binding. Everything here runs on this single dedicated
    // thread, so `Rc`/`RefCell` (not `Arc`/`Mutex`) is the right tool.
    let streams: Rc<RefCell<HashMap<u32, (pw::stream::Stream, pw::stream::StreamListener<()>)>>> =
        Rc::new(RefCell::new(HashMap::new()));
    let core_for_streams = core.clone();

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

            let Some(handle) = attach_monitor_stream(&core_for_streams, g.id, nodes.clone()) else {
                return;
            };
            streams.borrow_mut().insert(g.id, handle);
        })
        .global_remove({
            let streams = streams.clone();
            let nodes = nodes.clone();
            move |id| {
                streams.borrow_mut().remove(&id);
                nodes.lock().expect("node state lock poisoned").remove(&id);
            }
        })
        .register();

    let _ = ready.send(Ok(()));
    main_loop.run();
}

/// Creates a passive `Stream::Input` connected directly to node `node_id`'s own ports
/// (`StreamFlags::AUTOCONNECT | StreamFlags::PASSIVE`), whose `process` callback folds each
/// buffer's interleaved `F32LE` samples into that node's `peak` field via
/// `max-abs-sample`. Returns `None` on any negotiation failure (logged by discarding — a
/// single node's capture failing must never take down the whole discovery loop).
fn attach_monitor_stream(
    core: &pw::core::Core,
    node_id: u32,
    nodes: Arc<Mutex<HashMap<u32, NodeState>>>,
) -> Option<(pw::stream::Stream, pw::stream::StreamListener<()>)> {
    let props = pw::properties::properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Monitor",
        *pw::keys::MEDIA_ROLE => "Music",
    };
    let stream = pw::stream::Stream::new(core, "shadowcat-level-monitor", props).ok()?;

    let listener = stream
        .add_local_listener::<()>()
        .process(move |stream, _: &mut ()| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let datas = buffer.datas_mut();
            let Some(data) = datas.first_mut() else {
                return;
            };
            let Some(samples) = data.data() else {
                return;
            };
            let peak = samples
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]).abs())
                .fold(0f32, f32::max);
            if let Ok(mut guard) = nodes.lock() {
                if let Some(state) = guard.get_mut(&node_id) {
                    state.peak = state.peak.max(peak);
                }
            }
        })
        .register()
        .ok()?;

    let mut audio_info = spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::F32LE);
    let obj = spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    };
    let values: Vec<u8> = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(obj),
    )
    .ok()?
    .0
    .into_inner();
    let pod = spa::pod::Pod::from_bytes(&values)?;
    let mut params = [pod];

    stream
        .connect(
            spa::utils::Direction::Input,
            Some(node_id),
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::PASSIVE,
            &mut params,
        )
        .ok()?;

    Some((stream, listener))
}

#[cfg(test)]
mod tests;
