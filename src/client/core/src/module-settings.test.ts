import { expect, test, vi } from "vitest";
import { ModuleRegistry, type Module, type ModuleContext } from "./modules";
import { HookBus } from "./hooks";
import { ServiceRegistry } from "./services";
import { MiddlewareChain } from "./middleware";
import { DocumentStore } from "./store";
import { OptimisticClient } from "./optimistic";
import { ContributionRegistry } from "./contributions";
import { silentLogger } from "./logger";
import { I18n } from "./i18n";
import { KeybindingsController } from "./keybindings";
import { UserModuleSettings, pointerToken } from "./module-settings";
import type { WireDocument, WireOperation } from "./wire";

const DOC_ID = "ms-1";

/** A module-settings document whose `system` band is `system`. */
function settingsDoc(system: unknown): WireDocument {
  return {
    id: DOC_ID,
    world_id: "w",
    doc_type: "module-settings",
    engine: {},
    system,
  } as unknown as WireDocument;
}

/** Activates one module and returns the context it was handed, plus the shared pieces. */
async function setup(opts: {
  system?: unknown;
  withDoc?: boolean;
  dispatch?: (ops: WireOperation[]) => boolean;
  put?: ConstructorParameters<typeof UserModuleSettings>[1];
  user?: Record<string, Record<string, unknown>>;
}) {
  const hooks = new HookBus(silentLogger);
  const client = new OptimisticClient("self");
  if (opts.withDoc !== false) client.seedDocuments([settingsDoc(opts.system ?? {})]);
  const dispatched: WireOperation[][] = [];
  const dispatchIntent = opts.dispatch ?? ((ops: WireOperation[]) => (dispatched.push(ops), true));
  const registry = new ModuleRegistry({
    keybindings: new KeybindingsController(hooks),
    hooks,
    services: new ServiceRegistry(),
    middleware: new MiddlewareChain(),
    store: new DocumentStore(),
    client,
    logger: silentLogger,
    contributions: new ContributionRegistry(),
    i18n: new I18n("en", { en: {} }),
    dispatchIntent,
    userSettings: new UserModuleSettings(opts.user ?? {}, opts.put ?? (async () => {})),
  });
  let ctx: ModuleContext | undefined;
  const m: Module = {
    manifest: { id: "mine", version: "1.0.0", dependencies: {} },
    register: vi.fn((c: ModuleContext) => {
      ctx = c;
    }),
  };
  registry.add(m);
  await registry.activate();
  return { ctx: ctx as ModuleContext, dispatched };
}

test("setWorld sends one field change per patch key and leaves sibling keys unnamed", async () => {
  const { ctx, dispatched } = await setup({ system: { mine: { keep: 1, volume: 2 } } });
  await ctx.settings.setWorld({ volume: 3, fresh: "x" });
  expect(dispatched).toHaveLength(1);
  expect(dispatched[0]).toEqual([
    {
      op: "update",
      doc_id: DOC_ID,
      changes: [
        { path: "/system/mine/volume", old: 2, new: 3 },
        { path: "/system/mine/fresh", old: null, new: "x" },
      ],
    },
  ]);
  // Mutation that fails this: building one change at `/system/mine` (drops `keep`).
});

test("setWorld escapes pointer tokens in the module id and the key", async () => {
  expect(pointerToken("a/b~c")).toBe("a~1b~0c");
  const { ctx, dispatched } = await setup({});
  await ctx.settings.setWorld({ "a/b": 1 });
  const op = dispatched[0][0] as { changes: { path: string }[] };
  expect(op.changes[0].path).toBe("/system/mine/a~1b");
  // Mutation that fails this: joining the raw key into the pointer.
});

test("setWorld rejects when the world has no module-settings document", async () => {
  const { ctx, dispatched } = await setup({ withDoc: false });
  await expect(ctx.settings.setWorld({ a: 1 })).rejects.toThrow(/module-settings/);
  expect(dispatched).toHaveLength(0);
});

test("setWorld rejects when the host drops the intent", async () => {
  const { ctx } = await setup({ dispatch: () => false });
  await expect(ctx.settings.setWorld({ a: 1 })).rejects.toThrow(/dropped/);
});

test("getWorld returns only this module's slice, or {} when unset", async () => {
  const { ctx } = await setup({ system: { mine: { a: 1 }, other: { b: 2 } } });
  expect(ctx.settings.getWorld()).toEqual({ a: 1 });
  const none = await setup({ system: { other: { b: 2 } } });
  expect(none.ctx.settings.getWorld()).toEqual({});
  const noDoc = await setup({ withDoc: false });
  expect(noDoc.ctx.settings.getWorld()).toEqual({});
  // Mutation that fails this: reading the whole `system` band.
});

test("setUser puts only this module's namespace and updates the snapshot after success", async () => {
  const put = vi.fn(async () => {});
  const { ctx } = await setup({ put, user: { mine: { a: 1, gone: 2 }, other: { z: 9 } } });
  await ctx.settings.setUser({ b: 2, gone: null });
  expect(put).toHaveBeenCalledWith({ modules: { mine: { b: 2, gone: null } } });
  expect(ctx.settings.getUser()).toEqual({ a: 1, b: 2 });
  // Mutation that fails this: sending the module id from the patch, or not deleting null keys.
});

test("setUser leaves the snapshot unchanged when the write rejects", async () => {
  const put = vi.fn(async () => {
    throw new Error("PUT /api/me/ui-state → 500");
  });
  const { ctx } = await setup({ put, user: { mine: { a: 1 } } });
  await expect(ctx.settings.setUser({ a: 2 })).rejects.toThrow(/500/);
  expect(ctx.settings.getUser()).toEqual({ a: 1 });
});
