// @vitest-environment node
// Exercises plain state and pure functions: no component render and no DOM API use, so
// the package-default jsdom environment would be constructed per file and never touched.
import { test, expect } from "vitest";
import { DocumentStore, AssetResolver } from "@shadowcat/core";
import { SceneInteractionBridge } from "@shadowcat/ui-kit";
import { fakeSceneHost } from "@shadowcat/ui-kit/test";
import { makePingTool, type ToolContext } from "./controller.svelte";

const ev = {} as PointerEvent;

test("clicking with the ping tool broadcasts a ping at the scene point", () => {
  const pings: Array<{ x: number; y: number }> = [];
  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost());
  const ctx: ToolContext = {
    scene: bridge,
    dispatchIntent: () => {},
    documents: new DocumentStore(),
    t: (k) => k,
    assets: new AssetResolver(),
    world: "w1",
    role: "gm",
    sendPing: (x, y) => pings.push({ x, y }),
  };
  const tool = makePingTool(ctx);
  expect(tool.onPointerDown({ x: 42, y: 7 }, ev)).toBe(true);
  expect(pings).toEqual([{ x: 42, y: 7 }]);
});
