// #region rules
import { parseFormula, evaluate, isFormulaError, type FormulaValue } from "@shadowcat/formula";

/** d20-family ability modifier: floor((score - 10) / 2). */
export function abilityMod(score: number): number {
  return Math.floor((score - 10) / 2);
}

/**
 * Evaluates a formula string against an actor's opaque `system` body, resolving
 * dotted references (`attributes.str`) as paths into it. Returns null (never
 * throws) on parse or evaluation failure — degenerate sheet data must not crash
 * the sheet.
 * @example
 * ```ts
 * import { evalFormula } from "shadowcat-example-system-minimal";
 *
 * const total = evalFormula("attributes.str + 2", { attributes: { str: 16 } }); // 18
 * ```
 */
export function evalFormula(formula: string, system: unknown): number | null {
  const expr = parseFormula(formula);
  if ("error" in expr) return null;
  // The formula library never throws (fail-closed error VALUES); the resolver
  // reports a missing/non-numeric stat as the library's own unknown-ref error.
  const value: FormulaValue = evaluate(expr, (path) => {
    let node: unknown = system;
    for (const key of path) node = (node as Record<string, unknown> | undefined)?.[key];
    return typeof node === "number"
      ? node
      : { error: "unknown-ref", detail: `no numeric value at ${path.join(".")}` };
  });
  return isFormulaError(value) ? null : value;
}
// #endregion rules
