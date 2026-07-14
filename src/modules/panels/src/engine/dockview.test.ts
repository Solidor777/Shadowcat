import { test, expect, afterEach } from "vitest";
import { defaultLayout, applyOp, type LayoutOp, type PanelLayoutV1 } from "../layout/tree";
import { DockviewEngine } from "./dockview";
import { STAGE_ID } from "./policy";
import { silentLogger, type PanelMeta } from "@shadowcat/core";
import type { DockviewWillDropEvent } from "dockview-core";

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

test("W3: programmatic removal of the stage panel leaves a live stage panel", () => {
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
  // W3 must still survive.
  api!.removePanel(stagePanel!);

  const restored = api!.getPanel(STAGE_ID);
  expect(restored).toBeDefined();
  expect(restored!.id).toBe(STAGE_ID);
});

test("focus() and apply() ignore the stage id (W2 adapter-level guard)", () => {
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
    group: unknown;
  }>,
): { defaultPrevented: boolean } {
  let prevented = false;
  const event = {
    kind: overrides.kind ?? "edge",
    position: overrides.position ?? "top",
    panel: undefined,
    group: overrides.group,
    getData: () => ({ viewId: "v", groupId: "g", panelId: overrides.panelId ?? null }),
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

test("Finding 1+2: a whole-group transfer (panelId null) at the container's TOP edge is vetoed", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);
  engine.apply(twoPanelLayout().expanded, new Map());

  const event = fireWillDrop(engine, { kind: "edge", position: "top", panelId: null });
  expect(event.defaultPrevented).toBe(true);
});

test("Finding 1+2: a whole-group transfer at a zone-edge position is ALSO vetoed (v1 vetoes every group transfer, not just top)", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);
  engine.apply(twoPanelLayout().expanded, new Map());

  const event = fireWillDrop(engine, { kind: "edge", position: "right", panelId: null });
  expect(event.defaultPrevented).toBe(true);
});

test("Finding 5: a will-drop event before any apply() fails closed (defaultPrevented)", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);
  // Deliberately no `apply()` call — `#expanded` is still null.

  const event = fireWillDrop(engine, { kind: "edge", position: "top", panelId: "chat" });
  expect(event.defaultPrevented).toBe(true);
});

test("Finding 4: a tree naming the stage id in a zone group applies without throwing, and the real stage stays alive in its own locked group", () => {
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

test("Finding 4 (mixed): a zone group naming BOTH the stage id and a real panel places the real panel and leaves the stage untouched", () => {
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);

  let layout = defaultLayout([{ id: "chat" }]);
  layout = applyOp(layout, { op: "dock", id: "chat", zone: "right", group: "new" });
  // A MIXED group — the stage id sits alongside a real panel id in the same
  // group's tabs, unlike Finding 4's all-stage case (which skips the whole
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

test("group-onto-group: a whole-group transfer targeting an existing group's content is vetoed via the per-group onWillDrop wire", () => {
  // Regression test for the residual the fix-confirmation buddy-check
  // flagged: `DockviewApi.onWillDrop` (subscribed once in `init()`) NEVER
  // fires for a drop targeting an existing group — the component only
  // forwards a group model's own `onWillDrop` through the permanently-unwired
  // `_advancedDnDService` optional chain (`dockviewComponent.ts:4592-4594`).
  // This exercises the mechanism that actually closes the gap: a per-group
  // subscription to `group.model.onWillDrop` (`#groupWillDropSubs`, wired in
  // `apply()`), fired here via the group model's own private `_onWillDrop`
  // emitter — the SAME emitter `group.model.onWillDrop(cb)` subscribes to in
  // production (mirrors the file's existing `_onDidDimensionChange.fire`
  // pattern for testing a real dockview event without a native drag gesture).
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  const slotFor = makeSlots(["chat", "assets"]);

  engine = new DockviewEngine(silentLogger);
  engine.init(host, slotFor, stageEl);
  engine.apply(twoPanelLayout().expanded, new Map());

  const chatGroup = engine.debugApi!.getPanel("chat")!.group;

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

test("Finding 3: a group's live dimension change emits resizeZone + resizeGroup ops with sane values", () => {
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

test("Finding 3: dimension changes synchronously triggered from inside apply() are NOT emitted (guarded by #applying)", () => {
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
