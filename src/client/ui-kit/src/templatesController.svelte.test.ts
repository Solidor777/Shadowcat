import { describe, it, expect } from "vitest";
import { TemplatesController } from "./templatesController.svelte";
import { DocumentStore, silentLogger, type WireDocument, type WireOperation } from "@shadowcat/core";

function doc(over: Partial<WireDocument> & { id: string }): WireDocument {
  return {
    id: over.id, scope: { kind: "world", world_id: "w1" }, doc_type: over.doc_type ?? "actor",
    schema_version: 1, name: over.name ?? null, source: over.source ?? null, owner: over.owner ?? null,
    permissions: over.permissions ?? { default: "owner", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
    embedded: over.embedded ?? {}, parent_id: null, engine: over.engine, system: over.system ?? {},
    created_at: 0, updated_at: 0,
  };
}

function make(docs: WireDocument[], over: Partial<{ role: "gm" | "player"; selfId: string }> = {}) {
  const store = new DocumentStore();
  store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((d) => ({ op: "create", doc: d } as WireOperation)) });
  const calls: WireOperation[][] = [];
  const ctrl = new TemplatesController({
    store, documents: store, dispatchIntent: (ops) => calls.push(ops),
    role: over.role ?? "gm", selfId: over.selfId ?? "u-self",
    canEdit: () => true, logger: silentLogger,
  });
  return { store, ctrl, calls };
}

describe("TemplatesController", () => {
  it("pull with no conflicts dispatches an Update directly (no modal)", () => {
    const tmpl = doc({ id: "T", name: "T", system: { hp: 5 } });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 }, system: { hp: 1 } });
    child.base = { name: "T", engine: null, system: { hp: 1 }, embedded: {} };
    const { ctrl, calls } = make([tmpl, child]);
    ctrl.pull("C");
    expect(ctrl.pending).toBeNull();
    expect(calls).toHaveLength(1);
    expect(calls[0][0].op).toBe("update");
  });

  it("pull with a conflict opens the modal and dispatches on resolve", () => {
    const tmpl = doc({ id: "T", system: { hp: 5 } });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 }, system: { hp: 9 } });
    child.base = { name: null, engine: null, system: { hp: 1 }, embedded: {} };
    const { ctrl, calls } = make([tmpl, child]);
    ctrl.pull("C");
    expect(ctrl.pending).not.toBeNull();
    expect(calls).toHaveLength(0);
    ctrl.pending!.resolve(new Map([["C", new Set(["/system/hp"])]]));
    expect(ctrl.pending).toBeNull();
    expect(calls).toHaveLength(1);
    const sys = (calls[0][0] as { changes: { path: string; new: unknown }[] }).changes.find((c) => c.path === "/system")!;
    expect(sys.new).toEqual({ hp: 5 }); // took template
  });

  it("pull is unavailable (no dispatch) when the template is not in store", () => {
    const child = doc({ id: "C", source: { id: "MISSING", pack: null, version: 1 } });
    const { ctrl, calls } = make([child]);
    ctrl.pull("C");
    expect(calls).toHaveLength(0);
    expect(ctrl.pending).toBeNull();
  });

  it("push dispatches one Update per conflict-free instance and groups conflicts", () => {
    const tmpl = doc({ id: "T", system: { hp: 5 } });
    const clean = doc({ id: "A", source: { id: "T", pack: null, version: 1 }, system: { hp: 1 } });
    clean.base = { name: null, engine: null, system: { hp: 1 }, embedded: {} };
    const conflicted = doc({ id: "B", source: { id: "T", pack: null, version: 1 }, system: { hp: 9 } });
    conflicted.base = { name: null, engine: null, system: { hp: 1 }, embedded: {} };
    const { ctrl, calls } = make([tmpl, clean, conflicted]);
    ctrl.push("T");
    expect(calls).toHaveLength(1); // clean instance applied immediately
    expect(ctrl.pending).not.toBeNull();
    expect(ctrl.pending!.groups.map((g) => g.key)).toEqual(["B"]);
  });

  it("push excludes an instance the pusher cannot write, even when it conflicts", () => {
    const tmpl = doc({ id: "T", system: { hp: 5 } });
    const writable = doc({ id: "A", owner: "u-self", source: { id: "T", pack: null, version: 1 }, system: { hp: 9 } });
    writable.base = { name: null, engine: null, system: { hp: 1 }, embedded: {} };
    const notWritable = doc({ id: "B", owner: "someone-else", source: { id: "T", pack: null, version: 1 }, system: { hp: 9 } });
    notWritable.base = { name: null, engine: null, system: { hp: 1 }, embedded: {} };
    const store = new DocumentStore();
    store.applyCommand({
      seq: 1, world_id: "w1", author: "a", ts: 0,
      ops: [tmpl, writable, notWritable].map((d) => ({ op: "create", doc: d } as WireOperation)),
    });
    const calls: WireOperation[][] = [];
    const ctrl = new TemplatesController({
      store, documents: store, dispatchIntent: (ops) => calls.push(ops),
      role: "player", selfId: "u-self",
      canEdit: (doc) => doc.owner === "u-self",
      logger: silentLogger,
    });
    ctrl.push("T");
    // Both instances conflict, but only the writable one should surface at all.
    expect(calls).toHaveLength(0);
    expect(ctrl.pending).not.toBeNull();
    expect(ctrl.pending!.groups.map((g) => g.key)).toEqual(["A"]);
  });

  it("canPull is false for a non-owner non-GM", () => {
    const tmpl = doc({ id: "T" });
    const child = doc({ id: "C", owner: "someone-else", source: { id: "T", pack: null, version: 1 } });
    const { ctrl } = make([tmpl, child], { role: "player", selfId: "u-self" });
    expect(ctrl.canPull("C")).toBe(false);
  });

  it("findInstances returns instances of the template from the store", () => {
    const tmpl = doc({ id: "T" });
    const a = doc({ id: "A", source: { id: "T", pack: null, version: 1 } });
    const { ctrl } = make([tmpl, a]);
    expect(ctrl.findInstances("T").map((d) => d.id)).toEqual(["A"]);
  });
});
