//! The Shadowcat server library: authoritative state, persistence, permissions,
//! and realtime sync for the self-hosted virtual tabletop, compiled into one
//! binary with the web client embedded (`rust-embed` — `dist/` must exist at
//! compile time). Clients send intents; this crate validates, applies, and
//! broadcasts per-recipient-filtered events. Server-side code never executes
//! third-party module code.

/// Server-side audio: the pure transport-state engine (`audio::state::apply`)
/// and the GM-only `AudioTransport` handler (`audio::transport`).
pub mod audio;
/// Accounts, sessions, password hashing, and the first-run setup flow.
pub mod auth;
pub mod backup;
pub mod chat;
/// The server-owned combat clock: snapshot loading, pure transitions, and
/// effect-lifecycle helpers.
pub mod combat;
/// CLI/env/TOML config layering (`Cli` > `SHADOWCAT_*` > TOML > default).
pub mod config;
/// Documents, permissions, engine-band validation, search, and SQLite persistence.
pub mod data;
/// SQLite pool bootstrap (deliberately single-connection).
pub mod db;
pub mod dice;
/// The engine's formula language (server twin of `@shadowcat/formula`).
pub mod formula;
/// Liveness probe endpoint plumbing.
pub mod health;
/// Axum router: REST surface, asset serving, module serving, embedded client.
pub mod http;
/// The server-owned template merge engine: the exact behavioural twin of the
/// client `@shadowcat/core` 3-way merge, corpus-pinned against it.
pub mod merge;
/// Installed community-module discovery + the engine-compat semver gate.
pub mod modules;
pub mod scene;
/// Server-side rollable-table draws (`ClientMsg::DrawTable` -> a posted
/// `Segment::TableDraw` chat message).
pub mod tables;
/// Per-world export/import: builds/reads the `.tar` bundle format (see
/// `data::world_bundle` for the row/manifest types, `http::world_bundle` for
/// the HTTP routes).
pub mod world_bundle;
pub mod ws;
