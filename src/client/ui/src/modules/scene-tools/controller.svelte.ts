// scene-tools active-tool state + the SceneTool implementations. Reaches the engine
// only through the public AppContext seams (the scene bridge for tool activation/snap,
// dispatchIntent for document writes); it never imports core-ui (contract-only
// boundary). The tool factories close over the context.
import { rectPoints, ellipsePoints, parseColor, type SceneTool, type Point } from "@shadowcat/render";
import { buildTokenDoc, buildSceneEntityDoc, type ReadableDocuments, type AssetResolver, type WireOperation } from "@shadowcat/core";
import type { SceneInteraction } from "../../lib/sceneInteraction";
import { topTokenAt } from "./hit-test";

export type ToolId = "select" | "place" | "draw";
export type DrawMode = "freehand" | "rect" | "ellipse" | "line";

/** The AppContext slice the tools need. `documents` is the optimistic view, so a
 * just-auto-created scene / just-placed token is visible to the tools immediately. */
export interface ToolContext {
  scene: SceneInteraction;
  dispatchIntent: (ops: WireOperation[]) => void;
  documents: ReadableDocuments;
  assets: AssetResolver;
  world: string;
  /** Monotonic clock for drag-intent coalescing; defaults to Date.now (injected in tests). */
  now?: () => number;
}

/** The active scene (single scene in M8d §15) + its grid cell size (default 100). */
function activeScene(ctx: ToolContext): { id: string; size: number } | null {
  const scene = ctx.documents.query("scene")[0];
  if (!scene) return null;
  const size = (scene.system as { grid?: { size?: number } } | undefined)?.grid?.size ?? 100;
  return { id: scene.id, size };
}

/** Owns the active-tool + selected-asset UI state and routes activation to the engine
 * via the scene bridge. */
export class ToolController {
  active = $state<ToolId | null>(null);
  /** The token art the place tool stamps; chosen in the asset picker. */
  selectedAsset = $state<string | null>(null);
  /** Draw-tool shape mode + stroke color. */
  drawMode = $state<DrawMode>("freehand");
  strokeColor = $state("#e0e0e0");
  readonly #tools: Record<ToolId, SceneTool>;

  constructor(private readonly ctx: ToolContext) {
    this.#tools = {
      select: makeSelectMoveTool(ctx),
      place: makePlaceTool(ctx, this),
      draw: makeDrawTool(ctx, this),
    };
  }

  /** Toggle a tool: re-selecting the active one clears it (back to camera). */
  toggle(id: ToolId): void {
    this.active = this.active === id ? null : id;
    this.ctx.scene.setActiveTool(this.active ? this.#tools[this.active] : null);
  }
}

/** Click stamps a token (the selected asset) at the snapped cell of the active scene.
 * No scene or no selected asset → unhandled (the camera pans instead). */
export function makePlaceTool(ctx: ToolContext, controller: ToolController): SceneTool {
  return {
    onPointerDown(p: Point): boolean {
      const scene = activeScene(ctx);
      const asset = controller.selectedAsset;
      if (!scene || !asset) return false;
      const c = ctx.scene.snap(p);
      ctx.dispatchIntent([
        {
          op: "create",
          doc: buildTokenDoc(ctx.world, scene.id, {
            x: c.x,
            y: c.y,
            w: scene.size,
            h: scene.size,
            rotation: 0,
            visual: { kind: "image", asset },
          }),
        },
      ]);
      return true;
    },
    onPointerMove(): void {},
    onPointerUp(): void {},
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
      if (scene) {
        const mode = controller.drawMode;
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

/** Leading-edge coalescing window for drag-move intents: the first move sends
 * immediately, then at most one per window, with the final position flushed on release.
 * Caps optimistic-pending churn during a drag without starving the remote view. */
const DRAG_THROTTLE_MS = 50;

/** Pick a token on pointerdown and drag it: the local sprite snaps to the pointer (via
 * setDraggingToken), and snapped position-update intents stream coalesced to the server
 * (and other clients), with the final position flushed on release. Picks nothing →
 * unhandled, so the camera pans. */
export function makeSelectMoveTool(ctx: ToolContext): SceneTool {
  const now = ctx.now ?? ((): number => Date.now());
  let draggingId: string | null = null;
  let offset: Point = { x: 0, y: 0 };
  let moved = false;
  let lastSentAt = -Infinity;

  /** Snapped token center for a pointer at scene point `p`, holding the grab offset. */
  const targetFor = (p: Point): Point => ctx.scene.snap({ x: p.x - offset.x, y: p.y - offset.y });

  const sendMove = (id: string, target: Point): void => {
    const sys = ctx.documents.get(id)?.system as { x?: number; y?: number } | undefined;
    ctx.dispatchIntent([
      {
        op: "update",
        doc_id: id,
        changes: [
          { path: "/system/x", old: sys?.x ?? null, new: target.x },
          { path: "/system/y", old: sys?.y ?? null, new: target.y },
        ],
      },
    ]);
  };

  return {
    onPointerDown(p: Point): boolean {
      const id = topTokenAt(ctx.documents.query("token"), p);
      if (!id) return false;
      const sys = ctx.documents.get(id)?.system as { x?: number; y?: number } | undefined;
      offset = { x: p.x - (sys?.x ?? p.x), y: p.y - (sys?.y ?? p.y) }; // grab point within the token
      draggingId = id;
      moved = false;
      lastSentAt = -Infinity;
      ctx.scene.setDraggingToken(id);
      return true;
    },
    onPointerMove(p: Point): void {
      if (!draggingId) return;
      moved = true;
      const t = now();
      if (t - lastSentAt >= DRAG_THROTTLE_MS) {
        sendMove(draggingId, targetFor(p)); // leading-edge coalesced stream
        lastSentAt = t;
      }
    },
    onPointerUp(p: Point): void {
      if (!draggingId) return;
      // Send the authoritative release position (a pure click that never moved sends
      // nothing). This captures a touch/pen lift-off delta the throttled stream missed.
      if (moved) sendMove(draggingId, targetFor(p));
      ctx.scene.setDraggingToken(null);
      draggingId = null;
      moved = false;
    },
  };
}
