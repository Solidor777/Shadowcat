# M13e — Generic Templates & 3-Way Merge Engine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add generic template documents plus an explicit, on-command 3-way pull / push / revert merge engine — computed entirely client-side, applied as ordinary batched `Update`s — with the minimal server storage/authz a self-contained base snapshot needs.

**Architecture:** The server gains ONE opaque envelope field (`base`), one `required_cap_for_path` mapping, and one size-cap extension — no merge logic, no new operation, no new wire frame. The merge is pure client-core (`@shadowcat/core`): `structuralDiff` + `merge3` over the `name`+`engine`+`system`+`embedded` bands, plus `stampInstance`/`computePull`/`computeRevert`/`findInstances`. `@shadowcat/ui-kit` adds a generic conflict modal, an `AppContext.templates` seam + `TemplatesController`, and host-rendered sheet chrome so every doc_type's sheet gets template controls for free. The shell wires the controller in.

**Tech Stack:** Rust (serde, ts-rs v12) + SQLite server; TypeScript + Zod client-core; Svelte 5 (Runes) + SCSS ui-kit; Vitest + @testing-library/svelte; pnpm workspaces.

## Global Constraints

Every task's requirements implicitly include this section.

- **Merge scope** covers ONLY the `name` + `engine` + `system` + `embedded` bands (recursively). Envelope identity/authz (`id`, `owner`, `permissions`, `parent_id`, `source`, `base`, `doc_type`, `schema_version`, `scope`) is child-local at every level and NEVER merges.
- **OCC:** every `Update` `FieldChange.old` is the REAL current stored value at that path (never `null` for an existing field; `null` only for a genuinely-absent key).
- **Deep-clone (`structuredClone`) at every fork; never `{...doc}` for nested bands** ([[embedded-copy-needs-deep-clone]]).
- **Server never runs system code and never merges;** a merge is an ordinary batched `Update` on the existing optimistic/OCC/broadcast/event-log path.
- **`base` is opaque** to the server, **size-capped at `MAX_SYSTEM_BYTES` (256 KiB)** independently of `system`/`engine`, and **exempt from engine ingress validation** (`validate_engine_tree` never walks it — a base snapshot may hold a now-invalid engine shape and must still store).
- **`/source` is immutable** (`required_cap_for_path("/source") == None` → `Forbidden` on Update); `/base` and `/base/…` → `cap::WRITE_FIELDS`.
- **Placement exclusion set** (never merges, never conflicts): `token` → `["/engine/x", "/engine/y", "/engine/rotation"]`; empty for every other `doc_type`.
- **Emission granularity is band-level:** a merge emits at most one `FieldChange` per changed band (`/name`, `/engine`, `/system`) and one per changed embedded collection (`/embedded/<coll>`, whole array), plus one `/base` refresh. `old` = the child's current value at that path; `new` = the merged value. This is the only `set_pointer`-compatible way to delete object keys / grow embedded arrays (server `set_pointer` cannot delete keys or extend arrays), and it matches `SystemTreeEditor.removeField`'s whole-container-rewrite convention. Cap union: `WRITE_FIELDS` (bands + `/base`) plus `MANAGE_EMBEDDED` iff any embedded collection changes.
- **`merge3` is pure and order-independent:** sorted-key traversal; embedded correlation by `source.id`, not position. Property-tested across permuted inputs.
- **ts-rs types are generated:** edit the Rust struct, regenerate (`cargo test` in `src/server` re-emits `#[ts(export)]` types into `src/types/generated/`), then mirror the Zod schema. NEVER hand-edit `src/types/generated/`.
- **Cross-platform:** `std::path` only, no hardcoded separators; responsive/touch-sized UI (≥44px targets under `@media (pointer: coarse)`).
- **`dist/` must exist before any server `cargo` build** (rust-embed compile-time validation for release; debug reads at runtime). `dist/` is already present in the worktree; the final gate rebuilds it.

## Model/Effort directives
- Plan-writer: sdd-plan-writer-opus (this plan).
- Dispatcher: mainline session (Opus/high).
- Implementers: shadowcat-coder (sonnet, effort medium); escalate to shadowcat-coder-opus on BLOCKED.
- Per-task reviewers: shadowcat-spec-reviewer + shadowcat-code-reviewer (the two-reviewer pair, effort high); escalate to -opus twins on shallow/uncertain findings.
- Final whole-branch review: shadowcat-code-reviewer-opus + shadowcat-spec-reviewer-opus.
- Per-task gate MUST include typecheck (esbuild-based vitest strips types — a green vitest is not a green typecheck): run `pnpm -r typecheck` for any task touching TS.

## Buddy-check directives
Tasks 2, 5, 6, 7 are pre-authorized for a buddy-check (two independent blind reviewers) IN PLACE OF the single two-reviewer gate — server authz boundary (2), embedded recursive-merge correctness (5), stamp deep-clone independence (6), and real-OCC-pre-image correctness (7) are the high-risk seams. Standing rule for any unflagged task that surfaces a security/concurrency/correctness-critical change during implementation: apply a buddy-check. A mandatory whole-branch buddy-check runs before merge regardless.

---

## File Structure

**Server (Rust, `src/server/src/`):**
- `data/document.rs` — MODIFY: add `pub base: Option<serde_json::Value>` to `Document` (`#[serde(default)]`, `#[ts(type = "unknown")]`). Update the three test doc-builders (`sample_doc`, `world_scoped_doc` already flows through `sample_doc`, `permission.rs`/`validation.rs`/`command.rs` local `doc()` helpers) so `base: None` is set.
- `data/permission.rs` — MODIFY: `required_cap_for_path` maps `/base` + `/base/…` → `cap::WRITE_FIELDS`.
- `data/validation.rs` — MODIFY: `validate_system_size` bounds `base` independently.
- `src/types/generated/Document.ts` — REGENERATED (do not hand-edit).

**Client core (`src/client/core/src/`):**
- `wire.ts` — MODIFY: add `base?: unknown` to `WireDocument`; `base: z.unknown().nullish()` to `DocumentSchema`.
- `capabilities.ts` — MODIFY: `baseCapForPath` maps `/base` → `core:write_fields` (client mirror).
- `merge.ts` — CREATE: `structuralDiff`, `deletePointer`, `deepEqual`, `merge3Tree`, `takeTemplate`, `merge3Embedded`, `merge3`, `restampSubtree`, `placementExclusions`, `isPlacementExcluded`; types `Diff`, `Conflict`, `MergeBase`, `EmbeddedBaseChild`, `MergeBands`, `MergePlan`.
- `templates.ts` — CREATE: `snapshotBase`, `stampInstance`, `computePull`, `computeRevert`, `planToUpdate`, `applyResolutions`, `findInstances`, `syncState`; types `StampOpts`, `SyncState`.
- `index.ts` — MODIFY: export the new merge + templates surface.
- `merge.test.ts`, `templates.test.ts` — CREATE.

**ui-kit (`src/client/ui-kit/src/`):**
- `MergeConflictModal.svelte` — CREATE: generic per-field conflict modal, grouped per instance.
- `templatesController.svelte.ts` — CREATE: `TemplatesController` (orchestration + reactive `pending` conflict session).
- `TemplateModalHost.svelte` — CREATE: mounts `MergeConflictModal` from the controller's `pending`.
- `TemplateControls.svelte` — CREATE: source badge + pull/push/revert buttons.
- `SheetHost.svelte` — CREATE: generic wrapper mounting `TemplateControls` above any module sheet body.
- `sheetsController.svelte.ts` — MODIFY: register `SheetHost` around the picked sheet.
- `appContext.ts` — MODIFY: add `templates: TemplatesApi`.
- `__fixtures__/appContextTest.ts` — MODIFY: default `templates` in the test fixture.
- `locales/en.ts` — MODIFY: add `templates.*` copy keys.
- `index.ts` — MODIFY: export the new components + controller + types.
- `MergeConflictModal.test.ts`, `templatesController.svelte.test.ts`, `TemplateControls.test.ts` — CREATE.

**Shell (`src/client/shell/src/lib/`):**
- `worldSession.svelte.ts` — MODIFY: expose what `TemplatesController` needs (already has `store`, `documents`, `dispatchIntent`, `role`, `selfId`, `canEdit`).
- `Table.svelte` — MODIFY: construct `TemplatesController`, provide `templates` in `setAppContext`, mount `<TemplateModalHost>`.

---

## Task 1: Server `Document.base` field + ts-rs regen + client wire mirror

**Files:**
- Modify: `src/server/src/data/document.rs`
- Modify (test helpers only): `src/server/src/data/validation.rs`, `src/server/src/data/command.rs`, `src/server/src/data/permission.rs` (their local `doc*(...)` builders gain `base: None`)
- Modify: `src/client/core/src/wire.ts`
- Modify: `src/client/core/src/capabilities.ts`
- Regenerated: `src/types/generated/Document.ts`
- Test: `src/server/src/data/document.rs` (inline `#[cfg(test)]`), `src/client/core/src/wire.test.ts`

**Interfaces:**
- Produces: `Document.base: Option<serde_json::Value>` (Rust); `WireDocument.base?: unknown` + `DocumentSchema` accepting `base` (TS); `baseCapForPath("/base") === "core:write_fields"` (client mirror).

- [ ] **Step 1: Write the failing Rust test** (append to `document.rs` `tests` mod)

```rust
    #[test]
    fn document_round_trips_base_snapshot_and_defaults_none() {
        // base defaults to None when absent (serde default).
        let bare = serde_json::json!({
            "id": Uuid::from_u128(1), "scope": {"kind": "world", "world_id": Uuid::from_u128(9)},
            "doc_type": "actor", "schema_version": 1, "system": {}, "created_at": 0, "updated_at": 0
        });
        let doc: Document = serde_json::from_value(bare).unwrap();
        assert!(doc.base.is_none());

        // A present base round-trips verbatim, even holding an engine shape that is
        // invalid for the current doc_type (base is an opaque historical snapshot).
        let mut with_base = sample_doc();
        with_base.base = Some(serde_json::json!({
            "name": "Old", "engine": { "not": "a-valid-token-engine" },
            "system": { "hp": 1 }, "embedded": {}
        }));
        let s = serde_json::to_string(&with_base).unwrap();
        let back: Document = serde_json::from_str(&s).unwrap();
        assert_eq!(with_base, back);
    }
```

- [ ] **Step 2: Run it, verify it fails to compile** (no `base` field yet)

Run: `(cd src/server && cargo test --lib document_round_trips_base_snapshot_and_defaults_none 2>&1 | tail -20)`
Expected: FAIL — `no field 'base' on type '&data::document::Document'` (and the `sample_doc()` construction has no `base`).

- [ ] **Step 3: Add the `base` field to `Document`**

In `src/server/src/data/document.rs`, in `struct Document`, immediately after the `source` field (~line 213):

```rust
    #[serde(default)]
    pub source: Option<Source>,
    /// Opaque snapshot of this child's mergeable content (`name`/`engine`/`system`/
    /// `embedded`) at last sync (stamp or a successful pull/push/revert). Present only
    /// on stamped children. The server NEVER interprets it: exempt from
    /// `validate_engine_tree`, size-capped by `validate_system_size`, and writable at
    /// `/base` under `cap::WRITE_FIELDS`. Client-owned shape (`MergeBase`, `@shadowcat/core`).
    #[serde(default)]
    #[ts(type = "unknown")]
    pub base: Option<serde_json::Value>,
```

- [ ] **Step 4: Set `base: None` in every local test doc-builder**

In `document.rs` `sample_doc()`, after `source: Some(Source { ... }),` add `base: None,`.
In `validation.rs` `doc_with_system(...)`, after `source: None,` add `base: None,`.
In `command.rs` `doc(id)` and the inline `rich` doc in `command_round_trips_through_json`, after their `source: ...,` add `base: None,`.

(Search each file for `source:` inside a `Document { ... }` literal; add `base: None,` beside it.)

- [ ] **Step 5: Run the Rust test + full data suite, regenerate types**

Run: `(cd src/server && cargo test --lib 2>&1 | tail -25)`
Expected: PASS (all data tests green; `#[ts(export)]` re-emits `src/types/generated/Document.ts`).

- [ ] **Step 6: Verify the generated type gained `base`**

Run: `grep -n "base" src/types/generated/Document.ts`
Expected: a `base: unknown` member in the `Document` type.

- [ ] **Step 7: Write the failing client wire test** (append to `wire.test.ts`, inside `describe("DocumentSchema — envelope name + engine band", ...)`)

```ts
  it("parses a document carrying a base snapshot", () => {
    const parsed = DocumentSchema.parse({
      ...base, name: "Inst", engine: {},
      base: { name: "Tmpl", engine: null, system: { hp: 1 }, embedded: {} },
    });
    expect((parsed.base as { name: string }).name).toBe("Tmpl");
  });

  it("parses a document with base absent or null (unstamped)", () => {
    expect(DocumentSchema.parse({ ...base, name: null, engine: null }).base).toBeUndefined();
    expect(DocumentSchema.parse({ ...base, name: null, engine: null, base: null }).base).toBeNull();
  });
```

- [ ] **Step 8: Run it, verify it fails**

Run: `pnpm --filter @shadowcat/core exec vitest run src/wire.test.ts 2>&1 | tail -20`
Expected: FAIL — `base` is stripped (schema has no `base` key), so `parsed.base` is `undefined` in the first test.

- [ ] **Step 9: Add `base` to `WireDocument` + `DocumentSchema`**

In `src/client/core/src/wire.ts`, in the `WireDocument` type after `source: ...;` (~line 97):

```ts
  source: z.infer<typeof SourceSchema> | null;
  // Opaque mergeable-content snapshot at last sync (`MergeBase`, `./merge`). Present only on
  // stamped children; absent/undefined otherwise. Server-opaque; the client owns the shape.
  base?: unknown;
```

In `DocumentSchema` (the `z.lazy` object, ~line 116) after `source: SourceSchema.nullable(),`:

```ts
    source: SourceSchema.nullable(),
    // `.nullish()` accepts absent (unstamped) OR explicit null, matching the server's
    // `Option` field; a present snapshot is passed through opaque (`z.unknown()`).
    base: z.unknown().nullish(),
```

- [ ] **Step 10: Add `/base` to the client cap mirror**

In `src/client/core/src/capabilities.ts`, in `baseCapForPath`, extend the first `if` (the `core:write_fields` branch) to include `/base`:

```ts
  if (
    path === "/system" ||
    path.startsWith("/system/") ||
    path === "/engine" ||
    path.startsWith("/engine/") ||
    path === "/name" ||
    path === "/base" ||
    path.startsWith("/base/")
  ) {
    return "core:write_fields";
  }
```

- [ ] **Step 11: Run client test + typecheck**

Run: `pnpm --filter @shadowcat/core exec vitest run src/wire.test.ts 2>&1 | tail -15`
Expected: PASS.
Run: `pnpm --filter @shadowcat/core typecheck`
Expected: no errors.

- [ ] **Step 12: Commit**

```bash
git add src/server/src/data/document.rs src/server/src/data/validation.rs src/server/src/data/command.rs src/server/src/data/permission.rs src/types/generated/Document.ts src/client/core/src/wire.ts src/client/core/src/wire.test.ts src/client/core/src/capabilities.ts
git commit -m "feat(m13e): Document.base opaque snapshot field + wire/cap mirror"
```

---

## Task 2 [BUDDY-CHECK]: Server `/base` authz + size cap + engine-validation exemption

**Files:**
- Modify: `src/server/src/data/permission.rs`
- Modify: `src/server/src/data/validation.rs`
- Test: both files (inline `#[cfg(test)]`)

**Interfaces:**
- Consumes: `Document.base` (Task 1).
- Produces: `required_cap_for_path("/base") == Some(cap::WRITE_FIELDS)`, `required_cap_for_path("/base/embedded/foo") == Some(cap::WRITE_FIELDS)`, `required_cap_for_path("/source") == None`; `validate_system_size` rejects an oversized `base`; `validate_engine_tree` ignores `base`.

- [ ] **Step 1: Write the failing authz unit tests** (append to `permission.rs` `required_cap_tests` mod)

```rust
    #[test]
    fn base_whole_and_subpaths_require_write_fields() {
        assert_eq!(required_cap_for_path("/base"), Some(cap::WRITE_FIELDS));
        assert_eq!(required_cap_for_path("/base/system/hp"), Some(cap::WRITE_FIELDS));
        assert_eq!(required_cap_for_path("/base/embedded/actor/0/name"), Some(cap::WRITE_FIELDS));
    }

    #[test]
    fn base_boundary_neighbor_does_not_match() {
        assert_eq!(required_cap_for_path("/based"), None);
    }

    #[test]
    fn source_is_immutable_no_cap() {
        // `/source` maps to no capability, so an Update targeting it is Forbidden for everyone.
        assert_eq!(required_cap_for_path("/source"), None);
        assert_eq!(required_cap_for_path("/source/id"), None);
    }
```

- [ ] **Step 2: Run, verify failure**

Run: `(cd src/server && cargo test --lib base_whole_and_subpaths_require_write_fields 2>&1 | tail -15)`
Expected: FAIL — `assertion failed: left == right` (`/base` currently returns `None`).

- [ ] **Step 3: Add `/base` to `required_cap_for_path`**

In `src/server/src/data/permission.rs`, extend the first branch of `required_cap_for_path` (the `WRITE_FIELDS` branch, ~line 30):

```rust
    if path == "/system"
        || path.starts_with("/system/")
        || path == "/engine"
        || path.starts_with("/engine/")
        || path == "/name"
        || path == "/base"
        || path.starts_with("/base/")
    {
        Some(cap::WRITE_FIELDS)
```

- [ ] **Step 4: Run authz tests**

Run: `(cd src/server && cargo test --lib required_cap 2>&1 | tail -15)`
Expected: PASS.

- [ ] **Step 5: Write the failing validation tests** (append to `validation.rs` `tests` mod)

```rust
    #[test]
    fn oversized_base_is_rejected() {
        let mut doc = doc_with_system(serde_json::json!({ "hp": 1 }));
        doc.base = Some(serde_json::json!({ "blob": "x".repeat(MAX_SYSTEM_BYTES + 1) }));
        assert!(matches!(validate_system_size(&doc), Err(DataError::TooLarge(_))));
    }

    #[test]
    fn small_base_passes() {
        let mut doc = doc_with_system(serde_json::json!({ "hp": 1 }));
        doc.base = Some(serde_json::json!({ "name": "T", "system": { "hp": 1 } }));
        assert!(validate_system_size(&doc).is_ok());
    }

    #[test]
    fn base_holding_stale_engine_is_exempt_from_engine_validation() {
        // base is a historical snapshot that may predate the current engine schema; it must
        // store even when it carries an engine shape that is invalid for this doc_type.
        let mut doc = doc_with_engine(valid_wall_engine());
        doc.base = Some(serde_json::json!({
            "name": "Old", "engine": { "seg": { "x1": "not-a-number" } },
            "system": {}, "embedded": {}
        }));
        assert!(validate_engine_tree(&mut doc).is_ok(), "base must not be walked by validate_engine_tree");
        // And the stale base survives untouched.
        assert_eq!(doc.base.unwrap()["engine"]["seg"]["x1"], serde_json::json!("not-a-number"));
    }
```

- [ ] **Step 6: Run, verify failure**

Run: `(cd src/server && cargo test --lib oversized_base_is_rejected 2>&1 | tail -15)`
Expected: FAIL — `base` is not yet bounded, so no `TooLarge`.

- [ ] **Step 7: Bound `base` in `validate_system_size`**

In `src/server/src/data/validation.rs`, in `validate_system_size`, after the `engine` block (after the `if let Some(eng) = &doc.engine { ... }` block, before the `for children in doc.embedded.values()` loop):

```rust
    if let Some(base) = &doc.base {
        let base_bytes = serde_json::to_vec(base)?.len();
        if base_bytes > MAX_SYSTEM_BYTES {
            return Err(DataError::TooLarge(base_bytes));
        }
    }
```

(`validate_engine_tree` needs NO change — it only touches `doc.engine` + embedded; `base` is naturally exempt. The `base_holding_stale_engine...` test proves it.)

- [ ] **Step 8: Run the full data suite**

Run: `(cd src/server && cargo test --lib 2>&1 | tail -25)`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add src/server/src/data/permission.rs src/server/src/data/validation.rs
git commit -m "feat(m13e): /base authz (WRITE_FIELDS) + size cap; /source immutable; base engine-validation exempt"
```

---

## Task 3: Core `structuralDiff` + `deletePointer` + `deepEqual`

**Files:**
- Create: `src/client/core/src/merge.ts`
- Create: `src/client/core/src/merge.test.ts`
- Modify: `src/client/core/src/index.ts`

**Interfaces:**
- Consumes: `setPointer`, `getPointer` from `./store`.
- Produces:
  - `type Diff = { path: string; kind: "set"; value: unknown } | { path: string; kind: "delete" }`
  - `structuralDiff(base: unknown, now: unknown, prefix?: string): Diff[]`
  - `deletePointer(root: unknown, pointer: string): void`
  - `deepEqual(a: unknown, b: unknown): boolean`

- [ ] **Step 1: Write the failing tests** (`src/client/core/src/merge.test.ts`)

```ts
import { describe, it, expect } from "vitest";
import { structuralDiff, deletePointer, deepEqual } from "./merge";

describe("deepEqual", () => {
  it("compares objects key-order-independently and arrays positionally", () => {
    expect(deepEqual({ a: 1, b: 2 }, { b: 2, a: 1 })).toBe(true);
    expect(deepEqual([1, 2], [2, 1])).toBe(false);
    expect(deepEqual({ a: [1, { x: 2 }] }, { a: [1, { x: 2 }] })).toBe(true);
    expect(deepEqual(0, false)).toBe(false);
    expect(deepEqual(null, undefined)).toBe(false);
  });
});

describe("structuralDiff", () => {
  it("no change yields no diffs", () => {
    expect(structuralDiff({ a: 1, b: { c: 2 } }, { a: 1, b: { c: 2 } })).toEqual([]);
  });

  it("recurses objects, emitting the deepest changed leaf as a set", () => {
    expect(structuralDiff({ a: { b: 1 } }, { a: { b: 2 } })).toEqual([
      { path: "/a/b", kind: "set", value: 2 },
    ]);
  });

  it("a key present only in `now` is a set of that key", () => {
    expect(structuralDiff({ a: 1 }, { a: 1, b: 3 })).toEqual([
      { path: "/b", kind: "set", value: 3 },
    ]);
  });

  it("a key present only in `base` is a delete", () => {
    expect(structuralDiff({ a: 1, b: 2 }, { a: 1 })).toEqual([
      { path: "/b", kind: "delete" },
    ]);
  });

  it("arrays are opaque leaves — any inequality is one whole-array set", () => {
    expect(structuralDiff({ a: [1, 2] }, { a: [1, 2, 3] })).toEqual([
      { path: "/a", kind: "set", value: [1, 2, 3] },
    ]);
    expect(structuralDiff({ a: [{ x: 1 }] }, { a: [{ x: 2 }] })).toEqual([
      { path: "/a", kind: "set", value: [{ x: 2 }] },
    ]);
  });

  it("a scalar-to-object type change is a whole set at that path", () => {
    expect(structuralDiff({ a: 1 }, { a: { b: 2 } })).toEqual([
      { path: "/a", kind: "set", value: { b: 2 } },
    ]);
  });

  it("emits sorted, RFC-6901-escaped pointers", () => {
    const diffs = structuralDiff({}, { "b/x": 1, "a~y": 2 });
    expect(diffs.map((d) => d.path)).toEqual(["/a~0y", "/b~1x"]);
  });
});

describe("deletePointer", () => {
  it("removes an object key", () => {
    const root = { a: { b: 1, c: 2 } };
    deletePointer(root, "/a/b");
    expect(root).toEqual({ a: { c: 2 } });
  });

  it("splices an array element", () => {
    const root = { xs: [10, 20, 30] };
    deletePointer(root, "/xs/1");
    expect(root).toEqual({ xs: [10, 30] });
  });

  it("no-ops on a missing intermediate segment", () => {
    const root = { a: 1 };
    deletePointer(root, "/b/c");
    expect(root).toEqual({ a: 1 });
  });
});
```

- [ ] **Step 2: Run, verify failure**

Run: `pnpm --filter @shadowcat/core exec vitest run src/merge.test.ts 2>&1 | tail -15`
Expected: FAIL — cannot resolve `./merge`.

- [ ] **Step 3: Create `merge.ts` with the diff primitives**

```ts
// Pure, order-independent 3-way merge primitives (client-core). The server never merges;
// the merge is computed here and applied as an ordinary batched `Update` (M13e). Every value
// is plain JSON (objects recurse key-by-key, arrays are opaque leaves, scalars are leaves).
import { setPointer, getPointer } from "./store";

/** One structural change between two JSON trees at an RFC-6901 pointer. */
export type Diff =
  | { path: string; kind: "set"; value: unknown }
  | { path: string; kind: "delete" };

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return v !== null && typeof v === "object" && !Array.isArray(v);
}

/** Deep structural equality: objects key-order-independent, arrays positional, scalars strict. */
export function deepEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (Array.isArray(a) && Array.isArray(b)) {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (!deepEqual(a[i], b[i])) return false;
    return true;
  }
  if (isPlainObject(a) && isPlainObject(b)) {
    const ak = Object.keys(a);
    if (ak.length !== Object.keys(b).length) return false;
    for (const k of ak) {
      if (!Object.prototype.hasOwnProperty.call(b, k)) return false;
      if (!deepEqual(a[k], b[k])) return false;
    }
    return true;
  }
  return false;
}

/** RFC-6901 token escaping (`~` → `~0`, `/` → `~1`). */
function escapeToken(k: string): string {
  return k.replace(/~/g, "~0").replace(/\//g, "~1");
}

/**
 * Structural diff of `now` against `base` as one JSON tree. Objects recurse key-by-key;
 * arrays are opaque leaves (any inequality → one whole-array `set`); scalars/type-changes are
 * leaves. Sorted-key traversal makes the output order-independent.
 */
export function structuralDiff(base: unknown, now: unknown, prefix = ""): Diff[] {
  if (isPlainObject(base) && isPlainObject(now)) {
    const out: Diff[] = [];
    const keys = new Set([...Object.keys(base), ...Object.keys(now)]);
    for (const key of [...keys].sort()) {
      const p = `${prefix}/${escapeToken(key)}`;
      const inBase = Object.prototype.hasOwnProperty.call(base, key);
      const inNow = Object.prototype.hasOwnProperty.call(now, key);
      if (inBase && !inNow) out.push({ path: p, kind: "delete" });
      else if (!inBase && inNow) out.push({ path: p, kind: "set", value: now[key] });
      else out.push(...structuralDiff(base[key], now[key], p));
    }
    return out;
  }
  if (deepEqual(base, now)) return [];
  return [{ path: prefix, kind: "set", value: now }];
}

function tokenize(pointer: string): string[] {
  return pointer.split("/").slice(1).map((t) => t.replace(/~1/g, "/").replace(/~0/g, "~"));
}

/**
 * Remove the object key or array element at `pointer` in `root` (mutates). No-op on any missing
 * intermediate segment. The set-only server `set_pointer` cannot delete; a merge that removes a
 * key/element rewrites the whole enclosing container (see `planToUpdate`), and this builds that
 * rewritten container in memory first.
 */
export function deletePointer(root: unknown, pointer: string): void {
  if (pointer === "") throw new Error("cannot delete the document root");
  const tokens = tokenize(pointer);
  let cur: unknown = root;
  for (const tok of tokens.slice(0, -1)) {
    if (Array.isArray(cur)) cur = cur[Number(tok)];
    else if (isPlainObject(cur)) cur = cur[tok];
    else return;
  }
  const last = tokens[tokens.length - 1];
  if (Array.isArray(cur)) {
    const i = Number(last);
    if (Number.isInteger(i) && i >= 0 && i < cur.length) cur.splice(i, 1);
  } else if (isPlainObject(cur)) {
    delete cur[last];
  }
}
```

(Note: `setPointer`/`getPointer` are imported now because later steps in Tasks 4–5 use them; keep the import.)

- [ ] **Step 4: Run tests**

Run: `pnpm --filter @shadowcat/core exec vitest run src/merge.test.ts 2>&1 | tail -15`
Expected: PASS.

- [ ] **Step 5: Export from `index.ts`**

In `src/client/core/src/index.ts`, after the `scene-docs` export block (~line 82), add:

```ts
export { structuralDiff, deletePointer, deepEqual } from "./merge";
export type { Diff } from "./merge";
```

- [ ] **Step 6: Typecheck**

Run: `pnpm --filter @shadowcat/core typecheck`
Expected: no errors (an unused `setPointer`/`getPointer` import would fail `noUnusedLocals`; if it does, temporarily add `void setPointer; void getPointer;` at module end — REMOVED in Task 4 once they're consumed. Prefer to defer the import to Task 4: if typecheck fails on unused import here, drop the `import { setPointer, getPointer }` line and re-add it in Task 4 Step 3.)

- [ ] **Step 7: Commit**

```bash
git add src/client/core/src/merge.ts src/client/core/src/merge.test.ts src/client/core/src/index.ts
git commit -m "feat(m13e): core structuralDiff + deletePointer + deepEqual"
```

---

## Task 4: Core `merge3Tree` (scalar/object/array 3-way + exclusions)

**Files:**
- Modify: `src/client/core/src/merge.ts`
- Modify: `src/client/core/src/merge.test.ts`
- Modify: `src/client/core/src/index.ts`

**Interfaces:**
- Consumes: `structuralDiff`, `deletePointer`, `deepEqual`, `setPointer`, `getPointer`.
- Produces:
  - `type Conflict = { path: string; base: unknown; parent: unknown; child: unknown; parentKind: "set" | "delete" }`
  - `merge3Tree(base, parentNow, childNow, exclusions: string[]): { merged: unknown; conflicts: Conflict[] }` — `merged` defaults conflicts to the CHILD value ("keep mine").
  - `takeTemplate(root: unknown, c: Conflict): void` — mutate `root` to apply the parent's decision at `c.path`.
  - `isPlacementExcluded(path: string, exclusions: string[]): boolean`

- [ ] **Step 1: Write the failing tests** (append to `merge.test.ts`)

```ts
import { merge3Tree, takeTemplate, isPlacementExcluded, type Conflict } from "./merge";

describe("merge3Tree", () => {
  it("disjoint changes auto-merge (parent value applied, child value kept)", () => {
    const base = { a: 1, b: 2, c: 3 };
    const parent = { a: 1, b: 20, c: 3 }; // parent changed b
    const child = { a: 10, b: 2, c: 3 }; // child changed a
    const { merged, conflicts } = merge3Tree(base, parent, child, []);
    expect(conflicts).toEqual([]);
    expect(merged).toEqual({ a: 10, b: 20, c: 3 });
  });

  it("set/set on the same path with equal result is a no-op", () => {
    const { conflicts } = merge3Tree({ a: 1 }, { a: 2 }, { a: 2 }, []);
    expect(conflicts).toEqual([]);
  });

  it("set/set differing is a conflict; merged keeps child by default", () => {
    const { merged, conflicts } = merge3Tree({ a: 1 }, { a: 2 }, { a: 3 }, []);
    expect(conflicts).toEqual([
      { path: "/a", base: 1, parent: 2, child: 3, parentKind: "set" },
    ]);
    expect(merged).toEqual({ a: 3 });
  });

  it("set/delete is a conflict", () => {
    const { conflicts } = merge3Tree({ a: 1 }, { a: 2 }, {}, []);
    expect(conflicts).toEqual([
      { path: "/a", base: 1, parent: 2, child: undefined, parentKind: "set" },
    ]);
  });

  it("delete/set is a conflict", () => {
    const { conflicts } = merge3Tree({ a: 1 }, {}, { a: 3 }, []);
    expect(conflicts).toEqual([
      { path: "/a", base: 1, parent: undefined, child: 3, parentKind: "delete" },
    ]);
  });

  it("parent-only delete auto-applies (key removed from merged)", () => {
    const { merged, conflicts } = merge3Tree({ a: 1, b: 2 }, { a: 1 }, { a: 1, b: 2 }, []);
    expect(conflicts).toEqual([]);
    expect(merged).toEqual({ a: 1 });
  });

  it("arrays merge wholesale (parent array replaces base→child when child untouched)", () => {
    const { merged, conflicts } = merge3Tree(
      { xs: [1, 2] }, { xs: [1, 2, 3] }, { xs: [1, 2] }, [],
    );
    expect(conflicts).toEqual([]);
    expect(merged).toEqual({ xs: [1, 2, 3] });
  });

  it("map key-level: independent keys merge, one key conflicts", () => {
    const base = { m: { x: 1, y: 1 } };
    const parent = { m: { x: 2, y: 1 } };
    const child = { m: { x: 1, y: 9 } };
    const { merged, conflicts } = merge3Tree(base, parent, child, []);
    expect(conflicts).toEqual([]);
    expect(merged).toEqual({ m: { x: 2, y: 9 } });
  });

  it("excluded paths are dropped from parent's changes (never merge, never conflict)", () => {
    const base = { engine: { x: 0, hp: 1 } };
    const parent = { engine: { x: 99, hp: 5 } }; // parent moved x AND changed hp
    const child = { engine: { x: 3, hp: 1 } }; // child placed at x:3
    const { merged, conflicts } = merge3Tree(base, parent, child, ["/engine/x"]);
    expect(conflicts).toEqual([]);
    expect(merged).toEqual({ engine: { x: 3, hp: 5 } }); // child x kept, parent hp merged
  });

  it("is order-independent across permuted object keys", () => {
    const base = { a: 1, b: 2, c: 3, d: 4 };
    const parent = { a: 10, b: 2, c: 30, d: 4 };
    const child = { a: 1, b: 20, c: 3, d: 40 };
    const forward = merge3Tree(base, parent, child, []);
    const rev = (o: Record<string, number>) =>
      Object.fromEntries(Object.entries(o).reverse());
    const permuted = merge3Tree(rev(base), rev(parent), rev(child), []);
    expect(permuted.merged).toEqual(forward.merged);
    expect(permuted.conflicts).toEqual(forward.conflicts);
  });

  it("takeTemplate applies the parent's set/delete into a merged tree", () => {
    const merged = { a: 3, b: 5 };
    const setC: Conflict = { path: "/a", base: 1, parent: 2, child: 3, parentKind: "set" };
    takeTemplate(merged, setC);
    expect(merged).toEqual({ a: 2, b: 5 });
    const delC: Conflict = { path: "/b", base: 5, parent: undefined, child: 5, parentKind: "delete" };
    takeTemplate(merged, delC);
    expect(merged).toEqual({ a: 2 });
  });
});

describe("isPlacementExcluded", () => {
  it("matches a path or its descendants against the exclusion set", () => {
    expect(isPlacementExcluded("/engine/x", ["/engine/x"])).toBe(true);
    expect(isPlacementExcluded("/engine/x/deep", ["/engine/x"])).toBe(true);
    expect(isPlacementExcluded("/engine/xylophone", ["/engine/x"])).toBe(false);
    expect(isPlacementExcluded("/engine/y", ["/engine/x"])).toBe(false);
  });
});
```

- [ ] **Step 2: Run, verify failure**

Run: `pnpm --filter @shadowcat/core exec vitest run src/merge.test.ts 2>&1 | tail -15`
Expected: FAIL — `merge3Tree`/`takeTemplate`/`isPlacementExcluded` not exported.

- [ ] **Step 3: Implement in `merge.ts`** (append after `deletePointer`; ensure `setPointer`/`getPointer` are imported at the top)

```ts
/** A field changed on both the template (parent) and instance (child) sides since the last
 * sync. `parent`/`child` are `undefined` when that side deleted the key. `parentKind` records
 * how "take template" resolves it (set the parent value, or delete the key). */
export type Conflict = {
  path: string;
  base: unknown;
  parent: unknown;
  child: unknown;
  parentKind: "set" | "delete";
};

/** Whether `path` is inside the placement exclusion set (equal or a descendant). */
export function isPlacementExcluded(path: string, exclusions: string[]): boolean {
  return exclusions.some((e) => path === e || path.startsWith(`${e}/`));
}

/** JSON-pointer subtree overlap (either contains the other, or equal). */
function pathsOverlap(a: string, b: string): boolean {
  return a === b || a.startsWith(`${b}/`) || b.startsWith(`${a}/`);
}

function sameResult(a: Diff, b: Diff): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "delete") return true;
  return deepEqual(a.value, (b as { value: unknown }).value);
}

function applyDiff(root: unknown, d: Diff): void {
  if (d.kind === "set") setPointer(root, d.path, d.value);
  else deletePointer(root, d.path);
}

/**
 * 3-way merge of one JSON tree (used for the `name`+`engine`+`system` synthetic band tree).
 * `merged` starts from `childNow` and applies parent-only changes; a path changed on both sides
 * with a differing result is a conflict, left at the child value ("keep mine" default). Paths in
 * `exclusions` are dropped from the parent side (never merge, never conflict). Pure +
 * order-independent (sorted-key `structuralDiff`).
 *
 * Ancestor/descendant overlap (e.g. child deletes an object the parent edits inside) is treated
 * as a conflict at the parent change's path (`pathsOverlap`) — the safe direction; arrays are
 * opaque leaves so deep-array changes are single-path.
 */
export function merge3Tree(
  base: unknown,
  parentNow: unknown,
  childNow: unknown,
  exclusions: string[],
): { merged: unknown; conflicts: Conflict[] } {
  const parentDiff = structuralDiff(base, parentNow).filter((d) => !isPlacementExcluded(d.path, exclusions));
  const childDiff = structuralDiff(base, childNow);
  const merged = structuredClone(childNow) as unknown;
  const conflicts: Conflict[] = [];
  for (const p of parentDiff) {
    const overlapping = childDiff.filter((c) => pathsOverlap(c.path, p.path));
    if (overlapping.length === 0) {
      applyDiff(merged, p);
      continue;
    }
    const exact = overlapping.find((c) => c.path === p.path);
    if (exact && overlapping.length === 1 && sameResult(p, exact)) continue;
    conflicts.push({
      path: p.path,
      base: getPointer(base, p.path),
      parent: p.kind === "set" ? p.value : undefined,
      child: exact && exact.kind === "set" ? exact.value : getPointer(childNow, p.path),
      parentKind: p.kind,
    });
  }
  return { merged, conflicts };
}

/** Apply the parent's decision for a conflict into `root` (in place). */
export function takeTemplate(root: unknown, c: Conflict): void {
  if (c.parentKind === "delete") deletePointer(root, c.path);
  else setPointer(root, c.path, c.parent);
}
```

- [ ] **Step 4: Run tests**

Run: `pnpm --filter @shadowcat/core exec vitest run src/merge.test.ts 2>&1 | tail -15`
Expected: PASS.

- [ ] **Step 5: Export from `index.ts`**

Update the merge export line in `src/client/core/src/index.ts`:

```ts
export { structuralDiff, deletePointer, deepEqual, merge3Tree, takeTemplate, isPlacementExcluded } from "./merge";
export type { Diff, Conflict } from "./merge";
```

- [ ] **Step 6: Typecheck**

Run: `pnpm --filter @shadowcat/core typecheck`
Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git add src/client/core/src/merge.ts src/client/core/src/merge.test.ts src/client/core/src/index.ts
git commit -m "feat(m13e): core merge3Tree (object/array/scalar 3-way + exclusions + takeTemplate)"
```

---

## Task 5 [BUDDY-CHECK]: Core embedded recursion + `merge3` top-level + `restampSubtree`

**Files:**
- Modify: `src/client/core/src/merge.ts`
- Modify: `src/client/core/src/merge.test.ts`
- Modify: `src/client/core/src/index.ts`

**Interfaces:**
- Consumes: `merge3Tree`, `structuralDiff`, `deepEqual`, `WireDocument`.
- Produces:
  - `type MergeBands = { name: string | null; engine: unknown; system: unknown; embedded: Record<string, WireDocument[]> }`
  - `type EmbeddedBaseChild = { sourceId: string; name: string | null; engine: unknown; system: unknown; embedded: Record<string, EmbeddedBaseChild[]> }`
  - `type MergeBase = { name: string | null; engine: unknown; system: unknown; embedded: Record<string, EmbeddedBaseChild[]> }`
  - `type MergePlan = { mergedBands: MergeBands; conflicts: Conflict[] }`
  - `placementExclusions(docType: string): string[]`
  - `restampSubtree(doc: WireDocument): WireDocument`
  - `merge3(base: MergeBase, parentNow: WireDocument, childNow: WireDocument, exclusions: string[]): MergePlan`

- [ ] **Step 1: Write the failing tests** (append to `merge.test.ts`)

```ts
import { merge3, restampSubtree, placementExclusions, type MergeBase } from "./merge";
import type { WireDocument } from "./wire";

function doc(over: Partial<WireDocument> & { id: string }): WireDocument {
  return {
    id: over.id,
    scope: over.scope ?? { kind: "world", world_id: "w1" },
    doc_type: over.doc_type ?? "actor",
    schema_version: 1,
    name: over.name ?? null,
    source: over.source ?? null,
    owner: over.owner ?? null,
    permissions: { default: "none", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
    embedded: over.embedded ?? {},
    parent_id: over.parent_id ?? null,
    engine: over.engine,
    system: over.system ?? {},
    created_at: 0,
    updated_at: 0,
  };
}

/** MergeBase snapshot of a document's bands (test helper mirroring snapshotBase). */
function baseOf(d: WireDocument): MergeBase {
  const emb: MergeBase["embedded"] = {};
  for (const [coll, kids] of Object.entries(d.embedded)) {
    emb[coll] = kids.map((k) => ({
      sourceId: k.source?.id ?? k.id,
      name: k.name,
      engine: k.engine ?? null,
      system: k.system ?? null,
      embedded: baseOf(k).embedded,
    }));
  }
  return { name: d.name, engine: d.engine ?? null, system: d.system ?? null, embedded: emb };
}

describe("placementExclusions", () => {
  it("excludes token placement, nothing for other doc types", () => {
    expect(placementExclusions("token")).toEqual(["/engine/x", "/engine/y", "/engine/rotation"]);
    expect(placementExclusions("actor")).toEqual([]);
  });
});

describe("restampSubtree", () => {
  it("assigns a fresh id + source pointing to the template, recursively", () => {
    const child = doc({ id: "gc", name: "GC" });
    const parent = doc({ id: "tmpl", name: "T", embedded: { items: [child] } });
    const stamped = restampSubtree(parent);
    expect(stamped.id).not.toBe("tmpl");
    expect(stamped.source).toEqual({ id: "tmpl", pack: null, version: 1 });
    const sc = stamped.embedded.items[0];
    expect(sc.id).not.toBe("gc");
    expect(sc.source).toEqual({ id: "gc", pack: null, version: 1 });
  });
});

describe("merge3 embedded", () => {
  it("matched child recurses; a disjoint system change auto-merges", () => {
    const tmplChild = doc({ id: "tc", system: { hp: 1 } });
    const instChild = doc({ id: "ic", source: { id: "tc", pack: null, version: 1 }, system: { hp: 1 } });
    const template = doc({ id: "T", embedded: { items: [tmplChild] } });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 }, embedded: { items: [instChild] } });
    const base = baseOf(child); // captured at stamp: instChild@hp:1
    const tmplChild2 = doc({ id: "tc", system: { hp: 5 } }); // template changed hp
    const template2 = doc({ id: "T", embedded: { items: [tmplChild2] } });
    const { mergedBands, conflicts } = merge3(base, template2, child, []);
    expect(conflicts).toEqual([]);
    expect((mergedBands.embedded.items[0].system as { hp: number }).hp).toBe(5);
    expect(mergedBands.embedded.items[0].id).toBe("ic"); // instance envelope preserved
  });

  it("template-added child is stamped into the instance", () => {
    const template = doc({ id: "T", embedded: { items: [doc({ id: "new-tc", system: { k: 1 } })] } });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 }, embedded: { items: [] } });
    const base = baseOf(child); // no items at stamp
    const { mergedBands } = merge3(base, template, child, []);
    expect(mergedBands.embedded.items).toHaveLength(1);
    expect(mergedBands.embedded.items[0].source).toEqual({ id: "new-tc", pack: null, version: 1 });
    expect(mergedBands.embedded.items[0].id).not.toBe("new-tc");
  });

  it("instance-added child (no correlation) is preserved", () => {
    const template = doc({ id: "T", embedded: { items: [] } });
    const localChild = doc({ id: "local", system: { own: true } });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 }, embedded: { items: [localChild] } });
    const base: MergeBase = { name: null, engine: null, system: {}, embedded: { items: [] } };
    const { mergedBands, conflicts } = merge3(base, template, child, []);
    expect(conflicts).toEqual([]);
    expect(mergedBands.embedded.items.map((c) => c.id)).toEqual(["local"]);
  });

  it("template-deleted + instance unchanged removes the child", () => {
    const instChild = doc({ id: "ic", source: { id: "tc", pack: null, version: 1 }, system: { hp: 1 } });
    const template = doc({ id: "T", embedded: { items: [] } }); // template dropped tc
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 }, embedded: { items: [instChild] } });
    const base = baseOf(child); // base had tc@hp:1
    const { mergedBands } = merge3(base, template, child, []);
    expect(mergedBands.embedded.items).toHaveLength(0);
  });

  it("template-deleted + instance modified is a conflict", () => {
    const instChild = doc({ id: "ic", source: { id: "tc", pack: null, version: 1 }, system: { hp: 9 } });
    const template = doc({ id: "T", embedded: { items: [] } });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 }, embedded: { items: [instChild] } });
    const base: MergeBase = { name: null, engine: null, system: {}, embedded: { items: [{ sourceId: "tc", name: null, engine: null, system: { hp: 1 }, embedded: {} }] } };
    const { conflicts } = merge3(base, template, child, []);
    expect(conflicts).toEqual([
      { path: "/embedded/items/0", base: { hp: 1 }, parent: undefined, child: { hp: 9 }, parentKind: "delete" },
    ]);
  });

  it("recurses 2 levels of embedding", () => {
    const gcTmpl = doc({ id: "gc", system: { deep: 1 } });
    const tcTmpl = doc({ id: "tc", embedded: { sub: [gcTmpl] } });
    const gcInst = doc({ id: "gci", source: { id: "gc", pack: null, version: 1 }, system: { deep: 1 } });
    const tcInst = doc({ id: "tci", source: { id: "tc", pack: null, version: 1 }, embedded: { sub: [gcInst] } });
    const template = doc({ id: "T", embedded: { items: [doc({ id: "tc", embedded: { sub: [doc({ id: "gc", system: { deep: 7 } })] } })] } });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 }, embedded: { items: [tcInst] } });
    const base = baseOf(child);
    const { mergedBands, conflicts } = merge3(base, template, child, []);
    expect(conflicts).toEqual([]);
    expect((mergedBands.embedded.items[0].embedded.sub[0].system as { deep: number }).deep).toBe(7);
  });
});
```

- [ ] **Step 2: Run, verify failure**

Run: `pnpm --filter @shadowcat/core exec vitest run src/merge.test.ts 2>&1 | tail -15`
Expected: FAIL — `merge3`/`restampSubtree`/`placementExclusions` not exported.

- [ ] **Step 3: Implement in `merge.ts`** (append; add `import type { WireDocument } from "./wire";` at the top)

```ts
/** The mergeable bands of a live document; `embedded` children are full documents (envelope
 * preserved). Produced by `merge3`, written whole-band by `planToUpdate`. */
export type MergeBands = {
  name: string | null;
  engine: unknown;
  system: unknown;
  embedded: Record<string, WireDocument[]>;
};

/** One embedded child inside a `base` snapshot: bands + the `sourceId` correlation key (the
 * child's `source.id` at sync time — the template child's id). Recurses (finite-depth embedding). */
export type EmbeddedBaseChild = {
  sourceId: string;
  name: string | null;
  engine: unknown;
  system: unknown;
  embedded: Record<string, EmbeddedBaseChild[]>;
};

/** The opaque `Document.base` snapshot shape (client-owned). Top-level bands + recursive
 * embedded content keyed for provenance correlation. */
export type MergeBase = {
  name: string | null;
  engine: unknown;
  system: unknown;
  embedded: Record<string, EmbeddedBaseChild[]>;
};

/** Result of a 3-way merge: the child-wins-default merged bands + the conflicts to resolve. */
export type MergePlan = { mergedBands: MergeBands; conflicts: Conflict[] };

/** Per-`doc_type` instance-local paths that never merge (E8). */
export function placementExclusions(docType: string): string[] {
  return docType === "token" ? ["/engine/x", "/engine/y", "/engine/rotation"] : [];
}

/** Deep-clone `doc` into a new subtree: fresh `id`, `source` pointing at the template (`doc.id`),
 * recursively for every embedded child. Used to stamp a template-added embedded child into an
 * instance. Deep-clone independence is load-bearing ([[embedded-copy-needs-deep-clone]]). */
export function restampSubtree(doc: WireDocument): WireDocument {
  const out = structuredClone(doc) as WireDocument;
  out.id = crypto.randomUUID();
  out.source = { id: doc.id, pack: null, version: doc.source?.version ?? 1 };
  const embedded: Record<string, WireDocument[]> = {};
  for (const [coll, kids] of Object.entries(doc.embedded)) embedded[coll] = kids.map(restampSubtree);
  out.embedded = embedded;
  return out;
}

/** The three synthetic-tree bands as one object, so `merge3Tree` addresses `/name`, `/engine/*`,
 * `/system/*` at exactly the document's real pointers. */
function bandsTree(name: string | null, engine: unknown, system: unknown): Record<string, unknown> {
  return { name, engine: engine ?? null, system: system ?? null };
}

/** MergeBase-shaped bands of a live embedded child (for the recursive 3-way base). */
function baseFromChild(b: EmbeddedBaseChild): MergeBase {
  return { name: b.name, engine: b.engine, system: b.system, embedded: b.embedded };
}

/** Bands of a live document as a MergeBase (no sourceId at the top). */
function bandsMergeBase(d: WireDocument): MergeBase {
  const emb: Record<string, EmbeddedBaseChild[]> = {};
  for (const [coll, kids] of Object.entries(d.embedded)) {
    emb[coll] = kids.map((k) => ({
      sourceId: k.source?.id ?? k.id,
      name: k.name,
      engine: k.engine ?? null,
      system: k.system ?? null,
      embedded: bandsMergeBase(k).embedded,
    }));
  }
  return { name: d.name, engine: d.engine ?? null, system: d.system ?? null, embedded: emb };
}

/** Whether an instance child's bands are unchanged versus its base record. */
function childUnchangedVsBase(child: WireDocument, b: EmbeddedBaseChild): boolean {
  return structuralDiff(
    { name: b.name, engine: b.engine, system: b.system, embedded: b.embedded },
    bandsMergeBase(child),
  ).length === 0;
}

function applyMergedBands(child: WireDocument, bands: MergeBands): WireDocument {
  return { ...child, name: bands.name, engine: bands.engine, system: bands.system, embedded: bands.embedded };
}

function prefixConflicts(conflicts: Conflict[], coll: string, idx: number): Conflict[] {
  const p = `/embedded/${coll}/${idx}`;
  return conflicts.map((c) => ({ ...c, path: `${p}${c.path}` }));
}

/** 3-way merge of the embedded collections, correlating instance↔template children by
 * `source.id`↔`id` (E7), using `base.embedded[coll][*].sourceId` as the membership record. */
function merge3Embedded(
  base: Record<string, EmbeddedBaseChild[]>,
  parentEmbedded: Record<string, WireDocument[]>,
  childEmbedded: Record<string, WireDocument[]>,
): { merged: Record<string, WireDocument[]>; conflicts: Conflict[] } {
  const merged: Record<string, WireDocument[]> = {};
  const conflicts: Conflict[] = [];
  const colls = new Set([...Object.keys(base), ...Object.keys(parentEmbedded), ...Object.keys(childEmbedded)]);
  for (const coll of [...colls].sort()) {
    const baseKids = base[coll] ?? [];
    const parentKids = parentEmbedded[coll] ?? [];
    const childKids = childEmbedded[coll] ?? [];
    const templateById = new Map(parentKids.map((t) => [t.id, t]));
    const baseBySource = new Map(baseKids.map((b) => [b.sourceId, b]));
    const out: WireDocument[] = [];

    // Pass 1: walk instance children in order.
    for (const cd of childKids) {
      const sid = cd.source?.id ?? null;
      const correlated = sid !== null && (templateById.has(sid) || baseBySource.has(sid));
      if (!correlated) {
        out.push(cd); // instance-added → keep
        continue;
      }
      const t = sid !== null ? templateById.get(sid) : undefined;
      const b = sid !== null ? baseBySource.get(sid) : undefined;
      if (t) {
        if (b) {
          const idx = out.length;
          const plan = merge3(baseFromChild(b), t, cd, placementExclusions(cd.doc_type));
          out.push(applyMergedBands(cd, plan.mergedBands));
          conflicts.push(...prefixConflicts(plan.conflicts, coll, idx));
        } else {
          out.push(cd); // base-missing matched → keep instance (fail-safe; no 3-way base)
        }
        continue;
      }
      // template absent, base present → template-deleted this correlation
      if (b) {
        if (childUnchangedVsBase(cd, b)) continue; // drop
        const idx = out.length;
        out.push(cd);
        conflicts.push({
          path: `/embedded/${coll}/${idx}`,
          base: b.system,
          parent: undefined,
          child: cd.system ?? null,
          parentKind: "delete",
        });
      }
    }

    // Pass 2: template-added children (in template, absent from base, no instance copy).
    for (const t of parentKids) {
      if (baseBySource.has(t.id)) continue;
      if (childKids.some((cd) => cd.source?.id === t.id)) continue;
      out.push(restampSubtree(t));
    }

    merged[coll] = out;
  }
  return { merged, conflicts };
}

/** Full 3-way merge over the mergeable bands (`name`+`engine`+`system` tree + `embedded`).
 * `exclusions` apply to the top-level doc; embedded children use their own doc_type exclusions.
 * Pure + order-independent. Conflicts default to the child ("keep mine") in `mergedBands`. */
export function merge3(
  base: MergeBase,
  parentNow: WireDocument,
  childNow: WireDocument,
  exclusions: string[],
): MergePlan {
  const tree = merge3Tree(
    bandsTree(base.name, base.engine, base.system),
    bandsTree(parentNow.name, parentNow.engine ?? null, parentNow.system),
    bandsTree(childNow.name, childNow.engine ?? null, childNow.system),
    exclusions,
  );
  const m = tree.merged as { name: string | null; engine: unknown; system: unknown };
  const emb = merge3Embedded(base.embedded, parentNow.embedded, childNow.embedded);
  return {
    mergedBands: { name: m.name, engine: m.engine, system: m.system, embedded: emb.merged },
    conflicts: [...tree.conflicts, ...emb.conflicts],
  };
}
```

- [ ] **Step 4: Run tests**

Run: `pnpm --filter @shadowcat/core exec vitest run src/merge.test.ts 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Export from `index.ts`**

Update the merge exports:

```ts
export { structuralDiff, deletePointer, deepEqual, merge3Tree, takeTemplate, isPlacementExcluded, merge3, restampSubtree, placementExclusions } from "./merge";
export type { Diff, Conflict, MergeBands, MergeBase, EmbeddedBaseChild, MergePlan } from "./merge";
```

- [ ] **Step 6: Typecheck**

Run: `pnpm --filter @shadowcat/core typecheck`
Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git add src/client/core/src/merge.ts src/client/core/src/merge.test.ts src/client/core/src/index.ts
git commit -m "feat(m13e): core merge3 embedded recursion + restampSubtree (buddy-check)"
```

---

## Task 6 [BUDDY-CHECK]: Core `snapshotBase` + `stampInstance`

**Files:**
- Create: `src/client/core/src/templates.ts`
- Create: `src/client/core/src/templates.test.ts`
- Modify: `src/client/core/src/index.ts`

**Interfaces:**
- Consumes: `restampSubtree`, `MergeBase`, `EmbeddedBaseChild` from `./merge`; `WireDocument` from `./wire`.
- Produces:
  - `interface StampOpts { worldId: string; ownerId: string | null; parentId: string | null; permissions?: WireDocument["permissions"] }`
  - `snapshotBase(doc: WireDocument): MergeBase`
  - `stampInstance(source: WireDocument, opts: StampOpts): WireDocument`

- [ ] **Step 1: Write the failing tests** (`src/client/core/src/templates.test.ts`)

```ts
import { describe, it, expect } from "vitest";
import { snapshotBase, stampInstance, type StampOpts } from "./templates";
import type { WireDocument } from "./wire";
import type { MergeBase } from "./merge";

function doc(over: Partial<WireDocument> & { id: string }): WireDocument {
  return {
    id: over.id,
    scope: over.scope ?? { kind: "world", world_id: "w1" },
    doc_type: over.doc_type ?? "actor",
    schema_version: 1,
    name: over.name ?? null,
    source: over.source ?? null,
    owner: over.owner ?? null,
    permissions: over.permissions ?? { default: "none", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
    embedded: over.embedded ?? {},
    parent_id: over.parent_id ?? null,
    engine: over.engine,
    system: over.system ?? {},
    created_at: 0,
    updated_at: 0,
  };
}

const opts: StampOpts = { worldId: "w1", ownerId: "u-self", parentId: "scene-1" };

describe("snapshotBase", () => {
  it("captures bands + embedded children keyed by their source.id", () => {
    const child = doc({ id: "ic", source: { id: "tc", pack: null, version: 2 }, name: "Kid", system: { hp: 3 } });
    const d = doc({ id: "C", name: "Inst", engine: { hp: 9 }, system: { a: 1 }, embedded: { items: [child] } });
    const snap = snapshotBase(d);
    expect(snap).toEqual<MergeBase>({
      name: "Inst",
      engine: { hp: 9 },
      system: { a: 1 },
      embedded: { items: [{ sourceId: "tc", name: "Kid", engine: null, system: { hp: 3 }, embedded: {} }] },
    });
  });

  it("deep-clones so the snapshot does not alias the document", () => {
    const d = doc({ id: "C", system: { nested: { x: 1 } } });
    const snap = snapshotBase(d);
    (d.system as { nested: { x: number } }).nested.x = 99;
    expect((snap.system as { nested: { x: number } }).nested.x).toBe(1);
  });
});

describe("stampInstance", () => {
  it("creates a new doc: fresh id, initiator owner/parent, source pointing at the template", () => {
    const tmpl = doc({ id: "T", name: "Preset", owner: "gm", system: { hp: 10 } });
    const inst = stampInstance(tmpl, opts);
    expect(inst.id).not.toBe("T");
    expect(inst.owner).toBe("u-self");
    expect(inst.parent_id).toBe("scene-1");
    expect(inst.source).toEqual({ id: "T", pack: null, version: 1 });
    expect(inst.system).toEqual({ hp: 10 });
  });

  it("deep-clone independence: nested bands are not aliased (recursively)", () => {
    const tmplChild = doc({ id: "tc", system: { deep: { v: 1 } } });
    const tmpl = doc({ id: "T", system: { s: { v: 1 } }, embedded: { items: [tmplChild] } });
    const inst = stampInstance(tmpl, opts);
    expect(inst.system).not.toBe(tmpl.system);
    expect(inst.embedded.items[0].system).not.toBe(tmplChild.system);
    (tmpl.system as { s: { v: number } }).s.v = 42;
    (tmplChild.system as { deep: { v: number } }).deep.v = 42;
    expect((inst.system as { s: { v: number } }).s.v).toBe(1);
    expect((inst.embedded.items[0].system as { deep: { v: number } }).deep.v).toBe(1);
  });

  it("recursively assigns embedded children fresh ids + source = template child id", () => {
    const tmplChild = doc({ id: "tc", name: "Item" });
    const tmpl = doc({ id: "T", embedded: { items: [tmplChild] } });
    const inst = stampInstance(tmpl, opts);
    const sc = inst.embedded.items[0];
    expect(sc.id).not.toBe("tc");
    expect(sc.source).toEqual({ id: "tc", pack: null, version: 1 });
  });

  it("sets base to a snapshot keyed by the new children's source.id (correlation)", () => {
    const tmpl = doc({ id: "T", name: "P", system: { hp: 1 }, embedded: { items: [doc({ id: "tc", system: { k: 1 } })] } });
    const inst = stampInstance(tmpl, opts);
    const base = inst.base as MergeBase;
    expect(base.name).toBe("P");
    expect(base.system).toEqual({ hp: 1 });
    expect(base.embedded.items[0].sourceId).toBe("tc"); // == the stamped child's source.id
    expect(base.embedded.items[0].system).toEqual({ k: 1 });
  });

  it("copies the compendium pack into source when the template is compendium-scoped", () => {
    const tmpl = doc({ id: "T", scope: { kind: "compendium", pack: "nightfox" } });
    const inst = stampInstance(tmpl, opts);
    expect(inst.source).toEqual({ id: "T", pack: "nightfox", version: 1 });
  });
});
```

- [ ] **Step 2: Run, verify failure**

Run: `pnpm --filter @shadowcat/core exec vitest run src/templates.test.ts 2>&1 | tail -15`
Expected: FAIL — cannot resolve `./templates`.

- [ ] **Step 3: Create `templates.ts`**

```ts
// Client-core template operations: stamp (create-from-template) + the 3-way pull/push/revert
// emission (M13e). All produce document ops; the caller dispatches via `dispatchIntent`. The
// server never merges — a merge is an ordinary batched `Update`.
import type { WireDocument } from "./wire";
import { restampSubtree, type MergeBase, type EmbeddedBaseChild } from "./merge";

/** Where a stamped instance lands: the initiator's world/owner/parent (never the template's). */
export interface StampOpts {
  worldId: string;
  ownerId: string | null;
  parentId: string | null;
  /** The initiator's own permissions for the new doc; a deny-all default when omitted. */
  permissions?: WireDocument["permissions"];
}

function defaultPerms(): WireDocument["permissions"] {
  return { default: "none", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null };
}

function snapshotEmbedded(embedded: Record<string, WireDocument[]>): Record<string, EmbeddedBaseChild[]> {
  const out: Record<string, EmbeddedBaseChild[]> = {};
  for (const [coll, kids] of Object.entries(embedded)) {
    out[coll] = kids.map((k) => ({
      // Correlation key: the child's source.id (== its template child's id). A non-provenance
      // child falls back to its own id (still a stable per-child key).
      sourceId: k.source?.id ?? k.id,
      name: k.name,
      engine: structuredClone(k.engine ?? null),
      system: structuredClone(k.system ?? null),
      embedded: snapshotEmbedded(k.embedded),
    }));
  }
  return out;
}

/** The opaque `base` snapshot of a document's current mergeable content. Works for both a stamped
 * instance (children keyed by their `source.id`) and a template (children key on `source.id ?? id`,
 * which for a template child is its own id — the same correlation key its instances point to). */
export function snapshotBase(doc: WireDocument): MergeBase {
  return {
    name: doc.name,
    engine: structuredClone(doc.engine ?? null),
    system: structuredClone(doc.system ?? null),
    embedded: snapshotEmbedded(doc.embedded),
  };
}

/** Deep-clone `source`'s mergeable bands into a NEW document (fresh id, initiator owner/perms,
 * caller parent/scope, `source` provenance, recursively fresh embedded ids + provenance), then
 * capture `base`. Deep-clone independence is load-bearing — never `{...doc}` for nested bands
 * ([[embedded-copy-needs-deep-clone]]). */
export function stampInstance(source: WireDocument, opts: StampOpts): WireDocument {
  const clone = structuredClone(source) as WireDocument;
  const embedded: Record<string, WireDocument[]> = {};
  for (const [coll, kids] of Object.entries(clone.embedded)) embedded[coll] = kids.map(restampSubtree);
  const pack = source.scope.kind === "compendium" ? source.scope.pack : (source.source?.pack ?? null);
  const stamped: WireDocument = {
    ...clone,
    id: crypto.randomUUID(),
    scope: { kind: "world", world_id: opts.worldId },
    owner: opts.ownerId,
    permissions: opts.permissions ?? defaultPerms(),
    parent_id: opts.parentId,
    source: { id: source.id, pack, version: source.source?.version ?? 1 },
    embedded,
  };
  stamped.base = snapshotBase(stamped);
  return stamped;
}
```

- [ ] **Step 4: Run tests**

Run: `pnpm --filter @shadowcat/core exec vitest run src/templates.test.ts 2>&1 | tail -15`
Expected: PASS.

- [ ] **Step 5: Export from `index.ts`**

Add after the merge exports:

```ts
export { snapshotBase, stampInstance } from "./templates";
export type { StampOpts } from "./templates";
```

- [ ] **Step 6: Typecheck**

Run: `pnpm --filter @shadowcat/core typecheck`
Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git add src/client/core/src/templates.ts src/client/core/src/templates.test.ts src/client/core/src/index.ts
git commit -m "feat(m13e): core snapshotBase + stampInstance deep-clone (buddy-check)"
```

---

## Task 7 [BUDDY-CHECK]: Core `computePull` / `computeRevert` / `planToUpdate` / `applyResolutions` / `findInstances` / `syncState`

**Files:**
- Modify: `src/client/core/src/templates.ts`
- Modify: `src/client/core/src/templates.test.ts`
- Modify: `src/client/core/src/index.ts`

**Interfaces:**
- Consumes: `merge3`, `merge3Tree`, `takeTemplate`, `structuralDiff`, `isPlacementExcluded`, `placementExclusions`, `deepEqual`, `restampSubtree`, `MergePlan`, `MergeBands`, `Conflict`, `MergeBase` from `./merge`; `WireDocument`, `WireOperation`, `WireFieldChange` from `./wire`.
- Note: `computeRevert` does NOT reuse `merge3Embedded` (Task 5) for its embedded reset — that
  correlation table's "no correlation → keep" rule is a PULL default (preserve local additions),
  which is the opposite of what revert needs (discard local additions). `computeRevert` uses its
  own `revertEmbedded`/`revertChild` (private to `templates.ts`): a child correlated to a CURRENT
  template child recurses/resets; an uncorrelated child is dropped; an unmatched template child is
  freshly stamped in. Scalar/object bands still reuse `merge3Tree` via the "child as its own base"
  trick (`revertBands`), which is safe because ALWAYS-take-template has no keep-mine ambiguity.
- Produces:
  - `type SyncState = "none" | "up_to_date" | "template_changed"`
  - `computePull(child: WireDocument, template: WireDocument): MergePlan`
  - `planToUpdate(child: WireDocument, template: WireDocument, mergedBands: MergeBands): WireOperation`
  - `applyResolutions(mergedBands: MergeBands, conflicts: Conflict[], theirs: Set<string>): MergeBands`
  - `computeRevert(child: WireDocument, template: WireDocument): WireOperation`
  - `findInstances(templateId: string, all: Iterable<WireDocument>): WireDocument[]`
  - `syncState(child: WireDocument, template: WireDocument | undefined): SyncState`

- [ ] **Step 1: Write the failing tests** (append to `templates.test.ts`; reuse the `doc` helper)

```ts
import {
  computePull, computeRevert, planToUpdate, applyResolutions, findInstances, syncState,
} from "./templates";
import type { MergeBands, Conflict } from "./merge";

describe("computePull + planToUpdate", () => {
  it("emits whole-band FieldChanges with REAL child pre-images + a /base refresh", () => {
    const tmpl = doc({ id: "T", name: "T2", system: { hp: 5 } });
    const child = doc({ id: "C", name: "C1", source: { id: "T", pack: null, version: 1 }, system: { hp: 1, note: "mine" } });
    child.base = { name: "C1", engine: null, system: { hp: 1, note: "mine" }, embedded: {} };
    // Template changed hp 1→5 (child's base hp was 1); child kept note.
    const plan = computePull(child, tmpl);
    expect(plan.conflicts).toEqual([]);
    const op = planToUpdate(child, tmpl, plan.mergedBands);
    expect(op.op).toBe("update");
    if (op.op !== "update") return;
    const system = op.changes.find((c) => c.path === "/system")!;
    expect(system.old).toEqual({ hp: 1, note: "mine" }); // real pre-image
    expect(system.new).toEqual({ hp: 5, note: "mine" });  // merged
    const baseChange = op.changes.find((c) => c.path === "/base")!;
    expect(baseChange.old).toEqual(child.base);
    expect(baseChange.new).toEqual({ name: "T2", engine: null, system: { hp: 5 }, embedded: {} });
    // /name unchanged on the merged bands → no /name change emitted.
    expect(op.changes.some((c) => c.path === "/name")).toBe(false);
  });

  it("emits a whole /embedded/<coll> array change when a child was added by the template", () => {
    const tmpl = doc({ id: "T", embedded: { items: [doc({ id: "tc", system: { k: 1 } })] } });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 }, embedded: { items: [] } });
    child.base = { name: null, engine: null, system: {}, embedded: { items: [] } };
    const plan = computePull(child, tmpl);
    const op = planToUpdate(child, tmpl, plan.mergedBands);
    if (op.op !== "update") throw new Error("expected update");
    const emb = op.changes.find((c) => c.path === "/embedded/items")!;
    expect(emb.old).toEqual([]);
    expect((emb.new as WireDocument[])).toHaveLength(1);
  });

  it("token placement never merges (child x/y/rotation kept even when template moved)", () => {
    const tmpl = doc({ id: "T", doc_type: "token", engine: { x: 99, y: 99, rotation: 90, hp: 5 } });
    const child = doc({ id: "C", doc_type: "token", source: { id: "T", pack: null, version: 1 }, engine: { x: 3, y: 4, rotation: 0, hp: 1 } });
    child.base = { name: null, engine: { x: 3, y: 4, rotation: 0, hp: 1 }, system: {}, embedded: {} };
    const plan = computePull(child, tmpl);
    expect((plan.mergedBands.engine as { x: number; y: number; rotation: number; hp: number })).toEqual({ x: 3, y: 4, rotation: 0, hp: 5 });
  });
});

describe("applyResolutions", () => {
  it("takes the template value only for conflicts chosen 'theirs'", () => {
    const bands: MergeBands = { name: null, engine: null, system: { a: "mine", b: "mine" }, embedded: {} };
    const conflicts: Conflict[] = [
      { path: "/system/a", base: "x", parent: "theirs", child: "mine", parentKind: "set" },
      { path: "/system/b", base: "x", parent: "theirs", child: "mine", parentKind: "set" },
    ];
    const resolved = applyResolutions(bands, conflicts, new Set(["/system/a"]));
    expect(resolved.system).toEqual({ a: "theirs", b: "mine" });
    // input not mutated
    expect(bands.system).toEqual({ a: "mine", b: "mine" });
  });
});

describe("computeRevert", () => {
  it("discards child diffs on merged bands (template wins) but keeps placement + refreshes base", () => {
    const tmpl = doc({ id: "T", doc_type: "token", name: "T", engine: { x: 99, hp: 5 }, system: { s: 1 } });
    const child = doc({ id: "C", doc_type: "token", source: { id: "T", pack: null, version: 1 }, name: "C", engine: { x: 3, hp: 8 }, system: { s: 2, extra: true } });
    child.base = { name: "T", engine: { x: 99, hp: 5 }, system: { s: 1 }, embedded: {} };
    const op = computeRevert(child, tmpl);
    if (op.op !== "update") throw new Error("expected update");
    const engine = op.changes.find((c) => c.path === "/engine")!;
    expect(engine.new).toEqual({ x: 3, hp: 5 }); // template hp, child placement x
    const system = op.changes.find((c) => c.path === "/system")!;
    expect(system.new).toEqual({ s: 1 }); // child 'extra' discarded
    expect(op.changes.some((c) => c.path === "/base")).toBe(true);
  });

  it("drops child-added embedded children and restores template-deleted ones", () => {
    const tmpl = doc({ id: "T", embedded: { items: [doc({ id: "tc", system: { k: 1 } })] } });
    const localChild = doc({ id: "local", system: { own: 1 } });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 }, embedded: { items: [localChild] } });
    child.base = snapshotBaseForTest(child);
    const op = computeRevert(child, tmpl);
    if (op.op !== "update") throw new Error("expected update");
    const emb = op.changes.find((c) => c.path === "/embedded/items")!;
    const kids = emb.new as WireDocument[];
    expect(kids).toHaveLength(1);
    expect(kids[0].source).toEqual({ id: "tc", pack: null, version: 1 }); // template child stamped
    expect(kids.some((k) => k.id === "local")).toBe(false); // child-added dropped
  });
});

// local helper: base snapshot for the revert test above
function snapshotBaseForTest(d: WireDocument): MergeBase {
  return snapshotBase(d);
}

describe("findInstances", () => {
  it("returns only docs whose source.id is the template id", () => {
    const a = doc({ id: "a", source: { id: "T", pack: null, version: 1 } });
    const b = doc({ id: "b", source: { id: "OTHER", pack: null, version: 1 } });
    const c = doc({ id: "c" });
    expect(findInstances("T", [a, b, c]).map((d) => d.id)).toEqual(["a"]);
  });
});

describe("syncState", () => {
  it("none when the doc has no source, or the template is not in store", () => {
    expect(syncState(doc({ id: "C" }), undefined)).toBe("none");
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 } });
    expect(syncState(child, undefined)).toBe("none");
  });

  it("up_to_date when base equals the template's current snapshot", () => {
    const tmpl = doc({ id: "T", name: "T", system: { hp: 1 } });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 } });
    child.base = { name: "T", engine: null, system: { hp: 1 }, embedded: {} };
    expect(syncState(child, tmpl)).toBe("up_to_date");
  });

  it("template_changed when the template diverged from base (ignoring placement)", () => {
    const tmpl = doc({ id: "T", doc_type: "token", name: "T", engine: { x: 5, hp: 9 }, system: {} });
    const child = doc({ id: "C", doc_type: "token", source: { id: "T", pack: null, version: 1 } });
    // base engine hp:1; template hp:9 → changed. But an x-only move must NOT count.
    child.base = { name: "T", engine: { x: 0, hp: 1 }, system: {}, embedded: {} };
    expect(syncState(child, tmpl)).toBe("template_changed");
    child.base = { name: "T", engine: { x: 0, hp: 9 }, system: {}, embedded: {} };
    expect(syncState(child, tmpl)).toBe("up_to_date"); // only x differs → excluded
  });
});
```

- [ ] **Step 2: Run, verify failure**

Run: `pnpm --filter @shadowcat/core exec vitest run src/templates.test.ts 2>&1 | tail -20`
Expected: FAIL — the new functions are not exported.

- [ ] **Step 3: Implement in `templates.ts`** (append; extend the imports from `./merge` and add wire types)

Update the top import:

```ts
import type { WireDocument, WireOperation, WireFieldChange } from "./wire";
import {
  merge3, merge3Tree, takeTemplate, structuralDiff, isPlacementExcluded, placementExclusions, deepEqual,
  restampSubtree, type MergeBase, type MergeBands, type MergePlan, type Conflict, type EmbeddedBaseChild,
} from "./merge";
```

Append:

```ts
/** Provenance/sync state of a document for the sheet chrome (§6.4). */
export type SyncState = "none" | "up_to_date" | "template_changed";

/** 3-way pull: merge the template's current state into the child, preserving child-local diffs.
 * `base` is the child's stored snapshot (falls back to the child's own snapshot when absent, so a
 * base-less child yields a clean template-wins result). */
export function computePull(child: WireDocument, template: WireDocument): MergePlan {
  const base: MergeBase = (child.base as MergeBase | undefined) ?? snapshotBase(child);
  return merge3(base, template, child, placementExclusions(child.doc_type));
}

function pushIfChanged(changes: WireFieldChange[], path: string, before: unknown, after: unknown): void {
  if (!deepEqual(before, after)) changes.push({ path, old: before, new: after });
}

/** Turn merged bands into ONE `Update`: at most one whole-band change per changed band
 * (`/name`, `/engine`, `/system`), one per changed embedded collection (whole array), plus a
 * `/base` refresh (new = the template's current snapshot). Every `old` is the child's REAL
 * current stored value (OCC pre-image). Whole-band/whole-collection writes are the only
 * `set_pointer`-compatible way to delete keys / grow embedded arrays. */
export function planToUpdate(child: WireDocument, template: WireDocument, mergedBands: MergeBands): WireOperation {
  const changes: WireFieldChange[] = [];
  pushIfChanged(changes, "/name", child.name, mergedBands.name);
  pushIfChanged(changes, "/engine", child.engine ?? null, mergedBands.engine);
  pushIfChanged(changes, "/system", child.system ?? null, mergedBands.system);
  const colls = new Set([...Object.keys(child.embedded), ...Object.keys(mergedBands.embedded)]);
  for (const coll of [...colls].sort()) {
    const before = child.embedded[coll] ?? [];
    const after = mergedBands.embedded[coll] ?? [];
    if (!deepEqual(before, after)) changes.push({ path: `/embedded/${coll}`, old: before, new: after });
  }
  changes.push({ path: "/base", old: (child.base ?? null) as unknown, new: snapshotBase(template) });
  return { op: "update", doc_id: child.id, changes };
}

/** Apply the user's per-field conflict choices: for each conflict whose path is in `theirs`, take
 * the template value/deletion; the rest keep the child ("mine") value already in `mergedBands`.
 * Pure (clones its input). */
export function applyResolutions(mergedBands: MergeBands, conflicts: Conflict[], theirs: Set<string>): MergeBands {
  const root = structuredClone({
    name: mergedBands.name, engine: mergedBands.engine, system: mergedBands.system, embedded: mergedBands.embedded,
  });
  for (const c of conflicts) if (theirs.has(c.path)) takeTemplate(root, c);
  return { name: root.name, engine: root.engine, system: root.system, embedded: root.embedded };
}

type Bands = { name: string | null; engine: unknown; system: unknown };

/** Reset one node's own bands to the template's current value, keeping placement (E8). Reuses
 * `merge3Tree` with the child as its OWN base (so `childDiff` is always empty and every parent
 * diff auto-applies with zero conflicts) — the "always take template" trick. NOTE: this handles
 * only `name`/`engine`/`system`; embedded reset is a SEPARATE algorithm (`revertEmbedded`) — see
 * below for why `merge3Embedded`'s pull-shaped correlation table cannot be reused for revert. */
function revertBands(child: Bands, template: Bands, exclusions: string[]): Bands {
  const selfBase = { name: child.name, engine: child.engine ?? null, system: child.system ?? null };
  const templateNow = { name: template.name, engine: template.engine ?? null, system: template.system ?? null };
  const { merged } = merge3Tree(selfBase, templateNow, selfBase, exclusions);
  return merged as Bands;
}

/**
 * Embedded-collection reset for revert. `merge3Embedded` (used by pull/push) KEEPS an
 * uncorrelated instance child ("instance-added" — correct when preserving local additions is
 * the point). Revert wants the opposite: discard every local addition. Correlation is by
 * `childKid.source.id` against a CURRENT template child's id — no stored `base` is consulted
 * (there is nothing to preserve), so:
 * - a child correlated to a current template child → recurse-reset it (`revertChild`);
 * - a child with no correlation (no `source`, or `source.id` not among the template's current
 *   children) → DROPPED (a locally-added child never belongs in a reverted instance);
 * - a template child with no correlating instance child → freshly stamped in (restores content
 *   the instance had locally deleted, or never had).
 */
function revertEmbedded(
  parentEmbedded: Record<string, WireDocument[]>,
  childEmbedded: Record<string, WireDocument[]>,
): Record<string, WireDocument[]> {
  const merged: Record<string, WireDocument[]> = {};
  const colls = new Set([...Object.keys(parentEmbedded), ...Object.keys(childEmbedded)]);
  for (const coll of [...colls].sort()) {
    const parentKids = parentEmbedded[coll] ?? [];
    const childKids = childEmbedded[coll] ?? [];
    const templateById = new Map(parentKids.map((t) => [t.id, t]));
    const out: WireDocument[] = [];
    for (const cd of childKids) {
      const t = cd.source ? templateById.get(cd.source.id) : undefined;
      if (t) out.push(revertChild(cd, t));
      // else: no correlation → child-added → dropped.
    }
    for (const t of parentKids) {
      if (!childKids.some((cd) => cd.source?.id === t.id)) out.push(restampSubtree(t));
    }
    merged[coll] = out;
  }
  return merged;
}

/** Reset one matched embedded child: its own bands to the template counterpart (placement kept),
 * recursing into its own embedded collections the same way. */
function revertChild(child: WireDocument, template: WireDocument): WireDocument {
  const bands = revertBands(child, template, placementExclusions(child.doc_type));
  return {
    ...child,
    name: bands.name,
    engine: bands.engine,
    system: bands.system,
    embedded: revertEmbedded(template.embedded, child.embedded),
  };
}

/** Revert: discard the child's local diffs on the mergeable bands — every path becomes the
 * template's current value, embedded content resets per `revertEmbedded` — except placement
 * paths (kept, E8), then refresh `base`. No conflicts are possible (revert never asks the user
 * to choose; it always takes the template). */
export function computeRevert(child: WireDocument, template: WireDocument): WireOperation {
  const bands = revertBands(child, template, placementExclusions(child.doc_type));
  const mergedBands: MergeBands = {
    name: bands.name,
    engine: bands.engine,
    system: bands.system,
    embedded: revertEmbedded(template.embedded, child.embedded),
  };
  return planToUpdate(child, template, mergedBands);
}

/** All in-store documents stamped from `templateId` (correlated by `source.id`). Same-world,
 * see+write scoped is the caller's responsibility (it passes the visible store snapshot). */
export function findInstances(templateId: string, all: Iterable<WireDocument>): WireDocument[] {
  const out: WireDocument[] = [];
  for (const d of all) if (d.source?.id === templateId) out.push(d);
  return out;
}

/** "template changed" iff base diverges from the template's current mergeable snapshot, ignoring
 * placement exclusions. Purely local; `none` when unstamped or the template is not in store. */
export function syncState(child: WireDocument, template: WireDocument | undefined): SyncState {
  if (!child.source || !template) return "none";
  const base: MergeBase = (child.base as MergeBase | undefined) ?? snapshotBase(child);
  const excl = placementExclusions(child.doc_type);
  const diverged = structuralDiff(base, snapshotBase(template)).filter((d) => !isPlacementExcluded(d.path, excl));
  return diverged.length === 0 ? "up_to_date" : "template_changed";
}
```

- [ ] **Step 4: Run tests**

Run: `pnpm --filter @shadowcat/core exec vitest run src/templates.test.ts 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Export from `index.ts`**

Update the templates export block:

```ts
export { snapshotBase, stampInstance, computePull, computeRevert, planToUpdate, applyResolutions, findInstances, syncState } from "./templates";
export type { StampOpts, SyncState } from "./templates";
```

- [ ] **Step 6: Full core suite + typecheck**

Run: `pnpm --filter @shadowcat/core test 2>&1 | tail -15`
Expected: PASS (all core tests).
Run: `pnpm --filter @shadowcat/core typecheck`
Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git add src/client/core/src/templates.ts src/client/core/src/templates.test.ts src/client/core/src/index.ts
git commit -m "feat(m13e): core pull/revert/plan emission + findInstances + syncState (buddy-check)"
```

---

## Task 8: ui-kit `MergeConflictModal.svelte`

**Files:**
- Create: `src/client/ui-kit/src/MergeConflictModal.svelte`
- Create: `src/client/ui-kit/src/MergeConflictModal.test.ts`
- Modify: `src/client/ui-kit/src/locales/en.ts`
- Modify: `src/client/ui-kit/src/index.ts`

**Interfaces:**
- Consumes: `Conflict` from `@shadowcat/core`; `getAppContext().t`.
- Produces: `MergeConflictModal` with props:
  - `type ConflictGroup = { key: string; label: string | null; conflicts: Conflict[] }`
  - `groups: ConflictGroup[]`
  - `onApply: (theirsByGroup: Map<string, Set<string>>) => void`
  - `onCancel: () => void`

- [ ] **Step 1: Add i18n keys** (in `src/client/ui-kit/src/locales/en.ts`, after the `sheets.*` block ~line 229)

```ts
  "templates.conflict.title": "Resolve template conflicts",
  "templates.conflict.field": "Field",
  "templates.conflict.base": "Was",
  "templates.conflict.template": "Template",
  "templates.conflict.mine": "Mine",
  "templates.conflict.keepMine": "Keep mine",
  "templates.conflict.takeTemplate": "Take template",
  "templates.conflict.apply": "Apply",
  "templates.conflict.cancel": "Cancel",
  "templates.conflict.deleted": "(deleted)",
  "templates.badge.upToDate": "Up to date",
  "templates.badge.changed": "Template changed",
  "templates.badge.source": "From {name}",
  "templates.action.pull": "Pull from template",
  "templates.action.push": "Push to instances",
  "templates.action.revert": "Revert to template",
```

- [ ] **Step 2: Write the failing component test** (`src/client/ui-kit/src/MergeConflictModal.test.ts`)

```ts
import { describe, it, expect } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import MergeConflictModal from "./MergeConflictModal.svelte";
import { setAppContextForTest } from "./__fixtures__/appContextTest";
import type { Conflict } from "@shadowcat/core";

const conflicts: Conflict[] = [
  { path: "/system/hp", base: 1, parent: 5, child: 9, parentKind: "set" },
  { path: "/system/name", base: "x", parent: "y", child: "z", parentKind: "set" },
];

describe("MergeConflictModal", () => {
  it("renders one row per conflict with base/template/mine values", () => {
    const context = setAppContextForTest();
    const { getByText } = render(MergeConflictModal, {
      props: { groups: [{ key: "C", label: null, conflicts }], onApply: () => {}, onCancel: () => {} },
      context,
    });
    expect(getByText("/system/hp")).toBeTruthy();
    expect(getByText("5")).toBeTruthy(); // template value
    expect(getByText("9")).toBeTruthy(); // mine value
  });

  it("Apply reports only the fields switched to 'take template'", async () => {
    const applied: Map<string, Set<string>>[] = [];
    const context = setAppContextForTest();
    const { getByText, getAllByRole } = render(MergeConflictModal, {
      props: { groups: [{ key: "C", label: null, conflicts }], onApply: (m) => applied.push(m), onCancel: () => {} },
      context,
    });
    // radios come in pairs (keep mine / take template) per row; switch row 0 to template.
    const radios = getAllByRole("radio") as HTMLInputElement[];
    const takeTemplateRow0 = radios.find((r) => r.value === "theirs" && r.name === "C /system/hp")!;
    await fireEvent.click(takeTemplateRow0);
    await fireEvent.click(getByText("templates.conflict.apply"));
    expect(applied).toHaveLength(1);
    expect([...applied[0].get("C")!]).toEqual(["/system/hp"]);
  });

  it("Cancel reports nothing applied", async () => {
    let cancelled = false;
    const context = setAppContextForTest();
    const { getByText } = render(MergeConflictModal, {
      props: { groups: [{ key: "C", label: null, conflicts }], onApply: () => {}, onCancel: () => { cancelled = true; } },
      context,
    });
    await fireEvent.click(getByText("templates.conflict.cancel"));
    expect(cancelled).toBe(true);
  });

  it("groups rows under a per-instance label when provided (push)", () => {
    const context = setAppContextForTest();
    const { getByText } = render(MergeConflictModal, {
      props: {
        groups: [
          { key: "i1", label: "Goblin A", conflicts: [conflicts[0]] },
          { key: "i2", label: "Goblin B", conflicts: [conflicts[1]] },
        ],
        onApply: () => {}, onCancel: () => {},
      },
      context,
    });
    expect(getByText("Goblin A")).toBeTruthy();
    expect(getByText("Goblin B")).toBeTruthy();
  });
});
```

- [ ] **Step 3: Run, verify failure**

Run: `pnpm --filter @shadowcat/ui-kit exec vitest run src/MergeConflictModal.test.ts 2>&1 | tail -15`
Expected: FAIL — cannot resolve `./MergeConflictModal.svelte`.

- [ ] **Step 4: Create `MergeConflictModal.svelte`**

```svelte
<script lang="ts">
  import { getAppContext } from "./appContext";
  import type { Conflict } from "@shadowcat/core";

  export type ConflictGroup = { key: string; label: string | null; conflicts: Conflict[] };

  let { groups, onApply, onCancel }: {
    groups: ConflictGroup[];
    onApply: (theirsByGroup: Map<string, Set<string>>) => void;
    onCancel: () => void;
  } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  // Radio group name per (group,field), NUL-joined so it never collides with a field path.
  const rowKey = (groupKey: string, path: string): string => `${groupKey} ${path}`;

  // Selection: rowKey → "mine" | "theirs". Default "mine" (keep child).
  let choice = $state<Record<string, "mine" | "theirs">>({});

  function display(v: unknown): string {
    return v === undefined ? t("templates.conflict.deleted") : typeof v === "string" ? v : JSON.stringify(v);
  }

  function apply(): void {
    const out = new Map<string, Set<string>>();
    for (const g of groups) {
      const set = new Set<string>();
      for (const c of g.conflicts) if (choice[rowKey(g.key, c.path)] === "theirs") set.add(c.path);
      if (set.size > 0) out.set(g.key, set);
    }
    onApply(out);
  }
</script>

<div class="modal-scrim" role="presentation" onclick={onCancel}>
  <div class="modal" role="dialog" aria-modal="true" aria-label={t("templates.conflict.title")}
       onclick={(e) => e.stopPropagation()}>
    <h2>{t("templates.conflict.title")}</h2>
    {#each groups as g (g.key)}
      {#if g.label !== null}<h3>{g.label}</h3>{/if}
      <ul class="rows">
        {#each g.conflicts as c (c.path)}
          <li class="row">
            <span class="field">{c.path}</span>
            <span class="was">{t("templates.conflict.base")}: {display(c.base)}</span>
            <label>
              <input type="radio" name={rowKey(g.key, c.path)} value="mine"
                     checked={(choice[rowKey(g.key, c.path)] ?? "mine") === "mine"}
                     onchange={() => (choice[rowKey(g.key, c.path)] = "mine")} />
              {t("templates.conflict.mine")}: {display(c.child)}
            </label>
            <label>
              <input type="radio" name={rowKey(g.key, c.path)} value="theirs"
                     checked={choice[rowKey(g.key, c.path)] === "theirs"}
                     onchange={() => (choice[rowKey(g.key, c.path)] = "theirs")} />
              {t("templates.conflict.template")}: {display(c.parent)}
            </label>
          </li>
        {/each}
      </ul>
    {/each}
    <div class="actions">
      <button type="button" class="cancel" onclick={onCancel}>{t("templates.conflict.cancel")}</button>
      <button type="button" class="apply" onclick={apply}>{t("templates.conflict.apply")}</button>
    </div>
  </div>
</div>

<style lang="scss">
  .modal-scrim { position: fixed; inset: 0; background: rgba(0, 0, 0, 0.5); display: flex; align-items: center; justify-content: center; z-index: 1000; }
  .modal { background: var(--surface-raised); color: var(--text); border: 1px solid var(--border); border-radius: var(--radius-2); padding: var(--space-3); max-width: min(90vw, 40rem); max-height: 85vh; overflow: auto; display: flex; flex-direction: column; gap: var(--space-2); }
  h2 { margin: 0; font-size: var(--font-lg); }
  h3 { margin: var(--space-1) 0 0; font-size: var(--font-md); }
  .rows { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--space-1); }
  .row { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); padding: var(--space-1); border-bottom: 1px solid var(--border); }
  .field { font-family: monospace; font-weight: 600; }
  .was { opacity: 0.7; }
  label { display: inline-flex; align-items: center; gap: var(--space-1); }
  .actions { display: flex; justify-content: flex-end; gap: var(--space-2); margin-top: var(--space-2); }
  button { min-height: 44px; padding: 0 var(--space-3); border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface); color: inherit; }
  .apply { background: var(--accent); color: var(--accent-contrast, #fff); }
  input:focus-visible, button:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
</style>
```

- [ ] **Step 5: Run tests**

Run: `pnpm --filter @shadowcat/ui-kit exec vitest run src/MergeConflictModal.test.ts 2>&1 | tail -15`
Expected: PASS.

- [ ] **Step 6: Export from `index.ts`**

In `src/client/ui-kit/src/index.ts` add:

```ts
export { default as MergeConflictModal } from "./MergeConflictModal.svelte";
export type { ConflictGroup } from "./MergeConflictModal.svelte";
```

- [ ] **Step 7: Typecheck**

Run: `pnpm --filter @shadowcat/ui-kit typecheck`
Expected: no errors.

- [ ] **Step 8: Commit**

```bash
git add src/client/ui-kit/src/MergeConflictModal.svelte src/client/ui-kit/src/MergeConflictModal.test.ts src/client/ui-kit/src/locales/en.ts src/client/ui-kit/src/index.ts
git commit -m "feat(m13e): generic MergeConflictModal (per-field radios, per-instance groups)"
```

---

## Task 9: ui-kit `AppContext.templates` seam + `TemplatesController` + modal host

**Files:**
- Create: `src/client/ui-kit/src/templatesController.svelte.ts`
- Create: `src/client/ui-kit/src/TemplateModalHost.svelte`
- Create: `src/client/ui-kit/src/templatesController.svelte.test.ts`
- Modify: `src/client/ui-kit/src/appContext.ts`
- Modify: `src/client/ui-kit/src/__fixtures__/appContextTest.ts`
- Modify: `src/client/ui-kit/src/index.ts`

**Interfaces:**
- Consumes: `computePull`, `computeRevert`, `planToUpdate`, `applyResolutions`, `findInstances`, `syncState`, `stampInstance`, `WireDocument`, `WireOperation`, `MergePlan`, `Conflict`, `StampOpts`, `SyncState`, `Logger`, `DocumentStore`, `ReadableDocuments` from `@shadowcat/core`; `ConflictGroup` from `./MergeConflictModal.svelte`.
- Produces:
  - `interface TemplatesApi { stampInstance(source, opts): WireDocument; pull(childId): void; push(templateId): void; revert(childId): void; findInstances(templateId): WireDocument[]; syncState(childId): SyncState; canPull(childId): boolean; canPush(templateId): boolean }` (in `appContext.ts`)
  - `class TemplatesController` with `deps` + a reactive `pending: { groups: ConflictGroup[]; resolve(theirsByGroup): void } | null`, and `cancel()`.
  - `TemplateModalHost` — mounts `MergeConflictModal` from `ctx.templates` pending.

- [ ] **Step 1: Add `TemplatesApi` to `AppContext`**

In `src/client/ui-kit/src/appContext.ts`, extend the imports:

```ts
import type { ContributionRegistry, DocumentStore, ReadableDocuments, AssetResolver, SceneFrame, SceneSubscription, WireOperation, WireDocument, PathResult, MoveStream, WireActorOwnerRef, WireAudience, SheetRef, SubscriptionHandle, WireSearchHit, StampOpts, SyncState } from "@shadowcat/core";
```

Add before the `ChatApi` interface:

```ts
/** Template pull/push/revert/stamp seam (§6.3). Thin orchestration over `store`/`documents` +
 * `dispatchIntent`; the controller opens the conflict modal when needed. */
export interface TemplatesApi {
  /** Deep-clone `source` into a new stamped instance; the caller dispatches the Create. */
  stampInstance(source: WireDocument, opts: StampOpts): WireDocument;
  /** Merge the template into the child; opens the modal on conflicts, else dispatches directly. */
  pull(childId: string): void;
  /** Push the template to every in-store instance the pusher can see + write. */
  push(templateId: string): void;
  /** Reset the child's mergeable bands to the template (keeping placement); refresh base. */
  revert(childId: string): void;
  /** In-store instances stamped from `templateId`. */
  findInstances(templateId: string): WireDocument[];
  /** Provenance/sync state for the sheet badge. */
  syncState(childId: string): SyncState;
  /** Whether the current user may pull/revert this child (owner-or-GM + write caps). */
  canPull(childId: string): boolean;
  /** Whether the current user may push this template (owner-or-GM). */
  canPush(templateId: string): boolean;
}
```

Add the field to the `AppContext` interface (after `chat: ChatApi;`):

```ts
  /** Template merge seam: stamp + pull/push/revert (M13e). */
  templates: TemplatesApi;
```

- [ ] **Step 2: Write the failing controller test** (`src/client/ui-kit/src/templatesController.svelte.test.ts`)

```ts
import { describe, it, expect } from "vitest";
import { TemplatesController } from "./templatesController.svelte";
import { DocumentStore, silentLogger, type WireDocument, type WireOperation } from "@shadowcat/core";

function doc(over: Partial<WireDocument> & { id: string }): WireDocument {
  return {
    id: over.id, scope: { kind: "world", world_id: "w1" }, doc_type: over.doc_type ?? "actor",
    schema_version: 1, name: over.name ?? null, source: over.source ?? null, owner: over.owner ?? null,
    permissions: over.permissions ?? { default: "owner", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
    embedded: over.embedded ?? {}, parent_id: null, engine: over.engine, system: over.system ?? {},
    created_at: 0, updated_at: 0,
  };
}

function make(docs: WireDocument[], over: Partial<{ role: "gm" | "player"; selfId: string }> = {}) {
  const store = new DocumentStore();
  store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((d) => ({ op: "create", doc: d } as WireOperation)) });
  const calls: WireOperation[][] = [];
  const ctrl = new TemplatesController({
    store, documents: store, dispatchIntent: (ops) => calls.push(ops),
    role: over.role ?? "gm", selfId: over.selfId ?? "u-self",
    canEdit: () => true, logger: silentLogger,
  });
  return { store, ctrl, calls };
}

describe("TemplatesController", () => {
  it("pull with no conflicts dispatches an Update directly (no modal)", () => {
    const tmpl = doc({ id: "T", name: "T", system: { hp: 5 } });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 }, system: { hp: 1 } });
    child.base = { name: "T", engine: null, system: { hp: 1 }, embedded: {} };
    const { ctrl, calls } = make([tmpl, child]);
    ctrl.pull("C");
    expect(ctrl.pending).toBeNull();
    expect(calls).toHaveLength(1);
    expect(calls[0][0].op).toBe("update");
  });

  it("pull with a conflict opens the modal and dispatches on resolve", () => {
    const tmpl = doc({ id: "T", system: { hp: 5 } });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 }, system: { hp: 9 } });
    child.base = { name: null, engine: null, system: { hp: 1 }, embedded: {} };
    const { ctrl, calls } = make([tmpl, child]);
    ctrl.pull("C");
    expect(ctrl.pending).not.toBeNull();
    expect(calls).toHaveLength(0);
    ctrl.pending!.resolve(new Map([["C", new Set(["/system/hp"])]]));
    expect(ctrl.pending).toBeNull();
    expect(calls).toHaveLength(1);
    const sys = (calls[0][0] as { changes: { path: string; new: unknown }[] }).changes.find((c) => c.path === "/system")!;
    expect(sys.new).toEqual({ hp: 5 }); // took template
  });

  it("pull is unavailable (no dispatch) when the template is not in store", () => {
    const child = doc({ id: "C", source: { id: "MISSING", pack: null, version: 1 } });
    const { ctrl, calls } = make([child]);
    ctrl.pull("C");
    expect(calls).toHaveLength(0);
    expect(ctrl.pending).toBeNull();
  });

  it("push dispatches one Update per conflict-free instance and groups conflicts", () => {
    const tmpl = doc({ id: "T", system: { hp: 5 } });
    const clean = doc({ id: "A", source: { id: "T", pack: null, version: 1 }, system: { hp: 1 } });
    clean.base = { name: null, engine: null, system: { hp: 1 }, embedded: {} };
    const conflicted = doc({ id: "B", source: { id: "T", pack: null, version: 1 }, system: { hp: 9 } });
    conflicted.base = { name: null, engine: null, system: { hp: 1 }, embedded: {} };
    const { ctrl, calls } = make([tmpl, clean, conflicted]);
    ctrl.push("T");
    expect(calls).toHaveLength(1); // clean instance applied immediately
    expect(ctrl.pending).not.toBeNull();
    expect(ctrl.pending!.groups.map((g) => g.key)).toEqual(["B"]);
  });

  it("canPull is false for a non-owner non-GM", () => {
    const tmpl = doc({ id: "T" });
    const child = doc({ id: "C", owner: "someone-else", source: { id: "T", pack: null, version: 1 } });
    const { ctrl } = make([tmpl, child], { role: "player", selfId: "u-self" });
    expect(ctrl.canPull("C")).toBe(false);
  });

  it("findInstances returns instances of the template from the store", () => {
    const tmpl = doc({ id: "T" });
    const a = doc({ id: "A", source: { id: "T", pack: null, version: 1 } });
    const { ctrl } = make([tmpl, a]);
    expect(ctrl.findInstances("T").map((d) => d.id)).toEqual(["A"]);
  });
});
```

- [ ] **Step 3: Run, verify failure**

Run: `pnpm --filter @shadowcat/ui-kit exec vitest run src/templatesController.svelte.test.ts 2>&1 | tail -15`
Expected: FAIL — cannot resolve `./templatesController.svelte`.

- [ ] **Step 4: Create `templatesController.svelte.ts`**

```ts
// Template merge orchestration (M13e). Thin glue: pure core functions → the conflict modal →
// `dispatchIntent`. Holds a reactive `pending` conflict session the `TemplateModalHost` renders.
// Constructed by the shell alongside `SheetsController`; imports no module.
import {
  computePull, computeRevert, planToUpdate, applyResolutions, findInstances, syncState, stampInstance,
  type WireDocument, type WireOperation, type StampOpts, type SyncState, type Logger,
  type DocumentStore, type ReadableDocuments, type MergePlan,
} from "@shadowcat/core";
import type { ConflictGroup } from "./MergeConflictModal.svelte";

export interface TemplatesControllerDeps {
  store: DocumentStore;
  documents: ReadableDocuments;
  dispatchIntent: (ops: WireOperation[]) => void;
  role: "gm" | "player" | "spectator";
  selfId: string;
  /** Advisory write gate (mirrors the server). */
  canEdit: (doc: WireDocument, path: string) => boolean;
  logger: Logger;
}

/** An open conflict-resolution session: the grouped conflicts + a resolver the modal calls. */
export interface PendingSession {
  groups: ConflictGroup[];
  resolve: (theirsByGroup: Map<string, Set<string>>) => void;
}

export class TemplatesController {
  #deps: TemplatesControllerDeps;
  pending = $state<PendingSession | null>(null);

  constructor(deps: TemplatesControllerDeps) {
    this.#deps = deps;
  }

  #get(id: string): WireDocument | undefined {
    return this.#deps.documents.get(id);
  }

  #templateOf(child: WireDocument): WireDocument | undefined {
    return child.source ? this.#get(child.source.id) : undefined;
  }

  #isOwnerOrGm(doc: WireDocument): boolean {
    return this.#deps.role === "gm" || doc.owner === this.#deps.selfId;
  }

  stampInstance(source: WireDocument, opts: StampOpts): WireDocument {
    return stampInstance(source, opts);
  }

  findInstances(templateId: string): WireDocument[] {
    return findInstances(templateId, this.#deps.store.snapshot());
  }

  syncState(childId: string): SyncState {
    const child = this.#get(childId);
    if (!child) return "none";
    return syncState(child, this.#templateOf(child));
  }

  canPull(childId: string): boolean {
    const child = this.#get(childId);
    if (!child || !this.#templateOf(child)) return false;
    return this.#isOwnerOrGm(child) && this.#deps.canEdit(child, "/base") && this.#deps.canEdit(child, "/system");
  }

  canPush(templateId: string): boolean {
    const tmpl = this.#get(templateId);
    if (!tmpl) return false;
    return this.#isOwnerOrGm(tmpl) && this.findInstances(templateId).length > 0;
  }

  pull(childId: string): void {
    const child = this.#get(childId);
    if (!child) return;
    const template = this.#templateOf(child);
    if (!template) {
      this.#deps.logger.warn(`templates.pull: template ${child.source?.id ?? "?"} not in store; pull unavailable`);
      return;
    }
    const plan = computePull(child, template);
    if (plan.conflicts.length === 0) {
      this.#deps.dispatchIntent([planToUpdate(child, template, plan.mergedBands)]);
      return;
    }
    this.#openSession([{ key: childId, label: null, conflicts: plan.conflicts }], new Map([[childId, { child, template, plan }]]));
  }

  revert(childId: string): void {
    const child = this.#get(childId);
    if (!child) return;
    const template = this.#templateOf(child);
    if (!template) {
      this.#deps.logger.warn(`templates.revert: template ${child.source?.id ?? "?"} not in store; revert unavailable`);
      return;
    }
    this.#deps.dispatchIntent([computeRevert(child, template)]);
  }

  push(templateId: string): void {
    const template = this.#get(templateId);
    if (!template) return;
    const instances = this.findInstances(templateId);
    const groups: ConflictGroup[] = [];
    const conflicted = new Map<string, { child: WireDocument; template: WireDocument; plan: MergePlan }>();
    for (const inst of instances) {
      const plan = computePull(inst, template);
      if (plan.conflicts.length === 0) {
        this.#deps.dispatchIntent([planToUpdate(inst, template, plan.mergedBands)]);
      } else {
        groups.push({ key: inst.id, label: inst.name ?? inst.id, conflicts: plan.conflicts });
        conflicted.set(inst.id, { child: inst, template, plan });
      }
    }
    if (groups.length > 0) this.#openSession(groups, conflicted);
  }

  cancel(): void {
    this.pending = null;
  }

  #openSession(
    groups: ConflictGroup[],
    byKey: Map<string, { child: WireDocument; template: WireDocument; plan: MergePlan }>,
  ): void {
    this.pending = {
      groups,
      resolve: (theirsByGroup) => {
        for (const [key, entry] of byKey) {
          const theirs = theirsByGroup.get(key) ?? new Set<string>();
          const resolved = applyResolutions(entry.plan.mergedBands, entry.plan.conflicts, theirs);
          this.#deps.dispatchIntent([planToUpdate(entry.child, entry.template, resolved)]);
        }
        this.pending = null;
      },
    };
  }
}
```

- [ ] **Step 5: Run controller tests**

Run: `pnpm --filter @shadowcat/ui-kit exec vitest run src/templatesController.svelte.test.ts 2>&1 | tail -15`
Expected: PASS.

- [ ] **Step 6: Create `TemplateModalHost.svelte`**

```svelte
<script lang="ts">
  // Mounts the conflict modal from the templates controller's reactive `pending` session. The
  // shell renders exactly one of these; the seam exposes `_pending`/`_cancel` for it.
  import { getAppContext } from "./appContext";
  import MergeConflictModal from "./MergeConflictModal.svelte";
  import type { TemplatesController } from "./templatesController.svelte";

  const ctx = getAppContext();
  // The shell provides the concrete controller behind the seam.
  let { controller }: { controller: TemplatesController } = $props();
  void ctx;
</script>

{#if controller.pending}
  <MergeConflictModal
    groups={controller.pending.groups}
    onApply={(m) => controller.pending?.resolve(m)}
    onCancel={() => controller.cancel()}
  />
{/if}
```

- [ ] **Step 7: Add `templates` to the test fixture**

In `src/client/ui-kit/src/__fixtures__/appContextTest.ts`, add before the closing `};` of the `ctx` object:

```ts
    templates: over.templates ?? {
      stampInstance: (s) => s,
      pull: () => {},
      push: () => {},
      revert: () => {},
      findInstances: () => [],
      syncState: () => "none",
      canPull: () => false,
      canPush: () => false,
    },
```

- [ ] **Step 8: Export from `index.ts`**

```ts
export { TemplatesController } from "./templatesController.svelte";
export type { TemplatesControllerDeps, PendingSession } from "./templatesController.svelte";
export { default as TemplateModalHost } from "./TemplateModalHost.svelte";
export type { TemplatesApi } from "./appContext";
```

- [ ] **Step 9: Run ui-kit tests + typecheck**

Run: `pnpm --filter @shadowcat/ui-kit test 2>&1 | tail -15`
Expected: PASS.
Run: `pnpm --filter @shadowcat/ui-kit typecheck`
Expected: no errors.

- [ ] **Step 10: Commit**

```bash
git add src/client/ui-kit/src/templatesController.svelte.ts src/client/ui-kit/src/TemplateModalHost.svelte src/client/ui-kit/src/templatesController.svelte.test.ts src/client/ui-kit/src/appContext.ts src/client/ui-kit/src/__fixtures__/appContextTest.ts src/client/ui-kit/src/index.ts
git commit -m "feat(m13e): AppContext.templates seam + TemplatesController + modal host"
```

---

## Task 10: ui-kit `TemplateControls.svelte` + generic `SheetHost` wrapper

**Files:**
- Create: `src/client/ui-kit/src/TemplateControls.svelte`
- Create: `src/client/ui-kit/src/SheetHost.svelte`
- Create: `src/client/ui-kit/src/TemplateControls.test.ts`
- Modify: `src/client/ui-kit/src/sheetsController.svelte.ts`
- Modify: `src/client/ui-kit/src/index.ts`

**Interfaces:**
- Consumes: `getAppContext().templates`, `.documents`; `WireDocument` from `@shadowcat/core`.
- Produces:
  - `TemplateControls` with props `{ docId: string }` — badge + pull/push/revert buttons, shown only when warranted.
  - `SheetHost` with props `{ docId: string; systemPrefix: string; close: () => void; inner: unknown }` — renders `TemplateControls` above the module sheet body.

- [ ] **Step 1: Write the failing test** (`src/client/ui-kit/src/TemplateControls.test.ts`)

```ts
import { describe, it, expect } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import TemplateControls from "./TemplateControls.svelte";
import { setAppContextForTest } from "./__fixtures__/appContextTest";
import { DocumentStore, type WireDocument, type WireOperation } from "@shadowcat/core";

function doc(over: Partial<WireDocument> & { id: string }): WireDocument {
  return {
    id: over.id, scope: { kind: "world", world_id: "w1" }, doc_type: "actor", schema_version: 1,
    name: over.name ?? null, source: over.source ?? null, owner: over.owner ?? null,
    permissions: { default: "owner", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
    embedded: {}, parent_id: null, engine: over.engine, system: over.system ?? {}, created_at: 0, updated_at: 0,
  };
}

function storeWith(docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((d) => ({ op: "create", doc: d } as WireOperation)) });
  return s;
}

describe("TemplateControls", () => {
  it("renders nothing for a document with no source and no instances", () => {
    const store = storeWith([doc({ id: "C" })]);
    const context = setAppContextForTest({ store, documents: store });
    const { queryByRole } = render(TemplateControls, { props: { docId: "C" }, context });
    expect(queryByRole("button")).toBeNull();
  });

  it("shows the source badge + pull/revert for a stamped, pull-authorized doc", () => {
    const tmpl = doc({ id: "T", name: "Preset" });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 } });
    const store = storeWith([tmpl, child]);
    const pulled: string[] = [];
    const context = setAppContextForTest({
      store, documents: store,
      templates: {
        stampInstance: (s) => s, pull: (id) => pulled.push(id), push: () => {}, revert: () => {},
        findInstances: () => [], syncState: () => "template_changed", canPull: () => true, canPush: () => false,
      },
    });
    const { getByText } = render(TemplateControls, { props: { docId: "C" }, context });
    expect(getByText("templates.badge.source")).toBeTruthy();
    fireEvent.click(getByText("templates.action.pull"));
    expect(pulled).toEqual(["C"]);
  });

  it("shows push when the doc has instances and push is authorized", () => {
    const tmpl = doc({ id: "T" });
    const store = storeWith([tmpl, doc({ id: "A", source: { id: "T", pack: null, version: 1 } })]);
    const pushed: string[] = [];
    const context = setAppContextForTest({
      store, documents: store,
      templates: {
        stampInstance: (s) => s, pull: () => {}, push: (id) => pushed.push(id), revert: () => {},
        findInstances: () => [doc({ id: "A", source: { id: "T", pack: null, version: 1 } })],
        syncState: () => "none", canPull: () => false, canPush: () => true,
      },
    });
    const { getByText } = render(TemplateControls, { props: { docId: "T" }, context });
    fireEvent.click(getByText("templates.action.push"));
    expect(pushed).toEqual(["T"]);
  });
});
```

- [ ] **Step 2: Run, verify failure**

Run: `pnpm --filter @shadowcat/ui-kit exec vitest run src/TemplateControls.test.ts 2>&1 | tail -15`
Expected: FAIL — cannot resolve `./TemplateControls.svelte`.

- [ ] **Step 3: Create `TemplateControls.svelte`**

```svelte
<script lang="ts">
  // Host-rendered template chrome for any doc_type's sheet (§6.1). Reads provenance/instances
  // from the templates seam; shows a source badge + pull/revert (stamped, authorized) and push
  // (has instances, authorized). The module sheet body never opts in.
  import { getAppContext } from "./appContext";
  import type { WireDocument } from "@shadowcat/core";

  let { docId }: { docId: string } = $props();
  const ctx = getAppContext();
  const t = ctx.t;

  const doc = $derived(ctx.documents.get(docId));
  const template = $derived(doc?.source ? ctx.documents.get(doc.source.id) : undefined);
  const sync = $derived(ctx.templates.syncState(docId));
  const canPull = $derived(ctx.templates.canPull(docId));
  const canPush = $derived(ctx.templates.canPush(docId));
  const hasSource = $derived(!!doc?.source && !!template);
</script>

{#if hasSource || canPush}
  <div class="template-controls">
    {#if hasSource}
      <span class="badge" class:changed={sync === "template_changed"}>
        {t("templates.badge.source", { name: template?.name ?? "" })}
        <span class="state">{sync === "template_changed" ? t("templates.badge.changed") : t("templates.badge.upToDate")}</span>
      </span>
      {#if canPull}
        <button type="button" onclick={() => ctx.templates.pull(docId)}>{t("templates.action.pull")}</button>
        <button type="button" onclick={() => ctx.templates.revert(docId)}>{t("templates.action.revert")}</button>
      {/if}
    {/if}
    {#if canPush}
      <button type="button" onclick={() => ctx.templates.push(docId)}>{t("templates.action.push")}</button>
    {/if}
  </div>
{/if}

<style lang="scss">
  .template-controls { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); padding: var(--space-1) var(--space-2); border-bottom: 1px solid var(--border); background: var(--surface); }
  .badge { display: inline-flex; align-items: center; gap: var(--space-1); font-size: var(--font-sm); opacity: 0.85; }
  .badge.changed .state { color: var(--accent); font-weight: 600; }
  button { min-height: 44px; padding: 0 var(--space-2); border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface-raised); color: inherit; }
  button:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
</style>
```

- [ ] **Step 4: Create `SheetHost.svelte`**

```svelte
<script lang="ts">
  // Generic wrapper the sheets controller mounts around EVERY module sheet: renders the
  // host-owned template chrome above the picked sheet body, so any doc_type gets template
  // controls without opting in. `inner` is the picked sheet component; props are forwarded.
  import type { Component } from "svelte";
  import TemplateControls from "./TemplateControls.svelte";

  let { docId, systemPrefix, close, inner }: {
    docId: string; systemPrefix: string; close: () => void; inner: Component<Record<string, unknown>>;
  } = $props();

  const Inner = $derived(inner);
</script>

<div class="sheet-host">
  <TemplateControls {docId} />
  <div class="sheet-body">
    <Inner {docId} {systemPrefix} {close} />
  </div>
</div>

<style lang="scss">
  .sheet-host { display: flex; flex-direction: column; height: 100%; }
  .sheet-body { flex: 1; min-height: 0; overflow: auto; }
</style>
```

- [ ] **Step 5: Mount `SheetHost` in the sheets controller**

In `src/client/ui-kit/src/sheetsController.svelte.ts`, add the import at the top:

```ts
import SheetHost from "./SheetHost.svelte";
```

Change `#register` to wrap the picked component with `SheetHost` (the picked component becomes the `inner` prop):

```ts
  #register(panelId: string, component: unknown, docId: string, systemPrefix: string): void {
    const dispose = this.#deps.contributions.contribute(
      {
        id: panelId,
        contract: PANEL_CONTRACT,
        component: SheetHost,
        props: { docId, systemPrefix, close: () => this.closeDocument(panelId), inner: component },
        panel: { icon: "\u{1F4C4}", labelKey: "sheets.title", defaultPlacement: { kind: "floating" } },
      },
      { module: "sheets" },
    );
    this.#open.set(panelId, dispose);
  }
```

- [ ] **Step 6: Export from `index.ts`**

```ts
export { default as TemplateControls } from "./TemplateControls.svelte";
export { default as SheetHost } from "./SheetHost.svelte";
```

- [ ] **Step 7: Run ui-kit tests + typecheck**

Run: `pnpm --filter @shadowcat/ui-kit test 2>&1 | tail -15`
Expected: PASS (TemplateControls + existing sheet tests still green — `SheetHost` forwards `docId`/`systemPrefix`/`close` unchanged).
Run: `pnpm --filter @shadowcat/ui-kit typecheck`
Expected: no errors.

- [ ] **Step 8: Commit**

```bash
git add src/client/ui-kit/src/TemplateControls.svelte src/client/ui-kit/src/SheetHost.svelte src/client/ui-kit/src/TemplateControls.test.ts src/client/ui-kit/src/sheetsController.svelte.ts src/client/ui-kit/src/index.ts
git commit -m "feat(m13e): host-rendered TemplateControls + generic SheetHost wrapper"
```

---

## Task 11: Shell wiring + full gate

**Files:**
- Modify: `src/client/shell/src/lib/Table.svelte`
- Modify (if needed): `src/client/shell/src/lib/worldSession.svelte.ts`

**Interfaces:**
- Consumes: `TemplatesController`, `TemplateModalHost` from `@shadowcat/ui-kit`; `session.store`/`documents`/`dispatchIntent`/`role`/`selfId`/`canEdit`.
- Produces: `ctx.templates` populated in `setAppContext`; `<TemplateModalHost controller={templates} />` mounted once.

- [ ] **Step 1: Construct the controller in `Table.svelte`**

In `src/client/shell/src/lib/Table.svelte`, extend the ui-kit import:

```ts
  import { setAppContext, Surface, PanelsBridge, SheetsController, SceneSelection, TemplatesController, TemplateModalHost } from "@shadowcat/ui-kit";
```

After the `sheets` controller construction (~line 27), add:

```ts
  // Template merge controller (M13e): stamp/pull/push/revert orchestration + the conflict modal.
  const templates = new TemplatesController({
    store: session.store,
    documents: session.documents,
    dispatchIntent: (ops) => session.dispatchIntent(ops),
    role: session.role!,
    selfId: session.selfId,
    canEdit: (doc, path) => session.canEdit(doc, path),
    logger: consoleLogger(),
  });
```

- [ ] **Step 2: Provide `templates` in `setAppContext`**

In the `setAppContext({ ... })` call, after the `chat: { ... }` block, add:

```ts
    templates: {
      stampInstance: (s, opts) => templates.stampInstance(s, opts),
      pull: (id) => templates.pull(id),
      push: (id) => templates.push(id),
      revert: (id) => templates.revert(id),
      findInstances: (id) => templates.findInstances(id),
      syncState: (id) => templates.syncState(id),
      canPull: (id) => templates.canPull(id),
      canPush: (id) => templates.canPush(id),
    },
```

- [ ] **Step 3: Mount the modal host**

Change the markup at the bottom of `Table.svelte` from:

```svelte
<Surface contract="shadowcat.surface:root" />
```

to:

```svelte
<Surface contract="shadowcat.surface:root" />
<TemplateModalHost controller={templates} />
```

- [ ] **Step 4: Shell typecheck**

Run: `pnpm --filter @shadowcat/shell typecheck 2>&1 | tail -20`
Expected: no errors. (If `session.role`/`selfId`/`store`/`documents`/`canEdit`/`dispatchIntent` are missing on `WorldSession`, they already exist — verified in `worldSession.svelte.ts`; no change needed.)

- [ ] **Step 5: Full workspace gate**

Run: `pnpm -r test 2>&1 | tail -30`
Expected: PASS across all packages (`@shadowcat/core`, `@shadowcat/ui-kit`, `@shadowcat/shell`, and any others). A required-field wire change breaking a fixture would surface here per [[shared-wire-schema-change-needs-full-repo-test]] — `base` is nullish/optional, so no fixture should break, but this gate proves it.
Run: `pnpm -r typecheck 2>&1 | tail -20`
Expected: no errors.
Run: `pnpm lint 2>&1 | tail -20`
Expected: no errors.

- [ ] **Step 6: Server gate (build dist first per rust-embed)**

Run: `pnpm build 2>&1 | tail -5`
Expected: the client bundle builds into `dist/`.
Run: `(cd src/server && cargo test 2>&1 | tail -30)`
Expected: PASS (all server tests, including the Task 1/2 data tests).
Run: `(cd src/server && cargo clippy --all-targets -- -D warnings 2>&1 | tail -20)`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add src/client/shell/src/lib/Table.svelte
git commit -m "feat(m13e): wire TemplatesController + modal host into the shell AppContext"
```

- [ ] **Step 8: Skill-update gate (mandatory, doc-sync tier)**

Update the affected `shadowcat-codebase-*` skill(s) — at minimum `shadowcat-codebase-documents-permissions` (new `base` envelope field + `/base` authz + the client merge engine seam) and `shadowcat-codebase-core` (new `merge.ts`/`templates.ts` public surface). If templates touch actor/token stamping knowledge, update `shadowcat-codebase-actors-tokens` too. Then dispatch `shadowcat-spec-reviewer` on the skill diff (PASS required). These edits land on the feature branch (skills are git-tracked). If no subsystem knowledge changed for a given skill, state so explicitly.

```bash
git add .claude/skills/
git commit -m "docs(skills): M13e templates + base field + merge engine"
```

---

## Self-Review (completed by the plan-writer)

**1. Spec coverage:**
- E1/§4 pull/push/revert/stamp → Tasks 6 (stamp), 7 (pull/revert/plan), 9 (push orchestration).
- E2/§3.1 base snapshot on the child → Task 1 (field), Task 6 (`snapshotBase`), Task 7 (`/base` refresh).
- E3 templates are not a doc_type → no doc_type added; `stampInstance` is generic (Task 6). ✔
- E4/§5 merge scope name+engine+system+embedded → Tasks 3–5; envelope never merged (only band/embedded/base FieldChanges emitted, Task 7). ✔
- E5/§6.2 field-level conflict modal → Task 8. ✔
- E6/§5.1 arrays wholesale, objects key-level → Task 3/4. ✔
- E7/§5.2 embedded correlation by source.id → Task 5. ✔
- E8 placement exclusions → `placementExclusions` (Task 5), applied in pull/revert/syncState (Task 7). ✔
- E9 push same-world see+write → `findInstances` over `store.snapshot()` (Tasks 7/9). ✔
- E10/§7 server surface (base field, `/base`→WRITE_FIELDS, size cap, ts-rs+Zod, no merge) → Tasks 1–2. ✔
- §6.1 host-rendered chrome for every doc_type → `SheetHost` wrapper (Task 10). ✔
- §6.3 AppContext.templates seam → Task 9. ✔
- §6.4 sync-state derivation → `syncState` (Task 7), badge (Task 10). ✔
- §8 testing battery → every task is TDD; permutation/order-independence (Task 4), embedded table (Task 5), deep-clone independence (Task 6), real pre-images + template-not-in-store (Tasks 7/9), server parity/size/exemption (Tasks 1/2). ✔

**2. Placeholder scan:** No TBD/TODO-as-work/"similar to"/"add error handling" left; every code step carries complete code; every command has expected output.

**3. Type consistency:** `MergeBase`/`MergeBands`/`EmbeddedBaseChild`/`Conflict`/`MergePlan`/`Diff`/`StampOpts`/`SyncState` are defined once (Tasks 3–7) and imported by name thereafter; `ConflictGroup` (Task 8) is consumed by the controller (Task 9); `TemplatesApi` (Task 9) is consumed by the fixture, controls (Task 10), and shell (Task 11); `TemplatesController` method names (`pull`/`push`/`revert`/`stampInstance`/`findInstances`/`syncState`/`canPull`/`canPush`/`cancel`, `pending`) are stable across Tasks 9–11.
