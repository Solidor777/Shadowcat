import { test, expect, afterEach } from "vitest";
import { defaultLayout, applyOp, type LayoutOp, type PanelLayoutV1 } from "../layout/tree";
import { DockviewEngine } from "./dockview";
import { STAGE_ID } from "./policy";
import { silentLogger, type PanelMeta } from "@shadowcat/core";
import type { DockviewApi, DockviewWillDropEvent, IDockviewGroupPanel } from "dockview-core";

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
    group: unknown;
  }>,
): { defaultPrevented: boolean } {
  let prevented = false;
  const event = {
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
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (engine.debugApi as any).component._onWillDrop.fire(event);
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
  // production (mirrors the file's existing `_onDidDimensionChange.fire`
  // pattern for testing a real dockview event without a native drag gesture).
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
  const event = {
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
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (chatGroup.model as any)._onWillDrop.fire(event);

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
  const event = {
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
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (chatGroup.model as any)._onWillDrop.fire(event);

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
  const event = {
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
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (chatGroup.model as any)._onWillDrop.fire(event);

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

  const event = {
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
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (chatGroup.model as any)._onWillDrop.fire(event);

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

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (notesGroup.api as any)._onDidDimensionChange.fire({ width: 320, height: 150 });
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (chatGroup.api as any)._onDidDimensionChange.fire({ width: 320, height: 300 });

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
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (event.panel.group.api as any)._onDidDimensionChange.fire({ width: 500, height: 500 });
  });

  layout = applyOp(layout, { op: "dock", id: "notes", zone: "right", group: 0 });
  engine.apply(layout.expanded, new Map());
  unsub.dispose();

  expect(ops.filter((o) => o.op === "resizeZone" || o.op === "resizeGroup")).toHaveLength(0);
});

test("F3: a live drag/resize of an already-floating panel emits a resizeFloating op syncing its new Rect", async () => {
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

  // jsdom never runs real layout, so `boundingBox`'s `getBoundingClientRect`
  // reads are stubbed directly on the floating group's element to simulate
  // the box a real drag/resize gesture would leave behind.
  const groupEl = engine.debugApi!.getPanel("chat")!.group.element;
  groupEl.getBoundingClientRect = () =>
    ({ left: 50, top: 60, width: 220, height: 160, right: 270, bottom: 220, x: 50, y: 60, toJSON: () => ({}) }) as DOMRect;

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (engine.debugApi as any).component._bufferOnDidLayoutChange.fire();
  // `onDidLayoutChange` is dockview's `AsapEvent` — listeners run on the next microtask.
  await Promise.resolve();
  await Promise.resolve();

  expect(ops).toContainEqual({ op: "resizeFloating", id: "chat", rect: { x: 50, y: 60, w: 220, h: 160 } });
});

test("F3: a resizeFloating op's own round trip through apply() does not re-emit (self-caused churn is suppressed)", async () => {
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

  const groupEl = engine.debugApi!.getPanel("chat")!.group.element;
  groupEl.getBoundingClientRect = () =>
    ({ left: 10, top: 10, width: 200, height: 150, right: 210, bottom: 160, x: 10, y: 10, toJSON: () => ({}) }) as DOMRect;

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (engine.debugApi as any).component._bufferOnDidLayoutChange.fire();
  await Promise.resolve();
  await Promise.resolve();

  expect(ops.filter((o) => o.op === "resizeFloating")).toHaveLength(0);
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
  expect(dialogEl!.getAttribute("aria-label")).toBe("test.chatLabel");
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

  // Reconcile the SAME popped-out tree: chat still marked poppedOut.
  let l = defaultLayout([{ id: "chat" }]);
  l = applyOp(l, { op: "dock", id: "chat", zone: "right", group: "new" });
  l = applyOp(l, { op: "popOut", id: "chat" });
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
  l = applyOp(l, { op: "popOut", id: "chat" });
  l = applyOp(l, { op: "dock", id: "assets", zone: "bottom", group: "new" });
  engine!.apply(l.expanded, new Map());

  expect(firedDuringApply).toBe(true);
  // Suppressed: no `popIn` op emitted for a removal `apply()` itself caused.
  expect(ops.some((o) => o.op === "popIn")).toBe(false);
  // Cleanup still runs unconditionally, regardless of the `#applying` guard.
  expect(engine!.debugPoppedOutGroupPanels.has(groupId)).toBe(false);
  expect(engine!.debugPoppedOutOriginGroups.has("chat")).toBe(false);
});
