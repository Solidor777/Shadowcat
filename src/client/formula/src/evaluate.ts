import type { Expr } from "./parser";
import { type FormulaValue, isFormulaError } from "./types";
import { finite, validateResolverOutput } from "./internal";

/** Structural recursion over `Expr`. Resolver errors and non-finite arithmetic
 * results both short-circuit as `FormulaValue` errors — the evaluator never
 * throws and never returns `NaN`/`Infinity` (spec §5.2). Operands evaluate
 * left-to-right; the FIRST error encountered wins. Recursion depth is bounded
 * indirectly by `MAX_AST_NODES`, not `MAX_PARSE_DEPTH`: a flat additive/
 * multiplicative chain (e.g. `a + b + c + ...`) builds one `evaluate` stack
 * frame per node while the parser's own depth counter — which only advances
 * at structural nesting boundaries (parens, calls, unary-minus) — never
 * increments across that chain, so `MAX_PARSE_DEPTH` does not bound it. */
export function evaluate(
  expr: Expr,
  resolve: (path: string[]) => FormulaValue,
): FormulaValue {
  switch (expr.kind) {
    case "num":
      return expr.value;

    case "ref": {
      let v: unknown;
      try {
        v = resolve(expr.path);
      } catch (e) {
        return {
          error: "resolver-error",
          detail: `resolver threw: ${e instanceof Error ? e.message : String(e)}`,
        };
      }
      return validateResolverOutput(v);
    }

    case "neg": {
      const v = evaluate(expr.operand, resolve);
      if (isFormulaError(v)) return v;
      return finite(-v);
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
  return finite(result);
}

/** Arity is a caller obligation, not one enforced here: today only
 * `parseFormula`'s `checkArity` validates it, at parse time. `Expr` and
 * `evaluate` are public API, so a hand-constructed `Expr` (bypassing the
 * parser) with the wrong argument count is not defended against in this
 * function — `vals[0]` would read `undefined` for a no-arg `floor`/`ceil`/
 * `round`, which `finite()` then rejects as non-finite rather than crashing. */
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
      // JS-native tie behavior: Math.round rounds ties toward +Infinity, not
      // away from zero — Math.round(-2.5) === -2 (not -3).
      result = Math.round(vals[0]);
      break;
  }
  return finite(result);
}

