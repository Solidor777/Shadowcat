//! Realtime WebSocket event bus: per-world rooms, sequenced broadcasts,
//! ring-buffer + log-backed resync, a server time source, and telemetry.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use uuid::Uuid;

pub mod bounded_sink;
pub mod conn;
pub(crate) mod move_clip;
pub mod protocol;
pub mod room;
pub(crate) mod ruler;
#[cfg(test)]
pub(crate) mod test_support;
pub mod time;
pub mod tunnel_gate;
pub mod vfx;

pub use room::RoomRegistry;

/// Per-user sliding-window ping budget on shared state. Unlike the per-connection
/// window it replaces, a user's N concurrent sockets share one budget — a stronger
/// abuse backstop. 60 s window; over-budget pings drop silently at the call site.
///
/// # Examples
///
/// ```
/// use shadowcat::ws::PingRateLimiter;
///
/// let limiter = PingRateLimiter::default();
/// assert!(limiter.check(uuid::Uuid::nil(), 0, 1));
/// assert!(!limiter.check(uuid::Uuid::nil(), 0, 1)); // budget of 1 already spent
/// ```
#[derive(Default)]
pub struct PingRateLimiter {
    /// Per-user hit timestamps within the sliding window.
    hits: Mutex<HashMap<Uuid, Vec<i64>>>,
}

impl PingRateLimiter {
    /// An empty limiter (one per `WsState`; budget shared per user).
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::ws::PingRateLimiter;
    ///
    /// let limiter = PingRateLimiter::new();
    /// let user = uuid::Uuid::new_v4();
    /// assert!(limiter.check(user, 0, 5)); // first hit is within budget
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a ping for `user` at `now_ms`, returning whether it is within the
    /// `per_min` budget over the trailing 60 s window.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::ws::PingRateLimiter;
    ///
    /// let limiter = PingRateLimiter::new();
    /// let user = uuid::Uuid::new_v4();
    /// assert!(limiter.check(user, 0, 2));
    /// assert!(limiter.check(user, 1_000, 2));
    /// assert!(!limiter.check(user, 2_000, 2)); // third hit within the 60s window
    /// ```
    pub fn check(&self, user: Uuid, now_ms: i64, per_min: usize) -> bool {
        let mut g = self.hits.lock().expect("ping rate-limiter mutex poisoned");
        let v = g.entry(user).or_default();
        v.retain(|&t| t > now_ms - 60_000);
        if v.len() >= per_min {
            return false;
        }
        v.push(now_ms);
        true
    }
}

/// The per-user, per-trailing-60s budget every caller of `WsState::message_rate` spends
/// against: the chat handlers (`SendMessage`/`EditMessage`/`DeleteMessage`/`RecalcRoll`) and
/// `combat::handle_combat_intent`. INVARIANT: declared ONCE, beside the limiter it governs.
/// Those callers share a single `PingRateLimiter` instance and therefore a single per-user hit
/// list, so a second constant naming the same budget would be a forked decision — raising one
/// copy silently changes the other's effective behaviour against the very same counter, with
/// nothing failing to report it.
pub(crate) const MESSAGE_RATE_PER_MIN: usize = 30;

/// The per-user, per-trailing-60s budget the `ClientMsg::AudioTransport` handler spends
/// against, from its OWN bucket (`WsState::audio_rate` — a transport flood must not starve
/// chat/pings/emotes or vice versa). Declared once, beside the limiter it governs, for the
/// same no-fork reason `MESSAGE_RATE_PER_MIN` states above.
pub(crate) const AUDIO_RATE_PER_MIN: usize = 30;

/// Whether `ctx` may broadcast an out-of-band frame (`ScenePing`, `PlayVfx`, `RulerUpdate`)
/// onto `scene`: the document must exist, be a `scene`, belong to THIS world, and grant the
/// sender `cap::READ`. Admits a token-less spectator (READ on the scene is enough — deliberately
/// weaker than `handle_pathfind`'s controls-a-token gate, which selects server state; a relay
/// selects none). Denial is a SILENT drop at the call site: any error frame or behavior split
/// would leak scene existence to a non-reader. ONE definition shared by every scene-addressed
/// relay so their admission cannot drift; a relay adding a role restriction (`vfx::vfx_permitted`)
/// applies it before calling this.
pub(crate) async fn scene_broadcast_permitted(
    scene: Uuid,
    ctx: &crate::data::membership::PermissionContext,
    world_id: Uuid,
    repo: &dyn crate::data::repository::Repository,
) -> bool {
    let Ok(Some(doc)) = repo.get_document(scene).await else {
        return false;
    };
    if doc.doc_type != "scene" {
        return false;
    }
    // World scope: a scene doc from another world is refused even for a member of both (the
    // relay stamps THIS room).
    if crate::data::document::world_of(&doc) != Some(world_id) {
        return false;
    }
    let Ok(defaults) = repo.world_cap_defaults(world_id).await else {
        return false;
    };
    // A scene doc never carries an actor link, so the no-join resolution is
    // exact — and fails closed if a non-scene doc ever reached here.
    let access = crate::data::permission::resolve_access_world(
        ctx.user_id,
        ctx.world_role,
        &doc,
        &defaults.grants_for(&doc.doc_type),
        crate::data::permission::effective_owner(&doc, None),
    );
    access.has(crate::data::permission::cap::READ)
}

/// Realtime state shared in `AppState`. A thin handle today; the seam for future
/// bus internals (actor pool / external broker) without touching callers.
///
/// # Examples
///
/// ```
/// use shadowcat::ws::WsState;
///
/// let state = WsState::default();
/// assert!(state.rooms.get(uuid::Uuid::nil()).is_none());
/// ```
#[derive(Clone)]
pub struct WsState {
    /// The world -> room fan-out registry.
    pub rooms: Arc<RoomRegistry>,
    /// Per-user ping budget (shared across a user's connections).
    pub ping_rate: Arc<PingRateLimiter>,
    /// Per-user emote budget (shared across a user's connections); a SEPARATE bucket
    /// from `ping_rate` so emote spam cannot starve pings and vice versa.
    pub emote_rate: Arc<PingRateLimiter>,
    /// Per-user flood budget for every handler `MESSAGE_RATE_PER_MIN` governs (shared across a
    /// user's connections).
    pub message_rate: Arc<PingRateLimiter>,
    /// Per-user audio-transport budget (shared across a user's connections); a
    /// SEPARATE bucket from ping/emote/message so transport spam cannot starve them or vice versa.
    pub audio_rate: Arc<PingRateLimiter>,
    /// Per-user VFX one-shot budget (shared across a user's connections); a SEPARATE bucket
    /// from `ping_rate`/`emote_rate`/`message_rate` so a VFX spam burst cannot starve any other
    /// relay. Charged by BOTH entry paths — the raw `ClientMsg::PlayVfx` arm in `ws::conn` and
    /// the `/fx` chat command (`chat::fx::run_fx`) — at the same inline 30/min/user budget, so
    /// neither front door buys more plays than the other.
    pub vfx_rate: Arc<PingRateLimiter>,
    /// Per-user shared-ruler budget (shared across a user's connections); a SEPARATE bucket from
    /// `ping_rate`/`emote_rate`/`message_rate`/`vfx_rate` so a live ruler drag cannot starve any
    /// other relay and vice versa. Charged at `ruler::RULER_RATE_PER_MIN` by the
    /// `ClientMsg::RulerUpdate` arm in `ws::conn`.
    pub ruler_rate: Arc<PingRateLimiter>,
    /// The link-preview SSRF-guarded fetch client, built ONCE via
    /// `chat::build_link_preview_client()` (the no-flag production
    /// constructor — never the test-only loopback-permitting one) and
    /// shared across every connection/world; a preview's target is
    /// world-independent.
    pub link_preview_client: Arc<reqwest::Client>,
    /// In-memory per-URL preview cache, shared across every
    /// connection/world (a URL's preview is world-independent).
    pub link_preview_cache: Arc<crate::chat::LinkPreviewCache>,
    /// Per-user outbound-fetch budget for the link-preview fetcher (shared
    /// across a user's connections, same shape as `message_rate`).
    pub preview_rate: Arc<crate::chat::PreviewRateLimiter>,
    /// In-memory installed-module scan cache, shared across every
    /// connection/world (the installed-module set is server-wide, not
    /// per-world). See `crate::modules::ModuleScanCache`'s own doc for the
    /// invalidation rule.
    pub module_scan_cache: Arc<crate::modules::ModuleScanCache>,
    /// The server-wide graceful-shutdown sender; kept alive here so the paired `shutdown`
    /// receiver's channel never closes on its own — an unexpectedly closed `watch` channel
    /// would make every connection's `shutdown.changed()` select arm resolve immediately (a
    /// `Recv` error, treated the same as a real shutdown signal — see `egress_loop`'s own
    /// doc), closing every socket as though a real shutdown had been requested.
    /// `trigger_shutdown` is the only place this sends.
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    /// Cloned by `ws::conn::handle_socket` into each connection's `EgressConnState` so its
    /// egress loop can select on it and exit gracefully (send a Close frame, then return) once
    /// `trigger_shutdown` fires — see `http::serve_with`'s drain hook.
    pub shutdown: tokio::sync::watch::Receiver<bool>,
    /// Count of currently-live WebSocket connection tasks. `axum::WebSocketUpgrade::on_upgrade`
    /// spawns its callback as a task DETACHED from the request future that produced the
    /// handshake response, and over HTTP/1.1 the hyper connection future completes at the
    /// handshake itself, so `http::conn_serve::serve_router` does not wait for such a session to
    /// finish closing (an HTTP/2 session is a stream of its connection, so the connection does
    /// outlive it). `connection_guard`
    /// increments this on connect and decrements it (waking `drain_notify`) when the guard
    /// drops at the end of `ws::conn::handle_socket`; `wait_for_drain` is what actually blocks a
    /// caller until every such task has exited.
    live_connections: Arc<AtomicUsize>,
    /// Woken by a `ConnectionGuard::drop` whenever `live_connections` reaches zero (see
    /// `wait_for_drain`).
    drain_notify: Arc<tokio::sync::Notify>,
    /// Count of WebSocket sends currently pending across every live session: incremented when a
    /// `BoundedSink` built by `bounded_sink` first returns `Pending` for a send, decremented when
    /// that send completes, fails or its sink drops. Read by `pending_sends`.
    pending_sends: Arc<AtomicUsize>,
}

/// RAII handle held for the lifetime of one WebSocket connection task
/// (`ws::conn::handle_socket`), so `WsState::live_connections` accurately counts live sessions
/// regardless of which `return`/`break`/panic path the task exits through. Never constructed
/// directly outside this module — obtain one via `WsState::connection_guard`.
///
/// # Examples
///
/// ```
/// use shadowcat::ws::WsState;
///
/// let state = WsState::new();
/// assert_eq!(state.live_connection_count(), 0);
/// let guard = state.connection_guard();
/// assert_eq!(state.live_connection_count(), 1);
/// drop(guard);
/// assert_eq!(state.live_connection_count(), 0);
/// ```
pub struct ConnectionGuard {
    /// Back-reference to the counter/notifier this guard decrements/wakes on drop.
    live_connections: Arc<AtomicUsize>,
    /// See `ConnectionGuard::live_connections`.
    drain_notify: Arc<tokio::sync::Notify>,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        // `fetch_sub` returns the PRE-decrement value; a post-decrement value of zero is what
        // `wait_for_drain` is waiting to observe.
        if self.live_connections.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.drain_notify.notify_waiters();
        }
    }
}

impl WsState {
    /// Fresh realtime state (empty registry + limiters).
    ///
    /// # Examples
    ///
    /// ```
    /// let ws = shadowcat::ws::WsState::new();
    /// assert!(ws.rooms.get(uuid::Uuid::nil()).is_none());
    /// ```
    pub fn new() -> Self {
        // `rooms` owns the canonical VFX-budget limiter; `vfx_rate` clones the SAME instance
        // (never a fresh one) so the raw `ClientMsg::PlayVfx` frame, `/fx`, and a `Room`-internal
        // trigger-fired VFX (`fire_region_triggers`'s Teleport arm) share one bucket.
        let rooms = Arc::new(RoomRegistry::new());
        let vfx_rate = rooms.vfx_rate();
        let (shutdown_tx, shutdown) = tokio::sync::watch::channel(false);
        Self {
            rooms,
            ping_rate: Arc::new(PingRateLimiter::new()),
            emote_rate: Arc::new(PingRateLimiter::new()),
            message_rate: Arc::new(PingRateLimiter::new()),
            audio_rate: Arc::new(PingRateLimiter::new()),
            vfx_rate,
            ruler_rate: Arc::new(PingRateLimiter::new()),
            link_preview_client: Arc::new(crate::chat::build_link_preview_client()),
            link_preview_cache: Arc::new(crate::chat::LinkPreviewCache::new()),
            preview_rate: Arc::new(crate::chat::PreviewRateLimiter::new()),
            module_scan_cache: Arc::new(crate::modules::ModuleScanCache::new()),
            shutdown_tx,
            shutdown,
            live_connections: Arc::new(AtomicUsize::new(0)),
            drain_notify: Arc::new(tokio::sync::Notify::new()),
            pending_sends: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// A `WsState` whose rooms use a custom broadcast ring capacity. Test-only:
    /// shrinking the ring forces the lag-driven resync path deterministically.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::ws::WsState;
    ///
    /// let state = WsState::with_broadcast_capacity(4);
    /// assert!(state.rooms.get(uuid::Uuid::nil()).is_none());
    /// ```
    pub fn with_broadcast_capacity(capacity: usize) -> Self {
        // See `WsState::new`'s doc: `vfx_rate` clones `rooms`'s own canonical limiter.
        let rooms = Arc::new(RoomRegistry::with_capacity(capacity));
        let vfx_rate = rooms.vfx_rate();
        let (shutdown_tx, shutdown) = tokio::sync::watch::channel(false);
        Self {
            rooms,
            ping_rate: Arc::new(PingRateLimiter::new()),
            emote_rate: Arc::new(PingRateLimiter::new()),
            message_rate: Arc::new(PingRateLimiter::new()),
            audio_rate: Arc::new(PingRateLimiter::new()),
            vfx_rate,
            ruler_rate: Arc::new(PingRateLimiter::new()),
            link_preview_client: Arc::new(crate::chat::build_link_preview_client()),
            link_preview_cache: Arc::new(crate::chat::LinkPreviewCache::new()),
            preview_rate: Arc::new(crate::chat::PreviewRateLimiter::new()),
            module_scan_cache: Arc::new(crate::modules::ModuleScanCache::new()),
            shutdown_tx,
            shutdown,
            live_connections: Arc::new(AtomicUsize::new(0)),
            drain_notify: Arc::new(tokio::sync::Notify::new()),
            pending_sends: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Signal every live connection's `egress_loop` to exit gracefully: send a Close frame and
    /// return. This alone does NOT guarantee the connection's task has finished exiting by the
    /// time it returns — see `wait_for_drain` for the mechanism that actually waits for that.
    /// Called by `http::serve_with` as `serve_router`'s drain hook, for a shutdown request and for
    /// a serve loop whose connection source ended alike.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::ws::WsState;
    ///
    /// let state = WsState::new();
    /// state.trigger_shutdown();
    /// assert!(*state.shutdown.borrow());
    /// ```
    pub fn trigger_shutdown(&self) {
        // A dropped receiver (no live connections) makes `send` return `Err`, which is not an
        // error here — there is nothing left to notify.
        let _ = self.shutdown_tx.send(true);
    }

    /// Register one live WebSocket connection task, returning an RAII guard that decrements the
    /// count (and wakes any `wait_for_drain` caller once it reaches zero) when dropped. Called
    /// once at the top of `ws::conn::handle_socket`, held for the task's whole lifetime.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::ws::WsState;
    ///
    /// let state = WsState::new();
    /// assert_eq!(state.live_connection_count(), 0);
    /// let guard = state.connection_guard();
    /// assert_eq!(state.live_connection_count(), 1);
    /// drop(guard);
    /// assert_eq!(state.live_connection_count(), 0);
    /// ```
    pub fn connection_guard(&self) -> ConnectionGuard {
        self.live_connections.fetch_add(1, Ordering::AcqRel);
        ConnectionGuard {
            live_connections: self.live_connections.clone(),
            drain_notify: self.drain_notify.clone(),
        }
    }

    /// The current count of live WebSocket connection tasks (see `connection_guard`).
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::ws::WsState;
    ///
    /// let state = WsState::new();
    /// assert_eq!(state.live_connection_count(), 0);
    /// ```
    pub fn live_connection_count(&self) -> usize {
        self.live_connections.load(Ordering::Acquire)
    }

    /// The one constructor of the stall-bounded sink every session's socket is wrapped in: the
    /// window is `Config::egress_window` (the value `ConnLimits::for_config` gives the response
    /// bounds), the shutdown signal is this state's, and the sink counts its pending sends into
    /// `pending_sends`. `tunnel` is the session's own HTTP/2 stream, when it has one: the sink
    /// then also stops accepting frames while the session is more than one send buffer ahead of
    /// the wire (`tunnel_gate`), which is what makes a stalled tunnel's send pending at all. The
    /// time bound is on one pending flush, not on progress through a frame: a frame of F bytes
    /// needs at least 8F / window bits per second to complete, and the socket the sink wraps
    /// (`axum::extract::ws::WebSocket`) exposes no byte counter to measure progress with.
    ///
    /// # Examples
    ///
    /// ```
    /// use axum::extract::ws::Message;
    /// use shadowcat::config::Config;
    /// use shadowcat::ws::WsState;
    ///
    /// let state = WsState::new();
    /// let _sink = state.bounded_sink(
    ///     futures_util::sink::drain::<Message>(),
    ///     &Config::default(),
    ///     None,
    /// );
    /// assert_eq!(state.pending_sends(), 0);
    /// ```
    pub fn bounded_sink<S>(
        &self,
        socket: S,
        config: &crate::config::Config,
        tunnel: Option<crate::http::conn_serve::TunnelProbe>,
    ) -> bounded_sink::BoundedSink<tunnel_gate::TunnelGate<S>> {
        bounded_sink::BoundedSink::for_config(
            tunnel_gate::TunnelGate::new(socket, tunnel),
            self.shutdown.clone(),
            config,
        )
        .with_pending_gauge(self.pending_sends.clone())
    }

    /// The number of WebSocket sends pending right now across every live session: a nonzero
    /// value is a peer that is not taking a frame. `http::serve_with`'s drain warning reports it
    /// beside `live_connection_count`.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::ws::WsState;
    ///
    /// assert_eq!(WsState::new().pending_sends(), 0);
    /// ```
    pub fn pending_sends(&self) -> usize {
        self.pending_sends.load(Ordering::Acquire)
    }

    /// Block until every live WebSocket connection task has exited (its `ConnectionGuard`
    /// dropped), or `timeout` elapses — whichever comes first. Returns `true` if every
    /// connection drained within the timeout, `false` on timeout (some connections were still
    /// live). This is the mechanism that closes the gap `trigger_shutdown`'s own doc names:
    /// `http::conn_serve::serve_router` does not itself wait for an HTTP/1.1 WebSocket session's
    /// task to finish, so a caller (`http::serve_with`, after `serve_router` returns) must call
    /// this explicitly before letting the process (and its tokio runtime) exit, or an in-flight
    /// `Message::Close` send can be dropped mid-write.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use shadowcat::ws::WsState;
    ///
    /// # #[tokio::main] async fn main() {
    /// let state = WsState::new();
    /// // No live connections: returns immediately, successfully.
    /// assert!(state.wait_for_drain(Duration::from_millis(50)).await);
    /// # }
    /// ```
    pub async fn wait_for_drain(&self, timeout: std::time::Duration) -> bool {
        if self.live_connection_count() == 0 {
            return true;
        }
        // `notified()` must be constructed BEFORE the re-check below to avoid the classic
        // notify-before-wait race: a `ConnectionGuard::drop` between the count check above and
        // this call would otherwise call `notify_waiters()` with nobody registered yet to
        // receive it. `Notify::notified()` returns a future that, once polled, registers itself
        // to be woken by any FUTURE `notify_waiters()` call.
        let notified = self.drain_notify.notified();
        if self.live_connection_count() == 0 {
            return true;
        }
        tokio::time::timeout(timeout, notified).await.is_ok() && self.live_connection_count() == 0
    }
}

impl Default for WsState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
