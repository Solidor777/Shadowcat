// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;
use uuid::Uuid;

use crate::data::document::Document;
use crate::data::DataError;

/// One field-level change with its pre-image.
///
/// A `set` change (`remove: false`) writes `new` at `path`; its OCC pre-image is
/// `old`. A `remove` change (`remove: true`) deletes the object key at `path`,
/// making it genuinely absent (`null` != absent); `new` is unused (conventionally
/// `Null`) and `old` remains the OCC pre-image of the value being removed.
///
/// Self-inverting for sets. A `remove` inverts to a re-inserting `set` (correct
/// one-way undo: the removed slot is absent afterward, so the inverse's pre-image
/// is `Null`); it is NOT byte-identical under double `invert` because `old: Null`
/// cannot distinguish "was absent" from "was explicitly null", so a set-creating-a-
/// key is not re-derived as a removal. `invert` has no live caller (undo/redo is
/// not wired), so this asymmetry is inert today.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct FieldChange {
    /// JSON pointer to the field, e.g. `/system/hp`.
    pub path: String,
    /// OCC pre-image: the raw currently-stored value (`values_semantically_eq`
    /// compares it at apply time; a mismatch rejects the intent).
    #[ts(type = "unknown")]
    pub old: Value,
    /// The value to write (unused when `remove` is true).
    #[ts(type = "unknown")]
    pub new: Value,
    /// When true, REMOVE the object key at `path` instead of setting `new`.
    /// Object keys only — array-index removal is rejected (see `remove_pointer`).
    /// Omitted on the wire when false (`#[serde(default)]` on ingest); the
    /// client Zod mirror makes it optional to match.
    #[serde(default, skip_serializing_if = "is_false")]
    pub remove: bool,
}

/// Serde `skip_serializing_if` helper: keeps `remove: false` off the wire.
///
/// # Examples
///
/// ```text
/// is_false(&false) == true   // field omitted
/// ```
fn is_false(b: &bool) -> bool {
    !*b
}

/// A single operation within a command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Operation {
    /// Insert a whole new document.
    Create {
        /// The full document to insert.
        doc: Document,
    },
    /// Remove a document (carries the full pre-image for invertibility).
    Delete {
        /// The document as it existed at deletion.
        doc: Document,
    },
    /// Field-level changes against an existing document.
    Update {
        /// Target document id.
        doc_id: Uuid,
        /// Ordered field changes, each with its OCC pre-image.
        changes: Vec<FieldChange>,
    },
}

/// A command awaiting a sequence number (constructed by callers).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnsequencedCommand {
    /// World the command applies to.
    pub world_id: Uuid,
    /// Originating user.
    pub author: Uuid,
    /// Author-side timestamp, Unix epoch milliseconds.
    pub ts: i64,
    /// The operations, applied in order (all-or-nothing).
    pub ops: Vec<Operation>,
}

/// A command that has been assigned a per-world sequence number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct Command {
    /// Per-world monotonic sequence number (the client's replay watermark).
    pub seq: i64,
    /// World the command applied to.
    pub world_id: Uuid,
    /// Originating user.
    pub author: Uuid,
    /// Author-side timestamp, Unix epoch milliseconds.
    pub ts: i64,
    /// The applied operations, in order.
    pub ops: Vec<Operation>,
}

impl Operation {
    /// The inverse operation: Create<->Delete; Update swaps old/new per change, reversed.
    ///
    /// Operates on the wire `Operation` only — `StoredCommand`/`CommandSnapshot` (server-internal
    /// commit-time redaction state) do not exist at this layer and are not this function's
    /// concern; a future undo/redo feature that resurrects a `StoredCommand`'s `command` via
    /// `invert` must derive a FRESH snapshot for the inverted write, never carry the original's
    /// forward.
    ///
    /// # Examples
    ///
    /// ```
    /// use serde_json::json;
    /// use shadowcat::data::command::{FieldChange, Operation};
    ///
    /// let op = Operation::Update {
    ///     doc_id: uuid::Uuid::nil(),
    ///     changes: vec![FieldChange {
    ///         path: "/system/hp".into(),
    ///         old: json!(10),
    ///         new: json!(7),
    ///         remove: false,
    ///     }],
    /// };
    /// let Operation::Update { changes, .. } = op.invert() else { unreachable!() };
    /// assert_eq!(changes[0].old, json!(7));
    /// assert_eq!(changes[0].new, json!(10));
    /// ```
    pub fn invert(&self) -> Operation {
        match self {
            Operation::Create { doc } => Operation::Delete { doc: doc.clone() },
            Operation::Delete { doc } => Operation::Create { doc: doc.clone() },
            Operation::Update { doc_id, changes } => Operation::Update {
                doc_id: *doc_id,
                changes: changes
                    .iter()
                    .rev()
                    .map(|c| {
                        if c.remove {
                            // Inverse of a key removal: re-set the removed value.
                            // The slot is absent post-removal, so the inverse's OCC
                            // pre-image is Null.
                            FieldChange {
                                path: c.path.clone(),
                                old: Value::Null,
                                new: c.old.clone(),
                                remove: false,
                            }
                        } else {
                            FieldChange {
                                path: c.path.clone(),
                                old: c.new.clone(),
                                new: c.old.clone(),
                                remove: false,
                            }
                        }
                    })
                    .collect(),
            },
        }
    }
}

impl UnsequencedCommand {
    /// The inverse command: every op inverted, op order reversed.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::command::UnsequencedCommand;
    ///
    /// let cmd = UnsequencedCommand {
    ///     world_id: uuid::Uuid::nil(),
    ///     author: uuid::Uuid::nil(),
    ///     ts: 0,
    ///     ops: vec![],
    /// };
    /// assert_eq!(cmd.invert().ops.len(), 0);
    /// ```
    pub fn invert(&self) -> UnsequencedCommand {
        UnsequencedCommand {
            world_id: self.world_id,
            author: self.author,
            ts: self.ts,
            ops: self.ops.iter().rev().map(Operation::invert).collect(),
        }
    }
}

impl Command {
    /// Inverse as an unsequenced command (re-applied gets a fresh seq).
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::command::Command;
    ///
    /// let cmd = Command {
    ///     seq: 7,
    ///     world_id: uuid::Uuid::nil(),
    ///     author: uuid::Uuid::nil(),
    ///     ts: 0,
    ///     ops: vec![],
    /// };
    /// let undo = cmd.invert(); // UnsequencedCommand: gets a fresh seq on apply
    /// assert_eq!(undo.world_id, cmd.world_id);
    /// ```
    pub fn invert(&self) -> UnsequencedCommand {
        UnsequencedCommand {
            world_id: self.world_id,
            author: self.author,
            ts: self.ts,
            ops: self.ops.iter().rev().map(Operation::invert).collect(),
        }
    }
}

/// Who originated a write reaching `apply_intent`. A stored `message` doc's
/// `Update` is blanket-rejected for `Client`; `ServerMessageRevision`
/// — set ONLY by the server edit/delete handlers, never derivable from any wire
/// frame — re-opens that path for the sanitized authoritative revision.
/// `CombatTransition` — set ONLY by the combat clock's own write path, never
/// derivable from any wire frame — skips the ordinary per-op capability gates
/// on a batch while every structural/OCC check still runs; it may `Create` a
/// `message` doc (roll results, event messages) but is blanket-rejected from
/// `Update`-ing one, same as `Client`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOrigin {
    /// Any wire-derived write (WS intent or HTTP).
    Client,
    /// The server's own sanitized chat edit/delete revision, OR the post-publish
    /// image-enrichment republish (`chat::post_publish::run_pending_enrichments`)
    /// — never derivable from a wire frame.
    ServerMessageRevision,
    /// Server-authored combat clock write: per-op capability gates are
    /// skipped; scope, size, engine, containment, singleton,
    /// one-active-per-scene, schema and OCC checks all run; never derivable
    /// from a wire frame.
    CombatTransition,
}

impl WriteOrigin {
    /// Whether this origin is server-authored — set only by a trusted
    /// internal caller, never derivable from any wire frame.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::command::WriteOrigin;
    ///
    /// assert!(!WriteOrigin::Client.is_server_authored());
    /// assert!(WriteOrigin::CombatTransition.is_server_authored());
    /// ```
    pub fn is_server_authored(&self) -> bool {
        matches!(
            self,
            WriteOrigin::ServerMessageRevision | WriteOrigin::CombatTransition
        )
    }
}

/// Tokenize a non-empty RFC 6901 JSON pointer into its unescaped path segments.
/// A pointer that does not begin with `/` is rejected as `BadPath`; empty path
/// tokens (from a trailing slash) and `-` are treated as literal object keys.
/// Shared by `set_pointer` and `remove_pointer`; their descent semantics differ
/// (set creates missing intermediates, remove treats them as already-absent), so
/// only the tokenization is factored out, not the traversal.
///
/// # Examples
///
/// ```text
/// pointer_tokens("/system/a~1b")? == ["system", "a/b"]   // ~1 unescapes to /
/// ```
fn pointer_tokens(pointer: &str) -> Result<Vec<String>, DataError> {
    if !pointer.starts_with('/') {
        return Err(DataError::BadPath(pointer.to_string()));
    }
    Ok(pointer
        .split('/')
        .skip(1)
        .map(|t| t.replace("~1", "/").replace("~0", "~"))
        .collect())
}

/// Set `new` at JSON-pointer `pointer` in `root`, creating intermediate
/// objects as needed. Existing array indices may be replaced; array growth
/// and `-` append are out of scope (handled by the deferred merge engine).
/// A non-empty pointer must begin with `/` (RFC 6901) or it is rejected as
/// `BadPath`; empty path tokens (from a trailing slash) and `-` are treated
/// as literal object keys.
///
/// # Examples
///
/// ```
/// use serde_json::json;
/// use shadowcat::data::command::set_pointer;
///
/// // Creates missing intermediates, and replaces a null intermediate with an
/// // object (a serialized `Option::None` engine field is `null`, not absent).
/// let mut doc = json!({ "engine": null });
/// set_pointer(&mut doc, "/engine/vision/range", json!(30)).unwrap();
/// assert_eq!(doc, json!({ "engine": { "vision": { "range": 30 } } }));
/// ```
pub fn set_pointer(root: &mut Value, pointer: &str, new: Value) -> Result<(), DataError> {
    if pointer.is_empty() {
        *root = new;
        return Ok(());
    }
    let tokens = pointer_tokens(pointer)?;
    let mut cur = root;
    for (i, tok) in tokens.iter().enumerate() {
        let last = i == tokens.len() - 1;
        if last {
            match cur {
                Value::Object(m) => {
                    m.insert(tok.clone(), new);
                    return Ok(());
                }
                Value::Array(a) => {
                    let idx: usize = tok
                        .parse()
                        .map_err(|_| DataError::BadPath(pointer.to_string()))?;
                    if idx < a.len() {
                        a[idx] = new;
                        return Ok(());
                    }
                    return Err(DataError::BadPath(pointer.to_string()));
                }
                _ => return Err(DataError::BadPath(pointer.to_string())),
            }
        }
        cur = match cur {
            Value::Object(m) => {
                let entry = m
                    .entry(tok.clone())
                    .or_insert_with(|| Value::Object(Default::default()));
                // An explicit `null` intermediate (e.g. an Option<T> field with no
                // `skip_serializing_if`, serialized as `null` rather than omitted) descends
                // the same as a missing key: `remove_pointer` and serde_json reads
                // (`Value::pointer`) already treat a null intermediate as absent, so set now agrees
                // for the intermediate-descent case. Leaf null-vs-absent (the `last` branch above)
                // is unchanged.
                if entry.is_null() {
                    *entry = Value::Object(Default::default());
                }
                entry
            }
            Value::Array(a) => {
                let idx: usize = tok
                    .parse()
                    .map_err(|_| DataError::BadPath(pointer.to_string()))?;
                a.get_mut(idx)
                    .ok_or_else(|| DataError::BadPath(pointer.to_string()))?
            }
            _ => return Err(DataError::BadPath(pointer.to_string())),
        };
    }
    Ok(())
}

/// Remove the object key at JSON-pointer `pointer` from `root`, making it
/// genuinely absent (`null` != absent). Object keys only.
///
/// - Removing an already-absent key — or any key beneath an already-absent OR
///   explicit-`null` intermediate — is a no-op success (no intermediate is
///   created, unlike `set_pointer`).
/// - Array-index removal is rejected as `BadPath`: an array shrinks only via
///   whole-array replacement (a `set_pointer` of the parent), mirroring the merge
///   engine's band-level array handling; a leaf remove has no defined
///   element-shift semantics.
/// - An empty pointer, a missing leading `/`, descent through a scalar, or a
///   non-numeric token into an array are rejected as `BadPath` (matching
///   `set_pointer`'s malformed-path handling).
///
/// # Examples
///
/// ```
/// use serde_json::json;
/// use shadowcat::data::command::remove_pointer;
///
/// let mut doc = json!({ "system": { "hp": 10 } });
/// remove_pointer(&mut doc, "/system/hp").unwrap();
/// assert_eq!(doc, json!({ "system": {} })); // key genuinely absent, not null
///
/// // Array-index removal has no shift semantics — rejected, array unmutated.
/// let mut arr = json!({ "tags": ["a", "b"] });
/// assert!(remove_pointer(&mut arr, "/tags/0").is_err());
/// assert_eq!(arr, json!({ "tags": ["a", "b"] }));
/// ```
pub fn remove_pointer(root: &mut Value, pointer: &str) -> Result<(), DataError> {
    if pointer.is_empty() {
        return Err(DataError::BadPath(pointer.to_string()));
    }
    let tokens = pointer_tokens(pointer)?;
    let mut cur = root;
    for (i, tok) in tokens.iter().enumerate() {
        let last = i == tokens.len() - 1;
        if last {
            return match cur {
                Value::Object(m) => {
                    m.remove(tok);
                    Ok(())
                }
                Value::Array(_) => Err(DataError::BadPath(pointer.to_string())),
                _ => Err(DataError::BadPath(pointer.to_string())),
            };
        }
        cur = match cur {
            // A missing OR explicit-null intermediate means the target is already absent:
            // no-op — uniform with `set_pointer` (descends by creating a container) and serde_json
            // reads, which treat a null intermediate as absent.
            Value::Object(m) => match m.get_mut(tok) {
                Some(v) if !v.is_null() => v,
                _ => return Ok(()),
            },
            Value::Array(a) => {
                let idx: usize = tok
                    .parse()
                    .map_err(|_| DataError::BadPath(pointer.to_string()))?;
                match a.get_mut(idx) {
                    Some(v) => v,
                    None => return Ok(()),
                }
            }
            _ => return Err(DataError::BadPath(pointer.to_string())),
        };
    }
    Ok(())
}

/// Apply one `FieldChange` to a serialized document value. THE mutation rule for a
/// field-path change, stated once for the whole repository: `remove: true` deletes the
/// key at `path` and `new` is unused; anything else sets `new`.
///
/// INVARIANT: every store of document state — the authoritative SQLite rows and every
/// derived mirror of them (`SceneEcs`) — must reach the SAME value for the same change.
/// Restating the `remove`/set branch at a call site is the defect: `new` is constrained
/// by NEITHER the OCC pre-image comparison (which reads `old`) NOR
/// `required_cap_for_path`, so a mirror that unconditionally calls `set_pointer` lands an
/// attacker-chosen value where the store lands absence. On a path authz then reads
/// (`/owner`, `/engine/actor_id`) that divergence hands write-refused documents to
/// whoever `new` names. Call this; do not re-derive it.
///
/// Errors are the underlying pointer ops' (`BadPath` for a malformed path, an array-index
/// removal, or descent through a scalar). The authoritative paths propagate with `?` so a
/// rejected mutation aborts the transaction before commit; a derived mirror — whose input
/// may be already-committed OR client-proposed (`SceneEcs::token_move` mirrors changes
/// that have not yet been authorized) — cannot reject and handles the error locally,
/// at a level chosen by which of the two it is holding.
///
/// # Examples
///
/// ```
/// use serde_json::json;
/// use shadowcat::data::command::{apply_field_change, FieldChange};
///
/// let mut doc = json!({ "system": { "hp": 10 } });
/// apply_field_change(&mut doc, &FieldChange {
///     path: "/system/hp".into(),
///     old: json!(10),
///     new: json!(7),
///     remove: false,
/// }).unwrap();
/// assert_eq!(doc, json!({ "system": { "hp": 7 } }));
///
/// apply_field_change(&mut doc, &FieldChange {
///     path: "/system/hp".into(),
///     old: json!(7),
///     new: json!(null),
///     remove: true,
/// }).unwrap();
/// assert_eq!(doc, json!({ "system": {} }));
/// ```
pub fn apply_field_change(v: &mut Value, ch: &FieldChange) -> Result<(), DataError> {
    if ch.remove {
        remove_pointer(v, &ch.path)
    } else {
        set_pointer(v, &ch.path, ch.new.clone())
    }
}

#[cfg(test)]
mod tests;
