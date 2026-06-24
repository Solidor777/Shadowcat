# M10c — Conditions (status markers) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add world-configurable status **conditions** (markers only — no mechanical effects) that render as small emoji badges on tokens, seeded by a replaceable first-party `module-conditions`, with GM-or-owner toggling gated by the capability model.

**Architecture:** Mirror M10b's factions split exactly — the engine owns the *mechanism* (a world-scoped `condition-registry` config-document of `{id → {name, icon}}`, an actor-data `conditions: string[]` field already present, and badge rendering), while a replaceable first-party `module-conditions` owns the *content* (seeds a generic emoji set + a GM editor + a selection-driven toggle palette). Toggling writes the resolved actor data: linked tokens write the shared actor doc's `/system/conditions`; instanced tokens write the embedded copy's `/embedded/actor/0/system/conditions`. A new advisory `AppContext.canEdit(doc, path)` (mirroring the server's Update-path check via the existing `canWritePath` capability mirror) gates the toggle so a token's owner — not just the GM — can toggle their own.

**Tech Stack:** Rust (server stays structural-only — no server changes), Svelte 5 Runes, TypeScript, PixiJS v8 (Text badges), Vitest, SCSS.

## Global Constraints

- **Server stays structural-only (ARCHITECTURE §2 invariant 6).** Conditions live in the opaque `system`/`embedded` body. No server code changes — the registry is an ordinary document; toggles are ordinary field Updates honored by the existing apply path.
- **Registries are config-documents** (world-scoped, parentless, runtime-editable) keyed by id as a **MAP not an array** — adding an entry is a single-key Update (`set_pointer` cannot grow arrays). [actors-tokens skill; M10b]
- **Cross-platform from day one (CI-verified).** Emoji badge **glyph art** varies by OS font; this is an accepted cosmetic difference. The data model is `icon: string` — any glyph renders via one Pixi `Text` path, no per-OS code.
- **Optimistic with rollback (#1/#5).** Toggles dispatch through `ctx.dispatchIntent`; documents are source of truth, badges reconcile from the optimistic store.
- **Fail-closed advisory gate.** `canEdit` is **advisory UX only** — the server remains authoritative and rejects a bypass at `apply_intent`. GM bypasses all checks.
- **TDD, DRY, YAGNI, frequent commits.** Each task ends green: `pnpm -r test`, `pnpm -r typecheck`, `pnpm lint` (and `cargo fmt/clippy/test` unaffected — no Rust touched).

---

## File Structure

**Core (`@shadowcat/core`):**
- Modify `src/client/core/src/scene-docs.ts` — `Condition`, `ConditionRegistrySystem`, `buildConditionRegistryDoc` (mirror `Faction`).
- Modify `src/client/core/src/actor.ts` — `resolveConditions(token, store)`, `conditionTarget(token, store)` (write-target resolution: actor doc vs embedded copy).
- Modify `src/client/core/src/index.ts` — re-export the new types/functions.

**Render (`@shadowcat/render`):**
- Modify `src/client/render/src/types.ts` — add `badges: string[]` to `TokenNodeSpec`.
- Modify `src/client/render/src/token-view.ts` — resolve condition icon glyphs into `spec.badges`.
- Modify `src/client/render/src/pixi-backend.ts` — render badges as upright `Text` chips tracking the token; clean up on remove.
- Modify `src/client/render/src/backend.mock.test.ts`, `src/client/render/src/token-view.test.ts` — `badges` in existing spec literals + new coverage.

**UI-kit (`@shadowcat/ui-kit`):**
- Modify `src/client/ui-kit/src/appContext.ts` — add `selfId: string` + `canEdit(doc, path): boolean`.
- Modify `src/client/ui-kit/src/__fixtures__/appContextTest.ts` + `src/client/ui-kit/src/__fixtures__/SurfaceHarness.svelte` — seed the new fields.
- Modify `src/client/ui-kit/src/locales/en.ts` — `conditions.*` strings.

**Shell (`@shadowcat/shell`):**
- Modify `src/client/shell/src/lib/worldSession.svelte.ts` — store Welcome grants/requirements; expose `selfId` + `canEdit`.
- Modify `src/client/shell/src/lib/Table.svelte` — pass `selfId` + `canEdit` into AppContext.
- Modify `src/client/shell/src/App.svelte` — register the `conditions` module.

**New module (`@shadowcat/module-conditions`):**
- Create `src/modules/conditions/{package.json, tsconfig.json, svelte.config.js, vitest.config.ts, vitest.setup.ts}`.
- Create `src/modules/conditions/src/{index.ts, ConditionsPanel.svelte, index.test.ts, ConditionsPanel.test.ts}`.

**Docs / skills (completion gate):**
- Modify `docs/PLAN.md` — mark M10c complete.
- Modify `.claude/skills/shadowcat-codebase-actors-tokens/SKILL.md` — note the condition registry seam (reviewed skill-update gate).

---

### Task 1: Core — condition registry model + builder

**Files:**
- Modify: `src/client/core/src/scene-docs.ts` (append after the faction-registry block, ~line 151)
- Modify: `src/client/core/src/index.ts` (re-export)
- Test: `src/client/core/src/scene-docs.test.ts` (create if absent, else append)

**Interfaces:**
- Consumes: `envelope(worldId, docType, parentId, system, id?)` (existing private helper), `WireDocument`.
- Produces:
  - `interface Condition { name: string; icon: string }`
  - `interface ConditionRegistrySystem { conditions: Record<string, Condition> }`
  - `function buildConditionRegistryDoc(worldId: string, conditions: Record<string, Condition>, id?: string): WireDocument` — doc_type `"condition-registry"`, world-scoped, parentless.

- [ ] **Step 1: Write the failing test**

In `src/client/core/src/scene-docs.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { buildConditionRegistryDoc, type Condition } from "./scene-docs";

describe("buildConditionRegistryDoc", () => {
  it("builds a world-scoped, parentless condition-registry config document", () => {
    const conditions: Record<string, Condition> = { dead: { name: "Dead", icon: "💀" } };
    const doc = buildConditionRegistryDoc("w1", conditions);
    expect(doc.doc_type).toBe("condition-registry");
    expect(doc.parent_id).toBeNull();
    expect(doc.scope).toEqual({ kind: "world", world_id: "w1" });
    expect((doc.system as { conditions: Record<string, Condition> }).conditions).toEqual(conditions);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/core test -- scene-docs`
Expected: FAIL — `buildConditionRegistryDoc` is not exported.

- [ ] **Step 3: Write minimal implementation**

In `src/client/core/src/scene-docs.ts`, append after `buildFactionRegistryDoc`:

```ts
/** A status condition's display. `icon` is a short glyph (emoji) rendered as a token badge. */
export interface Condition {
  name: string;
  icon: string;
}

/** The world's condition registry: a singleton config document (doc_type "condition-registry").
 * `conditions` is keyed by condition id — an actor's `conditions` array holds keys. A MAP, not an
 * array, so adding a condition is a single-key Update (`set_pointer` cannot grow arrays). */
export interface ConditionRegistrySystem {
  conditions: Record<string, Condition>;
}

/** A top-level (world-scoped, parentless) condition-registry document. */
export function buildConditionRegistryDoc(worldId: string, conditions: Record<string, Condition>, id?: string): WireDocument {
  return envelope(worldId, "condition-registry", null, { conditions } satisfies ConditionRegistrySystem, id);
}
```

In `src/client/core/src/index.ts`, add to the existing `scene-docs` re-export block the names: `Condition`, `ConditionRegistrySystem`, `buildConditionRegistryDoc` (match the existing export style for `Faction`/`buildFactionRegistryDoc`).

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/core test -- scene-docs`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/client/core/src/scene-docs.ts src/client/core/src/index.ts src/client/core/src/scene-docs.test.ts
git commit -m "feat(m10c): condition-registry config-document model + builder"
```

---

### Task 2: Core — condition resolution + write-target helper

**Files:**
- Modify: `src/client/core/src/actor.ts`
- Modify: `src/client/core/src/index.ts` (re-export `resolveConditions`, `conditionTarget`, `ConditionTarget`)
- Test: `src/client/core/src/actor.test.ts` (append; create if absent)

**Interfaces:**
- Consumes: `resolveTokenActor(token, store)`, `EffectiveActor`, `ActorSystem`, `ConditionRegistrySystem`, `ReadableDocuments`, `WireDocument`.
- Produces:
  - `function resolveConditions(token: WireDocument, store: ReadableDocuments): { id: string; name: string; icon: string }[]` — the token's effective conditions resolved through the world registry; ids absent from the registry are dropped (fail-closed).
  - `interface ConditionTarget { doc: WireDocument; path: string; conditions: string[] }`
  - `function conditionTarget(token: WireDocument, store: ReadableDocuments): ConditionTarget | null` — where a token's conditions live + the current set. Linked → the shared actor doc, `/system/conditions`. Instanced → the token doc, `/embedded/actor/0/system/conditions`. `null` for a raw/dangling token.

- [ ] **Step 1: Write the failing test**

In `src/client/core/src/actor.test.ts`, append:

```ts
import { describe, it, expect } from "vitest";
import { DocumentStore } from "./store";
import { buildActorDoc, buildTokenFromActor, buildConditionRegistryDoc } from "./scene-docs";
import { resolveConditions, conditionTarget } from "./actor";
import type { ActorSystem } from "./scene-docs";

function actorSys(over: Partial<ActorSystem> = {}): ActorSystem {
  return { name: "Goblin", displayName: "Goblin", visual: { kind: "image", asset: "a1" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: true, ...over };
}

describe("resolveConditions", () => {
  it("resolves effective condition ids through the world registry, dropping unknown ids", () => {
    const store = new DocumentStore();
    const actor = buildActorDoc("w1", actorSys({ conditions: ["dead", "ghost"] }));
    store.upsert(actor);
    store.upsert(buildConditionRegistryDoc("w1", { dead: { name: "Dead", icon: "💀" } }));
    const token = buildTokenFromActor("w1", "s1", actor, "link", { x: 0, y: 0 }, 100);
    expect(resolveConditions(token, store)).toEqual([{ id: "dead", name: "Dead", icon: "💀" }]);
  });
});

describe("conditionTarget", () => {
  it("targets the shared actor doc for a linked token", () => {
    const store = new DocumentStore();
    const actor = buildActorDoc("w1", actorSys({ conditions: ["dead"] }));
    store.upsert(actor);
    const token = buildTokenFromActor("w1", "s1", actor, "link", { x: 0, y: 0 }, 100);
    const tgt = conditionTarget(token, store)!;
    expect(tgt.doc.id).toBe(actor.id);
    expect(tgt.path).toBe("/system/conditions");
    expect(tgt.conditions).toEqual(["dead"]);
  });

  it("targets the embedded copy for an instanced token", () => {
    const store = new DocumentStore();
    const actor = buildActorDoc("w1", actorSys({ conditions: ["dead"] }));
    const token = buildTokenFromActor("w1", "s1", actor, "instance", { x: 0, y: 0 }, 100);
    store.upsert(token);
    const tgt = conditionTarget(token, store)!;
    expect(tgt.doc.id).toBe(token.id);
    expect(tgt.path).toBe("/embedded/actor/0/system/conditions");
    expect(tgt.conditions).toEqual(["dead"]);
  });
});
```

> Note: verify the `DocumentStore` write method name (`upsert`/`set`/`applyCommand`) against the existing `actor.test.ts` / `store.ts` and match it; the surrounding tests in this file already use the correct one.

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/core test -- actor`
Expected: FAIL — `resolveConditions` / `conditionTarget` not exported.

- [ ] **Step 3: Write minimal implementation**

In `src/client/core/src/actor.ts`, add the import and functions:

```ts
import type { ActorSystem, ActorVisual, TokenOverrides, ConditionRegistrySystem } from "./scene-docs";
```

```ts
/** Resolve a token's effective conditions to display entries (id preserved for keying), via the
 * world registry. Ids absent from the registry are dropped — a stale/garbled id yields no badge,
 * never a render error (fail-closed). The single read-through every condition consumer uses. */
export function resolveConditions(token: WireDocument, store: ReadableDocuments): { id: string; name: string; icon: string }[] {
  const eff = resolveTokenActor(token, store);
  if (!eff) return [];
  const reg = store.query("condition-registry")[0]?.system as ConditionRegistrySystem | undefined;
  const map = reg?.conditions ?? {};
  const out: { id: string; name: string; icon: string }[] = [];
  for (const id of eff.conditions) {
    const c = map[id];
    if (c) out.push({ id, name: c.name, icon: c.icon });
  }
  return out;
}

/** Where a token's conditions live + the current set. Linked tokens write the shared actor doc's
 * `/system/conditions`; instanced tokens write the embedded copy at
 * `/embedded/actor/0/system/conditions`. Returns null for a raw/dangling token. The caller gates
 * the write via `AppContext.canEdit(doc, path)` (the embedded path requires `core:manage_embedded`,
 * the actor path `core:write_fields` — the capability model decides owner eligibility per mode). */
export interface ConditionTarget {
  doc: WireDocument;
  path: string;
  conditions: string[];
}

export function conditionTarget(token: WireDocument, store: ReadableDocuments): ConditionTarget | null {
  const sys = token.system as { actor_id?: string | null } | undefined;
  if (sys?.actor_id) {
    const actor = store.get(sys.actor_id);
    if (!actor) return null;
    return { doc: actor, path: "/system/conditions", conditions: (actor.system as ActorSystem).conditions ?? [] };
  }
  const embedded = token.embedded?.actor?.[0];
  if (embedded) {
    return { doc: token, path: "/embedded/actor/0/system/conditions", conditions: (embedded.system as ActorSystem).conditions ?? [] };
  }
  return null;
}
```

In `src/client/core/src/index.ts`, add `resolveConditions`, `conditionTarget`, and the type `ConditionTarget` to the `actor` re-export (match the existing `resolveTokenActor` / `EffectiveActor` export style).

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/core test -- actor`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/client/core/src/actor.ts src/client/core/src/index.ts src/client/core/src/actor.test.ts
git commit -m "feat(m10c): resolveConditions + conditionTarget (linked vs instanced write site)"
```

---

### Task 3: Render — condition badges on tokens

**Files:**
- Modify: `src/client/render/src/types.ts:49-58` (`TokenNodeSpec`)
- Modify: `src/client/render/src/token-view.ts`
- Modify: `src/client/render/src/pixi-backend.ts` (`setToken` ~140-180, `removeToken` ~182-194, field map ~31)
- Test: `src/client/render/src/token-view.test.ts`, `src/client/render/src/backend.mock.test.ts`

**Interfaces:**
- Consumes: `resolveConditions(doc, store)` (Task 2), existing `TokenNodeSpec`, Pixi `Text` (already imported in pixi-backend).
- Produces: `TokenNodeSpec.badges: string[]` — emoji glyphs, rendered as upright chips along the token's top edge.

- [ ] **Step 1: Write the failing test**

In `src/client/render/src/token-view.test.ts`, append (reuse the file's existing store/asset/backend harness — match its setup helpers):

```ts
it("resolves condition icons into token badges via the registry", () => {
  // Arrange: an actor with a condition + a condition-registry, a linked token.
  // (Mirror this file's existing actor+token+store setup.)
  // ...build `store`, `actor` with system.conditions = ["dead"], registry { dead: { name, icon: "💀" } },
  //    a linked token `tok1`, then construct TokenView and reconcile().
  expect(backend.tokens.get("tok1")!.badges).toEqual(["💀"]);
});

it("emits no badges for a token whose actor has no conditions", () => {
  // ... linked token, actor.system.conditions = []
  expect(backend.tokens.get("tok2")!.badges).toEqual([]);
});
```

Also update the existing exact-equality assertions in this file (the `toEqual({ x, y, w, h, rotation, url, borderColor })` literals, e.g. ~line 41) to include `badges: []`.

In `src/client/render/src/backend.mock.test.ts`, update the three spec literals (lines 6–8) to include `badges: []`.

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/render test`
Expected: FAIL — `badges` missing on `TokenNodeSpec` (type error) and the new assertions fail.

- [ ] **Step 3: Write minimal implementation**

In `src/client/render/src/types.ts`, add to `TokenNodeSpec`:

```ts
  /** Condition marker glyphs (emoji), rendered as upright chips along the token's top edge. */
  badges: string[];
```

In `src/client/render/src/token-view.ts`:

```ts
import { resolveTokenActor, resolveConditions } from "@shadowcat/core";
```

In `toSpec`, after computing `borderColor`, before the `return`:

```ts
    const badges = resolveConditions(doc, this.store).map((c) => c.icon);
```

and add `badges` to the returned object:

```ts
    return {
      x: s.x, y: s.y, w: s.w, h: s.h, rotation: s.rotation ?? 0,
      url: this.assets.url(visual.asset),
      borderColor,
      badges,
    };
```

In `src/client/render/src/pixi-backend.ts`, add the field (after `tokenBorders`, ~line 31):

```ts
  /** Condition badge glyph nodes per token (upright; absent when the token has no conditions). */
  private readonly tokenBadges = new Map<string, Text[]>();
```

In `setToken`, after the faction-border block (after ~line 171, before the URL load):

```ts
    // Condition badges: upright glyph chips along the token's top edge, tracking its position
    // (not rotation — status markers stay upright). Rebuilt each push; cheap for a few glyphs.
    const prevBadges = this.tokenBadges.get(id);
    if (prevBadges) for (const b of prevBadges) b.destroy();
    if (spec.badges.length === 0) {
      this.tokenBadges.delete(id);
    } else {
      const size = Math.max(12, Math.min(spec.w, spec.h) * 0.28);
      const nodes: Text[] = [];
      spec.badges.forEach((glyph, i) => {
        const txt = new Text({ text: glyph, style: { fontSize: size, fontFamily: "sans-serif" } });
        txt.anchor.set(0.5);
        txt.position.set(spec.x - spec.w / 2 + size / 2 + i * (size + 2), spec.y - spec.h / 2 + size / 2);
        this.layers.get("tokens")?.addChild(txt);
        nodes.push(txt);
      });
      this.tokenBadges.set(id, nodes);
    }
```

In `removeToken`, after the border cleanup (before the closing brace ~line 193):

```ts
    const badges = this.tokenBadges.get(id);
    if (badges) {
      for (const b of badges) b.destroy();
      this.tokenBadges.delete(id);
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/render test`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/client/render/src/types.ts src/client/render/src/token-view.ts src/client/render/src/pixi-backend.ts src/client/render/src/token-view.test.ts src/client/render/src/backend.mock.test.ts
git commit -m "feat(m10c): render condition badges as upright glyph chips on tokens"
```

---

### Task 4: AppContext — `selfId` + advisory `canEdit` gate

**Files:**
- Modify: `src/client/ui-kit/src/appContext.ts` (interface)
- Modify: `src/client/shell/src/lib/worldSession.svelte.ts` (store grants/reqs; `selfId` + `canEdit`)
- Modify: `src/client/shell/src/lib/Table.svelte` (pass through)
- Modify: `src/client/ui-kit/src/__fixtures__/appContextTest.ts`, `src/client/ui-kit/src/__fixtures__/SurfaceHarness.svelte`
- Test: `src/client/shell/src/lib/worldSession.test.ts`

**Interfaces:**
- Consumes (core): `resolveCaps`, `canWritePath` (already exported), `WireDocument`, `WireWelcome` (carries `world_default_grants`, `capability_requirements`).
- Produces (AppContext):
  - `selfId: string` — the current user's id.
  - `canEdit(doc: WireDocument, path: string): boolean` — advisory Update-path gate; GM ⇒ always true; mirrors the server's `canWritePath`.

> **Public-API note:** This extends `AppContext`, a module-facing public API (CLAUDE.md Collaboration §2). The execution-handoff captures consent before this task runs.

- [ ] **Step 1: Write the failing test**

In `src/client/shell/src/lib/worldSession.test.ts`, append (match the file's existing `WorldSession` construction + mock-connect/Welcome harness):

```ts
it("canEdit: GM bypasses; a non-GM owner may write /system/conditions, a non-owner may not", () => {
  // Construct a WorldSession with selfId "u-self"; drive a Welcome with empty grants/requirements.
  // role "gm":
  //   expect(session.canEdit(anyDoc, "/system/conditions")).toBe(true)
  // role "player":
  //   ownedDoc.permissions.users["u-self"] = "owner"  (DocRole owner ⇒ core:write_fields floor)
  //   expect(session.canEdit(ownedDoc, "/system/conditions")).toBe(true)
  //   otherDoc.permissions.default = "observer"
  //   expect(session.canEdit(otherDoc, "/system/conditions")).toBe(false)
  // expect(session.selfId).toBe("u-self")
});
```

> Build the docs with `buildActorDoc` and set `permissions.users`/`permissions.default` directly. Use the file's existing helper to deliver a Welcome (so `role` + grants/requirements populate). If the harness only drives Welcome via the WS mock, follow that path.

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/shell test -- worldSession`
Expected: FAIL — `canEdit` / `selfId` not on `WorldSession`.

- [ ] **Step 3: Write minimal implementation**

In `src/client/shell/src/lib/worldSession.svelte.ts`, extend the core import:

```ts
import {
  // ...existing...
  resolveCaps,
  canWritePath,
  type WireDocument,
  type WireCapabilityRequirement,
} from "@shadowcat/core";
```

Add fields (near `members`):

```ts
  /** World-default capability grants + declarative requirements from the latest Welcome; inputs
   * to the advisory `canEdit` gate. Re-set on every (re)connect. */
  #worldGrants: WireWelcome["world_default_grants"] = { by_role: {}, by_user: {} };
  #requirements: WireCapabilityRequirement[] = [];
```

Add accessors (after the `documents` getter):

```ts
  /** The current user's id (ownership checks). */
  get selfId(): string {
    return this.opts.selfId;
  }

  /** Advisory client-side mirror of the server's Update-path check, for showing/hiding write
   * controls. GM bypasses; the server remains authoritative and rejects a bypass at apply_intent. */
  canEdit(doc: WireDocument, path: string): boolean {
    if (this.role === "gm") return true;
    if (!this.role) return false;
    const caps = resolveCaps(doc.permissions, this.opts.selfId, this.role, this.#worldGrants);
    return canWritePath(path, caps, false, this.#requirements);
  }
```

In `#onWelcome`, after `this.role = w.user_role;`:

```ts
      this.#worldGrants = w.world_default_grants;
      this.#requirements = w.capability_requirements;
```

In `src/client/ui-kit/src/appContext.ts`, add to the import from core: `WireDocument`, and to the `AppContext` interface:

```ts
  /** The current user's id (ownership checks). */
  selfId: string;
  /** Advisory client-side edit gate (mirrors the server's Update-path check) for showing/hiding
   * write controls. The server remains authoritative. GM ⇒ always true. */
  canEdit(doc: WireDocument, path: string): boolean;
```

In `src/client/shell/src/lib/Table.svelte`, add to the `setAppContext({...})` object:

```ts
    selfId: session.selfId,
    canEdit: (doc, path) => session.canEdit(doc, path),
```

In `src/client/ui-kit/src/__fixtures__/appContextTest.ts`, add to the `ctx` literal:

```ts
    selfId: over.selfId ?? "u-self",
    canEdit: over.canEdit ?? (() => true),
```

In `src/client/ui-kit/src/__fixtures__/SurfaceHarness.svelte`, add to its inline `setAppContext({...})`:

```ts
selfId: "u1", canEdit: () => true,
```

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/shell test -- worldSession && pnpm --filter @shadowcat/ui-kit test`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/client/ui-kit/src/appContext.ts src/client/ui-kit/src/__fixtures__/appContextTest.ts src/client/ui-kit/src/__fixtures__/SurfaceHarness.svelte src/client/shell/src/lib/worldSession.svelte.ts src/client/shell/src/lib/Table.svelte src/client/shell/src/lib/worldSession.test.ts
git commit -m "feat(m10c): AppContext.selfId + advisory canEdit gate (owner-write UX)"
```

---

### Task 5: `module-conditions` — seed + editor + selection toggle palette

**Files:**
- Create: `src/modules/conditions/package.json`, `tsconfig.json`, `svelte.config.js`, `vitest.config.ts`, `vitest.setup.ts`
- Create: `src/modules/conditions/src/index.ts`, `src/ConditionsPanel.svelte`, `src/index.test.ts`, `src/ConditionsPanel.test.ts`
- Modify: `src/client/ui-kit/src/locales/en.ts` (strings)
- Modify: `src/client/shell/src/App.svelte` (register module)

**Interfaces:**
- Consumes: `Module` (core), `getAppContext` (ui-kit), `buildConditionRegistryDoc`, `Condition`, `ConditionRegistrySystem`, `conditionTarget`, `WireDocument` (core), `ctx.tokenSelection.ids`, `ctx.canEdit`.
- Produces: `export const conditions: Module` (sidebar contribution, `order: 4`).

- [ ] **Step 1: Scaffold the package (config files)**

`src/modules/conditions/package.json`:

```json
{
  "name": "@shadowcat/module-conditions",
  "version": "0.0.0",
  "private": true,
  "type": "module",
  "main": "src/index.ts",
  "dependencies": {
    "@shadowcat/core": "workspace:*",
    "@shadowcat/ui-kit": "workspace:*",
    "@shadowcat/types": "workspace:^"
  },
  "devDependencies": {
    "@testing-library/svelte": "^5.3.1",
    "jsdom": "^29.1.1",
    "sass": "^1.101.0"
  },
  "scripts": {
    "typecheck": "svelte-check --tsconfig ./tsconfig.json",
    "test": "vitest run --passWithNoTests"
  }
}
```

`src/modules/conditions/tsconfig.json`:

```json
{
  "extends": "../../../tsconfig.base.json",
  "compilerOptions": { "types": ["svelte"] },
  "include": ["src/**/*.ts", "src/**/*.svelte"]
}
```

`src/modules/conditions/svelte.config.js`:

```js
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

export default { preprocess: vitePreprocess() };
```

`src/modules/conditions/vitest.config.ts`:

```ts
import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { svelteTesting } from "@testing-library/svelte/vite";

export default defineConfig({
  plugins: [svelte(), svelteTesting()],
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./vitest.setup.ts"],
    include: ["src/**/*.test.ts"],
  },
});
```

`src/modules/conditions/vitest.setup.ts`:

```ts
// jsdom lacks ResizeObserver and WebGL; stub both so Svelte component init completes under
// tests. Real resize/GL behavior is covered by Playwright.
if (typeof globalThis.ResizeObserver === "undefined") {
  globalThis.ResizeObserver = class {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  } as unknown as typeof ResizeObserver;
}
HTMLCanvasElement.prototype.getContext = (() => null) as typeof HTMLCanvasElement.prototype.getContext;
```

Then install workspace links:

Run: `pnpm install`

- [ ] **Step 2: Write the failing module test**

`src/modules/conditions/src/index.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { ContributionRegistry } from "@shadowcat/core";
import { conditions } from "./index";

describe("conditions module", () => {
  it("contributes a sidebar panel and requires the sidebar surface", () => {
    expect(conditions.manifest.id).toBe("conditions");
    expect(conditions.manifest.requires).toContain("shadowcat.surface:sidebar");
    const contributions = new ContributionRegistry();
    conditions.register({ contributions } as never);
    expect(contributions.contributionsFor("shadowcat.surface:sidebar").length).toBe(1);
  });
});
```

- [ ] **Step 3: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/module-conditions test`
Expected: FAIL — `./index` has no `conditions` export.

- [ ] **Step 4: Implement the panel + module**

`src/modules/conditions/src/ConditionsPanel.svelte`:

```svelte
<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { buildConditionRegistryDoc, conditionTarget, type Condition, type ConditionRegistrySystem, type WireDocument } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const registry = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("condition-registry")[0];
  });
  const conditionEntries = $derived.by((): [string, Condition][] => {
    const sys = registry?.system as ConditionRegistrySystem | undefined;
    return Object.entries(sys?.conditions ?? {});
  });

  // Selected tokens drive the toggle palette: a glyph chip toggles the condition on every
  // selected token whose conditions the current user may edit (GM, or owner via canEdit).
  const selectedTokens = $derived.by((): WireDocument[] => {
    subscribe();
    const ids = ctx.tokenSelection.ids;
    return ctx.documents.query("token").filter((tok) => ids.has(tok.id));
  });

  // Idempotent GM seed: a generic emoji set, created once when the registry is absent.
  const SEED: Record<string, Condition> = {
    dead: { name: "Dead", icon: "💀" },
    unconscious: { name: "Unconscious", icon: "😵" },
    prone: { name: "Prone", icon: "🛌" },
    stunned: { name: "Stunned", icon: "💫" },
    poisoned: { name: "Poisoned", icon: "🤢" },
    blinded: { name: "Blinded", icon: "🙈" },
    invisible: { name: "Invisible", icon: "👻" },
    hasted: { name: "Hasted", icon: "⚡" },
    slowed: { name: "Slowed", icon: "🐌" },
  };
  let seeded = false;
  $effect(() => {
    if (ctx.role !== "gm" || seeded) return;
    subscribe();
    if (ctx.documents.query("condition-registry").length > 0) {
      seeded = true;
      return;
    }
    seeded = true;
    ctx.dispatchIntent([{ op: "create", doc: buildConditionRegistryDoc(ctx.world, SEED) }]);
  });

  function update(id: string, patch: Partial<Condition>): void {
    if (!registry) return;
    for (const [k, v] of Object.entries(patch)) {
      ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: `/system/conditions/${id}/${k}`, old: null, new: v }] }]);
    }
  }
  function add(): void {
    if (!registry) return;
    const id = crypto.randomUUID();
    const c: Condition = { name: "New condition", icon: "⭐" };
    ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: `/system/conditions/${id}`, old: null, new: c }] }]);
  }
  function remove(id: string): void {
    const sys = registry?.system as ConditionRegistrySystem | undefined;
    if (!registry || !sys) return;
    const next = { ...sys.conditions };
    delete next[id];
    ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: "/system/conditions", old: sys.conditions, new: next }] }]);
  }

  /** Whether the condition is set on every editable selected token (for chip active state). */
  function isActive(conditionId: string): boolean {
    const targets = selectedTokens.map((tok) => conditionTarget(tok, ctx.documents)).filter((x): x is NonNullable<typeof x> => x !== null);
    return targets.length > 0 && targets.every((tgt) => tgt.conditions.includes(conditionId));
  }

  /** Toggle a condition on each selected token whose conditions the user may edit. */
  function toggle(conditionId: string): void {
    const active = isActive(conditionId);
    for (const tok of selectedTokens) {
      const tgt = conditionTarget(tok, ctx.documents);
      if (!tgt || !ctx.canEdit(tgt.doc, tgt.path)) continue;
      const has = tgt.conditions.includes(conditionId);
      // When some-but-not-all are active, `active` is false → add to those missing it.
      if (active === has) {
        const next = has ? tgt.conditions.filter((c) => c !== conditionId) : [...tgt.conditions, conditionId];
        ctx.dispatchIntent([{ op: "update", doc_id: tgt.doc.id, changes: [{ path: tgt.path, old: tgt.conditions, new: next }] }]);
      }
    }
  }
</script>

<section class="conditions">
  <h3>{t("conditions.title")}</h3>

  {#if selectedTokens.length > 0}
    <p class="hint">{t("conditions.toggleHint")}</p>
    <div class="palette">
      {#each conditionEntries as [id, c] (id)}
        <button type="button" class:active={isActive(id)} title={c.name} onclick={() => toggle(id)}>{c.icon}</button>
      {/each}
    </div>
  {:else}
    <p class="hint">{t("conditions.selectHint")}</p>
  {/if}

  {#if ctx.role === "gm"}
    <ul class="list">
      {#each conditionEntries as [id, c] (id)}
        <li>
          <span class="glyph">{c.icon}</span>
          <input aria-label={t("conditions.name")} value={c.name} onchange={(e) => update(id, { name: e.currentTarget.value })} />
          <input aria-label={t("conditions.icon")} value={c.icon} maxlength="4" onchange={(e) => update(id, { icon: e.currentTarget.value })} />
          <button type="button" onclick={() => remove(id)}>{t("conditions.remove")}</button>
        </li>
      {/each}
    </ul>
    <button type="button" onclick={add}>{t("conditions.add")}</button>
  {/if}
</section>

<style lang="scss">
  .conditions {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-1);
  }
  .hint {
    margin: 0;
    color: var(--text-secondary);
    font-size: 0.85em;
  }
  .palette {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }
  .palette button {
    min-width: 36px;
    min-height: 36px;
    font-size: 1.1em;
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    cursor: pointer;
  }
  .palette button.active {
    border-color: var(--accent);
    background: var(--accent);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .list li {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }
  .glyph {
    font-size: 1.1em;
    flex: 0 0 auto;
  }
  input,
  button {
    min-height: 32px;
  }
</style>
```

`src/modules/conditions/src/index.ts`:

```ts
import type { Module } from "@shadowcat/core";
import ConditionsPanel from "./ConditionsPanel.svelte";

/** World condition registry: seeds a generic emoji set (GM, idempotent) + a GM editor, and a
 * selection-driven toggle palette. Replaceable — a game-system module can supply its own
 * seed/editor. Requires core-ui's sidebar. */
export const conditions: Module = {
  manifest: {
    id: "conditions",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: ["shadowcat.surface:sidebar"],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({ id: "conditions:sidebar", contract: "shadowcat.surface:sidebar", order: 4, component: ConditionsPanel });
  },
};
```

Add `conditions.*` strings to `src/client/ui-kit/src/locales/en.ts` (after the `factions.*` block):

```ts
  "conditions.title": "Conditions",
  "conditions.name": "Name",
  "conditions.icon": "Icon",
  "conditions.add": "Add condition",
  "conditions.remove": "Remove",
  "conditions.selectHint": "Select a token to toggle conditions.",
  "conditions.toggleHint": "Toggle a condition on the selected token(s).",
```

Register the module in `src/client/shell/src/App.svelte` — import `conditions` and add it to the `modules:` array in the `WorldSession` construction (next to `factions`):

```ts
import { conditions } from "@shadowcat/module-conditions";
// ...
modules: [coreUi, topBar, statusBar, stage, settings, assets, actors, factions, conditions, sceneTools]
```

> Add `"@shadowcat/module-conditions": "workspace:*"` to `src/client/shell/package.json` dependencies, then `pnpm install`.

- [ ] **Step 5: Run module test to verify it passes**

Run: `pnpm --filter @shadowcat/module-conditions test`
Expected: PASS.

- [ ] **Step 6: Write the panel behavior test**

`src/modules/conditions/src/ConditionsPanel.test.ts` — render under a test AppContext (use `setAppContextForTest` from `@shadowcat/ui-kit` fixtures via the `context` option). Cover:
1. **GM seed**: render as `role: "gm"` with an empty `documents` store + a spy `dispatchIntent`; assert one `create` of a `condition-registry` whose system has the 9 default ids.
2. **No double-seed**: render with a registry already present; assert `dispatchIntent` not called with a `create`.
3. **Toggle palette gated by canEdit**: pre-seed a registry + a linked token whose actor the user can't edit (`canEdit: () => false`); select the token (`tokenSelection.set([tokenId])`); click a glyph; assert no `update` dispatched. Then with `canEdit: () => true`, click; assert an `update` to `/system/conditions` adding the id.

```ts
import { describe, it, expect, vi } from "vitest";
import { render } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/__fixtures__/appContextTest"; // match the real import path used by factions/actors tests
import { DocumentStore, buildActorDoc, buildTokenFromActor, buildConditionRegistryDoc } from "@shadowcat/core";
import { TokenSelection } from "@shadowcat/ui-kit";
import ConditionsPanel from "./ConditionsPanel.svelte";
// ...assemble store, dispatch spy, tokenSelection; render with { context: setAppContextForTest({...}) };
// query buttons by title/role and assert dispatch payloads.
```

> Confirm the fixture import specifier the existing `factions`/`actors` panel tests use (they render Svelte panels under a seeded AppContext) and copy it verbatim — do not invent a path.

- [ ] **Step 7: Run the behavior test**

Run: `pnpm --filter @shadowcat/module-conditions test`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src/modules/conditions src/client/ui-kit/src/locales/en.ts src/client/shell/src/App.svelte src/client/shell/package.json pnpm-lock.yaml
git commit -m "feat(m10c): module-conditions — emoji seed + GM editor + selection toggle palette"
```

---

### Task 6: Integration round-trip + skill/doc sync (completion gate)

**Files:**
- Test: `src/server/tests/` (integration) **or** a client integration test — pick the layer that already round-trips an embedded-doc field Update.
- Modify: `.claude/skills/shadowcat-codebase-actors-tokens/SKILL.md`
- Modify: `docs/PLAN.md`

- [ ] **Step 1: Verify the instanced-token embedded write round-trips**

The instanced path writes `/embedded/actor/0/system/conditions`. Confirm the server's `apply_intent` accepts an array-indexed embedded JSON-pointer Update (M10b already addresses `/embedded/actor/0/...` for redaction, so the pointer form is supported). Add/extend an integration test that:
1. Creates an instanced token (embedded actor copy with `conditions: []`).
2. Applies an Update at `/embedded/actor/0/system/conditions` → `["dead"]`.
3. Asserts the broadcast/persisted doc reflects the change.

Run the relevant suite:
Run: `cd src/server && cargo test` (if server-side) **or** `pnpm --filter @shadowcat/core test` (if a wire round-trip exists client-side).
Expected: PASS — the embedded conditions Update applies.

> If the indexed embedded pointer is rejected, fix-forward: in `conditionTarget`, return `path: "/embedded"` with a whole-object replacement built by the panel (mirroring `FactionsPanel.remove`'s whole-map replace). Update Task 2's test accordingly. Surface this immediately rather than deferring.

- [ ] **Step 2: Full-suite green**

Run:
```bash
pnpm -r test && pnpm -r typecheck && pnpm lint
cd src/server && cargo fmt --check && cargo clippy -- -D warnings && cargo test
```
Expected: all green. (No Rust source changed; the server suite is a regression guard.)

- [ ] **Step 3: Reviewed skill-update gate**

Update `.claude/skills/shadowcat-codebase-actors-tokens/SKILL.md`:
- Under **Key files & seams** / **Hard invariants**, add the condition registry (`condition-registry` config-doc; `conditions: string[]` resolved via `resolveConditions`; `conditionTarget` linked-vs-instanced write site; badges render via `TokenNodeSpec.badges`; `AppContext.canEdit` advisory gate).
- Keep it orientation+index — point into this plan/spec, don't duplicate.

This gate is mandatory (CLAUDE.md `## Codebase Skills & Agents` §1): after writing the diff, dispatch `shadowcat-spec-reviewer` on the skill diff to confirm it accurately captures the change (no omission/drift/broken pointer). Record the verdict.

- [ ] **Step 4: Update PLAN.md**

Mark **M10c — Conditions** complete in `docs/PLAN.md` (mirror the M10b entry style). Note: registry config-doc + `module-conditions` emoji seed + badges + GM/owner toggle via `canEdit`; markers-only (effects deferred to combat).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "docs(m10c): mark conditions complete; sync actors-tokens skill"
```

---

## Buddy-check directives

This plan adds a **secrecy-adjacent capability gate** (`canEdit`) and **two divergent write paths** (linked actor doc vs instanced embedded copy) — high-risk signals. Per the user's standing M10 directive (mainline development + buddy-checking, `/clear` between checkpoints), after execution:

- Run **`mainline-plan-execution`'s** final review, then a **buddy-check** with the two-reviewer pair: `shadowcat-spec-reviewer` (does it match this plan + spec §8, nothing skipped/downgraded) **and** `shadowcat-code-reviewer` (bugs, the `canEdit` gate's correctness, the linked/instanced write divergence, emoji badge cleanup/leak).
- Converge findings; fix Critical/Important inline with regression tests before merge.
- Merge `--no-ff` to local main (do **not** push — the M10 push gate is the full milestone).

## Self-Review (completed)

- **Spec §8 coverage:** registry config-doc (Task 1) ✓; `conditions: string[]` already present, resolved (Task 2) ✓; overlay badges (Task 3) ✓; GM-toggles-any + owner-toggles-own via capability model (Task 4 `canEdit` + Task 5 gate) ✓; replaceable `module-conditions` generic seed (Task 5) ✓; markers-only, effects deferred ✓.
- **Mirror-of-M10b:** config-doc registry + seed module + engine-mechanism/module-content split ✓.
- **Type consistency:** `Condition{name,icon}`, `ConditionRegistrySystem{conditions}`, `buildConditionRegistryDoc`, `resolveConditions`, `conditionTarget`/`ConditionTarget`, `TokenNodeSpec.badges`, `AppContext.{selfId,canEdit}` used consistently across tasks.
- **Open validation (flagged, not deferred):** the array-indexed embedded pointer write (Task 6 Step 1) — verified by an integration round-trip with a whole-`/embedded`-replace fix-forward if rejected.
