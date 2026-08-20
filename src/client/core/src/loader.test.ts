import { expect, test, vi } from "vitest";
import { loadModules } from "./loader";
import { ModuleRegistry, type Module } from "./modules";
import type { ModuleManifest } from "./manifest";
import { HookBus } from "./hooks";
import { ServiceRegistry } from "./services";
import { MiddlewareChain } from "./middleware";
import { DocumentStore } from "./store";
import { OptimisticClient } from "./optimistic";
import { ContributionRegistry } from "./contributions";
import { silentLogger } from "./logger";
import { I18n } from "./i18n";

function registry() {
  return new ModuleRegistry({
    hooks: new HookBus(silentLogger),
    services: new ServiceRegistry(),
    middleware: new MiddlewareChain(),
    store: new DocumentStore(),
    client: new OptimisticClient("self"),
    logger: silentLogger,
    contributions: new ContributionRegistry(),
    i18n: new I18n("en", { en: {} }),
  });
}

const mod: Module = {
  manifest: { id: "a", version: "1.0.0", dependencies: {} },
  register: vi.fn(),
};

test("loadModules imports entries, adds them to the registry, and reports loaded ids", async () => {
  const r = registry();
  const importFn = vi.fn(async () => ({ default: mod }));
  const result = await loadModules({
    entries: [{ manifest: mod.manifest, entry: "./a.js" }],
    importFn,
    registry: r,
  });
  expect(importFn).toHaveBeenCalledWith("./a.js");
  expect(r.list().map((m) => m.id)).toEqual(["a"]);
  expect(result.loaded).toEqual(["a"]);
  expect(result.failed).toEqual([]);
});

test("a namespace export (no default) is accepted", async () => {
  const r = registry();
  const result = await loadModules({
    entries: [{ manifest: mod.manifest, entry: "./a.js" }],
    importFn: async () => mod,
    registry: r,
  });
  expect(r.list()).toHaveLength(1);
  expect(result.loaded).toEqual(["a"]);
});

test("a manifest id mismatch is contained per-module, not thrown", async () => {
  const r = registry();
  const result = await loadModules({
    entries: [
      { manifest: { id: "declared", version: "1.0.0", dependencies: {} }, entry: "./a.js" },
    ],
    importFn: async () => mod, // module's own id is "a"
    registry: r,
  });
  expect(result.loaded).toEqual([]);
  expect(result.failed).toHaveLength(1);
  expect(result.failed[0].id).toBe("declared");
  expect(result.failed[0].error).toMatch(/id/i);
  expect(r.list()).toHaveLength(0);
});

test("one failing entry does not block a later valid one", async () => {
  const r = registry();
  const good: Module = {
    manifest: { id: "b", version: "1.0.0", dependencies: {} },
    register: vi.fn(),
  };
  const result = await loadModules({
    entries: [
      { manifest: { id: "declared", version: "1.0.0", dependencies: {} }, entry: "./a.js" },
      { manifest: good.manifest, entry: "./b.js" },
    ],
    importFn: async (entry) => (entry === "./a.js" ? mod : good),
    registry: r,
  });
  expect(result.loaded).toEqual(["b"]);
  expect(result.failed.map((f) => f.id)).toEqual(["declared"]);
});

test("an engine-compat mismatch is contained and reported", async () => {
  const r = registry();
  const incompatible: Module = {
    manifest: { id: "c", version: "1.0.0", dependencies: {}, engines: { shadowcat: "^2.0.0" } },
    register: vi.fn(),
  };
  const result = await loadModules({
    entries: [{ manifest: incompatible.manifest, entry: "./c.js" }],
    importFn: async () => incompatible,
    registry: r,
    shadowcatVersion: "1.0.0",
  });
  expect(result.loaded).toEqual([]);
  expect(result.failed).toHaveLength(1);
  expect(result.failed[0].id).toBe("c");
  expect(result.failed[0].error).toMatch(/shadowcat/i);
});

test("shadowcatVersion is optional: compat is skipped entirely when omitted", async () => {
  const r = registry();
  const withRange: Module = {
    manifest: { id: "d", version: "1.0.0", dependencies: {}, engines: { shadowcat: "^99.0.0" } },
    register: vi.fn(),
  };
  const result = await loadModules({
    entries: [{ manifest: withRange.manifest, entry: "./d.js" }],
    importFn: async () => withRange,
    registry: r,
  });
  expect(result.loaded).toEqual(["d"]);
});

test("a manifest with no engines field always passes compat, even when shadowcatVersion is given", async () => {
  const r = registry();
  const result = await loadModules({
    entries: [{ manifest: mod.manifest, entry: "./a.js" }],
    importFn: async () => mod,
    registry: r,
    shadowcatVersion: "1.0.0",
  });
  expect(result.loaded).toEqual(["a"]);
});

test("a structurally-invalid discovered manifest is contained; a sibling valid entry still loads", async () => {
  const r = registry();
  const good: Module = {
    manifest: { id: "b", version: "1.0.0", dependencies: {} },
    register: vi.fn(),
  };
  const result = await loadModules({
    // Missing `id`/`version` fails ManifestSchema.parse in loadModules itself,
    // before importFn is ever invoked for this entry.
    entries: [
      { manifest: { dependencies: {} } as unknown as ModuleManifest, entry: "./broken.js" },
      { manifest: good.manifest, entry: "./b.js" },
    ],
    importFn: async (entry) => (entry === "./b.js" ? good : mod),
    registry: r,
  });
  expect(result.loaded).toEqual(["b"]);
  expect(result.failed).toHaveLength(1);
  expect(result.failed[0].entry).toBe("./broken.js");
  expect(result.failed[0].error.length).toBeGreaterThan(0);
  expect(r.list().map((m) => m.id)).toEqual(["b"]);
});

test("an importFn rejection is contained; a sibling valid entry still loads", async () => {
  const r = registry();
  const good: Module = {
    manifest: { id: "e", version: "1.0.0", dependencies: {} },
    register: vi.fn(),
  };
  const broken: ModuleManifest = { id: "f", version: "1.0.0", dependencies: {} };
  const result = await loadModules({
    entries: [
      { manifest: broken, entry: "./f.js" },
      { manifest: good.manifest, entry: "./e.js" },
    ],
    importFn: async (entry) => {
      if (entry === "./f.js") throw new Error("network error");
      return good;
    },
    registry: r,
  });
  expect(result.loaded).toEqual(["e"]);
  expect(result.failed).toHaveLength(1);
  expect(result.failed[0].id).toBe("f");
  expect(result.failed[0].error).toMatch(/network error/);
  expect(r.list().map((m) => m.id)).toEqual(["e"]);
});

test("an empty shadowcatVersion fails closed (does not silently skip the compat gate)", async () => {
  const r = registry();
  const withRange: Module = {
    manifest: { id: "g", version: "1.0.0", dependencies: {}, engines: { shadowcat: "^1.0.0" } },
    register: vi.fn(),
  };
  const result = await loadModules({
    entries: [{ manifest: withRange.manifest, entry: "./g.js" }],
    importFn: async () => withRange,
    registry: r,
    shadowcatVersion: "",
  });
  expect(result.loaded).toEqual([]);
  expect(result.failed).toHaveLength(1);
  expect(result.failed[0].id).toBe("g");
  expect(r.list()).toHaveLength(0);
});
