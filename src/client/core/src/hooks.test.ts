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
