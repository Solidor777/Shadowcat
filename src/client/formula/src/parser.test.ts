import { describe, expect, it } from "vitest";
import { parseFormula } from "./parser";

/** Builds a formula string whose parsed AST has exactly
 * `prefixNegs + 1 + 2 * plusOnes` nodes: `prefixNegs` unary-neg nodes,
 * 1 num node for the leading literal, then `plusOnes` repetitions of
 * `+1` each contributing 1 num node + 1 bin node. */
function nodeCountFormula(prefixNegs: number, plusOnes: number): string {
  return "-".repeat(prefixNegs) + "1" + "+1".repeat(plusOnes);
}

describe("parseFormula", () => {
  it("parses precedence: unary minus > mul > add", () => {
    expect(parseFormula("1 + 2 * -dex")).toEqual({
      kind: "bin", op: "+",
      left: { kind: "num", value: 1 },
      right: { kind: "bin", op: "*", left: { kind: "num", value: 2 },
               right: { kind: "neg", operand: { kind: "ref", path: ["dex"] } } },
    });
  });
  it("parses dotted refs and calls", () => {
    expect(parseFormula("min(hp.max, floor(parent.str / 2))")).toEqual({
      kind: "call", fn: "min", args: [
        { kind: "ref", path: ["hp", "max"] },
        { kind: "call", fn: "floor", args: [
          { kind: "bin", op: "/", left: { kind: "ref", path: ["parent", "str"] },
            right: { kind: "num", value: 2 } } ] },
      ],
    });
  });
  it("a word before '(' must be a known function", () => {
    expect(parseFormula("dex(1)")).toMatchObject({ error: "parse" });
  });
  it("enforces arity: floor/ceil/round take 1 arg, min/max take >= 1", () => {
    expect(parseFormula("floor(1, 2)")).toMatchObject({ error: "parse" });
    expect(parseFormula("min()")).toMatchObject({ error: "parse" });
  });
  it("caps AST size and nesting depth", () => {
    expect(parseFormula("1" + "+1".repeat(200))).toMatchObject({ error: "cap" });
    expect(parseFormula("(".repeat(40) + "1" + ")".repeat(40))).toMatchObject({ error: "cap" });
  });
  it("rejects trailing garbage", () => {
    expect(parseFormula("1 + 2 3")).toMatchObject({ error: "parse" });
  });

  describe("depth cap: exact 32-level boundary, uniform across constructs", () => {
    it("32 nested parens parses; 33 caps", () => {
      const at32 = "(".repeat(32) + "1" + ")".repeat(32);
      const at33 = "(".repeat(33) + "1" + ")".repeat(33);
      expect(parseFormula(at32)).toEqual({ kind: "num", value: 1 });
      expect(parseFormula(at33)).toMatchObject({ error: "cap" });
    });

    it("32 nested single-arg calls parses; 33 caps", () => {
      const at32 = "floor(".repeat(32) + "1" + ")".repeat(32);
      const at33 = "floor(".repeat(33) + "1" + ")".repeat(33);
      expect(parseFormula(at32)).toMatchObject({ kind: "call", fn: "floor" });
      expect(parseFormula(at33)).toMatchObject({ error: "cap" });
    });

    it("a 32-long unary-minus chain parses; 33 caps", () => {
      const at32 = "-".repeat(32) + "1";
      const at33 = "-".repeat(33) + "1";
      expect(parseFormula(at32)).toMatchObject({ kind: "neg" });
      expect(parseFormula(at33)).toMatchObject({ error: "cap" });
    });
  });

  describe("sad-path battery", () => {
    it("unterminated paren is a parse error", () => {
      expect(parseFormula("(1+2")).toMatchObject({ error: "parse" });
    });
    it("unterminated call is a parse error", () => {
      expect(parseFormula("min(1")).toMatchObject({ error: "parse" });
    });
    it("dangling dot is a parse error", () => {
      expect(parseFormula("hp.")).toMatchObject({ error: "parse" });
    });
    it("empty and whitespace-only input are parse errors", () => {
      expect(parseFormula("")).toMatchObject({ error: "parse" });
      expect(parseFormula("   ")).toMatchObject({ error: "parse" });
    });
    it("trailing comma in call args is a parse error", () => {
      expect(parseFormula("floor(1,)")).toMatchObject({ error: "parse" });
    });
    it("a bare allowlisted function name used as an identifier parses as a ref (no reserved names)", () => {
      expect(parseFormula("floor + 1")).toEqual({
        kind: "bin",
        op: "+",
        left: { kind: "ref", path: ["floor"] },
        right: { kind: "num", value: 1 },
      });
    });
    it("a dotted path is never callable — a parse error", () => {
      expect(parseFormula("hp.max(1)")).toMatchObject({ error: "parse" });
    });
    it("wrong arity on ceil/round is a parse error", () => {
      expect(parseFormula("ceil()")).toMatchObject({ error: "parse" });
      expect(parseFormula("round(1,2)")).toMatchObject({ error: "parse" });
    });
    it("exact MAX_AST_NODES boundary: 256 nodes parses, 257 caps", () => {
      // 256 = 1 leading neg + 1 num + 127 * (num + bin); 257 adds one more neg.
      const at256 = nodeCountFormula(1, 127);
      const at257 = nodeCountFormula(2, 127);
      expect(at256.length).toBeLessThanOrEqual(512);
      expect(at257.length).toBeLessThanOrEqual(512);
      expect(parseFormula(at256)).not.toMatchObject({ error: "cap" });
      expect(parseFormula(at257)).toMatchObject({ error: "cap" });
    });
  });

  describe("EOF error positions cite end-of-input, not a stale token", () => {
    it("unterminated paren cites the source length", () => {
      const src = "(1+2";
      expect(parseFormula(src)).toMatchObject({
        error: "parse",
        detail: expect.stringContaining(`position ${src.length}`),
      });
    });
    it("dangling dot cites the source length", () => {
      const src = "hp.";
      expect(parseFormula(src)).toMatchObject({
        error: "parse",
        detail: expect.stringContaining(`position ${src.length}`),
      });
    });
  });

  it("1e999 is not exponent notation — lexes as num(1) + word('e999'), a trailing-input parse error", () => {
    expect(parseFormula("1e999")).toMatchObject({ error: "parse" });
  });
});
