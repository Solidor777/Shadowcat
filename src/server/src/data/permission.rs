use uuid::Uuid;

use crate::data::command::{Command, Operation};
use crate::data::document::{DocRole, Document, Visibility, WorldRole};
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;

/// Effective access for a (user, document) pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Access {
    pub can_read: bool,
    pub can_write: bool,
    pub see_gm_only: bool,
}

/// Resolve a user's effective access to a document. A world GM has full
/// access including GM-only properties; otherwise the document's per-user
/// role (falling back to its default role) decides.
pub fn resolve_access(user: Uuid, world_role: WorldRole, doc: &Document) -> Access {
    if world_role == WorldRole::Gm {
        return Access {
            can_read: true,
            can_write: true,
            see_gm_only: true,
        };
    }
    let role = doc
        .permissions
        .users
        .get(&user)
        .copied()
        .unwrap_or(doc.permissions.default);
    match role {
        DocRole::Owner => Access {
            can_read: true,
            can_write: true,
            see_gm_only: false,
        },
        DocRole::Observer => Access {
            can_read: true,
            can_write: false,
            see_gm_only: false,
        },
        DocRole::None => Access {
            can_read: false,
            can_write: false,
            see_gm_only: false,
        },
    }
}

/// Produce the recipient's view of a document: when `access.see_gm_only` is
/// false, strip every property whose override is `GmOnly`.
pub fn filter_properties(doc: &Document, access: Access) -> Document {
    let mut out = doc.clone();
    if access.see_gm_only {
        return out;
    }
    let gm_only: Vec<String> = doc
        .permissions
        .property_overrides
        .iter()
        .filter(|(_, v)| **v == Visibility::GmOnly)
        .map(|(p, _)| p.clone())
        .collect();
    let mut whole = serde_json::to_value(&out).expect("document serializes");
    for pointer in gm_only {
        strip_pointer(&mut whole, &pointer);
    }
    out = serde_json::from_value(whole).expect("filtered document deserializes");
    out
}

/// The recipient's view of a broadcast command: ops on unreadable documents
/// are dropped, GmOnly properties/changes stripped. seq/world/author/ts are
/// preserved so the recipient's sequence guard never sees a false gap — a fully
/// redacted command keeps its seq with empty ops.
///
/// Async because `Update` ops carry only deltas, not the document's
/// `PermissionSet`; the current doc is loaded per op to resolve visibility.
pub async fn filter_command(
    repo: &dyn Repository,
    cmd: &Command,
    ctx: &PermissionContext,
) -> Command {
    let mut out_ops = Vec::with_capacity(cmd.ops.len());
    for op in &cmd.ops {
        match op {
            Operation::Create { doc } => {
                let access = resolve_access(ctx.user_id, ctx.world_role, doc);
                if access.can_read {
                    out_ops.push(Operation::Create {
                        doc: filter_properties(doc, access),
                    });
                }
            }
            Operation::Delete { doc } => {
                // A delete is visible to anyone who could read the document.
                let access = resolve_access(ctx.user_id, ctx.world_role, doc);
                if access.can_read {
                    out_ops.push(Operation::Delete {
                        doc: filter_properties(doc, access),
                    });
                }
            }
            Operation::Update { doc_id, changes } => {
                let Ok(Some(cur)) = repo.get_document(*doc_id).await else {
                    continue;
                };
                let access = resolve_access(ctx.user_id, ctx.world_role, &cur);
                if !access.can_read {
                    continue;
                }
                let kept: Vec<_> = if access.see_gm_only {
                    changes.clone()
                } else {
                    changes
                        .iter()
                        .cloned()
                        .filter(|ch| {
                            cur.permissions.property_overrides.get(&ch.path)
                                != Some(&Visibility::GmOnly)
                        })
                        .collect()
                };
                out_ops.push(Operation::Update {
                    doc_id: *doc_id,
                    changes: kept,
                });
            }
        }
    }
    Command {
        seq: cmd.seq,
        world_id: cmd.world_id,
        author: cmd.author,
        ts: cmd.ts,
        ops: out_ops,
    }
}

/// Remove the value at a JSON pointer, if present.
fn strip_pointer(root: &mut serde_json::Value, pointer: &str) {
    let tokens: Vec<String> = pointer
        .split('/')
        .skip(1)
        .map(|t| t.replace("~1", "/").replace("~0", "~"))
        .collect();
    if tokens.is_empty() {
        return;
    }
    let mut cur = root;
    for tok in &tokens[..tokens.len() - 1] {
        match cur.get_mut(tok) {
            Some(next) => cur = next,
            None => return,
        }
    }
    if let serde_json::Value::Object(m) = cur {
        m.remove(&tokens[tokens.len() - 1]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::document::{PermissionSet, Scope};

    fn doc(perms: PermissionSet, system: serde_json::Value) -> Document {
        Document {
            id: Uuid::from_u128(1),
            scope: Scope::World {
                world_id: Uuid::from_u128(9),
            },
            doc_type: "actor".into(),
            schema_version: 1,
            source: None,
            owner: None,
            permissions: perms,
            embedded: Default::default(),
            system,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn gm_sees_everything() {
        let a = resolve_access(
            Uuid::from_u128(5),
            WorldRole::Gm,
            &doc(Default::default(), serde_json::json!({})),
        );
        assert_eq!(
            a,
            Access {
                can_read: true,
                can_write: true,
                see_gm_only: true
            }
        );
    }

    #[test]
    fn owner_observer_none_resolve_correctly() {
        let mut perms = PermissionSet::default();
        perms.users.insert(Uuid::from_u128(1), DocRole::Owner);
        perms.users.insert(Uuid::from_u128(2), DocRole::Observer);
        let d = doc(perms, serde_json::json!({}));
        assert!(resolve_access(Uuid::from_u128(1), WorldRole::Player, &d).can_write);
        let obs = resolve_access(Uuid::from_u128(2), WorldRole::Player, &d);
        assert!(obs.can_read && !obs.can_write);
        let other = resolve_access(Uuid::from_u128(3), WorldRole::Player, &d);
        assert!(!other.can_read);
    }

    #[test]
    fn gm_only_property_is_stripped_for_non_gm() {
        let mut perms = PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        };
        perms
            .property_overrides
            .insert("/system/secret".into(), Visibility::GmOnly);
        let d = doc(perms, serde_json::json!({ "secret": 42, "public": 1 }));

        let player = resolve_access(Uuid::from_u128(7), WorldRole::Player, &d);
        let view = filter_properties(&d, player);
        assert_eq!(view.system.get("secret"), None);
        assert_eq!(view.system["public"], serde_json::json!(1));

        let gm = resolve_access(Uuid::from_u128(7), WorldRole::Gm, &d);
        assert_eq!(
            filter_properties(&d, gm).system["secret"],
            serde_json::json!(42)
        );
    }

    #[tokio::test]
    async fn filter_command_strips_and_preserves_seq() {
        use crate::auth::role::ServerRole;
        use crate::data::command::{Command, FieldChange, Operation};
        use crate::data::membership::PermissionContext;
        use crate::data::sqlite::SqliteRepository;

        let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = r.create_user("gm", None, ServerRole::User, 0).await.unwrap();
        let w = r.create_world_owned("W", gm, 0).await.unwrap();
        let gm_ctx = PermissionContext {
            user_id: gm,
            world_role: WorldRole::Gm,
        };

        let mut d = doc(
            PermissionSet {
                default: DocRole::Observer,
                ..Default::default()
            },
            serde_json::json!({ "secret": 1, "public": 2 }),
        );
        d.scope = Scope::World { world_id: w.id };
        d.permissions
            .property_overrides
            .insert("/system/secret".into(), Visibility::GmOnly);
        r.apply_intent(&gm_ctx, w.id, vec![Operation::Create { doc: d.clone() }], 1)
            .await
            .unwrap();

        // An update touching both a GmOnly and a public field.
        let cmd = Command {
            seq: 2,
            world_id: w.id,
            author: gm,
            ts: 0,
            ops: vec![Operation::Update {
                doc_id: d.id,
                changes: vec![
                    FieldChange {
                        path: "/system/secret".into(),
                        old: serde_json::json!(1),
                        new: serde_json::json!(9),
                    },
                    FieldChange {
                        path: "/system/public".into(),
                        old: serde_json::json!(2),
                        new: serde_json::json!(8),
                    },
                ],
            }],
        };

        // Player sees the public change only; seq is preserved.
        let player = PermissionContext {
            user_id: Uuid::from_u128(77),
            world_role: WorldRole::Player,
        };
        let filtered = filter_command(&r, &cmd, &player).await;
        assert_eq!(filtered.seq, 2);
        if let Operation::Update { changes, .. } = &filtered.ops[0] {
            assert_eq!(changes.len(), 1);
            assert_eq!(changes[0].path, "/system/public");
        } else {
            panic!("expected Update");
        }

        // GM sees both changes.
        let gm_view = filter_command(&r, &cmd, &gm_ctx).await;
        if let Operation::Update { changes, .. } = &gm_view.ops[0] {
            assert_eq!(changes.len(), 2);
        } else {
            panic!("expected Update");
        }
    }
}
