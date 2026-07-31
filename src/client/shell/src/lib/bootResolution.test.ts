import { test, expect } from "vitest";
import { resolveBootWorld } from "./bootResolution";
import type { WorldEntry } from "@shadowcat/types";

function world(id: string): WorldEntry {
  return { id, name: `World ${id}`, role: "player" };
}

test("route-world wins over a different lastWorld", () => {
  const worlds = [world("route-world"), world("last-world")];
  const result = resolveBootWorld({ name: "world", id: "route-world" }, "last-world", worlds);
  expect(result).toEqual({ enterWorldId: "route-world", clearLastWorld: false });
});

test("bare load falls back to lastWorld when it still exists", () => {
  const worlds = [world("last-world")];
  const result = resolveBootWorld({ name: "worlds" }, "last-world", worlds);
  expect(result).toEqual({ enterWorldId: "last-world", clearLastWorld: false });
});

test("bare load with no lastWorld resolves to nothing, nothing to clear", () => {
  const result = resolveBootWorld({ name: "worlds" }, null, []);
  expect(result).toEqual({ enterWorldId: null, clearLastWorld: false });
});

test("bare load with a stale lastWorld clears it", () => {
  const result = resolveBootWorld({ name: "worlds" }, "deleted-world", []);
  expect(result).toEqual({ enterWorldId: null, clearLastWorld: true });
});

test("route-world missing from listWorlds falls through to stale handling, ignoring a still-valid lastWorld", () => {
  const worlds = [world("last-world")];
  const result = resolveBootWorld({ name: "world", id: "deleted-route-world" }, "last-world", worlds);
  expect(result).toEqual({ enterWorldId: null, clearLastWorld: true });
});

test("route-world missing from listWorlds falls through to stale handling with no lastWorld", () => {
  const result = resolveBootWorld({ name: "world", id: "deleted-route-world" }, null, []);
  expect(result).toEqual({ enterWorldId: null, clearLastWorld: true });
});
