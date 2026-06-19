import { expect, test } from "vitest";
import { resolveCaps, canWritePath } from "./capabilities";
import type { WireDocument } from "./wire";

const emptyGrants = { by_role: {}, by_user: {} };

function perms(p: Partial<WireDocument["permissions"]>): WireDocument["permissions"] {
  return {
    default: "none",
    users: {},
    property_overrides: {},
    capabilities: { by_role: {}, by_user: {} },
    ...p,
  };
}

test("owner floor is read + write_fields", () => {
  const caps = resolveCaps(perms({ users: { u1: "owner" } }), "u1", "player", emptyGrants);
  expect(caps.has("core:read")).toBe(true);
  expect(caps.has("core:write_fields")).toBe(true);
  expect(caps.has("core:manage_embedded")).toBe(false);
});

test("world grant widens the floor", () => {
  const caps = resolveCaps(perms({ users: { u1: "owner" } }), "u1", "player", {
    by_role: { owner: ["core:manage_embedded"] },
    by_user: {},
  });
  expect(caps.has("core:manage_embedded")).toBe(true);
});

test("per-user world grant applies", () => {
  const caps = resolveCaps(perms({ users: { u1: "owner" } }), "u1", "player", {
    by_role: {},
    by_user: { u1: ["dnd5e:cast"] },
  });
  expect(caps.has("dnd5e:cast")).toBe(true);
});

test("canWritePath enforces the base cap", () => {
  const caps = new Set(["core:read", "core:write_fields"]);
  expect(canWritePath("/system/hp", caps, false, [])).toBe(true);
  expect(canWritePath("/embedded/x", caps, false, [])).toBe(false); // needs manage_embedded
  expect(canWritePath("/id", caps, false, [])).toBe(false); // immutable envelope
});

test("canWritePath enforces a declared requirement additively", () => {
  const caps = new Set(["core:read", "core:write_fields"]);
  const reqs = [{ path_prefix: "/system/vision", caps: ["dnd5e:gm_vision"] }];
  expect(canWritePath("/system/vision/range", caps, false, reqs)).toBe(false);
  const withVision = new Set([...caps, "dnd5e:gm_vision"]);
  expect(canWritePath("/system/vision/range", withVision, false, reqs)).toBe(true);
});

test("canWritePath gates an ancestor write that covers a protected subtree", () => {
  const caps = new Set(["core:read", "core:write_fields"]);
  const reqs = [{ path_prefix: "/system/vision", caps: ["dnd5e:gm_vision"] }];
  // writing /system wholesale would replace /system/vision → gated
  expect(canWritePath("/system", caps, false, reqs)).toBe(false);
  // an unrelated sibling is not gated
  expect(canWritePath("/system/hp", caps, false, reqs)).toBe(true);
});

test("GM bypasses all checks", () => {
  expect(
    canWritePath("/system/vision", new Set(), true, [
      { path_prefix: "/system/vision", caps: ["dnd5e:gm_vision"] },
    ]),
  ).toBe(true);
});
