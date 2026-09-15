// @vitest-environment node
// Exercises plain controller state and pure functions: no component render and no DOM API
// use, so the package-default jsdom environment would be constructed per file and never
// touched.
import { test, expect } from "vitest";
import { DocumentStore, AssetResolver, type SceneToolMeta } from "@shadowcat/core";
import type { SceneTool } from "@shadowcat/render";
import { SceneInteractionBridge } from "@shadowcat/ui-kit";
import { fakeSceneHost } from "@shadowcat/ui-kit/test";
import { ToolController, type ToolContext } from "./controller.svelte";

const ev = {} as PointerEvent;

/** A controller over a bridge that records `setActiveTool`, plus the recording list. */
function makeController(): { controller: ToolController; tools: (SceneTool | null)[] } {
  const tools: (SceneTool | null)[] = [];
  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({ setActiveTool: (t) => tools.push(t) }));
  const ctx: ToolContext = {
    scene: bridge,
    dispatchIntent: () => {},
    documents: new DocumentStore(),
    t: (k) => k,
    assets: new AssetResolver(),
    world: "w1",
    role: "gm",
    sendPing: () => {},
  };
  return { controller: new ToolController(ctx), tools };
}

function fakeMeta(id: string, clicks: Array<{ x: number; y: number }>): SceneToolMeta {
  return { id, icon: "i", labelKey: `tool.${id}`, onSceneClick: (x, y) => clicks.push({ x, y }) };
}

test("toggleContributed activates a contributed tool whose clicks forward the scene point", () => {
  const { controller, tools } = makeController();
  const clicks: Array<{ x: number; y: number }> = [];
  controller.toggleContributed(fakeMeta("fx", clicks));
  expect(controller.activeContributedId).toBe("fx");
  const tool = tools.at(-1);
  expect(tool).not.toBeNull();
  expect(tool!.onPointerDown({ x: 42, y: 7 }, ev)).toBe(true); // claims the gesture
  expect(clicks).toEqual([{ x: 42, y: 7 }]);
});

test("re-selecting the active contributed tool clears it back to camera", () => {
  const { controller, tools } = makeController();
  const meta = fakeMeta("fx", []);
  controller.toggleContributed(meta);
  controller.toggleContributed(meta);
  expect(controller.activeContributedId).toBeNull();
  expect(tools.at(-1)).toBeNull();
});

test("built-in and contributed activations are mutually exclusive in both directions", () => {
  const { controller, tools } = makeController();
  controller.toggleContributed(fakeMeta("fx", []));
  expect(controller.activeContributedId).toBe("fx");
  // A built-in toggle clears the contributed tool.
  controller.toggle("ping");
  expect(controller.activeContributedId).toBeNull();
  expect(controller.active).toBe("ping");
  // A contributed activation clears the built-in tool.
  controller.toggleContributed(fakeMeta("fx", []));
  expect(controller.active).toBeNull();
  expect(controller.activeContributedId).toBe("fx");
  expect(tools.at(-1)).not.toBeNull();
});
