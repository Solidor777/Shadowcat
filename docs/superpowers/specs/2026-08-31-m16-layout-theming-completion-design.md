# M16 — Layout + theming completion — Design

Status: approved design, pending implementation plan.
Parent context: [`PLAN.md`](../../PLAN.md) M16; the M7 design
([`2026-06-19-m7-layout-theming-design.md`](2026-06-19-m7-layout-theming-design.md)) deferred
"multi-theme, user themes, module styling modes, drag-resize, pop-out / multi-window" to Phase 2;
the M12 spec ([`2026-07-13-m12-dockable-panels-default-modules-design.md`](2026-07-13-m12-dockable-panels-default-modules-design.md))
pulled pop-out forward (M12e) and explicitly left drag-resize gaps, themes, user themes, and
styling modes here.

## Decisions (settled in brainstorm)

| Question | Decision |
|---|---|
| Theme representation | **Themes are data, not stylesheets.** A `ThemeDefinition` maps every tier-1 + tier-2 token name to a value plus a `colorScheme`; application writes inline `style.setProperty` on each themed document's `documentElement`. Inline `:root` styles beat the cloned stylesheet rules inside pop-out windows, which is the one mechanism that works uniformly for the main document, already-open popouts, and future popouts. A `data-theme` attribute selector approach is rejected: dockview's `addStyles` clones stylesheets, not attributes, so popouts would escape theming. |
| Theme state owner | A framework-neutral `ThemeController` singleton in `@shadowcat/ui-kit`, mirroring the `i18n` singleton's shape (`$state`-backed, `createSubscriber` reactivity, subscribe-on-change persistence wired by the shell's `sessionState`). Not a document, not server state — a per-account UI preference. |
| Theme persistence | `UiState.global.theme` (per-account, cross-world), following the locale template end to end: `GlobalField` union + `copyGlobalField`'s `satisfies never` switch force compile-time handling; the server's leaf-merge needs no change. A thin `localStorage` mirror (one key, the serialized `global.theme` value) applies the last-used theme pre-login so the login/world-select screens don't flash the default; `loadSessionState` overwrites the mirror on every load. First `localStorage` use in the codebase — deliberately scoped to this one cosmetic, non-secret preference. |
| Built-in themes | Three: `slate-dark` (the current palette, default), `slate-light`, `contrast-dark` (WCAG-AAA-oriented high-contrast dark). Each ships with every tier-2 token AA-checked against its own surfaces. |
| Tier 3 | Not a file. The M7d build redefined tier 3 as per-component scoped `<style>` consumption of tier 2; that convention stands. M16 instead completes tier 2 (font-size scale incl. the long-deferred caption token, `--scrim`) and fixes every token-name mismatch so the token set is exact. |
| User themes | Named per-account themes stored beside the picker state in `UiState.global.theme.custom`: `{ label, base: builtinId, tokens: partial overrides }`. A theme editor in the settings panel edits tier-2 token overrides on a built-in base. Unknown token keys are dropped at decode; garbage falls back to the default theme. |
| Theme picker home | The settings module's `Settings.svelte`, beside the existing locale switcher — account-level preference, not world config. |
| Canvas recolor | `Stage.svelte` re-reads `--surface-base`/`--grid-line` on theme change and pushes them into a new runtime color-update seam on `RenderEngine`/the display backend (today both are construction-time-only). No re-mount, no engine recreation. |
| Floating drag-resize scope | DockviewEngine already provides 8-direction pointer resize with tree sync. The gaps M16 closes: (1) resize-handle hit targets (4px → ≥24px, 44px coarse) in `panels.scss`; (2) keyboard move/resize of a focused floating window, emitted as `resizeFloating` ops through `applyOp` — which also gives FakeEngine command-driven resize for free; (3) the latent one-way sync: `DockviewEngine.apply()` must reposition an already-floating widget when the tree rect changes from a non-engine source (keyboard op, arrangement restore, layout reset). Compact mode stays without floating panels by M12 design. FakeEngine gains no pointer-gesture emulation — it is a test double / bespoke fallback; ops-driven (keyboard/menu) resize reaching it through the reducer is the durable shape. |
| Multi-window arrangement persistence | The tree's `ExpandedLayout.poppedOut: string[]` becomes `popouts: PopoutWindowLayout[]` — `{ key, panels: string[], rect: ScreenRect \| null }` — preserving window grouping and screen geometry. Codec migrates old blobs. Geometry is captured from dockview's popout size/position events via a new reducer op; the vendored `onDidPopoutGroupPositionChange.screenY` bug is routed around by reading the popout entry's own dimensions, never the event payload. On reload, popouts still rehydrate to floating (browser gesture rule, unchanged) — but the restore notice gains a **"Reopen windows" action**: one click (a user gesture) re-opens every persisted popout window at its saved rect with its saved panel set. A gesture-time pop-out from the panel menu also reuses that panel's last saved popout rect when one exists. |
| Module styling modes | A `styling?: "host" \| "isolated"` field on the contribution record (default `"host"`), honored at the two content chokepoints (`<Surface>` and `PanelHost`). Isolated content is wrapped in a container carrying a generated reset class that re-declares every token at its engine-default value — generated **from the theme data module** so the reset can never drift from the token set. |
| External-module CSS | The loader gains a manifest-declared stylesheet: optional `style` key in `module.json`, fetched from the module's static route and injected as a `<link>` at activation, removed at unload. The example module and the creating-a-module guide are updated so the documented install flow actually ships component CSS (today the lib-mode `style.css` is emitted but never installed — verified at plan time). |
| dockview floating chrome | The `panels.scss` "future work" floating/drop-overlay skin lands here as part of theming completion — all chrome reads tier-2 tokens. |

Excluded (unchanged from PLAN.md): pop-out windows themselves (shipped, M12e). Additionally excluded
here: compact-mode floating panels (M12 design: the compact view has no windowing concept);
re-opening pop-out windows without a user gesture (impossible — browser popup policy);
per-world themes (a theme is an account preference; world-owned visual identity is a separate,
unrequested feature); `prefers-color-scheme` auto-follow (an explicit picker only — auto-follow
fights the persisted choice; the built-in set includes both schemes).

## Sub-milestones

- **M16a — theme engine + built-in themes.** Theme data model, `ThemeController`, persistence
  (incl. the localStorage mirror), settings picker, pop-out propagation, stage recolor seam,
  tier-2 completion + mismatch fixes, the three built-in themes with AA audits, floating/drop
  chrome skin, `color-scheme` and webmanifest alignment.
- **M16b — layout completion.** Resize-hit-target CSS, keyboard move/resize, the
  `apply()` already-floating reconcile branch, pop-out arrangement persistence (tree + codec +
  geometry capture + restore action + gesture-time rect reuse), e2e coverage for both.
- **M16c — user themes + module styling modes.** Theme editor, custom-theme storage/validation,
  contribution `styling` field + isolation wrapper + generated reset, external-module CSS
  pipeline (manifest key, loader injection, example + guide), author theming docs.

Build order: M16a → M16b → M16c (c consumes a's theme data for the reset generator; b is
independent of a but shares the panels package, so landing a first keeps the diff reviewable).

## §A — Theme engine

- **`ThemeDefinition`** (ui-kit, framework-neutral data):

  ```ts
  interface ThemeDefinition {
    readonly id: string;             // built-in: "slate-dark" | "slate-light" | "contrast-dark"
    readonly labelKey: string;       // i18n key resolved by the host
    readonly colorScheme: "dark" | "light";
    readonly tokens: Readonly<Record<ThemeTokenName, string>>;
  }
  ```

  `ThemeTokenName` is the enumerated union of every tier-1 and tier-2 custom property the engine
  declares (no `var()` chains inside values — each token resolves to a literal, so pop-out
  application writes fully-resolved values and never depends on cross-rule cascade order).
  The token list is declared once, as data, in the theme module; `_primitives.scss` /
  `_semantic.scss` keep declaring the *default* values for no-JS/no-flash first paint, and a
  build-time test pins the SCSS-declared set ≡ the data-declared set (read both, diff — the
  fork-a-decision guard for the two declaration sites).

- **`ThemeController`** (ui-kit singleton, exported like `i18n`):
  - `$state` active theme id + resolved `ThemeDefinition` (built-in or custom-over-base).
  - `apply()` writes every token + `colorScheme` as inline styles on `document.documentElement`,
    then on every registered pop-out `Document`. Removing inline overrides (switch back to a
    theme whose value equals the stylesheet default is still written inline — no removeProperty
    asymmetry) keeps the mechanism single-pathed.
  - `registerDocument(doc: Document): () => void` — the pop-out seam: applies the current theme
    to `doc` immediately and on every later change until the returned unregister runs.
    `DockviewEngine` calls it from the pop-out success path (`onDidOpen`) and unregisters on
    pop-out removal; the panel module reaches the controller through `@shadowcat/ui-kit` like
    any other shared runtime.
  - Reactivity for Svelte consumers via `createSubscriber`, same pattern as `i18n`.

- **Shell wiring** (`sessionState.svelte.ts`, mirroring locale): `loadSessionState` reads
  `global.theme`, validates (unknown active id → default; custom themes validated token-by-token),
  constructs/updates the controller, and installs a once-per-lifetime change subscriber that
  marks `global.theme` dirty and updates the localStorage mirror. Boot applies the mirror
  synchronously from `main.ts` before first paint (inline module script or earliest import),
  so pre-login screens honor the last-used theme.

- **`UiState` shape** (client-owned; server stays opaque):

  ```ts
  global: {
    locale: string;
    lastWorld: string | null;
    theme?: {
      active: string; // built-in id or `custom:<id>`
      custom: Record<string, { label: string; base: string; tokens: Record<string, string> }>;
    };
  }
  ```

  `GlobalField` gains `"theme"`; `copyGlobalField`'s switch gains the arm (the `satisfies never`
  default keeps this compile-checked). The 64 KB blob cap is unaffected (a full custom theme is
  ~1 KB).

## §B — Built-in themes + token completion

- **Token additions (tier 2 unless noted):** a font-size scale at tier 1
  (`--font-size-sm`, `--font-size-md`, `--font-size-caption`) delivering the caption token
  deferred since the M8 audit; `--scrim` (modal/overlay backdrop, replacing the three
  hardcoded `rgba(0,0,0,…)` scrims). `--z-popover` stays a token but `MergeConflictModal`'s
  hardcoded `z-index: 1000` is replaced by it.
- **Mismatch fixes (live bugs today):** `--text-on-accent` → `--on-accent` (asset-browser ×2),
  `--border-color` → `--border` (chat-card), `--surface` → `--surface-raised` (chat-card
  `RollTooltip`), `--text-on-surface`/`--border-subtle` → real tokens (stage), `--accent-contrast`
  → `--on-accent` (ui-kit `MergeConflictModal`), plus `--accent, #46f` and the off-palette
  3-digit fallbacks swept to token-only references (a fallback that can never be needed is a
  lie about the token set — remove them; the theme module guarantees declaration).
- **The three themes** ship complete `tokens` maps. Light theme: full surface ramp inversion
  with every pairing AA-checked (the existing comments' WCAG ratios are re-derived per theme —
  the check runs as a unit test over the theme data computing WCAG contrast ratios for the
  documented pairings: text/surface, on-accent/accent, danger/surface, etc., so the audit is
  executable, not aspirational). `contrast-dark`: near-black surfaces, max-ratio text, thickened
  borders.
- **`color-scheme`** follows the active theme (`documentElement.style.colorScheme`), so native
  scrollbars/form controls match; `global.scss`'s static `color-scheme: dark` remains the
  pre-JS default.
- **Chrome completion:** `panels.scss`'s recorded future work (drop overlays, tab-group chips,
  floating titlebar) lands — all dockview chrome consumes tier-2 tokens through the existing
  `--dv-*` bridge, which then follows every theme for free.
- **`site.webmanifest` + any `theme-color` meta**: aligned to the default theme's sunken surface
  (static files can't follow runtime swaps; the default is the honest value).

## §C — Stage / canvas recolor

- `RenderEngine` gains a runtime seam (e.g. `setThemeColors({ background, gridColor })`) routed
  to the display backend; `gridColor` stops being construction-`readonly`. The backend interface
  + pixi implementation + mock all grow the method together (the mock's recorded field already
  exists — the pixi backend's gap is verified at plan time).
- `Stage.svelte` subscribes to the `ThemeController` (via `createSubscriber`), re-reads the two
  tokens through the existing `readColor` probe, and calls the seam. Construction-time reads stay
  as the initial values.
- Domain colors (faction colors, tool colors, environment defaults, fog/ping hex constants in the
  render layer) are content data, not theme tokens — explicitly untouched.

## §D — Settings picker

- `Settings.svelte` gains a theme section beside the locale switcher: a `<select>` of built-ins +
  saved custom themes (labels via `t()` for built-ins, user label for custom), applying on change
  through the controller (persistence rides the change subscriber — no panel-side write path).
- M16c adds the editor affordance ("New custom theme" → editor dialog/panel section: base picker,
  per-token color inputs for the tier-2 set with live preview, label, save/delete).
- i18n: `settings.theme.*` keys in `ui-kit/src/locales/en.ts` (the only shipped catalog — no
  cross-locale fallback exists, so every key lands there).

## §E — Layout: drag-resize completion

- **Hit targets:** `.dv-resize-handle-*` grow to a ≥24px transparent hit zone (44px under
  `@media (pointer: coarse)`) without changing the visual 4px edge — pseudo-element or
  negative-inset technique in `panels.scss`.
- **Keyboard move/resize:** the floating dialog wrapper (`#wireFloatingA11y`'s `role="dialog"`
  element) gains a keydown map: arrows move (8px; Shift 32px), Ctrl+arrows resize from the
  bottom/right edges (same steps). Every keystroke batch (keydown repeat is fine — one op per
  event) emits `LayoutOp.resizeFloating` through `PanelsController.dispatch`, never touching the
  widget directly. `describeOp` keeps not narrating resize (deliberate); the dialog's
  `aria-label` gains the shortcut hint and the PanelMenu documents it.
- **Tree→widget reconcile:** `DockviewEngine.apply()`'s floating loop gains the
  already-floating branch: tree rect ≠ live `boundingBox` → `group.api.setSize` + position update
  (the position half has no clean public API — plan-time verification against the vendored
  7.0.2 source; the internal overlay `setBounds` is the expected route, isolated in one private
  method with the version-bump re-verify note the other vendored-source couplings carry). The
  `#lastFloatingRect` snapshot discipline is preserved so the reconcile never echoes an op back
  as a new op.

## §F — Multi-window arrangement persistence

- **Tree:** `ExpandedLayout.poppedOut: string[]` → `popouts: PopoutWindowLayout[]`:

  ```ts
  interface PopoutWindowLayout {
    /** Engine-agnostic window identity, minted (uuid) by the engine at pop-out time. */
    key: string;
    /** Panels in this window, tab order. */
    panels: string[];
    /** Last known screen rect (`window.open` feature semantics), or null if never observed. */
    rect: { left: number; top: number; width: number; height: number } | null;
  }
  ```

  Codec: a legacy `poppedOut: string[]` blob migrates to one single-panel window per id
  (`rect: null`, fresh keys); a malformed entry fails the whole blob like today
  (`withPoppedOut`'s pattern extended). `prune`, `locate`, `placeFromPersistedLocation`, and the
  popOut/popIn ops are updated; `popIn` removes the panel from its window (empty window ⇒ the
  window entry is dropped).
- **Ops:** `LayoutOp.popOut` gains `{ key, rect }`; new `LayoutOp.updatePopoutGeometry { key, rect }`
  and `LayoutOp.popOutInto { id, key }` (drag-into-existing-popout, fed by the existing
  `#popoutGroupSubs` wiring). Same-reference no-op contract holds for all.
- **Capture:** `DockviewEngine` subscribes `onDidPopoutGroupSizeChange` /
  `onDidPopoutGroupPositionChange`, but reads geometry from the popout entry's own
  `dimensions()`/window — **never the event payload** (vendored 7.0.2 populates `screenY` from
  `screenX`; an `OPEN_BUGS.md` entry records the upstream defect and the avoidance). Both events
  already feed `onDidLayoutChange`; the dedicated events are the capture site (debounced by
  dockview already).
- **Restore:** `#rehydratePoppedOut` keeps converting to floating at load (gesture rule), but now
  restores *grouping* information into the tree's floating cascade order (panels of one saved
  window cascade adjacently) and retains the `popouts` record (marked dormant) so the restore
  action has the arrangement. The existing `panels.popoutRestoredFloating` notice gains a
  **"Reopen windows" action** (one click = the required user gesture): for each saved window,
  `#requestPopOut`-equivalent with `addPopoutGroup(panel, { position: savedRect })`, first panel
  then the rest via `popOutInto`. Notification action support is verified at plan time (the
  toast/notice host may need an action slot).
- **Gesture-time reuse:** menu pop-out on a panel with a saved rect passes
  `position: savedRect` to `addPopoutGroup` (clamped to the current screen's available bounds;
  garbage was already excluded by codec validation).
- **Cascade parity:** `SHEET_CASCADE_*` ≡ `REHYDRATE_FLOAT_*` and the parity test are untouched —
  rehydration still cascades; only *adjacency* is informed by saved grouping.

## §G — Module styling modes

- **Contract:** `Contribution.styling?: "host" | "isolated"` (default `"host"`). Panel
  contributions read it via `Contribution.panel` inheritance (the field lives on the contribution
  root, not inside `panel` metadata, so non-panel surfaces use the same word).
- **Isolation mechanism:** the theme data module exports `themeIsolationCss(): string` — a
  generated `.sc-theme-isolate { … }` rule re-declaring every token at its engine-default value
  (single source: the default `ThemeDefinition`'s token map — impossible to drift from the token
  set, and a test pins the emitted property set ≡ `ThemeTokenName`). ui-kit injects the sheet
  once. `<Surface>` and `PanelHost` wrap `styling: "isolated"` content in a container with that
  class; the wrapper travels with the panel DOM into pop-outs (slot adoption), so isolation holds
  across pop-out. Isolation scopes *tokens*, not layout — the module's own styles then apply
  unaffected by user themes.
- **External-module CSS:**
  - `module.json` gains optional `style?: string` (a single CSS file relative to the module
    folder; server manifest validation extended, path-traversal guard covers it like the entry).
  - The client loader (`loadModules` / `WorldSession.#loadExternalModules`) injects
    `<link rel="stylesheet" href="<module static route>/<style>">` after a successful activation
    and removes it on unload (`ModuleRegistry.unload` cascade).
  - The example tracker's `vite.config.ts` pins `build.lib.cssFileName: "style.css"`, declares
    `"style": "style.css"` in its `module.json`, and the guide's install flow copies it.
  - Plan-time verification: build the example and confirm the emitted CSS filename/behavior;
    confirm the server's module static route serves `.css` with the right MIME.
- **Author docs:** the creating-a-module guide gains a theming section — consume tier-2 `var(--*)`
  tokens (list + contract), never hardcode colors, `styling: "isolated"` semantics and its cost
  (the module re-implements surface/text/accent itself), pop-out and theme-swap behavior.

## Test strategy

- **Unit (Vitest/jsdom):** theme controller (apply/register/persist shapes, garbage decode),
  token-set parity test (SCSS ↔ data), WCAG ratio audit over each built-in's documented pairings,
  isolation-CSS emission parity, codec migration (legacy `poppedOut` blob → `popouts`), reducer
  ops (popOut/updatePopoutGeometry/popOutInto, no-op identity), `apply()` already-floating
  reconcile (jsdom stubbed `boundingBox` like the existing sync tests), keyboard op emission,
  sessionState `copyGlobalField` arm, loader CSS injection/removal.
- **e2e (Playwright, real binary):** theme switch → visible token change + reload persistence;
  pop-out → move/resize window → reload → floating rehydrate → "Reopen windows" restores rect +
  grouping; keyboard move/resize of a floating panel; isolated vs host contribution under a
  non-default theme. Playwright drives real pointer drags and real popups — closing part of the
  recorded real-pointer manual-QA gap.
- **Gates:** all repo gates stay green (`pnpm -r test`, `typecheck`, `lint`, `lint:docs`,
  `lint:props`, `lint:comments`, `docs:check-examples`, `lint:file-size`, `lint:inline-tests`,
  `cargo test`, `build:all`). New public symbols carry doc comments; comments cite symbols only
  (RULE 15/16).

## Loose ends folded in (from exploration; none deferred)

1. M8→M12-deferred font-size/caption token — delivered (§B).
2. Six token-name mismatch bugs + off-palette fallbacks — fixed (§B).
3. `Stage.svelte` once-at-mount token reads — reactive seam (§C).
4. Pop-out theme propagation (static style cloning) — register-document seam (§A).
5. External-module CSS dropped by the documented install flow — pipeline + guide (§G).
6. dockview-core 7.0.2 `screenY`-from-`screenX` upstream bug — avoided, logged (§F).
7. `apply()` one-way floating sync — reconcile branch (§E).
8. 4px resize handles below the a11y floor — hit-target CSS (§E).
9. `panels.scss` floating-chrome future work — landed (§B).
10. `site.webmanifest` off-palette colors — aligned (§B).
