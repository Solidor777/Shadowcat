import { test, expect } from "vitest";
import { DocumentStore, AssetResolver } from "@shadowcat/core";
import { SceneInteractionBridge } from "../../lib/sceneInteraction";
import { fakeSceneHost } from "../../lib/__fixtures__/fakeSceneHost";
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
    assets: new AssetResolver(),
    world: "w1",
    sendPing: (x, y) => pings.push({ x, y }),
  };
  const tool = makePingTool(ctx);
  expect(tool.onPointerDown({ x: 42, y: 7 }, ev)).toBe(true);
  expect(pings).toEqual([{ x: 42, y: 7 }]);
});
