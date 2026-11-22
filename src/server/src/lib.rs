//! The Shadowcat server library: authoritative state, persistence, permissions, and realtime sync
//! for the self-hosted virtual tabletop.
//!
//! The crate is compiled into one binary with the web client embedded (`rust-embed` — `dist/` must
//! exist at compile time). Clients send intents; this crate validates, applies, and broadcasts
//! per-recipient-filtered events. Server-side code never executes third-party module code, except
//! opted-in sandboxed validators (`sandbox`) running inside a fuel/memory-limited `wasmi`
//! interpreter with no host imports beyond a rate-limited debug log.

pub mod audio;
pub mod audio_monitor;
pub mod auth;
pub mod backup;
pub mod chat;
pub mod combat;
pub mod config;
pub mod data;
pub mod db;
pub mod dice;
pub mod fd_budget;
pub mod formula;
pub mod health;
pub mod http;
pub mod merge;
pub mod modules;
pub mod portmap;
pub mod sandbox;
pub mod scene;
pub mod tables;
pub mod tls;
pub mod trash;
pub mod world_bundle;
pub mod ws;

#[cfg(test)]
pub(crate) mod test_ids;
