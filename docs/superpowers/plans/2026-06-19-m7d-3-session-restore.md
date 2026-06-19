# M7d-3 — Session-restore Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: this project executes plans with
> the **mainline-plan-execution** skill (inline, per-task spec-compliance check +
> a single final branch review) — NOT subagent-driven-development or
> executing-plans. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Per-user session-restore via the M7a `ui_state` blob: on reload, apply
the saved locale and **return the user to the world they were in** (auto-enter
`lastWorld`); persist locale + last-world changes; add a leave/switch-world control.

**Architecture:** A `sessionState` module holds the opaque blob, loads it on auth,
applies the saved locale to the `I18n` core, observes locale changes (and
last-world changes) and writes the blob back via a debounced `PUT /api/me/ui-state`.
`App.svelte` orchestrates: after auth, restore → auto-enter `lastWorld` (if still
accessible) or world-select; entering/leaving a world updates `lastWorld`. A
"Leave world" action (exposed via `AppContext.leaveWorld`) lives in the Settings
panel.

**Tech Stack:** TypeScript, Svelte 5, Vitest.

## Global Constraints

- The client owns the blob's structure; the server stores it opaquely (M7a). Shape:
  `{ global: { locale, lastWorld }, worlds: { [id]: { activeTab? } } }`.
- **`activeTab` is deferred** (see Decisions): no tabbed-sidebar UI exists yet (the
  sidebar renders one contribution, Settings), so there is no active tab to
  restore. M7d-3 restores **locale + lastWorld**; the `worlds[id].activeTab` slot
  stays in the schema (forward-compatible) and M11/M12 populate it when a tabbed
  sidebar with multiple panels exists.
- **Reload returns you to your last world** (auto-enter `lastWorld`) only if it is
  still accessible (present in `GET /api/worlds`); else fall back to world-select.
- A failed persist is logged (core `consoleLogger`), never blocks the UI.
- TDD for the testable units (api, `sessionState`, the App restore/leave routing).
- Commands: `pnpm --filter @shadowcat/ui exec vitest run src/<path>.test.ts`;
  full `pnpm --filter @shadowcat/ui test`; typecheck `… typecheck`.

---

### Task 1: `ui_state` API client methods

**Files:**
- Modify: `src/client/ui/src/lib/api.ts` (add `getUiState`/`putUiState`; a `PUT`
  helper)
- Modify: `src/client/ui/src/lib/api.test.ts`

**Interfaces:**
- Produces:
  ```ts
  export interface UiState {
    global: { locale: string; lastWorld: string | null };
    worlds: Record<string, { activeTab?: string }>;
  }
  export function getUiState(): Promise<UiState>;   // normalizes {} → defaults
  export function putUiState(state: UiState): Promise<void>;
  ```

- [ ] **Step 1: Write the failing test**

Add to `src/client/ui/src/lib/api.test.ts`:

```ts
test("getUiState normalizes an empty server blob to defaults", async () => {
  mockFetch(200, {});
  const s = await api.getUiState();
  expect(s).toEqual({ global: { locale: "en", lastWorld: null }, worlds: {} });
});

test("getUiState passes through a stored blob", async () => {
  mockFetch(200, { global: { locale: "en", lastWorld: "w1" }, worlds: { w1: { activeTab: "settings" } } });
  const s = await api.getUiState();
  expect(s.global.lastWorld).toBe("w1");
});

test("putUiState PUTs the blob", async () => {
  const f = mockFetch(204);
  await api.putUiState({ global: { locale: "en", lastWorld: null }, worlds: {} });
  expect(f).toHaveBeenCalledWith("/api/me/ui-state", expect.objectContaining({ method: "PUT" }));
});
```

- [ ] **Step 2: Run it to verify it fails**

Run: `pnpm --filter @shadowcat/ui exec vitest run src/lib/api.test.ts`
Expected: FAIL — `getUiState`/`putUiState` not exported.

- [ ] **Step 3: Implement**

In `src/client/ui/src/lib/api.ts` — add a `PUT` helper (generalize the existing
`postJson` if preferred) and the methods:

```ts
export interface UiState {
  global: { locale: string; lastWorld: string | null };
  worlds: Record<string, { activeTab?: string }>;
}

function defaultUiState(): UiState {
  return { global: { locale: "en", lastWorld: null }, worlds: {} };
}

export async function getUiState(): Promise<UiState> {
  const raw = await getJson<Partial<UiState>>("/api/me/ui-state");
  const def = defaultUiState();
  return {
    global: { ...def.global, ...(raw.global ?? {}) },
    worlds: raw.worlds ?? {},
  };
}

export async function putUiState(state: UiState): Promise<void> {
  const res = await fetch("/api/me/ui-state", {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(state),
  });
  if (!res.ok) throw new Error(`PUT /api/me/ui-state → ${res.status}`);
}
```

- [ ] **Step 4: Run it to verify it passes**

Run: `pnpm --filter @shadowcat/ui exec vitest run src/lib/api.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/client/ui/src/lib/api.ts src/client/ui/src/lib/api.test.ts
git commit -m "feat(ui): getUiState/putUiState API client methods"
```

---

### Task 2: `sessionState` — load, locale persistence, debounced PUT

**Files:**
- Create: `src/client/ui/src/lib/sessionState.svelte.ts`, `sessionState.test.ts`

**Interfaces:**
- Consumes: `getUiState`/`putUiState`; the `i18n` singleton; `consoleLogger`.
- Produces:
  ```ts
  export function loadSessionState(): Promise<UiState>;  // fetches, applies locale, starts observing
  export function getSessionState(): UiState;
  export function setLastWorld(id: string | null): void; // persists (debounced)
  export function flushSessionState(): Promise<void>;     // test/teardown: force-persist now
  ```

- [ ] **Step 1: Write the failing test**

`src/client/ui/src/lib/sessionState.test.ts`:

```ts
import { test, expect, vi, afterEach, beforeEach } from "vitest";
import * as api from "./api";
import { i18n } from "./i18n.svelte";
import { loadSessionState, getSessionState, setLastWorld, flushSessionState } from "./sessionState.svelte";

beforeEach(() => i18n.setLocale("en"));
afterEach(() => vi.restoreAllMocks());

test("load applies the saved locale and exposes the blob", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue({
    global: { locale: "en", lastWorld: "w1" }, worlds: {},
  });
  const s = await loadSessionState();
  expect(s.global.lastWorld).toBe("w1");
  expect(i18n.locale).toBe("en");
  expect(getSessionState().global.lastWorld).toBe("w1");
});

test("setLastWorld updates state and persists (debounced)", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue({
    global: { locale: "en", lastWorld: null }, worlds: {},
  });
  const put = vi.spyOn(api, "putUiState").mockResolvedValue();
  await loadSessionState();
  setLastWorld("w2");
  expect(getSessionState().global.lastWorld).toBe("w2");
  await flushSessionState();
  expect(put).toHaveBeenCalled();
  expect(put.mock.calls.at(-1)?.[0].global.lastWorld).toBe("w2");
});

test("a locale change persists the new locale", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue({
    global: { locale: "en", lastWorld: null }, worlds: {},
  });
  const put = vi.spyOn(api, "putUiState").mockResolvedValue();
  await loadSessionState();
  i18n.setLocale("zz");
  await flushSessionState();
  expect(put.mock.calls.at(-1)?.[0].global.locale).toBe("zz");
});
```

- [ ] **Step 2: Run it to verify it fails**

Run: `pnpm --filter @shadowcat/ui exec vitest run src/lib/sessionState.test.ts`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement**

`src/client/ui/src/lib/sessionState.svelte.ts`:

```ts
import { consoleLogger } from "@shadowcat/core";
import { getUiState, putUiState, type UiState } from "./api";
import { i18n } from "./i18n.svelte";

const logger = consoleLogger();
let state: UiState = { global: { locale: "en", lastWorld: null }, worlds: {} };
let loaded = false;

// Leading-edge debounce with a trailing catch-up (ui_state changes are user-paced;
// the leading edge persists promptly, the trailing flush captures a change made
// during the cooldown). See [[debounce-leading-edge-not-trailing-rearm]].
const COOLDOWN_MS = 500;
let timer: ReturnType<typeof setTimeout> | null = null;
let pendingDuringCooldown = false;

async function persist(): Promise<void> {
  try {
    await putUiState(state);
  } catch (e) {
    logger.warn("ui_state persist failed", e);
  }
}

function schedulePersist(): void {
  if (!loaded) return; // don't write back during the initial restore
  if (timer === null) {
    void persist(); // leading edge
    timer = setTimeout(() => {
      timer = null;
      if (pendingDuringCooldown) {
        pendingDuringCooldown = false;
        schedulePersist();
      }
    }, COOLDOWN_MS);
  } else {
    pendingDuringCooldown = true;
  }
}

/** Fetch the blob, apply the saved locale, and start observing locale changes. */
export async function loadSessionState(): Promise<UiState> {
  state = await getUiState();
  // Apply locale before marking loaded so the initial apply does not persist.
  if (i18n.locale !== state.global.locale) i18n.setLocale(state.global.locale);
  loaded = true;
  // Observe future locale changes (switcher, etc.) and persist them.
  i18n.subscribe(() => {
    if (state.global.locale !== i18n.locale) {
      state.global.locale = i18n.locale;
      schedulePersist();
    }
  });
  return state;
}

export function getSessionState(): UiState {
  return state;
}

export function setLastWorld(id: string | null): void {
  if (state.global.lastWorld === id) return;
  state.global.lastWorld = id;
  schedulePersist();
}

/** Force any pending persist to run now (test/teardown helper). */
export async function flushSessionState(): Promise<void> {
  if (timer) {
    clearTimeout(timer);
    timer = null;
  }
  pendingDuringCooldown = false;
  await persist();
}
```

- [ ] **Step 4: Run it to verify it passes**

Run: `pnpm --filter @shadowcat/ui exec vitest run src/lib/sessionState.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/client/ui/src/lib/sessionState.svelte.ts src/client/ui/src/lib/sessionState.test.ts
git commit -m "feat(ui): sessionState — load, locale persistence, debounced PUT"
```

---

### Task 3: App restore wiring + leave-world control

**Files:**
- Modify: `src/client/ui/src/App.svelte` (boot restore + auto-enter; `lastWorld` on
  enter; `leaveWorld`)
- Modify: `src/client/ui/src/lib/appContext.ts` (add `leaveWorld`)
- Modify: `src/client/ui/src/lib/Table.svelte` (provide `leaveWorld`)
- Modify: `src/client/ui/src/lib/__fixtures__/SurfaceHarness.svelte` (provide `leaveWorld`)
- Modify: `src/client/ui/src/modules/core-ui/panels/Settings.svelte` (Leave-world button)
- Modify: `src/client/ui/src/App.test.ts`

**Interfaces:**
- Consumes: `loadSessionState`/`setLastWorld`; `listWorlds`; `AppContext`.
- Produces: `AppContext.leaveWorld: () => void`.

- [ ] **Step 1: Extend AppContext**

In `src/client/ui/src/lib/appContext.ts`, add to `AppContext`:

```ts
  leaveWorld: () => void;
```

- [ ] **Step 2: App boot restore + auto-enter + enter/leave**

In `src/client/ui/src/App.svelte`, import the session-state + worlds API and rework
`boot`/`enterWorld`, add `leaveWorld`:

```svelte
  import { getConfig, getMe, listWorlds, type Me } from "./lib/api";
  import { loadSessionState, setLastWorld } from "./lib/sessionState.svelte";
  ...
  async function boot() {
    try {
      const cfg = await getConfig();
      if (!cfg.initialized) {
        navigate({ name: "setup" });
        return;
      }
      me = await getMe();
      if (!me) {
        navigate({ name: "login" });
        return;
      }
      const ui = await loadSessionState(); // applies the saved locale
      const last = ui.global.lastWorld;
      if (last) {
        const worlds = await listWorlds();
        if (worlds.some((w) => w.id === last)) {
          enterWorld(last); // reload returns you to your last world
          return;
        }
        setLastWorld(null); // stale (deleted / revoked) — clear it
      }
      navigate({ name: "worlds" });
    } catch {
      navigate({ name: "login" });
    } finally {
      booted = true;
    }
  }
  boot();

  async function afterAuth() {
    try {
      me = await getMe();
      await loadSessionState();
    } catch {
      me = null;
    }
    navigate({ name: me ? "worlds" : "login" });
  }

  function enterWorld(worldId: string) {
    if (!me) return;
    const wsUrl =
      (location.protocol === "https:" ? "wss:" : "ws:") +
      "//" + location.host + "/ws?world=" + worldId;
    const s = new WorldSession({ selfId: me.id, connect: webSocketConnect(wsUrl), coreUiModule: coreUi });
    session = s;
    void s.enter(worldId);
    setLastWorld(worldId);
    navigate({ name: "world", id: worldId });
  }

  function leaveWorld() {
    session?.leave();
    session = null;
    setLastWorld(null);
    navigate({ name: "worlds" });
  }
```

- [ ] **Step 3: Provide `leaveWorld` from Table + fixture**

`Table.svelte` `setAppContext({ ..., t, leaveWorld })` — but `leaveWorld` lives in
`App`. Pass it down as a prop: `App` renders `<Table {session} {leaveWorld} />`;
`Table` accepts `leaveWorld` and includes it in the context:

```svelte
  // Table.svelte
  let { session, leaveWorld }: { session: WorldSession; leaveWorld: () => void } = $props();
  ...
  setAppContext({ contributions: session.contributions, store: session.store,
    world: session.world!, role: session.role!, t, leaveWorld });
```

In `App.svelte`, the Table branch becomes `<Table {session} {leaveWorld} />`.

In `SurfaceHarness.svelte`, add `leaveWorld: () => {}` to the provided context.

- [ ] **Step 4: Leave-world button in Settings**

In `src/client/ui/src/modules/core-ui/panels/Settings.svelte`, read `leaveWorld`
from context and add a button (a new `settings.leaveWorld` catalog key):

```svelte
  const { role, t, leaveWorld } = getAppContext();
  ...
  <button onclick={leaveWorld}>{t("settings.leaveWorld")}</button>
  <button onclick={doLogout}>{t("settings.logout")}</button>
```

Add to `src/client/ui/src/locales/en.ts`: `"settings.leaveWorld": "Leave world",`.

- [ ] **Step 5: Write the App restore tests**

In `src/client/ui/src/App.test.ts`, add (stub `WebSocket` so auto-enter's session
connect does not crash jsdom and stays "connecting"):

```ts
test("auto-enters the saved lastWorld on load", async () => {
  vi.stubGlobal("WebSocket", class { addEventListener() {} send() {} close() {} } as unknown);
  vi.spyOn(api, "getConfig").mockResolvedValue({ initialized: true });
  vi.spyOn(api, "getMe").mockResolvedValue({ id: "u1", username: "gm", server_role: "user" });
  vi.spyOn(api, "getUiState").mockResolvedValue({ global: { locale: "en", lastWorld: "w1" }, worlds: {} });
  vi.spyOn(api, "putUiState").mockResolvedValue();
  vi.spyOn(api, "listWorlds").mockResolvedValue([{ id: "w1", name: "W", role: "gm" }]);
  render(App);
  // The session is entered (connecting) rather than world-select.
  expect(await screen.findByText("Connecting…")).toBeTruthy();
});

test("falls back to world-select when lastWorld is no longer accessible", async () => {
  vi.spyOn(api, "getConfig").mockResolvedValue({ initialized: true });
  vi.spyOn(api, "getMe").mockResolvedValue({ id: "u1", username: "gm", server_role: "user" });
  vi.spyOn(api, "getUiState").mockResolvedValue({ global: { locale: "en", lastWorld: "gone" }, worlds: {} });
  vi.spyOn(api, "putUiState").mockResolvedValue();
  vi.spyOn(api, "listWorlds").mockResolvedValue([]); // "gone" not present
  render(App);
  expect(await screen.findByText("Your worlds")).toBeTruthy();
});
```

(Update the existing "authenticated routes to WorldSelect" test to also mock
`getUiState` → `{ global: { locale: "en", lastWorld: null }, worlds: {} }` and
`putUiState`, since `boot` now loads the blob.)

- [ ] **Step 6: Run the tests**

Run: `pnpm --filter @shadowcat/ui exec vitest run src/App.test.ts`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/client/ui/src/App.svelte src/client/ui/src/lib/appContext.ts \
        src/client/ui/src/lib/Table.svelte "src/client/ui/src/lib/__fixtures__/SurfaceHarness.svelte" \
        src/client/ui/src/modules/core-ui/panels/Settings.svelte src/client/ui/src/locales/en.ts \
        src/client/ui/src/App.test.ts
git commit -m "feat(ui): session-restore — reload returns to last world + leave control"
```

---

### Task 4: Full green + typecheck

- [ ] **Step 1: Full ui suite + core**

Run: `pnpm --filter @shadowcat/ui test` → PASS.
Run: `pnpm --filter @shadowcat/core test` → PASS (unchanged, 104).

- [ ] **Step 2: ui typecheck**

Run: `pnpm --filter @shadowcat/ui typecheck`
Expected: 0 errors / 0 warnings (the `leaveWorld` field on `AppContext`, the Table
prop, the Settings read all type-check).

- [ ] **Step 3: Build**

Run: `pnpm --filter @shadowcat/ui build`
Expected: success.

---

## Self-Review

**Spec coverage (spec §6):**
- `ui_state` shape (§6.1) → Task 1 (`UiState`). ✓
- Load + restore locale; **reload → auto-enter lastWorld** with the accessibility
  fall-back (§6.2) → Tasks 2 (locale) + 3 (auto-enter). ✓
- Debounced persist on locale + lastWorld changes (§6.3) → Task 2. ✓
- Leave/switch-world control (§6.4) → Task 3 (in the Settings panel, see Decisions).
  ✓
- `activeTab` (§6.1/§6.2) → **deferred** (no tab UI; see Decisions). The schema slot
  remains.

**Placeholder scan:** No TBD/TODO; code is complete.

**Type/consistency:** `UiState`, `getUiState`/`putUiState`, `loadSessionState`/
`setLastWorld`/`flushSessionState`, `AppContext.leaveWorld`, and the `settings.*`
catalog keys are consistent across tasks.

## Decisions resolved during planning (flag for review)

1. **`activeTab` deferred.** The spec §6 includes restoring the active sidebar tab,
   but no tabbed-sidebar UI exists — the sidebar renders a single contribution
   (Settings). Building a tabbed panel for one tab is premature; the
   `worlds[id].activeTab` slot stays in the (forward-compatible) blob, and M11/M12
   build the tabbed sidebar + activeTab restore when there are multiple panels.
2. **Leave-world control → Settings panel**, not the spec §6.4 "topbar user menu"
   (which does not exist). Settings is the existing session-controls panel (logout,
   language); adding "Leave world" there is the smallest correct home. A topbar
   user menu can host it later.

## Out of scope (completes M7)

This is the final M7 sub-milestone. After it merges: **M7 is complete** — update
`PLAN.md`/`docs`, then **push to origin/main** (the first push since M6).
