import { test, expect } from "vitest";
import { LayerRegistry, CORE_LAYERS } from "./index";

test("core layers are in the fixed z-order", () => {
  const r = new LayerRegistry();
  expect(r.orderedIds()).toEqual([...CORE_LAYERS]);
  expect(CORE_LAYERS).toEqual([
    "background", "grid", "tiles", "regions", "drawings", "walls",
    "tokens", "templates", "vfx", "lighting", "mask", "overlays",
  ]);
});

test("vfx sits between templates and lighting, below the fog mask", () => {
  // Below `mask`: a VFX one-shot reaches every recipient over the wire, but the
  // fog mask still visually hides it at a point the recipient cannot see.
  expect(CORE_LAYERS.indexOf("vfx")).toBe(8);
  expect(CORE_LAYERS.indexOf("vfx")).toBeLessThan(CORE_LAYERS.indexOf("lighting"));
  expect(CORE_LAYERS.indexOf("vfx")).toBeLessThan(CORE_LAYERS.indexOf("mask"));
  expect(CORE_LAYERS.indexOf("vfx")).toBeGreaterThan(CORE_LAYERS.indexOf("templates"));
});

test("a module layer is spliced by ascending order; dispose removes it", () => {
  const r = new LayerRegistry();
  const dispose = r.register("fx", 6.5); // between tokens(6) and templates(7); lighting(9), mask(10)
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
