// @vitest-environment node
// Exercises plain state and pure functions: no component render and no DOM API use.
import { test, expect } from "vitest";
import { DocumentStore, AssetResolver, buildSceneDoc, buildTokenDoc } from "@shadowcat/core";
import type { Point } from "@shadowcat/render";
import { SceneInteractionBridge, TokenSelection, ShareRulerPreference } from "@shadowcat/ui-kit";
import { fakeSceneHost } from "@shadowcat/ui-kit/test";
import { makeMeasureTool, type ToolContext } from "./controller.svelte";
import { RULER_SEND_INTERVAL_MS, RULER_KEEPALIVE_MS } from "./ruler-share";

/** Stub PointerEvent for tests that need to pass an event object. */
function ev(): PointerEvent {
  return {} as PointerEvent;
}

/** Drain the microtask queue so async pathfind stubs resolve. */
function flush(): Promise<void> {
  return new Promise((r) => setTimeout(r, 0));
}

interface Sent {
  scene: string;
  points: [number, number][];
  active: boolean;
}

/** A measure tool over a scene `s1` with a logical clock, recording every shared-ruler send.
 * `route` selects route mode (one token selected + a pathfind stub) versus the plain measure. */
function setup(over: { route?: ToolContext["pathfind"]; share?: boolean } = {}) {
  const docs = new DocumentStore();
  docs.applyCommand({
    seq: 1,
    world_id: "w1",
    author: "a",
    ts: 0,
    ops: [
      { op: "create", doc: buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: { perCell: 5, unit: "ft" } } }, "s1") },
      { op: "create", doc: buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: { perCell: 5, unit: "ft" } } }, "s2") },
      {
        op: "create",
        doc: buildTokenDoc("w1", "s1", { x: 50, y: 50, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null, lock_rotation: false }, "tok-1"),
      },
    ],
  });
  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({ snap: (p: Point) => p, gridDistance: () => 1 }));
  const sel = new TokenSelection();
  if (over.route) sel.set(["tok-1"]);

  let t = 1_000_000;
  const timers: Array<{ id: number; at: number; fn: () => void }> = [];
  let nextId = 1;
  const sent: Sent[] = [];
  const viewed = { id: "s1" };
  const shareRuler = new ShareRulerPreference("u1");
  shareRuler.set(over.share ?? true);
  const ctx: ToolContext = {
    scene: bridge,
    dispatchIntent: () => {},
    documents: docs,
    assets: new AssetResolver(),
    world: "w1",
    role: "gm",
    sendPing: () => {},
    tokenSelection: sel,
    pathfind: over.route,
    shareRuler,
    viewedSceneId: () => viewed.id,
    sendRulerUpdate: (scene, points, active) => sent.push({ scene, points, active }),
    now: () => t,
    scheduleTimeout: (fn, ms) => {
      const id = nextId++;
      timers.push({ id, at: t + ms, fn });
      return id;
    },
    clearScheduledTimeout: (h) => {
      const i = timers.findIndex((x) => x.id === h);
      if (i >= 0) timers.splice(i, 1);
    },
    t: (k) => k,
  };
  const advance = (ms: number): void => {
    const target = t + ms;
    for (;;) {
      timers.sort((a, b) => a.at - b.at);
      const due = timers[0];
      if (!due || due.at > target) break;
      timers.shift();
      t = due.at;
      due.fn();
    }
    t = target;
  };
  return { tool: makeMeasureTool(ctx), sent, shareRuler, advance, sel, viewed };
}

/** A route that bends, so a test comparing the shared points to it cannot pass on the straight
 * anchor-to-goal segment the plain measure would send. */
const BENT: [number, number][] = [[50, 50], [150, 50], [150, 150], [250, 150]];
const bentPathfind: ToolContext["pathfind"] = async () => ({ path: BENT, cost: 4, arrested: false, truncated: false, budgetCells: null });

test("with sharing off (the default) a route drag and release send nothing", async () => {
  const { tool, sent } = setup({ route: bentPathfind, share: false });
  tool.onPointerDown({ x: 50, y: 50 }, ev());
  tool.onPointerMove({ x: 250, y: 150 }, ev());
  await flush();
  tool.onPointerUp({ x: 250, y: 150 }, ev());
  expect(sent).toEqual([]);
});

test("with sharing on, the previewed ROUTE (not the straight segment) is sent, and release ends it", async () => {
  const { tool, sent } = setup({ route: bentPathfind });
  tool.onPointerDown({ x: 50, y: 50 }, ev());
  tool.onPointerMove({ x: 250, y: 150 }, ev());
  await flush();
  expect(sent).toEqual([{ scene: "s1", points: BENT, active: true }]);

  tool.onPointerUp({ x: 250, y: 150 }, ev());
  expect(sent.at(-1)).toEqual({ scene: "s1", points: [], active: false });
});

test("the plain measure shares its anchor-to-cursor segment while sharing is on", () => {
  const { tool, sent } = setup();
  tool.onPointerDown({ x: 10, y: 20 }, ev());
  tool.onPointerMove({ x: 300, y: 40 }, ev());
  expect(sent).toEqual([{ scene: "s1", points: [[10, 20], [300, 40]], active: true }]);
  tool.onPointerUp({ x: 300, y: 40 }, ev());
  expect(sent.at(-1)).toEqual({ scene: "s1", points: [], active: false });
});

test("a burst of moves is coalesced to the interval and the newest cursor wins", () => {
  const { tool, sent, advance } = setup();
  tool.onPointerDown({ x: 0, y: 0 }, ev());
  tool.onPointerMove({ x: 100, y: 0 }, ev());
  tool.onPointerMove({ x: 200, y: 0 }, ev());
  tool.onPointerMove({ x: 300, y: 0 }, ev());
  expect(sent).toHaveLength(1);
  advance(RULER_SEND_INTERVAL_MS);
  expect(sent).toHaveLength(2);
  expect(sent[1].points).toEqual([[0, 0], [300, 0]]);
});

test("switching sharing off mid-drag ends the shared ruler while the local preview continues", () => {
  const { tool, sent, shareRuler } = setup();
  tool.onPointerDown({ x: 0, y: 0 }, ev());
  tool.onPointerMove({ x: 100, y: 0 }, ev());
  shareRuler.set(false);
  tool.onPointerMove({ x: 200, y: 0 }, ev());
  expect(sent.map((s) => s.active)).toEqual([true, false]);
});

test("deactivating the tool mid-drag ends the shared ruler", () => {
  const { tool, sent } = setup();
  tool.onPointerDown({ x: 0, y: 0 }, ev());
  tool.onPointerMove({ x: 100, y: 0 }, ev());
  tool.onDeactivate?.();
  expect(sent.at(-1)).toEqual({ scene: "s1", points: [], active: false });
});

test("an unreachable route ends the shared ruler instead of leaving the last route shown", async () => {
  let calls = 0;
  const flaky: ToolContext["pathfind"] = async () => {
    calls++;
    if (calls === 1) return { path: BENT, cost: 4, arrested: false, truncated: false, budgetCells: null };
    throw new Error("unreachable");
  };
  const { tool, sent, advance } = setup({ route: flaky });
  tool.onPointerDown({ x: 50, y: 50 }, ev());
  tool.onPointerMove({ x: 250, y: 150 }, ev());
  await flush();
  expect(sent.map((s) => s.active)).toEqual([true]);
  advance(1000); // past the route-preview debounce
  tool.onPointerMove({ x: 900, y: 900 }, ev());
  await flush();
  expect(sent.at(-1)).toEqual({ scene: "s1", points: [], active: false });
});

test("an unchanged shared ruler is kept alive while the drag is held still", () => {
  const { tool, sent, advance } = setup();
  tool.onPointerDown({ x: 0, y: 0 }, ev());
  tool.onPointerMove({ x: 100, y: 0 }, ev());
  advance(2500);
  const active = sent.filter((s) => s.active);
  expect(active.length).toBeGreaterThanOrEqual(3);
  expect(active.every((s) => JSON.stringify(s.points) === "[[0,0],[100,0]]")).toBe(true);
});

test("switching sharing off with the pointer held still ends the ruler on the keepalive clock alone, once", () => {
  const { tool, sent, shareRuler, advance } = setup();
  tool.onPointerDown({ x: 0, y: 0 }, ev());
  tool.onPointerMove({ x: 100, y: 0 }, ev());
  shareRuler.set(false);
  advance(RULER_KEEPALIVE_MS * 4); // no further pointer event
  expect(sent.map((s) => s.active)).toEqual([true, false]);
});

test("sharing off from the start sends nothing on any path, including the clock", () => {
  const { tool, sent, advance } = setup({ share: false });
  tool.onPointerDown({ x: 0, y: 0 }, ev());
  tool.onPointerMove({ x: 100, y: 0 }, ev());
  advance(RULER_KEEPALIVE_MS * 4);
  tool.onPointerUp({ x: 100, y: 0 }, ev());
  tool.onDeactivate?.();
  expect(sent).toEqual([]);
});

test("a route drag whose token selection empties mid-drag still ends the shared ruler on release, and no keepalive follows", async () => {
  const { tool, sent, sel, advance } = setup({ route: bentPathfind });
  tool.onPointerDown({ x: 50, y: 50 }, ev());
  tool.onPointerMove({ x: 250, y: 150 }, ev());
  await flush();
  expect(sent.map((s) => s.active)).toEqual([true]);
  sel.set([]); // route mode lapses: the release takes the plain fallback with no plain anchor
  tool.onPointerUp({ x: 250, y: 150 }, ev());
  expect(sent.map((s) => s.active)).toEqual([true, false]);
  advance(RULER_KEEPALIVE_MS * 5);
  expect(sent).toHaveLength(2);
});

test("a viewed-scene change mid-drag ends the ruler on the scene it was sent on at the next timer, with no pointer event", () => {
  const { tool, sent, viewed, advance } = setup();
  tool.onPointerDown({ x: 0, y: 0 }, ev());
  tool.onPointerMove({ x: 100, y: 0 }, ev());
  viewed.id = "s2";
  advance(RULER_KEEPALIVE_MS * 4);
  expect(sent.map((s) => [s.scene, s.active])).toEqual([["s1", true], ["s1", false]]);
});
