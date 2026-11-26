// @vitest-environment node
// Exercises plain state and pure functions: no component render and no DOM API use, so
// the package-default jsdom environment would be constructed per file and never touched.
import { test, expect } from "vitest";
import { noticeOncePerGesture, DocumentStore, AssetResolver, RequestTimeoutError, PathfindError, buildTokenDoc, buildSceneDoc, buildLightDoc, buildSceneEntityDoc, type WireOperation, type MoveStream } from "@shadowcat/core";
import { SceneInteractionBridge, TokenSelection } from "@shadowcat/ui-kit";
import { fakeSceneHost } from "@shadowcat/ui-kit/test";
import { ToolController, makeSelectMoveTool, type ToolContext } from "./controller.svelte";

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
    ops: [{ op: "create", doc: buildTokenDoc("w1", "s1", { x: 100, y: 100, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null, lock_rotation: false, identity_hidden: false }, "t1") }],
  });
  const drags: (string | null)[] = [];
  const overlays: unknown[][] = [];
  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({ setDraggingToken: (id) => drags.push(id), previewOverlay: (s) => overlays.push([...s]) }));
  const sent: WireOperation[][] = [];
  let t = 0;
  const ctx: ToolContext = {
    scene: bridge, dispatchIntent: (ops) => sent.push(ops), documents: docs,
    assets: new AssetResolver(), world: "w1", role: "gm", sendPing: () => {}, t: (k) => k, now: () => t,
    tokenSelection: new TokenSelection(),
  };
  const tool = makeSelectMoveTool(ctx, new ToolController(ctx));
  return { tool, sent, drags, overlays, ctx, setTime: (n: number) => { t = n; } };
}

/** Two tokens at known centers (tok1 @ (100,100), tok2 @ (300,100)) + a selection holder. */
function setupTwo() {
  const docs = new DocumentStore();
  docs.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [
      { op: "create", doc: buildTokenDoc("w1", "s1", { x: 100, y: 100, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null, lock_rotation: false, identity_hidden: false }, "tok1") },
      { op: "create", doc: buildTokenDoc("w1", "s1", { x: 300, y: 100, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null, lock_rotation: false, identity_hidden: false }, "tok2") },
    ],
  });
  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({}));
  const sent: WireOperation[][] = [];
  const ctx: ToolContext = {
    scene: bridge, dispatchIntent: (ops) => sent.push(ops), documents: docs,
    assets: new AssetResolver(), world: "w1", role: "gm", sendPing: () => {}, t: (k) => k, now: () => 0,
    tokenSelection: new TokenSelection(),
  };
  return { ctx, sent };
}

test("moves all selected tokens together by the snapped delta", () => {
  const { ctx, sent } = setupTwo();
  ctx.tokenSelection!.set(["tok1", "tok2"]);
  const tool = makeSelectMoveTool(ctx, new ToolController(ctx));
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
  const tool = makeSelectMoveTool(ctx, new ToolController(ctx));
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

// Movement authority: a GM writes a token position directly; a player's drag issues a
// pathfind + moveRequest per token instead, committed once on release.

/** Builds a ToolContext driving the select/move tool for a role + a seeded scene + tokens,
 * plus stubs for pathfind/moveRequest recording every call. Minimal harness scoped to the 4
 * gesture-authority behaviors below — not a general-purpose ToolContext factory. */
function harness(opts: {
  role: "gm" | "player";
  tokens: { id: string; x: number; y: number }[];
  moveRequestRejects?: boolean;
  /** `moveRequest` throws synchronously instead of returning a promise. */
  moveRequestThrows?: boolean;
  /** How the route request resolves: rejects, or answers with a route that never leaves the start
   * (`emptyBlocked` flags it truncated; `emptyPlain` is a zero-displacement route). */
  pathfindMode?: "rejects" | "emptyBlocked" | "emptyPlain";
  /** The rejection value `pathfindMode: "rejects"` throws; defaults to the server's `unreachable`. */
  pathfindError?: () => unknown;
  /** Only these token ids' route requests reject; the rest resolve normally. */
  routeRejectsFor?: string[];
  /** The session pause freezes this user's token movement (`ToolContext.movesFrozen`). */
  movesFrozen?: boolean;
}) {
  const docs = new DocumentStore();
  docs.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [
      { op: "create", doc: buildSceneDoc("w1", {}, "s1") },
      ...opts.tokens.map((t) => ({
        op: "create" as const,
        doc: buildTokenDoc("w1", "s1", { x: t.x, y: t.y, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null, lock_rotation: false, identity_hidden: false }, t.id),
      })),
    ],
  });

  const dispatchedOps: WireOperation[] = [];
  const moveRequests: { scene: string; token: string; goal: [number, number] }[] = [];
  const gestures: (object | undefined)[] = [];
  const previewOverlayCalls: unknown[][] = [];
  let t = 0;

  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({ previewOverlay: (s) => previewOverlayCalls.push([...s]) }));

  const sel = new TokenSelection();

  const notices: { message: string; level: string | undefined }[] = [];
  const pathfind: ToolContext["pathfind"] = async (_scene, start, waypoints, _footprint, token) => {
    if (token !== undefined && opts.routeRejectsFor?.includes(token)) throw (opts.pathfindError ?? (() => new PathfindError("unreachable", "unreachable")))();
    if (opts.pathfindMode === "rejects") throw (opts.pathfindError ?? (() => new PathfindError("unreachable", "unreachable")))();
    if (opts.pathfindMode === "emptyBlocked") return { path: [start], cost: 0, arrested: false, truncated: true, budgetCells: 0 };
    if (opts.pathfindMode === "emptyPlain") return { path: [start], cost: 0, arrested: false, truncated: false, budgetCells: null };
    const goal = waypoints.at(-1)!;
    return { path: [start, goal], cost: 1, arrested: false, truncated: false, budgetCells: null };
  };
  const moveRequest: ToolContext["moveRequest"] = (scene, token, path, gesture): Promise<MoveStream> => {
    const goal = path.at(-1)!;
    moveRequests.push({ scene, token, goal });
    gestures.push(gesture);
    if (opts.moveRequestThrows) throw new Error("transport seam threw");
    if (opts.moveRequestRejects) return Promise.reject(new Error("refused"));
    return Promise.resolve({
      requestId: "r1", tokenId: token, mover: "u1", scene, startServerMs: 0,
      durationMs: 0, stop: goal, samples: [], moverVision: null, moverLight: null, cost: 1, truncated: false,
    });
  };

  const ctx: ToolContext = {
    scene: bridge,
    dispatchIntent: (ops) => dispatchedOps.push(...ops),
    documents: docs,
    assets: new AssetResolver(),
    world: "w1",
    role: opts.role,
    sendPing: () => {}, t: (k) => k,
    now: () => t,
    tokenSelection: sel,
    pathfind,
    moveRequest,
    notify: (message, level) => notices.push({ message, level }),
    movesFrozen: () => opts.movesFrozen ?? false,
  };
  const tool = makeSelectMoveTool(ctx, new ToolController(ctx));

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
    get gestures(): (object | undefined)[] { return gestures; },
    get notices(): { message: string; level: string | undefined }[] { return notices; },
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

test("a drag while the session pause freezes movement sends nothing, raises the paused notice once and clears the preview", async () => {
  const h = harness({ role: "player", tokens: [{ id: "t1", x: 0, y: 0 }, { id: "t2", x: 100, y: 0 }], movesFrozen: true });
  h.select(["t1", "t2"]);
  await h.drag({ dx: 100, dy: 0 });
  expect(h.moveRequests).toEqual([]);
  expect(h.dispatchedOps).toEqual([]);
  expect(h.notices).toEqual([{ message: "intent.rejected.paused", level: "warning" }]);
  expect(h.previewOverlayCalls.at(-1)).toEqual([]);
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

test("a rejected drag route request raises the no-route notice and drops the preview", async () => {
  const h = harness({ role: "player", tokens: [{ id: "t1", x: 0, y: 0 }], pathfindMode: "rejects" });
  h.select(["t1"]);
  await h.drag({ dx: 100, dy: 0 });
  expect(h.moveRequests).toEqual([]);
  expect(h.notices).toEqual([{ message: "tools.moveNoRoute", level: "warning" }]);
  expect(h.previewOverlayCalls.at(-1)).toEqual([]);
});


test("a drag route failure is worded by `routeFailureKey`: a typed rejection reaches its own notice", async () => {
  const typed = harness({ role: "player", tokens: [{ id: "t1", x: 0, y: 0 }], pathfindMode: "rejects", pathfindError: () => new PathfindError("boom", "invalid") });
  typed.select(["t1"]);
  await typed.drag({ dx: 100, dy: 0 });
  expect(typed.notices).toEqual([{ message: "tools.moveInvalid", level: "warning" }]);
  expect(typed.previewOverlayCalls.at(-1)).toEqual([]);
  const textOnly = harness({ role: "player", tokens: [{ id: "t1", x: 0, y: 0 }], pathfindMode: "rejects", pathfindError: () => new Error("not connected") });
  textOnly.select(["t1"]);
  await textOnly.drag({ dx: 100, dy: 0 });
  expect(textOnly.notices).toEqual([{ message: "tools.moveRejected", level: "warning" }]);
});

test("a drag whose tokens fail differently has ONE notice owner: the tool's notice and the host's rejection share the gesture", async () => {
  const h = harness({ role: "player", tokens: [{ id: "t1", x: 0, y: 0 }, { id: "t2", x: 100, y: 0 }], routeRejectsFor: ["t1"] });
  h.select(["t1", "t2"]);
  await h.drag({ dx: 100, dy: 0 });
  // t1's route failed and the tool spoke for the gesture; t2's request went out under the same gesture.
  expect(h.notices).toEqual([{ message: "tools.moveNoRoute", level: "warning" }]);
  expect(h.gestures).toHaveLength(1);
  const hostNotices: string[] = [];
  expect(noticeOncePerGesture(h.gestures[0], () => hostNotices.push("tools.moveRejected"))).toBe(false);
  expect(hostNotices).toEqual([]);
});

test("a multi-token drag whose route requests all time out raises one timeout notice", async () => {
  const h = harness({ role: "player", tokens: [{ id: "t1", x: 0, y: 0 }, { id: "t2", x: 100, y: 0 }], pathfindMode: "rejects", pathfindError: () => new RequestTimeoutError("pathfind request timeout") });
  h.select(["t1", "t2"]);
  await h.drag({ dx: 100, dy: 0 });
  expect(h.notices).toEqual([{ message: "tools.moveTimeout", level: "warning" }]);
});

test("a multi-token drag whose route requests all fail raises one notice, not one per token", async () => {
  const rejected = harness({ role: "player", tokens: [{ id: "t1", x: 0, y: 0 }, { id: "t2", x: 100, y: 0 }, { id: "t3", x: 200, y: 0 }], pathfindMode: "rejects" });
  rejected.select(["t1", "t2", "t3"]);
  await rejected.drag({ dx: 100, dy: 0 });
  expect(rejected.notices).toEqual([{ message: "tools.moveNoRoute", level: "warning" }]);
  const blocked = harness({ role: "player", tokens: [{ id: "t1", x: 0, y: 0 }, { id: "t2", x: 100, y: 0 }], pathfindMode: "emptyBlocked" });
  blocked.select(["t1", "t2"]);
  await blocked.drag({ dx: 100, dy: 0 });
  expect(blocked.notices).toEqual([{ message: "tools.moveBlocked", level: "warning" }]);
});

test("every moveRequest of one drag shares one gesture identity; the next drag gets a fresh one", async () => {
  const h = harness({ role: "player", tokens: [{ id: "t1", x: 0, y: 0 }, { id: "t2", x: 100, y: 0 }] });
  h.select(["t1", "t2"]);
  await h.drag({ dx: 100, dy: 0 });
  expect(h.gestures).toHaveLength(2);
  expect(h.gestures[0]).toBeDefined();
  expect(h.gestures[1]).toBe(h.gestures[0]);
  await h.drag({ dx: 100, dy: 0 });
  expect(h.gestures).toHaveLength(4);
  expect(h.gestures[2]).not.toBe(h.gestures[0]);
});

test("a synchronous throw from moveRequest is caught: preview dropped and one notice", async () => {
  // An unhandled rejection also fails the whole vitest run, so the assertions below pin the
  // recovery (preview dropped, exactly one notice) that the catch path provides.
  const h = harness({ role: "player", tokens: [{ id: "t1", x: 0, y: 0 }, { id: "t2", x: 100, y: 0 }], moveRequestThrows: true });
  h.select(["t1", "t2"]);
  await h.drag({ dx: 100, dy: 0 });
  await flush();
  expect(h.moveRequests).toHaveLength(2);
  expect(h.previewOverlayCalls.at(-1)).toEqual([]);
  expect(h.notices).toEqual([{ message: "tools.moveRejected", level: "warning" }]);
});

test("a route that never leaves the start raises the blocked notice only when the server truncated it", async () => {
  const blocked = harness({ role: "player", tokens: [{ id: "t1", x: 0, y: 0 }], pathfindMode: "emptyBlocked" });
  blocked.select(["t1"]);
  await blocked.drag({ dx: 100, dy: 0 });
  expect(blocked.notices).toEqual([{ message: "tools.moveBlocked", level: "warning" }]);
  const plain = harness({ role: "player", tokens: [{ id: "t1", x: 0, y: 0 }], pathfindMode: "emptyPlain" });
  plain.select(["t1"]);
  await plain.drag({ dx: 100, dy: 0 });
  expect(plain.notices).toEqual([]);
});

test("a rejected moveRequest raises no notice from the tool: the host's move-outcome consumer owns it", async () => {
  const h = harness({ role: "player", tokens: [{ id: "t1", x: 0, y: 0 }], moveRequestRejects: true });
  h.select(["t1"]);
  await h.drag({ dx: 100, dy: 0 });
  expect(h.moveRequests.length).toBe(1);
  expect(h.notices).toEqual([]);
});

test("a GM click on a light marker selects it for editing; a wall click selects the wall", () => {
  const docs = new DocumentStore();
  docs.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [
      { op: "create", doc: buildSceneDoc("w1", {}, "s1") },
      {
        op: "create",
        doc: buildLightDoc("w1", "s1", {
          x: 500, y: 500,
          elevation: null,
          emission: { color: "#fff", intensity: 1, brightRadius: 2, dimRadius: 6, falloff: null, enabled: true, rotation: 0, followTokenRotation: false },
        }, "light-1"),
      },
      {
        op: "create",
        doc: buildSceneEntityDoc("w1", "s1", "wall", {
          seg: { x1: 0, y1: 700, x2: 400, y2: 700 }, blocksSight: true, blocksMove: true, blocksLight: true,
        }),
      },
    ],
  });
  const wallId = docs.query("wall")[0].id;
  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({}));
  const ctx: ToolContext = {
    scene: bridge, dispatchIntent: () => {}, documents: docs,
    assets: new AssetResolver(), world: "w1", role: "gm", sendPing: () => {}, t: (k) => k,
    tokenSelection: new TokenSelection(),
  };
  const controller = new ToolController(ctx);
  const tool = makeSelectMoveTool(ctx, controller);

  // A click on the light marker opens the light editor selection.
  expect(tool.onPointerDown({ x: 500, y: 500 }, noShift)).toBe(false); // unhandled: camera may still pan
  expect(controller.editingEntity).toEqual({ kind: "light", id: "light-1" });

  // A click on the wall segment opens the wall editor selection.
  tool.onPointerUp({ x: 500, y: 500 }, noShift);
  expect(tool.onPointerDown({ x: 200, y: 700 }, noShift)).toBe(false);
  expect(controller.editingEntity).toEqual({ kind: "wall", id: wallId });

  // Empty space clears the editing selection.
  tool.onPointerUp({ x: 200, y: 700 }, noShift);
  tool.onPointerDown({ x: 900, y: 900 }, noShift);
  expect(controller.editingEntity).toBeNull();
});

test("a non-GM click on a wall or light opens NO editor selection", () => {
  const docs = new DocumentStore();
  docs.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [
      { op: "create", doc: buildSceneDoc("w1", {}, "s1") },
      {
        op: "create",
        doc: buildSceneEntityDoc("w1", "s1", "wall", {
          seg: { x1: 0, y1: 700, x2: 400, y2: 700 }, blocksSight: true, blocksMove: true, blocksLight: true,
        }),
      },
    ],
  });
  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({}));
  const ctx: ToolContext = {
    scene: bridge, dispatchIntent: () => {}, documents: docs,
    assets: new AssetResolver(), world: "w1", role: "player", sendPing: () => {}, t: (k) => k,
    tokenSelection: new TokenSelection(),
  };
  const controller = new ToolController(ctx);
  const tool = makeSelectMoveTool(ctx, controller);
  expect(tool.onPointerDown({ x: 200, y: 700 }, noShift)).toBe(false);
  expect(controller.editingEntity).toBeNull();
});

// --- Door click-to-toggle (`ClientMsg::ToggleDoor` via `ctx.toggleDoor`, never an Update op) ---

/** A ToolContext driving the select/move tool over one scene with one door wall (segment
 * (0,700)-(400,700)) and optionally one token owned by the clicker ("u1"), plus recording
 * stubs for the `toggleDoor` seam and `notify` toasts. The wall sits at y=700: a token at
 * (100,650) is 50 world units from it (in reach: ≤ 1.5 × the 100-unit cell); a token at
 * (100,100) is 600 units out (beyond reach). */
function doorHarness(opts: {
  role: "gm" | "player";
  door: "closed" | "open" | "locked" | null;
  tokenAt?: { x: number; y: number };
  toggleDoorRejects?: boolean;
}) {
  const docs = new DocumentStore();
  const ops: WireOperation[] = [
    { op: "create", doc: buildSceneDoc("w1", {}, "s1") },
    {
      op: "create",
      doc: buildSceneEntityDoc("w1", "s1", "wall", {
        seg: { x1: 0, y1: 700, x2: 400, y2: 700 },
        blocksSight: true, blocksMove: true, blocksLight: true,
        ...(opts.door ? { door: opts.door } : {}),
      }, "wall-1"),
    },
  ];
  if (opts.tokenAt) {
    const tok = buildTokenDoc("w1", "s1", {
      x: opts.tokenAt.x, y: opts.tokenAt.y, w: 100, h: 100, rotation: 0,
      visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null, lock_rotation: false, identity_hidden: false,
    }, "tok1");
    tok.owner = "u1";
    ops.push({ op: "create", doc: tok });
  }
  docs.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops });

  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({}));
  const toggles: { scene: string; wall: string; to: string }[] = [];
  const notes: string[] = [];
  const dispatched: WireOperation[] = [];
  const ctx: ToolContext = {
    scene: bridge,
    dispatchIntent: (o) => dispatched.push(...o),
    documents: docs,
    assets: new AssetResolver(),
    world: "w1",
    role: opts.role,
    sendPing: () => {},
    t: (k) => k,
    tokenSelection: new TokenSelection(),
    selfId: "u1",
    toggleDoor: (scene, wall, to) => {
      toggles.push({ scene, wall, to });
      return opts.toggleDoorRejects ? Promise.reject(new Error("door rejected")) : Promise.resolve();
    },
    notify: (m) => {
      notes.push(m);
    },
  };
  const controller = new ToolController(ctx);
  const tool = makeSelectMoveTool(ctx, controller);
  return { tool, toggles, notes, dispatched, controller };
}

test("a non-GM click on a closed door sends a ToggleDoor through the seam (never an Update intent)", () => {
  const h = doorHarness({ role: "player", door: "closed", tokenAt: { x: 100, y: 650 } });
  expect(h.tool.onPointerDown({ x: 300, y: 700 }, noShift)).toBe(false);
  expect(h.toggles).toEqual([{ scene: "s1", wall: "wall-1", to: "open" }]);
  expect(h.dispatched).toEqual([]); // no document write — a dedicated correlated request
  expect(h.controller.editingEntity).toBeNull(); // the editor pick stays GM-only
});

test("a non-GM click on an open door toggles it closed", () => {
  const h = doorHarness({ role: "player", door: "open", tokenAt: { x: 100, y: 650 } });
  h.tool.onPointerDown({ x: 300, y: 700 }, noShift);
  expect(h.toggles).toEqual([{ scene: "s1", wall: "wall-1", to: "closed" }]);
});

test("a rejected door toggle surfaces as a notify toast, never silently swallowed", async () => {
  const h = doorHarness({ role: "player", door: "closed", tokenAt: { x: 100, y: 650 }, toggleDoorRejects: true });
  h.tool.onPointerDown({ x: 300, y: 700 }, noShift);
  expect(h.toggles).toHaveLength(1); // the request WAS sent
  await flush(); // the rejection lands asynchronously
  expect(h.notes).toEqual(["tools.doorToggleNotVisible"]);
});

test("clicking a locked door sends nothing (any role) and toasts the locked hint", () => {
  const h = doorHarness({ role: "player", door: "locked", tokenAt: { x: 100, y: 650 } });
  h.tool.onPointerDown({ x: 300, y: 700 }, noShift);
  expect(h.toggles).toEqual([]);
  expect(h.dispatched).toEqual([]);
  expect(h.notes).toEqual(["tools.doorLocked"]);
});

test("a non-GM with no owned token within reach sends nothing and toasts out-of-reach", () => {
  const h = doorHarness({ role: "player", door: "closed", tokenAt: { x: 100, y: 100 } });
  h.tool.onPointerDown({ x: 300, y: 700 }, noShift);
  expect(h.toggles).toEqual([]);
  expect(h.notes).toEqual(["tools.doorToggleOutOfReach"]);
});

test("a non-GM with no token at all sends nothing and toasts out-of-reach", () => {
  const h = doorHarness({ role: "player", door: "closed" });
  h.tool.onPointerDown({ x: 300, y: 700 }, noShift);
  expect(h.toggles).toEqual([]);
  expect(h.notes).toEqual(["tools.doorToggleOutOfReach"]);
});

test("a GM click on a door toggles it (no advisory pre-check) AND selects the wall for editing", () => {
  const h = doorHarness({ role: "gm", door: "closed" });
  h.tool.onPointerDown({ x: 300, y: 700 }, noShift);
  expect(h.toggles).toEqual([{ scene: "s1", wall: "wall-1", to: "open" }]);
  expect(h.controller.editingEntity).toEqual({ kind: "wall", id: "wall-1" });
});

test("a click on a doorless wall sends no toggle (a GM still gets the editor pick)", () => {
  const h = doorHarness({ role: "gm", door: null });
  h.tool.onPointerDown({ x: 300, y: 700 }, noShift);
  expect(h.toggles).toEqual([]);
  expect(h.notes).toEqual([]);
  expect(h.controller.editingEntity).toEqual({ kind: "wall", id: "wall-1" });
});
