/** Remote shared rulers: other connections' live measured routes, drawn over the scene. Pure and
 * headless-testable; the engine ticker drives `tick` and feeds the result to
 * `DisplayBackend.drawRulers`. */

/** How long, in ms, a remote ruler survives without an update before it is dropped. A sender
 * keeps its ruler alive by re-sending at least once per this interval (scene-tools'
 * `RULER_KEEPALIVE_MS` is a fraction of it), so a sender that disconnects or loses its stop
 * frame to a rate limit leaves no permanent ghost overlay. */
export const RULER_STALE_MS = 3000;

/** Stroke colors assigned to remote senders, indexed by `rulerColor`. Distinct from the local
 * measure route color so a remote ruler is never mistaken for the viewer's own. */
const RULER_COLORS = [0x4fc3f7, 0xba68c8, 0x81c784, 0xffb74d, 0xf06292, 0x4db6ac] as const;

/** One remote ruler's current draw state. */
export interface RemoteRulerPath {
  /** The measuring user's id; the color is chosen from it. */
  user: string;
  /** The token of the connection that sent the ruler (`RulerNotice.origin`). One user measuring
   * on two connections yields two paths that differ only here. */
  origin: string;
  /** The ruler's vertices in scene coordinates, start through goal; one point draws as a dot. */
  points: [number, number][];
  /** Packed `0xRRGGBB` stroke color, stable per `user` (`rulerColor`). */
  color: number;
}

/** The map key of one remote ruler: the server-stamped sender plus the connection token. The
 * sender is part of the key because `origin` is client-chosen and relayed unverified, so keying
 * on it alone would let one user overwrite or clear another user's ruler by reusing that user's
 * token; the origin is part of the key so two connections of one user stay independent.
 * @param user The measuring user's id.
 * @param origin The sending connection's token.
 * @returns A key unique to that (user, connection) pair.
 * @example
 * ```ts
 * import { rulerKey } from "@shadowcat/render";
 *
 * rulerKey("u1", "a") !== rulerKey("u1", "b"); // true
 * ```
 */
export function rulerKey(user: string, origin: string): string {
  return JSON.stringify([user, origin]);
}

/** A stable stroke color for `user`: the same id always maps to the same palette entry, so two
 * simultaneous senders keep distinguishable, non-flickering colors. FNV-1a over the UTF-16 code
 * units of the id (Fowler–Noll–Vo, public-domain hash) chosen for its cheap, well-spread mapping
 * of short strings onto a small palette.
 * @param user The measuring user's id.
 * @returns A packed `0xRRGGBB` color from the ruler palette.
 * @example
 * ```ts
 * import { rulerColor } from "@shadowcat/render";
 *
 * rulerColor("user-a") === rulerColor("user-a"); // true
 * ```
 */
export function rulerColor(user: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < user.length; i++) {
    h ^= user.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return RULER_COLORS[h % RULER_COLORS.length];
}

/**
 * Tracks the live remote rulers, one per sending connection (`rulerKey`: user plus origin), so
 * simultaneous senders — including two connections of one user — never clobber each other. `set`
 * replaces exactly that connection's ruler, `clear` removes exactly that connection's, and `tick`
 * ages every ruler and drops any not updated within `RULER_STALE_MS`. Holds no reference to a
 * backend or store — `RenderEngine.start()` drives `tick` from its ticker callback.
 * @example
 * ```ts
 * import { RemoteRulerView } from "@shadowcat/render";
 *
 * const rulers = new RemoteRulerView();
 * rulers.set("user-a", "conn-1", [[0, 0], [70, 0]]);
 * rulers.set("user-a", "conn-2", [[0, 70], [70, 70]]);
 * rulers.clear("user-a", "conn-1");
 * rulers.tick(16).map((r) => r.origin); // ["conn-2"]
 * ```
 */
export class RemoteRulerView {
  /** Live rulers keyed by `rulerKey`, in first-seen order. */
  private rulers = new Map<
    string,
    {
      /** The measuring user's id. */
      user: string;
      /** The sending connection's token. */
      origin: string;
      /** The ruler's vertices in scene coordinates. */
      points: [number, number][];
      /** Milliseconds since the last `set` for this connection. */
      age: number;
    }
  >();

  /**
   * Sets the ruler of connection `origin` of `user` to `points`, age 0, replacing only that
   * connection's previous ruler. An empty list is the stop signal and removes it.
   * @param user The measuring user's id.
   * @param origin The sending connection's token.
   * @param points The ruler's vertices in scene coordinates.
   * @example
   * ```ts
   * import { RemoteRulerView } from "@shadowcat/render";
   *
   * const rulers = new RemoteRulerView();
   * rulers.set("user-a", "conn-1", [[0, 0], [70, 0]]);
   * ```
   */
  set(user: string, origin: string, points: [number, number][]): void {
    const key = rulerKey(user, origin);
    if (points.length === 0) {
      this.rulers.delete(key);
      return;
    }
    this.rulers.set(key, { user, origin, points, age: 0 });
  }

  /**
   * Removes the ruler of connection `origin` of `user`, leaving every other connection's
   * untouched.
   * @param user The measuring user's id.
   * @param origin The sending connection's token.
   * @example
   * ```ts
   * import { RemoteRulerView } from "@shadowcat/render";
   *
   * const rulers = new RemoteRulerView();
   * rulers.set("user-a", "conn-1", [[0, 0]]);
   * rulers.clear("user-a", "conn-1");
   * ```
   */
  clear(user: string, origin: string): void {
    this.rulers.delete(rulerKey(user, origin));
  }

  /**
   * Removes every ruler — the scene being viewed changed, so none belongs to it any longer.
   * @example
   * ```ts
   * import { RemoteRulerView } from "@shadowcat/render";
   *
   * const rulers = new RemoteRulerView();
   * rulers.set("user-a", "conn-1", [[0, 0]]);
   * rulers.clearAll();
   * ```
   */
  clearAll(): void {
    this.rulers.clear();
  }

  /**
   * Advances every ruler's age by `dtMs`, drops any that reached `RULER_STALE_MS`, and returns
   * the live rulers in first-seen order. The caller redraws every frame while the result is
   * non-empty, plus one final frame after it empties to send the clearing draw.
   * @param dtMs Milliseconds elapsed since the previous `tick` call.
   * @returns The currently-live rulers.
   * @example
   * ```ts
   * import { RemoteRulerView } from "@shadowcat/render";
   *
   * const rulers = new RemoteRulerView();
   * rulers.set("user-a", "conn-1", [[0, 0]]);
   * rulers.tick(3000); // [] — no update arrived within the staleness window
   * ```
   */
  tick(dtMs: number): RemoteRulerPath[] {
    const live: RemoteRulerPath[] = [];
    for (const [key, r] of this.rulers) {
      r.age += dtMs;
      if (r.age >= RULER_STALE_MS) {
        this.rulers.delete(key);
        continue;
      }
      live.push({ user: r.user, origin: r.origin, points: r.points, color: rulerColor(r.user) });
    }
    return live;
  }
}
