// @vitest-environment node
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { bandContains, levelOf, type SceneLevel } from "./levels";

/** One conformance case shared with the server's `scene::elevation` twin. */
interface LevelsCase {
  name: string;
  levels: SceneLevel[];
  elevation: number;
  expect: string | null;
}

const corpus: { cases: LevelsCase[] } = JSON.parse(
  readFileSync(new URL("./__fixtures__/levels-conformance.json", import.meta.url), "utf8"),
);

describe("levels conformance corpus (shared with the server twin)", () => {
  it("has unique case names", () => {
    const names = corpus.cases.map((c) => c.name);
    expect(new Set(names).size).toBe(names.length);
  });

  for (const c of corpus.cases) {
    it(c.name, () => {
      expect(levelOf(c.levels, c.elevation)?.id ?? null).toBe(c.expect);
    });
  }
});

describe("bandContains", () => {
  it("a null band contains every elevation", () => {
    expect(bandContains(null, 0)).toBe(true);
    expect(bandContains(null, -1e6)).toBe(true);
    expect(bandContains(null, 1e6)).toBe(true);
  });

  it("an unbounded-top band contains everything at or above its bottom", () => {
    const band = { bottom: 2, top: null };
    expect(bandContains(band, 2)).toBe(true);
    expect(bandContains(band, 1e6)).toBe(true);
    expect(bandContains(band, 1)).toBe(false);
  });

  it("an inverted band fails closed to containing everything", () => {
    const band = { bottom: 5, top: 1 };
    expect(bandContains(band, 0)).toBe(true);
    expect(bandContains(band, 100)).toBe(true);
    expect(bandContains(band, -100)).toBe(true);
  });

  it("a non-finite endpoint fails closed to containing everything", () => {
    expect(bandContains({ bottom: Number.NaN, top: 1 }, 0)).toBe(true);
    expect(bandContains({ bottom: null, top: Number.NEGATIVE_INFINITY }, 0)).toBe(true);
  });
});
