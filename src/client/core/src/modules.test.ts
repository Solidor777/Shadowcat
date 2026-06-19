import { expect, test, vi } from "vitest";
import { ModuleRegistry, type Module } from "./modules";
import { HookBus } from "./hooks";
import { ServiceRegistry } from "./services";
import { MiddlewareChain } from "./middleware";
import { DocumentStore } from "./store";
import { OptimisticClient } from "./optimistic";
import { ContributionRegistry } from "./contributions";
import { silentLogger } from "./logger";

function deps() {
  return {
    hooks: new HookBus(silentLogger),
    services: new ServiceRegistry(),
    middleware: new MiddlewareChain(),
    store: new DocumentStore(),
    client: new OptimisticClient("self"),
    logger: silentLogger,
    contributions: new ContributionRegistry(),
  };
}

function mod(id: string, dependencies: Record<string, string>, register = vi.fn()): Module {
  return { manifest: { id, version: "1.0.0", dependencies }, register };
}

test("activates a contract provider before a requirer (topological by contract)", async () => {
  const order: string[] = [];
  const r = new ModuleRegistry(deps());
  r.add({
    manifest: { id: "combat", version: "1.0.0", dependencies: {}, requires: ["s:sidebar"] },
    register: vi.fn(() => {
      order.push("combat");
    }),
  });
  r.add({
    manifest: {
      id: "sidebar", version: "1.0.0", dependencies: {},
      provides: [{ contract: "s:sidebar", cardinality: "singleton" }],
    },
    register: vi.fn(() => {
      order.push("sidebar");
    }),
  });
  await r.activate();
  expect(order).toEqual(["sidebar", "combat"]);
});

test("does not activate a module whose required contract has no provider", async () => {
  const r = new ModuleRegistry(deps());
  const reg = vi.fn();
  r.add({
    manifest: { id: "combat", version: "1.0.0", dependencies: {}, requires: ["s:missing"] },
    register: reg,
  });
  await r.activate();
  expect(reg).not.toHaveBeenCalled();
  expect(r.list().find((m) => m.id === "combat")!.active).toBe(false);
});

test("throws when two active modules provide the same singleton contract", async () => {
  const r = new ModuleRegistry(deps());
  r.add({
    manifest: { id: "a", version: "1.0.0", dependencies: {},
      provides: [{ contract: "s:sidebar", cardinality: "singleton" }] },
    register: vi.fn(),
  });
  r.add({
    manifest: { id: "b", version: "1.0.0", dependencies: {},
      provides: [{ contract: "s:sidebar", cardinality: "singleton" }] },
    register: vi.fn(),
  });
  await expect(r.activate()).rejects.toThrow(/singleton/);
});

test("allows two providers of a multi contract", async () => {
  const r = new ModuleRegistry(deps());
  r.add({
    manifest: { id: "a", version: "1.0.0", dependencies: {},
      provides: [{ contract: "s:panel", cardinality: "multi" }] },
    register: vi.fn(),
  });
  r.add({
    manifest: { id: "b", version: "1.0.0", dependencies: {},
      provides: [{ contract: "s:panel", cardinality: "multi" }] },
    register: vi.fn(),
  });
  await r.activate();
  expect(r.list().every((m) => m.active)).toBe(true);
});

test("removes a module's contributions on unload", async () => {
  const reg = new ContributionRegistry();
  const d = { ...deps(), contributions: reg };
  const r = new ModuleRegistry(d);
  r.add({
    manifest: { id: "m", version: "1.0.0", dependencies: {} },
    register: (ctx) => {
      ctx.contributions.contribute({ id: "p", contract: "s:sidebar", component: {} });
    },
  });
  await r.activate();
  expect(reg.contributionsFor("s:sidebar")).toHaveLength(1);
  await r.unload("m");
  expect(reg.contributionsFor("s:sidebar")).toHaveLength(0);
});

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
