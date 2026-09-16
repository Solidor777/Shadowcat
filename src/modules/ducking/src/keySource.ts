/** A ducking source's sink — the shape `AppContext.audio.duck.addSource(id)` returns
 * (`DuckSource` in `@shadowcat/core`'s not-yet-merged `audio.ts`, owned by the audio engine's
 * `DuckController`). Declared locally so this module's sources compile and test standalone
 * before that package exists in this worktree; the real integration passes the REAL
 * `DuckSource` here structurally — no cast needed, since the shapes match exactly. */
export interface DuckSink {
  /**
   * Sets this source's demand.
   * @param level Demand level 0..=1 (1 = fully ducked).
   */
  set(level: number): void;
}

/** A `DuckSink` that discards every demand change — the default sink for a source constructed
 * before its real `ctx.audio.duck` handle is wired in. */
export const NULL_SINK: DuckSink = { set() {} };

/** Default key `KeySource` binds to when the ducking module's settings have never chosen
 * one. */
export const DEFAULT_KEY = "Backquote";

/**
 * Held-key push-to-duck source: `keydown` sets demand 1, `keyup` sets demand 0. Ignores
 * events whose target is an editable element (a text input, textarea, or
 * `contenteditable`), so typing the bound key into chat does not duck.
 */
export class KeySource {
  /** The sink demand is forwarded to; replaceable via `setSink` once a real one exists. */
  private sink: DuckSink;
  /** The bound key's `KeyboardEvent.code`. */
  private key: string;
  /** Whether `start()` has attached listeners (idempotency guard). */
  private started = false;
  /**
   * Bound `keydown` handler, captured so `stop()` removes exactly what `start()` added.
   * @param e The DOM keydown event.
   * @example
   * ```
   * // private field; not part of the public API — attached to `window` by `start()`
   * window.addEventListener("keydown", this.onKeyDown);
   * ```
   */
  private readonly onKeyDown = (e: KeyboardEvent): void => {
    if (e.code !== this.key || isEditableTarget(e.target)) return;
    this.sink.set(1);
  };
  /**
   * Bound `keyup` handler.
   * @param e The DOM keyup event.
   * @example
   * ```
   * // private field; not part of the public API — attached to `window` by `start()`
   * window.addEventListener("keyup", this.onKeyUp);
   * ```
   */
  private readonly onKeyUp = (e: KeyboardEvent): void => {
    if (e.code !== this.key) return;
    this.sink.set(0);
  };

  /**
   * Constructs a push-to-duck key source.
   * @param sink The initial `DuckSource`-shaped sink to forward demand to; default `NULL_SINK`.
   * @param key The initial bound key's `KeyboardEvent.code`; default `DEFAULT_KEY`.
   * @example
   * ```
   * const source = new KeySource(NULL_SINK, "Backquote");
   * ```
   */
  constructor(sink: DuckSink = NULL_SINK, key: string = DEFAULT_KEY) {
    this.sink = sink;
    this.key = key;
  }

  /**
   * Attaches the `keydown`/`keyup` listeners; a no-op if already started.
   * @example
   * ```
   * const source = new KeySource();
   * source.start();
   * ```
   */
  start(): void {
    if (this.started) return;
    this.started = true;
    window.addEventListener("keydown", this.onKeyDown);
    window.addEventListener("keyup", this.onKeyUp);
  }

  /**
   * Removes the listeners and resets demand to 0; a no-op if not started.
   * @example
   * ```
   * const source = new KeySource();
   * source.stop();
   * ```
   */
  stop(): void {
    if (!this.started) return;
    this.started = false;
    window.removeEventListener("keydown", this.onKeyDown);
    window.removeEventListener("keyup", this.onKeyUp);
    this.sink.set(0);
  }

  /**
   * Replaces the sink demand is forwarded to (the integration task wires the real one).
   * @param sink The replacement sink.
   * @example
   * ```
   * const source = new KeySource();
   * source.setSink(NULL_SINK);
   * ```
   */
  setSink(sink: DuckSink): void {
    this.sink = sink;
  }

  /**
   * Rebinds the held key going forward (does not affect an already-held keypress).
   * @param key The replacement `KeyboardEvent.code`.
   * @example
   * ```
   * const source = new KeySource();
   * source.setKey("KeyV");
   * ```
   */
  setKey(key: string): void {
    this.key = key;
  }
}

/**
 * Whether `target` is a text input, textarea, or `contenteditable` element — the held key
 * must not duck while the user is typing it into chat.
 * @param target The event's `target`, typed loosely per `KeyboardEvent`'s DOM shape.
 * @returns Whether typing should suppress the push-to-duck binding.
 * @example
 * ```
 * const suppress = isEditableTarget(document.activeElement);
 * ```
 */
function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName;
  return tag === "INPUT" || tag === "TEXTAREA";
}
