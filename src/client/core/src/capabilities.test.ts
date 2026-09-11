import { expect, test } from "vitest";
import { resolveCaps, canWritePath, canCreateDoc } from "./capabilities";
import { grantAuthor, envelope } from "./scene-docs";
import type { WireDocument } from "./wire";

const emptyGrants = { by_role: {}, by_user: {} };

function perms(p: Partial<WireDocument["permissions"]>): WireDocument["permissions"] {
  return {
    default: "none",
    users: {},
    property_overrides: {},
    capabilities: { by_role: {}, by_user: {} },
    gm_role: null,
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

test("canWritePath treats /engine and /name like /system (write_fields, /name is a leaf)", () => {
  const caps = new Set(["core:read", "core:write_fields"]);
  expect(canWritePath("/engine", caps, false, [])).toBe(true);
  expect(canWritePath("/engine/x", caps, false, [])).toBe(true);
  expect(canWritePath("/engine_x", caps, false, [])).toBe(false); // boundary neighbor, not a match
  expect(canWritePath("/name", caps, false, [])).toBe(true);
  const noWrite = new Set(["core:read"]);
  expect(canWritePath("/engine/x", noWrite, false, [])).toBe(false);
  expect(canWritePath("/name", noWrite, false, [])).toBe(false);
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

test("canWritePath maps /base to no capability (server-owned), boundary neighbor included", () => {
  const caps = new Set([
    "core:read",
    "core:write_fields",
    "core:manage_embedded",
    "core:edit_permissions",
  ]);
  expect(canWritePath("/base", caps, false, [])).toBe(false); // server-owned — same posture as /source
  expect(canWritePath("/based", caps, false, [])).toBe(false); // boundary neighbor, not a match
});

test("GM bypasses all checks", () => {
  expect(
    canWritePath("/system/vision", new Set(), true, [
      { path_prefix: "/system/vision", caps: ["dnd5e:gm_vision"] },
    ]),
  ).toBe(true);
});

test("canCreateDoc: GM always may create", () => {
  expect(canCreateDoc("note", "gm", { all: [], by_type: {} })).toBe(true);
});

test("canCreateDoc: an `all` grant covers every doc_type", () => {
  expect(canCreateDoc("note", "player", { all: ["core:create"], by_type: {} })).toBe(true);
  expect(canCreateDoc("table", "player", { all: ["core:create"], by_type: {} })).toBe(true);
});

test("canCreateDoc: a by_type[docType] grant covers only that doc_type", () => {
  const roleCaps = { all: [], by_type: { note: ["core:create"] } };
  expect(canCreateDoc("note", "player", roleCaps)).toBe(true);
  expect(canCreateDoc("table", "player", roleCaps)).toBe(false);
});

test("canCreateDoc: a player with nothing granted may not create", () => {
  expect(canCreateDoc("note", "player", { all: [], by_type: {} })).toBe(false);
});

test("grantAuthor'd document resolves core:delete + core:edit_permissions for the owner and nothing extra for an observer", () => {
  const doc = grantAuthor(envelope("w1", "note", null, {}), "u1");
  const owned = resolveCaps(doc.permissions, "u1", "player", emptyGrants);
  expect(owned.has("core:delete")).toBe(true);
  expect(owned.has("core:edit_permissions")).toBe(true);

  const observed = resolveCaps(doc.permissions, "someone-else", "player", emptyGrants);
  expect(observed.has("core:delete")).toBe(false);
  expect(observed.has("core:edit_permissions")).toBe(false);
});
