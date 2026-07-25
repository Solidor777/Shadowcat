---
name: shadowcat-codebase-documents-permissions
description: "Use when touching Shadowcat documents, permissions, redaction, visibility tiers (all / gm_only / owner_or_gm), per-recipient broadcast filtering, the search index, the `Document.base` merge-snapshot field (its authz/size-cap/egress rules — not the client merge algorithm), or the client wire/Zod types. Covers src/server/src/data and its src/client/core wire mirror. Invoke shadowcat-codebase-core first."
---

# Shadowcat — Documents & Permissions

Orientation for the document data model and the server-side, per-recipient redaction layer.
Server is the source of truth; the client only mirrors the wire shape.

## Purpose

A document is a typed envelope (id, type, owner, permissions, `schema_version`, display `name`)
carrying **three bands (M13-0)**: the envelope `name: Option<String>` itself, a typed `engine`
JSONB body (present only for engine-defined `doc_type`s, strictly ingress-validated), and an
opaque `system` JSONB body the engine never interprets semantically. Permissions are enforced
server-side **per recipient**: hidden fields are stripped before transmission, never
sent-then-hidden. This subsystem also owns the visibility-partitioned full-text index.

## Key files & seams

- `src/server/src/data/document.rs` — the `Document` envelope: `name: Option<String>` (universal
  display name, `#[serde(default)]`) and `engine: Option<serde_json::Value>` (`#[ts(type =
  "unknown")]`, present iff `doc_type` is engine-defined) alongside the pre-existing `system`
  body; `enum Visibility { All, GmOnly, OwnerOrGm }` (the per-property visibility tiers);
  `PermissionSet.gm_role: Option<DocRole>` (`#[serde(default)]`, ts-rs exported) — see Hard
  Invariants below.
  - `base: Option<serde_json::Value>` (M13e, `#[serde(default)]`, `#[ts(type = "unknown")]`) —
    the opaque 3-way-merge snapshot the generic templates system stamps onto an instance at
    stamp/pull/push/revert time (see `shadowcat-codebase-templates` for the client-side
    `MergeBase` shape/algorithm). Purely a client-owned blob: the server never interprets it.
    `required_cap_for_path` (`permission.rs`) maps `/base` (and any subtree under it, e.g.
    `/base/system/hp`) to `cap::WRITE_FIELDS`, the same capability that gates `/name`/`/engine`/
    `/system` — no dedicated capability. `/source` (the sibling field naming what a document is
    an instance OF) stays unmapped/immutable — `required_cap_for_path` returns `None` for it, so
    no write path can ever re-target an existing document at a different template.
  - `world_of(doc: &Document) -> Option<Uuid>` (`pub(crate)`, Phase A) — the single chokepoint for
    "which world does this doc scope to" (`Scope::World { world_id } => Some(world_id)`,
    `Scope::Compendium => None`). Two call shapes: (1) a caller that already knows the world it
    scopes to PINS a doc reference by comparing `world_of(&doc)` against that known `world_id` —
    `ws/conn.rs`'s `scene_ping_permitted` (refuses a scene doc from another world even for a
    member of both) and `chat::handle_send_message`'s actor-attribution gate
    (`shadowcat-codebase-chat`). (2) an HTTP by-id route with no caller-known world instead
    DERIVES `world` from the doc itself — `http/routes.rs`'s `get_document`/`patch_document`/
    `delete_document` do `let world = world_of(&doc).ok_or(AppError::NotFound)?`, then use that
    extracted world as the authority for the subsequent `permission_context` lookup; `None`
    (a compendium doc) 404s uniformly with the missing-doc case, matching the behavior
    routes.rs's own now-deleted local copy used to return (existence-hiding). No remaining
    `Scope::World`/`Scope::Compendium` match duplicates this decision anywhere in the server —
    the ONE place to extend it is here.
- `src/server/src/data/engine/` (M13-0) — the typed `engine`-band structs + the ingress-validation
  registry, one module per doc-type family (`token.rs`, `scene.rs`, `geometry.rs`,
  `registries.rs`) plus `mod.rs`: `is_engine_doc_type(doc_type) -> bool` (the 17-entry registry:
  `token`/`scene`/`wall`/`region`/`light`/`drawing`/`template`/`actor`/`message`/
  `world-settings`/`vision-modes`/`light-gradation`/`chat-settings`/`dice-settings`/
  `channel-registry`/`faction-registry`/`condition-registry`), `validate_engine(doc_type, engine)
  -> Result<(), DataError>` (deserializes the body against that doc_type's typed struct;
  `deny_unknown_fields` on every struct — engine-defined types WITHOUT an `engine` body error, and
  non-engine types WITH one error too, so a non-engine `doc_type` can never smuggle a typed body
  in). `src/server/src/data/command.rs`'s `validate_engine_tree` is the recursive ingress
  chokepoint — called on every Create/Update POST-IMAGE (after all `FieldChange`s apply),
  including embedded children, so a wholesale `/engine` replacement, a leaf `/engine/x` write, and
  an embedded child's engine write are all covered by one call site.
- **Token ownership is EFFECTIVE, and it lives in THIS subsystem's files.** `effective_owner`
  (`data/permission.rs`) and `load_effective_owner` (`data/sqlite.rs`) resolve
  `token's own /owner, else the LINKED actor's owner` at authz time — never stamped. `/owner` is
  Update-writable under `cap::EDIT_PERMISSIONS`; `DocRole::Owner`'s BUILT-IN floor is
  `{READ, WRITE_FIELDS}` and excludes it, but the floored role also selects additive
  `by_role[Owner]` grants. **State the precedence rule exactly ONCE** — duplicating it via a
  short-circuit in the DB join let an inverted-precedence mutation survive. Full rule, the
  fail-closed list, and the instanced-token exclusion: `shadowcat-codebase-actors-tokens`.
  **The EGRESS half is this skill's own territory and is a KNOWN under-permit:**
  `filter_properties` / `collect_hidden` / `filter_command` and the document routes still resolve
  `is_owner` from the LITERAL `doc.owner`, so an inheriting owner can move a token and see through
  it while counting as a stranger for its `owner_or_gm` tiers and `/base`. Under-permit BY
  CONSTRUCTION, and the reason is subset-ness, not call ordering: literal `is_owner` is
  `doc.owner == user`, while the effective rule adds the linked-actor case ONLY when `doc.owner` is
  `None` — so the literal set is a strict SUBSET of the effective set and can never be the more
  permissive of the two. Logged in `docs/TODO.md`.
- **`command::apply_field_change(v, ch)` is THE store-equal mutation rule — every store of document
  state, authoritative or derived, applies a `FieldChange` through it. Never hand-write a
  `remove`/`set` branch.** One function, one statement of the rule, repo-wide (client mirror:
  `store.ts`'s `applyOperation`, shared by `DocumentStore` and `OptimisticClient`). Callers split
  only on error handling, and the split is meaningful: the two AUTHORITATIVE loops
  (`sqlite.rs`'s `apply_command` and `apply_intent` Phase 2) propagate with `?` so a bad change
  aborts before commit; the DERIVED mirrors in `scene/mod.rs` go through `mirror_field_change`
  (logs) / `reapply_changes` (adds the `Document` round-trip), because `apply_op` runs on the
  already-committed broadcast/replay path where the ECS has no authority to reject. **Why this is a
  hard invariant (Task 14i, `[sec]`, fixed a Critical):** `SceneEcs::apply_op` once mirrored with an
  unconditional `set_pointer`, ignoring `ch.remove`, while the store honoured it — so a `remove:
  true` change left the DB with the key ABSENT and the ECS holding the caller's unconstrained
  `new`. With `WRITE_FIELDS` alone, a player removing `/engine/actor_id` with a foreign actor id in
  `new` made the DB read "unowned" (nobody may write) while the ECS resolved ownership to another
  actor's owner — who then gained the token as a vision source. **Vision widened exactly where
  write authz refused.** Note the two call-site trust levels through one helper: `apply_op` sees
  committed changes, `token_move` sees CLIENT-PROPOSED, not-yet-authorized ones. `MirrorInput::
  {Committed, Proposed}` carries that and decides the LOG LEVEL, not the mutation: `error!` on a
  committed failure (an invariant breach), `debug!` on a proposed one (routine malformed input).
  Backwards in either direction is a real defect — `error!` on the proposed path is an
  attacker-controllable log channel. Both pinned by mutation-checked tests; rationale in full at
  `scene/mod.rs`'s `MirrorInput`.
- `src/server/src/data/validation.rs`'s `validate_field_change` — ingress shape rule: `remove: true`
  must carry a null `new`. Defense-in-depth only; the mirror is correct independently, which
  matters because replay and broadcast can carry shapes ingress never validated.
- `src/server/src/data/command.rs`'s `FieldChange.remove: bool` — a leaf-level object-key-removal
  discriminator on the existing `Operation::Update`/`FieldChange` wire shape, not a new `Command`
  variant: it reuses the same OCC pre-image check (`old`) and capability check
  (`required_cap_for_path`) as an ordinary `set` change. `remove: true` deletes the object key at
  `path` instead of writing `new` (unused, conventionally `Null`), making the key genuinely
  absent (`null` != absent); `#[serde(default)]` on ingest and `skip_serializing_if` on egress
  keep it omitted on the wire when false, and the client Zod mirror makes it optional to match.
  The mutation itself is `remove_pointer(root, pointer)`: **object keys only — a leaf array-index
  removal (e.g. `/tags/1`) is rejected with `DataError::BadPath`, unmutated** (array shrink is
  whole-array replacement only, per the merge-engine invariant; a leaf remove of an index has no
  defined shift semantics), while a missing OR explicit-`null` intermediate ancestor is treated as an
  already-absent no-op rather than an error. Sibling mechanism to `set_pointer` (leaf-SET-only: it
  can create or overwrite a key/index but can never delete a key or resize an array) — the pair
  covers set vs. remove, with array resize handled exclusively by whole-array replacement, not by
  either pointer op. **INVARIANT — all three pointer ops treat a `null` INTERMEDIATE as absent, in
  lockstep on both the server (`command.rs`) and the client mirror (`store.ts`):** `set_pointer`
  descends by replacing a `null` intermediate with a fresh object (`Option<T>` engine fields with no
  `skip_serializing_if` serialize as `null`, so this is the common case — e.g. a scene's
  `/engine/vision` override on a default-built scene doc); `remove_pointer` no-ops through it; reads
  yield absent (the client's `getPointer` → `undefined`; serde_json's `Value::pointer` server-side →
  `None` — there is no bespoke server `get_pointer`). The LEAF null-vs-absent distinction is preserved (`null !=
  absent` for a leaf value). Forking this null-handling across the two languages is the never-fork
  defect class — parity is pinned by matching tests on each side.
- `src/server/src/data/permission.rs` — the redaction core:
  - `resolve_access(user, world_role, doc) -> Access` (and `resolve_access_world`) builds the
    per-connection `Access { caps, all, see_gm_only, is_owner }`.
  - `effective_role(user, world_role, doc) -> Option<DocRole>` — the shared floor-resolution
    helper both `resolve_access` and `resolve_access_world` call; `None` means the unconditional
    GM/admin short-circuit applies (see `gm_role` invariant below), `Some(role)` means the caller
    must resolve capabilities from that per-document role floor like any other actor.
  - `Access::can_see(v: Visibility)` is the single predicate: `GmOnly => see_gm_only`,
    `OwnerOrGm => see_gm_only || is_owner`, `All => true`.
  - `filter_properties(doc, access)` strips hidden **properties** from an outgoing doc — a
    PROPERTY-visibility gate only (see Hard Invariants: it does NOT decide whole-document
    withholding). `/system`, `/engine`, and `/name` overrides all **null the field rather than
    strip the key** (M13-0 generalized this from a `/system`-only special case) — dropping the key
    would either fail re-deserialization (`system`) or be indistinguishable from a doc that never
    had a name/engine body, breaking the client's stable envelope shape; nested pointers one level
    down still strip normally. `/base` gets the same null-not-strip treatment, but its visibility
    is NOT driven by `property_overrides` at all — see the `base` egress invariant below.
  - `redact_change(change, gm_only)` redacts field-level change events on the broadcast path;
    `collect_hidden` (its companion that builds the `gm_only`/hidden-path list for embedded-depth
    redaction) applies the same unconditional `/base` policy at every embedded depth.
- `src/server/src/data/search.rs` — `index_content` (full) vs `index_content_public` (redacted):
  the index is **partitioned by visibility**, not redacted after the fact. Indexes `name ∪ engine
  ∪ system` (M13-0 added `name` and `engine` as leaf sources alongside `system`, same
  string-leaf-walk treatment; `index_content_public` needs no structural change — it re-runs
  `filter_properties` first, and a nulled `/engine`/`/name` band simply contributes nothing).
- `src/server/src/data/{repository.rs,validation.rs}` — `Repository` trait (storage seam; SQLite today, Postgres-capable later) +
  structural validation (size caps, field-path validity, `deny_unknown_fields`); `validation.rs`
  applies the same `MAX_SYSTEM_BYTES` (256 KiB) cap to `engine` as to `system` (M13-0), checked
  independently per block. `base` (M13e) gets the SAME independent size cap
  (`validate_system_size`'s cap function, shared across all three blocks) but is explicitly
  `EXEMPT` from `validate_engine_tree` — the tree walker only ever visits `/engine`, never
  `/base`, because `base` is a historical snapshot that may legitimately hold a stale
  `engine`/`system` shape from before the current schema (a template edited after an instance
  stamped from it); running current-schema validation against a deliberately-historical blob
  would be wrong, not defense-in-depth.
- `data/validation.rs::validate_system_schema_tree` (M13f, tier-2) — a read-only recursive
  `system`-band structural gate, run beside (not instead of) `validate_engine_tree`.
  `validate_value_against_schema(value, schema) -> Result<(), SchemaMismatch>` is the pure
  accept/reject matcher over the type-tree grammar. Types: `Schema`/`SchemaType`/
  `AdditionalProperties`/`SchemaDeclaration` (`data/document.rs`). Set-time authority:
  `http/routes.rs::validate_schema_declarations` (strict `/system/…`-descendant
  `subtree_pointer`, per-`doc_type` overlap/dup rejection, `schema_format` version gate via
  `SCHEMA_FORMAT_V1`, and resource bounds `MAX_SCHEMA_DECLARATIONS`/`MAX_SCHEMA_NODES`/
  `MAX_SCHEMA_DEPTH`), reached only through the GM-only `GET`/`PUT /api/worlds/{id}/schemas`
  pair (`routes::get_world_schema_declarations`/
  `set_world_schema_declarations`). Registry storage: `Repository::world_schema_declarations`/
  `set_world_schema_declarations` (`data/sqlite.rs`), a per-world settings row keyed by
  `world_schemas_key(world)` — same storage shape as other world-settings singletons, not a new
  table. Broadcast: `ServerMsg::Welcome.schema_declarations` (parity only; the client never
  enforces from it, see the Hard Invariants entry below).
- `src/server/src/data/sqlite.rs`'s `apply_intent` — the singleton-`doc_type` create-gate:
  `SINGLETON_DOC_TYPES` (world-settings/faction-registry/condition-registry/chat-settings/
  dice-settings — 5 entries; `light-gradation`/`vision-modes` are real engine doc_types but are
  NOT singleton-gated, and `channel-registry` has no gated const at all) + a tx-scoped
  `singleton_doc_exists` DB check reject a second `Create` of a singleton type. That DB check alone
  closes only the CROSS-CALL race (relies on the single-writer `max_connections(1)` pool + a
  tx-scoped executor). A `claimed_singletons: HashSet<String>` seeded before Phase 1's per-op loop
  and checked alongside the DB read closes the separate INTRA-BATCH race: two same-doc_type
  singleton `Create`s inside ONE `apply_intent` call's `ops` both read the DB as empty during
  Phase-1 validation (validated before any Phase-2 insert), so the DB check alone lets both pass; the
  `HashSet` is populated only after both checks pass, so the second op in the same batch is rejected
  regardless of N or ordering. A rejection unwinds the WHOLE `apply_intent` call (no partial insert of
  the batch's other ops) — this is the same whole-batch-rollback semantics every other
  `apply_intent` validation failure already has, not a new rollback path.
- `src/server/src/data/sqlite.rs`'s `apply_intent` — Phase-1 OCC pre-image comparison
  (`values_semantically_eq`) is **numeric-variant-aware, not raw equality** (M13-0). Same-variant
  integer pairs (both `PosInt`/`NegInt`) compare EXACTLY as `i128`, no magnitude limit — this never
  touches `f64`, so two distinct large integers past 2^53 never alias into a false match. Only a
  genuinely-mixed pair (one integer variant, one `Float`) falls back to an `f64` comparison, gated
  by a `|n| <= 2^53` exactness guard (`MAX_EXACT_F64_INT`) — outside that range a mixed-variant
  pair is unconditionally unequal, never a false-positive OCC pass. Recurses through
  `Object`/`Array` structure; any non-Number mismatch falls back to serde's derived `PartialEq`.
  `apply_intent` is also the tier-2 enforcement chokepoint: `validate_system_schema_tree` runs
  immediately after `validate_engine_tree`, at BOTH call sites — Create (Phase-1, against the
  new document) and Update (Phase-2, against the merged post-image: existing row + applied
  `FieldChange`s, never the pre-image) — recursing through embedded children by their own
  `doc_type` exactly as `validate_engine_tree` does. A violation returns `Err` before the
  transaction commits, so the per-world seq counter is NOT consumed on rejection, and surfaces
  to the client via the pre-existing rejected-intent path (`DataError::SchemaViolation { pointer,
  reason }`) — no new wire frame.
- `src/client/core/src/wire.ts` — Zod mirror: `VisibilitySchema = z.enum(["all","gm_only",
  "owner_or_gm"])`, `property_overrides`. ts-rs generates the TS types from the Rust source.
- `src/client/core/src/scene-docs.ts` — `ITEM_DOC_TYPE = "item"`, `ItemSystem`, `buildItemDoc`
  (M12c): a **client-only doc_type** — the server has NO Rust-side knowledge of `item` and
  requires none, since `doc_type` is an unconstrained wire string and `system` is opaque JSONB the
  server never interprets. An item document lives standalone (top-level, `parent_id: null`) or
  embedded in an actor's inventory (`actor.embedded.item[]`); write-site resolution for an embedded
  item is `/embedded/item/<idx>/system`, the same one-level `embeddedPath` scheme
  `resolveDocRef` uses for any embedded child ([[shadowcat-codebase-sheets]]).

## Hard invariants

- **Redaction is fail-closed and owner-aware.** `can_see` is the one chokepoint across every
  egress path; a partial-visibility tier (`OwnerOrGm`) uses a distinct flag — never overload the
  GM see-all boolean, or you leak `GmOnly` to owners [[ownerorgm-tier-no-widen]].
- **`filter_properties` is a PROPERTY-visibility gate, NOT a whole-document READ gate.** It only
  strips individual properties whose override is `GmOnly`; it does not withhold, and cannot be
  used to withhold, an entire document. Whole-doc withholding is decided entirely by callers
  checking `access.has(cap::READ)` BEFORE including the op/hit/row at all (see the
  `filter_command`'s `Create`/`Delete`/`Update` branches, `search`'s per-hit filter, and
  `query_documents`/`get_document`) — `filter_properties` runs only after that gate has already
  let the doc through. Any future egress path must follow the same order: check `has(cap::READ)`
  first, then (optionally) `filter_properties` for property redaction. Gating whole-doc delivery
  on `see_gm_only`/GM-ness alone instead of `has(cap::READ)` would leak a `gm_role`-capped
  document (see below) straight past its intended cap.
- **`PermissionSet.gm_role: Option<DocRole>` makes the GM's usual unconditional access
  conditional, per document.** `resolve_access`'s GM branch normally short-circuits to
  `Access { all: true, see_gm_only: true, is_owner: true, caps: {} }` for every `WorldRole::Gm`
  user, before any document-level permission is consulted — correct and load-bearing for every
  pre-existing document type (actors, scenes, secret regions: the GM must always see a secret
  region even though it's `default: DocRole::None`).
  - `gm_role: None` (the field's default via `#[serde(default)]`; every document type that
    predates this field deserializes to `None`) preserves that unconditional short-circuit
    exactly — no behavior change for anything but the new consumer below.
  - `gm_role: Some(role)` caps a GM to the SAME per-document role-floor resolution every other
    actor uses: `effective_role` looks the GM up in `doc.permissions.users` first, falling back to
    `role` (NOT `doc.permissions.default`) only if the GM isn't individually listed. This lets a
    document deny a GM by default (`Some(DocRole::None)`) while still admitting a GM who is
    individually granted a role in `users`, or grant EVERY current GM a role
    (`Some(DocRole::Observer)`) without listing any of them by name — resolved fresh on every call,
    so promotion/demotion to `WorldRole::Gm` takes effect immediately, not a frozen snapshot.
  - `resolve_access_world` deliberately reuses this SAME `effective_role` helper (not
    `doc.permissions.default`) to layer world-level capability grants, so a world-default grant
    for the GM's fallback role applies consistently even when that GM is `gm_role`-capped — the
    original (pre-refactor) sketch would have recomputed the role independently from
    `doc.permissions.default` here and silently diverged for a capped GM; this was a real bug
    caught before it shipped, not a hypothetical.
  - First (and so far only) consumer: `shadowcat-codebase-chat`'s `Audience`→`PermissionSet`
    mapping (`Whisper` sets `Some(DocRole::None)`, `GmOnly` sets `Some(DocRole::Observer)`,
    `Public` leaves it `None`).
  - `see_gm_only` stays `true` for any `WorldRole::Gm` actor regardless of `gm_role` capping —
    only `all`/`caps` (whole-document READ) become floor-gated. A `gm_role`-capped GM therefore
    still passes property-tier (`GmOnly`/`OwnerOrGm`) checks on any document they DO have READ on;
    the cap is purely about whole-document access, not GM-ness for property visibility.
- **The search index is visibility-partitioned.** Redacting only the returned doc leaks GM-only
  text via snippet/match/score — index public and full content separately
  [[search-index-must-be-visibility-partitioned]].
- **`engine` ingress validation is strict and fail-closed; `system` stays structural-only.**
  `validate_engine_tree` rejects an engine body with an unknown field, a wrong-typed field, a
  missing body on an engine `doc_type`, or a present body on a non-engine `doc_type` — this is a
  REAL semantic-shape gate, unlike `system`'s size/JSON-validity-only structural check. Do not
  conflate the two bands' authority models when reasoning about what the server does and doesn't
  validate.
- **OCC pre-image comparison at `apply_intent` is numeric-variant-aware, not raw equality.** A
  naive raw-`==` assumption is now wrong: `values_semantically_eq` (`data/sqlite.rs`) exists
  because JS clients cannot preserve the whole-number-vs-float distinction through a JSON
  round-trip (e.g. a server-computed `100.0` comes back over the wire and reparses as `PosInt(100)`,
  which raw `==` would treat as unequal to a stored `100`, causing a spurious `Conflict` on an
  otherwise up-to-date write). See the `sqlite.rs` seam entry above for the exact comparison rule.
- **`/base`'s egress visibility is hardcoded `OwnerOrGm`, UNCONDITIONAL — never driven by
  `property_overrides` (M13e).** `filter_properties` and `collect_hidden`/`redact_change` both
  independently hide `/base` from any recipient who is neither the document's owner nor a GM,
  regardless of what `permissions.property_overrides` says (a doc author cannot loosen or
  tighten `/base`'s visibility by setting an override on it — there is none to set). This is
  load-bearing: `base` is the merge-engine's raw pre-image snapshot of a document's
  `name`/`engine`/`system`/`embedded` bands, which can itself contain content an ordinary
  `GmOnly`/`OwnerOrGm` property override elsewhere on the doc was hiding from this same
  recipient — leaking the snapshot would bypass that override. Any future change to `base`'s
  redaction must keep both call sites (whole-doc `filter_properties`, broadcast-delta
  `collect_hidden`) in sync; they are two independent code paths, not one shared chokepoint.
- **Tier-2 (M13f) validates the `system` band's SHAPE only, never values — it EXTENDS invariant 6
  (three-band document shape), it does not replace it.** `engine`-band validation
  (`validate_engine`/`validate_engine_tree`) remains the separate, pre-existing REAL semantic
  ingress gate for the 17 engine-defined doc types (see the `engine ingress validation` invariant
  above); tier-2 is the `system`-band's analogous but strictly structural enforcement floor. The
  declarable `Schema` type-tree grammar (`type`/`properties`/`required`/`items`/
  `additionalProperties`/`nullable` — no `enum`, no numeric/string bounds, no `pattern`, no
  `anyOf`/`oneOf`/combinators, ever) cannot express a value rule by construction, so it can never
  become a semantic gate no matter what a GM configures; `additionalProperties` is closed by
  default (`None` behaves as `Bool(false)`, matching JSON Schema's spec-divergent-but-documented
  default here). Value legality stays where it always was: tier-1 (client-side Zod, per module)
  plus fail-closed readers. The server still runs no third-party code and never interprets what a
  `system` value MEANS — only whether its declared SHAPE matches.
- **The document writer NEVER supplies the schema that judges it.** The `SchemaDeclaration`
  registry is GM-controlled per-world state, set only through the GM-only
  `/api/worlds/{id}/schemas` endpoint pair (`require_gm`), loaded once before the `apply_intent`
  transaction and enforced read-only against Create/Update post-images — an ordinary writer has no
  path to alter the schema that will judge their own write. The Welcome-broadcast
  `schema_declarations` is informational parity only (lets a client preemptively validate/UX-hint)
  and carries zero enforcement authority; the server-side `apply_intent` load is the only copy
  that matters.
- **Path-prefix authz covers ancestor (subtree-replacing) writes AND whole-doc Create**, not just
  descendant field updates [[path-prefix-authz-covers-ancestor-and-create]].
- **The singleton create-gate must close BOTH cross-call and intra-batch duplicate-`Create` races,
  via two independent mechanisms.** A tx-scoped DB existence check alone is sufficient for
  cross-call races (serialized by the single-writer pool) but NOT for two same-doc_type Creates
  inside one `apply_intent` batch, since Phase 1 validates every op before Phase 2 inserts any of
  them — both same-batch DB reads see an empty table. The in-memory `claimed_singletons` HashSet
  closes that second gap; do not remove either mechanism assuming the other already covers it.
- **Check-then-act across two queries needs one transaction** — TOCTOU-racy even at
  `max_connections(1)` [[two-query-guard-needs-tx]].
- **`INSERT … ON CONFLICT(id)` on a mutated id duplicates rather than moves** the row
  [[upsert-on-conflict-duplicates-not-moves]].

## Gotchas

- **Wire types are generated** — change the Rust `Visibility`/`Document`, regenerate ts-rs, then
  mirror in the Zod schema (a drift guard enforces parity). Never hand-edit `src/types/generated`.
- **A naive raw-equality assumption about OCC pre-images is wrong (M13-0).** Any code (or reviewer)
  reasoning about `apply_intent`'s Phase-1 conflict check must account for
  `values_semantically_eq`'s numeric-variant awareness — see the Hard Invariants entry above and
  the `sqlite.rs` seam. Treating pre-image comparison as plain `serde_json::Value` `==` will
  misdiagnose both false-conflict and false-pass scenarios.
- **Embedded copies need a deep clone** — `{...doc}` aliases nested `system`/`permissions`/
  `embedded` until the wire round-trip; use `structuredClone` at construction
  [[embedded-copy-needs-deep-clone]].
- **Test harness:** `doc(perms, system)` not `doc(id)`; an `owner_id` is a FK, so a test owner
  must be a real `create_user`, not a synthetic `Uuid` [[server-test-doc-helper-and-owner-fk]].

## Pointers

- Rationale: `docs/design/M2-data-foundation.md`; invariants in `docs/design/ARCHITECTURE.md`
  §2 invariant 4 (per-recipient permissions) + invariant 6 (three-band document shape) + §6 (data
  model). M13-0 design: `docs/superpowers/specs/2026-07-15-m13-0-document-shape-design.md`. M13f
  (tier-2 structural schema registry) design:
  `docs/superpowers/specs/2026-07-18-m13f-server-schema-registry-design.md`.
- Relationships: `graphify query "document permissions redaction filter_properties can_see"`,
  `graphify path "permission.rs" "search.rs"`.
- Deferred merge model: [[document-inheritance-merge-model]].
- `shadowcat-codebase-chat` — the first (and so far only) consumer of `gm_role`, via its
  `Audience` enum's `PermissionSet` mapping (see that skill's Key files & seams).
- `shadowcat-codebase-templates` — the client-side 3-way merge engine + `TemplatesController`
  that produces/consumes `base`; this skill owns only the server-side field/authz/redaction/size
  facts above.
