/**
 * Maps a volume fader's linear travel position (0..1) to the mixer gain (0..1) it selects.
 *
 * Algorithm: cubic taper, `gain = position^3`. Source: the audio-taper fader law family
 * (IEC 60268-18 describes logarithmic and exponential fader laws); a cubic is the standard
 * single-exponent approximation of perceived loudness against amplitude, chosen over a true
 * logarithm because it reaches exactly 0 at the bottom of the travel and has an exact inverse
 * (`gainToSliderPosition`). Applies at fader controls only: numeric gain inputs show the raw
 * gain and bypass this curve.
 * @param position The fader's travel position; values outside 0..1 are clamped.
 * @returns The gain in 0..1.
 * @example
 * ```ts
 * import { sliderPositionToGain } from "@shadowcat/audio";
 *
 * sliderPositionToGain(0.5); // 0.125
 * ```
 */
export function sliderPositionToGain(position: number): number {
  const p = Math.max(0, Math.min(1, position));
  return p * p * p;
}

/**
 * Maps a mixer gain (0..1) back to the fader travel position that selects it: the exact inverse
 * of `sliderPositionToGain`, used to place a fader's handle for a stored gain.
 * @param gain The mixer gain; values outside 0..1 are clamped.
 * @returns The fader's travel position in 0..1.
 * @example
 * ```ts
 * import { gainToSliderPosition } from "@shadowcat/audio";
 *
 * gainToSliderPosition(0.125); // 0.5
 * ```
 */
export function gainToSliderPosition(gain: number): number {
  const g = Math.max(0, Math.min(1, gain));
  return Math.cbrt(g);
}
