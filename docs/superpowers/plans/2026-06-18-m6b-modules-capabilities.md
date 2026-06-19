# M6b — Modules + Capabilities (declarative) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `@shadowcat/core` extensible by trusted, GM-activated modules (hook bus, service registry, middleware, manifest + loader) and add Capability Phase 2 — per-world declarative path→capability requirements the server enforces as data and the client replicates for advisory gating.

**Architecture:** Five Svelte-free TS primitives land in `src/client/core/src/` over the M6a store/optimistic/ws layer; one Rust change set extends the data layer, `apply_intent`, and the `Welcome` frame. The module registry is import-agnostic (the security chokepoint) and a thin loader adapter feeds it; declarative requirements reach the server only as pure data written by a GM-gated config op.

**Tech Stack:** TypeScript + Zod + vitest (`@shadowcat/core`); Rust + axum + sqlx + ts-rs (server); Node test harness driving the real Rust `test_server` over WS.

## Global Constraints

- **Server runs no module code; structural validation only** — declarative requirements reach the server as data, never manifests/code (ARCHITECTURE #6).
- **Headless core is Svelte-free / DOM-free** — no Svelte or browser globals in `@shadowcat/core`'s dependency closure (ARCHITECTURE #7).
- **Server-authoritative** — client capability-awareness is advisory only; the server is the sole authority (ARCHITECTURE #1, #3).
- **Capabilities are additive** — declarative requirements add required caps on top of the Phase-1 structural base; never widen or remove the floor.
- **No new runtime dependencies without consent** — implement a minimal internal semver matcher rather than adding `semver`.
- **No `console.log`** — client diagnostics go through the core `Logger` (project logging rule).
- **Cross-platform** — paths via `std::path`; the new CI job must run the Rust toolchain on a single OS only (the matrix already proves cross-OS Rust build).
- **ts-rs sync** — any Rust type with `#[ts(export)]` regenerates `src/types/generated/`; the CI `git diff --exit-code` gate must stay green (regen is Linux-LF).
- **Decisions from the spec (§14/§15):** typed overlay over open hook keyspace, 3 dispatch kinds, per-hook semver; hybrid registry + import() adapter; per-world (flat, no doc_type) requirements written by a GM-gated op, additive over base; middleware ships both `intent-submit` + `inbound-event`; `on()` with omitted `requires` accepts any hook version. Build the real Node↔Rust e2e harness now.

Reference spec: `docs/superpowers/specs/2026-06-18-m6b-modules-capabilities-design.md`.

---

## Slice 1 — Hook bus, service registry, middleware

### Task 1: Core `Logger` seam

**Files:**
- Create: `src/client/core/src/logger.ts`
- Test: `src/client/core/src/logger.test.ts`
- Modify: `src/client/core/src/index.ts` (export)

**Interfaces:**
- Produces: `interface Logger { debug(msg: string, meta?: unknown): void; warn(msg: string, meta?: unknown): void; error(msg: string, meta?: unknown): void; }`, `const silentLogger: Logger`, `function consoleLogger(): Logger`.

- [ ] **Step 1: Write the failing test**

```ts
// src/client/core/src/logger.test.ts
import { expect, test, vi } from "vitest";
import { silentLogger, consoleLogger } from "./logger";

test("silentLogger swallows all levels", () => {
  expect(() => {
    silentLogger.debug("d");
    silentLogger.warn("w");
    silentLogger.error("e");
  }).not.toThrow();
});

test("consoleLogger routes warn/error to console methods", () => {
  const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
  consoleLogger().warn("hello", { a: 1 });
  expect(warn).toHaveBeenCalledWith("[shadowcat] hello", { a: 1 });
  warn.mockRestore();
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/core test -- logger`
Expected: FAIL — cannot find module `./logger`.

- [ ] **Step 3: Write minimal implementation**

```ts
// src/client/core/src/logger.ts
// The core's diagnostic seam. Production hosts inject their own Logger; the
// core never calls console.* directly (raw console output is banned). Hook /
// module errors are isolated and reported here rather than thrown.
export interface Logger {
  debug(msg: string, meta?: unknown): void;
  warn(msg: string, meta?: unknown): void;
  error(msg: string, meta?: unknown): void;
}

export const silentLogger: Logger = {
  debug() {},
  warn() {},
  error() {},
};

/** A development logger that prefixes the project tag; not used in the bundle. */
export function consoleLogger(): Logger {
  return {
    debug: (m, meta) => console.debug(`[shadowcat] ${m}`, meta),
    warn: (m, meta) => console.warn(`[shadowcat] ${m}`, meta),
    error: (m, meta) => console.error(`[shadowcat] ${m}`, meta),
  };
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/core test -- logger`
Expected: PASS.

- [ ] **Step 5: Export + commit**

Add to `src/client/core/src/index.ts`:
```ts
export { silentLogger, consoleLogger } from "./logger";
export type { Logger } from "./logger";
```

```bash
git add src/client/core/src/logger.ts src/client/core/src/logger.test.ts src/client/core/src/index.ts
git commit -m "feat(m6b): core Logger seam"
```

---

### Task 2: Minimal semver matcher

**Files:**
- Create: `src/client/core/src/semver.ts`
- Test: `src/client/core/src/semver.test.ts`

**Interfaces:**
- Produces: `function satisfies(version: string, range: string): boolean` — supports exact (`1.2.3`), caret (`^1.2.3`), tilde (`~1.2.3`), and wildcard (`*`). Throws `Error` on an unparseable version.

- [ ] **Step 1: Write the failing test**

```ts
// src/client/core/src/semver.test.ts
import { expect, test } from "vitest";
import { satisfies } from "./semver";

test("wildcard matches anything", () => {
  expect(satisfies("9.9.9", "*")).toBe(true);
});
test("exact match", () => {
  expect(satisfies("1.2.3", "1.2.3")).toBe(true);
  expect(satisfies("1.2.4", "1.2.3")).toBe(false);
});
test("caret allows same-major, >= patch/minor", () => {
  expect(satisfies("1.4.0", "^1.2.3")).toBe(true);
  expect(satisfies("1.2.2", "^1.2.3")).toBe(false);
  expect(satisfies("2.0.0", "^1.2.3")).toBe(false);
});
test("tilde allows same-major.minor, >= patch", () => {
  expect(satisfies("1.2.9", "~1.2.3")).toBe(true);
  expect(satisfies("1.3.0", "~1.2.3")).toBe(false);
});
test("invalid version throws", () => {
  expect(() => satisfies("not-a-version", "*")).toThrow();
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/core test -- semver`
Expected: FAIL — cannot find module `./semver`.

- [ ] **Step 3: Write minimal implementation**

```ts
// src/client/core/src/semver.ts
// Internal semver matcher for module dependency ranges and hook versions.
// Deliberately tiny (exact / ^ / ~ / *) to avoid a runtime dependency; swap for
// the `semver` package only if richer ranges become a real requirement.
type V = [number, number, number];

function parse(v: string): V {
  const m = /^(\d+)\.(\d+)\.(\d+)$/.exec(v.trim());
  if (!m) throw new Error(`invalid semver: ${v}`);
  return [Number(m[1]), Number(m[2]), Number(m[3])];
}

function gte(a: V, b: V): boolean {
  for (let i = 0; i < 3; i++) {
    if (a[i] > b[i]) return true;
    if (a[i] < b[i]) return false;
  }
  return true;
}

export function satisfies(version: string, range: string): boolean {
  const r = range.trim();
  const v = parse(version);
  if (r === "*") return true;
  if (r.startsWith("^")) {
    const b = parse(r.slice(1));
    return v[0] === b[0] && gte(v, b);
  }
  if (r.startsWith("~")) {
    const b = parse(r.slice(1));
    return v[0] === b[0] && v[1] === b[1] && gte(v, b);
  }
  const b = parse(r);
  return v[0] === b[0] && v[1] === b[1] && v[2] === b[2];
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/core test -- semver`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/client/core/src/semver.ts src/client/core/src/semver.test.ts
git commit -m "feat(m6b): minimal internal semver matcher"
```

---

### Task 3: HookBus — registration, the three dispatch kinds, versioning, error isolation, module cleanup

**Files:**
- Create: `src/client/core/src/hooks.ts`
- Test: `src/client/core/src/hooks.test.ts`
- Modify: `src/client/core/src/index.ts`

**Interfaces:**
- Consumes: `Logger` (Task 1), `satisfies` (Task 2).
- Produces:
  - `type HookKind = "info" | "mutate" | "cancel"`
  - `interface HookDefinition { version: string; kind: HookKind }`
  - `interface OnOptions { module?: string; priority?: number; requires?: string }`
  - `const STOP: unique symbol` (cancel sentinel)
  - `type Handler<P> = (payload: P) => unknown | Promise<unknown>`
  - `class HookBus` with `defineHook(name, def)`, `on(name, handler, opts?) => () => void`, `emitInfo<P>(name, payload): Promise<void>`, `emitMutate<P>(name, payload): Promise<P>`, `emitCancel<P>(name, payload): Promise<{ cancelled: boolean; by?: string }>`, `removeModule(moduleId): void`.
  - `interface CoreHooks {}` (typed-overlay seam; first-party hooks declaration-merge `name → payload` here).

- [ ] **Step 1: Write the failing tests**

```ts
// src/client/core/src/hooks.test.ts
import { expect, test, vi } from "vitest";
import { HookBus, STOP } from "./hooks";
import { silentLogger } from "./logger";

function bus() {
  return new HookBus(silentLogger);
}

test("emitInfo awaits all handlers; return values ignored", async () => {
  const b = bus();
  b.defineHook("core:test", { version: "1.0.0", kind: "info" });
  const seen: number[] = [];
  b.on("core:test", (p: number) => {
    seen.push(p);
  });
  b.on("core:test", async (p: number) => {
    seen.push(p + 1);
  });
  await b.emitInfo("core:test", 10);
  expect(seen.sort()).toEqual([10, 11]);
});

test("emitMutate chains payload by priority then registration", async () => {
  const b = bus();
  b.defineHook("core:m", { version: "1.0.0", kind: "mutate" });
  b.on("core:m", (n: number) => n + 1, { priority: 0 });
  b.on("core:m", (n: number) => n * 10, { priority: 10 }); // higher priority first
  expect(await b.emitMutate("core:m", 1)).toBe(11); // (1*10)+1
});

test("emitCancel halts on false / STOP and reports who", async () => {
  const b = bus();
  b.defineHook("core:c", { version: "1.0.0", kind: "cancel" });
  b.on("core:c", () => true);
  b.on("core:c", () => false, { module: "blocker" });
  const after = vi.fn();
  b.on("core:c", after);
  const r = await b.emitCancel("core:c", {});
  expect(r).toEqual({ cancelled: true, by: "blocker" });
  expect(after).not.toHaveBeenCalled();
});

test("a throwing handler is isolated and does not abort the chain", async () => {
  const log = { debug: vi.fn(), warn: vi.fn(), error: vi.fn() };
  const b = new HookBus(log);
  b.defineHook("core:m", { version: "1.0.0", kind: "mutate" });
  b.on("core:m", () => {
    throw new Error("boom");
  });
  b.on("core:m", (n: number) => n + 5);
  expect(await b.emitMutate("core:m", 1)).toBe(6); // thrower skipped, prior carried
  expect(log.error).toHaveBeenCalled();
});

test("on() refuses an incompatible version requirement", () => {
  const b = bus();
  b.defineHook("core:v", { version: "1.0.0", kind: "info" });
  expect(() => b.on("core:v", () => {}, { requires: "^2.0.0" })).toThrow();
  expect(() => b.on("core:v", () => {}, { requires: "^1.0.0" })).not.toThrow();
});

test("removeModule drops all of a module's listeners", async () => {
  const b = bus();
  b.defineHook("core:test", { version: "1.0.0", kind: "info" });
  const fn = vi.fn();
  b.on("core:test", fn, { module: "m1" });
  b.removeModule("m1");
  await b.emitInfo("core:test", 1);
  expect(fn).not.toHaveBeenCalled();
});

test("emitting an undefined hook is a no-op error, not a throw", async () => {
  const log = { debug: vi.fn(), warn: vi.fn(), error: vi.fn() };
  const b = new HookBus(log);
  await b.emitInfo("core:missing", 1);
  expect(log.warn).toHaveBeenCalled();
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/core test -- hooks`
Expected: FAIL — cannot find module `./hooks`.

- [ ] **Step 3: Write the implementation**

```ts
// src/client/core/src/hooks.ts
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

  on(name: string, handler: Handler<any>, opts: OnOptions = {}): () => void {
    const def = this.defs.get(name);
    if (def && opts.requires && !satisfies(def.version, opts.requires)) {
      throw new Error(
        `hook ${name} is ${def.version}; listener requires ${opts.requires}`,
      );
    }
    const entry: Listener = {
      handler: handler as Handler<unknown>,
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
      this.logger.error(
        `hook ${name} is ${def.kind}; emitted as ${kind}`,
      );
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/core test -- hooks`
Expected: PASS (all 7).

- [ ] **Step 5: Export + commit**

Add to `index.ts`:
```ts
export { HookBus, STOP } from "./hooks";
export type { HookKind, HookDefinition, OnOptions, Handler, CoreHooks } from "./hooks";
```

```bash
git add src/client/core/src/hooks.ts src/client/core/src/hooks.test.ts src/client/core/src/index.ts
git commit -m "feat(m6b): versioned hook bus (info/mutate/cancel, error isolation, module cleanup)"
```

---

### Task 4: ServiceRegistry

**Files:**
- Create: `src/client/core/src/services.ts`
- Test: `src/client/core/src/services.test.ts`
- Modify: `src/client/core/src/index.ts`

**Interfaces:**
- Produces: `class ServiceRegistry` with `provide<T>(name, impl, opts: { module?: string; version: string }): void`, `get<T>(name): T | undefined`, `has(name): boolean`, `removeModule(moduleId): void`, `versionOf(name): string | undefined`.

- [ ] **Step 1: Write the failing tests**

```ts
// src/client/core/src/services.test.ts
import { expect, test } from "vitest";
import { ServiceRegistry } from "./services";

test("provide then get returns the impl and version", () => {
  const r = new ServiceRegistry();
  r.provide("dice", { roll: () => 4 }, { module: "core", version: "1.0.0" });
  expect(r.has("dice")).toBe(true);
  expect(r.get<{ roll: () => number }>("dice")!.roll()).toBe(4);
  expect(r.versionOf("dice")).toBe("1.0.0");
});

test("duplicate provide is an error", () => {
  const r = new ServiceRegistry();
  r.provide("x", {}, { version: "1.0.0" });
  expect(() => r.provide("x", {}, { version: "1.0.0" })).toThrow();
});

test("removeModule drops that module's services", () => {
  const r = new ServiceRegistry();
  r.provide("x", {}, { module: "m1", version: "1.0.0" });
  r.removeModule("m1");
  expect(r.has("x")).toBe(false);
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/core test -- services`
Expected: FAIL — cannot find module `./services`.

- [ ] **Step 3: Write the implementation**

```ts
// src/client/core/src/services.ts
// Named singletons modules provide for others to consume. Duplicate names are a
// hard error (no silent override); a module's services are removed on unload.
interface Entry {
  impl: unknown;
  version: string;
  module?: string;
}

export class ServiceRegistry {
  private entries = new Map<string, Entry>();

  provide<T>(name: string, impl: T, opts: { module?: string; version: string }): void {
    if (this.entries.has(name)) {
      throw new Error(`service ${name} already provided`);
    }
    this.entries.set(name, { impl, version: opts.version, module: opts.module });
  }

  get<T>(name: string): T | undefined {
    return this.entries.get(name)?.impl as T | undefined;
  }

  has(name: string): boolean {
    return this.entries.has(name);
  }

  versionOf(name: string): string | undefined {
    return this.entries.get(name)?.version;
  }

  removeModule(moduleId: string): void {
    for (const [name, e] of this.entries) {
      if (e.module === moduleId) this.entries.delete(name);
    }
  }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/core test -- services`
Expected: PASS.

- [ ] **Step 5: Export + commit**

Add to `index.ts`: `export { ServiceRegistry } from "./services";`

```bash
git add src/client/core/src/services.ts src/client/core/src/services.test.ts src/client/core/src/index.ts
git commit -m "feat(m6b): service registry (provide/get, conflict, module cleanup)"
```

---

### Task 5: MiddlewareChain

**Files:**
- Create: `src/client/core/src/middleware.ts`
- Test: `src/client/core/src/middleware.test.ts`
- Modify: `src/client/core/src/index.ts`

**Interfaces:**
- Produces:
  - `type PipelineName = "intent-submit" | "inbound-event"`
  - `type Middleware<C> = (ctx: C, next: () => Promise<void>) => Promise<void>`
  - `class MiddlewareChain` with `use<C>(pipeline, mw, opts?: { module?: string }): void`, `run<C>(pipeline, ctx): Promise<void>`, `removeModule(moduleId): void`.

- [ ] **Step 1: Write the failing tests**

```ts
// src/client/core/src/middleware.test.ts
import { expect, test } from "vitest";
import { MiddlewareChain } from "./middleware";

test("middleware runs in registration order and can transform ctx", async () => {
  const c = new MiddlewareChain();
  const log: string[] = [];
  c.use("intent-submit", async (ctx: { v: number }, next) => {
    log.push("a-in");
    ctx.v += 1;
    await next();
    log.push("a-out");
  });
  c.use("intent-submit", async (ctx: { v: number }, next) => {
    log.push("b");
    ctx.v *= 10;
    await next();
  });
  const ctx = { v: 1 };
  await c.run("intent-submit", ctx);
  expect(ctx.v).toBe(20); // (1+1)*10
  expect(log).toEqual(["a-in", "b", "a-out"]);
});

test("a middleware that does not call next short-circuits", async () => {
  const c = new MiddlewareChain();
  let reached = false;
  c.use("intent-submit", async () => {
    /* no next() */
  });
  c.use("intent-submit", async (_ctx, next) => {
    reached = true;
    await next();
  });
  await c.run("intent-submit", {});
  expect(reached).toBe(false);
});

test("removeModule drops that module's middleware", async () => {
  const c = new MiddlewareChain();
  let ran = false;
  c.use("inbound-event", async (_ctx, next) => {
    ran = true;
    await next();
  }, { module: "m1" });
  c.removeModule("m1");
  await c.run("inbound-event", {});
  expect(ran).toBe(false);
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/core test -- middleware`
Expected: FAIL — cannot find module `./middleware`.

- [ ] **Step 3: Write the implementation**

```ts
// src/client/core/src/middleware.ts
// Ordered next()-style pipelines around core operations. v1 pipelines:
// "intent-submit" (transform/cancel an outgoing optimistic intent before
// OptimisticClient) and "inbound-event" (observe a confirmed event as applied).
// A middleware that omits next() short-circuits the remainder of the chain.
export type PipelineName = "intent-submit" | "inbound-event";
export type Middleware<C> = (ctx: C, next: () => Promise<void>) => Promise<void>;

interface Entry {
  mw: Middleware<unknown>;
  module?: string;
}

export class MiddlewareChain {
  private chains = new Map<PipelineName, Entry[]>();

  use<C>(pipeline: PipelineName, mw: Middleware<C>, opts: { module?: string } = {}): void {
    const arr = this.chains.get(pipeline) ?? [];
    arr.push({ mw: mw as Middleware<unknown>, module: opts.module });
    this.chains.set(pipeline, arr);
  }

  async run<C>(pipeline: PipelineName, ctx: C): Promise<void> {
    const arr = this.chains.get(pipeline) ?? [];
    const dispatch = async (i: number): Promise<void> => {
      if (i >= arr.length) return;
      await arr[i].mw(ctx, () => dispatch(i + 1));
    };
    await dispatch(0);
  }

  removeModule(moduleId: string): void {
    for (const [name, arr] of this.chains) {
      this.chains.set(name, arr.filter((e) => e.module !== moduleId));
    }
  }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/core test -- middleware`
Expected: PASS.

- [ ] **Step 5: Export + commit**

Add to `index.ts`:
```ts
export { MiddlewareChain } from "./middleware";
export type { PipelineName, Middleware } from "./middleware";
```

```bash
git add src/client/core/src/middleware.ts src/client/core/src/middleware.test.ts src/client/core/src/index.ts
git commit -m "feat(m6b): middleware chain (ordered next() pipelines, short-circuit, cleanup)"
```

---

## Slice 2 — Module manifest, registry, loader adapter

### Task 6: Manifest schema + types

**Files:**
- Create: `src/client/core/src/manifest.ts`
- Test: `src/client/core/src/manifest.test.ts`
- Modify: `src/client/core/src/index.ts`

**Interfaces:**
- Consumes: `HookKind` (Task 3).
- Produces:
  - `interface CapRequirement { path_prefix: string; caps: string[] }`
  - `interface HookDecl { name: string; version: string; kind: HookKind }`
  - `interface ModuleManifest { id: string; version: string; name?: string; dependencies: Record<string, string>; capabilities?: string[]; requirements?: CapRequirement[]; hooks?: HookDecl[] }`
  - `const ManifestSchema: z.ZodType<ModuleManifest>`
  - `function parseManifest(value: unknown): ModuleManifest` (throws on invalid).

- [ ] **Step 1: Write the failing tests**

```ts
// src/client/core/src/manifest.test.ts
import { expect, test } from "vitest";
import { parseManifest } from "./manifest";

test("valid manifest parses with defaults", () => {
  const m = parseManifest({ id: "dnd5e", version: "1.0.0", dependencies: {} });
  expect(m.id).toBe("dnd5e");
  expect(m.dependencies).toEqual({});
});

test("requirements and hooks parse", () => {
  const m = parseManifest({
    id: "vision",
    version: "0.1.0",
    dependencies: { core: "^1.0.0" },
    capabilities: ["dnd5e:gm_vision"],
    requirements: [{ path_prefix: "/system/vision", caps: ["dnd5e:gm_vision"] }],
    hooks: [{ name: "dnd5e:preRollAttack", version: "1.0.0", kind: "cancel" }],
  });
  expect(m.requirements![0].path_prefix).toBe("/system/vision");
  expect(m.hooks![0].kind).toBe("cancel");
});

test("missing id is rejected", () => {
  expect(() => parseManifest({ version: "1.0.0", dependencies: {} })).toThrow();
});

test("requirement path_prefix must start with /", () => {
  expect(() =>
    parseManifest({
      id: "x",
      version: "1.0.0",
      dependencies: {},
      requirements: [{ path_prefix: "system", caps: ["x:y"] }],
    }),
  ).toThrow();
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/core test -- manifest`
Expected: FAIL — cannot find module `./manifest`.

- [ ] **Step 3: Write the implementation**

```ts
// src/client/core/src/manifest.ts
// The module manifest: identity, semver, dependencies, declared capabilities,
// declarative path->capability requirements, and declared hooks. Validated with
// Zod before a module is admitted to the registry. The `requirements` are the
// data the GM publishes to the server's per-world capability_requirements record.
import { z } from "zod";
import type { HookKind } from "./hooks";

export interface CapRequirement {
  path_prefix: string;
  caps: string[];
}
export interface HookDecl {
  name: string;
  version: string;
  kind: HookKind;
}
export interface ModuleManifest {
  id: string;
  version: string;
  name?: string;
  dependencies: Record<string, string>;
  capabilities?: string[];
  requirements?: CapRequirement[];
  hooks?: HookDecl[];
}

const HookKindSchema = z.enum(["info", "mutate", "cancel"]);

const CapRequirementSchema = z.object({
  path_prefix: z.string().startsWith("/"),
  caps: z.array(z.string()).min(1),
});

export const ManifestSchema: z.ZodType<ModuleManifest> = z.object({
  id: z.string().min(1),
  version: z.string().min(1),
  name: z.string().optional(),
  dependencies: z.record(z.string()),
  capabilities: z.array(z.string()).optional(),
  requirements: z.array(CapRequirementSchema).optional(),
  hooks: z
    .array(z.object({ name: z.string(), version: z.string(), kind: HookKindSchema }))
    .optional(),
});

export function parseManifest(value: unknown): ModuleManifest {
  return ManifestSchema.parse(value);
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/core test -- manifest`
Expected: PASS.

- [ ] **Step 5: Export + commit**

Add to `index.ts`:
```ts
export { ManifestSchema, parseManifest } from "./manifest";
export type { ModuleManifest, CapRequirement, HookDecl } from "./manifest";
```

```bash
git add src/client/core/src/manifest.ts src/client/core/src/manifest.test.ts src/client/core/src/index.ts
git commit -m "feat(m6b): module manifest schema + parser"
```

---

### Task 7: ModuleRegistry — validation, topo-sort, activation, capability collection

**Files:**
- Create: `src/client/core/src/modules.ts`
- Test: `src/client/core/src/modules.test.ts`
- Modify: `src/client/core/src/index.ts`

**Interfaces:**
- Consumes: `HookBus` (3), `ServiceRegistry` (4), `MiddlewareChain` (5), `Logger` (1), `DocumentStore` (M6a `store.ts`), `OptimisticClient` (M6a `optimistic.ts`), `ModuleManifest`/`CapRequirement` (6), `parseManifest` (6), `satisfies` (2).
- Produces:
  - `interface ModuleContext { hooks: HookBus; services: ServiceRegistry; use: MiddlewareChain["use"]; store: DocumentStore; client: OptimisticClient; logger: Logger; moduleId: string }`
  - `interface Module { manifest: ModuleManifest; register(ctx: ModuleContext): void | Promise<void>; unregister?(): void | Promise<void> }`
  - `interface ModuleInfo { id: string; version: string; active: boolean }`
  - `class ModuleRegistry` with constructor `(deps: { hooks; services; middleware; store; client; logger })`, `add(module: Module): void`, `activate(): Promise<void>`, `unload(id: string, opts?: { cascade?: boolean }): Promise<void>`, `list(): ModuleInfo[]`, `collectRequirements(): CapRequirement[]`.

> Note: `ctx.hooks` is the shared bus; the registry passes `{ module: moduleId }` automatically by wrapping `on`. To keep the wrap simple, `ModuleContext.hooks` is a thin per-module facade whose `on` injects the module id; `emit*`/`defineHook` pass through. (Implemented inline below.)

- [ ] **Step 1: Write the failing tests**

```ts
// src/client/core/src/modules.test.ts
import { expect, test, vi } from "vitest";
import { ModuleRegistry, type Module } from "./modules";
import { HookBus } from "./hooks";
import { ServiceRegistry } from "./services";
import { MiddlewareChain } from "./middleware";
import { DocumentStore } from "./store";
import { OptimisticClient } from "./optimistic";
import { silentLogger } from "./logger";

function deps() {
  const store = new DocumentStore();
  return {
    hooks: new HookBus(silentLogger),
    services: new ServiceRegistry(),
    middleware: new MiddlewareChain(),
    store,
    client: new OptimisticClient(store),
    logger: silentLogger,
  };
}

function mod(id: string, dependencies: Record<string, string>, register = vi.fn()): Module {
  return { manifest: { id, version: "1.0.0", dependencies }, register };
}

test("activate calls register in dependency order", async () => {
  const order: string[] = [];
  const d = deps();
  const r = new ModuleRegistry(d);
  r.add(mod("b", { a: "^1.0.0" }, vi.fn(() => order.push("b"))));
  r.add(mod("a", {}, vi.fn(() => order.push("a"))));
  await r.activate();
  expect(order).toEqual(["a", "b"]);
});

test("missing dependency skips the module and its dependents", async () => {
  const d = deps();
  const r = new ModuleRegistry(d);
  const reg = vi.fn();
  r.add(mod("needs-missing", { ghost: "^1.0.0" }, reg));
  await r.activate();
  expect(reg).not.toHaveBeenCalled();
  expect(r.list().find((m) => m.id === "needs-missing")!.active).toBe(false);
});

test("incompatible dependency version is rejected", async () => {
  const d = deps();
  const r = new ModuleRegistry(d);
  r.add(mod("a", {})); // a@1.0.0
  const reg = vi.fn();
  r.add(mod("b", { a: "^2.0.0" }, reg));
  await r.activate();
  expect(reg).not.toHaveBeenCalled();
});

test("dependency cycle throws with the cycle path", async () => {
  const d = deps();
  const r = new ModuleRegistry(d);
  r.add(mod("a", { b: "^1.0.0" }));
  r.add(mod("b", { a: "^1.0.0" }));
  await expect(r.activate()).rejects.toThrow(/cycle/i);
});

test("invalid manifest is rejected at add()", () => {
  const d = deps();
  const r = new ModuleRegistry(d);
  expect(() =>
    r.add({ manifest: { id: "", version: "1.0.0", dependencies: {} }, register: vi.fn() }),
  ).toThrow();
});

test("collectRequirements unions active modules' requirements", async () => {
  const d = deps();
  const r = new ModuleRegistry(d);
  r.add({
    manifest: {
      id: "vision",
      version: "1.0.0",
      dependencies: {},
      requirements: [{ path_prefix: "/system/vision", caps: ["dnd5e:gm_vision"] }],
    },
    register: vi.fn(),
  });
  await r.activate();
  expect(r.collectRequirements()).toEqual([
    { path_prefix: "/system/vision", caps: ["dnd5e:gm_vision"] },
  ]);
});

test("unload removes the module's registrations and refuses depended-upon unless cascade", async () => {
  const d = deps();
  const r = new ModuleRegistry(d);
  r.add({
    manifest: { id: "a", version: "1.0.0", dependencies: {} },
    register: (ctx) => {
      ctx.hooks.defineHook("a:evt", { version: "1.0.0", kind: "info" });
      ctx.hooks.on("a:evt", () => {});
      ctx.services.provide("a:svc", {}, { version: "1.0.0" });
    },
  });
  r.add({ manifest: { id: "b", version: "1.0.0", dependencies: { a: "^1.0.0" } }, register: vi.fn() });
  await r.activate();

  await expect(r.unload("a")).rejects.toThrow(/depend/i);
  await r.unload("a", { cascade: true });
  expect(d.services.has("a:svc")).toBe(false);
  expect(r.list().find((m) => m.id === "a")!.active).toBe(false);
  expect(r.list().find((m) => m.id === "b")!.active).toBe(false);
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/core test -- modules`
Expected: FAIL — cannot find module `./modules`.

- [ ] **Step 3: Write the implementation**

```ts
// src/client/core/src/modules.ts
// Import-agnostic module registry: validates manifests, resolves dependencies
// (presence + semver), activates in topological order, and tracks every
// registration per module so unload is a clean teardown. This is the trust
// chokepoint — each module sees only the capability-scoped ModuleContext. How a
// Module object is produced (dynamic import, static host wiring, future sandbox)
// is the loader adapter's concern, never the registry's.
import { HookBus, type HookDefinition, type Handler, type OnOptions } from "./hooks";
import { ServiceRegistry } from "./services";
import { MiddlewareChain, type Middleware, type PipelineName } from "./middleware";
import { DocumentStore } from "./store";
import { OptimisticClient } from "./optimistic";
import type { Logger } from "./logger";
import { parseManifest, type ModuleManifest, type CapRequirement } from "./manifest";
import { satisfies } from "./semver";

export interface ModuleContext {
  hooks: {
    defineHook(name: string, def: HookDefinition): void;
    on(name: string, handler: Handler<any>, opts?: OnOptions): () => void;
    emitInfo(name: string, payload: unknown): Promise<void>;
    emitMutate<P>(name: string, payload: P): Promise<P>;
    emitCancel(name: string, payload: unknown): Promise<{ cancelled: boolean; by?: string }>;
  };
  services: {
    provide<T>(name: string, impl: T, opts: { version: string }): void;
    get<T>(name: string): T | undefined;
    has(name: string): boolean;
  };
  use<C>(pipeline: PipelineName, mw: Middleware<C>): void;
  store: DocumentStore;
  client: OptimisticClient;
  logger: Logger;
  moduleId: string;
}

export interface Module {
  manifest: ModuleManifest;
  register(ctx: ModuleContext): void | Promise<void>;
  unregister?(): void | Promise<void>;
}

export interface ModuleInfo {
  id: string;
  version: string;
  active: boolean;
}

interface Deps {
  hooks: HookBus;
  services: ServiceRegistry;
  middleware: MiddlewareChain;
  store: DocumentStore;
  client: OptimisticClient;
  logger: Logger;
}

interface Record_ {
  module: Module;
  active: boolean;
}

export class ModuleRegistry {
  private records = new Map<string, Record_>();

  constructor(private readonly deps: Deps) {}

  add(module: Module): void {
    parseManifest(module.manifest); // throws on invalid
    const id = module.manifest.id;
    if (this.records.has(id)) throw new Error(`module ${id} already added`);
    this.records.set(id, { module, active: false });
  }

  list(): ModuleInfo[] {
    return [...this.records.values()].map((r) => ({
      id: r.module.manifest.id,
      version: r.module.manifest.version,
      active: r.active,
    }));
  }

  collectRequirements(): CapRequirement[] {
    const out: CapRequirement[] = [];
    for (const r of this.records.values()) {
      if (r.active) out.push(...(r.module.manifest.requirements ?? []));
    }
    return out;
  }

  async activate(): Promise<void> {
    const order = this.topoSort(); // throws on cycle
    for (const id of order) {
      const r = this.records.get(id)!;
      if (r.active) continue;
      if (!this.depsSatisfied(r.module)) {
        this.deps.logger.warn(`module ${id} not activated: dependency unmet`);
        continue;
      }
      await r.module.register(this.contextFor(id));
      r.active = true;
    }
  }

  async unload(id: string, opts: { cascade?: boolean } = {}): Promise<void> {
    const r = this.records.get(id);
    if (!r) return;
    const dependents = this.activeDependentsOf(id);
    if (dependents.length > 0) {
      if (!opts.cascade) {
        throw new Error(`cannot unload ${id}: modules depend on it: ${dependents.join(", ")}`);
      }
      for (const dep of dependents) await this.unload(dep, { cascade: true });
    }
    if (r.active && r.module.unregister) await r.module.unregister();
    this.deps.hooks.removeModule(id);
    this.deps.services.removeModule(id);
    this.deps.middleware.removeModule(id);
    r.active = false;
  }

  private depsSatisfied(m: Module): boolean {
    for (const [depId, range] of Object.entries(m.manifest.dependencies)) {
      const dep = this.records.get(depId);
      if (!dep || !dep.active) return false;
      if (!satisfies(dep.module.manifest.version, range)) return false;
    }
    return true;
  }

  private activeDependentsOf(id: string): string[] {
    return [...this.records.values()]
      .filter((r) => r.active && id in r.module.manifest.dependencies)
      .map((r) => r.module.manifest.id);
  }

  private topoSort(): string[] {
    const visited = new Set<string>();
    const onstack = new Set<string>();
    const out: string[] = [];
    const visit = (id: string, path: string[]): void => {
      if (visited.has(id)) return;
      if (onstack.has(id)) {
        throw new Error(`dependency cycle: ${[...path, id].join(" -> ")}`);
      }
      const r = this.records.get(id);
      if (!r) return; // missing dep handled by depsSatisfied
      onstack.add(id);
      for (const depId of Object.keys(r.module.manifest.dependencies)) {
        visit(depId, [...path, id]);
      }
      onstack.delete(id);
      visited.add(id);
      out.push(id);
    };
    for (const id of this.records.keys()) visit(id, []);
    return out;
  }

  private contextFor(moduleId: string): ModuleContext {
    const { hooks, services, middleware, store, client, logger } = this.deps;
    return {
      moduleId,
      store,
      client,
      logger,
      hooks: {
        defineHook: (name, def) => hooks.defineHook(name, def),
        on: (name, handler, opts) => hooks.on(name, handler, { ...opts, module: moduleId }),
        emitInfo: (name, p) => hooks.emitInfo(name, p),
        emitMutate: (name, p) => hooks.emitMutate(name, p),
        emitCancel: (name, p) => hooks.emitCancel(name, p),
      },
      services: {
        provide: (name, impl, opts) => services.provide(name, impl, { ...opts, module: moduleId }),
        get: (name) => services.get(name),
        has: (name) => services.has(name),
      },
      use: (pipeline, mw) => middleware.use(pipeline, mw, { module: moduleId }),
    };
  }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/core test -- modules`
Expected: PASS (all 7).

- [ ] **Step 5: Export + commit**

Add to `index.ts`:
```ts
export { ModuleRegistry } from "./modules";
export type { Module, ModuleContext, ModuleInfo } from "./modules";
```

```bash
git add src/client/core/src/modules.ts src/client/core/src/modules.test.ts src/client/core/src/index.ts
git commit -m "feat(m6b): module registry (topo-sort, semver deps, hot-unload, requirement collection)"
```

---

### Task 8: Loader adapter

**Files:**
- Create: `src/client/core/src/loader.ts`
- Test: `src/client/core/src/loader.test.ts`
- Modify: `src/client/core/src/index.ts`

**Interfaces:**
- Consumes: `ModuleRegistry`, `Module` (7), `parseManifest`, `ModuleManifest` (6).
- Produces:
  - `type ImportFn = (entry: string) => Promise<{ default: Module } | Module>`
  - `interface ModuleEntry { manifest: ModuleManifest; entry: string }`
  - `async function loadModules(opts: { entries: ModuleEntry[]; importFn: ImportFn; registry: ModuleRegistry }): Promise<void>` — validates each manifest, imports each entry, normalizes default/namespace export, verifies `module.manifest.id === manifest.id`, and `registry.add`s it. Does not call `activate()` (the caller controls activation timing).

- [ ] **Step 1: Write the failing tests**

```ts
// src/client/core/src/loader.test.ts
import { expect, test, vi } from "vitest";
import { loadModules } from "./loader";
import { ModuleRegistry, type Module } from "./modules";
import { HookBus } from "./hooks";
import { ServiceRegistry } from "./services";
import { MiddlewareChain } from "./middleware";
import { DocumentStore } from "./store";
import { OptimisticClient } from "./optimistic";
import { silentLogger } from "./logger";

function registry() {
  const store = new DocumentStore();
  return new ModuleRegistry({
    hooks: new HookBus(silentLogger),
    services: new ServiceRegistry(),
    middleware: new MiddlewareChain(),
    store,
    client: new OptimisticClient(store),
    logger: silentLogger,
  });
}

const mod: Module = {
  manifest: { id: "a", version: "1.0.0", dependencies: {} },
  register: vi.fn(),
};

test("loadModules imports entries and adds them to the registry", async () => {
  const r = registry();
  const importFn = vi.fn(async () => ({ default: mod }));
  await loadModules({
    entries: [{ manifest: mod.manifest, entry: "./a.js" }],
    importFn,
    registry: r,
  });
  expect(importFn).toHaveBeenCalledWith("./a.js");
  expect(r.list().map((m) => m.id)).toEqual(["a"]);
});

test("a namespace export (no default) is accepted", async () => {
  const r = registry();
  await loadModules({
    entries: [{ manifest: mod.manifest, entry: "./a.js" }],
    importFn: async () => mod,
    registry: r,
  });
  expect(r.list()).toHaveLength(1);
});

test("manifest id mismatch is rejected", async () => {
  const r = registry();
  await expect(
    loadModules({
      entries: [{ manifest: { id: "declared", version: "1.0.0", dependencies: {} }, entry: "./a.js" }],
      importFn: async () => mod, // module's own id is "a"
      registry: r,
    }),
  ).rejects.toThrow(/id/i);
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/core test -- loader`
Expected: FAIL — cannot find module `./loader`.

- [ ] **Step 3: Write the implementation**

```ts
// src/client/core/src/loader.ts
// Thin delivery adapter: turns discovered (manifest, entry) pairs into Module
// objects via an injectable importFn and hands them to the registry. Discovery
// (filesystem in Node, fetch in the browser) is the host's job; the adapter
// stays environment-neutral so a future sandboxed delivery is another importFn.
import { ModuleRegistry, type Module } from "./modules";
import { parseManifest, type ModuleManifest } from "./manifest";

export type ImportFn = (entry: string) => Promise<{ default: Module } | Module>;

export interface ModuleEntry {
  manifest: ModuleManifest;
  entry: string;
}

function normalize(imported: { default: Module } | Module): Module {
  return "default" in imported && (imported as { default: Module }).default
    ? (imported as { default: Module }).default
    : (imported as Module);
}

export async function loadModules(opts: {
  entries: ModuleEntry[];
  importFn: ImportFn;
  registry: ModuleRegistry;
}): Promise<void> {
  for (const { manifest, entry } of opts.entries) {
    parseManifest(manifest);
    const module = normalize(await opts.importFn(entry));
    if (module.manifest.id !== manifest.id) {
      throw new Error(
        `module at ${entry} declares id ${module.manifest.id}, manifest says ${manifest.id}`,
      );
    }
    opts.registry.add(module);
  }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/core test -- loader`
Expected: PASS.

- [ ] **Step 5: Export + commit**

Add to `index.ts`:
```ts
export { loadModules } from "./loader";
export type { ImportFn, ModuleEntry } from "./loader";
```

```bash
git add src/client/core/src/loader.ts src/client/core/src/loader.test.ts src/client/core/src/index.ts
git commit -m "feat(m6b): module loader adapter (import-fn injection, id verification)"
```

---

## Slice 3 — Server: declarative capability requirements + Welcome extension

### Task 9: `CapabilityRequirement` type + repository methods

**Files:**
- Modify: `src/server/src/data/document.rs` (add `CapabilityRequirement`)
- Modify: `src/server/src/data/repository.rs` (trait methods)
- Modify: `src/server/src/data/sqlite.rs` (impl + key fn + test)

**Interfaces:**
- Produces:
  - `pub struct CapabilityRequirement { pub path_prefix: String, pub caps: BTreeSet<String> }` (`Serialize, Deserialize, Clone, Debug, PartialEq, Eq, TS`, `#[ts(export, export_to = "../../types/generated/")]`).
  - Trait `Repository`: `async fn world_cap_requirements(&self, world: Uuid) -> Result<Vec<CapabilityRequirement>, DataError>;`
  - `SqliteRepository::set_world_cap_requirements(&self, world: Uuid, reqs: &[CapabilityRequirement]) -> Result<(), DataError>` + the trait impl of `world_cap_requirements`.

- [ ] **Step 1: Write the failing test** (in `sqlite.rs` `#[cfg(test)] mod tests`)

```rust
#[tokio::test]
async fn world_cap_requirements_round_trip() {
    use crate::data::document::CapabilityRequirement;
    let r = repo().await;
    let gm = r.create_user("gm", None, crate::auth::role::ServerRole::User, 0).await.unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    // Default is empty.
    assert!(r.world_cap_requirements(w.id).await.unwrap().is_empty());
    let reqs = vec![CapabilityRequirement {
        path_prefix: "/system/vision".into(),
        caps: ["dnd5e:gm_vision".to_string()].into_iter().collect(),
    }];
    r.set_world_cap_requirements(w.id, &reqs).await.unwrap();
    assert_eq!(r.world_cap_requirements(w.id).await.unwrap(), reqs);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat world_cap_requirements_round_trip`
Expected: FAIL — `CapabilityRequirement` / methods do not exist.

- [ ] **Step 3: Add the type** in `src/server/src/data/document.rs` near `CapabilityGrants`:

```rust
/// A declarative requirement: writing any field under `path_prefix` requires the
/// actor to additionally hold every capability in `caps` (on top of the
/// structural base capability for that path). Pure data — the server enforces
/// possession and never interprets the meaning of the path or the capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct CapabilityRequirement {
    pub path_prefix: String,
    pub caps: std::collections::BTreeSet<String>,
}
```

- [ ] **Step 4: Add the trait method** in `src/server/src/data/repository.rs` (alongside `world_cap_defaults`):

```rust
    async fn world_cap_requirements(
        &self,
        world: Uuid,
    ) -> Result<Vec<crate::data::document::CapabilityRequirement>, DataError>;
```
(Add `use` for `CapabilityRequirement` if the file imports types explicitly; otherwise the fully-qualified path above suffices.)

- [ ] **Step 5: Implement in `src/server/src/data/sqlite.rs`** — a setter near `set_world_cap_defaults`:

```rust
    /// Replace a world's declarative capability requirements (stored as JSON).
    pub async fn set_world_cap_requirements(
        &self,
        world: Uuid,
        reqs: &[crate::data::document::CapabilityRequirement],
    ) -> Result<(), DataError> {
        let json = serde_json::to_string(reqs)?;
        self.set_setting(&world_caps_req_key(world), &json).await
    }
```
the trait impl near `world_cap_defaults`:
```rust
    async fn world_cap_requirements(
        &self,
        world: Uuid,
    ) -> Result<Vec<crate::data::document::CapabilityRequirement>, DataError> {
        match self.get_setting(&world_caps_req_key(world)).await? {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Ok(Vec::new()),
        }
    }
```
and the key fn near `world_caps_key`:
```rust
/// Settings key holding a world's declarative capability requirements (JSON).
fn world_caps_req_key(world: Uuid) -> String {
    format!("world_caps_req:{world}")
}
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo test -p shadowcat world_cap_requirements_round_trip`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/server/src/data/document.rs src/server/src/data/repository.rs src/server/src/data/sqlite.rs src/types/generated/CapabilityRequirement.ts
git commit -m "feat(m6b): world capability-requirements record (data layer)"
```

---

### Task 10: Enforce declarative requirements in `apply_intent`

**Files:**
- Modify: `src/server/src/data/permission.rs` (add `declared_caps_for_path` + unit test)
- Modify: `src/server/src/data/sqlite.rs` (`apply_intent`: load reqs, check additively + integration test)

**Interfaces:**
- Consumes: `CapabilityRequirement` (9), existing `required_cap_for_path`, `resolve_access_world`, `Access::has` (permission.rs).
- Produces: `pub fn declared_caps_for_path<'a>(path: &str, reqs: &'a [CapabilityRequirement]) -> Vec<&'a str>` — every cap of every requirement whose `path_prefix` matches `path` (exact or `path_prefix`-prefix on a path boundary). Additive: callers also still require the base `required_cap_for_path`.

- [ ] **Step 1: Write the failing unit test** in `permission.rs` tests:

```rust
#[test]
fn declared_caps_match_prefix_on_boundaries() {
    use crate::data::document::CapabilityRequirement;
    let reqs = vec![CapabilityRequirement {
        path_prefix: "/system/vision".into(),
        caps: ["dnd5e:gm_vision".to_string()].into_iter().collect(),
    }];
    // exact and descendant match
    assert_eq!(declared_caps_for_path("/system/vision", &reqs), vec!["dnd5e:gm_vision"]);
    assert_eq!(declared_caps_for_path("/system/vision/range", &reqs), vec!["dnd5e:gm_vision"]);
    // sibling that merely shares a string prefix does NOT match (boundary check)
    assert!(declared_caps_for_path("/system/visionmode", &reqs).is_empty());
    // unrelated path
    assert!(declared_caps_for_path("/system/hp", &reqs).is_empty());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat declared_caps_match_prefix_on_boundaries`
Expected: FAIL — `declared_caps_for_path` undefined.

- [ ] **Step 3: Implement `declared_caps_for_path`** in `permission.rs` (below `required_cap_for_path`):

```rust
/// Additional capabilities required to write `path`, declared by the world's
/// capability requirements. Returned on top of `required_cap_for_path`'s
/// structural base. A requirement matches when `path` equals the prefix or is a
/// descendant of it (matched on a `/` boundary so `/system/visionmode` does not
/// match a `/system/vision` requirement).
pub fn declared_caps_for_path<'a>(
    path: &str,
    reqs: &'a [CapabilityRequirement],
) -> Vec<&'a str> {
    let mut out = Vec::new();
    for req in reqs {
        let matches = path == req.path_prefix
            || path.starts_with(&format!("{}/", req.path_prefix));
        if matches {
            out.extend(req.caps.iter().map(String::as_str));
        }
    }
    out
}
```
Add `CapabilityRequirement` to the `use crate::data::document::{...}` line.

- [ ] **Step 4: Run unit test to verify it passes**

Run: `cargo test -p shadowcat declared_caps_match_prefix_on_boundaries`
Expected: PASS.

- [ ] **Step 5: Write the failing integration test** in `sqlite.rs` tests (drives the real write path):

```rust
#[tokio::test]
async fn declarative_requirement_blocks_writer_without_extra_cap() {
    use crate::auth::role::ServerRole;
    use crate::data::command::{FieldChange, Operation};
    use crate::data::document::{CapabilityRequirement, DocRole, PermissionSet, Scope};
    use crate::data::membership::PermissionContext;

    let r = repo().await;
    let gm = r.create_user("gm", None, ServerRole::User, 0).await.unwrap();
    let player = r.create_user("pl", None, ServerRole::User, 0).await.unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();

    // A doc the player owns (owner floor: read + write_fields).
    let mut perms = PermissionSet::default();
    perms.users.insert(player, DocRole::Owner);
    let mut d = super::tests_doc(perms, serde_json::json!({ "vision": { "range": 30 }, "hp": 10 }));
    d.scope = Scope::World { world_id: w.id };
    let gm_ctx = PermissionContext { user_id: gm, world_role: WorldRole::Gm };
    r.apply_intent(&gm_ctx, w.id, vec![Operation::Create { doc: d.clone() }], 1).await.unwrap();

    // Require dnd5e:gm_vision to write /system/vision.
    r.set_world_cap_requirements(w.id, &[CapabilityRequirement {
        path_prefix: "/system/vision".into(),
        caps: ["dnd5e:gm_vision".to_string()].into_iter().collect(),
    }]).await.unwrap();

    let player_ctx = PermissionContext { user_id: player, world_role: WorldRole::Player };

    // Owner CAN write a non-restricted /system field (base cap only).
    r.apply_intent(&player_ctx, w.id, vec![Operation::Update {
        doc_id: d.id,
        changes: vec![FieldChange { path: "/system/hp".into(), old: serde_json::json!(10), new: serde_json::json!(8) }],
    }], 2).await.unwrap();

    // Owner CANNOT write /system/vision (lacks dnd5e:gm_vision).
    let err = r.apply_intent(&player_ctx, w.id, vec![Operation::Update {
        doc_id: d.id,
        changes: vec![FieldChange { path: "/system/vision/range".into(), old: serde_json::json!(30), new: serde_json::json!(60) }],
    }], 3).await;
    assert!(matches!(err, Err(DataError::Forbidden)));

    // GM is unaffected (holds everything).
    r.apply_intent(&gm_ctx, w.id, vec![Operation::Update {
        doc_id: d.id,
        changes: vec![FieldChange { path: "/system/vision/range".into(), old: serde_json::json!(30), new: serde_json::json!(60) }],
    }], 4).await.unwrap();
}
```
Add a small shared `tests_doc` helper in `sqlite.rs` tests if one is not already present (mirrors `permission.rs`'s `doc`):
```rust
#[cfg(test)]
pub(crate) fn tests_doc(perms: crate::data::document::PermissionSet, system: serde_json::Value) -> crate::data::document::Document {
    use crate::data::document::{Document, Scope};
    Document {
        id: uuid::Uuid::new_v4(),
        scope: Scope::World { world_id: uuid::Uuid::from_u128(9) },
        doc_type: "actor".into(),
        schema_version: 1,
        source: None,
        owner: None,
        permissions: perms,
        embedded: Default::default(),
        system,
        created_at: 0,
        updated_at: 0,
    }
}
```

- [ ] **Step 6: Run integration test to verify it fails**

Run: `cargo test -p shadowcat declarative_requirement_blocks_writer_without_extra_cap`
Expected: FAIL — requirements not yet enforced (the vision write currently succeeds).

- [ ] **Step 7: Wire enforcement into `apply_intent`** (`src/server/src/data/sqlite.rs`):

Load requirements next to defaults (before the tx, same single-writer reasoning):
```rust
        let world_defaults = self.world_cap_defaults(world_id).await?;
        let world_reqs = self.world_cap_requirements(world_id).await?;
```
In the `Operation::Update` authorize loop, after the base `need` check (the `if !access.has(need)` block), add the additive declared-cap check:
```rust
                        for extra in
                            crate::data::permission::declared_caps_for_path(&ch.path, &world_reqs)
                        {
                            if !access.has(extra) {
                                tracing::debug!(
                                    user = %ctx.user_id, path = %ch.path, capability = extra,
                                    "intent denied: missing declared capability"
                                );
                                return Err(DataError::Forbidden);
                            }
                        }
```

- [ ] **Step 8: Run both tests to verify they pass**

Run: `cargo test -p shadowcat declarative_requirement_blocks_writer_without_extra_cap declared_caps_match_prefix_on_boundaries`
Expected: PASS.

- [ ] **Step 9: Full server test sweep + commit**

Run: `cargo test -p shadowcat`
Expected: PASS (no regressions).

```bash
git add src/server/src/data/permission.rs src/server/src/data/sqlite.rs
git commit -m "feat(m6b): enforce declarative path->capability requirements in apply_intent (additive)"
```

---

### Task 11: Extend the `Welcome` frame + emit grants/role/requirements

**Files:**
- Modify: `src/server/src/ws/protocol.rs` (add 3 fields to `Welcome`)
- Modify: `src/server/src/ws/conn.rs` (populate them)
- Test: `src/server/src/ws/...` (a protocol serialization test) + ts-rs regen

**Interfaces:**
- Produces: `ServerMsg::Welcome { world, current_seq, server_time, world_default_grants: CapabilityGrants, actor_role: WorldRole, capability_requirements: Vec<CapabilityRequirement> }`. ts-rs regenerates `ServerMsg.ts`.

- [ ] **Step 1: Write the failing test** (serialization shape) — add to `protocol.rs` tests (create the `#[cfg(test)] mod tests` block if absent):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::document::{CapabilityGrants, WorldRole};

    #[test]
    fn welcome_carries_caps_role_and_requirements() {
        let w = ServerMsg::Welcome {
            world: uuid::Uuid::from_u128(1),
            current_seq: 0,
            server_time: 0,
            world_default_grants: CapabilityGrants::default(),
            actor_role: WorldRole::Player,
            capability_requirements: Vec::new(),
        };
        let json = serde_json::to_value(&w).unwrap();
        assert_eq!(json["type"], "welcome");
        assert_eq!(json["actor_role"], "player");
        assert!(json.get("world_default_grants").is_some());
        assert!(json.get("capability_requirements").is_some());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat welcome_carries_caps_role_and_requirements`
Expected: FAIL — `Welcome` has no such fields.

- [ ] **Step 3: Extend the `Welcome` variant** in `protocol.rs`:

```rust
    /// Sent right after a successful join. Carries the world's default capability
    /// grants, the connecting actor's world role, and the declarative capability
    /// requirements so the client can replicate access resolution for advisory
    /// UI gating (the server remains authoritative).
    Welcome {
        world: Uuid,
        current_seq: i64,
        server_time: i64,
        world_default_grants: crate::data::document::CapabilityGrants,
        actor_role: crate::data::document::WorldRole,
        capability_requirements: Vec<crate::data::document::CapabilityRequirement>,
    },
```
Ensure `protocol.rs` imports `CapabilityGrants`, `WorldRole`, `CapabilityRequirement` (or use fully-qualified paths as above). Confirm `CapabilityGrants` and `WorldRole` already derive `TS` (they appear in generated types).

- [ ] **Step 4: Populate in `conn.rs`** — replace the `Welcome` send (and load requirements alongside defaults):

```rust
    let world_defaults = repo.world_cap_defaults(world_id).await.unwrap_or_default();
    let world_reqs = repo.world_cap_requirements(world_id).await.unwrap_or_default();
    if sink
        .send(text(&ServerMsg::Welcome {
            world: world_id,
            current_seq,
            server_time: now_millis(),
            world_default_grants: world_defaults.clone(),
            actor_role: ctx.world_role,
            capability_requirements: world_reqs,
        }))
        .await
        .is_err()
    {
        return;
    }
```
(`world_defaults` is already loaded here and reused later by `send_filtered`; clone into the Welcome. `ctx.world_role` is the connecting actor's role.)

- [ ] **Step 5: Run the test + regen check**

Run: `cargo test -p shadowcat welcome_carries_caps_role_and_requirements`
Expected: PASS.
Run: `cargo test -p shadowcat` then `git status src/types/generated`
Expected: `ServerMsg.ts` (and `CapabilityRequirement.ts` from Task 9) updated by ts-rs.

- [ ] **Step 6: Commit (including regenerated bindings)**

```bash
git add src/server/src/ws/protocol.rs src/server/src/ws/conn.rs src/types/generated/ServerMsg.ts
git commit -m "feat(m6b): Welcome carries world grants, actor role, capability requirements"
```

---

### Task 12: GM-gated HTTP endpoints for world capability requirements

**Files:**
- Modify: `src/server/src/http/routes.rs` (get/set handlers + `validate_requirements`)
- Modify: `src/server/src/http/mod.rs` (route registration + a handler test)

**Interfaces:**
- Consumes: `require_gm`, `validate_capability` (routes.rs), `world_cap_requirements`/`set_world_cap_requirements` (Task 9), `CapabilityRequirement`.
- Produces: `get_world_capability_requirements` / `set_world_capability_requirements` axum handlers at `GET|PUT /worlds/:world/capability-requirements`.

- [ ] **Step 1: Write the failing handler test** in `src/server/src/http/mod.rs` tests (mirror `world_capability_defaults_enable_owner_embedded`):

```rust
#[tokio::test]
async fn set_and_get_world_capability_requirements_gm_only() {
    // Build app + GM session per the existing helper pattern in this module.
    let (app, st, gm_cookie, _player_cookie, world) = setup_world_with_gm_and_player().await;

    let body = serde_json::json!([
        { "path_prefix": "/system/vision", "caps": ["dnd5e:gm_vision"] }
    ]);
    let put = app.clone().oneshot(
        request_put(&format!("/worlds/{world}/capability-requirements"), &gm_cookie, &body),
    ).await.unwrap();
    assert_eq!(put.status(), StatusCode::NO_CONTENT);

    let stored = st.repo.world_cap_requirements(world).await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].path_prefix, "/system/vision");
}
```
(Use the module's existing request/cookie helpers; if `setup_world_with_gm_and_player` does not exist, reuse whatever fixture `world_capability_defaults_enable_owner_embedded` uses and adapt.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat set_and_get_world_capability_requirements_gm_only`
Expected: FAIL — route/handlers absent.

- [ ] **Step 3: Add handlers** in `routes.rs` (after the defaults handlers):

```rust
/// A world's declarative capability requirements. GM/admin only.
pub async fn get_world_capability_requirements(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
) -> Result<Json<Vec<crate::data::document::CapabilityRequirement>>, AppError> {
    require_gm(&state, &user, world).await?;
    Ok(Json(state.repo.world_cap_requirements(world).await?))
}

/// Replace a world's declarative capability requirements. GM/admin only.
pub async fn set_world_capability_requirements(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
    Json(reqs): Json<Vec<crate::data::document::CapabilityRequirement>>,
) -> Result<StatusCode, AppError> {
    require_gm(&state, &user, world).await?;
    for req in &reqs {
        if !req.path_prefix.starts_with('/') {
            return Err(AppError::bad_request("path_prefix must start with /"));
        }
        for token in &req.caps {
            validate_capability(token)?;
        }
    }
    state.repo.set_world_cap_requirements(world, &reqs).await?;
    Ok(StatusCode::NO_CONTENT)
}
```
(Use the same `AppError` bad-request constructor the file already uses; match its exact signature.)

- [ ] **Step 4: Register the route** in `src/server/src/http/mod.rs` next to the defaults route:

```rust
        .route(
            "/worlds/{world}/capability-requirements",
            get(routes::get_world_capability_requirements)
                .put(routes::set_world_capability_requirements),
        )
```
(Match the existing path-param syntax in this file — `{world}` vs `:world` — as used by the defaults route.)

- [ ] **Step 5: Run the test + commit**

Run: `cargo test -p shadowcat set_and_get_world_capability_requirements_gm_only`
Expected: PASS.
Run: `cargo test -p shadowcat && cargo clippy --all-targets -- -D warnings && cargo fmt --all -- --check`
Expected: PASS / clean.

```bash
git add src/server/src/http/routes.rs src/server/src/http/mod.rs
git commit -m "feat(m6b): GM-gated endpoints for world capability requirements"
```

---

## Slice 4 — Client capability-awareness + wire extension

### Task 13: Extend the wire `Welcome` schema + `WsClient.onWelcome`

**Files:**
- Modify: `src/client/core/src/wire.ts` (welcome schema + `CapabilityRequirementSchema`)
- Modify: `src/client/core/src/ws-client.ts` (`onWelcome` payload)
- Modify: `src/client/core/src/wire.test.ts`, `src/client/core/src/ws-client.test.ts`

**Interfaces:**
- Produces:
  - `WireCapabilityRequirement = { path_prefix: string; caps: string[] }`, `CapabilityRequirementSchema`.
  - `WireWelcome = { world: string; current_seq: number; server_time: number; world_default_grants: { by_role; by_user }; actor_role: WorldRole; capability_requirements: WireCapabilityRequirement[] }`.
  - `WsClientHandlers.onWelcome?(welcome: WireWelcome): void` (signature change from `(world, currentSeq)`).

- [ ] **Step 1: Update the drift/parse test** in `wire.test.ts` — assert the new welcome fields parse:

```ts
test("welcome parses grants, actor_role, and requirements", () => {
  const msg = parseServerMsg(
    JSON.stringify({
      type: "welcome",
      world: "w1",
      current_seq: 0,
      server_time: 0,
      world_default_grants: { by_role: {}, by_user: {} },
      actor_role: "player",
      capability_requirements: [{ path_prefix: "/system/vision", caps: ["dnd5e:gm_vision"] }],
    }),
  );
  expect(msg?.type).toBe("welcome");
  if (msg?.type === "welcome") {
    expect(msg.actor_role).toBe("player");
    expect(msg.capability_requirements[0].path_prefix).toBe("/system/vision");
  }
});
```
Also update any existing welcome fixture in `wire.test.ts`/`ws-client.test.ts` that builds a `welcome` frame to include the three new fields (otherwise `safeParse` now rejects them).

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/core test -- wire`
Expected: FAIL — new fields rejected / missing.

- [ ] **Step 3: Extend `wire.ts`** — add the requirement schema and the welcome fields:

```ts
export const CapabilityRequirementSchema = z.object({
  path_prefix: z.string(),
  caps: z.array(z.string()),
});
export type WireCapabilityRequirement = z.infer<typeof CapabilityRequirementSchema>;
```
Replace the welcome member of `ServerMsgSchema`:
```ts
  z.object({
    type: z.literal("welcome"),
    world: z.string(),
    current_seq: int,
    server_time: int,
    world_default_grants: CapabilityGrantsSchema,
    actor_role: WorldRoleSchema,
    capability_requirements: z.array(CapabilityRequirementSchema),
  }),
```

- [ ] **Step 4: Update `ws-client.ts`** — change the handler signature and the call site:

In `WsClientHandlers`:
```ts
  onWelcome?(welcome: Extract<import("./wire").ServerMsg, { type: "welcome" }>): void;
```
In `handleFrame` `case "welcome":`:
```ts
      case "welcome":
        this.serverOffsetMs = msg.server_time - this.now();
        this.opts.handlers.onWelcome?.(msg);
        if (msg.current_seq >= this.nextExpected) {
          this.send({ type: "resync_request", from_seq: this.nextExpected });
        }
        break;
```
Update `ws-client.test.ts` `onWelcome` assertions to read `welcome.world` / `welcome.current_seq` from the object.

- [ ] **Step 5: Run tests + typecheck to verify they pass**

Run: `pnpm --filter @shadowcat/core test && pnpm --filter @shadowcat/core typecheck`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/client/core/src/wire.ts src/client/core/src/wire.test.ts src/client/core/src/ws-client.ts src/client/core/src/ws-client.test.ts
git commit -m "feat(m6b): wire Welcome carries grants/role/requirements; onWelcome passes full frame"
```

---

### Task 14: Client capability resolution (`capabilities.ts`)

**Files:**
- Create: `src/client/core/src/capabilities.ts`
- Test: `src/client/core/src/capabilities.test.ts`
- Modify: `src/client/core/src/index.ts`

**Interfaces:**
- Consumes: `WireDocument`, `PermissionSetSchema` type, `WireCapabilityRequirement` (wire.ts), `WorldRole` (`@shadowcat/types`).
- Produces:
  - `function resolveCaps(perms: WireDocument["permissions"], userId: string, role: WorldRole, worldGrants: { by_role: Record<string, string[]>; by_user: Record<string, string[]> }): Set<string>` — mirrors the server `resolve_access_world` (GM ⇒ all; else DocRole floor + doc grants + world grants).
  - `function canWritePath(path: string, caps: Set<string>, isGm: boolean, requirements: WireCapabilityRequirement[]): boolean` — base path cap (`/system`→`core:write_fields`, `/embedded`→`core:manage_embedded`, `/permissions`→`core:edit_permissions`, else false) plus every declared cap for the most-specific matching requirement; GM bypasses.

> The floor mirrors server `role_floor`: Owner ⇒ {core:read, core:write_fields}; Observer ⇒ {core:read}; None ⇒ {}. GM ⇒ all (represented by `canWritePath` short-circuit + `resolveCaps` returning a sentinel-free full bypass via the `isGm` flag; `resolveCaps` itself returns the non-GM set, and callers pass `role === "gm"` to `canWritePath`).

- [ ] **Step 1: Write the failing tests**

```ts
// src/client/core/src/capabilities.test.ts
import { expect, test } from "vitest";
import { resolveCaps, canWritePath } from "./capabilities";
import type { WireDocument } from "./wire";

const emptyGrants = { by_role: {}, by_user: {} };

function perms(p: Partial<WireDocument["permissions"]>): WireDocument["permissions"] {
  return { default: "none", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, ...p };
}

test("owner floor is read + write_fields", () => {
  const caps = resolveCaps(perms({ users: { u1: "owner" } }), "u1", "player", emptyGrants);
  expect(caps.has("core:read")).toBe(true);
  expect(caps.has("core:write_fields")).toBe(true);
  expect(caps.has("core:manage_embedded")).toBe(false);
});

test("world grant widens the floor", () => {
  const caps = resolveCaps(
    perms({ users: { u1: "owner" } }),
    "u1",
    "player",
    { by_role: { owner: ["core:manage_embedded"] }, by_user: {} },
  );
  expect(caps.has("core:manage_embedded")).toBe(true);
});

test("canWritePath enforces base cap", () => {
  const caps = new Set(["core:read", "core:write_fields"]);
  expect(canWritePath("/system/hp", caps, false, [])).toBe(true);
  expect(canWritePath("/embedded/x", caps, false, [])).toBe(false); // needs manage_embedded
  expect(canWritePath("/id", caps, false, [])).toBe(false); // immutable envelope
});

test("canWritePath enforces declared requirement additively", () => {
  const caps = new Set(["core:read", "core:write_fields"]);
  const reqs = [{ path_prefix: "/system/vision", caps: ["dnd5e:gm_vision"] }];
  expect(canWritePath("/system/vision/range", caps, false, reqs)).toBe(false);
  const withVision = new Set([...caps, "dnd5e:gm_vision"]);
  expect(canWritePath("/system/vision/range", withVision, false, reqs)).toBe(true);
});

test("GM bypasses all checks", () => {
  expect(canWritePath("/system/vision", new Set(), true, [
    { path_prefix: "/system/vision", caps: ["dnd5e:gm_vision"] },
  ])).toBe(true);
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/core test -- capabilities`
Expected: FAIL — cannot find module `./capabilities`.

- [ ] **Step 3: Write the implementation**

```ts
// src/client/core/src/capabilities.ts
// Client-side mirror of the server's capability resolution (resolve_access_world
// + required_cap_for_path + declarative requirements). ADVISORY ONLY: used to
// gate module UI/actions for UX. The server remains authoritative — a bypass is
// rejected at apply_intent.
import type { WorldRole } from "@shadowcat/types";
import type { WireDocument, WireCapabilityRequirement } from "./wire";

type Grants = { by_role: Record<string, string[]>; by_user: Record<string, string[]> };
type Perms = WireDocument["permissions"];

function roleFloor(role: string): string[] {
  switch (role) {
    case "owner":
      return ["core:read", "core:write_fields"];
    case "observer":
      return ["core:read"];
    default:
      return [];
  }
}

export function resolveCaps(
  perms: Perms,
  userId: string,
  role: WorldRole,
  worldGrants: Grants,
): Set<string> {
  // GM/admin holds everything; callers short-circuit via the isGm flag in
  // canWritePath. This returns the concrete non-GM set.
  const docRole = perms.users[userId] ?? perms.default;
  const caps = new Set<string>(roleFloor(docRole));
  for (const c of perms.capabilities.by_role[docRole] ?? []) caps.add(c);
  for (const c of perms.capabilities.by_user[userId] ?? []) caps.add(c);
  for (const c of worldGrants.by_role[docRole] ?? []) caps.add(c);
  for (const c of worldGrants.by_user[userId] ?? []) caps.add(c);
  return caps;
}

function baseCapForPath(path: string): string | null {
  if (path === "/system" || path.startsWith("/system/")) return "core:write_fields";
  if (path === "/embedded" || path.startsWith("/embedded/")) return "core:manage_embedded";
  if (path === "/permissions" || path.startsWith("/permissions/")) return "core:edit_permissions";
  return null;
}

export function canWritePath(
  path: string,
  caps: Set<string>,
  isGm: boolean,
  requirements: WireCapabilityRequirement[],
): boolean {
  if (isGm) return true;
  const base = baseCapForPath(path);
  if (base === null || !caps.has(base)) return false;
  for (const req of requirements) {
    if (path === req.path_prefix || path.startsWith(`${req.path_prefix}/`)) {
      for (const c of req.caps) if (!caps.has(c)) return false;
    }
  }
  return true;
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/core test -- capabilities`
Expected: PASS.

- [ ] **Step 5: Export + full client sweep + commit**

Add to `index.ts`:
```ts
export { resolveCaps, canWritePath } from "./capabilities";
```
Run: `pnpm --filter @shadowcat/core test && pnpm --filter @shadowcat/core typecheck`
Expected: PASS.

```bash
git add src/client/core/src/capabilities.ts src/client/core/src/capabilities.test.ts src/client/core/src/index.ts
git commit -m "feat(m6b): client capability-awareness (advisory resolve + canWritePath)"
```

---

## Slice 5 — Node↔Rust end-to-end harness + CI job

### Task 15: Seed a richer fixture in the test server

**Files:**
- Modify: `src/server/src/bin/test_server.rs`

**Interfaces:**
- Produces: the test server seeds a GM (`gm`/`pw`), a player (`pl`/`pw`) who is a world member, one player-owned document with `system: { vision: { range: 30 }, hp: 10 }`, and a world capability requirement `{/system/vision → [dnd5e:gm_vision]}`. It prints a machine-readable line `e2e-fixture: {json}` with the world id, doc id, and user ids.

- [ ] **Step 1: Extend `test_server.rs`** to build the fixture:

```rust
// after creating repo + world, before AppState:
use shadowcat::data::command::Operation;
use shadowcat::data::document::{
    CapabilityRequirement, DocRole, Document, PermissionSet, Scope, WorldRole,
};
use shadowcat::data::membership::PermissionContext;
use std::collections::BTreeSet;

let hash = hash_password("pw")?;
let gm = repo.create_user("gm", Some(&hash), ServerRole::User, 0).await?;
let player = repo.create_user("pl", Some(&hash), ServerRole::User, 0).await?;
repo.add_member(world.id, gm, WorldRole::Gm).await?;
repo.add_member(world.id, player, WorldRole::Player).await?;

let mut perms = PermissionSet::default();
perms.users.insert(player, DocRole::Owner);
let doc = Document {
    id: uuid::Uuid::new_v4(),
    scope: Scope::World { world_id: world.id },
    doc_type: "actor".into(),
    schema_version: 1,
    source: None,
    owner: Some(player),
    permissions: perms,
    embedded: Default::default(),
    system: serde_json::json!({ "vision": { "range": 30 }, "hp": 10 }),
    created_at: 0,
    updated_at: 0,
};
let gm_ctx = PermissionContext { user_id: gm, world_role: WorldRole::Gm };
repo.apply_intent(&gm_ctx, world.id, vec![Operation::Create { doc: doc.clone() }], 0).await?;
repo.set_world_cap_requirements(world.id, &[CapabilityRequirement {
    path_prefix: "/system/vision".into(),
    caps: BTreeSet::from(["dnd5e:gm_vision".to_string()]),
}]).await?;

println!(
    "e2e-fixture: {}",
    serde_json::json!({
        "world": world.id, "doc": doc.id, "gm": gm, "player": player
    })
);
```
(Keep the existing `create_user("u", ...)` seed or replace it — the harness only needs `gm`/`pl`. Remove the now-duplicate single-user seed if it conflicts.)

- [ ] **Step 2: Verify it builds + runs**

Run: `cargo run -p shadowcat --bin test_server` (Ctrl-C after the lines print)
Expected: prints `test_server: http://… world=…` and `e2e-fixture: {…}`.

- [ ] **Step 3: Commit**

```bash
git add src/server/src/bin/test_server.rs
git commit -m "test(m6b): seed gm/player/doc/requirement fixture in test_server for e2e"
```

---

### Task 16: e2e harness — real client drives the real server

**Files:**
- Create: `src/client/core/src/e2e/server-process.ts` (spawn + parse helpers)
- Create: `src/client/core/src/e2e/capabilities.e2e.test.ts`
- Create: `src/client/core/src/e2e/README.md`
- Modify: `src/client/core/package.json` (add `test:e2e` script + `ws` devDependency)
- Modify: `src/client/core/vitest config` (exclude `*.e2e.test.ts` from the default `test` run)

**Interfaces:**
- Consumes: `WsClient` (ws-client.ts), the `e2e-fixture` line (Task 15).
- Produces: `startTestServer(): Promise<{ baseUrl: string; fixture: { world; doc; gm; player }; stop(): void }>`; a `login(baseUrl, user, pw): Promise<string>` returning the session cookie; a Node `Transport` over the `ws` package.

> Environment: this test requires the Rust toolchain and is excluded from the default `web` CI job. It runs in the new combined job (Task 17). Default `pnpm -r test` must NOT pick up `*.e2e.test.ts`.

- [ ] **Step 1: Add the `ws` devDependency + scripts** to `src/client/core/package.json`:

```json
  "scripts": {
    "typecheck": "tsc --noEmit",
    "test": "vitest run --exclude '**/*.e2e.test.ts'",
    "test:e2e": "vitest run **/*.e2e.test.ts"
  },
  "devDependencies": {
    "ws": "^8.18.0",
    "@types/ws": "^8.5.12"
  }
```
Run: `pnpm install`

- [ ] **Step 2: Write the spawn/login helper** (`src/client/core/src/e2e/server-process.ts`):

```ts
// Spawns the Rust test_server (release build), parses its printed bind address
// and e2e fixture, and exposes login + teardown. Node-only; used by *.e2e.test.ts.
import { spawn, type ChildProcess } from "node:child_process";

export interface Fixture { world: string; doc: string; gm: string; player: string; }
export interface TestServer { baseUrl: string; wsUrl: string; fixture: Fixture; stop(): void; }

export async function startTestServer(): Promise<TestServer> {
  const proc: ChildProcess = spawn(
    "cargo",
    ["run", "--release", "-p", "shadowcat", "--bin", "test_server"],
    { stdio: ["ignore", "pipe", "inherit"] },
  );
  let baseUrl = "";
  let fixture: Fixture | null = null;
  await new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("test_server did not start in time")), 120_000);
    let buf = "";
    proc.stdout!.on("data", (chunk: Buffer) => {
      buf += chunk.toString();
      const addr = /test_server: (http:\/\/[\d.:]+)/.exec(buf);
      if (addr) baseUrl = addr[1];
      const fx = /e2e-fixture: (\{.*\})/.exec(buf);
      if (fx) fixture = JSON.parse(fx[1]);
      if (baseUrl && fixture) {
        clearTimeout(timer);
        resolve();
      }
    });
    proc.on("exit", (code) => reject(new Error(`test_server exited early (${code})`)));
  });
  const wsUrl = baseUrl.replace(/^http/, "ws") + "/ws";
  return { baseUrl, wsUrl, fixture: fixture!, stop: () => proc.kill() };
}

/** Log in via the HTTP auth route; returns the session cookie header value. */
export async function login(baseUrl: string, username: string, password: string): Promise<string> {
  const res = await fetch(`${baseUrl}/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ username, password }),
  });
  if (!res.ok) throw new Error(`login failed: ${res.status}`);
  const cookie = res.headers.get("set-cookie");
  if (!cookie) throw new Error("no session cookie returned");
  return cookie.split(";")[0];
}
```
(Verify the login route path + payload field names against `src/server/src/http/routes.rs` / `auth/` and adjust the URL/body to match exactly.)

- [ ] **Step 3: Write the e2e test** (`src/client/core/src/e2e/capabilities.e2e.test.ts`):

```ts
import { afterAll, beforeAll, expect, test } from "vitest";
import WebSocket from "ws";
import { WsClient } from "../ws-client";
import type { Transport, TransportHandlers } from "../transport";
import type { ClientMsg, RejectReason } from "../wire";
import { startTestServer, login, type TestServer } from "./server-process";

let server: TestServer;
beforeAll(async () => {
  server = await startTestServer();
}, 180_000);
afterAll(() => server?.stop());

function nodeConnect(wsUrl: string, world: string, cookie: string) {
  return (handlers: TransportHandlers): Promise<Transport> =>
    new Promise((resolve, reject) => {
      const sock = new WebSocket(`${wsUrl}?world=${world}`, { headers: { cookie } });
      sock.on("open", () =>
        resolve({
          send: (d: string) => sock.send(d),
          close: () => sock.close(),
        }),
      );
      sock.on("message", (d) => handlers.onMessage(d.toString()));
      sock.on("close", () => handlers.onClose());
      sock.on("error", reject);
    });
}

test("player is rejected writing a GM-gated path; the gated write never lands", async () => {
  const cookie = await login(server.baseUrl, "pl", "pw");
  const { world, doc } = server.fixture;

  let rejected: RejectReason | null = null;
  let welcomeSeen = false;
  const client = new WsClient({
    connect: nodeConnect(server.wsUrl, world, cookie),
    handlers: {
      onCommand: () => {},
      onReject: (_id, reason) => {
        rejected = reason;
      },
      onWelcome: (w) => {
        welcomeSeen = true;
        // The extended Welcome carries the requirement + the player's role.
        expect(w.actor_role).toBe("player");
        expect(w.capability_requirements.some((r) => r.path_prefix === "/system/vision")).toBe(true);
      },
    },
  });
  await client.start();
  await new Promise((r) => setTimeout(r, 500));
  expect(welcomeSeen).toBe(true);

  // Attempt the gated write directly over the wire.
  const intent: ClientMsg = {
    type: "intent",
    intent_id: "11111111-1111-1111-1111-111111111111",
    ops: [{ op: "update", doc_id: doc, changes: [{ path: "/system/vision/range", old: 30, new: 60 }] }],
  };
  client.send(intent);
  await new Promise((r) => setTimeout(r, 500));
  expect(rejected).toBe("forbidden");

  client.stop();
});
```
(Adjust the WS URL/query/cookie auth to match the M5 `/ws` handshake. If the harness needs the GM to have created the requirement at runtime instead of via the seed, log in as `gm` and `PUT /worlds/:world/capability-requirements` first.)

- [ ] **Step 4: Write `src/client/core/src/e2e/README.md`** documenting: requires the Rust toolchain; run with `pnpm --filter @shadowcat/core test:e2e`; excluded from the default `web` CI job; covered by the `e2e` CI job.

- [ ] **Step 5: Run the e2e suite locally**

Run: `pnpm --filter @shadowcat/core test:e2e`
Expected: PASS (player write → `forbidden`).
Run: `pnpm --filter @shadowcat/core test` (default)
Expected: PASS and does NOT execute the e2e file.

- [ ] **Step 6: Commit**

```bash
git add src/client/core/src/e2e src/client/core/package.json pnpm-lock.yaml
git commit -m "test(m6b): Node<->Rust e2e harness — real client driven against real server"
```

---

### Task 17: Combined CI job (both toolchains)

**Files:**
- Modify: `.github/workflows/ci.yml` (add an `e2e` job)

**Interfaces:**
- Produces: a single-OS `e2e` job (ubuntu) with Rust + Node that runs `pnpm --filter @shadowcat/core test:e2e`.

- [ ] **Step 1: Add the job** to `.github/workflows/ci.yml`:

```yaml
  e2e:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - uses: dtolnay/rust-toolchain@stable
      - uses: pnpm/action-setup@v6
        with:
          version: 9
      - uses: actions/setup-node@v6
        with:
          node-version: 22
          cache: pnpm
      - run: pnpm install --frozen-lockfile
      # Warm the release build so the in-test spawn does not hit the 120s timeout.
      - run: cargo build --release -p shadowcat --bin test_server
      - run: pnpm --filter @shadowcat/core test:e2e
```

- [ ] **Step 2: Validate the workflow YAML locally** (syntax)

Run: `node -e "require('js-yaml')" 2>/dev/null || true` then visually confirm indentation, or use any installed YAML linter.
Expected: well-formed (the job mirrors the existing `web`/`rust` jobs).

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci(m6b): combined Rust+Node e2e job for the cross-runtime harness"
```

---

## Slice 6 — Documentation sync

### Task 18: Update tracking docs

**Files:**
- Modify: `docs/PLAN.md` (mark M6b ✅, point to the spec)
- Modify: `docs/TODO.md` (remove the now-built Node↔Rust harness item; note the TS-lint item status)
- Modify: `docs/design/ARCHITECTURE.md` only if a new invariant emerged (none expected — the module system honors existing invariants)
- Modify: `docs/POST_WORK_FINDINGS.md` if the final review surfaced anything

- [ ] **Step 1: Mark M6b complete** in `docs/PLAN.md` (`#### M6b · Modules + capabilities (declarative) ✅`) and ensure the line references `docs/superpowers/specs/2026-06-18-m6b-modules-capabilities-design.md`.

- [ ] **Step 2: Remove the built TODO** — delete the "Node↔Rust client/server end-to-end test" item from `docs/TODO.md` (now delivered in Slice 5).

- [ ] **Step 3: Final verification sweep**

Run: `cargo test -p shadowcat && cargo clippy --all-targets -- -D warnings && cargo fmt --all -- --check`
Run: `pnpm -r typecheck && pnpm -r test && pnpm lint`
Expected: all PASS / clean.

- [ ] **Step 4: Commit + run graphify update**

```bash
git add docs/PLAN.md docs/TODO.md
git commit -m "docs(m6b): mark M6b complete; retire delivered e2e TODO"
```
Run: `graphify update .`

---

## Self-Review

**Spec coverage:**
- §4 Hook system → Tasks 1–3 (logger, semver, HookBus: 3 kinds, versioning, isolation, cleanup, typed overlay `CoreHooks`). ✓
- §5 Service registry → Task 4. ✓
- §6 Middleware (both pipelines) → Task 5. ✓
- §7 Manifest + ModuleRegistry (topo-sort, semver, hot-unload, ModuleContext chokepoint, local registry via `list`) → Tasks 6–7. ✓
- §8 Loader adapter → Task 8. ✓
- §9.1 Server data record + additive `apply_intent` enforcement → Tasks 9–10. ✓
- §9.2 Welcome extension + ts-rs regen → Task 11. ✓
- §9.1 GM-gated write op → Task 12. ✓
- §9.3 Client cap-awareness → Tasks 13–14. ✓
- §12 Testing: unit (Tasks 1–8, 14), Rust integration (Tasks 9–12), Node↔Rust e2e + CI (Tasks 15–17). ✓
- §13 slices ↔ this plan's slices 1–6. ✓
- §15 open decisions: both middleware pipelines (Task 5), `requires` omitted = any version (Task 3 `on`), flat per-world requirements (Task 9). ✓

**Placeholder scan:** No "TBD"/"handle edge cases"/"similar to". Each code step shows full code. The few "match the existing helper/route syntax" notes are concrete verification instructions against named files, not deferred work.

**Type consistency:** `CapRequirement {path_prefix, caps}` (TS) ↔ `CapabilityRequirement {path_prefix, caps}` (Rust, ts-rs-exported) — names match field-for-field; the TS wire type is `WireCapabilityRequirement` (validated separately in `wire.ts`). `HookKind`/`HookDefinition`/`OnOptions`/`Handler` defined in Task 3 and reused in Tasks 6–7. `ModuleContext.hooks.on` injects `module` consistently with `HookBus.on`'s `OnOptions.module`. `resolveCaps`/`canWritePath` signatures match between Task 14 def and its test. `onWelcome(welcome)` change (Task 13) is propagated to the e2e harness (Task 16).

## Buddy-check directives

Task 10 (declarative capability enforcement in `apply_intent`) is **security-critical**: a defect is a privilege-escalation / authorization-bypass. Task 11 (Welcome now ships world grants + actor role to every client) widens what data crosses the trust boundary. At the execution handoff, OFFER a buddy-check (two independent blind reviewers + reconciliation per `superpowers:buddy-checking`) scoped to Tasks 9–12 (the server capability change set) before merge. The rest of the branch (pure-TS primitives with thorough unit tests) is suitable for the standard single final-branch review. The human decides whether to take the buddy-check offer.
