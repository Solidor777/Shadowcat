import { type FormulaError, MAX_FORMULA_LENGTH } from "./types";

/** Operator token payloads. `.` is a bare operator — the parser assembles
 * dotted reference paths (e.g. `parent.dex`) from adjacent word/op tokens. */
export type Op = "+" | "-" | "*" | "/" | "%" | "(" | ")" | "," | ".";

export type Tok =
  | { kind: "num"; value: number; pos: number }
  | { kind: "word"; value: string; pos: number }
  | { kind: "op"; value: Op; pos: number };

const OPS = new Set<string>(["+", "-", "*", "/", "%", "(", ")", ",", "."]);

function isDigit(ch: string): boolean {
  return ch >= "0" && ch <= "9";
}

function isWordStart(ch: string): boolean {
  return (ch >= "a" && ch <= "z") || (ch >= "A" && ch <= "Z") || ch === "_";
}

function isWordChar(ch: string): boolean {
  return isWordStart(ch) || isDigit(ch);
}

/** Single left-to-right scan into tokens. Identifiers are lowercased on read
 * (spec §3.1 case-insensitive identifiers). Never throws — unrecognized
 * input returns a `FormulaError` value instead. */
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
