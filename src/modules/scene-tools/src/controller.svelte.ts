// scene-tools active-tool state + the SceneTool implementations. Reaches the engine
// only through the public AppContext seams (the scene bridge for tool activation/snap,
// dispatchIntent for document writes); it never imports core-ui (contract-only
// boundary). The tool factories close over the context.
import { rectPoints, ellipsePoints, circlePoints, conePoints, squarePoints, parseColor, type SceneTool, type Point } from "@shadowcat/render";
import { buildTokenDoc, buildTokenFromActor, buildSceneEntityDoc, resolveTokenBox, resolveTokenActor, footprintRadius, type ReadableDocuments, type AssetResolver, type WireOperation, type PathResult, type MoveStream } from "@shadowcat/core";
import type { SceneInteraction, ActorSelection, TokenSelection } from "@shadowcat/ui-kit";
import { topTokenAt } from "./hit-test";

export type ToolId = "select" | "place" | "draw" | "template" | "measure" | "ping" | "wall";
export type DrawMode = "freehand" | "rect" | "ellipse" | "line";
export type TemplateMode = "circle" | "cone" | "rect" | "line";

/** The AppContext slice the tools need. `documents` is the optimistic view, so a
 * just-auto-created scene / just-placed token is visible to the tools immediately. */
export interface ToolContext {
  scene: SceneInteraction;
  /** The actor to stamp (the place tool); when set it takes precedence over selectedAsset. */
  actorSelection?: ActorSelection;
  /** Selected token ids (group-select); the select tool reads + moves the whole set. */
  tokenSelection?: TokenSelection;
  dispatchIntent: (ops: WireOperation[]) => void;
  documents: ReadableDocuments;
  assets: AssetResolver;
  world: string;
  /** Broadcast a transient ping at scene coords (the ping tool). */
  sendPing: (x: number, y: number) => void;
  /** Monotonic clock for drag-intent coalescing; defaults to Date.now (injected in tests). */
  now?: () => number;
  /** Grid A* pathfind seam (from AppContext). When present and a single token is
   * selected, the measure tool routes through it instead of the plain gridDistance
   * mode. When absent (older host or not connected), the tool falls back gracefully. */
  pathfind?: (
    scene: string,
    start: [number, number],
    waypoints: [number, number][],
    footprintRadius: number,
  ) => Promise<PathResult>;
  /** Request server-authoritative move execution (from AppContext). When present,
   * double-click commit sends a MoveRequest; animation is broadcast-driven via MoveStream
   * for all viewers. When absent, double-click is a no-op (graceful degradation). */
  moveRequest?: (
    scene: string,
    tokenId: string,
    path: [number, number][],
  ) => Promise<MoveStream>;
}

/** The active scene (single scene in M8d §15) + its grid cell size (default 100) and
 * distance scale (default 5 ft/cell, matching `resolveSceneSettings` defaults). */
function activeScene(ctx: ToolContext): { id: string; size: number; perCell: number; unit: string } | null {
  const scene = ctx.documents.query("scene")[0];
  if (!scene) return null;
  const grid = (scene.system as { grid?: { size?: number; distance?: { perCell: number; unit: string } } } | undefined)?.grid;
  const size = grid?.size ?? 100;
  const { perCell, unit } = grid?.distance ?? { perCell: 5, unit: "ft" };
  return { id: scene.id, size, perCell, unit };
}

/** Route color for the A* preview polyline (blue-teal, distinct from walls and selection). */
const ROUTE_COLOR = 0x3399ff;
/** Maximum milliseconds between two pointer-downs to count as a double-click. */
const DOUBLE_CLICK_MS = 350;
/** Maximum scene-coord distance between two pointer-downs to count as a double-click
 * (generous: post-snap, a double-click lands on the same cell center). */
const COMMIT_RADIUS = 12;

/** Owns the active-tool + selected-asset UI state and routes activation to the engine
 * via the scene bridge. */
export class ToolController {
  active = $state<ToolId | null>(null);
  /** The token art the place tool stamps; chosen in the asset picker. */
  selectedAsset = $state<string | null>(null);
  /** Draw-tool shape mode + stroke color. */
  drawMode = $state<DrawMode>("freehand");
  strokeColor = $state("#e0e0e0");
  /** Template-tool shape mode + color. */
  templateMode = $state<TemplateMode>("circle");
  templateColor = $state("#3388ff");
  readonly #tools: Record<ToolId, SceneTool>;

  constructor(private readonly ctx: ToolContext) {
    this.#tools = {
      select: makeSelectMoveTool(ctx),
      place: makePlaceTool(ctx, this),
      draw: makeDrawTool(ctx, this),
      template: makeTemplateTool(ctx, this),
      measure: makeMeasureTool(ctx),
      ping: makePingTool(ctx),
      wall: makeWallTool(ctx),
    };
  }

  /** Toggle a tool: re-selecting the active one clears it (back to camera).
   * Fires `onDeactivate` on the outgoing tool (if any) so tools with live
   * overlays can tear down before the new tool activates (mid-gesture-clear invariant). */
  toggle(id: ToolId): void {
    // Deactivate the outgoing tool before updating `active` so it can still read state.
    if (this.active) this.#tools[this.active].onDeactivate?.();
    this.active = this.active === id ? null : id;
    this.ctx.scene.setActiveTool(this.active ? this.#tools[this.active] : null);
  }
}

/** Click stamps a token at the snapped cell of the active scene. A selected actor takes
 * precedence (instanced if its `prototype` is set, else linked); otherwise the selected raw
 * asset is stamped. No scene, or neither an actor nor an asset selected → unhandled (camera pans). */
export function makePlaceTool(ctx: ToolContext, controller: ToolController): SceneTool {
  return {
    onPointerDown(p: Point): boolean {
      const scene = activeScene(ctx);
      if (!scene) return false;
      const c = ctx.scene.snap(p);
      const actorId = ctx.actorSelection?.selectedId ?? null;
      if (actorId) {
        const actor = ctx.documents.get(actorId);
        if (!actor) return false;
        const mode = (actor.system as { prototype?: boolean })?.prototype ? "instance" : "link";
        ctx.dispatchIntent([{ op: "create", doc: buildTokenFromActor(ctx.world, scene.id, actor, mode, c, scene.size) }]);
        // A unique (linked) actor places once by default: clear the selection so repeated
        // clicks don't stamp duplicate live-views. The user can opt to keep it selected
        // (keepAfterPlace). Instanced actors always stay selected for placing many.
        if (mode === "link" && !ctx.actorSelection?.keepAfterPlace) ctx.actorSelection?.select(null);
        return true;
      }
      const asset = controller.selectedAsset;
      if (!asset) return false;
      ctx.dispatchIntent([
        {
          op: "create",
          doc: buildTokenDoc(ctx.world, scene.id, { x: c.x, y: c.y, w: scene.size, h: scene.size, rotation: 0, visual: { kind: "image", asset } }),
        },
      ]);
      return true;
    },
    onPointerMove(): void {},
    onPointerUp(): void {},
  };
}

/** A draw gesture has visible extent: a freehand path of ≥2 points, or a two-corner
 * shape whose corners are ≥1 unit apart. A pure click has none — persisting it would
 * write an invisible junk drawing to the scene + event log. */
function hasExtent(mode: DrawMode, a: Point, b: Point, freehand: number[]): boolean {
  if (mode === "freehand") return freehand.length >= 4;
  return Math.hypot(b.x - a.x, b.y - a.y) >= 1;
}

/** Wall preview/segment color (matches the WallView render color). */
const WALL_COLOR = 0xd06060;

/** Drag to draw a wall segment (snapped endpoints); release persists a `wall` doc
 * (`blocksSight`+`blocksMove`). The server's collision check reads the same `seg`. GM-gated
 * (all rail tools are). No active scene → unhandled. */
export function makeWallTool(ctx: ToolContext): SceneTool {
  let anchor: Point | null = null;
  return {
    onPointerDown(p: Point): boolean {
      if (!activeScene(ctx)) return false;
      anchor = ctx.scene.snap(p);
      return true;
    },
    onPointerMove(p: Point): void {
      if (!anchor) return;
      const b = ctx.scene.snap(p);
      ctx.scene.previewOverlay([{ points: [anchor.x, anchor.y, b.x, b.y], closed: false, stroke: { color: WALL_COLOR, width: 4 }, fill: null }]);
    },
    onPointerUp(p: Point): void {
      if (!anchor) return;
      const scene = activeScene(ctx);
      const b = ctx.scene.snap(p);
      if (scene && Math.hypot(b.x - anchor.x, b.y - anchor.y) >= 1) {
        ctx.dispatchIntent([
          {
            op: "create",
            doc: buildSceneEntityDoc(ctx.world, scene.id, "wall", {
              seg: { x1: anchor.x, y1: anchor.y, x2: b.x, y2: b.y },
              blocksSight: true,
              blocksMove: true,
              blocksLight: true,
            }),
          },
        ]);
      }
      ctx.scene.clearOverlay();
      anchor = null;
    },
  };
}

/** Click to ping a location: broadcasts a transient marker. The server relays it back to
 * all members (incl. us), so the local ring arrives via the ping listener like any other —
 * no separate local echo. */
export function makePingTool(ctx: ToolContext): SceneTool {
  return {
    onPointerDown(p: Point): boolean {
      ctx.sendPing(p.x, p.y);
      return true;
    },
    onPointerMove(): void {},
    onPointerUp(): void {},
  };
}

/** Drag to measure: a client-local segment + whole-cell distance label. Never persists a
 * document or broadcasts — purely an overlay on the dragging client.
 *
 * Route mode activates when ALL of:
 *   1. `ctx.tokenSelection` has exactly ONE token id, AND
 *   2. `ctx.pathfind` is defined (i.e. the host provides the seam), AND
 *   3. There is an active scene.
 * In route mode each `onPointerMove` issues an A* pathfind request from the selected
 * token's center through any accumulated waypoints to the provisional goal; on resolve
 * `previewOverlay` renders the routed polyline and `drawMeasure` shows the movement
 * budget (cost × perCell + unit). `onPointerDown` snaps a waypoint onto the list.
 * `onPointerUp` / tool deactivation clears all overlays (mid-gesture-clear invariant).
 *
 * With 0 or >1 tokens selected, or no `ctx.pathfind`: falls back to the original
 * anchor→point gridDistance measure so plain measurement is always available. */
export function makeMeasureTool(ctx: ToolContext): SceneTool {
  // Plain-measure state.
  let anchor: Point | null = null;

  // Route-mode state.
  // `waypoints` accumulates user-clicked intermediate goals (snapped); the start
  // is always derived live from the selected token's position.
  let waypoints: [number, number][] = [];
  // The path returned by the most-recently-resolved preview pathfind. Reused by
  // commitRoute to avoid a second pathfind round-trip when the route is already known.
  let lastPreviewedPath: [number, number][] | null = null;
  // Track the in-flight pathfind so we can ignore stale responses that arrive
  // after a newer request has been issued (last-write-wins coalescing).
  let pendingSeq = 0;
  // When a commit is in flight, suppress pointer-down/move/up so a trailing
  // pointer-up cannot bump pendingSeq and invalidate the commit's seq guard.
  // Also prevents a stray down from starting a second commit concurrently.
  // Constraint: committing is set before seq is captured and cleared in finish()
  // (resolve), the reject handler, or onDeactivate (abort path).
  let committing = false;

  // Monotonic clock (injected in tests; defaults to Date.now).
  const now = ctx.now ?? ((): number => Date.now());
  // Double-click detection state: timestamp and snapped position of the last pointer-down.
  let lastDownAt = -Infinity;
  let lastDownPt: Point = { x: 0, y: 0 };

  /** True when the measure tool should operate in route mode (see above). */
  function inRouteMode(): boolean {
    return (
      ctx.pathfind !== undefined &&
      ctx.tokenSelection !== undefined &&
      ctx.tokenSelection.ids.size === 1
    );
  }

  /** Center of the single selected token, or null if unavailable. */
  function tokenCenter(): [number, number] | null {
    const sel = ctx.tokenSelection;
    if (!sel || sel.ids.size !== 1) return null;
    const [id] = [...sel.ids];
    const sys = ctx.documents.get(id)?.system as { x?: number; y?: number } | undefined;
    return [sys?.x ?? 0, sys?.y ?? 0];
  }

  /** Footprint radius of the single selected token (for pathfind clearance). Falls
   * back to 0.4 grid units (sub-cell) when the actor cannot be resolved. */
  function resolveFootprint(): number {
    const sel = ctx.tokenSelection;
    if (!sel || sel.ids.size !== 1) return 0.4;
    const [id] = [...sel.ids];
    const tokenDoc = ctx.documents.get(id);
    if (!tokenDoc) return 0.4;
    const eff = resolveTokenActor(tokenDoc, ctx.documents);
    return eff ? footprintRadius(eff) : 0.4;
  }

  /** Issue a pathfind request for the current waypoints + provisional goal `p`.
   * Ignores the response if a newer request has since been issued. The final element
   * of `allWaypoints` IS the goal (server contract: goal = waypoints.last(), spec §3.2). */
  function requestRoute(scene: { id: string; perCell: number; unit: string }, start: [number, number], goal: Point): void {
    if (!ctx.pathfind) return;
    const seq = ++pendingSeq;
    const fp = resolveFootprint();
    const allWaypoints: [number, number][] = [...waypoints, [goal.x, goal.y]];
    ctx.pathfind(scene.id, start, allWaypoints, fp).then(
      (result) => {
        if (seq !== pendingSeq) return; // superseded by a newer move
        // Cache the resolved path for reuse by commitRoute (avoids a second pathfind).
        lastPreviewedPath = result.path;
        // Render the routed polyline via previewOverlay.
        const pts = result.path.flat();
        ctx.scene.previewOverlay([{ points: pts, closed: false, stroke: { color: ROUTE_COLOR, width: 3 }, fill: null }]);
        // Budget label: rounds to whole distance units for display; the server-side cost
        // stays exact (diagonal rules like alternating/euclidean yield fractional cells).
        const budget = Math.round(result.cost * scene.perCell);
        const startPt: Point = { x: start[0], y: start[1] };
        ctx.scene.drawMeasure(startPt, goal, `${budget} ${scene.unit}`);
      },
      () => {
        if (seq !== pendingSeq) return;
        lastPreviewedPath = null;
        // No route available: clear the overlay and show a "no route" label.
        ctx.scene.clearOverlay();
        const startPt: Point = { x: start[0], y: start[1] };
        ctx.scene.drawMeasure(startPt, goal, "—");
      },
    );
  }

  /** Clear all route-mode overlays and reset waypoints (mid-gesture-clear invariant). */
  function clearRoute(): void {
    pendingSeq++; // invalidate any in-flight request
    ctx.scene.clearOverlay();
    ctx.scene.clearMeasure();
    waypoints = [];
    lastPreviewedPath = null;
  }

  /** Commit a route from the selected token's center to `goal`: send a MoveRequest to
   * the server and animate along the returned render-path on resolve. The authoritative
   * position arrives via the normal store Event (token → stop); the animator's any-ahead
   * rule recognizes it. On reject, clear the route overlay (no move).
   *
   * Simpler path: reuse the last previewed PathResult.path when already computed for the
   * same goal; if none is cached, do one pathfind then send the moveRequest.
   *
   * Invariant: `committing` is set TRUE before `seq` is captured, and cleared ONLY by
   * `finish()` on resolve, by the reject handler, or by `onDeactivate` (abort path).
   * This ensures pointer-up (which calls clearRoute in non-committing paths) cannot bump
   * `pendingSeq` between commit start and the async resolve — keeping `seq === pendingSeq`
   * true so the commit proceeds. */
  function commitRoute(goal: Point): void {
    if (!ctx.pathfind || !ctx.moveRequest || !ctx.tokenSelection || ctx.tokenSelection.ids.size !== 1) return;
    const scene = activeScene(ctx);
    const start = tokenCenter();
    if (!scene || !start) return;
    const tokenId = [...ctx.tokenSelection.ids][0];
    const fp = resolveFootprint();
    // Set committing BEFORE capturing seq so the pointer-up guard (committing check) is
    // already in place before the async call starts. onPointerUp/Move check committing
    // and return early, so they cannot bump pendingSeq while this commit is in flight.
    committing = true;
    const seq = ++pendingSeq;
    // Teardown shared by the success path and the reject path.
    const finish = (): void => { committing = false; clearRoute(); };

    // Inner function: given a proposed path, send the moveRequest and animate on resolve.
    // `moveRequest` is narrowed to non-undefined here; `commitRoute` gates on it above.
    const moveRequest = ctx.moveRequest;
    const sendRequest = (proposedPath: [number, number][]): void => {
      moveRequest(scene.id, tokenId, proposedPath).then(
        () => {
          // Stale: a newer commit (or onDeactivate) now owns committing + clearRoute.
          if (seq !== pendingSeq) return;
          // Animation is broadcast-driven via onMoveStream for all scene viewers;
          // no local animation from the moveRequest resolve value.
          finish();
        },
        // Stale reject: do nothing. Current reject: clear route (no move).
        () => { if (seq === pendingSeq) finish(); },
      );
    };

    // Reuse the last previewed path when available (avoids a redundant pathfind round-trip).
    // If none is cached, do one pathfind then send the moveRequest.
    if (lastPreviewedPath && lastPreviewedPath.length >= 2) {
      sendRequest(lastPreviewedPath);
    } else {
      ctx.pathfind(scene.id, start, [...waypoints, [goal.x, goal.y]], fp).then(
        (result) => {
          if (seq !== pendingSeq) return;
          if (result.path.length < 2) { finish(); return; }
          sendRequest(result.path);
        },
        () => { if (seq === pendingSeq) finish(); },
      );
    }
  }

  return {
    onPointerDown(p: Point): boolean {
      if (inRouteMode()) {
        // A commit is in flight: ignore further input until it settles. Prevents a
        // stray pointer-down from starting a second commit or bumping pendingSeq.
        if (committing) return true;
        const scene = activeScene(ctx);
        if (scene) {
          const snapped = ctx.scene.snap(p);
          const t = now();
          const isDouble =
            t - lastDownAt < DOUBLE_CLICK_MS &&
            Math.hypot(snapped.x - lastDownPt.x, snapped.y - lastDownPt.y) < COMMIT_RADIUS;
          if (isDouble) {
            // Consume the gesture so the next down starts fresh.
            lastDownAt = -Infinity;
            commitRoute(snapped);
            return true;
          }
          // Record this down as a potential first half of a double-click, then push
          // the waypoint for the existing preview behavior.
          lastDownAt = t;
          lastDownPt = snapped;
          waypoints.push([snapped.x, snapped.y]);
          return true;
        }
      }
      // Fallback: plain anchor-point measure.
      anchor = p;
      return true;
    },
    onPointerMove(p: Point): void {
      if (inRouteMode()) {
        // Suppress preview requests during a commit: a new pathfind call would bump
        // pendingSeq and invalidate the in-flight commit's seq guard.
        if (committing) return;
        const scene = activeScene(ctx);
        const start = tokenCenter();
        if (scene && start) {
          const goal = ctx.scene.snap(p);
          requestRoute(scene, start, goal);
        }
        return;
      }
      // Fallback: plain gridDistance measure.
      if (!anchor) return;
      ctx.scene.drawMeasure(anchor, p, String(ctx.scene.gridDistance(anchor, p)));
    },
    onPointerUp(_p: Point): void {
      if (inRouteMode()) {
        // When a commit is in flight, the commit owns its own teardown via finish().
        // Do NOT call clearRoute() here — that would bump pendingSeq and abort the commit.
        if (committing) return;
        // Release: clear overlays (mid-gesture-clear invariant). The actual move is
        // handled by the select-move tool / M10e-4 server gate — not here.
        clearRoute();
        return;
      }
      // Fallback: plain measure cleanup.
      if (!anchor) return;
      ctx.scene.clearMeasure();
      anchor = null;
    },
    onDeactivate(): void {
      // Tool-swap teardown: abort any in-flight commit and clear all overlays.
      // Setting committing=false before clearRoute lets clearRoute bump pendingSeq,
      // which causes any in-flight commit's seq guard to fail (seq !== pendingSeq).
      committing = false;
      clearRoute();
      // Also clear any in-progress plain-measure anchor.
      if (anchor) {
        ctx.scene.clearMeasure();
        anchor = null;
      }
    },
  };
}

/** Preview/persist points for a two-corner shape (or the freehand path). */
function shapePath(mode: DrawMode, a: Point, b: Point, freehand: number[]): { points: number[]; closed: boolean } {
  switch (mode) {
    case "freehand":
      return { points: freehand, closed: false };
    case "line":
      return { points: [a.x, a.y, b.x, b.y], closed: false };
    case "rect":
      return { points: rectPoints(a.x, a.y, b.x, b.y), closed: true };
    case "ellipse":
      return { points: ellipsePoints(a.x, a.y, b.x, b.y), closed: true };
  }
}

/** Drag to draw: freehand collects the path; rect/ellipse/line span two corners. A live
 * preview overlays while dragging; release persists a `drawing` doc (optimistic). No active
 * scene → unhandled (camera pans). */
export function makeDrawTool(ctx: ToolContext, controller: ToolController): SceneTool {
  let anchor: Point | null = null;
  let freehand: number[] = [];

  return {
    onPointerDown(p: Point): boolean {
      if (!activeScene(ctx)) return false;
      anchor = p;
      freehand = [p.x, p.y];
      return true;
    },
    onPointerMove(p: Point): void {
      if (!anchor) return;
      if (controller.drawMode === "freehand") freehand.push(p.x, p.y);
      const { points, closed } = shapePath(controller.drawMode, anchor, p, freehand);
      ctx.scene.previewOverlay([{ points, closed, stroke: { color: parseColor(controller.strokeColor), width: 2 }, fill: null }]);
    },
    onPointerUp(p: Point): void {
      if (!anchor) return;
      const scene = activeScene(ctx);
      const mode = controller.drawMode;
      // A pure click has no extent — skip it so no invisible drawing is persisted.
      if (scene && hasExtent(mode, anchor, p, freehand)) {
        const points = mode === "freehand" ? freehand : [anchor.x, anchor.y, p.x, p.y];
        ctx.dispatchIntent([
          {
            op: "create",
            doc: buildSceneEntityDoc(ctx.world, scene.id, "drawing", {
              shape: { kind: mode, points },
              stroke: { color: controller.strokeColor, width: 2 },
              fill: null,
            }),
          },
        ]);
      }
      ctx.scene.clearOverlay();
      anchor = null;
      freehand = [];
    },
  };
}

/** Template area from an anchor + size + direction (degrees). */
function templatePath(mode: TemplateMode, ax: number, ay: number, size: number, direction: number): { points: number[]; closed: boolean } {
  switch (mode) {
    case "circle":
      return { points: circlePoints(ax, ay, size), closed: true };
    case "cone":
      return { points: conePoints(ax, ay, size, direction), closed: true };
    case "rect":
      return { points: squarePoints(ax, ay, size, direction), closed: true };
    case "line": {
      const a = (direction * Math.PI) / 180;
      return { points: [ax, ay, ax + size * Math.cos(a), ay + size * Math.sin(a)], closed: false };
    }
  }
}

/** Drag from the anchor sets the template's size (distance) + direction (angle). A near-zero
 * drag falls back to one grid cell so a click places a default template. */
function sizeDir(a: Point, b: Point, cell: number): { size: number; direction: number } {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const d = Math.hypot(dx, dy);
  if (d < 1) return { size: cell, direction: 0 };
  return { size: d, direction: (Math.atan2(dy, dx) * 180) / Math.PI };
}

/** Drag to place a template area (circle/cone/rect/line) anchored at the snapped cell; the
 * drag sets size + direction. Live preview; release persists a `template` doc (optimistic). */
export function makeTemplateTool(ctx: ToolContext, controller: ToolController): SceneTool {
  let anchor: Point | null = null;
  let cell = 100;

  return {
    onPointerDown(p: Point): boolean {
      const scene = activeScene(ctx);
      if (!scene) return false;
      anchor = ctx.scene.snap(p);
      cell = scene.size;
      return true;
    },
    onPointerMove(p: Point): void {
      if (!anchor) return;
      const { size, direction } = sizeDir(anchor, p, cell);
      const { points, closed } = templatePath(controller.templateMode, anchor.x, anchor.y, size, direction);
      const color = parseColor(controller.templateColor);
      ctx.scene.previewOverlay([{ points, closed, stroke: { color, width: 2 }, fill: closed ? { color, alpha: 0.25 } : null }]);
    },
    onPointerUp(p: Point): void {
      if (!anchor) return;
      const scene = activeScene(ctx);
      if (scene) {
        const { size, direction } = sizeDir(anchor, p, cell);
        ctx.dispatchIntent([
          {
            op: "create",
            doc: buildSceneEntityDoc(ctx.world, scene.id, "template", {
              shape: { kind: controller.templateMode, x: anchor.x, y: anchor.y, size, direction },
              color: controller.templateColor,
            }),
          },
        ]);
      }
      ctx.scene.clearOverlay();
      anchor = null;
    },
  };
}

/** Leading-edge coalescing window for drag-move intents: the first move sends
 * immediately, then at most one per window, with the final position flushed on release.
 * Caps optimistic-pending churn during a drag without starving the remote view. */
const DRAG_THROTTLE_MS = 50;

/** Pick a token on pointerdown and drag the whole selection. Clicking an unselected token
 * replaces the selection with just it; Shift toggles it in/out. Dragging moves every selected
 * token by the same snapped delta, preserving relative offsets; intents stream coalesced with
 * the final position flushed on release. Empty space clears the selection and yields the gesture
 * to the camera. A ring overlay marks the selection. */
export function makeSelectMoveTool(ctx: ToolContext): SceneTool {
  const now = ctx.now ?? ((): number => Date.now());
  const sel = ctx.tokenSelection;
  let draggingId: string | null = null;
  let grabOrigin: Point = { x: 0, y: 0 };
  let origins = new Map<string, Point>(); // selected id -> original center at grab time
  let moved = false;
  let lastSentAt = -Infinity;

  const centerOf = (id: string): Point => {
    const s = ctx.documents.get(id)?.system as { x?: number; y?: number } | undefined;
    return { x: s?.x ?? 0, y: s?.y ?? 0 };
  };

  /** A closed ring per selected token into the tool overlay (cleared when empty). Circle
   * tokens receive an ellipse ring so the ring, hit-test, and faction border agree on shape. */
  const drawSelection = (): void => {
    if (!sel) return;
    const rings = [...sel.ids].map((id) => {
      const c = centerOf(id);
      const doc = ctx.documents.get(id);
      const box = doc ? resolveTokenBox(doc, ctx.documents) : null;
      const w = (box?.w || 0) || 100;
      const h = (box?.h || 0) || 100;
      const hw = w / 2;
      const hh = h / 2;
      const points = box?.shape === "circle"
        ? ellipsePoints(c.x - hw, c.y - hh, c.x + hw, c.y + hh)
        : [c.x - hw, c.y - hh, c.x + hw, c.y - hh, c.x + hw, c.y + hh, c.x - hw, c.y + hh];
      return { points, closed: true, stroke: { color: 0xffd400, width: 2 }, fill: null };
    });
    if (rings.length === 0) ctx.scene.clearOverlay();
    else ctx.scene.previewOverlay(rings);
  };

  const sendMoves = (delta: Point): void => {
    const ops: WireOperation[] = [];
    for (const [id, o] of origins) {
      const target = ctx.scene.snap({ x: o.x + delta.x, y: o.y + delta.y });
      const sys = ctx.documents.get(id)?.system as { x?: number; y?: number } | undefined;
      ops.push({ op: "update", doc_id: id, changes: [
        { path: "/system/x", old: sys?.x ?? null, new: target.x },
        { path: "/system/y", old: sys?.y ?? null, new: target.y },
      ] });
    }
    if (ops.length > 0) ctx.dispatchIntent(ops);
  };

  return {
    onPointerDown(p: Point, ev: PointerEvent): boolean {
      const id = topTokenAt(ctx.documents.query("token"), p, ctx.documents);
      if (!id) {
        sel?.clear();
        ctx.scene.clearOverlay();
        return false;
      }
      if (sel) {
        if (ev.shiftKey) sel.toggle(id);
        else if (!sel.has(id)) sel.set([id]);
      }
      draggingId = id;
      grabOrigin = { x: p.x, y: p.y };
      origins = new Map([...(sel?.ids ?? [id])].map((sid) => [sid, centerOf(sid)]));
      if (!origins.has(id)) origins.set(id, centerOf(id));
      moved = false;
      lastSentAt = -Infinity;
      ctx.scene.setDraggingToken(id);
      drawSelection();
      return true;
    },
    onPointerMove(p: Point): void {
      if (!draggingId) return;
      moved = true;
      const delta = { x: p.x - grabOrigin.x, y: p.y - grabOrigin.y };
      const t = now();
      if (t - lastSentAt >= DRAG_THROTTLE_MS) {
        sendMoves(delta); // leading-edge coalesced stream
        lastSentAt = t;
      }
      drawSelection();
    },
    onPointerUp(p: Point): void {
      if (!draggingId) return;
      // Flush the authoritative release delta (a pure click that never moved sends nothing).
      if (moved) sendMoves({ x: p.x - grabOrigin.x, y: p.y - grabOrigin.y });
      ctx.scene.setDraggingToken(null);
      draggingId = null;
      moved = false;
      drawSelection();
    },
  };
}
