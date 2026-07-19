# M13f — Server Declarative Schema Registry (tier-2 validation) — Design

**Status:** approved 2026-07-18. Sub-spec of the M13 Nightfox milestone
(`docs/superpowers/specs/2026-07-15-m13-nightfox-system-design.md` §9). Base: `main` @ `38886cf`
(post-M13e). Successor of M13-0 (three-band shape) and M13e (merge engine); M13f is the final M13
sub-milestone.

**Goal:** give the server a *structural* (shape-only) enforcement floor over module-declared
subtrees of the opaque `system` body, expressed as declarative data the server interprets but
never executes — closing the tier-2 half of the phased-validation model (D6) while keeping
ARCHITECTURE §2 invariant 6 intact.

**One-line frame:** the server is a **structural authority over the current write**, never a
**semantic authority over the corpus**. Every decision below falls out of that stance plus
fail-closed defaults plus "authority = GM-controlled world state."

---

## 1. Context & the invariant-6 boundary

Validation is phased (D6): **tier-1** (client) is rich Zod write-validation in sheets + fail-closed
readers, already shipped for Nightfox (M13b/M13c); **tier-2** (server) is this milestone — a
declarative, data-driven schema registry. ARCHITECTURE §2 invariant 6 lets the server's authority
over the `system` body be **structural only** (size caps, field-path validity,
`deny_unknown_fields`, permissions) and forbids **semantic/mechanical** validation of
system-defined content, because the server runs no third-party code.

M13f's whole design is chosen so tier-2 stays provably on the structural side of that line: the
declarable schema is a **JSON type-tree** and nothing more. It cannot inspect a value's magnitude,
content, or legal combination, so it is *incapable* of expressing a game rule — invariant 6 holds
**by construction, not by vigilance.** M13f is a *generalization of the structural vocabulary the
server already has* (`deny_unknown_fields`, field-path validity) to arbitrary declared shapes, not
a new category of authority.

The band split already exists (M13-0): the typed `engine` band gets real server-side semantic
ingress validation (`validate_engine_tree` → `engine::normalize_engine_opt`), because it is
engine-owned, not third-party. The opaque `system` band gets structural-only treatment. **M13f
guards the `system` band exclusively; the `engine` band is out of scope (already server-typed).**

## 2. Decisions

| # | Decision |
|---|---|
| F1 | **Tier-2 guards shape, never values.** The declarable schema is a JSON type-tree: `type`, `properties`, `required`, `items`, `additionalProperties`, `nullable`. No `enum`, no numeric/string bounds, no `pattern`, no `oneOf`/`anyOf`/`if`/`$ref`. Value legality stays tier-1 + fail-closed readers. |
| F2 | **`additionalProperties` defaults to `false` (closed)** on every registered schema node. A registered schema declares the *full* shape of its subtree; undeclared keys are rejected. Open maps/records set `additionalProperties` explicitly (a subschema, or `true`). Unregistered subtrees stay fully open (no schema ⇒ no constraint). |
| F3 | **Manifest-sourced, GM-committed-on-enable, module-keyed, per-world registry.** Schemas are declared as data in the module manifest and committed to a per-world server registry when a GM enables the module in that world — mirroring the existing `ContractDeclaration` flow. Authority is GM-controlled world state; the writer never supplies the schema. |
| F4 | **Subtree-scoped `(doc_type, /system/… pointer) → schema`.** Never the whole `system` body. Pointers are restricted to `/system/…`; `/engine`, `/permissions`, `/name`, and envelope pointers are rejected at set-time. Composable: disjoint subtrees of one doc_type may come from different modules; the same/overlapping `(doc_type, pointer)` is a topology conflict rejected fail-closed at set-time. |
| F5 | **Enforce-on-write, post-image, recursive, fail-closed.** A new read-only `system`-band validator runs in `apply_intent` Phase-1, parallel to `validate_engine_tree`, on Create/Update post-images, recursing embedded descendants (each looked up by *its own* `doc_type`). Violation rejects the whole command, shaped like a capability denial — **zero new wire frames.** |
| F6 | **Latest-wins upgrade; no retroactive validation, no migration, no per-doc-version routing.** The registry holds the *current* schema per `(module, doc_type, subtree)`. Enforcement always uses the currently-registered schema against the write's post-image. Pre-existing invalid docs linger until next touched (migrate-at-the-boundary). |
| F7 | **Two independent versions.** An engine-owned **schema-format version** (the vocabulary itself, so the server can reject a format it does not understand and the vocabulary can evolve compatibly) and the module's **content version** (`module_id` + `version`, provenance/observability only). |
| F8 | **The schema itself is structurally validated at set-time**, fail-closed (well-formed type-tree, bounded size/depth/count, legal pointers, no collisions) — like `validate_contract_declarations`. ts-rs-generated TS type + Zod mirror under the drift guard. |

## 3. Schema vocabulary (F1)

The complete grammar — a recursive tagged type-tree. This is *every* construct; there are no others.

```
Schema =
  | { type: "object",
      properties?:        { <key>: Schema },   // per-named-key subschema
      required?:          [ <key> ],            // keys that must be present
      additionalProperties?: boolean | Schema,  // default false (F2)
      nullable?:          true }                 // value may also be JSON null
  | { type: "array",
      items?:             Schema,               // subschema every element must match
      nullable?:          true }
  | { type: "string" | "number" | "boolean" | "null",
      nullable?:          true }
  | { }                                          // no `type` ⇒ "any JSON" (open escape hatch)
```

Semantics:
- **`type`** is a single JSON type per node (no type-union list — keeps each node's shape
  unambiguous). `nullable: true` widens exactly one node to "or JSON `null`."
- **object**: `properties` gives per-key subschemas; `required` lists keys that must be present;
  `additionalProperties` governs keys not in `properties` — `false` rejects them, a `Schema`
  requires each to match it (the open-typed-map case), `true` permits any. Absent ⇒ `false` (F2).
- **array**: `items` is one subschema all elements must satisfy (uniform element typing only — no
  positional/tuple typing, which edges into positional-semantics; and no `minItems`/`maxItems`,
  which are value bounds).
- **scalars**: shape only; no bounds/pattern/enum.
- **`{}` (no `type`)**: matches any JSON value — the natural bottom of the lattice and the escape
  hatch for "this key exists and may hold anything."

`required` (presence) and `nullable` (may-be-null) are orthogonal: a field can be
required-and-nullable (must be present, may be `null`), optional-and-non-nullable, etc.

**Worked example — Nightfox `system.stats` (open user-keyed map of typed stat entries) and
`system.mechanics` (closed record):**

```jsonc
// (doc_type "actor", pointer "/system/stats")
{ "type": "object",
  "additionalProperties": {                 // open key set; each VALUE is a stat entry
    "type": "object",
    "required": ["kind"],
    "properties": {
      "kind":  { "type": "string" },        // NOTE: not enum — value legality is tier-1
      "base":  { "type": "number", "nullable": true },
      "label": { "type": "string" }
    }
  } }

// (doc_type "actor", pointer "/system/mechanics")
{ "type": "object",
  "required": ["version"],
  "properties": {                            // additionalProperties defaults to false (F2)
    "version":   { "type": "number" },
    "modifiers": { "type": "array", "items": { "type": "object" } },
    "active":    { "type": "boolean" },
    "transfer":  { "type": "boolean" }
  } }
```

The `stats` schema sets `additionalProperties` to the entry subschema explicitly, so F2's
closed-by-default never bites the open-map case; it only bites a *record* that forgot a field —
exactly where a loud authoring-time rejection is wanted. Keys of `system` outside `/system/stats`
and `/system/mechanics` remain unconstrained (subtree scoping, not whole-body).

## 4. Declaration channel & registry (F3, F4, F7, F8)

**Precedent reused verbatim in shape:** `CapabilityRequirement` / `set_world_cap_requirements`
(the exact M6b Capability-Phase-2 precedent D6 cites) and `ContractDeclaration` /
`set_world_contract_declarations` — both are per-world, GM/admin-set, structurally-validated,
`Welcome`-broadcast registries of declarative data flowing from module manifests to server state.
M13f adds a third registry of the same shape.

**Registry entry (new type, ts-rs-exported):**

```
SchemaDeclaration {
  module_id:       String,          // provenance + teardown key
  version:         String,          // module content version (F7; observability only)
  schema_format:   u32,             // engine-owned vocabulary version (F7)
  doc_type:        String,          // which doc_type this guards
  subtree_pointer: String,          // RFC-6901, must be under /system (F4)
  schema:          Schema,          // the type-tree (§3), typed — not raw Value
}
```

The **enforcement lookup** `(doc_type, subtree_pointer) → Schema` is derived from the world's
declared set.

**Declaration flow:** the module manifest carries its `SchemaDeclaration`s as data. Enabling the
module in a world (an existing GM action) commits them into that world's registry. The server
stores and enforces them; it never loads or executes manifest *code* — the commit is a
GM-authorized *write of declarative data*, identical to how `ContractDeclaration`s already reach
the server. Disabling a module drops its entries (module-keyed).

**Set-time validation (F8) — fail-closed, the server is the consistency authority.** Mirrors
`validate_contract_declarations`:
- bounded count / per-schema node-count / nesting depth (backstops against a pathological schema;
  concrete caps resolved in the plan, sized like `MAX_CONTRACT_DECLARATIONS`);
- `module_id` / `version` non-empty; a `module_id` appears at most once;
- `subtree_pointer` is a well-formed RFC-6901 pointer that is a **strict descendant of `/system`**
  (`/system/<key>…`, any depth) — reject `/engine`, `/permissions`, `/name`, `""` (whole doc), and
  bare `/system` itself (guarding the whole `system` body is not subtree-scoped and re-introduces
  the `deny_unknown_fields`-on-`system` problem the band split avoids — parent spec §9 "never the
  whole body"). Mirrors the `path_prefix`-within-writable-namespace check
  `set_world_cap_requirements` already performs, narrowed to strict `/system` descendants only.
- **no two entries with the same `(doc_type, subtree_pointer)`, and no two whose pointers overlap**
  (one a prefix of the other) for the same `doc_type` — overlap is an ambiguous-authority
  contradiction (which schema governs the nested value?), rejected like the singleton-contract
  check;
- `schema_format` is one the server understands (else reject — forward-compat gate, F7);
- each `schema` is a well-formed type-tree (§3): every node's `type` is legal, `properties`/`items`
  recurse validly, `additionalProperties` is `bool | Schema`, no unknown schema keys
  (`deny_unknown_fields` on the `Schema` structs themselves).

**Wire / types:** `SchemaDeclaration` and the recursive `Schema` are Rust structs/enums,
ts-rs-exported, mirrored in the client Zod schema under the existing drift guard (never hand-edit
`src/types/generated`). `Schema` is a typed enum, not a raw `serde_json::Value`, so a malformed
schema fails to deserialize at the set endpoint.

**Endpoint + storage + broadcast:** a GM/admin-only `get`/`set world schema declarations` HTTP
pair (like `get/set_world_contract_declarations`), a `Repository` method pair
(`world_schema_declarations` / `set_world_schema_declarations`), and inclusion in the `Welcome`
payload alongside `cap_requirements` and `contract_declarations` (so clients can mirror tier-2
expectations; tier-1 already validates, so this is informational/parity, not a client gate).

## 5. Enforcement (F5)

**Location:** `sqlite.rs::apply_intent`, Phase-1 (authorize + structurally validate + check
pre-images — the pre-mutation loop whose failure drops the transaction so a rejected intent never
consumes the per-world seq). A new validator is called **immediately after `validate_engine_tree`**
for each `Create`/`Update` post-image:

```
validate_system_schema_tree(doc: &Document, registry: &WorldSchemaSet) -> Result<(), DataError>
```

- **Read-only** (`&Document`) — unlike `validate_engine_tree(&mut Document)`, there is no
  normalization; tier-2 only accepts/rejects. (This keeps M13f from mutating the opaque `system`
  body, which the server must not reshape.)
- **Recurses embedded descendants** exactly like `validate_engine_tree` / `validate_system_size`,
  looking each child up by *its own* `doc_type`. An embedded item's `/system/…` is validated by the
  *item* schema, not by a nested pointer in the parent's actor schema — the composable,
  doc_type-keyed model.
- For each `(doc.doc_type, subtree_pointer)` present in the registry: resolve the pointer within
  the post-image `system` body; if the subtree is absent, that is not a violation (presence of a
  subtree is not compelled by registering a schema for it — only its *shape when present* is
  governed); if present, validate it against the type-tree (§3), `additionalProperties` closed by
  default (F2).
- **The world registry is loaded once before the transaction begins** (like
  `world_cap_requirements` / `world_cap_defaults`), never mid-transaction (the single-writer pool
  would deadlock).

**Rejection:** a new `DataError` variant (e.g. `DataError::SchemaViolation { pointer, reason }`)
carrying the offending JSON-pointer + a structural reason (`expected number, got string`;
`unknown key not permitted by schema`; `missing required key`). It maps onto the **same
rejected-intent path** as an OCC `Conflict` / capability denial — the client surfaces it through
existing rejected-optimistic-op error UX (M5/M6). **No new wire frame, no new operation type.**

**Ordering within Phase-1 (cheap → expensive, fail-closed at each):** scope check → size cap
(`validate_system_size`) → engine tree (`validate_engine_tree`) → **system schema tree (new)** →
capability authorization → OCC pre-image. (Exact interleave with authz resolved in the plan; the
invariant is that any gate's failure drops the transaction.)

## 6. Upgrade & versioning (F6, F7)

- The registry stores the **current** schema per `(module_id, doc_type, subtree_pointer)`.
  Re-enabling a module at a new content `version` replaces its entries wholesale (set-endpoint
  semantics). Enforcement always reads the currently-registered set.
- **No retroactive validation, no migration, no per-doc-version routing.** A doc written under a
  prior schema is untouched until its next write, whose post-image must satisfy the *current*
  schema (or be rejected — surfacing the migration need at the boundary, consistent with OCC/merge
  reconciliation). The server never migrates `system` data (it has no system code) and never routes
  a write by a doc's claimed version.
- **Honest guarantee boundary:** M13f guarantees *all writes conform to the current schema*, **not**
  *the whole corpus conforms*. A reader can still meet old-shaped data (readers already fail-closed,
  so no crash). This is inherent to enforce-on-write and unavoidable without a migration capability
  invariant 6 forbids.
- **`schema_format` (F7)** lets the vocabulary evolve: the set endpoint rejects a `schema_format`
  the server does not understand, so a future format bump can't be silently half-enforced.

## 7. Composition with existing structural gates

- **Size cap** (`MAX_SYSTEM_BYTES`, 256 KiB/block via `validate_system_size`): unchanged and
  orthogonal; runs first (cheap) before the tree-walk. It already covers the DoS-via-huge-value
  concern, which is why F1 needs no length/size constructs.
- **`deny_unknown_fields`**: applies to the typed `engine`/envelope structs as today. F2's
  `additionalProperties: false` is the *opt-in, per-subtree* equivalent for the free-form `system`
  band — one coherent "declared ⇒ closed" model across both bands.
- **`engine` band**: out of scope. It is already server-typed and semantically validated
  (`validate_engine_tree`); registering a schema at an `/engine` pointer is rejected at set-time
  (F4).
- **Permissions / redaction**: unchanged. Stat blocks are ordinary `system` data under the existing
  per-recipient model (property overrides, `OwnerOrGm`, redaction-before-transmission). M13f adds
  **no egress path** — it is an ingress write-gate only (Nightfox spec §10).

## 8. Security & permissions

- The registry is **GM-controlled world state**; the writer of a document never supplies the schema
  that judges it (that would let a hostile client ship a permissive schema and self-approve — the
  load-bearing reason the registry is world config, not per-write input).
- Set endpoints are GM/admin-only (`require_gm`), like the cap-requirement and contract endpoints.
- M13f is the milestone's **only new server enforcement surface** and gets its own security-lens
  review: **buddy-check pre-authorization is recommended at plan level** for the enforcement
  chokepoint and the set-time validator (like every prior wire/enforcement checkpoint —
  [[shared-wire-schema-change-needs-full-repo-test]], [[m11c3-buddy-check-seam-scoping]]).
- No new egress, no new frame, no formula/notation surface (M13f is pure structural ingress).

## 9. Testing strategy (Nightfox spec §11: "accept/reject matrices per schema, subtree scoping, unregistered-doc_type passthrough, upgrade behavior")

Server integration + unit tests:
- **Accept/reject matrix per construct**: object/array/scalar type match & mismatch;
  `required`-present vs missing; `nullable` accepts `null` and rejects `null` when absent;
  `additionalProperties` closed-by-default rejects an undeclared key; explicit
  `additionalProperties` subschema accepts an open map and rejects a wrong-typed value; `{}`
  accepts any JSON.
- **Subtree scoping**: a registered `/system/stats` schema does not constrain `/system/other`; two
  disjoint subtrees on one doc_type both enforce.
- **Unregistered passthrough**: a doc_type with no registered schema writes freely; a subtree with
  no schema writes freely.
- **Embedded recursion**: an embedded child violating *its own* doc_type's schema rejects the whole
  command; a grandchild too.
- **Set-time validation**: malformed schema (bad `type`, unknown schema key,
  `additionalProperties` wrong shape) rejected; non-`/system`-descendant pointer (incl. bare
  `/system`, `/engine`, `""`) rejected; overlapping/duplicate
  `(doc_type, pointer)` rejected; unknown `schema_format` rejected; bounded-count/depth enforced.
- **Upgrade**: latest-wins (re-set changes the governing schema); a pre-existing doc invalid under a
  new schema is untouched until its next write, then rejected (no retroactive sweep).
- **Absent-subtree**: registering a schema does not compel the subtree's presence.
- **Rejection shape**: violation returns the new `DataError` on the rejected-intent path; the
  per-world seq is not consumed (transaction dropped).
- **Drift guard**: ts-rs ↔ Zod parity for `SchemaDeclaration` / `Schema`.

## 10. Rejected alternatives (with reasons)

- **Value constraints (`enum`, `min`/`max`, `minLength`, `pattern`)** — rejected: inspect a value's
  content/magnitude = game rules = semantic validation invariant 6 forbids the server. `enum` as a
  discriminator is half a feature without `oneOf` (can't type the variant it selects) and is the
  thin end of the wedge toward it. `pattern` is a server-side ReDoS surface; numeric bounds reopen
  the PosInt/Float/NaN edges that bit OCC in M13-0. The one non-semantic case (DoS via huge values)
  is already covered by the size cap. Value legality is tier-1 + fail-closed readers, which handle a
  wrong value by degrading to inert — no integrity hole.
- **Combinators (`oneOf`/`anyOf`/`if`-`then`/`$ref`)** — rejected: unambiguously the server
  evaluating "which fields are legal together" = mechanical validation; largest surface (recursive
  combinator eval, `$ref` cycle risk); direct invariant-6 violation.
- **`additionalProperties` open-by-default** (standard JSON Schema) — rejected: fail-open (a forgotten
  field silently permits anything); a guard's default must be the safe posture. Closed-by-default is
  low-cost because the open-map case sets it explicitly anyway, and any node can opt open.
- **Hand-set world-config schemas (no manifest)** — not chosen: loses module→schema cohesion and
  auto-teardown, duplicates authorship, and is less authentic for the reference-implementation
  purpose. Not precluded — the same set endpoint can accept a hand-set entry later if a non-module
  case appears (none today; D14 = one system per world, from a module).
- **Server reads module manifest files directly** — rejected: makes the server load/trust
  module-authored files as write-path input, coupling enforcement to manifest parsing/availability
  and eroding the no-third-party-content-on-the-server line. The GM-commit step is the clean trust
  seam.
- **Schema carried on the wire per-write / per-document** — rejected decisively: lets the *writer*
  supply the schema; a hostile client ships a permissive one and self-approves. Authority must be
  GM-controlled world state.
- **Retroactive re-validation on schema change** — rejected: the server can't migrate `system` data,
  so it could only reject the schema change (blocks evolution) or quarantine docs (unscoped repair
  UX); re-validating a whole world per change is unbounded. Breaks migrate-at-boundary.
- **Per-doc-version schema routing** — rejected: unbounded stateful schema history + per-document
  version negotiation (semantic complexity, no integrity benefit — tier-2 is a *current* floor); a
  hostile client claims an old permissive version to dodge the guard.

## 11. Out of scope / deferred

- Semantic/mechanical validation of system content (permanently server-side — invariant 6; lives in
  tier-1 + Phase-3 sandboxed validators per the Nightfox spec).
- Value bounds, enums, conditionals, cross-field invariants (tier-1's job).
- Migration / corpus re-validation / doc-version routing (F6).
- Schema guarding of `/engine`, `/permissions`, `/name`, `/embedded`-envelope shape (engine band is
  server-typed; the rest are envelope-structural already).
- Client-side tier-2 *enforcement* (the client validates via tier-1 Zod; the `Welcome`-broadcast
  registry is informational/parity only).

## 12. Seams touched (main @ 38886cf)

- `src/server/src/data/document.rs` — new `SchemaDeclaration` + `Schema` types (ts-rs-exported);
  the `Document` bands (`system`, `engine`, `base`) are unchanged.
- `src/server/src/data/validation.rs` — new `validate_system_schema_tree` (read-only, recursive),
  beside `validate_system_size` / `validate_engine_tree`.
- `src/server/src/data/engine/` — untouched (engine band out of scope); M13f is a sibling
  `system`-band gate, not an engine-registry entry.
- `src/server/src/data/sqlite.rs::apply_intent` — call `validate_system_schema_tree` in Phase-1
  after `validate_engine_tree`, with the pre-transaction-loaded world schema set; new
  `DataError::SchemaViolation` on the rejected-intent path.
- `src/server/src/data/repository.rs` + `sqlite.rs` — `world_schema_declarations` /
  `set_world_schema_declarations` (mirror the contract-declaration pair); storage row (JSON in
  world settings, like cap requirements/contracts).
- `src/server/src/http/routes.rs` — GM-only `get`/`set world schema declarations` + set-time
  `validate_schema_declarations` (mirror `validate_contract_declarations`).
- `src/server/src/ws/conn.rs` — include the schema set in `Welcome`.
- `src/client/core/src/wire.ts` — Zod mirror of `SchemaDeclaration` / `Schema` (drift guard).
- `docs/` — `PLAN.md` (M13f → done), `ARCHITECTURE.md` if invariant 6's structural-vocabulary note
  needs the tier-2 pointer; the `shadowcat-codebase-documents-permissions` skill (new tier-2 seam
  + invariant) and possibly a note in `-core`.

## 13. Pointers

- Parent spec: `docs/superpowers/specs/2026-07-15-m13-nightfox-system-design.md` §9 (M13f seed),
  §10 (security), §11 (testing); decisions D6/D13/D14.
- Invariant 6 + three-band shape: `docs/design/ARCHITECTURE.md` §2 invariant 6; M13-0 spec
  `docs/superpowers/specs/2026-07-15-m13-0-document-shape-design.md`.
- Precedents: `CapabilityRequirement`/`set_world_cap_requirements` (M6b Capability-Phase-2),
  `ContractDeclaration`/`set_world_contract_declarations`/`validate_contract_declarations`
  (`document.rs`, `routes.rs`, `sqlite.rs`).
- Enforcement model: `validate_engine_tree` (`validation.rs`) + `apply_intent` Phase-1
  (`sqlite.rs`).
- Relationships: `graphify query "apply_intent validate_engine_tree world_cap_requirements Welcome"`.
