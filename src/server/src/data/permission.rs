use uuid::Uuid;

use crate::data::document::{DocRole, Document, Visibility, WorldRole};

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
}
