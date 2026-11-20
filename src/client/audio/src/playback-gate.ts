import type { MediaElementLike } from "./context";

/** Whether a rejected `play()` is the browser's autoplay policy refusing playback without a
 * user gesture (the DOM `NotAllowedError`). Other rejections do not mean a gesture would help:
 * `AbortError` is a `pause()`/reload racing the request (the next tick retries while the
 * element is still paused) and `NotSupportedError` is an unplayable source.
 * @param error The rejection reason.
 * @returns `true` for the autoplay refusal.
 * @example
 * ```
 * // private helper; exercised through `playback-gate.test.ts`
 * ```
 */
function isGestureRefusal(error: unknown): boolean {
  return typeof error === "object" && error !== null && "name" in error && error.name === "NotAllowedError";
}

/** The single place a media element is started: `play()` is called only on an element that is
 * paused, so a periodic re-sync of a playing element never restarts it, and its rejection is
 * always handled. An autoplay refusal (`isGestureRefusal`) marks the element blocked — it is
 * not retried on later ticks, because every retry is another refused request — and reports
 * through the callback so the engine can ask the user for a gesture; `release` clears the marks
 * once a gesture arrives. */
export class PlaybackGate {
  /** Elements whose last `play()` was refused for want of a gesture. */
  #blocked = new WeakSet<MediaElementLike>();
  /** Reports an autoplay refusal (the engine's `needsGesture` setter). */
  #onBlocked: () => void;

  /** Construct a gate.
   * @param onBlocked Called each time a `play()` is refused for want of a user gesture.
   * @example
   * ```ts
   * const gate = new PlaybackGate(() => {});
   * gate.release();
   * ```
   */
  constructor(onBlocked: () => void) {
    this.#onBlocked = onBlocked;
  }

  /** Start `el` if it is paused and not blocked; otherwise do nothing.
   * @param el The element to start.
   * @example
   * ```
   * // exercised through `playback-gate.test.ts`
   * ```
   */
  play(el: MediaElementLike): void {
    if (!el.paused || this.#blocked.has(el)) return;
    el.play().catch((error: unknown) => {
      if (!isGestureRefusal(error)) return;
      this.#blocked.add(el);
      this.#onBlocked();
    });
  }

  /** Forget every refusal: called from a user gesture so the next sync starts the elements.
   * @example
   * ```
   * // exercised through the `PlaybackGate` describe block and the
   * // `AudioEngine interruption and suspension recovery` gesture-retry case
   * ```
   */
  release(): void {
    this.#blocked = new WeakSet();
  }
}
