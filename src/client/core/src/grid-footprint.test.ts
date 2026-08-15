import { test, expect } from "vitest";
import { resolveFootprintGeometry } from "./grid-footprint";

test("square: box is the authored w×h block, radius is circle=max/2, square=half-diagonal", () => {
  expect(resolveFootprintGeometry("square", { w: 1, h: 1 }, "square")).toEqual({
    boxW: 1,
    boxH: 1,
    radius: Math.SQRT2 / 2,
  });
  expect(resolveFootprintGeometry("circle", { w: 2, h: 4 }, "square")).toEqual({
    boxW: 2,
    boxH: 4,
    radius: 2,
  });
});

test("an unrecognized grid kind resolves as square (fail-safe default)", () => {
  expect(resolveFootprintGeometry("square", { w: 1, h: 1 }, "triangle")).toEqual({
    boxW: 1,
    boxH: 1,
    radius: Math.SQRT2 / 2,
  });
});

test("hex: a 1-hex token's box is a single hex's own bounding box, radius is the circumradius", () => {
  // A 1×1 token on a hex grid occupies exactly one hex, never a `w:1,h:1` square: the hex it
  // sits in spans `√3` wide, `2` tall, with a circumscribing (conservative-enclosure) radius of
  // `1.0` — not the square half-diagonal `hypot(1,1)/2 ≈ 0.707` a square formula would give.
  expect(resolveFootprintGeometry("square", { w: 1, h: 1 }, "hex")).toEqual({
    boxW: Math.sqrt(3),
    boxH: 2,
    radius: 1,
  });
});

test("hex ignores shape — a hex tessellation has no square/circle footprint distinction", () => {
  expect(resolveFootprintGeometry("circle", { w: 1, h: 1 }, "hex")).toEqual(
    resolveFootprintGeometry("square", { w: 1, h: 1 }, "hex"),
  );
});

test("hex: an N-hex token scales linearly by n = max(w,h)", () => {
  expect(resolveFootprintGeometry("square", { w: 2, h: 1 }, "hex")).toEqual({
    boxW: 2 * Math.sqrt(3),
    boxH: 4,
    radius: 2,
  });
});
