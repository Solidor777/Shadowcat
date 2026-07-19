# M13e — Generic Templates & 3-Way Merge Engine (design)

Status: approved 2026-07-18. Engine-level checkpoint of M13; stays in the Shadowcat repo.
Realizes the deferred provenance-based document-inheritance model
([[document-inheritance-merge-model]], `docs/design/M2-data-foundation.md` §1 Deferred + §2,
Nightfox spec `2026-07-15-m13-nightfox-system-design.md` §8 / D5).

## 1. Goal & principle

Provide **generic template documents + an explicit, on-command 3-way pull / push / revert merge
engine** as a first-class Shadowcat engine capability. A document may be *stamped* from any other
document (its template/source); thereafter the child and its template can be synced on command,
never automatically and never as live inheritance.

**Built on Shadowcat's own merits, not to serve Nightfox.** Nightfox's "templates" are merely
easy presets for spawning actors — that need is satisfied by the *stamp* half alone, through the
generic seam, with zero Nightfox-specific server or engine code. The full merge engine closes the
long-deferred inheritance model and is the substrate compendium pull/push rides later. Nightfox is
its first *consumer*.

**Core mechanism: the merge is computed entirely client-side and applied as an ordinary batched
`Update`.** It rides the existing optimistic/OCC/broadcast/event-log path. The server gains no
merge logic, no new operation type, and no new wire frame — only the storage + authz for the base
snapshot a 3-way merge needs.

## 2. Decisions locked

| # | Decision |
|---|---|
| E1 | **Provenance-based, explicit, bidirectional 3-way merge — never live, never automatic** (D5, M2 §2). A child carries a `source` pointer; a human runs pull / push / revert. |
| E2 | **Base retrieval = a snapshot stored on the child** at stamp/last-sync time. Self-contained, survives parent deletion, needs no server version-history machinery (M2 §2 rationale). |
| E3 | **Templates are not a new `doc_type`.** Any document is a potential template; "template" = the role a document plays when others are stamped from it. (The engine `template` doc_type is the unrelated scene AoE-measurement shape.) |
| E4 | **Merge scope = the `name` + `engine` + `system` + `embedded` bands** (recursively). Envelope identity/authz (`id`, `owner`, `permissions`, `parent_id`, `source`, `base`, `doc_type`, `schema_version`, `scope`) stay child-local at every level and are never merged. |
| E5 | **Conflict resolution v1 = a field-level review modal.** A field changed on both sides since the last sync surfaces `base → template → mine`; the human picks a winner per field before any write. Disjoint fields merge automatically. |
| E6 | **Arrays merge wholesale-replace; objects/maps merge key-level** (M2 §2 default; Nightfox `system.stats`/`modifiers` are maps precisely for this, D11). |
| E7 | **Embedded children merge recursively, correlated by provenance** — an instance child matches its template child iff `instanceChild.source.id == templateChild.id`. |
| E8 | **Placement fields are instance-local and never merge**: for `token`, `/engine/x`, `/engine/y`, `/engine/rotation`. A small per-`doc_type` exclusion set so a generic template can never relocate its instances. |
| E9 | **Push is client-side, same-world, scoped to what the pusher can see + write** (the client store's natural scope). Cross-world push is deferred. |
| E10 | **Minimal server surface**: one opaque envelope field `base`, one `required_cap_for_path` mapping (`/base` → `WRITE_FIELDS`), one size-cap extension. `source` stays immutable. No server-side merge. |

## 3. Data model

### 3.1 Envelope fields

`Source { id: Uuid, pack: Option<String>, version: u32 }` **already exists** on the envelope,
bidirectionally indexed (`documents_by_source`, `idx_documents_source`). It is **immutable after
stamp** (`required_cap_for_path("/source") == None`). `version` is informational (set at stamp,
not bumped by merges — sync state is derived by comparing `base` to the parent's current state,
§6.4, so no monotonic counter is needed).

**New field `Document.base: Option<serde_json::Value>`** — an opaque snapshot of the child's
mergeable content at last sync (stamp or a successful pull/push/revert). Present only on stamped
children. Shape (client-owned; the server never interprets it):

```
Base = {
  name:     string | null,
  engine:   unknown | null,          // the engine band verbatim at sync time
  system:   unknown,                 // the system band verbatim at sync time
  embedded: Record<collection, EmbeddedBaseChild[]>   // full mergeable content of each child
}
EmbeddedBaseChild = {
  sourceId: Uuid,                    // the correlation key: this child's source.id at sync time
  name:     string | null,
  engine:   unknown | null,
  system:   unknown,
  embedded: Record<collection, EmbeddedBaseChild[]>   // recursion (Shadowcat embedding is finite-depth)
}
```

`base` is stored **top-level only** — embedded children carry `source` (for correlation) but no
`base` of their own; the recursive 3-way reads each node from the corresponding subtree of the one
top-level blob. `base` is **exempt from engine ingress validation** (`validate_engine_tree` is not
run over it): it is a historical snapshot that may predate the current engine schema.

### 3.2 Size

`validate_system_size` bounds `base` at `MAX_SYSTEM_BYTES` (256 KiB), independently of `system`
and `engine`. A stamped document therefore costs roughly twice its mergeable size; this is the
documented cost of self-contained base retrieval (M2 §2), policed by the existing caps.

### 3.3 Authorization of the base field

`required_cap_for_path` maps `/base` and `/base/…` → `cap::WRITE_FIELDS`, so a merge (an owner- or
GM-initiated operation) may refresh it. `/source` remains unmapped (immutable). Provenance is not
a security boundary in a trusted self-hosted VTT; `base`/`source` integrity is a correctness
concern, not an authz one.

**Egress: `/base` is hardcoded `OwnerOrGm` visibility, non-overridable, on BOTH redaction
chokepoints** (`filter_properties` whole-document egress, `collect_hidden`/`redact_change`
field-level broadcast egress) — found during Task 2's buddy-check, not anticipated in the original
design. `base` is a snapshot of the document's own `name`/`engine`/`system`/`embedded` bands,
which can carry `GmOnly`/`OwnerOrGm`-hidden fields; the snapshot has no property-level visibility
information of its own (`EmbeddedBaseChild` carries no `permissions`), so mirroring individual
hidden fields into the snapshot's structure is not attempted. Instead, the whole field is treated
as `OwnerOrGm` unconditionally: only the document's owner or a GM ever legitimately needs `base`
(to compute a pull/push/revert), so hiding the entire snapshot from every other recipient closes
the leak completely and matches who the feature actually serves. This mirrors the existing
`OwnerOrGm` tier (`Access::can_see`) but is NOT driven by `property_overrides` — it is unconditional
server policy for this one field, the same way `/permissions` itself is always
`cap::EDIT_PERMISSIONS`-gated regardless of any override.

## 4. The four operations

All four are pure client-core functions that produce document ops; the caller dispatches them
through the existing `dispatchIntent` seam.

### 4.1 Stamp (create-from-template)

`stampInstance(source: WireDocument, opts): WireDocument`

- Deep-clone (`structuredClone`, [[embedded-copy-needs-deep-clone]]) the source's `name`, `engine`,
  `system`, and `embedded` bands into a **new** document: fresh `id`, the initiator's own
  `owner`/`permissions` (never the template's), `parent_id`/`scope` per the caller.
- Set `source = { id: source.id, pack: source.scope.pack ?? null, version: source.source?.version ?? 1 }`.
- **Recursively** assign every embedded child a fresh `id` and set its
  `source = { id: templateChild.id, pack: …, version: … }` (correlation for future merges).
- Capture `base` = the snapshot of the just-stamped mergeable content (§3.1).
- Dispatched as a `Create` op (whole-document authz via `cap::CREATE`; no field-path authz issue).

*This is the half Nightfox consumes: its actor-create UI calls `stampInstance(presetActor, …)`.*

### 4.2 Pull (child ← template)

`computePull(child, template): MergePlan` then, after conflict resolution, dispatch one `Update`.

- 3-way merge the template's current state into the child, preserving the child's local diffs (§5).
- Emits merged-band `FieldChange`s + a `/base` refresh (`old` = child's current `base`, `new` =
  the template's current mergeable snapshot), all with **real OCC pre-images** read from the
  child's current stored values.
- Authorized when the initiator is the **child's owner or a GM** AND holds the caps the emitted
  changes require (`WRITE_FIELDS`; `MANAGE_EMBEDDED` iff embedded children change).
- Requires the template to be present in the initiator's store; otherwise pull is unavailable
  (with a visible reason).

### 4.3 Push (template → instances)

`findInstances(templateId)` → for each instance the pusher can see + write, `computePull` from the
template's perspective (base = that instance's base, parentNow = the template, childNow = the
instance) and dispatch one `Update` per instance.

- Authorized when the initiator is the **template's owner or a GM**.
- Same-world only (client-store scope, E9). An instance with unresolved conflicts is surfaced in
  the modal (aggregated across instances); instances with only disjoint changes apply directly.

### 4.4 Revert (child → template)

`computeRevert(child, template): Update` — discard the child's local diffs on the merged bands:
set every mergeable path to the template's current value, drop child-added embedded children,
restore template-deleted ones, and refresh `base`. No conflicts are possible (the child side is
discarded). Authorized like pull (child owner or GM).

## 5. Merge algorithm (client-core, pure, order-independent)

`merge3(base, parentNow, childNow, exclusions): { autoChanges, conflicts }` over the mergeable
bands as one JSON tree.

### 5.1 Scalar/object/array diffing

- Recurse **objects** key-by-key. **Arrays** are opaque leaves (any change → wholesale replace of
  the whole array path). **Scalars** are leaves.
- `parentΔ = structuralDiff(base, parentNow)`; `childΔ = structuralDiff(base, childNow)`. A diff is
  a set of `(path, kind, value)` where `kind ∈ {set, delete}`.
- Per path:
  - in `parentΔ` only → **auto**: apply parent's value/deletion.
  - in `childΔ` only → **keep** (already the child's state; no op).
  - in both, equal result → no-op.
  - in both, different result (set/set differing, set/delete, delete/set) → **conflict**
    `{ path, base, parent, child }`.
- Paths in the per-`doc_type` **exclusion set** (E8) are dropped from `parentΔ` before comparison
  (never merged, never conflicting).

### 5.2 Embedded collection merge

For each embedded collection key present in `base`, `parentNow`, or `childNow`, correlate children
by `child.source.id ↔ templateChild.id` (E7), using `base.embedded[collection][*].sourceId` as the
record of membership at last sync:

| Template child | In base? | Instance child | Action |
|---|---|---|---|
| present | — | matched | **recurse** `merge3` on that child's bands; conflicts bubble up under `/embedded/<coll>/<idx>/…` |
| present | absent | none | **template-added** → stamp it into the instance (append `Create`-shaped `/embedded/<coll>/<idx>` write) |
| absent (deleted) | present | present, unchanged vs base | **template-deleted** → remove from instance |
| absent (deleted) | present | present, modified vs base | **delete-vs-modify conflict** |
| — | — | instance child with no correlating template child | **instance-added** → keep (pull never removes) |

Emitted embedded writes use `/embedded/<collection>/…` paths (existing `MANAGE_EMBEDDED` cap).
Index stability: removals and additions are computed against the child's current array so emitted
pointers address real indices (arrays are otherwise opaque leaves per §5.1, but embedded arrays are
the one identity-bearing collection and are addressed positionally in the op, matching
`set_pointer`'s existing embedded-index semantics).

### 5.3 Order independence

`merge3` is a pure function of `(base, parentNow, childNow)` — independent of key/child iteration
order (sorted-key traversal; embedded correlation is by id, not position). Property-tested across
permuted inputs.

## 6. UI

### 6.1 Generic sheet chrome

The sheet panel **host** (not any module's sheet body) renders template controls in the panel
header for any document whose provenance state warrants them:

- **Source badge** — shown when the doc has a `source`: the template's name (resolved from the
  store) + a sync indicator ("up to date" vs "template changed" by comparing `base` to the
  template's current mergeable state, §6.4).
- **Pull** / **Revert** — shown when the doc has a resolvable `source` and the user passes the pull
  authz (§4.2).
- **Push** — shown when the doc has ≥1 instance in the store (`findInstances`) and the user passes
  the push authz (§4.3).

Because the host renders these, every `doc_type`'s sheet (Nightfox actor/item/effect sheets, the
generic system-tree editor, future sheets) gets template controls for free.

### 6.2 Conflict modal

A generic ui-kit component: one row per conflicting field (`path`, `base → template → mine`), a
per-field radio (keep mine / take template), **Apply** (writes the resolved plan) / **Cancel**
(writes nothing). For push, rows are grouped per instance.

### 6.3 AppContext seam

`AppContext.templates`:

```
stampInstance(source, opts): WireDocument     // returns the doc; caller dispatches the Create
pull(childId): void                            // computes, opens the modal if needed, dispatches
push(templateId): void
revert(childId): void
findInstances(templateId): WireDocument[]      // in-store, see+write scoped
```

Thin orchestration over `store`/`documents` + `dispatchIntent`; no logic beyond wiring the pure
core functions to the modal and the dispatch path.

### 6.4 Sync-state derivation

"template changed" iff `structuralDiff(child.base, mergeableSnapshot(template))` is non-empty
(restricted to merged bands, exclusions removed). Purely local; requires the template in-store.

## 7. Server surface (the whole server change)

1. `Document.base: Option<serde_json::Value>` — `#[serde(default)]`, `#[ts(type = "unknown")]`,
   opaque, **not** passed to `validate_engine_tree`.
2. `required_cap_for_path`: `/base` and `/base/…` → `cap::WRITE_FIELDS`. `/source` unchanged
   (immutable).
3. `validate_system_size`: bound `base` (when present) at `MAX_SYSTEM_BYTES`, independently.
4. ts-rs regenerate + Zod mirror (`WireDocument` gains `base`), drift guard satisfied.

No new `Operation` variant, no new wire frame, no server-side merge, no change to the broadcast /
event-log / OCC paths (a merge is an ordinary batched `Update`).

## 8. Testing

- **Core merge battery** (pure): disjoint auto-merge; each conflict class (set/set, set/delete,
  delete/set); array wholesale-replace; object/map key-level; placement-exclusion; deletions;
  order-independence (permuted inputs) — mirrors the Nightfox modifier permutation-battery
  discipline.
- **Embedded recursion**: matched-recurse, template-added, instance-added-preserved,
  template-deleted, delete-vs-modify conflict; nested (2-level) embedding.
- **Stamp**: deep-clone independence (`copy.system !== source.system`, recursively); `source`+`base`
  set at top level and `source` set on every embedded child; fresh ids throughout.
- **Pull/push/revert emission**: correct `FieldChange`s + `/base` refresh with **real** pre-images;
  cap union honored; template-not-in-store disables pull.
- **Server**: `/base` writable under `WRITE_FIELDS`; `/source` still `Forbidden`; `base` size-capped;
  `base` exempt from engine validation (a base holding a now-invalid engine shape still stores);
  ts-rs/Zod parity.

## 9. Exclusions (v1)

- **Cross-world push** — client store is same-world; deferred.
- **Compendium (`Scope::Compendium`) parents** — the compendium surface is not live yet.
- **Automatic / live / read-time inheritance** — never, by design (M2 §2).
- **Server-side merge** — never; the client computes, the server applies plain `Update`s.
- **Per-field merge-strategy declaration** (systems overriding array-wholesale vs element-merge) —
  the M2 "systems declare per-field strategy" hook is out of scope; v1 is array-wholesale + map-key
  uniformly. Revisited when a system needs it.

## 10. Open items resolved from D5

D5 flagged four open items for this sub-spec; all are resolved above: conflict UX (E5 / §6.2),
authorization (§4.2–4.4), interaction with the M5 field-level machinery (the merge emits ordinary
field-path `Update`s — no new server merge; §1, §7), and how map-shaped `system` data merges (E6 /
§5.1).
