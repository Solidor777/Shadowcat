// @vitest-environment node
import { describe, it, expect } from "vitest";
import { seedFromRollId, mulberry32 } from "./rng";

describe("seedFromRollId", () => {
  it("is deterministic: the same roll id always hashes to the same seed", () => {
    const id = "11111111-1111-1111-1111-111111111111";
    expect(seedFromRollId(id)).toBe(seedFromRollId(id));
  });

  it("different roll ids hash to different seeds", () => {
    expect(seedFromRollId("a")).not.toBe(seedFromRollId("b"));
  });
});

describe("mulberry32", () => {
  it("the same seed produces the same sequence", () => {
    const a = mulberry32(7);
    const b = mulberry32(7);
    expect([a(), a(), a()]).toEqual([b(), b(), b()]);
  });

  it("different seeds produce different first values", () => {
    expect(mulberry32(1)()).not.toBe(mulberry32(2)());
  });

  it("every value stays in [0, 1)", () => {
    const next = mulberry32(123);
    for (let i = 0; i < 50; i++) {
      const v = next();
      expect(v).toBeGreaterThanOrEqual(0);
      expect(v).toBeLessThan(1);
    }
  });
});
