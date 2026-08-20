import { FORMULA_ERROR_KINDS, type FormulaError, type FormulaValue } from "./types";

const FORMULA_ERROR_KIND_SET: ReadonlySet<string> = new Set(FORMULA_ERROR_KINDS);

// Type-only extraction (no runtime change): both `isWellFormedError` casts an
// untrusted `v` to this same probe shape twice inline; factoring it out gives
// each field one place to carry its doc rather than restating it per cast.
/** Ad-hoc probe shape for reading `error`/`detail` off an untrusted value before
 * either field is confirmed to satisfy `FormulaError`'s stricter contract. */
interface ErrorProbe {
  /** Candidate `FormulaErrorKind` tag — `unknown` until narrowed against `FORMULA_ERROR_KIND_SET`. */
  error?: unknown;
  /** Candidate detail message — `unknown` until narrowed to a string. */
  detail?: unknown;
}

// Not part of the package's public surface — `@shadowcat/formula`'s public
// exports omit this module — shared trust-boundary helpers for `evaluate` and
// `resolveAll`, both of which validate a consumer-supplied callback's return
// value against the `FormulaValue` contract.
// A `//` header, not a `/** */` block: a doc block here would precede another doc
// block rather than a declaration, and every consumer of doc blocks (TypeDoc,
// editor hover, jsdoc lint) binds to the NEAREST preceding one — so it would
// attach to nothing while still reading as attached.

/** True shape check for a `FormulaError` — mirrors `isErr`, not
 * the type-only `isFormulaError` (which merely asserts `typeof v !== "number"`
 * and cannot detect a malformed non-number).
 * @param v An untrusted value, typically a consumer callback's return value.
 * @returns `true` only when `v` has a `FormulaErrorKind`-valued `error` field
 * and a string `detail` field.
 * @example
 * ```
 * // not part of the public `@shadowcat/formula` surface (this module is not
 * // re-exported).
 * isWellFormedError({ error: "cap", detail: "too big" }); // true
 * isWellFormedError({ error: "nonsense", detail: "x" });  // false — not a FormulaErrorKind
 * ```
 */
export function isWellFormedError(v: unknown): v is FormulaError {
  if (typeof v !== "object" || v === null) return false;
  const probe = v as ErrorProbe;
  return (
    typeof probe.error === "string" &&
    FORMULA_ERROR_KIND_SET.has(probe.error) &&
    typeof probe.detail === "string"
  );
}

/** Gates any arithmetic result behind `Number.isFinite` — an overflow to
 * `Infinity`/`-Infinity` (or a stray `NaN`) becomes a `FormulaValue` error
 * instead of leaking past the library boundary.
 * @param n A raw arithmetic result (e.g. from `+`, `/`, or a builtin call).
 * @returns `n` itself when finite, else a `"non-finite"` `FormulaError`.
 * @example
 * ```
 * // not part of the public `@shadowcat/formula` surface (this module is not
 * // re-exported).
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
 * boundary: neither a resolver (`evaluate`'s `ref` case) nor an `evalNode`
 * callback is guaranteed to honor `FormulaValue`'s contract. A finite number passes
 * through `finite()` (overflow still rejected); a well-formed `FormulaError`
 * propagates unchanged; anything else (wrong shape, `undefined`, a raw
 * string, etc.) becomes a synthetic `resolver-error` rather than being
 * trusted and later crashing a caller that reads `.detail` off it.
 * @param v The raw return value of a consumer-supplied `resolve`/`evalNode` callback.
 * @returns `v` narrowed to a `FormulaValue` — a finite number, the well-formed
 * `FormulaError` unchanged, or a synthetic `"resolver-error"` for anything else.
 * @example
 * ```
 * // not part of the public `@shadowcat/formula` surface (this module is not
 * // re-exported).
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

/** Invokes a consumer resolver callback, converting a thrown value into a synthetic
 * `"resolver-error"` `FormulaError` rather than letting it propagate — the try/catch boundary
 * shared by `evaluate`'s `ref` case and `substituteIdentifier`. Does not itself validate a
 * non-throwing return value; the caller still applies `validateResolverOutput` to the result
 * (safe unconditionally — a synthesized resolver-error here passes back through it unchanged,
 * the same `isWellFormedError` path a resolver's own directly-returned `FormulaError` already
 * takes). Never interpolates the caught exception's message: `FormulaError.detail` is
 * player-presentable, a resolver's own thrown message is not.
 * @param path The dotted ref path passed to `resolve`, used only for the error detail if it
 * throws.
 * @param resolve The consumer resolver callback to invoke.
 * @returns `resolve`'s raw, unvalidated return value, or a `"resolver-error"` `FormulaError` if
 * `resolve` threw.
 * @example
 * ```ts
 * import { callResolver, validateResolverOutput } from "./internal"; // not exported from @shadowcat/formula
 * validateResolverOutput(callResolver(["hp", "max"], () => 10)); // 10
 * ```
 */
export function callResolver(
  path: string[],
  resolve: (path: string[]) => FormulaValue,
): unknown {
  try {
    return resolve(path);
  } catch {
    return {
      error: "resolver-error",
      detail: `resolver threw for '${path.join(".")}'`,
    };
  }
}
