/** Marked failure value. INVARIANT: library functions return these — they never throw
 * and never emit NaN/Infinity (spec §3.2 fail-closed error values). */
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

export interface FormulaError {
  readonly error: FormulaErrorKind;
  /** Player-presentable, e.g. "unexpected '?' at position 4". Never internal dumps. */
  readonly detail: string;
}

export type FormulaValue = number | FormulaError;

export function isFormulaError(v: FormulaValue): v is FormulaError {
  return typeof v !== "number";
}

export const MAX_FORMULA_LENGTH = 512;
export const MAX_AST_NODES = 256;
export const MAX_PARSE_DEPTH = 32;
export const MAX_GRAPH_VISITS = 2048;
