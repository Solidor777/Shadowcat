// #region rules
import { parseFormula, evaluate, isFormulaError, type FormulaValue } from "@shadowcat/formula";

/**
 * Ability-score modifier under the d20-family convention THIS example system
 * chooses to use: `floor((score - 10) / 2)`. This is a convention of this
 * system's opaque `system` body, not an engine rule — Shadowcat's server
 * treats `system` as opaque and never interprets it semantically
 * (docs/design/ARCHITECTURE.md §2 invariant 6).
 * @param score - The raw ability score (e.g. 16).
 * @returns The floored modifier: `floor((score - 10) / 2)`.
 * @example
 * ```ts
 * import { abilityMod } from "shadowcat-example-system-minimal";
 *
 * const mod = abilityMod(16); // 3
 * ```
 */
export function abilityMod(score: number): number {
  return Math.floor((score - 10) / 2);
}

/**
 * Evaluates a formula string against an actor's opaque `system` body, resolving
 * dotted references (`attributes.str`) as paths into it. Returns null (never
 * throws) on parse or evaluation failure — degenerate sheet data must not crash
 * the sheet. This function itself has no `try`; the guarantee is inherited from
 * `@shadowcat/formula`: neither `parseFormula`
 * (src/client/formula/src/parser.ts:388) nor `evaluate`
 * (src/client/formula/src/evaluate.ts:5-7) ever throws, and `evaluate` also
 * catches a throwing resolver callback itself rather than propagating
 * (src/client/formula/src/evaluate.ts:39-49) — which covers the resolver below.
 * @param formula - The formula source text (e.g. `"attributes.str + 2"`).
 * @param system - The opaque `system` body to resolve dotted references against.
 * @returns The evaluated number, or `null` on any parse/evaluation failure.
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
  // This resolver reports a missing/non-numeric stat using "unknown-ref", one of
  // @shadowcat/formula's own FormulaErrorKind values (src/client/formula/src/types.ts:5)
  // — the library only validates the tag and passes it through unchanged; it does
  // not detect the missing/non-numeric condition itself
  // (src/client/formula/src/internal.ts:76-83).
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
