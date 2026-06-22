import { test, expect } from "vitest";
import type { SceneTool, SceneToolHost, Point } from "@shadowcat/render";
import { SceneInteractionBridge } from "./sceneInteraction";

const tool: SceneTool = { onPointerDown: () => true, onPointerMove: () => {}, onPointerUp: () => {} };

function fakeHost(): SceneToolHost & { tools: (SceneTool | null)[]; drags: (string | null)[] } {
  const tools: (SceneTool | null)[] = [];
  const drags: (string | null)[] = [];
  return {
    tools,
    drags,
    setActiveTool: (t) => tools.push(t),
    snap: (p: Point) => ({ x: p.x + 1, y: p.y + 1 }),
    setDraggingToken: (id) => drags.push(id),
  };
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
