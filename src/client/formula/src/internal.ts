import { FORMULA_ERROR_KINDS, type FormulaError, type FormulaValue } from "./types";

const FORMULA_ERROR_KIND_SET: ReadonlySet<string> = new Set(FORMULA_ERROR_KINDS);

/** Not part of the package's public surface (`index.ts` does not re-export
 * this module) — shared trust-boundary helpers for `evaluate.ts` and
 * `graph.ts`, both of which validate a consumer-supplied callback's return
 * value against the `FormulaValue` contract. */

/** True shape check for a `FormulaError` — mirrors parser.ts's `isErr`, not
 * the type-only `isFormulaError` (which merely asserts `typeof v !== "number"`
 * and cannot detect a malformed non-number).
 * @param v An untrusted value, typically a consumer callback's return value.
 * @returns `true` only when `v` has a `FormulaErrorKind`-valued `error` field
 * and a string `detail` field.
 * @example
 * ```
 * // not part of the public `@shadowcat/formula` surface (index.ts does not
 * // re-export this module).
 * isWellFormedError({ error: "cap", detail: "too big" }); // true
 * isWellFormedError({ error: "nonsense", detail: "x" });  // false — not a FormulaErrorKind
 * ```
 */
export function isWellFormedError(v: unknown): v is FormulaError {
  return (
    typeof v === "object" &&
    v !== null &&
    typeof (v as { error?: unknown }).error === "string" &&
    FORMULA_ERROR_KIND_SET.has((v as { error: string }).error) &&
    typeof (v as { detail?: unknown }).detail === "string"
  );
}

/** Gates any arithmetic result behind `Number.isFinite` — an overflow to
 * `Infinity`/`-Infinity` (or a stray `NaN`) becomes a `FormulaValue` error
 * instead of leaking past the library boundary (spec §5.2).
 * @param n A raw arithmetic result (e.g. from `+`, `/`, or a builtin call).
 * @returns `n` itself when finite, else a `"non-finite"` `FormulaError`.
 * @example
 * ```
 * // not part of the public `@shadowcat/formula` surface (index.ts does not
 * // re-export this module).
 * finite(1 / 3);  // 0.333...
 * finite(1 / 0);  // { error: "non-finite", detail: "..." } — Infinity never leaks
 * ```
 */
export function finite(n: number): FormulaValue {
  if (!Number.isFinite(n)) {
    return { error: "non-finite", detail: `arithmetic result is not finite (${n})` };
  }
  return n;
}

/** Validates a consumer-supplied callback's return value at the trust
 * boundary: neither a resolver (`evaluate.ts`) nor an `evalNode` (`graph.ts`)
 * is guaranteed to honor `FormulaValue`'s contract. A finite number passes
 * through `finite()` (overflow still rejected); a well-formed `FormulaError`
 * propagates unchanged; anything else (wrong shape, `undefined`, a raw
 * string, etc.) becomes a synthetic `resolver-error` rather than being
 * trusted and later crashing a caller that reads `.detail` off it.
 * @param v The raw return value of a consumer-supplied `resolve`/`evalNode` callback.
 * @returns `v` narrowed to a `FormulaValue` — a finite number, the well-formed
 * `FormulaError` unchanged, or a synthetic `"resolver-error"` for anything else.
 * @example
 * ```
 * // not part of the public `@shadowcat/formula` surface (index.ts does not
 * // re-export this module).
 * validateResolverOutput(5);           // 5
 * validateResolverOutput("oops");      // { error: "resolver-error", detail: "..." }
 * ```
 */
export function validateResolverOutput(v: unknown): FormulaValue {
  if (typeof v === "number") return finite(v);
  if (isWellFormedError(v)) return v;
  return {
    error: "resolver-error",
    detail: `resolver returned a malformed value (expected number or FormulaError, got ${typeof v})`,
  };
}
