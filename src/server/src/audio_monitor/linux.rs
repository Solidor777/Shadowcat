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
//! `main_loop::MainLoop::new`/`run`/`quit`/`loop_`, `context::Context::new`/`connect`,
//! `Core::get_registry`, `Registry`'s listener builder (`global`/`global_remove`),
//! `GlobalObject`'s `props`, `spa::utils::dict::DictRef::get`,
//! `stream::Stream::new`/`connect`/`add_local_listener`, `StreamListener`,
//! `StreamFlags::{AUTOCONNECT, MAP_BUFFERS}`, `Stream::dequeue_buffer`, `Stream::state`,
//! `StreamState`, `Buffer::datas_mut`,
//! `Data::data`/`Data::chunk`, `Chunk::offset`/`Chunk::size`, `LoopRef::add_timer`,
//! `TimerSource::update_timer`, `spa::param::audio::AudioInfoRaw`,
//! `spa::pod::serialize::PodSerializer`, and `spa::pod::Pod::from_bytes` — was checked against
//! the resolved crate's own vendored source (`~/.cargo/registry/src/…/pipewire-0.8.0` and
//! `libspa-0.8.0`) and its `examples/audio-capture.rs`/`src/channel.rs`, not merely against
//! documentation.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
/// use std::sync::atomic::AtomicBool;
/// use std::sync::Arc;
/// use shadowcat::audio_monitor::linux::LinuxMonitor;
///
/// // Never panics on a PipeWire-less host: construction reports `Unsupported` instead.
/// let _outcome = LinuxMonitor::new(Arc::new(AtomicBool::new(false)));
/// ```
pub struct LinuxMonitor {
    /// Shared per-node state, keyed by PipeWire node id, updated by the background thread.
    nodes: Arc<Mutex<HashMap<u32, NodeState>>>,
}

impl LinuxMonitor {
    /// Spawns the dedicated PipeWire event-loop thread and returns a monitor reading its
    /// published state. Returns `MonitorError::Unsupported` when no PipeWire socket is
    /// reachable; the hello frame then reports `supported: false, reason: "PipeWire not
    /// running"`. `shutdown` is polled (via a PipeWire loop timer, since the dedicated thread
    /// spends nearly all its time blocked inside `MainLoop::run`) by the event loop itself;
    /// setting it calls `MainLoop::quit`, which returns control to `run_pipewire_loop` so it can
    /// tear down every live capture stream (and the PipeWire connection itself) before the
    /// thread exits, rather than holding them for the rest of the process's lifetime.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::atomic::AtomicBool;
    /// use std::sync::Arc;
    /// use shadowcat::audio_monitor::linux::LinuxMonitor;
    /// use shadowcat::audio_monitor::SessionMonitor;
    ///
    /// if let Ok(mut monitor) = LinuxMonitor::new(Arc::new(AtomicBool::new(false))) {
    ///     let _levels = monitor.poll(); // Ok(_) or a runtime `Backend` error, never a panic
    /// }
    /// ```
    pub fn new(shutdown: Arc<AtomicBool>) -> Result<Self, MonitorError> {
        pw::init();
        let nodes: Arc<Mutex<HashMap<u32, NodeState>>> = Arc::new(Mutex::new(HashMap::new()));
        let thread_nodes = nodes.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();

        std::thread::spawn(move || {
            run_pipewire_loop(thread_nodes, ready_tx, shutdown);
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

/// One capture stream's live PipeWire resources. Field order is load-bearing: struct fields
/// drop in declaration order, and `StreamListener::drop` (`spa::utils::hook::remove`) patches
/// pointers within the stream's own internal listener list — it must run before `Stream::drop`
/// (`pw_stream_destroy`) can free that memory. Source: `pipewire-0.8.0/examples/audio-capture.rs`
/// binds its `stream` variable before its `_listener` variable for the same reason (reverse-drop
/// order at scope exit tears the listener down first).
struct CaptureStream {
    /// Dropped FIRST — unregisters the `process` callback from the stream's listener list.
    listener: pw::stream::StreamListener<()>,
    /// Dropped SECOND — destroys the underlying `pw_stream`, safe only once no listener still
    /// points into it.
    stream: pw::stream::Stream,
}

impl CaptureStream {
    /// Reads the underlying `pw_stream`'s own connection state. A stream can transition to
    /// `StreamState::Error`/`Unconnected` asynchronously (its node's underlying device
    /// disappearing without the registry ever firing `global_remove` for the node object
    /// itself) — `run_pipewire_loop`'s shutdown timer sweeps on this every poll so a dead
    /// stream's stale peak reading is pruned rather than reported forever.
    fn is_connected(&self) -> bool {
        matches!(
            self.stream.state(),
            pw::stream::StreamState::Connecting
                | pw::stream::StreamState::Paused
                | pw::stream::StreamState::Streaming
        )
    }
}

/// Runs the PipeWire main loop on the calling (dedicated) thread for the process's lifetime (or
/// until `shutdown` is observed): connects to the session's PipeWire core, registers a `global`
/// listener that records every `Stream/Output/Audio` node, and updates `nodes` from each
/// stream's `process` callback. Signals readiness (or the reason it could not connect) once over
/// `ready`. A loop timer polls `shutdown` and calls `MainLoop::quit` once it is set, at which
/// point `MainLoop::run` returns and every local here (`streams`, `registry`, `core`, `context`,
/// `main_loop`) drops in reverse declaration order — `streams` (and so every `CaptureStream`,
/// hence every PipeWire stream) tears down before `core`/`context` do, which is the order a live
/// PipeWire connection requires.
fn run_pipewire_loop(
    nodes: Arc<Mutex<HashMap<u32, NodeState>>>,
    ready: std::sync::mpsc::Sender<Result<(), String>>,
    shutdown: Arc<AtomicBool>,
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
    let streams: Rc<RefCell<HashMap<u32, CaptureStream>>> = Rc::new(RefCell::new(HashMap::new()));
    let core_for_streams = core.clone();

    let _listener = registry
        .add_listener_local()
        .global({
            // `nodes`/`streams` are cloned here rather than moved directly into this `move`
            // closure: both are also needed by the `global_remove` closure below, and a `move`
            // closure captures whatever it uses BY VALUE (the whole `Arc`/`Rc`, not a
            // reference), consuming the outer binding — so a shared reference for the sibling
            // closure to clone from would no longer exist by the time it runs.
            let nodes = nodes.clone();
            let streams = streams.clone();
            move |g| {
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

                let Some(handle) = attach_monitor_stream(&core_for_streams, g.id, nodes.clone())
                else {
                    return;
                };
                streams.borrow_mut().insert(g.id, handle);
            }
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

    // Polled every 100ms from inside the loop itself: `MainLoop::run` blocks for the process's
    // lifetime otherwise, and `MainLoop`/`LoopRef` are `Rc`-based (not `Send`), so `shutdown`
    // cannot be observed from another thread the way the macOS backend's simple poll-loop does.
    // `pipewire::channel` (the crate's own documented cross-thread wakeup primitive) is the
    // alternative; a loop timer avoids the extra pipe/fd pair for a flag that only needs
    // checking, not real-time delivery. Kept alive for `main_loop.run()`'s duration — dropping
    // it early would disarm the timer. Also sweeps `streams` for any capture whose
    // `CaptureStream::is_connected` has gone false (see its doc comment) so a dead stream's
    // node entry does not linger reporting a stale peak.
    let shutdown_timer = main_loop.loop_().add_timer({
        let main_loop = main_loop.clone();
        let streams = streams.clone();
        let nodes = nodes.clone();
        move |_expirations| {
            if shutdown.load(Ordering::Relaxed) {
                main_loop.quit();
                return;
            }
            streams.borrow_mut().retain(|id, handle| {
                let connected = handle.is_connected();
                if !connected {
                    nodes.lock().expect("node state lock poisoned").remove(id);
                }
                connected
            });
        }
    });
    let _ = shutdown_timer.update_timer(
        Some(Duration::from_millis(100)),
        Some(Duration::from_millis(100)),
    );

    let _ = ready.send(Ok(()));
    main_loop.run();
}

/// Creates a `Stream::Input` connected directly to node `node_id`'s own ports
/// (`StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS`), whose `process` callback folds each
/// buffer's interleaved `F32LE` samples into that node's `peak` field via
/// `max-abs-sample`. Returns `None` on any negotiation failure (logged by discarding — a
/// single node's capture failing must never take down the whole discovery loop).
fn attach_monitor_stream(
    core: &pw::core::Core,
    node_id: u32,
    nodes: Arc<Mutex<HashMap<u32, NodeState>>>,
) -> Option<CaptureStream> {
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
            // `Data::chunk()` reports the VALID region this cycle actually wrote
            // (`offset()`/`size()`); `Data::data()` alone returns the full `maxsize`-sized
            // allocation, which can include stale/uninitialized bytes beyond it. Read only the
            // chunk's own window, clamped defensively (`valid_sample_window`) against a
            // malformed/out-of-range chunk rather than panicking on a bad slice range.
            let chunk_offset = data.chunk().offset() as usize;
            let chunk_size = data.chunk().size() as usize;
            let Some(samples) = data.data() else {
                return;
            };
            let (start, end) = valid_sample_window(samples.len(), chunk_offset, chunk_size);
            let samples = &samples[start..end];
            let peak = samples
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b).abs())
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
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .ok()?;

    Some(CaptureStream { listener, stream })
}

/// Pure bounds logic for reading only a `Data`'s valid chunk window (`chunk_offset`/`chunk_size`)
/// out of its full `data_len`-sized allocation, clamped so an out-of-range or malformed chunk can
/// never produce a slice range that panics: `start <= end <= data_len` always holds. Split out of
/// `attach_monitor_stream`'s `process` callback so the clamping logic is unit-testable without a
/// real `libspa::buffer::Data`.
fn valid_sample_window(data_len: usize, chunk_offset: usize, chunk_size: usize) -> (usize, usize) {
    let end = chunk_offset.saturating_add(chunk_size).min(data_len);
    let start = chunk_offset.min(end);
    (start, end)
}

#[cfg(test)]
mod tests;
