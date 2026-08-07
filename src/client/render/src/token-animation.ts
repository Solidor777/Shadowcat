// Pure animated-token frame math. Extracted for the same reason as
// `computeFogBlendFactor` — `PixiBackend` itself is Playwright-covered
// only (no WebGL in jsdom), so the frame-selection logic lives here where it's unit-testable.
// A `//` header, not a `/** */` block: a doc block preceding another doc block rather
// than a declaration binds to nothing, since every consumer takes the NEAREST one.

/**
 * The frame index to display after `elapsedMs` of playback at `fps`, over `frameCount` frames.
 * `loop:true` wraps (`elapsedMs` can be arbitrarily large); `loop:false` clamps to the last frame
 * once the sequence completes (a one-shot animation holds its final frame, never wraps or stops
 * rendering). Degenerate input (`frameCount<=0`, non-finite `elapsedMs`/`fps`, `fps<=0`) fails
 * closed to frame 0 — always a valid index into a non-empty frame array, never a crash.
 * @param elapsedMs Milliseconds of playback elapsed since the animation started.
 * @param fps Playback rate in frames per second.
 * @param frameCount Total frame count in the source (sprite-sheet cell count or frame-URL list length).
 * @param loop Whether the sequence wraps (`true`) or holds its final frame (`false`).
 * @returns A valid index in `[0, frameCount)`, or `0` on degenerate input.
 * @example
 * ```
 * // not exported from @shadowcat/render; internal to TokenView/PixiBackend
 * computeAnimatedFrame(250, 8, 4, true); // 2 (0.25s × 8fps = 2 frames elapsed, into a 4-frame loop)
 * computeAnimatedFrame(10000, 8, 4, false); // 3 (clamped: the one-shot sequence has completed)
 * ```
 */
export function computeAnimatedFrame(elapsedMs: number, fps: number, frameCount: number, loop: boolean): number {
  if (!Number.isFinite(elapsedMs) || !Number.isFinite(fps) || fps <= 0 || frameCount <= 0) return 0;
  const frame = Math.floor((elapsedMs / 1000) * fps);
  if (loop) return ((frame % frameCount) + frameCount) % frameCount;
  return Math.min(Math.max(frame, 0), frameCount - 1);
}
