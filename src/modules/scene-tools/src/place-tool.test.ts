import { test, expect } from "vitest";
import { DocumentStore, AssetResolver, buildSceneDoc, buildActorDoc, type WireOperation } from "@shadowcat/core";
import { SceneInteractionBridge, ActorSelection } from "@shadowcat/ui-kit";
import { fakeSceneHost } from "@shadowcat/ui-kit/test";
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
    sendPing: () => {},
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

const actorSys = (prototype: boolean) => ({
  name: "G",
  displayName: "G",
  visual: { kind: "image" as const, asset: "a1" },
  size: { w: 1, h: 1 },
  shape: "square" as const,
  faction: null,
  conditions: [],
  prototype,
});

function docsWithSceneAndActor(id: string, prototype: boolean): DocumentStore {
  const d = docsWithScene(true);
  d.applyCommand({ seq: 2, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: buildActorDoc("w1", actorSys(prototype), id) }] });
  return d;
}

test("place stamps the selected actor as an instanced token (prototype actor)", () => {
  const { ctx, sent } = ctxWith(docsWithSceneAndActor("act1", true));
  ctx.actorSelection = new ActorSelection();
  ctx.actorSelection.select("act1");
  const controller = new ToolController(ctx);
  expect(makePlaceTool(ctx, controller).onPointerDown({ x: 140, y: 160 }, ev)).toBe(true);
  expect(sent).toHaveLength(1);
  const op = sent[0][0];
  expect(op.op).toBe("create");
  if (op.op === "create") {
    expect(op.doc.doc_type).toBe("token");
    expect(op.doc.parent_id).toBe("scene-1");
    expect(op.doc.system).toMatchObject({ x: 141, y: 161, w: 100, h: 100 });
    expect(op.doc.embedded.actor[0].source).toEqual({ id: "act1", pack: null, version: 1 });
  }
  // Instanced actors stay selected so the GM can place several.
  expect(ctx.actorSelection!.selectedId).toBe("act1");
});

test("place links the selected actor when prototype is false", () => {
  const { ctx, sent } = ctxWith(docsWithSceneAndActor("act2", false));
  ctx.actorSelection = new ActorSelection();
  ctx.actorSelection.select("act2");
  const controller = new ToolController(ctx);
  expect(makePlaceTool(ctx, controller).onPointerDown({ x: 0, y: 0 }, ev)).toBe(true);
  const op = sent[0][0];
  if (op.op === "create") {
    expect((op.doc.system as { actor_id?: string }).actor_id).toBe("act2");
    expect(op.doc.embedded.actor).toBeUndefined();
  }
  // A unique linked actor places once, then the selection clears.
  expect(ctx.actorSelection!.selectedId).toBeNull();
});

test("place keeps a linked actor selected when keepAfterPlace is set", () => {
  const { ctx } = ctxWith(docsWithSceneAndActor("act3", false));
  ctx.actorSelection = new ActorSelection();
  ctx.actorSelection.select("act3");
  ctx.actorSelection.setKeepAfterPlace(true);
  const controller = new ToolController(ctx);
  expect(makePlaceTool(ctx, controller).onPointerDown({ x: 0, y: 0 }, ev)).toBe(true);
  expect(ctx.actorSelection.selectedId).toBe("act3");
});
