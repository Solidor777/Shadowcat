//! The Shadowcat server library: authoritative state, persistence, permissions,
//! and realtime sync for the self-hosted virtual tabletop, compiled into one
//! binary with the web client embedded (`rust-embed` — `dist/` must exist at
//! compile time). Clients send intents; this crate validates, applies, and
//! broadcasts per-recipient-filtered events. Server-side code never executes
//! third-party module code.

/// Accounts, sessions, password hashing, and the first-run setup flow.
pub mod auth;
pub mod backup;
pub mod chat;
/// CLI/env/TOML config layering (`Cli` > `SHADOWCAT_*` > TOML > default).
pub mod config;
/// Documents, permissions, engine-band validation, search, and SQLite persistence.
pub mod data;
/// SQLite pool bootstrap (deliberately single-connection).
pub mod db;
pub mod dice;
/// Liveness probe endpoint plumbing.
pub mod health;
/// Axum router: REST surface, asset serving, module serving, embedded client.
pub mod http;
/// Installed community-module discovery + the engine-compat semver gate.
pub mod modules;
pub mod scene;
/// Per-world export/import: builds/reads the `.tar` bundle format (see
/// `data::world_bundle` for the row/manifest types, `http::world_bundle` for
/// the HTTP routes).
pub mod world_bundle;
pub mod ws;
