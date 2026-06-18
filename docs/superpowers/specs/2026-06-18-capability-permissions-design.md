# Capability-Based Permissions — Design Spec

> Status: **Phase 1 IMPLEMENTED** on branch `capability-permissions-phase1`
> (core capabilities + per-document and per-world grants + path→capability
> gating + world-defaults endpoints). Deferred follow-ups: world-level
> `core:create` authorization (§7.1) and doc_type-scoped world defaults (§7.2)
> — both logged in `POST_WORK_FINDINGS.md`. Phase 2 (module-defined
> capabilities and action enforcement, §5) lands with the M6 module/hook layer.

## 1. Motivation

M5 ships a binary write gate: `Access { can_read, can_write, see_gm_only }`,
where `can_write` authorizes *every* mutation. This cannot express the required
distinction between, e.g., a player-owner who may roll from a sheet (write
`/system` fields) and a GM who alone may add or remove embedded documents.

The system must be **Fully Modular** (project core directive): system/module
builders define their own actions and the permission levels that gate them. So
the model is not a fixed Rust enum of capabilities — it is data-driven over
**namespaced capability strings**, of which `core:*` is one (server-understood)
namespace.

## 2. Invariants preserved

- **Server-authoritative, structural validation only** (ARCHITECTURE #1, #6).
  The server enforces capability *possession* — "does actor A hold capability C
  on document D?" — which is pure data. It never interprets the meaning of the
  opaque `system` body or of a module's action.
- **GM omnipotence within a world.** `WorldRole::Gm` (and server admin) holds
  every capability on every document in the world. Unchanged.
- **Per-recipient read redaction.** Property-level `Visibility::GmOnly`
  stripping is layered on top of `core:read`. Unchanged.
- **Ordered realtime, one write path.** All mutations still flow through
  `apply_intent`; capability checks slot into its existing per-op authorize phase.

## 3. Capabilities

A capability is a namespaced string `"<namespace>:<verb>"`. The server treats
all capabilities as opaque grant tokens **except** the built-in `core:*` set,
which it maps to concrete operations:

| Capability | Gates |
|---|---|
| `core:read` | reading the document (then GmOnly redaction applies) |
| `core:write_fields` | `Update` field-changes under `/system` |
| `core:manage_embedded` | `Update` field-changes under `/embedded` (add/remove/modify embedded docs) |
| `core:delete` | `Delete` of the document |
| `core:edit_permissions` | `Update` field-changes under `/permissions` (the only sanctioned ACL-edit path) |

Document creation is governed at the world/container level (who may create
documents in a world), not by a per-document capability — see §7.

Envelope fields outside the above (`/id`, `/scope`, `/owner`, `/doc_type`,
`/schema_version`, `/source`, timestamps) remain **immutable via `Update`** for
everyone, exactly as M5 enforces for `/id` and `/scope`.

### 3.1 Path → capability map (generalizes M5's `/system`-only rule)

M5 restricted `Update` field paths to `/system`. Phase 1 generalizes this: each
`FieldChange.path` resolves to a required capability, and the actor must hold it.

```
/system/...        -> core:write_fields
/embedded/...      -> core:manage_embedded
/permissions/...   -> core:edit_permissions
(anything else)    -> denied (immutable envelope)
```

A multi-change `Update` requires the actor to hold the capability for *every*
change's path (all-or-nothing, in the existing single transaction).

## 4. Grant model

Resolution of an actor's effective capability set on a document, highest
precedence first:

1. **World role short-circuit.** `WorldRole::Gm` / server admin ⇒ all
   capabilities. (Resolution stops here.)
2. **Built-in role tier (the floor).** The actor's `DocRole` on the document
   (per-user, falling back to the document `default`) maps to a built-in default
   capability set:
   - `Owner` → `{core:read, core:write_fields}`
   - `Observer` → `{core:read}`
   - `None` → `{}`
3. **World default grants** (optionally keyed by `doc_type`): *additive* grants
   layered on the tier — e.g. "in this world, Owners also hold
   `core:manage_embedded`."
4. **Per-document grants**: *additive* grants on the specific document
   (per-role and per-user), highest precedence below GM.

Phase 1 grants are **additive only** (they widen beyond the built-in floor).
Revocation (removing a default capability) is deferred; it is rarely needed for
the motivating cases and complicates conflict resolution.

### 4.1 Storage

`PermissionSet` (in the document JSON envelope) gains a `capabilities` field:

```rust
pub struct PermissionSet {
    pub default: DocRole,                          // existing
    pub users: BTreeMap<Uuid, DocRole>,            // existing
    pub property_overrides: BTreeMap<String, Visibility>, // existing (read redaction)
    #[serde(default)]
    pub capabilities: CapabilityGrants,            // new
}

#[derive(Default)]
pub struct CapabilityGrants {
    // additive grants beyond the built-in DocRole floor
    pub by_role: BTreeMap<DocRole, BTreeSet<String>>,
    pub by_user: BTreeMap<Uuid, BTreeSet<String>>,
}
```

`#[serde(default)]` makes this backward-compatible: existing M5 documents
deserialize with empty grants and behave exactly as the built-in floor (Owner
writes fields, etc.) — no data migration.

World defaults live in a `world_capability_defaults` settings record (JSON,
keyed by world and optional `doc_type`), read when resolving access. (Phase 1
may ship document-level grants first and world defaults second; both use the
same `CapabilityGrants` shape.)

### 4.2 Resolution API

`resolve_access` is replaced/augmented to return an effective capability set:

```rust
pub struct Access {
    pub caps: BTreeSet<String>, // effective capabilities (core:* and custom)
    pub see_gm_only: bool,      // retained for read redaction
}
impl Access { pub fn has(&self, cap: &str) -> bool { ... } }
```

`apply_intent` checks `access.has("core:write_fields")` etc. per op/path.
Read paths check `access.has("core:read")`. `see_gm_only` continues to drive
`filter_properties` / `filter_command`.

## 5. Phase 2 — module-defined capabilities & actions (with M6)

Designed now so Phase 1 storage is forward-compatible; **implemented with the
M6 module/hook layer**, not in Phase 1.

- **Declaration.** A system/module declares its capabilities in its manifest
  (namespaced, e.g. `dnd5e:cast`). Declared capabilities are registered so the
  permission UI can offer them; the server still treats them as opaque tokens.
- **Action enforcement.** A module action decomposes into core ops. The server
  cannot know "this Update is a cast" — so a custom capability is enforced as an
  **additional** gate, never a replacement for core-op gating:
  1. The intent may carry an optional `action` tag naming the module + required
     capability.
  2. The server verifies the actor holds that capability (possession check), AND
  3. every underlying op still passes its `core:*` path/op capability check, so a
     mislabeled or hostile intent can never exceed what core grants allow.
  4. A registered **server-side module validation hook** (M6) may add semantic
     checks/transforms within the authoritative path.
- **Trust/sandboxing.** Running module-supplied validation server-side is an M6
  security decision (sandboxing, resource bounds). Out of scope here; flagged as
  the gating dependency for Phase 2.

Because the grant store already holds arbitrary capability strings, custom
capabilities and their grants require **no schema migration** when Phase 2 lands.

## 6. Compatibility & migration

- No DB migration: documents are JSON; the new `capabilities` field defaults to
  empty and reproduces M5 behavior exactly.
- `Access` changes are internal (data layer + http/ws callers). ts-rs bindings
  for `PermissionSet` regenerate.
- The M5 "`Update` restricted to `/system`" rule becomes the §3.1 path map;
  `/permissions` edits become possible **only** with `core:edit_permissions`
  (still denied for ordinary writers — no regression to the M5 escalation fix).

## 7. Open decisions (for review)

1. **Create authorization.** Who may create documents in a world — any member, a
   per-world `core:create` grant, or per-`doc_type`? (Leaning: a world-level
   grant, GM always allowed.)
2. **World-default granularity.** Per world, or per world × `doc_type`?
   (Leaning: per world × optional `doc_type`.)
3. **Revocation.** Additive-only in Phase 1 (proposed) vs. allow revoking a
   built-in default capability.
4. **Grant subject shape.** `by_role` + `by_user` additive maps (proposed) vs. a
   capability→grantees inversion.
5. **World defaults now or follow-up.** Ship per-document grants + the path map
   in the first Phase-1 increment, world defaults immediately after?

## 8. Phase 1 scope summary (what gets built now)

- `CapabilityGrants` on `PermissionSet` (+ ts-rs); built-in DocRole→caps floor.
- `Access { caps, see_gm_only }` + capability-aware `resolve_access`.
- `apply_intent`: §3.1 path→capability gating for `Update`; `core:delete` for
  `Delete`; `core:manage_embedded` enables `/embedded` mutation (the new
  operation M5 lacked); `core:read` for reads; `core:edit_permissions` for
  `/permissions`.
- HTTP/WS: grant-management endpoint(s) gated by `core:edit_permissions`/GM;
  world-default config per decision §7.2/§7.5.
- Tests: player-owner rolls but cannot manage embedded; granting
  `core:manage_embedded` lets an owner add/remove embedded; ACL edit gated;
  GM unaffected; backward-compat (M5 docs behave unchanged).
