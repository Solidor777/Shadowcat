/** Transient location-ping animation. Each ping expands an outline ring from 0 to
 * `PING_RADIUS` and fades from opaque to transparent over `PING_MS`, then drops. Pure +
 * headless-testable; the engine ticker drives `tick` and feeds the result to the backend. */
const PING_MS = 2000;
const PING_RADIUS = 60;

/** One ping's current draw state: scene-coord center, current outline radius, and alpha. */
export interface PingRing {
  x: number;
  y: number;
  radius: number;
  alpha: number;
}

/**
 * Tracks in-flight ping rings and advances them each frame. Holds no reference to a
 * backend or store — `RenderEngine.start()` drives `tick` from its ticker callback and
 * feeds the returned specs to `DisplayBackend.drawPings` (`SceneToolHost.addPing` forwards
 * to `add`).
 * @example
 * ```ts
 * import { PingView } from "@shadowcat/render";
 *
 * const pings = new PingView();
 * pings.add(10, 20);
 * pings.tick(16); // one live ring, radius/alpha just past their initial values
 * ```
 */
export class PingView {
  private pings: { x: number; y: number; age: number }[] = [];

  /**
   * Spawns a ping at scene `(x,y)`, age 0.
   * @param x The ping's scene x-coordinate.
   * @param y The ping's scene y-coordinate.
   * @example
   * ```ts
   * import { PingView } from "@shadowcat/render";
   *
   * const pings = new PingView();
   * pings.add(10, 20);
   * ```
   */
  add(x: number, y: number): void {
    this.pings.push({ x, y, age: 0 });
  }

  /**
   * Advances every tracked ping's age by `dtMs`, drops any that have reached `PING_MS`
   * (2000ms) of age, and returns the live ring specs — radius grows linearly `0` →
   * `PING_RADIUS` and alpha fades linearly `1` → `0` over that same span. Returns `[]`
   * once every ping has expired. The caller (`RenderEngine.start()`'s ticker callback)
   * redraws every frame while the result is non-empty, plus exactly one more frame right
   * after it goes from non-empty to empty, to send a final clearing draw — it does not
   * redraw on every subsequent idle frame.
   * @param dtMs Milliseconds elapsed since the previous `tick` call.
   * @returns The currently-live rings, oldest first.
   * @example
   * ```ts
   * import { PingView } from "@shadowcat/render";
   *
   * const pings = new PingView();
   * pings.add(0, 0);
   * pings.tick(2000); // [] — the ping has fully faded and is dropped
   * ```
   */
  tick(dtMs: number): PingRing[] {
    for (const p of this.pings) p.age += dtMs;
    this.pings = this.pings.filter((p) => p.age < PING_MS);
    return this.pings.map((p) => {
      const t = p.age / PING_MS; // 0 → 1
      return { x: p.x, y: p.y, radius: PING_RADIUS * t, alpha: 1 - t };
    });
  }
}
