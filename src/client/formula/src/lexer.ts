import { type FormulaError, MAX_FORMULA_LENGTH } from "./types";
import { isDigit, isWordChar, isWordStart } from "./chars";

/** Operator token payloads. `.` is a bare operator — the parser assembles
 * dotted reference paths (e.g. `parent.dex`) from adjacent word/op tokens. */
export type Op = "+" | "-" | "*" | "/" | "%" | "(" | ")" | "," | ".";

/** A single lexed token, tagged by `kind`. */
export type Tok =
  | {
      /** Discriminant: a numeric literal. */
      kind: "num";
      /** The literal's parsed value — already overflow/finite-checked by `tokenize`. */
      value: number;
      /** UTF-16 code-unit offset of the token's first character, used in error messages. */
      pos: number;
    }
  | {
      /** Discriminant: an identifier or dotted-reference-path segment. */
      kind: "word";
      /** Lowercased text — identifiers are case-insensitive; casing is normalized here, at the lexer. */
      value: string;
      /** UTF-16 code-unit offset of the token's first character. */
      pos: number;
    }
  | {
      /** Discriminant: a single-character operator/punctuator from `Op`. */
      kind: "op";
      /** The matched operator. */
      value: Op;
      /** UTF-16 code-unit offset of the token's first character. */
      pos: number;
    };

const OPS = new Set<string>(["+", "-", "*", "/", "%", "(", ")", ",", "."]);

/** Single left-to-right scan into tokens. Identifiers are lowercased on read
 * (identifiers are case-insensitive). Never throws — unrecognized
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
