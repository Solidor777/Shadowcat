import { describe, expect, it } from "vitest";
import { tokenize } from "./lexer";
import { isFormulaError } from "./types";

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
});
