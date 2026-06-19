use std::collections::BTreeSet;

use uuid::Uuid;

use crate::data::command::{Command, FieldChange, Operation};
use crate::data::document::{
    CapabilityGrants, CapabilityRequirement, DocRole, Document, Visibility, WorldRole,
};
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;

/// Built-in, server-understood capabilities. Modules may grant additional
/// namespaced capabilities (`<ns>:<verb>`); the server treats those as opaque
/// tokens and enforces only possession (Phase 2 gates custom actions).
pub mod cap {
    pub const READ: &str = "core:read";
    pub const WRITE_FIELDS: &str = "core:write_fields";
    pub const MANAGE_EMBEDDED: &str = "core:manage_embedded";
    pub const DELETE: &str = "core:delete";
    pub const EDIT_PERMISSIONS: &str = "core:edit_permissions";
    pub const CREATE: &str = "core:create";
}

/// The capability required to write a document field at `path`, or `None` when
/// the path targets an immutable envelope field (not patchable via `Update`).
pub fn required_cap_for_path(path: &str) -> Option<&'static str> {
    if path == "/system" || path.starts_with("/system/") {
        Some(cap::WRITE_FIELDS)
    } else if path == "/embedded" || path.starts_with("/embedded/") {
        Some(cap::MANAGE_EMBEDDED)
    } else if path == "/permissions" || path.starts_with("/permissions/") {
        Some(cap::EDIT_PERMISSIONS)
    } else {
        None
    }
}

/// Whether `p` is a descendant of `ancestor` on a JSON-pointer boundary
/// (`/a/b` is a descendant of `/a`, but `/ab` is not).
fn is_descendant(p: &str, ancestor: &str) -> bool {
    p.len() > ancestor.len()
        && p.as_bytes()[ancestor.len()] == b'/'
        && p.as_bytes()[..ancestor.len()] == *ancestor.as_bytes()
}

/// Whether two JSON-pointer paths overlap as subtrees: equal, or either is a
/// descendant of the other.
fn paths_overlap(a: &str, b: &str) -> bool {
    a == b || is_descendant(a, b) || is_descendant(b, a)
}

/// Additional capabilities required to write `path`, declared by the world's
/// capability requirements, on top of `required_cap_for_path`'s structural base.
/// A requirement matches when the change path **overlaps** its prefix in either
/// direction: the change writes into the protected subtree (descendant), is the
/// prefix exactly, OR is an ancestor that *covers* the protected subtree (writing
/// `/system` replaces `/system/vision` wholesale). The ancestor case is
/// security-critical — a descendant-only check is bypassable by a coarse parent
/// write. This over-approximates (an ancestor write that does not touch the
/// protected leaf still demands the cap), the safe direction for an authz gate.
/// Boundary-matched, so `/system/visionmode` does not match `/system/vision`.
pub fn declared_caps_for_path<'a>(path: &str, reqs: &'a [CapabilityRequirement]) -> Vec<&'a str> {
    let mut out = Vec::new();
    for req in reqs {
        if paths_overlap(path, &req.path_prefix) {
            out.extend(req.caps.iter().map(String::as_str));
        }
    }
    out
}

/// Capabilities required to create/replace a whole document body, declared by the
/// world's capability requirements. A requirement applies when its protected path
/// is **present** in `doc_json` — the Create path writes the entire body at once,
/// so a populated protected subtree must be authorized exactly as an Update to it
/// would be. Closes the create-time bypass of declarative requirements.
pub fn declared_caps_for_document<'a>(
    doc_json: &serde_json::Value,
    reqs: &'a [CapabilityRequirement],
) -> Vec<&'a str> {
    let mut out = Vec::new();
    for req in reqs {
        if doc_json.pointer(&req.path_prefix).is_some() {
            out.extend(req.caps.iter().map(String::as_str));
        }
    }
    out
}

/// A user's effective capabilities on a document. `all` is the GM/admin
/// short-circuit (holds every capability); `caps` is the resolved set for a
/// non-GM. `see_gm_only` continues to drive property-level read redaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Access {
    pub caps: BTreeSet<String>,
    pub all: bool,
    pub see_gm_only: bool,
}

impl Access {
    /// Whether the actor holds capability `c` (GM holds everything).
    pub fn has(&self, c: &str) -> bool {
        self.all || self.caps.contains(c)
    }
}

/// The built-in capability floor for a `DocRole` (before additive grants).
fn role_floor(role: DocRole) -> BTreeSet<String> {
    let mut s = BTreeSet::new();
    match role {
        DocRole::Owner => {
            s.insert(cap::READ.to_string());
            s.insert(cap::WRITE_FIELDS.to_string());
        }
        DocRole::Observer => {
            s.insert(cap::READ.to_string());
        }
        DocRole::None => {}
    }
    s
}

/// Resolve a user's effective capabilities on a document. A world GM (or server
/// admin, which resolves to GM) holds every capability. Otherwise the actor's
/// `DocRole` (per-user, else the document default) seeds a built-in floor that
/// the document's additive grants (`by_role`, `by_user`) widen.
pub fn resolve_access(user: Uuid, world_role: WorldRole, doc: &Document) -> Access {
    if world_role == WorldRole::Gm {
        return Access {
            caps: BTreeSet::new(),
            all: true,
            see_gm_only: true,
        };
    }
    let role = doc
        .permissions
        .users
        .get(&user)
        .copied()
        .unwrap_or(doc.permissions.default);
    let mut caps = role_floor(role);
    if let Some(extra) = doc.permissions.capabilities.by_role.get(&role) {
        caps.extend(extra.iter().cloned());
    }
    if let Some(extra) = doc.permissions.capabilities.by_user.get(&user) {
        caps.extend(extra.iter().cloned());
    }
    Access {
        caps,
        all: false,
        see_gm_only: false,
    }
}

/// `resolve_access` plus a world's default capability grants, layered additively
/// on top of the per-document resolution (GM is unaffected — already holds all).
/// World defaults let a deployment grant, e.g., every Owner in a world
/// `core:manage_embedded` without editing each document.
pub fn resolve_access_world(
    user: Uuid,
    world_role: WorldRole,
    doc: &Document,
    world_grants: &CapabilityGrants,
) -> Access {
    let mut access = resolve_access(user, world_role, doc);
    if access.all {
        return access;
    }
    let role = doc
        .permissions
        .users
        .get(&user)
        .copied()
        .unwrap_or(doc.permissions.default);
    if let Some(extra) = world_grants.by_role.get(&role) {
        access.caps.extend(extra.iter().cloned());
    }
    if let Some(extra) = world_grants.by_user.get(&user) {
        access.caps.extend(extra.iter().cloned());
    }
    access
}

/// Project world-default grants down to what a single actor needs to replicate
/// access resolution client-side: the per-role tiers (world policy, no PII) plus
/// **only** this actor's own per-user grants. Other users' UUIDs and grants are
/// dropped — the full `by_user` map must never cross to a client.
pub fn project_grants_for(grants: &CapabilityGrants, user: Uuid) -> CapabilityGrants {
    CapabilityGrants {
        by_role: grants.by_role.clone(),
        by_user: grants
            .by_user
            .get(&user)
            .map(|caps| std::iter::once((user, caps.clone())).collect())
            .unwrap_or_default(),
    }
}

/// Produce the recipient's view of a document: when `access.see_gm_only` is
/// false, strip every property whose override is `GmOnly`.
pub fn filter_properties(doc: &Document, access: &Access) -> Document {
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
    world_defaults: &CapabilityGrants,
) -> Command {
    // `world_defaults` is passed in (loaded once per connection / request) rather
    // than fetched here: this runs per event per recipient on the egress hot
    // path, and a per-event DB read contends with apply_intent on the
    // single-writer pool.
    let mut out_ops = Vec::with_capacity(cmd.ops.len());
    for op in &cmd.ops {
        match op {
            Operation::Create { doc } => {
                let access = resolve_access_world(ctx.user_id, ctx.world_role, doc, world_defaults);
                if access.has(cap::READ) {
                    out_ops.push(Operation::Create {
                        doc: filter_properties(doc, &access),
                    });
                }
            }
            Operation::Delete { doc } => {
                // A delete is visible to anyone who could read the document.
                let access = resolve_access_world(ctx.user_id, ctx.world_role, doc, world_defaults);
                if access.has(cap::READ) {
                    out_ops.push(Operation::Delete {
                        doc: filter_properties(doc, &access),
                    });
                }
            }
            Operation::Update { doc_id, changes } => {
                let Ok(Some(cur)) = repo.get_document(*doc_id).await else {
                    continue;
                };
                let access =
                    resolve_access_world(ctx.user_id, ctx.world_role, &cur, world_defaults);
                if !access.has(cap::READ) {
                    continue;
                }
                let kept: Vec<FieldChange> = if access.see_gm_only {
                    changes.clone()
                } else {
                    let gm_only: Vec<String> = cur
                        .permissions
                        .property_overrides
                        .iter()
                        .filter(|(_, v)| **v == Visibility::GmOnly)
                        .map(|(p, _)| p.clone())
                        .collect();
                    changes
                        .iter()
                        .filter_map(|ch| redact_change(ch, &gm_only))
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

/// Redact one `FieldChange` for a recipient who cannot see GM-only properties,
/// using the same subtree semantics as `filter_properties` (exact-pointer
/// matching would leak nested fields):
/// - if the change targets a GM-only pointer or any descendant of one, the
///   change carries hidden data and is dropped entirely (`None`);
/// - if the change writes an ancestor of a GM-only pointer, the hidden subtree
///   is stripped from both the pre-image (`old`) and the new value.
fn redact_change(ch: &FieldChange, gm_only: &[String]) -> Option<FieldChange> {
    for ov in gm_only {
        if &ch.path == ov || ch.path.starts_with(&format!("{ov}/")) {
            return None;
        }
    }
    let mut old = ch.old.clone();
    let mut new = ch.new.clone();
    let mut changed = false;
    let prefix = format!("{}/", ch.path);
    for ov in gm_only {
        if let Some(rel) = ov.strip_prefix(&prefix) {
            let rel_ptr = format!("/{rel}");
            strip_pointer(&mut old, &rel_ptr);
            strip_pointer(&mut new, &rel_ptr);
            changed = true;
        }
    }
    if changed {
        Some(FieldChange {
            path: ch.path.clone(),
            old,
            new,
        })
    } else {
        Some(ch.clone())
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
    fn declared_caps_match_prefix_on_boundaries() {
        let reqs = vec![CapabilityRequirement {
            path_prefix: "/system/vision".into(),
            caps: ["dnd5e:gm_vision".to_string()].into_iter().collect(),
        }];
        // exact and descendant match
        assert_eq!(
            declared_caps_for_path("/system/vision", &reqs),
            vec!["dnd5e:gm_vision"]
        );
        assert_eq!(
            declared_caps_for_path("/system/vision/range", &reqs),
            vec!["dnd5e:gm_vision"]
        );
        // sibling that merely shares a string prefix does NOT match (boundary check)
        assert!(declared_caps_for_path("/system/visionmode", &reqs).is_empty());
        // unrelated path
        assert!(declared_caps_for_path("/system/hp", &reqs).is_empty());
        // ANCESTOR write that covers the protected subtree DOES match (a coarse
        // `/system` write replaces `/system/vision` wholesale).
        assert_eq!(
            declared_caps_for_path("/system", &reqs),
            vec!["dnd5e:gm_vision"]
        );
    }

    #[test]
    fn declared_caps_for_document_matches_present_paths() {
        let reqs = vec![CapabilityRequirement {
            path_prefix: "/system/vision".into(),
            caps: ["dnd5e:gm_vision".to_string()].into_iter().collect(),
        }];
        // body with a populated /system/vision subtree → requirement applies
        let with = serde_json::json!({ "system": { "vision": { "range": 30 }, "hp": 10 } });
        assert_eq!(
            declared_caps_for_document(&with, &reqs),
            vec!["dnd5e:gm_vision"]
        );
        // body without the protected path → no requirement
        let without = serde_json::json!({ "system": { "hp": 10 } });
        assert!(declared_caps_for_document(&without, &reqs).is_empty());
    }

    #[test]
    fn project_grants_drops_other_users() {
        use crate::data::document::CapabilityGrants;
        let me = Uuid::from_u128(1);
        let other = Uuid::from_u128(2);
        let mut grants = CapabilityGrants::default();
        grants
            .by_role
            .entry(DocRole::Owner)
            .or_default()
            .insert("core:manage_embedded".to_string());
        grants
            .by_user
            .entry(me)
            .or_default()
            .insert("dnd5e:cast".to_string());
        grants
            .by_user
            .entry(other)
            .or_default()
            .insert("dnd5e:secret".to_string());

        let projected = project_grants_for(&grants, me);
        // Role tiers are world policy — preserved.
        assert_eq!(projected.by_role, grants.by_role);
        // Only this actor's own per-user grant survives; the other user's UUID
        // and grants are gone.
        assert!(projected.by_user.contains_key(&me));
        assert!(!projected.by_user.contains_key(&other));
        assert_eq!(projected.by_user.len(), 1);
    }

    #[test]
    fn gm_holds_every_capability() {
        let a = resolve_access(
            Uuid::from_u128(5),
            WorldRole::Gm,
            &doc(Default::default(), serde_json::json!({})),
        );
        assert!(a.all && a.see_gm_only);
        assert!(a.has(cap::WRITE_FIELDS) && a.has(cap::MANAGE_EMBEDDED) && a.has("dnd5e:anything"));
    }

    #[test]
    fn floor_grants_by_role() {
        let mut perms = PermissionSet::default();
        perms.users.insert(Uuid::from_u128(1), DocRole::Owner);
        perms.users.insert(Uuid::from_u128(2), DocRole::Observer);
        let d = doc(perms, serde_json::json!({}));
        // Owner: read + write fields, but NOT manage embedded by default.
        let owner = resolve_access(Uuid::from_u128(1), WorldRole::Player, &d);
        assert!(owner.has(cap::READ) && owner.has(cap::WRITE_FIELDS));
        assert!(!owner.has(cap::MANAGE_EMBEDDED) && !owner.has(cap::DELETE));
        // Observer: read only.
        let obs = resolve_access(Uuid::from_u128(2), WorldRole::Player, &d);
        assert!(obs.has(cap::READ) && !obs.has(cap::WRITE_FIELDS));
        // Stranger falls to default (None): nothing.
        let other = resolve_access(Uuid::from_u128(3), WorldRole::Player, &d);
        assert!(!other.has(cap::READ));
    }

    #[test]
    fn additive_grants_widen_the_floor() {
        use crate::data::document::CapabilityGrants;
        let mut perms = PermissionSet::default();
        perms.users.insert(Uuid::from_u128(1), DocRole::Owner);
        let mut grants = CapabilityGrants::default();
        // Grant Owners on this doc the ability to manage embedded documents.
        grants
            .by_role
            .entry(DocRole::Owner)
            .or_default()
            .insert(cap::MANAGE_EMBEDDED.to_string());
        // Grant a specific user a custom module capability.
        grants
            .by_user
            .entry(Uuid::from_u128(1))
            .or_default()
            .insert("dnd5e:cast".to_string());
        perms.capabilities = grants;
        let d = doc(perms, serde_json::json!({}));
        let a = resolve_access(Uuid::from_u128(1), WorldRole::Player, &d);
        assert!(a.has(cap::WRITE_FIELDS)); // floor retained
        assert!(a.has(cap::MANAGE_EMBEDDED)); // role grant
        assert!(a.has("dnd5e:cast")); // user grant
        assert!(!a.has(cap::DELETE)); // not granted
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
        let view = filter_properties(&d, &player);
        assert_eq!(view.system.get("secret"), None);
        assert_eq!(view.system["public"], serde_json::json!(1));

        let gm = resolve_access(Uuid::from_u128(7), WorldRole::Gm, &d);
        assert_eq!(
            filter_properties(&d, &gm).system["secret"],
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
        let gm = r
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
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
        let filtered = filter_command(&r, &cmd, &player, &CapabilityGrants::default()).await;
        assert_eq!(filtered.seq, 2);
        if let Operation::Update { changes, .. } = &filtered.ops[0] {
            assert_eq!(changes.len(), 1);
            assert_eq!(changes[0].path, "/system/public");
        } else {
            panic!("expected Update");
        }

        // GM sees both changes.
        let gm_view = filter_command(&r, &cmd, &gm_ctx, &CapabilityGrants::default()).await;
        if let Operation::Update { changes, .. } = &gm_view.ops[0] {
            assert_eq!(changes.len(), 2);
        } else {
            panic!("expected Update");
        }
    }

    #[tokio::test]
    async fn filter_command_redacts_nested_gm_only_paths() {
        use crate::auth::role::ServerRole;
        use crate::data::command::{Command, FieldChange, Operation};
        use crate::data::membership::PermissionContext;
        use crate::data::sqlite::SqliteRepository;

        let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = r
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
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
            serde_json::json!({
                "secret": { "value": 1 },
                "sheet": { "hidden": 2, "shown": 3 },
                "public": 4
            }),
        );
        d.scope = Scope::World { world_id: w.id };
        // A GM-only object and a GM-only nested leaf.
        d.permissions
            .property_overrides
            .insert("/system/secret".into(), Visibility::GmOnly);
        d.permissions
            .property_overrides
            .insert("/system/sheet/hidden".into(), Visibility::GmOnly);
        r.apply_intent(&gm_ctx, w.id, vec![Operation::Create { doc: d.clone() }], 1)
            .await
            .unwrap();

        let cmd = Command {
            seq: 2,
            world_id: w.id,
            author: gm,
            ts: 0,
            ops: vec![Operation::Update {
                doc_id: d.id,
                changes: vec![
                    // Descendant of a GM-only pointer → dropped entirely.
                    FieldChange {
                        path: "/system/secret/value".into(),
                        old: serde_json::json!(1),
                        new: serde_json::json!(9),
                    },
                    // Ancestor of a GM-only pointer → hidden child stripped from
                    // both pre-image and new value, siblings preserved.
                    FieldChange {
                        path: "/system/sheet".into(),
                        old: serde_json::json!({ "hidden": 2, "shown": 3 }),
                        new: serde_json::json!({ "hidden": 20, "shown": 30 }),
                    },
                    // Unrelated public field → kept whole.
                    FieldChange {
                        path: "/system/public".into(),
                        old: serde_json::json!(4),
                        new: serde_json::json!(40),
                    },
                ],
            }],
        };

        let player = PermissionContext {
            user_id: Uuid::from_u128(77),
            world_role: WorldRole::Player,
        };
        let filtered = filter_command(&r, &cmd, &player, &CapabilityGrants::default()).await;
        let Operation::Update { changes, .. } = &filtered.ops[0] else {
            panic!("expected Update");
        };
        assert_eq!(changes.len(), 2, "the GM-only descendant change is dropped");
        let sheet = changes.iter().find(|c| c.path == "/system/sheet").unwrap();
        assert!(
            sheet.new.get("hidden").is_none(),
            "hidden child stripped from new"
        );
        assert!(
            sheet.old.get("hidden").is_none(),
            "hidden child stripped from old"
        );
        assert_eq!(sheet.new["shown"], serde_json::json!(30));
        let public = changes.iter().find(|c| c.path == "/system/public").unwrap();
        assert_eq!(public.new, serde_json::json!(40));

        // The GM sees every change unredacted.
        let gm_view = filter_command(&r, &cmd, &gm_ctx, &CapabilityGrants::default()).await;
        let Operation::Update { changes, .. } = &gm_view.ops[0] else {
            panic!("expected Update");
        };
        assert_eq!(changes.len(), 3);
    }
}
