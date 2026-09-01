/** A point in scene coordinates. */
export interface Point {
  /** Scene x-coordinate. */
  x: number;
  /** Scene y-coordinate. */
  y: number;
}

/** A line segment in scene coordinates (grid lines). */
export interface LineSeg {
  /** First endpoint's scene x-coordinate. */
  x1: number;
  /** First endpoint's scene y-coordinate. */
  y1: number;
  /** Second endpoint's scene x-coordinate. */
  x2: number;
  /** Second endpoint's scene y-coordinate. */
  y2: number;
}

/** Resolution-independent polygon geometry, scene coords, flat
 * [x0,y0,x1,y1,…]. Consumed by the compositor; defined here so the public
 * value-type surface is one module. */
export interface Polygon {
  /** Flat `[x0,y0,x1,y1,…]` coordinate pairs — see the interface doc for the encoding. */
  points: number[];
}

/** Camera transform applied to the world container: translate then uniform scale. */
export interface CameraTransform {
  /** World-container translation, x. */
  x: number;
  /** World-container translation, y. */
  y: number;
  /** Uniform scale factor applied after translation. */
  scale: number;
}

/** Visibility for the mask slot (scene coords). `mode:"all"` = no fog (GM / no occlusion).
 * `mode:"masked"` = three-state fog: **unexplored** (outside both sets) = darkest, **explored**
 * (in `explored`, not `visible`) = dimmed memory, **visible** = clear. Empty `visible` + empty
 * `explored` ⇒ full dark fog (see nothing), NOT "see everything". `explored` is the persistent
 * memory layer (rect polygons rasterized from the server's per-(scene,player) explored cells);
 * `visible ⊆ explored` semantically (a visible cell is also explored). */
export interface VisibilityInput {
  /** `"all"` (no fog) or `"masked"` (three-state fog) — see the interface doc. */
  mode: "all" | "masked";
  /** Polygons rendered as the clear (currently visible) fog state — see the interface doc. */
  visible: Polygon[];
  /** Polygons rendered as the dimmed-memory (explored, not currently visible) fog state — see
   * the interface doc. */
  explored: Polygon[];
}

/** A token's animatable transform (scene coords; `(x,y)` = center). */
export interface TokenTransform {
  /** Center's scene x-coordinate. */
  x: number;
  /** Center's scene y-coordinate. */
  y: number;
  /** Facing, in degrees — the value `PixiBackend.setToken` assigns to
   * `visualContainer.angle`. */
  rotation: number;
}

/** Asset UUIDs already resolved to serve URLs by the AssetResolver — the backend never
 * resolves asset ids itself, mirroring today's `assets.url(...)` call in `TokenView.toSpec`. */
export type ResolvedAnimatedSource =
  | {
      /** Discriminant: an ordered list of independently-loaded frame images. */
      type: "frames";
      /** Serve URLs, in playback order (already asset-id-resolved). */
      urls: string[];
    }
  | {
      /** Discriminant: a single sprite-sheet image sliced into a `rows`×`cols` grid. */
      type: "sheet";
      /** Sprite-sheet image serve URL (already asset-id-resolved). */
      url: string;
      /** Sheet row count; sliced row-major by `PixiBackend.loadAnimatedTextures`. */
      rows: number;
      /** Sheet column count; sliced row-major by `PixiBackend.loadAnimatedTextures`. */
      cols: number;
      /** Frame count to use, capped at `rows*cols`; omitted uses every cell. */
      count?: number;
    };

/** A resolved, already-URL'd art visual: a static image, or a tick-driven animation. This is
 * both a `TokenNodeSpec.visual` arm in its own right and the `art` payload of a resolved
 * `"generated"` visual (whose frame composes around exactly these two drawable kinds). */
export type ResolvedArtVisual =
  | {
      /** Discriminant: a static image visual. */
      kind: "image";
      /** Resolved image serve URL (already asset-id-resolved). */
      url: string;
    }
  | {
      /** Discriminant: a tick-driven animated visual. */
      kind: "animated";
      /** Resolved frame/sheet source — see `ResolvedAnimatedSource`. */
      source: ResolvedAnimatedSource;
      /** Playback rate, in frames per second — the `fps` `computeAnimatedFrame` scales
       * `elapsedMs` by. */
      fps: number;
      /** `true` wraps at the end of the sequence; `false` holds the final frame — see
       * `computeAnimatedFrame`. */
      loop: boolean;
    };

/** One built-in per-token art effect, applied to the token's visual by the backend (composed
 * into a single `ColorMatrixFilter` by `PixiBackend.updateTokenFx`, in array order). Every color
 * is already packed `0xRRGGBB` and every strength already clamped to `[0,1]` by `TokenView.toSpec`
 * — the backend never parses colors. Entries compose: a `tint` scales the
 * art's channels toward the given color, a `desaturate` strips color to luminance, a `highlight`
 * brightens the art toward the given color. */
export type TokenFx =
  | {
      /** Discriminant: scale the art's RGB channels toward `color`, by `strength`. */
      kind: "tint";
      /** Tint target color, packed `0xRRGGBB`. */
      color: number;
      /** Blend amount, `[0,1]`: `0` = no effect, `1` = full channel multiply by `color`. */
      strength: number;
    }
  | {
      /** Discriminant: strip the art to luminance (full desaturation; no strength — partial
       * desaturation is a `tint` toward the art's own gray). */
      kind: "desaturate";
    }
  | {
      /** Discriminant: brighten the art toward `color`, by `strength`. Produced for a token's
       * selection signifier and for condition-registry `highlight` fx; a `"target"` source is
       * reserved for this same arm (a targeted token would brighten the same way) but has no
       * producer — no targeting feature exists. */
      kind: "highlight";
      /** Brighten-toward target color, packed `0xRRGGBB`. */
      color: number;
      /** Blend amount, `[0,1]`: `0` = no effect, `1` = the art replaced by `color`. */
      strength: number;
    };

/** A resolved token render node: transform + size + resolved visual + faction border + footprint shape. */
export interface TokenNodeSpec {
  /** Center's scene x-coordinate — from `resolveTokenBox`. */
  x: number;
  /** Center's scene y-coordinate — from `resolveTokenBox`. */
  y: number;
  /** Bounding-box width, in scene (px) units — `resolveTokenBox`'s read of the server's resolved
   * extent, or the token engine's own `w` where the server has stated none. */
  w: number;
  /** Bounding-box height, in scene (px) units — `resolveTokenBox`'s read of the server's resolved
   * extent, or the token engine's own `h` where the server has stated none. */
  h: number;
  /** Facing, in degrees — see `TokenTransform.rotation`. */
  rotation: number;
  /** The resolved, already-URL'd visual to draw: an art visual (see `ResolvedArtVisual`), or a
   * generated composition framing one. */
  visual:
    | ResolvedArtVisual
    | {
        /** Discriminant: a generated composition — `art` cropped to `crop`, over an optional
         * background fill, ringed by an optional decorative border. */
        kind: "generated";
        /** The art being framed — never itself `"generated"` (`resolveTokenVisual` fails closed
         * on nesting before a spec reaches the backend). */
        art: ResolvedArtVisual;
        /** The shape `art` is cropped to: the inscribed ellipse of the token extent
         * (`"circle"`), or the extent rect (`"square"`). */
        crop: "circle" | "square";
        /** Decorative ring drawn around the cropped art, or absent for none — authored data,
         * distinct from the outer faction border (`borderColor`). */
        border?: {
          /** Ring color, packed `0xRRGGBB` (already `parseColor`-resolved). */
          color: number;
          /** Ring width, in token-fraction px. */
          width: number;
        };
        /** Fill drawn behind the cropped art (in the crop shape), or absent for none. */
        background?: {
          /** Fill color, packed `0xRRGGBB` (already `parseColor`-resolved). */
          color: number;
        };
      };
  /** Faction border color (0xRRGGBB), or null for no border. */
  borderColor: number | null;
  /** Condition marker glyphs (emoji), rendered as upright chips along the token's top edge. */
  badges: string[];
  /** Footprint shape: drives the border outline + hit-test. */
  shape: "square" | "circle";
  /** The token's aura disc, drawn UNDER the art (a `container` child ordered below
   * `visualContainer`), or absent for none. Fully pre-resolved by `TokenView.toSpec`: color is
   * already `parseColor`-packed, radius already converted from grid cells to scene units — the
   * backend never parses colors or computes grid math. */
  aura?: {
    /** Disc fill color, packed `0xRRGGBB`. */
    color: number;
    /** Disc fill opacity, read-side-clamped to `[0,1]` by the resolver. */
    opacity: number;
    /** Disc radius, in scene units. */
    radius: number;
  };
  /** Built-in art effects (see `TokenFx`), composed by the backend onto the token's visual
   * container — the fx rotate with the art; the badge chips stay clean. Fully pre-resolved by
   * `TokenView.toSpec` (condition-registry fx folded in condition array order, then the
   * selection highlight); absent or empty for no effects. */
  fx?: TokenFx[];
}

/** A drawn shape node: a polyline/polygon (flat scene-coord points) with optional fill
 * and stroke, parented to `layer`. Drawings + templates reconcile to this; all shape
 * tessellation (`circlePoints`/`conePoints`/`squarePoints`/`rectPoints`/`ellipsePoints`)
 * happens before reaching the backend. */
export interface ShapeNodeSpec {
  /** Target core-layer id (e.g. `"drawings"`, `"regions"`) — see the `layers` module's
   * `CORE_LAYERS`. */
  layer: string;
  /** Flat `[x0,y0,x1,y1,…]` scene-coord points, already tessellated before reaching the
   * backend. */
  points: number[];
  /** `true` draws the point list as a closed polygon (fill-eligible); `false` as an open
   * polyline (no fill). */
  closed: boolean;
  /** Outline styling, or `null` for no stroke. */
  stroke: {
    /** Stroke color, `0xRRGGBB`. */
    color: number;
    /** Stroke width, in scene (px) units. */
    width: number;
  } | null;
  /** Fill styling, or `null` for no fill (also unused when `closed` is `false`). */
  fill: {
    /** Fill color, `0xRRGGBB`. */
    color: number;
    /** Fill opacity, `[0,1]`. */
    alpha: number;
  } | null;
}

/** A canvas tool. The engine routes pointer events (in scene coords) to the active
 * tool first; `onPointerDown` returning true claims the gesture (else camera pans).
 * `onDeactivate` is called by `ToolController.toggle` whenever this tool is swapped
 * away (including toggling it off); tools that hold overlay state must clear it here
 * to satisfy the mid-gesture-clear invariant. */
export interface SceneTool {
  /** Handle a pointer-down at scene point `p`. Returning `true` claims the gesture (further
   * move/up events for it route to this tool); returning `false` lets the engine fall back to
   * camera pan for this gesture.
   * @param p The pointer-down position, in scene coordinates.
   * @param ev The originating `PointerEvent`.
   * @returns `true` to claim the gesture, `false` to defer to camera pan. */
  onPointerDown(p: Point, ev: PointerEvent): boolean;
  /** Handle a pointer-move while this tool owns the gesture (only called for a gesture whose
   * `onPointerDown` returned `true`).
   * @param p The pointer's current position, in scene coordinates.
   * @param ev The originating `PointerEvent`. */
  onPointerMove(p: Point, ev: PointerEvent): void;
  /** Handle a pointer-up while this tool owns the gesture, ending it.
   * @param p The pointer-up position, in scene coordinates.
   * @param ev The originating `PointerEvent`. */
  onPointerUp(p: Point, ev: PointerEvent): void;
  /** Optional teardown called on tool swap/deactivation. Clear any live overlays here. */
  onDeactivate?(): void;
}

/** A vision-polygon sample paired with a MoveSample by `tMs` (mover-only trajectory vision; a
 * MoveStream's `moverVision` is `null` for observers). Feeds the fog progressively during the
 * mover's own animation playback (`animateSamples`'s `moverVision` argument): the sample with the
 * greatest `tMs <= clock` is applied via `VisibilityInput{mode:"masked"}`, reverting to the last
 * subscription-derived vision at animation end. */
export interface MoveVisionSample {
  /** Server-clock offset, in ms, this sample applies from — see the interface doc's
   * `tMs <= clock` selection rule. */
  tMs: number;
  /** One raycast-visibility polygon per group, each a list of `[x,y]` scene-coord vertices. */
  polygons: [number, number][][];
}

/** One visible cell's lighting: grid coords + gradation band index + packed tint + hint ref +
 * resolved corner geometry. `hint` is an index into `LightingInput.hints`; -1 = no hint. */
export interface LitCell {
  /** Grid column index (square), or hex axial q. */
  i: number;
  /** Grid row index (square), or hex axial r. */
  j: number;
  /** Gradation band index — see `LightingInput.bands`. */
  band: number;
  /** Packed tint value for this cell. */
  tint: number;
  /** Index into `LightingInput.hints` — see the interface doc for the `-1` sentinel. */
  hint: number;
  /** This cell's scene-coordinate corners — `Grid.cellVertices(i, j)` on the active grid, resolved
   * by the caller so `Lighting`/the backend never re-derive cell geometry from `i`/`j` and a flat
   * cell size: a hex scene's axial indices are not square-indexable as `i*cellSize`/`j*cellSize`,
   * so the geometry is resolved once here. Square: an axis-aligned rect. Hex: a pointy-top
   * hexagon. */
  corners: Point[];
}

/** Parsed lighting for the active scene (engine-internal, pre-resolution). `null` ⇒ no overlay
 * (GM `mode:"all"`, or garbled/missing data — lighting is cosmetic, fog is the secrecy gate).
 * Fail-safe: null means no tint overlay, which is always safe because the server already decided
 * which cells are visible (`toVisibility` is the secrecy gate, not this). */
export interface LightingInput {
  /** Active scene's cell size, in px. */
  cell: number;
  /** Gradation bands, brightest-first. */
  bands: {
    /** Band name (e.g. `"bright"`, `"dim"`, `"dark"`). */
    name: string;
    /** Band floor — the minimum [0,1] illumination value admitting this band; mirrors
     * `scene::lighting::Band.min_illumination`, wire-keyed `min` by `compute_derived`. */
    min: number;
  }[];
  /** `renderHints` lookup table; `LitCell.hint` indexes into this. */
  hints: string[];
  /** Active scene's visible cells. */
  cells: LitCell[];
}

/** The engine surface tools drive (via the AppContext `scene` bridge). The
 * RenderEngine implements this; a detached bridge no-ops. */
export interface SceneToolHost {
  /** Set (or clear) the active tool; the no-tool case falls back to camera pan/zoom.
   * @param tool The tool to activate, or `null` so the camera owns all gestures. */
  setActiveTool(tool: SceneTool | null): void;
  /** Snap a scene point to the active grid's nearest cell CENTER (square: the containing
   * cell's center; hex: the nearest hex's center) — never a vertex/corner.
   * @param p A scene-coordinate point.
   * @returns `p` snapped to the active grid, or `p` unchanged when snapping is disabled. */
  snap(p: Point): Point;
  /** Toggle the scene-level snap-to-grid axis: disabled makes `snap` identity
   * (free-form float placement/movement for a snap-off scene); grid RENDERING is unaffected —
   * a snap-off scene may still display its reference grid. Every tool that calls `snap` via
   * the AppContext `scene` bridge inherits this automatically.
   * @param enabled `false` disables snapping (free-form placement/movement); `true` re-enables
   * it. */
  setSnapEnabled(enabled: boolean): void;
  /** Mark a token as locally dragging so its sprite snaps to the authoritative
   * transform (no tween lag) while a remote move still tweens; null clears it.
   * @param id The token document id to mark as dragging, or `null` to clear it. */
  setDraggingToken(id: string | null): void;
  /** Draw an ephemeral, non-document preview (tool in-progress shape) into the overlay.
   * @param shapes The preview shapes (no `layer` field — always the `overlays` layer). */
  previewOverlay(shapes: Omit<ShapeNodeSpec, "layer">[]): void;
  /** Clear the ephemeral preview overlay. */
  clearOverlay(): void;
  /** Whole-cell distance between two scene points via the active grid (measurement).
   * @param a The first scene point.
   * @param b The second scene point.
   * @returns The whole-cell distance between `a` and `b` on the active grid. */
  gridDistance(a: Point, b: Point): number;
  /** Draw the client-local measurement overlay (a segment + a distance label).
   * @param from The segment's start point (scene coords).
   * @param to The segment's end point (scene coords).
   * @param label The distance label text to center on the segment. */
  drawMeasure(from: Point, to: Point, label: string): void;
  /** Clear the measurement overlay. */
  clearMeasure(): void;
  /** Spawn a transient ping ring at scene `(x,y)` (from a received/own ping).
   * @param x The ping's scene x-coordinate.
   * @param y The ping's scene y-coordinate. */
  addPing(x: number, y: number): void;
  /** Drive a smooth local walk of a token along a route's scene-coord waypoints.
   * @param id The token document id to animate.
   * @param path Scene-coord `[x,y]` waypoints, in walk order. */
  animateAlongPath(id: string, path: [number, number][]): void;
  /** Drive server-broadcast sample-based playback: interpolates position between adjacent
   * MoveSamples by tMs; hides the token across occlusion gaps. `serverNow` is optional, used
   * once at call time as `Math.max(0, serverNow() - startServerMs)` to compute catch-up elapsed
   * time; when absent, elapsed starts at `0` (no catch-up assumed — NOT a `Date.now` fallback).
   * `moverVision` (mover-only; null for observers) drives a progressive fog sweep in step with
   * this same clock — see `MoveVisionSample`.
   * @param id The token document id whose playback this drives.
   * @param samples Position samples, each a `{tMs, pos}` pair; interpolated between adjacent
   * samples by `tMs`.
   * @param durationMs Total playback duration in ms (also the vision sweep's duration, when one
   * starts).
   * @param startServerMs The server clock time (ms) at which this playback begins.
   * @param serverNow Optional server-clock accessor, used once at call time to compute catch-up
   * elapsed time (see above); when absent, elapsed starts at `0`.
   * @param moverVision Mover-only per-sample vision polygons (`null`/absent for observers);
   * presence starts a fog vision-sweep alongside the position tween. */
  animateSamples(
    id: string,
    samples: {
      /** Server-clock offset, in ms, this sample applies from. */
      tMs: number;
      /** Scene-coord `[x,y]` position at this sample. */
      pos: [number, number];
    }[],
    durationMs: number,
    startServerMs: number,
    serverNow?: () => number,
    moverVision?: MoveVisionSample[] | null,
  ): void;
}
