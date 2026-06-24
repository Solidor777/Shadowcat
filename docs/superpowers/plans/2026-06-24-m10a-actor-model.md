# M10a — Actor Model Implementation Plan

> **For agentic workers:** This plan is executed mainline (per the project's
> `mainline-plan-execution` directive): implement task-by-task in this session with an
> inline spec-compliance check per task and ONE buddy-check of the full branch before
> merge (see "Buddy-check directives"). Steps use checkbox (`- [ ]`) syntax.

**Goal:** Introduce the game `Actor` document and back tokens with it — linked (shared,
live) or instanced (independent embedded copy) — with a single resolution read-through,
a minimal create/list/pick UI, and the user-side "actor" → "user" terminology rename.

**Architecture:** A new world-scoped `doc_type:"actor"` (zero server machinery — the
server has no doc_type allowlist and stays structural-only #6). A token references its
actor by `actor_id` (linked) or carries an embedded copy with `source` provenance
(instanced). `resolveTokenActor(token, store) → EffectiveActor` is the one read-through
every consumer uses. A `module-actors` package provides the UI; the scene-tools place
tool stamps the selected actor via a small `ActorSelection` seam on `AppContext`.

**Tech Stack:** Rust (axum/ts-rs) server; TypeScript `@shadowcat/core` (framework-neutral)
+ `@shadowcat/ui-kit` (Svelte 5 runtime) + `@shadowcat/render` (PixiJS); Svelte 5 modules
under `src/modules/*`. Tests: Vitest (`pnpm -r test`), cargo test (server).

## Global Constraints

- **Server structural-only (#6):** the actor `system` body is opaque; add NO server-side
  actor schema/validation beyond the existing 256 KB `system` size cap.
- **ts-rs sync (CI-enforced):** any Rust wire-type change must regenerate
  `src/types/generated/*.ts` (run `cargo test` in `src/server`, which runs the ts-rs
  export tests) AND be mirrored in the hand-written Zod schema in
  `src/client/core/src/wire.ts`.
- **Naming:** the game entity is `Actor` (`doc_type:"actor"`); the user-side principal is
  `user` (never "actor").
- **Wire-shape single source of truth:** all document construction goes through builders
  in `src/client/core/src/scene-docs.ts` (never inline document literals in modules).
- **Center origin:** a token's `(x, y)` is its CENTER (matches `Grid.snap`).
- **TDD, DRY, YAGNI, frequent commits.** Run `pnpm -r typecheck` before each commit.

---

## Buddy-check directives

Per the user's standing instruction, buddy-check the **full M10a branch** (two
independent reviewers, reconciled) before merge. Focus areas:
1. **Rename completeness** — no stale `actor_role` anywhere (Rust, generated TS, Zod,
   consumers); generated types in sync with the Rust source (grep the whole tree, all
   file types — see the stale-ref lesson).
2. **Link/instance correctness** — instanced tokens carry an embedded copy with correct
   `source` provenance and a fresh id; linked tokens carry `actor_id` + apply the
   `overrides` whitelist; `resolveTokenActor` handles linked, instanced, missing-actor,
   and raw (no-actor) cases.
3. **#6 preserved** — the actor doc adds zero server semantics (no new server validation).

---

## Task 1: Rename user-side `actor_role` → `user_role`

**Files:**
- Modify: `src/server/src/ws/protocol.rs:123` (field), `:388`, `:394` (test)
- Modify: `src/server/src/ws/conn.rs:456` (construction site)
- Regenerate: `src/types/generated/ServerMsg.ts` (via `cargo test`)
- Modify: `src/client/core/src/wire.ts:147` (Zod schema)
- Modify: `src/client/shell/src/lib/worldSession.svelte.ts:231,246` (consumers)
- Modify: doc-comments meaning the requesting user in `src/server/src/data/permission.rs`
  (lines ~102,125,185,187), `src/server/src/data/membership.rs:1`,
  `src/client/core/src/capabilities.ts` (lines ~1,18), `src/client/ui-kit/src/appContext.ts:9`

**Interfaces:**
- Produces: `WireWelcome.user_role` (was `actor_role`) — the requesting user's `WorldRole`.

- [ ] **Step 1: Update the Rust test to assert the new field name (failing test)**

In `src/server/src/ws/protocol.rs`, change the test assertion (line ~394):
```rust
assert_eq!(json["user_role"], "player");
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd src/server && cargo test --lib protocol`
Expected: FAIL (the serialized field is still `actor_role`).

- [ ] **Step 3: Rename the Rust field + construction + test literal**

`src/server/src/ws/protocol.rs:123`:
```rust
        user_role: crate::data::document::WorldRole,
```
`src/server/src/ws/protocol.rs:388` (test builder):
```rust
            user_role: WorldRole::Player,
```
`src/server/src/ws/conn.rs:456`:
```rust
            user_role: ctx.world_role,
```

- [ ] **Step 4: Run server tests; this also regenerates the ts-rs bindings**

Run: `cd src/server && cargo test`
Expected: PASS. Confirm `src/types/generated/ServerMsg.ts` now shows `user_role` (not
`actor_role`).

- [ ] **Step 5: Mirror the rename in the Zod schema + consumers**

`src/client/core/src/wire.ts:147`:
```ts
    user_role: WorldRoleSchema,
```
`src/client/shell/src/lib/worldSession.svelte.ts` (lines ~231 and ~246):
```ts
      this.role = w.user_role;
```
```ts
      if (w.user_role === "gm") {
```
Update the doc-comments listed above: "the actor" → "the user"/"the requesting user".

- [ ] **Step 6: Verify the whole tree has no stale `actor_role` and typechecks**

Run: `grep -rn "actor_role" src` → expect no matches.
Run: `pnpm -r typecheck && pnpm -r test`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor(m10a): rename user-side actor_role -> user_role"
```

---

## Task 2: Actor document model in core (`ActorSystem` + `buildActorDoc`)

**Files:**
- Modify: `src/client/core/src/scene-docs.ts`
- Test: `src/client/core/src/scene-docs.test.ts` (add cases; create if absent)
- Modify: `src/client/core/src/index.ts` (export `buildActorDoc`, `ActorSystem`)

**Interfaces:**
- Produces:
  ```ts
  interface ActorVisual { kind: "image"; asset: string }
  interface ActorSystem {
    name: string;
    displayName: string;          // non-secret fallback (M10b name privacy)
    visual: ActorVisual;
    size: { w: number; h: number };   // grid units (M10d uses this)
    shape: "square" | "circle";
    faction: string | null;       // faction id (M10b); null for now
    conditions: string[];         // condition ids (M10c); [] for now
    prototype: boolean;           // true ⇒ instance on drop; false ⇒ link
  }
  function buildActorDoc(worldId: string, system: ActorSystem, id?: string): WireDocument
  ```

- [ ] **Step 1: Write the failing test**

Add to `src/client/core/src/scene-docs.test.ts`:
```ts
import { describe, it, expect } from "vitest";
import { buildActorDoc, type ActorSystem } from "./scene-docs";

const sys: ActorSystem = {
  name: "Goblin", displayName: "Goblin",
  visual: { kind: "image", asset: "a1" },
  size: { w: 1, h: 1 }, shape: "square",
  faction: null, conditions: [], prototype: true,
};

describe("buildActorDoc", () => {
  it("builds a world-scoped, parentless actor document", () => {
    const d = buildActorDoc("w1", sys, "act1");
    expect(d.doc_type).toBe("actor");
    expect(d.parent_id).toBeNull();
    expect(d.scope).toEqual({ kind: "world", world_id: "w1" });
    expect(d.system).toEqual(sys);
    expect(d.id).toBe("act1");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/core test scene-docs`
Expected: FAIL ("buildActorDoc is not a function").

- [ ] **Step 3: Implement**

Add to `src/client/core/src/scene-docs.ts` (reusing the existing private `envelope`):
```ts
/** An actor's appearance + defaults (M10a). Stats/sheet are M12; this is only what
 * backs a token. The server is structural-only — this `system` shape is client-owned. */
export interface ActorVisual { kind: "image"; asset: string }
export interface ActorSystem {
  name: string;
  displayName: string;
  visual: ActorVisual;
  size: { w: number; h: number };
  shape: "square" | "circle";
  faction: string | null;
  conditions: string[];
  prototype: boolean;
}

/** A top-level (world-scoped, parentless) actor document. */
export function buildActorDoc(worldId: string, system: ActorSystem, id?: string): WireDocument {
  return envelope(worldId, "actor", null, system, id);
}
```

- [ ] **Step 4: Export + run test to verify it passes**

Add to `src/client/core/src/index.ts` the exports for `buildActorDoc`, `ActorSystem`,
`ActorVisual` (follow the existing scene-docs re-export style).
Run: `pnpm --filter @shadowcat/core test scene-docs`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/client/core/src/scene-docs.ts src/client/core/src/scene-docs.test.ts src/client/core/src/index.ts
git commit -m "feat(m10a): actor document model (ActorSystem + buildActorDoc)"
```

---

## Task 3: Token linkage fields + `buildTokenFromActor`

**Files:**
- Modify: `src/client/core/src/scene-docs.ts` (extend `TokenSystem`; add builder)
- Test: `src/client/core/src/scene-docs.test.ts`
- Modify: `src/client/core/src/index.ts` (export `buildTokenFromActor`)

**Interfaces:**
- Consumes: `ActorSystem`, `buildActorDoc` (Task 2); `WireDocument` (`embedded`, `source`).
- Produces:
  ```ts
  // TokenSystem gains: actor_id?: string | null; overrides?: TokenOverrides; visual now optional
  interface TokenOverrides { name?: string; visual?: ActorVisual; size?: { w: number; h: number } }
  function buildTokenFromActor(
    worldId: string, sceneId: string, actor: WireDocument,
    mode: "link" | "instance", pos: { x: number; y: number }, cellSize: number, id?: string,
  ): WireDocument
  ```

- [ ] **Step 1: Write the failing test**

Add to `scene-docs.test.ts`:
```ts
import { buildTokenFromActor } from "./scene-docs";

const actor = buildActorDoc("w1", sys, "act1");

describe("buildTokenFromActor", () => {
  it("link mode references the actor by id, no embedded copy", () => {
    const t = buildTokenFromActor("w1", "scene1", actor, "link", { x: 50, y: 50 }, 100);
    expect(t.doc_type).toBe("token");
    expect(t.parent_id).toBe("scene1");
    expect((t.system as any).actor_id).toBe("act1");
    expect((t.system as any).overrides).toEqual({});
    expect(t.embedded.actor).toBeUndefined();
  });
  it("instance mode embeds an independent copy with provenance, no actor_id", () => {
    const t = buildTokenFromActor("w1", "scene1", actor, "instance", { x: 0, y: 0 }, 100);
    expect((t.system as any).actor_id ?? null).toBeNull();
    expect(t.embedded.actor).toHaveLength(1);
    const copy = t.embedded.actor[0];
    expect(copy.id).not.toBe(actor.id);                 // fresh id
    expect(copy.source).toEqual({ id: "act1", pack: null, version: 1 });
    expect(copy.system).toEqual(actor.system);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/core test scene-docs`
Expected: FAIL ("buildTokenFromActor is not a function").

- [ ] **Step 3: Implement**

In `scene-docs.ts`, change `TokenSystem` so `visual` is optional and add linkage fields:
```ts
export interface TokenOverrides { name?: string; visual?: ActorVisual; size?: { w: number; h: number } }
export interface TokenSystem {
  x: number;
  y: number;
  w: number;
  h: number;
  rotation: number;
  /** Set on raw (actorless) tokens; actor-backed tokens resolve visual via the actor. */
  visual?: { kind: "image"; asset: string };
  /** Linked token: the shared actor's id. */
  actor_id?: string | null;
  /** Linked-only per-token override whitelist (name/visual/size). */
  overrides?: TokenOverrides;
}
```
Add the builder:
```ts
/** Build a token from an actor. `link` references the shared actor; `instance` embeds an
 * independent copy with `source` provenance (the deferred merge engine consumes it).
 * Size/shape resolve from the actor (M10d); `w`/`h` seed the rendered cell size now. */
export function buildTokenFromActor(
  worldId: string,
  sceneId: string,
  actor: WireDocument,
  mode: "link" | "instance",
  pos: { x: number; y: number },
  cellSize: number,
  id?: string,
): WireDocument {
  const base: TokenSystem = { x: pos.x, y: pos.y, w: cellSize, h: cellSize, rotation: 0 };
  if (mode === "link") {
    const doc = envelope(worldId, "token", sceneId, { ...base, actor_id: actor.id, overrides: {} }, id);
    return doc;
  }
  const copy: WireDocument = { ...actor, id: crypto.randomUUID(), source: { id: actor.id, pack: null, version: 1 } };
  const doc = envelope(worldId, "token", sceneId, base, id);
  doc.embedded = { actor: [copy] };
  return doc;
}
```

- [ ] **Step 4: Export + run test to verify it passes**

Add `buildTokenFromActor`, `TokenOverrides` to `src/client/core/src/index.ts`.
Run: `pnpm --filter @shadowcat/core test scene-docs`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/client/core/src/scene-docs.ts src/client/core/src/scene-docs.test.ts src/client/core/src/index.ts
git commit -m "feat(m10a): token actor linkage fields + buildTokenFromActor"
```

---

## Task 4: `EffectiveActor` + `resolveTokenActor`

**Files:**
- Create: `src/client/core/src/actor.ts`
- Test: `src/client/core/src/actor.test.ts`
- Modify: `src/client/core/src/index.ts` (export)

**Interfaces:**
- Consumes: `ActorSystem`, `TokenOverrides` (Task 2/3); `WireDocument`; `ReadableDocuments`.
- Produces:
  ```ts
  interface EffectiveActor {
    name: string; displayName: string; visual: ActorVisual;
    size: { w: number; h: number }; shape: "square" | "circle";
    faction: string | null; conditions: string[];
  }
  function resolveTokenActor(token: WireDocument, store: ReadableDocuments): EffectiveActor | null
  ```

- [ ] **Step 1: Write the failing test**

Create `src/client/core/src/actor.test.ts`:
```ts
import { describe, it, expect } from "vitest";
import { DocumentStore } from "./store";
import { buildActorDoc, buildTokenFromActor, type ActorSystem } from "./scene-docs";
import { resolveTokenActor } from "./actor";

const sys: ActorSystem = {
  name: "Goblin", displayName: "Unknown",
  visual: { kind: "image", asset: "a1" },
  size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: true,
};

function storeWith(...docs: ReturnType<typeof buildActorDoc>[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world: "w1", author: "u1", ts: 0, ops: docs.map((doc) => ({ op: "create", doc })) } as any);
  return s;
}

describe("resolveTokenActor", () => {
  it("resolves a linked token from the shared actor", () => {
    const actor = buildActorDoc("w1", sys, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100);
    const eff = resolveTokenActor(token, storeWith(actor));
    expect(eff?.name).toBe("Goblin");
    expect(eff?.visual.asset).toBe("a1");
  });
  it("applies the per-token override whitelist over the linked actor", () => {
    const actor = buildActorDoc("w1", sys, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100);
    (token.system as any).overrides = { name: "Boss", visual: { kind: "image", asset: "a2" } };
    const eff = resolveTokenActor(token, storeWith(actor));
    expect(eff?.name).toBe("Boss");
    expect(eff?.visual.asset).toBe("a2");
  });
  it("resolves an instanced token from its embedded copy (store-independent)", () => {
    const actor = buildActorDoc("w1", sys, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "instance", { x: 0, y: 0 }, 100);
    const eff = resolveTokenActor(token, new DocumentStore()); // empty store
    expect(eff?.name).toBe("Goblin");
  });
  it("returns null for a linked token whose actor is missing, and for a raw token", () => {
    const actor = buildActorDoc("w1", sys, "act1");
    const linked = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100);
    expect(resolveTokenActor(linked, new DocumentStore())).toBeNull();
    const raw = { ...linked, system: { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "z" } }, embedded: {} };
    expect(resolveTokenActor(raw as any, new DocumentStore())).toBeNull();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/core test actor`
Expected: FAIL ("Cannot find module './actor'").

- [ ] **Step 3: Implement**

Create `src/client/core/src/actor.ts`:
```ts
// Resolves a token to its EffectiveActor: the single read-through every token-decoration
// consumer (render visual, faction border [M10b], conditions [M10c], displayName) uses.
// Linked tokens read the shared actor + apply the override whitelist; instanced tokens
// read their embedded copy. Returns null for a raw (actorless) or dangling-link token.
import type { WireDocument } from "./wire";
import type { ReadableDocuments } from "./store";
import type { ActorSystem, ActorVisual, TokenOverrides } from "./scene-docs";

export interface EffectiveActor {
  name: string;
  displayName: string;
  visual: ActorVisual;
  size: { w: number; h: number };
  shape: "square" | "circle";
  faction: string | null;
  conditions: string[];
}

function project(base: ActorSystem, overrides?: TokenOverrides): EffectiveActor {
  return {
    name: overrides?.name ?? base.name,
    displayName: base.displayName,
    visual: overrides?.visual ?? base.visual,
    size: overrides?.size ?? base.size,
    shape: base.shape,
    faction: base.faction,
    conditions: base.conditions,
  };
}

export function resolveTokenActor(token: WireDocument, store: ReadableDocuments): EffectiveActor | null {
  const sys = token.system as { actor_id?: string | null; overrides?: TokenOverrides } | undefined;
  if (sys?.actor_id) {
    const actor = store.get(sys.actor_id);
    if (!actor) return null;
    return project(actor.system as ActorSystem, sys.overrides);
  }
  const embedded = token.embedded?.actor?.[0];
  if (embedded) return project(embedded.system as ActorSystem);
  return null;
}
```

- [ ] **Step 4: Export + run test to verify it passes**

Add to `src/client/core/src/index.ts`: `export { resolveTokenActor, type EffectiveActor } from "./actor";`
Run: `pnpm --filter @shadowcat/core test actor`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/client/core/src/actor.ts src/client/core/src/actor.test.ts src/client/core/src/index.ts
git commit -m "feat(m10a): resolveTokenActor -> EffectiveActor read-through"
```

---

## Task 5: TokenView resolves visual via EffectiveActor

**Files:**
- Modify: `src/client/render/src/token-view.ts`
- Test: `src/client/render/src/token-view.test.ts`

**Interfaces:**
- Consumes: `resolveTokenActor` (Task 4).
- Produces: TokenView renders an actor-linked token using the actor's (or override's)
  visual; raw tokens unchanged.

- [ ] **Step 1: Write the failing test**

Add to `src/client/render/src/token-view.test.ts` a case where a linked token (no
`system.visual`, `system.actor_id` set) and its actor doc are in the store; assert the
backend `setToken` spec's `url` resolves the **actor's** asset. (Mirror the existing
raw-token test's store/AssetResolver/MockBackend setup; add the actor doc to the store and
build the token with `actor_id`.)
```ts
it("renders a linked token using the actor's visual", () => {
  // store has actor "act1" (visual asset "a1") + a token with actor_id "act1", no system.visual
  // ...existing harness setup...
  view.reconcile();
  expect(backend.tokens.get("tok1")?.url).toBe(assets.url("a1"));
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/render test token-view`
Expected: FAIL (toSpec returns null for a token without `system.visual`).

- [ ] **Step 3: Implement**

In `src/client/render/src/token-view.ts`, import `resolveTokenActor` from `@shadowcat/core`
and rewrite `toSpec` to resolve the visual via the actor first, falling back to the raw
`system.visual`:
```ts
import { resolveTokenActor, type ReadableDocuments, type AssetResolver, type WireDocument } from "@shadowcat/core";
// ...
  private toSpec(doc: WireDocument): TokenNodeSpec | null {
    const s = doc.system as { x: number; y: number; w: number; h: number; rotation?: number; visual?: { kind: "image"; asset: string } } | undefined;
    if (!s) return null;
    const eff = resolveTokenActor(doc, this.store);
    const visual = eff?.visual ?? s.visual;
    if (visual?.kind !== "image") return null; // nothing renderable
    return {
      x: s.x, y: s.y, w: s.w, h: s.h, rotation: s.rotation ?? 0,
      url: this.assets.url(visual.asset),
    };
  }
```
(Update the local `TokenSystem` interface in this file — remove the hard `visual` requirement — or delete it in favor of the inline type above.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/render test token-view`
Expected: PASS (the new case + the existing raw-token cases).

- [ ] **Step 5: Commit**

```bash
git add src/client/render/src/token-view.ts src/client/render/src/token-view.test.ts
git commit -m "feat(m10a): TokenView resolves visual via EffectiveActor"
```

---

## Task 6: `ActorSelection` seam (AppContext + wiring)

**Files:**
- Create: `src/client/ui-kit/src/actorSelection.svelte.ts`
- Modify: `src/client/ui-kit/src/index.ts` (export), `src/client/ui-kit/src/appContext.ts`
  (add field)
- Modify: `src/client/shell/src/lib/worldSession.svelte.ts` (own a stable instance),
  `src/client/shell/src/lib/Table.svelte` (pass it into the AppContext literal)
- Modify: test fixtures that build an `AppContext`:
  `src/client/ui-kit/src/__fixtures__/SurfaceHarness.svelte`,
  `src/client/ui-kit/src/__fixtures__/appContextTest.ts`,
  `src/modules/assets/src/__fixtures__/AssetsHarness.svelte`
- Modify: `src/modules/scene-tools/src/controller.svelte.ts` (`ToolContext` gains optional
  field), `src/modules/scene-tools/src/ToolRail.svelte` (pass it through)
- Test: `src/client/ui-kit/src/actorSelection.test.ts`

**Interfaces:**
- Produces:
  ```ts
  // ui-kit
  class ActorSelection { readonly selectedId: string | null; select(id: string | null): void }
  // AppContext gains:  actorSelection: ActorSelection   (required)
  // ToolContext gains: actorSelection?: ActorSelection  (optional — tool tests unaffected)
  ```

- [ ] **Step 1: Write the failing test**

Create `src/client/ui-kit/src/actorSelection.test.ts`:
```ts
import { describe, it, expect } from "vitest";
import { ActorSelection } from "./actorSelection.svelte";

describe("ActorSelection", () => {
  it("holds and updates the selected actor id (stable instance)", () => {
    const sel = new ActorSelection();
    expect(sel.selectedId).toBeNull();
    sel.select("act1");
    expect(sel.selectedId).toBe("act1");
    sel.select(null);
    expect(sel.selectedId).toBeNull();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/ui-kit test actorSelection`
Expected: FAIL (module missing).

- [ ] **Step 3: Implement the stable selection holder**

Create `src/client/ui-kit/src/actorSelection.svelte.ts`:
```ts
// The actor the place tool will stamp. A stable instance held by WorldSession and shared
// via AppContext (module-actors sets it; scene-tools reads it). Mutated in place — never
// reassigned — so the AppContext-captured reference stays valid (the stable-ref rule).
export class ActorSelection {
  #id = $state<string | null>(null);
  get selectedId(): string | null { return this.#id; }
  select(id: string | null): void { this.#id = id; }
}
```

- [ ] **Step 4: Wire it through types + construction**

`src/client/ui-kit/src/index.ts`: `export { ActorSelection } from "./actorSelection.svelte";`
`src/client/ui-kit/src/appContext.ts` — import and add to `AppContext`:
```ts
import type { ActorSelection } from "./actorSelection.svelte";
// ...inside AppContext:
  /** The actor the place tool stamps; set by module-actors, read by scene-tools. */
  actorSelection: ActorSelection;
```
`src/client/shell/src/lib/worldSession.svelte.ts` — add a stable field (near
`sceneInteraction`):
```ts
import { SceneInteractionBridge, ActorSelection } from "@shadowcat/ui-kit";
// ...
  readonly actorSelection = new ActorSelection();
```
`src/client/shell/src/lib/Table.svelte` — add `actorSelection: session.actorSelection` to
the `setAppContext({ ... })` literal (use the session variable the file already references).
Add `actorSelection: <new ActorSelection()>` (import from `@shadowcat/ui-kit`) to the three
test fixtures listed in **Files** so they satisfy the interface.

`src/modules/scene-tools/src/controller.svelte.ts` — add to `ToolContext`:
```ts
import type { SceneInteraction, ActorSelection } from "@shadowcat/ui-kit";
// ...inside ToolContext:
  /** The actor to stamp (the place tool); when set it takes precedence over selectedAsset. */
  actorSelection?: ActorSelection;
```
`src/modules/scene-tools/src/ToolRail.svelte` — add `actorSelection: ctx.actorSelection`
to the `new ToolController({ ... })` literal (where `ctx` is the AppContext).

- [ ] **Step 5: Run typecheck + tests to verify pass**

Run: `pnpm -r typecheck && pnpm --filter @shadowcat/ui-kit test actorSelection`
Expected: PASS (all AppContext literals now satisfy the interface).

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(m10a): ActorSelection seam on AppContext + ToolContext"
```

---

## Task 7: Place tool stamps the selected actor

**Files:**
- Modify: `src/modules/scene-tools/src/controller.svelte.ts` (`makePlaceTool`)
- Test: `src/modules/scene-tools/src/place-tool.test.ts`

**Interfaces:**
- Consumes: `buildTokenFromActor` (Task 3), `ActorSelection` via `ToolContext` (Task 6).
- Produces: place tool creates an actor-backed token (link/instance per the actor's
  `prototype`) when an actor is selected; otherwise the existing `selectedAsset` path.

- [ ] **Step 1: Write the failing test**

Add to `src/modules/scene-tools/src/place-tool.test.ts` (reuse `ctxWith`): seed the store
with a scene and an actor (`prototype: true`), set `ctx.actorSelection.select(actor.id)`,
fire `onPointerDown`, and assert the dispatched op is a token `create` whose
`embedded.actor[0].source.id` equals the actor id (instanced). Add a second case with a
`prototype: false` actor asserting `system.actor_id === actor.id` and no embedded copy.
```ts
it("stamps the selected actor as an instanced token", () => {
  const { ctx, sent } = ctxWith(storeWithSceneAndActor(/* prototype: true */));
  ctx.actorSelection!.select("act1");
  const controller = new ToolController(ctx);
  makePlaceTool(ctx, controller).onPointerDown({ x: 50, y: 50 }, {} as PointerEvent);
  const doc = (sent[0][0] as any).doc;
  expect(doc.doc_type).toBe("token");
  expect(doc.embedded.actor[0].source.id).toBe("act1");
});
```
(Provide a `storeWithSceneAndActor` helper that applies a `create` command for a `scene`
doc and a `buildActorDoc` with the desired `prototype`. The existing place-tool test file
already shows the store-seeding pattern.)

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/module-scene-tools test place-tool`
Expected: FAIL (place tool ignores `actorSelection`).

- [ ] **Step 3: Implement**

Rewrite `makePlaceTool` in `controller.svelte.ts`:
```ts
import { buildTokenDoc, buildTokenFromActor, /* ...existing... */ } from "@shadowcat/core";
// ...
export function makePlaceTool(ctx: ToolContext, controller: ToolController): SceneTool {
  return {
    onPointerDown(p: Point): boolean {
      const scene = activeScene(ctx);
      if (!scene) return false;
      const c = ctx.scene.snap(p);
      const actorId = ctx.actorSelection?.selectedId ?? null;
      if (actorId) {
        const actor = ctx.documents.get(actorId);
        if (!actor) return false;
        const mode = (actor.system as { prototype?: boolean })?.prototype ? "instance" : "link";
        ctx.dispatchIntent([{ op: "create", doc: buildTokenFromActor(ctx.world, scene.id, actor, mode, c, scene.size) }]);
        return true;
      }
      const asset = controller.selectedAsset;
      if (!asset) return false;
      ctx.dispatchIntent([
        { op: "create", doc: buildTokenDoc(ctx.world, scene.id, { x: c.x, y: c.y, w: scene.size, h: scene.size, rotation: 0, visual: { kind: "image", asset } }) },
      ]);
      return true;
    },
    onPointerMove(): void {},
    onPointerUp(): void {},
  };
}
```

- [ ] **Step 4: Run tests to verify pass**

Run: `pnpm --filter @shadowcat/module-scene-tools test place-tool`
Expected: PASS (new actor cases + existing asset case).

- [ ] **Step 5: Commit**

```bash
git add src/modules/scene-tools/src/controller.svelte.ts src/modules/scene-tools/src/place-tool.test.ts
git commit -m "feat(m10a): place tool stamps the selected actor (link/instance)"
```

---

## Task 8: `module-actors` package + shell registration

**Files:**
- Create: `src/modules/actors/package.json`
- Create: `src/modules/actors/src/index.ts`
- Create: `src/modules/actors/src/ActorsPanel.svelte`
- Test: `src/modules/actors/src/index.test.ts`
- Modify: `src/client/shell/src/App.svelte` (import + add to the `modules` array)

**Interfaces:**
- Consumes: `buildActorDoc`, `listAssets`, `uploadAsset` (`@shadowcat/core`); `getAppContext`,
  `ActorSelection` (`@shadowcat/ui-kit`).
- Produces: `export const actors: Module` contributing `ActorsPanel` into
  `shadowcat.surface:sidebar` (order 2).

- [ ] **Step 1: Write the failing test**

Create `src/modules/actors/src/index.test.ts`:
```ts
import { describe, it, expect } from "vitest";
import { ContributionRegistry } from "@shadowcat/core";
import { actors } from "./index";

describe("actors module", () => {
  it("contributes a sidebar panel", () => {
    expect(actors.manifest.id).toBe("actors");
    expect(actors.manifest.requires).toContain("shadowcat.surface:sidebar");
    const contributions = new ContributionRegistry();
    actors.register({ contributions } as any);
    expect(contributions.contributionsFor("shadowcat.surface:sidebar").length).toBe(1);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/module-actors test`
Expected: FAIL (package/module missing).

- [ ] **Step 3: Create the package**

`src/modules/actors/package.json` (mirror `src/modules/statusbar/package.json`):
```json
{
  "name": "@shadowcat/module-actors",
  "version": "0.0.0",
  "private": true,
  "type": "module",
  "main": "src/index.ts",
  "scripts": { "typecheck": "svelte-check --tsconfig ./tsconfig.json", "test": "vitest run" },
  "dependencies": { "@shadowcat/core": "workspace:*", "@shadowcat/ui-kit": "workspace:*" }
}
```
(Copy `tsconfig.json` + any `vitest`/svelte config from `src/modules/statusbar` so the
package builds/tests identically. Run `pnpm install` to link the new workspace package.)

`src/modules/actors/src/index.ts`:
```ts
import type { Module } from "@shadowcat/core";
import ActorsPanel from "./ActorsPanel.svelte";

/** Actor create/list/pick panel. Requires core-ui's sidebar region. */
export const actors: Module = {
  manifest: {
    id: "actors",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: ["shadowcat.surface:sidebar"],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({ id: "actors:sidebar", contract: "shadowcat.surface:sidebar", order: 2, component: ActorsPanel });
  },
};
```

- [ ] **Step 4: Implement the panel**

`src/modules/actors/src/ActorsPanel.svelte` (list actors, a create form using the asset
list, and click-to-select). Mirror `src/modules/assets/src/Assets.svelte` for the
`getAppContext` + asset-list pattern:
```svelte
<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import { buildActorDoc, listAssets, type ActorSystem } from "@shadowcat/core";

  const ctx = getAppContext();
  let name = $state("");
  let displayName = $state("");
  let assetId = $state<string | null>(null);
  let prototype = $state(true);

  const actors = $derived(ctx.documents.query("actor"));
  let assetList = $state<{ id: string; original_name: string; content_type: string }[]>([]);
  $effect(() => { void listAssets(ctx.world).then((a) => (assetList = a.filter((x) => x.content_type.startsWith("image/")))); });

  function create(): void {
    if (!name || !assetId) return;
    const system: ActorSystem = {
      name, displayName: displayName || name,
      visual: { kind: "image", asset: assetId },
      size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype,
    };
    ctx.dispatchIntent([{ op: "create", doc: buildActorDoc(ctx.world, system) }]);
    name = ""; displayName = ""; assetId = null;
  }
</script>

<section class="actors">
  <h3>{ctx.t("actors.title")}</h3>
  <ul>
    {#each actors as a (a.id)}
      <li>
        <button
          class:selected={ctx.actorSelection.selectedId === a.id}
          onclick={() => ctx.actorSelection.select(a.id)}
        >{(a.system as { name?: string }).name ?? a.id}</button>
      </li>
    {/each}
  </ul>
  <form onsubmit={(e) => { e.preventDefault(); create(); }}>
    <input placeholder={ctx.t("actors.name")} bind:value={name} />
    <input placeholder={ctx.t("actors.displayName")} bind:value={displayName} />
    <label><input type="checkbox" bind:checked={prototype} /> {ctx.t("actors.instanceOnDrop")}</label>
    <div class="picker">
      {#each assetList as a (a.id)}
        <button type="button" class:selected={assetId === a.id} onclick={() => (assetId = a.id)}>
          <img src={ctx.assets.url(a.id)} alt={a.original_name} />
        </button>
      {/each}
    </div>
    <button type="submit" disabled={!name || !assetId}>{ctx.t("actors.create")}</button>
  </form>
</section>
```
Add the `actors.*` i18n keys to the `en` catalog (follow how `src/modules/assets` registers
its strings; if modules ship their own catalog fragments, add one here).

- [ ] **Step 5: Register in the shell + run tests**

`src/client/shell/src/App.svelte`: add `import { actors } from "@shadowcat/module-actors";`
and append `actors` to the modules array (line ~81):
```ts
modules: [coreUi, topBar, statusBar, stage, settings, assets, actors, sceneTools]
```
Add `@shadowcat/module-actors` to `src/client/shell/package.json` dependencies; run
`pnpm install`.
Run: `pnpm --filter @shadowcat/module-actors test && pnpm -r typecheck`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(m10a): module-actors (create/list/pick) + shell registration"
```

---

## Task 9: Full-suite verification

- [ ] **Step 1: Run the whole client suite + typecheck + lint**

Run: `pnpm -r test && pnpm -r typecheck && pnpm lint`
Expected: PASS.

- [ ] **Step 2: Run the server suite (rename + ts-rs sync)**

Run: `cd src/server && cargo test && cargo fmt --check && cargo clippy -- -D warnings`
Expected: PASS; `git status` shows no uncommitted regenerated `src/types/generated/*`.

- [ ] **Step 3: Manual smoke (optional, GM flow)**

Build the client (`pnpm --filter @shadowcat/ui build`), run the server binary, enter a
world as GM: the Actors panel lists/creates an actor (upload an image), selecting it +
the place tool stamps a token rendering the actor's art; editing the actor's art updates a
*linked* token's art (instanced stays fixed).

---

## Self-review (completed)

- **Spec coverage (§4, §12 M10a):** Actor doc (Task 2), link/instance + overrides +
  provenance (Task 3), `resolveTokenActor` (Task 4), render via actor (Task 5),
  `module-actors` create/list/pick (Task 8), place-tool wiring (Tasks 6–7), `user` rename
  (Task 1). Size/shape *resolution into rendering* + footprint are explicitly M10d; M10a
  resolves visual + name only.
- **Type consistency:** `ActorSystem`/`ActorVisual`/`TokenOverrides` defined in Task 2/3,
  consumed unchanged in Tasks 4/5/7/8; `ActorSelection` defined in Task 6, consumed in
  Tasks 6–8; `buildTokenFromActor` signature identical across Tasks 3/7.
- **Placeholder scan:** none — each step carries real code or an exact command.
