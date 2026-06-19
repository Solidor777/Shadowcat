# M7b-2 — Client Contribution Registry + Resolution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: this project executes plans with
> the **mainline-plan-execution** skill (inline, per-task spec-compliance check +
> a single final branch review) — NOT subagent-driven-development or
> executing-plans. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the framework-neutral client side of the contribution
architecture in `@shadowcat/core`: a `ContributionRegistry`, manifest
`provides`/`requires`, generalized module resolution (contract dependencies +
singleton loud-fail), and `Welcome`-topology reconciliation.

**Architecture:** Pure framework-neutral TypeScript in `src/client/core/` (no
Svelte, no DOM). A `ContributionRegistry` with the same `subscribe`/snapshot
shape as `DocumentStore`, holding opaque (`unknown`) component handles. The
existing `ModuleRegistry` gains contract-based dependency resolution; the
`Welcome` wire schema gains `contract_declarations` (already sent by the M7b-1
server) for advisory reconciliation.

**Tech Stack:** TypeScript, Zod, vitest.

## Global Constraints

- Framework-neutral: no Svelte/DOM imports in `@shadowcat/core`. `component` is
  `unknown` (opaque). Reactivity is `subscribe(listener) → unsubscribe` +
  snapshot, exactly like `DocumentStore` (store.ts) — no runes.
- The `Welcome` reconciliation is **advisory** (warn-on-mismatch via the
  `Logger`); it never throws or refuses. Client resolution drives rendering.
- Singleton conflict is a **hard throw** (loud-fail); a `requires` with no active
  provider → the module is **not activated** (existing "dependency unmet" warn
  path), never a throw.
- Wire validation: `contract_declarations` is a **required** array on the
  `welcome` frame (mirroring `capability_requirements` — required, not
  defaulted); the M7b-1 server always sends it. Every synthetic `welcome` test
  fixture is updated.
- ts-rs alignment: the client `ContractDeclaration` shape structurally matches
  the generated `@shadowcat/types` (`ContractDeclaration`, `ContractProvide`,
  `Cardinality`) from M7b-1; the wire drift guard (`wire.test.ts`) asserts it.
- TDD: failing test first, watch it fail, minimal impl, watch it pass, commit.
- Commands (from repo root):
  - Single test file: `pnpm --filter @shadowcat/core exec vitest run src/<file>.test.ts`
  - Full unit suite: `pnpm --filter @shadowcat/core test`
  - Typecheck: `pnpm --filter @shadowcat/core typecheck`
  (The e2e suite `test:e2e` spawns a real server and is not required per task.)

---

### Task 1: `ContributionRegistry`

**Files:**
- Create: `src/client/core/src/contributions.ts`
- Create: `src/client/core/src/contributions.test.ts`
- Modify: `src/client/core/src/index.ts` (export the registry + types)

**Interfaces:**
- Produces:
  ```ts
  export type Cardinality = "singleton" | "multi";
  export interface Contribution {
    id: string;
    contract: string;
    order?: number;
    props?: Record<string, unknown>;
    component: unknown;
  }
  export class ContributionRegistry {
    contribute(c: Contribution, opts?: { module?: string }): () => void;
    contributionsFor(contract: string): readonly Contribution[];
    subscribe(listener: () => void): () => void;
    removeModule(moduleId: string): void;
  }
  ```

- [ ] **Step 1: Write the failing test**

`src/client/core/src/contributions.test.ts`:

```ts
import { describe, it, expect, vi } from "vitest";
import { ContributionRegistry, type Contribution } from "./contributions";

const c = (over: Partial<Contribution>): Contribution => ({
  id: "x",
  contract: "s:sidebar",
  component: {},
  ...over,
});

describe("ContributionRegistry", () => {
  it("returns contributions for a contract sorted by order then insertion", () => {
    const r = new ContributionRegistry();
    r.contribute(c({ id: "b", order: 2 }));
    r.contribute(c({ id: "a", order: 1 }));
    r.contribute(c({ id: "c" })); // order undefined → 0
    expect(r.contributionsFor("s:sidebar").map((x) => x.id)).toEqual(["c", "a", "b"]);
    expect(r.contributionsFor("s:other")).toEqual([]);
  });

  it("dispose removes a single contribution and notifies subscribers", () => {
    const r = new ContributionRegistry();
    const listener = vi.fn();
    r.subscribe(listener);
    const dispose = r.contribute(c({ id: "a" }));
    expect(listener).toHaveBeenCalledTimes(1);
    expect(r.contributionsFor("s:sidebar")).toHaveLength(1);
    dispose();
    expect(listener).toHaveBeenCalledTimes(2);
    expect(r.contributionsFor("s:sidebar")).toHaveLength(0);
  });

  it("removeModule drops every contribution tagged with that module", () => {
    const r = new ContributionRegistry();
    r.contribute(c({ id: "a" }), { module: "m1" });
    r.contribute(c({ id: "b" }), { module: "m1" });
    r.contribute(c({ id: "k" }), { module: "m2" });
    r.removeModule("m1");
    expect(r.contributionsFor("s:sidebar").map((x) => x.id)).toEqual(["k"]);
  });

  it("subscribe returns an unsubscribe that stops notifications", () => {
    const r = new ContributionRegistry();
    const listener = vi.fn();
    const off = r.subscribe(listener);
    off();
    r.contribute(c({ id: "a" }));
    expect(listener).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run it to verify it fails**

Run: `pnpm --filter @shadowcat/core exec vitest run src/contributions.test.ts`
Expected: FAIL — cannot find module `./contributions`.

- [ ] **Step 3: Implement the registry**

`src/client/core/src/contributions.ts`:

```ts
// The framework-neutral UI contribution registry: modules contribute opaque
// component handles into named string-contract "surfaces"; a host (e.g. the
// Svelte <Surface> adapter) renders them. Same subscribe/snapshot reactivity as
// DocumentStore — no framework runtime here; `component` is opaque to core.

/** One provider or many for a surface contract. */
export type Cardinality = "singleton" | "multi";

export interface Contribution {
  id: string;
  contract: string;
  /** Ascending sort key within a contract; default 0. */
  order?: number;
  props?: Record<string, unknown>;
  /** Opaque host-rendered component handle. */
  component: unknown;
}

interface Entry {
  c: Contribution;
  module?: string;
  seq: number;
}

export type Listener = () => void;

export class ContributionRegistry {
  private entries: Entry[] = [];
  private listeners = new Set<Listener>();
  private seqCounter = 0;

  /** Register a contribution; returns a dispose that removes exactly it. */
  contribute(c: Contribution, opts: { module?: string } = {}): () => void {
    const entry: Entry = { c, module: opts.module, seq: this.seqCounter++ };
    this.entries.push(entry);
    this.emit();
    return () => {
      const i = this.entries.indexOf(entry);
      if (i >= 0) {
        this.entries.splice(i, 1);
        this.emit();
      }
    };
  }

  /** Contributions for a contract, sorted by `order` (default 0) then insertion. */
  contributionsFor(contract: string): readonly Contribution[] {
    return this.entries
      .filter((e) => e.c.contract === contract)
      .sort((a, b) => (a.c.order ?? 0) - (b.c.order ?? 0) || a.seq - b.seq)
      .map((e) => e.c);
  }

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Drop every contribution tagged with `moduleId` (module unload teardown). */
  removeModule(moduleId: string): void {
    const before = this.entries.length;
    this.entries = this.entries.filter((e) => e.module !== moduleId);
    if (this.entries.length !== before) this.emit();
  }

  private emit(): void {
    for (const fn of this.listeners) fn();
  }
}
```

- [ ] **Step 4: Run it to verify it passes**

Run: `pnpm --filter @shadowcat/core exec vitest run src/contributions.test.ts`
Expected: PASS.

- [ ] **Step 5: Export from the barrel**

In `src/client/core/src/index.ts`, add:

```ts
export { ContributionRegistry } from "./contributions";
export type { Contribution, Cardinality } from "./contributions";
```

- [ ] **Step 6: Commit**

```bash
git add src/client/core/src/contributions.ts src/client/core/src/contributions.test.ts \
        src/client/core/src/index.ts
git commit -m "feat(core): framework-neutral ContributionRegistry"
```

---

### Task 2: Manifest `provides` / `requires`

**Files:**
- Modify: `src/client/core/src/manifest.ts` (extend `ModuleManifest` +
  `ManifestSchema`; add `ContractProvide`, `declarationOf`)
- Modify: `src/client/core/src/manifest.test.ts` (accept/reject + projection)
- Modify: `src/client/core/src/index.ts` (export the new type + helper)

**Interfaces:**
- Consumes: `Cardinality` (Task 1).
- Produces:
  - `ContractProvide { contract: string; cardinality: Cardinality }` on
    `ModuleManifest` as `provides?: ContractProvide[]` and `requires?: string[]`.
  - `ContractDeclaration { module_id: string; version: string; provides:
    ContractProvide[]; requires: string[] }` (structurally matches the M7b-1
    ts-rs type).
  - `declarationOf(m: ModuleManifest): ContractDeclaration` — projects a manifest
    to its declaration (empty arrays when the fields are absent).

- [ ] **Step 1: Write the failing test**

Add to `src/client/core/src/manifest.test.ts`:

```ts
import { declarationOf } from "./manifest";

it("accepts provides/requires and projects to a declaration", () => {
  const m = parseManifest({
    id: "sidebar",
    version: "1.0.0",
    dependencies: {},
    provides: [{ contract: "s:sidebar", cardinality: "singleton" }],
    requires: ["s:root"],
  });
  expect(declarationOf(m)).toEqual({
    module_id: "sidebar",
    version: "1.0.0",
    provides: [{ contract: "s:sidebar", cardinality: "singleton" }],
    requires: ["s:root"],
  });
});

it("defaults provides/requires to empty in a projection", () => {
  const m = parseManifest({ id: "m", version: "1.0.0", dependencies: {} });
  expect(declarationOf(m)).toEqual({
    module_id: "m",
    version: "1.0.0",
    provides: [],
    requires: [],
  });
});

it("rejects an invalid cardinality", () => {
  expect(() =>
    parseManifest({
      id: "m",
      version: "1.0.0",
      dependencies: {},
      provides: [{ contract: "s:x", cardinality: "lots" }],
    }),
  ).toThrow();
});
```

(Match the existing import style at the top of `manifest.test.ts`; `parseManifest`
is already imported there.)

- [ ] **Step 2: Run it to verify it fails**

Run: `pnpm --filter @shadowcat/core exec vitest run src/manifest.test.ts`
Expected: FAIL — `declarationOf` is not exported.

- [ ] **Step 3: Extend the manifest**

In `src/client/core/src/manifest.ts`:

Add to imports: `import type { Cardinality } from "./contributions";`

Add the types and extend `ModuleManifest`:

```ts
export interface ContractProvide {
  contract: string;
  cardinality: Cardinality;
}

/** A module's UI contract declaration (structurally matches the ts-rs type). */
export interface ContractDeclaration {
  module_id: string;
  version: string;
  provides: ContractProvide[];
  requires: string[];
}
```

Add to the `ModuleManifest` interface:

```ts
  provides?: ContractProvide[];
  requires?: string[];
```

Add to the `ManifestSchema` Zod object (alongside the existing optional fields):

```ts
  provides: z
    .array(z.object({ contract: z.string(), cardinality: z.enum(["singleton", "multi"]) }))
    .optional(),
  requires: z.array(z.string()).optional(),
```

Add the projection helper (bottom of the file):

```ts
/** Project a manifest to its UI contract declaration (empty arrays when unset). */
export function declarationOf(m: ModuleManifest): ContractDeclaration {
  return {
    module_id: m.id,
    version: m.version,
    provides: m.provides ?? [],
    requires: m.requires ?? [],
  };
}
```

- [ ] **Step 4: Run it to verify it passes**

Run: `pnpm --filter @shadowcat/core exec vitest run src/manifest.test.ts`
Expected: PASS.

- [ ] **Step 5: Export from the barrel**

In `src/client/core/src/index.ts`, extend the manifest export line and types:

```ts
export { ManifestSchema, parseManifest, declarationOf } from "./manifest";
export type {
  ModuleManifest,
  CapRequirement,
  HookDecl,
  ContractProvide,
  ContractDeclaration,
} from "./manifest";
```

- [ ] **Step 6: Commit**

```bash
git add src/client/core/src/manifest.ts src/client/core/src/manifest.test.ts \
        src/client/core/src/index.ts
git commit -m "feat(core): manifest provides/requires + declarationOf projection"
```

---

### Task 3: Generalized resolution + `ModuleContext.contributions`

**Files:**
- Modify: `src/client/core/src/modules.ts` (`Deps`, `ModuleContext`, `contextFor`,
  `depsSatisfied`, `topoSort`, `activate`, `unload`)
- Modify: `src/client/core/src/modules.test.ts` (resolution tests)

**Interfaces:**
- Consumes: `ContributionRegistry` (Task 1); manifest `provides`/`requires` (Task 2).
- Produces: `ModuleContext.contributions: { contribute(c: Contribution): () => void }`
  (module-tagged); resolution honoring contract `requires`/`provides`.

- [ ] **Step 1: Write the failing tests**

Add to `src/client/core/src/modules.test.ts` (follow the file's existing
`deps()`/module-builder helpers; a module's `manifest.provides`/`requires` drive
resolution):

```ts
it("activates a contract provider before a requirer (topological by contract)", async () => {
  const order: string[] = [];
  const provider = mod({
    id: "sidebar", version: "1.0.0", dependencies: {},
    provides: [{ contract: "s:sidebar", cardinality: "singleton" }],
  }, () => order.push("sidebar"));
  const requirer = mod({
    id: "combat", version: "1.0.0", dependencies: {},
    requires: ["s:sidebar"],
  }, () => order.push("combat"));
  const r = new ModuleRegistry(deps());
  r.add(requirer);
  r.add(provider);
  await r.activate();
  expect(order).toEqual(["sidebar", "combat"]);
});

it("does not activate a module whose required contract has no provider", async () => {
  const requirer = mod({
    id: "combat", version: "1.0.0", dependencies: {}, requires: ["s:missing"],
  });
  const r = new ModuleRegistry(deps());
  r.add(requirer);
  await r.activate();
  expect(r.list().find((m) => m.id === "combat")?.active).toBe(false);
});

it("throws when two active modules provide the same singleton contract", async () => {
  const a = mod({ id: "a", version: "1.0.0", dependencies: {},
    provides: [{ contract: "s:sidebar", cardinality: "singleton" }] });
  const b = mod({ id: "b", version: "1.0.0", dependencies: {},
    provides: [{ contract: "s:sidebar", cardinality: "singleton" }] });
  const r = new ModuleRegistry(deps());
  r.add(a);
  r.add(b);
  await expect(r.activate()).rejects.toThrow(/singleton/);
});

it("allows two providers of a multi contract", async () => {
  const a = mod({ id: "a", version: "1.0.0", dependencies: {},
    provides: [{ contract: "s:panel", cardinality: "multi" }] });
  const b = mod({ id: "b", version: "1.0.0", dependencies: {},
    provides: [{ contract: "s:panel", cardinality: "multi" }] });
  const r = new ModuleRegistry(deps());
  r.add(a);
  r.add(b);
  await r.activate();
  expect(r.list().every((m) => m.active)).toBe(true);
});

it("removes a module's contributions on unload", async () => {
  const reg = new ContributionRegistry();
  const d = { ...deps(), contributions: reg };
  const m = mod({ id: "m", version: "1.0.0", dependencies: {} }, (ctx) => {
    ctx.contributions.contribute({ id: "p", contract: "s:sidebar", component: {} });
  });
  const r = new ModuleRegistry(d);
  r.add(m);
  await r.activate();
  expect(reg.contributionsFor("s:sidebar")).toHaveLength(1);
  await r.unload("m");
  expect(reg.contributionsFor("s:sidebar")).toHaveLength(0);
});
```

Update the file's `deps()` helper to include `contributions: new ContributionRegistry()`
and import `ContributionRegistry`. Add a `mod(manifest, register?)` helper if the
file doesn't already have an equivalent (the existing module-builder pattern in
`modules.test.ts` — reuse it; pass `provides`/`requires` through the manifest).

- [ ] **Step 2: Run it to verify it fails**

Run: `pnpm --filter @shadowcat/core exec vitest run src/modules.test.ts`
Expected: FAIL — `Deps` has no `contributions`; resolution does not consider
contracts.

- [ ] **Step 3: Implement resolution + context**

In `src/client/core/src/modules.ts`:

Imports:
```ts
import { ContributionRegistry, type Contribution } from "./contributions";
```

Add to `ModuleContext`:
```ts
  contributions: {
    contribute(c: Contribution): () => void;
  };
```

Add to `Deps`:
```ts
  contributions: ContributionRegistry;
```

In `contextFor`, add to the returned object (tagging by module, mirroring
`services.provide`):
```ts
      contributions: {
        contribute: (c) => contributions.contribute(c, { module: moduleId }),
      },
```
and destructure `contributions` from `this.deps` at the top of `contextFor`.

Add a private helper to index contract providers among **active** modules:
```ts
  /** Active modules that provide `contract`. */
  private activeProvidersOf(contract: string): string[] {
    return [...this.records.values()]
      .filter(
        (r) =>
          r.active &&
          (r.module.manifest.provides ?? []).some((p) => p.contract === contract),
      )
      .map((r) => r.module.manifest.id);
  }
```

Extend `depsSatisfied` to also require an active provider for each contract:
```ts
  private depsSatisfied(m: Module): boolean {
    for (const [depId, range] of Object.entries(m.manifest.dependencies)) {
      const dep = this.records.get(depId);
      if (!dep || !dep.active) return false;
      if (!satisfies(dep.module.manifest.version, range)) return false;
    }
    for (const contract of m.manifest.requires ?? []) {
      if (this.activeProvidersOf(contract).length === 0) return false;
    }
    return true;
  }
```

Extend `topoSort` to add requirer→provider edges. Build a contract→provider-ids
index once, and in `visit`, after visiting `dependencies`, also visit providers of
each required contract:
```ts
    const providersByContract = new Map<string, string[]>();
    for (const r of this.records.values()) {
      for (const p of r.module.manifest.provides ?? []) {
        const arr = providersByContract.get(p.contract) ?? [];
        arr.push(r.module.manifest.id);
        providersByContract.set(p.contract, arr);
      }
    }
```
and inside `visit(id, path)`, after the `dependencies` loop:
```ts
      for (const contract of r.module.manifest.requires ?? []) {
        for (const providerId of providersByContract.get(contract) ?? []) {
          visit(providerId, [...path, id]);
        }
      }
```

In `activate`, before `await r.module.register(...)`, add the singleton loud-fail
check (after the `depsSatisfied` guard):
```ts
      for (const p of r.module.manifest.provides ?? []) {
        if (p.cardinality === "singleton") {
          const others = this.activeProvidersOf(p.contract).filter((x) => x !== id);
          if (others.length > 0) {
            throw new Error(
              `singleton contract ${p.contract} already provided by ${others[0]}`,
            );
          }
        }
      }
```

In `unload`, alongside the existing `hooks/services/middleware.removeModule(id)`
calls, add:
```ts
    this.deps.contributions.removeModule(id);
```

- [ ] **Step 4: Run it to verify it passes**

Run: `pnpm --filter @shadowcat/core exec vitest run src/modules.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/client/core/src/modules.ts src/client/core/src/modules.test.ts
git commit -m "feat(core): contract-based resolution + ModuleContext.contributions"
```

---

### Task 4: `Welcome` carries `contract_declarations` (wire)

**Files:**
- Modify: `src/client/core/src/wire.ts` (schemas + welcome field + type export)
- Modify: `src/client/core/src/wire.test.ts` (drift guard + 2 fixtures)
- Modify: `src/client/core/src/mock-server.ts` (welcome fixture)
- Modify: `src/client/core/src/index.ts` (export `WireContractDeclaration`)

**Interfaces:**
- Produces: `ServerMsg` `welcome` variant gains
  `contract_declarations: WireContractDeclaration[]`;
  `WireContractDeclaration = z.infer<typeof ContractDeclarationSchema>`.

- [ ] **Step 1: Add the failing drift-guard assertion + fixtures**

In `src/client/core/src/wire.test.ts`, add to the "Welcome capability fields
match ts-rs" test:

```ts
    expectTypeOf<W["contract_declarations"]>().toEqualTypeOf<
      T["contract_declarations"]
    >();
```

And add `contract_declarations: []` to BOTH `parseServerMsg` welcome fixtures
(the well-formed frame ~line 79 and the capability-fields frame ~line 93).

- [ ] **Step 2: Run it to verify it fails**

Run: `pnpm --filter @shadowcat/core exec vitest run src/wire.test.ts`
Expected: FAIL — `contract_declarations` missing on the Zod-inferred `W`
(type error) and/or the ts-rs `T`.

- [ ] **Step 3: Add the wire schemas + welcome field**

In `src/client/core/src/wire.ts`, add near `CapabilityRequirementSchema`:

```ts
export const CardinalitySchema = z.enum(["singleton", "multi"]);

export const ContractProvideSchema = z.object({
  contract: z.string(),
  cardinality: CardinalitySchema,
});

export const ContractDeclarationSchema = z.object({
  module_id: z.string(),
  version: z.string(),
  provides: z.array(ContractProvideSchema),
  requires: z.array(z.string()),
});
export type WireContractDeclaration = z.infer<typeof ContractDeclarationSchema>;
```

Add to the `welcome` object in `ServerMsgSchema` (after
`capability_requirements`):

```ts
    contract_declarations: z.array(ContractDeclarationSchema),
```

- [ ] **Step 4: Update the mock server fixture**

In `src/client/core/src/mock-server.ts`, add to the `welcome` object (after
`capability_requirements: [],`):

```ts
        contract_declarations: [],
```

- [ ] **Step 5: Run it to verify it passes**

Run: `pnpm --filter @shadowcat/core exec vitest run src/wire.test.ts`
Expected: PASS.

- [ ] **Step 6: Export + commit**

In `src/client/core/src/index.ts`, add `WireContractDeclaration` to the wire type
exports. Then:

```bash
git add src/client/core/src/wire.ts src/client/core/src/wire.test.ts \
        src/client/core/src/mock-server.ts src/client/core/src/index.ts
git commit -m "feat(core): Welcome wire schema carries contract_declarations"
```

---

### Task 5: Topology reconciliation

**Files:**
- Create: `src/client/core/src/topology.ts`
- Create: `src/client/core/src/topology.test.ts`
- Modify: `src/client/core/src/modules.ts` (add `ModuleRegistry.declarations()`)
- Modify: `src/client/core/src/index.ts` (export `reconcileTopology`)

**Interfaces:**
- Consumes: `ContractDeclaration` (Task 2), `declarationOf` (Task 2),
  `WireContractDeclaration` (Task 4), `Logger`.
- Produces:
  - `ModuleRegistry.declarations(): ContractDeclaration[]` — active modules
    projected via `declarationOf`.
  - `reconcileTopology(local: ContractDeclaration[], remote: WireContractDeclaration[], logger: Logger): void`
    — `logger.warn` for each module present locally but absent remotely, and
    vice-versa (keyed by `module_id`). Never throws.

- [ ] **Step 1: Write the failing test**

`src/client/core/src/topology.test.ts`:

```ts
import { describe, it, expect, vi } from "vitest";
import { reconcileTopology } from "./topology";
import type { Logger } from "./logger";

const decl = (module_id: string) => ({ module_id, version: "1", provides: [], requires: [] });
const logger = (): Logger => ({ debug: vi.fn(), info: vi.fn(), warn: vi.fn(), error: vi.fn() });

describe("reconcileTopology", () => {
  it("does not warn when local and remote module sets match", () => {
    const l = logger();
    reconcileTopology([decl("a"), decl("b")], [decl("a"), decl("b")], l);
    expect(l.warn).not.toHaveBeenCalled();
  });

  it("warns for a module loaded locally but absent from the world topology", () => {
    const l = logger();
    reconcileTopology([decl("a"), decl("x")], [decl("a")], l);
    expect(l.warn).toHaveBeenCalledTimes(1);
  });

  it("warns for a module in the world topology but not loaded locally", () => {
    const l = logger();
    reconcileTopology([decl("a")], [decl("a"), decl("y")], l);
    expect(l.warn).toHaveBeenCalledTimes(1);
  });
});
```

(Confirm the `Logger` interface shape in `logger.ts` and match it in the test
helper — adjust the mocked methods to the real ones if they differ.)

- [ ] **Step 2: Run it to verify it fails**

Run: `pnpm --filter @shadowcat/core exec vitest run src/topology.test.ts`
Expected: FAIL — cannot find `./topology`.

- [ ] **Step 3: Implement reconciliation**

`src/client/core/src/topology.ts`:

```ts
// Advisory reconciliation of the client's loaded module topology against the
// server-broadcast world topology (Welcome.contract_declarations). Warn-only —
// the client renders from its own resolution; the server copy is the
// consistency authority. Hard enforcement lands with module management.
import type { Logger } from "./logger";
import type { ContractDeclaration } from "./manifest";

interface WireLike {
  module_id: string;
}

/** Warn for each module present on exactly one side, keyed by module_id. */
export function reconcileTopology(
  local: ContractDeclaration[],
  remote: WireLike[],
  logger: Logger,
): void {
  const localIds = new Set(local.map((d) => d.module_id));
  const remoteIds = new Set(remote.map((d) => d.module_id));
  for (const id of localIds) {
    if (!remoteIds.has(id)) {
      logger.warn(`module ${id} is loaded but absent from the world contract topology`);
    }
  }
  for (const id of remoteIds) {
    if (!localIds.has(id)) {
      logger.warn(`world contract topology declares module ${id} which is not loaded`);
    }
  }
}
```

In `src/client/core/src/modules.ts`, add a method on `ModuleRegistry`:

```ts
  /** Active modules projected to their UI contract declarations. */
  declarations(): ContractDeclaration[] {
    return [...this.records.values()]
      .filter((r) => r.active)
      .map((r) => declarationOf(r.module.manifest));
  }
```

Add `import { declarationOf, type ContractDeclaration } from "./manifest";` to
`modules.ts`.

- [ ] **Step 4: Run it to verify it passes**

Run: `pnpm --filter @shadowcat/core exec vitest run src/topology.test.ts`
Expected: PASS.

- [ ] **Step 5: Export + commit**

In `src/client/core/src/index.ts`, add:

```ts
export { reconcileTopology } from "./topology";
```

```bash
git add src/client/core/src/topology.ts src/client/core/src/topology.test.ts \
        src/client/core/src/modules.ts src/client/core/src/index.ts
git commit -m "feat(core): advisory Welcome-topology reconciliation"
```

---

### Task 6: Full suite green + typecheck

**Files:** none (verification only)

- [ ] **Step 1: Run the whole core unit suite**

Run: `pnpm --filter @shadowcat/core test`
Expected: PASS — all existing tests plus the new contribution/manifest/modules/
wire/topology tests green.

- [ ] **Step 2: Typecheck**

Run: `pnpm --filter @shadowcat/core typecheck`
Expected: no errors (the wire drift guard confirms `contract_declarations`
matches the ts-rs `ServerMsg`; the barrel re-exports compile).

---

## Self-Review

**Spec coverage (spec §5):**
- `ContributionRegistry` (§5.1) → Task 1. ✓
- `ModuleContext.contributions` + per-module teardown (§5.2) → Task 3. ✓
- Manifest `provides`/`requires` + Zod (§5.3) → Task 2. ✓
- Generalized `depsSatisfied`/`topoSort` + singleton loud-fail (§5.4) → Task 3. ✓
- `Welcome` reconciliation (advisory warn) (§5.5) → Tasks 4 (wire) + 5
  (reconcile). ✓

**Placeholder scan:** No TBD/TODO; every code/test block is complete. The two
"confirm the existing helper/Logger shape" notes (Task 3 `mod`/`deps`, Task 5
`Logger`) are instructions to match real signatures, not placeholders. ✓

**Type consistency:** `ContributionRegistry`, `Contribution`, `Cardinality`,
`ContractProvide`, `ContractDeclaration`, `declarationOf`, `reconcileTopology`,
`ModuleRegistry.declarations()`, and `ModuleContext.contributions.contribute`
match across tasks. The client `Cardinality` (`"singleton" | "multi"`) and the
wire `CardinalitySchema` enum agree; the manifest `ContractDeclaration` shape
structurally matches the wire `WireContractDeclaration` (asserted by the Task-4
drift guard). ✓

## Out of scope (M7b-3)

The Svelte `<Surface>` adapter, `appContext`, and the wiring of `WsClient.onWelcome`
→ `reconcileTopology(registry.declarations(), welcome.contract_declarations)` are
M7b-3 (the Svelte host) / M7c (the shell). M7b-2 ships only the framework-neutral
core mechanism and the reconciliation function it will call.
