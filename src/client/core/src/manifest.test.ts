import { expect, test, expectTypeOf } from "vitest";
import type * as Ts from "@shadowcat/types";
import {
  parseManifest,
  declarationOf,
  normalizeRequires,
  type ContractDeclaration,
  type ContractProvide,
} from "./manifest";

// Drift guard: the hand-written manifest declaration shapes (consumed by
// declarationOf / ModuleRegistry.declarations() and reconciled against the wire
// topology) must stay pinned to the ts-rs generated types. `ContractProvide.priority` is
// excluded here deliberately: it is local singleton-tie-break metadata with no wire
// counterpart (see that field's own doc comment), and `ContractDeclaration.provides` is
// itself typed to omit it, so the declaration-level assertion below already covers the
// wire-relevant fields exactly.
test("manifest contract declaration shapes match the ts-rs types", () => {
  expectTypeOf<ContractDeclaration>().toEqualTypeOf<Ts.ContractDeclaration>();
  expectTypeOf<Omit<ContractProvide, "priority">>().toEqualTypeOf<Ts.ContractProvide>();
});

test("accepts provides/requires and projects to a declaration", () => {
  const m = parseManifest({
    id: "sidebar",
    version: "1.0.0",
    dependencies: {},
    provides: [{ contract: "s:sidebar", cardinality: "singleton" }],
    requires: ["s:root"],
  });
  expect(declarationOf(m)).toEqual({
    module_id: "sidebar",
    version: "1.0.0",
    provides: [{ contract: "s:sidebar", cardinality: "singleton" }],
    requires: ["s:root"],
  });
});

test("defaults provides/requires to empty in a projection", () => {
  const m = parseManifest({ id: "m", version: "1.0.0", dependencies: {} });
  expect(declarationOf(m)).toEqual({
    module_id: "m",
    version: "1.0.0",
    provides: [],
    requires: [],
  });
});

test("accepts a version-ranged requires entry and projects it to a bare contract id", () => {
  const m = parseManifest({
    id: "combat",
    version: "1.0.0",
    dependencies: {},
    requires: ["s:sidebar", { contract: "s:root", version: "^2.0.0" }],
  });
  expect(m.requires).toEqual(["s:sidebar", { contract: "s:root", version: "^2.0.0" }]);
  expect(declarationOf(m)).toEqual({
    module_id: "combat",
    version: "1.0.0",
    provides: [],
    requires: ["s:sidebar", "s:root"],
  });
});

test("normalizeRequires normalizes a bare string and passes an object through unchanged", () => {
  expect(normalizeRequires(["a", { contract: "b", version: "^2.0.0" }])).toEqual([
    { contract: "a" },
    { contract: "b", version: "^2.0.0" },
  ]);
  expect(normalizeRequires(undefined)).toEqual([]);
});

test("rejects an invalid cardinality", () => {
  expect(() =>
    parseManifest({
      id: "m",
      version: "1.0.0",
      dependencies: {},
      provides: [{ contract: "s:x", cardinality: "lots" }],
    }),
  ).toThrow();
});

test("valid manifest parses with defaults", () => {
  const m = parseManifest({ id: "dnd5e", version: "1.0.0", dependencies: {} });
  expect(m.id).toBe("dnd5e");
  expect(m.dependencies).toEqual({});
});

test("requirements and hooks parse", () => {
  const m = parseManifest({
    id: "vision",
    version: "0.1.0",
    dependencies: { core: "^1.0.0" },
    capabilities: ["dnd5e:gm_vision"],
    requirements: [{ path_prefix: "/system/vision", caps: ["dnd5e:gm_vision"] }],
    hooks: [{ name: "dnd5e:preRollAttack", version: "1.0.0", kind: "cancel" }],
  });
  expect(m.requirements![0].path_prefix).toBe("/system/vision");
  expect(m.hooks![0].kind).toBe("cancel");
});

test("missing id is rejected", () => {
  expect(() => parseManifest({ version: "1.0.0", dependencies: {} })).toThrow();
});

test("requirement path_prefix must start with /", () => {
  expect(() =>
    parseManifest({
      id: "x",
      version: "1.0.0",
      dependencies: {},
      requirements: [{ path_prefix: "system", caps: ["x:y"] }],
    }),
  ).toThrow();
});

test("a manifest with no engines field parses (first-party modules never set it)", () => {
  const m = parseManifest({ id: "a", version: "1.0.0", dependencies: {} });
  expect(m.engines).toBeUndefined();
});

test("a manifest with a valid engines.shadowcat range parses", () => {
  const m = parseManifest({
    id: "a",
    version: "1.0.0",
    dependencies: {},
    engines: { shadowcat: "^0.1.0" },
  });
  expect(m.engines?.shadowcat).toBe("^0.1.0");
});

test("an empty engines.shadowcat string is rejected", () => {
  expect(() =>
    parseManifest({
      id: "a",
      version: "1.0.0",
      dependencies: {},
      engines: { shadowcat: "" },
    }),
  ).toThrow();
});

test("a manifest with a valid style path parses", () => {
  const m = parseManifest({ id: "a", version: "1.0.0", dependencies: {}, style: "style.css" });
  expect(m.style).toBe("style.css");
});

test("a manifest without a style field parses with it absent", () => {
  const m = parseManifest({ id: "a", version: "1.0.0", dependencies: {} });
  expect(m.style).toBeUndefined();
});

test.each([
  ["/abs/style.css", "absolute"],
  ["../escape.css", "traversing"],
  ["style.css/..", "trailing-traversal"],
  // WHATWG URL resolution treats `\` as a path separator for special
  // schemes, so a backslash path is traversal even with clean `/`-segments.
  ["..\\..\\escape.css", "backslash-traversal"],
  ["sub\\style.css", "backslash-separator"],
  ["style.js", "non-css"],
  ["", "empty"],
])("a manifest with a %s style path (%s) is rejected", (style) => {
  expect(() =>
    parseManifest({ id: "a", version: "1.0.0", dependencies: {}, style }),
  ).toThrow();
});
