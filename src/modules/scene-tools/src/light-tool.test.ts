import { test, expect } from "vitest";
import { DocumentStore, AssetResolver, buildSceneDoc, buildLightDoc, type WireOperation, type LightEngine } from "@shadowcat/core";
import { SceneInteractionBridge } from "@shadowcat/ui-kit";
import { fakeSceneHost } from "@shadowcat/ui-kit/test";
import { ToolController, makeLightTool, type ToolContext } from "./controller.svelte";

const ev = {} as PointerEvent;

function setup(withScene = true) {
  const docs = new DocumentStore();
  if (withScene) docs.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: buildSceneDoc("w1", {}, "scene-1") }] });
  let previews = 0;
  let cleared = 0;
  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({ previewOverlay: () => { previews++; }, clearOverlay: () => { cleared++; } }));
  const sent: WireOperation[][] = [];
  const ctx: ToolContext = { scene: bridge, dispatchIntent: (ops) => sent.push(ops), documents: docs, assets: new AssetResolver(), world: "w1", role: "gm", sendPing: () => {} };
  const controller = new ToolController(ctx);
  return { tool: makeLightTool(ctx, controller), controller, docs, sent, previews: () => previews, clears: () => cleared };
}

test("a click on empty canvas places a light with the documented defaults", () => {
  const { tool, sent } = setup();
  expect(tool.onPointerDown({ x: 103, y: 48 }, ev)).toBe(true);
  const op = sent[0][0];
  expect(op.op).toBe("create");
  if (op.op === "create") {
    expect(op.doc.doc_type).toBe("light");
    expect(op.doc.parent_id).toBe("scene-1");
    // fakeSceneHost's snap is identity → the raw click point is the position.
    expect(op.doc.engine).toMatchObject({
      x: 103,
      y: 48,
      emission: { color: "#ffd9a0", intensity: 1, brightRadius: 2, dimRadius: 6, falloff: null, enabled: true },
    });
  }
});

test("clicking a light marker selects it into the shared editing selection (no intent)", () => {
  const { tool, controller, docs, sent, previews } = setup();
  const light = buildLightDoc("w1", "scene-1", {
    x: 200,
    y: 200,
    emission: { color: "#fff", intensity: 1, brightRadius: 2, dimRadius: 6, falloff: null, enabled: true },
  }, "light-1");
  docs.applyCommand({ seq: 2, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: light }] });
  expect(tool.onPointerDown({ x: 200, y: 200 }, ev)).toBe(true);
  tool.onPointerUp({ x: 200, y: 200 }, ev); // a pure click: select only
  expect(controller.editingEntity).toEqual({ kind: "light", id: "light-1" });
  expect(sent).toHaveLength(0);
  expect(previews()).toBeGreaterThan(0); // the reach rings drew
});

test("dragging a selected light repositions it with the raw stored position as OCC pre-image", () => {
  const { tool, docs, sent } = setup();
  const light = buildLightDoc("w1", "scene-1", {
    x: 200,
    y: 200,
    emission: { color: "#fff", intensity: 1, brightRadius: 2, dimRadius: 6, falloff: null, enabled: true },
  }, "light-1");
  docs.applyCommand({ seq: 2, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: light }] });
  expect(tool.onPointerDown({ x: 200, y: 200 }, ev)).toBe(true);
  tool.onPointerMove({ x: 300, y: 250 }, ev);
  tool.onPointerUp({ x: 300, y: 250 }, ev);
  const op = sent[0][0];
  expect(op.op).toBe("update");
  if (op.op === "update") {
    expect(op.doc_id).toBe("light-1");
    expect(op.changes).toEqual([
      { path: "/engine/x", old: 200, new: 300 },
      { path: "/engine/y", old: 200, new: 250 },
    ]);
  }
});

test("no active scene is unhandled; tool swap clears the ring overlay", () => {
  const b = setup(false);
  expect(b.tool.onPointerDown({ x: 0, y: 0 }, ev)).toBe(false);

  const { tool, clears } = setup();
  tool.onPointerDown({ x: 50, y: 50 }, ev); // places a light
  tool.onDeactivate?.();
  expect(clears()).toBe(1);
});

test("a light out of marker tolerance is not picked (a click beside it places instead)", () => {
  const { tool, controller, docs, sent } = setup();
  const light = buildLightDoc("w1", "scene-1", {
    x: 200,
    y: 200,
    emission: { color: "#fff", intensity: 1, brightRadius: 2, dimRadius: 6, falloff: null, enabled: true },
  }, "light-1");
  docs.applyCommand({ seq: 2, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: light }] });
  expect(tool.onPointerDown({ x: 200, y: 300 }, ev)).toBe(true); // 100 units away
  expect(sent[0][0].op).toBe("create"); // a NEW light was placed instead
  // The placed light's engine carries the nested emission shape end to end, and placement
  // retargets the editing selection at the NEW light (never the existing one beside it).
  const op = sent[0][0];
  if (op.op === "create") {
    expect((op.doc.engine as LightEngine).emission.enabled).toBe(true);
    expect(controller.editingEntity).toEqual({ kind: "light", id: op.doc.id });
  }
});
