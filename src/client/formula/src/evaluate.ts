import type { Expr } from "./parser";
import { type FormulaValue, isFormulaError } from "./types";
import { finite, validateResolverOutput } from "./internal";

/** Structural recursion over `Expr`. Resolver errors and non-finite arithmetic
 * results both short-circuit as `FormulaValue` errors — the evaluator never
 * throws and never returns `NaN`/`Infinity`. Operands evaluate
 * left-to-right; the FIRST error encountered wins. Recursion depth is bounded
 * indirectly by `MAX_AST_NODES`, not `MAX_PARSE_DEPTH`: a flat additive/
 * multiplicative chain (e.g. `a + b + c + ...`) builds one `evaluate` stack
 * frame per node while the parser's own depth counter — which only advances
 * at structural nesting boundaries (parens, calls, unary-minus) — never
 * increments across that chain, so `MAX_PARSE_DEPTH` does not bound it.
 * @param expr An AST produced by `parseFormula`.
 * @param resolve Consumer callback resolving a dotted ref path (e.g.
 * `["hp","max"]`) to a `FormulaValue`. May throw; a thrown value is caught
 * and converted to a `"resolver-error"` rather than propagating.
 * @returns The evaluated number, or the first `FormulaError` encountered.
 * @example
 * ```ts
 * import { evaluate, parseFormula } from "@shadowcat/formula";
 *
 * const expr = parseFormula("1 + hp.max");
 * if (!("error" in expr)) {
 *   evaluate(expr, (path) => (path.join(".") === "hp.max" ? 10 : { error: "unknown-ref", detail: path.join(".") }));
 * }
 * ```
 */
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
      } catch {
        // Never interpolate the caught exception's message: `FormulaError.detail` is
        // player-presentable, and a consumer resolver's thrown
        // message is an internal implementation detail, not for players.
        return {
          error: "resolver-error",
          detail: `resolver threw for '${expr.path.join(".")}'`,
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

/**
 * Applies one already-evaluated binary operator. `/` is float division and
 * `%` is JS's truncated remainder (`-7 % 2 === -1`, not floored modulo) —
 * neither implicitly rounds. `x / 0` and `x % 0` are both `"div-zero"`
 * regardless of the sign or finiteness of `x`; every other result passes
 * through `finite()`, so an overflow (e.g. two huge operands multiplied)
 * becomes `"non-finite"` rather than `Infinity`.
 * @param op One of `+ - * / %`.
 * @param left Left operand, already evaluated to a finite number.
 * @param right Right operand, already evaluated to a finite number.
 * @returns The arithmetic result, or a `"div-zero"`/`"non-finite"` error.
 * @example
 * ```
 * // not part of the public `@shadowcat/formula` surface (this module is not
 * // re-exported).
 * evalBin("%", -7, 2); // -1 (truncated, not floored)
 * evalBin("/", 1, 0);  // { error: "div-zero", detail: "..." }
 * ```
 */
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
 * `round`, which `finite()` then rejects as non-finite rather than crashing.
 * Likewise a hand-constructed zero-arg `min`/`max` call spreads an empty
 * array into `Math.min()`/`Math.max()`, which JS defines as `Infinity`/
 * `-Infinity` respectively — also caught by `finite()`, not a crash.
 * @param fn One of `min max floor ceil round`.
 * @param args Argument expressions, evaluated left-to-right before the call
 * (so an error in an earlier argument is returned before a later one is evaluated).
 * @param resolve Consumer callback forwarded to each argument's `evaluate`.
 * @returns The call's numeric result, or the first error among its
 * arguments, or a `"non-finite"` error (see the arity note above).
 * @example
 * ```
 * // not part of the public `@shadowcat/formula` surface (this module is not
 * // re-exported) — reachable only through `evaluate`'s "call" case.
 * evalCall("round", [{ kind: "num", value: -2.5 }], () => 0); // -2 (ties toward +Infinity)
 * ```
 */
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

