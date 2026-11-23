import { parseFormula } from "./parser";
import type { FormulaError } from "./types";

/** Coerces the text of a formula input into the value a stored `Formula` field holds: a finite
 * numeric literal becomes a number, any other text that parses is kept as its trimmed source.
 * Only the parse is checked; what a reference means stays the evaluator's decision.
 * @param text The raw input value.
 * @returns The number, the trimmed formula text, or the `FormulaError` the parse returned when
 * `text` is neither a finite number nor a valid formula (an empty `text` is one such error).
 * @example
 * ```ts
 * import { coerceFormula } from "@shadowcat/formula";
 *
 * coerceFormula("2"); // 2
 * coerceFormula(" hp.value + 2 "); // "hp.value + 2"
 * coerceFormula("1 +"); // { error: "parse", detail: "..." }
 * ```
 */
export function coerceFormula(text: string): number | string | FormulaError {
  const trimmed = text.trim();
  const n = Number(trimmed);
  if (trimmed !== "" && Number.isFinite(n)) return n;
  const parsed = parseFormula(trimmed);
  if ("error" in parsed) return parsed;
  return trimmed;
}
