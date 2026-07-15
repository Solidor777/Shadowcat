# M12b — Layout Refresh (topbar launcher · statusbar dock strip · mobile tool strip · token re-audit)

**Plan for checkpoint M12b of the M12 dockable-panels track.** Binding spec:
`docs/superpowers/specs/2026-07-13-m12-dockable-panels-default-modules-design.md` §4.4 (shell/region
changes), §8 (single compact axis), §9 (a11y for launcher/tool strip), §13 row M12b, §14 exclusions.
Locked decisions in §2 are NOT re-litigated. Builds on the merged M12a panel manager.

## Branch

Execute on a NEW branch `m12b-layout-refresh` off local `main` (post-M12a merge, HEAD `72f6e5d`).
Do not push (M11/M12 push is the user's call per the autonomous-run directive).

## Global constraints (carry into every task)

- **dockview-core imports stay confined** to `src/modules/panels/src/engine/dockview.ts` (ESLint
  `no-restricted-imports`). No task here touches that file; no new dockview import anywhere.
- **Modules talk only through seams** (contracts / `<Surface>` / `AppContext`). In particular
  **`module-topbar` must NOT import `@shadowcat/module-panels`** — it reaches the panel host solely
  through `AppContext.panels` (`PanelsApi & PanelsChipsView`). Do not add `@shadowcat/module-panels`
  to any topbar/statusbar/core-ui dependency.
- **All UI strings through `t()`** with catalog entries in `src/client/ui-kit/src/locales/en.ts`.
  New components read `ctx.t` (the reactive ui-kit `t`, already locale-reactive — see
  `Table.svelte` wiring). No hardcoded user-facing text.
- **Logging via injectable `Logger`** — no `console.*`. (No new logging is required in this
  checkpoint; the launcher/presence/toolrail changes are pure UI.)
- **Semantic tokens only** — colors from `src/client/shell/src/styles/_semantic.scss`; spacing/radius
  from `_primitives.scss` (`--space-*`, `--radius-*`). No raw hex/`rgb()`/`rgba()` in changed chrome.
- **Keep-mounted panel rule unchanged** — this checkpoint touches shell chrome + panel *defaults*,
  never panel body mount/unmount. The `{#if open}` popups added here are menus, not panel bodies;
  the keep-mounted rule does not apply to transient menus.
- **Per-task verification always includes BOTH** `pnpm --filter <pkg> test` AND
  `pnpm --filter <pkg> typecheck` — vitest strips types via esbuild, so `typecheck` is the only gate
  that catches type errors ([[vitest-skips-typecheck-in-sdd]]).
- **Comments zero-history / present-tense**, invariants-first, no process/ticket meta.
- **Run all commands from repo root** (`C:\Dev\Shadowcat`). Any client build precedes a cargo build
  (rust-embed validates `dist/` at compile time) — relevant only to the e2e task.

## Buddy-check directives

Spec §15 pre-authorizes buddy-checks for **M12a / M12c / M12e only** — **no M12b task is flagged**.
Standing rule for an unflagged task that surfaces a real risk signal during execution (a security
seam, a non-obvious control-flow change, a data-loss path): **auto-upgrade to a buddy-check** per the
autonomous-run directive, then continue. None is anticipated: every M12b change is presentation +
default-data + a client-advisory `gmOnly` filter already owned by the panel controller.

## Orientation (verified against current source)

- **The core-ui grid already reads `topbar / toolrail / main / statusbar`** (`Layout.svelte`). M12b's
  grid work is therefore: (a) replace the ad-hoc `@media (max-width: 40rem)` query with the ui-kit
  `sizeClass()` axis (48rem); (b) raise the statusbar row `1.5rem → 2rem`; (c) on compact, render the
  toolrail as a full-width bottom strip instead of `display:none`. The `.main` cell keeps its
  `min-height:0; overflow:hidden` growth cap (the 1fr-track cap that keeps tall content scrolling
  inside the panel host — `PanelHost` owns the inner scroll); this satisfies the spec's "growth cap
  stays with the panel host's panes" — it is already so.
- **`AppContext.panels`** (`PanelsApi & PanelsChipsView`, `src/client/ui-kit/src/panelsBridge.svelte.ts`)
  exposes `open/close/focus/toggle(id)` plus a live `metaMap: ReadonlyMap<string, PanelMeta>` and
  `minimized`. `metaMap` is **already gmOnly-filtered by `PanelsController.regsForRole`** and is
  `$state`-backed (unfreezes once the host binds; tracks install/uninstall). The launcher reads
  `metaMap` for its item list and calls `toggle(id)` — **no new seam**.
- **Panel contribution ids** (used as layout ids): `chat:panel`, `assets:panel`, `actors:panel`,
  `factions:panel`, `conditions:panel`, `game-settings:panel` (gmOnly), `settings:panel`.
- **`PanelsController.toggle(id)`**: minimized/closed ⇒ `open` (opens at the panel's own
  `defaultPlacement`, falling back to a new docked group in `right` when the panel has no
  `defaultPlacement`); docked/floating ⇒ `minimize`. The launcher uses `toggle`. Consequence: a
  launcher-closed panel opens docked-right on first click and minimizes (statusbar chip) on the next
  — the dock-chip metaphor, consistent with §4.4.
- **`defaultLayout`/`placeNewRegistrations` (`layout/tree.ts`)**: a registration with **no**
  `defaultPlacement` and no persisted source is added to `compact.order` only and placed in NONE of
  the expanded locations — i.e. **launcher-closed = ABSENT from `expanded`** (docked/floating/
  minimized), still present in the compact switcher order. This is exactly the defaults-flip target
  and is already pinned by the tree.test.ts case *"leaves a placement-less registration closed …"*.
- **`members: SvelteMap<string,string>`** (userId→username) is on `AppContext`, populated for EVERY
  role (M11d-1), reactive/in-place-mutated. The presence roster reads it directly.
- **Scene title deferral (resolved ambiguity, see below):** `SceneSystem` carries **no name field**
  (confirmed `scene-docs.ts`); multi-scene + scene naming are M12d. M12b's "world/scene title"
  therefore renders the **world title only**; the scene half is deferred to M12d.
- Test harness: `setAppContextForTest(over)` (`@shadowcat/ui-kit/test`) seeds a minimal `AppContext`
  with `t: (k) => k` (assert on i18n KEYS, not English) and overridable `panels`/`members`/`role`.
  Both `module-topbar` and `module-core-ui` run vitest under `jsdom` with testing-library; jsdom has
  no `matchMedia`, so `sizeClass()` returns `"expanded"` there (compact behavior is e2e-only).

## Resolved spec ambiguities (flagged for the dispatcher)

1. **"world/scene title" with no scene name field.** `SceneSystem` has no `name`; scene naming +
   multi-scene are M12d. **Resolution:** render the world title now (`t("topbar.world",{world})`);
   scope the scene-name half to M12d. No blocker.
2. **"registered panels + overflow menu" vs the single-axis mandate.** A width-measured inline/overflow
   split cannot be done in pure CSS without JS measurement and would fight the single 48rem axis.
   **Resolution:** realize the launcher as ONE WAI-ARIA menu (an "apps" trigger opening a menu that
   lists every gmOnly-filtered panel) — the menu IS the registered-panel list AND the overflow
   container, identical at all widths (the a11y path, the touch path, the overflow). The topbar's
   world title + presence + settings reflow via `sizeClass` (labels collapse on compact). This is the
   minimal correct solution honoring "no new seam" and the single-axis rule.
3. **Launcher verb.** §4.4 says "toggles via `toggle`/`open`." **Resolution:** menu items call
   `ctx.panels.toggle(id)` (open-if-hidden / minimize-if-shown), matching the controller contract and
   the dock-chip metaphor. The settings entry also uses `toggle("settings:panel")`.

None require escalation.

---

## Task 1 — core-ui Layout: single sizeClass axis, 2rem statusbar, compact bottom tool strip

**Files:** `src/modules/core-ui/src/Layout.svelte` (rewrite),
`src/modules/core-ui/src/Layout.test.ts` (new).

**Step 1 — write the failing test.** Create `src/modules/core-ui/src/Layout.test.ts`:

```ts
import { test, expect, afterEach } from "vitest";
import { render, cleanup } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import Layout from "./Layout.svelte";

afterEach(() => cleanup());

test("renders the four region cells inside the layout grid", () => {
  const { container } = render(Layout, { context: setAppContextForTest() });
  const layout = container.querySelector(".layout");
  expect(layout).toBeTruthy();
  expect(container.querySelector(".topbar")).toBeTruthy();
  expect(container.querySelector(".toolrail")).toBeTruthy();
  expect(container.querySelector(".main")).toBeTruthy();
  expect(container.querySelector(".statusbar")).toBeTruthy();
});

// jsdom has no matchMedia, so `sizeClass()` resolves to "expanded"; the compact
// grid (bottom tool strip) is asserted by the e2e viewport test, not here.
test("defaults to the expanded grid (no compact class) under jsdom", () => {
  const { container } = render(Layout, { context: setAppContextForTest() });
  expect(container.querySelector(".layout")?.classList.contains("compact")).toBe(false);
});
```

**Step 2 — rewrite `Layout.svelte` to make it pass:**

```svelte
<script lang="ts">
  import { Surface, sizeClass } from "@shadowcat/ui-kit";

  // Single breakpoint axis (ui-kit `sizeClass`, 48rem) — the only source of
  // truth for compact/expanded. Replaces the removed 40rem media query so the
  // toolrail-hide threshold and the panel host's compact switcher flip together.
  const compact = $derived(sizeClass() === "compact");
</script>

<div class="layout" class:compact>
  <div class="topbar"><Surface contract="shadowcat.surface:topbar" /></div>
  <div class="toolrail"><Surface contract="shadowcat.surface:toolrail" /></div>
  <div class="main"><Surface contract="shadowcat.surface:panel-host" /></div>
  <div class="statusbar"><Surface contract="shadowcat.surface:statusbar" /></div>
</div>

<style lang="scss">
  .layout {
    display: grid;
    height: 100vh;
    grid-template-columns: 3rem 1fr;
    grid-template-rows: 2.5rem 1fr 2rem;
    grid-template-areas:
      "topbar topbar"
      "toolrail main"
      "statusbar statusbar";
    background: var(--surface-base);
    color: var(--text-primary);
  }
  .topbar {
    grid-area: topbar;
    background: var(--surface-raised);
    border-bottom: 1px solid var(--border);
  }
  .toolrail {
    grid-area: toolrail;
    background: var(--surface-overlay);
    border-right: 1px solid var(--border);
  }
  .main {
    grid-area: main;
    /* Growth cap: zeroes the grid item's automatic minimum size so tall panel
     * content scrolls inside the panel host's panes instead of growing the 1fr
     * track past 100vh. Inner scrolling is owned by the panel host. */
    min-height: 0;
    overflow: hidden;
  }
  .statusbar {
    grid-area: statusbar;
    background: var(--surface-overlay);
    border-top: 1px solid var(--border);
    color: var(--text-muted);
    font-size: 0.8rem;
  }

  /* Compact (<48rem): single column; the toolrail becomes a full-width bottom
   * tool strip (an `auto` row that collapses to 0 when the GM-gated rail renders
   * nothing) instead of being hidden — real mobile tooling per spec §4.4/§8. */
  .layout.compact {
    grid-template-columns: 1fr;
    grid-template-rows: 2.5rem 1fr auto 2rem;
    grid-template-areas:
      "topbar"
      "main"
      "toolrail"
      "statusbar";
  }
  .layout.compact .toolrail {
    border-right: none;
    border-top: 1px solid var(--border);
  }
</style>
```

**Verify:**
```
pnpm --filter @shadowcat/module-core-ui test
pnpm --filter @shadowcat/module-core-ui typecheck
```

**Done when:** both pass; `Layout.svelte` has no `@media` query; statusbar row is `2rem`; compact
grid places `toolrail` as a bottom `auto` row and no longer hides it.

---

## Task 2 — Defaults flip: non-chat panels launcher-closed

The six non-chat panel modules currently default to `{ kind: "minimized" }` (statusbar chips). Flip
them to launcher-closed by **removing `defaultPlacement` entirely** (absent ⇒ launcher-only/closed,
per `DefaultPlacement`'s own doc comment). Chat stays docked right.

**Files (each an Edit to the `panel:` metadata + its `index.test.ts` expectation):**
`src/modules/{assets,actors,factions,conditions,game-settings,settings}/src/index.ts` and the matching
`index.test.ts`.

**Step 1 — flip each module's registration.** Apply these exact edits:

- `src/modules/assets/src/index.ts`:
  `panel: { icon: "🖼️", labelKey: "assets.tab", defaultPlacement: { kind: "minimized" } },`
  → `panel: { icon: "🖼️", labelKey: "assets.tab" },`
- `src/modules/actors/src/index.ts`:
  `panel: { icon: "👥", labelKey: "actors.tab", defaultPlacement: { kind: "minimized" } },`
  → `panel: { icon: "👥", labelKey: "actors.tab" },`
- `src/modules/factions/src/index.ts`:
  `panel: { icon: "🚩", labelKey: "factions.tab", defaultPlacement: { kind: "minimized" } },`
  → `panel: { icon: "🚩", labelKey: "factions.tab" },`
- `src/modules/conditions/src/index.ts`:
  `panel: { icon: "✨", labelKey: "conditions.tab", defaultPlacement: { kind: "minimized" } },`
  → `panel: { icon: "✨", labelKey: "conditions.tab" },`
- `src/modules/game-settings/src/index.ts`:
  `panel: { icon: "⚙️", labelKey: "gameSettings.tab", gmOnly: true, defaultPlacement: { kind: "minimized" } },`
  → `panel: { icon: "⚙️", labelKey: "gameSettings.tab", gmOnly: true },`
- `src/modules/settings/src/index.ts`:
  `panel: { icon: "🔧", labelKey: "settings.tab", defaultPlacement: { kind: "minimized" } },`
  → `panel: { icon: "🔧", labelKey: "settings.tab" },`

**Step 2 — update each module's `index.test.ts`** deep-equal expectation to drop the removed key.
Apply these exact `toEqual({...})` edits:

- `src/modules/assets/src/index.test.ts`:
  ```ts
    expect(list[0].panel).toEqual({
      icon: "🖼️",
      labelKey: "assets.tab",
    });
  ```
- `src/modules/actors/src/index.test.ts`:
  ```ts
    expect(list[0].panel).toEqual({
      icon: "👥",
      labelKey: "actors.tab",
    });
  ```
- `src/modules/factions/src/index.test.ts`:
  ```ts
    expect(list[0].panel).toEqual({
      icon: "🚩",
      labelKey: "factions.tab",
    });
  ```
- `src/modules/conditions/src/index.test.ts`:
  ```ts
    expect(list[0].panel).toEqual({
      icon: "✨",
      labelKey: "conditions.tab",
    });
  ```
- `src/modules/game-settings/src/index.test.ts`:
  ```ts
    expect(list[0].panel).toEqual({
      icon: "⚙️",
      labelKey: "gameSettings.tab",
      gmOnly: true,
    });
  ```
- `src/modules/settings/src/index.test.ts`:
  ```ts
    expect(list[0].panel).toEqual({
      icon: "🔧",
      labelKey: "settings.tab",
    });
  ```

> Read each `index.test.ts` first to match its exact surrounding lines; the deep-equal block above
> replaces only the object literal passed to `toEqual`. If any module's test asserts the panel a
> different way (e.g. field-by-field), drop only the `defaultPlacement` assertion.

> **No change to `tree.test.ts` / `controller.test.ts`:** those exercise the reducer with SYNTHETIC
> regs (`a:panel`/`b:panel`, explicit placements) — mechanism tests independent of module defaults;
> `{ kind: "minimized" }` remains a valid `DefaultPlacement` the reducer must still support.

**Verify:**
```
pnpm --filter @shadowcat/module-assets test && pnpm --filter @shadowcat/module-assets typecheck
pnpm --filter @shadowcat/module-actors test && pnpm --filter @shadowcat/module-actors typecheck
pnpm --filter @shadowcat/module-factions test && pnpm --filter @shadowcat/module-factions typecheck
pnpm --filter @shadowcat/module-conditions test && pnpm --filter @shadowcat/module-conditions typecheck
pnpm --filter @shadowcat/module-game-settings test && pnpm --filter @shadowcat/module-game-settings typecheck
pnpm --filter @shadowcat/module-settings test && pnpm --filter @shadowcat/module-settings typecheck
```

**Done when:** all six modules build and their index tests pass with `defaultPlacement` removed; chat
is unchanged (still docked right).

---

## Task 3 — Topbar launcher menu (LauncherMenu.svelte)

The launcher: a WAI-ARIA menu button ("apps") opening a menu of every gmOnly-filtered registered
panel; each item toggles its panel via `ctx.panels.toggle(id)`. Keyboard + touch accessible.

**Files:** `src/modules/topbar/src/LauncherMenu.svelte` (new),
`src/modules/topbar/src/LauncherMenu.test.ts` (new),
`src/client/ui-kit/src/locales/en.ts` (add `topbar.launcher`).

**Step 1 — add the locale key.** In `src/client/ui-kit/src/locales/en.ts`, immediately after the
`"topbar.world": "world {world}",` line, add:

```ts
  "topbar.launcher": "Panels",
  "topbar.presence": "Players",
```

(`topbar.presence` is consumed in Task 4; adding both here keeps the catalog edit to one place.)

**Step 2 — write the failing test** `src/modules/topbar/src/LauncherMenu.test.ts`:

```ts
import { test, expect, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { PanelsBridge } from "@shadowcat/ui-kit";
import { silentLogger, type PanelMeta } from "@shadowcat/core";
import type { PanelsApi, PanelsChipsView } from "@shadowcat/ui-kit";
import LauncherMenu from "./LauncherMenu.svelte";

afterEach(() => cleanup());

/** A bound bridge whose fake impl records toggle calls and exposes a fixed
 * metaMap — no `module-panels` import (seam boundary). */
function bridgeWith(meta: [string, PanelMeta][]): { bridge: PanelsBridge; toggles: string[] } {
  const toggles: string[] = [];
  const bridge = new PanelsBridge(silentLogger);
  const impl: PanelsApi & PanelsChipsView = {
    open: () => {},
    close: () => {},
    focus: () => {},
    toggle: (id) => toggles.push(id),
    restore: () => {},
    minimized: [],
    metaMap: new Map(meta),
  };
  bridge.bind(impl);
  return { bridge, toggles };
}

const META: [string, PanelMeta][] = [
  ["chat:panel", { icon: "💬", labelKey: "chat.tab" }],
  ["assets:panel", { icon: "🖼️", labelKey: "assets.tab" }],
];

test("the launcher is closed until its trigger is activated", () => {
  const { bridge } = bridgeWith(META);
  render(LauncherMenu, { context: setAppContextForTest({ panels: bridge }) });
  expect(screen.getByTestId("launcher-trigger").getAttribute("aria-expanded")).toBe("false");
  expect(screen.queryByTestId("launcher-menu")).toBeNull();
});

test("opening lists every gmOnly-filtered panel from metaMap as a menuitem", async () => {
  const { bridge } = bridgeWith(META);
  render(LauncherMenu, { context: setAppContextForTest({ panels: bridge }) });
  await fireEvent.click(screen.getByTestId("launcher-trigger"));
  const menu = screen.getByTestId("launcher-menu");
  expect(menu.getAttribute("role")).toBe("menu");
  expect(screen.getByTestId("launcher-item-chat:panel").getAttribute("role")).toBe("menuitem");
  expect(screen.getByTestId("launcher-item-assets:panel")).toBeTruthy();
  expect(screen.getByTestId("launcher-trigger").getAttribute("aria-expanded")).toBe("true");
});

test("activating an item toggles that panel through the bridge and closes the menu", async () => {
  const { bridge, toggles } = bridgeWith(META);
  render(LauncherMenu, { context: setAppContextForTest({ panels: bridge }) });
  await fireEvent.click(screen.getByTestId("launcher-trigger"));
  await fireEvent.click(screen.getByTestId("launcher-item-assets:panel"));
  expect(toggles).toEqual(["assets:panel"]);
  expect(screen.queryByTestId("launcher-menu")).toBeNull();
});

test("Escape on a menu item closes the menu (keyboard path)", async () => {
  const { bridge } = bridgeWith(META);
  render(LauncherMenu, { context: setAppContextForTest({ panels: bridge }) });
  await fireEvent.click(screen.getByTestId("launcher-trigger"));
  await fireEvent.keyDown(screen.getByTestId("launcher-item-chat:panel"), { key: "Escape" });
  expect(screen.queryByTestId("launcher-menu")).toBeNull();
});
```

**Step 3 — implement `src/modules/topbar/src/LauncherMenu.svelte`:**

```svelte
<script lang="ts">
  import { getAppContext, sizeClass } from "@shadowcat/ui-kit";

  const ctx = getAppContext();
  const t = ctx.t;
  const compact = $derived(sizeClass() === "compact");

  // Registered panels in metaMap order — already gmOnly-filtered by the bound
  // PanelsController (the host is the one place role filtering happens). `$state`-
  // backed on the bridge, so this unfreezes once the panel host binds and tracks
  // module install/uninstall.
  const panels = $derived([...ctx.panels.metaMap.entries()].map(([id, meta]) => ({ id, meta })));

  let open = $state(false);
  let triggerEl: HTMLButtonElement;
  let itemEls: HTMLButtonElement[] = [];

  function openMenu(): void {
    itemEls = [];
    open = true;
    // Focus the first item after Svelte binds the freshly-rendered menu.
    queueMicrotask(() => itemEls[0]?.focus());
  }
  function closeMenu(returnFocus = true): void {
    open = false;
    if (returnFocus) queueMicrotask(() => triggerEl?.focus());
  }
  function activate(id: string): void {
    ctx.panels.toggle(id);
    closeMenu();
  }
  function focusItem(index: number): void {
    const n = itemEls.length;
    if (n === 0) return;
    itemEls[((index % n) + n) % n]?.focus();
  }
  function onItemKeydown(event: KeyboardEvent, index: number): void {
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        focusItem(index + 1);
        break;
      case "ArrowUp":
        event.preventDefault();
        focusItem(index - 1);
        break;
      case "Home":
        event.preventDefault();
        focusItem(0);
        break;
      case "End":
        event.preventDefault();
        focusItem(itemEls.length - 1);
        break;
      case "Escape":
        event.preventDefault();
        event.stopPropagation();
        closeMenu();
        break;
      case "Tab":
        // A menu is a closed focus loop while open (WAI-ARIA Menu pattern).
        event.preventDefault();
        closeMenu();
        break;
    }
  }
  function onTriggerKeydown(event: KeyboardEvent): void {
    if (event.key === "ArrowDown" || event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      openMenu();
    }
  }
</script>

<div class="sc-launcher" class:compact>
  <button
    type="button"
    class="sc-launcher-trigger"
    bind:this={triggerEl}
    aria-haspopup="menu"
    aria-expanded={open}
    aria-label={t("topbar.launcher")}
    data-testid="launcher-trigger"
    onclick={() => (open ? closeMenu() : openMenu())}
    onkeydown={onTriggerKeydown}
  >
    <span class="sc-launcher-glyph" aria-hidden="true">☰</span>
    <span class="sc-launcher-label">{t("topbar.launcher")}</span>
  </button>

  {#if open}
    <!-- Outside-pointer dismissal; the menu itself is above this backdrop. -->
    <div
      class="sc-launcher-backdrop"
      aria-hidden="true"
      onpointerdown={() => closeMenu(false)}
    ></div>
    <div
      class="sc-launcher-menu"
      role="menu"
      aria-label={t("topbar.launcher")}
      data-testid="launcher-menu"
    >
      {#each panels as p, i (p.id)}
        <button
          type="button"
          role="menuitem"
          class="sc-launcher-item"
          data-testid="launcher-item-{p.id}"
          bind:this={itemEls[i]}
          onclick={() => activate(p.id)}
          onkeydown={(e) => onItemKeydown(e, i)}
        >
          <span class="sc-launcher-icon" aria-hidden="true">{p.meta.icon}</span>
          <span>{t(p.meta.labelKey)}</span>
        </button>
      {/each}
    </div>
  {/if}
</div>

<style lang="scss">
  .sc-launcher {
    position: relative;
    display: flex;
    align-items: center;
  }
  .sc-launcher-trigger {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    min-height: 44px; /* touch target (mobile invariant); >=24px a11y floor */
    padding: 0 var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: var(--text-primary);
    cursor: pointer;
    &:focus-visible {
      outline: 2px solid var(--accent);
      outline-offset: 2px;
    }
  }
  .sc-launcher.compact .sc-launcher-label {
    /* Compact: icon-only trigger to reclaim topbar width — the single axis. */
    display: none;
  }
  .sc-launcher-backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
  }
  .sc-launcher-menu {
    position: absolute;
    top: calc(100% + var(--space-1));
    left: 0;
    z-index: 41;
    display: flex;
    flex-direction: column;
    min-width: 12rem;
    padding: var(--space-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-2);
    background: var(--surface-overlay);
    box-shadow: var(--shadow-elevated);
  }
  .sc-launcher-item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-height: 36px; /* comfortably above the 24px a11y floor */
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-1);
    background: transparent;
    color: var(--text-primary);
    font-size: 0.9rem;
    text-align: left;
    cursor: pointer;
    &:hover {
      background: var(--surface-base);
    }
    &:focus-visible {
      outline: 2px solid var(--accent);
      outline-offset: -2px;
    }
  }
  .sc-launcher-icon {
    width: 1.25rem;
    text-align: center;
  }
</style>
```

**Verify:**
```
pnpm --filter @shadowcat/ui-kit test
pnpm --filter @shadowcat/module-topbar test
pnpm --filter @shadowcat/module-topbar typecheck
```
(ui-kit test runs to confirm the `en.ts` catalog edit didn't break any i18n snapshot/consumer.)

**Done when:** the four LauncherMenu tests pass; the menu opens/closes, lists metaMap panels as
`role="menuitem"`, toggles through the bridge, and honors Escape; topbar typecheck is clean.

---

## Task 4 — Topbar presence roster (Presence.svelte)

**Files:** `src/modules/topbar/src/Presence.svelte` (new),
`src/modules/topbar/src/Presence.test.ts` (new). (Locale key `topbar.presence` was added in Task 3.)

**Step 1 — write the failing test** `src/modules/topbar/src/Presence.test.ts`:

```ts
import { test, expect, afterEach } from "vitest";
import { render, screen, cleanup } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import Presence from "./Presence.svelte";

afterEach(() => cleanup());

test("renders a badge per world member (available to every role)", () => {
  const members = new Map([
    ["u1", "Ada"],
    ["u2", "Bo"],
  ]);
  render(Presence, { context: setAppContextForTest({ role: "player", members }) });

  const roster = screen.getByTestId("presence");
  expect(roster.getAttribute("role")).toBe("group");

  const a = screen.getByTestId("presence-u1");
  expect(a.getAttribute("title")).toBe("Ada");
  expect(a.getAttribute("aria-label")).toBe("Ada");
  expect(a.textContent?.trim()).toBe("A");

  expect(screen.getByTestId("presence-u2").getAttribute("title")).toBe("Bo");
});

test("renders an empty roster group when there are no members", () => {
  render(Presence, { context: setAppContextForTest({ members: new Map() }) });
  expect(screen.getByTestId("presence").children.length).toBe(0);
});
```

**Step 2 — implement `src/modules/topbar/src/Presence.svelte`:**

```svelte
<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";

  const ctx = getAppContext();
  const t = ctx.t;

  // `members` is a reactive SvelteMap (userId -> username), populated for every
  // role (M11d-1). Reading it here tracks join/leave updates in place.
  const roster = $derived([...ctx.members.entries()].map(([id, name]) => ({ id, name })));

  function initial(name: string): string {
    return name.trim().charAt(0).toUpperCase() || "?";
  }
</script>

<div class="sc-presence" role="group" aria-label={t("topbar.presence")} data-testid="presence">
  {#each roster as m (m.id)}
    <span
      class="sc-presence-badge"
      title={m.name}
      aria-label={m.name}
      data-testid="presence-{m.id}">{initial(m.name)}</span>
  {/each}
</div>

<style lang="scss">
  .sc-presence {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    overflow: hidden;
  }
  .sc-presence-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.5rem;
    height: 1.5rem;
    border-radius: 999px;
    background: var(--surface-overlay);
    border: 1px solid var(--border);
    color: var(--text-primary);
    font-size: 0.75rem;
    line-height: 1;
    flex: 0 0 auto;
  }
</style>
```

**Verify:**
```
pnpm --filter @shadowcat/module-topbar test
pnpm --filter @shadowcat/module-topbar typecheck
```

**Done when:** both Presence tests pass and topbar typecheck is clean.

---

## Task 5 — TopBar integration: launcher + world title + presence + settings entry

**Files:** `src/modules/topbar/src/TopBar.svelte` (rewrite),
`src/modules/topbar/src/TopBar.test.ts` (new).

**Step 1 — write the failing test** `src/modules/topbar/src/TopBar.test.ts`:

```ts
import { test, expect, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { PanelsBridge } from "@shadowcat/ui-kit";
import { silentLogger } from "@shadowcat/core";
import type { PanelsApi, PanelsChipsView } from "@shadowcat/ui-kit";
import TopBar from "./TopBar.svelte";

afterEach(() => cleanup());

function boundBridge(): { bridge: PanelsBridge; toggles: string[] } {
  const toggles: string[] = [];
  const bridge = new PanelsBridge(silentLogger);
  const impl: PanelsApi & PanelsChipsView = {
    open: () => {},
    close: () => {},
    focus: () => {},
    toggle: (id) => toggles.push(id),
    restore: () => {},
    minimized: [],
    metaMap: new Map([["chat:panel", { icon: "💬", labelKey: "chat.tab" }]]),
  };
  bridge.bind(impl);
  return { bridge, toggles };
}

test("shows the launcher, the world title, presence, and a settings entry", () => {
  const { bridge } = boundBridge();
  render(TopBar, {
    context: setAppContextForTest({ world: "Rivertown", panels: bridge, members: new Map([["u1", "Ada"]]) }),
  });
  expect(screen.getByTestId("launcher-trigger")).toBeTruthy();
  expect(screen.getByTestId("presence")).toBeTruthy();
  expect(screen.getByTestId("topbar-settings")).toBeTruthy();
  // World title text uses the topbar.world key (test `t` echoes keys).
  expect(screen.getByTestId("topbar-title").textContent).toContain("topbar.world");
});

test("the settings entry toggles the settings panel through the bridge", async () => {
  const { bridge, toggles } = boundBridge();
  render(TopBar, { context: setAppContextForTest({ panels: bridge }) });
  await fireEvent.click(screen.getByTestId("topbar-settings"));
  expect(toggles).toEqual(["settings:panel"]);
});
```

**Step 2 — rewrite `src/modules/topbar/src/TopBar.svelte`:**

```svelte
<script lang="ts">
  import { getAppContext, sizeClass } from "@shadowcat/ui-kit";
  import LauncherMenu from "./LauncherMenu.svelte";
  import Presence from "./Presence.svelte";

  const ctx = getAppContext();
  const { world, t } = ctx;
  const compact = $derived(sizeClass() === "compact");

  // The settings panel is a registered panel; the topbar's settings entry is a
  // stable, standard-location toggle for it (no new seam).
  const SETTINGS_PANEL_ID = "settings:panel";
  function toggleSettings(): void {
    ctx.panels.toggle(SETTINGS_PANEL_ID);
  }
</script>

<header class="topbar" class:compact>
  <LauncherMenu />

  <!-- World title. Scene title is deferred to M12d (scene docs carry no name yet). -->
  <div class="title" data-testid="topbar-title">
    <strong class="app">{t("app.name")}</strong>
    <span class="world">{t("topbar.world", { world })}</span>
  </div>

  <div class="spacer"></div>

  <Presence />

  <button
    type="button"
    class="settings-entry"
    data-testid="topbar-settings"
    aria-label={t("settings.tab")}
    title={t("settings.tab")}
    onclick={toggleSettings}
  >
    <span aria-hidden="true">🔧</span>
  </button>
</header>

<style lang="scss">
  .topbar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 0 var(--space-3);
    height: 100%;
  }
  .title {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    min-width: 0; /* let the world label truncate rather than push the row */
    overflow: hidden;
  }
  .title .app {
    white-space: nowrap;
  }
  .world {
    color: var(--text-muted);
    font-size: 0.875rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  /* Compact: drop the world label; the app name + launcher + presence stay. */
  .topbar.compact .world {
    display: none;
  }
  .spacer {
    flex: 1 1 auto;
  }
  .settings-entry {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 44px;
    min-height: 44px; /* touch target */
    padding: 0 var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: var(--text-primary);
    cursor: pointer;
    &:focus-visible {
      outline: 2px solid var(--accent);
      outline-offset: 2px;
    }
  }
</style>
```

**Verify:**
```
pnpm --filter @shadowcat/module-topbar test
pnpm --filter @shadowcat/module-topbar typecheck
```

**Done when:** both TopBar tests pass; the topbar renders launcher + title + presence + settings; the
settings entry toggles `settings:panel`; typecheck is clean.

---

## Task 6 — Mobile tool strip: ToolRail compact presentation

The rail already sits behind `SceneToolHost` (its controller) and renders GM-gated tool buttons.
M12b makes it lay out horizontally on compact (the bottom strip Task 1's grid positions), replacing
the old `display:none`-on-phones behavior. Presentation-only.

**Files:** `src/modules/scene-tools/src/ToolRail.svelte` (edits),
`src/modules/scene-tools/src/ToolRail.test.ts` (add one test).

**Step 1 — add the size-class import.** In `ToolRail.svelte`'s `<script>`, change:

```ts
  import { getAppContext } from "@shadowcat/ui-kit";
```
to:
```ts
  import { getAppContext, sizeClass } from "@shadowcat/ui-kit";
```

and, immediately after the `const isGm = ctx.role === "gm";` line, add:

```ts
  // Compact: the rail renders as a horizontal bottom strip (core-ui repositions
  // it into the compact grid's bottom row); expanded: a vertical side rail.
  const compact = $derived(sizeClass() === "compact");
```

**Step 2 — bind the class on the rail root.** Change:

```svelte
  <div class="tool-rail" role="toolbar" aria-label={t("tools.title")}>
```
to:
```svelte
  <div class="tool-rail" class:compact role="toolbar" aria-label={t("tools.title")}>
```

**Step 3 — add the compact style rules.** Append inside the `.tool-rail` block's `<style>` (after the
existing `.controls select, .controls input { min-height: 32px; }` rule), these new rules:

```scss
  /* Compact bottom strip: lay tools out horizontally with overflow scroll
   * instead of a vertical column; the active-tool controls follow suit. */
  .tool-rail.compact {
    flex-direction: row;
    flex-wrap: nowrap;
    align-items: center;
    overflow-x: auto;
  }
  .tool-rail.compact .controls {
    flex-direction: row;
    align-items: center;
  }
```

**Step 4 — add a regression test** to `src/modules/scene-tools/src/ToolRail.test.ts` (append; reuse the
file's existing `captureScene`/render idiom). Under jsdom `sizeClass()` is `"expanded"`, so this pins
the default (non-compact) shape; the compact layout is asserted by the e2e viewport test:

```ts
test("the tool rail renders as a non-compact side rail under jsdom (expanded default)", () => {
  const { scene } = captureScene();
  const { container } = render(ToolRail, { context: setAppContextForTest({ role: "gm", scene }) });
  const rail = container.querySelector(".tool-rail");
  expect(rail).toBeTruthy();
  expect(rail?.classList.contains("compact")).toBe(false);
});
```

> If `ToolRail.test.ts` does not already import `render`'s `container` destructure or
> `setAppContextForTest`, both are already used at the top of that file — reuse them as-is.

**Verify:**
```
pnpm --filter @shadowcat/module-scene-tools test
pnpm --filter @shadowcat/module-scene-tools typecheck
```

**Done when:** scene-tools tests pass (including the new one); the rail carries `class:compact` and
its compact rules; no `display:none` on the rail anywhere.

---

## Task 7 — Token / density re-audit (bounded)

Verify the new/changed chrome uses semantic color tokens (`_semantic.scss`) and `--space-*`/`--radius-*`
primitives only — no raw hex/`rgb()`/`rgba()` — and add a semantic token only if a genuinely new color
role is missing (none is anticipated; surfaces/border/text/accent/shadow already cover the launcher,
2rem statusbar, and tool strip).

**Files audited:** `src/modules/topbar/src/{TopBar,LauncherMenu,Presence}.svelte`,
`src/modules/core-ui/src/Layout.svelte`, `src/modules/scene-tools/src/ToolRail.svelte`,
`src/modules/statusbar/src/StatusBar.svelte`. **Reference:** `src/client/shell/src/styles/_semantic.scss`,
`src/client/shell/src/styles/_primitives.scss`.

**Step 1 — raw-color scan (acceptance gate).** From repo root:

```
grep -rnE "#[0-9a-fA-F]{3,8}\b|rgba?\(" \
  src/modules/topbar/src/TopBar.svelte \
  src/modules/topbar/src/LauncherMenu.svelte \
  src/modules/topbar/src/Presence.svelte \
  src/modules/core-ui/src/Layout.svelte \
  src/modules/scene-tools/src/ToolRail.svelte \
  src/modules/statusbar/src/StatusBar.svelte
```

This MUST return no matches. (The code in Tasks 1–6 already uses only `var(--…)` colors; this gate
proves it and catches any drift introduced during execution.)

**Step 2 — token-existence check.** Confirm every `var(--…)` referenced by the audited files resolves
to a token defined in `_semantic.scss` (colors/shadow: `--surface-base|raised|overlay`, `--border`,
`--text-primary|muted`, `--accent`, `--shadow-elevated`) or `_primitives.scss` (`--space-1|2|3`,
`--radius-1|2`). All values used in Tasks 1–6 are from those sets; **no new token is required**. If
execution discovered a genuinely missing SEMANTIC color role (e.g. a menu-hover surface distinct from
`--surface-base`), add it to `_semantic.scss` aliasing an existing `_primitives.scss` slate and cite
the role in a one-line comment — do NOT introduce a raw color.

**Step 3 — density sanity.** Confirm interactive targets in the new chrome meet the touch floor:
launcher trigger / settings entry `min-height: 44px`; launcher menu items `min-height: 36px`
(>24px a11y floor). These are set in Tasks 3/5; this step is a read-through confirmation, no edit.

**Verify:**
```
pnpm --filter @shadowcat/module-topbar test && pnpm --filter @shadowcat/module-topbar typecheck
pnpm --filter @shadowcat/module-core-ui typecheck
pnpm --filter @shadowcat/module-scene-tools typecheck
pnpm lint
```

**Done when:** the raw-color scan is empty; every referenced token exists; `pnpm lint` is clean.
State explicitly in the task report whether any semantic token was added (expected: none).

---

## Task 8 — e2e: launcher open/close flow + compact-mode axis + persistence rewrite

The existing `panels.spec.ts` persistence test restores `assets:panel` via a statusbar chip — but
after Task 2, assets is launcher-closed (no chip). Rewrite that test to drive the launcher, and add
launcher open/close + compact assertions.

**Scope amendment (dispatcher, from the Task 2 code review):** `stage.spec.ts` and
`assets.spec.ts` ALSO depend on the removed chips-by-default state (they click
`chip-assets:panel` / `chip-settings:panel` / `chip-actors:panel` on a fresh world). Repair both
in this task with the same substitution: open each needed panel via the topbar launcher
(`launcher` trigger → menu item, Tasks 3–5) instead of a dock chip, and update any
"starts minimized (M12a interim default)" comments to the launcher-closed reality. All three
spec files must be green in this task's e2e run — mid-branch e2e redness for these three files
is acknowledged and sequenced between Task 2 and this task; nothing else may stay red.

**File:** `src/client/shell/e2e/panels.spec.ts` (rewrite); `src/client/shell/e2e/stage.spec.ts`
and `src/client/shell/e2e/assets.spec.ts` (chip-click → launcher-path repairs).

**Replace the entire file with:**

```ts
import { test, expect } from "@playwright/test";

async function enterFreshWorld(page: import("@playwright/test").Page, name: string): Promise<void> {
  await page.goto("/");
  await page.getByLabel("Username").fill("ops");
  await page.getByLabel("Password").fill("pw-boot");
  await page.getByRole("button", { name: "Log in" }).click();
  await page.getByLabel("New world name").fill(name);
  await page.getByRole("button", { name: "Create world" }).click();
  await expect(page.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", {
    timeout: 30_000,
  });
}

// Task 2 default: non-chat panels are launcher-closed (absent from the layout),
// not minimized chips. Assets opens from the topbar launcher, docks right on
// first toggle, and survives a full reload (the M12a persisted-source path).
test("a panel opened from the launcher docks and survives a full page reload", async ({ page }) => {
  await enterFreshWorld(page, "Launcher Persistence World");

  const uploadInput = page.getByTestId("asset-upload");
  // Launcher-closed: assets content is not mounted-visible, and there is no chip.
  await expect(uploadInput).not.toBeVisible();
  await expect(page.getByTestId("chip-assets:panel")).toHaveCount(0);

  // Open it from the topbar launcher menu.
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-assets:panel").click();
  await expect(uploadInput).toBeVisible();

  // Full reload re-runs module registration/activation from scratch.
  await page.reload();
  await expect(page.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", {
    timeout: 30_000,
  });

  // The docked assets panel survives instead of reverting to launcher-closed.
  await expect(page.getByTestId("asset-upload")).toBeVisible();
});

// Toggling the same launcher item again minimizes the (now docked) panel — the
// dock-chip metaphor: it hides the body and drops a statusbar restore chip.
test("re-toggling a launcher item minimizes the open panel to a dock chip", async ({ page }) => {
  await enterFreshWorld(page, "Launcher Toggle World");

  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-assets:panel").click();
  await expect(page.getByTestId("asset-upload")).toBeVisible();

  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-assets:panel").click();
  await expect(page.getByTestId("asset-upload")).not.toBeVisible();
  await expect(page.getByTestId("chip-assets:panel")).toHaveCount(1);
});

// Single breakpoint axis (48rem): a narrow viewport puts the layout into compact
// (grid switch + bottom tool strip); a wide viewport does not. Directly pins the
// ui-kit sizeClass axis replacing the old 40rem media query.
test("the layout grid keys compact/expanded off the single 48rem axis", async ({ page }) => {
  await page.setViewportSize({ width: 1200, height: 900 });
  await enterFreshWorld(page, "Compact Axis World");

  await expect(page.locator(".layout")).not.toHaveClass(/\bcompact\b/);

  await page.setViewportSize({ width: 500, height: 900 });
  await expect(page.locator(".layout")).toHaveClass(/\bcompact\b/);

  // The launcher remains reachable on compact (icon-only trigger).
  await expect(page.getByTestId("launcher-trigger")).toBeVisible();

  // Scope amendment (dispatcher, from the Task 6 code review): the tool rail's
  // compact path is untestable under jsdom, so THIS test is its only automated
  // coverage. This block requires a GM session (enterFreshWorld creates the world
  // as GM, so the rail renders). The rail must carry the compact class and its
  // tool buttons must remain reachable in the horizontal strip.
  await expect(page.locator(".tool-rail")).toHaveClass(/\bcompact\b/);
  const firstTool = page.locator(".tool-rail .tool").first();
  await firstTool.scrollIntoViewIfNeeded();
  await expect(firstTool).toBeVisible();
});
```

**Verify (client build precedes the cargo build — rust-embed):**
```
pnpm --filter @shadowcat/shell e2e
```
(`e2e` = `vite build && cargo build -p shadowcat --bin shadowcat && playwright test`.)

**Done when:** all three e2e specs pass against the built binary: launcher open→dock→reload survival,
re-toggle→minimize-to-chip, and the compact/expanded axis flip at 48rem.

---

## Final review (before reporting M12b complete)

1. **Full suites green:** `pnpm -r test` and `pnpm -r typecheck` and `pnpm lint` from repo root.
2. **Spec coverage:** §4.4 grid (Task 1), launcher (Tasks 3/5), statusbar 2rem (Task 1), mobile tool
   strip (Tasks 1/6); §8 single axis (Tasks 1/6/8); §9 a11y menu/focus/targets (Tasks 3/5);
   TODO closures — defaults flip (Task 2) + 40↔48rem harmonization (Task 1). §14 exclusions
   untouched (no region drag-resize, no themes, no combat tracker).
3. **Seam boundary:** confirm `module-topbar` imports nothing from `@shadowcat/module-panels`
   (`grep -rn "module-panels" src/modules/topbar` returns nothing); dockview stays confined.
4. **Docs + skill gate (blocks completion):**
   - `docs/TODO.md`: mark the two "Client / panels (M12a whole-branch review deferrals)" items
     RESOLVED (defaults flipped to launcher-closed; 40rem query removed — compact now keys off the
     48rem `sizeClass` axis).
   - `docs/PLAN.md`: mark the M12b row done; note the token re-audit outcome (line ~128).
   - Update `shadowcat-codebase-client-shell` skill: topbar now hosts the launcher/presence/settings
     entry reading `AppContext.panels`; core-ui grid drives compact off `sizeClass` (no media query);
     statusbar row is 2rem; the toolrail renders as a compact bottom strip. Update
     `shadowcat-codebase-panels` if its interim-defaults note ("everything else minimized") needs the
     launcher-closed correction. Then dispatch `shadowcat-spec-reviewer` on the skill diff (reviewed
     skill-update gate) — PASS before merge.
5. **Two-reviewer close-out** (`shadowcat-spec-reviewer` + `shadowcat-code-reviewer`) per the project
   mainline-plan-execution final review.
