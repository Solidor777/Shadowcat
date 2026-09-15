//! The `shadowcat audio-monitor` localhost WebSocket server: origin-gated `/levels` upgrade,
//! the `hello`/`levels`/`watch` frame protocol, and the 10 Hz broadcast loop.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};

use super::{filter_for_watch_list, platform_monitor, MonitorError, SessionLevel, SessionMonitor};
use crate::config::AudioMonitorArgs;

/// How often the loop polls the backend and broadcasts a `levels` frame.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Default watched-process substring when `--watch`/the live `watch` frame is empty.
const DEFAULT_WATCH: &str = "discord";

/// The `hello` frame — sent once, immediately after a connection is accepted.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OutgoingFrame {
    /// Sent once on connect: the host OS and whether a working backend exists.
    Hello {
        /// A short OS label (`std::env::consts::OS`: `"windows"` | `"macos"` | `"linux"`).
        os: &'static str,
        /// Whether `platform_monitor()` returned a working backend.
        supported: bool,
        /// Present iff `supported` is false: the player-presentable reason.
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// Sent at `POLL_INTERVAL`: the current watch-filtered session levels.
    Levels {
        /// The watched sessions currently active, already filtered/clamped/truncated.
        sessions: Vec<WireSessionLevel>,
    },
}

/// Wire shape of one `SessionLevel`.
#[derive(Debug, Clone, Serialize)]
struct WireSessionLevel {
    /// The process's basename.
    process: String,
    /// Clamped peak level.
    peak: f32,
}

impl From<SessionLevel> for WireSessionLevel {
    fn from(s: SessionLevel) -> Self {
        Self {
            process: s.process,
            peak: s.peak,
        }
    }
}

/// A frame the client may send: replaces the live watch list without a restart.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum IncomingFrame {
    /// Replace the watch list with `names`.
    Watch {
        /// The new watch-list substrings (case-insensitive; replaces the previous list
        /// wholesale).
        names: Vec<String>,
    },
}

/// Shared server state: the origin allowlist, the live watch list (mutated by `watch`
/// frames), and the most recent poll result the background polling thread published.
struct SharedState {
    /// Origins allowed to complete the WS upgrade.
    allow_origin: Vec<String>,
    /// The live watch list; starts from `--watch` (default `["discord"]` when empty) and is
    /// replaced wholesale by every `watch` frame from ANY connected client.
    watch: Mutex<Vec<String>>,
    /// The latest raw (unfiltered) backend poll, published by the dedicated polling thread
    /// this module spawns in `run_with_monitor`.
    latest: Arc<Mutex<Result<Vec<SessionLevel>, MonitorError>>>,
    /// Whether `platform_monitor()` produced a working backend at all (drives the `hello`
    /// frame's `supported`/`reason`; independent of a later transient `MonitorError::Backend`).
    supported: bool,
    /// Present iff `!supported`: the player-presentable reason.
    unsupported_reason: Option<String>,
}

/// Runs `shadowcat audio-monitor`: constructs the real platform backend, then serves. Never
/// returns `Ok` while serving — only on a bind failure.
///
/// # Examples
///
/// ```no_run
/// use shadowcat::audio_monitor::server::run;
/// use shadowcat::config::AudioMonitorArgs;
///
/// # async fn example() -> anyhow::Result<()> {
/// // Binds 127.0.0.1 and serves until the process exits.
/// run(AudioMonitorArgs { port: 31998, allow_origin: vec![], watch: vec![] }).await
/// # }
/// ```
pub async fn run(args: AudioMonitorArgs) -> anyhow::Result<()> {
    let (supported, unsupported_reason, monitor) = match platform_monitor() {
        Ok(m) => (true, None, Some(m)),
        Err(MonitorError::Unsupported(reason)) => (false, Some(reason), None),
        Err(MonitorError::Backend(reason)) => (false, Some(reason), None),
    };
    run_with_monitor(args, supported, unsupported_reason, monitor).await
}

/// The testable core of `run`: takes the backend construction OUTCOME already decided (so
/// tests can inject a `FakeMonitor`/a scripted `Unsupported` outcome without touching a real
/// OS audio API). Spawns the polling thread (only when `monitor` is `Some`), binds
/// `127.0.0.1:<port>`, prints the bound port, and serves until the process exits.
pub(super) async fn run_with_monitor(
    args: AudioMonitorArgs,
    supported: bool,
    unsupported_reason: Option<String>,
    monitor: Option<Box<dyn SessionMonitor>>,
) -> anyhow::Result<()> {
    let initial_watch = if args.watch.is_empty() {
        vec![DEFAULT_WATCH.to_string()]
    } else {
        args.watch
    };
    let mut allow_origin = args.allow_origin;
    if allow_origin.is_empty() {
        allow_origin.push("http://localhost:30000".to_string());
        allow_origin.push("http://127.0.0.1:30000".to_string());
    }

    let latest: Arc<Mutex<Result<Vec<SessionLevel>, MonitorError>>> =
        Arc::new(Mutex::new(Ok(Vec::new())));
    if let Some(mut m) = monitor {
        let latest = latest.clone();
        std::thread::spawn(move || loop {
            let result = m.poll();
            *latest.lock().expect("audio-monitor poll state poisoned") = result;
            std::thread::sleep(POLL_INTERVAL);
        });
    }

    let state = Arc::new(SharedState {
        allow_origin,
        watch: Mutex::new(initial_watch),
        latest,
        supported,
        unsupported_reason,
    });

    let app = Router::new()
        .route("/levels", get(upgrade))
        .with_state(state);
    let addr: SocketAddr = ([127, 0, 0, 1], args.port).into();
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let bound = listener.local_addr()?;
    tracing::info!(port = bound.port(), "shadowcat audio-monitor listening");
    println!(
        "shadowcat audio-monitor listening on 127.0.0.1:{}",
        bound.port()
    );
    axum::serve(listener, app).await?;
    Ok(())
}

/// Origin-gated upgrade handler: refuses the upgrade outright (never reaching the `hello`
/// frame) when the request's `Origin` header is absent or not in `allow_origin`: an
/// unlisted origin is closed before the hello frame.
async fn upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<SharedState>>,
    headers: HeaderMap,
) -> Response {
    let origin = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok());
    let allowed = origin.is_some_and(|o| state.allow_origin.iter().any(|a| a == o));
    if !allowed {
        return StatusCode::FORBIDDEN.into_response();
    }
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Per-connection loop: sends `hello` once, then a `levels` frame every `POLL_INTERVAL`,
/// concurrently reading `watch` frames the client sends (each replaces the live watch list).
async fn handle_socket(mut socket: WebSocket, state: Arc<SharedState>) {
    let hello = OutgoingFrame::Hello {
        os: std::env::consts::OS,
        supported: state.supported,
        reason: state.unsupported_reason.clone(),
    };
    if send_frame(&mut socket, &hello).await.is_err() {
        return;
    }

    let mut interval = tokio::time::interval(POLL_INTERVAL);
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let raw = state.latest.lock().expect("audio-monitor poll state poisoned").clone();
                let sessions = match raw {
                    Ok(raw) => filter_for_watch_list(raw, &state.watch.lock().expect("audio-monitor watch list poisoned")),
                    Err(_) => Vec::new(),
                };
                let frame = OutgoingFrame::Levels {
                    sessions: sessions.into_iter().map(WireSessionLevel::from).collect(),
                };
                if send_frame(&mut socket, &frame).await.is_err() {
                    return;
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(IncomingFrame::Watch { names }) = serde_json::from_str(text.as_str()) {
                            *state.watch.lock().expect("audio-monitor watch list poisoned") = names;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => return,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => return,
                }
            }
        }
    }
}

/// Serializes and sends one JSON text frame, mapping any send failure to `Err(())` so the
/// caller can end the connection loop without inspecting axum's error type.
async fn send_frame(socket: &mut WebSocket, frame: &OutgoingFrame) -> Result<(), ()> {
    let text = serde_json::to_string(frame).map_err(|_| ())?;
    socket
        .send(Message::Text(text.into()))
        .await
        .map_err(|_| ())
}

#[cfg(test)]
mod tests;
