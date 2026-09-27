// @vitest-environment node
// Exercises plain state and pure functions: no component render and no DOM API use, so
// the package-default jsdom environment would be constructed per file and never touched.
import { test, expect } from "vitest";
import { DocumentStore, AssetResolver, buildSceneDoc, type WireOperation } from "@shadowcat/core";
import { SceneInteractionBridge } from "@shadowcat/ui-kit";
import { fakeSceneHost } from "@shadowcat/ui-kit/test";
import { ToolController, makeRegionTool, type ToolContext } from "./controller.svelte";

const ev = {} as PointerEvent;

function setup(withScene = true, viewedLevelBand?: () => { bottom: number; top: number } | null) {
  const docs = new DocumentStore();
  if (withScene) docs.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: buildSceneDoc("w1", {}, "scene-1") }] });
  let previews = 0;
  let cleared = 0;
  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({ previewOverlay: () => { previews++; }, clearOverlay: () => { cleared++; } }));
  const sent: WireOperation[][] = [];
  const ctx: ToolContext = { scene: bridge, dispatchIntent: (ops) => sent.push(ops), documents: docs, assets: new AssetResolver(), world: "w1", role: "gm", sendPing: () => {}, t: (k) => k, viewedLevelBand };
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
    expect(op.doc.engine).toMatchObject({ behavior: "impassable", shape: { kind: "rect", points: [0, 0, 100, 100] } });
  }
});

test("secret toggle declares default:none and /system gm_only on the created doc", () => {
  const { tool, controller, sent } = setup();
  controller.regionShapeMode = "circle";
  controller.regionSecret = true;
  tool.onPointerDown({ x: 50, y: 50 }, ev);
  tool.onPointerUp({ x: 80, y: 50 }, ev);
  expect(sent).toHaveLength(1);
  const op = sent[0][0];
  expect(op.op).toBe("create");
  if (op.op === "create") {
    expect(op.doc.permissions.property_overrides["/engine"]).toBe("gm_only");
    expect(op.doc.permissions.default).toBe("none");
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

test("with viewedLevelBand set, a new region's engine body carries the expected elevation", () => {
  const { tool, controller, sent } = setup(true, () => ({ bottom: 10, top: 20 }));
  controller.regionShapeMode = "rect";
  tool.onPointerDown({ x: 0, y: 0 }, ev);
  tool.onPointerUp({ x: 100, y: 100 }, ev);
  const op = sent[0][0];
  if (op.op === "create") expect(op.doc.engine).toMatchObject({ elevation: { bottom: 10, top: 20 } });
});

test("with viewedLevelBand unset, a new region's engine body's elevation stays null", () => {
  const { tool, controller, sent } = setup(true);
  controller.regionShapeMode = "rect";
  tool.onPointerDown({ x: 0, y: 0 }, ev);
  tool.onPointerUp({ x: 100, y: 100 }, ev);
  const op = sent[0][0];
  if (op.op === "create") expect(op.doc.engine).toMatchObject({ elevation: null });
});

test("ambient_light behavior persists the configured mode/level as ambient_override", () => {
  const { tool, controller, sent } = setup();
  controller.regionShapeMode = "rect";
  controller.regionBehavior = "ambient_light";
  controller.regionAmbientMode = "brighten";
  controller.regionAmbientLevel = 0.3;
  tool.onPointerDown({ x: 0, y: 0 }, ev);
  tool.onPointerUp({ x: 100, y: 100 }, ev);
  expect(sent).toHaveLength(1);
  const op = sent[0][0];
  if (op.op === "create") {
    expect(op.doc.engine).toMatchObject({
      behavior: "ambient_light",
      ambient_override: { mode: "brighten", level: 0.3 },
    });
  }
});

test("a non-ambient_light behavior always persists ambient_override: null, even with a configured level", () => {
  // Mutation coverage for `regionAmbientOverride`'s own gate: removing the
  // `regionBehavior !== "ambient_light"` check would leak the configured level onto every
  // region regardless of behavior, which this pins against.
  const { tool, controller, sent } = setup();
  controller.regionShapeMode = "rect";
  controller.regionBehavior = "terrain";
  controller.regionAmbientMode = "override";
  controller.regionAmbientLevel = 0.9;
  tool.onPointerDown({ x: 0, y: 0 }, ev);
  tool.onPointerUp({ x: 100, y: 100 }, ev);
  const op = sent[0][0];
  if (op.op === "create") expect(op.doc.engine).toMatchObject({ ambient_override: null });
});

test("the compound commit carries the controller's own behavior, not a hardcoded one", () => {
  const { tool, controller, sent } = setup();
  controller.regionShapeMode = "rect";
  controller.regionCompound = true;
  tool.onPointerDown({ x: 0, y: 0 }, ev);
  tool.onPointerUp({ x: 100, y: 100 }, ev);
  controller.commitRegionParts();
  const op = sent[0][0];
  if (op.op === "create") {
    expect(op.doc.engine).toMatchObject({ behavior: "terrain", cost: 2, ambient_override: null });
  }
});

test("the armed compound gesture stages parts and commits ONE compound region doc", () => {
  const { tool, controller, sent } = setup();
  controller.regionShapeMode = "rect";
  controller.regionCompound = true; // the rail's region-part-add arms the gesture
  tool.onPointerDown({ x: 0, y: 0 }, ev);
  tool.onPointerUp({ x: 100, y: 100 }, ev);
  tool.onPointerDown({ x: 200, y: 200 }, ev);
  tool.onPointerUp({ x: 300, y: 300 }, ev);
  expect(sent).toHaveLength(0); // nothing exists as a document until the whole list commits
  expect(controller.regionParts).toHaveLength(2);
  expect(controller.regionParts[0]).toEqual({ shape: { kind: "rect", points: [0, 0, 100, 100] }, hole: false });

  // The row's hole checkbox flips the part's flag in place (region-part-hole).
  controller.regionParts[1].hole = true;
  controller.regionBehavior = "ambient_light";
  controller.regionAmbientMode = "darken";
  controller.regionAmbientLevel = 0.4;
  controller.commitRegionParts();

  expect(sent).toHaveLength(1);
  const op = sent[0][0];
  expect(op.op).toBe("create");
  if (op.op === "create") {
    expect(op.doc.doc_type).toBe("region");
    expect(op.doc.engine).toMatchObject({
      behavior: "ambient_light",
      ambient_override: { mode: "darken", level: 0.4 },
      shape: [
        { shape: { kind: "rect", points: [0, 0, 100, 100] }, hole: false },
        { shape: { kind: "rect", points: [200, 200, 300, 300] }, hole: true },
      ],
    });
  }
  // The gesture resets after commit: the next plain drag is an immediate single-shape
  // persist again (the unchanged default flow).
  expect(controller.regionParts).toHaveLength(0);
  expect(controller.regionCompound).toBe(false);
});

test("a staged list discarded by onDeactivate leaves the plain drag flow untouched", () => {
  const { tool, controller, sent } = setup();
  controller.regionShapeMode = "rect";
  controller.regionCompound = true;
  tool.onPointerDown({ x: 0, y: 0 }, ev);
  tool.onPointerUp({ x: 100, y: 100 }, ev);
  expect(controller.regionParts).toHaveLength(1);

  tool.onDeactivate?.();
  expect(controller.regionParts).toHaveLength(0);
  expect(controller.regionCompound).toBe(false);
  expect(sent).toHaveLength(0);

  // The very next drag is the plain single-shape flow again — an armed-but-uncommitted
  // list must never silently change what a later drag does.
  tool.onPointerDown({ x: 0, y: 0 }, ev);
  tool.onPointerUp({ x: 50, y: 50 }, ev);
  expect(sent).toHaveLength(1);
  const op = sent[0][0];
  if (op.op === "create") {
    expect(op.doc.engine).toMatchObject({ shape: { kind: "rect", points: [0, 0, 50, 50] } });
  }
});
