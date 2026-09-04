// Pure cross-fade math for the mover vision-sweep fog.
// Extracted from `PixiBackend` so it is unit-testable without a GL/pixi.js context —
// `PixiBackend` itself is Playwright-covered only (no WebGL in jsdom).
// A `//` header, not a `/** */` block: a doc block preceding another doc block rather
// than a declaration binds to nothing, since every consumer takes the NEAREST one.

import type { Polygon, VisibilityInput } from "./types";

/**
 * Blend factor for cross-fading between two consecutive vision samples' rasterized fog
 * textures: 0 at `tCur` (fully the outgoing/"from" texture), 1 at `tNext` (fully the
 * incoming/"to" texture), linearly interpolated between and clamped to `[0,1]` outside.
 * A degenerate or non-finite span (`tNext <= tCur`, or any non-finite input) has nothing
 * meaningful to interpolate across and snaps to 1 (immediately "to") — fail-safe toward the
 * newer sample rather than freezing on a stale one.
 * @param clock The current playback time (same units/origin as `tCur`/`tNext`).
 * @param tCur The outgoing sample's timestamp.
 * @param tNext The incoming sample's timestamp.
 * @returns The blend factor in `[0,1]`, or `1` on a degenerate/non-finite input.
 * @example
 * ```
 * // not exported from @shadowcat/render; internal to PixiBackend's fog cross-fade
 * computeFogBlendFactor(150, 100, 200); // 0.5 — halfway between the two samples
 * computeFogBlendFactor(500, 100, 200); // 1 — past tNext, clamped
 * computeFogBlendFactor(50, 100, 100); // 1 — degenerate span (tNext <= tCur), fail-safe to newer
 * ```
 */
export function computeFogBlendFactor(clock: number, tCur: number, tNext: number): number {
  if (!Number.isFinite(clock) || !Number.isFinite(tCur) || !Number.isFinite(tNext)) return 1;
  if (tNext <= tCur) return 1;
  const f = (clock - tCur) / (tNext - tCur);
  return Math.min(1, Math.max(0, f));
}

/**
 * Whether a cached cross-fade `RenderTexture` (captured by `existing`'s `{width, height,
 * resolution}`) must be destroyed and recreated at the requested `width`/`height`/`resolution`,
 * rather than reused in place. `null` (no texture captured yet, e.g. first call) always stales.
 * Pure so `captureFog`'s reuse decision is unit-testable without a WebGL context.
 * @param existing The currently-captured texture's `{width, height, resolution}`, or `null`
 * if nothing has been captured yet.
 * @param width The requested capture width (CSS pixels).
 * @param height The requested capture height (CSS pixels).
 * @param resolution The requested device-pixel-ratio resolution.
 * @returns `true` if `existing` is `null` or any of the three dimensions differ (a resize or
 * DPR change), `false` if the existing texture can be reused in place.
 * @example
 * ```
 * // not exported from @shadowcat/render; internal to PixiBackend.setVisibilityBlend
 * fogBlendRtStale(null, 800, 600, 1); // true — nothing captured yet
 * fogBlendRtStale({ width: 800, height: 600, resolution: 1 }, 800, 600, 1); // false — reusable
 * fogBlendRtStale({ width: 800, height: 600, resolution: 1 }, 1024, 768, 1); // true — resized
 * ```
 */
export function fogBlendRtStale(existing: {
  /** The currently-captured texture's width, in CSS pixels. */
  width: number;
  /** The currently-captured texture's height, in CSS pixels. */
  height: number;
  /** The currently-captured texture's device-pixel-ratio resolution. */
  resolution: number;
} | null, width: number, height: number, resolution: number): boolean {
  if (!existing) return true;
  return existing.width !== width || existing.height !== height || existing.resolution !== resolution;
}

/**
 * A content fingerprint of one `VisibilityInput`, covering exactly the fields `paintFogSheets`
 * reads (`mode`, `visible`, `explored` — never `perceived`, which that function never touches).
 * Two inputs with equal keys paint pixel-identical fog, so `PixiBackend.setVisibilityBlend` uses
 * this to skip re-rasterizing a cross-fade endpoint whose content is unchanged from the previous
 * tick: a sweep holds the same `(from, to)` pair for many consecutive ticks (only the blend
 * `factor` moves), so recapturing on every tick redoes the SAME `paintFogSheets` draw + GPU
 * render-to-texture pass for content that has not changed since the prior call.
 * @param input The visibility sample to fingerprint.
 * @returns A string equal for two inputs `paintFogSheets` would draw identically.
 * @example
 * ```
 * // not exported from @shadowcat/render; internal to PixiBackend.setVisibilityBlend
 * visibilityInputKey({ mode: "all", visible: [], explored: [], perceived: [] }); // "all"
 * ```
 */
export function visibilityInputKey(input: VisibilityInput): string {
  if (input.mode !== "masked") return input.mode;
  const ring = (p: Polygon): string => p.points.join(",");
  return `masked|${input.visible.map(ring).join(";")}|${input.explored.map(ring).join(";")}`;
}

/** The sweep sample that should be showing at `elapsed` ms: the one with the greatest
 * `tMs <= elapsed`, or the first sample when `elapsed` precedes every sample. ONE rule for
 * every timed timeline — the fog sweep's `MoveVisionSample`s and the lighting sweep's
 * `MoveLightSample`s select through this same function, never a second copy.
 * INVARIANT (server parity): mirrors the server's `chosen_vision_sample` — the egress clip admits
 * a moving token's sample only where this sample's polygons will show it. Fixture-tested on both
 * sides and both sample kinds (`__fixtures__/chosen-vision-sample.json`).
 * @param samples The sweep's ordered samples (non-empty).
 * @param elapsed Milliseconds elapsed since the sweep started.
 * @returns The sample to show at `elapsed`.
 * @example
 * ```ts
 * chooseVisionSample([{ tMs: 0, polygons: [] }, { tMs: 500, polygons: [] }], 250).tMs; // 0
 * ```
 */
export function chooseVisionSample<T extends {
  /** The sample's elapsed offset from its timeline's origin, in ms. */
  tMs: number;
}>(samples: T[], elapsed: number): T {
  let chosen = samples[0];
  for (const s of samples) {
    if (s.tMs <= elapsed) chosen = s;
  }
  return chosen;
}
