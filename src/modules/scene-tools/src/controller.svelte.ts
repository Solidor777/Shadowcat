// scene-tools active-tool state + the SceneTool implementations. Reaches the engine
// only through the public AppContext seams (the scene bridge for tool activation/snap,
// dispatchIntent for document writes); it never imports core-ui (contract-only
// boundary). The tool factories close over the context.
import { rectPoints, ellipsePoints, circlePoints, conePoints, squarePoints, parseColor, type SceneTool, type Point } from "@shadowcat/render";
import { buildTokenDoc, buildTokenFromActor, buildSceneEntityDoc, EMPTY_FOOTPRINTS, buildRegionDoc, setRegionVisibility, buildLightDoc, DEFAULT_LIGHT_EMISSION, type ReadableDocuments, type AssetResolver, type WireOperation, type PathResult, type MoveStream, type FootprintLookup, type LightEmission, type LightEngine, type RegionTrigger, type RegionEngine } from "@shadowcat/core";
import type { SceneInteraction, ActorSelection, TokenSelection } from "@shadowcat/ui-kit";
import type { WorldRole } from "@shadowcat/types";
import { topTokenAt, topLightAt, topWallAt } from "./hit-test";

/** A tool id, keying `ToolController.#tools` and gating `ToolRail`'s per-role visibility. */
export type ToolId = "select" | "place" | "draw" | "template" | "measure" | "ping" | "wall" | "region" | "light";
/** The draw tool's shape mode; `"rect"` here is an axis-aligned two-corner bbox (`shapePath`),
 * distinct from the template tool's rotated-square `"rect"` (`templatePath`). */
export type DrawMode = "freehand" | "rect" | "ellipse" | "line";
/** The template tool's shape mode; `"rect"` here is a rotated square centered on the anchor
 * (`templatePath`), distinct from the draw tool's axis-aligned `"rect"` (`shapePath`). */
export type TemplateMode = "circle" | "cone" | "rect" | "line";
/** The region tool's shape mode, matching `parse_region_shape`'s point-count dispatch
 * (`regionShapeGeometry`). */
export type RegionShapeMode = "rect" | "circle" | "polygon";
/** A region's gameplay effect, matching the server `RegionBehavior` variants (precedence
 * `Impassable > Arrest > Terrain` at composition time). */
export type RegionBehaviorMode = "terrain" | "impassable" | "arrest";

/** The AppContext slice the tools need. `documents` is the optimistic view, so a
 * just-auto-created scene / just-placed token is visible to the tools immediately. */
export interface ToolContext {
  /** The render-engine bridge tools draw through: snapping, preview overlays, drag-state
   * signaling and grid distance — never document state. */
  scene: SceneInteraction;
  /** The actor to stamp (the place tool); when set it takes precedence over selectedAsset. */
  actorSelection?: ActorSelection;
  /** Selected token ids (group-select); the select tool reads + moves the whole set. */
  tokenSelection?: TokenSelection;
  /** Sends write ops to the server; fire-and-forget, no per-call reject signal exposed here. */
  dispatchIntent: (ops: WireOperation[]) => void;
  /** The optimistic document view, so a just-created scene/token is visible immediately. */
  documents: ReadableDocuments;
  /** Resolves asset ids to serve URLs (the asset picker's selection). */
  assets: AssetResolver;
  /** The world id every tool-created document is stamped with. */
  world: string;
  /** The caller's per-world role. Movement authority is role-asymmetric: a GM writes a token
   * position directly, a player's move is request-only and server-executed. */
  role: WorldRole;
  /** Broadcast a transient ping at scene coords (the ping tool). */
  sendPing: (x: number, y: number) => void;
  /** Monotonic clock for drag-intent coalescing; defaults to Date.now (injected in tests). */
  now?: () => number;
  /** Timer scheduler backing the measure tool's deferred route-preview fire; defaults to the
   * real `setTimeout`. Tests that inject a logical `now` (advanced without advancing real
   * time) must inject this too, or a scheduled deferred-fire timer is armed against REAL wall
   * time regardless of `now` and can fire during a later, unrelated test. */
  scheduleTimeout?: (fn: () => void, ms: number) => unknown;
  /** Cancels a handle returned by `scheduleTimeout`; defaults to the real `clearTimeout`. */
  clearScheduledTimeout?: (handle: unknown) => void;
  /** Grid A* pathfind seam (from AppContext). When present and a single token is
   * selected, the measure tool routes through it instead of the plain gridDistance
   * mode. When absent (older host or not connected), the tool falls back gracefully.
   * `token`, when given, names the token the route is for: the server derives the
   * footprint from it and ignores `footprintRadius` entirely. */
  pathfind?: (
    scene: string,
    start: [number, number],
    waypoints: [number, number][],
    footprintRadius: number,
    token?: string,
  ) => Promise<PathResult>;
  /** Request server-authoritative move execution (from AppContext). When present,
   * double-click commit sends a MoveRequest; animation is broadcast-driven via MoveStream
   * for all viewers. When absent, double-click is a no-op (graceful degradation). */
  moveRequest?: (
    scene: string,
    tokenId: string,
    path: [number, number][],
  ) => Promise<MoveStream>;
  /** The scene the tools act on. From `ctx.viewedSceneId`; absent ⇒ the first scene. */
  viewedSceneId?: () => string | null;
  /** The server's resolved token footprints (from `ctx.footprints`). The hit-test, the selection
   * ring and the place tool all read their extents from here rather than computing any; absent ⇒
   * an empty lookup, under which a token's own authored `w`/`h` stand. */
  footprints?: () => FootprintLookup;
}

/** The active (viewed) scene + its grid cell size (default 100 when
 * `grid.size` is absent) and distance scale (default `{ perCell: 5, unit: "ft" }` when
 * `grid.distance` is absent, matching `resolveSceneSettings`'s own default). These are
 * tool-local display/pathfind-arg defaults only; no parity claim is made with the server
 * beyond this: `SceneEcs::scene_grid_sizes` is the server's sole intentional cell-size
 * defaulting source and also falls back to 100, but the movement gates themselves REFUSE
 * rather than default when a scene has no live document (`Room::publish`).
 * @param ctx The tool context; reads `viewedSceneId()` (falling back to the first scene) and
 * `documents`.
 * @returns The resolved scene id + grid size + distance scale, or `null` when no scene is
 * viewed and none exists.
 * @example
 * ```
 * declare const ctx: ToolContext;
 * const scene = activeScene(ctx);
 * if (scene) console.log(scene.id, scene.size);
 * ```
 */
function activeScene(ctx: ToolContext): {
  /** The active scene's document id. */
  id: string;
  /** Grid cell size in pixels (defaults to 100 when the scene doc omits `grid.size`; the
   * circumradius on hex). */
  size: number;
  /** Distance-per-cell scale numerator (defaults to 5, matching `resolveSceneSettings`). */
  perCell: number;
  /** Distance unit label (defaults to `"ft"`, matching `resolveSceneSettings`). */
  unit: string;
} | null {
  const vsid = ctx.viewedSceneId?.() ?? ctx.documents.query("scene")[0]?.id ?? null;
  const scene = vsid ? ctx.documents.get(vsid) : undefined;
  if (!scene) return null;
  const grid = (scene.engine as {
    /** Per-scene grid config; absent ⇒ the defaults below apply. */
    grid?: {
      /** Grid cell size in pixels; falls back to 100 when absent. */
      size?: number;
      /** Distance-per-cell scale; falls back to `{ perCell: 5, unit: "ft" }` when absent. */
      distance?: {
        /** Distance-per-cell scale numerator. */
        perCell: number;
        /** Distance unit label (e.g. `"ft"`). */
        unit: string;
      };
    };
  } | undefined)?.grid;
  const size = grid?.size ?? 100;
  const { perCell, unit } = grid?.distance ?? { perCell: 5, unit: "ft" };
  return { id: scene.id, size, perCell, unit };
}

/** Format a whole-cell distance as the measure tool's shared distance label:
 * `${round(cells * perCell)} ${unit}`. Both the routed budget label (`requestRoute`) and the
 * plain-measure fallback (`onPointerMove`'s non-route branch) call this, so a distance is
 * expressed identically regardless of which branch measured it — the divergence this replaces
 * had the fallback printing a bare cell count (`"5"`) while the route branch printed the scaled,
 * unit-suffixed form (`"25 ft"`) for the same underlying distance. The function knows nothing
 * about arrest: only `requestRoute` has that information, so it appends the `⚠` marker itself to
 * this function's return value rather than this function taking a caller-specific flag.
 * @param cells The whole or fractional cell count to label.
 * @param scene The perCell/unit scale to label with.
 * @param scene.perCell Distance-per-cell scale used for the label.
 * @param scene.unit Distance unit label (e.g. `"ft"`) used for the label.
 * @returns The formatted label string, e.g. `"25 ft"`.
 * @example
 * ```
 * formatCellDistance(5, { perCell: 5, unit: "ft" }); // "25 ft"
 * ```
 */
function formatCellDistance(cells: number, scene: {
  /** Distance-per-cell scale used for the label. */
  perCell: number;
  /** Distance unit label (e.g. `"ft"`) used for the label. */
  unit: string;
}): string {
  return `${Math.round(cells * scene.perCell)} ${scene.unit}`;
}

/** The footprint radius a tokenless route preview is measured with. The server honors the wire
 * `footprint_radius` only when the request names no token, and a request that names no token also
 * names no actor to size a mover from — so this sub-cell value is all a hypothetical preview has,
 * and it matches the radius the server itself falls back to for an unsized token. Every route the
 * user actually walks names its token and is measured with the server's own resolved footprint
 * instead. */
const HYPOTHETICAL_PREVIEW_FOOTPRINT_CELLS = 0.4;

/** The server's resolved footprints, or an empty lookup on a host that supplies none.
 * @param ctx The tool context; reads `ctx.footprints`.
 * @returns The current lookup.
 * @example
 * ```
 * declare const ctx: ToolContext;
 * const unit = footprintsOf(ctx).unit("scene-1");
 * ```
 */
function footprintsOf(ctx: ToolContext): FootprintLookup {
  return ctx.footprints?.() ?? EMPTY_FOOTPRINTS;
}

/** Route color for the A* preview polyline (blue-teal, distinct from walls and selection). */
const ROUTE_COLOR = 0x3399ff;
/** Maximum milliseconds between two pointer-downs to count as a double-click. */
const DOUBLE_CLICK_MS = 350;
/** Maximum scene-coord distance between two pointer-downs to count as a double-click
 * (generous: post-snap, a double-click lands on the same cell center). */
const COMMIT_RADIUS = 12;
/** Leading-edge debounce window for route-preview pathfind requests: the first move in a
 * burst fires immediately, then a move arriving within the window is suppressed — but NOT
 * "at most one request per window": `schedulePendingRouteFire`/`firePendingRoute` add one
 * deferred TRAILING fire once the window closes, whenever a move was suppressed near its end
 * (so a hover-only stop, with no further `onPointerMove`, doesn't freeze the preview on a stale
 * goal). This does NOT mirror `DRAG_THROTTLE_MS`'s shape — that constant is pure leading-edge
 * with no trailing fire at all (the drag's final position is flushed on release instead).
 * Arming the leading edge only on an actual fire — never re-armed per event — is what keeps
 * this from starving under continuous pointer movement. */
const ROUTE_PREVIEW_DEBOUNCE_MS = 100;

/** Owns the active-tool + selected-asset UI state and routes activation to the engine
 * via the scene bridge. */
export class ToolController {
  /** The currently activated tool, or `null` when back to the plain camera (`toggle`'s
   * re-select-clears rule). */
  active = $state<ToolId | null>(null);
  /** The token art the place tool stamps; chosen in the asset picker. */
  selectedAsset = $state<string | null>(null);
  /** Draw-tool shape mode + stroke color. */
  drawMode = $state<DrawMode>("freehand");
  /** Draw-tool stroke color (hex string, parsed via `parseColor`). */
  strokeColor = $state("#e0e0e0");
  /** Template-tool shape mode + color. */
  templateMode = $state<TemplateMode>("circle");
  /** Template-tool fill/stroke color (hex string, parsed via `parseColor`). */
  templateColor = $state("#3388ff");
  /** Region-tool shape mode + behavior/cost/secrecy config. */
  regionShapeMode = $state<RegionShapeMode>("rect");
  /** Region-tool authored behavior, applied to the next region the tool persists. */
  regionBehavior = $state<RegionBehaviorMode>("terrain");
  /** Region-tool authored terrain cost multiplier; clamped to `>= 1` at persist time. */
  regionCost = $state<number>(2);
  /** Region-tool authored secrecy flag; `true` sets `gm_only` visibility on the persisted doc
   * via `setRegionVisibility`. */
  regionSecret = $state<boolean>(false);
  /** The non-token scene entity currently open for editing (a light or a wall picked with the
   * select or light tool), or `null`. Cleared on every tool switch (`toggle`) and on a GM's
   * empty-canvas click with the select tool; `ToolRail` renders the matching editor while it is
   * set. One shared selection source for both tools, so the editor can never disagree with the
   * canvas about which entity is being edited. */
  editingEntity = $state<{
    /** Which kind of scene entity is being edited. */
    kind: "light" | "wall";
    /** The edited document's id. */
    id: string;
  } | null>(null);

  /** Region-tool authored triggers, deep-copied onto the next region the tool persists (an
   * empty list persists a plain movement-only region). Editing a row in the rail must never
   * mutate an already-persisted doc, which is why `makeRegionTool` clones at persist time. */
  regionTriggers = $state<RegionTrigger[]>([]);
  /** One `SceneTool` instance per `ToolId`, built once in the constructor. */
  readonly #tools: Record<ToolId, SceneTool>;

  /** Builds each tool factory once, closing over `ctx` (and `this` for the factories that read
   * shared controller state: `selectedAsset`, draw/template mode, region shape/behavior/cost).
   * @param ctx The tool context every factory closes over.
   * @example
   * ```
   * declare const ctx: ToolContext;
   * const controller = new ToolController(ctx);
   * ```
   */
  constructor(private readonly ctx: ToolContext) {
    this.#tools = {
      select: makeSelectMoveTool(ctx, this),
      place: makePlaceTool(ctx, this),
      draw: makeDrawTool(ctx, this),
      template: makeTemplateTool(ctx, this),
      measure: makeMeasureTool(ctx),
      ping: makePingTool(ctx),
      wall: makeWallTool(ctx),
      region: makeRegionTool(ctx, this),
      light: makeLightTool(ctx, this),
    };
  }

  /** Toggle a tool: re-selecting the active one clears it (back to camera).
   * Fires `onDeactivate` on the outgoing tool (if any) so tools with live
   * overlays can tear down before the new tool activates (mid-gesture-clear invariant).
   * @param id The tool to activate (or clear, when it is already the active one).
   * @example
   * ```
   * declare const controller: ToolController;
   * controller.toggle("wall");
   * ```
   */
  toggle(id: ToolId): void {
    // Deactivate the outgoing tool before updating `active` so it can still read state.
    if (this.active) this.#tools[this.active].onDeactivate?.();
    this.editingEntity = null; // an edit selection never survives a tool switch
    this.active = this.active === id ? null : id;
    this.ctx.scene.setActiveTool(this.active ? this.#tools[this.active] : null);
  }
}

/** Click stamps a token at the snapped cell of the active scene. A selected actor takes
 * precedence (instanced if its `prototype` is set, else linked); otherwise the selected raw
 * asset is stamped. No scene, or neither an actor nor an asset selected → unhandled (camera
 * pans). A placed linked actor deselects itself by default (see the inline comment below) so
 * repeated clicks don't stamp duplicate live-views of the same actor; an instanced actor always
 * stays selected, since placing several is the expected use.
 * @param ctx The tool context; reads actor/asset selection, dispatches the create intent.
 * @param controller Supplies `selectedAsset`, the raw-asset stamp fallback.
 * @returns A `SceneTool` implementing only `onPointerDown` (placement is a single click).
 * @example
 * ```
 * declare const ctx: ToolContext;
 * declare const controller: ToolController;
 * const tool = makePlaceTool(ctx, controller);
 * ```
 */
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
        const mode = (actor.engine as {
          /** Whether this actor is a reusable prototype; when true, a placed token instances
           * (embeds a frozen copy) rather than links (shares the live document). */
          prototype?: boolean;
        } | undefined)?.prototype ? "instance" : "link";
        ctx.dispatchIntent([{ op: "create", doc: buildTokenFromActor(ctx.world, scene.id, actor, mode, c, footprintsOf(ctx).unit(scene.id)) }]);
        // A unique (linked) actor places once by default: clear the selection so repeated
        // clicks don't stamp duplicate live-views. The user can opt to keep it selected
        // (keepAfterPlace). Instanced actors always stay selected for placing many.
        if (mode === "link" && !ctx.actorSelection?.keepAfterPlace) ctx.actorSelection?.select(null);
        return true;
      }
      const asset = controller.selectedAsset;
      if (!asset) return false;
      // The scene's server-resolved unit footprint — a single hex's own bounding box on a hex
      // scene. A raw (actorless) token has no actor for the server to size it from, so this
      // authored extent is the one it keeps.
      const unit = footprintsOf(ctx).unit(scene.id);
      ctx.dispatchIntent([
        {
          op: "create",
          doc: buildTokenDoc(ctx.world, scene.id, { x: c.x, y: c.y, w: unit?.w ?? 0, h: unit?.h ?? 0, rotation: 0, visual: { kind: "image", asset }, actor_id: null, overrides: null, face: null, elevation: null }),
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
 * write an invisible junk drawing to the scene + event log.
 * @param mode The active draw mode; only `"freehand"` consults `freehand`.
 * @param a The drag start point (raw pointer scene coords; the sole caller, `makeDrawTool`,
 * never snaps).
 * @param b The current/drag-end point (raw pointer scene coords; the sole caller,
 * `makeDrawTool`, never snaps).
 * @param freehand The accumulated freehand path, as a flat `[x0,y0,x1,y1,...]` array.
 * @returns `true` when the gesture has visible extent and should persist.
 * @example
 * ```
 * declare const anchor: Point;
 * declare const b: Point;
 * const shouldPersist = hasExtent("rect", anchor, b, []);
 * ```
 */
function hasExtent(mode: DrawMode, a: Point, b: Point, freehand: number[]): boolean {
  if (mode === "freehand") return freehand.length >= 4;
  return Math.hypot(b.x - a.x, b.y - a.y) >= 1;
}

/** Wall preview/segment color (matches the WallView render color). */
const WALL_COLOR = 0xd06060;

/** Drag to draw a wall segment (snapped endpoints); release persists a `wall` doc
 * (`blocksSight`+`blocksMove`+`blocksLight`, all three). The server's collision check reads the
 * same `seg`. The tool rail hides this tool from non-GMs (`ToolRail`'s `visibleTools`
 * filter) — that is a UI-only visibility gate, not a permission this factory itself checks or
 * enforces. No active scene → unhandled.
 * @param ctx The tool context; reads the active scene, snaps points, dispatches the create.
 * @returns A `SceneTool` implementing the drag-to-draw gesture.
 * @example
 * ```
 * declare const ctx: ToolContext;
 * const tool = makeWallTool(ctx);
 * ```
 */
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
              elevation: null,
            }),
          },
        ]);
      }
      ctx.scene.clearOverlay();
      anchor = null;
    },
  };
}

/** The emission payload a freshly placed light is stamped with — the shared authoring default
 * (`DEFAULT_LIGHT_EMISSION`), spread-copied per placement (the constant is frozen). */
const NEW_LIGHT_EMISSION: LightEmission = { ...DEFAULT_LIGHT_EMISSION };

/** Overlay ring stroke for a light's reach preview (amber, distinct from walls/selection). */
const LIGHT_RING_COLOR = 0xe0b040;

/** Draw the selected/dragged light's reach rings into the tool overlay: one ring at the bright
 * radius, one at the dim radius, both in the emission's own geometry. Radii are authored in
 * CELLS; the ring is drawn at the scene's indexing scale (`activeScene`'s `size`) — a cosmetic
 * editing aid only (on hex the authoritative reach is the server's world-units conversion). A
 * zero/non-finite radius ring is skipped.
 * @param ctx The tool context (overlay + active scene).
 * @param pos The light's position in scene coords.
 * @param em The emission whose radii to ring.
 * @example
 * ```
 * declare const ctx: ToolContext;
 * declare const em: LightEmission;
 * drawLightRings(ctx, { x: 50, y: 50 }, em);
 * ```
 */
function drawLightRings(ctx: ToolContext, pos: Point, em: LightEmission): void {
  const scene = activeScene(ctx);
  const cell = scene?.size ?? 100;
  const rings: number[][] = [];
  for (const cells of [em.brightRadius, em.dimRadius]) {
    if (Number.isFinite(cells) && cells > 0) rings.push(circlePoints(pos.x, pos.y, cells * cell));
  }
  ctx.scene.previewOverlay(
    rings.map((points) => ({ points, closed: true, stroke: { color: LIGHT_RING_COLOR, width: 2 }, fill: null })),
  );
}

/** Click to place a light at the snapped point (stamped with `NEW_LIGHT_EMISSION`); click an
 * existing light's marker to select it for editing (`ToolController.editingEntity`, which the
 * rail editor reads); drag a selected light to reposition it (an `/engine/x,y` update on
 * release, with the RAW stored position as the OCC pre-image). The reach rings preview via the
 * tool overlay while a light is selected or dragged. The tool rail hides this tool from non-GMs
 * (`ToolRail`'s `visibleTools` filter) — a UI-only visibility gate, not a permission this
 * factory itself checks or enforces. No active scene → unhandled.
 * @param ctx The tool context; reads the active scene, snaps points, dispatches the intents.
 * @param controller Receives the editing selection (`editingEntity`).
 * @returns A `SceneTool` implementing the place/select/drag gesture.
 * @example
 * ```
 * declare const ctx: ToolContext;
 * declare const controller: ToolController;
 * const tool = makeLightTool(ctx, controller);
 * ```
 */
export function makeLightTool(ctx: ToolContext, controller: ToolController): SceneTool {
  /** The light being repositioned (its marker was the pointer-down target), or `null`. */
  let dragId: string | null = null;
  /** The dragged light's RAW stored position at grab time — the OCC pre-image for the update. */
  let dragOrigin: Point = { x: 0, y: 0 };
  /** Whether the pointer moved since the down (a pure click selects without repositioning). */
  let moved = false;

  /** The light document's position and emission, read RAW from the store, or `null` when the
   * document is gone or malformed.
   * @param id The light document id.
   * @returns The raw stored position + emission, or `null`.
   * @example
   * ```
   * // private helper; read by the drag gesture + ring overlay
   * declare function readLight(id: string): { pos: Point; em: LightEmission } | null;
   * declare const id: string;
   * readLight(id); // { pos, em } | null
   * ```
   */
  const readLight = (id: string): {
    /** The raw stored position, scene coords. */
    pos: Point;
    /** The raw stored emission payload. */
    em: LightEmission;
  } | null => {
    const eng = ctx.documents.get(id)?.engine as LightEngine | undefined;
    if (!eng || !Number.isFinite(eng.x) || !Number.isFinite(eng.y)) return null;
    return { pos: { x: eng.x, y: eng.y }, em: eng.emission };
  };

  return {
    onPointerDown(p: Point): boolean {
      const scene = activeScene(ctx);
      if (!scene) return false;
      const lights = ctx.documents.query("light").filter((d) => d.parent_id === scene.id);
      const hit = topLightAt(lights, p);
      if (hit) {
        const cur = readLight(hit);
        if (!cur) return false;
        controller.editingEntity = { kind: "light", id: hit };
        dragId = hit;
        dragOrigin = cur.pos;
        moved = false;
        drawLightRings(ctx, cur.pos, cur.em);
        return true;
      }
      const at = ctx.scene.snap(p);
      const doc = buildLightDoc(ctx.world, scene.id, { x: at.x, y: at.y, elevation: null, emission: { ...NEW_LIGHT_EMISSION } });
      ctx.dispatchIntent([{ op: "create", doc }]);
      // Placing selects the new light so the rail editor targets it immediately (a second
      // click would place ANOTHER light, not select this one).
      controller.editingEntity = { kind: "light", id: doc.id };
      drawLightRings(ctx, at, NEW_LIGHT_EMISSION);
      return true;
    },
    onPointerMove(p: Point): void {
      if (!dragId) return;
      moved = true;
      const cur = readLight(dragId);
      if (cur) drawLightRings(ctx, ctx.scene.snap(p), cur.em);
    },
    onPointerUp(p: Point): void {
      if (!dragId) return;
      if (moved) {
        const target = ctx.scene.snap(p);
        // A sub-pixel-jitter "drag" that snaps back to the origin writes nothing.
        if (target.x !== dragOrigin.x || target.y !== dragOrigin.y) {
          ctx.dispatchIntent([
            {
              op: "update",
              doc_id: dragId,
              changes: [
                { path: "/engine/x", old: dragOrigin.x, new: target.x },
                { path: "/engine/y", old: dragOrigin.y, new: target.y },
              ],
            },
          ]);
        }
        const cur = readLight(dragId);
        if (cur) drawLightRings(ctx, target, cur.em);
      }
      dragId = null;
      moved = false;
    },
    onDeactivate(): void {
      dragId = null;
      moved = false;
      ctx.scene.clearOverlay();
    },
  };
}

/** Region preview stroke color (distinct from walls/measure route). Actual persisted fill/stroke
 * is behavior-tinted by the render layer (`RegionView.toSpec`); this is just the drag preview. */
const REGION_PREVIEW_COLOR = 0xd0a030;

/** Author a vector-shaped region: rect/circle drag two opposite corners; polygon is a freehand
 * drag whose traced path becomes the closed boundary (mirrors `makeDrawTool`'s freehand capture).
 * Release persists a `region` doc with the controller's configured behavior/cost/secrecy.
 * Create-only (no edit UI) — a GM re-authors an existing region by delete+recreate, or toggles
 * `enabled` server-side. (Walls and lights, by contrast, are editable after placement: the
 * select tool picks one into `ToolController.editingEntity` and the rail editor writes it.)
 * The tool rail hides this tool from non-GMs (`ToolRail`'s
 * `visibleTools` filter) — a UI-only visibility gate, not a permission this factory itself
 * checks or enforces.
 * @param ctx The tool context; reads the active scene, snaps points, dispatches the create.
 * @param controller Supplies the configured `regionShapeMode`/`regionBehavior`/`regionCost`/
 * `regionSecret`/`regionTriggers`.
 * @returns A `SceneTool` implementing the drag-to-author gesture.
 * @example
 * ```
 * declare const ctx: ToolContext;
 * declare const controller: ToolController;
 * const tool = makeRegionTool(ctx, controller);
 * ```
 */
export function makeRegionTool(ctx: ToolContext, controller: ToolController): SceneTool {
  let anchor: Point | null = null;
  let freehand: number[] = [];

  return {
    onPointerDown(p: Point): boolean {
      if (!activeScene(ctx)) return false;
      anchor = ctx.scene.snap(p);
      freehand = [anchor.x, anchor.y];
      return true;
    },
    onPointerMove(p: Point): void {
      if (!anchor) return;
      const b = ctx.scene.snap(p);
      if (controller.regionShapeMode === "polygon") freehand.push(b.x, b.y);
      const { points, closed } = regionShapePath(controller.regionShapeMode, anchor, b, freehand);
      ctx.scene.previewOverlay([{ points, closed, stroke: { color: REGION_PREVIEW_COLOR, width: 2 }, fill: null }]);
    },
    onPointerUp(p: Point): void {
      if (!anchor) return;
      const scene = activeScene(ctx);
      const b = ctx.scene.snap(p);
      const mode = controller.regionShapeMode;
      const hasExtentForRegion =
        mode === "polygon" ? freehand.length >= 6 : Math.hypot(b.x - anchor.x, b.y - anchor.y) >= 1;
      if (scene && hasExtentForRegion) {
        const shapePoints = regionShapeGeometry(mode, anchor, b, freehand);
        const engine: RegionEngine = {
          shape: { kind: mode, points: shapePoints },
          behavior: controller.regionBehavior,
          cost: Math.max(1, controller.regionCost),
          enabled: true,
          // Deep copy: the rail's list stays editable for the NEXT region, and a later
          // row edit must never mutate the doc this persist already handed out.
          // `$state.snapshot`, not `structuredClone`: the reactive proxy a `$state`
          // array wraps its contents in is not cloneable.
          triggers: $state.snapshot(controller.regionTriggers),
        };
        const doc = buildRegionDoc(ctx.world, scene.id, engine);
        if (controller.regionSecret) setRegionVisibility(doc, true);
        ctx.dispatchIntent([{ op: "create", doc }]);
      }
      ctx.scene.clearOverlay();
      anchor = null;
      freehand = [];
    },
  };
}

/** Preview points for the in-progress region drag (closed for all three shape kinds — a region
 * is always an area, unlike walls/lines). For `"circle"` this is a tessellated polygon ring
 * (`circlePoints`), suitable for an overlay stroke — NOT the same shape as
 * `regionShapeGeometry`'s persisted `[cx, cy, r]` triple for the same mode; the two functions
 * are not interchangeable.
 * @param mode The region shape kind being authored.
 * @param a The drag anchor (scene coords, post-snap).
 * @param b The current drag point (scene coords, post-snap).
 * @param freehand The accumulated polygon path (consulted only for `"polygon"`).
 * @returns The preview polyline plus whether it should render closed.
 * @example
 * ```
 * declare const anchor: Point;
 * declare const b: Point;
 * const { points, closed } = regionShapePath("circle", anchor, b, []);
 * ```
 */
function regionShapePath(mode: RegionShapeMode, a: Point, b: Point, freehand: number[]): {
  /** The flat overlay polyline for the in-progress drag preview. */
  points: number[];
  /** Whether the overlay should render as a closed ring (always `true` here — a region is
   * always an area). */
  closed: boolean;
} {
  switch (mode) {
    case "rect":
      return { points: rectPoints(a.x, a.y, b.x, b.y), closed: true };
    case "circle":
      return { points: circlePoints(a.x, a.y, Math.hypot(b.x - a.x, b.y - a.y)), closed: true };
    case "polygon":
      return { points: freehand, closed: true };
  }
}

/** The persisted `engine.shape.points` layout for `mode`: rect=[x0,y0,x1,y1], circle=[cx,cy,r],
 * polygon=[x0,y0,x1,y1,...] — matches `parse_region_shape`'s own dispatch on point count
 * (`"rect"` requires exactly 4 points, `"circle"`
 * exactly 3, `"polygon"` an even count ≥6). For `"circle"` this is the raw `[cx, cy, r]` triple,
 * NOT `regionShapePath`'s tessellated preview ring — the two are deliberately different shapes
 * for the same mode and are not interchangeable.
 * @param mode The region shape kind being authored.
 * @param a The drag anchor (scene coords, post-snap).
 * @param b The current drag point (scene coords, post-snap).
 * @param freehand The accumulated polygon path (consulted only for `"polygon"`).
 * @returns The flat points array to persist as `engine.shape.points`.
 * @example
 * ```
 * declare const anchor: Point;
 * declare const b: Point;
 * const points = regionShapeGeometry("rect", anchor, b, []);
 * ```
 */
function regionShapeGeometry(mode: RegionShapeMode, a: Point, b: Point, freehand: number[]): number[] {
  switch (mode) {
    case "rect":
      return [a.x, a.y, b.x, b.y];
    case "circle":
      return [a.x, a.y, Math.hypot(b.x - a.x, b.y - a.y)];
    case "polygon":
      return freehand;
  }
}

/** Click to ping a location: broadcasts a transient marker. The server relays it back to
 * all members (incl. us), so the local ring arrives via the ping listener like any other —
 * this is why there is no separate local-render path here.
 * @param ctx The tool context; only `sendPing` is used.
 * @returns A `SceneTool` implementing the click-to-ping gesture.
 * @example
 * ```
 * declare const ctx: ToolContext;
 * const tool = makePingTool(ctx);
 * ```
 */
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
 * Route mode (`inRouteMode`) requires:
 *   1. `ctx.tokenSelection` has exactly ONE token id, AND
 *   2. `ctx.pathfind` is defined (i.e. the host provides the seam).
 * An active scene is NOT part of `inRouteMode`'s own predicate — it is a third condition
 * checked separately by each pointer handler, and the three handlers do not treat its
 * absence uniformly: `onPointerDown` falls back to the plain anchor-point measure when no
 * scene is active; `onPointerMove` silently no-ops (no route request, no fallback measure);
 * `onPointerUp` doesn't check for a scene at all, and calls `clearRoute()` while in route mode
 * UNLESS a commit is in flight (`committing`), in which case it returns without touching
 * `clearRoute`/`pendingSeq` — the in-flight commit owns its own teardown via `finish()`, and
 * calling `clearRoute()` here would bump `pendingSeq` and abort it. When route mode and an
 * active scene both hold, `onPointerDown` snaps a
 * waypoint onto the list, each `onPointerMove` issues an A* pathfind request
 * (`requestRoute`) from the selected token's center through any accumulated waypoints to
 * the provisional goal, and on resolve `previewOverlay` renders the routed polyline while
 * `drawMeasure` shows the movement budget (cost × perCell + unit). `onPointerUp` / tool
 * deactivation clears all overlays (mid-gesture-clear invariant).
 *
 * With 0 or >1 tokens selected, or no `ctx.pathfind`: falls back to the original
 * anchor→point gridDistance measure so plain measurement is always available.
 * @param ctx The tool context; reads token selection/pathfind/moveRequest, snaps points,
 * dispatches pathfind/moveRequest calls.
 * @returns A `SceneTool` implementing the drag-to-measure gesture (plain or routed).
 * @example
 * ```
 * declare const ctx: ToolContext;
 * const tool = makeMeasureTool(ctx);
 * ```
 */
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
  // Timer scheduler backing the deferred route-preview fire (injected in tests alongside
  // `now`; defaults to the real setTimeout/clearTimeout). Keeping this behind the same
  // injection seam as `now` means a test that fakes the clock also controls the timer,
  // instead of arming a real background timer regardless of the faked `now`.
  const scheduleTimeout = ctx.scheduleTimeout ?? ((fn: () => void, ms: number): unknown => setTimeout(fn, ms));
  const clearScheduledTimeout = ctx.clearScheduledTimeout ?? ((handle: unknown): void => clearTimeout(handle as ReturnType<typeof setTimeout>));
  // Double-click detection state: timestamp and snapped position of the last pointer-down.
  let lastDownAt = -Infinity;
  let lastDownPt: Point = { x: 0, y: 0 };
  // Leading-edge debounce state for route-preview requests (reduces REQUEST volume only;
  // the pendingSeq staleness guard above is untouched and still governs stale RESPONSES).
  let lastRouteRequestAt = -Infinity;
  // Deferred-fire state: a move suppressed by the debounce window still needs to reach the
  // server once the cursor settles (no further onPointerMove ever arrives after a hover-only
  // stop). `pendingRouteGoal` holds the latest suppressed goal; `pendingRouteTimer` fires it
  // once the remaining cooldown elapses, unless a newer request already fired in the meantime.
  let pendingRouteGoal: Point | null = null;
  let pendingRouteTimer: unknown | null = null;

  /** True when the measure tool's pointer handlers should attempt route mode: exactly one
   * token selected (`ctx.tokenSelection`) AND `ctx.pathfind` provided. Deliberately does NOT
   * check for an active scene — that is clause 3 of the factory doc's list above, enforced
   * per-handler instead (each handler resolves `activeScene(ctx)` itself and reacts
   * differently to its absence; see the factory doc).
   * @returns `true` when both conditions hold.
   * @example
   * ```
   * declare function inRouteMode(): boolean;
   * declare const waypoints: [number, number][];
   * declare const snapped: Point;
   * if (inRouteMode()) waypoints.push([snapped.x, snapped.y]);
   * ```
   */
  function inRouteMode(): boolean {
    return (
      ctx.pathfind !== undefined &&
      ctx.tokenSelection !== undefined &&
      ctx.tokenSelection.ids.size === 1
    );
  }

  /** Center of the single selected token, or `null` when zero or multiple tokens are
   * selected. When exactly one is selected but its document (or `engine.x`/`engine.y`) is
   * missing, this does NOT return `null` — it falls back to `[0, 0]` (scene origin), since
   * this function only gates on SELECTION COUNT, not document/field presence.
   * @returns The token center `[x, y]` in scene coords, or `null` when the selection isn't
   * exactly one token.
   * @example
   * ```
   * declare function tokenCenter(): [number, number] | null;
   * const start = tokenCenter();
   * ```
   */
  function tokenCenter(): [number, number] | null {
    const sel = ctx.tokenSelection;
    if (!sel || sel.ids.size !== 1) return null;
    const [id] = [...sel.ids];
    const eng = ctx.documents.get(id)?.engine as {
      /** Token center x in scene coords; absent ⇒ falls back to 0. */
      x?: number;
      /** Token center y in scene coords; absent ⇒ falls back to 0. */
      y?: number;
    } | undefined;
    return [eng?.x ?? 0, eng?.y ?? 0];
  }

  /** The single selected token's id, or `undefined` when zero or multiple are selected. Passed
   * as `pathfind`'s `token` so the server derives the AUTHORITATIVE footprint from that token
   * rather than honoring the `footprint_radius` argument passed to `ctx.pathfind` on the wire.
   * @returns The selected token id, or `undefined`.
   * @example
   * ```
   * declare function selectedTokenId(): string | undefined;
   * const token = selectedTokenId();
   * ```
   */
  function selectedTokenId(): string | undefined {
    const sel = ctx.tokenSelection;
    if (!sel || sel.ids.size !== 1) return undefined;
    const [id] = [...sel.ids];
    return id;
  }

  /** Issue a pathfind request for the current waypoints + provisional goal `goal`, from
   * `start`. Bumps `pendingSeq` and ignores the response once a newer request has since been
   * issued — last-write-wins coalescing of stale RESPONSES, a separate mechanism from the
   * leading-edge debounce above it (which only throttles REQUEST volume; see the
   * `pendingSeq`/`lastRouteRequestAt` field comments). The final element of `allWaypoints` IS
   * the goal, matching `SceneEcs::pathfind`'s `waypoints[last]`-is-destination contract.
   * @param scene The active scene.
   * @param scene.id The scene document id.
   * @param scene.perCell The distance-per-cell scale used for the budget label.
   * @param scene.unit The distance unit label (e.g. `"ft"`) used for the budget label.
   * @param start The route start, in scene coords (the selected token's center).
   * @param goal The provisional goal, in scene coords (post-snap).
   * @example
   * ```
   * declare function requestRoute(
   *   scene: { id: string; perCell: number; unit: string },
   *   start: [number, number],
   *   goal: Point,
   * ): void;
   * declare const scene: { id: string; perCell: number; unit: string };
   * declare const start: [number, number];
   * declare const goal: Point;
   * requestRoute(scene, start, goal);
   * ```
   */
  function requestRoute(scene: {
    /** The scene document id, passed as `Pathfind`'s scene. */
    id: string;
    /** Distance-per-cell scale used for the budget label. */
    perCell: number;
    /** Distance unit label (e.g. `"ft"`) used for the budget label. */
    unit: string;
  }, start: [number, number], goal: Point): void {
    if (!ctx.pathfind) return;
    const seq = ++pendingSeq;
    const fp = HYPOTHETICAL_PREVIEW_FOOTPRINT_CELLS;
    const allWaypoints: [number, number][] = [...waypoints, [goal.x, goal.y]];
    ctx.pathfind(scene.id, start, allWaypoints, fp, selectedTokenId()).then(
      (result) => {
        if (seq !== pendingSeq) return; // superseded by a newer move
        // Cache the resolved path for reuse by commitRoute (avoids a second pathfind).
        lastPreviewedPath = result.path;
        // Render the routed polyline via previewOverlay.
        const pts = result.path.flat();
        ctx.scene.previewOverlay([{ points: pts, closed: false, stroke: { color: ROUTE_COLOR, width: 3 }, fill: null }]);
        // Budget label: `formatCellDistance` rounds to whole distance units for display; the
        // server-side cost stays exact (diagonal rules like alternating/euclidean yield
        // fractional cells).
        const budgetLabel = formatCellDistance(result.cost, scene);
        const startPt: Point = { x: start[0], y: start[1] };
        // An arrested result means the server truncated the route at an arrest region; only this
        // branch knows that, so it appends the ⚠ marker itself — the shared formatter has no
        // arrest concept — signalling the previewed budget covers a shorter, non-final stop.
        const label = result.arrested ? `${budgetLabel} ⚠` : budgetLabel;
        ctx.scene.drawMeasure(startPt, goal, label);
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

  /** Cancel any pending deferred-fire timer and discard its goal. Must run whenever route
   * mode ends or a fresh request already supersedes it — a leaked timer firing after the
   * tool has moved on (teardown, tool swap, route clear) would be its own bug.
   * @example
   * ```
   * declare function clearPendingRouteTimer(): void;
   * clearPendingRouteTimer();
   * ```
   */
  function clearPendingRouteTimer(): void {
    if (pendingRouteTimer !== null) {
      clearScheduledTimeout(pendingRouteTimer);
      pendingRouteTimer = null;
    }
    pendingRouteGoal = null;
  }

  /** Fires once the debounce window elapses for a move that was suppressed. Re-checks
   * `committing` (mirrors onPointerMove's guard: firing here would otherwise bump
   * `pendingSeq` mid-commit via `requestRoute` and invalidate the in-flight commit's seq
   * guard) and re-resolves scene/start fresh rather than trusting values captured at
   * suppression time. This is the deferred TRAILING fire that follows the leading-edge
   * request `requestRoute` itself produces — see the field comments on
   * `lastRouteRequestAt`/`pendingRouteGoal`/`pendingRouteTimer` above for why a
   * hover-only stop (no further `onPointerMove`) needs one.
   * @example
   * ```
   * declare function firePendingRoute(): void;
   * firePendingRoute(); // fires the deferred route-preview request, if one is pending
   * ```
   */
  function firePendingRoute(): void {
    pendingRouteTimer = null;
    const goal = pendingRouteGoal;
    pendingRouteGoal = null;
    if (!goal || committing) return;
    const scene = activeScene(ctx);
    const start = tokenCenter();
    if (!scene || !start) return;
    lastRouteRequestAt = now();
    requestRoute(scene, start, goal);
  }

  /** (Re)schedule the deferred fire for the remaining cooldown time, cancelling any prior
   * pending timer first — only the LATEST suppressed goal must ever fire. Together with
   * `firePendingRoute`, this is what makes `ROUTE_PREVIEW_DEBOUNCE_MS` a leading-edge debounce
   * PLUS one deferred trailing fire per suppressed hover-stop, not a strict "at most one
   * request per window": a move suppressed near the end of a window still reaches the server
   * once the cooldown elapses, so a hover that stops moving doesn't leave the preview frozen
   * on a stale goal.
   * @param goal The suppressed goal to fire once the cooldown elapses.
   * @example
   * ```
   * declare function schedulePendingRouteFire(goal: Point): void;
   * declare const goal: Point;
   * schedulePendingRouteFire(goal);
   * ```
   */
  function schedulePendingRouteFire(goal: Point): void {
    if (pendingRouteTimer !== null) clearScheduledTimeout(pendingRouteTimer);
    pendingRouteGoal = goal;
    const remaining = Math.max(0, ROUTE_PREVIEW_DEBOUNCE_MS - (now() - lastRouteRequestAt));
    pendingRouteTimer = scheduleTimeout(firePendingRoute, remaining);
  }

  /** Clear all route-mode overlays and reset waypoints (mid-gesture-clear invariant).
   * Also invalidates any in-flight pathfind request (`pendingSeq++`) and resets the
   * leading-edge debounce clock (`lastRouteRequestAt = -Infinity`) so the next gesture's
   * first move always fires immediately rather than inheriting a stale cooldown.
   * @example
   * ```
   * declare function clearRoute(): void;
   * clearRoute();
   * ```
   */
  function clearRoute(): void {
    pendingSeq++; // invalidate any in-flight request
    clearPendingRouteTimer();
    ctx.scene.clearOverlay();
    ctx.scene.clearMeasure();
    waypoints = [];
    lastPreviewedPath = null;
    lastRouteRequestAt = -Infinity; // next gesture's first move fires leading-edge
  }

  /** Commit a route from the selected token's center to `goal`: sends a `moveRequest` to the
   * server. Does NOT animate from that call's resolve value — animation is broadcast-driven
   * via `MoveStream`/`onMoveStream` for all scene viewers (see the `sendRequest` inline
   * comment below). The authoritative position instead arrives separately via the normal
   * store Event (token → stop). Sample-driven playback wins regardless of which of the two
   * arrives first, and this doc deliberately asserts no ordering between them: if
   * `animateSamples` registered first, `TokenAnimator.setTarget`'s `samplesAnim` guard
   * makes the Event a no-op; if the Event
   * landed first, the ease tween it started is deleted when `TokenAnimator.animateSamples` registers.
   * Either way the broadcast trajectory owns the token's motion.
   * On reject, clears the route overlay (no move).
   *
   * Reuses the last-previewed `PathResult.path` (`lastPreviewedPath`) when already computed
   * for this route, avoiding a redundant pathfind round-trip; when none is cached, issues one
   * pathfind then sends the moveRequest with its result.
   *
   * Invariant: `committing` is set TRUE before `seq` is captured. It is cleared by `finish()`
   * on whichever SETTLE branch still owns this commit (`seq === pendingSeq`) or by
   * `onDeactivate` (abort path). The pathfind's resolve with a usable path is not a settle — it
   * hands the commit off to `sendRequest`, which settles later. Clearing is deliberately NOT
   * done on a STALE resolve/reject (`seq !== pendingSeq`, re-checked on all four async
   * continuations: `sendRequest`'s resolve and reject, and the pathfind's resolve and reject): a
   * stale branch means a newer commit, or `onDeactivate`, already bumped `pendingSeq` and now
   * owns `committing` + teardown, so calling `finish()` here would release that LIVE commit's
   * guard and teardown out from under it — `finish()` calls `clearRoute()`, which does
   * `pendingSeq++`, so the newer commit's captured `seq` stops matching and its own settle
   * silently becomes a no-op. What that costs the newer commit depends on the phase it was in:
   * if it had already reached `sendRequest`, its `moveRequest` stays in flight and still
   * executes server-side; if it was still mid-pathfind, its own resolve returns early at the
   * same staleness check, BEFORE `sendRequest`, so no `moveRequest` is ever sent and the move
   * simply does not happen. Either way the client loses ownership — `committing` drops while a
   * request may still be outstanding, so `onPointerDown`'s `committing` guard no longer blocks a
   * second `commitRoute`, and the overlay and waypoints are torn down early. This ensures
   * pointer-up (which calls `clearRoute` in non-committing paths) cannot bump `pendingSeq`
   * between commit start and the async resolve, keeping `seq === pendingSeq` true so the
   * commit proceeds.
   * @param goal The commit target, in scene coords (post-snap).
   * @example
   * ```
   * declare function commitRoute(goal: Point): void;
   * declare const goal: Point;
   * commitRoute(goal);
   * ```
   */
  function commitRoute(goal: Point): void {
    if (!ctx.pathfind || !ctx.moveRequest || !ctx.tokenSelection || ctx.tokenSelection.ids.size !== 1) return;
    // Never branches on `movementModel`: the server move-execution path
    // (`execute_move`/`gate_walk`) is engine-agnostic, so committing a route proceeds
    // identically for grid-stepped and continuous scenes.
    const scene = activeScene(ctx);
    const start = tokenCenter();
    if (!scene || !start) return;
    const tokenId = [...ctx.tokenSelection.ids][0];
    const fp = HYPOTHETICAL_PREVIEW_FOOTPRINT_CELLS;
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
      ctx.pathfind(scene.id, start, [...waypoints, [goal.x, goal.y]], fp, selectedTokenId()).then(
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
          // Leading-edge debounce: the first move in a burst fires immediately; moves
          // arriving within the cooldown window are suppressed. Never re-arms on every
          // event — only a request that actually fires advances the clock — so this
          // cannot starve under continuous pointer movement. Whichever move happens to
          // be current once the window elapses always carries the freshest goal, since
          // `goal` is computed fresh from the triggering event, never queued.
          const t = now();
          if (t - lastRouteRequestAt >= ROUTE_PREVIEW_DEBOUNCE_MS) {
            lastRouteRequestAt = t;
            clearPendingRouteTimer(); // this fire supersedes any earlier suppressed goal
            requestRoute(scene, start, goal);
          } else {
            // Suppressed: still update the pending goal and (re)schedule a deferred fire
            // for when the cooldown elapses, so a hover-only stop (no further move event)
            // still eventually sends the latest position instead of freezing the preview.
            schedulePendingRouteFire(goal);
          }
        }
        return;
      }
      // Fallback: plain gridDistance measure, labelled through the SAME `formatCellDistance`
      // the route branch uses, so the two branches express an identical distance identically.
      // `activeScene`'s own `{ perCell: 5, unit: "ft" }` default covers the case where no scene
      // is viewed, matching `resolveSceneSettings`'s default exactly.
      if (!anchor) return;
      const cells = ctx.scene.gridDistance(anchor, p);
      const scene = activeScene(ctx) ?? { perCell: 5, unit: "ft" };
      ctx.scene.drawMeasure(anchor, p, formatCellDistance(cells, scene));
    },
    onPointerUp(_p: Point): void {
      if (inRouteMode()) {
        // When a commit is in flight, the commit owns its own teardown via finish().
        // Do NOT call clearRoute() here — that would bump pendingSeq and abort the commit.
        if (committing) return;
        // Release: clear overlays (mid-gesture-clear invariant). The actual move is
        // handled by the select-move tool / the server's `execute_move` gate — not here.
        clearRoute();
        return;
      }
      // Fallback: plain measure cleanup.
      if (!anchor) return;
      ctx.scene.clearMeasure();
      anchor = null;
    },
    onDeactivate(): void {
      // Tool-swap teardown: abort any in-flight commit and clear all overlays. clearRoute()
      // unconditionally bumps pendingSeq, invalidating any in-flight commit's captured seq —
      // its resolve/reject handler then bails (seq !== pendingSeq) instead of re-entering
      // finish(). committing is cleared directly here (rather than via finish()) since
      // finish() is never reached on this abort path.
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

/** Preview/persist points for a two-corner drag shape (or the freehand path): `"freehand"`
 * returns the accumulated path open (`closed: false`); `"line"` returns the two corners open;
 * `"rect"`/`"ellipse"` return a closed, axis-aligned bbox between the two corners
 * (`rectPoints`/`ellipsePoints`). Contrast `templatePath` below: its `"rect"` is a ROTATED
 * SQUARE centered on the anchor, not this axis-aligned two-corner bbox — same mode name,
 * different geometry per tool. Each tool is internally consistent across its OWN modes (this
 * one never mixes bbox and rotated-square); no design doc names the cross-tool split itself as
 * a decision, so treat it as consistent-by-construction rather than a documented tradeoff — it
 * mirrors the render layer's own `DrawingView.toSpec` vs `TemplateView.toSpec` split, not a bug.
 * @param mode The active draw mode; only `"freehand"` consults `freehand`.
 * @param a The drag start point (raw pointer scene coords; the sole caller, `makeDrawTool`,
 * never snaps).
 * @param b The current/drag-end point (raw pointer scene coords; the sole caller,
 * `makeDrawTool`, never snaps).
 * @param freehand The accumulated freehand path, as a flat `[x0,y0,x1,y1,...]` array.
 * @returns The preview/persist points plus whether the shape should render/persist closed.
 * @example
 * ```
 * declare const anchor: Point;
 * declare const b: Point;
 * const { points, closed } = shapePath("rect", anchor, b, []);
 * ```
 */
function shapePath(mode: DrawMode, a: Point, b: Point, freehand: number[]): {
  /** The preview/persist polyline for `mode`. */
  points: number[];
  /** Whether the shape should render/persist closed. */
  closed: boolean;
} {
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

/** Drag to draw: freehand collects the path; rect/ellipse/line span two corners. Points are
 * used RAW (unsnapped) — unlike `makeWallTool`/`makeRegionTool`/`makeTemplateTool`/`makePlaceTool`,
 * this tool never calls `ctx.scene.snap`, so a drawing can land anywhere, not just grid-aligned.
 * A live preview overlays while dragging; release persists a `drawing` doc (optimistic) only
 * when `hasExtent` confirms the gesture has visible extent — a pure click is skipped so no
 * invisible drawing is written to the scene + event log. No active scene → unhandled (camera
 * pans).
 * @param ctx The tool context; reads the active scene, dispatches the create.
 * @param controller Supplies the configured `drawMode`/`strokeColor`.
 * @returns A `SceneTool` implementing the drag-to-draw gesture.
 * @example
 * ```
 * declare const ctx: ToolContext;
 * declare const controller: ToolController;
 * const tool = makeDrawTool(ctx, controller);
 * ```
 */
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

/** Template area from an anchor + size + direction (degrees): `"circle"`/`"cone"` are
 * tessellated rings (`circlePoints`/`conePoints`), closed; `"rect"` is a ROTATED SQUARE
 * centered on the anchor (`squarePoints`, side `2*size`) — NOT the axis-aligned two-corner
 * bbox `shapePath`'s `"rect"` produces, despite the same mode-name string. This tool is
 * internally consistent across its own modes (always the rotated square for `"rect"`); the
 * cross-tool split with `shapePath` is consistent-by-construction, not a documented tradeoff —
 * it mirrors the render layer's `TemplateView.toSpec` vs `DrawingView.toSpec` split; `"line"` is the two
 * endpoints computed from `direction`/`size`, open.
 * @param mode The active template shape mode.
 * @param ax The anchor x (scene coords, post-snap).
 * @param ay The anchor y (scene coords, post-snap).
 * @param size The template's size (radius for circle/cone, half-side for rect, length for line).
 * @param direction The facing angle in degrees (unused by `"circle"`).
 * @returns The preview/persist points plus whether the shape should render/persist closed.
 * @example
 * ```
 * declare const anchor: Point;
 * declare const size: number;
 * declare const direction: number;
 * const { points, closed } = templatePath("cone", anchor.x, anchor.y, size, direction);
 * ```
 */
function templatePath(mode: TemplateMode, ax: number, ay: number, size: number, direction: number): {
  /** The preview/persist polyline (or ring) for `mode`. */
  points: number[];
  /** Whether the shape should render/persist closed. */
  closed: boolean;
} {
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
 * drag (`< 1` scene unit) falls back to one grid cell (`cell`) at `direction: 0`, so a click
 * (no visible drag) still places a default-sized template rather than a degenerate zero-size one.
 * @param a The drag anchor (scene coords, post-snap).
 * @param b The current/drag-end point (scene coords, post-snap — see `onPointerMove`/
 * `onPointerUp` below).
 * @param cell The active scene's grid cell size (the near-zero-drag fallback size).
 * @returns The resolved size (scene units) and direction (degrees).
 * @example
 * ```
 * declare const anchor: Point;
 * declare const b: Point;
 * declare const scene: { size: number };
 * const { size, direction } = sizeDir(anchor, b, scene.size);
 * ```
 */
function sizeDir(a: Point, b: Point, cell: number): {
  /** The resolved template size in scene units. */
  size: number;
  /** The resolved facing angle in degrees. */
  direction: number;
} {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const d = Math.hypot(dx, dy);
  if (d < 1) return { size: cell, direction: 0 };
  return { size: d, direction: (Math.atan2(dy, dx) * 180) / Math.PI };
}

/** Drag to place a template area (circle/cone/rect/line) anchored at the snapped cell; the
 * drag sets size + direction via `sizeDir`, fed the drag-end point post-snap (matching the
 * anchor's own frame) so a plain click lands within `sizeDir`'s `d < 1` fallback and yields the
 * intended one-cell default rather than an arbitrary small size. Live preview; release ALWAYS
 * persists a `template` doc (optimistic) — unlike `makeDrawTool`/`makeWallTool`/
 * `makeRegionTool`, there is no has-extent skip here: `sizeDir`'s near-zero-drag fallback IS this
 * tool's extent guard, substituting the one-cell default for a degenerate size rather than
 * skipping document creation.
 * @param ctx The tool context; reads the active scene, snaps the anchor, dispatches the create.
 * @param controller Supplies the configured `templateMode`/`templateColor`.
 * @returns A `SceneTool` implementing the drag-to-place gesture.
 * @example
 * ```
 * declare const ctx: ToolContext;
 * declare const controller: ToolController;
 * const tool = makeTemplateTool(ctx, controller);
 * ```
 */
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
      const { size, direction } = sizeDir(anchor, ctx.scene.snap(p), cell);
      const { points, closed } = templatePath(controller.templateMode, anchor.x, anchor.y, size, direction);
      const color = parseColor(controller.templateColor);
      ctx.scene.previewOverlay([{ points, closed, stroke: { color, width: 2 }, fill: closed ? { color, alpha: 0.25 } : null }]);
    },
    onPointerUp(p: Point): void {
      if (!anchor) return;
      const scene = activeScene(ctx);
      if (scene) {
        const { size, direction } = sizeDir(anchor, ctx.scene.snap(p), cell);
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

/** Leading-edge throttle for the drag's PREVIEW-OVERLAY redraw only (`previewMoves`): the
 * first move in a drag redraws immediately, then at most one redraw per window. Nothing crosses
 * the wire during the drag — `previewMoves` calls only `ctx.scene.previewOverlay`, never
 * `dispatchIntent`/`pathfind`/`moveRequest` — so there is no optimistic write and no remote view
 * being fed while dragging. The actual move commits exactly once, on release, via `commitMoves`,
 * independent of this window. */
const DRAG_THROTTLE_MS = 50;

/** Pick a token on pointerdown and drag the whole selection. Clicking an unselected token
 * replaces the selection with just it; Shift toggles it in/out. Dragging moves every selected
 * token by the same delta (each token's own target independently snapped via `ctx.scene.snap`),
 * preserving relative offsets — but nothing crosses the wire during the drag itself:
 * `DRAG_THROTTLE_MS` throttles only the local preview-overlay redraw (leading-edge, no trailing
 * fire, unlike the measure tool's `ROUTE_PREVIEW_DEBOUNCE_MS`). The real move commits exactly
 * once, on release, via `commitMoves`: a GM's commit writes the exact snapped drop point; a
 * non-GM's is request-only (`pathfind`/`moveRequest`), so the landed position is
 * SERVER-DETERMINED and may differ from the drop point (a wall/mask refusal or an arrest
 * truncation can land the token short of, or not at, where it was released). Empty space clears
 * the selection and yields the gesture to the camera. The selection itself is signified on the
 * token node (the render layer's selection highlight fx, driven by `TokenView` off the same
 * `tokenSelection` state), never by a tool overlay. For a GM, an empty-space click additionally
 * picks a light marker or wall segment into `controller.editingEntity` (the rail editor's
 * selection source); a token hit clears it.
 * @param ctx The tool context; reads token selection, snaps points, dispatches
 * intents/pathfind/moveRequest depending on role.
 * @param controller Receives the light/wall editing selection (`editingEntity`).
 * @returns A `SceneTool` implementing the drag-to-move-selection gesture.
 * @example
 * ```
 * declare const ctx: ToolContext;
 * declare const controller: ToolController;
 * const tool = makeSelectMoveTool(ctx, controller);
 * ```
 */
export function makeSelectMoveTool(ctx: ToolContext, controller: ToolController): SceneTool {
  const now = ctx.now ?? ((): number => Date.now());
  const sel = ctx.tokenSelection;
  let draggingId: string | null = null;
  let grabOrigin: Point = { x: 0, y: 0 };
  let origins = new Map<string, Point>(); // selected id -> original center at grab time
  let moved = false;
  let lastSentAt = -Infinity;

  /** Center of token `id`, defaulting to `(0, 0)` when the document or its `engine.x`/`engine.y`
   * is missing (never throws) — the same fallback-to-origin convention as the measure tool's
   * `tokenCenter`.
   * @param id The token document id.
   * @returns The token's center in scene coords.
   * @example
   * ```
   * declare function centerOf(id: string): Point;
   * declare const tokenId: string;
   * const c = centerOf(tokenId);
   * ```
   */
  const centerOf = (id: string): Point => {
    const e = ctx.documents.get(id)?.engine as {
      /** Token center x in scene coords; absent ⇒ falls back to 0. */
      x?: number;
      /** Token center y in scene coords; absent ⇒ falls back to 0. */
      y?: number;
    } | undefined;
    return { x: e?.x ?? 0, y: e?.y ?? 0 };
  };


  /** Drag feedback only — never a document write and never a move request. A player's token
   * must not appear to move until the server executes it (no optimistic prediction for a gated
   * move), so this is the sole per-move overlay for both roles: a route line per selected token
   * from its grab origin to the snapped provisional target.
   * @param delta The drag offset from grab origin (scene coords, unsnapped).
   * @example
   * ```
   * declare function previewMoves(delta: Point): void;
   * previewMoves({ x: 10, y: 0 });
   * ```
   */
  const previewMoves = (delta: Point): void => {
    const pts: number[] = [];
    for (const [, o] of origins) {
      const target = ctx.scene.snap({ x: o.x + delta.x, y: o.y + delta.y });
      pts.push(o.x, o.y, target.x, target.y);
    }
    ctx.scene.previewOverlay(
      pts.length > 0 ? [{ points: pts, closed: false, stroke: { color: ROUTE_COLOR, width: 2 }, fill: null }] : [],
    );
  };

  /** Commit the gesture, exactly once, on release. A GM writes the position directly via
   * `dispatchIntent` (a GM places a token wherever they choose — no wall/mask gate applies to a
   * GM's own write). A player's move is request-only: each selected token gets its own
   * `pathfind` + `moveRequest`, and its rendered position advances only when the resulting
   * `MoveStream` arrives (never from this function's own return). Per-token rather than batched:
   * `moveRequest` is per-token on the wire and the server gates each token independently, so one
   * token arresting while another completes is correct.
   * @param delta The drag offset from grab origin (scene coords, unsnapped) to commit.
   * @example
   * ```
   * declare function commitMoves(delta: Point): void;
   * commitMoves({ x: 10, y: 0 });
   * ```
   */
  const commitMoves = (delta: Point): void => {
    if (ctx.role === "gm") {
      const ops: WireOperation[] = [];
      for (const [id, o] of origins) {
        const target = ctx.scene.snap({ x: o.x + delta.x, y: o.y + delta.y });
        const eng = ctx.documents.get(id)?.engine as {
          /** The token's current stored x, read RAW (not resolved/defaulted) so the dispatched
           * update's `old` matches the server's field-level optimistic-concurrency check. */
          x?: number;
          /** The token's current stored y, read RAW for the same reason as `x`. */
          y?: number;
        } | undefined;
        ops.push({ op: "update", doc_id: id, changes: [
          { path: "/engine/x", old: eng?.x ?? null, new: target.x },
          { path: "/engine/y", old: eng?.y ?? null, new: target.y },
        ] });
      }
      if (ops.length > 0) ctx.dispatchIntent(ops);
      return;
    }
    const scene = activeScene(ctx);
    if (!scene || !ctx.pathfind || !ctx.moveRequest) return;
    const pathfind = ctx.pathfind;
    const moveRequest = ctx.moveRequest;
    for (const [id, o] of origins) {
      const target = ctx.scene.snap({ x: o.x + delta.x, y: o.y + delta.y });
      pathfind(scene.id, [o.x, o.y], [[target.x, target.y]], HYPOTHETICAL_PREVIEW_FOOTPRINT_CELLS, id)
        .then((result) => {
          if (result.path.length >= 2) return moveRequest(scene.id, id, result.path);
        })
        .catch(() => {
          // A refusal (out-of-mask destination, a springing secret wall, an arrest region, the
          // per-token in-flight lock) is the NORMAL outcome for a player. Swallowing it would
          // leave the token silently not moving with no explanation, indistinguishable from a
          // hung connection, so the preview line is dropped rather than left stale — the app-wide
          // `onMoveOutcome` observability signal (wired at the Stage level) already surfaces the
          // rejected/truncated/executed outcome for this same request.
          ctx.scene.previewOverlay([]);
        });
    }
  };

  return {
    onPointerDown(p: Point, ev: PointerEvent): boolean {
      const id = topTokenAt(ctx.documents.query("token"), p, ctx.documents, footprintsOf(ctx));
      if (!id) {
        // GM-only scene-entity editing: a click that hits no token picks a light marker, then a
        // wall segment, into the shared editing selection the rail editor reads. Both write
        // paths are GM-gated (the server rejects a non-GM's document write regardless; this
        // branch only decides which editor opens, and a player gets no editor affordance).
        if (ctx.role === "gm") {
          const scene = activeScene(ctx);
          const lightHit = scene
            ? topLightAt(ctx.documents.query("light").filter((d) => d.parent_id === scene.id), p)
            : null;
          const wallHit =
            !lightHit && scene
              ? topWallAt(ctx.documents.query("wall").filter((d) => d.parent_id === scene.id), p)
              : null;
          controller.editingEntity = lightHit
            ? { kind: "light", id: lightHit }
            : wallHit
              ? { kind: "wall", id: wallHit }
              : null;
        }
        sel?.clear();
        ctx.scene.clearOverlay();
        return false;
      }
      controller.editingEntity = null;
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
      return true;
    },
    onPointerMove(p: Point): void {
      if (!draggingId) return;
      moved = true;
      const delta = { x: p.x - grabOrigin.x, y: p.y - grabOrigin.y };
      const t = now();
      if (t - lastSentAt >= DRAG_THROTTLE_MS) {
        previewMoves(delta); // leading-edge coalesced preview
        lastSentAt = t;
      }
    },
    onPointerUp(p: Point): void {
      if (!draggingId) return;
      // Commit exactly once, on release (a pure click that never moved commits nothing).
      if (moved) commitMoves({ x: p.x - grabOrigin.x, y: p.y - grabOrigin.y });
      ctx.scene.setDraggingToken(null);
      draggingId = null;
      moved = false;
      ctx.scene.clearOverlay();
    },
  };
}
