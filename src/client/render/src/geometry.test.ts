import { test, expect } from "vitest";
import * as core from "@shadowcat/core";
import { parseColor, rectPoints, ellipsePoints, circlePoints, conePoints, squarePoints, linePoints, DEFAULT_CONE_ANGLE_DEGREES } from "./geometry";

test("parseColor reads #rrggbb (with or without #); malformed → 0", () => {
  expect(parseColor("#ff8000")).toBe(0xff8000);
  expect(parseColor("ff8000")).toBe(0xff8000);
  expect(parseColor("#000000")).toBe(0);
  expect(parseColor("nope")).toBe(0);
  expect(parseColor("#fff")).toBe(0); // short form not supported
});

// The tessellation helpers are defined once in `@shadowcat/core`; the render module re-exports
// them. Identity (not equal output) is what stops a second copy from drifting: a copy would be a
// distinct function object that this fails on.
test("the render tessellation exports are the same function objects as the core ones", () => {
  expect(rectPoints).toBe(core.rectPoints);
  expect(ellipsePoints).toBe(core.ellipsePoints);
  expect(circlePoints).toBe(core.circlePoints);
  expect(conePoints).toBe(core.conePoints);
  expect(squarePoints).toBe(core.squarePoints);
  expect(linePoints).toBe(core.linePoints);
  expect(DEFAULT_CONE_ANGLE_DEGREES).toBe(core.DEFAULT_CONE_ANGLE_DEGREES);
});
