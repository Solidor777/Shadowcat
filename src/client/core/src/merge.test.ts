import { describe, it, expect } from "vitest";
import { structuralDiff, deletePointer, deepEqual } from "./merge";

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
