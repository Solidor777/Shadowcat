import { test, expect } from "vitest";
import { DocumentStore, AssetResolver, buildSceneDoc, type WireOperation } from "@shadowcat/core";
import { SceneInteractionBridge } from "../../lib/sceneInteraction";
import { ToolController, makeDrawTool, type ToolContext, type DrawMode } from "./controller.svelte";

const ev = {} as PointerEvent;

function setup(mode: DrawMode, withScene = true) {
  const docs = new DocumentStore();
  if (withScene) docs.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: buildSceneDoc("w1", {}, "scene-1") }] });
  const previews: Array<Array<{ points: number[]; closed: boolean }>> = [];
  let cleared = 0;
  const bridge = new SceneInteractionBridge();
  bridge.attach({
    setActiveTool: () => {}, snap: (p) => p, setDraggingToken: () => {},
    previewOverlay: (s) => previews.push(s as Array<{ points: number[]; closed: boolean }>),
    clearOverlay: () => { cleared++; },
  });
  const sent: WireOperation[][] = [];
  const ctx: ToolContext = { scene: bridge, dispatchIntent: (ops) => sent.push(ops), documents: docs, assets: new AssetResolver(), world: "w1" };
  const controller = new ToolController(ctx);
  controller.drawMode = mode;
  return { tool: makeDrawTool(ctx, controller), previews, sent, clears: () => cleared };
}

test("freehand drag previews the path then persists a freehand drawing", () => {
  const { tool, previews, sent, clears } = setup("freehand");
  expect(tool.onPointerDown({ x: 0, y: 0 }, ev)).toBe(true);
  tool.onPointerMove({ x: 5, y: 5 }, ev);
  tool.onPointerMove({ x: 10, y: 0 }, ev);
  expect(previews.length).toBeGreaterThan(0);
  expect(previews.at(-1)![0].closed).toBe(false);
  tool.onPointerUp({ x: 10, y: 0 }, ev);
  expect(clears()).toBe(1);
  expect(sent).toHaveLength(1);
  const op = sent[0][0];
  expect(op.op).toBe("create");
  if (op.op === "create") {
    expect(op.doc.doc_type).toBe("drawing");
    expect(op.doc.parent_id).toBe("scene-1");
    expect(op.doc.system).toMatchObject({ shape: { kind: "freehand", points: [0, 0, 5, 5, 10, 0] }, stroke: { width: 2 } });
  }
});

test("a rect drag persists a rect drawing with its two corner points", () => {
  const { tool, sent } = setup("rect");
  tool.onPointerDown({ x: 0, y: 0 }, ev);
  tool.onPointerMove({ x: 10, y: 20 }, ev);
  tool.onPointerUp({ x: 10, y: 20 }, ev);
  const op = sent[0][0];
  if (op.op === "create") expect(op.doc.system).toMatchObject({ shape: { kind: "rect", points: [0, 0, 10, 20] } });
});

test("draw is unhandled with no active scene", () => {
  const { tool, sent } = setup("freehand", false);
  expect(tool.onPointerDown({ x: 0, y: 0 }, ev)).toBe(false);
  tool.onPointerUp({ x: 0, y: 0 }, ev);
  expect(sent).toHaveLength(0);
});
