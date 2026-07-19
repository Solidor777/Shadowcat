import { describe, it, expect } from "vitest";
import { structuralDiff, deletePointer, deepEqual } from "./merge";
import { merge3Tree, takeTemplate, isPlacementExcluded, type Conflict } from "./merge";
import { merge3, restampSubtree, placementExclusions, type MergeBase } from "./merge";
import type { WireDocument } from "./wire";

describe("deepEqual", () => {
  it("compares objects key-order-independently and arrays positionally", () => {
    expect(deepEqual({ a: 1, b: 2 }, { b: 2, a: 1 })).toBe(true);
    expect(deepEqual([1, 2], [2, 1])).toBe(false);
    expect(deepEqual({ a: [1, { x: 2 }] }, { a: [1, { x: 2 }] })).toBe(true);
    expect(deepEqual(0, false)).toBe(false);
    expect(deepEqual(null, undefined)).toBe(false);
  });
});

describe("structuralDiff", () => {
  it("no change yields no diffs", () => {
    expect(structuralDiff({ a: 1, b: { c: 2 } }, { a: 1, b: { c: 2 } })).toEqual([]);
  });

  it("recurses objects, emitting the deepest changed leaf as a set", () => {
    expect(structuralDiff({ a: { b: 1 } }, { a: { b: 2 } })).toEqual([
      { path: "/a/b", kind: "set", value: 2 },
    ]);
  });

  it("a key present only in `now` is a set of that key", () => {
    expect(structuralDiff({ a: 1 }, { a: 1, b: 3 })).toEqual([
      { path: "/b", kind: "set", value: 3 },
    ]);
  });

  it("a key present only in `base` is a delete", () => {
    expect(structuralDiff({ a: 1, b: 2 }, { a: 1 })).toEqual([
      { path: "/b", kind: "delete" },
    ]);
  });

  it("arrays are opaque leaves — any inequality is one whole-array set", () => {
    expect(structuralDiff({ a: [1, 2] }, { a: [1, 2, 3] })).toEqual([
      { path: "/a", kind: "set", value: [1, 2, 3] },
    ]);
    expect(structuralDiff({ a: [{ x: 1 }] }, { a: [{ x: 2 }] })).toEqual([
      { path: "/a", kind: "set", value: [{ x: 2 }] },
    ]);
  });

  it("a scalar-to-object type change is a whole set at that path", () => {
    expect(structuralDiff({ a: 1 }, { a: { b: 2 } })).toEqual([
      { path: "/a", kind: "set", value: { b: 2 } },
    ]);
  });

  it("emits sorted, RFC-6901-escaped pointers", () => {
    const diffs = structuralDiff({}, { "b/x": 1, "a~y": 2 });
    expect(diffs.map((d) => d.path)).toEqual(["/a~0y", "/b~1x"]);
  });
});

describe("deletePointer", () => {
  it("removes an object key", () => {
    const root = { a: { b: 1, c: 2 } };
    deletePointer(root, "/a/b");
    expect(root).toEqual({ a: { c: 2 } });
  });

  it("splices an array element", () => {
    const root = { xs: [10, 20, 30] };
    deletePointer(root, "/xs/1");
    expect(root).toEqual({ xs: [10, 30] });
  });

  it("no-ops on a missing intermediate segment", () => {
    const root = { a: 1 };
    deletePointer(root, "/b/c");
    expect(root).toEqual({ a: 1 });
  });
});

describe("merge3Tree", () => {
  it("disjoint changes auto-merge (parent value applied, child value kept)", () => {
    const base = { a: 1, b: 2, c: 3 };
    const parent = { a: 1, b: 20, c: 3 }; // parent changed b
    const child = { a: 10, b: 2, c: 3 }; // child changed a
    const { merged, conflicts } = merge3Tree(base, parent, child, []);
    expect(conflicts).toEqual([]);
    expect(merged).toEqual({ a: 10, b: 20, c: 3 });
  });

  it("set/set on the same path with equal result is a no-op", () => {
    const { conflicts } = merge3Tree({ a: 1 }, { a: 2 }, { a: 2 }, []);
    expect(conflicts).toEqual([]);
  });

  it("set/set differing is a conflict; merged keeps child by default", () => {
    const { merged, conflicts } = merge3Tree({ a: 1 }, { a: 2 }, { a: 3 }, []);
    expect(conflicts).toEqual([
      { path: "/a", base: 1, parent: 2, child: 3, parentKind: "set" },
    ]);
    expect(merged).toEqual({ a: 3 });
  });

  it("set/delete is a conflict", () => {
    const { conflicts } = merge3Tree({ a: 1 }, { a: 2 }, {}, []);
    expect(conflicts).toEqual([
      { path: "/a", base: 1, parent: 2, child: undefined, parentKind: "set" },
    ]);
  });

  it("delete/set is a conflict", () => {
    const { conflicts } = merge3Tree({ a: 1 }, {}, { a: 3 }, []);
    expect(conflicts).toEqual([
      { path: "/a", base: 1, parent: undefined, child: 3, parentKind: "delete" },
    ]);
  });

  it("parent-only delete auto-applies (key removed from merged)", () => {
    const { merged, conflicts } = merge3Tree({ a: 1, b: 2 }, { a: 1 }, { a: 1, b: 2 }, []);
    expect(conflicts).toEqual([]);
    expect(merged).toEqual({ a: 1 });
  });

  it("arrays merge wholesale (parent array replaces base→child when child untouched)", () => {
    const { merged, conflicts } = merge3Tree(
      { xs: [1, 2] }, { xs: [1, 2, 3] }, { xs: [1, 2] }, [],
    );
    expect(conflicts).toEqual([]);
    expect(merged).toEqual({ xs: [1, 2, 3] });
  });

  it("map key-level: independent keys merge, one key conflicts", () => {
    const base = { m: { x: 1, y: 1 } };
    const parent = { m: { x: 2, y: 1 } };
    const child = { m: { x: 1, y: 9 } };
    const { merged, conflicts } = merge3Tree(base, parent, child, []);
    expect(conflicts).toEqual([]);
    expect(merged).toEqual({ m: { x: 2, y: 9 } });
  });

  it("excluded paths are dropped from parent's changes (never merge, never conflict)", () => {
    const base = { engine: { x: 0, hp: 1 } };
    const parent = { engine: { x: 99, hp: 5 } }; // parent moved x AND changed hp
    const child = { engine: { x: 3, hp: 1 } }; // child placed at x:3
    const { merged, conflicts } = merge3Tree(base, parent, child, ["/engine/x"]);
    expect(conflicts).toEqual([]);
    expect(merged).toEqual({ engine: { x: 3, hp: 5 } }); // child x kept, parent hp merged
  });

  it("is order-independent across permuted object keys", () => {
    const base = { a: 1, b: 2, c: 3, d: 4 };
    const parent = { a: 10, b: 2, c: 30, d: 4 };
    const child = { a: 1, b: 20, c: 3, d: 40 };
    const forward = merge3Tree(base, parent, child, []);
    const rev = (o: Record<string, number>) =>
      Object.fromEntries(Object.entries(o).reverse());
    const permuted = merge3Tree(rev(base), rev(parent), rev(child), []);
    expect(permuted.merged).toEqual(forward.merged);
    expect(permuted.conflicts).toEqual(forward.conflicts);
  });

  it("takeTemplate applies the parent's set/delete into a merged tree", () => {
    const merged = { a: 3, b: 5 };
    const setC: Conflict = { path: "/a", base: 1, parent: 2, child: 3, parentKind: "set" };
    takeTemplate(merged, setC);
    expect(merged).toEqual({ a: 2, b: 5 });
    const delC: Conflict = { path: "/b", base: 5, parent: undefined, child: 5, parentKind: "delete" };
    takeTemplate(merged, delC);
    expect(merged).toEqual({ a: 2 });
  });

  it("does not alias parentNow/childNow subtrees into merged/Conflict values (purity)", () => {
    // Disjoint parent-only set of a nested object: merged's subtree must not be the same
    // reference as parentNow's, or mutating one would corrupt the other in place.
    const base = { a: 1, nested: { x: 1 } };
    const parentNow = { a: 1, nested: { x: 1 }, added: { deep: { v: 1 } } };
    const childNow = { a: 2, nested: { x: 1 } };
    const { merged } = merge3Tree(base, parentNow, childNow, []) as {
      merged: { added: { deep: { v: number } } };
    };
    expect(merged.added).toEqual({ deep: { v: 1 } });
    expect(merged.added).not.toBe(parentNow.added);
    parentNow.added.deep.v = 999;
    expect(merged.added.deep.v).toBe(1);

    // Conflicting nested-object case: both sides add the same key with a different nested
    // object value, so `Conflict.parent`/`.child` must not alias `parentNow`/`childNow`.
    const base2 = { obj: { x: 1 } };
    const parentNow2 = { obj: { x: 1, deep: { v: 1 } } };
    const childNow2 = { obj: { x: 1, deep: { v: 2 } } };
    const { conflicts } = merge3Tree(base2, parentNow2, childNow2, []);
    expect(conflicts).toHaveLength(1);
    const c = conflicts[0] as { parent: { v: number }; child: { v: number } };
    expect(c.parent).toEqual({ v: 1 });
    expect(c.child).toEqual({ v: 2 });
    expect(c.parent).not.toBe(parentNow2.obj.deep);
    expect(c.child).not.toBe(childNow2.obj.deep);
    parentNow2.obj.deep.v = 999;
    childNow2.obj.deep.v = 888;
    expect(c.parent.v).toBe(1);
    expect(c.child.v).toBe(2);
  });
});

describe("isPlacementExcluded", () => {
  it("matches a path or its descendants against the exclusion set", () => {
    expect(isPlacementExcluded("/engine/x", ["/engine/x"])).toBe(true);
    expect(isPlacementExcluded("/engine/x/deep", ["/engine/x"])).toBe(true);
    expect(isPlacementExcluded("/engine/xylophone", ["/engine/x"])).toBe(false);
    expect(isPlacementExcluded("/engine/y", ["/engine/x"])).toBe(false);
  });
});

function doc(over: Partial<WireDocument> & { id: string }): WireDocument {
  return {
    id: over.id,
    scope: over.scope ?? { kind: "world", world_id: "w1" },
    doc_type: over.doc_type ?? "actor",
    schema_version: 1,
    name: over.name ?? null,
    source: over.source ?? null,
    owner: over.owner ?? null,
    permissions: { default: "none", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
    embedded: over.embedded ?? {},
    parent_id: over.parent_id ?? null,
    engine: over.engine,
    system: over.system ?? {},
    created_at: 0,
    updated_at: 0,
  };
}

/** MergeBase snapshot of a document's bands (test helper mirroring snapshotBase). */
function baseOf(d: WireDocument): MergeBase {
  const emb: MergeBase["embedded"] = {};
  for (const [coll, kids] of Object.entries(d.embedded)) {
    emb[coll] = kids.map((k) => ({
      sourceId: k.source?.id ?? k.id,
      name: k.name,
      engine: k.engine ?? null,
      system: k.system ?? null,
      embedded: baseOf(k).embedded,
    }));
  }
  return { name: d.name, engine: d.engine ?? null, system: d.system ?? null, embedded: emb };
}

describe("placementExclusions", () => {
  it("excludes token placement, nothing for other doc types", () => {
    expect(placementExclusions("token")).toEqual(["/engine/x", "/engine/y", "/engine/rotation"]);
    expect(placementExclusions("actor")).toEqual([]);
  });
});

describe("restampSubtree", () => {
  it("assigns a fresh id + source pointing to the template, recursively", () => {
    const child = doc({ id: "gc", name: "GC" });
    const parent = doc({ id: "tmpl", name: "T", embedded: { items: [child] } });
    const stamped = restampSubtree(parent);
    expect(stamped.id).not.toBe("tmpl");
    expect(stamped.source).toEqual({ id: "tmpl", pack: null, version: 1 });
    const sc = stamped.embedded.items[0];
    expect(sc.id).not.toBe("gc");
    expect(sc.source).toEqual({ id: "gc", pack: null, version: 1 });
  });

  it("clears a stale `base` inherited from the input document", () => {
    const child = doc({ id: "tmpl" });
    child.base = { name: null, engine: null, system: {}, embedded: {} };
    const stamped = restampSubtree(child);
    expect(stamped.base).toBeUndefined();
  });
});

describe("merge3 embedded", () => {
  it("matched child recurses; a disjoint system change auto-merges", () => {
    const instChild = doc({ id: "ic", source: { id: "tc", pack: null, version: 1 }, system: { hp: 1 } });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 }, embedded: { items: [instChild] } });
    const base = baseOf(child); // captured at stamp: instChild@hp:1
    const tmplChild2 = doc({ id: "tc", system: { hp: 5 } }); // template changed hp
    const template2 = doc({ id: "T", embedded: { items: [tmplChild2] } });
    const { mergedBands, conflicts } = merge3(base, template2, child, []);
    expect(conflicts).toEqual([]);
    expect((mergedBands.embedded.items[0].system as { hp: number }).hp).toBe(5);
    expect(mergedBands.embedded.items[0].id).toBe("ic"); // instance envelope preserved
  });

  it("matched-child recursion does not alias the instance child's envelope objects (purity)", () => {
    const instChild = doc({ id: "ic", source: { id: "tc", pack: null, version: 1 }, system: { hp: 1 } });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 }, embedded: { items: [instChild] } });
    const base = baseOf(child);
    const tmplChild2 = doc({ id: "tc", system: { hp: 5 } });
    const template2 = doc({ id: "T", embedded: { items: [tmplChild2] } });
    const { mergedBands } = merge3(base, template2, child, []);
    const mergedChild = mergedBands.embedded.items[0];
    expect(mergedChild.permissions).not.toBe(instChild.permissions);
    instChild.permissions.default = "observer";
    expect(mergedChild.permissions.default).toBe("none");
  });

  it("template-added child is stamped into the instance", () => {
    const template = doc({ id: "T", embedded: { items: [doc({ id: "new-tc", system: { k: 1 } })] } });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 }, embedded: { items: [] } });
    const base = baseOf(child); // no items at stamp
    const { mergedBands } = merge3(base, template, child, []);
    expect(mergedBands.embedded.items).toHaveLength(1);
    expect(mergedBands.embedded.items[0].source).toEqual({ id: "new-tc", pack: null, version: 1 });
    expect(mergedBands.embedded.items[0].id).not.toBe("new-tc");
  });

  it("instance-added child (no correlation) is preserved", () => {
    const template = doc({ id: "T", embedded: { items: [] } });
    const localChild = doc({ id: "local", system: { own: true } });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 }, embedded: { items: [localChild] } });
    const base: MergeBase = { name: null, engine: null, system: {}, embedded: { items: [] } };
    const { mergedBands, conflicts } = merge3(base, template, child, []);
    expect(conflicts).toEqual([]);
    expect(mergedBands.embedded.items.map((c) => c.id)).toEqual(["local"]);
  });

  it("template-deleted + instance unchanged removes the child", () => {
    const instChild = doc({ id: "ic", source: { id: "tc", pack: null, version: 1 }, system: { hp: 1 } });
    const template = doc({ id: "T", embedded: { items: [] } }); // template dropped tc
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 }, embedded: { items: [instChild] } });
    const base = baseOf(child); // base had tc@hp:1
    const { mergedBands } = merge3(base, template, child, []);
    expect(mergedBands.embedded.items).toHaveLength(0);
  });

  it("template-deleted + instance modified is a conflict", () => {
    const instChild = doc({ id: "ic", source: { id: "tc", pack: null, version: 1 }, system: { hp: 9 } });
    const template = doc({ id: "T", embedded: { items: [] } });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 }, embedded: { items: [instChild] } });
    const base: MergeBase = { name: null, engine: null, system: {}, embedded: { items: [{ sourceId: "tc", name: null, engine: null, system: { hp: 1 }, embedded: {} }] } };
    const { conflicts } = merge3(base, template, child, []);
    expect(conflicts).toEqual([
      { path: "/embedded/items/0", base: { hp: 1 }, parent: undefined, child: { hp: 9 }, parentKind: "delete" },
    ]);
  });

  it("recurses 2 levels of embedding", () => {
    const gcInst = doc({ id: "gci", source: { id: "gc", pack: null, version: 1 }, system: { deep: 1 } });
    const tcInst = doc({ id: "tci", source: { id: "tc", pack: null, version: 1 }, embedded: { sub: [gcInst] } });
    const template = doc({ id: "T", embedded: { items: [doc({ id: "tc", embedded: { sub: [doc({ id: "gc", system: { deep: 7 } })] } })] } });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 }, embedded: { items: [tcInst] } });
    const base = baseOf(child);
    const { mergedBands, conflicts } = merge3(base, template, child, []);
    expect(conflicts).toEqual([]);
    expect((mergedBands.embedded.items[0].embedded.sub[0].system as { deep: number }).deep).toBe(7);
  });
});
