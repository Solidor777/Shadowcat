# M7 — Layout-lite + Theming Scaffold: Design Spec

> Status: **DRAFT for review.** First UI milestone. M7 turns the stub
> `src/client/ui/` into the default Svelte 5 UI over the headless
> `@shadowcat/core`, and does so as a **UI-as-modules contribution
> architecture**: every UI element (regions, panels, future combat tracker,
> dice tray, HUDs) is a module that contributes components into **surfaces**
> declared by other modules, with contract-based (virtual) dependencies, built
> on the existing M6b module system. Covers the full entry flow (setup → login →
> world select → table), one brand-derived dark theme via a 3-tier SCSS token
> system, an i18n scaffold, and per-user UI session state in the DB. Decomposed
> into **M7a** (server surface), **M7b** (UI contribution architecture), **M7c**
> (shell + entry flow as modules + reactivity bridge), **M7d** (theming + i18n +
> session + tests) — each its own plan+execute cycle, mirroring the M6 split.
>
> **Pre-release framing:** no public release until ≥2 internal systems exercise
> the API (Phase 4 freeze gate). There are therefore no third-party consumers to
> break during M7→pre-release; the contribution API is built in full now and
> hardens through internal use. "Internal/0.x" means *unfrozen and free to
> evolve*, not *hidden behind a smaller surface*.

## 1. Goal

A default Svelte 5 UI a self-hoster reaches in a browser, where the UI itself is
a composition of modules:

1. **Entry flow** — on load, route to first-run **Setup**, **Login**,
   **WorldSelect**, or the in-world **Table** based on server state and session.
2. **Contribution architecture** — a generic, assumption-free mechanism where a
   module **declares a surface** (a named mount point, by string contract) and
   any module **contributes** components into surfaces by contract. Inter-module
   dependencies resolve **by contract** ("requires *a* sidebar"), not just by
   module id. Nothing in the architecture names a specific element.
3. **Table shell as modules** — the region skeleton (top bar · tool rail · stage
   · sidebar · status bar) is provided by a first-party `core-ui` module as
   surfaces; the default panels (Settings + stubs) are contributions. Later
   milestones (chat M11, combat tracker, browsers M12) add modules, not layout
   edits.
4. **One dark theme** via a 3-tier SCSS token system, palette derived from
   `assets/icon`.
5. **i18n scaffold** — all UI strings route through `t()`; one `en` locale.
6. **Session state in the DB** — per-user opaque UI-state blob restoring active
   world, active tab, locale across reload and server restart.

The Vite bundle replaces the transitional `src/server/static/` HTML; `embed.rs`'s
documented seam flips from `static/` → client `dist/`.

## 2. Non-goals

- **Deferred to Phase 2** (PLAN line 99/131): drag-resize, pop-out / multi-window,
  multi-theme, user themes, module *styling* modes. M7 builds the panel/surface
  system; Phase 2 makes the arrangement user-customizable and persisted.
- **Deferred within the contribution architecture (no definable answer without a
  real second provider):** the multi-provider **conflict policy** for a
  `singleton` surface (two modules claiming the same singleton contract — who
  wins / user selection) and capability **version negotiation**. M7 ships the
  contract data model (incl. a `singleton`/`multi` cardinality marker) and
  deterministic single-provider resolution; a duplicate `singleton` provider
  **fails loudly** until the policy lands (§5.5).
- **Deferred to M8+:** scene/canvas rendering (the stage is a placeholder), chat
  (M11), actor/scene browsers and sheets (M12), user-roster presence (status bar
  shows connection state only).
- **Not threatened:** whole-UI replacement (a modder shipping a different-framework
  UI against the headless core) stays possible — the contribution architecture is
  expressed as a framework-neutral **service contract**, so an alternate host can
  implement it (§5.4).

## 3. Package & boundaries

- **Core (`src/client/core/`, framework-neutral):** owns **contract resolution** —
  manifest `provides`/`requires` fields, activation ordering and presence checks
  generalized from the existing id+semver `depsSatisfied`/`topoSort`. No DOM, no
  Svelte. (§5.2)
- **UI (`src/client/ui/`, Svelte 5 + Vite):** owns the **surface host** — a
  `ui.surfaces` service (provided by the shell bootstrap, consumed by modules via
  `ctx.services`) that registers surface mount targets and mounts/teardowns
  contributed Svelte components. Depends on `@shadowcat/core` (`workspace:*`) and
  reaches document state only through the §7 reactivity bridge.
- Server changes are additive (§9). Build ordering becomes load-bearing: the
  client `dist/` must build **before** the server embeds it (CI + single-binary
  build wire this).

## 4. Entry flow & routing

- **Views:** `Setup`, `Login`, `WorldSelect`, `Table` (each a contribution into a
  root surface — see §6).
- **Routing:** hash-based (`#/login`, `#/worlds`, `#/world/:id`). No server SPA
  fallback needed; `static_handler`'s 404 behavior is untouched. The active world
  is also restored from session state (§8) as the authoritative source.
- **Bootstrap on load:**
  1. `GET /api/config` → `{ initialized }`. `false` → **Setup**.
  2. `GET /me`. `401` → **Login**. `200` → authenticated.
  3. Authenticated: restore `lastWorld` from session state → **Table** for that
     world, else **WorldSelect**.
- **Login** → `POST /login` then re-fetch `/me`. **Setup** → `POST /setup`
  (admin username/password + setup token when required), folded into the themed
  SPA (all of `src/server/static/` retired).
- **Entering a world** opens `WsClient` to `/ws?world=<id>`, consumes `Welcome`
  (world cap-defaults + role), binds the Table to the `DocumentStore`. **Leaving**
  closes the socket and clears the bound store. Connection state surfaces in the
  status bar from `WsClient`.

## 5. UI contribution architecture

The heart of M7. Built on M6b primitives (`ModuleRegistry`, `ServiceRegistry`,
`HookBus`) — minimal genuinely-new surface.

### 5.1 Concepts

- **Contract** — a string id naming an extension point or interface, e.g.
  `shadowcat.surface:sidebar`, `shadowcat.surface:root`, `shadowcat.service:combat`.
  Generic; core attaches no meaning to any specific id.
- **Surface** — a UI contract: a named mount point. A module **declares** it
  provides a surface (manifest `provides`) and at runtime **registers** the mount
  target via the `ui.surfaces` service.
- **Contribution** — a component + metadata (`{ contract, order, id, titleKey?,
  icon?, component }`) a module registers into a surface contract.
- **Cardinality** — a contract is `singleton` (one provider; e.g. "the sidebar")
  or `multi` (many; e.g. a list of sidebar panels). Declared with the contract.

### 5.2 Manifest + resolution (core, framework-neutral)

Extend `ModuleManifest` (currently `dependencies: Record<id, semver>`,
`capabilities`, `requirements`, `hooks`) with two **interface-contract** fields,
kept distinct from the existing server-permission `capabilities`:

```ts
provides?: { contract: string; cardinality: "singleton" | "multi" }[];
requires?: string[];   // contract ids this module needs an active provider for
```

`ModuleRegistry` resolution generalizes the existing logic:
- **`depsSatisfied`** also checks every `requires` contract has ≥1 active
  provider (alongside id+semver deps).
- **`topoSort`** adds edges requirer → provider(s) of each required contract, so
  providers activate first. Cycles still throw.
- **Single-provider rule (M7):** a `singleton` contract with a second active
  provider throws (mirrors `ServiceRegistry.provide`'s existing duplicate-name
  hard error). The conflict policy (§2 deferred) refines this later; the
  deterministic loud failure is the safe placeholder, not a guess.
- Unload teardown is unchanged (registry already tracks per-module registrations;
  the `ui.surfaces` service cleans contributions on `removeModule`).

### 5.3 Surface host (ui package, Svelte)

- The shell bootstrap **provides** a `ui.surfaces@1` service:
  ```ts
  interface UiSurfaces {
    registerSurface(contract: string, target: SurfaceTarget): () => void;
    contribute(contract: string, c: Contribution): () => void;
    contributionsFor(contract: string): readonly Contribution[]; // reactive
  }
  ```
- Modules get it via `ctx.services.get("ui.surfaces")` and register/contribute in
  their `register(ctx)`. The host mounts each contribution's Svelte component into
  the surface's target and tears it down on unload (the registry already drives
  unload). `contributionsFor` is reactive (rune-backed) so a surface re-renders
  when contributions change.

### 5.4 Framework neutrality

`UiSurfaces` is a TS interface; only `component` values are Svelte-specific. An
alternate-framework UI can provide its own `ui.surfaces` host implementation,
preserving whole-UI replacement. Core never imports it.

### 5.5 What "fails loudly" means

A duplicate `singleton` provider, a `requires` with no provider, or a contribution
to an unknown surface logs at error and refuses activation of the offending module
(via the existing `depsSatisfied` "not activated" path) — never a silent no-op.

## 6. Table shell as the `core-ui` module

- A first-party `core-ui` module **provides** the root + region surfaces:
  `shadowcat.surface:root` (`singleton`), and within the Table,
  `…:topbar`/`…:toolrail`/`…:stage`/`…:sidebar`/`…:statusbar` (toolrail/sidebar as
  `multi`; stage `singleton`). `Layout.svelte` is its CSS-grid render target.
- **Entry-flow views** (Setup/Login/WorldSelect/Table) are contributions into
  `:root`, switched by the hash router.
- **M7 default contributions:** a `Settings` panel (logout, locale switcher, role
  badge) + empty stub tabs reserved for Chat (M11)/browsers (M12) contribute to
  `:sidebar`; a stage placeholder ("scene rendering — M8") to `:stage`; world
  name + connection indicator + user menu to `:topbar`; connection/world/role to
  `:statusbar`. The toolrail is an empty `multi` surface (M8/M9/M10 fill it).
- **Responsive/touch (CLAUDE.md invariant):** regions stack on a phone viewport,
  sidebar becomes a drawer; `index.html` declares the responsive viewport;
  targets are touch-sized. Verified in tests.

This proves the architecture against its first real consumers and means M11/M12
add a module + contributions, never a layout rewrite.

## 7. Core↔Svelte reactivity bridge

`DocumentStore` exposes `subscribe(listener) → unsubscribe` + `snapshot()/query()/
get()`. A thin adapter (`src/client/ui/src/lib/coreStore.svelte.ts`) wraps
`subscribe` with **`createSubscriber`** (`svelte/reactivity`); read accessors call
the tracker then read through, so any rune-tracked read re-runs on the store's
`emit()`. No polling, no new dependency, UI never holds core internals. Single
sanctioned bridge for reactive document reads. (`contributionsFor` in §5.3 uses
the same rune-reactive pattern for the contribution registry.)

## 8. Session state — opaque per-user blob

- **Server model** (mirrors the opaque document-body invariant): per-user JSON
  `ui_state`. Server validates only *is a JSON object* + *under a size cap*; never
  interprets contents. Endpoints `GET /me/ui-state` (returns `{}` when unset),
  `PUT /me/ui-state` (replace; 422 on non-object/over cap). Stored as a `ui_state`
  TEXT column on `users` (§14.2), added by a new migration.
- **Client owns structure:**
  ```jsonc
  { "global": { "locale": "en", "lastWorld": "<uuid|null>" },
    "worlds": { "<worldId>": { "activeTab": "settings" } } }
  ```
  Restored on load (drives entry routing + initial locale/tab); written
  (debounced) on change. Survives reload and server restart; cross-device.
- **Substrate:** M8 adds per-user "active scene" under `worlds.<id>`; Phase 2 adds
  panel arrangement — both extend the blob with no schema change.

## 9. New server surface

Additive; no change to the M5 write path or M4/M6 wire protocol.

1. **`GET /worlds`** — worlds the caller can access, with role: `[{id,name,role}]`
   (new `Repository` query; WorldSelect needs it — only `POST /worlds` exists).
2. **`GET /api/config`** — public `{ initialized }` for Setup-vs-Login routing.
3. **`GET/PUT /me/ui-state`** + migration — the §8 blob.
4. **`embed.rs` seam flip** — `#[folder = "static/"]` → client `dist/`; retire all
   of `src/server/static/`; rework `init_gate` to serve the SPA for the setup view
   instead of redirecting to `/setup.html`; update the single-binary build + embed
   tests; CI build ordering (client → server). **Lands in M7c, not M7a** — the flip
   needs the Svelte bundle to exist (serving an empty `dist/` and breaking the
   setup redirect otherwise), so it ships with the shell that produces `dist/`.

## 10. Theming — 3-tier SCSS token system

Brand-derived from `assets/icon` (cool near-black "shadow cat" on a vivid
brand-blue circle, bright-blue eyes). Single dark theme; near-black base, never
pure black.

- **Tier 1 primitives** (raw): cool charcoal surface ramp + brand-blue accent ramp
  sampled from the icon, spacing scale, type scale, radii, z-index. Starting
  values (refined to exact samples in M7d):

  | role | token | start hex |
  |---|---|---|
  | accent (circle) | `--blue-500` | `#3D7BE8` |
  | accent bright (eyes) | `--blue-400` | `#5B9CF8` |
  | accent pressed | `--blue-600` | `#2E63C4` |
  | surface base | `--ink-900` | `#16181D` |
  | surface raised | `--ink-800` | `#1E2128` |
  | surface overlay | `--ink-700` | `#272B33` |
  | border | `--ink-600` | `#333843` |
  | text primary | `--ink-50` | `#E6E9EF` |
  | text muted | `--ink-300` | `#9AA1AD` |
  | danger | `--red-500` | `#E5565B` |
  | success | `--green-500` | `#3FB079` |
  | warning | `--amber-500` | `#E0A23B` |

- **Tier 2 semantic:** `surface/{base,raised,overlay}`, `text/{primary,muted}`,
  `accent`/`accent-hover`/`accent-active`, `border`, `danger|success|warning`.
  Components reference only this tier.
- **Tier 3 component:** per-component vars referencing tier 2.
- Exposed as CSS custom properties so Phase 2 theme switching swaps tier-2 values
  without recompiling components. WCAG AA verified. **Re-audit gates (PLAN line
  98):** provisional set, re-audited at M8 (canvas overlays) and M12 (sheets).

## 11. i18n scaffold

- **`typesafe-i18n`** (§14.1): generated types from the locale dictionary align
  with the project's type-safety posture (ts-rs, Zod); small tree-shaken runtime.
  All UI strings via `t()`; one `en` locale; locale switcher present (scaffold).
- New dependency → logged in `ARCHITECTURE.md` with license + rationale.

## 12. Testing

`ui` package has no tests today. M7 establishes the setup.

- **Vitest + `@testing-library/svelte`**: contract resolution (provides/requires
  ordering, missing-provider refusal, duplicate-singleton throw), the surface host
  (register/contribute/teardown, reactive `contributionsFor`), the reactivity
  bridge, the i18n seam, bootstrap/routing logic, token application, responsive
  reflow.
- **One Playwright smoke** reusing the core e2e spawned-server harness
  (`server-process.ts`): setup → login → world-select → enter table.
- CI matrix gains the UI unit job + the Playwright job; build ordering added.

## 13. Sub-milestone decomposition

Each its own plan → execute → review cycle (mirroring M6a/b/c).

- **M7a — Server surface.** `GET /api/worlds`, `GET /api/config`,
  `GET/PUT /api/me/ui-state` + migration. Purely additive (existing static auth
  flow untouched); independently shippable. Rust tests. Buddy-check candidate
  (auth-adjacent reads: world-list visibility + `ui_state` on the users table).
  Plan: [`superpowers/plans/2026-06-19-m7a-server-surface.md`](plans/2026-06-19-m7a-server-surface.md).
  (The `embed.rs` seam flip + `init_gate` rework + static retirement moved to M7c
  — they need the Svelte bundle to exist.)
- **M7b — UI contribution architecture.** Core manifest `provides`/`requires` +
  resolution generalization (incl. singleton loud-fail); the `ui.surfaces` host
  service in the ui package + Svelte mount/teardown + reactive `contributionsFor`.
  Vitest (headless core resolution + host). The foundation everything else rides.
- **M7c — Shell + entry flow as modules + bridge.** Svelte SPA scaffold (Vite dev
  proxy), hash router, the `core-ui` module providing root/region surfaces,
  Setup/Login/WorldSelect/Table contributions, `WsClient`/`Welcome` lifecycle, the
  `createSubscriber` bridge, default panels (Settings + stubs) as contributions.
  **Also: the `embed.rs` seam flip → `dist/`, `init_gate` rework to serve the SPA
  setup view, static retirement, and CI client→server build ordering** (moved from
  M7a; needs the bundle this sub-milestone produces). Vitest + the Playwright smoke.
- **M7d — Theming, i18n, session wiring.** 3-tier SCSS tokens + dark theme,
  `typesafe-i18n` + `en` + switcher, session-state load/save wiring, Settings
  panel content, responsive reflow. Remaining Vitest; `ARCHITECTURE.md` dependency
  log + token re-audit note.

## 14. Decisions (resolved at sign-off)

1. **i18n library → `typesafe-i18n`** — type-safety alignment; small runtime.
2. **`ui_state` storage → `ui_state` TEXT column on `users`** — single-row
   read/write; the opaque-blob contract means UI churn never alters the column
   *shape*, so co-locating with the auth table carries no migration risk.
3. **Setup → folded into the themed SPA** — single styling system; all of
   `src/server/static/` retired.
4. **UI moddability → full contribution architecture now, unfrozen.** Pre-release
   (no public until ≥2 internal systems), so no third parties to break; the API
   hardens through internal use. Architecture built in full; only the
   multi-provider singleton **conflict policy** + version negotiation are deferred
   (no definable answer without a real second provider), with a deterministic
   loud-fail placeholder. §2, §5.5.
5. **Contract identity → string-keyed, framework-neutral**, distinct from the
   server-permission `capabilities` field; `provides` carries a
   `singleton`/`multi` cardinality marker. §5.2.
