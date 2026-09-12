// The roll -> 3D dice bridge. A stable handle owned by WorldSession and
// exposed on AppContext, so a system module can trigger a 3D tumble even though the
// dice-3d overlay is created lazily inside DiceOverlay's mount effect. DiceOverlay
// attaches on mount; before/after attachment every call no-ops, so a caller never
// crashes when no overlay is mounted (e.g. `dice3d: false`, or the stage module absent).
import type { RollOutcome } from "@shadowcat/core";

/** The overlay-facing seam: what `Dice3DBridge` forwards to once attached. */
export interface Dice3DHost {
  /** Plays a roll's settling tumble.
   * @param outcome - The roll's full deterministic outcome.
   * @param rollId - The roll's stable id. */
  roll(outcome: RollOutcome, rollId: string): void;
  /** Dismisses whatever is currently tumbling. */
  clear(): void;
}

/** The host-facing seam plus late-attachment (see `AppContext.dice3d`'s doc). */
export interface Dice3DInteraction extends Dice3DHost {
  /** Attach the live overlay; returns a detach that only clears the host if it is still
   * the current one (a stale detach after re-attach is a no-op).
   * @param host - The live overlay implementing `Dice3DHost`.
   * @returns A detach function; safe to call multiple times or after superseded. */
  attach(host: Dice3DHost): () => void;
}

/**
 * Late-binding {@link Dice3DInteraction}: every method forwards to the attached host when
 * one is present, and no-ops before attach / after detach — so a caller can invoke
 * `ctx.dice3d.roll(...)` unconditionally without checking whether the overlay has mounted.
 */
export class Dice3DBridge implements Dice3DInteraction {
  /** The attached live overlay, or `null` before attach / after detach. */
  #host: Dice3DHost | null = null;

  /** Attach `host` as the live overlay. A later `attach` replaces the current host outright.
   * @param host - The live overlay implementing `Dice3DHost`.
   * @returns A detach function; safe to call multiple times or after superseded.
   * @example const detach = dice3d.attach(overlayHost);
   */
  attach(host: Dice3DHost): () => void {
    this.#host = host;
    return () => {
      if (this.#host === host) this.#host = null;
    };
  }

  /** Forward to the attached host; a no-op when detached.
   * @param outcome - The roll's full deterministic outcome.
   * @param rollId - The roll's stable id.
   * @example dice3d.roll(outcome, "roll-1");
   */
  roll(outcome: RollOutcome, rollId: string): void {
    this.#host?.roll(outcome, rollId);
  }

  /** Forward to the attached host; a no-op when detached.
   * @example dice3d.clear();
   */
  clear(): void {
    this.#host?.clear();
  }
}
