import type { DisplayBackend } from "./backend";
import type { VisibilityInput } from "./types";

/** Owns the mask slot. A thin pass-through to the backend's fog rendering
 * (`PixiBackend.setVisibility`/`setVisibilityBlend` paint the actual sheet+hole fog graphics —
 * see `pixi-backend.ts`) plus a `last`-applied cache so `current()` can answer without a
 * backend round-trip. This class has no fog-drawing logic of its own. */
export class Compositor {
  private last: VisibilityInput = { mode: "all", visible: [], explored: [] };

  /**
   * Constructs the compositor bound to a single backend.
   * @param backend The display backend `setVisibility`/`setVisibilityBlend` forward to.
   * @example
   * ```ts
   * import { Compositor, type DisplayBackend } from "@shadowcat/render";
   *
   * declare const backend: DisplayBackend;
   * const compositor = new Compositor(backend);
   * ```
   */
  constructor(private readonly backend: DisplayBackend) {}

  /** Apply a visibility mask: forwards to `backend.setVisibility` and caches it for `current()`.
   * Ends any in-flight `setVisibilityBlend` cross-fade the backend was mid-way through (see
   * `PixiBackend.setVisibility`'s own doc for that side effect).
   * @param input The visibility mask to apply.
   * @example
   * ```ts
   * import { Compositor, type DisplayBackend } from "@shadowcat/render";
   *
   * declare const backend: DisplayBackend;
   * const compositor = new Compositor(backend);
   * compositor.setVisibility({ mode: "all", visible: [], explored: [] });
   * ```
   */
  setVisibility(input: VisibilityInput): void {
    this.last = input;
    this.backend.setVisibility(input);
  }

  /** Cross-fade the mask between two consecutive vision samples (M2 §T7). `current()`
   * tracks the nearer endpoint (< 0.5 ⇒ `from`, else `to`) as a best-effort snapshot — the
   * backend, not this value, owns the actual blended visual. Falls back to a plain
   * `setVisibility` nearest-sample snap when the backend has no cross-fade support.
   * @param from The outgoing sample's visibility mask.
   * @param to The incoming sample's visibility mask.
   * @param factor Blend position in `[0,1]`: 0 = fully `from`, 1 = fully `to`.
   * @example
   * ```ts
   * import { Compositor, type DisplayBackend } from "@shadowcat/render";
   *
   * declare const backend: DisplayBackend;
   * const compositor = new Compositor(backend);
   * compositor.setVisibilityBlend(
   *   { mode: "masked", visible: [], explored: [] },
   *   { mode: "masked", visible: [], explored: [] },
   *   0.5,
   * );
   * ```
   */
  setVisibilityBlend(from: VisibilityInput, to: VisibilityInput, factor: number): void {
    this.last = factor < 0.5 ? from : to;
    if (this.backend.setVisibilityBlend) this.backend.setVisibilityBlend(from, to, factor);
    else this.backend.setVisibility(this.last);
  }

  /** The last applied visibility — a snapshot of the most recent `setVisibility`/
   * `setVisibilityBlend` argument (the blend's nearest-endpoint approximation, per that
   * method's own doc). Not re-applied on any lifecycle event today (e.g. a backend resize —
   * `RenderEngine.resize` calls `backend.resize` only and never reads this value); it exists
   * purely as a cheap read for a caller that wants "what did we last tell the mask to show".
   * @returns The most recently applied `VisibilityInput`.
   * @example
   * ```ts
   * import { Compositor, type DisplayBackend } from "@shadowcat/render";
   *
   * declare const backend: DisplayBackend;
   * const compositor = new Compositor(backend);
   * compositor.current(); // { mode: "all", visible: [], explored: [] } before any apply
   * ```
   */
  current(): VisibilityInput {
    return this.last;
  }
}
