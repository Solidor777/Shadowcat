/** Transient emote animation. Each emote anchors its glyph(s) above the token's position
 * AT FIRE TIME — it never tracks a moving token, marking WHERE the emote was fired — rises
 * `rise` scene units and fades from opaque to transparent over `EMOTE_MS`, then drops. Pure +
 * headless-testable; the engine ticker drives `tick` and feeds the result to the backend. */
/** Total emote lifetime, in ms — an emote is dropped once its age reaches this. */
const EMOTE_MS = 2000;

/** Which draw treatment an overlay wants: `"glyph"` is the existing 1-4-emoji player emote (a
 * single short cluster, single-line, large glyph-scale font — `PixiBackend.drawEmotes` never
 * wraps this case); `"caption"` is a longer server-authored prose overlay (e.g. a region
 * trigger's `ScrollingText`), rendered at a smaller, wrapped, multi-line font. The producing
 * call site declares this — `RenderEngine.addEmote`'s `kind` parameter — rather than the
 * renderer guessing a treatment from string length, so a future third overlay kind is an
 * additive branch, never a heuristic. */
export type EmoteKind = "glyph" | "caption";

/** Upper bound (Unicode codepoints) an overlay DISPLAYS, regardless of how much text the
 * server accepted (`chat::MAX_MESSAGE_CHARS`-bounded at ingress, per `TriggerEffect::
 * ScrollingText`'s own doc — display truncation is this module's job, not the ingress
 * validator's). Truncated with a trailing "…" via `truncateForDisplay`.
 *
 * Arithmetic behind `80`: `PixiBackend.drawEmotes`'s caption treatment wraps at
 * `EMOTE_WRAP_WIDTH` (280 world units) at `CAPTION_FONT_SIZE` (20); a sans-serif glyph at that
 * size averages roughly half its font size in width, so ~28 characters fit one wrapped line
 * (280 / (20 * 0.5)). 80 characters therefore wraps to about 3 lines; at PixiJS's default
 * ~1.2x line-height spacing (24 world units/line) that block stands roughly 72 world units
 * tall — about ONE token height at this codebase's own canonical 70-unit example cell size
 * (every `new Grid({ kind: "square", size: 70 })` doc example in `grid.ts`), comfortably
 * within "a few lines, on the order of a token or two tall." A glyph-kind overlay (at most a
 * handful of emoji, itself already far under this cap) is never visibly affected. */
export const MAX_OVERLAY_DISPLAY_CHARS = 80;

/** Upper bound (Unicode codepoints) a `"glyph"` overlay DISPLAYS. Far tighter than
 * `MAX_OVERLAY_DISPLAY_CHARS` because the glyph treatment draws at `GLYPH_FONT_SIZE` on ONE
 * unwrapped line: 81 codepoints there would stand roughly 800 world units wide. A player emote
 * arrives already bounded by its own 16-byte wire field, but `SceneInteractionBridge.addEmote`
 * (reached by module code through `AppContext`) carries no such bound, so the display cap is what
 * holds. `16` matches that wire field's own budget — a handful of emoji, which is what the glyph
 * treatment is for; a caller wanting prose passes `kind: "caption"` and gets wrapping. */
export const MAX_GLYPH_DISPLAY_CHARS = 16;

/** Truncates `text` to the display cap for `kind` — `MAX_GLYPH_DISPLAY_CHARS` for `"glyph"`,
 * `MAX_OVERLAY_DISPLAY_CHARS` for `"caption"` — appending "…" when it
 * was cut — codepoint-safe (`Array.from`, never a raw UTF-16 slice) so a cut never splits a
 * surrogate pair. Codepoint granularity, not grapheme-cluster granularity: a cut between two
 * codepoints that render as one glyph (a flag's two regional indicators, a ZWJ sequence, an
 * emoji plus its skin-tone modifier) yields well-formed codepoints but a broken cluster. That is
 * cosmetic: a cut lands on a cluster only in text long enough to be truncated at all, which a
 * player emote (bounded by its own 16-byte wire field) never is — though nothing stops a module
 * caller of `SceneInteractionBridge.addEmote` from passing such a string, which is why the cap
 * applies to every kind rather than to captions alone. This is the ONE place overlay text
 * is bounded for DISPLAY (`EmoteView.add`
 * calls it for every kind); the server's own, more permissive `chat::MAX_MESSAGE_CHARS` ingress
 * bound is a separate, independent limit — see `MAX_OVERLAY_DISPLAY_CHARS`'s own doc for why
 * the two must not be conflated.
 * @param text The candidate overlay text.
 * @param kind The draw treatment the text is destined for, which selects the cap.
 * @returns `text` unchanged when it is already at or under that cap, else its first
 * capped codepoints plus a trailing "…".
 * @example
 * ```ts
 * import { truncateForDisplay } from "@shadowcat/render";
 *
 * truncateForDisplay("short"); // "short" — unchanged
 * truncateForDisplay("x".repeat(200)).endsWith("…"); // true — truncated
 * ```
 */
export function truncateForDisplay(text: string, kind: EmoteKind = "glyph"): string {
  const cap = kind === "caption" ? MAX_OVERLAY_DISPLAY_CHARS : MAX_GLYPH_DISPLAY_CHARS;
  const codepoints = Array.from(text);
  if (codepoints.length <= cap) return text;
  return codepoints.slice(0, cap).join("") + "…";
}

/** One emote's current draw state: scene-coord anchor, glyph(s), draw treatment, and alpha. */
export interface EmoteGlyph {
  /** Emote anchor's scene x-coordinate (fixed at spawn). */
  x: number;
  /** Current scene y-coordinate — starts at the spawn anchor and rises `rise` units over
   * the emote's lifetime. */
  y: number;
  /** The emote glyph(s) or caption text to draw, already display-truncated by `add`. */
  emote: string;
  /** Which draw treatment this overlay wants — see `EmoteKind`. */
  kind: EmoteKind;
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
 * emotes.add(10, 20, "😀", 100, "glyph");
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
    /** The emote glyph(s) or caption text, already display-truncated. */
    emote: string;
    /** Which draw treatment this overlay wants — see `EmoteKind`. */
    kind: EmoteKind;
    /** Accumulated elapsed time, in ms, since this emote was added. */
    age: number;
  }[] = [];

  /**
   * Spawns an emote anchored at scene `(x,y)`, age 0. `emote` is display-truncated via
   * `truncateForDisplay` before being tracked — the SAME bound applies regardless of `kind`,
   * so a future caller cannot bypass it by omitting `kind`.
   * @param x The emote anchor's scene x-coordinate.
   * @param y The emote anchor's scene y-coordinate.
   * @param emote The emote glyph(s) or caption text to draw.
   * @param rise The total rise over the emote's lifetime, in scene (px) units.
   * @param kind Which draw treatment this overlay wants; defaults to `"glyph"`, which draws the
   * player-emote glyph.
   * @example
   * ```ts
   * import { EmoteView } from "@shadowcat/render";
   *
   * const emotes = new EmoteView();
   * emotes.add(10, 20, "😀", 100, "glyph");
   * ```
   */
  add(x: number, y: number, emote: string, rise: number, kind: EmoteKind = "glyph"): void {
    this.emotes.push({ x, y, rise, emote: truncateForDisplay(emote, kind), kind, age: 0 });
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
   * emotes.add(0, 0, "😀", 100, "glyph");
   * emotes.tick(2000); // [] — the emote has fully faded and is dropped
   * ```
   */
  tick(dtMs: number): EmoteGlyph[] {
    for (const e of this.emotes) e.age += dtMs;
    this.emotes = this.emotes.filter((e) => e.age < EMOTE_MS);
    return this.emotes.map((e) => {
      const t = e.age / EMOTE_MS; // 0 → 1
      return { x: e.x, y: e.y - e.rise * t, emote: e.emote, kind: e.kind, alpha: 1 - t };
    });
  }
}
