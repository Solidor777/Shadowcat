// Versioned hook bus with an open, namespaced string keyspace ("ns:event").
// The keyspace is open at runtime because modules define hooks the core cannot
// know at compile time; a typed overlay (CoreHooks) layers compile-time safety
// over statically-known hook names. Three dispatch kinds, each a distinct
// contract: informational (await all, results ignored), mutating (chained
// transform), cancellable (halts on false/STOP). A throwing handler is isolated
// and logged — one faulty module cannot abort dispatch or corrupt a pipeline.
import type { Logger } from "./logger";
import { satisfies } from "./semver";

export type HookKind = "info" | "mutate" | "cancel";
export interface HookDefinition {
  version: string;
  kind: HookKind;
}
export interface OnOptions {
  module?: string;
  priority?: number;
  requires?: string;
}
export const STOP: unique symbol = Symbol("hook.stop");
export type Handler<P> = (payload: P) => unknown | Promise<unknown>;

/** Declaration-merge `name -> payload` here to type a first-party hook. */
// eslint-disable-next-line @typescript-eslint/no-empty-object-type
export interface CoreHooks {}

interface Listener {
  handler: Handler<unknown>;
  module?: string;
  priority: number;
  seq: number;
}

export class HookBus {
  private defs = new Map<string, HookDefinition>();
  private listeners = new Map<string, Listener[]>();
  private seqCounter = 0;

  constructor(private readonly logger: Logger) {}

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

  private ordered(name: string): Listener[] {
    return this.listeners.get(name) ?? [];
  }

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

  async emitInfo<K extends keyof CoreHooks>(name: K, payload: CoreHooks[K]): Promise<void>;
  async emitInfo(name: string, payload: unknown): Promise<void>;
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

  async emitMutate<K extends keyof CoreHooks>(name: K, payload: CoreHooks[K]): Promise<CoreHooks[K]>;
  async emitMutate<P>(name: string, payload: P): Promise<P>;
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

  async emitCancel(
    name: string,
    payload: unknown,
  ): Promise<{ cancelled: boolean; by?: string }> {
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

  removeModule(moduleId: string): void {
    for (const [name, arr] of this.listeners) {
      this.listeners.set(name, arr.filter((l) => l.module !== moduleId));
    }
  }
}
