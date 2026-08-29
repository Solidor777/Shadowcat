//! Field-path `Operation`/`FieldChange` construction shared by every
//! transition: reading a document's own current value as the OCC pre-image,
//! the same convention `data::command`'s doc comment on `FieldChange`
//! establishes for the whole repository.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde_json::Value;
use uuid::Uuid;

use crate::data::command::{FieldChange, Operation};
use crate::data::document::Document;
use crate::data::DataError;

use super::CombatError;

/// Builds a `FieldChange` writing `new` at `pointer` on `doc`. The OCC
/// pre-image is read from `doc`'s OWN current serialized value at `pointer`
/// (`Value::Null` when the pointer is absent) — never guessed or copied from
/// a caller's separately-tracked belief about the old value, so a transition
/// can never construct a stale pre-image against a document it has already
/// mutated earlier in the same batch (`transition::Working` re-derives every
/// later `set_engine` call against its own progressively-applied copy).
pub fn set_engine(doc: &Document, pointer: &str, new: Value) -> Result<FieldChange, CombatError> {
    let doc_value = serde_json::to_value(doc).map_err(DataError::from)?;
    let old = doc_value.pointer(pointer).cloned().unwrap_or(Value::Null);
    Ok(FieldChange {
        path: pointer.to_string(),
        old,
        new,
        remove: false,
    })
}

/// Wraps `changes` into one `Update` operation against `doc_id`.
pub fn update(doc_id: Uuid, changes: Vec<FieldChange>) -> Operation {
    Operation::Update { doc_id, changes }
}

/// A `FieldChange` replacing a document's WHOLE `/engine` band, pre-imaged
/// against the document's current engine — used by history restore (a later
/// task) to snap an entire engine body back to a `TurnRecord`'s stored copy
/// in one write instead of per-field diffing.
pub fn whole_engine_replace(doc: &Document, new_engine: Value) -> FieldChange {
    FieldChange {
        path: "/engine".to_string(),
        old: doc.engine.clone().unwrap_or(Value::Null),
        new: new_engine,
        remove: false,
    }
}
