# Task 8 Report: Per-token face-swap palette (M10h)

## Summary
Added a selection-driven face-swap palette to `ActorsPanel.svelte`, mirroring
`ConditionsPanel.svelte`'s toggle-palette pattern: when a token whose effective actor visual is
`"faces"` is selected, buttons for each declared face name appear; clicking one dispatches a
`/system/face` Update on the TOKEN document with `old` read from the raw stored value.

## Files changed
- `src/modules/actors/src/ActorsPanel.svelte` — added `selectedFaceToken`, `selectedFaceNames`,
  `currentFace()`, `swapFace()` (script), and the face-palette markup block, inserted right after
  `<h3>{t("actors.title")}</h3>`. Implemented the brief's **corrected** final script block only
  (not its flagged first exploratory draft).
- `src/modules/actors/src/ActorsPanel.test.ts` — new `describe("ActorsPanel — per-token face
  swap", ...)` block (3 tests) + `buildTokenFromActor`/`TokenSelection` imports.

## Tests added + result
- `pnpm --filter @shadowcat/module-actors test ActorsPanel.test.ts` → 16/16 passed (13 pre-existing
  + 3 new).
- `pnpm --filter @shadowcat/module-actors test` (full package) → 17/17 passed.

## Lint/format/typecheck status
- `pnpm --filter @shadowcat/module-actors typecheck` → 938 files, 0 errors, 0 warnings.
- `pnpm lint` (workspace ESLint) → clean.

## Deviations from the task spec
1. **Did not add `resolveTokenVisual` to the `@shadowcat/core` import**, contrary to brief Step 3's
   literal instruction. The brief's own corrected final script (the block explicitly marked "use
   this corrected form") never calls `resolveTokenVisual` — it derives `selectedFaceNames` by
   reading the actor doc's raw `system.visual` directly. `resolveTokenVisual` collapses `faces` →
   a resolved `image`/`animated` `RenderVisual` and can't answer "is this token's visual kind
   `faces`", so it has no legitimate use in this task's logic. Importing it unused would fail
   `noUnusedLocals: true` (`tsconfig.base.json`), which is enforced by `typecheck` (Step 5). This
   import line is a leftover of the same exploratory-draft cleanup the brief itself flags for the
   script logic; omitting it is required for the plan's own Step 5 to pass.
2. **Test's `tokenSelection` fixture uses a real `new TokenSelection()` instance (with `.set([...])`)
   instead of the brief's literal `{ ids: new Set([...]) }`.** `TokenSelection` (`src/client/ui-
   kit/src/tokenSelection.svelte.ts`) has a private `#ids` field, so a structurally-similar object
   literal does not satisfy its type under `svelte-check` (3 errors: "missing #ids, has, set,
   toggle, clear"), failing Step 5's typecheck gate. `ConditionsPanel.test.ts` (the brief's own
   named precedent) already uses `new TokenSelection(); tokenSelection.set([...])` for exactly this
   reason — followed that established, typecheck-clean pattern instead.

Both deviations are required to satisfy the brief's own Step 5 ("typecheck + full package test
run must PASS"); the brief's literal code as written does not typecheck. No behavioral/markup
deviation — the palette script logic and template are implemented exactly as the brief's
corrected form specifies.

## Residual risks / skill-update notes
This is the last authoring-UI piece of M10h per the task list; no new subsystem/seam opened.
CORRECTION (see Fix section below): `shadowcat-codebase-actors-tokens` DOES need updating for the
shipped `TokenVisual`/`resolveTokenVisual`/faces-palette content shipped across M10h — that update
is deliberately DEFERRED to Task 10 (a dedicated final "reviewed skill-update gate" task, already
written, covering exactly this content), not skipped. The original wording above ("no skill update
needed") was inaccurate and is corrected here rather than left standing.

## Commit
`b1cf776` — "feat(m10h): per-token face-swap palette (mirrors module-conditions' toggle palette)"

## Fix (resolver reuse + styling + report accuracy)

Addressed three code-review findings against commit `b1cf776`.

### 1. [Important] `selectedFaceNames` now routes through `resolveTokenActor`
Replaced the inline `tok.system.actor_id` / `tok.embedded?.actor?.[0]` branch-and-read-raw-`system.
visual` logic with a call through `resolveTokenActor(token, store)` (`@shadowcat/core`,
`src/client/core/src/actor.ts`), the canonical single read-through documented in
`shadowcat-codebase-actors-tokens`. This also picks up `project()`'s `overrides?.visual ?? base.
visual` whitelist, so a token with a per-token `overrides.visual` now resolves its effective kind
correctly instead of showing a stale/wrong palette derived from the actor's raw visual.

- Added `resolveTokenActor` to the existing `@shadowcat/core` import line in
  `src/modules/actors/src/ActorsPanel.svelte` (no new import statement).
- `selectedFaceNames` body:
  ```typescript
  const selectedFaceNames = $derived.by((): string[] => {
    subscribe();
    const tok = selectedFaceToken;
    if (!tok) return [];
    const eff = resolveTokenActor(tok, ctx.documents);
    return eff?.visual.kind === "faces" ? Object.keys(eff.visual.faces) : [];
  });
  ```
- `TokenVisual` import remains used elsewhere in the file (`buildVisual(): TokenVisual | null`),
  so it was kept.

### 2. [Minor] Added scoped CSS for the face-palette markup
Added `.hint`, `.face-palette`, `.face-palette button`, `.face-palette button.active` rules to
`ActorsPanel.svelte`'s own `<style lang="scss">` block, adapted from `ConditionsPanel.svelte`'s
`.hint`/`.palette`/`.palette button.active` rules (renamed `.palette` → `.face-palette` to avoid
colliding with this file's own class naming and to match the markup's actual class). Buttons get
`min-width: 44px; min-height: 44px` (this file's own `.list button` convention uses `min-height:
44px`; ConditionsPanel's `.palette button` only used 36px — bumped to 44px here to satisfy the
project's touch-target requirement rather than copying the smaller precedent verbatim).

### 3. Report-accuracy correction
Corrected the "Residual risks / skill-update notes" section above: the original claim that no
`shadowcat-codebase-actors-tokens` update was needed was inaccurate. The skill genuinely needs
updating for the `TokenVisual`/`resolveTokenVisual`/faces-palette content shipped across M10h, but
that update is deliberately deferred to Task 10 (a dedicated final skill-update-gate task already
scoped to cover it), not skipped or waived.

### Verification
- `pnpm --filter @shadowcat/module-actors test ActorsPanel.test.ts` → 16/16 passed, unchanged
  (existing tests construct no token with `overrides.visual`, so behavior for linked/instanced
  tokens without overrides is identical to the pre-fix inline logic).
- `pnpm --filter @shadowcat/module-actors typecheck` → 938 files, 0 errors, 0 warnings.

### Status
DONE.

### Deviations from the fix spec
None.

### Residual risks
None beyond the already-scheduled Task 10 skill-update gate (see correction above). No new
subsystem/seam opened by this fix.
