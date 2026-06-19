import { expect, test } from "vitest";
import { parseManifest, declarationOf } from "./manifest";

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
