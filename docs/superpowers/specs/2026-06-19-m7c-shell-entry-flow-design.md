# M7c — Shell + Entry Flow as Modules: Design Spec

> Status: **DRAFT for review.** The third M7 sub-milestone (parent:
> [`2026-06-19-m7-layout-theming-design.md`](2026-06-19-m7-layout-theming-design.md)).
> Assembles the running app: the Svelte 5 SPA, the entry flow (setup → login →
> world-select → table), the per-world session controller, and the first-party
> `core-ui` module that provides the region surfaces (on the M7b contribution
> architecture) — then flips the server to serve the SPA. Decomposed into
> **M7c-1** (client SPA + shell) and **M7c-2** (server embed integration).
>
> **Builds on:** M7a (server surface: `/api/config`, `/api/worlds`,
> `/api/me/ui-state`), M7b (contribution architecture: `ContributionRegistry`,
> manifest `provides`/`requires`, `<Surface>`, `appContext`, `reconcileTopology`,
> server contract mirror + `Welcome.contract_declarations`).

## 1. Goal

A self-hoster opens the app in a browser and reaches a working table:
1. **Entry flow** — `GET /api/config` routes uninitialized → **Setup**; else
   `GET /api/me` 401 → **Login**, 200 → **WorldSelect**; entering a world → **Table**.
2. **In-world shell as modules** — entering a world starts the `WsClient`, loads
   the first-party `core-ui` module which **provides** the region surfaces and
   **contributes** the default panels; the Table renders surfaces via `<Surface>`.
3. **The single binary serves the SPA** — `embed.rs` serves the Vite `dist/`
   bundle; `src/server/static/` is retired.

## 2. Decomposition

- **M7c-1 — Client SPA + shell. ✅ DONE** (merged `--no-ff` to local main, not
  pushed). Hash router + typed API client + Vite dev proxy; plain-routed
  Setup/Login/WorldSelect; App bootstrap; `WorldSession` controller; `core-ui`
  module (region surfaces + default panels) + `Layout`; AppContext extension +
  DocumentStore bridge. Single-reviewed: fixed the store-wiring Critical (feed
  both mirrors), reconnect-idempotency + boot-resilience Importants, plus minors.
  ui 18 unit tests + typecheck green. Reachable via `vite dev` proxying to a
  test-server; the binary serves the old static bundle until M7c-2.
  Plan: [`plans/2026-06-19-m7c-1-client-spa.md`](plans/2026-06-19-m7c-1-client-spa.md).
- **M7c-2 — Server embed integration. ✅ DONE** (merged `--no-ff` to local main,
  not pushed). Flipped `embed.rs` `static/` → `../../dist/`; **removed** `init_gate`
  (SPA + endpoint self-gating cover it); re-homed favicon/PWA assets into
  `src/client/ui/public/`; retired `src/server/static/`; CI builds the client in
  the `rust` matrix job before any cargo step; Playwright entry-flow smoke against
  the built binary (`ui-e2e` CI job). Single-reviewed: no Critical/Important
  (init_gate removal confirmed safe); fixed stale `/setup.html` references. Binary
  hand-verified serving the SPA + login; release build embeds `dist/`.
  Plan: [`plans/2026-06-19-m7c-2-server-embed.md`](plans/2026-06-19-m7c-2-server-embed.md).

**M7c COMPLETE** — the single binary serves the running, navigable SPA. Next: M7d
(theming + i18n + session-restore).

## 3. Non-goals (deferred)

- **Session-restore** (lastWorld / active tab / locale persistence) → **M7d**
  (session wiring). M7c's flow is login → world-select → table with no
  persistence; reload returns to world-select.
- **Theming** → **M7d.** M7c uses **minimal structural styles** (scoped CSS:
  visible grid regions, readable forms), no token system; M7d replaces it with
  the 3-tier SCSS tokens + dark theme.
- **i18n** → **M7d.** Strings are literal in M7c; routed through `t()` in M7d.
- Real content panels (chat M11, browsers M12), canvas (M8) — the stage is a
  placeholder; sidebar Chat/browser tabs are empty stubs.

## 4. Revises the parent spec

**Parent §6 said entry-flow views are `:root` contributions.** This spec
**revises that**: entry views (Setup/Login/WorldSelect) are **plain routed
components**, and the contribution architecture (`core-ui` + surfaces + panels)
activates **only in-world** (Table). Rationale: modules load per-world (the
`Welcome` frame carries the world's contract topology; `reconcileTopology`
compares against it), so before entering a world no module system is active and
no surface host exists — a "contribution with no host" is incoherent. The shell
is in-world chrome; the entry flow is pre-world plumbing.

## 5. SPA scaffold & routing (M7c-1)

- `src/client/ui/src/App.svelte` (currently a title stub) becomes the app root;
  `main.ts` mounts it (already does).
- **Hash router** — `src/client/ui/src/lib/route.svelte.ts`: a `$state` current
  route parsed from `location.hash`, updated on `hashchange`; routes `#/setup`,
  `#/login`, `#/worlds`, `#/world/:id`. No router dependency (4 routes).
- `vite.config.ts` gains `server.proxy` for `/api` (HTTP) and `/ws` (WS) → the
  Rust server (configurable target; default `http://127.0.0.1:30000`), so
  `vite dev` runs the SPA against a real backend.
- **Bootstrap** (`App.svelte` on mount): `GET /api/config`; uninitialized →
  Setup. Else `GET /api/me`: 401 → Login; 200 → WorldSelect. The router then
  honors the hash for navigation.

## 6. Entry-flow views (M7c-1, plain routed)

`src/client/ui/src/lib/views/`:
- **`Setup.svelte`** — username/password (+ optional setup token) → `POST
  /api/setup`; on 204 → Login. Inline error on 403 (token)/409 (initialized).
- **`Login.svelte`** — username/password → `POST /api/login`; on 204 re-fetch
  `/api/me` → WorldSelect. Inline error on 401.
- **`WorldSelect.svelte`** — `GET /api/worlds` → list `{id,name,role}`; clicking a
  world routes to `#/world/:id`. A "create world" affordance (`POST /api/worlds`).
These replace the transitional `src/server/static/{login,setup}.html` + `auth.js`
(retired in M7c-2).

## 7. The per-world session controller (M7c-1)

`src/client/ui/src/lib/worldSession.svelte.ts` — a `WorldSession` owning the
in-world lifecycle, the single orchestration point:
- **`enter(worldId)`:** construct `DocumentStore`, `OptimisticClient`,
  `ContributionRegistry`, `ModuleRegistry`; construct `WsClient`
  (`webSocketConnect` to `/ws?world=<id>`) with handlers; `start()`. On
  **`Welcome`**: capture `world` + `role`; run `reconcileTopology(
  registry.declarations(), welcome.contract_declarations)`; add + activate the
  `core-ui` module. Expose reactive `connectionState`
  (`connecting|open|reconnecting|closed`) and `role`/`world`.
- **`leave()`:** `WsClient.stop()`; `ModuleRegistry.unload` (cascade); clear.
- **AppContext** (extends M7b-3's): `{ contributions, store, world, role }` —
  `store`/`world`/`role` added here; `t` (i18n) still M7d. Provided at the Table
  root via `setAppContext`.

## 8. The `core-ui` module + Table shell (M7c-1)

- `src/client/ui/src/modules/core-ui/` — a first-party module (bundled,
  statically `add()`-ed to the `ModuleRegistry` in `WorldSession.enter`):
  - **Manifest** `provides`: `shadowcat.surface:root` (singleton),
    `…:topbar`/`…:statusbar`/`…:stage` (singleton), `…:toolrail`/`…:sidebar`
    (multi).
  - **`register(ctx)`** contributes the M7 defaults: a `Settings` panel (logout
    via `POST /api/logout` → Login; role badge) + empty stub tabs (Chat→M11,
    browsers→M12) into `:sidebar`; a stage placeholder ("scene — M8") into
    `:stage`; world name + connection indicator + user menu into `:topbar`;
    connection + world + role into `:statusbar`. The toolrail surface is provided
    empty (M8/M9/M10 fill it).
- **`Layout.svelte`** — the CSS-grid render target for the Table, hosting
  `<Surface contract="…">` for each region (the M7b-3 component). Responsive:
  regions stack on a phone viewport, sidebar → drawer; viewport meta declared in
  `index.html`.
- **Table view** routes to `Layout` once `WorldSession` is entered and the
  `core-ui` module active.

## 9. DocumentStore reactivity bridge (M7c-1)

A `createSubscriber` bridge over `DocumentStore.subscribe` (the pattern M7b-3
used for the registry), exposed through AppContext so future document-driven
panels read reactively. Established here per parent §7; lightly used in M7c (no
real document panels yet — Settings/stubs don't read documents).

## 10. Server embed integration (M7c-2)

- **`embed.rs`:** `#[folder = "static/"]` → the client `dist/` output (relative
  path from `src/server/`); `static_handler` callers unchanged.
- **`init_gate` rework:** while uninitialized, **serve the SPA** (so its Setup
  view renders) instead of redirecting to `/setup.html`. The allowlist no longer
  references `/setup.html`/`/auth.js`; `/api/config` (M7a) is the SPA's
  uninitialized signal. Non-asset, non-API routes serve `index.html`.
- **Retire** `src/server/static/` entirely (login/setup/index html + auth.js +
  styles).
- **Build ordering:** the client `dist/` must build **before** the server embeds
  it — wire into the single-binary build + CI (client build step precedes the
  Rust build that runs `rust-embed`).
- **Embed tests:** the binary serves `index.html` at `/` and named `dist/`
  assets; uninitialized serves the SPA (not a redirect).

## 11. Testing

- **Vitest + @testing-library/svelte (M7c-1):** router/bootstrap logic
  (config/me → view); entry-view form behavior (success + error paths, mocked
  fetch); `WorldSession` enter/leave lifecycle against the core `mock-server.ts`
  transport (Welcome → role/world/reconcile, core-ui activation, leave teardown);
  `core-ui` surface registration + default contributions; `Layout` renders the
  region surfaces; AppContext provision.
- **One Playwright smoke (M7c-2, moved from M7c-1):** setup → login →
  world-select → enter table. **Moved to M7c-2** because once the binary serves
  the SPA + `/api` on one origin (the embed flip), the e2e is a single-process
  spawn (`startTestServer`, point Playwright `baseURL` at it) — far simpler and
  more faithful than M7c-1's dual-process `vite dev` + proxy + dynamically-ported
  test-server. New dev-dep `@playwright/test` + a CI job across the matrix, in
  M7c-2.
- **Server (M7c-2):** embed tests (§10); `init_gate`-serves-SPA test; the
  Playwright entry-flow smoke against the built binary.
- Responsive/touch reflow asserted in the `Layout` Vitest test (CLAUDE.md
  mobile invariant).

## 12. New surface (summary)

- **Client (M7c-1):** App root + hash router + Vite dev proxy; Setup/Login/
  WorldSelect views; `WorldSession`; `core-ui` module + `Layout` + default
  panels; AppContext extension; DocumentStore bridge; Vitest + Playwright.
- **Server (M7c-2):** `embed.rs` seam flip; `init_gate` rework; static
  retirement; build-ordering wiring; embed tests. No new endpoints (M7a already
  shipped `/api/config`, `/api/worlds`, `/api/me/ui-state`).

## 13. Decisions (resolved at brainstorm)

1. **Entry views → plain routed**, not `:root` contributions; surfaces activate
   in-world only (revises parent §6). §4.
2. **Hash routing**, no router dependency (parent §4 confirmed). §5.
3. **`WorldSession` controller** owns the per-world lifecycle (WsClient/store/
   registries/modules); the single orchestration point. §7.
4. **M7c-1 / M7c-2 split** — client SPA, then server embed flip. §2.
5. **Minimal structural styles** in M7c (scoped CSS, no tokens); theming is M7d.
   §3.
6. **Playwright smoke → M7c-2** (moved from M7c-1): run against the built binary
   serving SPA + `/api` on one origin (single-process spawn), which is simpler and
   more faithful than M7c-1's dual-process `vite dev` + proxy + dynamically-ported
   test-server. M7c-1 ships full Vitest coverage of the entry flow + shell logic;
   the binary e2e lands with the embed flip. §11.
