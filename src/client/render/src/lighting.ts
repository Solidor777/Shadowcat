import type { DisplayBackend } from "./backend";
import type { LightingInput, LitCell, Point, Polygon } from "./types";

/** A resolved + interpolated cell ready to draw: alpha = band darkening, tint = packed color,
 * tintAlpha (0 ⇒ no tint), desaturate from the render hint, corners = resolved paint geometry.
 * This class treats `corners` opaquely — copied through from `LightingInput.cells` (via
 * `resolve`) and carried forward unchanged across a fade (see `currentInterpolated`) — it never
 * computes cell geometry itself; that stays `Grid`'s job. */
export interface LitDrawCell {
  /** Grid column index (square), or hex axial q. */
  i: number;
  /** Grid row index (square), or hex axial r. */
  j: number;
  /** Darkening fill opacity, `[0, MAX_DARK_ALPHA]`; `0` = no darkening (brightest band). */
  alpha: number;
  /** Packed `0xRRGGBB` tint color; `0` means no tint (see `tintAlpha`). */
  tint: number;
  /** Tint fill opacity — `0` when `tint === 0` (nothing to paint), else `TINT_ALPHA`. */
  tintAlpha: number;
  /** Whether to paint the flat desaturation wash — see `resolve`'s doc for the hint-name rule. */
  desaturate: boolean;
  /** This cell's scene-coordinate corners, carried through from `LitCell.corners` unchanged —
   * see the interface doc. */
  corners: Point[];
}
/** A resolved lighting overlay ready for `DisplayBackend.setLighting`. */
export interface LightingFrame {
  /** Active scene's cell size, in px. */
  cell: number;
  /** The resolved per-cell paint instructions. */
  cells: LitDrawCell[];
  /** The regions to paint at the darkest band wherever no `cells` entry lights them: the
   * viewer's line of sight (`Lighting.setDarkness`). A cell the viewer has line of sight to
   * but no light on is visible terrain only to a sense that needs no light, so it renders as
   * dark as the darkest gradation band — never as a clear, lit cell. Empty when no lighting
   * model applies (`Lighting.setTarget(null)`: a GM's `mode:"all"` frame, or no frame yet). */
  darkness: Polygon[];
}

/**
 * A content fingerprint of one `LightingFrame`, covering exactly the fields
 * `PixiBackend.setLighting` reads (`cell`, `cells`' `i`/`j`/`alpha`/`tint`/`tintAlpha`/
 * `desaturate`/`corners`, `darkness`'s polygons). Two frames with equal keys paint
 * pixel-identical overlays, so `Lighting.apply` uses this to skip a real `backend.setLighting`
 * repaint when the frame it just resolved is unchanged from the last one actually painted — a
 * carried-light or vision sweep holds the SAME resolved content for many consecutive ticks
 * (only the sweep's blend factor moves between two sample boundaries), and every caller
 * (`setSweep`, `setDarkness`, `tick`) rebuilds its argument as a fresh array/object each call, so
 * a reference-equality check would never catch the repeat.
 * @param frame The resolved lighting frame to fingerprint.
 * @returns A string equal for two frames `backend.setLighting` would paint identically.
 * @example
 * ```ts
 * import { lightingFrameKey } from "@shadowcat/render";
 *
 * lightingFrameKey({ cell: 100, cells: [], darkness: [] }); // "100||"
 * ```
 */
export function lightingFrameKey(frame: LightingFrame): string {
  const poly = (p: Polygon): string => p.points.join(",");
  const cell = (c: LitDrawCell): string =>
    `${c.i},${c.j},${c.alpha},${c.tint},${c.tintAlpha},${c.desaturate ? 1 : 0},${c.corners.map((pt) => `${pt.x}:${pt.y}`).join(",")}`;
  return `${frame.cell}|${frame.darkness.map(poly).join(";")}|${frame.cells.map(cell).join(";")}`;
}

/** Duration, in ms, of the day/night fade `Lighting.setTarget` starts and
 * `Lighting.tick` advances. Cosmetic only — fog/lit-mask is the secrecy gate;
 * this layer is purely visual. */
export const LIGHTING_FADE_MS = 250;
/** Maximum darkening opacity applied at the darkest gradation band. */
export const MAX_DARK_ALPHA = 0.6;
/** Tint overlay opacity when a packed color is present (tint !== 0). */
export const TINT_ALPHA = 0.25;

/** The darkening alpha of gradation band `band` under a `bandCount`-band gradation:
 * `(band / max(1, bandCount - 1)) * MAX_DARK_ALPHA`, so band 0 (brightest) darkens nothing and
 * the last band darkens fully. THE one band→alpha rule — `resolve` and the light sweep's
 * `lightSampleCells` both read it.
 * @param band The gradation band index (0 = brightest).
 * @param bandCount The gradation's band count.
 * @returns The darkening fill opacity in `[0, MAX_DARK_ALPHA]`.
 * @example
 * ```ts
 * import { bandAlpha } from "@shadowcat/render";
 *
 * bandAlpha(0, 3); // 0
 * bandAlpha(2, 3); // 0.6
 * ```
 */
export function bandAlpha(band: number, bandCount: number): number {
  return (band / Math.max(1, bandCount - 1)) * MAX_DARK_ALPHA;
}

/** Union a light sweep's cells over a base frame: a sweep cell REPLACES the base cell with the
 * same `"i,j"` key (the glow decides that cell's darkening and tint outright) and a sweep cell
 * with no base twin is appended. Pure; `Lighting.apply` paints the result while a sweep is set.
 * @param base The fade-interpolated committed frame.
 * @param sweep The sweep's cells, or `null` when no light sweep is in flight.
 * @returns The frame to paint.
 * @example
 * ```ts
 * import { mergeSweepCells } from "@shadowcat/render";
 *
 * mergeSweepCells({ cell: 100, cells: [], darkness: [] }, null).cells.length; // 0
 * ```
 */
export function mergeSweepCells(base: LightingFrame, sweep: LitDrawCell[] | null): LightingFrame {
  if (!sweep || sweep.length === 0) return base;
  const byKey = new Map(sweep.map((c) => [key(c), c]));
  const cells: LitDrawCell[] = base.cells.map((c) => byKey.get(key(c)) ?? c);
  const present = new Set(base.cells.map(key));
  for (const c of sweep) if (!present.has(key(c))) cells.push(c);
  return { cell: base.cell, cells, darkness: base.darkness };
}

/** Union two committed lighting inputs for the same scene: every cell of `prev` that `next`
 * does not restate is kept beside `next`'s cells, which win on a shared `"i,j"` key; `cell`,
 * `bands` and `hints` are `next`'s (a `prev` cell's `band`/`hint` index is read against
 * them). `RenderEngine` paints this while the viewer's own vision sweep plays: the committed
 * frame that follows a move lights the cells seen from the STOP, and the frame before it the
 * cells seen from the START — the sweeping line of sight runs between the two, so both sets
 * stay lit until the sweep ends and `next` alone applies. Pure.
 * @param prev The lighting in force when the sweep started.
 * @param next The newest committed lighting.
 * @returns The unioned input.
 * @example
 * ```ts
 * import { unionLightingInputs } from "@shadowcat/render";
 *
 * const a = { cell: 100, bands: [], hints: [], cells: [{ i: 0, j: 0, band: 0, tint: 0, hint: -1, corners: [] }] };
 * const b = { cell: 100, bands: [], hints: [], cells: [{ i: 1, j: 0, band: 0, tint: 0, hint: -1, corners: [] }] };
 * unionLightingInputs(a, b).cells.length; // 2
 * ```
 */
export function unionLightingInputs(prev: LightingInput, next: LightingInput): LightingInput {
  const present = new Set(next.cells.map(key));
  const cells: LitCell[] = [...next.cells];
  for (const c of prev.cells) if (!present.has(key(c))) cells.push(c);
  return { cell: next.cell, bands: next.bands, hints: next.hints, cells };
}

/** Hold a set of cells at their previous committed values while a carried-light sweep plays:
 * every cell of `next` whose `"i,j"` key is in `held` is replaced by the cell `prev` carries
 * under that key (or dropped when `prev` has none); every other cell is `next`'s, as are
 * `cell`, `bands` and `hints` (a `prev` cell's `band`/`hint` index is read against them, as
 * in `unionLightingInputs`). `RenderEngine` paints this while a light sweep plays, with `held`
 * the cells the sweeping torch's LAST sample lights: the post-move rebroadcast carries the
 * light at that final position, and applying those cells mid-walk would light the corridor's
 * far end before the torch gets there — while every OTHER change a committed frame carries
 * (another light switched on, a door opened) applies at once. Pure.
 * @param prev The lighting in force when the light sweep started.
 * @param next The newest committed lighting.
 * @param held The `"i,j"` keys to hold at `prev`'s values.
 * @returns `next` with the held cells taken from `prev`.
 * @example
 * ```ts
 * import { holdLightingCells } from "@shadowcat/render";
 *
 * const prev = { cell: 100, bands: [], hints: [], cells: [] };
 * const next = { cell: 100, bands: [], hints: [], cells: [{ i: 3, j: 0, band: 0, tint: 0, hint: -1, corners: [] }] };
 * holdLightingCells(prev, next, new Set(["3,0"])).cells.length; // 0 — held at prev, which has no such cell
 * ```
 */
export function holdLightingCells(prev: LightingInput, next: LightingInput, held: ReadonlySet<string>): LightingInput {
  if (held.size === 0) return next;
  const cells: LitCell[] = next.cells.filter((c) => !held.has(key(c)));
  for (const c of prev.cells) if (held.has(key(c))) cells.push(c);
  return { cell: next.cell, bands: next.bands, hints: next.hints, cells };
}

/** Cell identity key for matching a cell across `prev`/`target` frames during a fade.
 * @param c A cell's grid coordinates.
 * @param c.i Grid column index.
 * @param c.j Grid row index.
 * @returns `"i,j"`.
 * @example
 * ```
 * // module-private helper; not exported from @shadowcat/render
 * key({ i: 0, j: 0 }); // "0,0"
 * ```
 */
const key = (c: {
  /** Grid column index. */
  i: number;
  /** Grid row index. */
  j: number;
}): string => `${c.i},${c.j}`;
/** Linear interpolation between `a` and `b` at position `t`.
 * @param a The value at `t=0`.
 * @param b The value at `t=1`.
 * @param t Interpolation position, typically `[0,1]`.
 * @returns `a + (b - a) * t`.
 * @example
 * ```
 * // module-private helper; not exported from @shadowcat/render
 * lerp(0, 10, 0.5); // 5
 * ```
 */
const lerp = (a: number, b: number, t: number): number => a + (b - a) * t;

/**
 * Channel-wise linear interpolation between two packed 0xRRGGBB colors — splits each into
 * R/G/B, `lerp`s each channel independently, rounds, and repacks.
 * @param a The starting packed color.
 * @param b The ending packed color.
 * @param t Interpolation position in `[0,1]` (not clamped — callers pass the fade's own `t`).
 * @returns The interpolated packed 0xRRGGBB color.
 * @example
 * ```
 * // not exported from @shadowcat/render; internal to Lighting's day/night fade
 * lerpRgb(0xff0000, 0x0000ff, 0.5); // ~0x800080
 * ```
 */
function lerpRgb(a: number, b: number, t: number): number {
  const ar = (a >> 16) & 0xff, ag = (a >> 8) & 0xff, ab = a & 0xff;
  const br = (b >> 16) & 0xff, bg = (b >> 8) & 0xff, bb = b & 0xff;
  return (Math.round(lerp(ar, br, t)) << 16) | (Math.round(lerp(ag, bg, t)) << 8) | Math.round(lerp(ab, bb, t));
}

/** Owns the lighting layer's data. Resolves the parsed LightingInput to drawable cells
 * (band→alpha, hint→desaturate), interpolates day/night transitions, and carries the
 * darkness regions (`setDarkness`) every painted frame darkens where no cell lights them; the
 * backend just paints LightingFrames. The server's `vision` frame decides what is visible
 * (the lit set IS the visible set); this layer decides only how a visible cell looks. */
export class Lighting {
  /** The frame captured at the start of the current fade — the interpolation source. */
  private prev: LightingFrame = { cell: 0, cells: [], darkness: [] };
  /** The frame this fade is interpolating toward, set by `setTarget`. */
  private target: LightingFrame = { cell: 0, cells: [], darkness: [] };
  /** Elapsed fade time; starts at LIGHTING_FADE_MS so the initial state is settled. */
  private elapsed = LIGHTING_FADE_MS;
  /** Cached result of the last apply(); avoids recomputing on every current() call and
   * eliminates the aliasing hazard of returning this.target by reference when settled. */
  private _current: LightingFrame = { cell: 0, cells: [], darkness: [] };
  /** The in-flight light sweep's cells (`setSweep`), unioned over the fade-interpolated frame
   * at every paint; `null` when no carried-light sweep is playing. */
  private sweep: LitDrawCell[] | null = null;
  /** The regions `setDarkness` last set — painted as `LightingFrame.darkness` while a lighting
   * model is in force (`hasModel`), withheld otherwise. */
  private darkness: Polygon[] = [];
  /** Whether the last `setTarget` carried a lighting model (non-`null` input). With no model
   * there is no darkness to lift, so no darkness is painted either. */
  private hasModel = false;
  /** `lightingFrameKey` of the frame last actually handed to `backend.setLighting` — `null`
   * before the first paint. `apply()` compares against this to skip a repaint whose resolved
   * content is unchanged (see `lightingFrameKey`'s doc). */
  private lastPaintedKey: string | null = null;

  /**
   * Constructs the lighting layer bound to a single backend.
   * @param backend The display backend `apply()` paints resolved frames into.
   * @param onApply Optional observer of every painted frame (the engine's host observability
   * hook); receives exactly what `backend.setLighting` was handed.
   * @example
   * ```ts
   * import { Lighting, type DisplayBackend } from "@shadowcat/render";
   *
   * declare const backend: DisplayBackend;
   * const lighting = new Lighting(backend);
   * ```
   */
  constructor(
    private readonly backend: DisplayBackend,
    private readonly onApply?: (frame: LightingFrame) => void,
  ) {}

  /**
   * Set (or clear) the carried-light sweep overlay and repaint immediately — no fade: the
   * sweep is already time-interpolated per tick by `RenderEngine`'s light sweep, and the
   * committed day/night fade underneath keeps its own clock. `null` removes the overlay.
   * @param cells The sweep's cells (`lightSampleCells`/`blendLightCells`), or `null`.
   * @example
   * ```ts
   * import { Lighting, type DisplayBackend } from "@shadowcat/render";
   *
   * declare const backend: DisplayBackend;
   * const lighting = new Lighting(backend);
   * lighting.setSweep([{ i: 0, j: 0, alpha: 0, tint: 0xffcc66, tintAlpha: 0.25, desaturate: false, corners: [] }]);
   * lighting.setSweep(null);
   * ```
   */
  setSweep(cells: LitDrawCell[] | null): void {
    this.sweep = cells;
    this.apply();
  }

  /**
   * Set the regions painted at the darkest band wherever no cell lights them — the viewer's
   * current line of sight (the fog's `visible` polygons, or the vision sweep's chosen sample
   * while one plays) — and repaint at once. The same reference is a no-op (no repaint); the
   * regions are carried through fades unchanged (a day/night fade interpolates cells, not
   * where the viewer can see). Painted only while a lighting model is in force — see
   * `LightingFrame.darkness`.
   * @param regions The polygons to darken, in scene coordinates.
   * @example
   * ```ts
   * import { Lighting, type DisplayBackend } from "@shadowcat/render";
   *
   * declare const backend: DisplayBackend;
   * const lighting = new Lighting(backend);
   * lighting.setDarkness([{ points: [0, 0, 100, 0, 100, 100, 0, 100] }]);
   * ```
   */
  setDarkness(regions: Polygon[]): void {
    if (regions === this.darkness) return;
    this.darkness = regions;
    this.apply();
  }

  /**
   * Set (or clear) the lighting overlay's target and start a {@link LIGHTING_FADE_MS}-ms
   * day/night fade toward it from whatever is currently displayed. `null` retargets to an
   * empty overlay at the previous cell size (no darkening/tint cells; the active scene has
   * `mode:"all"` or the payload is otherwise unusable — lighting is cosmetic, so this never
   * affects secrecy). Calling this again mid-fade retargets from the CURRENT interpolated
   * state (`currentInterpolated()`), not from the previous call's target — a rapid double
   * retarget continues smoothly rather than snapping.
   * @param input The parsed lighting for the active scene, or `null` to clear the overlay.
   * @example
   * ```ts
   * import { Lighting, type DisplayBackend } from "@shadowcat/render";
   *
   * declare const backend: DisplayBackend;
   * const lighting = new Lighting(backend);
   * lighting.setTarget({ cell: 100, bands: [{ name: "bright", min: 0.67 }], hints: [], cells: [] });
   * lighting.setTarget(null); // clear the overlay
   * ```
   */
  setTarget(input: LightingInput | null): void {
    // Capture whatever is on-screen right now as the interpolation start point.
    this.prev = this.currentInterpolated();
    this.target = input ? resolve(input) : { cell: this.target.cell, cells: [], darkness: [] };
    this.hasModel = input !== null;
    this.elapsed = 0;
    this.apply();
  }

  /**
   * Advance the day/night fade by `dtMs` and repaint. A no-op once the fade has already
   * reached {@link LIGHTING_FADE_MS} (settled) — repeat calls after settling neither
   * recompute nor repaint until the next `setTarget()`.
   * @param dtMs Milliseconds elapsed since the last tick.
   * @example
   * ```ts
   * import { Lighting, type DisplayBackend } from "@shadowcat/render";
   *
   * declare const backend: DisplayBackend;
   * const lighting = new Lighting(backend);
   * lighting.tick(16); // one frame at ~60fps
   * ```
   */
  tick(dtMs: number): void {
    if (this.elapsed >= LIGHTING_FADE_MS) return;
    this.elapsed = Math.min(LIGHTING_FADE_MS, this.elapsed + dtMs);
    this.apply();
  }

  /** Return the last applied/interpolated frame (the value most recently painted by apply()).
   * @returns The frame currently on-screen — snapshotted at the last `setTarget()`/`tick()`
   * call, not recomputed.
   * @example
   * ```ts
   * import { Lighting, type DisplayBackend } from "@shadowcat/render";
   *
   * declare const backend: DisplayBackend;
   * const lighting = new Lighting(backend);
   * lighting.current(); // { cell: 0, cells: [] } before any setTarget()
   * ```
   */
  current(): LightingFrame { return this._current; }

  /** Recompute the interpolated frame, union the light sweep over it (`mergeSweepCells`),
   * attach the darkness regions (withheld without a model) and paint it via
   * `backend.setLighting` — UNLESS its `lightingFrameKey` is unchanged from the last frame
   * actually painted, in which case the repaint is skipped: a sweep in flight calls this every
   * tick while its resolved content stays the same for many consecutive ticks (see
   * `lightingFrameKey`'s doc), and a per-cell Graphics rebuild is real work worth skipping
   * under a starved renderer. **`onApply` fires UNCONDITIONALLY on every call, dedup or not —
   * it is a separate observer contract, not gated by the repaint decision.**
   * `lightingFrameKey` fingerprints only what `backend.setLighting` paints
   * (`cell`/`cells`/`darkness`); `onApply`'s sole production consumer (`RenderEngine`'s
   * `onLightingApplied` wiring) closes over `this.lightSweeps.size > 0` at call time — state
   * outside the key entirely. Gating `onApply` on the same key would silently drop the
   * observer's notification whenever a sweep's final resolved content happens to equal the
   * frame already on screen (the ordinary case: the committed lighting at a walk's stop
   * usually lights the same cells the sweep's last sample did) — the observer would never
   * learn the sweep ended. Called by `setTarget` (new fade start), `tick` (fade progress),
   * `setSweep` and `setDarkness` — the sole path that reaches the backend, so `current()`
   * always reflects the latest RESOLVED frame regardless of whether it was repainted, and
   * `onApply` always reflects the latest CALL regardless of whether it repainted.
   * @example
   * ```
   * // private method; not part of the public API — invoked internally by setTarget/tick/setSweep
   * this.apply();
   * ```
   */
  private apply(): void {
    const merged = mergeSweepCells(this.currentInterpolated(), this.sweep);
    this._current = { cell: merged.cell, cells: merged.cells, darkness: this.hasModel ? this.darkness : [] };
    const key = lightingFrameKey(this._current);
    if (key !== this.lastPaintedKey) {
      this.lastPaintedKey = key;
      this.backend.setLighting(this._current);
    }
    this.onApply?.(this._current);
  }

  /** Compute the frame for the fade's current `elapsed`/`prev`/`target` state. At `t>=1`
   * (settled) returns `this.target` BY REFERENCE — safe only because callers read it through
   * `apply()`'s `_current` cache rather than holding this return value across a later mutation
   * (see `_current`'s own comment). Below `t=1`, blends per cell keyed by `"i,j"`: a cell
   * present only in `target` snaps in immediately (a newly-visible cell is not a day/night
   * fade — it appears as soon as `resolve()` produces it); a cell present only in `prev` is
   * simply absent from the result (snaps gone, never ghosts); a cell in both lerps `alpha`
   * linearly, holds `tint` at whichever side already has `tintAlpha>0` when the other side is
   * untinted (avoids visibly lerping a live color toward black), channel-blends `tint` via
   * `lerpRgb` when both sides are tinted, and lerps `tintAlpha`; `desaturate` is boolean and
   * always snaps to the target's value (no partial-desaturation gradient).
   * @returns The blended (or, once settled, target) `LightingFrame`.
   * @example
   * ```
   * // private method; not part of the public API — invoked internally by setTarget/apply
   * this.currentInterpolated();
   * ```
   */
  private currentInterpolated(): LightingFrame {
    const t = this.elapsed / LIGHTING_FADE_MS;
    if (t >= 1) return this.target;
    const prevByKey = new Map(this.prev.cells.map((c) => [key(c), c]));
    const cells: LitDrawCell[] = this.target.cells.map((tc) => {
      const pc = prevByKey.get(key(tc));
      // Cell only in target: snap (visibility changes are not day/night fades).
      if (!pc) return tc;
      const tintAlpha = lerp(pc.tintAlpha, tc.tintAlpha, t);
      // When one side has no tint (tintAlpha===0), hold the other side's color to
      // avoid lerping the live color toward black — only fade the alpha channel.
      const tint =
        pc.tintAlpha === 0 ? tc.tint :
        tc.tintAlpha === 0 ? pc.tint :
        lerpRgb(pc.tint, tc.tint, t);
      // desaturate is boolean — snaps (no gradient between saturation states). corners are
      // carried from the target unchanged: cell geometry is fixed by the active grid, not
      // something a day/night fade interpolates.
      return { i: tc.i, j: tc.j, alpha: lerp(pc.alpha, tc.alpha, t), tint, tintAlpha, desaturate: tc.desaturate, corners: tc.corners };
    });
    return { cell: this.target.cell, cells, darkness: [] };
  }
}

/** Resolve a parsed LightingInput into a LightingFrame with computed per-cell values.
 * alpha = `bandAlpha(band, bandCount)`; band 0 (brightest) → 0 darkening.
 * tintAlpha = 0 when tint===0 (no color), else TINT_ALPHA.
 * desaturate = hint index is in-range and names "desaturate" — any OTHER hint string (e.g. a
 * custom vision mode's `renderHint: "outline"`) resolves `desaturate: false`, silently applying
 * no visual treatment; `"desaturate"` is the only render hint this layer currently interprets.
 * @param input The parsed lighting for the active scene.
 * @returns A `LightingFrame` with one `LitDrawCell` per input cell, in the same order.
 * @example
 * ```
 * // not exported from @shadowcat/render; internal to Lighting.setTarget
 * resolve({ cell: 100, bands: [{ name: "bright", min: 0.67 }, { name: "dark", min: 0 }],
 *   hints: ["desaturate"], cells: [{ i: 0, j: 0, band: 1, tint: 0, hint: 0, corners: [] }] });
 * // { cell: 100, cells: [{ i: 0, j: 0, alpha: 0.6, tint: 0, tintAlpha: 0, desaturate: true, corners: [] }], darkness: [] }
 * ```
 */
function resolve(input: LightingInput): LightingFrame {
  const cells: LitDrawCell[] = input.cells.map((c) => ({
    i: c.i, j: c.j,
    alpha: bandAlpha(c.band, input.bands.length),
    tint: c.tint,
    tintAlpha: c.tint === 0 ? 0 : TINT_ALPHA,
    desaturate: c.hint >= 0 && input.hints[c.hint] === "desaturate",
    corners: c.corners,
  }));
  return { cell: input.cell, cells, darkness: [] };
}
