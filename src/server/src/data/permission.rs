// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::BTreeSet;

use uuid::Uuid;

use crate::data::command::{Command, FieldChange, Operation};
use crate::data::document::{
    CapabilityGrants, CapabilityRequirement, DocRole, Document, Visibility, WorldCapDefaults,
    WorldRole,
};
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;

/// Built-in, server-understood capabilities. Modules may grant additional
/// namespaced capabilities (`<ns>:<verb>`); the server treats those as opaque
/// tokens and enforces only possession (Phase 2 gates custom actions).
pub mod cap {
    /// See the document at all (whole-doc egress gate).
    pub const READ: &str = "core:read";
    /// Write `/name`, `/engine/…`, `/system/…`, `/base` field paths.
    pub const WRITE_FIELDS: &str = "core:write_fields";
    /// Add/remove/replace embedded child documents.
    pub const MANAGE_EMBEDDED: &str = "core:manage_embedded";
    /// Delete the document.
    pub const DELETE: &str = "core:delete";
    /// Write `/permissions` and `/owner`.
    pub const EDIT_PERMISSIONS: &str = "core:edit_permissions";
    /// World-level: create a document of a doc_type (no document exists yet,
    /// so this is granted via `RoleCaps`, never per-document).
    pub const CREATE: &str = "core:create";
}

/// The `doc_type` whose ownership resolves through a link to an actor document.
pub const TOKEN_DOC_TYPE: &str = "token";

/// The actor document a token LINKS to (`engine.actor_id`), or `None` for a raw
/// or INSTANCED token. Mirrors the client's `resolveTokenActor`: only
/// `engine.actor_id` is a link — an instanced token's `embedded.actor[0]` is a
/// frozen placement-time copy and is deliberately NOT a link, so it can never
/// re-derive ownership from stale embedded state.
/// # Examples
///
/// ```
/// use shadowcat::data::document::Document;
/// use shadowcat::data::permission::token_actor_link;
///
/// let token: Document = serde_json::from_value(serde_json::json!({
///     "id": "00000000-0000-0000-0000-000000000001",
///     "scope": { "kind": "world", "world_id": "00000000-0000-0000-0000-0000000000aa" },
///     "doc_type": "token",
///     "schema_version": 1,
///     "engine": { "x": 0.0, "y": 0.0, "actor_id": "00000000-0000-0000-0000-000000000002" },
///     "system": {},
///     "created_at": 0,
///     "updated_at": 0
/// })).unwrap();
/// // Only a `token`'s engine.actor_id is a link; other doc_types yield None.
/// assert!(token_actor_link(&token).is_some());
/// ```
pub fn token_actor_link(doc: &Document) -> Option<Uuid> {
    if doc.doc_type != TOKEN_DOC_TYPE {
        return None;
    }
    doc.engine
        .as_ref()?
        .get("actor_id")?
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
}

/// The user a document effectively belongs to — the SINGLE ownership rule every
/// consumer (write authz, scene vision/mask ownership, redaction) resolves
/// through. A second, divergent notion of "owner" is a fork; there is one.
///
/// `doc.owner` is the explicit per-document override (`None` means unset, never
/// a sentinel colliding with a real user id). A `token` with no override
/// inherits its LINKED actor's owner, resolved LIVE on every call — nothing is
/// stamped, so re-assigning an actor's owner re-owns every linked token at once.
///
/// Fail-closed: no link, a dangling link (`linked_actor: None`), a
/// `linked_actor` that is not the document `token_actor_link` names or lives
/// in a different scope, and an unowned actor all resolve to `None` — no
/// owner, therefore no owner-derived capability. Degenerate input
/// under-permits, never default-allows.
///
/// `linked_actor` MUST be the document `token_actor_link(doc)` names; the
/// identity/type re-check below rejects any other document rather than trusting
/// the caller's join.
/// # Examples
///
/// ```
/// use shadowcat::data::document::Document;
/// use shadowcat::data::permission::effective_owner;
///
/// let mut token: Document = serde_json::from_value(serde_json::json!({
///     "id": "00000000-0000-0000-0000-000000000001",
///     "scope": { "kind": "world", "world_id": "00000000-0000-0000-0000-0000000000aa" },
///     "doc_type": "token",
///     "schema_version": 1,
///     "system": {},
///     "created_at": 0,
///     "updated_at": 0
/// })).unwrap();
///
/// // No own owner, no link, no actor: unowned.
/// assert_eq!(effective_owner(&token, None), None);
/// // The token's OWN owner always wins over any linked actor's.
/// let user = uuid::Uuid::new_v4();
/// token.owner = Some(user);
/// assert_eq!(effective_owner(&token, None), Some(user));
/// ```
pub fn effective_owner(doc: &Document, linked_actor: Option<&Document>) -> Option<Uuid> {
    if let Some(own) = doc.owner {
        return Some(own);
    }
    let link = token_actor_link(doc)?;
    let actor = linked_actor?;
    if actor.id != link || actor.doc_type != "actor" || actor.scope != doc.scope {
        return None;
    }
    actor.owner
}

/// `effective_owner` joined through a caller-supplied in-memory actor source
/// (the room's `SceneEcs` actor table on the WS/HTTP write-read hot paths, or
/// `|_| None` where no such table exists). Never queries the pool — egress
/// relies on this no-pool-query-on-hot-path property.
///
/// # Examples
///
/// ```text
/// // WS hot path: join through the room's in-memory actor table.
/// let owner = effective_owner_via(&token, |id| ecs.actor(id));
/// ```
pub fn effective_owner_via<'a>(
    doc: &Document,
    actor_lookup: &impl Fn(&Uuid) -> Option<&'a Document>,
) -> Option<Uuid> {
    let linked = token_actor_link(doc).and_then(|l| actor_lookup(&l));
    effective_owner(doc, linked)
}

/// Current documents for every `Update` op in `cmd`, keyed by `doc_id` (a
/// missing key means the document was deleted or never existed, and the op
/// is dropped by `filter_command`). Hoisted out of the redaction core so it
/// can be awaited ONCE, before any scene-guard scope is entered — still one
/// pool read per distinct Update doc per recipient (count-neutral vs. the
/// prior per-op `repo.get_document` inside the loop).
pub async fn load_update_docs(
    repo: &dyn Repository,
    cmd: &Command,
) -> std::collections::HashMap<Uuid, Document> {
    let mut out = std::collections::HashMap::new();
    for op in &cmd.ops {
        if let Operation::Update { doc_id, .. } = op {
            if !out.contains_key(doc_id) {
                if let Ok(Some(d)) = repo.get_document(*doc_id).await {
                    out.insert(*doc_id, d);
                }
            }
        }
    }
    out
}

/// The four CONTENT bands of a `Document`. Redaction operates on these and never on
/// the structural envelope (`id`, `scope`, `doc_type`, `schema_version`, `source`,
/// `owner`, `permissions`, `parent_id`, `embedded`, `created_at`, `updated_at`), whose
/// fields are either required or carry access-control meaning. Exactly the set
/// `required_cap_for_path` maps to `cap::WRITE_FIELDS` — which reads THIS array rather
/// than re-spelling it, so the writable set and the redactable set cannot drift apart.
pub const REDACTABLE_BANDS: [&str; 4] = ["name", "engine", "system", "base"];

/// Whether a content band is a CONTAINER, i.e. has an interior a JSON pointer can descend
/// into. `name` is a display string — a leaf — so `/name/...` names nothing at all. Both
/// the write-capability rule (`required_cap_for_path`) and the egress classifier
/// (`redaction_target`) read this one statement of the rule; each then applies its own
/// boundary handling to the residual segment, which is where the two legitimately differ.
fn band_has_interior(band: &str) -> bool {
    band != "name"
}

/// Whether `path` writes a content band whole, or writes into one.
///
/// Derived from `REDACTABLE_BANDS`: the band SET is stated once, so adding a fifth band
/// cannot make a path redactable without also making it writable under `cap::WRITE_FIELDS`.
/// Only the set and the leaf rule are shared with `redaction_target`; the residual-segment
/// rule is not, and must not be — an empty residual (`/system/`) is a writable path here and
/// an unclassifiable override key there, because a `FieldChange` path and a
/// `property_overrides` key are different fields on different structures with different
/// validators.
fn writes_a_content_band(path: &str) -> bool {
    let Some(rest) = path.strip_prefix('/') else {
        return false;
    };
    REDACTABLE_BANDS.iter().any(|band| {
        rest == *band
            || (band_has_interior(band)
                && rest
                    .strip_prefix(*band)
                    .is_some_and(|tail| tail.starts_with('/')))
    })
}

/// The capability required to write a document field at `path`, or `None` when
/// the path targets an immutable envelope field (not patchable via `Update`).
/// # Examples
///
/// ```
/// use shadowcat::data::permission::{cap, required_cap_for_path};
///
/// assert_eq!(required_cap_for_path("/system/hp"), Some(cap::WRITE_FIELDS));
/// assert_eq!(required_cap_for_path("/permissions/default"), Some(cap::EDIT_PERMISSIONS));
/// assert_eq!(required_cap_for_path("/source"), None); // immutable: no cap reaches it
/// ```
pub fn required_cap_for_path(path: &str) -> Option<&'static str> {
    if writes_a_content_band(path) {
        Some(cap::WRITE_FIELDS)
    } else if path == "/embedded" || path.starts_with("/embedded/") {
        Some(cap::MANAGE_EMBEDDED)
    } else if path == "/permissions" || path.starts_with("/permissions/") {
        Some(cap::EDIT_PERMISSIONS)
    } else if path == "/owner" {
        // `/owner` is the ownership override the effective-owner rule reads, so it
        // is an access-control field: writing it re-targets who may write the
        // document. Gated by EDIT_PERMISSIONS — which the `DocRole::Owner` floor
        // does NOT include — so an owner (effective or explicit) can never
        // re-assign, retain, or steal ownership; only a GM (or an explicit
        // EDIT_PERMISSIONS grant) can. A leaf: `/owner/...` has no sub-path.
        Some(cap::EDIT_PERMISSIONS)
    } else {
        None
    }
}

/// What a `property_overrides` pointer targets, and therefore how egress removes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedactionTarget {
    /// A whole band. Nulled in place: dropping the key would fail re-deserialization
    /// for a required field, and for an `Option` field would be indistinguishable from
    /// a document that never carried one, breaking the client's stable envelope shape.
    Band,
    /// A path inside a band, landing in an untyped `serde_json::Value` or an `Option`.
    /// Removed with a pointer strip, where callers rely on true key absence.
    Within,
}

/// Classify a `property_overrides` pointer, or `None` when nothing may redact it.
///
/// INVARIANT: a `Within` result guarantees the STRIP lands in untyped or optional data,
/// never on a required field — that is what makes it provably non-destructive to
/// deserialization. A `Band` result carries no such guarantee and does not need one: it
/// nulls the field in place, which is precisely why `system` (required, not an `Option`)
/// is handled that way rather than stripped.
///
/// Both properties are what the ingress gate and the egress filter must agree on. They
/// agree by reading THIS function: two implementations kept in sync only by inspection
/// can silently diverge on an input neither author checked, and reading one shared
/// function is what prevents that.
///
/// Two things are shared with `required_cap_for_path` as symbols rather than by
/// inspection: the band set (`REDACTABLE_BANDS`) and the leaf rule (`band_has_interior`,
/// which is why `/name/...` classifies as `None`). Everything else is deliberately
/// unshared, because the two classify different input domains: `required_cap_for_path`
/// classifies a `FieldChange` path, `redaction_target` classifies a `property_overrides`
/// map key. Same JSON-pointer syntax, different fields on different structures, gated by
/// different validators — so they are NOT required to agree string-for-string, and do not
/// (`/system/` is a writable path there and unclassifiable here). Only the band set and
/// the leaf rule must agree, and those are single symbols.
/// # Examples
///
/// ```
/// use shadowcat::data::permission::{redaction_target, RedactionTarget};
///
/// assert_eq!(redaction_target("/system"), Some(RedactionTarget::Band));
/// assert_eq!(redaction_target("/system/hp"), Some(RedactionTarget::Within));
/// assert_eq!(redaction_target("/permissions/default"), None);
/// ```
pub fn redaction_target(pointer: &str) -> Option<RedactionTarget> {
    let rest = pointer.strip_prefix('/')?;
    for band in REDACTABLE_BANDS {
        if rest == band {
            return Some(RedactionTarget::Band);
        }
        if band_has_interior(band) {
            if let Some(inner) = rest.strip_prefix(band) {
                if let Some(tail) = inner.strip_prefix('/') {
                    if !tail.is_empty() {
                        return Some(RedactionTarget::Within);
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod required_cap_tests {
    use super::*;

    #[test]
    fn engine_whole_and_subpaths_require_write_fields() {
        assert_eq!(required_cap_for_path("/engine"), Some(cap::WRITE_FIELDS));
        assert_eq!(required_cap_for_path("/engine/x"), Some(cap::WRITE_FIELDS));
        assert_eq!(
            required_cap_for_path("/engine/vision/0/range"),
            Some(cap::WRITE_FIELDS)
        );
    }

    #[test]
    fn engine_boundary_neighbor_does_not_match() {
        // `/engine_x` must not fall under the `/engine` prefix rule.
        assert_eq!(required_cap_for_path("/engine_x"), None);
    }

    #[test]
    fn name_requires_write_fields_but_is_a_leaf() {
        assert_eq!(required_cap_for_path("/name"), Some(cap::WRITE_FIELDS));
        // `/name` has no sub-paths — a leaf value, not a container.
        assert_eq!(required_cap_for_path("/name/first"), None);
        assert_eq!(required_cap_for_path("/named"), None);
    }

    #[test]
    fn base_whole_and_subpaths_require_write_fields() {
        assert_eq!(required_cap_for_path("/base"), Some(cap::WRITE_FIELDS));
        assert_eq!(
            required_cap_for_path("/base/system/hp"),
            Some(cap::WRITE_FIELDS)
        );
        assert_eq!(
            required_cap_for_path("/base/embedded/actor/0/name"),
            Some(cap::WRITE_FIELDS)
        );
    }

    #[test]
    fn base_boundary_neighbor_does_not_match() {
        assert_eq!(required_cap_for_path("/based"), None);
    }

    #[test]
    fn owner_requires_edit_permissions_and_is_a_leaf() {
        // Re-assigning ownership is an access-control write: EDIT_PERMISSIONS,
        // which the `DocRole::Owner` floor does not include.
        assert_eq!(required_cap_for_path("/owner"), Some(cap::EDIT_PERMISSIONS));
        assert_eq!(required_cap_for_path("/owner/id"), None);
        assert_eq!(required_cap_for_path("/owners"), None);
    }

    #[test]
    fn the_write_fields_band_set_equals_the_redactable_band_set() {
        // The two functions are not per-string equal and must not be tested that way
        // (`/system/` is `WRITE_FIELDS` for one and unclassifiable for the other). What
        // must hold is that they admit the same BAND SET, so a fifth band cannot become
        // redactable without also becoming writable under the same capability.
        //
        // The universe is HARDCODED, never derived from `REDACTABLE_BANDS`: probing only
        // the constant's own contents would make the assertion definitionally true for any
        // contents, and would not notice a band silently added to one side.
        let universe = [
            "name",
            "engine",
            "system",
            "base",
            "id",
            "scope",
            "doc_type",
            "schema_version",
            "source",
            "owner",
            "permissions",
            "parent_id",
            "embedded",
            "created_at",
            "updated_at",
        ];
        let writable: Vec<&str> = universe
            .into_iter()
            .filter(|f| required_cap_for_path(&format!("/{f}")) == Some(cap::WRITE_FIELDS))
            .collect();
        let redactable: Vec<&str> = universe
            .into_iter()
            .filter(|f| redaction_target(&format!("/{f}")).is_some())
            .collect();
        assert_eq!(
            writable, redactable,
            "the WRITE_FIELDS set and the redactable set diverged"
        );
        assert_eq!(
            writable,
            ["name", "engine", "system", "base"],
            "both sets changed together but are no longer the four content bands"
        );
    }

    #[test]
    fn source_is_immutable_no_cap() {
        // `/source` maps to no capability, so an Update targeting it is Forbidden for everyone.
        assert_eq!(required_cap_for_path("/source"), None);
        assert_eq!(required_cap_for_path("/source/id"), None);
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
/// # Examples
///
/// ```
/// use shadowcat::data::document::CapabilityRequirement;
/// use shadowcat::data::permission::declared_caps_for_path;
///
/// let reqs = vec![CapabilityRequirement {
///     path_prefix: "/engine/vision".into(),
///     caps: ["module:edit_vision".to_string()].into(),
/// }];
/// assert_eq!(declared_caps_for_path("/engine/vision/range", &reqs), vec!["module:edit_vision"]);
/// assert!(declared_caps_for_path("/system/hp", &reqs).is_empty());
/// ```
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
///
/// # Examples
///
/// ```
/// use shadowcat::data::document::CapabilityRequirement;
/// use shadowcat::data::permission::declared_caps_for_document;
///
/// let reqs = vec![CapabilityRequirement {
///     path_prefix: "/engine/vision".into(),
///     caps: ["module:edit_vision".to_string()].into(),
/// }];
/// // Create-time: a requirement applies iff its protected path is PRESENT in the body.
/// let with = serde_json::json!({ "engine": { "vision": { "range": 30 } } });
/// let without = serde_json::json!({ "engine": { "x": 0 } });
/// assert_eq!(declared_caps_for_document(&with, &reqs), vec!["module:edit_vision"]);
/// assert!(declared_caps_for_document(&without, &reqs).is_empty());
/// ```
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
/// unconditional short-circuit (holds every capability); when the document
/// caps the GM via `gm_role`, that GM resolves `caps`/`all` like any other
/// actor through the same role-floor logic (`all: false`, a real populated
/// `caps` set), so `caps` is not exclusively "for a non-GM". `see_gm_only`
/// drives `GmOnly` redaction and stays `true` for any `WorldRole::Gm` actor
/// regardless of `gm_role` — property-tier visibility is unaffected by the
/// whole-document capability cap. `is_owner` additionally admits the
/// `OwnerOrGm` tier (a player still sees their own hidden PC's name) WITHOUT
/// widening to `GmOnly`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Access {
    /// Capabilities this user holds on the document (unioned role + user grants).
    pub caps: BTreeSet<String>,
    /// Unconditional everything (the un-capped GM/admin short-circuit); `has`
    /// returns true for every capability when set.
    pub all: bool,
    /// Passes the `GmOnly` property tier; stays true for any `WorldRole::Gm`
    /// even when `gm_role` caps their whole-document access.
    pub see_gm_only: bool,
    /// The recipient is the document's EFFECTIVE owner; admits the `OwnerOrGm`
    /// property tier without widening to `GmOnly`.
    pub is_owner: bool,
}

impl Access {
    /// Whether the user holds capability `c` (GM holds everything).
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::permission::{cap, Access};
    ///
    /// let access = Access {
    ///     caps: [cap::READ.to_string()].into(),
    ///     all: false,
    ///     see_gm_only: false,
    ///     is_owner: false,
    /// };
    /// assert!(access.has(cap::READ));
    /// assert!(!access.has(cap::DELETE));
    /// ```
    pub fn has(&self, c: &str) -> bool {
        self.all || self.caps.contains(c)
    }

    /// Whether a property declared at visibility tier `v` is readable by this
    /// recipient. `GmOnly` requires the GM short-circuit; `OwnerOrGm` also admits
    /// the document owner. The single redaction predicate (`filter_properties`,
    /// `collect_hidden`).
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::document::Visibility;
    /// use shadowcat::data::permission::Access;
    ///
    /// let owner = Access { caps: Default::default(), all: false, see_gm_only: false, is_owner: true };
    /// assert!(owner.can_see(Visibility::OwnerOrGm)); // owner sees their own hidden fields
    /// assert!(!owner.can_see(Visibility::GmOnly));   // without widening to GM-only
    /// ```
    pub fn can_see(&self, v: Visibility) -> bool {
        match v {
            Visibility::All => true,
            Visibility::GmOnly => self.see_gm_only,
            Visibility::OwnerOrGm => self.see_gm_only || self.is_owner,
        }
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

/// The document-level role this actor effectively holds, or `None` when they
/// hold the unconditional GM/admin all-access short-circuit — no single role
/// applies, because every capability is granted regardless. Shared by
/// `resolve_access` (which turns this into an `Access`) and
/// `resolve_access_world` (which needs the SAME effective role to layer
/// world-default grants consistently — recomputing it independently from
/// `doc.permissions.default` would silently diverge for a GM whose access is
/// capped via `gm_role`).
fn effective_role(
    user: Uuid,
    world_role: WorldRole,
    doc: &Document,
    effective_owner: Option<Uuid>,
) -> Option<DocRole> {
    // Effective ownership of a TOKEN floors that user at `DocRole::Owner`
    // (READ + WRITE_FIELDS), so a player can move a token assigned to them
    // without the GM stamping a per-token `permissions.users` entry. Scoped to
    // `token`: on every other doc_type `owner` keeps its existing
    // provenance-only meaning and grants no capability. `DocRole` orders
    // Owner < Observer < None, so `min` only ever strengthens — a document's
    // own stronger grant is never downgraded by this floor.
    //
    // READ + WRITE_FIELDS is the BUILT-IN floor (`role_floor`) only. The floored
    // role is the same role that then selects additive capability grants, so an
    // effective owner also picks up `permissions.capabilities.by_role[Owner]` and
    // the world's `by_role[Owner]` — up to and including DELETE /
    // MANAGE_EMBEDDED / EDIT_PERMISSIONS if a deployment grants them there.
    // Intended: it is exactly what a stamped `permissions.users[user] = Owner`
    // yields, so inherited and stamped ownership cannot diverge. Nothing widens
    // out of the box (the shipped `by_role` maps are empty); a deployment that
    // populates `by_role[Owner]` is choosing to hand every Owner that capability.
    //
    // CONSEQUENCE a deployment must weigh before granting `by_role[Owner] ⊇
    // EDIT_PERMISSIONS`: that reaches `/owner`, so an effective owner can write
    // `/owner = self`, which PINS the token — the override now wins, and a GM
    // re-assigning the actor no longer re-owns it, silently defeating the
    // inheritance mechanism. The same grant reaches `/permissions`, so they can
    // also lock the GM out of the document entirely. Parity with a stamped owner
    // still holds exactly; what changed is the POPULATION — "Owner" here is
    // "every player with an assigned actor", not a hand-enumerated set.
    let owner_floor = doc.doc_type == TOKEN_DOC_TYPE && effective_owner == Some(user);
    let floor = |r: DocRole| {
        if owner_floor {
            r.min(DocRole::Owner)
        } else {
            r
        }
    };
    if world_role == WorldRole::Gm {
        let fallback = doc.permissions.gm_role?;
        return Some(floor(
            doc.permissions
                .users
                .get(&user)
                .copied()
                .unwrap_or(fallback),
        ));
    }
    Some(floor(
        doc.permissions
            .users
            .get(&user)
            .copied()
            .unwrap_or(doc.permissions.default),
    ))
}

/// Resolve a user's effective capabilities on a document. A world GM (or
/// server admin, which resolves to GM) holds every capability UNLESS the
/// document's `gm_role` caps them to an ordinary per-document role (see
/// `effective_role`) — used by restricted-audience chat messages. Otherwise
/// the user's `DocRole` (per-user, else the document default) seeds a
/// built-in floor that the document's additive grants (`by_role`, `by_user`)
/// widen.
///
/// The caller MUST resolve `effective_owner` from its source before calling:
/// the ECS actor table (ws egress), a batched prefetch (list route),
/// `effective_owner_of` / `load_effective_owner` (single-doc routes, search,
/// write path). Passing the literal `doc.owner` is correct ONLY for document
/// types that can never carry an actor link (e.g. scene pings, region
/// fields) — for any other type it under-permits, treating an inheriting
/// owner as a stranger.
/// # Examples
///
/// ```
/// use shadowcat::data::document::{Document, WorldRole};
/// use shadowcat::data::permission::{cap, resolve_access};
///
/// let doc: Document = serde_json::from_value(serde_json::json!({
///     "id": "00000000-0000-0000-0000-000000000001",
///     "scope": { "kind": "world", "world_id": "00000000-0000-0000-0000-0000000000aa" },
///     "doc_type": "note",
///     "schema_version": 1,
///     "system": {},
///     "created_at": 0,
///     "updated_at": 0
/// })).unwrap();
///
/// // Default PermissionSet denies: a player gets no READ (fail-closed) ...
/// let player = uuid::Uuid::new_v4();
/// assert!(!resolve_access(player, WorldRole::Player, &doc, None).has(cap::READ));
/// // ... while a GM (no gm_role cap set) short-circuits to everything.
/// assert!(resolve_access(player, WorldRole::Gm, &doc, None).all);
/// ```
pub fn resolve_access(
    user: Uuid,
    world_role: WorldRole,
    doc: &Document,
    effective_owner: Option<Uuid>,
) -> Access {
    let Some(role) = effective_role(user, world_role, doc, effective_owner) else {
        return Access {
            caps: BTreeSet::new(),
            all: true,
            see_gm_only: true,
            is_owner: true,
        };
    };
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
        // A GM capped via `gm_role` remains the GM for property-tier
        // (`GmOnly`/`OwnerOrGm`) visibility purposes even though their
        // whole-document READ is now floor-gated like anyone else's.
        see_gm_only: world_role == WorldRole::Gm,
        // Same rule as the capability floor: the OwnerOrGm redaction tier admits
        // the EFFECTIVE owner, so ownership means one thing across authz and
        // redaction.
        is_owner: effective_owner == Some(user),
    }
}

/// `resolve_access` plus a world's default capability grants, layered
/// additively on top of the per-document resolution (unaffected when
/// `resolve_access` already returned the unconditional GM short-circuit).
/// World defaults let a deployment grant, e.g., every Owner in a world
/// `core:manage_embedded` without editing each document. Uses the SAME
/// `effective_role` as `resolve_access` — including a `gm_role`-capped GM's
/// fallback role — so a world-level grant for that role also applies to them.
///
/// Same `effective_owner` resolution contract as `resolve_access`: the
/// caller must resolve it from its source, and a literal `doc.owner` is
/// correct only for document types that can never carry an actor link.
/// # Examples
///
/// ```
/// use shadowcat::data::document::{DocRole, Document, WorldCapDefaults, WorldRole};
/// use shadowcat::data::permission::resolve_access_world;
///
/// let mut doc: Document = serde_json::from_value(serde_json::json!({
///     "id": "00000000-0000-0000-0000-000000000001",
///     "scope": { "kind": "world", "world_id": "00000000-0000-0000-0000-0000000000aa" },
///     "doc_type": "note",
///     "schema_version": 1,
///     "system": {},
///     "created_at": 0,
///     "updated_at": 0
/// })).unwrap();
/// doc.permissions.default = DocRole::Observer;
///
/// // A world-level by_role grant layers onto the per-document role floor.
/// let mut defaults = WorldCapDefaults::default();
/// defaults.all.by_role.entry(DocRole::Observer).or_default().insert("module:x".into());
///
/// let player = uuid::Uuid::new_v4();
/// let grants = defaults.grants_for(&doc.doc_type); // world defaults, projected per doc_type
/// let access = resolve_access_world(player, WorldRole::Player, &doc, &grants, None);
/// assert!(access.has("module:x"));
/// ```
pub fn resolve_access_world(
    user: Uuid,
    world_role: WorldRole,
    doc: &Document,
    world_grants: &CapabilityGrants,
    effective_owner: Option<Uuid>,
) -> Access {
    let mut access = resolve_access(user, world_role, doc, effective_owner);
    if access.all {
        return access;
    }
    let role = effective_role(user, world_role, doc, effective_owner)
        .expect("access.all was false, so effective_role returned Some (see resolve_access)");
    if let Some(extra) = world_grants.by_role.get(&role) {
        access.caps.extend(extra.iter().cloned());
    }
    if let Some(extra) = world_grants.by_user.get(&user) {
        access.caps.extend(extra.iter().cloned());
    }
    access
}

/// Project world-default grants down to what a single user needs to replicate
/// access resolution client-side: the per-role tiers (world policy, no PII) plus
/// **only** this user's own per-user grants. Other users' UUIDs and grants are
/// dropped — the full `by_user` map must never cross to a client.
/// # Examples
///
/// ```
/// use shadowcat::data::document::CapabilityGrants;
/// use shadowcat::data::permission::project_grants_for;
///
/// let me = uuid::Uuid::new_v4();
/// let other = uuid::Uuid::new_v4();
/// let mut grants = CapabilityGrants::default();
/// grants.by_user.entry(me).or_default().insert("module:x".into());
/// grants.by_user.entry(other).or_default().insert("module:y".into());
///
/// // Only the recipient's own by_user entry survives projection.
/// let mine = project_grants_for(&grants, me);
/// assert!(mine.by_user.contains_key(&me));
/// assert!(!mine.by_user.contains_key(&other));
/// ```
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

/// A redaction input the classifier could not place in a content band. Egress
/// withholds rather than guessing: the alternatives are shipping a document whose
/// structural envelope was silently rewritten, or panicking the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactionError {
    /// The pointer that could not be classified.
    pub pointer: String,
}

impl std::fmt::Display for RedactionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unclassifiable redaction pointer {}", self.pointer)
    }
}

impl std::error::Error for RedactionError {}

/// Produce the recipient's view of a document: when `access.see_gm_only` is
/// false, strip every property whose override is `GmOnly`. Fails closed with
/// `RedactionError` when a `property_overrides` pointer cannot be classified by
/// `redaction_target` — withholding, never guessing at what the pointer meant.
/// # Examples
///
/// ```
/// use shadowcat::data::document::Document;
/// use shadowcat::data::permission::{filter_properties, Access};
///
/// let doc: Document = serde_json::from_value(serde_json::json!({
///     "id": "00000000-0000-0000-0000-000000000001",
///     "scope": { "kind": "world", "world_id": "00000000-0000-0000-0000-0000000000aa" },
///     "doc_type": "note",
///     "schema_version": 1,
///     "permissions": {
///         "default": "observer",
///         "users": {},
///         "property_overrides": { "/system/secret": "gm_only" },
///         "capabilities": { "by_role": {}, "by_user": {} },
///         "gm_role": null
///     },
///     "system": { "secret": "MOCK_SECRET_A", "public": 1 },
///     "created_at": 0,
///     "updated_at": 0
/// })).unwrap();
///
/// let observer = Access { caps: Default::default(), all: false, see_gm_only: false, is_owner: false };
/// let filtered = filter_properties(&doc, &observer).unwrap();
/// assert!(filtered.system.get("secret").is_none()); // stripped BEFORE transmission
/// assert_eq!(filtered.system["public"], 1);
/// ```
pub fn filter_properties(doc: &Document, access: &Access) -> Result<Document, RedactionError> {
    let mut out = doc.clone();
    if access.see_gm_only {
        return Ok(out);
    }
    // Each embedded child carries its own `property_overrides`, independent of the
    // parent's. A non-GM recipient must not see any GmOnly field at any depth, so
    // recurse with the same recipient access (the `see_gm_only` flag is the
    // recipient's, applied at every level) before stripping the parent's own.
    let embedded = std::mem::take(&mut out.embedded);
    out.embedded = embedded
        .into_iter()
        .map(|(k, v)| -> Result<_, RedactionError> {
            let children = v
                .into_iter()
                .map(|c| filter_properties(&c, access))
                .collect::<Result<Vec<_>, _>>()?;
            Ok((k, children))
        })
        .collect::<Result<_, _>>()?;
    let mut hidden: Vec<String> = doc
        .permissions
        .property_overrides
        .iter()
        .filter(|(_, v)| !access.can_see(**v))
        .map(|(p, _)| p.clone())
        .collect();
    // `base` is a historical snapshot of this doc's own (possibly hidden) bands — it is
    // hardcoded `OwnerOrGm` visibility, unconditional and non-overridable, independent
    // of `property_overrides`. Only the document's owner or a GM ever needs it to compute a
    // pull/push/revert; no other recipient should receive the raw snapshot.
    if !access.can_see(Visibility::OwnerOrGm) {
        hidden.push("/base".to_string());
    }
    let mut whole = serde_json::to_value(&out).expect("document serializes");
    for pointer in hidden {
        match redaction_target(&pointer) {
            Some(RedactionTarget::Band) => {
                if let Some(f) = whole.get_mut(&pointer[1..]) {
                    *f = serde_json::Value::Null;
                }
            }
            Some(RedactionTarget::Within) => strip_pointer(&mut whole, &pointer),
            None => return Err(RedactionError { pointer }),
        }
    }
    serde_json::from_value(whole).map_err(|_| RedactionError {
        pointer: "<document>".to_string(),
    })
}

/// Collect every property pointer in `doc` (and embedded descendants) that `access`
/// may NOT see — `GmOnly` for any non-GM, `OwnerOrGm` for a non-owner non-GM — each
/// expressed absolute to `doc` (call with `prefix = ""` for parent-absolute paths: a
/// child at `embedded[key][i]` contributes `/embedded/<key>/<i>{pointer}`). Lets
/// `Update`-delta redaction honor hidden fields at any embedded depth — the same
/// coverage `filter_properties` gives whole-document egress. Classifies each
/// UNPREFIXED override key via `redaction_target` before the prefix is applied
/// (the same classifier `filter_properties` runs on whole-document egress), and
/// fails closed with `RedactionError` on a pointer it cannot place — that is what
/// keeps the change-delta path from diverging from whole-document egress.
fn collect_hidden(
    doc: &Document,
    access: &Access,
    prefix: &str,
    out: &mut Vec<String>,
) -> Result<(), RedactionError> {
    for (p, v) in &doc.permissions.property_overrides {
        if !access.can_see(*v) {
            if redaction_target(p).is_none() {
                return Err(RedactionError { pointer: p.clone() });
            }
            out.push(format!("{prefix}{p}"));
        }
    }
    // Mirrors `filter_properties`' hardcoded `OwnerOrGm` policy for `/base` — see
    // that function's comment. This recursion structure means the push fires at every
    // embedded depth too (each recursive call gets its own `prefix`), covering an embedded
    // child's own `base` the same way, even though `base` is documented as top-level-only —
    // defense in depth costs nothing here.
    if !access.can_see(Visibility::OwnerOrGm) {
        out.push(format!("{prefix}/base"));
    }
    for (key, children) in &doc.embedded {
        for (idx, child) in children.iter().enumerate() {
            collect_hidden(
                child,
                access,
                &format!("{prefix}/embedded/{key}/{idx}"),
                out,
            )?;
        }
    }
    Ok(())
}

/// Whether a change path writes into any document's envelope `permissions` (top-level
/// or embedded) — a `permissions` path segment. Triggers retroactive redaction so a
/// just-hidden field is retracted from recipients who can no longer see it. A `system`
/// field literally named `permissions` over-triggers, which is safe (it only re-nulls
/// already-hidden fields).
fn touches_permissions(path: &str) -> bool {
    path.split('/').any(|seg| seg == "permissions")
}

/// The recipient's view of a broadcast command: ops on unreadable documents
/// are dropped, GmOnly properties/changes stripped. seq/world/author/ts are
/// preserved so the recipient's sequence guard never sees a false gap — a fully
/// redacted command keeps its seq with empty ops.
///
/// `effective_owner_via` joined through a caller-supplied in-memory actor
/// source, so this never queries the pool. Sync core: the loads
/// this needs (`current`, the Update pre-images) are hoisted to
/// `load_update_docs` and awaited by the caller BEFORE calling in, and the
/// actor join reads an in-memory table (the room's `SceneEcs`, or `|_| None`
/// where no actor table exists), never the pool.
pub fn filter_command<'a>(
    cmd: &Command,
    ctx: &PermissionContext,
    world_defaults: &WorldCapDefaults,
    current: &std::collections::HashMap<Uuid, Document>,
    actor_lookup: impl Fn(&Uuid) -> Option<&'a Document>,
) -> Command {
    // `world_defaults` is passed in (loaded once per connection / request) rather
    // than fetched here: this runs per event per recipient on the egress hot
    // path, and a per-event DB read contends with apply_intent on the
    // single-writer pool.
    let mut out_ops = Vec::with_capacity(cmd.ops.len());
    for op in &cmd.ops {
        match op {
            Operation::Create { doc } => {
                let owner = effective_owner_via(doc, &actor_lookup);
                let access = resolve_access_world(
                    ctx.user_id,
                    ctx.world_role,
                    doc,
                    &world_defaults.grants_for(&doc.doc_type),
                    owner,
                );
                if access.has(cap::READ) {
                    // Withhold rather than guess: a recipient who cannot be given a
                    // correctly redacted document is given none.
                    match filter_properties(doc, &access) {
                        Ok(filtered) => out_ops.push(Operation::Create { doc: filtered }),
                        Err(e) => {
                            tracing::warn!(doc_id = %doc.id, error = %e, "redaction failed; dropping Create op for recipient");
                        }
                    }
                }
            }
            Operation::Delete { doc } => {
                // A delete is visible to anyone who could read the document.
                let owner = effective_owner_via(doc, &actor_lookup);
                let access = resolve_access_world(
                    ctx.user_id,
                    ctx.world_role,
                    doc,
                    &world_defaults.grants_for(&doc.doc_type),
                    owner,
                );
                if access.has(cap::READ) {
                    match filter_properties(doc, &access) {
                        Ok(filtered) => out_ops.push(Operation::Delete { doc: filtered }),
                        Err(e) => {
                            tracing::warn!(doc_id = %doc.id, error = %e, "redaction failed; dropping Delete op for recipient");
                        }
                    }
                }
            }
            Operation::Update { doc_id, changes } => {
                // Absent = deleted (or never existed) between commit and this
                // recipient's redaction pass → drop, preserving today's semantics.
                let Some(cur) = current.get(doc_id) else {
                    continue;
                };
                let owner = effective_owner_via(cur, &actor_lookup);
                let access = resolve_access_world(
                    ctx.user_id,
                    ctx.world_role,
                    cur,
                    &world_defaults.grants_for(&cur.doc_type),
                    owner,
                );
                if !access.has(cap::READ) {
                    continue;
                }
                let kept: Vec<FieldChange> = if access.see_gm_only {
                    changes.clone()
                } else {
                    // Collect pointers this recipient cannot see across the parent AND
                    // its embedded descendants (parent-absolute), so an Update into
                    // `/embedded/<child>/...` is redacted against the child's own
                    // overrides — not just the parent's. Matches filter_properties.
                    let mut hidden = Vec::new();
                    if let Err(e) = collect_hidden(cur, &access, "", &mut hidden) {
                        tracing::warn!(doc_id = %doc_id, error = %e, "redaction failed; dropping Update op for recipient");
                        continue;
                    }
                    let mut kept: Vec<FieldChange> = changes
                        .iter()
                        .filter_map(|ch| redact_change(ch, &hidden))
                        .collect();
                    // When the command writes any `permissions`, retract every field this
                    // recipient cannot see so no stale value persists client-side. old:null
                    // keeps the pre-image from carrying the real value; new:null clears it.
                    // Idempotent (re-nulling an absent field is a no-op). Per-recipient: an
                    // owner's OwnerOrGm fields are absent from `hidden` (can_see), so intact.
                    if changes.iter().any(|c| touches_permissions(&c.path)) {
                        for ptr in hidden {
                            kept.push(FieldChange {
                                remove: false,
                                path: ptr,
                                old: serde_json::Value::Null,
                                new: serde_json::Value::Null,
                            });
                        }
                    }
                    kept
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
        // Preserve the removal flag: redacting a nested GM-only subtree out of an
        // ancestor-targeting change must not downgrade a key REMOVAL into a set-to-null
        // (`null` != absent) for the recipient. Only `old`/`new` are stripped.
        Some(FieldChange {
            remove: ch.remove,
            path: ch.path.clone(),
            old,
            new,
        })
    } else {
        Some(ch.clone())
    }
}

/// Remove the value a `RedactionTarget::Within` pointer names, if present.
///
/// Both the descent and the terminal step handle an ARRAY container by index, because a
/// pointer segment carries no evidence of which it names: `/system/inventory/0` is an
/// object key `"0"` for one document and an array index for the next, so refusing index
/// segments at ingress is not decidable and skipping them at egress ships the hidden value.
/// A no-op here is a silent fail-open — this is a secrecy gate, so every classified pointer
/// must actually be acted on.
///
/// The terminal step differs by container, deliberately:
/// - an object key is REMOVED (true absence, which is what `Within`'s callers rely on);
/// - an array element is set to `Null` in place, never removed. Removal renumbers every
///   later element, so a recipient's copy would disagree with the authoritative array on
///   what index a value lives at; `remove_pointer` refuses leaf index removal for that same
///   reason, leaving whole-array replacement as the only way an array changes length.
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
        let next = match cur {
            serde_json::Value::Object(m) => m.get_mut(tok),
            serde_json::Value::Array(a) => match tok.parse::<usize>() {
                Ok(idx) => a.get_mut(idx),
                Err(_) => return,
            },
            _ => return,
        };
        match next {
            Some(v) => cur = v,
            None => return,
        }
    }
    let last = &tokens[tokens.len() - 1];
    match cur {
        serde_json::Value::Object(m) => {
            m.remove(last);
        }
        serde_json::Value::Array(a) => {
            if let Ok(idx) = last.parse::<usize>() {
                if let Some(slot) = a.get_mut(idx) {
                    *slot = serde_json::Value::Null;
                }
            }
        }
        _ => {}
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
            name: None,
            source: None,
            base: None,
            owner: None,
            permissions: perms,
            embedded: Default::default(),
            parent_id: None,
            // "actor" is engine-defined; a minimal valid body so `Create`
            // clears the ingress gate. These tests exercise `/system`
            // redaction only, unrelated to `engine`'s content.
            engine: crate::data::document::tests::default_test_engine("actor"),
            system,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn perms_with(overrides: &[(&str, Visibility)]) -> PermissionSet {
        let mut p = PermissionSet::default();
        for (ptr, v) in overrides {
            p.property_overrides.insert((*ptr).into(), *v);
        }
        p
    }

    /// Build a `PermissionSet` carrying one override at `pointer`, hidden from non-GMs.
    fn perms_with_override(pointer: &str) -> PermissionSet {
        let mut p = PermissionSet {
            default: crate::data::document::DocRole::Observer,
            ..Default::default()
        };
        p.property_overrides.insert(
            pointer.to_string(),
            crate::data::document::Visibility::GmOnly,
        );
        p
    }

    fn non_gm() -> Access {
        Access {
            caps: Default::default(),
            all: false,
            see_gm_only: false,
            is_owner: false,
        }
    }

    #[test]
    fn filter_properties_errors_instead_of_panicking_on_a_nested_permissions_override() {
        // A nested `/permissions/...` override strips a `PermissionSet` field carrying
        // no serde default, so the value cannot re-deserialize.
        let d = doc(
            perms_with_override("/permissions/default"),
            serde_json::json!({ "hp": 1 }),
        );
        let err = filter_properties(&d, &non_gm()).expect_err("must not deserialize");
        assert_eq!(err.pointer, "/permissions/default");
    }

    #[test]
    fn filter_properties_errors_on_a_whole_permissions_override() {
        // A whole `/permissions` override is refused as unclassifiable rather than
        // substituting the fail-closed default permission set for the real one: that
        // substitution does not panic, it ships a wrong document.
        let d = doc(
            perms_with_override("/permissions"),
            serde_json::json!({ "hp": 1 }),
        );
        assert!(filter_properties(&d, &non_gm()).is_err());
    }

    #[test]
    fn filter_properties_still_redacts_every_content_band() {
        for (pointer, check) in [
            ("/system/secret", "system"),
            ("/engine", "engine"),
            ("/name", "name"),
        ] {
            let mut d = doc(
                perms_with_override(pointer),
                serde_json::json!({ "secret": "MOCK_SECRET_A", "public": 1 }),
            );
            // A real name, so the "name" sub-case discriminates: `doc()` always
            // constructs `name: None`, and asserting `None` after redaction would
            // pass even if `/name` redaction never ran.
            d.name = Some("MOCK_NAME_A".into());
            let out = filter_properties(&d, &non_gm())
                .unwrap_or_else(|e| panic!("{pointer} must still redact cleanly: {e}"));
            match check {
                "system" => {
                    assert!(out.system.get("secret").is_none());
                    assert_eq!(out.system["public"], 1);
                }
                "engine" => assert!(out.engine.is_none()),
                "name" => assert!(out.name.is_none()),
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn a_gm_recipient_is_unaffected_by_an_unclassifiable_override() {
        // The GM short-circuit returns before any classification runs, so a GM never
        // loses a document to a poisoned override.
        let d = doc(
            perms_with_override("/permissions/default"),
            serde_json::json!({ "hp": 1 }),
        );
        let gm = Access {
            caps: Default::default(),
            all: true,
            see_gm_only: true,
            is_owner: false,
        };
        assert!(filter_properties(&d, &gm).is_ok());
    }

    #[test]
    fn owner_or_gm_visible_to_owner_and_gm_not_other_player() {
        let owner = Uuid::from_u128(1);
        let other = Uuid::from_u128(2);
        let mut d = doc(
            perms_with(&[("/system/name", Visibility::OwnerOrGm)]),
            serde_json::json!({ "name": "Goblin Skirmisher", "displayName": "Goblin" }),
        );
        d.owner = Some(owner);

        // Owner (non-GM) sees the real name.
        let a_owner = resolve_access(owner, WorldRole::Player, &d, d.owner);
        assert_eq!(
            filter_properties(&d, &a_owner).unwrap().system["name"],
            "Goblin Skirmisher"
        );

        // Another player does NOT (falls back to the non-secret displayName).
        let a_other = resolve_access(other, WorldRole::Player, &d, d.owner);
        let v_other = filter_properties(&d, &a_other).unwrap();
        assert!(v_other.system.get("name").is_none());
        assert_eq!(v_other.system["displayName"], "Goblin");

        // GM sees it.
        let a_gm = resolve_access(other, WorldRole::Gm, &d, d.owner);
        assert_eq!(
            filter_properties(&d, &a_gm).unwrap().system["name"],
            "Goblin Skirmisher"
        );
    }

    #[test]
    fn owner_cannot_see_gm_only() {
        let owner = Uuid::from_u128(1);
        let mut d = doc(
            perms_with(&[
                ("/system/name", Visibility::OwnerOrGm),
                ("/system/secret", Visibility::GmOnly),
            ]),
            serde_json::json!({ "name": "PC", "secret": "GM note" }),
        );
        d.owner = Some(owner);

        let a_owner = resolve_access(owner, WorldRole::Player, &d, d.owner);
        let v = filter_properties(&d, &a_owner).unwrap();
        assert_eq!(v.system["name"], "PC"); // owner sees OwnerOrGm
        assert!(v.system.get("secret").is_none()); // owner still denied GmOnly
    }

    #[test]
    fn embedded_owner_or_gm_redacted_for_non_owner() {
        let owner = Uuid::from_u128(1);
        let other = Uuid::from_u128(2);
        let child = doc(
            perms_with(&[("/system/name", Visibility::OwnerOrGm)]),
            serde_json::json!({ "name": "Hidden", "displayName": "Thing" }),
        );
        let mut parent = doc(PermissionSet::default(), serde_json::json!({}));
        parent.owner = Some(owner);
        parent.embedded.insert("actor".into(), vec![child]);

        let a_other = resolve_access(other, WorldRole::Player, &parent, parent.owner);
        let v = filter_properties(&parent, &a_other).unwrap();
        assert!(v.embedded["actor"][0].system.get("name").is_none());

        let a_owner = resolve_access(owner, WorldRole::Player, &parent, parent.owner);
        let vo = filter_properties(&parent, &a_owner).unwrap();
        assert_eq!(vo.embedded["actor"][0].system["name"], "Hidden");
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
        // Only this user's own per-user grant survives; the other user's UUID
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
            None,
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
        let owner = resolve_access(Uuid::from_u128(1), WorldRole::Player, &d, d.owner);
        assert!(owner.has(cap::READ) && owner.has(cap::WRITE_FIELDS));
        assert!(!owner.has(cap::MANAGE_EMBEDDED) && !owner.has(cap::DELETE));
        // Observer: read only.
        let obs = resolve_access(Uuid::from_u128(2), WorldRole::Player, &d, d.owner);
        assert!(obs.has(cap::READ) && !obs.has(cap::WRITE_FIELDS));
        // Stranger falls to default (None): nothing.
        let other = resolve_access(Uuid::from_u128(3), WorldRole::Player, &d, d.owner);
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
        let a = resolve_access(Uuid::from_u128(1), WorldRole::Player, &d, d.owner);
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

        let player = resolve_access(Uuid::from_u128(7), WorldRole::Player, &d, d.owner);
        let view = filter_properties(&d, &player).unwrap();
        assert_eq!(view.system.get("secret"), None);
        assert_eq!(view.system["public"], serde_json::json!(1));

        let gm = resolve_access(Uuid::from_u128(7), WorldRole::Gm, &d, d.owner);
        assert_eq!(
            filter_properties(&d, &gm).unwrap().system["secret"],
            serde_json::json!(42)
        );
    }

    #[test]
    fn whole_system_gm_only_nulls_rather_than_drops_the_required_field() {
        // A doc type (e.g. a secret region) may mark its ENTIRE `/system` body GmOnly,
        // not just a leaf field. `system` is a required `Document` field, so stripping
        // the key outright would make the redacted JSON fail to deserialize back into a
        // `Document` — it must be nulled instead, never dropped.
        let mut perms = PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        };
        perms
            .property_overrides
            .insert("/system".into(), Visibility::GmOnly);
        let d = doc(perms, serde_json::json!({ "secret": 42 }));

        let player = resolve_access(Uuid::from_u128(7), WorldRole::Player, &d, d.owner);
        let view = filter_properties(&d, &player).unwrap();
        assert_eq!(view.system, serde_json::Value::Null);

        let gm = resolve_access(Uuid::from_u128(7), WorldRole::Gm, &d, d.owner);
        assert_eq!(
            filter_properties(&d, &gm).unwrap().system["secret"],
            serde_json::json!(42)
        );
    }

    #[test]
    fn whole_engine_gm_only_nulls_rather_than_drops_the_field() {
        // `/engine` is an `Option<Value>` envelope field — nulling it under a
        // whole-band GmOnly override must round-trip exactly like `None`, not
        // strip the key outright (which would be indistinguishable from a doc
        // that carries no `engine` band at all, but is still safe to
        // deserialize either way since the field is optional).
        let mut perms = PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        };
        perms
            .property_overrides
            .insert("/engine".into(), Visibility::GmOnly);
        let mut d = doc(perms, serde_json::json!({}));
        d.engine = Some(serde_json::json!({ "x": 1.0, "y": 2.0 }));

        let player = resolve_access(Uuid::from_u128(7), WorldRole::Player, &d, d.owner);
        let view = filter_properties(&d, &player).unwrap();
        assert_eq!(view.engine, None);

        let gm = resolve_access(Uuid::from_u128(7), WorldRole::Gm, &d, d.owner);
        assert_eq!(
            filter_properties(&d, &gm).unwrap().engine,
            Some(serde_json::json!({ "x": 1.0, "y": 2.0 }))
        );
    }

    #[test]
    fn engine_leaf_gm_only_hides_the_leaf_but_not_a_boundary_neighbor() {
        // Boundary matching inside `/engine` must behave exactly like inside
        // `/system`: `/engine/vision` hides only that key, leaving a
        // string-prefixed sibling (`visionmode`) untouched.
        let mut perms = PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        };
        perms
            .property_overrides
            .insert("/engine/vision".into(), Visibility::GmOnly);
        let mut d = doc(perms, serde_json::json!({}));
        d.engine = Some(serde_json::json!({ "vision": 30, "visionmode": "dark" }));

        let player = resolve_access(Uuid::from_u128(7), WorldRole::Player, &d, d.owner);
        let view = filter_properties(&d, &player).unwrap();
        assert!(view.engine.as_ref().unwrap().get("vision").is_none());
        assert_eq!(view.engine.as_ref().unwrap()["visionmode"], "dark");

        let gm = resolve_access(Uuid::from_u128(7), WorldRole::Gm, &d, d.owner);
        assert_eq!(
            filter_properties(&d, &gm).unwrap().engine.unwrap()["vision"],
            30
        );
    }

    #[test]
    fn gm_only_array_element_is_nulled_in_place_for_non_gm() {
        // An override may name an ARRAY element inside a band (`/system/inventory/0`);
        // the classifier accepts it, so egress must actually redact it. The element is
        // nulled, never removed: removal shifts every later index, and an array shrinks
        // only by whole-array replacement (`remove_pointer` refuses index removal for the
        // same reason). Length and sibling positions are therefore part of the contract.
        let mut perms = PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        };
        perms
            .property_overrides
            .insert("/system/inventory/0".into(), Visibility::GmOnly);
        let d = doc(
            perms,
            serde_json::json!({ "inventory": ["MOCK_SECRET_A", "visible"] }),
        );

        let player = resolve_access(Uuid::from_u128(7), WorldRole::Player, &d, d.owner);
        let view = filter_properties(&d, &player).unwrap();
        assert_eq!(
            view.system["inventory"],
            serde_json::json!([null, "visible"]),
            "the hidden element must be nulled without shifting its siblings"
        );

        let gm = resolve_access(Uuid::from_u128(7), WorldRole::Gm, &d, d.owner);
        assert_eq!(
            filter_properties(&d, &gm).unwrap().system["inventory"],
            serde_json::json!(["MOCK_SECRET_A", "visible"])
        );
    }

    #[test]
    fn gm_only_key_beneath_an_array_element_is_stripped_for_non_gm() {
        // The same fail-open reaches the DESCENT step: an override may traverse an array
        // index on its way to an object key (`/system/inventory/0/secret`). The terminal
        // container is an object, so the key is genuinely removed; the sibling key and the
        // sibling element stay intact.
        let mut perms = PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        };
        perms
            .property_overrides
            .insert("/system/inventory/0/secret".into(), Visibility::GmOnly);
        let d = doc(
            perms,
            serde_json::json!({
                "inventory": [
                    { "secret": "MOCK_SECRET_A", "public": 1 },
                    { "secret": "MOCK_SECRET_B" }
                ]
            }),
        );

        let player = resolve_access(Uuid::from_u128(7), WorldRole::Player, &d, d.owner);
        let view = filter_properties(&d, &player).unwrap();
        assert_eq!(
            view.system["inventory"],
            serde_json::json!([{ "public": 1 }, { "secret": "MOCK_SECRET_B" }]),
            "only the pointed-at key is removed; the sibling element is untouched"
        );

        let gm = resolve_access(Uuid::from_u128(7), WorldRole::Gm, &d, d.owner);
        assert_eq!(
            filter_properties(&d, &gm).unwrap().system["inventory"][0]["secret"],
            serde_json::json!("MOCK_SECRET_A")
        );
    }

    #[test]
    fn a_gm_receives_every_band_unredacted_whatever_the_overrides_name() {
        // Whole-document equality, not per-pointer assertions: every band, including the
        // unconditional `/base` policy and an array-index override, must survive intact.
        //
        // This pins the OUTPUT rule, which is the part a change can break. It cannot pin
        // `filter_properties`' `see_gm_only` early return, because that return is not
        // observable: `can_see` yields `true` for a GM at every tier, so the hidden-pointer
        // set is empty and the loop is a no-op regardless. The early return is a hot-path
        // guard against the serialize/deserialize round-trip, not a visibility decision.
        let mut perms = PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        };
        for ptr in ["/system/inventory/0", "/system", "/engine/vision", "/name"] {
            perms
                .property_overrides
                .insert(ptr.into(), Visibility::GmOnly);
        }
        let mut d = doc(
            perms,
            serde_json::json!({ "inventory": ["MOCK_SECRET_A", "visible"] }),
        );
        d.name = Some("MOCK_NAME_A".into());
        d.engine = Some(serde_json::json!({ "vision": 30 }));
        d.base = Some(serde_json::json!({ "system": { "hp": 1 } }));

        let gm = resolve_access(Uuid::from_u128(7), WorldRole::Gm, &d, d.owner);
        assert_eq!(filter_properties(&d, &gm).unwrap(), d);
    }

    #[test]
    fn owner_or_gm_name_visible_to_owner_and_gm_not_other_player() {
        // `/name` mirrors the `/system/name` OwnerOrGm tier: an owner and the
        // GM see it; another player is redacted to `null` (not stripped, since
        // `name` is a top-level `Option` envelope field).
        let owner = Uuid::from_u128(1);
        let other = Uuid::from_u128(2);
        let mut d = doc(
            perms_with(&[("/name", Visibility::OwnerOrGm)]),
            serde_json::json!({}),
        );
        d.owner = Some(owner);
        d.name = Some("Goblin Skirmisher".into());

        let a_owner = resolve_access(owner, WorldRole::Player, &d, d.owner);
        assert_eq!(
            filter_properties(&d, &a_owner).unwrap().name.as_deref(),
            Some("Goblin Skirmisher")
        );

        let a_other = resolve_access(other, WorldRole::Player, &d, d.owner);
        assert_eq!(filter_properties(&d, &a_other).unwrap().name, None);

        let a_gm = resolve_access(other, WorldRole::Gm, &d, d.owner);
        assert_eq!(
            filter_properties(&d, &a_gm).unwrap().name.as_deref(),
            Some("Goblin Skirmisher")
        );
    }

    #[test]
    fn whole_name_gm_only_nulls_to_null() {
        let mut perms = PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        };
        perms
            .property_overrides
            .insert("/name".into(), Visibility::GmOnly);
        let mut d = doc(perms, serde_json::json!({}));
        d.name = Some("Strahd".into());

        let player = resolve_access(Uuid::from_u128(7), WorldRole::Player, &d, d.owner);
        assert_eq!(filter_properties(&d, &player).unwrap().name, None);

        let gm = resolve_access(Uuid::from_u128(7), WorldRole::Gm, &d, d.owner);
        assert_eq!(
            filter_properties(&d, &gm).unwrap().name.as_deref(),
            Some("Strahd")
        );
    }

    #[test]
    fn base_is_hardcoded_owner_or_gm_unconditional_of_overrides() {
        // `base` is a historical snapshot that may echo content hidden elsewhere in the
        // document (e.g. via `property_overrides`). Its own visibility is hardcoded
        // `OwnerOrGm` and must NOT depend on `property_overrides` at all — this doc has
        // NONE, proving the hiding isn't override-driven.
        let owner = Uuid::from_u128(1);
        let other = Uuid::from_u128(2);
        let mut d = doc(PermissionSet::default(), serde_json::json!({ "hp": 10 }));
        d.owner = Some(owner);
        d.base = Some(serde_json::json!({ "name": "Goblin", "system": { "hp": 10 } }));

        // Non-owner, non-GM: base is nulled.
        let a_other = resolve_access(other, WorldRole::Player, &d, d.owner);
        assert_eq!(filter_properties(&d, &a_other).unwrap().base, None);

        // Owner (non-GM): sees the real base.
        let a_owner = resolve_access(owner, WorldRole::Player, &d, d.owner);
        assert_eq!(filter_properties(&d, &a_owner).unwrap().base, d.base);

        // GM: sees the real base.
        let a_gm = resolve_access(other, WorldRole::Gm, &d, d.owner);
        assert_eq!(filter_properties(&d, &a_gm).unwrap().base, d.base);
    }

    #[tokio::test]
    async fn filter_command_update_drops_base_field_change_for_non_owner_non_gm() {
        // A field-level `/base` FieldChange in a broadcast Update must be entirely dropped
        // for a non-owner non-GM recipient (via `collect_hidden`/`redact_change`), but
        // passed through unchanged for the owner and for a GM.
        use crate::auth::role::ServerRole;
        use crate::data::command::{Command, FieldChange, Operation, WriteOrigin};
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
        let owner = r
            .create_user("owner", None, ServerRole::User, 0)
            .await
            .unwrap();

        let mut d = doc(
            PermissionSet {
                default: DocRole::Observer,
                ..Default::default()
            },
            serde_json::json!({ "hp": 10 }),
        );
        d.scope = Scope::World { world_id: w.id };
        d.owner = Some(owner);
        d.base = Some(serde_json::json!({ "system": { "hp": 5 } }));
        r.apply_intent(
            &gm_ctx,
            w.id,
            vec![Operation::Create { doc: d.clone() }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let cmd = Command {
            seq: 2,
            world_id: w.id,
            author: gm,
            ts: 0,
            ops: vec![Operation::Update {
                doc_id: d.id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/base".into(),
                    old: serde_json::json!({ "system": { "hp": 5 } }),
                    new: serde_json::json!({ "system": { "hp": 10 } }),
                }],
            }],
        };

        let current = load_update_docs(&r, &cmd).await;

        // Non-owner, non-GM: the change is dropped entirely.
        let other = PermissionContext {
            user_id: Uuid::from_u128(77),
            world_role: WorldRole::Player,
        };
        let out_other =
            filter_command(&cmd, &other, &WorldCapDefaults::default(), &current, |_| {
                None
            });
        let Operation::Update { changes, .. } = &out_other.ops[0] else {
            panic!("expected Update");
        };
        assert!(
            changes.is_empty(),
            "non-owner non-GM must not receive a /base FieldChange"
        );

        // Owner: passed through unchanged.
        let owner_ctx = PermissionContext {
            user_id: owner,
            world_role: WorldRole::Player,
        };
        let out_owner = filter_command(
            &cmd,
            &owner_ctx,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        let Operation::Update { changes, .. } = &out_owner.ops[0] else {
            panic!("expected Update");
        };
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "/base");
        assert_eq!(
            changes[0].new,
            serde_json::json!({ "system": { "hp": 10 } })
        );

        // GM: passed through unchanged.
        let out_gm = filter_command(
            &cmd,
            &gm_ctx,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        let Operation::Update { changes, .. } = &out_gm.ops[0] else {
            panic!("expected Update");
        };
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "/base");
    }

    #[test]
    fn collect_hidden_embedded_engine_override_is_prefixed() {
        // An embedded child's `/engine/...` override must surface, parent-
        // absolute, as `/embedded/<key>/<i>/engine/...` — the same coverage
        // `filter_properties` gives whole-document egress, needed by
        // `filter_command`'s Update-delta redaction.
        let mut child = doc(PermissionSet::default(), serde_json::json!({}));
        child.engine = Some(serde_json::json!({ "x": 1.0 }));
        child
            .permissions
            .property_overrides
            .insert("/engine/x".into(), Visibility::GmOnly);
        let mut parent = doc(PermissionSet::default(), serde_json::json!({}));
        parent.embedded.insert("actor".into(), vec![child]);

        let player = resolve_access(Uuid::from_u128(7), WorldRole::Player, &parent, parent.owner);
        let mut hidden = Vec::new();
        collect_hidden(&parent, &player, "", &mut hidden).unwrap();
        assert!(hidden.contains(&"/embedded/actor/0/engine/x".to_string()));
    }

    #[test]
    fn embedded_child_gm_only_is_stripped_for_non_gm() {
        let mut child = doc(
            PermissionSet::default(),
            serde_json::json!({ "secret": 9, "shown": 2 }),
        );
        child
            .permissions
            .property_overrides
            .insert("/system/secret".into(), Visibility::GmOnly);
        let mut parent = doc(
            PermissionSet {
                default: DocRole::Observer,
                ..Default::default()
            },
            serde_json::json!({ "public": 1 }),
        );
        parent.embedded.insert("items".into(), vec![child]);

        let player = resolve_access(Uuid::from_u128(7), WorldRole::Player, &parent, parent.owner);
        let view = filter_properties(&parent, &player).unwrap();
        let child_view = &view.embedded.get("items").unwrap()[0];
        assert_eq!(
            child_view.system.get("secret"),
            None,
            "child gm-only stripped"
        );
        assert_eq!(child_view.system["shown"], serde_json::json!(2));

        // The GM sees the embedded child's gm-only field.
        let gm = resolve_access(Uuid::from_u128(7), WorldRole::Gm, &parent, parent.owner);
        let gm_view = filter_properties(&parent, &gm).unwrap();
        assert_eq!(
            gm_view.embedded.get("items").unwrap()[0].system["secret"],
            serde_json::json!(9)
        );
    }

    #[test]
    fn redact_change_preserves_remove_flag_on_ancestor_of_hidden_leaf() {
        // A GM removes `/system/sheet` — a subtree that contains a nested gm_only leaf
        // `/system/sheet/hidden`. The redacted broadcast to a non-privileged recipient must
        // stay a REMOVAL (remove: true, new: Null), never downgrade to a set-to-null: the
        // latter would leave the key present-as-null on the recipient's client (the
        // `null` != absent violation this task exists to fix).
        let ch = FieldChange {
            remove: true,
            path: "/system/sheet".into(),
            old: serde_json::json!({ "shown": 1, "hidden": 42 }),
            new: serde_json::Value::Null,
        };
        let redacted = redact_change(&ch, &["/system/sheet/hidden".to_string()]).unwrap();
        assert!(redacted.remove, "removal flag preserved through redaction");
        assert_eq!(
            redacted.new,
            serde_json::Value::Null,
            "a removal carries no new value"
        );
        assert_eq!(
            redacted.old,
            serde_json::json!({ "shown": 1 }),
            "hidden leaf stripped from the pre-image; shown sibling remains"
        );
    }

    #[tokio::test]
    async fn filter_command_create_strips_embedded_gm_only() {
        use crate::auth::role::ServerRole;
        use crate::data::command::{Command, Operation};
        use crate::data::membership::PermissionContext;
        use crate::data::sqlite::SqliteRepository;

        let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = r
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = r.create_world_owned("W", gm, 0).await.unwrap();

        let mut child = doc(
            PermissionSet::default(),
            serde_json::json!({ "secret": 9, "shown": 2 }),
        );
        child
            .permissions
            .property_overrides
            .insert("/system/secret".into(), Visibility::GmOnly);
        let mut parent = doc(
            PermissionSet {
                default: DocRole::Observer,
                ..Default::default()
            },
            serde_json::json!({ "public": 1 }),
        );
        parent.scope = Scope::World { world_id: w.id };
        parent.embedded.insert("items".into(), vec![child]);

        let cmd = Command {
            seq: 1,
            world_id: w.id,
            author: gm,
            ts: 0,
            ops: vec![Operation::Create {
                doc: parent.clone(),
            }],
        };
        let player = PermissionContext {
            user_id: Uuid::from_u128(77),
            world_role: WorldRole::Player,
        };
        let current = load_update_docs(&r, &cmd).await;
        let filtered = filter_command(
            &cmd,
            &player,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        let Operation::Create { doc } = &filtered.ops[0] else {
            panic!("expected Create");
        };
        assert!(
            doc.embedded.get("items").unwrap()[0]
                .system
                .get("secret")
                .is_none(),
            "embedded child gm-only stripped on the Create broadcast"
        );
    }

    #[tokio::test]
    async fn filter_command_create_drops_op_entirely_for_default_none_region() {
        // A secret region declares `default: DocRole::None` (not just a
        // `/system` gm_only override), so `filter_command` must drop the Create op ENTIRELY
        // for a non-GM/non-owner recipient (no envelope at all — id/parent_id/existence must
        // never reach them), while a GM still receives the full op.
        use crate::auth::role::ServerRole;
        use crate::data::command::{Command, Operation};
        use crate::data::membership::PermissionContext;
        use crate::data::sqlite::SqliteRepository;

        let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = r
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = r.create_world_owned("W", gm, 0).await.unwrap();

        let mut region = doc(
            PermissionSet {
                default: DocRole::None,
                ..Default::default()
            },
            serde_json::json!({ "shape": "rect", "behavior": "arrest" }),
        );
        region.scope = Scope::World { world_id: w.id };
        region
            .permissions
            .property_overrides
            .insert("/system".into(), Visibility::GmOnly);

        let cmd = Command {
            seq: 1,
            world_id: w.id,
            author: gm,
            ts: 0,
            ops: vec![Operation::Create {
                doc: region.clone(),
            }],
        };

        let current = load_update_docs(&r, &cmd).await;
        let player = PermissionContext {
            user_id: Uuid::from_u128(77),
            world_role: WorldRole::Player,
        };
        let filtered_for_player = filter_command(
            &cmd,
            &player,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        assert!(
            filtered_for_player.ops.is_empty(),
            "a default:none secret region's Create op must be dropped entirely for a non-GM \
             recipient, not merely nulled — the doc's existence/id must not reach them"
        );

        let gm_ctx = PermissionContext {
            user_id: gm,
            world_role: WorldRole::Gm,
        };
        let filtered_for_gm = filter_command(
            &cmd,
            &gm_ctx,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        assert_eq!(
            filtered_for_gm.ops.len(),
            1,
            "the GM must still receive the region's Create op"
        );
        let Operation::Create { doc } = &filtered_for_gm.ops[0] else {
            panic!("expected Create");
        };
        assert_eq!(doc.system.get("behavior").unwrap(), "arrest");
    }

    #[tokio::test]
    async fn filter_command_drops_a_create_whose_redaction_cannot_be_classified() {
        // A poisoned document is withheld through `filter_command`, the per-recipient
        // broadcast egress path — not merely through the `filter_properties` unit
        // called directly by the other tests above.
        use crate::auth::role::ServerRole;
        use crate::data::command::{Command, Operation};
        use crate::data::membership::PermissionContext;
        use crate::data::sqlite::SqliteRepository;

        let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = r
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = r.create_world_owned("W", gm, 0).await.unwrap();

        let mut d = doc(
            perms_with_override("/permissions/default"),
            serde_json::json!({ "hp": 1 }),
        );
        d.scope = Scope::World { world_id: w.id };

        let cmd = Command {
            seq: 5,
            world_id: w.id,
            author: gm,
            ts: 0,
            ops: vec![Operation::Create { doc: d.clone() }],
        };
        let player = PermissionContext {
            user_id: Uuid::from_u128(77),
            world_role: WorldRole::Player,
        };
        let current = load_update_docs(&r, &cmd).await;
        let out = filter_command(
            &cmd,
            &player,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        assert!(
            out.ops.is_empty(),
            "the op must be withheld, not shipped half-redacted"
        );
        assert_eq!(
            out.seq, cmd.seq,
            "seq is preserved so the sequence guard sees no gap"
        );
    }

    #[tokio::test]
    async fn filter_command_drops_a_delete_whose_redaction_cannot_be_classified() {
        // Mirrors `filter_command_drops_a_create_whose_redaction_cannot_be_classified`
        // for the Delete arm, which has no other test poisoning its document.
        use crate::auth::role::ServerRole;
        use crate::data::command::{Command, Operation};
        use crate::data::membership::PermissionContext;
        use crate::data::sqlite::SqliteRepository;

        let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = r
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = r.create_world_owned("W", gm, 0).await.unwrap();

        let mut d = doc(
            perms_with_override("/permissions/default"),
            serde_json::json!({ "hp": 1 }),
        );
        d.scope = Scope::World { world_id: w.id };

        let cmd = Command {
            seq: 6,
            world_id: w.id,
            author: gm,
            ts: 0,
            ops: vec![Operation::Delete { doc: d.clone() }],
        };
        let player = PermissionContext {
            user_id: Uuid::from_u128(77),
            world_role: WorldRole::Player,
        };
        let current = load_update_docs(&r, &cmd).await;
        let out = filter_command(
            &cmd,
            &player,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        assert!(
            out.ops.is_empty(),
            "the op must be withheld, not shipped half-redacted"
        );
        assert_eq!(
            out.seq, cmd.seq,
            "seq is preserved so the sequence guard sees no gap"
        );
    }

    #[tokio::test]
    async fn filter_command_strips_and_preserves_seq() {
        use crate::auth::role::ServerRole;
        use crate::data::command::{Command, FieldChange, Operation, WriteOrigin};
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
        r.apply_intent(
            &gm_ctx,
            w.id,
            vec![Operation::Create { doc: d.clone() }],
            1,
            WriteOrigin::Client,
        )
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
                        remove: false,
                        path: "/system/secret".into(),
                        old: serde_json::json!(1),
                        new: serde_json::json!(9),
                    },
                    FieldChange {
                        remove: false,
                        path: "/system/public".into(),
                        old: serde_json::json!(2),
                        new: serde_json::json!(8),
                    },
                ],
            }],
        };

        let current = load_update_docs(&r, &cmd).await;

        // Player sees the public change only; seq is preserved.
        let player = PermissionContext {
            user_id: Uuid::from_u128(77),
            world_role: WorldRole::Player,
        };
        let filtered = filter_command(
            &cmd,
            &player,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        assert_eq!(filtered.seq, 2);
        if let Operation::Update { changes, .. } = &filtered.ops[0] {
            assert_eq!(changes.len(), 1);
            assert_eq!(changes[0].path, "/system/public");
        } else {
            panic!("expected Update");
        }

        // GM sees both changes.
        let gm_view = filter_command(
            &cmd,
            &gm_ctx,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        if let Operation::Update { changes, .. } = &gm_view.ops[0] {
            assert_eq!(changes.len(), 2);
        } else {
            panic!("expected Update");
        }
    }

    #[tokio::test]
    async fn permission_tightening_retracts_now_hidden_field_for_non_owner() {
        use crate::auth::role::ServerRole;
        use crate::data::command::{Command, FieldChange, Operation, WriteOrigin};
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
        // A real user — `owner_id` is a foreign key.
        let owner = r
            .create_user("owner", None, ServerRole::User, 0)
            .await
            .unwrap();

        // cur = post-apply doc: owner set, name present, /system/name now OwnerOrGm.
        let mut d = doc(
            PermissionSet {
                default: DocRole::Observer,
                ..Default::default()
            },
            serde_json::json!({ "name": "Goblin Skirmisher", "displayName": "Goblin" }),
        );
        d.scope = Scope::World { world_id: w.id };
        d.owner = Some(owner);
        d.permissions
            .property_overrides
            .insert("/system/name".into(), Visibility::OwnerOrGm);
        r.apply_intent(
            &gm_ctx,
            w.id,
            vec![Operation::Create { doc: d.clone() }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // The broadcast Update that tightened permissions (adds the name override).
        let cmd = Command {
            seq: 2,
            world_id: w.id,
            author: gm,
            ts: 0,
            ops: vec![Operation::Update {
                doc_id: d.id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/permissions/property_overrides".into(),
                    old: serde_json::json!({}),
                    new: serde_json::json!({ "/system/name": "owner_or_gm" }),
                }],
            }],
        };

        let current = load_update_docs(&r, &cmd).await;

        // Non-owner player: keeps the permission change PLUS a null retraction of /system/name.
        let other = PermissionContext {
            user_id: Uuid::from_u128(77),
            world_role: WorldRole::Player,
        };
        let out = filter_command(&cmd, &other, &WorldCapDefaults::default(), &current, |_| {
            None
        });
        let Operation::Update { changes, .. } = &out.ops[0] else {
            panic!("expected Update");
        };
        let retract = changes
            .iter()
            .find(|c| c.path == "/system/name")
            .expect("name retracted");
        assert_eq!(retract.new, serde_json::Value::Null);
        assert_eq!(retract.old, serde_json::Value::Null); // pre-image must not leak the real name

        // Owner: keeps the name (OwnerOrGm is visible) — no /system/name retraction.
        let owner_ctx = PermissionContext {
            user_id: owner,
            world_role: WorldRole::Player,
        };
        let out_owner = filter_command(
            &cmd,
            &owner_ctx,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        let Operation::Update { changes, .. } = &out_owner.ops[0] else {
            panic!("expected Update");
        };
        assert!(!changes.iter().any(|c| c.path == "/system/name"));

        // GM: sees everything; no synthesized retraction.
        let out_gm = filter_command(
            &cmd,
            &gm_ctx,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        let Operation::Update { changes, .. } = &out_gm.ops[0] else {
            panic!("expected Update");
        };
        assert!(!changes.iter().any(|c| c.path == "/system/name"));
    }

    #[tokio::test]
    async fn permission_tightening_retracts_embedded_owner_or_gm_for_non_owner() {
        use crate::auth::role::ServerRole;
        use crate::data::command::{Command, FieldChange, Operation, WriteOrigin};
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
        let owner = r
            .create_user("owner", None, ServerRole::User, 0)
            .await
            .unwrap();

        // Parent (owner set) embeds an actor copy whose name is OwnerOrGm-hidden.
        let mut child = doc(
            PermissionSet::default(),
            serde_json::json!({ "name": "Goblin Skirmisher", "displayName": "Goblin" }),
        );
        child
            .permissions
            .property_overrides
            .insert("/system/name".into(), Visibility::OwnerOrGm);
        let mut parent = doc(
            PermissionSet {
                default: DocRole::Observer,
                ..Default::default()
            },
            serde_json::json!({ "public": 0 }),
        );
        parent.scope = Scope::World { world_id: w.id };
        parent.owner = Some(owner);
        parent.embedded.insert("actor".into(), vec![child]);
        r.apply_intent(
            &gm_ctx,
            w.id,
            vec![Operation::Create {
                doc: parent.clone(),
            }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // Tighten the embedded child's permissions (adds the name override).
        let cmd = Command {
            seq: 2,
            world_id: w.id,
            author: gm,
            ts: 0,
            ops: vec![Operation::Update {
                doc_id: parent.id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/embedded/actor/0/permissions/property_overrides".into(),
                    old: serde_json::json!({}),
                    new: serde_json::json!({ "/system/name": "owner_or_gm" }),
                }],
            }],
        };

        let current = load_update_docs(&r, &cmd).await;

        // Non-owner player: the embedded name is retracted with a null pre-image.
        let other = PermissionContext {
            user_id: Uuid::from_u128(77),
            world_role: WorldRole::Player,
        };
        let out = filter_command(&cmd, &other, &WorldCapDefaults::default(), &current, |_| {
            None
        });
        let Operation::Update { changes, .. } = &out.ops[0] else {
            panic!("expected Update");
        };
        let retract = changes
            .iter()
            .find(|c| c.path == "/embedded/actor/0/system/name")
            .expect("embedded name retracted");
        assert_eq!(retract.new, serde_json::Value::Null);
        assert_eq!(retract.old, serde_json::Value::Null);

        // Owner: the embedded OwnerOrGm name stays visible — no retraction.
        let owner_ctx = PermissionContext {
            user_id: owner,
            world_role: WorldRole::Player,
        };
        let out_owner = filter_command(
            &cmd,
            &owner_ctx,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        let Operation::Update { changes, .. } = &out_owner.ops[0] else {
            panic!("expected Update");
        };
        assert!(!changes
            .iter()
            .any(|c| c.path == "/embedded/actor/0/system/name"));

        // GM: sees everything — no retraction.
        let out_gm = filter_command(
            &cmd,
            &gm_ctx,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        let Operation::Update { changes, .. } = &out_gm.ops[0] else {
            panic!("expected Update");
        };
        assert!(!changes
            .iter()
            .any(|c| c.path == "/embedded/actor/0/system/name"));
    }

    #[tokio::test]
    async fn filter_command_update_redacts_embedded_child_gm_only() {
        use crate::auth::role::ServerRole;
        use crate::data::command::{Command, FieldChange, Operation, WriteOrigin};
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

        let mut child = doc(
            PermissionSet::default(),
            serde_json::json!({ "secret": 1, "shown": 2 }),
        );
        child
            .permissions
            .property_overrides
            .insert("/system/secret".into(), Visibility::GmOnly);
        let mut parent = doc(
            PermissionSet {
                default: DocRole::Observer,
                ..Default::default()
            },
            serde_json::json!({ "public": 0 }),
        );
        parent.scope = Scope::World { world_id: w.id };
        parent.embedded.insert("items".into(), vec![child]);
        r.apply_intent(
            &gm_ctx,
            w.id,
            vec![Operation::Create {
                doc: parent.clone(),
            }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let cmd = Command {
            seq: 2,
            world_id: w.id,
            author: gm,
            ts: 0,
            ops: vec![Operation::Update {
                doc_id: parent.id,
                changes: vec![
                    // Direct write of the embedded child's gm-only field → dropped.
                    FieldChange {
                        remove: false,
                        path: "/embedded/items/0/system/secret".into(),
                        old: serde_json::json!(1),
                        new: serde_json::json!(9),
                    },
                    // Wholesale rewrite of the child's /system (ancestor of the gm-only
                    // leaf) → the hidden leaf is stripped from old + new, sibling kept.
                    FieldChange {
                        remove: false,
                        path: "/embedded/items/0/system".into(),
                        old: serde_json::json!({ "secret": 1, "shown": 2 }),
                        new: serde_json::json!({ "secret": 9, "shown": 3 }),
                    },
                    // Unrelated public parent field → kept.
                    FieldChange {
                        remove: false,
                        path: "/system/public".into(),
                        old: serde_json::json!(0),
                        new: serde_json::json!(5),
                    },
                ],
            }],
        };

        let current = load_update_docs(&r, &cmd).await;
        let player = PermissionContext {
            user_id: Uuid::from_u128(77),
            world_role: WorldRole::Player,
        };
        let filtered = filter_command(
            &cmd,
            &player,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        let Operation::Update { changes, .. } = &filtered.ops[0] else {
            panic!("expected Update");
        };
        assert_eq!(
            changes.len(),
            2,
            "the direct gm-only embedded change is dropped"
        );
        let sys = changes
            .iter()
            .find(|c| c.path == "/embedded/items/0/system")
            .unwrap();
        assert!(sys.new.get("secret").is_none(), "secret stripped from new");
        assert!(sys.old.get("secret").is_none(), "secret stripped from old");
        assert_eq!(sys.new["shown"], serde_json::json!(3));
        assert!(changes.iter().any(|c| c.path == "/system/public"));

        // GM sees all three unredacted.
        let gm_view = filter_command(
            &cmd,
            &gm_ctx,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        let Operation::Update { changes, .. } = &gm_view.ops[0] else {
            panic!("expected Update");
        };
        assert_eq!(changes.len(), 3);
    }

    #[tokio::test]
    async fn filter_command_nulls_a_gm_only_array_element_inside_an_ancestor_change() {
        // The delta path and whole-document egress must reach the same verdict on an
        // array-index override: a change writing the whole array carries the hidden element
        // in both `old` and `new`, so `redact_change` must null it in place there exactly as
        // `filter_properties` does on the whole document. Length and sibling positions are
        // preserved, so the recipient's indices still agree with the authoritative array.
        use crate::auth::role::ServerRole;
        use crate::data::command::{Command, FieldChange, Operation, WriteOrigin};
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
            serde_json::json!({ "inventory": ["MOCK_SECRET_A", "visible"] }),
        );
        d.scope = Scope::World { world_id: w.id };
        d.permissions
            .property_overrides
            .insert("/system/inventory/0".into(), Visibility::GmOnly);
        // Ingress accepts the override, which is what obliges egress to act on it.
        r.apply_intent(
            &gm_ctx,
            w.id,
            vec![Operation::Create { doc: d.clone() }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let cmd = Command {
            seq: 2,
            world_id: w.id,
            author: gm,
            ts: 0,
            ops: vec![Operation::Update {
                doc_id: d.id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/system/inventory".into(),
                    old: serde_json::json!(["MOCK_SECRET_A", "visible"]),
                    new: serde_json::json!(["MOCK_SECRET_B", "also visible"]),
                }],
            }],
        };

        let current = load_update_docs(&r, &cmd).await;
        let player = PermissionContext {
            user_id: Uuid::from_u128(77),
            world_role: WorldRole::Player,
        };
        let filtered = filter_command(
            &cmd,
            &player,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        let Operation::Update { changes, .. } = &filtered.ops[0] else {
            panic!("expected Update");
        };
        assert_eq!(changes[0].new, serde_json::json!([null, "also visible"]));
        assert_eq!(changes[0].old, serde_json::json!([null, "visible"]));

        let gm_view = filter_command(
            &cmd,
            &gm_ctx,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        let Operation::Update { changes, .. } = &gm_view.ops[0] else {
            panic!("expected Update");
        };
        assert_eq!(
            changes[0].new,
            serde_json::json!(["MOCK_SECRET_B", "also visible"])
        );
    }

    #[test]
    fn gm_role_denies_gm_unless_individually_granted() {
        let owner = Uuid::from_u128(1);
        let gm = Uuid::from_u128(2);
        let mut perms = PermissionSet {
            default: DocRole::None,
            gm_role: Some(DocRole::None),
            ..Default::default()
        };
        perms.users.insert(owner, DocRole::Owner);
        let d = doc(perms, serde_json::json!({}));

        // A GM not individually listed gets nothing — gm_role caps them like any other actor.
        let a_gm = resolve_access(gm, WorldRole::Gm, &d, d.owner);
        assert!(
            !a_gm.has(cap::READ),
            "unlisted GM must not read a gm_role:None document"
        );
        assert!(
            !a_gm.all,
            "gm_role:Some(_) must not grant the unconditional short-circuit"
        );

        // The owner is unaffected.
        let a_owner = resolve_access(owner, WorldRole::Player, &d, d.owner);
        assert!(a_owner.has(cap::READ));
    }

    #[test]
    fn gm_role_denies_but_admits_a_gm_individually_listed() {
        let owner = Uuid::from_u128(1);
        let gm = Uuid::from_u128(2);
        let mut perms = PermissionSet {
            default: DocRole::None,
            gm_role: Some(DocRole::None),
            ..Default::default()
        };
        perms.users.insert(owner, DocRole::Owner);
        perms.users.insert(gm, DocRole::Observer); // e.g. a whisper naming the GM
        let d = doc(perms, serde_json::json!({}));

        let a_gm = resolve_access(gm, WorldRole::Gm, &d, d.owner);
        assert!(
            a_gm.has(cap::READ),
            "a GM individually listed in `users` must read despite gm_role:None"
        );
        assert!(
            !a_gm.all,
            "still not the unconditional short-circuit — just an ordinary Observer grant"
        );
    }

    #[test]
    fn gm_role_option_none_default_preserves_unconditional_gm_access() {
        let owner = Uuid::from_u128(1);
        let gm = Uuid::from_u128(2);
        let mut perms = PermissionSet {
            default: DocRole::None,
            gm_role: None, // the field's actual default — Option::None, not Some(DocRole::None)
            ..Default::default()
        };
        perms.users.insert(owner, DocRole::Owner);
        let d = doc(perms, serde_json::json!({}));

        let a_gm = resolve_access(gm, WorldRole::Gm, &d, d.owner);
        assert!(
            a_gm.all,
            "gm_role: None (the default) must preserve the unconditional GM short-circuit \
             even when the document's own default/users would otherwise deny access"
        );
    }

    #[test]
    fn gm_role_observer_grants_any_gm_without_explicit_listing() {
        let owner = Uuid::from_u128(1);
        let gm = Uuid::from_u128(2);
        let stranger = Uuid::from_u128(3);
        let mut perms = PermissionSet {
            default: DocRole::None,
            gm_role: Some(DocRole::Observer),
            ..Default::default()
        };
        perms.users.insert(owner, DocRole::Owner);
        let d = doc(perms, serde_json::json!({}));

        // Any GM reads, even without being individually listed (dynamic resolution).
        let a_gm = resolve_access(gm, WorldRole::Gm, &d, d.owner);
        assert!(a_gm.has(cap::READ));
        assert!(a_gm.see_gm_only, "still a GM for property-tier purposes");

        // A non-owner, non-GM Player reads nothing.
        let a_stranger = resolve_access(stranger, WorldRole::Player, &d, d.owner);
        assert!(!a_stranger.has(cap::READ));
    }

    #[test]
    fn resolve_access_world_layers_world_grants_using_the_gm_role_fallback() {
        use crate::data::document::CapabilityGrants;
        let owner = Uuid::from_u128(1);
        let gm = Uuid::from_u128(2);
        let mut perms = PermissionSet {
            default: DocRole::None,
            gm_role: Some(DocRole::Observer),
            ..Default::default()
        };
        perms.users.insert(owner, DocRole::Owner);
        let d = doc(perms, serde_json::json!({}));

        let mut world_grants = CapabilityGrants::default();
        world_grants
            .by_role
            .entry(DocRole::Observer)
            .or_default()
            .insert("dnd5e:extra".to_string());

        // A GM not individually listed still resolves via the gm_role (Observer)
        // fallback, so world-level Observer grants must layer on top of it too —
        // not just `doc.permissions.default` (None here, which carries no such
        // grant). Proves resolve_access_world uses the SAME effective role as
        // resolve_access rather than recomputing it independently.
        let a_gm = resolve_access_world(gm, WorldRole::Gm, &d, &world_grants, d.owner);
        assert!(
            a_gm.has("dnd5e:extra"),
            "world grant for the gm_role fallback role must apply"
        );
    }

    #[tokio::test]
    async fn filter_command_redacts_nested_gm_only_paths() {
        use crate::auth::role::ServerRole;
        use crate::data::command::{Command, FieldChange, Operation, WriteOrigin};
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
        r.apply_intent(
            &gm_ctx,
            w.id,
            vec![Operation::Create { doc: d.clone() }],
            1,
            WriteOrigin::Client,
        )
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
                        remove: false,
                        path: "/system/secret/value".into(),
                        old: serde_json::json!(1),
                        new: serde_json::json!(9),
                    },
                    // Ancestor of a GM-only pointer → hidden child stripped from
                    // both pre-image and new value, siblings preserved.
                    FieldChange {
                        remove: false,
                        path: "/system/sheet".into(),
                        old: serde_json::json!({ "hidden": 2, "shown": 3 }),
                        new: serde_json::json!({ "hidden": 20, "shown": 30 }),
                    },
                    // Unrelated public field → kept whole.
                    FieldChange {
                        remove: false,
                        path: "/system/public".into(),
                        old: serde_json::json!(4),
                        new: serde_json::json!(40),
                    },
                ],
            }],
        };

        let current = load_update_docs(&r, &cmd).await;
        let player = PermissionContext {
            user_id: Uuid::from_u128(77),
            world_role: WorldRole::Player,
        };
        let filtered = filter_command(
            &cmd,
            &player,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
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
        let gm_view = filter_command(
            &cmd,
            &gm_ctx,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        let Operation::Update { changes, .. } = &gm_view.ops[0] else {
            panic!("expected Update");
        };
        assert_eq!(changes.len(), 3);
    }

    // ---- effective_owner: the single token-ownership rule ----

    fn token_linked_to(actor_id: Option<Uuid>) -> Document {
        let mut d = doc(PermissionSet::default(), serde_json::json!({}));
        d.id = Uuid::from_u128(100);
        d.doc_type = "token".into();
        d.engine = Some(match actor_id {
            Some(a) => serde_json::json!({
                "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0, "rotation": 0.0,
                "actor_id": a.to_string()
            }),
            None => serde_json::json!({
                "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0, "rotation": 0.0
            }),
        });
        d
    }

    fn actor_owned_by(id: Uuid, owner: Option<Uuid>) -> Document {
        let mut d = doc(PermissionSet::default(), serde_json::json!({}));
        d.id = id;
        d.owner = owner;
        d
    }

    #[test]
    fn token_actor_link_reads_only_a_tokens_engine_actor_id() {
        let a = Uuid::from_u128(42);
        assert_eq!(token_actor_link(&token_linked_to(Some(a))), Some(a));
        // A raw/instanced token carries no link.
        assert_eq!(token_actor_link(&token_linked_to(None)), None);
        // A non-token doc_type never links, even with a stray `actor_id` key.
        let mut impostor = token_linked_to(Some(a));
        impostor.doc_type = "actor".into();
        assert_eq!(token_actor_link(&impostor), None);
    }

    #[test]
    fn effective_owner_prefers_the_per_token_override() {
        let actor_id = Uuid::from_u128(42);
        let inheritor = Uuid::from_u128(1);
        let override_user = Uuid::from_u128(2);
        let actor = actor_owned_by(actor_id, Some(inheritor));

        // No override: inherits the linked actor's owner.
        let plain = token_linked_to(Some(actor_id));
        assert_eq!(effective_owner(&plain, Some(&actor)), Some(inheritor));

        // Override set: it wins over the same actor, same link.
        let mut overridden = token_linked_to(Some(actor_id));
        overridden.owner = Some(override_user);
        assert_eq!(
            effective_owner(&overridden, Some(&actor)),
            Some(override_user)
        );
    }

    #[test]
    fn effective_owner_fails_closed_on_degenerate_links() {
        let actor_id = Uuid::from_u128(42);
        let player = Uuid::from_u128(1);

        // No link, no override.
        assert_eq!(effective_owner(&token_linked_to(None), None), None);
        // Dangling link: the actor row does not exist.
        assert_eq!(
            effective_owner(&token_linked_to(Some(actor_id)), None),
            None
        );
        // Linked to an actor that nobody owns.
        assert_eq!(
            effective_owner(
                &token_linked_to(Some(actor_id)),
                Some(&actor_owned_by(actor_id, None))
            ),
            None
        );
        // A `linked_actor` that is NOT the document the link names is rejected
        // rather than trusted — a mis-joined caller under-permits, never leaks
        // write authority to the wrong actor's owner.
        assert_eq!(
            effective_owner(
                &token_linked_to(Some(actor_id)),
                Some(&actor_owned_by(Uuid::from_u128(999), Some(player)))
            ),
            None
        );
        // Same, for a correctly-identified document of the wrong doc_type.
        let mut wrong_type = actor_owned_by(actor_id, Some(player));
        wrong_type.doc_type = "token".into();
        assert_eq!(
            effective_owner(&token_linked_to(Some(actor_id)), Some(&wrong_type)),
            None
        );
        // Control: the same call with the correctly-joined owned actor resolves,
        // so the rejections above are the guards, not a constant `None`.
        assert_eq!(
            effective_owner(
                &token_linked_to(Some(actor_id)),
                Some(&actor_owned_by(actor_id, Some(player)))
            ),
            Some(player)
        );
    }

    #[test]
    fn effective_owner_rejects_a_cross_scope_actor() {
        // A candidate from another scope is an illegitimate join, same class as a
        // wrong-id or wrong-type candidate: fail closed to no owner.
        let actor_id = Uuid::from_u128(42);
        let mut token = token_linked_to(Some(actor_id));
        token.scope = Scope::World {
            world_id: Uuid::from_u128(1000),
        };
        let mut foreign = actor_owned_by(actor_id, Some(Uuid::from_u128(1)));
        foreign.scope = Scope::World {
            world_id: Uuid::from_u128(2000),
        };
        assert_eq!(effective_owner(&token, Some(&foreign)), None);

        // Same scope still resolves.
        let mut same = actor_owned_by(actor_id, Some(Uuid::from_u128(1)));
        same.scope = token.scope.clone();
        assert_eq!(
            effective_owner(&token, Some(&same)),
            Some(Uuid::from_u128(1))
        );
    }

    #[test]
    fn a_non_token_never_inherits_ownership() {
        let actor_id = Uuid::from_u128(42);
        let player = Uuid::from_u128(1);
        let mut not_a_token = token_linked_to(Some(actor_id));
        not_a_token.doc_type = "drawing".into();
        assert_eq!(
            effective_owner(&not_a_token, Some(&actor_owned_by(actor_id, Some(player)))),
            None,
            "inheritance is token-scoped: no other doc_type joins an actor"
        );
    }

    #[test]
    fn effective_ownership_grants_the_owner_floor_and_the_owner_or_gm_tier() {
        let actor_id = Uuid::from_u128(42);
        let player = Uuid::from_u128(1);
        let stranger = Uuid::from_u128(2);
        let mut token = token_linked_to(Some(actor_id));
        // The shipping `buildTokenDoc` default: READ-only for everyone.
        token.permissions.default = DocRole::Observer;
        let actor = actor_owned_by(actor_id, Some(player));
        let owner = effective_owner(&token, Some(&actor));

        let a_player = resolve_access(player, WorldRole::Player, &token, owner);
        assert!(
            a_player.has(cap::READ) && a_player.has(cap::WRITE_FIELDS),
            "an effective owner holds the DocRole::Owner floor"
        );
        assert!(
            !a_player.has(cap::EDIT_PERMISSIONS) && !a_player.has(cap::DELETE),
            "the BUILT-IN floor stops at WRITE_FIELDS — no re-assigning or deleting. \
             Additive `by_role[Owner]` grants can widen past it; see \
             `world_by_role_owner_grants_reach_an_inheriting_owner`"
        );
        assert!(
            a_player.is_owner && a_player.can_see(Visibility::OwnerOrGm),
            "redaction's OwnerOrGm tier admits the same effective owner"
        );
        assert!(
            !a_player.can_see(Visibility::GmOnly),
            "an owner is not a GM"
        );

        // Non-vacuity: same token, same call, different user.
        let a_stranger = resolve_access(stranger, WorldRole::Player, &token, owner);
        assert!(a_stranger.has(cap::READ) && !a_stranger.has(cap::WRITE_FIELDS));
        assert!(!a_stranger.is_owner);
    }

    #[test]
    fn the_owner_floor_never_downgrades_a_stronger_document_grant() {
        // A doc that already grants a user Owner keeps it when they are NOT the
        // effective owner: the floor only ever strengthens.
        let player = Uuid::from_u128(1);
        let mut token = token_linked_to(None);
        token.permissions.users.insert(player, DocRole::Owner);
        let a = resolve_access(player, WorldRole::Player, &token, None);
        assert!(a.has(cap::WRITE_FIELDS));
        assert!(
            !a.is_owner,
            "no effective owner => not the OwnerOrGm subject"
        );
    }

    #[test]
    fn world_by_role_owner_grants_reach_an_inheriting_owner() {
        use crate::data::document::CapabilityGrants;
        // The owner floor sets the ROLE, and that role also selects additive
        // capability grants — so an INHERITING owner receives `by_role[Owner]`
        // exactly as a stamped `permissions.users[user] = Owner` would. That
        // equivalence is the point: inherited and stamped ownership must not
        // diverge. A deployment that puts EDIT_PERMISSIONS in `by_role[Owner]`
        // is choosing to hand it to every Owner, inheriting ones included; this
        // test documents that as intended, not as an accident of the floor.
        let actor_id = Uuid::from_u128(42);
        let player = Uuid::from_u128(1);
        let stranger = Uuid::from_u128(2);
        let mut token = token_linked_to(Some(actor_id));
        token.permissions.default = DocRole::Observer;
        let actor = actor_owned_by(actor_id, Some(player));
        let owner = effective_owner(&token, Some(&actor));

        let mut world_grants = CapabilityGrants::default();
        world_grants
            .by_role
            .entry(DocRole::Owner)
            .or_default()
            .insert(cap::EDIT_PERMISSIONS.to_string());

        let inheriting =
            resolve_access_world(player, WorldRole::Player, &token, &world_grants, owner);
        assert!(
            inheriting.has(cap::EDIT_PERMISSIONS),
            "a world by_role[Owner] grant reaches an owner who inherits through the actor link"
        );

        // Equivalence leg: a STAMPED owner on the same doc, with no effective
        // owner at all, resolves the identical grant — the two paths agree.
        let mut stamped = token_linked_to(None);
        stamped.permissions.default = DocRole::Observer;
        stamped.permissions.users.insert(player, DocRole::Owner);
        assert!(resolve_access_world(
            player,
            WorldRole::Player,
            &stamped,
            &world_grants,
            stamped.owner
        )
        .has(cap::EDIT_PERMISSIONS));

        // Non-vacuity: the grant is role-selected, not unconditional — a
        // non-owner on the same document with the same world grants gets nothing.
        assert!(
            !resolve_access_world(stranger, WorldRole::Player, &token, &world_grants, owner)
                .has(cap::EDIT_PERMISSIONS)
        );
    }

    // ---- filter_command joins the effective owner (egress hot path) ----

    #[tokio::test]
    async fn filter_command_admits_the_inheriting_owner_of_a_linked_token() {
        // token: permissions.default = None, owner = None, linked to an actor owned
        // by P. Literal-owner egress treated P as a stranger (op dropped); the
        // effective join must now deliver Create/Update/Delete AND OwnerOrGm-tier
        // content to P, while a true stranger still receives nothing. A document
        // P can write (owner floor at apply_intent) is one P receives.
        let p = Uuid::from_u128(1);
        let stranger = Uuid::from_u128(2);
        let actor_id = Uuid::from_u128(42);
        let actor = actor_owned_by(actor_id, Some(p));
        let mut token = token_linked_to(Some(actor_id));
        token.permissions.default = DocRole::None;
        token
            .permissions
            .property_overrides
            .insert("/system/notes".into(), Visibility::OwnerOrGm);

        let cmd = Command {
            seq: 1,
            world_id: Uuid::from_u128(7),
            author: Uuid::from_u128(9),
            ts: 0,
            ops: vec![Operation::Create { doc: token.clone() }],
        };
        let lookup = |id: &Uuid| (id == &actor.id).then_some(&actor);
        let current = std::collections::HashMap::new();

        let p_ctx = PermissionContext {
            user_id: p,
            world_role: WorldRole::Player,
        };
        let out = filter_command(&cmd, &p_ctx, &WorldCapDefaults::default(), &current, lookup);
        assert_eq!(out.ops.len(), 1, "inheriting owner must RECEIVE the create");

        let s_ctx = PermissionContext {
            user_id: stranger,
            world_role: WorldRole::Player,
        };
        let out = filter_command(&cmd, &s_ctx, &WorldCapDefaults::default(), &current, lookup);
        assert!(
            out.ops.is_empty(),
            "a stranger still receives nothing (fail closed)"
        );

        // Without the actor join (dangling source) the op is withheld even from P:
        // degenerate input under-permits, never over-permits.
        let out = filter_command(&cmd, &p_ctx, &WorldCapDefaults::default(), &current, |_| {
            None
        });
        assert!(out.ops.is_empty());
    }

    #[tokio::test]
    async fn filter_command_update_keeps_owner_or_gm_changes_for_the_inheriting_owner() {
        use crate::auth::role::ServerRole;
        use crate::data::command::{Command, FieldChange, Operation, WriteOrigin};
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
        let p = r.create_user("p", None, ServerRole::User, 0).await.unwrap();

        let mut actor = doc(PermissionSet::default(), serde_json::json!({}));
        actor.id = Uuid::from_u128(42);
        actor.scope = Scope::World { world_id: w.id };
        actor.owner = Some(p);

        let mut token = doc(
            PermissionSet {
                default: DocRole::None,
                ..Default::default()
            },
            serde_json::json!({ "notes": "secret plan" }),
        );
        token.doc_type = "token".into();
        token.id = Uuid::from_u128(100);
        token.scope = Scope::World { world_id: w.id };
        token.engine = Some(serde_json::json!({
            "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0, "rotation": 0.0,
            "actor_id": actor.id.to_string()
        }));
        token
            .permissions
            .property_overrides
            .insert("/system/notes".into(), Visibility::OwnerOrGm);

        r.apply_intent(
            &gm_ctx,
            w.id,
            vec![
                Operation::Create { doc: actor.clone() },
                Operation::Create { doc: token.clone() },
            ],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let cmd = Command {
            seq: 2,
            world_id: w.id,
            author: gm,
            ts: 0,
            ops: vec![Operation::Update {
                doc_id: token.id,
                changes: vec![
                    FieldChange {
                        remove: false,
                        path: "/system/notes".into(),
                        old: serde_json::json!("secret plan"),
                        new: serde_json::json!("new plan"),
                    },
                    FieldChange {
                        remove: false,
                        path: "/base".into(),
                        old: serde_json::Value::Null,
                        new: serde_json::json!({ "system": { "notes": "template" } }),
                    },
                ],
            }],
        };

        let p_ctx = PermissionContext {
            user_id: p,
            world_role: WorldRole::Player,
        };
        let current = load_update_docs(&r, &cmd).await;
        let out = filter_command(&cmd, &p_ctx, &WorldCapDefaults::default(), &current, |id| {
            (id == &actor.id).then_some(&actor)
        });
        let Operation::Update { changes, .. } = &out.ops[0] else {
            panic!("expected Update");
        };
        assert_eq!(
            changes.len(),
            2,
            "the inheriting owner keeps both the OwnerOrGm /system/notes change and /base"
        );

        let stranger_ctx = PermissionContext {
            user_id: Uuid::from_u128(999),
            world_role: WorldRole::Player,
        };
        let out = filter_command(
            &cmd,
            &stranger_ctx,
            &WorldCapDefaults::default(),
            &current,
            |id| (id == &actor.id).then_some(&actor),
        );
        assert!(
            out.ops.is_empty(),
            "a stranger receives no READ on a default:none token, even via the actor join"
        );
    }

    // ---- write-receive parity + adversarial egress ownership ----

    #[tokio::test]
    async fn a_document_you_can_write_is_a_document_you_receive() {
        // A document a user can WRITE (the owner floor grants WRITE_FIELDS at
        // `apply_intent`) must also be a document that user RECEIVES at egress
        // (the same owner floor, joined through the same `effective_owner`
        // rule at `filter_command`) — write authz and read authz resolve
        // ownership through one shared join, never two. Reuses the persisted
        // actor+linked-token arrangement from
        // `filter_command_update_keeps_owner_or_gm_changes_for_the_inheriting_owner`.
        use crate::auth::role::ServerRole;
        use crate::data::command::{FieldChange, Operation, WriteOrigin};
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
        let p = r.create_user("p", None, ServerRole::User, 0).await.unwrap();

        let mut actor = doc(PermissionSet::default(), serde_json::json!({}));
        actor.id = Uuid::from_u128(42);
        actor.scope = Scope::World { world_id: w.id };
        actor.owner = Some(p);

        let mut token = doc(
            PermissionSet {
                default: DocRole::None,
                ..Default::default()
            },
            serde_json::json!({ "notes": "secret plan" }),
        );
        token.doc_type = "token".into();
        token.id = Uuid::from_u128(100);
        token.scope = Scope::World { world_id: w.id };
        token.engine = Some(serde_json::json!({
            "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0, "rotation": 0.0,
            "actor_id": actor.id.to_string()
        }));
        token
            .permissions
            .property_overrides
            .insert("/system/notes".into(), Visibility::OwnerOrGm);

        r.apply_intent(
            &gm_ctx,
            w.id,
            vec![
                Operation::Create { doc: actor.clone() },
                Operation::Create { doc: token.clone() },
            ],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let p_ctx = PermissionContext {
            user_id: p,
            world_role: WorldRole::Player,
        };

        // 1. apply_intent as P: patches /system/notes on a `default: None` token
        //    it does not literally own. Must SUCCEED — the owner floor (via the
        //    actor link) grants WRITE_FIELDS.
        let cmd = r
            .apply_intent(
                &p_ctx,
                w.id,
                vec![Operation::Update {
                    doc_id: token.id,
                    changes: vec![FieldChange {
                        remove: false,
                        path: "/system/notes".into(),
                        old: serde_json::json!("secret plan"),
                        new: serde_json::json!("new plan"),
                    }],
                }],
                2,
                WriteOrigin::Client,
            )
            .await
            .expect("owner floor grants WRITE_FIELDS: the patch must succeed");

        // 2. filter_command of the returned command for P, joined through the
        //    same actor link, must RETAIN the op — the owner floor also grants
        //    READ at egress through the same owner value.
        let current = load_update_docs(&r, &cmd).await;
        let lookup = |id: &Uuid| (id == &actor.id).then_some(&actor);
        let out_p = filter_command(&cmd, &p_ctx, &WorldCapDefaults::default(), &current, lookup);
        assert_eq!(
            out_p.ops.len(),
            1,
            "the writer must also receive the write it just made"
        );

        // 3. A true stranger receives nothing.
        let stranger_ctx = PermissionContext {
            user_id: Uuid::from_u128(999),
            world_role: WorldRole::Player,
        };
        let out_stranger = filter_command(
            &cmd,
            &stranger_ctx,
            &WorldCapDefaults::default(),
            &current,
            lookup,
        );
        assert!(
            out_stranger.ops.is_empty(),
            "a stranger receives nothing (fail closed)"
        );
    }

    #[test]
    fn egress_ownership_ignores_a_cross_scope_actor() {
        // The scope check in `effective_owner`, exercised through the egress
        // join: a linked actor from a DIFFERENT scope must not be treated as
        // the token's owner at `filter_command`, even though the linked id
        // matches.
        use crate::data::command::{Command, Operation};
        use crate::data::membership::PermissionContext;

        let p = Uuid::from_u128(1);
        let actor_id = Uuid::from_u128(42);
        let mut token = token_linked_to(Some(actor_id));
        token.permissions.default = DocRole::None;
        token.scope = Scope::World {
            world_id: Uuid::from_u128(1000),
        };

        let mut foreign_actor = actor_owned_by(actor_id, Some(p));
        foreign_actor.scope = Scope::World {
            world_id: Uuid::from_u128(2000),
        };

        let cmd = Command {
            seq: 1,
            world_id: Uuid::from_u128(7),
            author: Uuid::from_u128(9),
            ts: 0,
            ops: vec![Operation::Create { doc: token.clone() }],
        };
        let lookup = |id: &Uuid| (id == &foreign_actor.id).then_some(&foreign_actor);
        let current = std::collections::HashMap::new();

        let p_ctx = PermissionContext {
            user_id: p,
            world_role: WorldRole::Player,
        };
        let out = filter_command(&cmd, &p_ctx, &WorldCapDefaults::default(), &current, lookup);
        assert!(
            out.ops.is_empty(),
            "a cross-scope actor join must not be treated as the owner at egress"
        );
    }

    #[test]
    fn egress_ownership_honors_the_per_token_override() {
        // token.owner = A, linked actor owned by B: the per-token override wins
        // over the actor join, the same precedence the write path
        // (`effective_owner`) uses — A receives, B does not.
        use crate::data::command::{Command, Operation};
        use crate::data::membership::PermissionContext;

        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let actor_id = Uuid::from_u128(42);
        let actor = actor_owned_by(actor_id, Some(b));
        let mut token = token_linked_to(Some(actor_id));
        token.permissions.default = DocRole::None;
        token.owner = Some(a);

        let cmd = Command {
            seq: 1,
            world_id: Uuid::from_u128(7),
            author: Uuid::from_u128(9),
            ts: 0,
            ops: vec![Operation::Create { doc: token.clone() }],
        };
        let lookup = |id: &Uuid| (id == &actor.id).then_some(&actor);
        let current = std::collections::HashMap::new();

        let a_ctx = PermissionContext {
            user_id: a,
            world_role: WorldRole::Player,
        };
        let out_a = filter_command(&cmd, &a_ctx, &WorldCapDefaults::default(), &current, lookup);
        assert_eq!(
            out_a.ops.len(),
            1,
            "the per-token override wins over the actor join: A receives"
        );

        let b_ctx = PermissionContext {
            user_id: b,
            world_role: WorldRole::Player,
        };
        let out_b = filter_command(&cmd, &b_ctx, &WorldCapDefaults::default(), &current, lookup);
        assert!(
            out_b.ops.is_empty(),
            "the override wins over the actor: B (the linked actor's literal owner) does not receive"
        );
    }

    #[test]
    fn egress_gm_and_gm_role_cap_are_unchanged() {
        // The owner-join plumbing through `filter_command` must not widen the
        // pre-existing `gm_role` cap: a plain doc still delivers everything to
        // the GM, but a `gm_role: Some(DocRole::None)` doc (message-style,
        // e.g. a whisper) still drops the capped GM's op entirely.
        use crate::data::command::{Command, Operation};
        use crate::data::membership::PermissionContext;

        let gm = Uuid::from_u128(1);
        let gm_ctx = PermissionContext {
            user_id: gm,
            world_role: WorldRole::Gm,
        };
        let current = std::collections::HashMap::new();

        // Plain doc: the GM still receives everything unconditionally.
        let plain = doc(PermissionSet::default(), serde_json::json!({ "hp": 10 }));
        let cmd_plain = Command {
            seq: 1,
            world_id: Uuid::from_u128(7),
            author: Uuid::from_u128(9),
            ts: 0,
            ops: vec![Operation::Create { doc: plain.clone() }],
        };
        let out_plain = filter_command(
            &cmd_plain,
            &gm_ctx,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        assert_eq!(
            out_plain.ops.len(),
            1,
            "an uncapped GM still receives everything"
        );

        // `gm_role: Some(DocRole::None)` doc: the capped GM's op is still
        // dropped entirely — the owner plumb must not have widened this cap.
        let mut capped = doc(PermissionSet::default(), serde_json::json!({}));
        capped.permissions.gm_role = Some(DocRole::None);
        let cmd_capped = Command {
            seq: 2,
            world_id: Uuid::from_u128(7),
            author: Uuid::from_u128(9),
            ts: 0,
            ops: vec![Operation::Create {
                doc: capped.clone(),
            }],
        };
        let out_capped = filter_command(
            &cmd_capped,
            &gm_ctx,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        assert!(
            out_capped.ops.is_empty(),
            "a gm_role-capped GM must still be denied — the owner plumb must not widen the cap"
        );
    }

    #[test]
    fn redaction_target_classifies_each_whole_band() {
        // The expectation is a HARDCODED list, never `REDACTABLE_BANDS` itself. Deriving the
        // expected value from the constant under test makes the assertion definitionally true
        // for any array contents — it would stay green if a band were renamed, which is the
        // exact "both paths wrong the same way" shape this suite exists to refuse.
        for band in ["name", "engine", "system", "base"] {
            let pointer = format!("/{band}");
            assert_eq!(
                redaction_target(&pointer),
                Some(RedactionTarget::Band),
                "{pointer} must classify as a whole band"
            );
        }
        // Pins the constant's contents independently, so a band added or renamed fails HERE
        // with a message naming the obligation, rather than silently widening what egress
        // is willing to remove.
        assert_eq!(
            REDACTABLE_BANDS,
            ["name", "engine", "system", "base"],
            "the band list changed: re-audit every redaction call site and this suite"
        );
    }

    #[test]
    fn redaction_target_classifies_within_a_band() {
        for pointer in [
            "/system/hp",
            "/system/a/b/c",
            "/engine/vision",
            "/base/system/hp",
            // An empty middle segment still lands inside the untyped body.
            "/system//hp",
            // An index segment is indistinguishable from an object key named "0" from the
            // pointer alone, so it classifies as `Within` and egress must be able to act on
            // it — narrowing the classifier to refuse index-shaped segments would hide an
            // array element from redaction instead of redacting it.
            "/system/inventory/0",
            "/system/inventory/0/secret",
        ] {
            assert_eq!(
                redaction_target(pointer),
                Some(RedactionTarget::Within),
                "{pointer} must classify as within a band"
            );
        }
    }

    #[test]
    fn redaction_target_refuses_every_structural_envelope_field() {
        // The eleven non-content fields of `Document`. Nothing may redact these: a
        // whole-key strip either substitutes a defaulted value or leaves a shape that
        // cannot deserialize.
        for field in [
            "id",
            "scope",
            "doc_type",
            "schema_version",
            "source",
            "owner",
            "permissions",
            "parent_id",
            "embedded",
            "created_at",
            "updated_at",
        ] {
            assert_eq!(redaction_target(&format!("/{field}")), None, "/{field}");
            assert_eq!(
                redaction_target(&format!("/{field}/anything")),
                None,
                "/{field}/anything"
            );
        }
    }

    #[test]
    fn redaction_target_refuses_permissions_subpaths_lacking_serde_default() {
        // A nested pointer into `permissions` strips a field carrying no serde default,
        // leaving a value that cannot deserialize as a `PermissionSet`.
        for pointer in [
            "/permissions",
            "/permissions/default",
            "/permissions/users",
            "/permissions/property_overrides",
        ] {
            assert_eq!(redaction_target(pointer), None, "{pointer}");
        }
    }

    #[test]
    fn redaction_target_refuses_malformed_and_unknown_pointers() {
        for pointer in [
            "",
            "/",
            "system/hp",
            "/unknown",
            "/systemx",
            "/nameless",
            // A band name followed by a non-separator character is a collision, not a
            // match, for every band the shared prefix path handles — not just `system`.
            "/enginex",
            "/basex",
            // A band name plus a trailing separator leaves an empty residual segment,
            // which the guard refuses rather than treating as `Within`.
            "/system/",
        ] {
            assert_eq!(redaction_target(pointer), None, "{pointer:?}");
        }
    }

    #[test]
    fn name_is_a_leaf_band_with_no_interior() {
        // `/name` is a display string, not a container — mirrors the same rule in
        // `required_cap_for_path`.
        assert_eq!(redaction_target("/name"), Some(RedactionTarget::Band));
        assert_eq!(redaction_target("/name/first"), None);
    }
}
