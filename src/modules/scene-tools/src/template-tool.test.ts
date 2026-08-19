import { test, expect } from "vitest";
import { DocumentStore, AssetResolver, buildSceneDoc, type WireOperation } from "@shadowcat/core";
import { SceneInteractionBridge } from "@shadowcat/ui-kit";
import { fakeSceneHost } from "@shadowcat/ui-kit/test";
import { ToolController, makeTemplateTool, type ToolContext, type TemplateMode } from "./controller.svelte";

const ev = {} as PointerEvent;

function setup(mode: TemplateMode, snap: (p: { x: number; y: number }) => { x: number; y: number } = (p) => p) {
  const docs = new DocumentStore();
  docs.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: buildSceneDoc("w1", {}, "scene-1") }] });
  const previews: Array<Array<{ closed: boolean }>> = [];
  let cleared = 0;
  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({
    previewOverlay: (s) => previews.push(s as Array<{ closed: boolean }>),
    clearOverlay: () => { cleared++; },
    snap,
  }));
  const sent: WireOperation[][] = [];
  const ctx: ToolContext = { scene: bridge, dispatchIntent: (ops) => sent.push(ops), documents: docs, assets: new AssetResolver(), world: "w1", role: "gm", sendPing: () => {} };
  const controller = new ToolController(ctx);
  controller.templateMode = mode;
  return { tool: makeTemplateTool(ctx, controller), previews, sent, clears: () => cleared };
}

/** Snaps to the nearest 100-unit cell center, matching the default scene's `size: 100` grid —
 * mirrors real grid-snap closely enough to prove `onPointerMove`/`onPointerUp` share the
 * anchor's coordinate frame. */
function snapTo100(p: { x: number; y: number }): { x: number; y: number } {
  return { x: Math.round(p.x / 100) * 100, y: Math.round(p.y / 100) * 100 };
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
    expect(op.doc.engine).toMatchObject({ shape: { kind: "circle", x: 0, y: 0, size: 50 }, color: "#3388ff" }); // circle: direction irrelevant
  }
});

test("a cone drag records the drag direction", () => {
  const { tool, sent } = setup("cone");
  tool.onPointerDown({ x: 0, y: 0 }, ev);
  tool.onPointerUp({ x: 10, y: 0 }, ev); // due +x → direction 0
  const op = sent[0][0];
  if (op.op === "create") expect(op.doc.engine).toMatchObject({ shape: { kind: "cone", direction: 0, size: 10 } });
});

test("a click (no drag) places a default one-cell template", () => {
  const { tool, sent } = setup("circle");
  tool.onPointerDown({ x: 0, y: 0 }, ev);
  tool.onPointerUp({ x: 0, y: 0 }, ev); // zero drag
  const op = sent[0][0];
  if (op.op === "create") expect(op.doc.engine).toMatchObject({ shape: { kind: "circle", size: 100 } }); // default grid cell
});

test("a plain click in a snapping scene still places the default one-cell template, not an arbitrary small one", () => {
  // Anchor snaps to (0,0); release lands at (30,20) — raw distance ~36.06 units, well past
  // sizeDir's `d < 1` fallback threshold, but the release point is snapped to the same frame
  // as the anchor before reaching sizeDir, so both points land on the same (0,0) cell center,
  // d === 0, and the fallback fires.
  const { tool, sent, clears } = setup("circle", snapTo100);
  tool.onPointerDown({ x: 0, y: 0 }, ev); // anchor snaps to (0,0)
  tool.onPointerUp({ x: 30, y: 20 }, ev); // snaps to (0,0) too — same cell as the anchor
  expect(clears()).toBe(1);
  const op = sent[0][0];
  expect(op.op).toBe("create");
  if (op.op === "create") {
    expect(op.doc.engine).toMatchObject({ shape: { kind: "circle", x: 0, y: 0, size: 100, direction: 0 } });
  }
});

test("a genuine drag in a snapping scene still produces the dragged size/direction", () => {
  const { tool, sent } = setup("circle", snapTo100);
  tool.onPointerDown({ x: 0, y: 0 }, ev); // anchor snaps to (0,0)
  tool.onPointerUp({ x: 320, y: 410 }, ev); // snaps to (300,400) — well away from the anchor
  const op = sent[0][0];
  if (op.op === "create") {
    expect(op.doc.engine).toMatchObject({ shape: { kind: "circle", x: 0, y: 0, size: 500 } }); // hypot(300,400)
  }
});
