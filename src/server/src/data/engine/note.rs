//! Engine body for a rich-text note (`NOTE_DOC_TYPE`): author markdown in
//! `source`, rendered server-side at ingress into a sanitized `body` through
//! the same chat sanitizer boundary chat messages use
//! (`chat::body::compose_static` under the fixed `chat::NOTE_CONTENT_POLICY`).
//! Envelope `name` is the note's title; this module owns only the stored
//! shape and its ingress validation -- the tree/containment rules live in
//! `data::validation::validate_containment` and
//! `data::sqlite::notes::check_note_parent`.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two crate-level deny attributes this module declares.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::chat::{compose_static, Segment, NOTE_CONTENT_POLICY};

/// Doc_type for a rich-text note: a document tree (`parent_id` names another
/// note in the same scope, see `check_note_parent`), never embedded (see
/// `data::validation::validate_containment`'s `note` arm).
pub const NOTE_DOC_TYPE: &str = "note";

/// Cap on `NoteEngine.source`, in characters.
pub const MAX_NOTE_SOURCE_CHARS: usize = 65_536;
/// Cap on non-text `[[...]]` spans `NoteEngine::derive_body` extracts from one
/// note's `source` -- a journal page is far longer than one chat message, so
/// this is deliberately larger than `chat::rolls::MAX_INLINE_ROLLS`.
pub const MAX_NOTE_SPANS: usize = 64;

/// The engine body of a rich-text note. Envelope `name` is the note's title.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct NoteEngine {
    /// The author's markdown (≤ `MAX_NOTE_SOURCE_CHARS`). Never rendered as
    /// HTML by any client -- only `body` (below) is ever rendered.
    pub source: String,
    /// SERVER-DERIVED from `source` by `normalize_engine`'s `"note"` arm on
    /// every Create/Update post-image. Whatever a client sends here is
    /// discarded and overwritten; the client's optimistic mirror shows
    /// `source` until the authoritative echo arrives with the derived body.
    #[serde(default)]
    #[ts(type = "unknown[]")]
    pub body: Vec<Segment>,
    /// Sibling ordering under one parent (client-chosen; ties broken by
    /// `created_at`). Has no meaning for a root-level note.
    #[serde(default)]
    pub sort: i64,
}

impl NoteEngine {
    /// Validates `source`'s length. `body` is never validated here -- it is
    /// unconditionally overwritten by `derive_body` before a note is ever
    /// stored, so a client-supplied `body` (however shaped) never survives
    /// ingress.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::engine::note::NoteEngine;
    ///
    /// let note = NoteEngine { source: "hello".into(), body: vec![], sort: 0 };
    /// assert!(note.validate().is_ok());
    /// ```
    pub fn validate(&self) -> Result<(), String> {
        if self.source.chars().count() > MAX_NOTE_SOURCE_CHARS {
            return Err(format!(
                "note source exceeds {MAX_NOTE_SOURCE_CHARS} characters"
            ));
        }
        Ok(())
    }

    /// Derives `body` from `source` through the shared chat body composer,
    /// under the fixed `NOTE_CONTENT_POLICY` (never the world's own
    /// `chat-settings` policy -- a note is a journal page, not a chat
    /// message). Deterministic and I/O-free, so it runs identically under
    /// `apply_intent` and `apply_command` replay. A scan/parse failure
    /// (a malformed span, an over-cap span count, an unparseable inline
    /// formula) maps to the composer's `RollError`'s player-presentable
    /// `Display` text, surfaced to the client through `DataError::BadEngine`
    /// on the rejected intent.
    pub fn derive_body(&mut self) -> Result<(), String> {
        self.body = compose_static(&self.source, &NOTE_CONTENT_POLICY, MAX_NOTE_SPANS)
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
