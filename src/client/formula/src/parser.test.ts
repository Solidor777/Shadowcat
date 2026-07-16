import { describe, expect, it } from "vitest";
import { parseFormula } from "./parser";

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
});
