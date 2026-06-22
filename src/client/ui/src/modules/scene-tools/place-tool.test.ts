import { test, expect } from "vitest";
import { DocumentStore, AssetResolver, buildSceneDoc, type WireOperation } from "@shadowcat/core";
import { SceneInteractionBridge } from "../../lib/sceneInteraction";
import { fakeSceneHost } from "../../lib/__fixtures__/fakeSceneHost";
import { ToolController, makePlaceTool, type ToolContext } from "./controller.svelte";

const ev = {} as PointerEvent;

/** A documents view seeded with one scene (or none). */
function docsWithScene(withScene: boolean): DocumentStore {
  const d = new DocumentStore();
  if (withScene) {
    d.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: buildSceneDoc("w1", {}, "scene-1") }] });
  }
  return d;
}

/** A scene bridge whose snap shifts by +1 so the test proves snap is applied. */
function snapBridge(): SceneInteractionBridge {
  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({ snap: (p) => ({ x: p.x + 1, y: p.y + 1 }) }));
  return bridge;
}

function ctxWith(documents: DocumentStore): { ctx: ToolContext; sent: WireOperation[][] } {
  const sent: WireOperation[][] = [];
  const ctx: ToolContext = {
    scene: snapBridge(),
    dispatchIntent: (ops) => sent.push(ops),
    documents,
    assets: new AssetResolver(),
    world: "w1",
  };
  return { ctx, sent };
}

test("place stamps a snapped token from the selected asset, parented to the scene", () => {
  const { ctx, sent } = ctxWith(docsWithScene(true));
  const controller = new ToolController(ctx);
  const tool = makePlaceTool(ctx, controller);

  // No asset selected → unhandled, nothing dispatched.
  expect(tool.onPointerDown({ x: 140, y: 160 }, ev)).toBe(false);
  expect(sent).toHaveLength(0);

  controller.selectedAsset = "asset-1";
  expect(tool.onPointerDown({ x: 140, y: 160 }, ev)).toBe(true);
  expect(sent).toHaveLength(1);
  const op = sent[0][0];
  expect(op.op).toBe("create");
  if (op.op === "create") {
    expect(op.doc.doc_type).toBe("token");
    expect(op.doc.parent_id).toBe("scene-1");
    // snapped (+1,+1); size from the scene grid (default 100); visual from selection.
    expect(op.doc.system).toMatchObject({ x: 141, y: 161, w: 100, h: 100, visual: { kind: "image", asset: "asset-1" } });
  }
});

test("place is unhandled when no scene exists", () => {
  const { ctx, sent } = ctxWith(docsWithScene(false));
  const controller = new ToolController(ctx);
  controller.selectedAsset = "asset-1";
  expect(makePlaceTool(ctx, controller).onPointerDown({ x: 0, y: 0 }, ev)).toBe(false);
  expect(sent).toHaveLength(0);
});
