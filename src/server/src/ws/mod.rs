//! Realtime WebSocket event bus: per-world rooms, sequenced broadcasts,
//! ring-buffer + log-backed resync, a server time source, and telemetry.

pub mod conn;
pub mod protocol;
pub mod room;
pub mod time;
