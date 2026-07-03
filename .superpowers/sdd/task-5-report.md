# Task 5 Report — GM tool-rail snap toggle

**Status:** DONE

**Commit:** `7e00aac` — "feat(m10f-3): add a GM tool-rail toggle authoring the scene snapToGrid axis"

---

## Summary
Implemented the GM-authored scene-level `snapToGrid` toggle in `ToolRail.svelte`, wired to
`resolveSceneSettings(...).snapToGrid` for read and `ctx.dispatchIntent` (`/system/snapToGrid`
update) for write, plus the new `tools.snap` locale string. Followed the brief's TDD steps
verbatim.

## Files changed
- `src/client/ui-kit/src/locales/en.ts` — added `"tools.snap": "Snap to grid",` immediately after
  `"tools.color"`.
- `src/modules/scene-tools/src/ToolRail.test.ts` — added `DocumentStore`/`buildSceneDoc`/
  `WireOperation` imports from `@shadowcat/core`; added `sceneStore()` fixture + 3 new tests
  (grid-stepped default pressed + dispatch-on-click, continuous-scene false default, no-active-
  scene renders nothing).
- `src/modules/scene-tools/src/ToolRail.svelte` — added `createSubscriber` + `resolveSceneSettings`/
  `WireDocument` imports; added `subscribe`, `activeScene` (`$derived.by` over
  `ctx.documents.query("scene")[0]`), `snapToGrid` (`$derived.by` over `resolveSceneSettings`), and
  `toggleSnap()` (dispatches an `update` op with `path: "/system/snapToGrid", old: null, new:
  !snapToGrid`); added the `data-testid="snap-toggle"` button to the markup, gated on
  `{#if activeScene}`, immediately after the tool-buttons `{#each}` block and before the
  `controller.active === "place"` branch.

## Tests added + result
- `pnpm --filter @shadowcat/module-scene-tools test -- ToolRail`
  - Before implementation (Step 2, red): 2 of 8 new/existing tests failed as expected
    (`getByTestId("snap-toggle")` not found).
  - After implementation (Step 4, green): **8/8 passed.**
- Full package gate (Step 5):
  - `pnpm --filter @shadowcat/module-scene-tools typecheck` — 0 errors, 0 warnings.
  - `pnpm --filter @shadowcat/module-scene-tools test` — **53/53 passed** (11 test files).
  - `pnpm --filter @shadowcat/ui-kit test` — **19/19 passed** (5 test files).

## Lint/format/typecheck status
Typecheck green (svelte-check, 0 errors/warnings). No separate lint/format step was run for this
task per the brief's Step 5 gate (typecheck + the two test suites only); no lint issues observed
in the diff (matches existing file conventions).

## Deviations from the task spec
None. Implemented the brief's Step 1-6 verbatim (locale string, test imports/fixture/tests,
script-block edits, markup insertion, commit message).

## Reference-pattern verification
The brief's mid-task note asked me to verify the `createSubscriber` + `$derived.by` pattern
against the current codebase shape (e.g. `FactionsPanel`/`GameSettingsPanel`) before assuming it's
accurate. Checked both:
- `FactionsPanel.svelte`: `const subscribe = createSubscriber((update) =>
  ctx.documents.subscribe(update));` then per-derived `subscribe()` calls inside `$derived.by`.
- `GameSettingsPanel.svelte`: identical shape, same comment style ("Calling subscribe() inside...
  registers a reactive dependency on the document store").

Both match the brief's prescribed code exactly — no discrepancy found, no deviation needed.

## Residual risks / skill-update notes
No seam/invariant/gotcha changes to `shadowcat-codebase-scene-rendering` — this task only adds a
thin GM authoring UI over the already-documented `snapToGrid` resolver/data-model (Tasks 1-4,
already merged and presumably already reflected in the skill). No new subsystem opened. Stating
explicitly per the skill-update gate: **no skill update needed for this task.**

No other residual risks identified.

---

## Commit hashes

`7e00aac` (single commit; first == last for this task)

---

## Fix round 1 — Critical finding: stale `old: null` breaks repeated toggles

**Finding:** `toggleSnap()` hardcoded `old: null` in the dispatched `update` op's `FieldChange`.
The server (`Repository::apply_intent`) enforces field-level optimistic concurrency: an `Update`
whose `FieldChange.old` doesn't match the CURRENT stored value at that path is rejected
(`DataError::Conflict`). Since `snapToGrid` (the RESOLVED/defaulted read) is never `null` once
anything has been stored, but `toggleSnap` always sent `old: null`: the first click succeeds
(path genuinely absent, `null` matches), writing e.g. `false`; every subsequent click still sends
`old: null` against a now-present stored value — mismatch — server rejects with `Conflict`, write
never applies, toggle silently stops working after one use.

### Fix
`src/modules/scene-tools/src/ToolRail.svelte` — `toggleSnap()` now reads the RAW stored value off
`scene.system` (not the resolved/defaulted `snapToGrid`), falling back to `null` only when
genuinely absent:

```ts
function toggleSnap(): void {
  const scene = activeScene;
  if (!scene) return;
  const rawSnap = (scene.system as { snapToGrid?: boolean } | undefined)?.snapToGrid ?? null;
  ctx.dispatchIntent([
    { op: "update", doc_id: scene.id, changes: [{ path: "/system/snapToGrid", old: rawSnap, new: !snapToGrid }] },
  ]);
}
```

Mirrors the exact convention already used by this same file's sibling, `controller.svelte.ts`'s
`sendMoves` (`sys?.x ?? null`).

### Test update
- `src/modules/scene-tools/src/ToolRail.test.ts` — the pre-existing "grid-stepped default:
  pressed" test's `old: null` assertion is UNCHANGED (still correct: that fixture's scene has no
  `snapToGrid` field set, so the field is genuinely absent and `null` is the right raw value).
- Added a NEW regression test: "the snap toggle sends the ACTUAL stored value as `old`, not null,
  when snapToGrid was already explicitly stored" — seeds `sceneStore({ snapToGrid: true })`,
  clicks the toggle, and asserts the dispatched op's `old` is `true` (the actual stored value),
  not `null`. This is the test that would have caught the Critical finding.

### Second-occurrence check
Reviewed the full Task 5 diff (`git show 7e00aac --stat`): only 3 files touched —
`src/client/ui-kit/src/locales/en.ts` (one locale string, no op-dispatch code),
`ToolRail.svelte` (the single `toggleSnap` function — the only occurrence), and
`ToolRail.test.ts` (tests only). Confirmed: no second copy of the `old: null` shortcut exists
anywhere in the Task 5 diff. (The separately-noted pre-existing occurrences in
`GameSettingsPanel.svelte`/`FactionsPanel.svelte`/`ConditionsPanel.svelte` are out of scope for
this fix, per instruction — not touched.)

### Test output
```
pnpm --filter @shadowcat/module-scene-tools test -- ToolRail
 Test Files  1 passed (1)
      Tests  9 passed (9)

pnpm --filter @shadowcat/module-scene-tools typecheck
1783074457297 START "c:\\Dev\\Shadowcat\\src\\modules\\scene-tools"
1783074457303 COMPLETED 949 FILES 0 ERRORS 0 WARNINGS 0 FILES_WITH_PROBLEMS
```

Both green.

---

## Whole-branch buddy-check fix

**Finding:** The snap-toggle button is double-gated — an outer `{#if isGm}` AND an inner
`{#if activeScene}`. The pre-existing "a non-GM sees no tool buttons" test renders with
`{ role: "player" }` and no `documents` override, so `ctx.documents` defaults to an empty
`DocumentStore()` and `activeScene` is `undefined` there — meaning the INNER gate alone hides the
button in that test, not the `isGm` gate. A naive addition of a `snap-toggle` assertion to that
existing test would pass vacuously: it would still pass even if `{#if isGm}` were accidentally
deleted in a future refactor, since the inner gate would still hide the button.

### Fix
Added a NEW, separate test in `ToolRail.test.ts` that seeds a scene doc (via the existing
`sceneStore()` fixture, making `activeScene` defined) while using `role: "player"` — isolating
the `isGm` gate as the ONLY thing hiding the button in that test:

```ts
test("a non-GM does not see the snap toggle even with an active scene", () => {
  render(ToolRail, { context: setAppContextForTest({ role: "player", documents: sceneStore() }) });
  expect(screen.queryByTestId("snap-toggle")).toBeNull();
});
```

The existing "a non-GM sees no tool buttons" test was left untouched — it remains valid for its
own stated purpose (no tool buttons render for a non-GM with no active scene), just not sufficient
on its own to isolate the `isGm` gate specifically.

### Test output
```
pnpm --filter @shadowcat/module-scene-tools test -- ToolRail
 Test Files  1 passed (1)
      Tests  10 passed (10)

pnpm --filter @shadowcat/module-scene-tools typecheck
1783079639379 START "c:\\Dev\\Shadowcat\\src\\modules\\scene-tools"
1783079639383 COMPLETED 949 FILES 0 ERRORS 0 WARNINGS 0 FILES_WITH_PROBLEMS
```

Both green.
