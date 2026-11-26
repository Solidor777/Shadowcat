import { describe, expect, it } from "vitest";
import { coerceFormula } from "./coerce";

describe("coerceFormula", () => {
  it("turns a finite numeric literal into a number, trimming first", () => {
    expect(coerceFormula("2")).toBe(2);
    expect(coerceFormula("  -3.5 ")).toBe(-3.5);
    expect(coerceFormula("0")).toBe(0);
  });

  it("keeps a valid formula as its trimmed text", () => {
    expect(coerceFormula(" hp.value + 2 ")).toBe("hp.value + 2");
  });

  it("returns the parse error for text that is neither a number nor a valid formula", () => {
    expect(coerceFormula("1 +")).toMatchObject({ error: "parse" });
    expect(coerceFormula("")).toMatchObject({ error: "parse" });
    expect(coerceFormula("   ")).toMatchObject({ error: "parse" });
  });

  it("rejects the Number() spellings the lexer does not accept instead of converting them", () => {
    expect(coerceFormula("0x1F")).toMatchObject({ error: "parse" });
    expect(coerceFormula("0b11")).toMatchObject({ error: "parse" });
    expect(coerceFormula("1e3")).toMatchObject({ error: "parse" });
  });

  it("treats a non-finite numeral as a reference, never as a number", () => {
    expect(coerceFormula("Infinity")).toBe("Infinity");
    expect(coerceFormula("NaN")).toBe("NaN");
  });
});
