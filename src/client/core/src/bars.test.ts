import { test, expect } from "vitest";
import { parseBars, resolveTokenBarAttribute, barRevealed, EMPTY_BARS } from "./bars";
import type { WireDocument } from "./wire";

const payload = {
  scenes: {
    "scene-1": {
      tokens: {
        "tok-bars": {
          nameplate: "Goblin",
          nameplate_mode: "hover",
          bars: [{ label: "HP", current: 4, max: 10, color: "#cc2222", mode: "always" }],
        },
        "tok-secret": { nameplate: null, nameplate_mode: "always", bars: [] },
      },
    },
    "scene-2": { tokens: { "tok-other": { nameplate: null, nameplate_mode: null, bars: [{ label: "MP", current: 1, max: 2, color: "#2222cc", mode: "owner" }] } } },
  },
};

test("parseBars exposes each token's delivered nameplate, nameplate mode and bars across scenes", () => {
  const bars = parseBars(payload);
  expect(bars.token("tok-bars")).toEqual({
    nameplate: "Goblin",
    nameplateMode: "hover",
    bars: [{ label: "HP", current: 4, max: 10, color: "#cc2222", mode: "always" }],
  });
  expect(bars.token("tok-other")?.bars).toHaveLength(1);
});

test("a delivered nameplate that names nothing is distinct from a nameplate that is not delivered", () => {
  const bars = parseBars(payload);
  expect(bars.token("tok-secret")).toEqual({ nameplate: null, nameplateMode: "always", bars: [] });
  expect(bars.token("tok-other")?.nameplateMode).toBeNull();
});

test("a token the server states nothing for reads null", () => {
  expect(parseBars(payload).token("tok-never-mentioned")).toBeNull();
  expect(EMPTY_BARS.token("anything")).toBeNull();
});

test("a payload that does not validate fails closed to the empty lookup", () => {
  expect(parseBars(null)).toBe(EMPTY_BARS);
  expect(parseBars({ scenes: "x" })).toBe(EMPTY_BARS);
  const badMode = { scenes: { s: { tokens: { t: { nameplate: "n", nameplate_mode: "sometimes", bars: [] } } } } };
  expect(parseBars(badMode)).toBe(EMPTY_BARS);
  const badNumber = { scenes: { s: { tokens: { t: { nameplate: null, nameplate_mode: null, bars: [{ label: "x", current: "4", max: 1, color: "#fff", mode: "always" }] } } } } };
  expect(parseBars(badNumber)).toBe(EMPTY_BARS);
});

test("resolveTokenBarAttribute reads the entry for the token's id", () => {
  const token = { id: "tok-bars" } as WireDocument;
  expect(resolveTokenBarAttribute(token, parseBars(payload))?.nameplate).toBe("Goblin");
  expect(resolveTokenBarAttribute({ id: "nope" } as WireDocument, parseBars(payload))).toBeNull();
});

test("barRevealed draws hover only when emphasized, owner and always continuously, none never", () => {
  expect(barRevealed("hover", false)).toBe(false);
  expect(barRevealed("hover", true)).toBe(true);
  expect(barRevealed("owner", false)).toBe(true);
  expect(barRevealed("always", false)).toBe(true);
  expect(barRevealed("none", true)).toBe(false);
});
