//! Field-path `Operation`/`FieldChange` construction shared by every
//! transition: reading a document's own current value as the OCC pre-image,
//! the same convention `data::command`'s doc comment on `FieldChange`
//! establishes for the whole repository.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde_json::Value;

use crate::data::command::FieldChange;
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
///
/// # Examples
///
/// ```
/// use shadowcat::combat::ops::set_engine;
/// use shadowcat::data::document::{Document, PermissionSet, Scope};
/// use uuid::Uuid;
///
/// let doc = Document {
///     id: Uuid::new_v4(),
///     scope: Scope::World { world_id: Uuid::new_v4() },
///     doc_type: "combat".to_string(),
///     schema_version: 1,
///     name: None,
///     source: None,
///     base: None,
///     owner: None,
///     permissions: PermissionSet::default(),
///     embedded: Default::default(),
///     parent_id: None,
///     engine: Some(serde_json::json!({ "round": 1 })),
///     system: serde_json::json!({}),
///     created_at: 0,
///     updated_at: 0,
/// };
/// let change = set_engine(&doc, "/engine/round", serde_json::json!(2)).unwrap();
/// assert_eq!(change.old, serde_json::json!(1));
/// assert_eq!(change.new, serde_json::json!(2));
/// ```
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

/// A `FieldChange` replacing a document's WHOLE `/engine` band, pre-imaged
/// against the document's current engine — one write instead of per-field
/// diffing. Callers: `history::append_record`, `history::fast_forward` and
/// `transition::rewind`, each rewriting a `combat-history` document's
/// `records`/`cursor` wholesale. `history::restore` does NOT use it: it
/// writes a combatant's `/engine` through `set_engine` against the LIVE
/// document, since the value it writes comes from a record rather than from
/// the document's own current state.
///
/// # Examples
///
/// ```
/// use shadowcat::combat::ops::whole_engine_replace;
/// use shadowcat::data::document::{Document, PermissionSet, Scope};
/// use uuid::Uuid;
///
/// let doc = Document {
///     id: Uuid::new_v4(),
///     scope: Scope::World { world_id: Uuid::new_v4() },
///     doc_type: "combat-history".to_string(),
///     schema_version: 1,
///     name: None,
///     source: None,
///     base: None,
///     owner: None,
///     permissions: PermissionSet::default(),
///     embedded: Default::default(),
///     parent_id: Some(Uuid::new_v4()),
///     engine: Some(serde_json::json!({ "records": [], "cursor": 0 })),
///     system: serde_json::json!({}),
///     created_at: 0,
///     updated_at: 0,
/// };
/// let new_engine = serde_json::json!({ "records": [], "cursor": 1 });
/// let change = whole_engine_replace(&doc, new_engine.clone());
/// assert_eq!(change.path, "/engine");
/// assert_eq!(change.old, serde_json::json!({ "records": [], "cursor": 0 }));
/// assert_eq!(change.new, new_engine);
/// ```
pub fn whole_engine_replace(doc: &Document, new_engine: Value) -> FieldChange {
    FieldChange {
        path: "/engine".to_string(),
        old: doc.engine.clone().unwrap_or(Value::Null),
        new: new_engine,
        remove: false,
    }
}
