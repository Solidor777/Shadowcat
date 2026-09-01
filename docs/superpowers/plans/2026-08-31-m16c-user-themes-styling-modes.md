# M16c — User Themes + Module Styling Modes — Implementation Plan

> **For agentic workers:** implement task-by-task. Checkbox (`- [ ]`) steps. Commit per green task.

**Goal:** User-authored custom themes (editor + persistence, over the M16a theme engine) and the
module styling-mode contract (`host`/`isolated`) including the external-module CSS pipeline that
makes module styling real at all.

**Architecture:** Custom themes ride the M16a `ThemeController` (`saveCustom`/`deleteCustom`/
`setActive("custom:<id>")` — already implemented and persisted via `UiState.global.theme`). The
styling mode is a `Contribution.styling` field honored at the two content chokepoints
(`<Surface>` and `PanelHost`), with isolation implemented as a generated reset class emitted from
the theme data (never a hand-maintained token list). External-module CSS is a manifest-declared
file served by the existing traversal-guarded static route and injected as a `<link>` at
activation.

**Spec:** `docs/superpowers/specs/2026-08-31-m16-layout-theming-completion-design.md` §C (editor),
§G (styling modes + external CSS). Binding.

## Model/Effort directives

Kimi session; coder subagents per phase. Every subagent prompt carries the campaign owner's
verbatim directive paragraph + the report-delivery requirement.

## Global Constraints

- Same gates and comment rules as M16a/M16b (RULES 15/16; `lint:docs`/`lint:props`/
  `lint:comments` are errors; all strings via `t()` with keys in `en.ts`; safe deletion via
  `trash`; commit per green task; worktree `C:/Dev/Shadowcat-m16`, branch `m16-layout-theming`).
- Run every gate unpiped or with `set -o pipefail` (a piped `tail` masked real failures once
  already this milestone).
- The token set is declared ONCE (ui-kit theme data); the isolation reset and every enumeration
  derive from it.
- WCAG: custom themes are user-authored — the editor shows a live contrast WARNING for the
  documented pairings (text/surface, on-accent/accent) but does not block saving (user agency;
  the built-ins carry the executable audit).

---

## Phase 1 — User theme editor

### Task 1: Theme editor in the settings panel

**Files:**
- Create: `src/modules/settings/src/ThemeEditor.svelte`
- Modify: `src/modules/settings/src/Settings.svelte` (editor section under the theme picker:
  "New custom theme" button; per-custom-theme edit/delete)
- Modify: `src/client/ui-kit/src/locales/en.ts` (`settings.theme.editor.*` keys)
- Test: `src/modules/settings/src/ThemeEditor.test.ts`

**Behavior:**
- Editor state: name (free text), base (built-in picker), and an override row per COLOR token —
  the curated color subset = tokens whose value parses as a color (`#hex`/`rgb`/`hsl`) in EVERY
  built-in (derive by checking the built-in maps; non-color tokens like `--shadow-elevated`,
  `--z-popover`, spacing/font tokens are never user-editable and always inherit the base).
- Each row: token label + `<input type="color">` synced from the effective value (override ?? base
  value) + a per-row reset (clear the override back to base). `#rrggbb` conversion needed for
  color inputs (they only accept hex 6); non-hex-parsable colors (none today — assert in a test
  that every color token in every built-in is `#rrggbb`) get a text input fallback.
- Live preview: editing applies a draft custom theme immediately (`theme.saveCustom` on a draft
  id + `setActive`) — verify against the controller's semantics; if a draft/apply-preview seam
  fits the controller better, extend the controller with an explicit `previewCustom(draft)` and
  `commitPreview(id)` instead of abusing saveCustom. Choose the cleaner shape; the controller is
  ours.
- Contrast warnings: compute the WCAG ratio for the documented pairings (reuse the test-file
  helpers — promote them into a tiny exported `wcagContrast(a, b)` helper in ui-kit's theme
  module, tested) and flag rows below 4.5 inline. Non-blocking.
- Save persists (controller → sessionState subscriber → ui_state, already wired); delete asks
  confirm and falls back to the default theme when the deleted theme was active (controller
  already does the fallback).
- Compact-mode/touch: rows stack; color inputs meet the 44px coarse-pointer floor.

- [x] Failing tests first (editor renders a row per color token, edits call through to the
  controller, contrast warning appears for a bad pairing, reset clears an override).
- [x] Implement. Gates + commit.

## Phase 2 — Module styling modes

### Task 2: `Contribution.styling` + isolation wrapper

**Files:**
- Modify: `src/client/core/src/contributions.ts` (`Contribution.styling?: "host" | "isolated"`,
  default `"host"`, doc'd semantics)
- Modify: `src/client/ui-kit/src/theme.ts` — `themeIsolationCss(): string` generating
  `.sc-theme-isolate { … }` re-declaring every `THEME_TOKEN_NAMES` token at the DEFAULT theme's
  value (single source; a test pins emitted property set ≡ token names)
- Modify: ui-kit bootstrap (inject the sheet once — find where ui-kit/shell global setup lives;
  a tiny module that appends a `<style>` on first import from the shell, or expose
  `installThemeIsolation()` the shell's `main.ts` calls once — pick the shape that keeps ui-kit
  importable without side effects for tests; the explicit install function is preferred)
- Modify: `src/client/ui-kit/src/Surface.svelte` + `src/modules/panels/src/PanelHost.svelte` —
  wrap `styling: "isolated"` content in `<div class="sc-theme-isolate">` (check how each renders
  contribution content; PanelHost's panel content host may be the slot container — find the right
  wrap point that travels with slot adoption into pop-outs)
- Tests: theme.test additions (isolation CSS emission parity), Surface/PanelHost wrapping tests.

- [x] Failing tests first. Implement. Gates + commit.

### Task 3: External-module CSS pipeline

**Files:**
- Modify: `src/client/core/src/manifest.ts` (`ManifestSchema` gains optional `style: string` —
  doc: a single CSS file relative to the module folder)
- Modify: the client module loader (`src/client/core/src/loader.ts` and/or the shell's
  `WorldSession.#loadExternalModules` — find the right layer) — after a module with `style`
  activates, inject `<link rel="stylesheet" href="/modules/<folderId>/<style>">` (the folder id,
  matching the entry URL's construction); track per-module and remove on
  `ModuleRegistry.unload`/reconcile. A 404/failed fetch degrades gracefully (logged, never
  bricks the module — mirror the loader's containment pattern).
- Modify: `examples/module-initiative-tracker/vite.config.ts` (pin the emitted CSS name, e.g.
  `build.lib.cssFileName: "style"` — verify what Vite 8 emits and what option name it takes) and
  its `module.json` (`"style": "style.css"`)
- Modify: `docs/site/guides/creating-a-module.md` — install flow copies the CSS file; new
  theming section (consume tier-2 `var(--*)` tokens; never hardcode colors; `styling` modes and
  the isolation contract; pop-out behavior — theme follows the panel into pop-out windows)
- Tests: loader test (link injected with the right href, removed on unload, failure contained);
  example-module build assertion if the repo has one (check how the guide's code-imports are
  tested — `docs:check-examples` may compile the guide's snippets).

- [x] Verify first: what CSS filename the example build emits and what the vite lib-mode option
  is called (read the installed vite's docs/types).
- [x] Failing tests first. Implement. Gates + commit.

## Phase 3 — e2e + close-out

### Task 4: e2e — custom theme lifecycle

**Files:**
- Test: `src/client/shell/e2e/theme.spec.ts` (extend)

- [x] Create a custom theme in settings (name + base + one override), apply, assert the override
  token is live on `documentElement`, reload, assert the custom theme persists and is still
  active. Delete it, assert fallback to the default theme.
- [ ] Full e2e suite green. Commit.

## Final gates (whole M16c)

- [ ] `pnpm -r test`, `pnpm -r typecheck`, `pnpm lint`, `pnpm lint:docs`, `pnpm lint:props`,
  `pnpm lint:comments`, `pnpm docs:check-examples`, `pnpm lint:file-size`, `pnpm build:all`,
  full e2e suite — all unpiped or pipefail.
- [ ] Review checkpoint over the M16c diff.
