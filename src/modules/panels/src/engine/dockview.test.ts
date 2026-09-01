import { test, expect, afterEach, vi } from "vitest";
import { defaultLayout, applyOp, type LayoutOp, type PanelLayoutV1 } from "../layout/tree";
import { DockviewEngine } from "./dockview";
import { STAGE_ID } from "./policy";
import { silentLogger, type PanelMeta } from "@shadowcat/core";
import { i18n, theme } from "@shadowcat/ui-kit";
import type {
  DockviewApi,
  DockviewPopoutGroupOptions,
  DockviewWillDropEvent,
  IDockviewGroupPanel,
  IDockviewPanel,
} from "dockview-core";

/** A dockview-core internal event emitter. These are regular (non-`#`-private) class
 * fields, so a structural declaration reaches them without the library exposing them. */
interface InternalEmitter<T> {
  fire(payload: T): void;
}

/** The subset of `DockviewWillDropEvent` the engine's own drop listener reads. Tests
 * construct exactly this and push it through the internal emitter, exercising the same
 * listener path a real drop takes. */
interface WillDropProbe {
  kind: DockviewWillDropEvent["kind"];
  position: DockviewWillDropEvent["position"];
  panel: IDockviewPanel | undefined;
  group: IDockviewGroupPanel | undefined;
  getData: () => { viewId: string; groupId: string; panelId: string | null };
  readonly defaultPrevented: boolean;
  preventDefault: () => void;
}

/** `DockviewComponent` internals. Declaring the reached members narrowly keeps the
 * dependency checkable: a dockview upgrade that renames one fails at this declaration
 * rather than passing an untyped value through every call site. */
interface ComponentInternals {
  _onWillDrop: InternalEmitter<WillDropProbe>;
  _bufferOnDidLayoutChange: InternalEmitter<void>;
  _onDidPopoutGroupSizeChange: InternalEmitter<{ width: number; height: number; group: IDockviewGroupPanel }>;
  _onDidPopoutGroupPositionChange: InternalEmitter<{ screenX: number; screenY: number; group: IDockviewGroupPanel }>;
}

/** Group-level internals, reached per group rather than through the component. */
interface GroupModelInternals {
  _onWillDrop: InternalEmitter<WillDropProbe>;
  _onDidAddPanel: InternalEmitter<{ panel: IDockviewPanel }>;
}
interface GroupApiInternals {
  _onDidDimensionChange: InternalEmitter<{ width: number; height: number }>;
}

const componentOf = (api: DockviewApi): ComponentInternals =>
  (api as unknown as { component: ComponentInternals }).component;
const modelOf = (group: IDockviewGroupPanel): GroupModelInternals =>
  group.model as unknown as GroupModelInternals;
const apiOf = (group: IDockviewGroupPanel): GroupApiInternals =>
  group.api as unknown as GroupApiInternals;

/** The structural slice of dockview's `DockviewFloatingGroupPanel` a test
 * asserts reconcile writes against — mirrors the engine's own private
 * `FloatingWindowEntry` declaration (the class is not re-exported from
 * dockview-core's entry point). */
interface FloatingEntryProbe {
  group: unknown;
  overlay: { element: HTMLElement };
  position(bounds: { left?: number; top?: number; width?: number; height?: number }): void;
}

/** Finds the live floating-window entry hosting `panelId`'s group, by the same
 * membership rule (`FloatingGroupService.findByGroup`) the engine uses. */
function floatingEntryOf(api: DockviewApi, panelId: string): FloatingEntryProbe {
  const group = api.getPanel(panelId)!.group;
  const entries = (api as unknown as { component: { floatingGroups: readonly FloatingEntryProbe[] } }).component
    .floatingGroups;
  const entry = entries.find((e) => e.group === group || e.overlay.element.contains(group.element));
  if (!entry) throw new Error(`no floating window entry for panel "${panelId}"`);
  return entry;
}

/** A `DOMRect` stub for jsdom (which never runs real layout) — stands in for
 * the box a real gesture or reconcile would leave on the overlay element. */
function domRectOf(left: number, top: number, width: number, height: number): DOMRect {
  return {
    left,
    top,
    width,
    height,
    right: left + width,
    bottom: top + height,
    x: left,
    y: top,
    toJSON: () => ({}),
  } as DOMRect;
}

let engine: DockviewEngine | null = null;
// Hosts appended to `document.body` for focus-management tests (jsdom only
// tracks `document.activeElement` for attached elements) are torn down here
// alongside the engine, so no test leaks a detached-but-body-mounted `<div>`
// into a later test's `document.body`.
let attachedHost: HTMLElement | null = null;

afterEach(() => {
  engine?.destroy();
  engine = null;
  attachedHost?.remove();
  attachedHost = null;
});

function makeSlots(ids: string[]): (id: string) => HTMLElement {
  const map = new Map<string, HTMLElement>();
  for (const id of ids) {
    const el = document.createElement("div");
    el.dataset.panel = id;
    map.set(id, el);
  }
  return (id: string) => map.get(id) ?? document.createElement("div");
}

function twoPanelLayout(): PanelLayoutV1 {
  let l = defaultLayout([{ id: "chat" }, { id: "assets" }]);
  l = applyOp(l, { op: "dock", id: "chat", zone: "right", group: "new" });
  l = applyOp(l, { op: "dock", id: "assets", zone: "bottom", group: "new" });
  return l;
}

test("init mounts the stage and apply() adopts a two-panel tree's slot elements", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  stageEl.dataset.stage = "";
  const slotFor = makeSlots(["chat", "assets"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);

  // The stage element itself is adopted somewhere under host.
  expect(host.contains(stageEl)).toBe(true);

  const layout = twoPanelLayout();
  const meta = new Map();
  engine.apply(layout.expanded, meta);

  const chatSlot = slotFor("chat");
  const assetsSlot = slotFor("assets");
  expect(host.contains(chatSlot)).toBe(true);
  expect(host.contains(assetsSlot)).toBe(true);
  // Adoption is real (appendChild of the SAME node), not a copy.
  expect(chatSlot.isSameNode(slotFor("chat"))).toBe(true);
});

test("init passes a style-free theme class so dockview's default theme literals never override the token bridge", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots([]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);

  // dockview falls back to `themeAbyss` when no theme option is given, and
  // its `dockview-theme-abyss` class re-declares every --dv-* variable with
  // dark literals on the shell element — beating the `.sc-dockview-root`
  // token mapping (var() references) for the whole subtree under any
  // non-dark active theme.
  expect(host.querySelector(".dockview-theme-abyss")).toBeNull();
  expect(host.querySelector(".sc-dockview-theme")).toBeTruthy();
});

test("apply() is idempotent: applying the same tree twice adds no duplicate panels", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat", "assets"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);

  const layout = twoPanelLayout();
  const meta = new Map();
  engine.apply(layout.expanded, meta);
  const afterFirst = host.querySelectorAll("[data-panel]").length;
  engine.apply(layout.expanded, meta);
  const afterSecond = host.querySelectorAll("[data-panel]").length;

  expect(afterFirst).toBe(2);
  expect(afterSecond).toBe(2);
});

test("programmatic removal of the stage panel leaves a live stage panel", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots([]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);

  const api = engine.debugApi;
  expect(api).not.toBeNull();
  const stagePanel = api!.getPanel(STAGE_ID);
  expect(stagePanel).toBeDefined();

  // Simulates an external actor calling the underlying dockview API directly
  // against the stage panel id — a path the wrapper's own op vocabulary
  // never exposes (no registration ever contributes id "stage"), but which
  // the `#restoreStage` guard must still survive.
  api!.removePanel(stagePanel!);

  const restored = api!.getPanel(STAGE_ID);
  expect(restored).toBeDefined();
  expect(restored!.id).toBe(STAGE_ID);
});

test("focus() and apply() ignore the stage id (adapter-level guard)", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);

  // Neither call throws or does anything observable to the stage panel —
  // there is no registration path that could ever pass "stage" here, but the
  // adapter refuses it anyway (defense-in-depth).
  expect(() => engine!.focus(STAGE_ID)).not.toThrow();

  const stagePanelBefore = engine.debugApi!.getPanel(STAGE_ID);
  engine.apply(twoPanelLayout().expanded, new Map());
  const stagePanelAfter = engine.debugApi!.getPanel(STAGE_ID);
  expect(stagePanelAfter).toBe(stagePanelBefore);
});

test("a user tab-switch inside a docked group emits an activeTab op", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat", "notes"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);

  let layout = defaultLayout([{ id: "chat" }, { id: "notes" }]);
  layout = applyOp(layout, { op: "dock", id: "chat", zone: "right", group: "new" });
  layout = applyOp(layout, { op: "dock", id: "notes", zone: "right", group: 0 });
  engine.apply(layout.expanded, new Map());

  const ops: Array<{ op: string }> = [];
  engine.onOp((op) => ops.push(op));

  // `panel.api.setActive()` always runs under the wrapper's own 'api'
  // origin (see `DockviewPanelApiImpl.setActive`) — this drives activation
  // through the group model directly, the same path a real tab click takes,
  // so the resulting event carries the default 'user' origin.
  const chatPanel = engine.debugApi!.getPanel("chat")!;
  chatPanel.group.model.openPanel(chatPanel);

  expect(ops).toContainEqual({ op: "activeTab", zone: "right", group: 0, id: "chat" });
});

/** Fires a synthetic `onWillDrop` through the underlying dockview component —
 * `DockviewWillDropEvent` isn't exported from `dockview-core`, and simulating
 * a real native drag gesture isn't possible under jsdom, so a duck-typed
 * event object (the exact shape `#toDropSite`/`#handleWillDrop` read) is
 * pushed straight into the component's own `_onWillDrop` emitter (a regular,
 * non-`#`-private class field — accessible via a cast, not a real API). This
 * exercises the SAME listener path as `init()`'s `api.onWillDrop(...)`. */
function fireWillDrop(
  engine: DockviewEngine,
  overrides: Partial<{
    kind: DockviewWillDropEvent["kind"];
    position: DockviewWillDropEvent["position"];
    panelId: string | null;
    groupId: string;
    group: IDockviewGroupPanel;
  }>,
): { defaultPrevented: boolean } {
  let prevented = false;
  const event: WillDropProbe = {
    kind: overrides.kind ?? "edge",
    position: overrides.position ?? "top",
    panel: undefined,
    group: overrides.group,
    getData: () => ({ viewId: "v", groupId: overrides.groupId ?? "g", panelId: overrides.panelId ?? null }),
    get defaultPrevented() {
      return prevented;
    },
    preventDefault() {
      prevented = true;
    },
  };
  componentOf(engine.debugApi!)._onWillDrop.fire(event);
  return event;
}

test("a whole-group transfer (panelId null) at the container's TOP edge is vetoed", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);
  engine.apply(twoPanelLayout().expanded, new Map());

  const event = fireWillDrop(engine, { kind: "edge", position: "top", panelId: null });
  expect(event.defaultPrevented).toBe(true);
});

test("a whole-group transfer at a zone-edge position is ALSO vetoed (every group transfer is vetoed, not just top)", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);
  engine.apply(twoPanelLayout().expanded, new Map());

  const event = fireWillDrop(engine, { kind: "edge", position: "right", panelId: null });
  expect(event.defaultPrevented).toBe(true);
});

test("a whole-group drag translates into ordered per-tab dock ops instead of being vetoed", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["tab-1", "tab-2", "tab-3"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);

  let layout = defaultLayout([{ id: "tab-1" }, { id: "tab-2" }, { id: "tab-3" }]);
  layout = applyOp(layout, { op: "dock", id: "tab-1", zone: "right", group: "new" });
  layout = applyOp(layout, { op: "dock", id: "tab-2", zone: "right", group: 0 });
  layout = applyOp(layout, { op: "dock", id: "tab-3", zone: "right", group: 0 });
  engine.apply(layout.expanded, new Map());

  const ops: LayoutOp[] = [];
  engine.onOp((op) => ops.push(op));

  // Real dockview group id for the "right" zone's single group, derived from
  // its first tab (`groupIdFor`) — a titlebar drag of that whole group to the
  // (empty) "left" edge.
  const event = fireWillDrop(engine, { kind: "edge", position: "left", panelId: null, groupId: "sc-group:tab-1" });

  expect(ops).toEqual([
    { op: "dock", id: "tab-1", zone: "left", group: "new" },
    { op: "dock", id: "tab-2", zone: "left", group: 0, tabIndex: 1 },
    { op: "dock", id: "tab-3", zone: "left", group: 0, tabIndex: 2 },
  ]);
  expect(event.defaultPrevented).toBe(true);
});

test("a whole-group drag onto an unclassifiable target still vetoes (fail-closed unchanged for the newly-translated group case)", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["tab-1", "tab-2"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);

  let layout = defaultLayout([{ id: "tab-1" }, { id: "tab-2" }]);
  layout = applyOp(layout, { op: "dock", id: "tab-1", zone: "right", group: "new" });
  layout = applyOp(layout, { op: "dock", id: "tab-2", zone: "right", group: 0 });
  engine.apply(layout.expanded, new Map());

  const ops: LayoutOp[] = [];
  engine.onOp((op) => ops.push(op));

  // The container's TOP edge: no "top" `ZoneId` variant exists — genuinely
  // unclassifiable regardless of whether the transfer is a single tab or a
  // whole group.
  const event = fireWillDrop(engine, { kind: "edge", position: "top", panelId: null, groupId: "sc-group:tab-1" });

  expect(event.defaultPrevented).toBe(true);
  expect(ops).toHaveLength(0);
});

test("a will-drop event before any apply() fails closed (defaultPrevented)", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);
  // Deliberately no `apply()` call — `#expanded` is still null.

  const event = fireWillDrop(engine, { kind: "edge", position: "top", panelId: "chat" });
  expect(event.defaultPrevented).toBe(true);
});

test("a tree naming the stage id in a zone group applies without throwing, and the real stage stays alive in its own locked group", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);

  let layout = defaultLayout([{ id: "chat" }]);
  layout = applyOp(layout, { op: "dock", id: "chat", zone: "right", group: "new" });
  // Manually inject a stage-naming zone group the reducer itself would never
  // produce (STAGE_ID is never a real registration) — simulating a bug
  // elsewhere in the pipeline that lets it through to `apply()`.
  layout = {
    ...layout,
    expanded: {
      ...layout.expanded,
      zones: {
        ...layout.expanded.zones,
        left: { ...layout.expanded.zones.left, groups: [{ tabs: [STAGE_ID], active: STAGE_ID, size: 1 }] },
      },
    },
  };

  expect(() => engine!.apply(layout.expanded, new Map())).not.toThrow();

  const stagePanel = engine.debugApi!.getPanel(STAGE_ID);
  expect(stagePanel).toBeDefined();
  expect(stagePanel!.group.id).toBe("sc-stage-group");
});

test("a zone group naming BOTH the stage id and a real panel places the real panel and leaves the stage untouched", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);

  let layout = defaultLayout([{ id: "chat" }]);
  layout = applyOp(layout, { op: "dock", id: "chat", zone: "right", group: "new" });
  // A MIXED group — the stage id sits alongside a real panel id in the same
  // group's tabs, unlike the all-stage case above (which skips the whole
  // group). The per-tab STAGE_ID skip must fire for "stage" only; "chat"
  // still gets placed normally in the same (real, non-stage) group.
  layout = {
    ...layout,
    expanded: {
      ...layout.expanded,
      zones: {
        ...layout.expanded.zones,
        right: { ...layout.expanded.zones.right, groups: [{ tabs: [STAGE_ID, "chat"], active: "chat", size: 1 }] },
      },
    },
  };

  expect(() => engine!.apply(layout.expanded, new Map())).not.toThrow();

  const chatPanel = engine.debugApi!.getPanel("chat");
  expect(chatPanel).toBeDefined();
  expect(chatPanel!.group.id).not.toBe("sc-stage-group");

  const stagePanel = engine.debugApi!.getPanel(STAGE_ID);
  expect(stagePanel).toBeDefined();
  expect(stagePanel!.group.id).toBe("sc-stage-group");
});

test("group-onto-group: a whole-group transfer targeting an existing group's content is intercepted (defaultPrevented) and translated into a dock op per tab of the dragged group, via the per-group onWillDrop wire", () => {
  // `DockviewApi.onWillDrop` (subscribed once in `init()`) NEVER
  // fires for a drop targeting an existing group — the component only
  // forwards a group model's own `onWillDrop` through the permanently-unwired
  // `_advancedDnDService` optional chain (in `DockviewComponent.createGroup`'s
  // `onWillDrop` wiring).
  // This exercises the mechanism that actually closes the gap: a per-group
  // subscription to `group.model.onWillDrop` (`#groupWillDropSubs`, wired in
  // `apply()`), fired here via the group model's own private `_onWillDrop`
  // emitter — the SAME emitter `group.model.onWillDrop(cb)` subscribes to in
  // production (the same direct-emitter-fire technique used elsewhere for
  // `_onDidDimensionChange.fire`, testing a real dockview event without a native drag gesture).
  // The "assets" group here has a single tab ("assets" itself), so its
  // translated ops are identical in shape to a single-tab drop of that id.
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat", "assets"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);
  engine.apply(twoPanelLayout().expanded, new Map());

  const chatGroup = engine.debugApi!.getPanel("chat")!.group;

  const ops: LayoutOp[] = [];
  engine.onOp((op) => ops.push(op));

  let prevented = false;
  const event: WillDropProbe = {
    kind: "content",
    position: "center",
    panel: undefined,
    group: chatGroup,
    // panelId: null — a whole-GROUP transfer (a titlebar drag of the
    // "assets" group) dropped onto "chat"'s group content area.
    getData: () => ({ viewId: "v", groupId: "sc-group:assets", panelId: null }),
    get defaultPrevented() {
      return prevented;
    },
    preventDefault() {
      prevented = true;
    },
  };
  modelOf(chatGroup)._onWillDrop.fire(event);

  expect(prevented).toBe(true);
  expect(ops).toEqual([{ op: "dock", id: "assets", zone: "right", group: 0, tabIndex: 1 }]);
});

test("group-onto-group: a whole-group transfer targeting a SPECIFIC tab-strip position in an existing group produces sequential tabIndex ops starting at that position, correctly displacing the target group's own tabs", () => {
  // Regression coverage for `#expandGroupDockOp`'s existing-group branch: the
  // prior tests only covered a "new group" edge drop and a single-tab
  // content drop (no explicit `tabIndex`, falling back to append-at-end).
  // This exercises a real tab-strip drop target (`kind: "tab"`, `event.panel`
  // set) at a specific position WITHIN an existing multi-tab target group,
  // dragging a multi-tab group onto it.
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat", "p2", "p3", "tab-1", "tab-2"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);

  let layout = defaultLayout([{ id: "chat" }, { id: "p2" }, { id: "p3" }, { id: "tab-1" }, { id: "tab-2" }]);
  layout = applyOp(layout, { op: "dock", id: "chat", zone: "right", group: "new" });
  layout = applyOp(layout, { op: "dock", id: "p2", zone: "right", group: 0 });
  layout = applyOp(layout, { op: "dock", id: "p3", zone: "right", group: 0 });
  layout = applyOp(layout, { op: "dock", id: "tab-1", zone: "left", group: "new" });
  layout = applyOp(layout, { op: "dock", id: "tab-2", zone: "left", group: 0 });
  engine.apply(layout.expanded, new Map());

  const chatGroup = engine.debugApi!.getPanel("chat")!.group;
  const p2Panel = engine.debugApi!.getPanel("p2")!;

  const ops: LayoutOp[] = [];
  engine.onOp((op) => ops.push(op));

  let prevented = false;
  const event: WillDropProbe = {
    kind: "tab",
    position: "center",
    // A real tab-strip drop target: "p2" is the tab the pointer is hovering
    // over, at index 1 within chat's group ([chat, p2, p3]).
    panel: p2Panel,
    group: chatGroup,
    // panelId: null — a whole-GROUP transfer (a titlebar drag of the "left"
    // zone's ["tab-1", "tab-2"] group) dropped at "p2"'s tab-strip position.
    getData: () => ({ viewId: "v", groupId: "sc-group:tab-1", panelId: null }),
    get defaultPrevented() {
      return prevented;
    },
    preventDefault() {
      prevented = true;
    },
  };
  modelOf(chatGroup)._onWillDrop.fire(event);

  expect(prevented).toBe(true);
  // Both dragged tabs land at consecutive indices starting at "p2"'s
  // position (1), preserving their own relative order.
  expect(ops).toEqual([
    { op: "dock", id: "tab-1", zone: "right", group: 0, tabIndex: 1 },
    { op: "dock", id: "tab-2", zone: "right", group: 0, tabIndex: 2 },
  ]);

  // Applying the emitted ops through the reducer confirms the target
  // group's own tabs ("p2", "p3") are correctly displaced rather than
  // overwritten.
  let final = layout;
  for (const op of ops) final = applyOp(final, op);
  expect(final.expanded.zones.right.groups[0].tabs).toEqual(["chat", "tab-1", "tab-2", "p2", "p3"]);
});

test("group-onto-group: an ALLOWED single-panel drop onto an existing group's content is intercepted (defaultPrevented) and redispatched as exactly one dock op, via the per-group onWillDrop wire", () => {
  // An allowed classification always calls preventDefault() too — dockview's
  // own internal move machinery never completes the drop; the classified op
  // is emitted to op listeners instead so the tree stays canonical.
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat", "assets"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);
  engine.apply(twoPanelLayout().expanded, new Map());

  const chatGroup = engine.debugApi!.getPanel("chat")!.group;

  const ops: LayoutOp[] = [];
  engine.onOp((op) => ops.push(op));

  let prevented = false;
  const event: WillDropProbe = {
    kind: "content",
    position: "center",
    panel: undefined,
    group: chatGroup,
    // A real single-panel id ("assets") dragged onto "chat"'s group — a
    // legitimate cross-group move `classifyDrop` should allow.
    getData: () => ({ viewId: "v", groupId: "sc-group:assets", panelId: "assets" }),
    get defaultPrevented() {
      return prevented;
    },
    preventDefault() {
      prevented = true;
    },
  };
  modelOf(chatGroup)._onWillDrop.fire(event);

  expect(prevented).toBe(true);
  expect(ops).toHaveLength(1);
  expect(ops[0]).toEqual({ op: "dock", id: "assets", zone: "right", group: 0 });
});

test("root/edge: an ALLOWED edge drop is intercepted (defaultPrevented) and redispatched as exactly one dock op, via the component-level onWillDrop wire", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);
  engine.apply(twoPanelLayout().expanded, new Map());

  const ops: LayoutOp[] = [];
  engine.onOp((op) => ops.push(op));

  const event = fireWillDrop(engine, { kind: "edge", position: "left", panelId: "chat" });

  expect(event.defaultPrevented).toBe(true);
  expect(ops).toHaveLength(1);
  expect(ops[0]).toEqual({ op: "dock", id: "chat", zone: "left", group: "new" });
});

test("no spurious close op: an ALLOWED cross-group drop, applied through the reducer, never causes the moved panel's removal to also emit a close op", () => {
  // The moved panel's group DOES change (apply()'s remove+re-add-under-groupId
  // reconcile), but that removal runs inside apply()'s `#applying` window, so
  // it must never surface as a user-driven close.
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat", "assets"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);
  let layout = twoPanelLayout();
  engine.apply(layout.expanded, new Map());

  const chatGroup = engine.debugApi!.getPanel("chat")!.group;

  const ops: LayoutOp[] = [];
  engine.onOp((op) => ops.push(op));

  const event: WillDropProbe = {
    kind: "content",
    position: "center",
    panel: undefined,
    group: chatGroup,
    getData: () => ({ viewId: "v", groupId: "sc-group:assets", panelId: "assets" }),
    get defaultPrevented() {
      return false;
    },
    preventDefault() {},
  };
  modelOf(chatGroup)._onWillDrop.fire(event);

  // Apply the emitted op, mirroring the real controller — this drives the
  // reconcile that actually performs the cross-group move.
  const dockOp = ops.find((o) => o.op === "dock");
  expect(dockOp).toBeDefined();
  layout = applyOp(layout, dockOp!);
  engine.apply(layout.expanded, new Map());

  expect(ops.some((o) => o.op === "close")).toBe(false);
});

test("a group's live dimension change emits resizeZone + resizeGroup ops with sane values", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat", "notes"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);

  let layout = defaultLayout([{ id: "chat" }, { id: "notes" }]);
  layout = applyOp(layout, { op: "dock", id: "chat", zone: "right", group: "new" });
  layout = applyOp(layout, { op: "dock", id: "notes", zone: "right", group: "new" });
  engine.apply(layout.expanded, new Map());

  const ops: LayoutOp[] = [];
  engine.onOp((op) => ops.push(op));

  const chatGroup = engine.debugApi!.getPanel("chat")!.group;
  const notesGroup = engine.debugApi!.getPanel("notes")!.group;

  apiOf(notesGroup)._onDidDimensionChange.fire({ width: 320, height: 150 });
  apiOf(chatGroup)._onDidDimensionChange.fire({ width: 320, height: 300 });

  const resizeZoneOps = ops.filter((o): o is Extract<LayoutOp, { op: "resizeZone" }> => o.op === "resizeZone");
  const resizeGroupOps = ops.filter((o): o is Extract<LayoutOp, { op: "resizeGroup" }> => o.op === "resizeGroup");

  expect(resizeZoneOps.length).toBeGreaterThan(0);
  expect(resizeGroupOps.length).toBeGreaterThan(0);
  expect(resizeZoneOps.every((o) => o.zone === "right" && o.size === 320)).toBe(true);
  expect(resizeGroupOps.every((o) => o.zone === "right" && o.size > 0 && o.size <= 1)).toBe(true);
  // The final chat resize (height 300 of a 450 total) resolves to its exact fraction.
  const chatResize = resizeGroupOps.find((o) => Math.abs(o.size - 300 / 450) < 1e-9);
  expect(chatResize).toBeDefined();
});

test("dimension changes synchronously triggered from inside apply() are NOT emitted (guarded by #applying)", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat", "notes"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);

  let layout = defaultLayout([{ id: "chat" }, { id: "notes" }]);
  layout = applyOp(layout, { op: "dock", id: "chat", zone: "right", group: "new" });
  engine.apply(layout.expanded, new Map());

  const ops: LayoutOp[] = [];
  engine.onOp((op) => ops.push(op));

  // Piggyback on a REAL dockview event guaranteed to fire synchronously from
  // inside the next `apply()` call's own body (the active-tab reconciliation
  // — docking "notes" into the existing group activates it via
  // `activePanel.api.setActive()`) to prove the `#applying` guard closes over
  // a genuine mid-`apply()` window, not merely "nothing happened to fire".
  const unsub = engine.debugApi!.onDidActivePanelChange((event) => {
    if (event.panel?.id !== "notes") return;
    apiOf(event.panel.group)._onDidDimensionChange.fire({ width: 500, height: 500 });
  });

  layout = applyOp(layout, { op: "dock", id: "notes", zone: "right", group: 0 });
  engine.apply(layout.expanded, new Map());
  unsub.dispose();

  expect(ops.filter((o) => o.op === "resizeZone" || o.op === "resizeGroup")).toHaveLength(0);
});

test("a live drag/resize of an already-floating panel emits a resizeFloating op syncing its new Rect", async () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);

  let layout = defaultLayout([{ id: "chat" }]);
  layout = applyOp(layout, { op: "float", id: "chat", rect: { x: 10, y: 10, w: 200, h: 150 } });
  engine.apply(layout.expanded, new Map());

  const ops: LayoutOp[] = [];
  engine.onOp((op) => ops.push(op));

  // jsdom never runs real layout, so the overlay element's
  // `getBoundingClientRect` (the box `#handleFloatingLayoutChange` reads — the
  // floating window's OUTER frame) is stubbed directly to simulate the box a
  // real drag/resize gesture would leave behind.
  const overlayEl = floatingEntryOf(engine.debugApi!, "chat").overlay.element;
  overlayEl.getBoundingClientRect = () => domRectOf(50, 60, 220, 160);

  componentOf(engine.debugApi!)._bufferOnDidLayoutChange.fire();
  // `onDidLayoutChange` is dockview's `AsapEvent` — listeners run on the next microtask.
  await Promise.resolve();
  await Promise.resolve();

  expect(ops).toContainEqual({ op: "resizeFloating", id: "chat", rect: { x: 50, y: 60, w: 220, h: 160 } });
});

test("a resizeFloating op's own round trip through apply() does not re-emit (self-caused churn is suppressed)", async () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);

  let layout = defaultLayout([{ id: "chat" }]);
  layout = applyOp(layout, { op: "float", id: "chat", rect: { x: 10, y: 10, w: 200, h: 150 } });
  engine.apply(layout.expanded, new Map());

  // Round-trip the SAME rect the panel is already at back through the
  // reducer and into another `apply()` call, mirroring what the real
  // controller does with an emitted op.
  layout = applyOp(layout, { op: "resizeFloating", id: "chat", rect: { x: 10, y: 10, w: 200, h: 150 } });
  engine.apply(layout.expanded, new Map());

  const ops: LayoutOp[] = [];
  engine.onOp((op) => ops.push(op));

  const overlayEl = floatingEntryOf(engine.debugApi!, "chat").overlay.element;
  overlayEl.getBoundingClientRect = () => domRectOf(10, 10, 200, 150);

  componentOf(engine.debugApi!)._bufferOnDidLayoutChange.fire();
  await Promise.resolve();
  await Promise.resolve();

  expect(ops.filter((o) => o.op === "resizeFloating")).toHaveLength(0);
});

test("apply() repositions an already-floating widget when the tree rect changed from a non-engine source, with no resizeFloating echo", async () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);

  let layout = defaultLayout([{ id: "chat" }]);
  layout = applyOp(layout, { op: "float", id: "chat", rect: { x: 10, y: 10, w: 200, h: 150 } });
  engine.apply(layout.expanded, new Map());

  const entry = floatingEntryOf(engine.debugApi!, "chat");
  const positionSpy = vi.spyOn(entry, "position");
  // The live widget reads as sitting at the tree's rect (as a real browser
  // would after creation placed it there).
  entry.overlay.element.getBoundingClientRect = () => domRectOf(10, 10, 200, 150);

  // The tree rect then changes from a NON-engine source (a keyboard move op,
  // an arrangement restore, a layout reset — anything reduced through
  // `applyOp` rather than dragged on the widget).
  layout = applyOp(layout, { op: "resizeFloating", id: "chat", rect: { x: 42, y: 66, w: 300, h: 200 } });
  engine.apply(layout.expanded, new Map());

  // The reconcile pushed the tree's rect to the widget's overlay box.
  expect(positionSpy).toHaveBeenCalledWith({ left: 42, top: 66, width: 300, height: 200 });

  // No echo: once the widget sits at the reconciled rect, a layout-change
  // pass emits nothing back.
  const ops: LayoutOp[] = [];
  engine.onOp((op) => ops.push(op));
  entry.overlay.element.getBoundingClientRect = () => domRectOf(42, 66, 300, 200);
  componentOf(engine.debugApi!)._bufferOnDidLayoutChange.fire();
  await Promise.resolve();
  await Promise.resolve();
  expect(ops.filter((o) => o.op === "resizeFloating")).toHaveLength(0);
});

test("apply() leaves an already-floating widget untouched when the live box already matches the tree rect", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);

  let layout = defaultLayout([{ id: "chat" }]);
  layout = applyOp(layout, { op: "float", id: "chat", rect: { x: 10, y: 10, w: 200, h: 150 } });
  engine.apply(layout.expanded, new Map());

  const entry = floatingEntryOf(engine.debugApi!, "chat");
  const positionSpy = vi.spyOn(entry, "position");
  entry.overlay.element.getBoundingClientRect = () => domRectOf(10, 10, 200, 150);

  engine.apply(layout.expanded, new Map());

  expect(positionSpy).not.toHaveBeenCalled();
});

test("a genuine engine-side removal (outside apply()) still emits exactly one close op", () => {
  // Not every removal is a drop: dockview's own default tab close button
  // calls `panel.api.close()` -> `removePanel` directly, outside any
  // `apply()` window. `#handleDidRemovePanel` must still translate that one.
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat", "assets"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);
  engine.apply(twoPanelLayout().expanded, new Map());

  const ops: LayoutOp[] = [];
  engine.onOp((op) => ops.push(op));

  const assetsPanel = engine.debugApi!.getPanel("assets")!;
  engine.debugApi!.removePanel(assetsPanel);

  expect(ops.filter((o) => o.op === "close")).toHaveLength(1);
  expect(ops.find((o) => o.op === "close")).toEqual({ op: "close", id: "assets" });
});

test("custom tab: renders icon + label from the meta map, not dockview's own title", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);

  let layout = defaultLayout([{ id: "chat" }]);
  layout = applyOp(layout, { op: "dock", id: "chat", zone: "right", group: "new" });
  const meta = new Map<string, PanelMeta>([["chat", { icon: "💬", labelKey: "test.chatLabel" }]]);
  engine.apply(layout.expanded, meta);

  const tabEl = host.querySelector<HTMLElement>(".sc-tab")!;
  expect(tabEl).toBeTruthy();
  expect(tabEl.querySelector(".sc-tab-icon")!.textContent).toBe("💬");
  // "test.chatLabel" has no catalog entry — `I18n.t` falls back to the key
  // itself, which doubles as an unambiguous "this came from our own meta
  // map, not dockview's title" signal.
  expect(tabEl.querySelector(".sc-tab-label")!.textContent).toBe("test.chatLabel");
  expect(tabEl.querySelector(".sc-tab-menu-btn")).toBeTruthy();
});

class FakeBadge {
  #count = 0;
  #listeners = new Set<() => void>();
  get(): number {
    return this.#count;
  }
  set(count: number): void {
    this.#count = count;
    for (const cb of this.#listeners) cb();
  }
  subscribe(cb: () => void): () => void {
    this.#listeners.add(cb);
    return () => this.#listeners.delete(cb);
  }
}

test("custom tab: renders no badge when meta has none, and hides a badge that drops to zero", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);

  let layout = defaultLayout([{ id: "chat" }]);
  layout = applyOp(layout, { op: "dock", id: "chat", zone: "right", group: "new" });
  engine.apply(layout.expanded, new Map([["chat", { icon: "c", labelKey: "chat.tab" } as PanelMeta]]));

  const tabEl = host.querySelector<HTMLElement>(".sc-tab")!;
  const badgeEl = tabEl.querySelector<HTMLElement>(".sc-tab-badge")!;
  expect(badgeEl.hidden).toBe(true);
});

test("custom tab: renders a live badge count and updates on the badge's own subscribe, independent of apply()", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);
  const badge = new FakeBadge();

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);

  let layout = defaultLayout([{ id: "chat" }]);
  layout = applyOp(layout, { op: "dock", id: "chat", zone: "right", group: "new" });
  engine.apply(layout.expanded, new Map([["chat", { icon: "c", labelKey: "chat.tab", badge } as PanelMeta]]));

  const tabEl = host.querySelector<HTMLElement>(".sc-tab")!;
  const badgeEl = tabEl.querySelector<HTMLElement>(".sc-tab-badge")!;
  expect(badgeEl.hidden).toBe(true);

  // No `apply()` call between these two `set()`s — the tab renders the new count
  // purely from the badge's own subscribe, which is the whole point of the seam.
  badge.set(3);
  expect(badgeEl.hidden).toBe(false);
  expect(badgeEl.textContent).toBe("3");

  badge.set(0);
  expect(badgeEl.hidden).toBe(true);
});

test("custom tab: disposes its badge subscription so a stale tab never re-renders after removal", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);
  const badge = new FakeBadge();

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);

  let layout = defaultLayout([{ id: "chat" }]);
  layout = applyOp(layout, { op: "dock", id: "chat", zone: "right", group: "new" });
  engine.apply(layout.expanded, new Map([["chat", { icon: "c", labelKey: "chat.tab", badge } as PanelMeta]]));

  layout = applyOp(layout, { op: "close", id: "chat" });
  engine.apply(layout.expanded, new Map());

  expect(() => badge.set(5)).not.toThrow();
});

test("roving tabindex: ArrowRight/ArrowLeft move focus between tabs without activating; Enter activates the focused tab", () => {
  attachedHost = document.createElement("div");
  document.body.appendChild(attachedHost);
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat", "notes"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(attachedHost, slotFor, stageEl);

  let layout = defaultLayout([{ id: "chat" }, { id: "notes" }]);
  layout = applyOp(layout, { op: "dock", id: "chat", zone: "right", group: "new" });
  layout = applyOp(layout, { op: "dock", id: "notes", zone: "right", group: 0 });
  engine.apply(layout.expanded, new Map());

  const ops: LayoutOp[] = [];
  engine.onOp((op) => ops.push(op));

  // dockview's own `Tab` wrapper (`role="tab"`) is what carries the roving
  // tabindex — our custom tab component is its CONTENT, not a replacement
  // for it. `notes` docked second, so it starts active (tab order: chat, notes).
  // Filtered to tabs hosting OUR custom content: the stage's own tab (headerless
  // group, `hideHeader: true`) still exists in the DOM (just CSS-hidden), so an
  // unfiltered `[role="tab"]` query would also match it.
  const tabs = Array.from(attachedHost.querySelectorAll<HTMLElement>('[role="tab"]')).filter(
    (el) => el.querySelector(".sc-tab") !== null,
  );
  expect(tabs).toHaveLength(2);
  tabs[0].focus();
  expect(document.activeElement).toBe(tabs[0]);

  tabs[0].dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
  expect(document.activeElement).toBe(tabs[1]);
  expect(ops.some((o) => o.op === "activeTab")).toBe(false);

  tabs[1].dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }));
  expect(document.activeElement).toBe(tabs[0]);
  expect(ops.some((o) => o.op === "activeTab")).toBe(false);

  tabs[0].dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  expect(ops).toContainEqual({ op: "activeTab", zone: "right", group: 0, id: "chat" });
});

test("menu 'Float' command: the resulting floating dialog gets aria-label + focus-in, matching classifyDrop-parity op shape", async () => {
  attachedHost = document.createElement("div");
  document.body.appendChild(attachedHost);
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(attachedHost, slotFor, stageEl);

  let layout = defaultLayout([{ id: "chat" }]);
  layout = applyOp(layout, { op: "dock", id: "chat", zone: "right", group: "new" });
  const meta = new Map<string, PanelMeta>([["chat", { icon: "c", labelKey: "test.chatLabel" }]]);
  engine.apply(layout.expanded, meta);

  const ops: LayoutOp[] = [];
  engine.onOp((op) => ops.push(op));

  const menuBtn = attachedHost.querySelector<HTMLButtonElement>(".sc-tab-menu-btn")!;
  menuBtn.click();
  // `mountPanelMenu` mounts synchronously but defers the first-item focus by
  // a microtask; give both a turn.
  await Promise.resolve();
  await Promise.resolve();

  const floatItem = document.querySelector<HTMLButtonElement>('[data-testid="panel-menu-float"]')!;
  expect(floatItem).toBeTruthy();
  floatItem.click();

  const floatOp = ops.find((o) => o.op === "float");
  expect(floatOp).toBeDefined();
  layout = applyOp(layout, floatOp!);
  engine.apply(layout.expanded, meta);

  const dialogEl = attachedHost.querySelector<HTMLElement>('[role="dialog"]');
  expect(dialogEl).toBeTruthy();
  expect(dialogEl!.getAttribute("aria-label")).toBe(i18n.t("panels.floatingDialog", { panel: "test.chatLabel" }));
  expect(document.activeElement).toBe(dialogEl);

  // Escape (bubbled from anywhere inside the dialog) closes it via the same
  // op channel a menu/drag gesture uses.
  dialogEl!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  expect(ops.find((o) => o.op === "close")).toEqual({ op: "close", id: "chat" });

  // Applying that close removes "chat" entirely (its own docked tab — the
  // only current trigger for a menu-driven float — is gone by this point,
  // so there is no live invoker element to restore focus to); the teardown
  // must still run cleanly rather than leaving focus on the disposed dialog.
  layout = applyOp(layout, { op: "close", id: "chat" });
  engine.apply(layout.expanded, meta);
  expect(document.activeElement).not.toBe(dialogEl);
});

/** Mounts an engine with one panel floating at a known rect, returning the
 * dialog wrapper + the emitted-op log. Each `press` round-trips the emitted
 * op through the reducer + `apply()`, exactly as the real controller loop
 * does, so successive keystrokes accumulate off the updated tree rect. */
function floatingKeyboardSetup(): {
  dialogEl: HTMLElement;
  ops: LayoutOp[];
  press: (init: KeyboardEventInit & { key: string }) => void;
} {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);

  const eng = new DockviewEngine(silentLogger);
  engine = eng;
  eng.init(host, slotFor, stageEl);

  let layout = defaultLayout([{ id: "chat" }]);
  layout = applyOp(layout, { op: "float", id: "chat", rect: { x: 10, y: 10, w: 200, h: 150 } });
  eng.apply(layout.expanded, new Map());

  const ops: LayoutOp[] = [];
  eng.onOp((op) => ops.push(op));

  const dialogEl = host.querySelector<HTMLElement>('[role="dialog"]')!;
  const press = (init: KeyboardEventInit & { key: string }): void => {
    const before = ops.length;
    dialogEl.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, ...init }));
    const op = ops.length > before ? ops[ops.length - 1] : null;
    if (op) {
      layout = applyOp(layout, op);
      eng.apply(layout.expanded, new Map());
    }
  };
  return { dialogEl, ops, press };
}

test("floating dialog keyboard: arrows move 8px (Shift 32px), Ctrl+arrows resize the bottom/right edges (Ctrl+Shift 32px)", () => {
  const { ops, press } = floatingKeyboardSetup();

  press({ key: "ArrowRight" });
  expect(ops[0]).toEqual({ op: "resizeFloating", id: "chat", rect: { x: 18, y: 10, w: 200, h: 150 } });
  press({ key: "ArrowRight" }); // accumulates off the round-tripped tree rect
  expect(ops[1]).toEqual({ op: "resizeFloating", id: "chat", rect: { x: 26, y: 10, w: 200, h: 150 } });
  press({ key: "ArrowDown", shiftKey: true });
  expect(ops[2]).toEqual({ op: "resizeFloating", id: "chat", rect: { x: 26, y: 42, w: 200, h: 150 } });
  press({ key: "ArrowLeft" });
  expect(ops[3]).toEqual({ op: "resizeFloating", id: "chat", rect: { x: 18, y: 42, w: 200, h: 150 } });

  press({ key: "ArrowRight", ctrlKey: true });
  expect(ops[4]).toEqual({ op: "resizeFloating", id: "chat", rect: { x: 18, y: 42, w: 208, h: 150 } });
  press({ key: "ArrowDown", ctrlKey: true, shiftKey: true });
  expect(ops[5]).toEqual({ op: "resizeFloating", id: "chat", rect: { x: 18, y: 42, w: 208, h: 182 } });
  press({ key: "ArrowUp", ctrlKey: true });
  expect(ops[6]).toEqual({ op: "resizeFloating", id: "chat", rect: { x: 18, y: 42, w: 208, h: 174 } });
  press({ key: "ArrowLeft", ctrlKey: true });
  expect(ops[7]).toEqual({ op: "resizeFloating", id: "chat", rect: { x: 18, y: 42, w: 200, h: 174 } });
});

test("floating dialog keyboard: resize clamps at the minimum size instead of reaching zero/negative", () => {
  const { ops, press } = floatingKeyboardSetup();

  // 200 - 13*8 = 96 would undershoot the 100px floor; the clamp pins at 100.
  for (let i = 0; i < 15; i++) press({ key: "ArrowLeft", ctrlKey: true });
  const last = ops[ops.length - 1];
  expect(last).toEqual({ op: "resizeFloating", id: "chat", rect: { x: 10, y: 10, w: 100, h: 150 } });
});

test("floating dialog keyboard: a keydown whose target is inside the dialog's content (an input) emits no op", () => {
  const { dialogEl, ops } = floatingKeyboardSetup();

  const input = document.createElement("input");
  dialogEl.appendChild(input);
  input.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
  input.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true, ctrlKey: true }));

  expect(ops.filter((o) => o.op === "resizeFloating")).toHaveLength(0);
  dialogEl.removeChild(input);
});

test("Tab on a panel-menu item closes the popup but does NOT force focus back to the tab's menu button (APG Menu Button pattern)", async () => {
  attachedHost = document.createElement("div");
  document.body.appendChild(attachedHost);
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(attachedHost, slotFor, stageEl);

  let layout = defaultLayout([{ id: "chat" }]);
  layout = applyOp(layout, { op: "dock", id: "chat", zone: "right", group: "new" });
  engine.apply(layout.expanded, new Map());

  const menuBtn = attachedHost.querySelector<HTMLButtonElement>(".sc-tab-menu-btn")!;
  menuBtn.click();
  await Promise.resolve();
  await Promise.resolve();

  const item = document.querySelector<HTMLButtonElement>('[data-testid="panel-menu-dockRight"]')!;
  expect(item).toBeTruthy();
  const tabEvent = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true });
  item.dispatchEvent(tabEvent);
  // Native Tab traversal is not intercepted: preventDefault must not be called.
  expect(tabEvent.defaultPrevented).toBe(false);

  // The popup unmounts (its item is no longer in the document).
  expect(document.contains(item)).toBe(false);
  // ...but focus was not forced back onto the invoking tab's menu button.
  expect(document.activeElement).not.toBe(menuBtn);
});

test("docked->floating preserves the #floatInvokers entry across the transient remove/re-add; a later close returns focus to it once the invoker is live again, and degrades gracefully when it stays detached (the self-referential case)", async () => {
  attachedHost = document.createElement("div");
  document.body.appendChild(attachedHost);
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(attachedHost, slotFor, stageEl);

  let layout = defaultLayout([{ id: "chat" }]);
  layout = applyOp(layout, { op: "dock", id: "chat", zone: "right", group: "new" });
  engine.apply(layout.expanded, new Map());

  const ops: LayoutOp[] = [];
  engine.onOp((op) => ops.push(op));

  const menuBtn = attachedHost.querySelector<HTMLButtonElement>(".sc-tab-menu-btn")!;
  menuBtn.click();
  await Promise.resolve();
  await Promise.resolve();
  const floatItem = document.querySelector<HTMLButtonElement>('[data-testid="panel-menu-float"]')!;
  floatItem.click();

  const floatOp = ops.find((o) => o.op === "float");
  expect(floatOp).toBeDefined();
  layout = applyOp(layout, floatOp!);
  engine.apply(layout.expanded, new Map());

  // dockview tears down the outgoing docked tab's DOM (including its own
  // menu button) synchronously as part of the docked->floating transient
  // remove/re-add, BEFORE `onDidRemovePanel` even fires — the self-
  // referential trigger's invoker is therefore already gone by the time any
  // teardown logic runs, regardless of the `#floatTransitionIds` guard. This
  // is the graceful-degradation half of the self-referential case, not a
  // regression to guard against.
  expect(document.contains(menuBtn)).toBe(false);

  // Simulate a hypothetical non-self-referential invoker (e.g. a future
  // command-palette button, which the docked->floating churn never
  // destroys) by reattaching the SAME element reference the engine
  // recorded. This only lands on a LIVE element at close time if
  // `#floatInvokers`'s entry for "chat" actually survived the transient
  // churn intact: without the `#floatTransitionIds` guard, `#teardownFloatingA11y`
  // would delete that entry mid-churn (see `#floatTransitionIds`'s doc
  // comment), and no later close could ever recover the reference to
  // reattach here.
  document.body.appendChild(menuBtn);

  const dialogEl = attachedHost.querySelector<HTMLElement>('[role="dialog"]')!;
  dialogEl.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  expect(ops.find((o) => o.op === "close")).toEqual({ op: "close", id: "chat" });

  layout = applyOp(layout, { op: "close", id: "chat" });
  engine.apply(layout.expanded, new Map());

  expect(document.activeElement).toBe(menuBtn);
});

test("destroy() clears #floatInvokers and disposes+clears #floatingEscapeSubs", async () => {
  attachedHost = document.createElement("div");
  document.body.appendChild(attachedHost);
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(attachedHost, slotFor, stageEl);

  let layout = defaultLayout([{ id: "chat" }]);
  layout = applyOp(layout, { op: "dock", id: "chat", zone: "right", group: "new" });
  engine.apply(layout.expanded, new Map());

  const menuBtn = attachedHost.querySelector<HTMLButtonElement>(".sc-tab-menu-btn")!;
  menuBtn.click();
  await Promise.resolve();
  await Promise.resolve();
  const floatItem = document.querySelector<HTMLButtonElement>('[data-testid="panel-menu-float"]')!;
  floatItem.click();

  const ops: LayoutOp[] = [];
  engine.onOp((op) => ops.push(op));
  const floatOp = ops.find((o) => o.op === "float") ?? { op: "float" as const, id: "chat", rect: { x: 0, y: 0, w: 1, h: 1 } };
  layout = applyOp(layout, floatOp);
  engine.apply(layout.expanded, new Map());

  const dialogEl = attachedHost.querySelector<HTMLElement>('[role="dialog"]')!;
  expect(dialogEl).toBeTruthy();

  // destroy() while a floating dialog + its invoker bookkeeping are still
  // live must not leak either map, nor leave the Escape listener attached to
  // a dialog element that no longer belongs to any engine.
  engine.destroy();
  engine = null; // afterEach's own destroy() must not double-dispose.

  dialogEl.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  expect(ops.filter((o) => o.op === "close")).toHaveLength(0);
});

test("the stage's own tab never renders a .sc-tab-menu-btn — no menu-command affordance exists for it to invoke", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);
  engine.apply(twoPanelLayout().expanded, new Map());

  const stagePanel = engine.debugApi!.getPanel(STAGE_ID)!;
  expect(stagePanel.group.id).toBe("sc-stage-group");
  // The stage group is headerless: no tab strip renders for it at all,
  // so no `.sc-tab-menu-btn` for the stage exists anywhere in the host.
  const stageGroupEl = stagePanel.group.element;
  expect(stageGroupEl.querySelector(".sc-tab-menu-btn")).toBeNull();
});

/** Mounts an engine on a body-attached host with one docked panel and clicks
 * its tab menu's "Pop out" item, returning the ops emitted. `driver` stands in
 * for `addPopoutGroup` (jsdom has no real `window.open`). */
async function popOutViaMenu(driver: (panel: IDockviewPanel, options?: DockviewPopoutGroupOptions) => Promise<boolean>): Promise<{ ops: LayoutOp[]; notices: string[] }> {
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

test("pop-out: a successful driver emits a popOut op carrying a minted window key (no float, no notice)", async () => {
  const { ops, notices } = await popOutViaMenu(() => Promise.resolve(true));
  expect(ops).toContainEqual(expect.objectContaining({ op: "popOut", id: "chat", key: expect.any(String), rect: null }));
  expect(ops.some((o) => o.op === "float")).toBe(false);
  expect(notices).toEqual([]);
});

test("pop-out blocked: a false driver falls back to a float op + a notice", async () => {
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

/** Mounts an engine, docks chat, and starts a pop-out whose driver relocates
 * the panel SYNCHRONOUSLY (the way `addPopoutGroup` really does) but resolves
 * only when the test says so — leaving the gesture in flight while the tree
 * still records the panel's pre-popout location. Returns the pieces a test
 * needs to apply the stale tree and then settle the gesture. */
async function popOutInFlight(initial: "docked" | "floating"): Promise<{
  api: DockviewApi;
  ops: LayoutOp[];
  stale: PanelLayoutV1;
  settle: (ok: boolean) => void;
}> {
  const host = document.createElement("div");
  document.body.appendChild(host);
  attachedHost = host;
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);
  let settle: (ok: boolean) => void = () => {};
  const driver = (panel: IDockviewPanel): Promise<boolean> => {
    const api = engine!.debugApi!;
    const group = api.addGroup({ id: "sc-inflight-popout", direction: "right" });
    api.removePanel(panel);
    api.addPanel({ id: panel.id, component: "sc-panel", position: { referenceGroup: group.id, direction: "within" } });
    return new Promise<boolean>((res) => {
      settle = res;
    });
  };
  engine = new DockviewEngine(silentLogger, driver);
  engine.init(host, slotFor, stageEl);

  let l = defaultLayout([{ id: "chat" }]);
  l = applyOp(l, initial === "docked"
    ? { op: "dock", id: "chat", zone: "right", group: "new" }
    : { op: "float", id: "chat", rect: { x: 10, y: 10, w: 300, h: 200 } });
  const meta = new Map([["chat", { icon: "c", labelKey: "chat.tab" } as PanelMeta]]);
  engine.apply(l.expanded, meta);

  const ops: LayoutOp[] = [];
  engine.onOp((op) => ops.push(op));

  host.querySelector<HTMLButtonElement>(".sc-tab-menu-btn")?.click();
  document.querySelector<HTMLButtonElement>('[data-testid="panel-menu-popOut"]')?.click();
  return { api: engine.debugApi!, ops, stale: l, settle };
}

test("a stale-tree apply during an in-flight pop-out never relocates a docked-origin panel back", async () => {
  const { api, ops, stale, settle } = await popOutInFlight("docked");
  // The driver's synchronous relocation already happened; the tree applied
  // next is the PRE-popOut tree (the `popOut` op lands only when the driver
  // settles) — the apply must not drag the panel back into its old group.
  expect(api.getPanel("chat")?.group.id).toBe("sc-inflight-popout");
  engine!.apply(stale.expanded, new Map([["chat", { icon: "c", labelKey: "chat.tab" } as PanelMeta]]));
  expect(api.getPanel("chat")?.group.id).toBe("sc-inflight-popout");
  expect(ops.some((o) => o.op === "popOut")).toBe(false);

  settle(true);
  await Promise.resolve();
  await Promise.resolve();
  expect(ops).toContainEqual(expect.objectContaining({ op: "popOut", id: "chat", key: expect.any(String) }));
});

test("a stale-tree apply during an in-flight pop-out never re-docks a floating-origin panel", async () => {
  const { api, stale, settle } = await popOutInFlight("floating");
  expect(api.getPanel("chat")?.group.api.location.type).not.toBe("floating");
  engine!.apply(stale.expanded, new Map([["chat", { icon: "c", labelKey: "chat.tab" } as PanelMeta]]));
  expect(api.getPanel("chat")?.group.id).toBe("sc-inflight-popout");
  settle(true);
  await Promise.resolve();
  await Promise.resolve();
});

test("apply seeds seenPanelIds with the tree's popout windows so a live popout is never orphan-removed", () => {
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

  l = applyOp(l, { op: "popOut", id: "chat", key: "w-chat", rect: null });
  engine.apply(l.expanded, new Map());
  // The panel is NOT torn out of dockview's model by the orphan-removal loop.
  expect(engine.debugApi?.getPanel("chat")).toBeTruthy();
});

test("a duplicate pop-out request on the same id before the first settles invokes the driver only once", async () => {
  const host = document.createElement("div");
  document.body.appendChild(host);
  attachedHost = host;
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);

  // A driver whose promise is resolved manually, so a second click lands while
  // the first request is still in flight (the async window.open → re-parent gap).
  let resolveDriver: (ok: boolean) => void = () => {};
  let driverCalls = 0;
  const driver = (): Promise<boolean> => {
    driverCalls += 1;
    return new Promise<boolean>((res) => {
      resolveDriver = res;
    });
  };

  engine = new DockviewEngine(silentLogger, driver);
  engine.init(host, slotFor, stageEl);
  const ops: LayoutOp[] = [];
  engine.onOp((op) => ops.push(op));

  let l = defaultLayout([{ id: "chat" }]);
  l = applyOp(l, { op: "dock", id: "chat", zone: "right", group: "new" });
  engine.apply(l.expanded, new Map([["chat", { icon: "c", labelKey: "chat.tab" } as PanelMeta]]));

  // Clicking the "Pop out" item closes the popover but leaves the tab
  // renderer's toggle latch set; a fresh open therefore needs the menu button
  // clicked until the item actually re-appears.
  const clickPopOut = (): void => {
    const menuBtn = host.querySelector<HTMLButtonElement>(".sc-tab-menu-btn")!;
    menuBtn.click();
    if (!document.querySelector('[data-testid="panel-menu-popOut"]')) menuBtn.click();
    document.querySelector<HTMLButtonElement>('[data-testid="panel-menu-popOut"]')!.click();
  };

  clickPopOut();
  clickPopOut(); // second request while the first is still pending

  // Load-bearing: the in-flight guard blocked the second driver invocation.
  expect(driverCalls).toBe(1);

  resolveDriver(true);
  await Promise.resolve();
  await Promise.resolve();

  // Exactly one popOut op emitted (one request ever reached the driver).
  expect(ops.filter((o) => o.op === "popOut")).toHaveLength(1);
});

test("a successful pop-out seeds its origin group so the next apply() does not orphan-remove it", async () => {
  await popOutViaMenu(() => Promise.resolve(true));
  const api = engine!.debugApi!;
  const originGroupId = "sc-group:chat";

  // dockview's real `addPopoutGroup` keeps the origin group alive-but-hidden
  // while emptying it (the panel now lives in the popout window). The injected
  // driver can't produce that state, so reproduce it via the component's
  // options-accepting `removePanel` (the public `DockviewApi.removePanel`
  // forwards no options): strip the panel WITHOUT disposing the now-empty
  // group — matching dockview's origin-group-survives design.
  const chat = api.getPanel("chat")!;
  (api as unknown as { component: { removePanel(p: unknown, o: object): void } }).component.removePanel(
    chat,
    { removeEmptyGroup: false, skipDispose: true },
  );
  expect(api.getGroup(originGroupId)?.model.panels.length).toBe(0);

  // Reconcile the SAME popped-out tree: chat still marked popped-out.
  let l = defaultLayout([{ id: "chat" }]);
  l = applyOp(l, { op: "dock", id: "chat", zone: "right", group: "new" });
  l = applyOp(l, { op: "popOut", id: "chat", key: "w-chat", rect: null });
  engine!.apply(l.expanded, new Map());

  // The empty origin group is seeded into seenGroupIds and survives the
  // orphan-group loop (without the seeding fix it would be removeGroup'd).
  expect(api.getGroup(originGroupId)).toBeTruthy();
});

/** Fires dockview's real `onDidRemovePopoutGroup` event by reaching into the
 * component's internal emitter (`_onDidRemovePopoutGroup`, a plain field —
 * not `#`-private — on `DockviewComponent`; `DockviewApi#onDidRemovePopoutGroup`
 * is just `component.onDidRemovePopoutGroup`, its `.event` accessor). jsdom has
 * no real `window.open`/popout-window lifecycle to drive this event from a
 * genuine drag-out-a-window gesture, so — mirroring the existing
 * `component.removePanel(...)` reach-in used above for the origin-group-seed
 * test — this drives the SAME event shape (`{id, group, window}`, built by
 * `DockviewComponent`'s constructor's `popoutWindowService.onDidRemove`
 * wiring) that wiring itself would fire, directly at `DockviewEngine#handleRemovePopoutGroup`. */
function fireRemovePopoutGroup(api: DockviewApi, id: string, group: IDockviewGroupPanel): void {
  (
    api as unknown as {
      component: { _onDidRemovePopoutGroup: { fire(e: { id: string; group: IDockviewGroupPanel; window: null }): void } };
    }
  ).component._onDidRemovePopoutGroup.fire({ id, group, window: null });
}

test("onDidRemovePopoutGroup (user-closed) emits one popIn per tracked member and clears tracking maps", async () => {
  const { ops } = await popOutViaMenu(() => Promise.resolve(true));
  const api = engine!.debugApi!;
  const groupId = "sc-group:chat";
  // The stub driver never calls dockview's real `addPopoutGroup`, so "chat"
  // never actually moves group — `#requestPopOut` still records it as popped
  // out under its (unchanged) current group id, giving a real, populated
  // tracking-map entry to exercise the removal handler's tracked-lookup path.
  expect(engine!.debugPoppedOutGroupPanels.get(groupId)).toEqual(["chat"]);
  expect(engine!.debugPoppedOutOriginGroups.get("chat")).toBe(groupId);
  ops.length = 0; // drop the setup `popOut` op — only the removal-handler's own ops matter below

  fireRemovePopoutGroup(api, groupId, api.getGroup(groupId)!);

  expect(ops).toEqual([{ op: "popIn", id: "chat" }]);
  expect(engine!.debugPoppedOutGroupPanels.has(groupId)).toBe(false);
  expect(engine!.debugPoppedOutOriginGroups.has("chat")).toBe(false);
});

test("onDidRemovePopoutGroup falls back to live group membership for an untracked group id", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);
  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);

  const ops: LayoutOp[] = [];
  engine.onOp((op) => ops.push(op));

  let l = defaultLayout([{ id: "chat" }]);
  l = applyOp(l, { op: "dock", id: "chat", zone: "right", group: "new" });
  engine.apply(l.expanded, new Map());

  const api = engine.debugApi!;
  const groupId = "sc-group:chat";
  // Never went through the pop-out flow, so `#poppedOutGroupPanels` has no
  // entry for this id — exercises the `event.group.model.panels` fallback
  // directly, distinct from the tracked-map path above.
  fireRemovePopoutGroup(api, groupId, api.getGroup(groupId)!);

  expect(ops).toEqual([{ op: "popIn", id: "chat" }]);
});

test("onDidRemovePopoutGroup skips a group whose sole (fallback-resolved) member is the stage panel", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots([]);
  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);

  const ops: LayoutOp[] = [];
  engine.onOp((op) => ops.push(op));

  const api = engine.debugApi!;
  // The stage's own group id (`STAGE_GROUP_ID`, not exported by the `dockview` module; mirrors the
  // literal already asserted against in "a tree naming the stage id in a
  // zone group applies without throwing, and the real stage stays alive in its own
  // locked group"). The stage's
  // group is never tracked in `#poppedOutGroupPanels` (the stage is never
  // poppable), so this also exercises the fallback lookup — resolving to
  // `[STAGE_ID]` — which the loop's `id === STAGE_ID` guard must then skip
  // rather than emit a `popIn` for the stage.
  const stageGroupId = "sc-stage-group";
  fireRemovePopoutGroup(api, stageGroupId, api.getGroup(stageGroupId)!);

  expect(ops).toEqual([]);
});

test("onDidRemovePopoutGroup fired mid-apply() (our own reconcile) suppresses popIn but still clears tracking maps", async () => {
  const { ops } = await popOutViaMenu(() => Promise.resolve(true));
  const api = engine!.debugApi!;
  const groupId = "sc-group:chat";
  const group = api.getGroup(groupId)!;
  ops.length = 0;

  // Simulates the top-risk scenario: a "dock" command on a
  // popped-out panel causes dockview to remove the popout group as a side
  // effect of `apply()`'s own reconcile. `#applying` is true for the whole
  // synchronous duration of `apply()`, so patching `api.addGroup` (which the
  // zone loop below calls synchronously to create the new "assets" group)
  // lets this fire the removal event from squarely inside that window,
  // without needing a real dockview popout-window lifecycle.
  let firedDuringApply = false;
  const originalAddGroup = api.addGroup.bind(api);
  (api as unknown as { addGroup: typeof api.addGroup }).addGroup = ((...args: Parameters<typeof api.addGroup>) => {
    if (!firedDuringApply) {
      firedDuringApply = true;
      fireRemovePopoutGroup(api, groupId, group);
    }
    return originalAddGroup(...args);
  }) as typeof api.addGroup;

  let l = defaultLayout([{ id: "chat" }, { id: "assets" }]);
  l = applyOp(l, { op: "dock", id: "chat", zone: "right", group: "new" });
  l = applyOp(l, { op: "popOut", id: "chat", key: "w-chat", rect: null });
  l = applyOp(l, { op: "dock", id: "assets", zone: "bottom", group: "new" });
  engine!.apply(l.expanded, new Map());

  expect(firedDuringApply).toBe(true);
  // Suppressed: no `popIn` op emitted for a removal `apply()` itself caused.
  expect(ops.some((o) => o.op === "popIn")).toBe(false);
  // Cleanup still runs unconditionally, regardless of the `#applying` guard.
  expect(engine!.debugPoppedOutGroupPanels.has(groupId)).toBe(false);
  expect(engine!.debugPoppedOutOriginGroups.has("chat")).toBe(false);
});

/** Mounts an engine on a body-attached host with one docked panel and pops it
 * out via a driver that moves the panel into a genuinely NEW group
 * (`popoutGroupId`), the way real dockview's `addPopoutGroup` does — unlike
 * the stub driver `popOutViaMenu` uses elsewhere in this file, which resolves
 * `true` without moving anything, leaving `panel.group.id` equal to its
 * ORIGINAL zone-managed group (already tracked by `#groupWillDropSubs`). A
 * test exercising `#popoutGroupSubs`'s OWN wiring needs a group that wiring
 * alone reaches — reusing the original group could not tell the two apart.
 * Returns the live api and the popout group itself, for a caller to fire
 * further group-model events against directly. */
async function popOutToRealGroup(popoutGroupId: string): Promise<{ api: DockviewApi; group: IDockviewGroupPanel; ops: LayoutOp[] }> {
  const host = document.createElement("div");
  document.body.appendChild(host);
  attachedHost = host;
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat", "notes"]);
  const driver = (panel: IDockviewPanel): Promise<boolean> => {
    const api = engine!.debugApi!;
    const group = api.addGroup({ id: popoutGroupId, direction: "right" });
    api.removePanel(panel);
    api.addPanel({ id: panel.id, component: "sc-panel", position: { referenceGroup: group.id, direction: "within" } });
    return Promise.resolve(true);
  };
  engine = new DockviewEngine(silentLogger, driver);
  engine.init(host, slotFor, stageEl);

  let l = defaultLayout([{ id: "chat" }, { id: "notes" }]);
  l = applyOp(l, { op: "dock", id: "chat", zone: "right", group: "new" });
  engine.apply(l.expanded, new Map([["chat", { icon: "c", labelKey: "chat.tab" } as PanelMeta]]));

  const ops: LayoutOp[] = [];
  engine.onOp((op) => ops.push(op));

  const menuBtn = host.querySelector<HTMLButtonElement>(".sc-tab-menu-btn");
  menuBtn?.click();
  const popOutItem = document.querySelector<HTMLButtonElement>('[data-testid="panel-menu-popOut"]');
  popOutItem?.click();
  await Promise.resolve();
  await Promise.resolve();

  const api = engine.debugApi!;
  const group = api.getGroup(popoutGroupId)!;
  return { api, group, ops };
}

test("popout veto-bypass closed: a drop targeting an open popout group's own group model is intercepted (defaultPrevented) via the popout group's own onWillDrop wire", async () => {
  // Discrimination: with the `onWillDrop` entry removed from
  // `#requestPopOut`'s `#popoutGroupSubs` wiring, this test fails —
  // `prevented` stays `false`, because nothing subscribes to this popout
  // group's model `_onWillDrop` emitter and the event is never intercepted.
  // Manually verified by temporarily deleting that one subscription line and
  // confirming the failure, then restoring it.
  const { group } = await popOutToRealGroup("sc-real-popout-group");

  const ops: LayoutOp[] = [];
  engine!.onOp((op) => ops.push(op));

  let prevented = false;
  const event: WillDropProbe = {
    kind: "content",
    position: "center",
    panel: undefined,
    group,
    getData: () => ({ viewId: "v", groupId: "sc-real-popout-group", panelId: "chat" }),
    get defaultPrevented() {
      return prevented;
    },
    preventDefault() {
      prevented = true;
    },
  };
  modelOf(group)._onWillDrop.fire(event);

  expect(prevented).toBe(true);
});

test("popout panel list grows: dragging a second panel into an open popout group's own gridview updates debugPoppedOutGroupPanels to include it", async () => {
  const { api, group } = await popOutToRealGroup("sc-real-popout-group");

  // A real drop of a second panel into the popout group's own nested
  // gridview — dockview-core natively accepts this drop target and fires the
  // group model's own `onDidAddPanel`, which `#popoutGroupSubs` now listens
  // for.
  api.addPanel({ id: "notes", component: "sc-panel", position: { referenceGroup: group.id, direction: "within" } });

  expect(engine!.debugPoppedOutGroupPanels.get("sc-real-popout-group")).toEqual(["chat", "notes"]);
});

test("popout panel list shrinks: removing one of two panels from an open popout group drops just that id, keeping the other", async () => {
  const { api, group } = await popOutToRealGroup("sc-real-popout-group");
  api.addPanel({ id: "notes", component: "sc-panel", position: { referenceGroup: group.id, direction: "within" } });
  expect(engine!.debugPoppedOutGroupPanels.get("sc-real-popout-group")).toEqual(["chat", "notes"]);

  api.removePanel(api.getPanel("notes")!);

  expect(engine!.debugPoppedOutGroupPanels.get("sc-real-popout-group")).toEqual(["chat"]);
});

test("popout group subscriptions are disposed on window close: a later onDidAddPanel on the same (now-detached) group model no longer updates tracking", async () => {
  const { api, group } = await popOutToRealGroup("sc-real-popout-group");

  fireRemovePopoutGroup(api, "sc-real-popout-group", group);
  expect(engine!.debugPoppedOutGroupPanels.has("sc-real-popout-group")).toBe(false);

  // Fire the group model's own internal onDidAddPanel emitter directly (the
  // group object itself still exists as a JS value even though dockview's
  // removal path has already torn it out of the live api) — a leaked
  // subscription would resurrect a `sc-real-popout-group` entry here.
  modelOf(group)._onDidAddPanel.fire({ panel: api.getPanel("chat")! });

  expect(engine!.debugPoppedOutGroupPanels.has("sc-real-popout-group")).toBe(false);
});

/** A `popoutDriver` that stands in for the window-opening half of dockview's
 * `addPopoutGroup`: like the stub driver `popOutViaMenu` uses elsewhere it
 * moves nothing, but it fires the options' `onDidOpen` callback the way
 * dockview's `PopoutWindow.open` does — synchronously after the window opens,
 * carrying the fresh `Window` — so the engine's pop-out success path sees the
 * popout `Document` it must register with the ui-kit theme controller. */
function themePopoutDriver(popoutDoc: Document): (panel: IDockviewPanel, options?: DockviewPopoutGroupOptions) => Promise<boolean> {
  return (_panel, options) => {
    options?.onDidOpen?.({ id: "sc-popout-window", window: { document: popoutDoc } as unknown as Window });
    return Promise.resolve(true);
  };
}

/** Spies on the ui-kit `theme` singleton's `registerDocument`, calling through
 * (so registration really happens and the unregister really detaches) while
 * counting unregister invocations — both halves of the register/unregister
 * pairing become assertable. */
function spyOnThemeRegistration(): { spy: ReturnType<typeof vi.spyOn>; unregisterCount: () => number } {
  let calls = 0;
  const original = theme.registerDocument.bind(theme);
  const spy = vi.spyOn(theme, "registerDocument").mockImplementation((doc: Document) => {
    const unregister = original(doc);
    return () => {
      calls += 1;
      unregister();
    };
  });
  return { spy, unregisterCount: () => calls };
}

test("a successful pop-out registers the popout document with the ui-kit theme; window close unregisters it", async () => {
  const popoutDoc = document.implementation.createHTMLDocument("popout");
  const { spy, unregisterCount } = spyOnThemeRegistration();
  try {
    await popOutViaMenu(themePopoutDriver(popoutDoc));

    // Exactly one registration, for the popout window's own Document, and the
    // call-through applied the resolved theme inline.
    expect(spy).toHaveBeenCalledTimes(1);
    expect(spy).toHaveBeenCalledWith(popoutDoc);
    expect(popoutDoc.documentElement.style.getPropertyValue("--surface-base")).not.toBe("");
    expect(unregisterCount()).toBe(0);

    const api = engine!.debugApi!;
    const groupId = "sc-group:chat";
    fireRemovePopoutGroup(api, groupId, api.getGroup(groupId)!);
    expect(unregisterCount()).toBe(1);
  } finally {
    spy.mockRestore();
  }
});

test("destroy() unregisters a pop-out document still open at engine teardown", async () => {
  const popoutDoc = document.implementation.createHTMLDocument("popout");
  const { spy, unregisterCount } = spyOnThemeRegistration();
  try {
    await popOutViaMenu(themePopoutDriver(popoutDoc));
    expect(spy).toHaveBeenCalledTimes(1);

    engine!.destroy();
    expect(unregisterCount()).toBe(1);
  } finally {
    spy.mockRestore();
  }
});

test("a pop-out whose driver never fires onDidOpen registers no document with the theme", async () => {
  const spy = vi.spyOn(theme, "registerDocument");
  try {
    await popOutViaMenu(() => Promise.resolve(true));
    expect(spy).not.toHaveBeenCalled();
  } finally {
    spy.mockRestore();
  }
});

/** Mounts an engine with one FLOATING panel plus a dormant arrangement record
 * naming it (the post-reload shape: rehydrated to floating, window retained as
 * a dormant record), then clicks the tab menu's "Pop out" item. Returns the
 * emitted ops, the positions the driver saw, and the notices. */
async function popOutFloatingWithDormantRecord(
  savedRect: { left: number; top: number; width: number; height: number },
): Promise<{ ops: LayoutOp[]; positions: unknown[] }> {
  const host = document.createElement("div");
  document.body.appendChild(host);
  attachedHost = host;
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);
  const positions: unknown[] = [];
  const driver = (_panel: IDockviewPanel, options?: DockviewPopoutGroupOptions): Promise<boolean> => {
    positions.push(options?.position);
    return Promise.resolve(true);
  };
  engine = new DockviewEngine(silentLogger, driver);
  engine.init(host, slotFor, stageEl);

  const ops: LayoutOp[] = [];
  engine.onOp((op) => ops.push(op));

  let l = defaultLayout([{ id: "chat" }]);
  l = applyOp(l, { op: "float", id: "chat", rect: { x: 10, y: 10, w: 200, h: 150 } });
  l = {
    ...l,
    expanded: { ...l.expanded, popouts: [{ key: "w-old", panels: ["chat"], rect: savedRect, dormant: true }] },
  };
  engine.apply(l.expanded, new Map([["chat", { icon: "c", labelKey: "chat.tab" } as PanelMeta]]));

  host.querySelector<HTMLButtonElement>(".sc-tab-menu-btn")?.click();
  document.querySelector<HTMLButtonElement>('[data-testid="panel-menu-popOut"]')?.click();
  await Promise.resolve();
  await Promise.resolve();
  return { ops, positions };
}

test("menu pop-out reuses the panel's saved popout rect from a dormant record (position passed to the driver, rect carried on the op)", async () => {
  const saved = { left: 100, top: 40, width: 900, height: 700 };
  const { ops, positions } = await popOutFloatingWithDormantRecord(saved);
  expect(positions).toEqual([saved]);
  expect(ops).toContainEqual({ op: "popOut", id: "chat", key: expect.any(String), rect: saved });
});

/** Stubs `window.screen`'s available-bounds properties (read-only getters in
 * jsdom, which reports a degenerate all-zero screen) for the duration of a
 * clamp assertion; returns a restore function. */
function stubAvailableScreen(width: number, height: number): () => void {
  const screen = window.screen;
  const originals: Record<string, PropertyDescriptor | undefined> = {};
  for (const [key, value] of Object.entries({ availWidth: width, availHeight: height, availLeft: 0, availTop: 0 })) {
    originals[key] = Object.getOwnPropertyDescriptor(screen, key);
    Object.defineProperty(screen, key, { value, configurable: true });
  }
  return () => {
    for (const [key, desc] of Object.entries(originals)) {
      if (desc) Object.defineProperty(screen, key, desc);
    }
  };
}

test("menu pop-out with a saved rect off the current screen clamps the position to the available bounds", async () => {
  const restoreScreen = stubAvailableScreen(1024, 768);
  try {
    // Deliberately outside the stubbed 1024x768 available screen on every axis.
    const { positions } = await popOutFloatingWithDormantRecord({ left: 5000, top: -80, width: 2000, height: 100 });
    expect(positions).toEqual([{ left: 0, top: 0, width: 1024, height: 100 }]);
  } finally {
    restoreScreen();
  }
});

test("restorePopouts re-opens a saved window: first panel pops out at the saved rect, the rest move into the popout group via popOutInto", async () => {
  const host = document.createElement("div");
  document.body.appendChild(host);
  attachedHost = host;
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat", "notes"]);
  const positions: unknown[] = [];
  // A driver behaving like dockview's real `addPopoutGroup`: relocate the
  // panel into a genuinely new group (mirrors the `popOutToRealGroup`
  // helper's approach — the stub-true driver leaves the panel in its
  // original group, which cannot exercise the move-into-popout half).
  const driver = (panel: IDockviewPanel, options?: DockviewPopoutGroupOptions): Promise<boolean> => {
    positions.push(options?.position);
    const api = engine!.debugApi!;
    const group = api.addGroup({ id: "sc-restored-popout", direction: "right" });
    api.removePanel(panel);
    api.addPanel({ id: panel.id, component: "sc-panel", position: { referenceGroup: group.id, direction: "within" } });
    return Promise.resolve(true);
  };
  engine = new DockviewEngine(silentLogger, driver);
  engine.init(host, slotFor, stageEl);

  let l = defaultLayout([{ id: "chat" }, { id: "notes" }]);
  l = applyOp(l, { op: "float", id: "chat", rect: { x: 10, y: 10, w: 200, h: 150 } });
  l = applyOp(l, { op: "float", id: "notes", rect: { x: 38, y: 38, w: 200, h: 150 } });
  engine.apply(l.expanded, new Map());

  const ops: LayoutOp[] = [];
  engine.onOp((op) => ops.push(op));

  const rect = { left: 100, top: 40, width: 900, height: 700 };
  engine.restorePopouts!([{ key: "w1", panels: ["chat", "notes"], rect }]);
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();

  // One window opened (the first panel only), at the saved rect…
  expect(positions).toEqual([rect]);
  // …and the second panel moved INTO that popout group.
  const api = engine!.debugApi!;
  expect(api.getPanel("chat")!.group.id).toBe("sc-restored-popout");
  expect(api.getPanel("notes")!.group.id).toBe("sc-restored-popout");
  // Ops revive the retained record under its ORIGINAL key, then join the rest.
  expect(ops).toContainEqual({ op: "popOut", id: "chat", key: "w1", rect });
  expect(ops).toContainEqual({ op: "popOutInto", id: "notes", key: "w1" });
  // notes' move out of its floating group is engine-driven — never a close op.
  // (The driver's own removePanel of "chat" is a test-harness artifact; a real
  // `addPopoutGroup` moves the panel under dockview's `movingLock`, which
  // suppresses the component-level removal event.)
  expect(ops.some((o) => o.op === "close" && o.id === "notes")).toBe(false);
  // The popout group's tracked membership covers both panels (via the
  // success-branch record plus the group model's own add event).
  expect(engine!.debugPoppedOutGroupPanels.get("sc-restored-popout")).toEqual(["chat", "notes"]);
});

test("restorePopouts tolerates partial records: panels with no live panel or already popped out are skipped, the rest still restore", async () => {
  const host = document.createElement("div");
  document.body.appendChild(host);
  attachedHost = host;
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);
  const driver = (panel: IDockviewPanel): Promise<boolean> => {
    const api = engine!.debugApi!;
    const group = api.addGroup({ id: "sc-restored-partial", direction: "right" });
    api.removePanel(panel);
    api.addPanel({ id: panel.id, component: "sc-panel", position: { referenceGroup: group.id, direction: "within" } });
    return Promise.resolve(true);
  };
  engine = new DockviewEngine(silentLogger, driver);
  engine.init(host, slotFor, stageEl);

  let l = defaultLayout([{ id: "chat" }]);
  l = applyOp(l, { op: "float", id: "chat", rect: { x: 10, y: 10, w: 200, h: 150 } });
  engine.apply(l.expanded, new Map());

  const ops: LayoutOp[] = [];
  engine.onOp((op) => ops.push(op));

  // "ghost" has no live panel (unregistered/closed since the save); the
  // restore falls through to "chat" as the window's first panel.
  engine.restorePopouts!([{ key: "w1", panels: ["ghost", "chat"], rect: null }]);
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();

  expect(ops).toContainEqual({ op: "popOut", id: "chat", key: "w1", rect: null });
  expect(ops.some((o) => o.op === "popOut" && o.id === "ghost")).toBe(false);
});

test("restorePopouts on an empty arrangement (or before init) is a no-op", () => {
  const engineFresh = new DockviewEngine(silentLogger);
  engineFresh.restorePopouts!([{ key: "w1", panels: ["chat"], rect: null }]); // no init — must not throw
  engineFresh.destroy();
});

/** Stubs `DockviewApi.getPopouts` to report one entry for `group` whose live
 * popup `Window` reports `geometry` — jsdom has no real popup, so the entry's
 * window carries the values a real one would. Returns the spy's restore
 * function. */
function stubPopoutEntry(
  api: DockviewApi,
  group: IDockviewGroupPanel,
  geometry: { screenX: number; screenY: number; innerWidth: number; innerHeight: number },
): () => void {
  const popupWindow = geometry as unknown as Window;
  const spy = vi
    .spyOn(api, "getPopouts")
    .mockReturnValue([{ id: group.id, group, window: popupWindow }] as unknown as ReturnType<DockviewApi["getPopouts"]>);
  return () => spy.mockRestore();
}

test("popout geometry capture: a window move emits updatePopoutGeometry read from the popout entry's own window, never the event payload", async () => {
  const { api, group, ops } = await popOutToRealGroup("sc-geometry-move");
  const restore = stubPopoutEntry(api, group, { screenX: 640, screenY: 220, innerWidth: 900, innerHeight: 700 });
  try {
    // The vendored position event's payload is corrupt by construction
    // (`screenY` populated from `screenX`) — both axes deliberately disagree
    // with the window's real geometry, so a passing assertion proves the
    // payload is never read.
    componentOf(api)._onDidPopoutGroupPositionChange.fire({ screenX: 1, screenY: 1, group });
    expect(ops).toContainEqual({
      op: "updatePopoutGeometry",
      key: expect.any(String),
      rect: { left: 640, top: 220, width: 900, height: 700 },
    });
  } finally {
    restore();
  }
});

test("popout geometry capture: a window resize emits updatePopoutGeometry read from the entry's window (payload ignored)", async () => {
  const { api, group, ops } = await popOutToRealGroup("sc-geometry-resize");
  const restore = stubPopoutEntry(api, group, { screenX: 10, screenY: 20, innerWidth: 1024, innerHeight: 640 });
  try {
    componentOf(api)._onDidPopoutGroupSizeChange.fire({ width: 5, height: 5, group });
    expect(ops).toContainEqual({
      op: "updatePopoutGeometry",
      key: expect.any(String),
      rect: { left: 10, top: 20, width: 1024, height: 640 },
    });
  } finally {
    restore();
  }
});

test("popout geometry capture: an event for a group with no tracked window key emits nothing", async () => {
  const { api, group, ops } = await popOutToRealGroup("sc-geometry-untracked");
  const other = api.groups.find((g) => g.id !== group.id)!;
  componentOf(api)._onDidPopoutGroupPositionChange.fire({ screenX: 1, screenY: 1, group: other });
  componentOf(api)._onDidPopoutGroupSizeChange.fire({ width: 5, height: 5, group: other });
  expect(ops.some((o) => o.op === "updatePopoutGeometry")).toBe(false);
});

test("popout geometry capture: an event whose popout entry is already gone from getPopouts (window closed mid-delivery) emits nothing", async () => {
  const { api, group, ops } = await popOutToRealGroup("sc-geometry-gone");
  const spy = vi.spyOn(api, "getPopouts").mockReturnValue([]);
  try {
    componentOf(api)._onDidPopoutGroupPositionChange.fire({ screenX: 1, screenY: 1, group });
    expect(ops.some((o) => o.op === "updatePopoutGeometry")).toBe(false);
  } finally {
    spy.mockRestore();
  }
});

