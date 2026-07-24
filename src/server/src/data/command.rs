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
    pub path: String, // JSON pointer, e.g. "/system/hp"
    #[ts(type = "unknown")]
    pub old: Value,
    #[ts(type = "unknown")]
    pub new: Value,
    /// When true, REMOVE the object key at `path` instead of setting `new`.
    /// Object keys only — array-index removal is rejected (see `remove_pointer`).
    /// Omitted on the wire when false (`#[serde(default)]` on ingest); the
    /// client Zod mirror makes it optional to match.
    #[serde(default, skip_serializing_if = "is_false")]
    pub remove: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// A single operation within a command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Operation {
    Create {
        doc: Document,
    },
    Delete {
        doc: Document,
    },
    Update {
        doc_id: Uuid,
        changes: Vec<FieldChange>,
    },
}

/// A command awaiting a sequence number (constructed by callers).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnsequencedCommand {
    pub world_id: Uuid,
    pub author: Uuid,
    pub ts: i64,
    pub ops: Vec<Operation>,
}

/// A command that has been assigned a per-world sequence number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct Command {
    pub seq: i64,
    pub world_id: Uuid,
    pub author: Uuid,
    pub ts: i64,
    pub ops: Vec<Operation>,
}

impl Operation {
    /// The inverse operation: Create<->Delete; Update swaps old/new per change, reversed.
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
/// `Update` is blanket-rejected for `Client` (c-1 invariant); `ServerMessageRevision`
/// — set ONLY by the server edit/delete handlers, never derivable from any wire
/// frame — re-opens that path for the sanitized authoritative revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOrigin {
    Client,
    ServerMessageRevision,
}

/// Tokenize a non-empty RFC 6901 JSON pointer into its unescaped path segments.
/// A pointer that does not begin with `/` is rejected as `BadPath`; empty path
/// tokens (from a trailing slash) and `-` are treated as literal object keys.
/// Shared by `set_pointer` and `remove_pointer`; their descent semantics differ
/// (set creates missing intermediates, remove treats them as already-absent), so
/// only the tokenization is factored out, not the traversal.
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
            Value::Object(m) => m
                .entry(tok.clone())
                .or_insert_with(|| Value::Object(Default::default())),
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
/// - Removing an already-absent key — or any key beneath an already-absent
///   intermediate — is a no-op success (no intermediate is created, unlike
///   `set_pointer`).
/// - Array-index removal is rejected as `BadPath`: an array shrinks only via
///   whole-array replacement (a `set_pointer` of the parent), mirroring the merge
///   engine's band-level array handling; a leaf remove has no defined
///   element-shift semantics.
/// - An empty pointer, a missing leading `/`, descent through a scalar, or a
///   non-numeric token into an array are rejected as `BadPath` (matching
///   `set_pointer`'s malformed-path handling).
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
            // A missing intermediate means the target is already absent: no-op.
            Value::Object(m) => match m.get_mut(tok) {
                Some(v) => v,
                None => return Ok(()),
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
/// rejected mutation aborts the transaction; a derived mirror running on already-committed
/// state cannot reject and handles the error locally instead.
pub fn apply_field_change(v: &mut Value, ch: &FieldChange) -> Result<(), DataError> {
    if ch.remove {
        remove_pointer(v, &ch.path)
    } else {
        set_pointer(v, &ch.path, ch.new.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(id: u128) -> Document {
        Document {
            id: Uuid::from_u128(id),
            scope: crate::data::document::Scope::World {
                world_id: Uuid::from_u128(9),
            },
            doc_type: "item".into(),
            schema_version: 1,
            name: None,
            source: None,
            base: None,
            owner: None,
            permissions: Default::default(),
            embedded: Default::default(),
            parent_id: None,
            engine: None,
            system: serde_json::json!({}),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn create_inverts_to_delete_and_back() {
        let op = Operation::Create { doc: doc(1) };
        assert_eq!(op.invert(), Operation::Delete { doc: doc(1) });
        assert_eq!(op.invert().invert(), op);
    }

    #[test]
    fn update_invert_swaps_old_and_new_in_reverse() {
        let op = Operation::Update {
            doc_id: Uuid::from_u128(1),
            changes: vec![
                FieldChange {
                    remove: false,
                    path: "/system/a".into(),
                    old: serde_json::json!(1),
                    new: serde_json::json!(2),
                },
                FieldChange {
                    remove: false,
                    path: "/system/b".into(),
                    old: serde_json::json!(3),
                    new: serde_json::json!(4),
                },
            ],
        };
        let inv = op.invert();
        assert_eq!(
            inv,
            Operation::Update {
                doc_id: Uuid::from_u128(1),
                changes: vec![
                    FieldChange {
                        remove: false,
                        path: "/system/b".into(),
                        old: serde_json::json!(4),
                        new: serde_json::json!(3)
                    },
                    FieldChange {
                        remove: false,
                        path: "/system/a".into(),
                        old: serde_json::json!(2),
                        new: serde_json::json!(1)
                    },
                ],
            }
        );
        assert_eq!(op.invert().invert(), op);
    }

    #[test]
    fn unsequenced_command_invert_is_round_trip() {
        let cmd = UnsequencedCommand {
            world_id: Uuid::from_u128(9),
            author: Uuid::from_u128(5),
            ts: 1,
            ops: vec![
                Operation::Create { doc: doc(1) },
                Operation::Update {
                    doc_id: Uuid::from_u128(1),
                    changes: vec![FieldChange {
                        remove: false,
                        path: "/system/x".into(),
                        old: serde_json::json!(null),
                        new: serde_json::json!(7),
                    }],
                },
            ],
        };
        assert_eq!(cmd.invert().invert(), cmd);
    }

    #[test]
    fn set_pointer_sets_existing_and_creates_intermediate() {
        let mut v = serde_json::json!({ "system": { "hp": 10 } });
        set_pointer(&mut v, "/system/hp", serde_json::json!(5)).unwrap();
        assert_eq!(v["system"]["hp"], serde_json::json!(5));

        set_pointer(&mut v, "/system/attributes/str", serde_json::json!(14)).unwrap();
        assert_eq!(v["system"]["attributes"]["str"], serde_json::json!(14));
    }

    #[test]
    fn set_pointer_writes_into_an_indexed_embedded_actor_copy() {
        // An instanced token toggles conditions on its embedded actor copy at
        // `/embedded/actor/0/system/conditions`: array-index intermediate descent
        // followed by an object-leaf insert. (M10c instanced-condition write path.)
        let mut v =
            serde_json::json!({ "embedded": { "actor": [ { "system": { "conditions": [] } } ] } });
        set_pointer(
            &mut v,
            "/embedded/actor/0/system/conditions",
            serde_json::json!(["dead"]),
        )
        .unwrap();
        assert_eq!(
            v["embedded"]["actor"][0]["system"]["conditions"],
            serde_json::json!(["dead"])
        );
    }

    #[test]
    fn set_pointer_rejects_descend_into_scalar() {
        let mut v = serde_json::json!({ "hp": 10 });
        let err = set_pointer(&mut v, "/hp/value", serde_json::json!(1));
        assert!(matches!(err, Err(DataError::BadPath(_))));
    }

    #[test]
    fn remove_pointer_makes_an_object_key_genuinely_absent() {
        // A removed key is absent, NOT present-with-null (`null` != absent).
        let mut v = serde_json::json!({ "system": { "foo": "bar", "baz": 1 } });
        remove_pointer(&mut v, "/system/foo").unwrap();
        let sys = v["system"].as_object().unwrap();
        assert!(!sys.contains_key("foo"), "key must be absent, not null");
        assert_eq!(sys["baz"], serde_json::json!(1), "sibling keys untouched");
    }

    #[test]
    fn remove_pointer_on_already_absent_key_is_a_no_op() {
        let mut v = serde_json::json!({ "system": { "baz": 1 } });
        remove_pointer(&mut v, "/system/foo").unwrap();
        assert_eq!(v, serde_json::json!({ "system": { "baz": 1 } }));
    }

    #[test]
    fn remove_pointer_through_absent_intermediate_is_a_no_op() {
        // No intermediate is CREATED (unlike set_pointer): a target under a
        // missing ancestor is already absent, so removal is a silent success.
        let mut v = serde_json::json!({ "system": {} });
        remove_pointer(&mut v, "/system/missing/leaf").unwrap();
        assert_eq!(v, serde_json::json!({ "system": {} }));
    }

    #[test]
    fn remove_pointer_rejects_array_index_removal() {
        // Array shrink is whole-array replacement only (merge-engine invariant):
        // a leaf remove of an index has no defined shift semantics.
        let mut v = serde_json::json!({ "tags": ["a", "b", "c"] });
        assert!(matches!(
            remove_pointer(&mut v, "/tags/1"),
            Err(DataError::BadPath(_))
        ));
        assert_eq!(v, serde_json::json!({ "tags": ["a", "b", "c"] }));
    }

    #[test]
    fn remove_pointer_rejects_descend_into_scalar() {
        let mut v = serde_json::json!({ "hp": 10 });
        assert!(matches!(
            remove_pointer(&mut v, "/hp/value"),
            Err(DataError::BadPath(_))
        ));
    }

    #[test]
    fn remove_pointer_rejects_missing_leading_slash_and_empty() {
        let mut v = serde_json::json!({ "system": { "hp": 10 } });
        assert!(matches!(
            remove_pointer(&mut v, "system/hp"),
            Err(DataError::BadPath(_))
        ));
        assert!(matches!(
            remove_pointer(&mut v, ""),
            Err(DataError::BadPath(_))
        ));
        assert_eq!(v, serde_json::json!({ "system": { "hp": 10 } }));
    }

    #[test]
    fn remove_change_inverts_to_a_reinserting_set() {
        // Inverse of "remove key holding V" is "set key to V"; after the removal
        // the slot is absent, so the inverse's pre-image is Null.
        let op = Operation::Update {
            doc_id: Uuid::from_u128(1),
            changes: vec![FieldChange {
                path: "/system/foo".into(),
                old: serde_json::json!("bar"),
                new: serde_json::Value::Null,
                remove: true,
            }],
        };
        assert_eq!(
            op.invert(),
            Operation::Update {
                doc_id: Uuid::from_u128(1),
                changes: vec![FieldChange {
                    path: "/system/foo".into(),
                    old: serde_json::Value::Null,
                    new: serde_json::json!("bar"),
                    remove: false,
                }],
            }
        );
    }

    #[test]
    fn set_pointer_rejects_missing_leading_slash() {
        // A pointer without a leading "/" must error, not silently write the
        // wrong field (e.g. "system/hp" must not land on top-level "hp").
        let mut v = serde_json::json!({ "system": { "hp": 10 } });
        assert!(matches!(
            set_pointer(&mut v, "system/hp", serde_json::json!(5)),
            Err(DataError::BadPath(_))
        ));
        assert!(matches!(
            set_pointer(&mut v, "foo", serde_json::json!(5)),
            Err(DataError::BadPath(_))
        ));
        assert_eq!(v, serde_json::json!({ "system": { "hp": 10 } }));
    }

    #[test]
    fn command_round_trips_through_json() {
        use crate::data::document::{DocRole, PermissionSet, Scope, Source, Visibility};

        let mut perms = PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        };
        perms.users.insert(Uuid::from_u128(5), DocRole::Owner);
        perms
            .property_overrides
            .insert("/system/secret".into(), Visibility::GmOnly);

        let mut embedded = std::collections::BTreeMap::new();
        embedded.insert("items".to_string(), vec![doc(2)]);

        let rich = Document {
            id: Uuid::from_u128(1),
            scope: Scope::World {
                world_id: Uuid::from_u128(9),
            },
            doc_type: "actor".into(),
            schema_version: 1,
            name: None,
            source: Some(Source {
                id: Uuid::from_u128(3),
                pack: Some("dnd5e".into()),
                version: 2,
            }),
            base: None,
            owner: Some(Uuid::from_u128(5)),
            permissions: perms,
            embedded,
            parent_id: None,
            engine: None,
            system: serde_json::json!({ "hp": { "value": 10, "max": 12 }, "tags": ["a", "b"] }),
            created_at: 1,
            updated_at: 2,
        };

        let cmd = Command {
            seq: 7,
            world_id: Uuid::from_u128(9),
            author: Uuid::from_u128(5),
            ts: 100,
            ops: vec![
                Operation::Create { doc: rich },
                Operation::Delete { doc: doc(4) },
                Operation::Update {
                    doc_id: Uuid::from_u128(1),
                    changes: vec![
                        FieldChange {
                            remove: false,
                            path: "/system/hp/value".into(),
                            old: serde_json::json!(10),
                            new: serde_json::json!(3),
                        },
                        FieldChange {
                            remove: false,
                            path: "/name".into(),
                            old: serde_json::json!(null),
                            new: serde_json::json!("Gandalf"),
                        },
                    ],
                },
            ],
        };

        let s = serde_json::to_string(&cmd).unwrap();
        assert!(
            s.contains("\"op\":\"create\""),
            "internally-tagged discriminator present"
        );
        let back: Command = serde_json::from_str(&s).unwrap();
        assert_eq!(cmd, back);
    }
}
