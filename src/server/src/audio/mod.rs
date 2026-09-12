//! Server-side audio: the pure transport-state engine (`state::apply`) and the
//! `AudioTransport`/`AudioListenAs` WS handlers (`transport`).

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two crate-level deny attributes this module declares.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

pub mod state;
