//! Realtime WebSocket event bus: per-world rooms, sequenced broadcasts,
//! ring-buffer + log-backed resync, a server time source, and telemetry.

use std::sync::Arc;

pub mod conn;
pub mod protocol;
pub mod room;
pub mod time;

pub use room::RoomRegistry;

/// Realtime state shared in `AppState`. A thin handle today; the seam for future
/// bus internals (actor pool / external broker) without touching callers.
#[derive(Clone)]
pub struct WsState {
    pub rooms: Arc<RoomRegistry>,
}

impl WsState {
    pub fn new() -> Self {
        Self {
            rooms: Arc::new(RoomRegistry::new()),
        }
    }
}

impl Default for WsState {
    fn default() -> Self {
        Self::new()
    }
}
