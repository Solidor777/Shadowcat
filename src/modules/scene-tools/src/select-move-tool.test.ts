import { test, expect } from "vitest";
import { DocumentStore, AssetResolver, buildTokenDoc, buildActorDoc, buildSceneDoc, buildTokenFromActor, type WireOperation, type MoveStream } from "@shadowcat/core";
import { SceneInteractionBridge, TokenSelection } from "@shadowcat/ui-kit";
import { fakeSceneHost } from "@shadowcat/ui-kit/test";
import { makeSelectMoveTool, type ToolContext } from "./controller.svelte";

const ev = {} as PointerEvent;
const noShift = { shiftKey: false } as PointerEvent;

/** Drain the microtask queue so async pathfind/moveRequest stubs resolve. */
function flush(): Promise<void> {
  return new Promise((r) => setTimeout(r, 0));
}

function setup() {
  const docs = new DocumentStore();
  docs.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [{ op: "create", doc: buildTokenDoc("w1", "s1", { x: 100, y: 100, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null }, "t1") }],
  });
  const drags: (string | null)[] = [];
  const overlays: unknown[][] = [];
  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({ setDraggingToken: (id) => drags.push(id), previewOverlay: (s) => overlays.push([...s]) }));
  const sent: WireOperation[][] = [];
  let t = 0;
  const ctx: ToolContext = {
    scene: bridge, dispatchIntent: (ops) => sent.push(ops), documents: docs,
    assets: new AssetResolver(), world: "w1", role: "gm", sendPing: () => {}, now: () => t,
    tokenSelection: new TokenSelection(),
  };
  const tool = makeSelectMoveTool(ctx);
  return { tool, sent, drags, overlays, ctx, setTime: (n: number) => { t = n; } };
}

/** Two tokens at known centers (tok1 @ (100,100), tok2 @ (300,100)) + a selection holder. */
function setupTwo() {
  const docs = new DocumentStore();
  docs.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [
      { op: "create", doc: buildTokenDoc("w1", "s1", { x: 100, y: 100, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null }, "tok1") },
      { op: "create", doc: buildTokenDoc("w1", "s1", { x: 300, y: 100, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null }, "tok2") },
    ],
  });
  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({}));
  const sent: WireOperation[][] = [];
  const ctx: ToolContext = {
    scene: bridge, dispatchIntent: (ops) => sent.push(ops), documents: docs,
    assets: new AssetResolver(), world: "w1", role: "gm", sendPing: () => {}, now: () => 0,
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
  const xByDoc = new Map(moves.map((m) => [m.op === "update" ? m.doc_id : "", m.op === "update" ? m.changes.find((c) => c.path === "/engine/x")?.new : undefined]));
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

test("a GM drag previews via overlay on move (never dispatches mid-gesture) and commits exactly once, batched, on release", () => {
  const { tool, sent, drags, overlays, setTime } = setup();
  setTime(0);
  tool.onPointerDown({ x: 100, y: 100 }, ev); // grab the center (offset 0)
  tool.onPointerMove({ x: 150, y: 100 }, ev); // leading edge → previews
  expect(sent).toHaveLength(0); // no document write mid-gesture
  expect(overlays.length).toBeGreaterThan(0);
  setTime(10);
  tool.onPointerMove({ x: 160, y: 100 }, ev); // within the throttle window → suppressed
  const overlaysBeforeUp = overlays.length;
  tool.onPointerUp({ x: 160, y: 100 }, ev); // commits the final position exactly once
  expect(sent).toHaveLength(1);
  expect(drags).toEqual(["t1", null]);
  const ops = sent[0][0];
  expect(ops.op).toBe("update");
  if (ops.op === "update") {
    expect(ops.changes.find((c) => c.path === "/engine/x")?.new).toBe(160);
    expect(ops.changes.find((c) => c.path === "/engine/y")?.new).toBe(100);
  }
  // onPointerUp clears the overlay after committing — no new previewMoves overlay push.
  expect(overlays.length).toBe(overlaysBeforeUp);
});

test("a preview past the throttle window fires again", () => {
  const { tool, overlays, setTime } = setup();
  setTime(0);
  tool.onPointerDown({ x: 100, y: 100 }, ev);
  tool.onPointerMove({ x: 150, y: 100 }, ev); // preview 1 (leading)
  const afterFirst = overlays.length;
  setTime(60);
  tool.onPointerMove({ x: 170, y: 100 }, ev); // 60 - 0 >= 50 → preview 2
  expect(overlays.length).toBeGreaterThan(afterFirst);
});

test("circle-shaped token gets an ellipse selection ring (> 8 points), not a rect", () => {
  // Build an actor with shape:"circle" + a scene with grid size 100 so resolveTokenBox
  // returns shape:"circle", w:100, h:100. The selection ring must be an ellipsePoints
  // path (many points) rather than the 8-number rect path.
  const docs = new DocumentStore();
  const scene = buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: null } }, "scene1");
  const actor = buildActorDoc("w1", "Wraith", {
    displayName: "Wraith",
    visual: { kind: "image", asset: "a1" },
    size: { w: 1, h: 1 }, shape: "circle",
    faction: null, conditions: [], prototype: false, vision: null,
  }, "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 100, y: 100 }, { w: 100, h: 100 }, "tok1");
  docs.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [
    { op: "create", doc: scene },
    { op: "create", doc: actor },
    { op: "create", doc: token },
  ]});

  const overlays: Array<Array<{ points: number[] }>> = [];
  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({ previewOverlay: (s) => overlays.push(s as Array<{ points: number[] }>) }));
  const ctx: ToolContext = {
    scene: bridge, dispatchIntent: () => {}, documents: docs,
    assets: new AssetResolver(), world: "w1", role: "gm", sendPing: () => {}, now: () => 0,
    tokenSelection: new TokenSelection(),
  };
  const tool = makeSelectMoveTool(ctx);
  tool.onPointerDown({ x: 100, y: 100 }, noShift); // hits the circle token at its center
  // The selection overlay must have been called with an ellipse ring (> 8 numbers).
  expect(overlays.length).toBeGreaterThan(0);
  const ring = overlays.at(-1)![0];
  expect(ring.points.length).toBeGreaterThan(8);
});

// Movement authority: a GM writes a token position directly; a player's drag issues a
// pathfind + moveRequest per token instead, committed once on release.

/** Builds a ToolContext driving the select/move tool for a role + a seeded scene + tokens,
 * plus stubs for pathfind/moveRequest recording every call. Minimal harness scoped to the 4
 * gesture-authority behaviors below — not a general-purpose ToolContext factory. */
function harness(opts: {
  role: "gm" | "player";
  tokens: { id: string; x: number; y: number }[];
  moveRequestRejects?: boolean;
}) {
  const docs = new DocumentStore();
  docs.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [
      { op: "create", doc: buildSceneDoc("w1", {}, "s1") },
      ...opts.tokens.map((t) => ({
        op: "create" as const,
        doc: buildTokenDoc("w1", "s1", { x: t.x, y: t.y, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null }, t.id),
      })),
    ],
  });

  const dispatchedOps: WireOperation[] = [];
  const moveRequests: { scene: string; token: string; goal: [number, number] }[] = [];
  const previewOverlayCalls: unknown[][] = [];
  let t = 0;

  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({ previewOverlay: (s) => previewOverlayCalls.push([...s]) }));

  const sel = new TokenSelection();

  const pathfind: ToolContext["pathfind"] = async (_scene, start, waypoints) => {
    const goal = waypoints.at(-1)!;
    return { path: [start, goal], cost: 1, arrested: false, truncated: false };
  };
  const moveRequest: ToolContext["moveRequest"] = (scene, token, path): Promise<MoveStream> => {
    const goal = path.at(-1)!;
    moveRequests.push({ scene, token, goal });
    if (opts.moveRequestRejects) return Promise.reject(new Error("refused"));
    return Promise.resolve({
      requestId: "r1", tokenId: token, mover: "u1", scene, startServerMs: 0,
      durationMs: 0, stop: goal, samples: [], moverVision: null, cost: 1, truncated: false,
    });
  };

  const ctx: ToolContext = {
    scene: bridge,
    dispatchIntent: (ops) => dispatchedOps.push(...ops),
    documents: docs,
    assets: new AssetResolver(),
    world: "w1",
    role: opts.role,
    sendPing: () => {},
    now: () => t,
    tokenSelection: sel,
    pathfind,
    moveRequest,
  };
  const tool = makeSelectMoveTool(ctx);

  const originOf = (id: string): { x: number; y: number } => {
    const t = opts.tokens.find((tk) => tk.id === id)!;
    return { x: t.x, y: t.y };
  };

  return {
    select(ids: string[]): void { sel.set(ids); },
    async drag({ dx, dy }: { dx: number; dy: number }): Promise<void> {
      const origin = originOf([...sel.ids][0]);
      tool.onPointerDown(origin, noShift);
      tool.onPointerMove({ x: origin.x + dx, y: origin.y + dy }, ev);
      tool.onPointerUp({ x: origin.x + dx, y: origin.y + dy }, ev);
      await flush();
    },
    async dragWithTicks({ dx, dy, ticks }: { dx: number; dy: number; ticks: number }): Promise<void> {
      const origin = originOf([...sel.ids][0]);
      tool.onPointerDown(origin, noShift);
      for (let i = 1; i <= ticks; i++) {
        t += 50; // clear the DRAG_THROTTLE_MS window each tick
        tool.onPointerMove({ x: origin.x + (dx * i) / ticks, y: origin.y + (dy * i) / ticks }, ev);
      }
      tool.onPointerUp({ x: origin.x + dx, y: origin.y + dy }, ev);
      await flush();
    },
    get dispatchedOps(): WireOperation[] { return dispatchedOps; },
    get moveRequests(): { scene: string; token: string; goal: [number, number] }[] { return moveRequests; },
    get previewOverlayCalls(): unknown[][] { return previewOverlayCalls; },
    documents: docs,
  };
}

test("a non-GM drag issues exactly one moveRequest per selected token, on release", async () => {
  const h = harness({ role: "player", tokens: [{ id: "t1", x: 0, y: 0 }, { id: "t2", x: 100, y: 0 }] });
  h.select(["t1", "t2"]);
  await h.dragWithTicks({ dx: 100, dy: 0, ticks: 5 });
  expect(h.moveRequests).toEqual([
    { scene: "s1", token: "t1", goal: [100, 0] },
    { scene: "s1", token: "t2", goal: [200, 0] },
  ]);
  expect(h.dispatchedOps).toEqual([]);
});

test("a GM drag issues one batched update and zero move requests", async () => {
  const h = harness({ role: "gm", tokens: [{ id: "t1", x: 0, y: 0 }] });
  h.select(["t1"]);
  await h.drag({ dx: 100, dy: 0 });
  expect(h.moveRequests).toEqual([]);
  expect(h.dispatchedOps).toEqual([
    { op: "update", doc_id: "t1", changes: [
      { path: "/engine/x", old: 0, new: 100 },
      { path: "/engine/y", old: 0, new: 0 },
    ] },
  ]);
});

test("a non-GM drag does not move the rendered token before a MoveStream arrives", async () => {
  const h = harness({ role: "player", tokens: [{ id: "t1", x: 0, y: 0 }] });
  h.select(["t1"]);
  await h.drag({ dx: 100, dy: 0 });
  expect((h.documents.get("t1")!.engine as { x: number }).x).toBe(0);
  expect(h.previewOverlayCalls.length).toBeGreaterThan(0);
});

test("a refused player move surfaces feedback rather than failing silently", async () => {
  const h = harness({ role: "player", tokens: [{ id: "t1", x: 0, y: 0 }], moveRequestRejects: true });
  h.select(["t1"]);
  await h.drag({ dx: 100, dy: 0 });
  expect(h.moveRequests.length).toBeGreaterThan(0); // the request WAS made
  expect(h.previewOverlayCalls.at(-1)).toEqual([]); // preview cleared, not left stale
  expect((h.documents.get("t1")!.engine as { x: number }).x).toBe(0); // never moved locally
});
