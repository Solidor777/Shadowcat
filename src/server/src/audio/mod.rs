//! Server-side audio is a pure transport-state engine plus the `AudioTransport` handler.
//!
//! The `state` module holds the engine's entry point, `state::apply`. The `transport` module holds
//! the handler and `on_active_scene`.

// The two deny attributes below reject any item in this module that lacks a doc comment.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

pub mod state;
pub mod transport;
