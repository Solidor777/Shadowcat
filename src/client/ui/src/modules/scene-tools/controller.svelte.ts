// scene-tools active-tool state + the SceneTool implementations. Reaches the engine
// only through the public AppContext seams (the scene bridge for tool activation/snap,
// dispatchIntent for document writes); it never imports core-ui (contract-only
// boundary). The tool factories close over the context.
import { rectPoints, ellipsePoints, circlePoints, conePoints, squarePoints, parseColor, type SceneTool, type Point } from "@shadowcat/render";
import { buildTokenDoc, buildSceneEntityDoc, type ReadableDocuments, type AssetResolver, type WireOperation } from "@shadowcat/core";
import type { SceneInteraction } from "../../lib/sceneInteraction";
import { topTokenAt } from "./hit-test";

export type ToolId = "select" | "place" | "draw" | "template" | "measure";
export type DrawMode = "freehand" | "rect" | "ellipse" | "line";
export type TemplateMode = "circle" | "cone" | "rect" | "line";

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

/** A draw gesture has visible extent: a freehand path of ≥2 points, or a two-corner
 * shape whose corners are ≥1 unit apart. A pure click has none — persisting it would
 * write an invisible junk drawing to the scene + event log. */
function hasExtent(mode: DrawMode, a: Point, b: Point, freehand: number[]): boolean {
  if (mode === "freehand") return freehand.length >= 4;
  return Math.hypot(b.x - a.x, b.y - a.y) >= 1;
}

/** Drag to measure: a client-local segment + whole-cell distance label. Never persists a
 * document or broadcasts (#3) — purely an overlay on the dragging client. */
export function makeMeasureTool(ctx: ToolContext): SceneTool {
  let anchor: Point | null = null;
  return {
    onPointerDown(p: Point): boolean {
      anchor = p;
      return true; // measuring works anywhere; claim the gesture
    },
    onPointerMove(p: Point): void {
      if (!anchor) return;
      ctx.scene.drawMeasure(anchor, p, String(ctx.scene.gridDistance(anchor, p)));
    },
    onPointerUp(): void {
      if (!anchor) return;
      ctx.scene.clearMeasure();
      anchor = null;
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
