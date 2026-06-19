import { expect, test, vi } from "vitest";
import { ModuleRegistry, type Module } from "./modules";
import { HookBus } from "./hooks";
import { ServiceRegistry } from "./services";
import { MiddlewareChain } from "./middleware";
import { DocumentStore } from "./store";
import { OptimisticClient } from "./optimistic";
import { silentLogger } from "./logger";

function deps() {
  return {
    hooks: new HookBus(silentLogger),
    services: new ServiceRegistry(),
    middleware: new MiddlewareChain(),
    store: new DocumentStore(),
    client: new OptimisticClient("self"),
    logger: silentLogger,
  };
}

function mod(id: string, dependencies: Record<string, string>, register = vi.fn()): Module {
  return { manifest: { id, version: "1.0.0", dependencies }, register };
}

test("activate calls register in dependency order", async () => {
  const order: string[] = [];
  const r = new ModuleRegistry(deps());
  r.add(mod("b", { a: "^1.0.0" }, vi.fn(() => order.push("b"))));
  r.add(mod("a", {}, vi.fn(() => order.push("a"))));
  await r.activate();
  expect(order).toEqual(["a", "b"]);
});

test("missing dependency skips the module and its dependents", async () => {
  const r = new ModuleRegistry(deps());
  const reg = vi.fn();
  r.add(mod("needs-missing", { ghost: "^1.0.0" }, reg));
  await r.activate();
  expect(reg).not.toHaveBeenCalled();
  expect(r.list().find((m) => m.id === "needs-missing")!.active).toBe(false);
});

test("incompatible dependency version is rejected", async () => {
  const r = new ModuleRegistry(deps());
  r.add(mod("a", {})); // a@1.0.0
  const reg = vi.fn();
  r.add(mod("b", { a: "^2.0.0" }, reg));
  await r.activate();
  expect(reg).not.toHaveBeenCalled();
});

test("dependency cycle throws with the cycle path", async () => {
  const r = new ModuleRegistry(deps());
  r.add(mod("a", { b: "^1.0.0" }));
  r.add(mod("b", { a: "^1.0.0" }));
  await expect(r.activate()).rejects.toThrow(/cycle/i);
});

test("invalid manifest is rejected at add()", () => {
  const r = new ModuleRegistry(deps());
  expect(() =>
    r.add({ manifest: { id: "", version: "1.0.0", dependencies: {} }, register: vi.fn() }),
  ).toThrow();
});

test("collectRequirements unions active modules' requirements", async () => {
  const r = new ModuleRegistry(deps());
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
  r.add({
    manifest: { id: "b", version: "1.0.0", dependencies: { a: "^1.0.0" } },
    register: vi.fn(),
  });
  await r.activate();

  await expect(r.unload("a")).rejects.toThrow(/depend/i);
  await r.unload("a", { cascade: true });
  expect(d.services.has("a:svc")).toBe(false);
  expect(r.list().find((m) => m.id === "a")!.active).toBe(false);
  expect(r.list().find((m) => m.id === "b")!.active).toBe(false);
});
