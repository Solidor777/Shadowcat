/** Pure animated-token frame math (M10h). Extracted for the same reason as
 * `computeFogBlendFactor` in `fog-blend.ts` — `pixi-backend.ts` itself is Playwright-covered
 * only (no WebGL in jsdom), so the frame-selection logic lives here where it's unit-testable. */

/**
 * The frame index to display after `elapsedMs` of playback at `fps`, over `frameCount` frames.
 * `loop:true` wraps (`elapsedMs` can be arbitrarily large); `loop:false` clamps to the last frame
 * once the sequence completes (a one-shot animation holds its final frame, never wraps or stops
 * rendering). Degenerate input (`frameCount<=0`, non-finite `elapsedMs`/`fps`, `fps<=0`) fails
 * closed to frame 0 — always a valid index into a non-empty frame array, never a crash.
 */
export function computeAnimatedFrame(elapsedMs: number, fps: number, frameCount: number, loop: boolean): number {
  if (!Number.isFinite(elapsedMs) || !Number.isFinite(fps) || fps <= 0 || frameCount <= 0) return 0;
  const frame = Math.floor((elapsedMs / 1000) * fps);
  if (loop) return ((frame % frameCount) + frameCount) % frameCount;
  return Math.min(Math.max(frame, 0), frameCount - 1);
}
