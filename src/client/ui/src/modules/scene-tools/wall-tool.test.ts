import { test, expect } from "vitest";
import { DocumentStore, AssetResolver, buildSceneDoc, type WireOperation } from "@shadowcat/core";
import { SceneInteractionBridge } from "@shadowcat/ui-kit";
import { fakeSceneHost } from "@shadowcat/ui-kit/test";
import { ToolController, makeWallTool, type ToolContext } from "./controller.svelte";

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
  // Construct the controller so the tool is wired the same way the rail builds it.
  void new ToolController(ctx);
  return { tool: makeWallTool(ctx), sent, previews: () => previews, clears: () => cleared };
}

test("a wall drag previews then persists a wall doc with seg + both flags", () => {
  const { tool, sent, previews, clears } = setup();
  expect(tool.onPointerDown({ x: 0, y: 0 }, ev)).toBe(true);
  tool.onPointerMove({ x: 100, y: 50 }, ev);
  expect(previews()).toBeGreaterThan(0);
  tool.onPointerUp({ x: 100, y: 50 }, ev);
  expect(clears()).toBe(1);
  const op = sent[0][0];
  expect(op.op).toBe("create");
  if (op.op === "create") {
    expect(op.doc.doc_type).toBe("wall");
    expect(op.doc.parent_id).toBe("scene-1");
    expect(op.doc.system).toMatchObject({ seg: { x1: 0, y1: 0, x2: 100, y2: 50 }, blocksSight: true, blocksMove: true });
  }
});

test("a no-extent click persists no wall; no active scene is unhandled", () => {
  const a = setup();
  a.tool.onPointerDown({ x: 5, y: 5 }, ev);
  a.tool.onPointerUp({ x: 5, y: 5 }, ev);
  expect(a.sent).toHaveLength(0);

  const b = setup(false);
  expect(b.tool.onPointerDown({ x: 0, y: 0 }, ev)).toBe(false);
});
