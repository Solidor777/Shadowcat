/** Transient location-ping animation. Each ping expands an outline ring from 0 to
 * `PING_RADIUS` and fades from opaque to transparent over `PING_MS`, then drops. Pure +
 * headless-testable; the engine ticker drives `tick` and feeds the result to the backend. */
const PING_MS = 2000;
const PING_RADIUS = 60;

export interface PingRing {
  x: number;
  y: number;
  radius: number;
  alpha: number;
}

export class PingView {
  private pings: { x: number; y: number; age: number }[] = [];

  /** Spawn a ping at scene `(x,y)`. */
  add(x: number, y: number): void {
    this.pings.push({ x, y, age: 0 });
  }

  /** Advance every ping by `dtMs`; drop faded ones; return the live ring specs. */
  tick(dtMs: number): PingRing[] {
    for (const p of this.pings) p.age += dtMs;
    this.pings = this.pings.filter((p) => p.age < PING_MS);
    return this.pings.map((p) => {
      const t = p.age / PING_MS; // 0 → 1
      return { x: p.x, y: p.y, radius: PING_RADIUS * t, alpha: 1 - t };
    });
  }
}
