# M7d — Theming + i18n + Session-restore: Design Spec

> Status: **DRAFT for review.** The final M7 sub-milestone (parent:
> [`2026-06-19-m7-layout-theming-design.md`](2026-06-19-m7-layout-theming-design.md)).
> Adds the visual + localization + persistence layers over the running M7c SPA:
> the 3-tier SCSS token system + icon-derived dark theme, a **framework-neutral**
> i18n seam, and per-user session-restore via the M7a `ui_state` blob. Decomposed
> into **M7d-1** (theming), **M7d-2** (i18n), **M7d-3** (session-restore).
> Completing M7d completes **M7** → push to origin/main.
>
> **Builds on:** M7a (`GET/PUT /api/me/ui-state`), M7b (the framework-neutral
> `@shadowcat/core` primitives + `createSubscriber` Svelte bridge pattern), M7c
> (the SPA, `WorldSession`, `core-ui`, `AppContext`, the entry views).

## 1. Goal

1. **Theme** the SPA with a 3-tier SCSS token system, palette derived from the
   app icon (cool slate-violet charcoal + brand blue), replacing M7c's minimal
   structural styles. Single dark theme.
2. **Localize** via a framework-neutral i18n seam: all UI strings route through
   `t()` against one `en` catalog, with a locale switcher present.
3. **Persist** per-user session: restore locale, last world (auto-enter), and the
   active sidebar tab across reload/restart via the opaque `ui_state` blob.

## 2. Framework-neutrality constraint (load-bearing)

The UI is framework-neutral by design (a Vue/other-framework contributed component
must reach the same services). So the **i18n primitive lives in `@shadowcat/core`**
as a neutral object (no framework imports), exactly like `DocumentStore` and
`ContributionRegistry`; Svelte adapts it via `createSubscriber`, and any other
framework adapts the same object. (This rules out Svelte-coupled libraries like
`svelte-i18n`.)

## 3. World-model note (no change)

A user is in **one world at a time** per session (M7c's `WorldSession` = one WS
connection to one world's table) — that is the per-user reality M7d's `lastWorld`
auto-enter restores. The *server* can host multiple worlds concurrently (M4
per-world rooms) as free headroom for shared/community hosting; `world-select`
covers that. M7d adds **no** Foundry-style "activate THE world" gate — per-world
membership already controls access. Multiple worlds open in one client is a
non-goal.

## 4. Theming (M7d-1)

### 4.1 Palette (icon-derived, sampled from `assets/icon-256.png`)
- **Slate (surface) ramp** — cool violet-charcoal:
  `--slate-950 #16161F` (app bg), `900 #1F1F2C` (base), `850 #262635` (raised),
  `800 #2C2F40` (overlay), `700 #363645` (border), `600 #434558` (subtle),
  `400 #6B6D85` (muted text), `50 #E7E8F2` (text primary).
- **Blue (accent) ramp:** `600 #245AC0` (active), `500 #2D6EE8` (accent),
  `400 #538AEB` (hover), `300 #7FA8F1` (subtle).
- **Semantic supplements (cool-tuned):** `--red-500 #E5556B` (danger),
  `--green-500 #3FB089` (success), `--amber-500 #E0A23B` (warning).
- Scales: spacing (4/8px steps), type, radii, z-index.

### 4.2 Three tiers
- **Tier 1 primitives** — the raw ramps + scales above (`src/client/ui/src/styles/
  _primitives.scss`).
- **Tier 2 semantic** — aliases: `--surface-base/raised/overlay`,
  `--text-primary/muted`, `--accent/-hover/-active`, `--border`,
  `--danger/success/warning` (`_semantic.scss`). Components reference only this
  tier.
- **Tier 3 component** — per-component vars referencing tier 2.

### 4.3 Delivery
SCSS files compiled and exposed as **CSS custom properties on `:root`** via one
global stylesheet (`src/client/ui/src/styles/global.scss`) imported in `main.ts`.
Svelte components use `var(--token)` in scoped `<style lang="scss">`. The
`@sveltejs/vite-plugin-svelte` `vitePreprocess` already handles SCSS; add `sass`
as a dev dependency. M7c's structural styles in the entry views, `Layout`, and the
`core-ui` panels are replaced with token-based styling. WCAG AA verified for
text-on-surface and accent states.

### 4.4 Re-audit
Per the parent plan, the token set is **provisional** — re-audited at M8 (themed
canvas overlays) and M12 (default sheets/browsers). Recorded as such.

## 5. i18n (M7d-2)

### 5.1 Neutral `I18n` core (`@shadowcat/core`)
`src/client/core/src/i18n.ts` — a framework-neutral primitive mirroring
`DocumentStore`/`ContributionRegistry`:
```ts
export type Messages = Record<string, string>;
export class I18n {
  constructor(locale: string, catalogs: Record<string, Messages>);
  get locale(): string;
  setLocale(locale: string): void;          // notifies subscribers
  t(key: string, params?: Record<string, string | number>): string; // {name} interpolation
  subscribe(listener: () => void): () => void;
}
```
Backed by a flat catalog; missing keys return the key (visible, never throws).
Minimal `{name}` interpolation; ICU/pluralization deferred. No framework imports.
Barrel-exported.

### 5.2 Svelte adapter (`ui`)
`src/client/ui/src/lib/i18n.svelte.ts` — wraps `i18n.subscribe` with
`createSubscriber`, exposing a reactive `t(key, params)` (re-renders on
`setLocale`). A module-level singleton `I18n` instance (one `en` catalog) is the
app's source of truth; the entry views (pre-`AppContext`) import the adapter
directly, and in-world components read `t` via `AppContext` (extended with `t`).

### 5.3 The seam
- One `en` catalog (`src/client/ui/src/locales/en.ts`) holding every UI string.
- All literal strings in the entry views (`Setup`/`Login`/`WorldSelect`) and the
  `core-ui` panels (`Settings`/`TopBar`/`StatusBar`/`StagePlaceholder`) route
  through `t('key')`.
- A **locale switcher** in the `Settings` panel (only `en` exists; the control
  proves the seam and drives `i18n.setLocale`, which M7d-3 persists).

## 6. Session-restore (M7d-3)

### 6.1 Shape (client-owned; server stores opaque per M7a)
```jsonc
{ "global": { "locale": "en", "lastWorld": "<uuid|null>" },
  "worlds": { "<worldId>": { "activeTab": "settings" } } }
```

### 6.2 Load + restore
On every authenticated load — including a **browser reload** (the session cookie
persists, so `GET /api/me` still returns the user) — fetch `GET /api/me/ui-state`
and:
- apply `global.locale` to the `I18n` core;
- if `global.lastWorld` is set and the user can still access it (present in
  `GET /api/worlds`), **auto-enter** it — route straight back into that world's
  table, the same place the user left; else fall back to world-select (covers a
  since-deleted world or revoked access);
- restore `worlds[lastWorld].activeTab` as the sidebar's active tab.

So **reload returns you to the world you were in**, not to world-select.

### 6.3 Persist (debounced)
A `sessionState.svelte.ts` holds the blob and writes it back via debounced
`PUT /api/me/ui-state` (leading-edge debounce per
[[debounce-leading-edge-not-trailing-rearm]]) on: locale change, entering a world
(set `lastWorld` + activeTab default), leaving a world (`lastWorld = null`), and
active-tab change. A failed PUT is logged, never blocks the UI.

### 6.4 Leave / switch-world control
The `core-ui` shell gains a **leave-world control** (in the `:topbar` user menu):
calls `WorldSession.leave()`, clears `lastWorld`, routes to world-select — so
auto-enter is not a trap. The sidebar's `activeTab` becomes reactive state the
session reads/writes.

## 7. Decomposition

- **M7d-1 — Theming. ✅ DONE** (merged `--no-ff` to local main, not pushed).
  `sass` dep; 3-tier SCSS token files (icon-derived slate-violet + `#2D6EE8`) →
  CSS custom properties on `:root` + `global.scss` in `main.ts`; token-based
  styling across entry views, `Layout`, `core-ui` panels. Single-reviewed: fixed
  button `focus-visible`, lightened danger red (`#f37287`) + muted (`#9698AE`) to
  meet WCAG AA on the entry card, added a `--surface-sunken` token. All
  text/accent pairs ≥ AA; ui 18 tests + svelte-check green.
  Plan: [`plans/2026-06-19-m7d-1-theming.md`](plans/2026-06-19-m7d-1-theming.md).
- **M7d-2 — i18n.** Neutral `I18n` core (core, barrel-exported) + Svelte adapter;
  `en` catalog; route all strings through `t()`; locale switcher. Vitest (core +
  adapter).
- **M7d-3 — Session-restore.** `sessionState` load/restore/persist wiring;
  auto-enter `lastWorld`; reactive `activeTab`; the leave-world control. Vitest
  (mocked `api`).

## 8. Testing

- **Core (M7d-2):** `I18n` — `t` lookup, `{name}` interpolation, missing-key →
  key, `setLocale` + `subscribe` notification.
- **UI (Vitest + @testing-library/svelte):** the Svelte `t` adapter re-renders on
  `setLocale`; the session load→restore→persist logic against a mocked `api`
  (locale applied, lastWorld auto-enter, activeTab restored, debounced PUT fires);
  the leave-world flow (clears lastWorld, routes to world-select); a token-applied
  smoke render.
- **Regression:** svelte-check clean; the M7c Playwright entry-flow smoke stays
  green (the flow is unchanged; auto-enter only triggers with a persisted
  `lastWorld`, absent in the fresh-server e2e).

## 9. Decisions (resolved at brainstorm)

1. **Palette → icon-derived** (sampled slate-violet charcoal + `#2D6EE8` blue). §4.1.
2. **Theming delivery → SCSS tiers → CSS custom properties on `:root` → `var()`**
   in scoped Svelte styles (`svelte-ui-construction` pattern); `sass` dev-dep. §4.3.
3. **i18n → framework-neutral `I18n` core in `@shadowcat/core`** (subscribe/snapshot,
   Svelte adapts via `createSubscriber`); **not** `svelte-i18n` (Svelte-coupled).
   §2, §5.
4. **Session `lastWorld` → auto-enter + a leave/switch-world control.** §6.
5. **World model unchanged** — one world per user/session; server multi-world is
   free headroom; no activate-the-world gate. §3.
