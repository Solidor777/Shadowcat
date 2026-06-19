import { expect, test } from "vitest";
import { parseManifest } from "./manifest";

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
