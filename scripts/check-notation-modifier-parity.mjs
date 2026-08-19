// The dice-notation modifier vocabulary is declared twice, in two languages: the server's
// notation parser matches it in `P::modifiers`, and `@shadowcat/formula` reserves it in
// `NOTATION_KEYWORDS` so a template rewrite does not hand a modifier to a consumer's stat
// resolver. Neither language can read the other's declaration, so the two are a forked
// decision; this reads both and reports the difference, which is what turns a drift into a
// build failure instead of a wrong roll an author has to notice.
//
// Extraction is anchored on named items rather than on line positions: a rename of either
// anchor fails loudly here rather than silently matching nothing.

const MATCH_HEAD = "match id.as_str() {";
// The catch-all arm's message. Finding it inside the extracted block proves the block runs
// to the end of the match rather than stopping at an unbalanced brace inside a literal.
const CATCH_ALL_MARKER = "unknown dice modifier";
const ARM_HEAD_RE = /(?:^|\n)[ \t]*((?:"[a-z]+"[ \t]*\|[ \t]*)*"[a-z]+")[ \t]*=>/g;

// Returns the source of `P::modifiers`'s ident match block, from its `match` head to the
// brace that closes it. Throws when either anchor is missing or the block is unterminated —
// a scan that finds nothing must fail rather than report parity over an empty set.
export function extractModifierMatchBlock(rustSource) {
  const fnAt = rustSource.indexOf("fn modifiers(");
  if (fnAt === -1) throw new Error("`fn modifiers` not found in the notation parser source");
  const matchAt = rustSource.indexOf(MATCH_HEAD, fnAt);
  if (matchAt === -1) throw new Error("`match id.as_str()` not found inside `fn modifiers`");
  let depth = 0;
  for (let i = matchAt + MATCH_HEAD.length - 1; i < rustSource.length; i++) {
    if (rustSource[i] === "{") depth++;
    else if (rustSource[i] === "}") {
      depth--;
      if (depth === 0) {
        const block = rustSource.slice(matchAt, i + 1);
        if (!block.includes(CATCH_ALL_MARKER)) {
          throw new Error("the extracted match block is missing its catch-all arm");
        }
        return block;
      }
    }
  }
  throw new Error("`match id.as_str()` block is unterminated");
}

// Every modifier word the server's parser accepts, in declaration order, taken from the arm
// heads of that match block (an alternation arm contributes each of its literals).
export function extractRustModifierIdents(rustSource) {
  const block = extractModifierMatchBlock(rustSource);
  const idents = [];
  for (const match of block.matchAll(ARM_HEAD_RE)) {
    for (const literal of match[1].split("|")) idents.push(literal.trim().slice(1, -1));
  }
  return idents;
}

// The two sides' disagreement, as two lists. `diceOperator` is a token of the notation
// grammar rather than a modifier, so the template side reserves it with no counterpart in
// the modifier match.
export function modifierParityDifference(rustIdents, reservedKeywords, diceOperator) {
  const rust = new Set(rustIdents);
  const reserved = new Set(reservedKeywords.filter((word) => word !== diceOperator));
  return {
    missingFromTemplate: [...rust].filter((word) => !reserved.has(word)),
    unknownToServer: [...reserved].filter((word) => !rust.has(word)),
  };
}
