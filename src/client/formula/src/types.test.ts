import { describe, expect, it } from "vitest";
import { isFormulaError, MAX_FORMULA_LENGTH, type FormulaValue } from "./types";

describe("formula value model", () => {
  it("discriminates numbers from error values", () => {
    const ok: FormulaValue = 3;
    const bad: FormulaValue = { error: "parse", detail: "unexpected '?'" };
    expect(isFormulaError(ok)).toBe(false);
    expect(isFormulaError(bad)).toBe(true);
  });
  it("exposes caps", () => {
    expect(MAX_FORMULA_LENGTH).toBe(512);
  });
});
