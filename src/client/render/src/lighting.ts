import type { DisplayBackend } from "./backend";
import type { LightingInput } from "./types";

/** A resolved + interpolated cell ready to draw: alpha = band darkening, tint = packed color,
 * tintAlpha (0 ⇒ no tint), desaturate from the render hint. */
export interface LitDrawCell { i: number; j: number; alpha: number; tint: number; tintAlpha: number; desaturate: boolean }
export interface LightingFrame { cell: number; cells: LitDrawCell[] }

/** Cosmetic only — fog/lit-mask is the secrecy gate; this layer is purely visual. */
const LIGHTING_FADE_MS = 250;
/** Maximum darkening opacity applied at the darkest gradation band. */
const MAX_DARK_ALPHA = 0.6;
/** Tint overlay opacity when a packed color is present (tint !== 0). */
const TINT_ALPHA = 0.25;

const key = (c: { i: number; j: number }): string => `${c.i},${c.j}`;
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
 * (band→alpha, hint→desaturate) and interpolates day/night transitions; the backend
 * just paints LightingFrames. Cosmetic only — fog is the secrecy gate. */
export class Lighting {
  private prev: LightingFrame = { cell: 0, cells: [] };
  private target: LightingFrame = { cell: 0, cells: [] };
  /** Elapsed fade time; starts at LIGHTING_FADE_MS so the initial state is settled. */
  private elapsed = LIGHTING_FADE_MS;
  /** Cached result of the last apply(); avoids recomputing on every current() call and
   * eliminates the aliasing hazard of returning this.target by reference when settled. */
  private _current: LightingFrame = { cell: 0, cells: [] };

  /**
   * Constructs the lighting layer bound to a single backend.
   * @param backend The display backend `apply()` paints resolved frames into.
   * @example
   * ```ts
   * import { Lighting, type DisplayBackend } from "@shadowcat/render";
   *
   * declare const backend: DisplayBackend;
   * const lighting = new Lighting(backend);
   * ```
   */
  constructor(private readonly backend: DisplayBackend) {}

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
    this.target = input ? resolve(input) : { cell: this.target.cell, cells: [] };
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

  /** Recompute the interpolated frame and paint it via `backend.setLighting`. Called by both
   * `setTarget` (new fade start) and `tick` (fade progress) — the sole path that reaches the
   * backend, so `current()` and the painted frame can never disagree.
   * @example
   * ```
   * // private method; not part of the public API — invoked internally by setTarget/tick
   * this.apply();
   * ```
   */
  private apply(): void {
    this._current = this.currentInterpolated();
    this.backend.setLighting(this._current);
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
      // desaturate is boolean — snaps (no gradient between saturation states).
      return { i: tc.i, j: tc.j, alpha: lerp(pc.alpha, tc.alpha, t), tint, tintAlpha, desaturate: tc.desaturate };
    });
    return { cell: this.target.cell, cells };
  }
}

/** Resolve a parsed LightingInput into a LightingFrame with computed per-cell values.
 * alpha = (band / max(1, bandCount-1)) * MAX_DARK_ALPHA; band 0 (brightest) → 0 darkening.
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
 *   hints: ["desaturate"], cells: [{ i: 0, j: 0, band: 1, tint: 0, hint: 0 }] });
 * // { cell: 100, cells: [{ i: 0, j: 0, alpha: 0.6, tint: 0, tintAlpha: 0, desaturate: true }] }
 * ```
 */
function resolve(input: LightingInput): LightingFrame {
  const n = Math.max(1, input.bands.length - 1);
  const cells: LitDrawCell[] = input.cells.map((c) => ({
    i: c.i, j: c.j,
    alpha: (c.band / n) * MAX_DARK_ALPHA,
    tint: c.tint,
    tintAlpha: c.tint === 0 ? 0 : TINT_ALPHA,
    desaturate: c.hint >= 0 && input.hints[c.hint] === "desaturate",
  }));
  return { cell: input.cell, cells };
}
