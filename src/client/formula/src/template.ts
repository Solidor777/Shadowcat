import { MAX_FORMULA_LENGTH, type FormulaError, type FormulaValue, isFormulaError } from "./types";
import { validateResolverOutput } from "./internal";

/** Identifier words whose leading-alpha prefix means dice notation, not a stat.
 * Mirrors src/server/src/dice/notation/parser.rs keyword match (kh/kl/dh/dl/r/ro/cs/cf/t/e)
 * plus the 'd' dice operator. M13b's authoring validation imports this list. */
export const NOTATION_KEYWORDS: readonly string[] =
  ["d", "kh", "kl", "dh", "dl", "r", "ro", "cs", "cf", "t", "e"];

const I32_MAX = 2147483647;

function isAlpha(ch: string): boolean {
  return /[a-z_]/i.test(ch);
}

function isWordChar(ch: string): boolean {
  return /[a-z0-9_]/i.test(ch);
}

function isDigit(ch: string): boolean {
  return ch >= "0" && ch <= "9";
}

/** Reads the maximal `[a-z_]+` prefix (case-insensitive) starting at `i`. */
function readAlphaPrefix(src: string, i: number): string {
  let j = i;
  while (j < src.length && isAlpha(src[j])) j++;
  return src.slice(i, j);
}

/** Resolves a `.`-joined identifier path (e.g. "hp.max") to a labeled substitution.
 * INVARIANT: never throws — resolver faults propagate as ref-error/type/cap FormulaErrors. */
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
  if (value < 0) return `(0 - ${-value})`;
  return `${value}[${originalText}]`;
}

/** Rewrites a dice-notation template: identifiers resolve to labeled constants, existing
 * dice-notation atoms (and `[label]` spans) pass through untouched. Spec §3 template mode.
 * INVARIANT: never throws; every failure path returns a FormulaError. */
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
