# M12e — Pop-out Windows Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the fifth panel presentation state — **popped-out** (a panel rendered in a same-heap child `window`) — as M12's final sub-checkpoint, driving dockview-core's native `addPopoutGroup` from a user gesture, recording the state declaratively in the layout tree, and rehydrating popped-out panels as floating on reload.

**Architecture:** Pop-out is **dockview-only** and **imperative at gesture time** (the `window.open` inside `addPopoutGroup` must fire in the click's synchronous tick or the browser blocks it — the same gesture-timing constraint that already forces `#floatInvokers` to be captured synchronously). The pure reducer grows a `poppedOut: string[]` field + `popOut`/`popIn` ops; the engine calls `addPopoutGroup` synchronously and emits the tree op only after the async result resolves; `apply()` treats live popped-out ids as hands-off (never orphan-removes them). Popouts never survive reload (a page load is not a gesture), so a persisted popped-out id rehydrates to floating + a notice. The `FakeEngine` bespoke-fallback degrades pop-out to a floating window.

**Tech Stack:** TypeScript, Svelte 5 (runes), dockview-core@7.0.2 (already adopted, seam-confined to `engine/dockview.ts`), Vitest + jsdom, SCSS tokens, project i18n.

## Global Constraints

- **Branch:** `m12e-popout-windows` off local `main` (currently `76a7918`, post-M12d). **No push** (standing directive; M11+ body still unpushed).
- **Engine seam:** `dockview-core` imports are confined to `src/modules/panels/src/engine/dockview.ts` (+ its `.test.ts`) — ESLint `no-restricted-imports` enforced (`eslint.config.js:15-27`). All `window.open`/popup orchestration stays inside `dockview.ts` (driven through dockview's own `api.addPopoutGroup`, never a hand-rolled `window.open`).
- **Keep-mounted discipline** (M12a-onward, extended): dock⇄float⇄minimize⇄**pop-out** transitions re-parent the panel's DOM element; they NEVER destroy/recreate it. Chat scroll/composer drafts survive.
- **`svelte:boundary` crash containment** already wraps every panel body (`PanelHost.svelte:257`); a popped-out panel is the SAME mounted Svelte instance re-parented into the child window (same JS realm), so its existing boundary continues to contain crashes — no boundary change is needed and none is made (verified: the component is never re-created on pop-out).
- **Pop-out URL is same-origin-only** (verified in vendored source, see Design Decision 5): dockview's `PopoutWindow.open()` calls `assertSameOriginPopoutUrl` and rejects `about:blank`/`data:`/cross-origin; the popout loads `/popout.html`, served same-origin from the client build's `public/` dir (precedent: `site.webmanifest`, favicons).
- **i18n:** every user-facing string via `t(key)` with a key added to `src/client/ui-kit/src/locales/en.ts`.
- **A11y:** the pop-out affordance IS the menu path (no drag equivalent exists anywhere for pop-out); `:focus-visible`, ≥44px touch targets, live-region announcement — unchanged from prior M12 checkpoints.
- **Per-task gates:** `pnpm --filter @shadowcat/module-panels test` AND `pnpm --filter @shadowcat/module-panels typecheck` ([[vitest-skips-typecheck-in-sdd]]). Repo-wide `pnpm lint` on the final sweep (the seam rule is lint-enforced).
- **Zero-history comments** (present-tense, no ticket/PR/narrative meta).
- **No server-side change** — entirely a client-side panel-manager feature. `/popout.html` is a client build artifact (embedded from `dist/` by the existing static handler), not server code.

## Model/Effort directives

Per the user's standing directive (2026-07-13): plan written mainline on the design-session model; execution = SDD with `shadowcat-coder` (Sonnet, effort medium) implementers, `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` (effort high) per task, `-opus` twins for buddy-checks/escalation and the whole-branch final review. Dispatcher = this session, mainline.

## Buddy-check directives (pre-authorized — spec §15: "the M12e pop-out same-heap lifecycle")

- **Task 5** (DockviewEngine pop-out lifecycle) — **full two-reviewer blind buddy-check**. This is the exact surface §15 pre-authorizes: gesture-time `addPopoutGroup` invocation, cross-document element re-parenting (dockview's, spike-verified), popup-close detection (`onDidRemovePopoutGroup`), the blocked-popup fallback, and the reentrancy guards (`#applying`, the popped-out-group map) against a popup closing mid-`apply()`. Reviewers must construct sequences where a popout closes during reconcile and assert no double-`popIn`, no lost panel, no destroyed slot.
- **Task 6** (controller rehydration + host wiring) — **full buddy-check**, because it carries the load-bearing **persisted-id-array vs. live-window-handle** distinction (Design Decision 2): the persisted tree holds ids only; the live `Window` is owned by dockview; on reload the persisted ids can never re-open as popups and MUST rehydrate to floating. Reviewers verify no path tries to reopen a popup without a gesture and no popped-out id is ever dropped from the tree without being re-placed.

All other tasks take the standard single-reviewer-pair gate.

---

## Design Decisions (resolved ambiguities — verified against real vendored source, not survey prose)

1. **Pop-out is dockview-only; `FakeEngine` degrades to floating.** Re-implementing same-heap `window.open` + cross-document stylesheet cloning + close-lifecycle in the bespoke `FakeEngine` would duplicate exactly the machinery the M12a-0 spike adopted dockview to get for free ([[verify-crate-claims-against-vendored-source]]). Pop-out is the last, most-isolable checkpoint ("cannot destabilize the rest" — spec D4); scoping the real popout to the production engine keeps the fallback simple. `FakeEngine.apply()` renders a popped-out id as a floating window (spec §10's sanctioned fallback), so a slot is never lost and the keep-mounted invariant holds under it.

2. **Persisted state = id array; live handle = dockview's.** The tree gains `poppedOut: string[]` (persisted, like `minimized`). The live `Window` is owned by dockview (`group.model.location.getWindow()`); our engine keeps only a `Map<popoutGroupId, panelId[]>` for close-translation, cleared on removal/destroy — never a serialized `Window`. On reload the id array survives but the window cannot (Decision 4), so the ids rehydrate to floating.

3. **Pop-out does NOT fit the declarative `apply()`-reconcile pattern; it is gesture-time imperative.** `apply()` runs inside a Svelte `$effect` (a microtask after the gesture), so calling `window.open` from `apply()` loses the user-gesture context and the browser blocks the popup. Verified: `PopoutWindow.open()` calls `window.open(...)` synchronously before its first `await` (`popoutWindow.js:95`), so `api.addPopoutGroup(panel)` invoked synchronously in the menu-click handler keeps the popup within the gesture. This mirrors the existing `#floatInvokers` synchronous capture (`dockview.ts:382`). The engine therefore calls `addPopoutGroup` in `#handleMenuCommand` and emits the `popOut` tree op only after the returned `Promise<boolean>` resolves `true`.

4. **Popouts never survive reload → rehydrate to floating.** A page load is not a user gesture; reopening a popup at boot is popup-blocked. So a persisted `poppedOut` id is converted to a `float` op once, at controller construction (before first `apply()`), with a `panels.popoutRestoredFloating` notice. Spec §7's `"poppedOut": ["chat"]` persistence example is honored (the codec round-trips the field) but §10's gesture requirement makes live reopening impossible; this is the documented reconciliation of that tension (see Spec Gaps).

5. **Cross-document stylesheets are dockview's job, already solved — verified.** `popoutWindow.js:136` calls `addStyles(externalDocument, window.document.styleSheets, { nonce })`; `dom.js:135-165` clones every opener stylesheet into the child document as a `<link href>` (for external sheets) or an inline `<style>` from `cssRules` (for inline sheets). No bespoke stylesheet injection is built. The one thing dockview does NOT provide is the same-origin loader document, so we ship `/popout.html` (default `popoutUrl`, `dockviewComponent.js:666`).

6. **No `classifyDrop`/`DropSite` change.** Pop-out has no drag gesture anywhere in spec or code (confirmed: `MenuCommand` is menu-only; `#toDropSite` builds only `edge`/`group` sites). Pop-out flows solely through the command menu.

---

## File Structure (locked)

**Modified:**
- `src/modules/panels/src/layout/tree.ts` — `ExpandedLayout.poppedOut`, `PanelLocation` `"popped-out"`, `LayoutOp` `popOut`/`popIn`, reducer cases, `detach`/`locate`/`prune`/`placeFromPersistedLocation`/`defaultLayout` growth. (Task 1)
- `src/modules/panels/src/layout/tree.test.ts` — reducer + prune + persisted-location tests for the new state. (Task 1)
- `src/modules/panels/src/layout/persist.ts` — validate/round-trip/back-compat-normalize `poppedOut`. (Task 2)
- `src/modules/panels/src/layout/persist.test.ts` — codec tests. (Task 2)
- `src/modules/panels/src/engine/policy.ts` — `"popOut"` `MenuCommand` + `opForMenuCommand` case. (Task 3)
- `src/modules/panels/src/engine/policy.test.ts` — `opForMenuCommand("popOut")` test. (Task 3)
- `src/modules/panels/src/PanelMenu.svelte` — `Pop out` menu item. (Task 3)
- `src/client/ui-kit/src/locales/en.ts` — `panels.popOut` + notice keys. (Task 3)
- `src/modules/panels/src/engine/adapter.ts` — optional `onNotice` on `EngineAdapter`. (Task 5)
- `src/modules/panels/src/engine/dockview.ts` — pop-out lifecycle (BUDDY-CHECK). (Task 5)
- `src/modules/panels/src/engine/dockview.test.ts` — pop-out op-translation tests (fake `popoutDriver`). (Task 5)
- `src/modules/panels/src/engine/fake.ts` — `poppedOut`-as-floating degradation. (Task 6)
- `src/modules/panels/src/engine/fake.test.ts` — degradation test. (Task 6)
- `src/modules/panels/src/controller.svelte.ts` — `EMPTY_LAYOUT.poppedOut`, `onNotice` dep, `#rehydratePoppedOut`. (Task 6)
- `src/modules/panels/src/controller.test.ts` — rehydration test. (Task 6)
- `src/modules/panels/src/PanelHost.svelte` — wire `onNotice` → live region; `describeOp` popOut/popIn cases. (Task 6)
- `src/modules/panels/src/PanelHost.test.ts` — mount-counter pop-out leg. (Task 7)

**Created:**
- `src/client/shell/public/popout.html` — minimal same-origin loader document for the popout window. (Task 4)

---

### Task 1: Reducer — `poppedOut` state + `popOut`/`popIn` ops

**Files:**
- Modify: `src/modules/panels/src/layout/tree.ts`
- Test: `src/modules/panels/src/layout/tree.test.ts`

**Interfaces:**
- Consumes: existing `detach`, `locate`, `placeByPlacement`, `prune`, `compactZ`, `SHEET_CASCADE_BASE`.
- Produces: `ExpandedLayout.poppedOut: string[]`; `PanelLocation` variant `{ where: "popped-out" }`; `LayoutOp` variants `{ op: "popOut"; id: string }` and `{ op: "popIn"; id: string }`. `popOut` no-ops when already popped-out; `popIn` no-ops when not popped-out and otherwise docks the panel into a new `right` group (mirrors `restore`).

- [ ] **Step 1: Write the failing tests**

Add to `src/modules/panels/src/layout/tree.test.ts`:

```ts
import { describe, test, expect } from "vitest";
import { applyOp, defaultLayout, locate, prune, type PanelLayoutV1 } from "./tree";

describe("popOut / popIn", () => {
  function docked(): PanelLayoutV1 {
    let l = defaultLayout([{ id: "chat" }]);
    return applyOp(l, { op: "dock", id: "chat", zone: "right", group: "new" });
  }

  test("popOut detaches from its zone and records the id in poppedOut", () => {
    const l = applyOp(docked(), { op: "popOut", id: "chat" });
    expect(l.expanded.poppedOut).toEqual(["chat"]);
    expect(l.expanded.zones.right.groups).toEqual([]);
    expect(locate(l, "chat")).toEqual({ where: "popped-out" });
  });

  test("popOut on an already-popped-out id is a same-reference no-op", () => {
    const l1 = applyOp(docked(), { op: "popOut", id: "chat" });
    const l2 = applyOp(l1, { op: "popOut", id: "chat" });
    expect(l2).toBe(l1);
  });

  test("popIn removes the id from poppedOut and docks it right", () => {
    const l1 = applyOp(docked(), { op: "popOut", id: "chat" });
    const l2 = applyOp(l1, { op: "popIn", id: "chat" });
    expect(l2.expanded.poppedOut).toEqual([]);
    expect(l2.expanded.zones.right.groups[0].tabs).toEqual(["chat"]);
  });

  test("popIn on a non-popped-out id is a same-reference no-op", () => {
    const l = docked();
    expect(applyOp(l, { op: "popIn", id: "chat" })).toBe(l);
  });

  test("float on a popped-out id detaches it from poppedOut", () => {
    const l1 = applyOp(docked(), { op: "popOut", id: "chat" });
    const l2 = applyOp(l1, { op: "float", id: "chat", rect: { x: 1, y: 2, w: 3, h: 4 } });
    expect(l2.expanded.poppedOut).toEqual([]);
    expect(l2.expanded.floating.map((f) => f.id)).toEqual(["chat"]);
  });

  test("prune drops an unknown popped-out id", () => {
    const l = applyOp(docked(), { op: "popOut", id: "chat" });
    const pruned = prune(l, new Set());
    expect(pruned.expanded.poppedOut).toEqual([]);
  });

  test("prune with all ids known is a same-reference no-op", () => {
    const l = applyOp(docked(), { op: "popOut", id: "chat" });
    expect(prune(l, new Set(["chat"]))).toBe(l);
  });
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `pnpm --filter @shadowcat/module-panels test -- tree.test.ts`
Expected: FAIL (`poppedOut` is `undefined`; `popOut`/`popIn` not handled).

- [ ] **Step 3: Grow `ExpandedLayout`, `PanelLocation`, `LayoutOp`**

In `tree.ts`, change `ExpandedLayout`:

```ts
export interface ExpandedLayout {
  // All three ZoneId keys are always present, even with empty groups — callers never
  // guard a missing zone.
  zones: Record<ZoneId, ZoneNode>;
  floating: { id: string; rect: Rect; z: number }[];
  minimized: string[];
  // Ids currently rendered in a same-heap child window (dockview popout). PERSISTED
  // as ids only — the live `Window` handle is dockview's, never serialized. A page
  // load cannot reopen a popup (no user gesture), so a persisted entry rehydrates to
  // floating at controller construction; during a live session an id lands here only
  // after a gesture-time `addPopoutGroup` succeeds.
  poppedOut: string[];
}
```

Add the `LayoutOp` variants (after `compactView`):

```ts
  | { op: "compactView"; id: string }
  | { op: "popOut"; id: string }
  | { op: "popIn"; id: string };
```

Add the `PanelLocation` variant (after `minimized`):

```ts
  | { where: "minimized" }
  | { where: "popped-out" }
  | { where: "closed" };
```

- [ ] **Step 4: Grow `locate`, `detach`, `applyOp`, `prune`, `placeFromPersistedLocation`, `defaultLayout`**

In `locate`, add the check before the final `return`:

```ts
  if (l.expanded.minimized.includes(id)) return { where: "minimized" };
  if (l.expanded.poppedOut.includes(id)) return { where: "popped-out" };
  return { where: "closed" };
```

In `detach`'s switch, add a case (before `case "docked"`):

```ts
    case "popped-out": {
      const poppedOut = l.expanded.poppedOut.filter((p) => p !== id);
      return [{ ...l, expanded: { ...l.expanded, poppedOut } }, loc];
    }
```

In `applyOp`'s switch, add two cases (after `case "compactView"`):

```ts
    case "popOut": {
      // Same-reference no-op for an already-popped-out id, mirroring "float"'s
      // already-floating guard. The gesture-time `addPopoutGroup` in
      // `DockviewEngine` only emits this op AFTER a successful async open, so a
      // popped-out id here is always backed by a live child window.
      const loc = locate(l, o.id);
      if (loc.where === "popped-out") return l;
      const [l1] = detach(l, o.id);
      return { ...l1, expanded: { ...l1.expanded, poppedOut: [...l1.expanded.poppedOut, o.id] } };
    }

    case "popIn": {
      // Returns a popped-out panel to a new docked "right" group (mirrors
      // "restore"). Emitted by the engine when a popout window closes; the
      // menu's dock/float/minimize commands pop a panel in via their own ops
      // (detach handles the "popped-out" source location).
      const loc = locate(l, o.id);
      if (loc.where !== "popped-out") return l;
      const [l1] = detach(l, o.id);
      return placeByPlacement(l1, o.id, { kind: "docked", zone: "right" });
    }
```

In `prune`, add the filter (after the `minimizedKept` block):

```ts
  const poppedOutKept = l.expanded.poppedOut.filter((id) => known.has(id));
  const poppedOutChanged = poppedOutKept.length !== l.expanded.poppedOut.length;
  if (poppedOutChanged) changed = true;
```

And in `prune`'s return object's `expanded`, add:

```ts
      minimized: minimizedChanged ? minimizedKept : l.expanded.minimized,
      poppedOut: poppedOutChanged ? poppedOutKept : l.expanded.poppedOut,
```

In `placeFromPersistedLocation`'s switch, add a case (before `case "closed"`) — a persisted popout rehydrates to floating (Decision 4):

```ts
    case "popped-out": {
      // Popouts never survive reload (no gesture to reopen the window); a
      // persisted popped-out panel comes back as floating. Same rule as
      // `PanelsController.#rehydratePoppedOut`, applied to the not-yet-
      // registered-panel path.
      const maxZ = l.expanded.floating.reduce((m, f) => Math.max(m, f.z), -1);
      const floating = compactZ([...l.expanded.floating, { id, rect: { ...SHEET_CASCADE_BASE }, z: maxZ + 1 }]);
      return { ...l, expanded: { ...l.expanded, floating } };
    }
```

In `defaultLayout`'s `empty` literal, add `poppedOut: []`:

```ts
    expanded: { zones: emptyZones(), floating: [], minimized: [], poppedOut: [] },
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `pnpm --filter @shadowcat/module-panels test -- tree.test.ts`
Expected: PASS.

- [ ] **Step 6: Typecheck**

Run: `pnpm --filter @shadowcat/module-panels typecheck`
Expected: PASS (no missing-property errors on `ExpandedLayout` literals elsewhere; `persist.ts` and `controller.svelte.ts` literals are fixed in Tasks 2 and 6 — if typecheck flags them now, that is expected and those tasks close it; run this step's typecheck AFTER Tasks 2 and 6 in the final sweep. For this task, `tree.ts` + `tree.test.ts` compile).

- [ ] **Step 7: Commit**

```bash
git add src/modules/panels/src/layout/tree.ts src/modules/panels/src/layout/tree.test.ts
git commit -m "feat(panels/m12e): reducer poppedOut state + popOut/popIn ops"
```

---

### Task 2: Persistence codec — validate + round-trip + back-compat `poppedOut`

**Files:**
- Modify: `src/modules/panels/src/layout/persist.ts`
- Test: `src/modules/panels/src/layout/persist.test.ts`

**Interfaces:**
- Consumes: `decodeLayout(raw, known, fallback)` returning `{ layout, reset, source }`.
- Produces: same signature; a decoded layout always has `expanded.poppedOut` as an array (absent-in-blob normalizes to `[]`; present-but-malformed fails the whole decode → reset).

- [ ] **Step 1: Write the failing tests**

Add to `src/modules/panels/src/layout/persist.test.ts`:

```ts
import { test, expect } from "vitest";
import { decodeLayout } from "./persist";
import { defaultLayout, applyOp } from "./tree";

test("decode round-trips poppedOut ids", () => {
  let l = defaultLayout([{ id: "chat" }]);
  l = applyOp(l, { op: "dock", id: "chat", zone: "right", group: "new" });
  l = applyOp(l, { op: "popOut", id: "chat" });
  const { layout, reset } = decodeLayout(l, new Set(["chat"]), () => defaultLayout([]));
  expect(reset).toBe(false);
  expect(layout.expanded.poppedOut).toEqual(["chat"]);
});

test("decode of a pre-M12e blob (no poppedOut field) normalizes to []", () => {
  const legacy = {
    version: 1,
    expanded: { zones: { right: { groups: [], size: 320 }, bottom: { groups: [], size: 240 }, left: { groups: [], size: 320 } }, floating: [], minimized: [] },
    compact: { activeView: null, order: [] },
  };
  const { layout, reset } = decodeLayout(legacy, new Set(), () => defaultLayout([]));
  expect(reset).toBe(false);
  expect(layout.expanded.poppedOut).toEqual([]);
});

test("decode rejects a non-string-array poppedOut", () => {
  const bad = {
    version: 1,
    expanded: { zones: { right: { groups: [], size: 320 }, bottom: { groups: [], size: 240 }, left: { groups: [], size: 320 } }, floating: [], minimized: [], poppedOut: [1, 2] },
    compact: { activeView: null, order: [] },
  };
  const { reset } = decodeLayout(bad, new Set(), () => defaultLayout([]));
  expect(reset).toBe(true);
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `pnpm --filter @shadowcat/module-panels test -- persist.test.ts`
Expected: FAIL (`poppedOut` unvalidated / not normalized; the legacy-blob case may crash the reducer via `undefined.filter`).

- [ ] **Step 3: Validate `poppedOut` (tolerant read) in `isExpandedLayout`**

In `persist.ts`, replace the final line of `isExpandedLayout`:

```ts
  return isStringArray(e.minimized);
```

with:

```ts
  if (!isStringArray(e.minimized)) return false;
  // Back-compat: a pre-M12e blob has no `poppedOut`; absent normalizes to []
  // in `decodeLayout`. A present-but-malformed value fails the whole blob.
  return e.poppedOut === undefined || isStringArray(e.poppedOut);
```

- [ ] **Step 4: Normalize in `decodeLayout`**

In `persist.ts`, add a helper above `decodeLayout`:

```ts
/** Fills an absent `poppedOut` (pre-M12e blob) with `[]` so reducer arithmetic
 * (`prune`/`locate`/`detach`) never dereferences `undefined`. Returns the input
 * untouched when the field is already an array (the common, current-version path). */
function withPoppedOut(l: PanelLayoutV1): PanelLayoutV1 {
  if (Array.isArray(l.expanded.poppedOut)) return l;
  return { ...l, expanded: { ...l.expanded, poppedOut: [] } };
}
```

Replace `decodeLayout`'s body's non-reset return line:

```ts
  return { layout: prune(raw, known), reset: false, source: raw };
```

with:

```ts
  const normalized = withPoppedOut(raw);
  return { layout: prune(normalized, known), reset: false, source: normalized };
```

(The `raw` cast to `PanelLayoutV1` already permits an absent `poppedOut`; `withPoppedOut` makes it real before any reducer use, and `source` carries the normalized form so `placeNewRegistrations` sees a consistent shape.)

- [ ] **Step 5: Run the tests to verify they pass**

Run: `pnpm --filter @shadowcat/module-panels test -- persist.test.ts`
Expected: PASS.

- [ ] **Step 6: Typecheck**

Run: `pnpm --filter @shadowcat/module-panels typecheck`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/modules/panels/src/layout/persist.ts src/modules/panels/src/layout/persist.test.ts
git commit -m "feat(panels/m12e): persist poppedOut with pre-M12e back-compat normalize"
```

---

### Task 3: Menu command — `popOut` policy op + menu item + i18n

**Files:**
- Modify: `src/modules/panels/src/engine/policy.ts`
- Modify: `src/modules/panels/src/engine/policy.test.ts`
- Modify: `src/modules/panels/src/PanelMenu.svelte`
- Modify: `src/client/ui-kit/src/locales/en.ts`

**Interfaces:**
- Consumes: `opForMenuCommand(command, id)` returning `ClassifyResult`; the `STAGE_ID` up-front veto.
- Produces: `MenuCommand` gains `"popOut"`; `opForMenuCommand("popOut", id)` returns `{ op: "popOut", id }` (or the stage veto). `PanelMenu` renders a `Pop out` item (`data-testid="panel-menu-popOut"`). i18n keys `panels.popOut`, `panels.popoutBlocked`, `panels.popoutRestoredFloating`.

- [ ] **Step 1: Write the failing test**

Add to `src/modules/panels/src/engine/policy.test.ts`:

```ts
import { test, expect } from "vitest";
import { opForMenuCommand, STAGE_ID } from "./policy";

test("opForMenuCommand maps popOut to a popOut op", () => {
  expect(opForMenuCommand("popOut", "chat")).toEqual({ op: "popOut", id: "chat" });
});

test("opForMenuCommand vetoes popOut on the stage", () => {
  const result = opForMenuCommand("popOut", STAGE_ID);
  expect("veto" in result).toBe(true);
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter @shadowcat/module-panels test -- policy.test.ts`
Expected: FAIL (`"popOut"` not assignable to `MenuCommand`; no case).

- [ ] **Step 3: Grow `MenuCommand` + `opForMenuCommand`**

In `policy.ts`, change `MenuCommand`:

```ts
export type MenuCommand = "dockRight" | "dockBottom" | "dockLeft" | "float" | "minimize" | "popOut" | "close";
```

In `opForMenuCommand`'s switch, add a case (before `case "close"`):

```ts
    case "popOut":
      return { op: "popOut", id };
```

- [ ] **Step 4: Add the menu item**

In `PanelMenu.svelte`, add to the `items` array (before the `close` entry):

```ts
    { cmd: "popOut", labelKey: "panels.popOut" },
    { cmd: "close", labelKey: "panels.close" },
```

- [ ] **Step 5: Add i18n keys**

In `src/client/ui-kit/src/locales/en.ts`, add after `"panels.float"`:

```ts
  "panels.popOut": "Pop out",
  "panels.popoutBlocked": "Pop-out was blocked; opened as a floating window instead",
  "panels.popoutRestoredFloating": "Popped-out panels reopen as floating windows after reload",
```

- [ ] **Step 6: Run tests + typecheck**

Run: `pnpm --filter @shadowcat/module-panels test -- policy.test.ts`
Expected: PASS.
Run: `pnpm --filter @shadowcat/module-panels typecheck`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/modules/panels/src/engine/policy.ts src/modules/panels/src/engine/policy.test.ts src/modules/panels/src/PanelMenu.svelte src/client/ui-kit/src/locales/en.ts
git commit -m "feat(panels/m12e): pop-out menu command + policy op + i18n"
```

---

### Task 4: Same-origin popout loader document

**Files:**
- Create: `src/client/shell/public/popout.html`

**Interfaces:**
- Consumes: nothing (static asset).
- Produces: a same-origin document served at `/popout.html` (the default `popoutUrl`, `dockviewComponent.js:666`) that satisfies `assertSameOriginPopoutUrl` (`popoutWindow.js:19-31`). dockview appends the re-parented panel container into this document's `<body>` and clones the opener's stylesheets in via `addStyles` on the window's `load` event — no script is needed here (the panel is the same mounted Svelte instance from the opener's JS realm).

- [ ] **Step 1: Create the loader document**

`src/client/shell/public/popout.html`:

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>Shadowcat</title>
  </head>
  <body></body>
</html>
```

- [ ] **Step 2: Build the client and confirm the asset is emitted same-origin**

Run: `pnpm --filter @shadowcat/shell build`
Expected: build succeeds and `dist/popout.html` exists (Vite copies `public/` verbatim to the build root, the same path `site.webmanifest`/favicons take).

Run: `test -f dist/popout.html && echo PRESENT`
Expected: `PRESENT`.

(The server embeds `dist/` via rust-embed and serves exact-match embedded files at their path — the precedent is `site.webmanifest` served at `/site.webmanifest`. No server change is required. If a reviewer finds the static handler serves ONLY `index.html` for non-asset paths, flag it as the spec gap noted below and stop — do not add server code without consent.)

- [ ] **Step 3: Commit**

```bash
git add src/client/shell/public/popout.html
git commit -m "feat(panels/m12e): same-origin popout loader document"
```

---

### Task 5: DockviewEngine pop-out lifecycle — BUDDY-CHECK

**Files:**
- Modify: `src/modules/panels/src/engine/adapter.ts`
- Modify: `src/modules/panels/src/engine/dockview.ts`
- Test: `src/modules/panels/src/engine/dockview.test.ts`

**Interfaces:**
- Consumes: `api.addPopoutGroup(panel, options?): Promise<boolean>` (`false` on popup-block); `api.onDidRemovePopoutGroup(cb)` firing `{ id, group, window }` from dockview's single removal funnel (window-close AND explicit removal), suppressed during component teardown; `MENU_FLOAT_RECT`, `opForMenuCommand` from `policy.ts`; existing `#applying` guard, `#opListeners`, `#floatInvokers`.
- Produces: `EngineAdapter.onNotice?(cb: (key: string) => void): () => void` (optional). `DockviewEngine` constructor gains an optional 2nd param `popoutDriver?: (panel: IDockviewPanel) => Promise<boolean>` (test seam; defaults to `addPopoutGroup(panel, { popoutUrl: "/popout.html" })`). Gesture-time pop-out emits `{ op: "popOut", id }` on success, `{ op: "float", id, rect: MENU_FLOAT_RECT }` + a `panels.popoutBlocked` notice on block/throw. A closed popout window emits `{ op: "popIn", id }` per member (unless `#applying`). `apply()` seeds `seenPanelIds` with `expanded.poppedOut` so a live popped-out panel is never orphan-removed.

- [ ] **Step 1: Add optional `onNotice` to the adapter seam**

In `adapter.ts`, add to the `EngineAdapter` interface (after `onOp`):

```ts
  /** Subscribes to user-facing engine notices (spec §10) — a stable i18n key
   * the host resolves + surfaces (live region / toast). Optional: engines with
   * no notice source (`FakeEngine`) omit it. Returns an unsubscribe. */
  onNotice?(cb: (key: string) => void): () => void;
```

- [ ] **Step 2: Write the failing tests**

Add to `src/modules/panels/src/engine/dockview.test.ts` (the file already has the `makeSlots`/`twoPanelLayout` harness + `attachedHost` teardown; reuse them). These tests inject a fake `popoutDriver` so jsdom never touches `window.open`:

```ts
import { STAGE_ID } from "./policy";

/** Mounts an engine on a body-attached host with one docked panel and clicks
 * its tab menu's "Pop out" item, returning the ops emitted. `driver` stands in
 * for `addPopoutGroup` (jsdom has no real `window.open`). */
async function popOutViaMenu(driver: () => Promise<boolean>): Promise<{ ops: LayoutOp[]; notices: string[] }> {
  const host = document.createElement("div");
  document.body.appendChild(host);
  attachedHost = host;
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);
  engine = new DockviewEngine(silentLogger, driver);
  engine.init(host, slotFor, stageEl);

  const ops: LayoutOp[] = [];
  const notices: string[] = [];
  engine.onOp((op) => ops.push(op));
  engine.onNotice?.((key) => notices.push(key));

  let l = defaultLayout([{ id: "chat" }]);
  l = applyOp(l, { op: "dock", id: "chat", zone: "right", group: "new" });
  engine.apply(l.expanded, new Map([["chat", { icon: "c", labelKey: "chat.tab" } as PanelMeta]]));

  const menuBtn = host.querySelector<HTMLButtonElement>(".sc-tab-menu-btn");
  menuBtn?.click();
  const popOutItem = document.querySelector<HTMLButtonElement>('[data-testid="panel-menu-popOut"]');
  popOutItem?.click();
  // Let the injected driver's promise resolve.
  await Promise.resolve();
  await Promise.resolve();
  return { ops, notices };
}

test("pop-out: a successful driver emits a popOut op (no float, no notice)", async () => {
  const { ops, notices } = await popOutViaMenu(() => Promise.resolve(true));
  expect(ops).toContainEqual({ op: "popOut", id: "chat" });
  expect(ops.some((o) => o.op === "float")).toBe(false);
  expect(notices).toEqual([]);
});

test("pop-out blocked: a false driver falls back to a float op + a notice (spec §10)", async () => {
  const { ops, notices } = await popOutViaMenu(() => Promise.resolve(false));
  expect(ops.some((o) => o.op === "float" && o.id === "chat")).toBe(true);
  expect(ops.some((o) => o.op === "popOut")).toBe(false);
  expect(notices).toEqual(["panels.popoutBlocked"]);
});

test("pop-out rejected: a throwing driver falls back to a float op + a notice", async () => {
  const { ops, notices } = await popOutViaMenu(() => Promise.reject(new Error("boom")));
  expect(ops.some((o) => o.op === "float" && o.id === "chat")).toBe(true);
  expect(notices).toEqual(["panels.popoutBlocked"]);
});

test("apply seeds seenPanelIds with poppedOut so a live popout is never orphan-removed", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);
  engine = new DockviewEngine(silentLogger, () => Promise.resolve(true));
  engine.init(host, slotFor, stageEl);

  // Establish the panel, then a tree that marks it popped-out.
  let l = defaultLayout([{ id: "chat" }]);
  l = applyOp(l, { op: "dock", id: "chat", zone: "right", group: "new" });
  engine.apply(l.expanded, new Map());
  expect(engine.debugApi?.getPanel("chat")).toBeTruthy();

  l = applyOp(l, { op: "popOut", id: "chat" });
  engine.apply(l.expanded, new Map());
  // The panel is NOT torn out of dockview's model by the orphan-removal loop.
  expect(engine.debugApi?.getPanel("chat")).toBeTruthy();
});
```

- [ ] **Step 3: Run to verify they fail**

Run: `pnpm --filter @shadowcat/module-panels test -- dockview.test.ts`
Expected: FAIL (constructor takes no 2nd arg; `onNotice` undefined; pop-out click emits nothing / the panel is orphan-removed after `popOut`).

- [ ] **Step 4: Add the popout driver + notice channel + constructor**

In `dockview.ts`, add `MENU_FLOAT_RECT` to the existing `policy` import:

```ts
import { classifyDrop, opForMenuCommand, MENU_FLOAT_RECT, STAGE_ID, type DropSite, type MenuCommand } from "./policy";
```

Add fields to `DockviewEngine` (near `#opListeners`):

```ts
  #noticeListeners = new Set<(key: string) => void>();
  // Popout group id -> the panel ids it hosts, recorded when a pop-out succeeds
  // so `onDidRemovePopoutGroup` (window closed by the user) can translate a
  // group id back into `popIn` ops without depending on the group's live panel
  // membership at fire time (dockview's teardown may have already moved them).
  #poppedOutGroupPanels = new Map<string, string[]>();
  // Gesture-time popout invoker. Defaults to dockview's native popout (verified
  // same-heap, content re-parented, stylesheets cloned — M12a-0 spike +
  // popoutWindow.js:136). Injectable so unit tests exercise the async-result →
  // op translation without a real `window.open` (jsdom has none).
  #popoutDriver: (panel: IDockviewPanel) => Promise<boolean>;
```

Replace the constructor:

```ts
  constructor(logger?: Logger, popoutDriver?: (panel: IDockviewPanel) => Promise<boolean>) {
    this.#logger = logger ?? consoleLogger();
    // `/popout.html` is the same-origin loader document (Task 4); passed
    // explicitly to document the dependency rather than relying on dockview's
    // own default drifting. `assertSameOriginPopoutUrl` (popoutWindow.js)
    // rejects `about:blank`/cross-origin, so this URL is load-bearing.
    this.#popoutDriver = popoutDriver ?? ((panel) => this.#api!.addPopoutGroup(panel, { popoutUrl: "/popout.html" }));
  }
```

Add the `onNotice` method + emit helper (near `onOp`):

```ts
  onNotice(cb: (key: string) => void): () => void {
    this.#noticeListeners.add(cb);
    return () => this.#noticeListeners.delete(cb);
  }

  #emitNotice(key: string): void {
    for (const cb of this.#noticeListeners) cb(key);
  }
```

- [ ] **Step 5: Intercept `popOut` in `#handleMenuCommand` + implement `#requestPopOut`**

In `#handleMenuCommand`, replace the tail (after the veto check):

```ts
    if (cmd === "float") this.#floatInvokers.set(id, invoker);
    for (const cb of this.#opListeners) cb(result);
```

with:

```ts
    if (cmd === "float") this.#floatInvokers.set(id, invoker);
    if (cmd === "popOut") {
      // Pop-out is imperative + gesture-timed: `window.open` (inside
      // addPopoutGroup, synchronous before its first await) must run in THIS
      // click's tick or the browser blocks it. The tree op is emitted only
      // after the async open resolves — unlike every other menu command, which
      // reduces declaratively through `apply()`. Same gesture-timing reason
      // `#floatInvokers` is captured synchronously above.
      this.#requestPopOut(id);
      return;
    }
    for (const cb of this.#opListeners) cb(result);
```

Add the method:

```ts
  /** Gesture-time pop-out: drives dockview's native `addPopoutGroup`
   * synchronously (preserving the user gesture), then translates the async
   * result into a tree op. Success ⇒ `popOut` (records the id + its live popout
   * group for close-translation). Block/throw ⇒ spec §10 fallback: `float` +
   * a `panels.popoutBlocked` notice. */
  #requestPopOut(id: string): void {
    const api = this.#api;
    if (!api) return;
    const panel = api.getPanel(id);
    if (!panel) return;
    this.#popoutDriver(panel)
      .then((ok) => {
        if (ok) {
          const gid = api.getPanel(id)?.group.id;
          if (gid) this.#poppedOutGroupPanels.set(gid, [id]);
          for (const cb of this.#opListeners) cb({ op: "popOut", id });
        } else {
          for (const cb of this.#opListeners) cb({ op: "float", id, rect: MENU_FLOAT_RECT });
          this.#emitNotice("panels.popoutBlocked");
        }
      })
      .catch((err) => {
        this.#logger.warn("panels: pop-out failed; falling back to floating", { id, err });
        for (const cb of this.#opListeners) cb({ op: "float", id, rect: MENU_FLOAT_RECT });
        this.#emitNotice("panels.popoutBlocked");
      });
  }
```

- [ ] **Step 6: Subscribe to `onDidRemovePopoutGroup` + translate to `popIn`**

In `init()`, add to the `this.#disposables.push(...)` list:

```ts
      api.onDidActivePanelChange((event) => this.#handleActivePanelChange(event)),
      api.onDidRemovePopoutGroup((event) => this.#handleRemovePopoutGroup(event)),
```

Add the handler:

```ts
  /** A popout window closed — by the user (OS window close, funneled through
   * dockview's single removal path), or by our own reconcile (`apply()` moving
   * a panel out of its popout because the tree no longer marks it popped-out).
   * The map cleanup runs unconditionally; the `popIn` redispatch is suppressed
   * during `#applying` — that removal is our own reconciliation of a tree the
   * reducer already updated (identical reasoning to `#handleDidRemovePanel`/
   * `#handleActivePanelChange`), so re-emitting would replay a stale op. */
  #handleRemovePopoutGroup(event: { id: string; group: IDockviewGroupPanel }): void {
    const ids = this.#poppedOutGroupPanels.get(event.id) ?? event.group.model.panels.map((p) => p.id);
    this.#poppedOutGroupPanels.delete(event.id);
    if (this.#applying) return;
    for (const id of ids) {
      if (id === STAGE_ID) continue;
      for (const cb of this.#opListeners) cb({ op: "popIn", id });
    }
  }
```

- [ ] **Step 7: Seed `apply()`'s `seenPanelIds` with `poppedOut`**

In `apply()`, replace:

```ts
      const seenPanelIds = new Set<string>([STAGE_ID]);
```

with:

```ts
      // Popped-out ids stay in dockview's model (a same-heap popout group), so
      // seed them here — otherwise the orphan-removal loop below tears the live
      // popout panel out of its window. The zone/floating placement loops never
      // list a popped-out id, so `apply()` leaves the popout untouched (hands-
      // off). A menu dock/float on a popped-out panel drops the id from
      // `poppedOut` first, so it is NOT seeded and the placement loops move it
      // back (removePanel of the popout's last panel disposes the window — the
      // resulting onDidRemovePopoutGroup is `#applying`-suppressed).
      const seenPanelIds = new Set<string>([STAGE_ID, ...expanded.poppedOut]);
```

- [ ] **Step 8: Clean the map on destroy**

In `destroy()`, add (near the other `.clear()` calls):

```ts
    this.#poppedOutGroupPanels.clear();
    this.#noticeListeners.clear();
```

- [ ] **Step 9: Run the tests to verify they pass**

Run: `pnpm --filter @shadowcat/module-panels test -- dockview.test.ts`
Expected: PASS.

- [ ] **Step 10: Typecheck**

Run: `pnpm --filter @shadowcat/module-panels typecheck`
Expected: PASS.

- [ ] **Step 11: Commit**

```bash
git add src/modules/panels/src/engine/adapter.ts src/modules/panels/src/engine/dockview.ts src/modules/panels/src/engine/dockview.test.ts
git commit -m "feat(panels/m12e): DockviewEngine pop-out lifecycle (gesture-time addPopoutGroup + popIn on close)"
```

- [ ] **Buddy-check** (two blind reviewers, `-opus` twins — spec §15). Reviewers independently: (a) construct a sequence where a popout window closes DURING an `apply()` and assert no double-`popIn`, no stale op, no orphan-removed slot; (b) verify the gesture-timing claim (`window.open` is synchronous before the first `await` in `popoutWindow.js`) actually holds and that `#requestPopOut` is reached synchronously from the click; (c) confirm the blocked/throw fallbacks both emit `float` + exactly one notice; (d) confirm cross-document stylesheet propagation is dockview's (`addStyles`) and not silently missing; (e) confirm the seam boundary is intact (no `dockview-core` symbol escapes `dockview.ts`).

---

### Task 6: Controller rehydration + host wiring + FakeEngine degradation — BUDDY-CHECK

> **Dispatcher amendment (Task 1 review finding, 2026-07-15):** the `REHYDRATE_FLOAT_RECT`
> single fixed rect below was found to duplicate a real bug the Task 1 reviewer caught in
> `placeFromPersistedLocation`'s sibling code — every rehydrated id lands at the identical
> `(x,y)`, invisibly stacked and distinguishable only by z-order. Corrected below to cascade
> the same way `placeByPlacement`'s floating branch and the now-fixed `placeFromPersistedLocation`
> "popped-out" case do. When implementing this task, add at least one assertion in the
> rehydration test that rehydrates TWO popped-out ids and asserts their floating rects differ
> (mirroring the reducer-level regression test added to `tree.test.ts` in Task 1).

**Files:**
- Modify: `src/modules/panels/src/controller.svelte.ts`
- Modify: `src/modules/panels/src/controller.test.ts`
- Modify: `src/modules/panels/src/PanelHost.svelte`
- Modify: `src/modules/panels/src/engine/fake.ts`
- Modify: `src/modules/panels/src/engine/fake.test.ts`

**Interfaces:**
- Consumes: `applyOp`, `decodeLayout`'s `layout`/`source`; `PanelsControllerDeps`; `EngineAdapter.onNotice?`; the `announce` live-region `$state` in `PanelHost`.
- Produces: `PanelsControllerDeps.onNotice?: (key: string) => void`; `PanelsController` rehydrates every persisted `poppedOut` id to `float` at construction (Decision 4) and fires `onNotice("panels.popoutRestoredFloating")` when it did so; `EMPTY_LAYOUT` gains `poppedOut: []`. `FakeEngine.apply()` renders a `poppedOut` id as a floating window (Decision 1). `PanelHost` wires both `ctrl` `onNotice` and `eng.onNotice?` into `announce`, and `describeOp` narrates `popOut`/`popIn`.

- [ ] **Step 1: Write the failing tests**

Add to `src/modules/panels/src/controller.test.ts` (reuse its existing `makeDeps`/registry harness; the block below shows the shape — adapt id/field names to the file's existing helper):

```ts
import { test, expect } from "vitest";
import { PanelsController } from "./controller.svelte";
import { defaultLayout, applyOp } from "./layout/tree";
import { silentLogger } from "@shadowcat/core";

test("rehydratePoppedOut: a persisted popped-out id comes back as floating + a notice", () => {
  let saved = defaultLayout([{ id: "chat", placement: { kind: "docked", zone: "right" } }]);
  saved = applyOp(saved, { op: "dock", id: "chat", zone: "right", group: "new" });
  saved = applyOp(saved, { op: "popOut", id: "chat" });

  const notices: string[] = [];
  const ctrl = new PanelsController({
    // Reuse this file's existing registry/deps helper for `contributions`,
    // `role`, `bridge`; only these three fields differ per test:
    ...makeBaseDeps([{ id: "chat", panel: { icon: "c", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } } }]),
    getPanelLayout: () => saved,
    setPanelLayout: () => {},
    onNotice: (key) => notices.push(key),
    logger: silentLogger,
  });

  expect(ctrl.layout.expanded.poppedOut).toEqual([]);
  expect(ctrl.layout.expanded.floating.map((f) => f.id)).toEqual(["chat"]);
  expect(notices).toEqual(["panels.popoutRestoredFloating"]);
});
```

Add to `src/modules/panels/src/engine/fake.test.ts`:

```ts
test("poppedOut degrades to a floating window (bespoke-fallback, spec §10)", () => {
  const host = document.createElement("div");
  const slotFor = makeSlots(["chat"]); // reuse this file's existing slot helper
  const eng = new FakeEngine();
  eng.init(host, slotFor, document.createElement("div"));

  let l = defaultLayout([{ id: "chat" }]);
  l = applyOp(l, { op: "dock", id: "chat", zone: "right", group: "new" });
  l = applyOp(l, { op: "popOut", id: "chat" });
  eng.apply(l.expanded, new Map());

  // Rendered as a float window, so the slot stays adopted (never lost).
  const floatEl = eng.floatEl("chat");
  expect(floatEl).not.toBeNull();
  expect(floatEl?.contains(slotFor("chat"))).toBe(true);
  eng.destroy();
});
```

- [ ] **Step 2: Run to verify they fail**

Run: `pnpm --filter @shadowcat/module-panels test -- controller.test.ts fake.test.ts`
Expected: FAIL (`onNotice` not a dep; no rehydration; `FakeEngine` ignores `poppedOut`).

- [ ] **Step 3: Grow `PanelsControllerDeps` + `EMPTY_LAYOUT` + rehydration**

In `controller.svelte.ts`, add to `PanelsControllerDeps` (after `onOp`):

```ts
  /** Fired with a user-facing i18n key for an engine/layout notice the caller
   * surfaces (live region / toast) — e.g. `panels.popoutRestoredFloating` when
   * reload rehydrates a popped-out panel to floating (a page load cannot reopen
   * a popup), or `panels.popoutBlocked` forwarded from the engine. */
  onNotice?: (key: string) => void;
```

In `EMPTY_LAYOUT`, add `poppedOut: []`:

```ts
  expanded: { zones: { right: { groups: [], size: 320 }, bottom: { groups: [], size: 240 }, left: { groups: [], size: 320 } }, floating: [], minimized: [], poppedOut: [] },
```

Add a rehydrate rect constant (near `EMPTY_LAYOUT`):

```ts
// Cascade base/step for a reload-rehydrated (formerly popped-out) panel's floating
// rect — an unoffset rect would stack every rehydrated popout (and the first-ever
// floating panel) at the identical (x,y). Mirrors tree.ts's own
// SHEET_CASCADE_BASE/STEP formula (not imported — that pair is layout-internal;
// this is the controller's own, deliberately separate constant so the two call
// sites cannot silently drift together).
const REHYDRATE_FLOAT_BASE = { x: 96, y: 96, w: 420, h: 520 };
const REHYDRATE_FLOAT_STEP = 28;
```

In the constructor, after `this.#persistedSource = source;` and the `reset` block, add:

```ts
    this.#rehydratePoppedOut();
```

Add the method (near `syncRegistrations`):

```ts
  /** Popouts cannot be reopened without a user gesture (a page load is not one
   * — the browser blocks it), so every persisted popped-out id rehydrates to a
   * floating window at construction, before the first `apply()`. The tree's
   * `poppedOut` array persists across sessions; the live `Window` never does.
   * Runs once; persists + notifies only if it actually converted anything. */
  #rehydratePoppedOut(): void {
    const ids = [...this.#layout.expanded.poppedOut];
    if (ids.length === 0) return;
    let l = this.#layout;
    for (const id of ids) {
      // Cascade off the CURRENT floating count each iteration (not the loop
      // index) so rehydrated popouts interleave correctly with any panel
      // that was already floating before rehydration ran.
      const n = l.expanded.floating.length;
      const off = (n % 6) * REHYDRATE_FLOAT_STEP;
      const rect = { x: REHYDRATE_FLOAT_BASE.x + off, y: REHYDRATE_FLOAT_BASE.y + off, w: REHYDRATE_FLOAT_BASE.w, h: REHYDRATE_FLOAT_BASE.h };
      l = applyOp(l, { op: "float", id, rect });
    }
    this.#layout = l;
    this.#persist(l);
    this.#deps.onNotice?.("panels.popoutRestoredFloating");
  }
```

- [ ] **Step 4: Degrade `poppedOut` to floating in `FakeEngine.apply`**

In `fake.ts`, replace the floating section of `apply()` (the `floatIds`/cleanup/create block) so it treats `poppedOut` ids as floating windows too:

```ts
    // Floating: one container per floating panel, adopted directly and
    // positioned from its `Rect`. Popped-out ids are degraded to floating here
    // (this bespoke-fallback engine has no cross-window popout; spec §10) so a
    // slot is never lost and the keep-mounted invariant holds — production
    // pop-out is dockview-only.
    const POPOUT_FALLBACK_RECT = { x: 96, y: 96, w: 420, h: 520 };
    const floatEntries = [
      ...expanded.floating,
      ...expanded.poppedOut.map((id) => ({ id, rect: POPOUT_FALLBACK_RECT, z: 0 })),
    ];
    const floatIds = new Set(floatEntries.map((f) => f.id));
    for (const [id, el] of [...this.#floatEls]) {
      if (!floatIds.has(id)) {
        el.remove();
        this.#floatEls.delete(id);
      }
    }
    for (const f of floatEntries) {
      let el = this.#floatEls.get(f.id);
      if (!el) {
        el = document.createElement("div");
        el.dataset.floating = f.id;
        host.appendChild(el);
        this.#floatEls.set(f.id, el);
      }
      el.style.left = `${f.rect.x}px`;
      el.style.top = `${f.rect.y}px`;
      el.style.width = `${f.rect.w}px`;
      el.style.height = `${f.rect.h}px`;
      el.style.zIndex = String(f.z);
      const slot = slotFor(f.id);
      el.appendChild(slot);
      slot.style.display = "";
    }
```

- [ ] **Step 5: Wire the notice + narration in `PanelHost.svelte`**

In `PanelHost.svelte`, in the `PanelsController` construction, replace `onReset: () => {},` with:

```ts
        onReset: () => {},
        onNotice: (key) => {
          announce = t(key);
        },
```

In the engine-init `$effect`, wire the engine's optional notice channel:

```ts
    const unsubOp = eng.onOp((op) => {
      ctrl.dispatch(op);
    });
    const unsubNotice = eng.onNotice?.((key) => {
      announce = t(key);
    });
    return () => {
      unsubOp();
      unsubNotice?.();
      eng.destroy();
      stageHomeEl = null;
    };
```

In `describeOp`, add narration cases (before the `default`):

```ts
      case "popOut":
        where = t("panels.popOut");
        break;
      case "popIn":
        where = t("panels.restore");
        break;
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `pnpm --filter @shadowcat/module-panels test -- controller.test.ts fake.test.ts`
Expected: PASS.

- [ ] **Step 7: Typecheck**

Run: `pnpm --filter @shadowcat/module-panels typecheck`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src/modules/panels/src/controller.svelte.ts src/modules/panels/src/controller.test.ts src/modules/panels/src/PanelHost.svelte src/modules/panels/src/engine/fake.ts src/modules/panels/src/engine/fake.test.ts
git commit -m "feat(panels/m12e): rehydrate popouts to floating on reload + host notice wiring + FakeEngine degradation"
```

- [ ] **Buddy-check** (two blind reviewers, `-opus` twins — spec §15). Reviewers verify: (a) no path reopens a popup without a user gesture; (b) a popped-out id is never dropped from the tree without being re-placed (rehydration converts, never deletes); (c) the persisted-id-vs-live-window distinction holds end-to-end (nothing serializes a `Window`; `#persistedSource` staying popped-out is reconstructed as floating via `placeFromPersistedLocation`); (d) `FakeEngine` degradation keeps the slot adopted (mount count stable).

---

### Task 7: Mount-discipline pop-out leg + checkpoint verification sweep

**Files:**
- Modify: `src/modules/panels/src/PanelHost.test.ts`

**Interfaces:**
- Consumes: the existing mount-counter test harness (`engine.emitOp`, `mounts`, `tick`).
- Produces: the mount-counter guard extended with a `popOut ⇄ popIn` leg (spec §12: "extends the M11d-1 guard").

- [ ] **Step 1: Extend the mount-counter test**

In `src/modules/panels/src/PanelHost.test.ts`, inside the `"mount-counter: a docked panel's component mounts exactly once across the full op lifecycle"` test, after the existing `float` leg (the last `expect(mounts).toBe(1)` before the test closes), add:

```ts
  // Pop-out leg (M12e): dock⇄float⇄...⇄pop-out⇄pop-in never re-mounts. The
  // FakeEngine degrades pop-out to a floating window, so the slot is re-parented
  // (adopted), never recreated.
  engine.emitOp({ op: "popOut", id: "chat:panel" });
  await tick();
  expect(mounts).toBe(1);

  engine.emitOp({ op: "popIn", id: "chat:panel" });
  await tick();
  expect(mounts).toBe(1);
```

- [ ] **Step 2: Run the mount-counter test**

Run: `pnpm --filter @shadowcat/module-panels test -- PanelHost.test.ts`
Expected: PASS (`mounts` stays 1 across the pop-out leg).

- [ ] **Step 3: Full module test + typecheck sweep**

Run: `pnpm --filter @shadowcat/module-panels test`
Expected: PASS (all suites green).
Run: `pnpm --filter @shadowcat/module-panels typecheck`
Expected: PASS.

- [ ] **Step 4: Lint sweep (seam boundary is lint-enforced)**

Run: `pnpm lint`
Expected: PASS — no `dockview-core` import escapes `engine/dockview.ts`; the pop-out orchestration all lives inside that file.

- [ ] **Step 5: Full client build (embed ordering + popout asset)**

Run: `pnpm --filter @shadowcat/shell build`
Expected: PASS; `dist/popout.html` present ([[embed-dist-compile-ordering]] — the server embeds `dist/` at compile time).

- [ ] **Step 6: Commit**

```bash
git add src/modules/panels/src/PanelHost.test.ts
git commit -m "test(panels/m12e): mount-counter pop-out leg (extends the M11d-1 guard)"
```

---

## Post-execution gates (SDD process, after Task 7)

- **Whole-branch buddy-check** (`-opus` twins): the two buddy-checked tasks (5, 6) reviewed together against the assembled pop-out lifecycle — gesture timing, persisted-vs-live state, reentrancy, seam integrity, keep-mounted.
- **Reviewed skill-update gate** (mandatory, doc-sync tier): update `shadowcat-codebase-panels` (create it if M12a did not — a panels subsystem without a skill is itself a gate violation) with the new pop-out seam: `poppedOut` tree field, gesture-time imperative pop-out (does NOT flow through `apply()`), popouts-never-survive-reload rehydration, `/popout.html` same-origin dependency, dockview `addPopoutGroup`/`onDidRemovePopoutGroup`/`addStyles` reliance. Dispatch `shadowcat-spec-reviewer` on the skill diff (PASS required before merge).
- **Documentation sync:** `docs/PLAN.md` (M12e → complete; M12 milestone closed), `MEMORY.md`/topic file (M12e shipped). Log any API friction found to `docs/POST_WORK_FINDINGS.md` (PLAN's M12 rule).

---

## Self-Review

**1. Spec coverage:**
- D4 pop-out in v1 as M12e final sub-checkpoint → Tasks 1–7. ✔
- §9 menu exposes "Pop out" (menu = keyboard/SR/touch path) → Task 3 (`PanelMenu` item + i18n). ✔
- §10 blocked pop-out ⇒ floating + notice → Task 5 (`false`/throw → `float` + `panels.popoutBlocked`). ✔
- §10 `svelte:boundary` still wraps a popped-out panel → Global Constraints + Decision 3 (same mounted instance re-parented; boundary unchanged, verified). ✔
- §10 engine exceptions caught / state only from validated transitions → Task 5 (`.catch` fallback; the tree op is emitted only on resolved success). ✔
- §11 dockview adopted, spike-cited → Decision 5 + Task 5 (native `addPopoutGroup`, `addStyles`, `onDidRemovePopoutGroup`, all vendored-source-verified). ✔
- §12 mount-counter grows a pop-out leg → Task 7. ✔
- §15 buddy-check "M12e pop-out same-heap lifecycle" → Buddy-check directives (Tasks 5 + 6). ✔
- Keep-mounted extends to pop-out → Global Constraints + Task 7 guard. ✔
- Persisted-id-vs-live-window distinction → Decision 2 + Tasks 1/2/6. ✔
- FakeEngine parity decision → Decision 1 + Task 6. ✔
- Cross-document stylesheets → Decision 5 (dockview's `addStyles`, verified; no bespoke build). ✔

**2. Placeholder scan:** No "TBD"/"handle edge cases"/prose-only steps; every code step carries complete code. The two test blocks that say "reuse this file's existing helper" (`makeBaseDeps`/`makeSlots`) reference helpers already present in those test files (`controller.test.ts`, `fake.test.ts`) — the implementer adapts to the exact existing helper name; this is codebase-integration, not a placeholder for missing logic.

**3. Type consistency:** `poppedOut: string[]` used identically across `tree.ts`, `persist.ts`, `controller.svelte.ts`, `fake.ts`. Ops `{op:"popOut"|"popIn"; id}` consistent reducer↔policy↔engine↔host. `MenuCommand` `"popOut"` consistent policy↔`PanelMenu`. `onNotice(cb:(key:string)=>void):()=>void` identical on `EngineAdapter`, `DockviewEngine`, and `PanelsControllerDeps.onNotice`. `popoutDriver:(panel:IDockviewPanel)=>Promise<boolean>` consistent constructor↔field↔tests. i18n keys `panels.popOut`/`panels.popoutBlocked`/`panels.popoutRestoredFloating` consistent across Task 3, 5, 6.

## Spec Gaps (flagged for a human — not silently resolved)

1. **§7 persisted `poppedOut` vs. §10 gesture requirement (reconciled, but worth confirming).** §7 shows `"poppedOut": ["chat"]` persisting as if a popout re-opens on reload; §10 + browser reality forbid reopening a popup without a user gesture. This plan honors §7 (the codec round-trips the field) but rehydrates to floating on load (Decision 4) with a `panels.popoutRestoredFloating` notice. If the intended behavior is instead to prompt the user to re-pop-out on load (a gesture-gated "reopen" affordance), that is a different, larger design and should be a follow-up — not folded in here.
2. **`/popout.html` served same-origin by the static handler (Task 4, Step 2).** The plan asserts the rust-embed static handler serves exact-match embedded files at their path (precedent: `site.webmanifest`). This is verified by the build-artifact check but NOT by an integration test against the running server in this plan. If the handler in fact rewrites all non-`/api` paths to `index.html` (SPA-catch-all), `/popout.html` would load the full app and pop-out would misbehave — the implementer must stop and flag it (Task 4 note) rather than add server code without consent (constraint: no server-side change).
3. **jsdom cannot exercise the real `window.open`/`addStyles`/`onDidRemovePopoutGroup` path** (only dockview's spike-verified machinery does). The plan unit-tests our translation logic via the injected `popoutDriver` and the pure reducer; the real cross-window re-parent + stylesheet clone + OS-window-close→`popIn` is dockview's (spike-verified) plus a documented manual-QA item, consistent with the existing `#toDropSite` pointer-geometry manual-QA residue already accepted in `dockview.ts`.
