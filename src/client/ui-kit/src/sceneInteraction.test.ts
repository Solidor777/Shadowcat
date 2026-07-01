import { test, expect } from "vitest";
import type { SceneTool, SceneToolHost, Point } from "@shadowcat/render";
import { SceneInteractionBridge } from "./sceneInteraction";
import { fakeSceneHost } from "./__fixtures__/fakeSceneHost";

const tool: SceneTool = { onPointerDown: () => true, onPointerMove: () => {}, onPointerUp: () => {} };

function fakeHost(): SceneToolHost & { tools: (SceneTool | null)[]; drags: (string | null)[] } {
  const tools: (SceneTool | null)[] = [];
  const drags: (string | null)[] = [];
  return Object.assign(
    fakeSceneHost({
      setActiveTool: (t) => tools.push(t),
      snap: (p: Point) => ({ x: p.x + 1, y: p.y + 1 }),
      setDraggingToken: (id) => drags.push(id),
    }),
    { tools, drags },
  );
}

test("a detached bridge no-ops and snaps to identity", () => {
  const bridge = new SceneInteractionBridge();
  expect(() => bridge.setActiveTool(tool)).not.toThrow();
  expect(() => bridge.setDraggingToken("t1")).not.toThrow();
  expect(bridge.snap({ x: 5, y: 7 })).toEqual({ x: 5, y: 7 }); // identity
});

test("an attached bridge forwards to the host", () => {
  const bridge = new SceneInteractionBridge();
  const host = fakeHost();
  bridge.attach(host);
  bridge.setActiveTool(tool);
  bridge.setDraggingToken("t1");
  expect(host.tools).toEqual([tool]);
  expect(host.drags).toEqual(["t1"]);
  expect(bridge.snap({ x: 5, y: 7 })).toEqual({ x: 6, y: 8 }); // host snap
});

test("preview overlay forwards to the host; detached is a no-op", () => {
  const bridge = new SceneInteractionBridge();
  let previews = 0;
  let cleared = 0;
  expect(() => bridge.previewOverlay([])).not.toThrow(); // detached: no-op
  bridge.attach(fakeSceneHost({ previewOverlay: () => previews++, clearOverlay: () => cleared++ }));
  bridge.previewOverlay([{ points: [0, 0, 1, 1], closed: false, stroke: null, fill: null }]);
  bridge.clearOverlay();
  expect(previews).toBe(1);
  expect(cleared).toBe(1);
});

test("detach restores no-op behavior", () => {
  const bridge = new SceneInteractionBridge();
  const host = fakeHost();
  const detach = bridge.attach(host);
  detach();
  bridge.setActiveTool(tool);
  expect(host.tools).toEqual([]); // not forwarded after detach
  expect(bridge.snap({ x: 1, y: 2 })).toEqual({ x: 1, y: 2 });
});

test("a second attach replaces the host", () => {
  const bridge = new SceneInteractionBridge();
  const a = fakeHost();
  const b = fakeHost();
  bridge.attach(a);
  bridge.attach(b);
  bridge.setActiveTool(tool);
  expect(a.tools).toEqual([]);
  expect(b.tools).toEqual([tool]);
});

test("a stale detach does not clear a newer host", () => {
  const bridge = new SceneInteractionBridge();
  const a = fakeHost();
  const b = fakeHost();
  const detachA = bridge.attach(a);
  bridge.attach(b);
  detachA(); // stale: a was already replaced by b
  bridge.setDraggingToken("x");
  expect(b.drags).toEqual(["x"]); // b still attached
});

test("animateAlongPath forwards to the host (no-op when detached)", () => {
  const bridge = new SceneInteractionBridge();
  expect(() => bridge.animateAlongPath("t1", [[0, 0], [1, 1]])).not.toThrow(); // detached: no-op
  const calls: Array<{ id: string; path: [number, number][] }> = [];
  bridge.attach(fakeSceneHost({ animateAlongPath: (id, path) => calls.push({ id, path }) }));
  bridge.animateAlongPath("t1", [[0, 0], [1, 1]]);
  expect(calls).toEqual([{ id: "t1", path: [[0, 0], [1, 1]] }]);
});

test("animateSamples forwards to the host (no-op when detached)", () => {
  const bridge = new SceneInteractionBridge();
  const samples = [{ tMs: 0, pos: [0, 0] as [number, number] }, { tMs: 500, pos: [100, 0] as [number, number] }];
  expect(() => bridge.animateSamples("t1", samples, 1000, 0)).not.toThrow(); // detached: no-op
  type Call = { id: string; samples: typeof samples; durationMs: number; startServerMs: number };
  const calls: Call[] = [];
  bridge.attach(fakeSceneHost({ animateSamples: (id, s, d, st) => calls.push({ id, samples: s as typeof samples, durationMs: d, startServerMs: st }) }));
  bridge.animateSamples("t1", samples, 1000, 500);
  expect(calls).toEqual([{ id: "t1", samples, durationMs: 1000, startServerMs: 500 }]);
});

test("animateSamples forwards moverVision to the host (M2 §T6 seam)", () => {
  const bridge = new SceneInteractionBridge();
  const samples = [{ tMs: 0, pos: [0, 0] as [number, number] }, { tMs: 500, pos: [100, 0] as [number, number] }];
  const moverVision = [{ tMs: 0, polygons: [[[0, 0], [20, 0], [20, 20]] as [number, number][]] }];
  let gotMoverVision: unknown;
  bridge.attach(fakeSceneHost({ animateSamples: (_id, _s, _d, _st, _sn, mv) => { gotMoverVision = mv; } }));
  bridge.animateSamples("t1", samples, 1000, 0, () => 0, moverVision);
  expect(gotMoverVision).toEqual(moverVision);
});
