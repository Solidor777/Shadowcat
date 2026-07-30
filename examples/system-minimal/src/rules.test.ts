import { describe, it, expect } from "vitest";
import { abilityMod, evalFormula } from "./rules";

describe("abilityMod", () => {
  it("computes the d20-family modifier", () => {
    expect(abilityMod(10)).toBe(0);
    expect(abilityMod(8)).toBe(-1);
    expect(abilityMod(18)).toBe(4);
  });
});

describe("evalFormula", () => {
  it("evaluates formulas against the system body", () => {
    const sys = { attributes: { str: 16, dex: 12 } };
    expect(evalFormula("attributes.str + attributes.dex", sys)).toBe(28);
  });
  it("returns null on a malformed formula instead of throwing", () => {
    expect(evalFormula("attributes.str +", { attributes: { str: 1 } })).toBeNull();
  });
  it("returns null when a referenced value is missing", () => {
    expect(evalFormula("attributes.wis", { attributes: { str: 1 } })).toBeNull();
  });
});
