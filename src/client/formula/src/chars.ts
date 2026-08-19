// Not part of the package's public surface — `@shadowcat/formula`'s public exports omit
// this module — the character classes BOTH grammars in this package accept: the `lexer`
// module's formula tokenizer and the `template` module's notation rewriter.
// ONE declaration, deliberately: the two grammars accept the same identifier characters,
// so a per-module copy is a forked decision that silently disagrees the first time either
// set is widened, and a bare citation of a duplicated predicate resolves ambiguously.
// The grammars still differ ABOVE this layer — what may follow an identifier, which words
// are reserved, and whether a malformed reference is a loud parse error or a silent split.
// Nothing here encodes any of that, so sharing these predicates hides no difference.
// A `//` header, not a `/** */` block: a doc block here would attach to the first
// declaration below rather than to the module.

/**
 * True for an ASCII decimal digit — every place one must be recognized:
 * starting a numeric literal, continuing one, `tokenize`'s post-`.` lookahead,
 * a notation count/sides run, and (via `isWordChar`) mid-identifier.
 * IMPLICIT COUPLING: changing this set changes IDENTIFIER lexing too, not just
 * numeric-literal entry. A leading `.` is NOT in this set, so a formula like
 * `.5` is not recognized as a number (`tokenize` lexes `.` as a bare operator
 * token instead; write `0.5`).
 * @param ch A single character.
 * @returns `true` when `ch` is `'0'`–`'9'`.
 * @example
 * ```
 * // not part of the public `@shadowcat/formula` surface (this module is not
 * // re-exported).
 * isDigit("7"); // true
 * isDigit("."); // false
 * ```
 */
export function isDigit(ch: string): boolean {
  return ch >= "0" && ch <= "9";
}

/**
 * True for a character that may START an identifier: an ASCII letter or `_`.
 * In the notation grammar this doubles as the run boundary the keyword probe
 * reads — `readKeywordRun` advances while this holds — so a digit and a dot are
 * both outside it.
 * @param ch A single character.
 * @returns `true` when `ch` may begin an identifier.
 * @example
 * ```
 * // not part of the public `@shadowcat/formula` surface (this module is not
 * // re-exported).
 * isWordStart("_"); // true
 * isWordStart("1"); // false
 * ```
 */
export function isWordStart(ch: string): boolean {
  return (ch >= "a" && ch <= "z") || (ch >= "A" && ch <= "Z") || ch === "_";
}

/**
 * True for a character that may CONTINUE an identifier already begun by
 * `isWordStart` — letters, digits, and `_` (digits are legal mid-identifier,
 * just not as the first character).
 * @param ch A single character.
 * @returns `true` when `ch` may continue an identifier.
 * @example
 * ```
 * // not part of the public `@shadowcat/formula` surface (this module is not
 * // re-exported).
 * isWordChar("3"); // true — legal after the first character
 * isWordChar("."); // false — dots separate ref segments, not part of a word
 * ```
 */
export function isWordChar(ch: string): boolean {
  return isWordStart(ch) || isDigit(ch);
}
