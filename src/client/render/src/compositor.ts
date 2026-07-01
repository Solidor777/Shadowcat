import type { DisplayBackend } from "./backend";
import type { VisibilityInput } from "./types";

/** Owns the mask slot. M8 = identity (empty `visible` ⇒ transparent overlay). Feeds
 * VisibilityInput to the backend mask; M9 swaps an engine-owned fog shader + render
 * target behind this same surface with no API change. */
export class Compositor {
  private last: VisibilityInput = { mode: "all", visible: [], explored: [] };

  constructor(private readonly backend: DisplayBackend) {}

  setVisibility(input: VisibilityInput): void {
    this.last = input;
    this.backend.setVisibility(input);
  }

  /** Cross-fade the mask between two consecutive vision samples (M2 §T7). `current()`
   * tracks the nearer endpoint (< 0.5 ⇒ `from`, else `to`) as a best-effort snapshot — the
   * backend, not this value, owns the actual blended visual. Falls back to a plain
   * `setVisibility` nearest-sample snap when the backend has no cross-fade support. */
  setVisibilityBlend(from: VisibilityInput, to: VisibilityInput, factor: number): void {
    this.last = factor < 0.5 ? from : to;
    if (this.backend.setVisibilityBlend) this.backend.setVisibilityBlend(from, to, factor);
    else this.backend.setVisibility(this.last);
  }

  /** The last applied visibility (re-applied on resize in M9). */
  current(): VisibilityInput {
    return this.last;
  }
}
