import { expect, test, vi } from "vitest";
import { loadModules } from "./loader";
import { ModuleRegistry, type Module } from "./modules";
import { HookBus } from "./hooks";
import { ServiceRegistry } from "./services";
import { MiddlewareChain } from "./middleware";
import { DocumentStore } from "./store";
import { OptimisticClient } from "./optimistic";
import { ContributionRegistry } from "./contributions";
import { silentLogger } from "./logger";

function registry() {
  return new ModuleRegistry({
    hooks: new HookBus(silentLogger),
    services: new ServiceRegistry(),
    middleware: new MiddlewareChain(),
    store: new DocumentStore(),
    client: new OptimisticClient("self"),
    logger: silentLogger,
    contributions: new ContributionRegistry(),
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
      entries: [
        { manifest: { id: "declared", version: "1.0.0", dependencies: {} }, entry: "./a.js" },
      ],
      importFn: async () => mod, // module's own id is "a"
      registry: r,
    }),
  ).rejects.toThrow(/id/i);
});
