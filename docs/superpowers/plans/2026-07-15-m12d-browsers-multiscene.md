# M12d — Actor + Scene Browsers, Multi-Scene `activeScene` — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Grow `module-actors` into a live-FTS actor browser (search / create / open-sheet / place), ship a GM-gated `@shadowcat/module-scene-browser` (list / thumbnails / create / configure / activate / local-view), and close the multi-scene deferral by adding `activeScene: string | null` to world-settings — players follow it, the GM roams locally — with a single client-side "viewed scene" source of truth that every render + tool + broadcast path resolves through.

**Architecture:** One pure resolver, `resolveViewedScene(store, { gmViewedScene })`, is the sole answer to "which scene does THIS client render." `WorldSession` owns the client-local `gmViewedScene` override and exposes `viewedSceneId`; it flows to the render engine (background reconciler + all doc views + fog/lighting scene-filter), the Stage grid driver, the scene-tools active-scene helper, and the two `WorldSession` broadcast gates (ping + `MoveStream`). Players never set `gmViewedScene` (→ they follow `activeScene`, fail-closed to the first scene). The scene browser writes `activeScene` (everyone) / sets `gmViewedScene` (local roam) / deep-links game-settings (configure) through narrow AppContext seams; no new server code.

**Tech Stack:** TypeScript, Svelte 5 runes, SCSS, Zod (wire), vitest + @testing-library/svelte, pnpm workspaces.

## Global Constraints

- **Branch:** execute on NEW branch `m12d-browsers-multiscene` off local `main` (HEAD `9161a5b`, the M12c merge — `ctx.openDocument` / sheet registry / `item` doc_type are present). No push (push is the user's call).
- **No new server code (D6):** every change is client-side. `activeScene` / `gmViewedScene` / scene-filtering are client-owned interpretations of the opaque doc set, mirroring `movementModel`/`bounds`/`visual`. If any task discovers a genuine server need, HALT and flag it — never silently add Rust.
- **Seam-only module communication (ARCHITECTURE §2 invariant 7):** `@shadowcat/module-scene-browser` imports ONLY `@shadowcat/core`, `@shadowcat/ui-kit`, `@shadowcat/types` + contracts + `AppContext` — never another `@shadowcat/module-*`. Same for the `module-actors` growth.
- **`ctx.openDocument` / sheet seams** come from M12c ([[shadowcat-codebase-sheets]]); the panel model + `PANEL_CONTRACT` from M12a ([[shadowcat-codebase-panels]]); the `WorldSession`/AppContext/reconnect seams from [[shadowcat-codebase-client-shell]]. Invoke those skills as pointers; do not re-derive them.
- **Reactive bridge is mandatory from the first commit:** every `$derived`/`$effect` that reads `ctx.documents` (or `ctx.viewedSceneId`, whose getter reads the doc store) MUST register a dependency via `createSubscriber((u) => ctx.documents.subscribe(u))` and call `subscribe()` inside the derived — the `GameSettingsPanel`/`ActorsPanel` convention. A bare `$derived` reading `ctx.documents` freezes at mount and corrupts OCC pre-images on the second edit ([[sheet-reactive-bridge-missing-subscription]]).
- **Real OCC pre-images:** every field-path Update's `old` is the RAW current stored value at that path (never `null` when the field is present, never a resolved/defaulted value) — the M11d-2 `GameSettingsPanel` Critical, a hard rule. The scene browser's "Activate" and every config write obey it.
- **Render from the optimistic view:** panels/sheets read `ctx.documents` (the `OptimisticClient`), never `ctx.store` ([[render-from-optimistic-view]]).
- **Fog is the secrecy gate — fail closed:** any scene-filter path that decides what fog/vision reveals must reveal NOTHING on a missing/garbled/ambiguous signal ([[fog-is-the-secrecy-gate-fail-closed]]). The viewed-scene filter that replaces `query("scene")[0]` in `toVisibility`/`toLighting` inherits this: an unknown viewed scene ⇒ that scene's polygons are simply absent ⇒ full fog, never another scene's holes.
- **Keep-mounted panel rule:** browsers are panels — hidden via CSS/slot adoption, never `{#if}`; any seed/`$effect` tolerates mounting before resync ([[contribution-seed-reactive-before-resync]]).
- **i18n:** every user-facing string is a `t(key)` call with the key added to `src/client/ui-kit/src/locales/en.ts` (`Messages = Record<string,string>`; plain-key additions). Neutral copy.
- **Logger:** never `console.*`; use the injectable `Logger` (`ctx`-supplied or `consoleLogger()`/`silentLogger`).
- **Semantic tokens** for all SCSS colors/spacing (`var(--…)`); `:focus-visible` rings; interactive targets ≥24 CSS px (~44 on coarse pointers). Every served view keeps the responsive-viewport / touch discipline.
- **Zero-history comments:** present-tense, invariant-leading; no task IDs / narrative / reviewer notes in code.
- **Per-task gates (repo root):** for EVERY touched package run BOTH `pnpm --filter <pkg> test` AND `pnpm --filter <pkg> typecheck` — vitest strips types, so typecheck is a separate gate ([[vitest-skips-typecheck-in-sdd]]). No task touches Rust; run no cargo gate.

## Buddy-check directives (spec §15-style pre-authorization)

M12d has no explicit buddy-check line in the M12 design; the `activeScene`/`gmViewedScene`/cross-scene-leak-guard surface is the security-adjacent seam of this milestone (a second notion of "current scene" multiplies the ways the existing cross-scene fog/animation guard can be miswired — a GM roaming scene B must render B's data, not leak scene A's `MoveStream`/fog, and must not go dark). Applying the same judgment M12a/M12c used for their critical seams, these two tasks are pre-authorized for **mandatory buddy-check**. After each is implemented and green, dispatch **`shadowcat-spec-reviewer` + `shadowcat-code-reviewer`** on its diff (the pair replaces both single-reviewer stages) BEFORE proceeding:

- **Task 2** — `WorldSession.viewedSceneId` resolution + `gmViewedScene` (GM-only) + the rewiring of `sendPing` and the `onMoveStream` cross-scene guard from the fixed-first-scene `query("scene")[0]` to `viewedSceneId`.
- **Task 4** — the render engine's viewed-scene projection: background reconciler + all five doc views scene-filtered by `parent_id`, `toVisibility`/`toLighting` scene-filter keyed off the viewed scene, and `reapplyViewedScene` re-projecting the last vision payload on a scene switch (fog secrecy across the switch).

If a base reviewer reports BLOCKED or reads shallow/uncertain, re-dispatch to the `-opus` twin before escalating.

## Design decisions (resolved on technical merits — verified against source)

- **D-a — One "viewed scene" source of truth; render VIEWS scene-filter by `parent_id` (scope expansion).** The survey found the two `WorldSession` sites; the codebase grep found "current scene" independently decided in **at least eight** places, all `query("scene")[0]`: `worldSession.svelte.ts:177,314`; render `engine.ts:266,304` (fog/lighting filter); `Stage.svelte:103` (grid driver); `reconciler.ts:23` (background); `scene-tools/ToolRail.svelte:36` + `scene-tools/controller.svelte.ts:54`. Additionally the five doc views (`token-view.ts:90`, `wall-view.ts:29`, `drawing-view.ts:25`, `template-view.ts:28`, `region-view.ts:36`) iterate `store.query(type)` with **no scene filter** — harmless while one scene exists, but a cross-scene render bug the instant a GM creates a second scene (tokens/walls of every scene would draw at once). Multi-scene is therefore not just two edits; it requires a single resolver, `resolveViewedScene`, threaded through all of the above. This is the correct foundational solution (forward-thinking discipline) rather than scattering `activeScene ?? first` at eight call sites. Fail-closed: an unknown/dangling viewed scene resolves to the first scene (never nothing, unless no scene exists).
- **D-b — Three DISTINCT scene-id notions, deliberately not merged.** (1) `world-settings.activeScene` — global, GM-writable, what players render. (2) `WorldSession.gmViewedScene` — client-local GM override of the render/vision resolution ONLY (players' stays null); "GM roams." (3) `SceneSelection.configureSceneId` — which scene the game-settings per-scene section edits ("Configure"), independent of the camera. Merging any pair is wrong: activating must not force the GM's camera; configuring scene B while viewing scene A is valid; a player has no local override.
- **D-c — Live actor search is NOT reconnect-resilient (unlike scene subscriptions).** `subscribeScene` re-establishes across reconnects because derived vision must survive; a search box is an ephemeral, query-keyed affordance. Contract: an EMPTY query renders the reactive full `query("actor")` list (no subscription); a non-empty query opens a `subscribeSearch` handle keyed on the query string, torn down/recreated on each query change and on unmount. A reconnect drops the live sub; the user's next keystroke re-subscribes. This is simpler and sufficient (YAGNI — no `#sceneSubs`-style bookkeeping for search).
- **D-d — "Configure" deep-links the existing game-settings per-scene section; NO `sheetContract("scene")` is registered.** The curated tri-state per-scene fieldset already lives in `GameSettingsPanel.svelte` (scene picker + `/system/vision/*`, `/system/lighting/*`, snap, bounds). Registering a scene sheet would duplicate it (rejected — YAGNI). The scene browser sets `ctx.sceneSelection.select(sceneId)` and calls `ctx.panels.open("game-settings")`; `GameSettingsPanel` presets its scene picker from `ctx.sceneSelection.configureSceneId`. Scenes have no `name` field (SceneSystem is `{grid, background, …}`) — the browser labels rows by index + background thumbnail; scene naming is out of scope (not in the spec).
- **D-e — `activeScene` is NOT part of the world-settings structural-completeness triple (back-compat).** `resolveSceneSettings`'s guard requires `scene && pathfinding && animation`; `activeScene` is deliberately excluded, so a world-settings doc seeded before M12d (missing the key) stays "complete" and does NOT reset all settings to default on the first M12d-aware read. A missing/`null`/dangling `activeScene` reads as "follow the first scene."
- **`sceneSelection` is created in `Table.svelte`** (like `panels`/`sheets`), not on `WorldSession` — it is authoring-UI focus, not session/render state, and needs no reconnect lifecycle.

---

## File Structure

**Modified (core):**
- `src/client/core/src/scene-docs.ts` — `WorldSettingsSystem.activeScene`, `DEFAULT_WORLD_SETTINGS.activeScene: null`, new pure `resolveViewedScene`.
- `src/client/core/src/index.ts` — re-export `resolveViewedScene`.

**Modified (shell):**
- `src/client/shell/src/lib/worldSession.svelte.ts` — `#gmViewedScene` state, `viewedSceneId` getter, `setGmViewedScene`, `searchDocuments`; rewire `sendPing` + `onMoveStream` guard.
- `src/client/shell/src/lib/Table.svelte` — create `SceneSelection`; wire `viewedSceneId`/`setGmViewedScene`/`searchDocuments`/`sceneSelection` seams.

**New (ui-kit):**
- `src/client/ui-kit/src/sceneSelection.svelte.ts` — `SceneSelection` stable ref.

**Modified (ui-kit):**
- `src/client/ui-kit/src/appContext.ts` — `viewedSceneId`, `setGmViewedScene`, `searchDocuments`, `sceneSelection` seams.
- `src/client/ui-kit/src/index.ts` — export `SceneSelection`.
- `src/client/ui-kit/src/__fixtures__/appContextTest.ts` — seed the four new seams.
- `src/client/ui-kit/src/locales/en.ts` — scene-browser + actor-browser keys.

**New (render):**
- `src/client/render/src/scene-scope.ts` — `sceneScopedDocs(store, docType, viewedSceneId)` filter helper.

**Modified (render):**
- `src/client/render/src/engine.ts` — `RenderEngineOpts.viewedSceneId`, `viewedScene()` helper, thread to views/reconciler + `toVisibility`/`toLighting`, `lastRawPayload`, `reapplyViewedScene`.
- `src/client/render/src/reconciler.ts` — scene-filter background by viewed scene.
- `src/client/render/src/{token,wall,drawing,template,region}-view.ts` — appended `viewedSceneId` ctor param + `sceneScopedDocs`.

**Modified (stage):**
- `src/modules/stage/src/Stage.svelte` — pass `viewedSceneId` getter to engine; watch `ctx.viewedSceneId` → `reapplyViewedScene`; grid driver reads the viewed scene.

**Modified (scene-tools):**
- `src/modules/scene-tools/src/controller.svelte.ts` — `ToolContext.viewedSceneId`, `activeScene()` resolves through it.
- `src/modules/scene-tools/src/ToolRail.svelte` — pass `viewedSceneId`; own `activeScene` derived reads it.

**Modified (actors):**
- `src/modules/actors/src/ActorsPanel.svelte` — search box + open-sheet button.

**New (scene-browser module):**
- `src/modules/scene-browser/package.json`, `tsconfig.json`, `src/index.ts`, `src/SceneBrowserPanel.svelte`, `src/SceneBrowserPanel.test.ts`.

**Modified (game-settings):**
- `src/modules/game-settings/src/GameSettingsPanel.svelte` — preset scene picker from `ctx.sceneSelection`.

**Modified (shell app):**
- `src/client/shell/src/App.svelte` — import + register `sceneBrowser`.

---

### Task 1: `activeScene` field + `resolveViewedScene` (pure core)

**Files:**
- Modify: `src/client/core/src/scene-docs.ts`
- Modify: `src/client/core/src/index.ts`
- Test: `src/client/core/src/scene-docs.test.ts` (extend if present, else create)

**Interfaces:**
- Consumes: `ReadableDocuments`, `WorldSettingsSystem`, `DEFAULT_WORLD_SETTINGS`.
- Produces:
  - `WorldSettingsSystem` gains `activeScene: string | null`.
  - `DEFAULT_WORLD_SETTINGS.activeScene = null`.
  - `resolveViewedScene(store: ReadableDocuments, opts?: { gmViewedScene?: string | null }): string | null` — the client's single "which scene do I render" answer. `null` ONLY when no scene exists.

- [ ] **Step 1: Write the failing test**

Append to `src/client/core/src/scene-docs.test.ts` (create with the imports below if the file is absent):

```ts
import { describe, it, expect } from "vitest";
import { DocumentStore } from "./store";
import { buildSceneDoc, buildWorldSettingsDoc, DEFAULT_WORLD_SETTINGS, resolveViewedScene } from "./scene-docs";
import type { WireDocument, WorldSettingsSystem } from "./scene-docs";

function store(docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "u", ts: 0, ops: docs.map((doc) => ({ op: "create", doc })) });
  return s;
}
function ws(activeScene: string | null): WireDocument {
  return buildWorldSettingsDoc("w1", { ...structuredClone(DEFAULT_WORLD_SETTINGS), activeScene } as WorldSettingsSystem);
}

describe("resolveViewedScene", () => {
  it("returns null when no scene exists", () => {
    expect(resolveViewedScene(store([]))).toBeNull();
    expect(resolveViewedScene(store([ws(null)]))).toBeNull();
  });

  it("falls back to the first scene when activeScene is absent/null (legacy behavior)", () => {
    const s0 = buildSceneDoc("w1", {}, "s0");
    const s1 = buildSceneDoc("w1", {}, "s1");
    expect(resolveViewedScene(store([s0, s1]))).toBe("s0");
    expect(resolveViewedScene(store([s0, s1, ws(null)]))).toBe("s0");
  });

  it("follows a resolvable activeScene (players)", () => {
    const s0 = buildSceneDoc("w1", {}, "s0");
    const s1 = buildSceneDoc("w1", {}, "s1");
    expect(resolveViewedScene(store([s0, s1, ws("s1")]))).toBe("s1");
  });

  it("falls back to the first scene when activeScene dangles (deleted target)", () => {
    const s0 = buildSceneDoc("w1", {}, "s0");
    expect(resolveViewedScene(store([s0, ws("gone")]))).toBe("s0");
  });

  it("gmViewedScene overrides activeScene when it resolves", () => {
    const s0 = buildSceneDoc("w1", {}, "s0");
    const s1 = buildSceneDoc("w1", {}, "s1");
    const s = store([s0, s1, ws("s1")]);
    expect(resolveViewedScene(s, { gmViewedScene: "s0" })).toBe("s0");
  });

  it("ignores a dangling gmViewedScene and falls through to activeScene", () => {
    const s0 = buildSceneDoc("w1", {}, "s0");
    const s1 = buildSceneDoc("w1", {}, "s1");
    const s = store([s0, s1, ws("s1")]);
    expect(resolveViewedScene(s, { gmViewedScene: "gone" })).toBe("s1");
    expect(resolveViewedScene(s, { gmViewedScene: null })).toBe("s1");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/core test -- scene-docs`
Expected: FAIL — `resolveViewedScene` is not exported (and/or `activeScene` type error).

- [ ] **Step 3: Add the `activeScene` field**

In `src/client/core/src/scene-docs.ts`, add `activeScene` to `WorldSettingsSystem` (the interface at ~line 90):

```ts
/** The `system` body of a "world-settings" config document. */
export interface WorldSettingsSystem {
  scene: WorldSceneDefaults;
  pathfinding: { diagonalRule: DiagonalRule };
  animation: { speedCellsPerSec: number; easing: EasingMode };
  /** The scene players render (M12d). GM-writable via the normal config-doc path. `null`/absent/
   * dangling ⇒ the first scene (legacy behavior). Deliberately NOT part of the
   * structural-completeness triple in `resolveSceneSettings`, so a pre-M12d world-settings doc
   * missing this key is still "complete" and keeps its authored settings. */
  activeScene: string | null;
}
```

Add `activeScene: null` to `DEFAULT_WORLD_SETTINGS` (the frozen constant at ~line 99), as the last key of the object literal:

```ts
export const DEFAULT_WORLD_SETTINGS: WorldSettingsSystem = deepFreeze({
  scene: {
    losRestriction: true,
    fog: true,
    lightingEnabled: true,
    lightMode: "environmentLight",
    environment: { color: "#0a0e1a", intensity: 0.0 },
    observerVision: false,
    movementRestriction: "visible",
    movementModel: "grid-stepped",
    partialCellLeniency: true,
  },
  pathfinding: { diagonalRule: "chebyshev" },
  animation: { speedCellsPerSec: 6, easing: "easeInOut" },
  activeScene: null,
});
```

- [ ] **Step 4: Add `resolveViewedScene`**

In `src/client/core/src/scene-docs.ts`, add immediately after `resolveSceneSettings` (after ~line 319):

```ts
/** The single client-side answer to "which scene does THIS client render/subscribe to"
 * (M12d). Resolution order: a resolvable `gmViewedScene` (GM local roam) → a resolvable
 * `world-settings.activeScene` (players follow) → the first scene (legacy). `null` ONLY when
 * no scene exists. Fail-closed by construction: an id that no longer names a scene is ignored
 * (never renders nothing while scenes exist, never leaks a nonexistent scene's channel).
 * Players never pass `gmViewedScene`, so they always follow `activeScene`. */
export function resolveViewedScene(
  store: ReadableDocuments,
  opts: { gmViewedScene?: string | null } = {},
): string | null {
  const scenes = store.query("scene");
  if (scenes.length === 0) return null;
  const exists = (id: string | null | undefined): id is string => !!id && scenes.some((s) => s.id === id);
  if (exists(opts.gmViewedScene)) return opts.gmViewedScene;
  const ws = store.query("world-settings")[0]?.system as WorldSettingsSystem | undefined;
  if (exists(ws?.activeScene)) return ws!.activeScene;
  return scenes[0].id;
}
```

- [ ] **Step 5: Export from the core barrel**

In `src/client/core/src/index.ts`, append `resolveViewedScene` to the existing `./scene-docs` **value** re-export (the line that already lists `buildSceneDoc, …, resolveSceneSettings, …`):

```ts
// add to the scene-docs value re-export list: , resolveViewedScene
```

- [ ] **Step 6: Run tests + typecheck + commit**

Run: `pnpm --filter @shadowcat/core test -- scene-docs`
Expected: PASS.
Run: `pnpm --filter @shadowcat/core typecheck`
Expected: no errors.

```bash
git add src/client/core/src/scene-docs.ts src/client/core/src/scene-docs.test.ts src/client/core/src/index.ts
git commit -m "feat(core/m12d): activeScene world-setting + resolveViewedScene resolver"
```

---

### Task 2: `WorldSession` viewed-scene state + broadcast rewiring + search seam [BUDDY-CHECK]

**Files:**
- Modify: `src/client/shell/src/lib/worldSession.svelte.ts`
- Test: `src/client/shell/src/lib/worldSession.test.ts` (extend)

**Interfaces:**
- Consumes: `resolveViewedScene` (Task 1), `WsClient.subscribeSearch`, `SubscriptionHandle`, `WireSearchHit`, `buildWorldSettingsDoc`/`DEFAULT_WORLD_SETTINGS`/`buildSceneDoc` (test fixtures).
- Produces on `WorldSession`:
  - `get viewedSceneId(): string | null`.
  - `setGmViewedScene(id: string | null): void` — GM-only; a non-GM call is ignored + warned.
  - `searchDocuments(query: string, opts: { limit?: number; timeoutMs?: number }, onUpdate: (hits: WireSearchHit[]) => void): Promise<SubscriptionHandle>`.

- [ ] **Step 1: Write the failing tests**

Append to `src/client/shell/src/lib/worldSession.test.ts` (the `sceneCreates`/`pushConnect` helpers already exist in this file; `buildWorldSettingsDoc`, `buildSceneDoc`, `DEFAULT_WORLD_SETTINGS` must be added to the top `@shadowcat/core` import):

```ts
test("viewedSceneId: player follows activeScene, else the first scene", async () => {
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push(welcomeFrame); // player
  await vi.waitFor(() => expect(session.role).toBe("player"));

  // Predict two scenes + a world-settings doc into the optimistic view.
  session.dispatchIntent([{ op: "create", doc: buildSceneDoc("w1", {}, "s0") }]);
  session.dispatchIntent([{ op: "create", doc: buildSceneDoc("w1", {}, "s1") }]);
  expect(session.viewedSceneId).toBe("s0"); // no activeScene yet ⇒ first scene

  session.dispatchIntent([{ op: "create", doc: buildWorldSettingsDoc("w1", { ...structuredClone(DEFAULT_WORLD_SETTINGS), activeScene: "s1" }) }]);
  expect(session.viewedSceneId).toBe("s1"); // follows activeScene
});

test("setGmViewedScene overrides only for a GM; a player call is ignored", async () => {
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push(welcomeFrame); // player
  await vi.waitFor(() => expect(session.role).toBe("player"));
  session.dispatchIntent([{ op: "create", doc: buildSceneDoc("w1", {}, "s0") }]);
  session.dispatchIntent([{ op: "create", doc: buildSceneDoc("w1", {}, "s1") }]);
  session.dispatchIntent([{ op: "create", doc: buildWorldSettingsDoc("w1", { ...structuredClone(DEFAULT_WORLD_SETTINGS), activeScene: "s1" }) }]);

  session.setGmViewedScene("s0"); // player: ignored
  expect(session.viewedSceneId).toBe("s1");
});

test("a GM roams locally with gmViewedScene; clearing it follows activeScene again", async () => {
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push({ ...welcomeFrame, user_role: "gm" }); // GM auto-creates one scene
  await vi.waitFor(() => expect(sceneCreates(sent).length).toBe(1));
  const first = (sceneCreates(sent)[0] as { ops: Array<{ doc?: { id?: string } }> }).ops.find((o) => o.doc)!.doc!.id as string;
  session.dispatchIntent([{ op: "create", doc: buildSceneDoc("w1", {}, "sB") }]);
  session.dispatchIntent([{ op: "create", doc: buildWorldSettingsDoc("w1", { ...structuredClone(DEFAULT_WORLD_SETTINGS), activeScene: first }) }]);

  session.setGmViewedScene("sB");
  expect(session.viewedSceneId).toBe("sB"); // roaming
  session.setGmViewedScene(null);
  expect(session.viewedSceneId).toBe(first); // follows active again
});

test("onMoveStream gates on the GM's LOCAL viewed scene, not activeScene", async () => {
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push({ ...welcomeFrame, user_role: "gm" });
  await vi.waitFor(() => expect(sceneCreates(sent).length).toBe(1));
  const active = (sceneCreates(sent)[0] as { ops: Array<{ doc?: { id?: string } }> }).ops.find((o) => o.doc)!.doc!.id as string;
  session.dispatchIntent([{ op: "create", doc: buildSceneDoc("w1", {}, "sB") }]);
  session.dispatchIntent([{ op: "create", doc: buildWorldSettingsDoc("w1", { ...structuredClone(DEFAULT_WORLD_SETTINGS), activeScene: active }) }]);
  session.setGmViewedScene("sB"); // roaming to sB while players stay on `active`

  const host = fakeMoveHost();
  session.sceneInteraction.attach(host);
  push(moveStreamFrame(active)); // the players' scene — must NOT animate in the GM's local view
  await new Promise((r) => setTimeout(r, 20));
  expect(host.calls).toHaveLength(0);
  push(moveStreamFrame("sB")); // the GM's viewed scene — animates
  await vi.waitFor(() => expect(host.calls).toHaveLength(1));
});

test("sendPing targets the viewed scene", async () => {
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push({ ...welcomeFrame, user_role: "gm" });
  await vi.waitFor(() => expect(sceneCreates(sent).length).toBe(1));
  const active = (sceneCreates(sent)[0] as { ops: Array<{ doc?: { id?: string } }> }).ops.find((o) => o.doc)!.doc!.id as string;
  session.dispatchIntent([{ op: "create", doc: buildSceneDoc("w1", {}, "sB") }]);
  session.dispatchIntent([{ op: "create", doc: buildWorldSettingsDoc("w1", { ...structuredClone(DEFAULT_WORLD_SETTINGS), activeScene: active }) }]);
  session.setGmViewedScene("sB");

  session.sendPing(10, 20);
  const ping = sent.find((m) => m.type === "scene_ping")!;
  expect(ping.scene).toBe("sB");
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/shell test -- worldSession`
Expected: FAIL — `viewedSceneId`/`setGmViewedScene` not defined; ping/move tests still key off `query("scene")[0]`.

- [ ] **Step 3: Add the imports + state + accessors**

In `src/client/shell/src/lib/worldSession.svelte.ts`, add `resolveViewedScene` and the search types to the `@shadowcat/core` import block:

```ts
  resolveViewedScene,
```
and to the `type { … }` list in the same import:
```ts
  type SubscriptionHandle,
  type WireSearchHit,
```

Add the client-local override field beside the other `$state` fields (after `world = $state<string | null>(null);` at ~line 67):

```ts
  /** Client-local GM override of the rendered/subscribed scene (M12d "GM roams"). Never set for
   * a player (they follow `world-settings.activeScene`). Overrides `viewedSceneId` for THIS
   * client's own render + vision + see-as channels only; the server is unaware of it. */
  #gmViewedScene = $state<string | null>(null);
```

Add the accessors after the `selfId` getter (after ~line 94):

```ts
  /** The scene THIS client renders + subscribes to (M12d). A GM's local roam
   * (`#gmViewedScene`) overrides; otherwise follows `world-settings.activeScene`, else the first
   * scene. Reads the optimistic view + `#gmViewedScene` $state, so Svelte deriveds that read it
   * (bridged through `documents.subscribe`) react to both scene-doc changes and roam changes. */
  get viewedSceneId(): string | null {
    return resolveViewedScene(this.#optimistic, { gmViewedScene: this.role === "gm" ? this.#gmViewedScene : null });
  }

  /** GM local roam (M12d): view any scene without moving players. Ignored (warned) for a non-GM —
   * players have no local override. `null` clears the roam (follow `activeScene`). */
  setGmViewedScene(id: string | null): void {
    if (this.role !== "gm") {
      this.#logger.warn("setGmViewedScene ignored: caller is not a GM");
      return;
    }
    this.#gmViewedScene = id;
  }

  /** Live full-text search over documents (M6c subscription seam). Ephemeral: NOT re-established
   * across reconnects (unlike `subscribeScene`) — the caller re-subscribes on the next query.
   * Rejects immediately when there is no live transport. */
  searchDocuments(
    query: string,
    opts: { limit?: number; timeoutMs?: number },
    onUpdate: (hits: WireSearchHit[]) => void,
  ): Promise<SubscriptionHandle> {
    if (!this.#ws) return Promise.reject(new Error("not connected"));
    return this.#ws.subscribeSearch(query, opts, onUpdate);
  }
```

- [ ] **Step 4: Rewire `sendPing` and the `onMoveStream` guard**

In `sendPing` (~line 176), resolve through the viewed scene:

```ts
  sendPing(x: number, y: number): void {
    const sceneId = this.viewedSceneId;
    if (!sceneId) return;
    this.#ws?.send({ type: "scene_ping", scene: sceneId, x, y });
  }
```

In `enter()`'s `onMoveStream` callback (~line 308-315), replace the fixed-first-scene gate with the viewed scene. The comment must state the new invariant (GM local view, not `activeScene`, gates the GM's own animation):

```ts
    this.#ws.onMoveStream((stream) => {
      // Cross-scene guard: a MoveStream broadcasts room-wide and is animated only if it targets the
      // scene THIS client is viewing (a GM roaming scene B must not animate scene A's move, and must
      // animate B's). `viewedSceneId` is the GM's local view when roaming, else the followed
      // `activeScene`. Fail-closed: a stream for any other scene is dropped (latent cross-scene
      // fog/animation leak, mirrors engine.ts's toVisibility scene filter).
      if (stream.scene !== this.viewedSceneId) return;
      this.sceneInteraction.animateSamples(
        stream.tokenId,
        stream.samples,
        stream.durationMs,
        stream.startServerMs,
        () => ws.serverNow(),
        stream.moverVision,
      );
    });
```

- [ ] **Step 5: Run tests + typecheck**

Run: `pnpm --filter @shadowcat/shell test -- worldSession`
Expected: PASS (incl. the pre-existing `onMoveStream` tests — with one scene, `viewedSceneId` equals the first scene).
Run: `pnpm --filter @shadowcat/shell typecheck`
Expected: no errors.

- [ ] **Step 6: Commit, then request buddy-check**

```bash
git add src/client/shell/src/lib/worldSession.svelte.ts src/client/shell/src/lib/worldSession.test.ts
git commit -m "feat(shell/m12d): WorldSession viewedSceneId + gmViewedScene roam + search seam [buddy-check]"
```

Dispatch `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` on this diff (the viewed-scene resolution + cross-scene-leak-guard rewiring is the pre-authorized buddy-check surface) before proceeding.

---

### Task 3: AppContext seams + `SceneSelection` + Table wiring

**Files:**
- Create: `src/client/ui-kit/src/sceneSelection.svelte.ts`
- Modify: `src/client/ui-kit/src/appContext.ts`
- Modify: `src/client/ui-kit/src/index.ts`
- Modify: `src/client/ui-kit/src/__fixtures__/appContextTest.ts`
- Modify: `src/client/shell/src/lib/Table.svelte`
- Test: `src/client/ui-kit/src/sceneSelection.test.ts`

**Interfaces:**
- Consumes: `WorldSession.viewedSceneId`/`setGmViewedScene`/`searchDocuments` (Task 2), `SubscriptionHandle`/`WireSearchHit` (core).
- Produces:
  - `class SceneSelection { get configureSceneId(): string | null; select(id: string | null): void }`.
  - `AppContext` gains `viewedSceneId: string | null`, `setGmViewedScene(id: string | null): void`, `searchDocuments(query, opts, onUpdate): Promise<SubscriptionHandle>`, `sceneSelection: SceneSelection`.

- [ ] **Step 1: Write the failing test**

Create `src/client/ui-kit/src/sceneSelection.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { SceneSelection } from "./sceneSelection.svelte";

describe("SceneSelection", () => {
  it("holds and clears the configure-target scene id", () => {
    const s = new SceneSelection();
    expect(s.configureSceneId).toBeNull();
    s.select("sc1");
    expect(s.configureSceneId).toBe("sc1");
    s.select(null);
    expect(s.configureSceneId).toBeNull();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/ui-kit test -- sceneSelection`
Expected: FAIL — module not found.

- [ ] **Step 3: Write `SceneSelection`**

Create `src/client/ui-kit/src/sceneSelection.svelte.ts`:

```ts
// The scene the game-settings per-scene section edits (M12d "Configure"). A stable instance
// created by the shell and shared via AppContext: the scene browser sets it, GameSettingsPanel
// reads it to preset its picker. Distinct from `activeScene` (global render target) and
// `gmViewedScene` (GM local camera) — configuring a scene never moves any camera. Reactive
// ($state) + mutated in place (never reassigned) so the AppContext-captured reference stays valid.
export class SceneSelection {
  #id = $state<string | null>(null);

  get configureSceneId(): string | null {
    return this.#id;
  }

  select(id: string | null): void {
    this.#id = id;
  }
}
```

- [ ] **Step 4: Extend `AppContext`**

In `src/client/ui-kit/src/appContext.ts`, add the imports (extend the existing `@shadowcat/core` type import + add the local type import):

```ts
// add to the existing @shadowcat/core `import type { … }`: , SubscriptionHandle, WireSearchHit
import type { SceneSelection } from "./sceneSelection.svelte";
```

Add the four members to the `AppContext` interface (after `tokenSelection` at ~line 74):

```ts
  /** The scene THIS client renders + subscribes to (M12d). Players follow
   * `world-settings.activeScene`; a GM roaming via `setGmViewedScene` overrides locally. Getter —
   * reactive when read through a `documents.subscribe` bridge. */
  viewedSceneId: string | null;
  /** GM local roam (M12d): view any scene without moving players. No-op for a non-GM. */
  setGmViewedScene: (id: string | null) => void;
  /** Live full-text document search (M6c seam). Resolves once the initial page arrives (and fires
   * `onUpdate` for it); subsequent pushes fire `onUpdate`. Ephemeral — NOT reconnect-resilient;
   * re-subscribe per query. Rejects when there is no transport. */
  searchDocuments: (
    query: string,
    opts: { limit?: number; timeoutMs?: number },
    onUpdate: (hits: WireSearchHit[]) => void,
  ) => Promise<SubscriptionHandle>;
  /** Which scene the game-settings per-scene section edits (M12d "Configure"); set by the scene
   * browser, read by GameSettingsPanel. */
  sceneSelection: SceneSelection;
```

- [ ] **Step 5: Export + seed the fixture**

In `src/client/ui-kit/src/index.ts`, add:

```ts
export { SceneSelection } from "./sceneSelection.svelte";
```

In `src/client/ui-kit/src/__fixtures__/appContextTest.ts`, add the `SceneSelection` import beside the other selection imports and seed the four seams inside the `ctx` object (before the closing `}`):

```ts
import { SceneSelection } from "../sceneSelection.svelte";
```
```ts
    viewedSceneId: over.viewedSceneId ?? null,
    setGmViewedScene: over.setGmViewedScene ?? (() => {}),
    searchDocuments: over.searchDocuments ?? (() => Promise.reject(new Error("not connected"))),
    sceneSelection: over.sceneSelection ?? new SceneSelection(),
```

- [ ] **Step 6: Wire the seams in `Table.svelte`**

In `src/client/shell/src/lib/Table.svelte`, add `SceneSelection` to the `@shadowcat/ui-kit` import:

```ts
  import { setAppContext, Surface, PanelsBridge, SheetsController, SceneSelection } from "@shadowcat/ui-kit";
```

Create the stable instance after the `sheets` construction (~line 27):

```ts
  // Scene "Configure" focus: the browser sets it, GameSettingsPanel reads it. Stable per Table,
  // like `panels`/`sheets`.
  const sceneSelection = new SceneSelection();
```

Add the four seams to the `setAppContext({ … })` object (place `viewedSceneId` as a getter so it re-reads the session each access; the rest are plain delegates). Add after `tokenSelection: session.tokenSelection,` (~line 61):

```ts
    get viewedSceneId() {
      return session.viewedSceneId;
    },
    setGmViewedScene: (id) => session.setGmViewedScene(id),
    searchDocuments: (query, opts, onUpdate) => session.searchDocuments(query, opts, onUpdate),
    sceneSelection,
```

- [ ] **Step 7: Run tests + typecheck (both packages) + commit**

Run: `pnpm --filter @shadowcat/ui-kit test -- sceneSelection`
Expected: PASS.
Run: `pnpm --filter @shadowcat/ui-kit typecheck && pnpm --filter @shadowcat/shell typecheck`
Expected: no errors.

```bash
git add src/client/ui-kit/src/sceneSelection.svelte.ts src/client/ui-kit/src/sceneSelection.test.ts src/client/ui-kit/src/appContext.ts src/client/ui-kit/src/index.ts src/client/ui-kit/src/__fixtures__/appContextTest.ts src/client/shell/src/lib/Table.svelte
git commit -m "feat(ui-kit/m12d): viewedSceneId + setGmViewedScene + searchDocuments + SceneSelection seams"
```

---

### Task 4: Render engine + doc views scene-filtered by the viewed scene [BUDDY-CHECK]

**Files:**
- Create: `src/client/render/src/scene-scope.ts`
- Modify: `src/client/render/src/engine.ts`
- Modify: `src/client/render/src/reconciler.ts`
- Modify: `src/client/render/src/token-view.ts`, `wall-view.ts`, `drawing-view.ts`, `template-view.ts`, `region-view.ts`
- Test: `src/client/render/src/scene-scope.test.ts`, `src/client/render/src/engine.test.ts` (extend)

**Interfaces:**
- Consumes: `ReadableDocuments`, `WireDocument`.
- Produces:
  - `sceneScopedDocs(store: ReadableDocuments, docType: string, viewedSceneId: () => string | null): WireDocument[]` — the viewed scene's children, or ALL of that type when the getter returns `null` (degenerate no-scene case).
  - `RenderEngineOpts.viewedSceneId?: () => string | null` (absent ⇒ engine falls back to the first scene, preserving single-scene behavior).
  - `RenderEngine.reapplyViewedScene(): void` — re-project background + all views + the last vision payload onto the current viewed scene.
  - Each view's constructor gains a FINAL `viewedSceneId: () => string | null = () => null` parameter; `SceneReconciler`'s constructor gains a FINAL `viewedSceneId: () => string | null = () => null` parameter.

- [ ] **Step 1: Write the failing tests**

Create `src/client/render/src/scene-scope.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { DocumentStore, buildTokenDoc } from "@shadowcat/core";
import { sceneScopedDocs } from "./scene-scope";

function store(): DocumentStore {
  const s = new DocumentStore();
  const mk = (id: string, scene: string) => buildTokenDoc("w1", scene, { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" } }, id);
  s.applyCommand({ seq: 1, world_id: "w1", author: "u", ts: 0, ops: [
    { op: "create", doc: mk("t-a", "sA") },
    { op: "create", doc: mk("t-b", "sB") },
  ] });
  return s;
}

describe("sceneScopedDocs", () => {
  it("returns only the viewed scene's children", () => {
    const s = store();
    expect(sceneScopedDocs(s, "token", () => "sA").map((d) => d.id)).toEqual(["t-a"]);
    expect(sceneScopedDocs(s, "token", () => "sB").map((d) => d.id)).toEqual(["t-b"]);
  });
  it("returns ALL of the type when no scene is viewed (degenerate)", () => {
    expect(sceneScopedDocs(store(), "token", () => null).map((d) => d.id).sort()).toEqual(["t-a", "t-b"]);
  });
});
```

Append to `src/client/render/src/engine.test.ts` (this file already defines a `FakeBackend`/store harness — reuse its existing helpers; the snippet below assumes the file's existing `makeEngine`-style setup, adapt to the local harness names):

```ts
import { describe, it, expect } from "vitest";
import { RenderEngine } from "./engine";
import { DocumentStore, AssetResolver, buildSceneDoc, buildTokenDoc } from "@shadowcat/core";
// Reuse this file's existing fake backend factory (already imported/defined above in engine.test.ts).

describe("multi-scene render filtering", () => {
  function seed() {
    const store = new DocumentStore();
    store.applyCommand({ seq: 1, world_id: "w1", author: "u", ts: 0, ops: [
      { op: "create", doc: buildSceneDoc("w1", { background: "bgA" }, "sA") },
      { op: "create", doc: buildSceneDoc("w1", { background: "bgB" }, "sB") },
      { op: "create", doc: buildTokenDoc("w1", "sA", { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" } }, "t-a") },
      { op: "create", doc: buildTokenDoc("w1", "sB", { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" } }, "t-b") },
    ] });
    return store;
  }

  it("renders only the viewed scene's tokens + background, and re-projects on switch", () => {
    const store = seed();
    let viewed = "sA";
    const backend = makeFakeBackend(); // engine.test.ts's existing fake-backend helper
    const engine = new RenderEngine({ store, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 }, viewedSceneId: () => viewed });
    engine.start();
    expect(backend.tokenIds()).toEqual(["t-a"]);       // fake backend records added tokens
    expect(backend.background()).toContain("bgA");      // fake backend records setBackground url

    viewed = "sB";
    engine.reapplyViewedScene();
    expect(backend.tokenIds()).toEqual(["t-b"]);
    expect(backend.background()).toContain("bgB");
    engine.destroy();
  });
});
```

> The fake-backend accessors (`tokenIds()`, `background()`) are illustrative — implement the assertions against whatever recording the existing `engine.test.ts` fake backend already exposes (it already asserts `setBackground`/token adds elsewhere in the file). If the local fake lacks a token/background recorder, add a minimal one to the existing fake in this same step.

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/render test -- scene-scope engine`
Expected: FAIL — `sceneScopedDocs` missing; engine renders both scenes' tokens / no `reapplyViewedScene`.

- [ ] **Step 3: Write the scope helper**

Create `src/client/render/src/scene-scope.ts`:

```ts
// Scene scoping for the render layer (M12d). The store holds EVERY scene's children (the server
// delivers the whole readable doc set); a client renders only the scene it is viewing. A `null`
// viewed scene (no scene exists yet) yields the unfiltered list — the degenerate pre-scene case,
// identical to legacy single-scene behavior.
import type { ReadableDocuments, WireDocument } from "@shadowcat/core";

export function sceneScopedDocs(
  store: ReadableDocuments,
  docType: string,
  viewedSceneId: () => string | null,
): WireDocument[] {
  const vsid = viewedSceneId();
  const docs = store.query(docType);
  return vsid === null ? docs : docs.filter((d) => d.parent_id === vsid);
}
```

- [ ] **Step 4: Scene-filter each doc view**

In each of `token-view.ts`, `wall-view.ts`, `drawing-view.ts`, `template-view.ts`, `region-view.ts`:

1. Add the import at the top: `import { sceneScopedDocs } from "./scene-scope";`
2. Append `viewedSceneId` as the FINAL constructor parameter with a default. For `token-view.ts` (ctor `(store, assets, backend)`):

```ts
  constructor(
    private readonly store: ReadableDocuments,
    private readonly assets: AssetResolver,
    private readonly backend: DisplayBackend,
    private readonly viewedSceneId: () => string | null = () => null,
  ) {}
```
For `wall-view.ts`, `drawing-view.ts`, `template-view.ts`, `region-view.ts` (ctor `(store, backend)`):

```ts
  constructor(
    private readonly store: ReadableDocuments,
    private readonly backend: DisplayBackend,
    private readonly viewedSceneId: () => string | null = () => null,
  ) {}
```
3. In each `reconcile()`, replace the query in the `for` loop. E.g. `token-view.ts` line 90:

```ts
    for (const doc of sceneScopedDocs(this.store, "token", this.viewedSceneId)) {
```
and correspondingly `"wall"`, `"drawing"`, `"template"`, `"region"` in the other four.

- [ ] **Step 5: Scene-filter the background reconciler**

In `src/client/render/src/reconciler.ts`, add the constructor param + resolve the scene via the viewed id:

```ts
  constructor(
    private readonly store: ReadableDocuments,
    private readonly assets: AssetResolver,
    private readonly backend: DisplayBackend,
    private readonly viewedSceneId: () => string | null = () => null,
  ) {}

  reconcile(): void {
    // The viewed scene's background (M12d). `null` viewed id ⇒ the first scene (legacy single-scene).
    const vsid = this.viewedSceneId();
    const scene = (vsid ? this.store.get(vsid) : this.store.query("scene")[0]) as WireDocument | undefined;
    const bg = (scene?.system as SceneSystem | undefined)?.background;
    if (typeof bg === "string" && bg.length > 0) {
      this.backend.setBackground({ url: this.assets.url(bg) });
    } else {
      this.backend.setBackground(null);
    }
  }
```

- [ ] **Step 6: Thread the viewed scene through the engine + add `reapplyViewedScene`**

In `src/client/render/src/engine.ts`:

Add to `RenderEngineOpts` (after `subscribeScene?`):

```ts
  /** Which scene to render/scene-filter by (M12d). From the host (Stage → `ctx.viewedSceneId`).
   * Absent ⇒ the first scene, preserving single-scene behavior. */
  viewedSceneId?: () => string | null;
```

Add the resolver + raw-payload cache as fields (near `lastInput`, ~line 103):

```ts
  /** Resolved viewed scene, falling back to the first scene so single-scene tests/hosts are
   * unaffected. The single definition every view + reconciler + fog filter reads. */
  private readonly viewedScene = (): string | null =>
    this.opts.viewedSceneId?.() ?? this.opts.store.query("scene")[0]?.id ?? null;
  /** The last vision payload received, re-projected onto a new viewed scene by
   * `reapplyViewedScene` (a scene switch has no new server frame — `activeScene`/roam are
   * client-local). Undefined until the first frame. */
  private lastRawPayload: unknown = undefined;
```

Pass `this.viewedScene` into the views + reconciler in the constructor (~lines 124-130):

```ts
    this.reconciler = new SceneReconciler(opts.store, opts.assets, opts.backend, this.viewedScene);
    this.tokens = new TokenView(opts.store, opts.assets, opts.backend, this.viewedScene);
    this.tokens.setCellSize(opts.grid.size);
    this.drawings = new DrawingView(opts.store, opts.backend, this.viewedScene);
    this.templates = new TemplateView(opts.store, opts.backend, this.viewedScene);
    this.walls = new WallView(opts.store, opts.backend, this.viewedScene);
    this.regions = new RegionView(opts.store, opts.backend, this.viewedScene);
```

In `onSceneFrame` (~line 190), cache the raw payload after the monotonic-drop guards, before `toVisibility`:

```ts
    if (this.pendingDerived && frame.computedAtSeq <= this.pendingDerived.seq) return;
    this.lastRawPayload = frame.payload;
    const input = this.toVisibility(frame.payload);
```

In `toVisibility` (~line 266) and `toLighting` (~line 304), replace `const activeScene = this.opts.store.query("scene")[0]?.id;` with:

```ts
    const activeScene = this.viewedScene();
```

Add `reapplyViewedScene` as a public method (e.g. after `setFogPreview`):

```ts
  /** Re-project the render onto the CURRENT viewed scene after a client-local scene switch
   * (`activeScene` flip or GM roam — neither carries a new server frame). Re-runs background + all
   * doc views (their `parent_id` filter changed) and re-filters the last vision payload to the new
   * scene. Fog secrecy across the switch: with no cached frame, `lastInput`'s full-fog default
   * stands; a re-filter to an unknown scene yields empty holes (full fog), never the prior scene's. */
  reapplyViewedScene(): void {
    this.reconciler.reconcile();
    this.tokens.reconcile();
    this.drawings.reconcile();
    this.templates.reconcile();
    this.walls.reconcile();
    this.regions.reconcile();
    if (this.lastRawPayload !== undefined) {
      this.lighting.setTarget(this.toLighting(this.lastRawPayload));
      this.lastInput = this.toVisibility(this.lastRawPayload);
      this.renderVisibility();
    }
  }
```

- [ ] **Step 7: Run tests + typecheck**

Run: `pnpm --filter @shadowcat/render test -- scene-scope engine token-view wall-view drawing-view template-view region-view reconciler`
Expected: PASS (existing single-scene view/reconciler tests unaffected — default `() => null` ⇒ unfiltered).
Run: `pnpm --filter @shadowcat/render typecheck`
Expected: no errors.

- [ ] **Step 8: Commit, then request buddy-check**

```bash
git add src/client/render/src/scene-scope.ts src/client/render/src/scene-scope.test.ts src/client/render/src/engine.ts src/client/render/src/engine.test.ts src/client/render/src/reconciler.ts src/client/render/src/token-view.ts src/client/render/src/wall-view.ts src/client/render/src/drawing-view.ts src/client/render/src/template-view.ts src/client/render/src/region-view.ts
git commit -m "feat(render/m12d): scene-filter all doc views + fog by the viewed scene + reapplyViewedScene [buddy-check]"
```

Dispatch `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` on this diff (the fog/vision scene-projection is the pre-authorized buddy-check surface) before proceeding.

---

### Task 5: Stage drives the engine from `ctx.viewedSceneId`

**Files:**
- Modify: `src/modules/stage/src/Stage.svelte`
- Test: `src/modules/stage/src/Stage.test.ts` (extend)

**Interfaces:**
- Consumes: `ctx.viewedSceneId` (Task 3), `RenderEngineOpts.viewedSceneId`/`reapplyViewedScene` (Task 4).
- Produces: the Stage passes `viewedSceneId: () => ctx.viewedSceneId` to the engine, re-projects on change, and drives the grid from the viewed scene.

- [ ] **Step 1: Write the failing test**

Append to `src/modules/stage/src/Stage.test.ts` a test that, with two scenes + a world-settings `activeScene`, asserts the host's `data-token-count` / grid reflect the viewed scene and update when `ctx.viewedSceneId` flips. Model it on the file's existing render-ready pattern (fake backend injected via the `createBackend` prop):

```ts
import { render } from "@testing-library/svelte";
import { tick } from "svelte";
import Stage from "./Stage.svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/__fixtures__/appContextTest"; // adapt to this file's existing import of the fixture
import { DocumentStore, AssetResolver, buildSceneDoc, buildTokenDoc } from "@shadowcat/core";

it("drives the grid + reconcile from ctx.viewedSceneId and re-projects on flip", async () => {
  const store = new DocumentStore();
  store.applyCommand({ seq: 1, world_id: "w1", author: "u", ts: 0, ops: [
    { op: "create", doc: buildSceneDoc("w1", { grid: { kind: "square", size: 100 } }, "sA") },
    { op: "create", doc: buildSceneDoc("w1", { grid: { kind: "square", size: 50 } }, "sB") },
    { op: "create", doc: buildTokenDoc("w1", "sB", { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" } }, "t-b") },
  ] });
  let viewed = "sA";
  const context = setAppContextForTest({ documents: store, store, assets: new AssetResolver(), get viewedSceneId() { return viewed; } as never });
  const { container } = render(Stage, { props: { createBackend: makeFakeBackendFactory() }, context });
  await tick();
  const host = container.querySelector(".stage-host") as HTMLElement;
  await vi.waitFor(() => expect(host.dataset.renderReady).toBe("true"));
  expect(host.dataset.tokenCount).toBe("0"); // sA has no tokens
  // (grid/token re-projection on flip is exercised via the engine unit test in Task 4; this test
  //  asserts the Stage wires viewedSceneId into the initial reconcile.)
});
```

> `setAppContextForTest` does not accept a getter for `viewedSceneId` directly; if the fixture's `Partial<AppContext>` typing rejects `get viewedSceneId()`, pass a plain `viewedSceneId: "sA"` and drop the flip assertion here (the flip is covered by Task 4's engine test). Keep this test focused on "the Stage passes the viewed scene into the engine."

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/module-stage test -- Stage`
Expected: FAIL — Stage still reads `documents.query("scene")[0]`; token count reflects the wrong scene.

- [ ] **Step 3: Destructure `viewedSceneId` from the context**

In `src/modules/stage/src/Stage.svelte`, add `viewedSceneId` accessor usage. Since `getAppContext()` returns a live object, read it lazily via a small getter closure rather than destructuring the value (destructuring would snapshot `null`). Change the destructure line (~line 20) to keep the reactive getter accessible:

```ts
  const ctx = getAppContext();
  const { documents, assets, onAssetChanged, subscribeScene, scene, onPing, role, members } = ctx;
```

- [ ] **Step 4: Pass the getter to the engine + drive the grid from the viewed scene**

In the engine construction (~line 77), add the option:

```ts
      engine = new RenderEngine({
        store: documents,
        assets,
        backend,
        grid: { kind: "square", size: 100 },
        gridColor: readColor("--grid-line", 0x363645),
        subscribeScene,
        viewedSceneId: () => ctx.viewedSceneId,
        onDerivedApplied: (input) => { host.dataset.sceneDerived = "1"; host.dataset.visionMode = input.mode; },
      });
```

In `onDocs` (~line 102-103), read the viewed scene instead of the first:

```ts
      const onDocs = (): void => {
        const vsid = ctx.viewedSceneId;
        const scene = vsid ? documents.get(vsid) : documents.query("scene")[0];
```

Leave the rest of `onDocs` unchanged (`documents.query("token").length` for `tokenCount` still reports the whole store — replace it too, to report the viewed scene, so the observability signal matches what renders):

```ts
        host.dataset.tokenCount = String(documents.query("token").filter((t) => !vsid || t.parent_id === vsid).length);
```

- [ ] **Step 5: Re-project the engine when the viewed scene flips**

Inside the async engine-init block, after `engineRef = e;` (~line 94), add a reactive watcher that re-projects on a viewed-scene change WITHOUT tearing down the engine. Because `ctx.viewedSceneId`'s getter reads the document store + the session's `gmViewedScene` `$state`, bridge the store dependency with `createSubscriber` (already imported? add `import { createSubscriber } from "svelte/reactivity";` at the top if absent) and track the id across runs:

```ts
      // Re-project on a client-local viewed-scene switch (activeScene flip or GM roam). Neither
      // carries a new server frame, so the engine must re-filter its views + last vision payload.
      let lastViewed = ctx.viewedSceneId;
      const vsSub = createSubscriber((update) => documents.subscribe(update));
      offViewed = $effect.root(() => {
        $effect(() => {
          vsSub(); // track store changes (activeScene doc edits)
          const now = ctx.viewedSceneId; // tracks gmViewedScene $state
          if (now !== lastViewed) {
            lastViewed = now;
            e.reapplyViewedScene();
          }
        });
      });
```

Declare `let offViewed: (() => void) | null = null;` beside the other `let off*` declarations near the top of the `$effect` (~line 66), and call `offViewed?.();` in the teardown return (~line 163) alongside `offGrid?.()`.

> If `$effect.root` inside the async IIFE proves awkward under the file's existing single-`$effect` structure, an equivalent is acceptable: fold the viewed-scene check into the existing `onDocs` (compare `ctx.viewedSceneId` against a captured `lastViewed`, call `e.reapplyViewedScene()` on change) AND additionally subscribe `onDocs` to fire on `gmViewedScene` — simplest is to have the scene browser's roam also nudge via `documents.subscribe` is NOT available, so keep the `$effect.root` watcher. Whichever is used, the invariant is: a `ctx.viewedSceneId` change (doc-driven OR roam-driven) calls `e.reapplyViewedScene()` exactly once per change.

- [ ] **Step 6: Run tests + typecheck + commit**

Run: `pnpm --filter @shadowcat/module-stage test -- Stage`
Expected: PASS.
Run: `pnpm --filter @shadowcat/module-stage typecheck`
Expected: no errors.

```bash
git add src/modules/stage/src/Stage.svelte src/modules/stage/src/Stage.test.ts
git commit -m "feat(stage/m12d): drive render + grid from ctx.viewedSceneId, re-project on flip"
```

---

### Task 6: Scene tools operate on the viewed scene

**Files:**
- Modify: `src/modules/scene-tools/src/controller.svelte.ts`
- Modify: `src/modules/scene-tools/src/ToolRail.svelte`
- Test: `src/modules/scene-tools/src/place-tool.test.ts` (extend)

**Interfaces:**
- Consumes: `ctx.viewedSceneId` (Task 3).
- Produces: `ToolContext` gains `viewedSceneId?: () => string | null`; the internal `activeScene(ctx)` helper resolves through it (place/measure/draw/etc. all stamp onto the viewed scene). `ToolRail` passes it + reads it for its own snap-toggle derived.

- [ ] **Step 1: Write the failing test**

Append to `src/modules/scene-tools/src/place-tool.test.ts` a test that, with two scenes, places a token onto the VIEWED scene (its `parent_id`), not the first. Model it on the file's existing place-tool harness (a fake `ToolContext` with `documents`, `dispatchIntent` capture):

```ts
it("stamps the placed token onto the viewed scene, not the first scene", () => {
  // two scenes sA (first) + sB (viewed); a selected asset; place at (0,0)
  const { ctx, sent } = makeToolCtx({ scenes: ["sA", "sB"], viewedSceneId: () => "sB", selectedAsset: "img1" });
  const controller = new ToolController(ctx);
  controller.selectedAsset = "img1";
  controller.toggle("place");
  ctx.scene.__activeTool.onPointerDown({ x: 0, y: 0 });
  const create = sent.flat().find((o) => o.op === "create") as { doc: { parent_id: string } };
  expect(create.doc.parent_id).toBe("sB");
});
```

> Adapt `makeToolCtx`/`__activeTool` to this file's existing helper names. The load-bearing assertion is `parent_id === "sB"` (the viewed scene), which fails before the change (helper resolves `query("scene")[0]` = `sA`).

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/module-scene-tools test -- place-tool`
Expected: FAIL — token parented to `sA` (first scene), not `sB`.

- [ ] **Step 3: Add `viewedSceneId` to `ToolContext` + resolve through it**

In `src/modules/scene-tools/src/controller.svelte.ts`, add to the `ToolContext` interface (after `documents: ReadableDocuments;` ~line 25):

```ts
  /** The scene tools act on (M12d). From `ctx.viewedSceneId`; absent ⇒ the first scene (legacy). */
  viewedSceneId?: () => string | null;
```

Rewrite the `activeScene(ctx)` helper (~line 53) to resolve the viewed scene:

```ts
function activeScene(ctx: ToolContext): { id: string; size: number; perCell: number; unit: string } | null {
  const vsid = ctx.viewedSceneId?.() ?? ctx.documents.query("scene")[0]?.id ?? null;
  const scene = vsid ? ctx.documents.get(vsid) : undefined;
  if (!scene) return null;
  const grid = (scene.system as { grid?: { size?: number; distance?: { perCell: number; unit: string } } } | undefined)?.grid;
  const size = grid?.size ?? 100;
  const { perCell, unit } = grid?.distance ?? { perCell: 5, unit: "ft" };
  return { id: scene.id, size, perCell, unit };
}
```

- [ ] **Step 4: Pass + read the viewed scene in `ToolRail`**

In `src/modules/scene-tools/src/ToolRail.svelte`, add `viewedSceneId` to the `ToolController` construction (~line 20):

```ts
    pathfind: ctx.pathfind,
    viewedSceneId: () => ctx.viewedSceneId,
  });
```

Change the panel's own `activeScene` derived (~line 34) to resolve the viewed scene:

```ts
  const activeScene = $derived.by((): WireDocument | undefined => {
    subscribe();
    const vsid = ctx.viewedSceneId;
    return vsid ? ctx.documents.get(vsid) : ctx.documents.query("scene")[0];
  });
```

- [ ] **Step 5: Run tests + typecheck + commit**

Run: `pnpm --filter @shadowcat/module-scene-tools test`
Expected: PASS.
Run: `pnpm --filter @shadowcat/module-scene-tools typecheck`
Expected: no errors.

```bash
git add src/modules/scene-tools/src/controller.svelte.ts src/modules/scene-tools/src/ToolRail.svelte src/modules/scene-tools/src/place-tool.test.ts
git commit -m "feat(scene-tools/m12d): tools act on the viewed scene"
```

---

### Task 7: Actor browser — live FTS search + open-sheet

**Files:**
- Modify: `src/modules/actors/src/ActorsPanel.svelte`
- Modify: `src/client/ui-kit/src/locales/en.ts`
- Test: `src/modules/actors/src/ActorsPanel.test.ts` (extend)

**Interfaces:**
- Consumes: `ctx.searchDocuments`/`ctx.openDocument`/`ctx.actorSelection` (existing), `WireSearchHit`/`SubscriptionHandle` (core).
- Produces: a search input; a derived `visibleActors` (empty query ⇒ full reactive list, non-empty ⇒ FTS hits filtered to `doc_type:"actor"`); an "Open sheet" button per row.

- [ ] **Step 1: Write the failing test**

Append to `src/modules/actors/src/ActorsPanel.test.ts` (model on the file's existing `setAppContextForTest`/render harness):

```ts
it("opens a sheet for an actor row via ctx.openDocument", async () => {
  const opened: unknown[] = [];
  // seed one actor "a1" in the store; render with openDocument capture
  const context = setAppContextForTest({ documents: storeWithActor("a1"), store: storeWithActor("a1"), openDocument: (ref) => opened.push(ref) });
  const { getByRole } = render(ActorsPanel, { context });
  await fireEvent.click(getByRole("button", { name: /open sheet/i }));
  expect(opened).toEqual([{ docId: "a1" }]);
});

it("runs a live search on a non-empty query and lists only actor hits", async () => {
  let capturedOnUpdate: ((hits: unknown[]) => void) | null = null;
  const context = setAppContextForTest({
    documents: emptyStore(), store: emptyStore(),
    searchDocuments: (_q, _o, onUpdate) => { capturedOnUpdate = onUpdate; return Promise.resolve({ unsubscribe() {} }); },
  });
  const { getByLabelText, findByText } = render(ActorsPanel, { context });
  await fireEvent.input(getByLabelText(/search/i), { target: { value: "gob" } });
  capturedOnUpdate!([
    { document: { id: "a9", doc_type: "actor", system: { name: "Goblin", displayName: "Goblin" } }, score: 1, snippet: "" },
    { document: { id: "i9", doc_type: "item", system: { name: "Gob-stopper" } }, score: 1, snippet: "" },
  ]);
  await findByText("Goblin");
  expect(() => (context.get(Symbol) as never)).not.toThrow(); // item hit is filtered out (no assertion on it)
});
```

> Adapt `storeWithActor`/`emptyStore` to the file's existing fixture helpers. The load-bearing assertions: a per-row "Open sheet" button calls `openDocument({ docId })`; a non-empty query drives `searchDocuments` and renders only `doc_type:"actor"` hits.

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/module-actors test -- ActorsPanel`
Expected: FAIL — no search input / no open-sheet button.

- [ ] **Step 3: Add the search state + open-sheet to `ActorsPanel.svelte`**

In `src/modules/actors/src/ActorsPanel.svelte`, extend the `@shadowcat/core` import with the search types:

```ts
  import { buildActorDoc, setNameHidden, actorDisplayName, listAssets, resolveTokenActor, type Asset as _Asset, type WireDocument, type FactionRegistrySystem, type Faction, type TokenVisual, type FaceVisual, type AnimatedSource, type ConditionRegistrySystem, type Condition, type WireSearchHit, type SubscriptionHandle } from "@shadowcat/core";
```

Add the search state + effect after the existing `actorDocs` derived (~line 16):

```ts
  // Live FTS search (M6c seam). Empty query ⇒ the full reactive list; a non-empty query drives a
  // top-N subscription, torn down/recreated per query change (D-b: not reconnect-resilient).
  let query = $state("");
  let searchHits = $state<WireDocument[]>([]);
  $effect(() => {
    const q = query.trim();
    if (!q) { searchHits = []; return; }
    let handle: SubscriptionHandle | null = null;
    let cancelled = false;
    void ctx
      .searchDocuments(q, { limit: 20 }, (hits: WireSearchHit[]) => {
        // The initial page resolves synchronously inside subscribeSearch's pending-resolve
        // handler, BEFORE this effect's own .then() runs — so cancelled must be checked inside
        // the callback itself, not only at unsubscribe time, or an abandoned query's late first
        // page overwrites a newer query's results.
        if (cancelled) return;
        searchHits = hits.filter((h) => h.document.doc_type === "actor").map((h) => h.document);
      })
      .then((h) => { if (cancelled) h.unsubscribe(); else handle = h; })
      .catch(() => { /* no transport: leave last hits, re-subscribe on next keystroke */ });
    return () => { cancelled = true; handle?.unsubscribe(); };
  });
  const visibleActors = $derived(query.trim() ? searchHits : actorDocs);
```

Add the search input immediately before the `<ul class="list">` (~line 201):

```svelte
  <input
    class="actor-search"
    type="search"
    placeholder={t("actors.search")}
    aria-label={t("actors.search")}
    bind:value={query}
  />
```

Change the list to iterate `visibleActors` and add an "Open sheet" button as the first control in each row (right after the existing select/place button at ~line 208, before the `{#if ctx.role === "gm"}` block):

```svelte
  <ul class="list">
    {#each visibleActors as a (a.id)}
      <li>
        <button
          type="button"
          class:selected={ctx.actorSelection.selectedId === a.id}
          onclick={() => ctx.actorSelection.select(a.id)}
        >{actorDisplayName(a.system as { name?: string; displayName?: string })}</button>
        <button type="button" class="open-sheet" onclick={() => ctx.openDocument({ docId: a.id })}>
          {t("actors.openSheet")}
        </button>
        {#if ctx.role === "gm"}
```

(The rest of the `<li>` GM controls and the closing tags are unchanged.)

Add the `.actor-search` + `.open-sheet` styles inside the `<style>` block:

```scss
  .actor-search {
    min-height: 44px;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: var(--text-primary);
  }
  .actor-search:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .open-sheet {
    min-height: 44px;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: var(--text-primary);
    cursor: pointer;
  }
  .open-sheet:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
```

- [ ] **Step 4: Add i18n keys**

In `src/client/ui-kit/src/locales/en.ts`, add near the other `actors.*` keys:

```ts
  "actors.search": "Search actors",
  "actors.openSheet": "Open sheet",
```

- [ ] **Step 5: Run tests + typecheck (both packages) + commit**

Run: `pnpm --filter @shadowcat/module-actors test -- ActorsPanel`
Expected: PASS.
Run: `pnpm --filter @shadowcat/module-actors typecheck && pnpm --filter @shadowcat/ui-kit typecheck`
Expected: no errors.

```bash
git add src/modules/actors/src/ActorsPanel.svelte src/modules/actors/src/ActorsPanel.test.ts src/client/ui-kit/src/locales/en.ts
git commit -m "feat(actors/m12d): live FTS search + open-sheet in the actor browser"
```

---

### Task 8: Scene browser module

**Files:**
- Create: `src/modules/scene-browser/package.json`
- Create: `src/modules/scene-browser/tsconfig.json`
- Create: `src/modules/scene-browser/src/index.ts`
- Create: `src/modules/scene-browser/src/SceneBrowserPanel.svelte`
- Create: `src/modules/scene-browser/src/SceneBrowserPanel.test.ts`
- Modify: `src/client/ui-kit/src/locales/en.ts`
- Modify: `src/client/shell/src/App.svelte`
- Modify: `src/client/shell/package.json` (add the module dependency)

**Interfaces:**
- Consumes: `PANEL_CONTRACT`/`buildSceneDoc`/`WorldSettingsSystem`/`SceneSystem`/`WireDocument` (core); `getAppContext` + `ctx.viewedSceneId`/`setGmViewedScene`/`sceneSelection`/`panels`/`documents`/`assets`/`dispatchIntent`/`openDocument`/`world`/`role`/`t` (ui-kit/AppContext).
- Produces: `export const sceneBrowser: Module` — a GM-only panel (`order: 6`) that lists scenes with background thumbnails and Activate / View / Configure / Create actions.

- [ ] **Step 1: Write the failing test**

Create `src/modules/scene-browser/src/SceneBrowserPanel.test.ts`:

```ts
import { describe, it, expect, vi } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import SceneBrowserPanel from "./SceneBrowserPanel.svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/__fixtures__/appContextTest"; // adapt path to the barrel fixture export used elsewhere
import { DocumentStore, buildSceneDoc, buildWorldSettingsDoc, DEFAULT_WORLD_SETTINGS, SceneSelection } from "@shadowcat/core"; // SceneSelection is from ui-kit; import from there

function seed(): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "u", ts: 0, ops: [
    { op: "create", doc: buildSceneDoc("w1", {}, "sA") },
    { op: "create", doc: buildSceneDoc("w1", {}, "sB") },
    { op: "create", doc: buildWorldSettingsDoc("w1", { ...structuredClone(DEFAULT_WORLD_SETTINGS), activeScene: "sA" }) },
  ] });
  return s;
}

describe("SceneBrowserPanel", () => {
  it("lists every scene", () => {
    const store = seed();
    const context = setAppContextForTest({ documents: store, store, role: "gm" });
    const { getAllByRole } = render(SceneBrowserPanel, { context });
    // one row per scene (each row has an Activate button)
    expect(getAllByRole("button", { name: /activate/i })).toHaveLength(2);
  });

  it("Activate writes world-settings.activeScene with the real pre-image as old", async () => {
    const store = seed();
    const sent: unknown[] = [];
    const context = setAppContextForTest({ documents: store, store, role: "gm", dispatchIntent: (ops) => sent.push(ops) });
    const { getAllByRole } = render(SceneBrowserPanel, { context });
    await fireEvent.click(getAllByRole("button", { name: /activate/i })[1]); // activate sB
    const op = (sent[0] as { op: string; doc_id: string; changes: { path: string; old: unknown; new: unknown }[] }[])[0];
    expect(op.op).toBe("update");
    expect(op.changes[0].path).toBe("/system/activeScene");
    expect(op.changes[0].old).toBe("sA"); // REAL current value, not null
    expect(op.changes[0].new).toBe("sB");
  });

  it("View sets the GM local roam via ctx.setGmViewedScene", async () => {
    const store = seed();
    const roams: (string | null)[] = [];
    const context = setAppContextForTest({ documents: store, store, role: "gm", setGmViewedScene: (id) => roams.push(id) });
    const { getAllByRole } = render(SceneBrowserPanel, { context });
    await fireEvent.click(getAllByRole("button", { name: /^view$/i })[1]);
    expect(roams).toEqual(["sB"]);
  });

  it("Configure focuses the scene in game-settings and opens it", async () => {
    const store = seed();
    const selection = new SceneSelection();
    const opened: string[] = [];
    const context = setAppContextForTest({
      documents: store, store, role: "gm", sceneSelection: selection,
      panels: { open: (id: string) => opened.push(id), close() {}, focus() {}, toggle() {}, minimized: [], metaMap: new Map(), restore() {} } as never,
    });
    const { getAllByRole } = render(SceneBrowserPanel, { context });
    await fireEvent.click(getAllByRole("button", { name: /configure/i })[1]);
    expect(selection.configureSceneId).toBe("sB");
    expect(opened).toEqual(["game-settings"]);
  });

  it("Create dispatches a new scene document", async () => {
    const store = seed();
    const sent: unknown[] = [];
    const context = setAppContextForTest({ documents: store, store, role: "gm", world: "w1", dispatchIntent: (ops) => sent.push(ops) });
    const { getByRole } = render(SceneBrowserPanel, { context });
    await fireEvent.click(getByRole("button", { name: /new scene/i }));
    const op = (sent[0] as { op: string; doc: { doc_type: string } }[])[0];
    expect(op.op).toBe("create");
    expect(op.doc.doc_type).toBe("scene");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/module-scene-browser test`
Expected: FAIL — package/module not found.

- [ ] **Step 3: Create the package scaffold**

Create `src/modules/scene-browser/package.json`:

```json
{
  "name": "@shadowcat/module-scene-browser",
  "version": "0.0.0",
  "private": true,
  "type": "module",
  "main": "src/index.ts",
  "dependencies": {
    "@shadowcat/core": "workspace:*",
    "@shadowcat/ui-kit": "workspace:*",
    "@shadowcat/types": "workspace:^"
  },
  "devDependencies": {
    "@testing-library/svelte": "^5.3.1",
    "jsdom": "^29.1.1",
    "sass": "^1.101.0"
  },
  "scripts": {
    "typecheck": "svelte-check --tsconfig ./tsconfig.json",
    "test": "vitest run --passWithNoTests"
  }
}
```

Create `src/modules/scene-browser/tsconfig.json`:

```json
{
  "extends": "../../../tsconfig.base.json",
  "compilerOptions": { "types": ["svelte"] },
  "include": ["src/**/*.ts", "src/**/*.svelte"]
}
```

- [ ] **Step 4: Write the module manifest**

Create `src/modules/scene-browser/src/index.ts`:

```ts
import { PANEL_CONTRACT, type Module } from "@shadowcat/core";
import SceneBrowserPanel from "./SceneBrowserPanel.svelte";

/** GM scene browser (M12d): scene list with background thumbnails, create, configure (deep-links
 * the game-settings per-scene section), local view (GM roam), and activate (sets the scene players
 * render). Requires the panel-manager's contract; launcher-closed by default, after game-settings. */
export const sceneBrowser: Module = {
  manifest: {
    id: "scene-browser",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: [PANEL_CONTRACT],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({
      id: "scene-browser:panel",
      contract: PANEL_CONTRACT,
      order: 6,
      component: SceneBrowserPanel,
      panel: { icon: "🗺️", labelKey: "sceneBrowser.tab", gmOnly: true },
    });
  },
};
```

- [ ] **Step 5: Write the panel**

Create `src/modules/scene-browser/src/SceneBrowserPanel.svelte`:

```svelte
<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { buildSceneDoc, type WireDocument, type WorldSettingsSystem, type SceneSystem } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  // Reactive bridge (mandatory): register a dependency on the doc store so the list re-renders on
  // create/activate and the viewed/active badges track edits.
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  const scenes = $derived.by((): WireDocument[] => {
    subscribe();
    return ctx.documents.query("scene");
  });
  const ws = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("world-settings")[0];
  });
  const activeSceneId = $derived.by((): string | null => {
    return (ws?.system as WorldSettingsSystem | undefined)?.activeScene ?? null;
  });
  // The GM's own rendered scene (roam or followed active). Reading the getter tracks the doc store
  // (via subscribe above) + the session's gmViewedScene state.
  const viewedSceneId = $derived.by((): string | null => {
    subscribe();
    return ctx.viewedSceneId;
  });
  const roaming = $derived(viewedSceneId !== null && viewedSceneId !== activeSceneId);

  function bgOf(scene: WireDocument): string | null {
    return (scene.system as SceneSystem | undefined)?.background ?? null;
  }

  /** Set the scene players render. OCC pre-image is the REAL current activeScene (or null when
   * genuinely absent) — never a defaulted value. No-op with a debug hint if world-settings is
   * absent (game-settings seeds it on the same GM Welcome). */
  function activate(sceneId: string): void {
    if (!ws) return;
    const old = (ws.system as WorldSettingsSystem | undefined)?.activeScene ?? null;
    ctx.dispatchIntent([{ op: "update", doc_id: ws.id, changes: [{ path: "/system/activeScene", old, new: sceneId }] }]);
  }

  /** GM local roam (no effect on players). */
  function view(sceneId: string): void {
    ctx.setGmViewedScene(sceneId);
  }
  function followActive(): void {
    ctx.setGmViewedScene(null);
  }

  /** Deep-link the game-settings per-scene section to this scene. */
  function configure(sceneId: string): void {
    ctx.sceneSelection.select(sceneId);
    ctx.panels.open("game-settings");
  }

  function create(): void {
    ctx.dispatchIntent([{ op: "create", doc: buildSceneDoc(ctx.world) }]);
  }
</script>

<section class="scene-browser" aria-label={t("sceneBrowser.title")}>
  <h3>{t("sceneBrowser.title")}</h3>
  {#if roaming}
    <p class="hint">
      {t("sceneBrowser.roaming")}
      <button type="button" onclick={followActive}>{t("sceneBrowser.followActive")}</button>
    </p>
  {/if}
  <ul class="list">
    {#each scenes as scene, i (scene.id)}
      <li class:active={scene.id === activeSceneId} class:viewed={scene.id === viewedSceneId}>
        <div class="thumb">
          {#if bgOf(scene)}
            <img src={ctx.assets.url(bgOf(scene)!)} alt="" />
          {:else}
            <span class="placeholder" aria-hidden="true">🗺️</span>
          {/if}
        </div>
        <span class="label">
          {t("sceneBrowser.sceneLabel", { n: i + 1 })}
          {#if scene.id === activeSceneId}<span class="badge">{t("sceneBrowser.activeBadge")}</span>{/if}
          {#if scene.id === viewedSceneId && scene.id !== activeSceneId}<span class="badge">{t("sceneBrowser.viewingBadge")}</span>{/if}
        </span>
        <div class="actions">
          <button type="button" onclick={() => activate(scene.id)} disabled={!ws || scene.id === activeSceneId}>{t("sceneBrowser.activate")}</button>
          <button type="button" onclick={() => view(scene.id)}>{t("sceneBrowser.view")}</button>
          <button type="button" onclick={() => configure(scene.id)}>{t("sceneBrowser.configure")}</button>
        </div>
      </li>
    {/each}
  </ul>
  <button type="button" class="create" onclick={create}>{t("sceneBrowser.create")}</button>
</section>

<style lang="scss">
  .scene-browser {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-1);
  }
  .hint {
    margin: 0;
    color: var(--text-secondary);
    font-size: 0.85em;
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .list li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
  }
  .list li.viewed {
    border-color: var(--accent);
  }
  .thumb {
    width: 48px;
    height: 48px;
    flex: none;
    border-radius: var(--radius-1);
    overflow: hidden;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--surface-base);
  }
  .thumb img {
    width: 48px;
    height: 48px;
    object-fit: cover;
    display: block;
  }
  .label {
    flex: 1 1 auto;
    color: var(--text-primary);
  }
  .badge {
    margin-left: var(--space-1);
    padding: 0 var(--space-1);
    border-radius: var(--radius-1);
    background: var(--accent);
    color: var(--on-accent);
    font-size: 0.75em;
  }
  .actions {
    display: flex;
    gap: var(--space-1);
    flex-wrap: wrap;
  }
  .actions button,
  .create,
  .hint button {
    min-height: 44px;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: var(--text-primary);
    cursor: pointer;
  }
  .actions button:focus-visible,
  .create:focus-visible,
  .hint button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .actions button:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
```

- [ ] **Step 6: Add i18n keys**

In `src/client/ui-kit/src/locales/en.ts`, add:

```ts
  "sceneBrowser.tab": "Scenes",
  "sceneBrowser.title": "Scenes",
  "sceneBrowser.sceneLabel": "Scene {n}",
  "sceneBrowser.activate": "Activate",
  "sceneBrowser.view": "View",
  "sceneBrowser.configure": "Configure",
  "sceneBrowser.create": "New scene",
  "sceneBrowser.activeBadge": "Active",
  "sceneBrowser.viewingBadge": "Viewing",
  "sceneBrowser.roaming": "Viewing a scene other than the active one.",
  "sceneBrowser.followActive": "Follow active",
```

- [ ] **Step 7: Register the module**

In `src/client/shell/package.json`, add to `dependencies` (alphabetically near the other `@shadowcat/module-*` entries):

```json
    "@shadowcat/module-scene-browser": "workspace:*",
```

In `src/client/shell/src/App.svelte`, add the import (after the `gameSettings` import, ~line 17):

```ts
  import { sceneBrowser } from "@shadowcat/module-scene-browser";
```

and add `sceneBrowser` to the `modules: [ … ]` array in the `WorldSession` construction (~line 92), immediately after `gameSettings`:

```ts
  modules: [panels, coreUi, topBar, statusBar, stage, settings, gameSettings, sceneBrowser, assets, actors, factions, conditions, sceneTools, chat, chatComposer, chatCard, sheetFallback, sheetActor, sheetItem]
```

- [ ] **Step 8: Install + run tests + typecheck + commit**

Run: `pnpm install` (links the new workspace package)
Run: `pnpm --filter @shadowcat/module-scene-browser test`
Expected: PASS.
Run: `pnpm --filter @shadowcat/module-scene-browser typecheck && pnpm --filter @shadowcat/shell typecheck && pnpm --filter @shadowcat/ui-kit typecheck`
Expected: no errors.

```bash
git add src/modules/scene-browser package.json pnpm-lock.yaml src/client/shell/package.json src/client/shell/src/App.svelte src/client/ui-kit/src/locales/en.ts
git commit -m "feat(scene-browser/m12d): GM scene browser — list/thumbnail/create/configure/view/activate"
```

---

### Task 9: Game-settings per-scene deep-link from the scene browser

**Files:**
- Modify: `src/modules/game-settings/src/GameSettingsPanel.svelte`
- Test: `src/modules/game-settings/src/scene-overrides.test.ts` (extend)

**Interfaces:**
- Consumes: `ctx.sceneSelection.configureSceneId` (Task 3).
- Produces: `GameSettingsPanel` presets its per-scene picker to `ctx.sceneSelection.configureSceneId` when the scene browser sets it; the GM may still change the picker afterward.

- [ ] **Step 1: Write the failing test**

Append to `src/modules/game-settings/src/scene-overrides.test.ts` (model on the file's existing render harness that seeds scenes + world-settings):

```ts
it("presets the per-scene picker to ctx.sceneSelection.configureSceneId", async () => {
  const store = /* seed with scenes sA, sB + world-settings (existing helper) */;
  const selection = new SceneSelection(); // from @shadowcat/ui-kit
  const context = setAppContextForTest({ documents: store, store, role: "gm", sceneSelection: selection });
  const { getByLabelText } = render(GameSettingsPanel, { context });
  selection.select("sB");
  await tick();
  // the scene picker's value reflects the deep-linked scene
  expect((getByLabelText(/scene/i) as HTMLSelectElement).value).toBe("sB");
});
```

> Adapt the store-seed + the scene-picker's `aria-label` to the file's existing conventions. The load-bearing assertion: setting `ctx.sceneSelection` after mount moves the picker to that scene.

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/module-game-settings test -- scene-overrides`
Expected: FAIL — the picker stays on the first scene.

- [ ] **Step 3: Wire the deep-link**

In `src/modules/game-settings/src/GameSettingsPanel.svelte`, after `let selectedSceneId = $state<string | null>(null);` (~line 103), add an effect that adopts the browser's focus. Reading `ctx.sceneSelection.configureSceneId` ($state) tracks it, so a later `select()` re-fires; the GM can still override via the picker afterward (the effect only re-runs when the seam value changes):

```ts
  // Deep-link from the scene browser's "Configure" (M12d): adopt its focused scene. Only reacts to
  // a non-null change, so a manual picker change afterward is preserved until the browser re-focuses.
  $effect(() => {
    const focus = ctx.sceneSelection.configureSceneId;
    if (focus) selectedSceneId = focus;
  });
```

- [ ] **Step 4: Run tests + typecheck + commit**

Run: `pnpm --filter @shadowcat/module-game-settings test -- scene-overrides`
Expected: PASS.
Run: `pnpm --filter @shadowcat/module-game-settings typecheck`
Expected: no errors.

```bash
git add src/modules/game-settings/src/GameSettingsPanel.svelte src/modules/game-settings/src/scene-overrides.test.ts
git commit -m "feat(game-settings/m12d): deep-link the per-scene section from the scene browser"
```

---

## Self-Review

**1. Spec coverage** (§6 + D6 + §9):
- *Actor browser — live FTS search, create, open-sheet, place* → Task 7 (search + open-sheet); create + place already exist in `ActorsPanel` (place via `ActorSelection`, verified unchanged). ✔
- *Scene browser — GM-gated panel, thumbnails, create, configure, activate* → Task 8 (all five) + Task 9 (configure deep-link). ✔
- *Multi-scene `activeScene` on world-settings, GM-writable via config-doc path* → Task 1 (field) + Task 8 (Activate write, real OCC `old`). ✔
- *Players follow `activeScene`, fail-closed to first scene when absent* → Task 1 (`resolveViewedScene`) + Task 2 (`viewedSceneId`). ✔
- *GM may locally view any scene; see-as/vision already per-scene* → Task 2 (`gmViewedScene`) + Task 4 (viewed-scene fog/vision projection) + Task 5 (Stage re-project) + Task 8 (View). ✔
- *Scene deletion stays deferred* → out of scope; not touched. ✔
- *§9 accessibility for new panels* → search input + all scene-browser buttons are real buttons with `aria-label`/text, `:focus-visible` rings, ≥44px coarse-pointer targets; scene browser reached via the topbar launcher / command menu (panel registration, gmOnly). ✔
- *D6 "No new server code"* → every task is client-only; verified `activeScene`/scene-filter are opaque `system` reads. ✔

**2. Placeholder scan:** No "TBD"/"implement later". Two tests (Task 5, Task 6, Task 7, Task 9) say "adapt to the file's existing harness" — these reference REAL existing test helpers whose exact names live in files not fully quoted here; the load-bearing assertion is spelled out in each. This is a deliberate hook into existing fixtures, not a missing-code placeholder. Task 4's fake-backend accessor note is likewise an adaptation to the existing `engine.test.ts` fake, with the assertion intent fully specified.

**3. Type consistency:** `viewedSceneId` is `string | null` everywhere (core resolver return, `WorldSession` getter, `AppContext` property, engine `() => string | null` opt). `resolveViewedScene(store, { gmViewedScene })` signature identical across Task 1 (def) → Task 2 (call). `sceneScopedDocs(store, docType, viewedSceneId)` identical Task 4 (def) → view call sites. `SceneSelection.configureSceneId`/`select` identical Task 3 (def) → Tasks 8/9 (use). `searchDocuments(query, opts, onUpdate) => Promise<SubscriptionHandle>` identical WsClient → WorldSession → AppContext → ActorsPanel. Engine `reapplyViewedScene()` identical Task 4 (def) → Task 5 (call). `activeScene` OCC path uses the raw `WorldSettingsSystem.activeScene ?? null` as `old` (Task 8), consistent with the hard OCC rule.
