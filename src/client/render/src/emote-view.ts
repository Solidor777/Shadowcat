/** Transient emote animation. Each emote anchors its glyph(s) above the token's position
 * AT FIRE TIME — it never tracks a moving token, marking WHERE the emote was fired — rises
 * `rise` scene units and fades from opaque to transparent over `EMOTE_MS`, then drops. Pure +
 * headless-testable; the engine ticker drives `tick` and feeds the result to the backend. */
/** Total emote lifetime, in ms — an emote is dropped once its age reaches this. */
const EMOTE_MS = 2000;

/** One emote's current draw state: scene-coord anchor, glyph(s), and alpha. */
export interface EmoteGlyph {
  /** Emote anchor's scene x-coordinate (fixed at spawn). */
  x: number;
  /** Current scene y-coordinate — starts at the spawn anchor and rises `rise` units over
   * the emote's lifetime. */
  y: number;
  /** The emote glyph(s) to draw (an emoji grapheme or short cluster). */
  emote: string;
  /** Current opacity, `[0,1]` — `1` at spawn, `0` at expiry. */
  alpha: number;
}

/**
 * Tracks in-flight emotes and advances them each frame. Holds no reference to a backend or
 * store — `RenderEngine.start()` drives `tick` from its ticker callback and feeds the
 * returned specs to `DisplayBackend.drawEmotes` (`SceneToolHost.addEmote` resolves the
 * token anchor and forwards to `add`).
 * @example
 * ```ts
 * import { EmoteView } from "@shadowcat/render";
 *
 * const emotes = new EmoteView();
 * emotes.add(10, 20, "😀", 100);
 * emotes.tick(16); // one live glyph, risen and faded just past its initial values
 * ```
 */
export class EmoteView {
  /** Every in-flight (unexpired) emote, oldest first. */
  private emotes: {
    /** Spawn anchor's scene x-coordinate. */
    x: number;
    /** Spawn anchor's scene y-coordinate. */
    y: number;
    /** Total rise over the emote's lifetime, in scene (px) units — the engine passes the
     * active grid's per-cell world distance (`Grid.worldUnitsPerCell`), the view's ONE
     * cell-size source. */
    rise: number;
    /** The emote glyph(s). */
    emote: string;
    /** Accumulated elapsed time, in ms, since this emote was added. */
    age: number;
  }[] = [];

  /**
   * Spawns an emote anchored at scene `(x,y)`, age 0.
   * @param x The emote anchor's scene x-coordinate.
   * @param y The emote anchor's scene y-coordinate.
   * @param emote The emote glyph(s) to draw.
   * @param rise The total rise over the emote's lifetime, in scene (px) units.
   * @example
   * ```ts
   * import { EmoteView } from "@shadowcat/render";
   *
   * const emotes = new EmoteView();
   * emotes.add(10, 20, "😀", 100);
   * ```
   */
  add(x: number, y: number, emote: string, rise: number): void {
    this.emotes.push({ x, y, rise, emote, age: 0 });
  }

  /**
   * Advances every tracked emote's age by `dtMs`, drops any that have reached `EMOTE_MS`
   * (2000ms) of age, and returns the live glyph specs — y rises linearly `0` → `rise` above
   * the spawn anchor and alpha fades linearly `1` → `0` over that same span. Returns `[]`
   * once every emote has expired. The caller (`RenderEngine.start()`'s ticker callback)
   * redraws every frame while the result is non-empty, plus exactly one more frame right
   * after it goes from non-empty to empty, to send a final clearing draw — it does not
   * redraw on every subsequent idle frame.
   * @param dtMs Milliseconds elapsed since the previous `tick` call.
   * @returns The currently-live glyphs, oldest first.
   * @example
   * ```ts
   * import { EmoteView } from "@shadowcat/render";
   *
   * const emotes = new EmoteView();
   * emotes.add(0, 0, "😀", 100);
   * emotes.tick(2000); // [] — the emote has fully faded and is dropped
   * ```
   */
  tick(dtMs: number): EmoteGlyph[] {
    for (const e of this.emotes) e.age += dtMs;
    this.emotes = this.emotes.filter((e) => e.age < EMOTE_MS);
    return this.emotes.map((e) => {
      const t = e.age / EMOTE_MS; // 0 → 1
      return { x: e.x, y: e.y - e.rise * t, emote: e.emote, alpha: 1 - t };
    });
  }
}
