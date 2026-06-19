# M7b-3 — Svelte `<Surface>` Adapter + Harness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: this project executes plans with
> the **mainline-plan-execution** skill (inline, per-task spec-compliance check +
> a single final branch review) — NOT subagent-driven-development or
> executing-plans. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The default Svelte host for the contribution architecture — a
`<Surface contract>` component that reactively renders the contributions for a
contract (the M7b-2 `ContributionRegistry`), plus the `appContext` it reads the
registry from, plus the test setup the `ui` package currently lacks.

**Architecture:** `src/client/ui/` (Svelte 5 + Vite) gains its first real code and
test runner. `<Surface>` bridges the framework-neutral core registry to Svelte via
`createSubscriber` (svelte/reactivity) inside a `$derived`, rendering each
contribution's opaque `component` handle as a Svelte 5 dynamic component. The
registry is provided ambiently through Svelte context (`appContext`).

**Tech Stack:** Svelte 5 (runes), Vite, Vitest + `@testing-library/svelte` + jsdom,
`@shadowcat/core`.

## Global Constraints

- **Svelte 5 runes only** (no `export let`/`$:`/`<slot>`/`on:`): props via
  `$props()`, reactive reads via `$derived`/`$derived.by`, dynamic component via a
  capitalized variable in tag position (`{@const Comp = ...}` then `<Comp />`) —
  `<svelte:component>` is legacy, do not use.
- The bridge is `createSubscriber` (Svelte ≥5.7; svelte is 5.56 — **verify the
  installed version exposes it**, else bump svelte). The registry stays
  framework-neutral; `<Surface>` is the only Svelte coupling.
- New devDeps (`@testing-library/svelte`, `jsdom`) are logged in `ARCHITECTURE.md`
  with a one-line rationale (the project vets every dependency). `@testing-library/
  svelte` was approved in the M7b spec §8.
- **`AppContext` scope (sequencing-forced decomposition):** M7b-3 carries only
  `{ contributions: ContributionRegistry }` — the sole field with a provider +
  consumer now. `store`/`world`/`role` join in M7c (shell + `Welcome`), `t` in M7d
  (i18n). Documented as extensible; this narrows the spec §6.2 proposed shape on
  purpose.
- TDD: failing test first, watch it fail, minimal impl, watch it pass, commit.
- Commands (from repo root):
  - Single ui test: `pnpm --filter @shadowcat/ui exec vitest run src/lib/<file>.test.ts`
  - Full ui tests: `pnpm --filter @shadowcat/ui test`
  - ui typecheck: `pnpm --filter @shadowcat/ui typecheck` (svelte-check)

---

### Task 1: Test infrastructure + smoke test

**Files:**
- Modify: `src/client/ui/package.json` (devDeps + `test` script)
- Create: `src/client/ui/vitest.config.ts`
- Create: `src/client/ui/src/lib/__fixtures__/Probe.svelte`
- Create: `src/client/ui/src/lib/smoke.test.ts`

**Interfaces:**
- Produces: a working Vitest + `@testing-library/svelte` + jsdom setup for the ui
  package; `Probe.svelte` (a fixture rendering its `label` prop), reused by Task 2.

- [ ] **Step 1: Install the test dependencies**

Run: `pnpm --filter @shadowcat/ui add -D @testing-library/svelte jsdom`
Expected: `package.json` gains both under `devDependencies`; lockfile updates.

- [ ] **Step 2: Write the Vitest config**

`src/client/ui/vitest.config.ts`:

```ts
import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { svelteTesting } from "@testing-library/svelte/vite";

// Separate from vite.config.ts (the production build): adds the jsdom env and
// @testing-library/svelte's auto-cleanup + browser-condition resolution.
export default defineConfig({
  plugins: [svelte(), svelteTesting()],
  test: {
    environment: "jsdom",
    globals: true,
  },
});
```

- [ ] **Step 3: Update the test script**

In `src/client/ui/package.json`, replace the placeholder `test` script:

```json
    "test": "vitest run",
```

- [ ] **Step 4: Write the fixture + smoke test**

`src/client/ui/src/lib/__fixtures__/Probe.svelte`:

```svelte
<script lang="ts">
  let { label }: { label: string } = $props();
</script>

<span data-testid="probe">{label}</span>
```

`src/client/ui/src/lib/smoke.test.ts`:

```ts
import { render, screen } from "@testing-library/svelte";
import { test, expect } from "vitest";
import Probe from "./__fixtures__/Probe.svelte";

test("the Svelte test harness renders a component", () => {
  render(Probe, { props: { label: "hello" } });
  expect(screen.getByTestId("probe").textContent).toBe("hello");
});
```

- [ ] **Step 5: Run it to verify it passes**

Run: `pnpm --filter @shadowcat/ui exec vitest run src/lib/smoke.test.ts`
Expected: PASS (1 test). If it fails on `createSubscriber`/svelte version or a
missing `@testing-library/svelte/vite` export, resolve the dep versions before
proceeding.

- [ ] **Step 6: Commit**

```bash
git add src/client/ui/package.json src/client/ui/vitest.config.ts \
        src/client/ui/src/lib/__fixtures__/Probe.svelte \
        src/client/ui/src/lib/smoke.test.ts pnpm-lock.yaml
git commit -m "test(ui): Vitest + @testing-library/svelte + jsdom setup"
```

---

### Task 2: `appContext` + `<Surface>` adapter

**Files:**
- Create: `src/client/ui/src/lib/appContext.ts`
- Create: `src/client/ui/src/lib/Surface.svelte`
- Create: `src/client/ui/src/lib/__fixtures__/SurfaceHarness.svelte`
- Create: `src/client/ui/src/lib/Surface.test.ts`

**Interfaces:**
- Consumes: `ContributionRegistry`, `Contribution` from `@shadowcat/core`.
- Produces:
  - `AppContext { contributions: ContributionRegistry }`; `setAppContext(ctx)`;
    `getAppContext(): AppContext` (throws if unset).
  - `Surface` component, prop `{ contract: string }`, renders the contract's
    contributions sorted by `order`, reactively.

- [ ] **Step 1: Write the failing test**

`src/client/ui/src/lib/__fixtures__/SurfaceHarness.svelte` (sets the context, then
renders a `<Surface>` — context must be set by an ancestor during init):

```svelte
<script lang="ts">
  import { ContributionRegistry } from "@shadowcat/core";
  import { setAppContext } from "../appContext";
  import Surface from "../Surface.svelte";

  let { registry, contract }: { registry: ContributionRegistry; contract: string } =
    $props();
  setAppContext({ contributions: registry });
</script>

<Surface {contract} />
```

`src/client/ui/src/lib/Surface.test.ts`:

```ts
import { render, screen } from "@testing-library/svelte";
import { test, expect } from "vitest";
import { ContributionRegistry } from "@shadowcat/core";
import Harness from "./__fixtures__/SurfaceHarness.svelte";
import Probe from "./__fixtures__/Probe.svelte";

test("renders contributions for the contract, sorted by order", () => {
  const registry = new ContributionRegistry();
  registry.contribute({ id: "b", contract: "s:bar", order: 2, component: Probe, props: { label: "B" } });
  registry.contribute({ id: "a", contract: "s:bar", order: 1, component: Probe, props: { label: "A" } });
  registry.contribute({ id: "other", contract: "s:elsewhere", component: Probe, props: { label: "X" } });

  render(Harness, { props: { registry, contract: "s:bar" } });

  const probes = screen.getAllByTestId("probe").map((n) => n.textContent);
  expect(probes).toEqual(["A", "B"]); // order 1 before 2; the other contract excluded
});

test("an empty surface renders nothing", () => {
  const registry = new ContributionRegistry();
  render(Harness, { props: { registry, contract: "s:empty" } });
  expect(screen.queryByTestId("probe")).toBeNull();
});

test("updates reactively when a contribution is added then disposed", async () => {
  const registry = new ContributionRegistry();
  render(Harness, { props: { registry, contract: "s:live" } });
  expect(screen.queryByTestId("probe")).toBeNull();

  const dispose = registry.contribute({ id: "p", contract: "s:live", component: Probe, props: { label: "live" } });
  // Svelte flushes reactive DOM updates on a microtask; findBy awaits it.
  expect((await screen.findByTestId("probe")).textContent).toBe("live");

  dispose();
  await Promise.resolve();
  expect(screen.queryByTestId("probe")).toBeNull();
});
```

- [ ] **Step 2: Run it to verify it fails**

Run: `pnpm --filter @shadowcat/ui exec vitest run src/lib/Surface.test.ts`
Expected: FAIL — `../appContext` and `../Surface.svelte` do not exist.

- [ ] **Step 3: Implement `appContext`**

`src/client/ui/src/lib/appContext.ts`:

```ts
import { getContext, setContext } from "svelte";
import type { ContributionRegistry } from "@shadowcat/core";

/**
 * Ambient app state contributed components read via Svelte context. M7b-3 carries
 * the contribution registry the host renders; M7c adds store/world/role (shell +
 * Welcome) and M7d adds the i18n `t`. Extend this interface there.
 */
export interface AppContext {
  contributions: ContributionRegistry;
}

const KEY = Symbol("shadowcat.appContext");

export function setAppContext(ctx: AppContext): void {
  setContext(KEY, ctx);
}

export function getAppContext(): AppContext {
  const ctx = getContext<AppContext | undefined>(KEY);
  if (!ctx) {
    throw new Error("AppContext is not set; render within a provider that calls setAppContext");
  }
  return ctx;
}
```

- [ ] **Step 4: Implement `<Surface>`**

`src/client/ui/src/lib/Surface.svelte`:

```svelte
<script lang="ts">
  import type { Component } from "svelte";
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "./appContext";

  let { contract }: { contract: string } = $props();

  const { contributions } = getAppContext();

  // Bridge the framework-neutral registry's subscribe/snapshot to Svelte's
  // reactivity: reading `subscribe()` inside the $derived registers a dependency
  // that re-runs whenever the registry emits.
  const subscribe = createSubscriber((update) => {
    const off = contributions.subscribe(update);
    return () => off();
  });

  const items = $derived.by(() => {
    subscribe();
    return contributions.contributionsFor(contract);
  });
</script>

{#each items as item (item.id)}
  {@const Comp = item.component as Component<Record<string, unknown>>}
  <Comp {...(item.props ?? {})} />
{/each}
```

- [ ] **Step 5: Run it to verify it passes**

Run: `pnpm --filter @shadowcat/ui exec vitest run src/lib/Surface.test.ts`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
git add src/client/ui/src/lib/appContext.ts src/client/ui/src/lib/Surface.svelte \
        src/client/ui/src/lib/__fixtures__/SurfaceHarness.svelte \
        src/client/ui/src/lib/Surface.test.ts
git commit -m "feat(ui): <Surface> adapter + appContext (registry host)"
```

---

### Task 3: Typecheck + full green + dependency log

**Files:**
- Modify: `docs/design/ARCHITECTURE.md` (log the two new test devDeps, if the doc
  enumerates dependencies; otherwise skip)

- [ ] **Step 1: Typecheck the ui package**

Run: `pnpm --filter @shadowcat/ui typecheck`
Expected: no errors (svelte-check validates `Surface.svelte`, `appContext.ts`, the
fixtures, and the `Component` cast).

- [ ] **Step 2: Run the full ui suite**

Run: `pnpm --filter @shadowcat/ui test`
Expected: PASS — smoke + 3 Surface tests.

- [ ] **Step 3: Confirm the rest of the workspace is unaffected**

Run: `pnpm --filter @shadowcat/core test`
Expected: PASS (100 tests) — M7b-3 touches only the ui package.

- [ ] **Step 4: Log the dependencies**

If `docs/design/ARCHITECTURE.md` enumerates dependencies, add a row/line:
`@testing-library/svelte + jsdom — ui component testing (MIT) — Vendor`. If no
such enumeration exists, make no change (do not invent a section).

- [ ] **Step 5: Commit (if ARCHITECTURE.md changed)**

```bash
git add docs/design/ARCHITECTURE.md
git commit -m "docs(arch): log ui test dependencies"
```

---

## Self-Review

**Spec coverage (spec §6):**
- Declarative `<Surface contract>` rendering contributions via Svelte 5 dynamic
  components (§6.1) → Task 2. ✓
- `createSubscriber` bridge over the registry's subscribe (§6.1, §7) → Task 2. ✓
- Contribution input via Svelte context (`appContext`) (§6.2) → Task 2 (registry
  field now; store/world/role/t deferred to M7c/M7d per the constraint above). ✓
- Framework neutrality preserved — only `<Surface>` is Svelte; the registry is
  untouched (§6.3) → Task 2 consumes `@shadowcat/core` unchanged. ✓
- Vitest + `@testing-library/svelte` harness (§8) → Task 1. ✓

**Placeholder scan:** No TBD/TODO; every code/test block is complete. The "verify
the svelte version exposes createSubscriber" and "if ARCHITECTURE enumerates deps"
notes are conditional instructions, not placeholders. ✓

**Type consistency:** `AppContext`, `setAppContext`/`getAppContext`,
`ContributionRegistry`, `Contribution`, `Surface`'s `contract` prop, and the
`Probe`/`SurfaceHarness` fixture props are consistent across tasks. `Probe` is
created in Task 1 and reused in Task 2. ✓

## Out of scope (M7c / M7d)

The real shell, the `core-ui` module providing region surfaces, the entry-flow
views, the `WsClient.onWelcome → reconcileTopology` wiring, and the extension of
`AppContext` with `store`/`world`/`role` (M7c) and `t` (M7d) are later
sub-milestones. M7b-3 ships only the `<Surface>` mechanism, `appContext`, and the
test harness.
