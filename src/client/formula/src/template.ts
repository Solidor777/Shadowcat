import { MAX_FORMULA_LENGTH, type FormulaError, type FormulaValue, isFormulaError } from "./types";
import { validateResolverOutput } from "./internal";

/** Identifier words whose leading-alpha prefix means dice notation, not a stat.
 * Mirrors src/server/src/dice/notation/parser.rs keyword match (kh/kl/dh/dl/r/ro/cs/cf/t/e)
 * plus the 'd' dice operator. M13b's authoring validation imports this list. */
export const NOTATION_KEYWORDS: readonly string[] =
  ["d", "kh", "kl", "dh", "dl", "r", "ro", "cs", "cf", "t", "e"];

const I32_MAX = 2147483647;

/**
 * True for a character that may START a notation keyword or identifier:
 * a letter or `_`. Distinct from `lexer.ts`'s `isWordStart` (a separate
 * scanner over a separate grammar — this module rewrites dice-notation
 * template text, not formula source).
 * @param ch A single character.
 * @returns `true` when `ch` matches `[a-z_]` case-insensitively.
 * @example
 * ```
 * // not part of the public `@shadowcat/formula` surface — this helper is not exported.
 * isAlpha("_"); // true
 * ```
 */
function isAlpha(ch: string): boolean {
  return /[a-z_]/i.test(ch);
}

/**
 * True for a character that may CONTINUE an identifier or keyword already
 * begun by `isAlpha` — letters, digits, and `_`.
 * @param ch A single character.
 * @returns `true` when `ch` matches `[a-z0-9_]` case-insensitively.
 * @example
 * ```
 * // not part of the public `@shadowcat/formula` surface — this helper is not exported.
 * isWordChar("3"); // true
 * ```
 */
function isWordChar(ch: string): boolean {
  return /[a-z0-9_]/i.test(ch);
}

/**
 * True for an ASCII decimal digit — used to scan a literal count/sides run
 * in the surrounding dice-notation text (e.g. the `20` in `1d20`), which
 * this module passes through unchanged rather than resolving.
 * @param ch A single character.
 * @returns `true` when `ch` is `'0'`–`'9'`.
 * @example
 * ```
 * // not part of the public `@shadowcat/formula` surface — this helper is not exported.
 * isDigit("7"); // true
 * ```
 */
function isDigit(ch: string): boolean {
  return ch >= "0" && ch <= "9";
}

/** Reads the maximal `[a-z_]+` prefix (case-insensitive) starting at `i`.
 * @param src The template source text.
 * @param i Index to start scanning from (must point at an `isAlpha` character).
 * @returns The longest run of `isAlpha` characters starting at `i` (never empty
 * when `src[i]` is itself alphabetic).
 * @example
 * ```
 * // not part of the public `@shadowcat/formula` surface — this helper is not exported.
 * readAlphaPrefix("kh3", 0); // "kh"
 * ```
 */
function readAlphaPrefix(src: string, i: number): string {
  let j = i;
  while (j < src.length && isAlpha(src[j])) j++;
  return src.slice(i, j);
}

/** Resolves a `.`-joined identifier path (e.g. "hp.max") to a labeled substitution.
 * INVARIANT: never throws — resolver faults propagate as ref-error/type/cap FormulaErrors.
 * @param originalText The dotted identifier as it appeared in the template (e.g. `"hp.max"`).
 * @param resolve Consumer callback resolving the dotted path to a `FormulaValue`. May throw;
 * a thrown value is caught and converted to `"resolver-error"` rather than propagating.
 * @returns The labeled substitution text on success (see the negative-value note below),
 * or a `FormulaError` — `"resolver-error"` (thrown/malformed resolver output), `"type"`
 * (a non-integer resolved value — roll templates require integers), or `"cap"` (magnitude
 * exceeds `i32::MAX`).
 * @example
 * ```
 * // not part of the public `@shadowcat/formula` surface — this helper is not exported;
 * // reachable only through `resolveNotationTemplate`.
 * substituteIdentifier("hp.max", () => 10); // "10[hp.max]"
 * ```
 */
function substituteIdentifier(
  originalText: string,
  resolve: (path: string[]) => FormulaValue,
): string | FormulaError {
  const path = originalText.split(".");
  let rawValue: unknown;
  try {
    rawValue = resolve(path);
  } catch {
    return { error: "resolver-error", detail: `resolver threw for '${originalText}'` };
  }
  // Trust-boundary validation: `resolve` is a consumer-supplied callback and is not
  // guaranteed to honor the `FormulaValue` contract (same boundary evaluate.ts's `ref`
  // case and graph.ts's `evalNode` call already cross via this shared helper).
  const value = validateResolverOutput(rawValue);
  if (isFormulaError(value)) return value;
  if (!Number.isInteger(value)) {
    return {
      error: "type",
      detail: `'${originalText}' = ${value}: roll templates require integers (use floor/round in the stat formula)`,
    };
  }
  // Intentionally asymmetric: spec formula is `abs(value) > i32::MAX`, so the true i32
  // minimum (-2147483648) is rejected as a cap error even though it IS representable in
  // an i32. This mirrors the spec literally — do not "fix" it into a symmetric range check.
  if (Math.abs(value) > I32_MAX) {
    return { error: "cap", detail: `'${originalText}' = ${value}: out of i32 range` };
  }
  // A negative value is emitted as a parenthesized subtraction, never a
  // leading '-' (and unlabeled, unlike the positive branch below): the
  // dice-notation lexer (src/server/src/dice/notation/lexer.rs) tokenizes
  // each '-' independently — it never merges "--" into any other token —
  // and the grammar's `factor := '-' factor | ...` (parser.rs) accepts
  // arbitrarily many stacked unary-minus. So if this substitution's own
  // output started with '-' and the template text immediately preceding
  // this identifier ALSO ends in a literal '-' (e.g. the template
  // "atk-str" with `str` resolving to -5), the composed notation
  // "atk--5[str]" would parse as `atk - -(5)`, silently CANCELLING the
  // negative sign. Opening with '(' instead is unambiguous in any
  // preceding context (it never combines with an adjacent '-'), so the
  // sign survives regardless of what character the template placed
  // immediately before this identifier.
  if (value < 0) return `(0 - ${-value})`;
  return `${value}[${originalText}]`;
}

/** Rewrites a dice-notation template: identifiers resolve to labeled constants, existing
 * dice-notation atoms (and `[label]` spans) pass through untouched. Spec §3 template mode.
 * INVARIANT: never throws; every failure path returns a FormulaError.
 * @param src Template text, e.g. `"1d20 + str"` — a mix of dice-notation atoms
 * (numbers, the `d` operator, `NOTATION_KEYWORDS` modifiers, `[label]` spans)
 * and dotted identifier references.
 * @param resolve Consumer callback resolving a dotted ref path to a `FormulaValue`.
 * @returns The rewritten notation string on success, or a `FormulaError` —
 * `"cap"` (template exceeds `MAX_FORMULA_LENGTH`), `"parse"` (unterminated
 * `[` label), or any error `substituteIdentifier` returns for a referenced identifier.
 * @example
 * ```ts
 * import { resolveNotationTemplate } from "@shadowcat/formula";
 *
 * resolveNotationTemplate("1d20 + str", () => 3); // { notation: "1d20 + 3[str]" }
 * ```
 */
export function resolveNotationTemplate(
  src: string,
  resolve: (path: string[]) => FormulaValue,
): { notation: string } | FormulaError {
  if (src.length > MAX_FORMULA_LENGTH) {
    return { error: "cap", detail: `template exceeds ${MAX_FORMULA_LENGTH} characters` };
  }

  let out = "";
  let i = 0;
  // Tracks whether the immediately preceding emitted token was an integer literal,
  // so a bare 'd' keyword can be normalized to '1d' only when no count precedes it.
  let prevWasInt = false;

  while (i < src.length) {
    const ch = src[i];

    if (ch === "[") {
      const end = src.indexOf("]", i + 1);
      if (end === -1) {
        return { error: "parse", detail: `unterminated '[' label at position ${i}` };
      }
      out += src.slice(i, end + 1);
      i = end + 1;
      prevWasInt = false;
      continue;
    }

    if (isDigit(ch)) {
      let j = i;
      while (j < src.length && isDigit(src[j])) j++;
      out += src.slice(i, j);
      i = j;
      prevWasInt = true;
      continue;
    }

    if (isAlpha(ch)) {
      const prefix = readAlphaPrefix(src, i);
      const lower = prefix.toLowerCase();
      // An identifier whose name is exactly a single-letter keyword immediately followed
      // by a digit or another keyword-shaped run (e.g. "t1", "d2mod") cannot be resolved
      // as an identifier here: `readAlphaPrefix` stops at the first non-alpha char, so
      // the keyword letter alone matches NOTATION_KEYWORDS and the remainder re-lexes as
      // dice-notation atoms, not a continued identifier. M13b's tier-1 authoring
      // validation (reserved-key checking) must reject this compound shape too, not just
      // literal keyword collisions.
      if (NOTATION_KEYWORDS.includes(lower)) {
        if (lower === "d" && !prevWasInt) {
          out += `1${prefix}`;
        } else {
          out += prefix;
        }
        i += prefix.length;
        prevWasInt = false;
        continue;
      }

      // Identifier span: word segments joined by '.', same word rule per segment.
      let j = i;
      while (j < src.length && isWordChar(src[j])) j++;
      while (j < src.length && src[j] === "." && j + 1 < src.length && isAlpha(src[j + 1])) {
        let k = j + 1;
        while (k < src.length && isWordChar(src[k])) k++;
        j = k;
      }
      const word = src.slice(i, j);
      const result = substituteIdentifier(word, resolve);
      if (typeof result !== "string") return result;
      out += result;
      i = j;
      prevWasInt = false;
      continue;
    }

    out += ch;
    i++;
    prevWasInt = false;
  }

  return { notation: out };
}
