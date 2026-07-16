import { describe, expect, it } from "vitest";
import { parseFormula, type Expr } from "./parser";
import { evaluate } from "./evaluate";
import type { FormulaValue } from "./types";

const ast = (src: string) => parseFormula(src) as Expr;
const env = (vals: Record<string, FormulaValue>) => (path: string[]): FormulaValue =>
  vals[path.join(".")] ?? { error: "unknown-ref", detail: `unknown reference '${path.join(".")}'` };

describe("evaluate", () => {
  it("computes arithmetic with resolver-supplied refs", () => {
    expect(evaluate(ast("floor(parent.str / 2) + dex"), env({ "parent.str": 15, dex: 3 }))).toBe(10);
  });
  it("float division, explicit rounding only", () => {
    expect(evaluate(ast("7 / 2"), env({}))).toBe(3.5);
    expect(evaluate(ast("round(7 / 2)"), env({}))).toBe(4);
  });
  it("division and mod by zero are error values", () => {
    expect(evaluate(ast("1 / dex"), env({ dex: 0 }))).toMatchObject({ error: "div-zero" });
    expect(evaluate(ast("1 % 0"), env({}))).toMatchObject({ error: "div-zero" });
  });
  it("propagates resolver errors unchanged", () => {
    const cyc: FormulaValue = { error: "cycle", detail: "dex -> str -> dex" };
    expect(evaluate(ast("dex + 1"), env({ dex: cyc }))).toEqual(cyc);
  });
  it("unknown refs are error values", () => {
    expect(evaluate(ast("ghost + 1"), env({}))).toMatchObject({ error: "unknown-ref" });
  });
  it("non-finite results are error values", () => {
    // "1e308" is not exponent notation in this lexer (see the parser's 1e999
    // regression) — it lexes as num(1) + word("e308"), a parse error, so it
    // cannot exercise evaluator overflow. Two digit-only literals near the
    // f64 max (1e160 each, well within MAX_FORMULA_LENGTH) whose product
    // overflows to Infinity exercise the same non-finite-result path.
    const big = "1" + "0".repeat(160);
    expect(evaluate(ast(`${big} * ${big}`), env({}))).toMatchObject({ error: "non-finite" });
  });
  it("min/max n-ary", () => {
    expect(evaluate(ast("max(1, dex, 2)"), env({ dex: 9 }))).toBe(9);
  });
});
