import { type FormulaError, MAX_FORMULA_LENGTH } from "./types";

/** Operator token payloads. `.` is a bare operator — the parser assembles
 * dotted reference paths (e.g. `parent.dex`) from adjacent word/op tokens. */
export type Op = "+" | "-" | "*" | "/" | "%" | "(" | ")" | "," | ".";

export type Tok =
  | { kind: "num"; value: number; pos: number }
  | { kind: "word"; value: string; pos: number }
  | { kind: "op"; value: Op; pos: number };

const OPS = new Set<string>(["+", "-", "*", "/", "%", "(", ")", ",", "."]);

/**
 * True for an ASCII decimal digit — used wherever one must be recognized:
 * starting a numeric literal, continuing one, the post-`.` lookahead, and
 * (via `isWordChar`) mid-identifier. IMPLICIT COUPLING: changing this set
 * changes IDENTIFIER lexing too, not just numeric-literal entry.
 * A leading `.` is NOT in this set, so a formula like `.5` is not recognized
 * as a number (it lexes `.` as a bare operator token instead; write `0.5`).
 * @param ch A single character.
 * @returns `true` when `ch` is `'0'`–`'9'`.
 * @example
 * ```
 * isDigit("7"); // true
 * isDigit("."); // false
 * ```
 */
function isDigit(ch: string): boolean {
  return ch >= "0" && ch <= "9";
}

/**
 * True for a character that may START an identifier: ASCII letter or `_`.
 * @param ch A single character.
 * @returns `true` when `ch` may begin a `word` token.
 * @example
 * ```
 * isWordStart("_"); // true
 * isWordStart("1"); // false
 * ```
 */
function isWordStart(ch: string): boolean {
  return (ch >= "a" && ch <= "z") || (ch >= "A" && ch <= "Z") || ch === "_";
}

/**
 * True for a character that may CONTINUE an identifier already begun by
 * `isWordStart` — letters, digits, and `_` (digits are legal mid-identifier,
 * just not as the first character).
 * @param ch A single character.
 * @returns `true` when `ch` may continue a `word` token.
 * @example
 * ```
 * isWordChar("3"); // true — legal after the first character
 * isWordChar("."); // false — dots separate ref segments, not part of a word
 * ```
 */
function isWordChar(ch: string): boolean {
  return isWordStart(ch) || isDigit(ch);
}

/** Single left-to-right scan into tokens. Identifiers are lowercased on read
 * (spec §3.1 case-insensitive identifiers). Never throws — unrecognized
 * input returns a `FormulaError` value instead.
 * @param src Formula source text.
 * @returns The token stream on success, or a `FormulaError` — `"cap"` for a
 * source longer than `MAX_FORMULA_LENGTH` or an overflowing numeric literal,
 * `"parse"` for a malformed number (e.g. two `.` in one literal, or a
 * trailing `.` with no following digit) or an unrecognized character.
 * @example
 * ```
 * // not part of the public `@shadowcat/formula` surface (this module is not
 * // re-exported) — internal to the `parser` module.
 * tokenize("1 + hp.max"); // [{kind:"num",...}, {kind:"op",value:"+",...}, {kind:"word",value:"hp",...}, {kind:"op",value:".",...}, {kind:"word",value:"max",...}]
 * ```
 */
export function tokenize(src: string): Tok[] | FormulaError {
  if (src.length > MAX_FORMULA_LENGTH) {
    return { error: "cap", detail: "formula exceeds 512 characters" };
  }

  const toks: Tok[] = [];
  let i = 0;
  const n = src.length;

  while (i < n) {
    const ch = src[i];

    if (ch === " " || ch === "\t" || ch === "\n" || ch === "\r") {
      i++;
      continue;
    }

    if (isDigit(ch)) {
      const start = i;
      let sawDot = false;
      i++;
      while (i < n && (isDigit(src[i]) || src[i] === ".")) {
        if (src[i] === ".") {
          if (sawDot) {
            return { error: "parse", detail: `unexpected '.' at position ${i}` };
          }
          // Grammar: digits, optional '.' + digits — a dot not followed by a
          // digit is not part of this numeric literal.
          if (!(i + 1 < n && isDigit(src[i + 1]))) {
            return { error: "parse", detail: `unexpected '.' at position ${i}` };
          }
          sawDot = true;
        }
        i++;
      }
      const text = src.slice(start, i);
      const value = Number(text);
      // INVARIANT: library never emits Infinity/NaN — an over-long digit run
      // that overflows f64 is a DoS-shaped cap violation, not a valid literal.
      if (!Number.isFinite(value)) {
        return { error: "cap", detail: `numeric literal at position ${start} is out of range` };
      }
      toks.push({ kind: "num", value, pos: start });
      continue;
    }

    if (isWordStart(ch)) {
      const start = i;
      i++;
      while (i < n && isWordChar(src[i])) i++;
      toks.push({ kind: "word", value: src.slice(start, i).toLowerCase(), pos: start });
      continue;
    }

    if (OPS.has(ch)) {
      toks.push({ kind: "op", value: ch as Op, pos: i });
      i++;
      continue;
    }

    // Position/scanning stays UTF-16-code-unit-based; only the message uses
    // the full code point so an astral character (e.g. an emoji) is not
    // truncated to a lone, unpaired surrogate half.
    const displayCh = String.fromCodePoint(src.codePointAt(i)!);
    return { error: "parse", detail: `unexpected '${displayCh}' at position ${i}` };
  }

  return toks;
}
