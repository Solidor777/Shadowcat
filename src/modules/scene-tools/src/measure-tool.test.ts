import { test, expect } from "vitest";
import { DocumentStore, AssetResolver, buildSceneDoc, buildTokenDoc, type WireOperation } from "@shadowcat/core";
import type { Point } from "@shadowcat/render";
import { SceneInteractionBridge, TokenSelection } from "@shadowcat/ui-kit";
import { fakeSceneHost } from "@shadowcat/ui-kit/test";
import { makeMeasureTool, ToolController, type ToolContext } from "./controller.svelte";

const ev = {} as PointerEvent;

/** Drain the microtask queue so async pathfind stubs resolve. */
function flush(): Promise<void> {
  return new Promise((r) => setTimeout(r, 0));
}

function setup() {
  const measures: Array<{ from: Point; to: Point; label: string }> = [];
  let cleared = 0;
  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({
    gridDistance: () => 3,
    drawMeasure: (from, to, label) => measures.push({ from, to, label }),
    clearMeasure: () => { cleared++; },
  }));
  const sent: WireOperation[][] = [];
  const ctx: ToolContext = { scene: bridge, dispatchIntent: (ops) => sent.push(ops), documents: new DocumentStore(), assets: new AssetResolver(), world: "w1", sendPing: () => {} };
  return { tool: makeMeasureTool(ctx), measures, sent, clears: () => cleared };
}

test("measuring draws the distance label and persists nothing", () => {
  const { tool, measures, sent, clears } = setup();
  expect(tool.onPointerDown({ x: 0, y: 0 }, ev)).toBe(true);
  tool.onPointerMove({ x: 300, y: 0 }, ev);
  expect(measures.at(-1)).toEqual({ from: { x: 0, y: 0 }, to: { x: 300, y: 0 }, label: "3" });
  tool.onPointerUp({ x: 300, y: 0 }, ev);
  expect(clears()).toBe(1);
  expect(sent).toHaveLength(0); // client-local: no document, no broadcast
});

// --- Route-mode tests ---

/** Build a ToolContext with a scene + token seeded, a single token selected, and a pathfind stub. */
function setupRoute(over: {
  pathfind?: ToolContext["pathfind"];
  tokenIds?: string[];
} = {}) {
  const docs = new DocumentStore();
  // Scene with grid.distance so the budget label can be computed.
  docs.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [
      {
        op: "create",
        doc: buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: { perCell: 5, unit: "ft" } } }, "s1"),
      },
      {
        op: "create",
        doc: buildTokenDoc("w1", "s1", { x: 50, y: 50, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" } }, "tok-1"),
      },
    ],
  });

  const overlays: unknown[][] = [];
  const measures: Array<{ label: string }> = [];
  let measureClears = 0;
  let overlayClears = 0;

  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({
    snap: (p: Point) => p,
    gridDistance: () => 1,
    previewOverlay: (shapes) => overlays.push([...shapes]),
    clearOverlay: () => { overlayClears++; },
    drawMeasure: (_f, _t, label) => measures.push({ label }),
    clearMeasure: () => { measureClears++; },
  }));

  const tokenIds = over.tokenIds ?? ["tok-1"];
  const sel = new TokenSelection();
  sel.set(tokenIds);

  const ctx: ToolContext = {
    scene: bridge,
    dispatchIntent: () => {},
    documents: docs,
    assets: new AssetResolver(),
    world: "w1",
    sendPing: () => {},
    tokenSelection: sel,
    pathfind: over.pathfind,
  };

  return { tool: makeMeasureTool(ctx), overlays, measures, overlayClears: () => overlayClears, measureClears: () => measureClears };
}

test("measure tool routes via pathfind for the selected token and previews the path", async () => {
  const pathfind: ToolContext["pathfind"] = async () => ({
    path: [[50, 50], [150, 50]] as [number, number][],
    cost: 2,
  });
  const { tool, overlays, measures } = setupRoute({ pathfind });

  tool.onPointerDown({ x: 50, y: 50 }, ev);
  tool.onPointerMove({ x: 150, y: 50 }, ev);
  await flush(); // allow the async pathfind to resolve

  expect(overlays.length).toBeGreaterThan(0); // a routed polyline was previewed
  expect(measures.length).toBeGreaterThan(0);
  expect(measures.at(-1)!.label).toContain("10 ft"); // budget = cost(2) × perCell(5)
});

test("measure tool falls back to plain anchor-point measure when no token is selected", () => {
  const pathfinderCalled: boolean[] = [];
  const pathfind: ToolContext["pathfind"] = async () => {
    pathfinderCalled.push(true);
    return { path: [], cost: 0 };
  };
  // Build the route context but override tokenIds to empty — no selection.
  const { tool, overlays } = setupRoute({ pathfind, tokenIds: [] });

  tool.onPointerDown({ x: 0, y: 0 }, ev);
  tool.onPointerMove({ x: 100, y: 0 }, ev);
  tool.onPointerUp({ x: 100, y: 0 }, ev);

  expect(pathfinderCalled).toHaveLength(0); // fallback: plain measure, no pathfind
  expect(overlays).toHaveLength(0);         // no overlay in plain-measure mode
});

test("measure tool clears overlay and measure label on pointer up (mid-gesture-clear)", async () => {
  const pathfind: ToolContext["pathfind"] = async () => ({
    path: [[50, 50], [150, 50]] as [number, number][],
    cost: 2,
  });
  const { tool, overlayClears, measureClears } = setupRoute({ pathfind });

  tool.onPointerDown({ x: 50, y: 50 }, ev);
  tool.onPointerMove({ x: 150, y: 50 }, ev);
  await flush();
  tool.onPointerUp({ x: 150, y: 50 }, ev);

  expect(overlayClears()).toBeGreaterThan(0);   // overlay cleared on release
  expect(measureClears()).toBeGreaterThan(0);   // measure label cleared on release
});

test("measure tool with no pathfind function falls back to plain measure", () => {
  // No pathfind: undefined means the seam is absent (e.g. older host).
  const measures: Array<{ from: Point; to: Point; label: string }> = [];
  let cleared = 0;
  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({
    gridDistance: () => 4,
    drawMeasure: (from, to, label) => measures.push({ from, to, label }),
    clearMeasure: () => { cleared++; },
  }));
  const sel = new TokenSelection();
  sel.set(["tok-1"]);
  const ctx: ToolContext = {
    scene: bridge,
    dispatchIntent: () => {},
    documents: new DocumentStore(),
    assets: new AssetResolver(),
    world: "w1",
    sendPing: () => {},
    tokenSelection: sel,
    // pathfind intentionally omitted — defensive fallback
  };
  const tool = makeMeasureTool(ctx);

  tool.onPointerDown({ x: 0, y: 0 }, ev);
  tool.onPointerMove({ x: 200, y: 0 }, ev);
  expect(measures.at(-1)?.label).toBe("4"); // plain gridDistance label
  tool.onPointerUp({ x: 200, y: 0 }, ev);
  expect(cleared).toBe(1);
});

test("measure tool onDeactivate clears route overlay on tool swap (mid-gesture-clear on toggle)", async () => {
  // Verifies the ToolController.toggle contract: when switching away from measure mid-gesture,
  // onDeactivate fires and clears the live overlay + budget label. We test onDeactivate
  // directly because ToolController.toggle calls tool.onDeactivate?.() before swapping.
  const { tool, overlayClears, measureClears } = setupRoute({
    pathfind: async () => ({ path: [[50, 50], [150, 50]] as [number, number][], cost: 2 }),
  });

  tool.onPointerDown({ x: 50, y: 50 }, ev);
  tool.onPointerMove({ x: 150, y: 50 }, ev);
  await flush();

  const before = overlayClears() + measureClears();
  tool.onDeactivate!();
  expect(overlayClears() + measureClears()).toBeGreaterThan(before);
});

test("ToolController.toggle fires onDeactivate on outgoing measure tool", () => {
  // End-to-end: toggle measure → toggle ping → ToolController must call onDeactivate on
  // the outgoing measure tool, which in turn calls clearRoute (clearOverlay + clearMeasure).
  const docs = new DocumentStore();
  docs.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [
      { op: "create", doc: buildSceneDoc("w1", { grid: { kind: "square", size: 100 } }, "s1") },
    ],
  });

  let overlayCleared = 0;
  let measureCleared = 0;
  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({
    clearOverlay: () => { overlayCleared++; },
    clearMeasure: () => { measureCleared++; },
  }));

  const ctx: ToolContext = {
    scene: bridge,
    dispatchIntent: () => {},
    documents: docs,
    assets: new AssetResolver(),
    world: "w1",
    sendPing: () => {},
  };

  const controller = new ToolController(ctx);
  controller.toggle("measure"); // activate measure tool

  const before = overlayCleared + measureCleared;
  controller.toggle("ping");   // swap to ping — must fire measure.onDeactivate
  // clearRoute calls clearOverlay + clearMeasure, so the combined count must increase.
  expect(overlayCleared + measureCleared).toBeGreaterThan(before);
});

test("measure tool accumulates multiple waypoints and passes them to pathfind in order", async () => {
  // Verifies that two onPointerDown clicks build [wp1, wp2, goal] for the pathfind call.
  const docs = new DocumentStore();
  docs.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [
      { op: "create", doc: buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: { perCell: 5, unit: "ft" } } }, "s1") },
      { op: "create", doc: buildTokenDoc("w1", "s1", { x: 50, y: 50, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" } }, "tok-1") },
    ],
  });

  const calls: { start: [number,number]; waypoints: [number,number][] }[] = [];
  const pathfind: ToolContext["pathfind"] = async (_, start, waypoints) => {
    calls.push({ start, waypoints: [...waypoints] });
    return { path: [[start[0], start[1]], [waypoints.at(-1)![0], waypoints.at(-1)![1]]] as [number,number][], cost: waypoints.length };
  };

  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({ snap: (p: Point) => p, previewOverlay: () => {}, clearOverlay: () => {}, drawMeasure: () => {}, clearMeasure: () => {} }));

  const sel = new TokenSelection();
  sel.set(["tok-1"]);
  const ctx: ToolContext = { scene: bridge, dispatchIntent: () => {}, documents: docs, assets: new AssetResolver(), world: "w1", sendPing: () => {}, tokenSelection: sel, pathfind };

  const tool = makeMeasureTool(ctx);

  // Click waypoint 1 at (100,50), waypoint 2 at (150,50), then hover to goal (200,50).
  tool.onPointerDown({ x: 100, y: 50 }, ev); // wp1 pushed
  tool.onPointerDown({ x: 150, y: 50 }, ev); // wp2 pushed
  tool.onPointerMove({ x: 200, y: 50 }, ev); // triggers pathfind([100,50],[150,50],[200,50])
  await flush();

  expect(calls).toHaveLength(1);
  // start is the token center (50,50), NOT the first waypoint.
  expect(calls[0].start).toEqual([50, 50]);
  // waypoints must be [wp1, wp2, goal] in order.
  expect(calls[0].waypoints).toEqual([[100, 50], [150, 50], [200, 50]]);
});
