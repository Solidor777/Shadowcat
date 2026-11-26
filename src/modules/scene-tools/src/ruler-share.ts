// The measure tool's shared-ruler sender: throttles and keeps alive the broadcast of this user's
// live route preview. Pure of DOM and framework state; the clock and timers are injected.

import { RULER_LIMITS } from "@shadowcat/core";
import { thinPolyline, type Polyline } from "./polyline-simplify";

/** The most vertices one shared-ruler update carries. Equals the server's per-frame cap
 * (`ws::ruler::MAX_RULER_POINTS`, pinned by `RULER_LIMITS`): the server answers a longer list
 * with a generic error and relays nothing, so `thinPolyline` thins a longer route to this many
 * before it is sent. */
export const MAX_RULER_POINTS: number = RULER_LIMITS.maxPoints;

/** The minimum spacing, in ms, between an update and the update or keepalive sent before it. A
 * stop does not reset that clock: updates are therefore at most 5 a second, and because every
 * stop is preceded by an update, one connection cannot exceed 10 frames a second (an update then
 * its stop, repeatedly) — under the server's per-user budget (`RULER_LIMITS.ratePerMin`, 12 a
 * second), so a stop frame is never the one dropped by a single connection's own traffic. The
 * server budget is per user across connections, so a user measuring from several connections at
 * once shares it and can exceed it. Fine-grained enough that a remote route follows the
 * cursor without visible lag. */
export const RULER_SEND_INTERVAL_MS = 200;

/** How often, in ms, an unchanged ruler is re-sent while still shown. Must stay well under the
 * receiver's staleness window (`@shadowcat/render`'s `RULER_STALE_MS`), so a ruler held still —
 * or one whose stop frame is lost — neither expires early nor lingers as a ghost. */
export const RULER_KEEPALIVE_MS = 1000;

/** Whether every vertex of `points` is a frame the server relays: both coordinates finite and
 * within `RULER_LIMITS.maxCoord` in magnitude. A frame failing this is answered with a generic
 * error (non-finite, sent as `null`) or dropped silently (out of range), so the sender never
 * sends one.
 * @param points The route's vertices.
 * @returns `true` when every coordinate is admissible.
 * @example
 * ```ts
 * routeAdmissible([[0, 0], [1, 1]]); // true
 * routeAdmissible([[0, 0], [Number.NaN, 1]]); // false
 * ```
 */
function routeAdmissible(points: Polyline): boolean {
  const bound = RULER_LIMITS.maxCoord;
  return points.every((p) => Math.abs(p[0]) <= bound && Math.abs(p[1]) <= bound);
}

/** What `RulerSharer` reads and drives, injected so the sharer is testable without a session. */
export interface RulerSharerDeps {
  /** Whether the user currently shares their ruler (`AppContext.shareRuler`). Read on every
   * update and every timer firing, so switching it off ends a shown ruler at once. */
  enabled: () => boolean;
  /** The id of the scene currently viewed, or `null` when none. Read on every timer firing, so a
   * ruler shown on a scene that is no longer viewed ends without another pointer event. */
  viewedScene: () => string | null;
  /** Sends one update for `scene` (`AppContext.sendRulerUpdate`). */
  send: (scene: string, points: Polyline, active: boolean) => void;
  /** Clock in ms (`Date.now` in the tool). Need not be monotonic: a backwards step is tolerated
   * by re-baselining the send clock in `update`. */
  now: () => number;
  /** Schedules `fn` after `ms`; returns a handle for `cancel`. */
  schedule: (fn: () => void, ms: number) => unknown;
  /** Cancels a handle returned by `schedule`. */
  cancel: (handle: unknown) => void;
}

/**
 * Broadcasts one user's live ruler at a bounded rate. `update` records the latest route and
 * sends it (thinned to the frame cap at send time) at once when `RULER_SEND_INTERVAL_MS` has
 * passed since the last update or keepalive sent, else defers
 * a single trailing send to the end of that window (so the newest route always goes out and a
 * burst of pointer events is at most one frame per window). While a ruler is shown, an unchanged
 * route is re-sent every `RULER_KEEPALIVE_MS`, and each such timer firing first re-checks that
 * sharing is still on and the ruler's scene is still viewed, ending the ruler otherwise. `stop`
 * cancels every timer and, only if observers were actually sent a route, sends exactly one
 * terminating `active: false` frame for the scene it was sent on — never a stop for a ruler an
 * observer was never shown. With sharing off the sharer sends no route; the one frame it can
 * still send is the terminating stop of a ruler that was already shown.
 * @example
 * ```ts
 * import { RulerSharer } from "./ruler-share";
 *
 * const sharer = new RulerSharer({
 *   enabled: () => true,
 *   viewedScene: () => "scene-1",
 *   send: () => {},
 *   now: () => Date.now(),
 *   schedule: (fn, ms) => setTimeout(fn, ms),
 *   cancel: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
 * });
 * sharer.update("scene-1", [[0, 0], [70, 70]]);
 * sharer.stop();
 * ```
 */
export class RulerSharer {
  /** The injected enable switch, viewed-scene reader, transport, clock and timers. */
  private readonly deps: RulerSharerDeps;
  /** The scene of the latest requested route; `null` before the first update and after `stop`. */
  private scene: string | null = null;
  /** The scene observers were last sent an active route for; `null` when none is shown. A stop
   * is owed exactly when this is non-null. */
  private sentScene: string | null = null;
  /** A copy of the latest requested route, owned by the sharer so the caller may mutate or reuse
   * its array after `update` returns; `flush` thins it to at most `MAX_RULER_POINTS` only when
   * a frame is sent. */
  private points: Polyline = [];
  /** `now()` of the last update or keepalive sent; `-Infinity` before the first. Stops do not touch it. */
  private lastSentAt = Number.NEGATIVE_INFINITY;
  /** The pending trailing-send or keepalive timer, if any. */
  private timer: unknown | null = null;

  /** Creates a sharer over `deps`.
   * @param deps The injected enable switch, viewed-scene reader, transport, clock and timers.
   * @example
   * ```ts
   * import { RulerSharer } from "./ruler-share";
   *
   * new RulerSharer({
   *   enabled: () => false,
   *   viewedScene: () => null,
   *   send: () => {},
   *   now: () => 0,
   *   schedule: () => null,
   *   cancel: () => {},
   * });
   * ```
   */
  constructor(deps: RulerSharerDeps) {
    this.deps = deps;
  }

  /** Records `points` as the current route on `scene` and shares it. A no-op that also ends any
   * shown ruler when sharing is off, `points` is empty or any coordinate is not finite or exceeds
   * `RULER_LIMITS.maxCoord`; a change of `scene` ends the ruler on the scene it was sent on
   * first.
   * @param scene The scene the ruler is measured on.
   * @param points The route preview's vertices in scene coordinates.
   * @example
   * ```ts
   * import { RulerSharer } from "./ruler-share";
   *
   * declare const sharer: RulerSharer;
   * sharer.update("scene-1", [[0, 0], [70, 70]]);
   * ```
   */
  update(scene: string, points: Polyline): void {
    if (!this.deps.enabled() || points.length === 0 || !routeAdmissible(points)) {
      this.stop();
      return;
    }
    if (this.scene !== null && this.scene !== scene) this.stop();
    this.scene = scene;
    this.points = points.map((p) => [p[0], p[1]]);
    const now = this.deps.now();
    // A clock that stepped backwards would make the elapsed time negative and the wait exceed
    // the send interval, replacing the keepalive with a longer timer that every further update
    // would re-arm without it ever shrinking; re-baselining to `now` treats the last send as just
    // made, so `elapsed` is never negative and the wait stays within (0, RULER_SEND_INTERVAL_MS].
    if (now < this.lastSentAt) this.lastSentAt = now;
    const elapsed = now - this.lastSentAt;
    if (elapsed >= RULER_SEND_INTERVAL_MS) this.flush();
    else this.arm(RULER_SEND_INTERVAL_MS - elapsed);
  }

  /** Ends the shared ruler: cancels the timers and, if observers were sent a route, sends one
   * `active: false` frame on the scene it was sent on. Sends nothing when no route was sent.
   * @example
   * ```ts
   * import { RulerSharer } from "./ruler-share";
   *
   * declare const sharer: RulerSharer;
   * sharer.stop();
   * ```
   */
  stop(): void {
    this.disarm();
    if (this.sentScene !== null) {
      this.deps.send(this.sentScene, [], false);
    }
    this.sentScene = null;
    this.scene = null;
    this.points = [];
  }

  /** Sends the latest route now and arms the keepalive, unless sharing has been switched off or
   * the ruler's scene is no longer viewed, in which case the ruler ends instead.
   * @example
   * ```ts
   * declare const sharer: { flush(): void };
   * sharer.flush();
   * ```
   */
  private flush(): void {
    if (this.scene === null) return;
    if (!this.deps.enabled() || this.deps.viewedScene() !== this.scene) {
      this.stop();
      return;
    }
    this.disarm();
    this.deps.send(this.scene, thinPolyline(this.points, MAX_RULER_POINTS), true);
    this.sentScene = this.scene;
    this.lastSentAt = this.deps.now();
    this.arm(RULER_KEEPALIVE_MS);
  }

  /** Replaces the pending timer with one firing `flush` after `ms`.
   * @param ms Delay until the next send.
   * @example
   * ```ts
   * declare const sharer: { arm(ms: number): void };
   * sharer.arm(200);
   * ```
   */
  private arm(ms: number): void {
    this.disarm();
    this.timer = this.deps.schedule(() => {
      this.timer = null;
      this.flush();
    }, ms);
  }

  /** Cancels the pending timer, if any.
   * @example
   * ```ts
   * declare const sharer: { disarm(): void };
   * sharer.disarm();
   * ```
   */
  private disarm(): void {
    if (this.timer !== null) {
      this.deps.cancel(this.timer);
      this.timer = null;
    }
  }
}
