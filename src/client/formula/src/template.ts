import { MAX_FORMULA_LENGTH, type FormulaError, type FormulaValue, isFormulaError } from "./types";

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
  let value: FormulaValue;
  try {
    value = resolve(path);
  } catch {
    return { error: "resolver-error", detail: `resolver threw for '${originalText}'` };
  }
  if (isFormulaError(value)) return value;
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return {
      error: "resolver-error",
      detail: `resolver returned a non-numeric, non-error value for '${originalText}'`,
    };
  }
  if (!Number.isInteger(value)) {
    return {
      error: "type",
      detail: `'${originalText}' = ${value}: roll templates require integers (use floor/round in the stat formula)`,
    };
  }
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
