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

  /** The last applied visibility (re-applied on resize in M9). */
  current(): VisibilityInput {
    return this.last;
  }
}
