import { test, expect } from "vitest";
import { DocumentStore, AssetResolver, buildSceneDoc, type WireOperation } from "@shadowcat/core";
import { SceneInteractionBridge } from "@shadowcat/ui-kit";
import { fakeSceneHost } from "@shadowcat/ui-kit/test";
import { ToolController, makeRegionTool, type ToolContext } from "./controller.svelte";

const ev = {} as PointerEvent;

function setup(withScene = true) {
  const docs = new DocumentStore();
  if (withScene) docs.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: buildSceneDoc("w1", {}, "scene-1") }] });
  let previews = 0;
  let cleared = 0;
  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({ previewOverlay: () => { previews++; }, clearOverlay: () => { cleared++; } }));
  const sent: WireOperation[][] = [];
  const ctx: ToolContext = { scene: bridge, dispatchIntent: (ops) => sent.push(ops), documents: docs, assets: new AssetResolver(), world: "w1", sendPing: () => {} };
  // Construct the controller so region-specific reactive state exists and the tool is wired
  // the same way the rail builds it.
  const controller = new ToolController(ctx);
  return { tool: makeRegionTool(ctx, controller), controller, sent, previews: () => previews, clears: () => cleared };
}

test("rect mode: drag persists a region doc with the configured behavior/cost", () => {
  const { tool, controller, sent } = setup();
  controller.regionShapeMode = "rect";
  controller.regionBehavior = "impassable";
  controller.regionCost = 1;
  expect(tool.onPointerDown({ x: 0, y: 0 }, ev)).toBe(true);
  tool.onPointerMove({ x: 100, y: 100 }, ev);
  tool.onPointerUp({ x: 100, y: 100 }, ev);
  expect(sent).toHaveLength(1);
  const op = sent[0][0];
  expect(op.op).toBe("create");
  if (op.op === "create") {
    expect(op.doc.doc_type).toBe("region");
    expect(op.doc.parent_id).toBe("scene-1");
    expect(op.doc.system).toMatchObject({ behavior: "impassable", shape: { kind: "rect", points: [0, 0, 100, 100] } });
  }
});

test("secret toggle declares /system as gm_only on the created doc", () => {
  const { tool, controller, sent } = setup();
  controller.regionShapeMode = "circle";
  controller.regionSecret = true;
  tool.onPointerDown({ x: 50, y: 50 }, ev);
  tool.onPointerUp({ x: 80, y: 50 }, ev);
  expect(sent).toHaveLength(1);
  const op = sent[0][0];
  expect(op.op).toBe("create");
  if (op.op === "create") {
    expect(op.doc.permissions.property_overrides["/system"]).toBe("gm_only");
  }
});

test("a pure click with no extent persists nothing; no active scene is unhandled", () => {
  const a = setup();
  a.controller.regionShapeMode = "rect";
  a.tool.onPointerDown({ x: 10, y: 10 }, ev);
  a.tool.onPointerUp({ x: 10, y: 10 }, ev);
  expect(a.sent).toHaveLength(0);

  const b = setup(false);
  expect(b.tool.onPointerDown({ x: 0, y: 0 }, ev)).toBe(false);
});
