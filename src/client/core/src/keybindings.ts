// Module keybindings: a combo -> action map layered over the HookBus. One window keydown listener
// emits the cancel-kind "core:keydown" hook; the controller's own listener on that hook resolves
// the live combo against every registered (possibly rebound) binding and runs the first match.
// Combos are built from `KeyboardEvent.code` (the physical key position, independent of layout,
// Shift or AltGr), so a stored binding and a live keystroke always compare as the same string.
import type { HookBus } from "./hooks";
import type { I18nParams } from "./i18n";

/** Hook name carrying every keydown outside an editable element. */
export const KEYDOWN_HOOK = "core:keydown";
/** Semver of the `core:keydown` payload contract. */
export const KEYDOWN_HOOK_VERSION = "1.0.0";

/** Payload of the `core:keydown` hook. */
export interface KeydownPayload {
  /** The canonical combo for the keystroke (see `normalizeCombo`). */
  combo: string;
  /** The native event; a listener that claims the keystroke calls `preventDefault` on it. */
  event: KeyboardEvent;
}

/** The `CoreHooks` entries the keybindings framework adds. */
export interface KeybindingHooks {
  /** Fires on every keydown whose target is not an editable element; a `false`/`STOP` return
   * claims the keystroke and halts dispatch to later listeners. */
  "core:keydown": KeydownPayload;
}

/** Combo modifier tokens in canonical order. */
const MODIFIER_ORDER = ["ctrl", "shift", "alt", "meta"] as const;
/** `KeyboardEvent.code` values (lowercased) that are modifier keys, never a combo's main key. */
const MODIFIER_CODES = new Set([
  "controlleft",
  "controlright",
  "shiftleft",
  "shiftright",
  "altleft",
  "altright",
  "metaleft",
  "metaright",
  "osleft",
  "osright",
]);

/** The event fields `normalizeCombo` reads. */
export type ComboEvent = Pick<KeyboardEvent, "code" | "ctrlKey" | "shiftKey" | "altKey" | "metaKey">;

/**
 * Whether the event is a press of a modifier key on its own (Shift, Ctrl, Alt, Meta). A
 * capture control skips these so a combo is only recorded once its main key arrives.
 * @param event The keyboard event.
 * @returns `true` for a lone modifier press.
 * @example
 * ```ts
 * import { isModifierOnly } from "@shadowcat/core";
 *
 * const alone = isModifierOnly({ code: "ShiftLeft" });
 * ```
 */
export function isModifierOnly(event: Pick<KeyboardEvent, "code">): boolean {
  return MODIFIER_CODES.has(event.code.toLowerCase());
}

/**
 * Builds the canonical combo string for a keystroke: held modifiers in fixed order, then the
 * lowercased `event.code`. Reads `code`, never `key`, so the result names the physical key and
 * does not change with layout, Shift or AltGr.
 * @param event The keyboard event (or any object with the same fields).
 * @returns A string such as `"ctrl+shift+backquote"`.
 * @example
 * ```ts
 * import { normalizeCombo } from "@shadowcat/core";
 *
 * const combo = normalizeCombo({ code: "KeyK", ctrlKey: true, shiftKey: false, altKey: false, metaKey: false });
 * ```
 */
export function normalizeCombo(event: ComboEvent): string {
  const held: Record<(typeof MODIFIER_ORDER)[number], boolean> = {
    ctrl: event.ctrlKey,
    shift: event.shiftKey,
    alt: event.altKey,
    meta: event.metaKey,
  };
  const parts: string[] = MODIFIER_ORDER.filter((m) => held[m]);
  parts.push(event.code.toLowerCase());
  return parts.join("+");
}

/**
 * Rewrites a hand-written combo into the canonical form `normalizeCombo` produces: lowercase,
 * modifiers in fixed order, main key last.
 * @param combo A combo such as `"Shift+Ctrl+KeyK"`.
 * @returns The canonical combo, e.g. `"ctrl+shift+keyk"`.
 * @example
 * ```ts
 * import { canonicalizeCombo } from "@shadowcat/core";
 *
 * const combo = canonicalizeCombo("Shift+Ctrl+KeyK");
 * ```
 */
export function canonicalizeCombo(combo: string): string {
  const tokens = combo.toLowerCase().split("+");
  const mods = MODIFIER_ORDER.filter((m) => tokens.includes(m));
  const keys = tokens.filter((tok) => !(MODIFIER_ORDER as readonly string[]).includes(tok));
  return [...mods, ...keys].join("+");
}

/**
 * The main-key token of a canonical combo (the lowercased `KeyboardEvent.code`).
 * @param combo A canonical combo.
 * @returns The final token.
 * @example
 * ```ts
 * import { comboKey } from "@shadowcat/core";
 *
 * const key = comboKey("ctrl+backquote");
 * ```
 */
export function comboKey(combo: string): string {
  const tokens = combo.split("+");
  return tokens[tokens.length - 1];
}

/** The translation function `describeCombo` resolves display text through. */
export type ComboTranslate = (key: string, params?: I18nParams) => string;

/**
 * Turns a canonical combo into display text. Every modifier and key name resolves through an
 * i18n key (`keybindings.mod.<name>`, `keybindings.key.<code>`) and the parts join with
 * `keybindings.comboSeparator`. A key with no catalog entry falls back to its code with the
 * `key`/`digit` prefix stripped (`keyv` -> `V`, `digit1` -> `1`, `backquote` -> `Backquote`).
 * @param combo A canonical combo.
 * @param t The translation function.
 * @returns The localized label.
 * @example
 * ```ts
 * import { describeCombo } from "@shadowcat/core";
 *
 * const label = describeCombo("ctrl+keyk", (key) => key);
 * ```
 */
export function describeCombo(combo: string, t: ComboTranslate): string {
  const tokens = combo.split("+");
  const key = tokens[tokens.length - 1];
  const parts = tokens.slice(0, -1).map((mod) => t(`keybindings.mod.${mod}`));
  const keyKey = `keybindings.key.${key}`;
  const translated = t(keyKey);
  parts.push(translated === keyKey ? fallbackKeyName(key) : translated);
  return parts.join(t("keybindings.comboSeparator"));
}

/**
 * A readable name for a key code with no catalog entry.
 * @param code The lowercased `KeyboardEvent.code`.
 * @returns The code with a `key`/`digit` prefix stripped and the first letter capitalized.
 * @example
 * ```
 * // private helper; not part of the public API
 * fallbackKeyName("keyv");
 * ```
 */
function fallbackKeyName(code: string): string {
  const single = /^(?:key|digit)(.)$/.exec(code);
  const name = single ? single[1] : code;
  return name.charAt(0).toUpperCase() + name.slice(1);
}

/**
 * Whether `target` is a text input, textarea, or `contenteditable` element — keystrokes aimed
 * there are typing, never a binding.
 * @param target The event's `target`.
 * @returns `true` when typing should suppress keybindings.
 * @example
 * ```ts
 * import { isEditableTarget } from "@shadowcat/core";
 *
 * const typing = isEditableTarget(document.activeElement);
 * ```
 */
export function isEditableTarget(target: EventTarget | null): boolean {
  if (typeof HTMLElement === "undefined" || !(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName;
  return tag === "INPUT" || tag === "TEXTAREA";
}

/** Options for registering a binding. */
export interface KeybindingOptions {
  /** The combo the binding starts on, in any case/modifier order (canonicalized on register). */
  defaultCombo: string;
  /** i18n key of the binding's display name. */
  labelKey: string;
  /** Runs when the binding's current combo is pressed. */
  action: () => void;
}

/** What `register` returns. */
export interface KeybindingRegistration {
  /** Removes the binding. */
  dispose: () => void;
  /** The module that already holds this binding's combo, or `null` when nothing collides. The
   * binding still registers; only the earlier-registered one fires for the shared combo. */
  conflictsWith: string | null;
}

/** A registered binding as listed to the rebind UI. */
export interface BindingInfo {
  /** The registering module's id. */
  moduleId: string;
  /** The binding's id, unique within its module. */
  bindingId: string;
  /** i18n key of the display name. */
  labelKey: string;
  /** The combo in force: the user's override, else the default; `null` when unbound. */
  combo: string | null;
  /** The combo the binding registered with. */
  defaultCombo: string;
}

/** User overrides keyed by `"<moduleId>:<bindingId>"`; `null` is an explicit unbind. */
export type KeybindingOverrides = Record<string, string | null>;

/** Internal record of one registered binding. */
interface Registered {
  /** The registering module's id. */
  moduleId: string;
  /** The binding's id within the module. */
  bindingId: string;
  /** The registration options with the default combo canonicalized. */
  opts: KeybindingOptions;
  /** Registration order; the lower value wins a shared combo. */
  seq: number;
}

/**
 * The qualified override key of a binding.
 * @param moduleId The registering module's id.
 * @param bindingId The binding's id.
 * @returns `"<moduleId>:<bindingId>"`.
 * @example
 * ```
 * // private helper; not part of the public API
 * qualify("ducking", "push-to-talk");
 * ```
 */
function qualify(moduleId: string, bindingId: string): string {
  return `${moduleId}:${bindingId}`;
}

/**
 * Owns every registered keybinding, the user's overrides, and the window keydown listener that
 * feeds the `core:keydown` hook. A binding match runs `preventDefault`, the action, and returns
 * `false` so no later listener (and no second binding on the same combo) also fires. Keystrokes
 * aimed at editable elements and auto-repeats never reach the hook.
 * @example
 * ```ts
 * import { HookBus, KeybindingsController, silentLogger } from "@shadowcat/core";
 *
 * const controller = new KeybindingsController(new HookBus(silentLogger));
 * controller.register("example", "ping", { defaultCombo: "KeyP", labelKey: "example.ping", action: () => {} });
 * controller.dispose();
 * ```
 */
export class KeybindingsController {
  /** Registered bindings by qualified key. */
  #bindings = new Map<string, Registered>();
  /** User overrides by qualified key. */
  #overrides = new Map<string, string | null>();
  /** Change listeners. */
  #listeners = new Set<() => void>();
  /** Monotonic registration counter. */
  #seq = 0;
  /** Where the keydown listener is installed, or `undefined` outside a browser. */
  #target: EventTarget | undefined;
  /**
   * The installed keydown listener: skips auto-repeat and editable targets, then emits the cancel
   * hook with the normalized combo.
   * @param event The window keydown event.
   * @example
   * ```
   * // private field; installed on the target by the constructor
   * declare const target: EventTarget;
   * target.addEventListener("keydown", this.#onKeydown);
   * ```
   */
  #onKeydown = (event: Event): void => {
    const e = event as KeyboardEvent;
    if (e.repeat || isEditableTarget(e.target)) return;
    void this.#hooks.emitCancel(KEYDOWN_HOOK, { combo: normalizeCombo(e), event: e } satisfies KeydownPayload);
  };
  /** The bus `core:keydown` is defined and dispatched on. */
  #hooks: HookBus;

  /**
   * Defines `core:keydown`, listens on it, and installs the window keydown listener.
   * @param hooks The bus the hook lives on.
   * @param target Where to listen for keydown; defaults to `window` and installs nothing when no
   * window exists.
   * @example
   * ```ts
   * import { HookBus, KeybindingsController, silentLogger } from "@shadowcat/core";
   *
   * const controller = new KeybindingsController(new HookBus(silentLogger));
   * controller.dispose();
   * ```
   */
  constructor(hooks: HookBus, target: EventTarget | undefined = typeof window === "undefined" ? undefined : window) {
    this.#hooks = hooks;
    hooks.defineHook(KEYDOWN_HOOK, { version: KEYDOWN_HOOK_VERSION, kind: "cancel" });
    hooks.on(KEYDOWN_HOOK, (payload) => this.#dispatch(payload as KeydownPayload), { module: "core:keybindings" });
    this.#target = target;
    target?.addEventListener("keydown", this.#onKeydown);
  }

  /**
   * Removes the window listener. Registrations stay; the controller stops receiving keys.
   * @example
   * ```ts
   * import { HookBus, KeybindingsController, silentLogger } from "@shadowcat/core";
   *
   * const controller = new KeybindingsController(new HookBus(silentLogger));
   * controller.dispose();
   * ```
   */
  dispose(): void {
    this.#target?.removeEventListener("keydown", this.#onKeydown);
  }

  /**
   * Runs the first-registered binding whose current combo equals the keystroke's.
   * @param payload The `core:keydown` payload.
   * @returns `false` when a binding claimed the keystroke (halting dispatch), else `undefined`.
   * @example
   * ```
   * // private; registered as the controller's own `core:keydown` listener
   * declare const event: KeyboardEvent;
   * this.#dispatch({ combo: "keyp", event });
   * ```
   */
  #dispatch(payload: KeydownPayload): false | undefined {
    let winner: Registered | undefined;
    for (const b of this.#bindings.values()) {
      if (this.#comboOf(b) !== payload.combo) continue;
      if (winner === undefined || b.seq < winner.seq) winner = b;
    }
    if (winner === undefined) return undefined;
    payload.event.preventDefault();
    winner.opts.action();
    return false;
  }

  /**
   * The combo currently in force for a binding.
   * @param b The binding.
   * @returns The override when one exists (`null` for unbound), else the default.
   * @example
   * ```
   * // private helper; not part of the public API
   * declare const binding: Registered;
   * this.#comboOf(binding);
   * ```
   */
  #comboOf(b: Registered): string | null {
    const key = qualify(b.moduleId, b.bindingId);
    return this.#overrides.has(key) ? (this.#overrides.get(key) ?? null) : b.opts.defaultCombo;
  }

  /**
   * Notifies change listeners.
   * @example
   * ```
   * // private helper; not part of the public API
   * this.#emit();
   * ```
   */
  #emit(): void {
    for (const l of [...this.#listeners]) l();
  }

  /**
   * Registers a binding. A second registration under the same module and binding id replaces
   * the first.
   * @param moduleId The registering module's id.
   * @param bindingId The binding's id, unique within the module.
   * @param opts The default combo, label key and action.
   * @returns A dispose function and the module (if any) already holding the binding's combo.
   * @example
   * ```ts
   * import { HookBus, KeybindingsController, silentLogger } from "@shadowcat/core";
   *
   * const controller = new KeybindingsController(new HookBus(silentLogger));
   * const { dispose, conflictsWith } = controller.register("example", "ping", {
   *   defaultCombo: "KeyP",
   *   labelKey: "example.ping",
   *   action: () => {},
   * });
   * dispose();
   * ```
   */
  register(moduleId: string, bindingId: string, opts: KeybindingOptions): KeybindingRegistration {
    const key = qualify(moduleId, bindingId);
    const entry: Registered = {
      moduleId,
      bindingId,
      opts: { ...opts, defaultCombo: canonicalizeCombo(opts.defaultCombo) },
      seq: this.#seq++,
    };
    const combo = this.#comboOf(entry);
    let conflictsWith: string | null = null;
    if (combo !== null) {
      for (const other of this.#bindings.values()) {
        if (qualify(other.moduleId, other.bindingId) !== key && this.#comboOf(other) === combo) {
          conflictsWith = other.moduleId;
          break;
        }
      }
    }
    this.#bindings.set(key, entry);
    this.#emit();
    return {
      conflictsWith,
      dispose: () => {
        if (this.#bindings.get(key) !== entry) return;
        this.#bindings.delete(key);
        this.#emit();
      },
    };
  }

  /**
   * Removes every binding a module registered (module unload).
   * @param moduleId The module whose bindings to drop.
   * @example
   * ```ts
   * import { HookBus, KeybindingsController, silentLogger } from "@shadowcat/core";
   *
   * const controller = new KeybindingsController(new HookBus(silentLogger));
   * controller.removeModule("example");
   * ```
   */
  removeModule(moduleId: string): void {
    let removed = false;
    for (const [key, b] of this.#bindings) {
      if (b.moduleId === moduleId) {
        this.#bindings.delete(key);
        removed = true;
      }
    }
    if (removed) this.#emit();
  }

  /**
   * Sets a binding's combo. The override is kept even when the binding is not registered (yet),
   * so a saved choice applies when its module loads.
   * @param moduleId The binding's module.
   * @param bindingId The binding's id.
   * @param combo The new combo, or `null` to unbind. A combo equal to the default clears the
   * override.
   * @example
   * ```ts
   * import { HookBus, KeybindingsController, silentLogger } from "@shadowcat/core";
   *
   * const controller = new KeybindingsController(new HookBus(silentLogger));
   * controller.rebind("example", "ping", "ctrl+keyp");
   * ```
   */
  rebind(moduleId: string, bindingId: string, combo: string | null): void {
    const key = qualify(moduleId, bindingId);
    const next = combo === null ? null : canonicalizeCombo(combo);
    const registered = this.#bindings.get(key);
    if (registered !== undefined && next === registered.opts.defaultCombo) this.#overrides.delete(key);
    else this.#overrides.set(key, next);
    this.#emit();
  }

  /**
   * Every binding one module registered.
   * @param moduleId The module's id.
   * @returns The module's bindings in registration order.
   * @example
   * ```ts
   * import { HookBus, KeybindingsController, silentLogger } from "@shadowcat/core";
   *
   * const controller = new KeybindingsController(new HookBus(silentLogger));
   * const mine = controller.bindingsFor("example");
   * ```
   */
  bindingsFor(moduleId: string): BindingInfo[] {
    return this.allBindings().filter((b) => b.moduleId === moduleId);
  }

  /**
   * Every registered binding, for the rebind UI.
   * @returns All bindings in registration order.
   * @example
   * ```ts
   * import { HookBus, KeybindingsController, silentLogger } from "@shadowcat/core";
   *
   * const controller = new KeybindingsController(new HookBus(silentLogger));
   * const all = controller.allBindings();
   * ```
   */
  allBindings(): BindingInfo[] {
    return [...this.#bindings.values()]
      .sort((a, b) => a.seq - b.seq)
      .map((b) => ({
        moduleId: b.moduleId,
        bindingId: b.bindingId,
        labelKey: b.opts.labelKey,
        combo: this.#comboOf(b),
        defaultCombo: b.opts.defaultCombo,
      }));
  }

  /**
   * Replaces every override with a saved set (the account's persisted choices).
   * @param overrides The saved overrides.
   * @example
   * ```ts
   * import { HookBus, KeybindingsController, silentLogger } from "@shadowcat/core";
   *
   * const controller = new KeybindingsController(new HookBus(silentLogger));
   * controller.loadOverrides({ "example:ping": "ctrl+keyp" });
   * ```
   */
  loadOverrides(overrides: KeybindingOverrides): void {
    this.#overrides = new Map(
      Object.entries(overrides).map(([key, combo]) => [key, combo === null ? null : canonicalizeCombo(combo)]),
    );
    this.#emit();
  }

  /**
   * The current overrides, for persisting.
   * @returns A plain record keyed `"<moduleId>:<bindingId>"`.
   * @example
   * ```ts
   * import { HookBus, KeybindingsController, silentLogger } from "@shadowcat/core";
   *
   * const controller = new KeybindingsController(new HookBus(silentLogger));
   * const saved = controller.serializeOverrides();
   * ```
   */
  serializeOverrides(): KeybindingOverrides {
    return Object.fromEntries(this.#overrides);
  }

  /**
   * Subscribes to any change: a registration, removal, rebind or override load.
   * @param listener Called after each change.
   * @returns An unsubscribe function.
   * @example
   * ```ts
   * import { HookBus, KeybindingsController, silentLogger } from "@shadowcat/core";
   *
   * const controller = new KeybindingsController(new HookBus(silentLogger));
   * const off = controller.subscribe(() => {});
   * off();
   * ```
   */
  subscribe(listener: () => void): () => void {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }
}
