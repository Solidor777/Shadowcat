import { comboKey, type ModuleContext } from "@shadowcat/core";

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

/** The `KeyboardEvent.code` the push-to-duck binding starts on until the user rebinds it. */
export const DEFAULT_KEY = "Backquote";

/** The ducking module's manifest id, the owner `ModuleContext.keybindings` stamps on its bindings. */
export const DUCKING_MODULE_ID = "ducking";

/** The push-to-duck binding's id within `DUCKING_MODULE_ID`. */
export const PUSH_TO_DUCK_BINDING_ID = "push-to-duck";

/** The slice of `ModuleContext.keybindings` a `KeySource` uses. */
export type KeySourceBindings = Pick<ModuleContext["keybindings"], "register" | "bindingsFor">;

/**
 * Held-key push-to-duck source: a keybinding press sets demand 1, releasing the bound key sets
 * demand 0. The press arrives through the keybindings framework (`core:keydown`, which skips
 * keystrokes aimed at editable elements); the release stays here because a hold gesture's
 * second edge is not a framework concept. The binding is registered once at construction so it
 * is always listed for rebinding; it only ducks while the source is started.
 */
export class KeySource {
  /** The sink demand is forwarded to; replaceable via `setSink` once a real one exists. */
  private sink: DuckSink;
  /** The keybindings the held key's current combo is read from. */
  private readonly keybindings: KeySourceBindings;
  /** Whether `start()` has attached listeners (idempotency guard). */
  private started = false;
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
    const combo = this.keybindings
      .bindingsFor(DUCKING_MODULE_ID)
      .find((b) => b.bindingId === PUSH_TO_DUCK_BINDING_ID)?.combo;
    if (combo == null || comboKey(combo) !== e.code.toLowerCase()) return;
    this.sink.set(0);
  };
  /**
   * Resets demand to 0 when the window loses focus while the bound key is held (alt-tab, OS
   * focus switch) — `window` never receives the matching `keyup` in that case, so without this
   * demand would latch at 1 forever.
   * @example
   * ```
   * // private field; not part of the public API — attached to `window` by `start()`
   * window.addEventListener("blur", this.onBlur);
   * ```
   */
  private readonly onBlur = (): void => {
    this.sink.set(0);
  };

  /**
   * Constructs a push-to-duck key source and registers its binding.
   * @param keybindings The module's `ctx.keybindings`; the binding is stamped with its module id.
   * @param sink The initial `DuckSource`-shaped sink to forward demand to; default `NULL_SINK`.
   * @example
   * ```
   * declare const keybindings: KeySourceBindings;
   * const source = new KeySource(keybindings);
   * ```
   */
  constructor(keybindings: KeySourceBindings, sink: DuckSink = NULL_SINK) {
    this.sink = sink;
    this.keybindings = keybindings;
    keybindings.register(PUSH_TO_DUCK_BINDING_ID, {
      defaultCombo: DEFAULT_KEY,
      labelKey: "ducking.keySource.binding",
      action: () => {
        if (this.started) this.sink.set(1);
      },
    });
  }

  /**
   * Attaches the `keyup`/`blur` listeners and arms the binding; a no-op if already started.
   * @example
   * ```
   * declare const keybindings: KeySourceBindings;
   * const source = new KeySource(keybindings);
   * source.start();
   * ```
   */
  start(): void {
    if (this.started) return;
    this.started = true;
    window.addEventListener("keyup", this.onKeyUp);
    window.addEventListener("blur", this.onBlur);
  }

  /**
   * Removes the listeners, disarms the binding and resets demand to 0; a no-op if not started.
   * @example
   * ```
   * declare const keybindings: KeySourceBindings;
   * const source = new KeySource(keybindings);
   * source.stop();
   * ```
   */
  stop(): void {
    if (!this.started) return;
    this.started = false;
    window.removeEventListener("keyup", this.onKeyUp);
    window.removeEventListener("blur", this.onBlur);
    this.sink.set(0);
  }

  /**
   * Replaces the sink demand is forwarded to. `DuckSourcesController.wireToAudioDuck` calls
   * this with the real `ctx.audio.duck`-backed sink once `DuckingRuntime` constructs it.
   * @param sink The replacement sink.
   * @example
   * ```
   * declare const keybindings: KeySourceBindings;
   * const source = new KeySource(keybindings);
   * source.setSink(NULL_SINK);
   * ```
   */
  setSink(sink: DuckSink): void {
    this.sink = sink;
  }
}
