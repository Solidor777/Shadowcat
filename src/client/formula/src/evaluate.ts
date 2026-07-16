import type { Expr } from "./parser";
import { type FormulaValue, isFormulaError } from "./types";

/** Structural recursion over `Expr`. Resolver errors and non-finite arithmetic
 * results both short-circuit as `FormulaValue` errors — the evaluator never
 * throws and never returns `NaN`/`Infinity` (spec §5.2). Operands evaluate
 * left-to-right; the FIRST error encountered wins. Recursion depth mirrors
 * the parser's `MAX_PARSE_DEPTH`-bounded AST shape, so no separate cap is
 * needed here. */
export function evaluate(
  expr: Expr,
  resolve: (path: string[]) => FormulaValue,
): FormulaValue {
  switch (expr.kind) {
    case "num":
      return expr.value;

    case "ref":
      return resolve(expr.path);

    case "neg": {
      const v = evaluate(expr.operand, resolve);
      if (isFormulaError(v)) return v;
      return finite(-v, "non-finite");
    }

    case "bin": {
      const left = evaluate(expr.left, resolve);
      if (isFormulaError(left)) return left;
      const right = evaluate(expr.right, resolve);
      if (isFormulaError(right)) return right;
      return evalBin(expr.op, left, right);
    }

    case "call":
      return evalCall(expr.fn, expr.args, resolve);
  }
}

function evalBin(
  op: "+" | "-" | "*" | "/" | "%",
  left: number,
  right: number,
): FormulaValue {
  if ((op === "/" || op === "%") && right === 0) {
    return { error: "div-zero", detail: `division by zero (${op === "/" ? "'/'" : "'%'"})` };
  }
  let result: number;
  switch (op) {
    case "+":
      result = left + right;
      break;
    case "-":
      result = left - right;
      break;
    case "*":
      result = left * right;
      break;
    case "/":
      // Float division per Global Constraints — no implicit rounding.
      result = left / right;
      break;
    case "%":
      // JS truncated remainder (not floored modulo).
      result = left % right;
      break;
  }
  return finite(result, "non-finite");
}

function evalCall(
  fn: "min" | "max" | "floor" | "ceil" | "round",
  args: Expr[],
  resolve: (path: string[]) => FormulaValue,
): FormulaValue {
  const vals: number[] = [];
  for (const arg of args) {
    const v = evaluate(arg, resolve);
    if (isFormulaError(v)) return v;
    vals.push(v);
  }
  let result: number;
  switch (fn) {
    case "min":
      result = Math.min(...vals);
      break;
    case "max":
      result = Math.max(...vals);
      break;
    case "floor":
      result = Math.floor(vals[0]);
      break;
    case "ceil":
      result = Math.ceil(vals[0]);
      break;
    case "round":
      result = Math.round(vals[0]);
      break;
  }
  return finite(result, "non-finite");
}

/** Gates any arithmetic result behind `Number.isFinite` — an overflow to
 * `Infinity`/`-Infinity` (or a stray `NaN`) becomes a `FormulaValue` error
 * instead of leaking past the library boundary (spec §5.2). */
function finite(n: number, kind: "non-finite"): FormulaValue {
  if (!Number.isFinite(n)) {
    return { error: kind, detail: `arithmetic result is not finite (${n})` };
  }
  return n;
}
