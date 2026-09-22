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
  b.on("core:test", (p) => {
    seen.push(p as number);
  });
  b.on("core:test", async (p) => {
    seen.push((p as number) + 1);
  });
  await b.emitInfo("core:test", 10);
  expect(seen.sort()).toEqual([10, 11]);
});

test("emitMutate chains payload by priority then registration", async () => {
  const b = bus();
  b.defineHook("core:m", { version: "1.0.0", kind: "mutate" });
  b.on("core:m", (n) => (n as number) + 1, { priority: 0 });
  b.on("core:m", (n) => (n as number) * 10, { priority: 10 }); // higher priority first
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

test("emitCancel halts on the STOP sentinel", async () => {
  const b = bus();
  b.defineHook("core:c", { version: "1.0.0", kind: "cancel" });
  b.on("core:c", () => STOP, { module: "stopper" });
  const after = vi.fn();
  b.on("core:c", after);
  const r = await b.emitCancel("core:c", {});
  expect(r).toEqual({ cancelled: true, by: "stopper" });
  expect(after).not.toHaveBeenCalled();
});

test("a throwing handler is isolated and does not abort the chain", async () => {
  const log = { debug: vi.fn(), warn: vi.fn(), error: vi.fn() };
  const b = new HookBus(log);
  b.defineHook("core:m", { version: "1.0.0", kind: "mutate" });
  b.on("core:m", () => {
    throw new Error("boom");
  });
  b.on("core:m", (n) => (n as number) + 5);
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

test("a throwing emitInfo listener fires core:hook-fault with the hook name and error", async () => {
  const b = bus();
  b.defineHook("core:i", { version: "1.0.0", kind: "info" });
  const err = new Error("boom");
  b.on("core:i", () => {
    throw err;
  });
  const faults: unknown[] = [];
  b.on("core:hook-fault", (p) => faults.push(p));
  await b.emitInfo("core:i", null);
  expect(faults).toEqual([{ hook: "core:i", error: err }]);
});

test("a throwing emitMutate listener fires core:hook-fault", async () => {
  const b = bus();
  b.defineHook("core:m2", { version: "1.0.0", kind: "mutate" });
  const err = new Error("boom-mutate");
  b.on("core:m2", () => {
    throw err;
  });
  const faults: unknown[] = [];
  b.on("core:hook-fault", (p) => faults.push(p));
  await b.emitMutate("core:m2", 1);
  expect(faults).toEqual([{ hook: "core:m2", error: err }]);
});

test("a throwing emitCancel listener fires core:hook-fault", async () => {
  const b = bus();
  b.defineHook("core:c2", { version: "1.0.0", kind: "cancel" });
  const err = new Error("boom-cancel");
  b.on("core:c2", () => {
    throw err;
  });
  const faults: unknown[] = [];
  b.on("core:hook-fault", (p) => faults.push(p));
  await b.emitCancel("core:c2", {});
  expect(faults).toEqual([{ hook: "core:c2", error: err }]);
});

test("core:hook-fault is pre-declared: no defineHook call is needed before listening", async () => {
  const b = bus();
  const fn = vi.fn();
  b.on("core:hook-fault", fn);
  b.defineHook("core:i2", { version: "1.0.0", kind: "info" });
  b.on("core:i2", () => {
    throw new Error("x");
  });
  await b.emitInfo("core:i2", null);
  expect(fn).toHaveBeenCalledTimes(1);
});

test("a fault inside the fault hook's own listener does not re-enter (no infinite loop, no duplicate fault)", async () => {
  const log = { debug: vi.fn(), warn: vi.fn(), error: vi.fn() };
  const b = new HookBus(log);
  let faultCalls = 0;
  b.on("core:hook-fault", () => {
    faultCalls++;
    throw new Error("fault handler itself throws");
  });
  b.defineHook("core:i3", { version: "1.0.0", kind: "info" });
  b.on("core:i3", () => {
    throw new Error("original fault");
  });
  await b.emitInfo("core:i3", null);
  // Exactly one fault dispatch for the original throw; the fault-hook listener's own throw is
  // isolated by the ordinary emitInfo catch and must not trigger a second core:hook-fault.
  expect(faultCalls).toBe(1);
});

test("two throwing listeners on the SAME hook each produce their own core:hook-fault dispatch", async () => {
  const b = bus();
  b.defineHook("core:i4", { version: "1.0.0", kind: "info" });
  const err1 = new Error("first");
  const err2 = new Error("second");
  b.on("core:i4", () => {
    throw err1;
  });
  b.on("core:i4", () => {
    throw err2;
  });
  const faults: unknown[] = [];
  b.on("core:hook-fault", (p) => faults.push(p));
  await b.emitInfo("core:i4", null);
  // A persistent "one fault at a time" flag would drop the second listener's fault because the
  // first fault dispatch (itself awaiting core:hook-fault's own listeners) is still marked
  // in-flight when the second throw occurs synchronously afterward in the same outer loop.
  expect(faults).toEqual([
    { hook: "core:i4", error: err1 },
    { hook: "core:i4", error: err2 },
  ]);
});

test("a never-resolving core:hook-fault listener does not block a later, independent fault dispatch from being attempted", async () => {
  const b = bus();
  const faultCalls: unknown[] = [];
  b.on("core:hook-fault", (p) => {
    faultCalls.push(p);
    return new Promise(() => {}); // never resolves
  });
  b.defineHook("core:h1", { version: "1.0.0", kind: "info" });
  b.defineHook("core:h2", { version: "1.0.0", kind: "info" });
  const err1 = new Error("first");
  const err2 = new Error("second");
  b.on("core:h1", () => {
    throw err1;
  });
  b.on("core:h2", () => {
    throw err2;
  });
  // The first fault dispatch hangs forever on the listener above; a persistent in-flight flag
  // would never clear (the listener's promise never resolves) and would silently drop every
  // later fault for the rest of the session.
  await b.emitInfo("core:h1", null);
  await b.emitInfo("core:h2", null);
  expect(faultCalls).toEqual([
    { hook: "core:h1", error: err1 },
    { hook: "core:h2", error: err2 },
  ]);
});
