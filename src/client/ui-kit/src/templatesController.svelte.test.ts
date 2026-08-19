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

  it("pull and revert warn (not silently no-op) when the CHILD is not in store", () => {
    // An unresolvable child is indistinguishable from a working no-op without this log:
    // the control is enabled, the click dispatches nothing, and the console is empty.
    // `push` already warns on its equivalent branch; these two match it.
    const warned: string[] = [];
    const store = new DocumentStore();
    const ctrl = new TemplatesController({
      store, documents: store, dispatchIntent: () => {},
      role: "gm", selfId: "u-self", canEdit: () => true,
      logger: { ...silentLogger, warn: (m: string) => warned.push(m) },
    });

    ctrl.pull("ABSENT");
    ctrl.revert("ABSENT");

    expect(warned).toHaveLength(2);
    expect(warned[0]).toContain("templates.pull");
    expect(warned[0]).toContain("ABSENT");
    expect(warned[1]).toContain("templates.revert");
    expect(warned[1]).toContain("ABSENT");
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

  it("push excludes a conflict-free instance whose Update touches /embedded when the pusher lacks that capability, and warns", () => {
    // Template adds a brand-new embedded child (template-added case: no conflict). The instance
    // can write /base and /system but not /embedded, so the derived check must exclude it even
    // though the merge itself is conflict-free.
    const item = doc({ id: "item-1", doc_type: "item", system: { weight: 1 } });
    const tmpl = doc({ id: "T", embedded: { items: [item] } });
    const child = doc({ id: "A", source: { id: "T", pack: null, version: 1 } });
    child.base = { name: null, engine: null, system: {}, embedded: {} };
    const store = new DocumentStore();
    store.applyCommand({
      seq: 1, world_id: "w1", author: "a", ts: 0,
      ops: [tmpl, child].map((d) => ({ op: "create", doc: d } as WireOperation)),
    });
    const warned: string[] = [];
    const calls: WireOperation[][] = [];
    const ctrl = new TemplatesController({
      store, documents: store, dispatchIntent: (ops) => calls.push(ops),
      role: "player", selfId: "u-self",
      canEdit: (_doc, path) => !path.startsWith("/embedded"),
      logger: { ...silentLogger, warn: (m: string) => warned.push(m) },
    });
    ctrl.push("T");
    expect(calls).toHaveLength(0);
    expect(ctrl.pending).toBeNull();
    expect(warned).toHaveLength(1);
    expect(warned[0]).toContain("A");
  });

  it("push dispatches a conflict-free instance whose Update touches /embedded when the pusher CAN write it", () => {
    const item = doc({ id: "item-1", doc_type: "item", system: { weight: 1 } });
    const tmpl = doc({ id: "T", embedded: { items: [item] } });
    const child = doc({ id: "A", source: { id: "T", pack: null, version: 1 } });
    child.base = { name: null, engine: null, system: {}, embedded: {} };
    const { ctrl, calls } = make([tmpl, child]);
    ctrl.push("T");
    expect(calls).toHaveLength(1);
    const changes = (calls[0][0] as { changes: { path: string }[] }).changes;
    expect(changes.some((c) => c.path === "/embedded/items")).toBe(true);
  });

  it("push's conflict-resolution path (#openSession's resolve) applies the same derived check: a resolution that newly touches /embedded is excluded and warned, not silently dispatched", () => {
    // Base recorded a template-owned item; the template has since deleted it, and the instance
    // locally modified it (a real conflict, not an auto-drop). Pre-resolution the merge KEEPS the
    // instance's item unchanged, so /embedded never appears in the provisional Update and the
    // instance is admitted to the conflict modal. Choosing "theirs" (take the deletion) changes
    // /embedded/items in the FINAL Update — and the pusher cannot write /embedded.
    const tmpl = doc({ id: "T", embedded: {} });
    const childItem = doc({
      id: "child-item", doc_type: "item",
      source: { id: "orig-item", pack: null, version: 1 }, system: { foo: 2 },
    });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 }, embedded: { items: [childItem] } });
    child.base = {
      name: null, engine: null, system: {},
      embedded: { items: [{ sourceId: "orig-item", name: null, engine: null, system: { foo: 1 }, embedded: {} }] },
    };
    const store = new DocumentStore();
    store.applyCommand({
      seq: 1, world_id: "w1", author: "a", ts: 0,
      ops: [tmpl, child].map((d) => ({ op: "create", doc: d } as WireOperation)),
    });
    const warned: string[] = [];
    const calls: WireOperation[][] = [];
    const ctrl = new TemplatesController({
      store, documents: store, dispatchIntent: (ops) => calls.push(ops),
      role: "player", selfId: "u-self",
      canEdit: (_doc, path) => !path.startsWith("/embedded"),
      logger: { ...silentLogger, warn: (m: string) => warned.push(m) },
    });
    ctrl.push("T");
    expect(calls).toHaveLength(0); // admitted to the conflict session, not excluded up front
    expect(ctrl.pending).not.toBeNull();
    const conflictPath = ctrl.pending!.groups[0].conflicts[0].path;
    expect(conflictPath).toBe("/embedded/items/0");
    ctrl.pending!.resolve(new Map([["C", new Set([conflictPath])]])); // take template's deletion
    expect(calls).toHaveLength(0); // still excluded: the resolved Update now touches /embedded
    expect(ctrl.pending).toBeNull();
    expect(warned).toHaveLength(1);
    expect(warned[0]).toContain("C");
  });

  it("canPull is false for a non-owner non-GM", () => {
    const tmpl = doc({ id: "T" });
    const child = doc({ id: "C", owner: "someone-else", source: { id: "T", pack: null, version: 1 } });
    const { ctrl } = make([tmpl, child], { role: "player", selfId: "u-self" });
    expect(ctrl.canPull("C")).toBe(false);
  });

  it("canPull is false for a user who can edit base/system but not embedded", () => {
    const tmpl = doc({ id: "T" });
    const child = doc({ id: "C", owner: "u-self", source: { id: "T", pack: null, version: 1 } });
    const store = new DocumentStore();
    store.applyCommand({
      seq: 1, world_id: "w1", author: "a", ts: 0,
      ops: [tmpl, child].map((d) => ({ op: "create", doc: d } as WireOperation)),
    });
    const ctrl = new TemplatesController({
      store, documents: store, dispatchIntent: () => {},
      role: "player", selfId: "u-self",
      canEdit: (_doc, path) => path === "/base" || path === "/system",
      logger: silentLogger,
    });
    expect(ctrl.canPull("C")).toBe(false);
  });

  it("canPush is false for a user who can edit base/system but not embedded", () => {
    const tmpl = doc({ id: "T", owner: "u-self" });
    const inst = doc({ id: "A", source: { id: "T", pack: null, version: 1 } });
    const store = new DocumentStore();
    store.applyCommand({
      seq: 1, world_id: "w1", author: "a", ts: 0,
      ops: [tmpl, inst].map((d) => ({ op: "create", doc: d } as WireOperation)),
    });
    const ctrl = new TemplatesController({
      store, documents: store, dispatchIntent: () => {},
      role: "player", selfId: "u-self",
      canEdit: (_doc, path) => path === "/base" || path === "/system",
      logger: silentLogger,
    });
    expect(ctrl.canPush("T")).toBe(false);
  });

  it("findInstances returns instances of the template from the store", () => {
    const tmpl = doc({ id: "T" });
    const a = doc({ id: "A", source: { id: "T", pack: null, version: 1 } });
    const { ctrl } = make([tmpl, a]);
    expect(ctrl.findInstances("T").map((d) => d.id)).toEqual(["A"]);
  });

  it("treats the inheriting owner of a linked token as owner for template controls", () => {
    // token instance: owner null, engine.actor_id -> actor owned by self.
    // Literal doc.owner gate hid pull; the effectiveOwner mirror must show it.
    const tmpl = doc({ id: "T", doc_type: "actor" });
    const actor = doc({ id: "ACT", doc_type: "actor", owner: "u-self" });
    const token = doc({
      id: "TOK", doc_type: "token", owner: null,
      engine: { actor_id: "ACT" }, source: { id: "T", pack: null, version: 1 },
    });
    const store = new DocumentStore();
    store.applyCommand({
      seq: 1, world_id: "w1", author: "a", ts: 0,
      ops: [tmpl, actor, token].map((d) => ({ op: "create", doc: d } as WireOperation)),
    });
    const ctrl = new TemplatesController({
      store, documents: store, dispatchIntent: () => {},
      role: "player", selfId: "u-self",
      canEdit: () => true, logger: silentLogger,
    });
    expect(ctrl.canPull("TOK")).toBe(true);
  });
});
