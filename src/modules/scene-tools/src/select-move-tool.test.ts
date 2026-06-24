import { test, expect } from "vitest";
import { DocumentStore, AssetResolver, buildTokenDoc, type WireOperation } from "@shadowcat/core";
import { SceneInteractionBridge, TokenSelection } from "@shadowcat/ui-kit";
import { fakeSceneHost } from "@shadowcat/ui-kit/test";
import { makeSelectMoveTool, type ToolContext } from "./controller.svelte";

const ev = {} as PointerEvent;
const noShift = { shiftKey: false } as PointerEvent;

function setup() {
  const docs = new DocumentStore();
  docs.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [{ op: "create", doc: buildTokenDoc("w1", "s1", { x: 100, y: 100, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" } }, "t1") }],
  });
  const drags: (string | null)[] = [];
  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({ setDraggingToken: (id) => drags.push(id) }));
  const sent: WireOperation[][] = [];
  let t = 0;
  const ctx: ToolContext = {
    scene: bridge, dispatchIntent: (ops) => sent.push(ops), documents: docs,
    assets: new AssetResolver(), world: "w1", sendPing: () => {}, now: () => t,
    tokenSelection: new TokenSelection(),
  };
  const tool = makeSelectMoveTool(ctx);
  return { tool, sent, drags, ctx, setTime: (n: number) => { t = n; } };
}

/** Two tokens at known centers (tok1 @ (100,100), tok2 @ (300,100)) + a selection holder. */
function setupTwo() {
  const docs = new DocumentStore();
  docs.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [
      { op: "create", doc: buildTokenDoc("w1", "s1", { x: 100, y: 100, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" } }, "tok1") },
      { op: "create", doc: buildTokenDoc("w1", "s1", { x: 300, y: 100, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" } }, "tok2") },
    ],
  });
  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({}));
  const sent: WireOperation[][] = [];
  const ctx: ToolContext = {
    scene: bridge, dispatchIntent: (ops) => sent.push(ops), documents: docs,
    assets: new AssetResolver(), world: "w1", sendPing: () => {}, now: () => 0,
    tokenSelection: new TokenSelection(),
  };
  return { ctx, sent };
}

test("moves all selected tokens together by the snapped delta", () => {
  const { ctx, sent } = setupTwo();
  ctx.tokenSelection!.set(["tok1", "tok2"]);
  const tool = makeSelectMoveTool(ctx);
  tool.onPointerDown({ x: 100, y: 100 }, noShift); // grab tok1
  tool.onPointerMove({ x: 200, y: 100 }, ev); // +100 in x
  tool.onPointerUp({ x: 200, y: 100 }, ev);
  const moves = sent.flat().filter((o) => o.op === "update");
  const xByDoc = new Map(moves.map((m) => [m.op === "update" ? m.doc_id : "", m.op === "update" ? m.changes.find((c) => c.path === "/system/x")?.new : undefined]));
  expect(xByDoc.get("tok1")).toBe(200);
  expect(xByDoc.get("tok2")).toBe(400);
});

test("clicking an unselected token replaces the selection with just it", () => {
  const { ctx } = setupTwo();
  ctx.tokenSelection!.set(["tok2"]);
  const tool = makeSelectMoveTool(ctx);
  tool.onPointerDown({ x: 100, y: 100 }, noShift); // grab tok1
  expect([...ctx.tokenSelection!.ids]).toEqual(["tok1"]);
});

test("pointerdown on a token starts a drag (marks it dragging)", () => {
  const { tool, drags } = setup();
  expect(tool.onPointerDown({ x: 100, y: 100 }, ev)).toBe(true);
  expect(drags).toEqual(["t1"]);
});

test("pointerdown on empty space is unhandled so the camera pans", () => {
  const { tool, drags } = setup();
  expect(tool.onPointerDown({ x: 500, y: 500 }, ev)).toBe(false);
  expect(drags).toEqual([]);
});

test("a drag streams coalesced position intents and flushes the final on release", () => {
  const { tool, sent, drags, setTime } = setup();
  setTime(0);
  tool.onPointerDown({ x: 100, y: 100 }, ev); // grab the center (offset 0)
  tool.onPointerMove({ x: 150, y: 100 }, ev); // leading edge → sends
  expect(sent).toHaveLength(1);
  setTime(10);
  tool.onPointerMove({ x: 160, y: 100 }, ev); // within the window → suppressed
  expect(sent).toHaveLength(1);
  tool.onPointerUp({ x: 160, y: 100 }, ev); // flush the final unsent position
  expect(sent).toHaveLength(2);
  expect(drags).toEqual(["t1", null]);
  const last = sent[1][0];
  expect(last.op).toBe("update");
  if (last.op === "update") {
    expect(last.changes.find((c) => c.path === "/system/x")?.new).toBe(160);
    expect(last.changes.find((c) => c.path === "/system/y")?.new).toBe(100);
  }
});

test("a move past the throttle window sends again", () => {
  const { tool, sent, setTime } = setup();
  setTime(0);
  tool.onPointerDown({ x: 100, y: 100 }, ev);
  tool.onPointerMove({ x: 150, y: 100 }, ev); // send 1 (leading)
  setTime(60);
  tool.onPointerMove({ x: 170, y: 100 }, ev); // 60 - 0 >= 50 → send 2
  expect(sent).toHaveLength(2);
});
