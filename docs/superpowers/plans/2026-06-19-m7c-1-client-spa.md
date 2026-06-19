# M7c-1 — Client SPA + Shell Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: this project executes plans with
> the **mainline-plan-execution** skill (inline, per-task spec-compliance check +
> a single final branch review) — NOT subagent-driven-development or
> executing-plans. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The default Svelte 5 SPA: the entry flow (setup → login → world-select)
and the in-world shell (a `WorldSession` controller starting the `WsClient` and
loading the first-party `core-ui` module that provides the region surfaces +
default panels, rendered by `<Surface>`).

**Architecture:** Pure `src/client/ui/` (Svelte 5) consuming `@shadowcat/core` and
`@shadowcat/types`. Hash routing, no router dep. A typed `api.ts` over the M7a
endpoints. The contribution architecture (M7b) activates only in-world, owned by a
`WorldSession`. Minimal scoped CSS (theming is M7d). The binary still serves the
old static bundle until M7c-2; this milestone runs via `vite dev` + a spawned
test-server.

**Tech Stack:** Svelte 5 (runes), Vite, Vitest + `@testing-library/svelte` + jsdom,
Playwright, `@shadowcat/core`, `@shadowcat/types`.

## Global Constraints

- **Svelte 5 runes only** (`$props`/`$state`/`$derived`/`$effect`; `onclick=`;
  dynamic component via a capitalized variable). No `export let`/`$:`/`<slot>`/
  `on:`/`<svelte:component>`.
- The contribution architecture activates **in-world only**; entry views are plain
  routed components (spec §4).
- Minimal scoped CSS; no token system (M7d). Strings literal (i18n is M7d).
- New deps: `@shadowcat/types` (workspace), `@playwright/test` (dev). Logged where
  the project logs deps if applicable.
- `MeResponse` is not ts-rs-exported (server plain DTO); the client defines a local
  `Me` type. `WorldEntry`/`ServerConfig` come from `@shadowcat/types`.
- TDD: failing test first, watch it fail, minimal impl, pass, commit.
- Commands (from repo root):
  - Single ui test: `pnpm --filter @shadowcat/ui exec vitest run src/<path>.test.ts`
  - Full ui unit tests: `pnpm --filter @shadowcat/ui test`
  - ui typecheck: `pnpm --filter @shadowcat/ui typecheck`
  - Playwright: `pnpm --filter @shadowcat/ui exec playwright test`

---

## Phase A — Entry flow (pre-world)

### Task 1: Deps + Vite dev proxy + typed API client

**Files:**
- Modify: `src/client/ui/package.json` (add `@shadowcat/types` dependency)
- Modify: `src/client/ui/vite.config.ts` (dev proxy)
- Create: `src/client/ui/src/lib/api.ts`
- Create: `src/client/ui/src/lib/api.test.ts`

**Interfaces:**
- Produces:
  ```ts
  export interface Me { id: string; username: string; server_role: string }
  export function getConfig(): Promise<{ initialized: boolean }>;
  export function getMe(): Promise<Me | null>;        // null on 401
  export function login(username: string, password: string): Promise<boolean>;
  export function logout(): Promise<void>;
  export function setup(username: string, password: string, token?: string): Promise<{ ok: boolean; status: number }>;
  export function listWorlds(): Promise<WorldEntry[]>;
  export function createWorld(name: string): Promise<WorldEntry>;
  ```

- [ ] **Step 1: Add the workspace types dependency**

Run: `pnpm --filter @shadowcat/ui add @shadowcat/types@workspace:*`
Expected: `package.json` `dependencies` gains `"@shadowcat/types": "workspace:*"`.

- [ ] **Step 2: Add the dev proxy**

Replace `src/client/ui/vite.config.ts`:

```ts
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Dev: the SPA is served by Vite; /api and /ws proxy to the Rust server so
// `vite dev` runs against a real backend. SHADOWCAT_SERVER overrides the target.
const target = process.env.SHADOWCAT_SERVER ?? "http://127.0.0.1:30000";

export default defineConfig({
  plugins: [svelte()],
  build: { outDir: "../../../dist", emptyOutDir: true },
  server: {
    proxy: {
      "/api": { target, changeOrigin: true },
      "/ws": { target, ws: true, changeOrigin: true },
    },
  },
});
```

- [ ] **Step 3: Write the failing test**

`src/client/ui/src/lib/api.test.ts`:

```ts
import { test, expect, vi, afterEach } from "vitest";
import * as api from "./api";

afterEach(() => vi.restoreAllMocks());

function mockFetch(status: number, body?: unknown) {
  return vi.spyOn(globalThis, "fetch").mockResolvedValue(
    new Response(body === undefined ? null : JSON.stringify(body), { status }),
  );
}

test("getConfig returns the parsed config", async () => {
  mockFetch(200, { initialized: true });
  expect(await api.getConfig()).toEqual({ initialized: true });
});

test("getMe returns null on 401, the body on 200", async () => {
  mockFetch(401);
  expect(await api.getMe()).toBeNull();
  mockFetch(200, { id: "u1", username: "a", server_role: "user" });
  expect((await api.getMe())?.id).toBe("u1");
});

test("login returns true on 204, false on 401", async () => {
  mockFetch(204);
  expect(await api.login("a", "b")).toBe(true);
  mockFetch(401);
  expect(await api.login("a", "x")).toBe(false);
});

test("listWorlds returns the world array", async () => {
  mockFetch(200, [{ id: "w1", name: "W", role: "gm" }]);
  const worlds = await api.listWorlds();
  expect(worlds[0].name).toBe("W");
});
```

- [ ] **Step 2b: Run it to verify it fails**

Run: `pnpm --filter @shadowcat/ui exec vitest run src/lib/api.test.ts`
Expected: FAIL — `./api` does not exist.

- [ ] **Step 4: Implement the client**

`src/client/ui/src/lib/api.ts`:

```ts
import type { WorldEntry, ServerConfig } from "@shadowcat/types";

/** Local mirror of the server's MeResponse (not ts-rs-exported). */
export interface Me {
  id: string;
  username: string;
  server_role: string;
}

async function getJson<T>(url: string): Promise<T> {
  const res = await fetch(url, { headers: { accept: "application/json" } });
  if (!res.ok) throw new Error(`${url} → ${res.status}`);
  return (await res.json()) as T;
}

async function postJson(url: string, body: unknown): Promise<Response> {
  return fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

export function getConfig(): Promise<ServerConfig> {
  return getJson<ServerConfig>("/api/config");
}

export async function getMe(): Promise<Me | null> {
  const res = await fetch("/api/me", { headers: { accept: "application/json" } });
  if (res.status === 401) return null;
  if (!res.ok) throw new Error(`/api/me → ${res.status}`);
  return (await res.json()) as Me;
}

export async function login(username: string, password: string): Promise<boolean> {
  const res = await postJson("/api/login", { username, password });
  return res.ok;
}

export async function logout(): Promise<void> {
  await postJson("/api/logout", {});
}

export async function setup(
  username: string,
  password: string,
  token?: string,
): Promise<{ ok: boolean; status: number }> {
  const body: Record<string, string> = { username, password };
  if (token) body.token = token;
  const res = await postJson("/api/setup", body);
  return { ok: res.ok, status: res.status };
}

export function listWorlds(): Promise<WorldEntry[]> {
  return getJson<WorldEntry[]>("/api/worlds");
}

export async function createWorld(name: string): Promise<WorldEntry> {
  const res = await postJson("/api/worlds", { name });
  if (!res.ok) throw new Error(`/api/worlds → ${res.status}`);
  return (await res.json()) as WorldEntry;
}
```

- [ ] **Step 5: Run it to verify it passes**

Run: `pnpm --filter @shadowcat/ui exec vitest run src/lib/api.test.ts`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/client/ui/package.json src/client/ui/vite.config.ts \
        src/client/ui/src/lib/api.ts src/client/ui/src/lib/api.test.ts pnpm-lock.yaml
git commit -m "feat(ui): typed API client + Vite dev proxy"
```

---

### Task 2: Hash router

**Files:**
- Create: `src/client/ui/src/lib/route.svelte.ts`
- Create: `src/client/ui/src/lib/route.test.ts`

**Interfaces:**
- Produces:
  ```ts
  export type Route =
    | { name: "setup" } | { name: "login" } | { name: "worlds" }
    | { name: "world"; id: string } | { name: "unknown" };
  export function parseHash(hash: string): Route;
  export function navigate(route: Route): void;   // sets location.hash
  export function currentRoute(): Route;          // reactive ($state-backed)
  ```

- [ ] **Step 1: Write the failing test**

`src/client/ui/src/lib/route.test.ts`:

```ts
import { test, expect } from "vitest";
import { parseHash } from "./route";

test("parses the known routes", () => {
  expect(parseHash("#/login")).toEqual({ name: "login" });
  expect(parseHash("#/setup")).toEqual({ name: "setup" });
  expect(parseHash("#/worlds")).toEqual({ name: "worlds" });
  expect(parseHash("#/world/abc-123")).toEqual({ name: "world", id: "abc-123" });
  expect(parseHash("")).toEqual({ name: "unknown" });
  expect(parseHash("#/nonsense")).toEqual({ name: "unknown" });
});
```

- [ ] **Step 2: Run it to verify it fails**

Run: `pnpm --filter @shadowcat/ui exec vitest run src/lib/route.test.ts`
Expected: FAIL — `./route` not found.

- [ ] **Step 3: Implement the router**

`src/client/ui/src/lib/route.svelte.ts`:

```ts
export type Route =
  | { name: "setup" }
  | { name: "login" }
  | { name: "worlds" }
  | { name: "world"; id: string }
  | { name: "unknown" };

export function parseHash(hash: string): Route {
  const path = hash.replace(/^#/, "");
  if (path === "/setup") return { name: "setup" };
  if (path === "/login") return { name: "login" };
  if (path === "/worlds") return { name: "worlds" };
  const m = /^\/world\/(.+)$/.exec(path);
  if (m) return { name: "world", id: m[1] };
  return { name: "unknown" };
}

export function routeToHash(route: Route): string {
  switch (route.name) {
    case "world":
      return `#/world/${route.id}`;
    case "unknown":
      return "#/login";
    default:
      return `#/${route.name}`;
  }
}

export function navigate(route: Route): void {
  location.hash = routeToHash(route);
}

let route = $state<Route>(parseHash(location.hash));
if (typeof window !== "undefined") {
  window.addEventListener("hashchange", () => {
    route = parseHash(location.hash);
  });
}

export function currentRoute(): Route {
  return route;
}
```

- [ ] **Step 4: Run it to verify it passes**

Run: `pnpm --filter @shadowcat/ui exec vitest run src/lib/route.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/client/ui/src/lib/route.svelte.ts src/client/ui/src/lib/route.test.ts
git commit -m "feat(ui): hash router"
```

---

### Task 3: Login + Setup views

**Files:**
- Create: `src/client/ui/src/lib/views/Login.svelte`, `Setup.svelte`
- Create: `src/client/ui/src/lib/views/Login.test.ts`

**Interfaces:**
- Consumes: `api.login`/`api.setup` (Task 1); `navigate` (Task 2).
- Produces: `Login` (prop `{ onAuthed: () => void }`), `Setup` (prop
  `{ onDone: () => void }`).

- [ ] **Step 1: Write the failing test**

`src/client/ui/src/lib/views/Login.test.ts`:

```ts
import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import { test, expect, vi, afterEach } from "vitest";
import Login from "./Login.svelte";
import * as api from "../api";

afterEach(() => vi.restoreAllMocks());

test("calls onAuthed after a successful login", async () => {
  vi.spyOn(api, "login").mockResolvedValue(true);
  const onAuthed = vi.fn();
  render(Login, { props: { onAuthed } });

  await fireEvent.input(screen.getByLabelText("Username"), { target: { value: "gm" } });
  await fireEvent.input(screen.getByLabelText("Password"), { target: { value: "pw" } });
  await fireEvent.click(screen.getByRole("button", { name: "Log in" }));

  await waitFor(() => expect(onAuthed).toHaveBeenCalledOnce());
});

test("shows an error and does not call onAuthed on failure", async () => {
  vi.spyOn(api, "login").mockResolvedValue(false);
  const onAuthed = vi.fn();
  render(Login, { props: { onAuthed } });

  await fireEvent.input(screen.getByLabelText("Username"), { target: { value: "gm" } });
  await fireEvent.input(screen.getByLabelText("Password"), { target: { value: "x" } });
  await fireEvent.click(screen.getByRole("button", { name: "Log in" }));

  expect(await screen.findByRole("alert")).toBeTruthy();
  expect(onAuthed).not.toHaveBeenCalled();
});
```

- [ ] **Step 2: Run it to verify it fails**

Run: `pnpm --filter @shadowcat/ui exec vitest run src/lib/views/Login.test.ts`
Expected: FAIL — `./Login.svelte` not found.

- [ ] **Step 3: Implement Login.svelte**

`src/client/ui/src/lib/views/Login.svelte`:

```svelte
<script lang="ts">
  import { login } from "../api";

  let { onAuthed }: { onAuthed: () => void } = $props();
  let username = $state("");
  let password = $state("");
  let error = $state(false);
  let busy = $state(false);

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    error = false;
    const ok = await login(username, password);
    busy = false;
    if (ok) onAuthed();
    else error = true;
  }
</script>

<main class="entry">
  <h1>shadowcat</h1>
  <form onsubmit={submit}>
    <label>Username <input bind:value={username} autocomplete="username" /></label>
    <label>Password
      <input type="password" bind:value={password} autocomplete="current-password" />
    </label>
    {#if error}<p role="alert">Invalid username or password.</p>{/if}
    <button type="submit" disabled={busy}>Log in</button>
  </form>
</main>

<style>
  .entry { max-width: 22rem; margin: 4rem auto; display: grid; gap: 1rem; }
  form { display: grid; gap: 0.75rem; }
  label { display: grid; gap: 0.25rem; }
</style>
```

(Note: the `<label>Username <input/></label>` wrapping makes `getByLabelText`
match.)

- [ ] **Step 4: Implement Setup.svelte**

`src/client/ui/src/lib/views/Setup.svelte`:

```svelte
<script lang="ts">
  import { setup } from "../api";

  let { onDone }: { onDone: () => void } = $props();
  let username = $state("");
  let password = $state("");
  let token = $state("");
  let error = $state("");
  let busy = $state(false);

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    error = "";
    const { ok, status } = await setup(username, password, token || undefined);
    busy = false;
    if (ok) onDone();
    else error = status === 403 ? "Invalid setup token." : `Setup failed (${status}).`;
  }
</script>

<main class="entry">
  <h1>Create the admin account</h1>
  <form onsubmit={submit}>
    <label>Username <input bind:value={username} autocomplete="username" /></label>
    <label>Password
      <input type="password" bind:value={password} autocomplete="new-password" />
    </label>
    <label>Setup token (if required) <input bind:value={token} /></label>
    {#if error}<p role="alert">{error}</p>{/if}
    <button type="submit" disabled={busy}>Create admin</button>
  </form>
</main>

<style>
  .entry { max-width: 22rem; margin: 4rem auto; display: grid; gap: 1rem; }
  form { display: grid; gap: 0.75rem; }
  label { display: grid; gap: 0.25rem; }
</style>
```

- [ ] **Step 5: Run it to verify it passes**

Run: `pnpm --filter @shadowcat/ui exec vitest run src/lib/views/Login.test.ts`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/client/ui/src/lib/views/Login.svelte src/client/ui/src/lib/views/Setup.svelte \
        src/client/ui/src/lib/views/Login.test.ts
git commit -m "feat(ui): Login + Setup entry views"
```

---

### Task 4: WorldSelect view

**Files:**
- Create: `src/client/ui/src/lib/views/WorldSelect.svelte`
- Create: `src/client/ui/src/lib/views/WorldSelect.test.ts`

**Interfaces:**
- Consumes: `api.listWorlds`/`api.createWorld`; `navigate`.
- Produces: `WorldSelect` (prop `{ onEnter: (worldId: string) => void }`).

- [ ] **Step 1: Write the failing test**

`src/client/ui/src/lib/views/WorldSelect.test.ts`:

```ts
import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import { test, expect, vi, afterEach } from "vitest";
import WorldSelect from "./WorldSelect.svelte";
import * as api from "../api";

afterEach(() => vi.restoreAllMocks());

test("lists worlds and enters the chosen one", async () => {
  vi.spyOn(api, "listWorlds").mockResolvedValue([
    { id: "w1", name: "Alpha", role: "gm" },
    { id: "w2", name: "Beta", role: "player" },
  ]);
  const onEnter = vi.fn();
  render(WorldSelect, { props: { onEnter } });

  await waitFor(() => expect(screen.getByText("Alpha")).toBeTruthy());
  await fireEvent.click(screen.getByRole("button", { name: /Alpha/ }));
  expect(onEnter).toHaveBeenCalledWith("w1");
});
```

- [ ] **Step 2: Run it to verify it fails**

Run: `pnpm --filter @shadowcat/ui exec vitest run src/lib/views/WorldSelect.test.ts`
Expected: FAIL — not found.

- [ ] **Step 3: Implement WorldSelect.svelte**

`src/client/ui/src/lib/views/WorldSelect.svelte`:

```svelte
<script lang="ts">
  import type { WorldEntry } from "@shadowcat/types";
  import { listWorlds, createWorld } from "../api";

  let { onEnter }: { onEnter: (worldId: string) => void } = $props();
  let worlds = $state<WorldEntry[]>([]);
  let newName = $state("");

  async function refresh() {
    worlds = await listWorlds();
  }
  refresh();

  async function create(e: SubmitEvent) {
    e.preventDefault();
    if (!newName.trim()) return;
    const w = await createWorld(newName.trim());
    newName = "";
    await refresh();
    onEnter(w.id);
  }
</script>

<main class="entry">
  <h1>Your worlds</h1>
  <ul>
    {#each worlds as world (world.id)}
      <li>
        <button onclick={() => onEnter(world.id)}>
          {world.name} <small>({world.role})</small>
        </button>
      </li>
    {/each}
    {#if worlds.length === 0}<li class="empty">No worlds yet.</li>{/if}
  </ul>
  <form onsubmit={create}>
    <input bind:value={newName} placeholder="New world name" aria-label="New world name" />
    <button type="submit">Create world</button>
  </form>
</main>

<style>
  .entry { max-width: 30rem; margin: 4rem auto; display: grid; gap: 1rem; }
  ul { list-style: none; padding: 0; display: grid; gap: 0.5rem; }
  form { display: flex; gap: 0.5rem; }
</style>
```

- [ ] **Step 4: Run it to verify it passes**

Run: `pnpm --filter @shadowcat/ui exec vitest run src/lib/views/WorldSelect.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/client/ui/src/lib/views/WorldSelect.svelte \
        src/client/ui/src/lib/views/WorldSelect.test.ts
git commit -m "feat(ui): WorldSelect view"
```

---

## Phase B — In-world shell

### Task 5: AppContext extension + DocumentStore bridge + WorldSession

**Files:**
- Modify: `src/client/ui/src/lib/appContext.ts` (extend `AppContext`)
- Create: `src/client/ui/src/lib/reactiveStore.svelte.ts`
- Create: `src/client/ui/src/lib/worldSession.svelte.ts`
- Create: `src/client/ui/src/lib/worldSession.test.ts`

**Interfaces:**
- Consumes: `WsClient`, `OptimisticClient`, `DocumentStore`, `ContributionRegistry`,
  `ModuleRegistry`, `reconcileTopology`, `silentLogger`, `webSocketConnect`,
  `type Connect`, `type WireWelcome` from `@shadowcat/core`; the `core-ui` module
  (Task 6 — import lazily/inject for testability).
- Produces:
  - `AppContext { contributions, store, world, role }`.
  - `reactiveQuery(store, docType)` — a `createSubscriber`-backed reactive read.
  - `class WorldSession` with `enter(opts)`, `leave()`, reactive `state`/`role`/`world`.

- [ ] **Step 1: Write the failing test** (drives the controller against the core mock-server)

`src/client/ui/src/lib/worldSession.test.ts`:

```ts
import { test, expect, vi } from "vitest";
import { ContributionRegistry, type Connect } from "@shadowcat/core";
import { WorldSession } from "./worldSession.svelte";

// `MockServer` is internal core test code (not barrel-exported), so use a minimal
// inline Connect that delivers one valid Welcome frame on connect and ignores
// sends. The frame must satisfy parseServerMsg (all welcome fields present).
const welcomeFrame = {
  type: "welcome",
  world: "w1",
  current_seq: 0,
  server_time: 0,
  world_default_grants: { by_role: {}, by_user: {} },
  actor_role: "player",
  capability_requirements: [],
  contract_declarations: [],
};

function mockConnect(): Connect {
  return (handlers) => {
    queueMicrotask(() => handlers.onMessage(JSON.stringify(welcomeFrame)));
    return Promise.resolve({ send: () => {}, close: () => handlers.onClose() });
  };
}

// A core-ui stand-in: provides the root surface so activation is exercised without
// pulling the real (Svelte-importing) module into this unit test.
const coreUiStub = {
  manifest: {
    id: "core-ui",
    version: "0.1.0",
    dependencies: {},
    provides: [{ contract: "shadowcat.surface:root", cardinality: "singleton" as const }],
  },
  register: vi.fn(),
};

test("enter starts the socket, captures role from Welcome, activates core-ui", async () => {
  const session = new WorldSession({
    selfId: "u1",
    connect: mockConnect(),
    coreUiModule: coreUiStub,
  });

  await session.enter("w1");
  // Welcome arrives on a microtask after connect; poll until handled.
  await vi.waitFor(() => expect(session.role).toBe("player"));
  await vi.waitFor(() => expect(coreUiStub.register).toHaveBeenCalledOnce());
  expect(session.contributions).toBeInstanceOf(ContributionRegistry);

  session.leave();
  expect(session.state).toBe("closed");
});
```

- [ ] **Step 2: Run it to verify it fails**

Run: `pnpm --filter @shadowcat/ui exec vitest run src/lib/worldSession.test.ts`
Expected: FAIL — `./worldSession.svelte` not found.

- [ ] **Step 3: Extend AppContext**

In `src/client/ui/src/lib/appContext.ts`, extend the interface (import the
core types):

```ts
import type { ContributionRegistry, DocumentStore } from "@shadowcat/core";
import type { WorldRole } from "@shadowcat/types";

export interface AppContext {
  contributions: ContributionRegistry;
  store: DocumentStore;
  world: string;
  role: WorldRole;
}
```

(`setAppContext`/`getAppContext` are unchanged. Update the M7b-3
`SurfaceHarness.svelte` fixture to supply the new fields, or relax its provided
object — fix any resulting test/typecheck failure in this task.)

- [ ] **Step 4: Implement the DocumentStore reactive bridge**

`src/client/ui/src/lib/reactiveStore.svelte.ts`:

```ts
import { createSubscriber } from "svelte/reactivity";
import type { DocumentStore, WireDocument } from "@shadowcat/core";

/** Reactive `query` over a DocumentStore: reading it in a rune context re-runs
 * when the store emits. The same subscribe/snapshot bridge as <Surface>. */
export function makeReactiveStore(store: DocumentStore) {
  const subscribe = createSubscriber((update) => store.subscribe(update));
  return {
    query(docType: string): WireDocument[] {
      subscribe();
      return store.query(docType);
    },
    get(id: string): WireDocument | undefined {
      subscribe();
      return store.get(id);
    },
  };
}
```

- [ ] **Step 5: Implement WorldSession**

`src/client/ui/src/lib/worldSession.svelte.ts`:

```ts
import {
  WsClient,
  OptimisticClient,
  DocumentStore,
  ContributionRegistry,
  ModuleRegistry,
  HookBus,
  ServiceRegistry,
  MiddlewareChain,
  reconcileTopology,
  silentLogger,
  type Connect,
  type Module,
  type WireWelcome,
} from "@shadowcat/core";
import type { WorldRole } from "@shadowcat/types";

export type ConnState = "connecting" | "open" | "closed";

export interface WorldSessionOpts {
  selfId: string;
  /** Browser: webSocketConnect(wsUrl). Tests: a mock connect. */
  connect: Connect;
  /** The first-party shell module providing region surfaces. */
  coreUiModule: Module;
}

export class WorldSession {
  readonly store = new DocumentStore();
  readonly contributions = new ContributionRegistry();
  state = $state<ConnState>("closed");
  role = $state<WorldRole | null>(null);
  world = $state<string | null>(null);

  #ws: WsClient | null = null;
  #optimistic: OptimisticClient;
  #modules: ModuleRegistry;

  constructor(private readonly opts: WorldSessionOpts) {
    this.#optimistic = new OptimisticClient(opts.selfId);
    this.#modules = new ModuleRegistry({
      hooks: new HookBus(silentLogger),
      services: new ServiceRegistry(),
      middleware: new MiddlewareChain(),
      store: this.store,
      client: this.#optimistic,
      logger: silentLogger,
      contributions: this.contributions,
    });
  }

  async enter(worldId: string): Promise<void> {
    this.world = worldId;
    this.state = "connecting";
    this.#ws = new WsClient({
      connect: this.opts.connect,
      handlers: {
        onCommand: (cmd) => this.#optimistic.applyCommand(cmd),
        onReject: (id) => this.#optimistic.reject(id),
        onWelcome: (w) => this.#onWelcome(w),
        onError: () => {},
      },
    });
    await this.#ws.start();
    this.state = "open";
  }

  async #onWelcome(w: WireWelcome): Promise<void> {
    this.role = w.actor_role;
    this.#modules.add(this.opts.coreUiModule);
    await this.#modules.activate();
    reconcileTopology(this.#modules.declarations(), w.contract_declarations, silentLogger);
  }

  leave(): void {
    this.#ws?.stop();
    this.#ws = null;
    this.state = "closed";
    this.role = null;
    this.world = null;
  }
}
```

> `HookBus`, `ServiceRegistry`, `MiddlewareChain`, `silentLogger`, `WsClient`,
> `OptimisticClient`, `DocumentStore`, `ContributionRegistry`, `ModuleRegistry`,
> `reconcileTopology`, `webSocketConnect`, and the `Connect`/`Module`/`WireWelcome`
> types are all already exported from the core barrel (`index.ts`) — no core change
> needed. The `ModuleRegistry` `Deps` shape is hooks/services/middleware/store/
> client/logger/contributions (matches `modules.ts`).

- [ ] **Step 6: Run it to verify it passes**

Run: `pnpm --filter @shadowcat/ui exec vitest run src/lib/worldSession.test.ts`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/client/ui/src/lib/appContext.ts src/client/ui/src/lib/reactiveStore.svelte.ts \
        src/client/ui/src/lib/worldSession.svelte.ts src/client/ui/src/lib/worldSession.test.ts \
        "src/client/ui/src/lib/__fixtures__/SurfaceHarness.svelte"
git commit -m "feat(ui): WorldSession controller + AppContext extension + store bridge"
```

---

### Task 6: `core-ui` module + Layout

**Files:**
- Create: `src/client/ui/src/modules/core-ui/index.ts` (the `Module`)
- Create: `src/client/ui/src/modules/core-ui/panels/{Settings,StagePlaceholder,TopBar,StatusBar}.svelte`
- Create: `src/client/ui/src/lib/Layout.svelte`
- Create: `src/client/ui/src/modules/core-ui/coreUi.test.ts`

**Interfaces:**
- Consumes: `type Module`, `ContributionRegistry` from `@shadowcat/core`; the
  `<Surface>` component (M7b-3); `appContext`.
- Produces: `coreUi: Module` (provides root + region surfaces, contributes default
  panels); `Layout` (renders the region surfaces).

- [ ] **Step 1: Write the failing test**

`src/client/ui/src/modules/core-ui/coreUi.test.ts`:

```ts
import { test, expect } from "vitest";
import { ContributionRegistry } from "@shadowcat/core";
import { coreUi } from "./index";
import { silentLogger } from "@shadowcat/core";

test("core-ui declares the region surfaces and contributes default panels", () => {
  const provided = (coreUi.manifest.provides ?? []).map((p) => p.contract);
  expect(provided).toContain("shadowcat.surface:root");
  expect(provided).toContain("shadowcat.surface:sidebar");

  const contributions = new ContributionRegistry();
  // Minimal ModuleContext stand-in: only `contributions` is used by register.
  coreUi.register({
    contributions: { contribute: (c) => contributions.contribute(c) },
    // unused-by-register fields:
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
  } as any);
  expect(contributions.contributionsFor("shadowcat.surface:sidebar").length).toBeGreaterThan(0);
});
```

- [ ] **Step 2: Run it to verify it fails**

Run: `pnpm --filter @shadowcat/ui exec vitest run src/modules/core-ui/coreUi.test.ts`
Expected: FAIL — `./index` not found.

- [ ] **Step 3: Implement the panel components**

`src/client/ui/src/modules/core-ui/panels/Settings.svelte`:

```svelte
<script lang="ts">
  import { getAppContext } from "../../../lib/appContext";
  import { logout } from "../../../lib/api";
  import { navigate } from "../../../lib/route.svelte";

  const { role } = getAppContext();
  async function doLogout() {
    await logout();
    navigate({ name: "login" });
  }
</script>

<section class="panel">
  <h2>Settings</h2>
  <p>Role: {role}</p>
  <button onclick={doLogout}>Log out</button>
</section>
```

`src/client/ui/src/modules/core-ui/panels/StagePlaceholder.svelte`:

```svelte
<section class="stage"><p>Scene rendering arrives in M8.</p></section>
```

`src/client/ui/src/modules/core-ui/panels/TopBar.svelte`:

```svelte
<script lang="ts">
  import { getAppContext } from "../../../lib/appContext";
  const { world } = getAppContext();
</script>

<header class="topbar"><strong>shadowcat</strong> <span>world {world}</span></header>
```

`src/client/ui/src/modules/core-ui/panels/StatusBar.svelte`:

```svelte
<script lang="ts">
  import { getAppContext } from "../../../lib/appContext";
  const { role } = getAppContext();
</script>

<footer class="statusbar"><span>{role}</span></footer>
```

- [ ] **Step 4: Implement the module**

`src/client/ui/src/modules/core-ui/index.ts`:

```ts
import type { Module } from "@shadowcat/core";
import Settings from "./panels/Settings.svelte";
import StagePlaceholder from "./panels/StagePlaceholder.svelte";
import TopBar from "./panels/TopBar.svelte";
import StatusBar from "./panels/StatusBar.svelte";

/** First-party shell module: provides the region surfaces and contributes the
 * M7 default panels. Region content for M8+ tools/M11 chat/M12 browsers is
 * contributed by their own modules later. */
export const coreUi: Module = {
  manifest: {
    id: "core-ui",
    version: "0.1.0",
    dependencies: {},
    provides: [
      { contract: "shadowcat.surface:root", cardinality: "singleton" },
      { contract: "shadowcat.surface:topbar", cardinality: "singleton" },
      { contract: "shadowcat.surface:stage", cardinality: "singleton" },
      { contract: "shadowcat.surface:statusbar", cardinality: "singleton" },
      { contract: "shadowcat.surface:toolrail", cardinality: "multi" },
      { contract: "shadowcat.surface:sidebar", cardinality: "multi" },
    ],
  },
  register(ctx) {
    ctx.contributions.contribute({ id: "core-ui:topbar", contract: "shadowcat.surface:topbar", component: TopBar });
    ctx.contributions.contribute({ id: "core-ui:stage", contract: "shadowcat.surface:stage", component: StagePlaceholder });
    ctx.contributions.contribute({ id: "core-ui:statusbar", contract: "shadowcat.surface:statusbar", component: StatusBar });
    ctx.contributions.contribute({ id: "core-ui:settings", contract: "shadowcat.surface:sidebar", order: 0, component: Settings });
  },
};
```

- [ ] **Step 5: Implement Layout.svelte**

`src/client/ui/src/lib/Layout.svelte`:

```svelte
<script lang="ts">
  import Surface from "./Surface.svelte";
</script>

<div class="layout">
  <div class="topbar"><Surface contract="shadowcat.surface:topbar" /></div>
  <div class="toolrail"><Surface contract="shadowcat.surface:toolrail" /></div>
  <div class="stage"><Surface contract="shadowcat.surface:stage" /></div>
  <div class="sidebar"><Surface contract="shadowcat.surface:sidebar" /></div>
  <div class="statusbar"><Surface contract="shadowcat.surface:statusbar" /></div>
</div>

<style>
  .layout {
    display: grid;
    height: 100vh;
    grid-template-columns: 3rem 1fr 20rem;
    grid-template-rows: 2.5rem 1fr 1.5rem;
    grid-template-areas:
      "topbar topbar sidebar"
      "toolrail stage sidebar"
      "statusbar statusbar sidebar";
  }
  .topbar { grid-area: topbar; }
  .toolrail { grid-area: toolrail; }
  .stage { grid-area: stage; }
  .sidebar { grid-area: sidebar; overflow: auto; }
  .statusbar { grid-area: statusbar; }

  /* Phone: stack regions, sidebar below the stage. */
  @media (max-width: 40rem) {
    .layout {
      grid-template-columns: 1fr;
      grid-template-rows: 2.5rem 1fr auto 1.5rem;
      grid-template-areas: "topbar" "stage" "sidebar" "statusbar";
    }
    .toolrail { display: none; }
  }
</style>
```

- [ ] **Step 6: Run it to verify it passes**

Run: `pnpm --filter @shadowcat/ui exec vitest run src/modules/core-ui/coreUi.test.ts`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/client/ui/src/modules/core-ui src/client/ui/src/lib/Layout.svelte
git commit -m "feat(ui): core-ui module (region surfaces + default panels) + Layout"
```

---

### Task 7: App root — bootstrap + entry routing + in-world Table

**Files:**
- Modify: `src/client/ui/src/App.svelte` (replace the stub)
- Create: `src/client/ui/src/lib/Table.svelte`
- Create: `src/client/ui/src/App.test.ts`

**Interfaces:**
- Consumes: everything above; `coreUi` (Task 6); `webSocketConnect`.
- Produces: `App` (the root); `Table` (provides AppContext from a `WorldSession`,
  renders `Layout`).

- [ ] **Step 1: Implement `Table.svelte`**

`src/client/ui/src/lib/Table.svelte`:

```svelte
<script lang="ts">
  import { setAppContext } from "./appContext";
  import Layout from "./Layout.svelte";
  import type { WorldSession } from "./worldSession.svelte";

  let { session }: { session: WorldSession } = $props();
</script>

{#if session.role && session.world}
  {@const _ctx = setAppContext({
    contributions: session.contributions,
    store: session.store,
    world: session.world,
    role: session.role,
  })}
  <Layout />
{:else}
  <p class="connecting">Connecting…</p>
{/if}
```

> `setAppContext` must run during component init while `role`/`world` are set;
> if the `{@const}` ordering fights reactivity, set the context unconditionally
> from a `$derived` snapshot and guard `Layout` with the `{#if}` — keep the
> intent: AppContext is provided before `Layout` mounts.

- [ ] **Step 2: Write the failing App test**

`src/client/ui/src/App.test.ts`:

```ts
import { render, screen, waitFor } from "@testing-library/svelte";
import { test, expect, vi, afterEach } from "vitest";
import App from "./App.svelte";
import * as api from "./lib/api";

afterEach(() => vi.restoreAllMocks());

test("uninitialized server routes to Setup", async () => {
  vi.spyOn(api, "getConfig").mockResolvedValue({ initialized: false });
  vi.spyOn(api, "getMe").mockResolvedValue(null);
  render(App);
  expect(await screen.findByText("Create the admin account")).toBeTruthy();
});

test("initialized + unauthenticated routes to Login", async () => {
  vi.spyOn(api, "getConfig").mockResolvedValue({ initialized: true });
  vi.spyOn(api, "getMe").mockResolvedValue(null);
  render(App);
  await waitFor(() => expect(screen.getByRole("button", { name: "Log in" })).toBeTruthy());
});

test("authenticated routes to WorldSelect", async () => {
  vi.spyOn(api, "getConfig").mockResolvedValue({ initialized: true });
  vi.spyOn(api, "getMe").mockResolvedValue({ id: "u1", username: "gm", server_role: "user" });
  vi.spyOn(api, "listWorlds").mockResolvedValue([]);
  render(App);
  expect(await screen.findByText("Your worlds")).toBeTruthy();
});
```

- [ ] **Step 3: Run it to verify it fails**

Run: `pnpm --filter @shadowcat/ui exec vitest run src/App.test.ts`
Expected: FAIL — App still renders the old stub.

- [ ] **Step 4: Implement App.svelte**

`src/client/ui/src/App.svelte`:

```svelte
<script lang="ts">
  import { webSocketConnect } from "@shadowcat/core";
  import { getConfig, getMe, type Me } from "./lib/api";
  import { currentRoute, navigate } from "./lib/route.svelte";
  import { coreUi } from "./modules/core-ui/index";
  import { WorldSession } from "./lib/worldSession.svelte";
  import Setup from "./lib/views/Setup.svelte";
  import Login from "./lib/views/Login.svelte";
  import WorldSelect from "./lib/views/WorldSelect.svelte";
  import Table from "./lib/Table.svelte";

  let me = $state<Me | null>(null);
  let booted = $state(false);
  let session = $state<WorldSession | null>(null);

  async function boot() {
    const cfg = await getConfig();
    if (!cfg.initialized) {
      navigate({ name: "setup" });
      booted = true;
      return;
    }
    me = await getMe();
    navigate({ name: me ? "worlds" : "login" });
    booted = true;
  }
  boot();

  async function afterAuth() {
    me = await getMe();
    navigate({ name: "worlds" });
  }

  function enterWorld(worldId: string) {
    if (!me) return;
    const wsUrl =
      (location.protocol === "https:" ? "wss:" : "ws:") +
      "//" + location.host + "/ws?world=" + worldId;
    const s = new WorldSession({ selfId: me.id, connect: webSocketConnect(wsUrl), coreUiModule: coreUi });
    session = s;
    s.enter(worldId);
    navigate({ name: "world", id: worldId });
  }

  const route = $derived(currentRoute());
</script>

{#if !booted}
  <p class="connecting">Loading…</p>
{:else if route.name === "setup"}
  <Setup onDone={() => navigate({ name: "login" })} />
{:else if route.name === "world" && session}
  <Table {session} />
{:else if route.name === "worlds"}
  <WorldSelect onEnter={enterWorld} />
{:else}
  <Login onAuthed={afterAuth} />
{/if}
```

- [ ] **Step 5: Run it to verify it passes**

Run: `pnpm --filter @shadowcat/ui exec vitest run src/App.test.ts`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/client/ui/src/App.svelte src/client/ui/src/lib/Table.svelte src/client/ui/src/App.test.ts
git commit -m "feat(ui): App root — bootstrap, entry routing, in-world Table"
```

---

### Task 8: Playwright entry-flow smoke — MOVED TO M7c-2

> **Moved to M7c-2 during execution.** Wiring Playwright against the M7c-1 SPA
> requires a dual-process setup (`vite dev` + proxy + a dynamically-ported
> test-server), which is brittle cross-platform. Once M7c-2's embed flip has the
> single binary serving the SPA + `/api` on one origin, the e2e is a one-process
> spawn (`startTestServer`, point Playwright `baseURL` at it) — simpler and more
> faithful. M7c-1 ships full Vitest coverage of the entry flow + shell logic. The
> task below is retained for M7c-2's plan.

**Files:**
- Modify: `src/client/ui/package.json` (add `@playwright/test`; `e2e` script)
- Create: `src/client/ui/playwright.config.ts`
- Create: `src/client/ui/e2e/entry-flow.spec.ts`

**Interfaces:**
- Consumes: the running SPA (`vite dev`) + a spawned test-server (the core e2e
  `server-process.ts` pattern, driven from Playwright's `webServer`/global setup).

- [ ] **Step 1: Add the dependency + script**

Run: `pnpm --filter @shadowcat/ui add -D @playwright/test`
Then add to `src/client/ui/package.json` scripts: `"e2e": "playwright test"`.

- [ ] **Step 2: Write the Playwright config**

`src/client/ui/playwright.config.ts`:

```ts
import { defineConfig } from "@playwright/test";

// Drives the SPA (vite dev) against a real server. SHADOWCAT_SERVER points Vite's
// proxy at a test-server the harness starts (admin seeded via env), so the smoke
// walks login → world-select. setup-token is "off" on loopback.
export default defineConfig({
  testDir: "./e2e",
  webServer: {
    command: "vite dev --port 5174",
    url: "http://127.0.0.1:5174",
    reuseExistingServer: !process.env.CI,
    env: { SHADOWCAT_SERVER: process.env.SHADOWCAT_SERVER ?? "http://127.0.0.1:30000" },
  },
  use: { baseURL: "http://127.0.0.1:5174" },
});
```

- [ ] **Step 3: Write the smoke spec**

`src/client/ui/e2e/entry-flow.spec.ts`:

```ts
import { test, expect } from "@playwright/test";

// Assumes a test-server at SHADOWCAT_SERVER with admin "ops"/"pw-boot" seeded and
// the setup window closed (bootstrap_admin), matching the core e2e harness.
test("login → world-select → enter table", async ({ page }) => {
  await page.goto("/#/login");
  await page.getByLabel("Username").fill("ops");
  await page.getByLabel("Password").fill("pw-boot");
  await page.getByRole("button", { name: "Log in" }).click();

  await expect(page.getByText("Your worlds")).toBeVisible();
  await page.getByLabel("New world name").fill("Smoke World");
  await page.getByRole("button", { name: "Create world" }).click();

  // Entering a world reaches the table shell (the stage placeholder).
  await expect(page.getByText("Scene rendering arrives in M8.")).toBeVisible();
});
```

> The test-server lifecycle: the runner sets `SHADOWCAT_SERVER` to a server it
> started (reuse `src/client/core/src/e2e/server-process.ts` — `startTestServer`
> with `admin_user`/`admin_password` env, setup-token `off`). Wire it via a
> Playwright global-setup that spawns the server and exposes its URL, OR document
> the manual `SHADOWCAT_SERVER` precondition if global-setup is deferred. Do NOT
> silently skip the server dependency — if global-setup is out of scope for this
> task, mark the spec `test.skip` with a comment pointing at the precondition and
> log it to `TODO.md`.

- [ ] **Step 4: Run it (CI gates this; locally requires a server)**

Run: `SHADOWCAT_SERVER=<url> pnpm --filter @shadowcat/ui exec playwright test`
Expected: PASS against a seeded test-server (or skipped-with-reason if the server
harness wiring is deferred per the note).

- [ ] **Step 5: Commit**

```bash
git add src/client/ui/package.json src/client/ui/playwright.config.ts \
        src/client/ui/e2e/entry-flow.spec.ts pnpm-lock.yaml
git commit -m "test(ui): Playwright entry-flow smoke"
```

---

### Task 9: Full green + typecheck

- [ ] **Step 1: Typecheck**

Run: `pnpm --filter @shadowcat/ui typecheck`
Expected: no errors (svelte-check over all new components, the `WorldSession`,
the `core-ui` module, `Table`).

- [ ] **Step 2: Full ui unit suite**

Run: `pnpm --filter @shadowcat/ui test`
Expected: PASS — api/route/Login/WorldSelect/worldSession/coreUi/App + the M7b-3
Surface/smoke tests.

- [ ] **Step 3: Core unaffected**

Run: `pnpm --filter @shadowcat/core test`
Expected: PASS (100).

---

## Self-Review

**Spec coverage (spec §5–9, §11):**
- SPA scaffold + hash router + dev proxy (§5) → Tasks 1, 2, 7. ✓
- Entry views plain-routed (§6) → Tasks 3, 4. ✓
- WorldSession + AppContext extension + reconcile wiring (§7) → Task 5. ✓
- core-ui module + region surfaces + default panels + Layout (§8) → Task 6. ✓
- DocumentStore bridge (§9) → Task 5 (`reactiveStore`). ✓
- Vitest coverage + Playwright smoke (§11) → Tasks 1–8, 8. ✓

**Placeholder scan:** No TBD/TODO. Several steps carry "verify the real
mock-server / ModuleRegistry Deps / barrel exports and adapt" notes — these are
instructions to reconcile with actual core signatures during execution, not
content placeholders; the intent and the surrounding code are complete.

**Type consistency:** `api.*`, `Route`/`parseHash`/`navigate`/`currentRoute`,
`AppContext` (4 fields), `WorldSession` (`enter`/`leave`/`state`/`role`/`world`/
`contributions`/`store`), `coreUi` (`Module`), `Surface` contract ids, and the
view callback props (`onAuthed`/`onDone`/`onEnter`) are consistent across tasks.

## Out of scope (M7c-2 / M7d)

The `embed.rs` seam flip, `init_gate` rework, and static retirement (so the binary
serves the SPA) are **M7c-2**. Theming tokens, i18n `t()`, and session-restore are
**M7d**. This plan ships the SPA running under `vite dev`.
