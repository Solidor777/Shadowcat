import { describe, expect, it } from "vitest";
import { tokenize } from "./lexer";
import { isFormulaError } from "./types";
import { MAX_FORMULA_LENGTH } from "./types";

describe("tokenize", () => {
  it("lexes numbers, words, dots, and operators", () => {
    expect(tokenize("floor(Parent.dex / 2) + 1.5")).toEqual([
      { kind: "word", value: "floor", pos: 0 },
      { kind: "op", value: "(", pos: 5 },
      { kind: "word", value: "parent", pos: 6 },
      { kind: "op", value: ".", pos: 12 },
      { kind: "word", value: "dex", pos: 13 },
      { kind: "op", value: "/", pos: 17 },
      { kind: "num", value: 2, pos: 19 },
      { kind: "op", value: ")", pos: 20 },
      { kind: "op", value: "+", pos: 22 },
      { kind: "num", value: 1.5, pos: 24 },
    ]);
  });
  it("rejects unknown characters as a parse error value", () => {
    const r = tokenize("dex ? 2");
    expect(isFormulaError(r as never)).toBe(true);
  });
  it("rejects over-length sources (cap)", () => {
    const r = tokenize("1+".repeat(300) + "1");
    expect((r as { error: string }).error).toBe("cap");
  });

  it("rejects a second dot in a numeric literal as a parse error", () => {
    const r = tokenize("1.2.3");
    expect(isFormulaError(r as never)).toBe(true);
    expect((r as { error: string }).error).toBe("parse");
  });

  it("tokenizes a source exactly MAX_FORMULA_LENGTH chars long", () => {
    // Build "1+1+...+1" sized to exactly MAX_FORMULA_LENGTH chars (odd length,
    // alternating digit/op, all-finite) so length-boundary and finiteness
    // don't interact.
    const repeats = Math.floor((MAX_FORMULA_LENGTH - 1) / 2);
    const src = "1+".repeat(repeats) + "1".repeat(MAX_FORMULA_LENGTH - repeats * 2);
    expect(src.length).toBe(MAX_FORMULA_LENGTH);
    const r = tokenize(src);
    expect(Array.isArray(r)).toBe(true);
  });

  it("rejects a source one char over MAX_FORMULA_LENGTH (cap)", () => {
    const repeats = Math.floor((MAX_FORMULA_LENGTH - 1) / 2);
    const src = "1+".repeat(repeats) + "1".repeat(MAX_FORMULA_LENGTH - repeats * 2) + "1";
    expect(src.length).toBe(MAX_FORMULA_LENGTH + 1);
    const r = tokenize(src);
    expect((r as { error: string }).error).toBe("cap");
  });

  it("rejects a numeric literal that overflows to Infinity (cap)", () => {
    const r = tokenize("9".repeat(400));
    expect((r as { error: string }).error).toBe("cap");
  });

  it("rejects a trailing dot not followed by a digit as a parse error", () => {
    const r1 = tokenize("5.");
    expect((r1 as { error: string }).error).toBe("parse");
    const r2 = tokenize("5.)");
    expect((r2 as { error: string }).error).toBe("parse");
  });

  it("embeds the full astral code point in the unrecognized-character detail", () => {
    const r = tokenize("dex \u{1F600} 2");
    expect(isFormulaError(r as never)).toBe(true);
    const detail = (r as { detail: string }).detail;
    expect(detail).toContain("\u{1F600}");
  });
});
