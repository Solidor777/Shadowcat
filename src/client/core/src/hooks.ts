// Versioned hook bus with an open, namespaced string keyspace ("ns:event").
// The keyspace is open at runtime because modules define hooks the core cannot
// know at compile time; a typed overlay (CoreHooks) layers compile-time safety
// over statically-known hook names. Three dispatch kinds, each a distinct
// contract: informational (await all, results ignored), mutating (chained
// transform), cancellable (halts on false/STOP). A throwing handler is isolated
// and logged — one faulty module cannot abort dispatch or corrupt a pipeline.
import type { Logger } from "./logger";
import { satisfies } from "./semver";

/** The three dispatch contracts a hook name declares one of: `"info"` awaits every listener and
 * discards return values; `"mutate"` threads a payload through listeners as a chained transform;
 * `"cancel"` halts remaining dispatch when a listener returns `false`/`STOP`. */
export type HookKind = "info" | "mutate" | "cancel";
/** A hook name's declared contract, recorded once via `HookBus.defineHook`. */
export interface HookDefinition {
  /** Semver version of the hook's payload contract; a mismatched re-declaration throws. */
  version: string;
  /** Which of the three dispatch kinds this hook name uses. */
  kind: HookKind;
}
/** Options for `HookBus.on`. */
export interface OnOptions {
  /** The registering module's id, recorded so `removeModule` can find and drop this listener. */
  module?: string;
  /** Dispatch order among listeners on the same hook; higher runs first. Defaults to 0. */
  priority?: number;
  /** A semver range the hook's currently-declared version must satisfy, checked once at
   * registration time (never re-checked on a later version bump). */
  requires?: string;
}
/** Sentinel a `"cancel"`-kind handler returns to halt dispatch, distinct from `false` so a
 * handler can cancel without its return value being misread as an ordinary falsy payload. */
export const STOP: unique symbol = Symbol("hook.stop");
/** A hook listener. Its return value is ignored for `"info"`, becomes the next listener's input
 * for `"mutate"`, and is checked against `false`/`STOP` for `"cancel"`. */
export type Handler<P> = (payload: P) => unknown | Promise<unknown>;

/** Declaration-merge `name -> payload` here to type a first-party hook. */
// eslint-disable-next-line @typescript-eslint/no-empty-object-type
export interface CoreHooks {}

/** A registered listener, as stored internally per hook name (not exported). */
interface Listener {
  /** The registered callback. */
  handler: Handler<unknown>;
  /** The registering module's id, if any (set from `OnOptions.module`). */
  module?: string;
  /** Dispatch order among listeners on the same hook; higher runs first. */
  priority: number;
  /** Registration-order tiebreaker among listeners sharing a `priority` (lower registered first). */
  seq: number;
}

/** The versioned hook bus. Handles three dispatch kinds — informational (await
 * all, results ignored), mutating (chained transform), cancellable (halts on
 * `false`/`STOP`) — over an open, namespaced string keyspace (`"ns:event"`). A
 * throwing handler is caught and logged; it never aborts dispatch to the
 * remaining listeners or corrupts a mutate chain (the prior payload carries
 * forward). */
export class HookBus {
  /** Every declared hook name's contract, set via `defineHook`. */
  private defs = new Map<string, HookDefinition>();
  /** Every hook name's listeners, already priority/registration sorted by `on`. Populated for a
   * name as soon as `defineHook` runs, even before any listener registers. */
  private listeners = new Map<string, Listener[]>();
  /** Monotonic counter stamped onto each `Listener.seq` at registration time. */
  private seqCounter = 0;

  /** Builds a hook bus that logs handler failures and undefined-hook emits to `logger`.
   * @param logger Diagnostic sink for a throwing handler or an emit on an undefined/
   * wrong-kind hook.
   * @example
   * ```ts
   * import { HookBus, silentLogger } from "@shadowcat/core";
   *
   * const hooks = new HookBus(silentLogger);
   * ```
   */
  constructor(private readonly logger: Logger) {}

  /** Declares a hook name's kind and version. Re-declaring an existing name only
   * throws if `def.version` differs from the version already on record — a
   * version bump would silently reinterpret every existing listener's payload
   * contract. Re-declaring with the SAME version overwrites the stored
   * definition unconditionally, including a changed `kind`; this function does
   * not itself guard against that.
   * @param name The hook's namespaced name (`"ns:event"`).
   * @param def The hook's kind and version.
   * @example
   * ```ts
   * import { HookBus, silentLogger } from "@shadowcat/core";
   *
   * const hooks = new HookBus(silentLogger);
   * hooks.defineHook("example:event", { version: "1.0.0", kind: "info" });
   * ```
   */
  defineHook(name: string, def: HookDefinition): void {
    const existing = this.defs.get(name);
    if (existing && existing.version !== def.version) {
      throw new Error(
        `hook ${name} already defined at ${existing.version}; cannot redefine at ${def.version}`,
      );
    }
    this.defs.set(name, def);
    if (!this.listeners.has(name)) this.listeners.set(name, []);
  }

  /** Registers a listener for `name`. Listeners fire highest-`priority` first;
   * ties keep registration order. `opts.requires` checks the currently declared
   * hook version (`satisfies`) at registration time and throws if unmet — it does
   * NOT re-check on a later `defineHook` version bump.
   * @param name The hook's namespaced name.
   * @param handler Called with the hook's payload on every emit.
   * @param opts Listener options.
   * @param opts.module The registering module's id, recorded so `removeModule`
   * can find it.
   * @param opts.priority Dispatch order among listeners on the same hook; higher
   * runs first. Defaults to 0.
   * @param opts.requires A semver range the hook's currently-declared version
   * must satisfy, checked once at registration time.
   * @returns An unsubscribe function.
   * @example
   * ```ts
   * import { HookBus, silentLogger } from "@shadowcat/core";
   *
   * const hooks = new HookBus(silentLogger);
   * hooks.defineHook("example:event", { version: "1.0.0", kind: "info" });
   * const unsubscribe = hooks.on("example:event", () => {}, { priority: 10 });
   * unsubscribe();
   * ```
   */
  on(name: string, handler: Handler<unknown>, opts: OnOptions = {}): () => void {
    const def = this.defs.get(name);
    if (def && opts.requires && !satisfies(def.version, opts.requires)) {
      throw new Error(
        `hook ${name} is ${def.version}; listener requires ${opts.requires}`,
      );
    }
    const entry: Listener = {
      handler,
      module: opts.module,
      priority: opts.priority ?? 0,
      seq: this.seqCounter++,
    };
    const arr = this.listeners.get(name) ?? [];
    arr.push(entry);
    // Higher priority first; ties keep registration order.
    arr.sort((a, b) => b.priority - a.priority || a.seq - b.seq);
    this.listeners.set(name, arr);
    return () => {
      const cur = this.listeners.get(name);
      if (cur) this.listeners.set(name, cur.filter((l) => l !== entry));
    };
  }

  /** `name`'s listeners in dispatch order (already priority/registration
   * sorted by `on`). Not exported — folded into the `emit*` methods' public
   * surface.
   * @param name The hook name to look up.
   * @returns The registered listeners for `name`, or `[]` if none.
   * @example
   * ```
   * // internal helper; not part of the public API
   * this.ordered("example:event");
   * ```
   */
  private ordered(name: string): Listener[] {
    return this.listeners.get(name) ?? [];
  }

  /** Guards an `emit*` call: logs and returns `false` if `name` is undefined or
   * declared as a different `HookKind` than `kind`. Not exported — folded into
   * the `emit*` methods' public surface.
   * @param name The hook name being emitted.
   * @param kind The dispatch kind the caller is about to perform.
   * @returns `true` if `name` is declared as `kind`.
   * @example
   * ```
   * // internal helper; not part of the public API
   * this.expectKind("example:event", "info");
   * ```
   */
  private expectKind(name: string, kind: HookKind): boolean {
    const def = this.defs.get(name);
    if (!def) {
      this.logger.warn(`emit on undefined hook ${name}`);
      return false;
    }
    if (def.kind !== kind) {
      this.logger.error(`hook ${name} is ${def.kind}; emitted as ${kind}`);
      return false;
    }
    return true;
  }

  /** Emits an informational hook with a statically-known payload type, when
   * `name` is one of the keys declared on `CoreHooks` via declaration-merging
   * elsewhere in the app.
   * @param name A key declared on `CoreHooks`.
   * @param payload The payload type declared for `name` on `CoreHooks`.
   * @returns Resolves once every listener has settled.
   * @example
   * ```ts
   * import { HookBus, silentLogger } from "@shadowcat/core";
   *
   * const hooks = new HookBus(silentLogger);
   * await hooks.emitInfo("example:info", { value: 1 });
   * ```
   */
  async emitInfo<K extends keyof CoreHooks>(name: K, payload: CoreHooks[K]): Promise<void>;
  /** Emits an informational hook by an open (not statically declared) name.
   * @param name The hook's namespaced name.
   * @param payload The payload to pass to every listener.
   * @returns Resolves once every listener has settled.
   * @example
   * ```ts
   * import { HookBus, silentLogger } from "@shadowcat/core";
   *
   * const hooks = new HookBus(silentLogger);
   * await hooks.emitInfo("module:custom-event", { value: 1 });
   * ```
   */
  async emitInfo(name: string, payload: unknown): Promise<void>;
  /** Implementation shared by both `emitInfo` overloads above: awaits every
   * listener on `name` in dispatch order, ignoring return values. A throwing
   * listener is caught, logged, and does not stop dispatch to the rest. A no-op
   * (logged, not thrown) if `name` is undefined or declared as a non-`"info"`
   * kind.
   * @param name The hook name.
   * @param payload The payload to pass to every listener.
   * @returns Resolves once every listener has settled, or immediately on a
   * failed `expectKind` guard.
   * @example
   * ```
   * // implementation signature; call emitInfo via one of its declared overloads
   * ```
   */
  async emitInfo(name: string, payload: unknown): Promise<void> {
    if (!this.expectKind(name, "info")) return;
    for (const l of this.ordered(name)) {
      try {
        await l.handler(payload);
      } catch (err) {
        this.logger.error(`hook ${name} handler threw`, err);
      }
    }
  }

  /** Emits a mutating hook with a statically-known payload type, when `name` is
   * one of the keys declared on `CoreHooks` via declaration-merging elsewhere in
   * the app.
   * @param name A key declared on `CoreHooks`.
   * @param payload The initial payload, declared for `name` on `CoreHooks`.
   * @returns The payload after every listener has had a chance to transform it
   * in dispatch order.
   * @example
   * ```ts
   * import { HookBus, silentLogger } from "@shadowcat/core";
   *
   * const hooks = new HookBus(silentLogger);
   * const result = await hooks.emitMutate("example:mutate", { value: 1 });
   * ```
   */
  async emitMutate<K extends keyof CoreHooks>(name: K, payload: CoreHooks[K]): Promise<CoreHooks[K]>;
  /** Emits a mutating hook by an open (not statically declared) name.
   * @param name The hook's namespaced name.
   * @param payload The initial payload.
   * @returns The payload after every listener has had a chance to transform it
   * in dispatch order.
   * @example
   * ```ts
   * import { HookBus, silentLogger } from "@shadowcat/core";
   *
   * const hooks = new HookBus(silentLogger);
   * const result = await hooks.emitMutate("module:custom-mutate", { value: 1 });
   * ```
   */
  async emitMutate<P>(name: string, payload: P): Promise<P>;
  /** Implementation shared by both `emitMutate` overloads above: threads the
   * payload through every listener on `name` in dispatch order, each listener's
   * return value becoming the next listener's input. A throwing listener is
   * caught, logged, and carries the PRIOR payload forward unchanged rather than
   * stopping the chain. A no-op returning `payload` unchanged (logged, not
   * thrown) if `name` is undefined or declared as a non-`"mutate"` kind.
   * @param name The hook name.
   * @param payload The initial payload.
   * @returns The final payload after every listener has run (or the original
   * payload, on a failed `expectKind` guard).
   * @example
   * ```
   * // implementation signature; call emitMutate via one of its declared overloads
   * ```
   */
  async emitMutate<P>(name: string, payload: P): Promise<P> {
    if (!this.expectKind(name, "mutate")) return payload;
    let cur = payload;
    for (const l of this.ordered(name)) {
      try {
        cur = (await l.handler(cur)) as P;
      } catch (err) {
        this.logger.error(`hook ${name} handler threw; carrying prior payload`, err);
      }
    }
    return cur;
  }

  /** Emits a cancellable hook: calls every listener on `name` in dispatch order
   * until one returns `false` or the `STOP` symbol, at which point dispatch
   * halts immediately and the remaining listeners are never called. A throwing
   * listener is caught, logged, and does not itself cancel dispatch. A no-op
   * returning `{ cancelled: false }` (logged, not thrown) if `name` is undefined
   * or declared as a non-`"cancel"` kind.
   * @param name The hook's namespaced name.
   * @param payload The payload passed to every listener.
   * @returns `{ cancelled: true, by }` naming the listener's `module` (if any)
   * that halted dispatch, or `{ cancelled: false }` if every listener ran to
   * completion.
   * @example
   * ```ts
   * import { HookBus, silentLogger } from "@shadowcat/core";
   *
   * const hooks = new HookBus(silentLogger);
   * hooks.defineHook("example:cancel", { version: "1.0.0", kind: "cancel" });
   * const result = await hooks.emitCancel("example:cancel", { value: 1 });
   * ```
   */
  async emitCancel(
    name: string,
    payload: unknown,
  ): Promise<{
    /** Whether a listener returned `false`/`STOP` and halted dispatch. */
    cancelled: boolean;
    /** The halting listener's `module`, if it registered one. */
    by?: string;
  }> {
    if (!this.expectKind(name, "cancel")) return { cancelled: false };
    for (const l of this.ordered(name)) {
      try {
        const r = await l.handler(payload);
        if (r === false || r === STOP) {
          return { cancelled: true, by: l.module };
        }
      } catch (err) {
        this.logger.error(`hook ${name} handler threw`, err);
      }
    }
    return { cancelled: false };
  }

  /** Drop every listener tagged with `moduleId` (module unload teardown), on
   * every hook name.
   * @param moduleId The module id whose listeners should be removed.
   * @example
   * ```ts
   * import { HookBus, silentLogger } from "@shadowcat/core";
   *
   * const hooks = new HookBus(silentLogger);
   * hooks.removeModule("example-module");
   * ```
   */
  removeModule(moduleId: string): void {
    for (const [name, arr] of this.listeners) {
      this.listeners.set(name, arr.filter((l) => l.module !== moduleId));
    }
  }
}
