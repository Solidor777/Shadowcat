import { test, expect } from "vitest";
import { DocumentStore, AssetResolver, buildSceneDoc, type WireOperation } from "@shadowcat/core";
import { SceneInteractionBridge } from "@shadowcat/ui-kit";
import { fakeSceneHost } from "@shadowcat/ui-kit/test";
import { ToolController, makeTemplateTool, type ToolContext, type TemplateMode } from "./controller.svelte";

const ev = {} as PointerEvent;

function setup(mode: TemplateMode) {
  const docs = new DocumentStore();
  docs.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: buildSceneDoc("w1", {}, "scene-1") }] });
  const previews: Array<Array<{ closed: boolean }>> = [];
  let cleared = 0;
  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({
    previewOverlay: (s) => previews.push(s as Array<{ closed: boolean }>),
    clearOverlay: () => { cleared++; },
  }));
  const sent: WireOperation[][] = [];
  const ctx: ToolContext = { scene: bridge, dispatchIntent: (ops) => sent.push(ops), documents: docs, assets: new AssetResolver(), world: "w1", sendPing: () => {} };
  const controller = new ToolController(ctx);
  controller.templateMode = mode;
  return { tool: makeTemplateTool(ctx, controller), previews, sent, clears: () => cleared };
}

test("a circle drag previews then persists a circle template sized by the drag", () => {
  const { tool, previews, sent, clears } = setup("circle");
  expect(tool.onPointerDown({ x: 0, y: 0 }, ev)).toBe(true); // anchor (0,0)
  tool.onPointerMove({ x: 30, y: 40 }, ev); // radius 50
  expect(previews.length).toBeGreaterThan(0);
  tool.onPointerUp({ x: 30, y: 40 }, ev);
  expect(clears()).toBe(1);
  const op = sent[0][0];
  expect(op.op).toBe("create");
  if (op.op === "create") {
    expect(op.doc.doc_type).toBe("template");
    expect(op.doc.parent_id).toBe("scene-1");
    expect(op.doc.system).toMatchObject({ shape: { kind: "circle", x: 0, y: 0, size: 50 }, color: "#3388ff" }); // circle: direction irrelevant
  }
});

test("a cone drag records the drag direction", () => {
  const { tool, sent } = setup("cone");
  tool.onPointerDown({ x: 0, y: 0 }, ev);
  tool.onPointerUp({ x: 10, y: 0 }, ev); // due +x → direction 0
  const op = sent[0][0];
  if (op.op === "create") expect(op.doc.system).toMatchObject({ shape: { kind: "cone", direction: 0, size: 10 } });
});

test("a click (no drag) places a default one-cell template", () => {
  const { tool, sent } = setup("circle");
  tool.onPointerDown({ x: 0, y: 0 }, ev);
  tool.onPointerUp({ x: 0, y: 0 }, ev); // zero drag
  const op = sent[0][0];
  if (op.op === "create") expect(op.doc.system).toMatchObject({ shape: { kind: "circle", size: 100 } }); // default grid cell
});
