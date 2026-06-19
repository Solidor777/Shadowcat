# M7 — Layout-lite + Theming Scaffold: Design Spec

> Status: **DRAFT for review.** First UI milestone. Scope: a themed Svelte 5 SPA
> wrapping the finished headless `@shadowcat/core`, covering the full entry flow
> (first-run setup → login → world select → in-world table shell), a fixed
> VTT-standard panel layout built on a movable-later region/panel abstraction,
> one brand-derived dark theme via a 3-tier SCSS token system, an i18n scaffold,
> and per-user UI session state persisted in the DB. Decomposed into **M7a**
> (server surface), **M7b** (shell + entry flow + reactivity bridge), **M7c**
> (theming + i18n + session wiring + tests) — each its own plan+execute cycle,
> mirroring the M6 split.

## 1. Goal

Turn the stub `src/client/ui/` (`App.svelte` currently renders only a title)
into the default Svelte 5 UI that a self-hoster actually reaches in a browser:

1. **Entry flow** — on load, route to first-run admin **Setup**, **Login**,
   **WorldSelect**, or the in-world **Table** based on server state and session.
2. **Table shell** — a fixed VTT-standard region layout (top bar · tool rail
   stub · stage · tabbed sidebar · status bar) whose panels are self-contained
   components slotted into **named regions**, so Phase 2 can make the regions
   draggable/poppable without rewriting any panel.
3. **One dark theme** via a 3-tier SCSS token system, palette derived from the
   app icon (`assets/icon`).
4. **i18n scaffold** — every UI string routes through a `t()` lookup; one `en`
   locale; locale switcher present though only `en` exists.
5. **Session state in the DB** — per-user opaque UI-state blob restoring active
   world, active tab, and locale across reload and server restart.

The Vite bundle replaces the transitional `src/server/static/` HTML; `embed.rs`'s
documented seam flips from `static/` → client `dist/`.

## 2. Non-goals (explicitly later, per PLAN line 99 / Phase 2 line 131)

Drag-resize, pop-out / multi-window, multi-theme, user themes, module styling
modes — all **Phase 2** ("layout / theming completion"). M7 builds the *panel
system*; Phase 2 makes it customizable. Also out: scene/canvas rendering (M8 —
the stage is a placeholder); chat (M11); actor/scene browsers and sheets (M12);
user-roster presence (needs server support that does not yet exist — status bar
shows connection state only); exposing panel registration as a public module API
(Module API stays 0.x; the registry is internal in M7).

## 3. Package & boundaries

- All UI lives in `src/client/ui/` (Svelte 5 + Vite, present as a stub). It
  depends on `@shadowcat/core` (`workspace:*`) and never reaches into core
  internals — all document state flows through the §6 reactivity bridge.
- The server changes are additive (§8): new read endpoints, one migration, and
  the `embed.rs` folder seam flip. No change to the M5 write path or M4/M6
  protocol.
- Build ordering becomes load-bearing: **the client bundle (`dist/`) must build
  before the server embeds it.** CI and the single-binary build wire this
  ordering (§8.4, §10).

## 4. Entry flow & routing

- **Views:** `Setup`, `Login`, `WorldSelect`, `Table`.
- **Routing:** hash-based (`#/login`, `#/worlds`, `#/world/:id`). Hash routing
  needs no server-side SPA fallback, so `static_handler`'s existing 404 behavior
  for unknown paths is untouched. URL reflects the view so refresh returns the
  user to the right place; the active world is *also* restored from session
  state (§7) as the authoritative source.
- **Bootstrap sequence on load:**
  1. `GET /api/config` → `{ initialized }`. `false` → **Setup**.
  2. `GET /me`. `401` → **Login**. `200` → authenticated.
  3. Authenticated: restore `lastWorld` from session state and route to
     **Table** for that world, else **WorldSelect**.
- **Login** posts to the existing `POST /login`, then re-fetches `/me`.
  **Setup** posts to the existing `POST /setup` (admin username/password, plus
  the setup token when the server requires one), then routes to Login.
- **Entering a world** opens `WsClient` to `/ws?world=<id>`, consumes the
  `Welcome` frame (world capability-defaults + the actor's role), and binds the
  Table to the `DocumentStore`. **Leaving** closes the socket and clears the
  bound store. Connection state (connecting / open / reconnecting / closed) is
  surfaced in the status bar from `WsClient`.

## 5. Table layout — VTT-standard region skeleton

`Layout.svelte` is a CSS-grid with five **named regions**:

```
+--------------------------------------------------+
| topbar:  world name | conn indicator | user menu |
+----+----------------------------------+----------+
| t  |                                  | sidebar  |
| o  |   stage (M8 canvas placeholder)  | (tabbed) |
| o  |                                  |          |
| l  |                                  |          |
| r  +----------------------------------+          |
| a  | statusbar: conn · world · role   |          |
+----+----------------------------------+----------+
```

- **Region/panel abstraction.** Each region is filled by a self-contained
  component. The sidebar hosts a `TabbedPanel` driven by a **static internal
  panel registry**: an array of `{ id, icon, titleKey, component }`. Later
  milestones add a panel by *registering an entry*, never by editing the layout;
  Phase 2 makes the same named regions draggable without touching panels. This
  is the structural payoff of building the panel system now.
- **M7 real content:**
  - *topbar* — world name, connection indicator, user menu (logout).
  - *toolrail* — present but a stub (icons disabled / placeholder); becomes the
    home for M8 scene controls, M9 walls/vision, M10 tokens.
  - *stage* — placeholder ("scene rendering arrives in M8").
  - *sidebar* — `Settings` panel (logout, locale switcher, role badge) plus
    empty stub tabs reserved for Chat (M11) and browsers (M12).
  - *statusbar* — connection state + world name + role badge (real data only).
- **Responsive/touch (CLAUDE.md invariant):** the layout reflows to a phone
  viewport (regions stack; sidebar becomes a drawer); `index.html` declares the
  responsive viewport; interactive targets are touch-sized. Verified in tests.

## 6. Core↔Svelte reactivity bridge

The core `DocumentStore` exposes `subscribe(listener): () => void` plus
`snapshot()/query()/get()` — a classic external-store shape. The bridge:

- A thin adapter (`src/client/ui/src/lib/coreStore.svelte.ts`) wraps
  `DocumentStore.subscribe` with **`createSubscriber`** from `svelte/reactivity`.
  Read accessors (`query(type)`, `get(id)`, `snapshot()`) call the subscriber's
  tracker and then read through to the store, so any rune-tracked read in a
  component re-runs when the store calls `emit()`.
- No polling, no new dependency, and the UI never holds core internals. This is
  the **single sanctioned bridge**; all reactive document reads go through it.

## 7. Session state — opaque per-user blob

- **Server model (mirrors the opaque document-body invariant — ARCHITECTURE
  §"Validation at boundaries"):** a per-user JSON `ui_state`. The server
  validates only *is a JSON object* and *under a size cap*; it never interprets
  the contents. Endpoints: `GET /me/ui-state` (returns `{}` when unset),
  `PUT /me/ui-state` (replace; 422 on non-object or over cap). Stored as a
  `ui_state` TEXT column on the users table (or a `user_ui_state` row), added by
  a new migration.
- **Client owns structure:**
  ```jsonc
  {
    "global": { "locale": "en", "lastWorld": "<uuid|null>" },
    "worlds": { "<worldId>": { "activeTab": "settings" } }
  }
  ```
  Restored on load (drives entry routing and the initial locale/tab); written
  (debounced) when the active world, active tab, or locale changes. Survives
  reload and server restart; cross-device because it is server-side.
- **Substrate, not a one-off:** M8 adds per-user "active scene" under
  `worlds.<id>`; Phase 2 adds panel arrangement — both extend the blob with **no
  schema change** (the server stays structure-only).

## 8. New server surface

All additive; no change to the M5 write path or M4/M6 wire protocol.

1. **`GET /worlds`** — worlds the authenticated caller can access, each with the
   caller's role: `[{ id, name, role }]`. New `Repository` query (membership
   join, plus server-admins seeing all / their owned worlds consistent with
   `resolve_access_world`). WorldSelect needs this; today only `POST /worlds`
   (create) exists.
2. **`GET /api/config`** — public (no auth), returns `{ initialized: bool }` so
   the SPA routes Setup vs Login without leaking more than the existing
   `setup`-409 already reveals.
3. **`GET /me/ui-state` + `PUT /me/ui-state`** + migration — the §7 blob.
4. **`embed.rs` seam flip** — repoint `#[folder = "static/"]` → the client
   `dist/` output; retire `src/server/static/{login.html, auth.js, setup.html,
   index stub}` (their flows are folded into the themed SPA). The single-binary
   build and `serves_index_at_root_and_named_assets`-style tests update to the
   bundled app. Build ordering (client → server) is enforced in CI and the
   release build.

## 9. Theming — 3-tier SCSS token system

Brand-derived from `assets/icon` (a cool near-black "shadow cat" on a vivid
brand-blue circle with bright-blue eyes). **Single dark theme.** Near-black base,
never pure black.

- **Tier 1 — primitives** (raw, no semantics): a cool charcoal **surface ramp**
  and a **brand-blue accent ramp** sampled from the icon, plus spacing scale,
  type scale, radii, z-index. Starting values (sampled from the icon; refined to
  exact hexes during M7c implementation):

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

  Reds/greens/ambers are cool-tuned to harmonize with the blue base.

- **Tier 2 — semantic aliases:** `surface/{base,raised,overlay}`,
  `text/{primary,muted}`, `accent`, `accent-hover`, `accent-active`, `border`,
  `danger|success|warning`. Components reference *only* this tier.
- **Tier 3 — component vars:** per-component variables referencing tier 2
  (e.g. `--panel-bg: var(--surface-raised)`).
- Delivered as SCSS following the `sass-scss` skill structure; exposed as CSS
  custom properties so Phase 2's theme switching swaps tier-2 values without
  recompiling components. WCAG AA contrast verified for text-on-surface and
  accent states.
- **Re-audit gates (PLAN line 98):** the token set is explicitly provisional —
  re-audited at **M8** (first themed canvas overlays) and **M12** (default sheets
  / browsers). Recorded as such, not treated as final.

## 10. i18n scaffold

- **Library:** a runtime i18n library, all UI strings routed through its
  `t()`/store; one `en` locale; a locale switcher control present (scaffold).
- **Recommendation: `typesafe-i18n`** — generated types from the locale
  dictionary align with the project's type-safety posture (ts-rs, Zod), and it
  tree-shakes to a small runtime. **Open decision (§13.1):** `typesafe-i18n`'s
  maintenance cadence has slowed; **`svelte-i18n`** (ICU-based, more actively
  maintained) is the alternative. One is chosen at spec sign-off.
- New dependency → logged in `ARCHITECTURE.md` with license + one-line rationale,
  per the project's dependency-vetting rule.

## 11. Testing

The `ui` package has no tests today (`"test": echo … exit 0`). M7 establishes a
real test setup.

- **Vitest + `@testing-library/svelte`** (new dev-deps): the reactivity bridge
  (emit → re-render), the i18n seam (`t()` resolves; missing-key behavior), auth
  /bootstrap routing logic, the panel registry, token application, and
  responsive reflow assertions.
- **One Playwright smoke** (new dev-dep + CI job across the ubuntu/macos/windows
  matrix) reusing the core e2e spawned-server harness
  (`src/client/core/src/e2e/server-process.ts`): setup → login → world-select →
  enter table. Proves the entry flow end-to-end against a real server.
- CI: the existing matrix gains the UI unit job and the Playwright job; build
  ordering (client `dist/` before server embed) added.

## 12. Sub-milestone decomposition

Each sub-milestone is its own brainstorm-validated spec section → plan → execute
→ review cycle (mirroring M6a/b/c).

- **M7a — Server surface.** `GET /worlds`, `GET /api/config`,
  `GET/PUT /me/ui-state` + migration, `embed.rs` seam flip + static retirement +
  build-ordering wiring. Rust unit/integration tests. Independently shippable;
  unblocks M7b's data needs. Buddy-check candidate (touches auth-adjacent read
  surface + the embed seam).
- **M7b — Shell, entry flow, reactivity bridge.** Svelte SPA scaffold (Vite dev
  proxy to the Rust server), hash router, Setup/Login/WorldSelect/Table views,
  `WsClient`/`Welcome` lifecycle, the `createSubscriber` bridge, the named-region
  `Layout` + panel registry (placeholder panels). Vitest + the Playwright smoke.
- **M7c — Theming, i18n, session wiring.** The 3-tier SCSS token system + dark
  theme, i18n library + `en` locale + switcher, session-state load/save wiring
  (locale, lastWorld, activeTab), Settings panel, responsive reflow. Remaining
  Vitest coverage; `ARCHITECTURE.md` dependency log + token re-audit note.

## 13. Open decisions (resolve at sign-off)

1. **i18n library:** `typesafe-i18n` (recommended, type-safety alignment) vs
   `svelte-i18n` (more actively maintained, ICU). §10.
2. **`ui_state` storage shape:** `ui_state` TEXT column on `users` vs a separate
   `user_ui_state` table. Both satisfy §7; column is simplest, separate table
   isolates UI churn from the auth-critical table. Lean toward the column unless
   the users table is treated as auth-frozen.
3. **Setup in the SPA vs kept static:** folding first-run Setup into the themed
   SPA (recommended — single styling system, branded first run) requires the SPA
   to render before any admin exists. Alternative: keep a minimal static
   `setup.html` for the one-time pre-app step. §4 / §8.4 assume folding in.
