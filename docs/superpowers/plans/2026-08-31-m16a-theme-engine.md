# M16a — Theme Engine + Built-in Themes — Implementation Plan

> **For agentic workers:** implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax
> for tracking. Commit per green task.

**Goal:** Themes become runtime data. Ship the `ThemeController` (ui-kit), three AA-audited
built-in themes over a completed tier-2 token set, account-level persistence with a pre-login
localStorage mirror, pop-out window theme propagation, a reactive stage-canvas recolor seam, and
the settings-panel theme picker.

**Architecture:** A `ThemeDefinition` maps every enumerated tier-1/tier-2 token to a literal
value. Application = inline `style.setProperty` on each themed document's `documentElement`
(main + every registered pop-out), which beats both the shell stylesheet and dockview's cloned
pop-out stylesheets. The shell's `sessionState` persists `UiState.global.theme` exactly the way
it persists `global.locale`; `main.ts` applies the localStorage mirror before first paint.

**Tech Stack:** Svelte 5 runes, SCSS, vitest, Playwright, Zod-free (ui_state is opaque).

**Spec:** `docs/superpowers/specs/2026-08-31-m16-layout-theming-completion-design.md` (§A–§D;
the decisions table is binding).

## Model/Effort directives

This session runs **Kimi**. Execute with coder subagents per task or small task cluster; every
subagent prompt MUST carry the campaign owner's verbatim directive paragraph and the
report-delivery requirement (return report as the agent-tool result, via send-message, or written
to a document). Review checkpoints: a shadowcat-codebase-informed review pass at the end of each
phase before commit.

## Global Constraints

- Comments cite SYMBOLS, never files/lines; no ephemeral refs (milestones, specs, history) in
  code or code-facing strings (`docs/design/doc-sweep-truthfulness-rules.md` RULES 15/16; gates
  `pnpm lint:comments`, `check-skill-symbol-refs`).
- No lint suppressions, no dead code; no file over 5,000 lines (`pnpm lint:file-size`).
- Doc coverage gates are errors repo-wide: every new exported item documented
  (`pnpm lint:docs`, `pnpm lint:props`) — both the `.ts` and `.svelte` blocks of each config.
- All UI strings through `t()`; new keys land in `ui-kit/src/locales/en.ts` (the only shipped
  catalog — no cross-locale fallback).
- Cross-platform: responsive/touch UI (≥44px coarse-pointer targets).
- Never fork a decision: the token set is declared ONCE as data; the SCSS default-theme files
  and the isolation/audit machinery derive from or are test-pinned against that one list.
- Safe deletion only (`trash`, never `rm`).
- `pnpm -r test` + `pnpm -r typecheck` + `pnpm lint` green before every commit; commit per task.
- Work happens in the `C:/Dev/Shadowcat-m16` worktree on branch `m16-layout-theming`.

---

## Phase 1 — Theme data + controller (ui-kit)

### Task 1: `theme` data module — token enumeration + built-in definitions

**Files:**
- Create: `src/client/ui-kit/src/theme.ts`
- Test: `src/client/ui-kit/src/theme.test.ts`

**Interfaces:**
- `ThemeTokenName` — union of EVERY tier-1 + tier-2 custom property name **without** the `--`
  prefix (e.g. `"slate-950"`, `"surface-base"`, `"accent"`, `"on-accent"`, `"space-1"`,
  `"radius-1"`, `"font-sans"`, `"input-height-coarse"`, `"grid-line"`, `"shadow-elevated"`,
  `"z-popover"`, plus the Task-2 additions `"font-size-sm"`, `"font-size-md"`,
  `"font-size-caption"`, `"scrim"`). Source of truth: read `_primitives.scss` and
  `_semantic.scss` and enumerate — the Task-3 parity test then pins the two sets equal forever.
- `ThemeDefinition { id: string; labelKey: string; colorScheme: "dark" | "light"; tokens:
  Readonly<Record<ThemeTokenName, string>> }` — values are LITERALS (no `var()` chains).
- `BUILTIN_THEMES: readonly ThemeDefinition[]` — `slate-dark` (default; literal transcription of
  the current `_primitives.scss`/`_semantic.scss` values), `slate-light`, `contrast-dark`.
- `DEFAULT_THEME_ID = "slate-dark"`.
- `resolveTheme(active: string, custom: Record<string, CustomTheme>): ThemeDefinition` —
  built-in id → that theme; `custom:<id>` → base built-in + validated overrides (unknown token
  keys dropped); anything unresolvable → the default theme.
- `CustomTheme { label: string; base: string; tokens: Partial<Record<ThemeTokenName, string>> }`.

- [ ] Step 1: Write failing tests — every built-in defines every `ThemeTokenName`; `resolveTheme`
  resolution/fallback/override-drop behavior; `slate-dark` literals equal the SCSS defaults
  (transcribe carefully — this equality is what makes the default theme a visual no-op).
- [ ] Step 2: Implement `theme.ts` with full doc comments (every export — `lint:docs`/
  `lint:props` are errors).
- [ ] Step 3: Light + contrast-dark palettes: derive and check contrast per the pairings the
  existing SCSS comments document (text/surfaces ≥4.5, `--on-accent` on `--accent` ≥4.5,
  `--danger`/`--on-danger`, `--text-muted` on raised/overlay ≥4.5). Task 4 makes this executable.
- [ ] Step 4: Export from `ui-kit/src/index.ts`. Gates: `pnpm --filter @shadowcat/ui-kit test`,
  `typecheck`, `pnpm lint`. Commit.

### Task 2: Token completion + mismatch sweep

**Files:**
- Modify: `src/client/shell/src/styles/_primitives.scss` (font-size scale: `--font-size-sm`,
  `--font-size-md`, `--font-size-caption` — sizes per the design's type ramp: caption 0.75rem,
  sm 0.875rem, md 1rem matching the body default)
- Modify: `src/client/shell/src/styles/_semantic.scss` (`--scrim` — modal backdrop,
  `rgba(0,0,0,0.5)` in the default theme)
- Modify (mismatch fixes — token-only references, off-palette fallbacks deleted):
  `src/modules/asset-browser/src/FilterBar.svelte`, `src/modules/asset-browser/src/AssetGrid.svelte`,
  `src/modules/asset-browser/src/UploadQueue.svelte`, `src/modules/asset-browser/src/BulkBar.svelte`,
  `src/modules/asset-browser/src/FolderTree.svelte`, `src/modules/asset-browser/src/PreviewPane.svelte`,
  `src/modules/chat-card/src/MessageCard.svelte`, `src/modules/chat-card/src/RollTooltip.svelte`,
  `src/modules/stage/src/Stage.svelte`, `src/client/ui-kit/src/MergeConflictModal.svelte`
  (incl. its hardcoded `z-index: 1000` → `var(--z-popover)` and scrim → `var(--scrim)`),
  `src/modules/asset-browser/src/AssetPickOverlay.svelte` (scrim → `var(--scrim)`)
- Modify: `src/client/shell/public/site.webmanifest` (`theme_color`/`background_color` → the
  default theme's `--surface-sunken` `#16161f`)

- [ ] Step 1: Grep-sweep for remaining `#[0-9a-fA-F]{3,8}` / `rgba?(` in `src/modules` and
  `src/client` component styles (exclude domain/content colors: faction seeds, scene-tools tool
  colors, render-layer fog/ping constants, generated types, tests) — every UI-chrome color reads
  a tier-2 token.
- [ ] Step 2: Apply the fixes; add the new tokens to the SCSS files with the same comment style
  (WCAG notes where relevant). Update `theme.ts`'s `slate-dark` map in the SAME commit.
- [ ] Step 3: Gates: `pnpm -r test`, `pnpm -r typecheck`, `pnpm lint`, `pnpm lint:docs`,
  `pnpm lint:props`. Commit.

### Task 3: Token-set parity test (SCSS ≡ data)

**Files:**
- Test: `src/client/shell/src/styles/tokenParity.test.ts` (shell owns the SCSS; it imports the
  ui-kit token list — shell already depends on ui-kit)

- [ ] Step 1: Test reads `_primitives.scss` + `_semantic.scss` via `node:fs`, extracts every
  `--name:` declaration, strips the prefix, and asserts the set EQUALS the `ThemeTokenName`
  universe (export a `THEME_TOKEN_NAMES: readonly ThemeTokenName[]` from `theme.ts` for this).
  Mutation-check: the test fails when either side gains/lacks a name.
- [ ] Step 2: Gates + commit.

### Task 4: Executable WCAG audit

**Files:**
- Test: `src/client/ui-kit/src/themeContrast.test.ts`

- [ ] Step 1: Implement WCAG 2.x relative-luminance/contrast-ratio helpers INSIDE the test file
  (no new runtime dependency).
- [ ] Step 2: For EACH built-in theme assert: `text-primary` ≥4.5 on every surface token;
  `text-muted` ≥4.5 on `surface-raised`/`surface-overlay`; `on-accent` on `accent` ≥4.5;
  `on-danger` on `danger` ≥4.5; `danger` ≥4.5 on every surface (it serves as inline alert text);
  `accent` ≥3:1 against `surface-base` (non-text UI component contrast, WCAG 1.4.11).
- [ ] Step 3: Fix palette values until green (iterate the DATA, not the test). Gates + commit.

### Task 5: `ThemeController` singleton

**Files:**
- Create: `src/client/ui-kit/src/theme.svelte.ts`
- Test: `src/client/ui-kit/src/theme.svelte.test.ts`
- Modify: `src/client/ui-kit/src/index.ts` (export `theme` singleton + types)

**Interfaces:**
- `class ThemeController` mirroring `I18n`'s shape: `$state` `#active`/`#custom`; public
  `readonly active: string`, `resolved: ThemeDefinition` (derived), `customThemes` (readonly
  snapshot); `setActive(id: string)`, `saveCustom(id, theme)`, `deleteCustom(id)` (M16c consumes
  the latter two — implement + test them NOW, no deferred API);
  `load(state: { active, custom } | undefined)` (garbage-tolerant);
  `serialize(): { active, custom }` (the persisted shape);
  `subscribe(listener): () => void` + a `createSubscriber`-backed reactive read (the `i18n`
  adapter pattern).
- Application: `applyTo(doc: Document)` writes every token + `colorScheme` inline on
  `doc.documentElement.style`; `#applyAll()` on any change → main `document` (guarded for jsdom/
  SSR: `typeof document !== "undefined"`) + every registered extra document.
  `registerDocument(doc): () => void` — immediate apply + tracked for future changes.
- The singleton instance: `export const theme = new ThemeController();`

- [ ] Step 1: Failing tests — setActive/saveCustom/deleteCustom/load-garbage/serialize round
  trip; applyTo writes all tokens + colorScheme; registerDocument applies immediately and on
  later change, unregister stops updates; subscriber fires.
- [ ] Step 2: Implement. Doc comments on every export.
- [ ] Step 3: Gates + commit.

## Phase 2 — Persistence + boot wiring (shell)

### Task 6: `UiState.global.theme` + sessionState wiring

**Files:**
- Modify: `src/client/shell/src/lib/api.ts` (`UiState` type + `defaultUiState`)
- Modify: `src/client/shell/src/lib/sessionState.svelte.ts` (`GlobalField` + `"theme"` arm in
  `copyGlobalField`; load/apply in `loadSessionState`; once-per-lifetime change subscriber that
  marks dirty AND writes the localStorage mirror)
- Test: existing sessionState tests + new arms

- [ ] Step 1: Failing tests — `copyGlobalField` theme arm copies the value; loading a ui_state
  with a theme applies it to the ui-kit `theme` singleton; a `theme.setActive` call schedules a
  dirty `global.theme` patch (assert via the existing persist-machinery test patterns) and writes
  the localStorage mirror; garbage theme state → default, no throw.
- [ ] Step 2: Implement. The `satisfies never` switch forces the arm — follow the locale arm's
  shape exactly, including dirty-tracking and failure re-marking behavior riding the existing
  machinery (no parallel persist path).
- [ ] Step 3: Gates + commit.

### Task 7: Pre-login boot application (localStorage mirror)

**Files:**
- Modify: `src/client/shell/src/main.ts` (earliest-possible synchronous read of the mirror key;
  garbage → ignore; apply via `theme.load` + `applyTo(document)`)
- Test: extend shell boot tests if a seam exists; otherwise cover the parse/apply helper as a
  pure function in `sessionState` tests (the mirror read/parse is a pure helper:
  `readThemeMirror(storage): PersistedTheme | undefined`).

- [ ] Step 1: Extract `readThemeMirror`/`writeThemeMirror` pure helpers (injectable storage) —
  failing tests first (absent key, malformed JSON, valid).
- [ ] Step 2: Wire `main.ts` to apply before the app mounts; wire the subscriber from Task 6 to
  `writeThemeMirror`. One storage key: `"shadowcat.theme"`.
- [ ] Step 3: Gates + commit.

## Phase 3 — Propagation + canvas (panels/render/stage)

### Task 8: Pop-out document registration

**Files:**
- Modify: `src/modules/panels/src/engine/dockview.ts` (pop-out success path: register the
  pop-out's `Document` with ui-kit `theme.registerDocument`; unregister in
  `#handleRemovePopoutGroup` and `destroy`)
- Test: `src/modules/panels/src/engine/dockview.test.ts` (inject/inspect via the existing
  popout-driver test seams; assert register/unregister calls with the popout document)

- [ ] Step 1: Verify the popout `Document` is reachable at `onDidOpen`/driver-resolution time
  (vendored `PopoutWindow` exposes its window — read the vendored source; the injected
  `popoutDriver` in tests stands in). Register EXACTLY once per pop-out open; pair every register
  with an unregister on both removal paths.
- [ ] Step 2: Failing test → implement → gates → commit.

### Task 9: Stage/canvas runtime recolor

**Files:**
- Modify: `src/client/render/src/backend.ts` (`DisplayBackend` gains
  `setClearColor(color: number): void` — the renderer background clear color)
- Modify: `src/client/render/src/pixi-backend.ts` (implement via the Pixi renderer's background;
  verify the v8 API surface — `renderer.background.color`)
- Modify: `src/client/render/src/backend.mock.ts` (record the last clear color)
- Modify: `src/client/render/src/engine.ts` (`gridColor` drops `readonly`; public
  `setThemeColors({ background, gridColor }: { background?: number; gridColor?: number }): void`
  — routes clear color to the backend, stores gridColor, and re-drawing the grid follows the
  existing redraw path; no engine recreation)
- Modify: `src/modules/stage/src/Stage.svelte` (subscribe to the ui-kit `theme` singleton via
  its reactive read; on change re-read `--surface-base`/`--grid-line` through `readColor` and
  call `engine.setThemeColors`)
- Tests: render engine + stage tests

- [ ] Step 1: Failing tests — engine.setThemeColors updates backend clear color + later
  `drawGrid` calls use the new color; mock records; Stage re-reads on theme change (jsdom: stub
  `getComputedStyle` per the existing `readColor` tests).
- [ ] Step 2: Implement; doc comments. `backend.mock.ts`'s recorded `gridColor` pattern is the
  template.
- [ ] Step 3: Gates + commit.

## Phase 4 — Picker UI + chrome + e2e

### Task 10: Settings theme picker

**Files:**
- Modify: `src/modules/settings/src/Settings.svelte` (theme section beside the locale switcher:
  `<select>` of `BUILTIN_THEMES` + saved customs, bound to `theme.active` via `setActive`)
- Modify: `src/client/ui-kit/src/locales/en.ts` (`settings.theme.*` keys: section label,
  option labels for the three built-ins — `ThemeDefinition.labelKey` resolves here)
- Test: settings module tests (picker renders options, change calls through to the controller)

- [ ] Step 1: Failing test (identity-echo `t` caveat from the shell skill: assert against keys/
  structure, not resolved English).
- [ ] Step 2: Implement following the locale-switcher block's exact shape. Gates + commit.

### Task 11: Floating/drop chrome completion

**Files:**
- Modify: `src/modules/panels/src/panels.scss` (the header's recorded future work: drop
  overlays, tab-group chips, floating titlebar — skin all dockview chrome onto tier-2 tokens
  through the existing `--dv-*` bridge mapping)

- [ ] Step 1: Read dockview-core 7.0.2's `dist/styles/dockview.css` for the full `--dv-*` and
  class surface; map every chrome color/shadow to tier-2 tokens (no hex literals).
- [ ] Step 2: Visual smoke via the dev server; screenshot-review floating + drop-overlay states.
  Gates + commit.

### Task 12: e2e — theme switch + persistence

**Files:**
- Test: `src/client/shell/e2e/theme.spec.ts`

- [ ] Step 1: Following `panels.spec.ts`'s persistence pattern (payload-matched
  `waitForResponse` on the ui-state PUT before reload): open settings, switch to `slate-light`,
  assert a computed-style token change on `document.documentElement` (inline style), assert the
  PUT payload carries `global.theme`, reload, assert the theme survives (inline styles present
  post-reload, pre-login mirror applied on logout→login screen too if reachable).
- [ ] Step 2: Requires the e2e binary — `pnpm build` then cargo build per the e2e config's
  expectations; run `pnpm --filter @shadowcat/shell exec playwright test e2e/theme.spec.ts`.
  Green → commit.

## Final gates (whole M16a)

- [ ] `pnpm -r test`, `pnpm -r typecheck`, `pnpm lint`, `pnpm lint:docs`, `pnpm lint:props`,
  `pnpm lint:comments`, `pnpm docs:check-examples`, `pnpm lint:file-size`
- [ ] `pnpm build:all` (client build + docs generation incl. exemption count)
- [ ] Full e2e suite green
- [ ] Review checkpoint pass over the whole M16a diff before the milestone notes
