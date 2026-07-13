# M12a — Panel-Manager Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the fixed tabbed sidebar with the unified dockable panel system (`@shadowcat/module-panels`): zones → tabbed groups → splits, floating, minimize-to-chip, drag-to-dock, per-world persistence, and a compact (mobile) switcher — with the stage as a locked center well.

**Architecture:** A pure, engine-agnostic layout tree (`PanelLayoutV1` + reducer) is canonical; `dockview-core@7.0.2` is bound behind an `EngineAdapter` interface inside module-panels only (zero engine types in any contract; a `FakeEngine` drives host tests and doubles as the bespoke-fallback seam). Panels mount ONCE into slot elements rendered declaratively under the Svelte tree; every host (engine groups, floating, compact switcher) adopts the same DOM node — keep-mounted by construction. Spec: `docs/superpowers/specs/2026-07-13-m12-dockable-panels-default-modules-design.md`; engine gate: `docs/superpowers/specs/2026-07-13-m12a-dockview-core-spike.md` (ADOPT + W1–W3).

**Tech Stack:** Svelte 5 (runes), TypeScript, dockview-core 7.0.2 (exact pin), Vitest (+jsdom), SCSS tokens, pnpm workspace.

## Model/Effort directives

Per the user's standing directive (2026-07-13): plan written mainline on the design-session model; execution = SDD with `shadowcat-coder` (Sonnet, effort medium) implementers, `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` (effort high) per task, `-opus` twins for buddy-checks/escalation and the whole-branch final review. Dispatcher = this session, mainline.

## Buddy-check directives (pre-authorized — spec §15 + spike ruling)

- **Task 2** (layout reducer) — full two-reviewer blind buddy-check.
- **Task 6** (DockviewEngine: W1–W3 stage-well enforcement + drop-veto policy) — full buddy-check; reviewers must attempt to construct a sequence that closes/relocates/displaces the stage.
- **Task 8** (sidebar swap integration) — full buddy-check on the assembled swap (contract retirement completeness + no stale refs; [[stale-ref-grep-include-rs]] applies: grep the WHOLE tree).
- All other tasks: standard two-reviewer gate.

## Global Constraints

- `dockview-core` pinned EXACTLY `7.0.2` (the spike-verified version) in `src/modules/panels/package.json`; no other new dependencies.
- Zero dockview types/imports outside `src/modules/panels/src/engine/dockview.ts` (+ its test). CI-greppable: `dockview` appears nowhere else in `src/`.
- Panels hide via CSS only, never `{#if}` unmount (M11d-1 discipline); mount-counter tests enforce.
- The stage panel can never be closed, relocated, displaced, or covered by a docked group (W1–W3).
- `gmOnly` filtering is advisory UI only; server redaction remains the gate.
- Every task ends with `pnpm -r typecheck` green (vitest alone strips types — [[vitest-skips-typecheck-in-sdd]]) plus the task's own tests.
- Commit per task with the project's format; branch `m12a-panel-core` off local `main`; merge --no-ff at checkpoint end after the whole-branch review + skill-update gate.
- i18n: all new user-visible strings go through `t(key)` with entries added to `ui-kit/src/locales/en.json`.
- Cross-platform: no pointer-only affordances — every drag action reachable via the Task 9 command menu; touch targets ≥ 24px.

## File Structure (locked)

```
src/modules/panels/                     NEW  @shadowcat/module-panels
  package.json
  src/index.ts                          manifest: provides shadowcat.panel (multi) decl +
                                        panel-host contribution + panel-dock chips contribution
  src/layout/tree.ts                    PanelLayoutV1 + LayoutOp reducer + defaultLayout (pure)
  src/layout/tree.test.ts
  src/layout/persist.ts                 validate/prune/version-fallback codec (pure)
  src/layout/persist.test.ts
  src/engine/adapter.ts                 EngineAdapter interface (engine-agnostic)
  src/engine/fake.ts                    FakeEngine for host tests / bespoke seam
  src/engine/dockview.ts                DockviewEngine: binding, zone mapping, veto, W1-W3
  src/engine/policy.ts                  pure drop/veto/normalization helpers
  src/engine/policy.test.ts
  src/engine/dockview.test.ts           jsdom integration (best-effort; W3 guard mandatory)
  src/PanelHost.svelte                  slots + expanded/compact presentation switch
  src/PanelHost.test.ts                 mount-counter + boundary containment
  src/CompactSwitcher.svelte
  src/DockChips.svelte
  src/controller.svelte.ts              PanelsController: ops→tree→engine/persist; binds bridge
  src/controller.test.ts
  src/panels.scss                       dockview skin via SCSS tokens
src/client/core/src/contributions.ts    MODIFY: +PanelMeta/DefaultPlacement, Contribution.panel
src/client/ui-kit/src/panelsBridge.ts   NEW: PanelsApi + PanelsBridge (late-bound, scene-bridge pattern)
src/client/ui-kit/src/sizeClass.svelte.ts NEW: compact|expanded via matchMedia (min-width: 48rem)
src/client/ui-kit/src/appContext.ts     MODIFY: +panels: PanelsApi; uiState +panelLayout accessors
src/client/ui-kit/src/index.ts          MODIFY exports (add above; DELETE TabbedSurface in Task 8)
src/client/shell/src/lib/api.ts         MODIFY: UiState.worlds[w].panelLayout?: unknown
src/client/shell/src/lib/sessionState.svelte.ts MODIFY: getPanelLayout/setPanelLayout
src/client/shell/src/lib/Table.svelte   MODIFY: ctx.panels + uiState wiring
src/client/shell/src/App.svelte         MODIFY: modules array (panels replaces sidebar)
src/client/shell/src/lib/defaultModuleOrder.test.ts REWRITE
src/modules/core-ui/src/index.ts        MODIFY: surface decls (−sidebar-host −sidebar +panel-host)
src/modules/core-ui/src/Layout.svelte   REWRITE grid: topbar/toolrail/main/statusbar
src/modules/statusbar/src/{index.ts,StatusBar.svelte} MODIFY: declare+host shadowcat.surface:panel-dock
src/modules/{chat,assets,actors,factions,conditions,game-settings,settings}/src/index.ts
                                        MODIFY: re-target to shadowcat.panel + PanelMeta
DELETE (Task 8): src/modules/sidebar/ (whole package), ui-kit TabbedSurface.svelte + test,
                 ContributionTab type + all tab reads
```

**Contract model (plan-level pin, consistent with spec §4):** ONE multi contract id `shadowcat.panel` (mirroring today's `shadowcat.surface:sidebar` shape — module-panels declares it, panel modules `require` it and contribute into it with `Contribution.panel` metadata). The spec's `shadowcat.panel:*` notation is realized as contribution `id`s (`"chat:panel"` etc.), not per-panel contract ids.

**Interim default layout (M12a only, flipped in M12b):** chat docked right; the other six panels minimized (visible as chips — the restore affordance). M12b's topbar launcher then changes defaults to spec D3's chat-only + launcher.

---

### Task 1: Core contribution model — `PanelMeta`

**Files:** Modify `src/client/core/src/contributions.ts`, `src/client/core/src/index.ts`; test `src/client/core/src/contributions.test.ts` (extend).

**Interfaces — Produces (used by every later task):**
```ts
export type ZoneId = "right" | "bottom" | "left";
export type DefaultPlacement =
  | { kind: "docked"; zone: ZoneId; order?: number }
  | { kind: "minimized" };            // absent ⇒ launcher-only (closed)
export interface PanelMeta {
  icon: string;
  labelKey: string;                    // host-resolved i18n key (M11d-1 precedent)
  gmOnly?: boolean;                    // advisory UI filter only
  defaultPlacement?: DefaultPlacement;
}
export interface Contribution { /* existing fields */; panel?: PanelMeta; }
export const PANEL_CONTRACT = "shadowcat.panel";
```
`tab?: ContributionTab` REMAINS until Task 8 deletes it (both exist during the transition; no runtime reads either field together).

- [ ] Write failing test: a contribution registered under `PANEL_CONTRACT` with `panel` metadata round-trips through `contributionsFor(PANEL_CONTRACT)` (order-sorted, metadata intact).
- [ ] Run: `pnpm --filter @shadowcat/core test` → FAIL (unknown export).
- [ ] Implement the types above in `contributions.ts`; export `PanelMeta`, `DefaultPlacement`, `ZoneId`, `PANEL_CONTRACT` from `core/src/index.ts`.
- [ ] Run tests + `pnpm -r typecheck` → PASS.
- [ ] Commit: `feat(core/m12a): PanelMeta + shadowcat.panel contract types`

### Task 2: Layout tree + reducer (pure) — BUDDY-CHECK

**Files:** Create `src/modules/panels/src/layout/tree.ts`, `tree.test.ts`. (Package scaffolding — `package.json` with deps `@shadowcat/core`, `@shadowcat/ui-kit`, `@shadowcat/types`, `dockview-core: "7.0.2"`, vitest config matching sibling modules, pnpm-workspace pickup — is folded into this task since the deliverable needs it.)

**Interfaces — Produces:**
```ts
export interface Rect { x: number; y: number; w: number; h: number }
export interface GroupNode { tabs: string[]; active: string; size: number }   // size = fraction within zone
export interface ZoneNode  { groups: GroupNode[]; size: number }              // size = px basis of the zone
export interface ExpandedLayout {
  zones: Record<ZoneId, ZoneNode>;    // all three keys always present (possibly empty groups)
  floating: { id: string; rect: Rect; z: number }[];
  minimized: string[];
}
export interface CompactLayout { activeView: string | null; order: string[] }
export interface PanelLayoutV1 { version: 1; expanded: ExpandedLayout; compact: CompactLayout }

export type LayoutOp =
  | { op: "open"; id: string; placement?: DefaultPlacement }
  | { op: "close"; id: string }
  | { op: "dock"; id: string; zone: ZoneId; group: number | "new"; tabIndex?: number }
  | { op: "float"; id: string; rect: Rect }
  | { op: "minimize"; id: string }
  | { op: "restore"; id: string }
  | { op: "activeTab"; zone: ZoneId; group: number; id: string }
  | { op: "resizeZone"; zone: ZoneId; size: number }
  | { op: "resizeGroup"; zone: ZoneId; group: number; size: number }
  | { op: "compactView"; id: string };

export type PanelLocation =
  | { where: "docked"; zone: ZoneId; group: number; tabIndex: number }
  | { where: "floating"; index: number } | { where: "minimized" } | { where: "closed" };

export function locate(l: PanelLayoutV1, id: string): PanelLocation;
export function applyOp(l: PanelLayoutV1, o: LayoutOp): PanelLayoutV1;        // pure; new object
export function prune(l: PanelLayoutV1, known: ReadonlySet<string>): PanelLayoutV1;
export function defaultLayout(regs: { id: string; placement?: DefaultPlacement }[]): PanelLayoutV1;
```

**Reducer invariants (each is a test):** a panel id appears in AT MOST ONE location; removing a group's last tab removes the group (and empty zones keep `size`); `close`/`minimize`/`float` from any prior location first detaches; `open` on an open panel is a focus no-op (returns same location, bumps floating `z` / sets group `active`); `dock` with `group: "new"` inserts at end with equal-share `size` renormalization; `activeTab` on a non-member id is a no-op returning the SAME reference (cheap change detection); floating `z` is compacted (no unbounded growth); `prune` drops unknown ids everywhere incl. `compact.order` and fixes `active`/`activeView` to a surviving member; `compactView` on unknown id no-ops; every op is total — no throw on any input location.

- [ ] Write the failing test suite: one `describe` per op + the invariants above; construct via `defaultLayout([{id:"chat", placement:{kind:"docked",zone:"right"}}, {id:"assets",placement:{kind:"minimized"}}, ...])` and assert chat docked-right group `{tabs:["chat"],active:"chat"}`, six minimized, compact order = registration order, `activeView:"chat"`.
- [ ] Run: `pnpm --filter @shadowcat/module-panels test` → FAIL (module missing).
- [ ] Implement `tree.ts` (immutable updates via structured spread; helper `detach(l,id): [PanelLayoutV1, PanelLocation]` used by every mutating op; `renormalize(groups)` equal-share on insert/remove).
- [ ] Run tests + typecheck → PASS. Commit: `feat(panels/m12a): pure layout tree + reducer`
- [ ] **Buddy-check** (two blind reviewers; hand-trace `detach`+renormalize sequences and the one-location invariant under randomized op sequences — reviewer-constructed, not corpus).

### Task 3: Persistence codec + shell `ui_state` wiring

**Files:** Create `src/modules/panels/src/layout/persist.ts`, `persist.test.ts`; modify `src/client/shell/src/lib/api.ts` (UiState), `sessionState.svelte.ts`, plus its existing test file.

**Interfaces — Produces:**
```ts
// persist.ts (pure; module-panels)
export function encodeLayout(l: PanelLayoutV1): unknown;                       // JSON-safe
export function decodeLayout(raw: unknown, known: ReadonlySet<string>,
                             fallback: () => PanelLayoutV1): { layout: PanelLayoutV1; reset: boolean };
// shell sessionState
export function getPanelLayout(world: string): unknown | null;
export function setPanelLayout(world: string, blob: unknown): void;            // debounced PUT (existing path)
```
`decodeLayout` returns `reset: true` (and the fallback) on: non-object, `version !== 1`, structural mismatch (hand-rolled guards — shell `UiState` is deliberately Zod-free), or any panel id non-string. Valid blobs are then `prune`d against `known`. `api.ts`: `worlds: Record<string, { activeTab?: string; panelLayout?: unknown }>` (`activeTab` field deleted in Task 8).

- [ ] Failing tests: round-trip encode→decode identity; garbage/`version: 2`/truncated blobs ⇒ `reset: true` + fallback; unknown ids pruned (`reset: false`); sessionState `setPanelLayout` schedules the SAME debounced persist as `setActiveTab` (reuse `schedulePersist`; assert via existing `flushSessionState` test helper).
- [ ] Run → FAIL. Implement. Run tests + typecheck → PASS.
- [ ] Commit: `feat(panels/m12a): layout persistence codec + ui_state panelLayout`

### Task 4: ui-kit primitives — size class, PanelsBridge, AppContext

**Files:** Create `src/client/ui-kit/src/sizeClass.svelte.ts`, `panelsBridge.ts`; modify `appContext.ts`, `index.ts`; tests `sizeClass.test.ts`, `panelsBridge.test.ts`.

**Interfaces — Produces:**
```ts
// sizeClass.svelte.ts — single source of truth; replaces ad-hoc 40rem queries as they're touched
export type SizeClass = "compact" | "expanded";
export function sizeClass(): SizeClass;          // createSubscriber over matchMedia("(min-width: 48rem)")
// panelsBridge.ts — scene-bridge pattern: late-bound, no-op-warn before bind
export interface PanelsApi {
  open(id: string): void; close(id: string): void;
  focus(id: string): void; toggle(id: string): void;
}
export class PanelsBridge implements PanelsApi { bind(impl: PanelsApi): void; /* delegates or warns */ }
// appContext.ts additions
panels: PanelsApi;
uiState: { getPanelLayout(): unknown | null; setPanelLayout(blob: unknown): void;
           getActiveTab(): string | null; setActiveTab(id: string): void }   // tab pair deleted Task 8
```

- [ ] Failing tests: `PanelsBridge.open` before bind warns once + no-throw, after bind delegates; `sizeClass()` reflects a mocked `matchMedia` and updates on listener fire.
- [ ] Run → FAIL. Implement (mirror `SceneInteractionBridge`'s shape; matchMedia guarded for jsdom absence → default "expanded"). Export from `index.ts`.
- [ ] Run tests + typecheck → PASS. Commit: `feat(ui-kit/m12a): sizeClass + PanelsBridge + AppContext.panels`

### Task 5: PanelHost + slots + CompactSwitcher + DockChips (FakeEngine)

**Files:** Create `src/modules/panels/src/engine/adapter.ts`, `engine/fake.ts`, `PanelHost.svelte`, `CompactSwitcher.svelte`, `DockChips.svelte`, `PanelHost.test.ts`; locale keys in `ui-kit/src/locales/en.json` (`panels.minimize`, `panels.restore`, `panels.close`, `panels.dockRight`, `panels.dockBottom`, `panels.dockLeft`, `panels.float`, `panels.moreViews`).

**Interfaces — Produces:**
```ts
// adapter.ts — the ONLY seam DockviewEngine (Task 6) implements
export interface EngineAdapter {
  init(host: HTMLElement, slotFor: (id: string) => HTMLElement, stageEl: HTMLElement): void;
  apply(expanded: ExpandedLayout, meta: ReadonlyMap<string, PanelMeta>): void; // reconcile to tree
  onOp(cb: (op: LayoutOp) => void): () => void;   // user gestures → normalized LayoutOps
  focus(id: string): void;
  destroy(): void;
}
```
**Slot mechanism (keep-mounted by construction):** `PanelHost.svelte` renders, for every visible-to-role registration, `<div class="panel-slot" data-panel={id}><svelte:boundary onerror={...}><C.component/></svelte:boundary></div>` inside a `display:none` staging container — Svelte context flows normally. Hosts (engine groups / compact views / chips-restore) ADOPT slot elements via `appendChild`; nothing ever unmounts except module unload or registration removal. Crash boundary renders `panel crashed · reload` with a re-mount key bump, panel-local.

`PanelHost` chooses presentation off `sizeClass()`: expanded → engine container + `DockChips` data; compact → `CompactSwitcher` (bottom switcher listing registry order, full-screen active view — adopts the active slot; non-active slots stay in staging). `DockChips` renders minimized ids (+ gmOnly filter) as labeled buttons → `restore` op; contributed into `shadowcat.surface:panel-dock`.

- [ ] Failing tests (`PanelHost.test.ts`, jsdom, FakeEngine): **mount-counter** — a counting fixture panel mounts exactly once across: open→dock→minimize→restore→float(FakeEngine ops)→compact flip→expanded flip; **gmOnly** — game-settings-like registration absent from switcher/chips when `role !== "gm"`; **boundary** — a fixture that throws on an event leaves siblings alive and shows the reload affordance; **adoption** — after `apply`, the chat slot's element is a descendant of the FakeEngine's group container (same node identity, `isSameNode`).
- [ ] Run → FAIL. Implement `fake.ts` (in-memory groups as plain divs honoring `apply`/`onOp`), the three components, staging/adoption.
- [ ] Run tests + typecheck → PASS. Commit: `feat(panels/m12a): PanelHost slots + compact switcher + chips (FakeEngine)`

### Task 6: DockviewEngine — zone mapping, veto policy, W1–W3 — BUDDY-CHECK

**Files:** Create `src/modules/panels/src/engine/policy.ts`, `policy.test.ts`, `engine/dockview.ts`, `dockview.test.ts`, `panels.scss` (minimal skin: dockview CSS import + token overrides; full skin Task 9).

**Interfaces — Consumes:** `EngineAdapter` (Task 5), tree types (Task 2). **Produces:** `export class DockviewEngine implements EngineAdapter` and pure `policy.ts`:
```ts
export function classifyDrop(target: DropSite, layout: ExpandedLayout): LayoutOp | { veto: true; reason: string };
// DropSite is OUR type {kind: "group"|"edge"|"floating"; zone?: ZoneId; group?: number; position: "left"|"right"|"top"|"bottom"|"center"} —
// dockview event objects are translated to DropSite INSIDE dockview.ts; policy.ts stays engine-free.
```
**Vetoes (each a policy test):** any drop targeting the stage group or splitting it (defense-in-depth behind `locked: 'no-drop-target'`); any drop resolving to a position ABOVE the stage row (no top zone — spec D4); any op whose subject is `"stage"`. **W1:** stage panel added at init with a custom headerless group (custom tab renderer rendering nothing + `.sc-stage-group .dv-tabs-container { display:none }`), `locked: 'no-drop-target'`. **W2:** `onWillDrop` → `classifyDrop` → `event.preventDefault()` on veto; adapter `apply`/`focus` ignore `"stage"`. **W3:** `onDidRemovePanel` for the stage id re-adds it immediately + `console.error` (fail-safe guard, mandatory jsdom test — programmatic `removePanel` on stage leaves a live stage panel).

- [ ] Failing policy tests (pure, no DOM): the veto table + happy-path classifications (edge-right ⇒ `dock zone:"right" group:"new"`; tab-strip drop ⇒ `dock` with `tabIndex`; float-drag ⇒ `float` with rect).
- [ ] Run → FAIL. Implement `policy.ts`. Run → PASS.
- [ ] jsdom integration tests for `dockview.ts`: init creates stage + applies a two-panel tree (assert slot elements adopted into dockview group DOM — `isSameNode`); W3 guard test; `apply` idempotence (double-apply produces no duplicate panels). If dockview proves un-runnable under jsdom, STOP and report to dispatcher (spec's honest-failure rule) — do not fake the coverage; the fallback is a documented browser-manual checklist + policy tests, decided by the dispatcher, not silently.
- [ ] Implement `dockview.ts` (translate dockview `onDidDrop`/`onWillDrop`/group-resize/tab-active events → `DropSite`/`LayoutOp`; apply = diff current engine panels/groups vs tree, add/move/remove via dockview API; exact-pin import).
- [ ] Run all module tests + typecheck → PASS. Commit: `feat(panels/m12a): DockviewEngine + stage-well W1-W3 + drop policy`
- [ ] **Buddy-check** (adversarial: reviewers independently attempt stage-well violations via op sequences, drop classifications, and direct engine API, and verify the veto/guard closes each).

### Task 7: Controller + module manifest + persistence wiring

**Files:** Create `src/modules/panels/src/controller.svelte.ts`, `controller.test.ts`, `src/index.ts` (manifest); modify `PanelHost.svelte` to consume the controller.

**Interfaces — Consumes:** everything above. **Produces:** module `panels`:
```ts
// manifest (mirror module-sidebar's shape, inverted to the new contracts)
// Contract ownership follows the sidebar-host precedent: the HOSTING module declares the
// surface (core-ui declares panel-host; statusbar declares panel-dock — Task 8); module-panels
// requires both and contributes into them.
{ id: "panels", dependencies: { "core-ui": "^0.1.0" },
  requires: ["shadowcat.surface:panel-host", "shadowcat.surface:panel-dock"],
  provides: [{ contract: "shadowcat.panel", cardinality: "multi" }],
  register(ctx) {
    ctx.contribute({ id: "panels:host", contract: "shadowcat.surface:panel-host", component: PanelHost });
    ctx.contribute({ id: "panels:chips", contract: "shadowcat.surface:panel-dock", component: DockChips });
  } }
```
`PanelsController` (runes class): builds registrations from `contributionsFor(PANEL_CONTRACT)`; layout = `decodeLayout(ctx.uiState.getPanelLayout(), knownIds, () => defaultLayout(regs))`; on `reset: true` → toast (statusbar text via live region, `panels.layoutReset` key). Dispatch path: `PanelsApi`/engine ops → `applyOp` → engine `apply` (expanded) → `encodeLayout` → `ctx.uiState.setPanelLayout`. Binds itself into the shell's `PanelsBridge` at mount (bridge instance arrives via AppContext).

- [ ] Failing tests: `open` on a closed reg uses `defaultPlacement`; op→persist flow calls `setPanelLayout` with the encoded new tree (spy); reset path fires the toast callback + persists the default; gmOnly registration invisible to non-GM (`regsForRole`).
- [ ] Run → FAIL. Implement controller + manifest + host wiring.
- [ ] Run module tests + typecheck → PASS. Commit: `feat(panels/m12a): PanelsController + module manifest + persistence flow`

### Task 8: The swap — core-ui/Layout, statusbar, 7 re-targets, sidebar deletion, shell wiring — BUDDY-CHECK

**Files:** Modify `core-ui/src/index.ts` (provides: −`sidebar-host` −`sidebar` +`shadowcat.surface:panel-host` singleton), `core-ui/src/Layout.svelte` (grid `topbar/toolrail/main/statusbar`, `main` hosts panel-host, growth-cap comment moves with the `min-height:0` to `.main`; phone media query keeps toolrail hidden — M12b replaces it); `statusbar/src/index.ts` (+provides `{ contract: "shadowcat.surface:panel-dock", cardinality: "singleton" }`) and `StatusBar.svelte` (renders `<Surface contract="shadowcat.surface:panel-dock" />` right-aligned — the hosting module declares the surface, per the sidebar-host precedent); the seven module `index.ts` re-targets + their `index.test.ts`; `App.svelte:89` array (`sidebar` → `panels`); `Table.svelte` (ctx.panels = shared `PanelsBridge` instance, uiState accessors swap to panelLayout pair, DELETE activeTab pair); `sessionState.svelte.ts` + `api.ts` (delete activeTab accessors + field); REWRITE `defaultModuleOrder.test.ts`; DELETE `src/modules/sidebar/` package, `ui-kit/src/TabbedSurface.svelte` + `TabbedSurface.test.ts` (mount-counter coverage already superseded by `PanelHost.test.ts` in Task 5), `ContributionTab` + `Contribution.tab` + ui-kit exports of both.

All seven re-targets (each replaces the module's existing `shadowcat.surface:sidebar` contribution; `order` becomes launcher order, values unchanged; contribution ids renamed `<id>:sidebar` → `<id>:panel`):
```ts
// chat/src/index.ts
{ id: "chat:panel", contract: PANEL_CONTRACT, order: 0, component: ChatPanel,
  panel: { icon: "💬", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } } }
// assets/src/index.ts
{ id: "assets:panel", contract: PANEL_CONTRACT, order: 1, component: Assets,
  panel: { icon: "🖼️", labelKey: "assets.tab", defaultPlacement: { kind: "minimized" } } }
// actors/src/index.ts
{ id: "actors:panel", contract: PANEL_CONTRACT, order: 2, component: ActorsPanel,
  panel: { icon: "👥", labelKey: "actors.tab", defaultPlacement: { kind: "minimized" } } }
// factions/src/index.ts
{ id: "factions:panel", contract: PANEL_CONTRACT, order: 3, component: FactionsPanel,
  panel: { icon: "🚩", labelKey: "factions.tab", defaultPlacement: { kind: "minimized" } } }
// conditions/src/index.ts
{ id: "conditions:panel", contract: PANEL_CONTRACT, order: 4, component: ConditionsPanel,
  panel: { icon: "✨", labelKey: "conditions.tab", defaultPlacement: { kind: "minimized" } } }
// game-settings/src/index.ts
{ id: "game-settings:panel", contract: PANEL_CONTRACT, order: 5, component: GameSettingsPanel,
  panel: { icon: "⚙️", labelKey: "gameSettings.tab", gmOnly: true, defaultPlacement: { kind: "minimized" } } }
// settings/src/index.ts
{ id: "settings:panel", contract: PANEL_CONTRACT, order: 6, component: Settings,
  panel: { icon: "🔧", labelKey: "settings.tab", defaultPlacement: { kind: "minimized" } } }
```
Each module's manifest `requires` swaps `["shadowcat.surface:sidebar"]` → `["shadowcat.panel"]`
(chat keeps its additional `chat.composer`/`chat.message` provides untouched). The
`{ kind: "minimized" }` defaults are the documented M12a interim; M12b's launcher flips them to
absent (launcher-only) per spec D3.
New `defaultModuleOrder.test.ts` invariant: with the full default module set registered, `contributionsFor(PANEL_CONTRACT)[0].id === "chat:panel"` AND `defaultLayout(regs)` docks exactly `chat` (everything else minimized) — the default-visible-panel collision guard, restated for panels.

- [ ] Failing first: rewrite `defaultModuleOrder.test.ts` + one updated module test (chat) → run → FAIL against old wiring.
- [ ] Execute the swap file-by-file per the list above.
- [ ] **Whole-tree stale-ref grep** (all file types, no allowlist): `grep -rn "sidebar-host\|shadowcat.surface:sidebar\|TabbedSurface\|ContributionTab\|activeTab" src/ docs/design/` → expected: zero hits in `src/` (design-doc mentions get updated in the doc-sync gate).
- [ ] Run FULL gates: `pnpm -r test && pnpm -r typecheck && pnpm lint && pnpm build` → all green (the build proves the shell still assembles; `dist/` refresh keeps the server compile-ready — [[embed-dist-compile-ordering]]).
- [ ] Commit: `feat(m12a): the sidebar→panels swap — 7 re-targets, sidebar/TabbedSurface deleted`
- [ ] **Buddy-check** (assembled-swap review: contract retirement completeness, App/Table/sessionState wiring, the grep evidence, default-layout invariant).

### Task 9: A11y command layer + full skin

**Files:** Modify `dockview.ts` (custom tab component: icon + `t(labelKey)` label + menu button), create `src/modules/panels/src/PanelMenu.svelte`, extend `panels.scss`, locale keys; tests in `PanelHost.test.ts` (extend) + `policy.test.ts` (menu-command parity).

Requirements (spec §9): every tab/floating-header carries a menu — items `dockRight/dockBottom/dockLeft/float/minimize/close` — dispatching the SAME `LayoutOp`s as drags (parity test: menu action ⇒ `applyOp` result identical to the equivalent `classifyDrop` op); tab strips implement APG roving tabindex (ArrowLeft/Right/Home/End, one tab stop per strip); floating groups get `role="dialog"` + `aria-label` = panel label, focus-in on float/restore, focus-return to the invoking element on close, Escape closes focused floating panel; chips are `<button>`s with names; a polite `aria-live` region announces `t("panels.moved", {panel, where})` on every layout op; `:focus-visible` ring on all interactive parts; targets ≥24px. Skin: dockview CSS vars mapped to project tokens (`--surface-*`, `--border`, `--text-*`), dark-first, chips/tab styling consistent with existing sidebar look.

- [ ] Failing tests: menu-op parity; roving tabindex (arrow moves focus without activating, Enter activates); Escape closes floating; live-region text fires on minimize.
- [ ] Implement. Run module tests + typecheck + `pnpm lint` → PASS.
- [ ] Commit: `feat(panels/m12a): command menu a11y layer + token skin`

### Task 10: Checkpoint verification sweep

**Files:** none new (fixes only if red).

- [ ] `pnpm -r test && pnpm -r typecheck && pnpm lint && pnpm build` — all green.
- [ ] `grep -rn "dockview" src/ --include="*" | grep -v "src/modules/panels"` → only the panels package (global constraint).
- [ ] `cd src/server && cargo test` in a subshell ([[bash-cwd-drift-breaks-edit-hook-and-git]]) — server untouched but embed still compiles against fresh `dist/`.
- [ ] Manual smoke via `verify` skill: run the binary, enter a world; confirm default layout (chat docked, chips), drag chat to bottom dock, float it, minimize, restore, resize zone, reload page → layout persists; narrow window < 48rem → switcher; GM-only chip hidden for a player session.
- [ ] Fix anything red (fix-forward); re-run.
- [ ] Commit: `chore(m12a): checkpoint verification sweep`

---

## Post-execution gates (SDD process, after Task 10)

1. Whole-branch final review: two `-opus` reviewers (spec lens + code lens) over the full `m12a-panel-core` diff.
2. Reviewed skill-update gate: update `shadowcat-codebase-client-shell` AND create `shadowcat-codebase-panels` (new subsystem — fixed shape, hook globs `src/modules/panels/**`); dispatch `shadowcat-spec-reviewer` on the skill diffs.
3. Doc sync: PLAN.md M12a entry, TODO.md (log: M12b flips interim minimized-defaults to launcher-closed; jsdom-dockview outcome if the Task 6 fallback fired), POST_WORK_FINDINGS for any API friction (bug-report rule).
4. Merge --no-ff to local `main`. NO push.
