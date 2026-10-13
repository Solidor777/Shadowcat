//! Server-side audio consists of the pure transport-state engine (`state::apply`) and the
//! `AudioTransport` handler (`transport`).

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two crate-level deny attributes this module declares.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

pub mod state;
pub mod transport;
