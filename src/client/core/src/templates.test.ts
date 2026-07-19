import { describe, it, expect } from "vitest";
import { snapshotBase, stampInstance, type StampOpts } from "./templates";
import type { WireDocument } from "./wire";
import type { MergeBase } from "./merge";

function doc(over: Partial<WireDocument> & { id: string }): WireDocument {
  return {
    id: over.id,
    scope: over.scope ?? { kind: "world", world_id: "w1" },
    doc_type: over.doc_type ?? "actor",
    schema_version: 1,
    name: over.name ?? null,
    source: over.source ?? null,
    owner: over.owner ?? null,
    permissions: over.permissions ?? { default: "none", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
    embedded: over.embedded ?? {},
    parent_id: over.parent_id ?? null,
    engine: over.engine,
    system: over.system ?? {},
    created_at: 0,
    updated_at: 0,
  };
}

const opts: StampOpts = { worldId: "w1", ownerId: "u-self", parentId: "scene-1" };

describe("snapshotBase", () => {
  it("captures bands + embedded children keyed by their source.id", () => {
    const child = doc({ id: "ic", source: { id: "tc", pack: null, version: 2 }, name: "Kid", system: { hp: 3 } });
    const d = doc({ id: "C", name: "Inst", engine: { hp: 9 }, system: { a: 1 }, embedded: { items: [child] } });
    const snap = snapshotBase(d);
    expect(snap).toEqual<MergeBase>({
      name: "Inst",
      engine: { hp: 9 },
      system: { a: 1 },
      embedded: { items: [{ sourceId: "tc", name: "Kid", engine: null, system: { hp: 3 }, embedded: {} }] },
    });
  });

  it("deep-clones so the snapshot does not alias the document", () => {
    const d = doc({ id: "C", system: { nested: { x: 1 } } });
    const snap = snapshotBase(d);
    (d.system as { nested: { x: number } }).nested.x = 99;
    expect((snap.system as { nested: { x: number } }).nested.x).toBe(1);
  });
});

describe("stampInstance", () => {
  it("creates a new doc: fresh id, initiator owner/parent, source pointing at the template", () => {
    const tmpl = doc({ id: "T", name: "Preset", owner: "gm", system: { hp: 10 } });
    const inst = stampInstance(tmpl, opts);
    expect(inst.id).not.toBe("T");
    expect(inst.owner).toBe("u-self");
    expect(inst.parent_id).toBe("scene-1");
    expect(inst.source).toEqual({ id: "T", pack: null, version: 1 });
    expect(inst.system).toEqual({ hp: 10 });
  });

  it("deep-clone independence: nested bands are not aliased (recursively)", () => {
    const tmplChild = doc({ id: "tc", system: { deep: { v: 1 } } });
    const tmpl = doc({ id: "T", system: { s: { v: 1 } }, embedded: { items: [tmplChild] } });
    const inst = stampInstance(tmpl, opts);
    expect(inst.system).not.toBe(tmpl.system);
    expect(inst.embedded.items[0].system).not.toBe(tmplChild.system);
    (tmpl.system as { s: { v: number } }).s.v = 42;
    (tmplChild.system as { deep: { v: number } }).deep.v = 42;
    expect((inst.system as { s: { v: number } }).s.v).toBe(1);
    expect((inst.embedded.items[0].system as { deep: { v: number } }).deep.v).toBe(1);
  });

  it("recursively assigns embedded children fresh ids + source = template child id", () => {
    const tmplChild = doc({ id: "tc", name: "Item" });
    const tmpl = doc({ id: "T", embedded: { items: [tmplChild] } });
    const inst = stampInstance(tmpl, opts);
    const sc = inst.embedded.items[0];
    expect(sc.id).not.toBe("tc");
    expect(sc.source).toEqual({ id: "tc", pack: null, version: 1 });
  });

  it("sets base to a snapshot keyed by the new children's source.id (correlation)", () => {
    const tmpl = doc({ id: "T", name: "P", system: { hp: 1 }, embedded: { items: [doc({ id: "tc", system: { k: 1 } })] } });
    const inst = stampInstance(tmpl, opts);
    const base = inst.base as MergeBase;
    expect(base.name).toBe("P");
    expect(base.system).toEqual({ hp: 1 });
    expect(base.embedded.items[0].sourceId).toBe("tc"); // == the stamped child's source.id
    expect(base.embedded.items[0].system).toEqual({ k: 1 });
  });

  it("copies the compendium pack into source when the template is compendium-scoped", () => {
    const tmpl = doc({ id: "T", scope: { kind: "compendium", pack: "nightfox" } });
    const inst = stampInstance(tmpl, opts);
    expect(inst.source).toEqual({ id: "T", pack: "nightfox", version: 1 });
  });

  it("does not inherit the template's own provenance pack when the template itself is world-scoped", () => {
    // The template is itself an instance of a compendium item, so its own `source.pack` is set —
    // but that provenance belongs to the TEMPLATE, not to this stamp.
    const tmpl = doc({ id: "T", scope: { kind: "world", world_id: "w1" }, source: { id: "orig", pack: "nightfox", version: 3 } });
    const inst = stampInstance(tmpl, opts);
    expect(inst.source).toEqual({ id: "T", pack: null, version: 3 });
  });

  it("gives the stamped instance fresh created_at/updated_at, not the template's", () => {
    const tmpl = doc({ id: "T", name: "Old" });
    (tmpl as { created_at: number }).created_at = 12345;
    (tmpl as { updated_at: number }).updated_at = 12345;
    const inst = stampInstance(tmpl, opts);
    expect(inst.created_at).not.toBe(tmpl.created_at);
    expect(inst.updated_at).not.toBe(tmpl.updated_at);
    expect(inst.created_at).toBeGreaterThan(tmpl.created_at);
    expect(inst.updated_at).toBeGreaterThan(tmpl.updated_at);
  });

  it("clones opts.permissions rather than aliasing the caller's object", () => {
    const tmpl = doc({ id: "T" });
    const perms: WireDocument["permissions"] = {
      default: "none",
      users: { u1: "owner" },
      property_overrides: {},
      capabilities: { by_role: {}, by_user: {} },
      gm_role: null,
    };
    const inst = stampInstance(tmpl, { ...opts, permissions: perms });
    perms.users.u1 = "observer";
    expect(inst.permissions.users.u1).toBe("owner");
  });

  it("deep-clone independence holds 2 levels deep (grandchild)", () => {
    const grandchild = doc({ id: "gc", system: { deep: { v: 1 } } });
    const child = doc({ id: "tc", system: { v: 1 }, embedded: { items: [grandchild] } });
    const tmpl = doc({ id: "T", embedded: { items: [child] } });
    const inst = stampInstance(tmpl, opts);
    const instChild = inst.embedded.items[0];
    const instGrandchild = instChild.embedded.items[0];
    (grandchild.system as { deep: { v: number } }).deep.v = 99;
    expect((instGrandchild.system as { deep: { v: number } }).deep.v).toBe(1);
  });
});
