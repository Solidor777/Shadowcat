import { test, expect, afterEach } from "vitest";
import { defaultLayout, applyOp, type PanelLayoutV1 } from "../layout/tree";
import { DockviewEngine } from "./dockview";
import { STAGE_ID } from "./policy";
import { silentLogger } from "@shadowcat/core";

let engine: DockviewEngine | null = null;

afterEach(() => {
  engine?.destroy();
  engine = null;
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
