import { test, expect } from "vitest";
import { LayerRegistry, CORE_LAYERS } from "./index";

test("core layers are in the fixed §6.1 z-order", () => {
  const r = new LayerRegistry();
  expect(r.orderedIds()).toEqual([...CORE_LAYERS]);
  expect(CORE_LAYERS).toEqual([
    "background", "grid", "tiles", "drawings", "walls",
    "tokens", "templates", "mask", "overlays",
  ]);
});

test("a module layer is spliced by ascending order; dispose removes it", () => {
  const r = new LayerRegistry();
  const dispose = r.register("fx", 6.5); // between tokens(5) and templates(6)
  const ids = r.orderedIds();
  expect(ids.indexOf("fx")).toBeGreaterThan(ids.indexOf("tokens"));
  expect(ids.indexOf("fx")).toBeLessThan(ids.indexOf("mask"));
  dispose();
  expect(r.orderedIds()).not.toContain("fx");
});

test("registering a reserved core id or duplicate throws", () => {
  const r = new LayerRegistry();
  expect(() => r.register("tokens", 1)).toThrow();
  r.register("fx", 6.5);
  expect(() => r.register("fx", 7)).toThrow();
});
