// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::{BTreeSet, HashMap};

use uuid::Uuid;

use crate::data::command::{Command, FieldChange, Operation};
use crate::data::document::{
    CapabilityGrants, CapabilityRequirement, DocRole, Document, PermissionSet, Visibility,
    WorldCapDefaults, WorldRole,
};
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;
use crate::data::snapshot::{CommandSnapshot, OpSnapshot};

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

/// A document's current state, as loaded for redaction: its live envelope plus its
/// `documents.created_seq` generation marker. The marker is compared against
/// `OpSnapshot::created_seq_at_commit` to detect a document id reused since a replayed
/// command's commit (the id was deleted and a new document created at the same id).
pub struct CurrentDoc {
    /// The document's current envelope.
    pub doc: Document,
    /// `documents.created_seq` — this id's current generation marker.
    pub created_seq: i64,
}

/// Current documents for every `Update`, `Create`, and `Delete` op in `cmd`, keyed by the op's
/// own doc_id (a `Create`'s newly-created id; an `Update`/`Delete`'s existing target). A missing
/// key means the document does not currently exist at that id; `filter_command` drops the
/// corresponding op. The whole-document commit∧current access gate applies uniformly to all three
/// op kinds (see `filter_command`'s doc comment), so `Create`/`Delete` need a current-state read
/// just as `Update` does — the gate cannot be evaluated for any of the three without one. Hoisted
/// out of the redaction core so it can be awaited ONCE, before any scene-guard scope is entered —
/// one pool read per distinct doc_id in `cmd`, per recipient.
pub async fn load_current_docs(repo: &dyn Repository, cmd: &Command) -> HashMap<Uuid, CurrentDoc> {
    let mut out = HashMap::new();
    for op in &cmd.ops {
        let doc_id = match op {
            Operation::Update { doc_id, .. } => *doc_id,
            Operation::Create { doc } => doc.id,
            Operation::Delete { doc } => doc.id,
        };
        if let std::collections::hash_map::Entry::Vacant(e) = out.entry(doc_id) {
            if let Ok(Some((doc, created_seq))) = repo.get_document_with_created_seq(doc_id).await {
                e.insert(CurrentDoc { doc, created_seq });
            }
        }
    }
    out
}

/// A `CommandSnapshot` for `cmd` whose commit-time state mirrors `current`'s live state, scoped
/// to the single recipient named by `ctx`. Its sole caller is `http::routes::write_ops`'s
/// author-read-back-of-their-own-write, which carries no persisted commit-time snapshot: at that
/// call site "commit time" and "now" are the same instant by construction (an author reading
/// back the write they just applied), so mirroring the current document loses no information the
/// commit-time half of `filter_command` would otherwise see. `ws::conn`'s live-broadcast and
/// replay paths instead carry the real, persisted commit-time snapshot end to end
/// (`ws::conn::send_filtered_event` reads `StoredCommand.snapshot` directly) and never call this
/// function. `world_gm_at_commit` carries exactly the one entry `filter_command` ever looks up
/// for a given `ctx`: `ctx.user_id -> (ctx.world_role == WorldRole::Gm)`.
pub fn mirror_current_snapshot<'a>(
    cmd: &Command,
    ctx: &PermissionContext,
    current: &HashMap<Uuid, CurrentDoc>,
    actor_lookup: &impl Fn(&Uuid) -> Option<&'a Document>,
) -> CommandSnapshot {
    let mut per_op = Vec::with_capacity(cmd.ops.len());
    for op in &cmd.ops {
        let target_doc: Option<&Document> = match op {
            Operation::Update { doc_id, .. } => current.get(doc_id).map(|c| &c.doc),
            Operation::Create { doc } => Some(doc),
            Operation::Delete { doc } => Some(doc),
        };
        let Some(d) = target_doc else {
            per_op.push(None);
            continue;
        };
        // Best-effort: a document reaching this function has already passed
        // `validation::validate_property_overrides` at its own write time, so this classifier
        // never meets a genuinely unclassifiable override outside a poisoned test fixture.
        let mut overrides = Vec::new();
        let _ = collect_overrides(d, "", &mut overrides);
        let touches_perms = matches!(
            op,
            Operation::Update { changes, .. }
                if changes.iter().any(|c| touches_permissions(&c.path))
        );
        per_op.push(Some(OpSnapshot {
            owner_at_commit: effective_owner_via(d, actor_lookup),
            doc_type: d.doc_type.clone(),
            overrides_at_commit: overrides.clone(),
            retraction_hidden_at_commit: if touches_perms { Some(overrides) } else { None },
            created_seq_at_commit: None,
            permissions_at_commit: Some(PermissionSet {
                property_overrides: Default::default(),
                ..d.permissions.clone()
            }),
            // Commit time and now are the SAME instant here (see this function's own doc
            // comment), so there is no distinct pre-image to report: the READ-transition
            // rule needs a snapshot from BEFORE this op, which this mirror never had.
            permissions_before_commit: None,
        }));
    }
    CommandSnapshot {
        per_op,
        world_gm_at_commit: HashMap::from([(ctx.user_id, ctx.world_role == WorldRole::Gm)]),
    }
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
    /// Redacted with a pointer strip, whose terminal step differs by container: an object
    /// key is removed, where callers rely on true key absence; an array element is nulled in
    /// place, because removal would renumber every later element. See `strip_pointer`.
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
mod required_cap_tests;

/// Whether `p` is a descendant of `ancestor` on a JSON-pointer boundary
/// (`/a/b` is a descendant of `/a`, but `/ab` is not).
fn is_descendant(p: &str, ancestor: &str) -> bool {
    p.len() > ancestor.len()
        && p.as_bytes()[ancestor.len()] == b'/'
        && p.as_bytes()[..ancestor.len()] == *ancestor.as_bytes()
}

/// Whether two JSON-pointer paths overlap as subtrees: equal, or either is a
/// descendant of the other.
/// Consumed cross-module by `SqliteRepository::build_op_snapshot` to prune an Update op's
/// commit-time override set to its own changed-paths closure.
pub(crate) fn paths_overlap(a: &str, b: &str) -> bool {
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

/// Collect every `(absolute_pointer, tier)` pair in `doc`'s own `property_overrides`, plus the
/// hardcoded `/base` `OwnerOrGm` entry (see `filter_properties`'s doc comment), recursing into
/// embedded descendants (parent-absolute addressing: a child at `embedded[key][i]` contributes
/// `/embedded/<key>/<i>{pointer}` — the SAME positional addressing `filter_properties`'s own
/// recursion uses). Access-independent: every override regardless of tier, so ONE traversal
/// feeds BOTH the live redaction path (`collect_hidden`, via `hidden_from_overrides`) and
/// commit-time snapshot construction (`OpSnapshot::overrides_at_commit`) — they cannot diverge
/// on how an embedded index is addressed because they share this one walk.
///
/// Classifies every REAL override pointer via `redaction_target` at traversal time (not lazily,
/// unlike a per-recipient filter would): safe because every document reaching this function has
/// already passed `validation::validate_property_overrides` at its OWN write time (both
/// `apply_command` and `apply_intent` call it on the full post-image, recursing into every
/// embedded descendant, before any document reaches storage) — an unclassifiable REAL override
/// pointer cannot exist in persisted data. Still returns `Result` to fail closed on
/// pre-validation legacy/hand-seeded data. The synthetic `/base` entry is never classified (it
/// is hardcoded, not user-supplied — mirrors the un-classified unconditional `/base` push this
/// function replaces).
pub(crate) fn collect_overrides(
    doc: &Document,
    prefix: &str,
    out: &mut Vec<(String, Visibility)>,
) -> Result<(), RedactionError> {
    for (p, v) in &doc.permissions.property_overrides {
        if redaction_target(p).is_none() {
            return Err(RedactionError { pointer: p.clone() });
        }
        out.push((format!("{prefix}{p}"), *v));
    }
    // Mirrors `filter_properties`' hardcoded `OwnerOrGm` policy for `/base` — see that
    // function's comment. Fires at every embedded depth too (each recursive call gets its own
    // `prefix`), covering an embedded child's own `base` the same way.
    out.push((format!("{prefix}/base"), Visibility::OwnerOrGm));
    for (key, children) in &doc.embedded {
        for (idx, child) in children.iter().enumerate() {
            collect_overrides(child, &format!("{prefix}/embedded/{key}/{idx}"), out)?;
        }
    }
    Ok(())
}

/// Filter `overrides` (as produced by `collect_overrides`) down to the absolute pointers
/// `access` may NOT see — the `can_see`-filtering half of the traversal split.
fn hidden_from_overrides(overrides: &[(String, Visibility)], access: &Access) -> Vec<String> {
    overrides
        .iter()
        .filter(|(_, v)| !access.can_see(*v))
        .map(|(p, _)| p.clone())
        .collect()
}

/// Lets `Update`-delta redaction honor hidden fields at any embedded depth — the same coverage
/// `filter_properties` gives whole-document egress. A thin wrapper: `collect_overrides` performs
/// the traversal (and classification), `hidden_from_overrides` performs the `can_see` filter —
/// kept as one function because this is `filter_command`'s ONE call site's exact shape.
fn collect_hidden(
    doc: &Document,
    access: &Access,
    prefix: &str,
    out: &mut Vec<String>,
) -> Result<(), RedactionError> {
    let mut overrides = Vec::new();
    collect_overrides(doc, prefix, &mut overrides)?;
    out.extend(hidden_from_overrides(&overrides, access));
    Ok(())
}

/// Whether a change path writes into any document's envelope `permissions` (top-level
/// or embedded) — a `permissions` path segment. Triggers retroactive redaction so a
/// just-hidden field is retracted from recipients who can no longer see it. A `system`
/// field literally named `permissions` over-triggers, which is safe (it only re-nulls
/// already-hidden fields).
/// Consumed cross-module by `SqliteRepository::build_op_snapshot` to decide whether an Update
/// op's commit-time snapshot needs a retraction set.
pub(crate) fn touches_permissions(path: &str) -> bool {
    path.split('/').any(|seg| seg == "permissions")
}

/// The recipient's view of a broadcast command: ops on unreadable documents are dropped,
/// GmOnly/OwnerOrGm properties/changes stripped. seq/world/author/ts are preserved so the
/// recipient's sequence guard never sees a false gap — a fully redacted command keeps its seq
/// with empty ops.
///
/// Redaction is the CONJUNCTION of two views: what was permitted at commit (`snapshot`) and what
/// is permitted now (`current`) — never fewer checks than either view alone would apply. A
/// pointer is redacted iff it was hidden at commit OR is hidden now; a whole op is dropped
/// unless BOTH the commit-time and current-time whole-document `cap::READ` gate admit it — this
/// asymmetry-closing gate applies uniformly to `Create`/`Update`/`Delete`, not just `Update`
/// (a recipient denied at a document's Create commit-time but currently permitted must not see
/// a LATER Update to the same doc_id either, or they receive field-level data for a document
/// they were never told exists).
///
/// `effective_owner_via` is joined through a caller-supplied in-memory actor source, so this
/// never queries the pool for the CURRENT-time half. The loads this needs (`current`, from
/// `load_current_docs`) are hoisted and awaited by the caller BEFORE calling in. The commit-time
/// half never queries anything live — it is fully derived from `snapshot`, by construction (no
/// live-state parameter exists on this function to reintroduce one from).
///
/// World capability GRANTS (`world_defaults`) are never snapshotted — both halves of the READ
/// conjunction resolve access via `resolve_access_world` with the SAME live `world_defaults`,
/// differing only in the document state, owner, and world ROLE (`world_role_commit` vs
/// `ctx.world_role`) each half carries. World ROLE (GM standing) IS snapshotted, via
/// `world_role_commit`.
pub fn filter_command<'a>(
    cmd: &Command,
    snapshot: &CommandSnapshot,
    ctx: &PermissionContext,
    world_defaults: &WorldCapDefaults,
    current: &HashMap<Uuid, CurrentDoc>,
    actor_lookup: impl Fn(&Uuid) -> Option<&'a Document>,
) -> Command {
    let mut out_ops = Vec::with_capacity(cmd.ops.len());
    for (idx, op) in cmd.ops.iter().enumerate() {
        // A `None` per-op snapshot entry (a legacy `world_events` row carrying no recorded
        // snapshot) drops the op on replay rather than falling back to a live-lookup redaction.
        let Some(op_snapshot) = snapshot.per_op.get(idx).and_then(|s| s.as_ref()) else {
            continue;
        };
        let gm_at_commit = snapshot
            .world_gm_at_commit
            .get(&ctx.user_id)
            .copied()
            .unwrap_or(false);
        let world_role_commit = if gm_at_commit {
            WorldRole::Gm
        } else {
            WorldRole::Player
        };
        match op {
            Operation::Create { doc } => {
                // World capability GRANTS stay current-only at BOTH halves of the READ
                // conjunction (world ROLE is still commit-scoped via `world_role_commit`) — the
                // same live `world_defaults` layered onto `access_current` below.
                let access_commit = resolve_access_world(
                    ctx.user_id,
                    world_role_commit,
                    doc,
                    &world_defaults.grants_for(&doc.doc_type),
                    op_snapshot.owner_at_commit,
                );
                let owner_current = effective_owner_via(doc, &actor_lookup);
                let access_current = resolve_access_world(
                    ctx.user_id,
                    ctx.world_role,
                    doc,
                    &world_defaults.grants_for(&doc.doc_type),
                    owner_current,
                );
                if !access_commit.has(cap::READ) || !access_current.has(cap::READ) {
                    continue;
                }
                match filter_properties(doc, &access_current) {
                    Ok(filtered) => out_ops.push(Operation::Create { doc: filtered }),
                    Err(e) => {
                        tracing::warn!(doc_id = %doc.id, error = %e, "redaction failed; dropping Create op for recipient");
                    }
                }
            }
            Operation::Delete { doc } => {
                // Existence check is INVERTED vs Update: a Delete's current doc is EXPECTED to
                // be absent (that is the point of the op). The created_seq mismatch check
                // applies only when a current doc DOES exist (the id was reused).
                if let Some(commit_seq) = op_snapshot.created_seq_at_commit {
                    if let Some(cur) = current.get(&doc.id) {
                        if cur.created_seq != commit_seq {
                            continue;
                        }
                    }
                }
                // World capability GRANTS stay current-only at BOTH halves of the READ
                // conjunction (world ROLE is still commit-scoped via `world_role_commit`).
                let access_commit = resolve_access_world(
                    ctx.user_id,
                    world_role_commit,
                    doc,
                    &world_defaults.grants_for(&doc.doc_type),
                    op_snapshot.owner_at_commit,
                );
                let owner_current = effective_owner_via(doc, &actor_lookup);
                let access_current = resolve_access_world(
                    ctx.user_id,
                    ctx.world_role,
                    doc,
                    &world_defaults.grants_for(&doc.doc_type),
                    owner_current,
                );
                if !access_commit.has(cap::READ) || !access_current.has(cap::READ) {
                    continue;
                }
                match filter_properties(doc, &access_current) {
                    Ok(filtered) => out_ops.push(Operation::Delete { doc: filtered }),
                    Err(e) => {
                        tracing::warn!(doc_id = %doc.id, error = %e, "redaction failed; dropping Delete op for recipient");
                    }
                }
            }
            Operation::Update { doc_id, changes } => {
                // Absent = does not currently exist → drop, preserving today's semantics.
                let Some(cur) = current.get(doc_id) else {
                    continue;
                };
                if let Some(commit_seq) = op_snapshot.created_seq_at_commit {
                    if cur.created_seq != commit_seq {
                        continue;
                    }
                }
                let commit_doc = Document {
                    doc_type: op_snapshot.doc_type.clone(),
                    permissions: op_snapshot
                        .permissions_at_commit
                        .clone()
                        .unwrap_or_default(),
                    ..cur.doc.clone()
                };
                // World capability GRANTS stay current-only at BOTH halves of the READ
                // conjunction (world ROLE is still commit-scoped via `world_role_commit`).
                // Update never changes doc_type, so `commit_doc.doc_type` and `cur.doc.doc_type`
                // agree; kept on `commit_doc` to pair with the rest of this half's arguments.
                let access_commit = resolve_access_world(
                    ctx.user_id,
                    world_role_commit,
                    &commit_doc,
                    &world_defaults.grants_for(&commit_doc.doc_type),
                    op_snapshot.owner_at_commit,
                );
                let owner_current = effective_owner_via(&cur.doc, &actor_lookup);
                let access_current = resolve_access_world(
                    ctx.user_id,
                    ctx.world_role,
                    &cur.doc,
                    &world_defaults.grants_for(&cur.doc.doc_type),
                    owner_current,
                );
                // READ-transition synthesis: a permission change that grants or revokes
                // this recipient's whole-document READ cannot travel as a field delta —
                // a recipient who never received the Create drops an Update for an
                // unknown id, and one who loses READ would otherwise keep a stale copy.
                // Scoped to THIS op's own before→commit transition; the current-time
                // half still gates delivery of a synthesized Create, and a synthesized
                // Delete carries a stub so nothing hidden rides it.
                //
                // `owner_at_commit` stands in for the before-state owner too: `OpSnapshot`
                // stores only the post-image owner, so an op that changes BOTH `/owner`
                // and `/permissions/default` in the same Update resolves `access_before`
                // against the NEW owner. That is the safe direction — the post-image
                // owner is the one who must now see the document, so this can only widen
                // a Create synthesis toward the recipient who is correct after the op,
                // never toward one who should have been excluded.
                if let Some(before_perms) = &op_snapshot.permissions_before_commit {
                    let before_doc = Document {
                        permissions: before_perms.clone(),
                        ..commit_doc.clone()
                    };
                    let access_before = resolve_access_world(
                        ctx.user_id,
                        world_role_commit,
                        &before_doc,
                        &world_defaults.grants_for(&before_doc.doc_type),
                        op_snapshot.owner_at_commit,
                    );
                    let read_before = access_before.has(cap::READ);
                    let read_commit = access_commit.has(cap::READ);
                    if !read_before && read_commit {
                        if access_current.has(cap::READ) {
                            match filter_properties(&cur.doc, &access_current) {
                                Ok(filtered) => out_ops.push(Operation::Create { doc: filtered }),
                                Err(e) => {
                                    tracing::warn!(doc_id = %doc_id, error = %e, "redaction failed; dropping synthesized Create for recipient");
                                }
                            }
                        }
                        continue;
                    }
                    if read_before && !read_commit {
                        out_ops.push(Operation::Delete {
                            doc: delete_stub(&cur.doc),
                        });
                        continue;
                    }
                }
                if !access_commit.has(cap::READ) || !access_current.has(cap::READ) {
                    continue;
                }
                let kept: Vec<FieldChange> = if access_current.see_gm_only
                    && access_commit.see_gm_only
                {
                    changes.clone()
                } else {
                    let mut hidden_current = Vec::new();
                    if let Err(e) =
                        collect_hidden(&cur.doc, &access_current, "", &mut hidden_current)
                    {
                        tracing::warn!(doc_id = %doc_id, error = %e, "redaction failed; dropping Update op for recipient");
                        continue;
                    }
                    let hidden_commit =
                        hidden_from_overrides(&op_snapshot.overrides_at_commit, &access_commit);
                    let mut hidden = hidden_current;
                    hidden.extend(hidden_commit);
                    hidden.sort();
                    hidden.dedup();
                    let mut kept: Vec<FieldChange> = changes
                        .iter()
                        .filter_map(|ch| redact_change(ch, &hidden))
                        .collect();
                    // Retraction: use this command's OWN commit-time hidden set, filtered
                    // through THIS recipient's commit-time access only — never the union, and
                    // never whatever is live now. Each retracting command owns its own
                    // retraction moment.
                    if changes.iter().any(|c| touches_permissions(&c.path)) {
                        if let Some(retraction) = &op_snapshot.retraction_hidden_at_commit {
                            for ptr in hidden_from_overrides(retraction, &access_commit) {
                                kept.push(FieldChange {
                                    remove: false,
                                    path: ptr,
                                    old: serde_json::Value::Null,
                                    new: serde_json::Value::Null,
                                });
                            }
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

/// The envelope-only shape a recipient losing READ receives as a `Delete`: identity and
/// placement (`id`, `doc_type`, `scope`, `parent_id`, `schema_version`, timestamps) with
/// every content band emptied and fail-closed default permissions, so the retraction
/// itself discloses nothing the recipient may no longer see.
fn delete_stub(doc: &Document) -> Document {
    Document {
        name: None,
        source: None,
        base: None,
        owner: None,
        permissions: PermissionSet::default(),
        embedded: Default::default(),
        engine: None,
        system: serde_json::Value::Object(Default::default()),
        ..doc.clone()
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
mod tests;
