/** Marked failure value. INVARIANT: library functions return these — they never throw
 * and never emit NaN/Infinity; every error path returns a value instead (fail-closed). */
export type FormulaErrorKind =
  | "parse"        // source text does not lex/parse
  | "unknown-ref"  // resolver had no value for a reference (consumer-originated)
  | "type"         // non-numeric operand (consumer-originated, e.g. text stat)
  | "div-zero"     // x / 0 or x % 0
  | "non-finite"   // arithmetic overflowed to Infinity/NaN
  | "cycle"        // reference cycle in graph resolution
  | "cap"          // a DoS bound tripped
  | "ref-error"    // referenced value was itself an error (propagation wrapper)
  | "resolver-error"; // resolver threw, or returned a value that is neither a number
                      // nor a well-formed FormulaError (consumer-originated boundary fault)

/** Runtime mirror of `FormulaErrorKind`, kept in sync by hand — the single source of truth
 * consumers use to validate an untrusted `error` tag against the actual union (rather than
 * merely checking `typeof === "string"`, which admits any string). */
export const FORMULA_ERROR_KINDS = [
  "parse",
  "unknown-ref",
  "type",
  "div-zero",
  "non-finite",
  "cycle",
  "cap",
  "ref-error",
  "resolver-error",
] as const satisfies readonly FormulaErrorKind[];

/** A library failure value — see `FormulaErrorKind`'s doc comment for the
 * fail-closed contract this shape exists to satisfy. */
export interface FormulaError {
  /** Which failure category occurred; see `FormulaErrorKind`'s per-variant comments. */
  readonly error: FormulaErrorKind;
  /** Player-presentable, e.g. "unexpected '?' at position 4". Never internal dumps. */
  readonly detail: string;
}

/** A function's result: either a finite number or a `FormulaError` — the
 * library never throws and never returns `NaN`/`Infinity` in this slot. */
export type FormulaValue = number | FormulaError;

/**
 * Type-guard distinguishing a `FormulaError` from a successful numeric result.
 * A type-only check (`typeof v !== "number"`) — it trusts that `v` already
 * conforms to `FormulaValue`'s shape; it does not verify a malformed object
 * is a well-formed `FormulaError` (that stronger check is
 * `isWellFormedError`, used at consumer-callback trust boundaries).
 * @param v A value returned by `evaluate`, `resolveAll`, or `resolveNotationTemplate`.
 * @returns `true` when `v` is a `FormulaError` (not a finite number).
 * @example
 * ```ts
 * import { evaluate, isFormulaError, parseFormula } from "@shadowcat/formula";
 *
 * const expr = parseFormula("1 / 0");
 * if (!("error" in expr)) {
 *   const result = evaluate(expr, () => 0);
 *   if (isFormulaError(result)) console.log(result.error); // "div-zero"
 * }
 * ```
 */
export function isFormulaError(v: FormulaValue): v is FormulaError {
  return typeof v !== "number";
}

export const MAX_FORMULA_LENGTH = 512;
export const MAX_AST_NODES = 256;
export const MAX_PARSE_DEPTH = 32;
export const MAX_GRAPH_VISITS = 2048;
