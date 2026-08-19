import { test, expect } from "vitest";
import { parseFootprints, EMPTY_FOOTPRINTS } from "./footprints";

const payload = {
  scenes: [
    {
      scene: "scene-1",
      unit: { w: 173.20508075688772, h: 200 },
      tokens: [
        { token: "tok-hex", extent: { w: 346.41016151377545, h: 400 } },
        { token: "tok-refused", extent: null },
      ],
    },
    { scene: "scene-2", unit: { w: 100, h: 100 }, tokens: [] },
  ],
};

test("parseFootprints exposes each scene's unit extent and each token's resolved extent", () => {
  const fp = parseFootprints(payload);
  expect(fp.unit("scene-1")).toEqual({ w: 173.20508075688772, h: 200 });
  expect(fp.unit("scene-2")).toEqual({ w: 100, h: 100 });
  expect(fp.token("tok-hex")).toEqual({ w: 346.41016151377545, h: 400 });
});

test("a refused extent reads the same as an unstated one", () => {
  // The server states `null` for a token whose authored size it declines to resolve. The client
  // has no permissive reading available for either: both fall back to the token's own authored
  // extent, and neither grants anything, since drawing and picking are all an extent is used for.
  const fp = parseFootprints(payload);
  expect(fp.token("tok-refused")).toBeNull();
  expect(fp.token("tok-never-mentioned")).toBeNull();
  expect(fp.unit("scene-absent")).toBeNull();
  expect(fp.unit(null)).toBeNull();
});

test("a payload that does not validate yields nothing at all, never a partial read", () => {
  for (const bad of [
    undefined,
    null,
    {},
    { scenes: "no" },
    { scenes: [{ scene: "s", unit: { w: 1 }, tokens: [] }] },
    { scenes: [{ scene: "s", unit: { w: Number.NaN, h: 1 }, tokens: [] }] },
    { scenes: [{ scene: "s", unit: { w: -1, h: 1 }, tokens: [] }] },
    { scenes: [{ scene: "s", unit: { w: 1, h: 1 }, tokens: [{ token: "t", extent: { w: "wide", h: 1 } }] }] },
  ]) {
    const fp = parseFootprints(bad);
    expect(fp.unit("s")).toBeNull();
    expect(fp.token("t")).toBeNull();
  }
});

test("EMPTY_FOOTPRINTS answers nothing for every query", () => {
  expect(EMPTY_FOOTPRINTS.token("anything")).toBeNull();
  expect(EMPTY_FOOTPRINTS.unit("anything")).toBeNull();
});
