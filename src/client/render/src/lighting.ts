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

/** Channel-wise linear interpolation between two packed 0xRRGGBB colors. */
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

  constructor(private readonly backend: DisplayBackend) {}

  setTarget(input: LightingInput | null): void {
    // Capture whatever is on-screen right now as the interpolation start point.
    this.prev = this.currentInterpolated();
    this.target = input ? resolve(input) : { cell: this.target.cell, cells: [] };
    this.elapsed = 0;
    this.apply();
  }

  tick(dtMs: number): void {
    if (this.elapsed >= LIGHTING_FADE_MS) return;
    this.elapsed = Math.min(LIGHTING_FADE_MS, this.elapsed + dtMs);
    this.apply();
  }

  /** Return the last applied/interpolated frame (the value most recently painted by apply()). */
  current(): LightingFrame { return this._current; }

  private apply(): void {
    this._current = this.currentInterpolated();
    this.backend.setLighting(this._current);
  }

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
 * desaturate = hint index is in-range and names "desaturate". */
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
