//! Server-side audio is a pure transport-state engine plus the `AudioTransport` handler.
//!
//! The `state` module holds the engine's entry point, `state::apply`. The `transport` module holds
//! the handler and `on_active_scene`.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two crate-level deny attributes this module declares.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

pub mod state;
pub mod transport;
