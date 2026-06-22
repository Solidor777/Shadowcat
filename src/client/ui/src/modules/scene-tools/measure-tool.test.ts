import { test, expect } from "vitest";
import { DocumentStore, AssetResolver, type WireOperation } from "@shadowcat/core";
import type { Point } from "@shadowcat/render";
import { SceneInteractionBridge } from "../../lib/sceneInteraction";
import { fakeSceneHost } from "../../lib/__fixtures__/fakeSceneHost";
import { makeMeasureTool, type ToolContext } from "./controller.svelte";

const ev = {} as PointerEvent;

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
  const ctx: ToolContext = { scene: bridge, dispatchIntent: (ops) => sent.push(ops), documents: new DocumentStore(), assets: new AssetResolver(), world: "w1" };
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
