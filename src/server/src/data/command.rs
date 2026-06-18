use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::data::document::Document;
use crate::data::DataError;

/// One field-level change with its pre-image, so it is self-inverting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldChange {
    pub path: String, // JSON pointer, e.g. "/system/hp"
    pub old: Value,
    pub new: Value,
}

/// A single operation within a command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
                    .map(|c| FieldChange {
                        path: c.path.clone(),
                        old: c.new.clone(),
                        new: c.old.clone(),
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

/// Set `new` at JSON-pointer `pointer` in `root`, creating intermediate
/// objects as needed. Existing array indices may be replaced; array growth
/// and `-` append are out of scope (handled by the deferred merge engine).
pub fn set_pointer(root: &mut Value, pointer: &str, new: Value) -> Result<(), DataError> {
    if pointer.is_empty() {
        *root = new;
        return Ok(());
    }
    let tokens: Vec<String> = pointer
        .split('/')
        .skip(1)
        .map(|t| t.replace("~1", "/").replace("~0", "~"))
        .collect();
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
            source: None,
            owner: None,
            permissions: Default::default(),
            embedded: Default::default(),
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
                    path: "/system/a".into(),
                    old: serde_json::json!(1),
                    new: serde_json::json!(2),
                },
                FieldChange {
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
                        path: "/system/b".into(),
                        old: serde_json::json!(4),
                        new: serde_json::json!(3)
                    },
                    FieldChange {
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
    fn set_pointer_rejects_descend_into_scalar() {
        let mut v = serde_json::json!({ "hp": 10 });
        let err = set_pointer(&mut v, "/hp/value", serde_json::json!(1));
        assert!(matches!(err, Err(DataError::BadPath(_))));
    }
}
