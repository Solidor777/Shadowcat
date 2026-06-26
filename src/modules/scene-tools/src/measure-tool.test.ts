import { test, expect } from "vitest";
import { DocumentStore, AssetResolver, buildSceneDoc, buildTokenDoc, type WireOperation, type MoveStream } from "@shadowcat/core";
import type { Point } from "@shadowcat/render";
import { SceneInteractionBridge, TokenSelection } from "@shadowcat/ui-kit";
import { fakeSceneHost } from "@shadowcat/ui-kit/test";
import { makeMeasureTool, ToolController, type ToolContext } from "./controller.svelte";

/** Stub PointerEvent for tests that need to pass an event object. */
function ev(): PointerEvent { return {} as PointerEvent; }

/** Drain the microtask queue so async pathfind stubs resolve. */
function flush(): Promise<void> {
  return new Promise((r) => setTimeout(r, 0));
}
/** Alias used by commit-gesture tests (matches brief naming). */
const drain = flush;

function setup() {
  const measures: Array<{ from: Point; to: Point; label: string }> = [];
  let cleared = 0;
  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({
    gridDistance: () => 3,
    drawMeasure: (from, to, label) => measures.push({ from, to, label }),
    clearMeasure: () => { cleared++; },
  }));
  const sent: WireOperation[][] = [];
  const ctx: ToolContext = { scene: bridge, dispatchIntent: (ops) => sent.push(ops), documents: new DocumentStore(), assets: new AssetResolver(), world: "w1", sendPing: () => {} };
  return { tool: makeMeasureTool(ctx), measures, sent, clears: () => cleared };
}

test("measuring draws the distance label and persists nothing", () => {
  const { tool, measures, sent, clears } = setup();
  expect(tool.onPointerDown({ x: 0, y: 0 }, ev())).toBe(true);
  tool.onPointerMove({ x: 300, y: 0 }, ev());
  expect(measures.at(-1)).toEqual({ from: { x: 0, y: 0 }, to: { x: 300, y: 0 }, label: "3" });
  tool.onPointerUp({ x: 300, y: 0 }, ev());
  expect(clears()).toBe(1);
  expect(sent).toHaveLength(0); // client-local: no document, no broadcast
});

// --- Route-mode tests ---

/** Build a ToolContext with a scene + token seeded, a single token selected, and a pathfind stub. */
function setupRoute(over: {
  pathfind?: ToolContext["pathfind"];
  tokenIds?: string[];
} = {}) {
  const docs = new DocumentStore();
  // Scene with grid.distance so the budget label can be computed.
  docs.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [
      {
        op: "create",
        doc: buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: { perCell: 5, unit: "ft" } } }, "s1"),
      },
      {
        op: "create",
        doc: buildTokenDoc("w1", "s1", { x: 50, y: 50, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" } }, "tok-1"),
      },
    ],
  });

  const overlays: unknown[][] = [];
  const measures: Array<{ label: string }> = [];
  let measureClears = 0;
  let overlayClears = 0;

  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({
    snap: (p: Point) => p,
    gridDistance: () => 1,
    previewOverlay: (shapes) => overlays.push([...shapes]),
    clearOverlay: () => { overlayClears++; },
    drawMeasure: (_f, _t, label) => measures.push({ label }),
    clearMeasure: () => { measureClears++; },
  }));

  const tokenIds = over.tokenIds ?? ["tok-1"];
  const sel = new TokenSelection();
  sel.set(tokenIds);

  const ctx: ToolContext = {
    scene: bridge,
    dispatchIntent: () => {},
    documents: docs,
    assets: new AssetResolver(),
    world: "w1",
    sendPing: () => {},
    tokenSelection: sel,
    pathfind: over.pathfind,
  };

  return { tool: makeMeasureTool(ctx), overlays, measures, overlayClears: () => overlayClears, measureClears: () => measureClears };
}

test("measure tool routes via pathfind for the selected token and previews the path", async () => {
  const pathfind: ToolContext["pathfind"] = async () => ({
    path: [[50, 50], [150, 50]] as [number, number][],
    cost: 2,
  });
  const { tool, overlays, measures } = setupRoute({ pathfind });

  tool.onPointerDown({ x: 50, y: 50 }, ev());
  tool.onPointerMove({ x: 150, y: 50 }, ev());
  await flush(); // allow the async pathfind to resolve

  expect(overlays.length).toBeGreaterThan(0); // a routed polyline was previewed
  expect(measures.length).toBeGreaterThan(0);
  expect(measures.at(-1)!.label).toContain("10 ft"); // budget = cost(2) × perCell(5)
});

test("measure tool falls back to plain anchor-point measure when no token is selected", () => {
  const pathfinderCalled: boolean[] = [];
  const pathfind: ToolContext["pathfind"] = async () => {
    pathfinderCalled.push(true);
    return { path: [], cost: 0 };
  };
  // Build the route context but override tokenIds to empty — no selection.
  const { tool, overlays } = setupRoute({ pathfind, tokenIds: [] });

  tool.onPointerDown({ x: 0, y: 0 }, ev());
  tool.onPointerMove({ x: 100, y: 0 }, ev());
  tool.onPointerUp({ x: 100, y: 0 }, ev());

  expect(pathfinderCalled).toHaveLength(0); // fallback: plain measure, no pathfind
  expect(overlays).toHaveLength(0);         // no overlay in plain-measure mode
});

test("measure tool clears overlay and measure label on pointer up (mid-gesture-clear)", async () => {
  const pathfind: ToolContext["pathfind"] = async () => ({
    path: [[50, 50], [150, 50]] as [number, number][],
    cost: 2,
  });
  const { tool, overlayClears, measureClears } = setupRoute({ pathfind });

  tool.onPointerDown({ x: 50, y: 50 }, ev());
  tool.onPointerMove({ x: 150, y: 50 }, ev());
  await flush();
  tool.onPointerUp({ x: 150, y: 50 }, ev());

  expect(overlayClears()).toBeGreaterThan(0);   // overlay cleared on release
  expect(measureClears()).toBeGreaterThan(0);   // measure label cleared on release
});

test("measure tool with no pathfind function falls back to plain measure", () => {
  // No pathfind: undefined means the seam is absent (e.g. older host).
  const measures: Array<{ from: Point; to: Point; label: string }> = [];
  let cleared = 0;
  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({
    gridDistance: () => 4,
    drawMeasure: (from, to, label) => measures.push({ from, to, label }),
    clearMeasure: () => { cleared++; },
  }));
  const sel = new TokenSelection();
  sel.set(["tok-1"]);
  const ctx: ToolContext = {
    scene: bridge,
    dispatchIntent: () => {},
    documents: new DocumentStore(),
    assets: new AssetResolver(),
    world: "w1",
    sendPing: () => {},
    tokenSelection: sel,
    // pathfind intentionally omitted — defensive fallback
  };
  const tool = makeMeasureTool(ctx);

  tool.onPointerDown({ x: 0, y: 0 }, ev());
  tool.onPointerMove({ x: 200, y: 0 }, ev());
  expect(measures.at(-1)?.label).toBe("4"); // plain gridDistance label
  tool.onPointerUp({ x: 200, y: 0 }, ev());
  expect(cleared).toBe(1);
});

test("measure tool onDeactivate clears route overlay on tool swap (mid-gesture-clear on toggle)", async () => {
  // Verifies the ToolController.toggle contract: when switching away from measure mid-gesture,
  // onDeactivate fires and clears the live overlay + budget label. We test onDeactivate
  // directly because ToolController.toggle calls tool.onDeactivate?.() before swapping.
  const { tool, overlayClears, measureClears } = setupRoute({
    pathfind: async () => ({ path: [[50, 50], [150, 50]] as [number, number][], cost: 2 }),
  });

  tool.onPointerDown({ x: 50, y: 50 }, ev());
  tool.onPointerMove({ x: 150, y: 50 }, ev());
  await flush();

  const before = overlayClears() + measureClears();
  tool.onDeactivate!();
  expect(overlayClears() + measureClears()).toBeGreaterThan(before);
});

test("ToolController.toggle fires onDeactivate on outgoing measure tool", () => {
  // End-to-end: toggle measure → toggle ping → ToolController must call onDeactivate on
  // the outgoing measure tool, which in turn calls clearRoute (clearOverlay + clearMeasure).
  const docs = new DocumentStore();
  docs.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [
      { op: "create", doc: buildSceneDoc("w1", { grid: { kind: "square", size: 100 } }, "s1") },
    ],
  });

  let overlayCleared = 0;
  let measureCleared = 0;
  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({
    clearOverlay: () => { overlayCleared++; },
    clearMeasure: () => { measureCleared++; },
  }));

  const ctx: ToolContext = {
    scene: bridge,
    dispatchIntent: () => {},
    documents: docs,
    assets: new AssetResolver(),
    world: "w1",
    sendPing: () => {},
  };

  const controller = new ToolController(ctx);
  controller.toggle("measure"); // activate measure tool

  const before = overlayCleared + measureCleared;
  controller.toggle("ping");   // swap to ping — must fire measure.onDeactivate
  // clearRoute calls clearOverlay + clearMeasure, so the combined count must increase.
  expect(overlayCleared + measureCleared).toBeGreaterThan(before);
});

test("measure tool accumulates multiple waypoints and passes them to pathfind in order", async () => {
  // Verifies that two onPointerDown clicks build [wp1, wp2, goal] for the pathfind call.
  const docs = new DocumentStore();
  docs.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [
      { op: "create", doc: buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: { perCell: 5, unit: "ft" } } }, "s1") },
      { op: "create", doc: buildTokenDoc("w1", "s1", { x: 50, y: 50, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" } }, "tok-1") },
    ],
  });

  const calls: { start: [number,number]; waypoints: [number,number][] }[] = [];
  const pathfind: ToolContext["pathfind"] = async (_, start, waypoints) => {
    calls.push({ start, waypoints: [...waypoints] });
    return { path: [[start[0], start[1]], [waypoints.at(-1)![0], waypoints.at(-1)![1]]] as [number,number][], cost: waypoints.length };
  };

  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({ snap: (p: Point) => p, previewOverlay: () => {}, clearOverlay: () => {}, drawMeasure: () => {}, clearMeasure: () => {} }));

  const sel = new TokenSelection();
  sel.set(["tok-1"]);
  const ctx: ToolContext = { scene: bridge, dispatchIntent: () => {}, documents: docs, assets: new AssetResolver(), world: "w1", sendPing: () => {}, tokenSelection: sel, pathfind };

  const tool = makeMeasureTool(ctx);

  // Click waypoint 1 at (100,50), waypoint 2 at (150,50), then hover to goal (200,50).
  tool.onPointerDown({ x: 100, y: 50 }, ev()); // wp1 pushed
  tool.onPointerDown({ x: 150, y: 50 }, ev()); // wp2 pushed
  tool.onPointerMove({ x: 200, y: 50 }, ev()); // triggers pathfind([100,50],[150,50],[200,50])
  await flush();

  expect(calls).toHaveLength(1);
  // start is the token center (50,50), NOT the first waypoint.
  expect(calls[0].start).toEqual([50, 50]);
  // waypoints must be [wp1, wp2, goal] in order.
  expect(calls[0].waypoints).toEqual([[100, 50], [150, 50], [200, 50]]);
});

// --- Route-commit (double-click) tests ---

/** Controllable clock injected into ctx.now for double-click timing tests. */
interface FakeNow {
  /** Returns the current fake timestamp. */
  (): number;
  /** Advance the fake clock by `ms` milliseconds. */
  advance(ms: number): void;
}

function makeFakeNow(initial = 0): FakeNow {
  let t = initial;
  const fn = (): number => t;
  fn.advance = (ms: number): void => { t += ms; };
  return fn;
}

/**
 * Build a ToolContext wired for route-commit tests: a scene + token seeded in the store,
 * a single token selected, injected pathfind/moveRequest/animateAlongPath stubs, and
 * a controllable clock. Returns the ctx, the clock, and the backing store.
 */
function seedRouteCtx(over: {
  pathfind: ToolContext["pathfind"];
  moveRequest?: ToolContext["moveRequest"];
  dispatchIntent?: (ops: WireOperation[]) => void;
  animateAlongPath?: (id: string, path: [number, number][]) => void;
  onClearOverlay?: () => void;
  onClearMeasure?: () => void;
  tokenAt: { id: string; x: number; y: number };
}): { ctx: ToolContext; now: FakeNow; docs: DocumentStore } {
  const docs = new DocumentStore();
  docs.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [
      {
        op: "create",
        doc: buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: { perCell: 5, unit: "ft" } } }, "s1"),
      },
      {
        op: "create",
        doc: buildTokenDoc("w1", "s1", {
          x: over.tokenAt.x, y: over.tokenAt.y, w: 100, h: 100, rotation: 0,
          visual: { kind: "image", asset: "a" },
        }, over.tokenAt.id),
      },
    ],
  });

  // Sequence counter for the store-advancing default dispatchIntent.
  let seq = 2;

  const now = makeFakeNow();

  const animateSpy = over.animateAlongPath ?? (() => {});
  const bridge = new SceneInteractionBridge();
  bridge.attach(fakeSceneHost({
    snap: (p: Point) => p,
    previewOverlay: () => {},
    clearOverlay: over.onClearOverlay ?? (() => {}),
    drawMeasure: () => {},
    clearMeasure: over.onClearMeasure ?? (() => {}),
    animateAlongPath: animateSpy,
  }));

  const sel = new TokenSelection();
  sel.set([over.tokenAt.id]);

  // Default dispatchIntent applies each batch's ops into `docs` via applyCommand so
  // subsequent ctx.documents.get() calls see the updated system values.
  const defaultDispatch = (ops: WireOperation[]): void => {
    docs.applyCommand({ seq: seq++, world_id: "w1", author: "a", ts: 0, ops });
  };

  const ctx: ToolContext = {
    scene: bridge,
    dispatchIntent: over.dispatchIntent ?? defaultDispatch,
    documents: docs,
    assets: new AssetResolver(),
    world: "w1",
    sendPing: () => {},
    tokenSelection: sel,
    pathfind: over.pathfind,
    moveRequest: over.moveRequest,
    now,
  };

  return { ctx, now, docs };
}

test("double-click commits via moveRequest (animation is broadcast-driven)", async () => {
  const moves: Array<{ tokenId: string; path: [number,number][] }> = [];
  const moveRequest: ToolContext["moveRequest"] = async (_s, tokenId, path) => {
    moves.push({ tokenId, path });
    return { requestId: "r1", tokenId, mover: "u1", scene: "s1", startServerMs: 0, durationMs: 300, stop: path.at(-1)!, samples: [], moverVision: null };
  };
  const { ctx, now } = seedRouteCtx({
    pathfind: async () => ({ path: [[0,0],[100,0],[100,100]] as [number,number][], cost: 2 }),
    moveRequest,
    tokenAt: { id: "tok1", x: 0, y: 0 },
  });
  const tool = makeMeasureTool(ctx);
  tool.onPointerDown({ x: 100, y: 100 }, ev()); tool.onPointerUp({ x: 100, y: 100 }, ev());
  now.advance(100);
  tool.onPointerDown({ x: 100, y: 100 }, ev()); tool.onPointerUp({ x: 100, y: 100 }, ev());
  await drain();
  expect(moves).toEqual([{ tokenId: "tok1", path: [[0,0],[100,0],[100,100]] }]);
  // Animation is now broadcast-driven via onMoveStream for all scene viewers.
});

test("a single click in route mode does NOT commit", async () => {
  const moves: Array<unknown> = [];
  const { ctx } = seedRouteCtx({
    pathfind: async () => ({ path: [[0, 0], [100, 0]] as [number, number][], cost: 1 }),
    moveRequest: async (_s, tokenId, path) => {
      moves.push({ tokenId, path });
      return { requestId: "r1", tokenId, mover: "u1", scene: "s1", startServerMs: 0, durationMs: 300, stop: path.at(-1)!, samples: [], moverVision: null };
    },
    tokenAt: { id: "tok1", x: 0, y: 0 },
  });
  const tool = makeMeasureTool(ctx);
  tool.onPointerDown({ x: 100, y: 0 }, ev());
  await drain();
  expect(moves.length).toBe(0); // moveRequest must not fire on a single click
});

test("route commit survives its own pointer-up: moveRequest resolves after pointer-up and still fires", async () => {
  // REGRESSION guard: in production, onPointerDown(double) is immediately followed by
  // onPointerUp. The committing flag must suppress clearRoute() in onPointerUp so the
  // in-flight moveRequest's seq guard is not invalidated.
  //
  // The deferred moveRequest resolves ONLY after both pointer-ups to catch any code that
  // allows onPointerUp to invalidate an in-flight commit.

  let resolveMoveRequest!: (r: MoveStream) => void;
  const deferredMoveRequest: ToolContext["moveRequest"] = (_scene, _tokenId, _path) =>
    new Promise((res) => { resolveMoveRequest = res; });

  let clearCount = 0;

  const { ctx, now } = seedRouteCtx({
    pathfind: async () => ({ path: [[0, 0], [200, 0], [200, 200]] as [number, number][], cost: 4 }),
    moveRequest: deferredMoveRequest,
    onClearOverlay: () => { clearCount++; },
    tokenAt: { id: "tok1", x: 0, y: 0 },
  });

  const tool = makeMeasureTool(ctx);

  // First pointer-down — starts the double-click window.
  tool.onPointerDown({ x: 200, y: 200 }, ev());
  tool.onPointerUp({ x: 200, y: 200 }, ev()); // trailing up (real engine always sends up)

  // Advance time within DOUBLE_CLICK_MS, then second pointer-down → commit fires.
  now.advance(100);
  tool.onPointerDown({ x: 200, y: 200 }, ev()); // second down → commitRoute called
  tool.onPointerUp({ x: 200, y: 200 }, ev()); // trailing up on the double-click release

  // Let the internal pathfind resolve so moveRequest is called and resolveMoveRequest is assigned.
  // moveRequest itself is still deferred (resolveMoveRequest not yet called).
  await drain();

  // moveRequest has been called but NOT resolved. committing=true must suppress the already-fired
  // pointer-up's clearRoute — committing must still be true at this point.
  const clearsBefore = clearCount;

  resolveMoveRequest({
    requestId: "r1", tokenId: "tok1", mover: "u1", scene: "s1",
    startServerMs: 0, durationMs: 500,
    stop: [200, 200] as [number, number],
    samples: [], moverVision: null,
  });
  await drain();

  // The commit must have survived: finish() ran, calling clearRoute() → clearOverlay.
  expect(clearCount).toBeGreaterThan(clearsBefore);
});

test("rejected moveRequest calls clearRoute and does NOT animate", async () => {
  // Fix 3: reject path — moveRequest rejects → overlay/measure cleared, no animation.
  let overlayClears = 0;
  let measureClears = 0;
  const animated: Array<{ id: string; path: [number, number][] }> = [];

  const { ctx, now } = seedRouteCtx({
    pathfind: async () => ({ path: [[0, 0], [100, 0]] as [number, number][], cost: 1 }),
    moveRequest: async () => Promise.reject(new Error("server denied")),
    animateAlongPath: (id, path) => animated.push({ id, path }),
    onClearOverlay: () => { overlayClears++; },
    onClearMeasure: () => { measureClears++; },
    tokenAt: { id: "tok1", x: 0, y: 0 },
  });

  const tool = makeMeasureTool(ctx);

  // First click — seeds lastDownAt / lastDownPt.
  tool.onPointerDown({ x: 100, y: 0 }, ev());
  tool.onPointerUp({ x: 100, y: 0 }, ev());
  // Double-click within window → commitRoute fires.
  now.advance(100);
  tool.onPointerDown({ x: 100, y: 0 }, ev());
  tool.onPointerUp({ x: 100, y: 0 }, ev());
  await drain();

  expect(animated.length).toBe(0);              // no animation on reject
  expect(overlayClears + measureClears).toBeGreaterThan(0); // route cleared via finish()
});

test("cache-hit: commitRoute reuses lastPreviewedPath and does not call pathfind again", async () => {
  // Fix 4: preview populates lastPreviewedPath; double-click commit reuses it — pathfind
  // must NOT be called a second time, and moveRequest receives the cached path.
  const pathfindCalls: number[] = [];
  const cachedPath: [number, number][] = [[0, 0], [100, 0], [100, 100]];

  const moveRequestReceived: Array<[number, number][]> = [];
  const moveRequest: ToolContext["moveRequest"] = async (_s, _id, path) => {
    moveRequestReceived.push(path);
    return { requestId: "r1", tokenId: "tok1", mover: "u1", scene: "s1", startServerMs: 0, durationMs: 300, stop: path.at(-1)!, samples: [], moverVision: null };
  };

  const { ctx, now } = seedRouteCtx({
    pathfind: async () => {
      pathfindCalls.push(1);
      return { path: cachedPath, cost: 2 };
    },
    moveRequest,
    tokenAt: { id: "tok1", x: 0, y: 0 },
  });

  const tool = makeMeasureTool(ctx);

  // Seed the preview: onPointerMove triggers pathfind → lastPreviewedPath = cachedPath.
  tool.onPointerDown({ x: 100, y: 100 }, ev());
  tool.onPointerMove({ x: 100, y: 100 }, ev());
  await flush(); // let preview pathfind resolve → lastPreviewedPath populated

  const pathfindCallsAfterPreview = pathfindCalls.length;
  expect(pathfindCallsAfterPreview).toBeGreaterThan(0); // preview did call pathfind

  // Double-click at the same goal (no intermediate clear).
  now.advance(100);
  tool.onPointerDown({ x: 100, y: 100 }, ev());
  tool.onPointerUp({ x: 100, y: 100 }, ev());
  await drain();

  // Pathfind must NOT have been called again — commit reuses lastPreviewedPath.
  expect(pathfindCalls.length).toBe(pathfindCallsAfterPreview);
  // moveRequest must have received the cached path.
  expect(moveRequestReceived).toHaveLength(1);
  expect(moveRequestReceived[0]).toEqual(cachedPath);
});

test("stale commit resolve does not clear a newer commit's suppression flag", async () => {
  // Invariant: only the still-current commit (seq === pendingSeq) may mutate `committing`.
  // Commit A starts → onDeactivate aborts A → tool reactivated → commit B starts →
  // A's stale moveRequest resolves. The stale resolve must not touch `committing`, or it
  // wipes B's pointer-up suppression and a trailing up bumps pendingSeq, silently bailing B.

  // Two independently-controllable deferreds so A and B resolve in controlled order.
  let resolveA!: (r: MoveStream) => void;
  let resolveB!: (r: MoveStream) => void;
  let callCount = 0;
  const deferredMoveRequest: ToolContext["moveRequest"] = (_scene, _tokenId, _path) =>
    new Promise((res) => {
      if (callCount === 0) { resolveA = res; } else { resolveB = res; }
      callCount++;
    });

  let clearCount = 0;

  const { ctx, now } = seedRouteCtx({
    pathfind: async (_s, _start, waypoints) => ({
      path: [_start, waypoints.at(-1)!] as [number, number][],
      cost: 1,
    }),
    moveRequest: deferredMoveRequest,
    onClearOverlay: () => { clearCount++; },
    tokenAt: { id: "tok1", x: 0, y: 0 },
  });

  const tool = makeMeasureTool(ctx);

  // Start commit A via double-click, then abort it with onDeactivate.
  tool.onPointerDown({ x: 100, y: 0 }, ev());
  now.advance(100);
  tool.onPointerDown({ x: 100, y: 0 }, ev()); // double-click → commit A in flight
  await drain(); // let A's pathfind resolve so moveRequest is called
  tool.onDeactivate!(); // aborts A: committing=false, pendingSeq bumped

  // Start commit B (tool reactivated on same instance, as ToolController reuses instances).
  tool.onPointerDown({ x: 200, y: 0 }, ev()); // first click of new double-click
  now.advance(100);
  tool.onPointerDown({ x: 200, y: 0 }, ev()); // double-click → commit B in flight
  await drain(); // let B's pathfind resolve so moveRequest is called

  // A resolves while B is in flight. A is stale (seq_A < pendingSeq).
  // Must not touch committing — B's suppression flag must remain true.
  resolveA({ requestId: "rA", tokenId: "tok1", mover: "u1", scene: "s1", startServerMs: 0, durationMs: 300, stop: [100, 0] as [number, number], samples: [], moverVision: null });
  await drain();

  // A trailing pointer-up must be suppressed by B's still-intact committing flag.
  // If the stale resolve wiped committing, clearRoute fires here and bumps pendingSeq,
  // making B stale too — so B's finish() would never run.
  tool.onPointerUp({ x: 200, y: 0 }, ev());

  // Capture count AFTER the pointer-up: if committing was wiped, the pointer-up already
  // bumped pendingSeq, so B would resolve stale with no finish() call.
  const clearsBefore = clearCount;

  // B resolves: must find seq === pendingSeq and call finish() → clearRoute().
  resolveB({ requestId: "rB", tokenId: "tok1", mover: "u1", scene: "s1", startServerMs: 0, durationMs: 300, stop: [200, 0] as [number, number], samples: [], moverVision: null });
  await drain();

  // B's finish() fired: clearRoute() was called, incrementing clearCount.
  expect(clearCount).toBeGreaterThan(clearsBefore);
});
