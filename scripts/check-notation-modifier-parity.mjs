// The dice-notation modifier vocabulary is ONE decision with THREE declarations: the server's
// notation parser matches it in `P::modifiers`, `@shadowcat/formula` reserves it in
// `NOTATION_KEYWORDS`, and the server's formula twin declares its own `NOTATION_KEYWORDS` —
// both template sides reserve the set so a template rewrite does not hand a modifier to a
// consumer's stat resolver. No declaration can read another, so the three are a forked
// decision; this extracts each and diffs every pair, which is what turns a drift into a
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

const TEMPLATE_LIST_HEAD_RE = /NOTATION_KEYWORDS\s*:\s*\[&str;\s*\d+\]\s*=\s*\[/;
const DICE_OPERATOR_DECL_RE = /const DICE_OPERATOR: &str = "([a-z]+)";/;

// The server formula twin's reserved-keyword list, in declaration order, extracted from its
// `NOTATION_KEYWORDS` const. The list's first member is the `DICE_OPERATOR` constant rather
// than a literal, so the constant's own declaration is read and substituted — a rename of
// either anchor throws rather than silently reporting parity over an empty or shifted set.
export function extractRustTemplateKeywords(rustTemplateSource) {
  const opDecl = rustTemplateSource.match(DICE_OPERATOR_DECL_RE);
  if (!opDecl) {
    throw new Error("`const DICE_OPERATOR` declaration not found in the template twin source");
  }
  const head = rustTemplateSource.match(TEMPLATE_LIST_HEAD_RE);
  if (!head) {
    throw new Error("`NOTATION_KEYWORDS` declaration not found in the template twin source");
  }
  const start = head.index + head[0].length;
  const end = rustTemplateSource.indexOf("];", start);
  if (end === -1) throw new Error("`NOTATION_KEYWORDS` declaration is unterminated");
  const body = rustTemplateSource.slice(start, end);
  return body
    .split(",")
    .map((entry) => entry.trim())
    .filter((token) => token !== "")
    .map((token) => {
      if (token === "DICE_OPERATOR") return opDecl[1];
      const literal = token.match(/^"([a-z]+)"$/);
      if (!literal) {
        throw new Error(`unrecognized entry '${token}' in the template twin's NOTATION_KEYWORDS`);
      }
      return literal[1];
    });
}

const FUNCTIONS_LIST_HEAD_RE = /NOTATION_FUNCTIONS\s*:\s*\[&str;\s*\d+\]\s*=\s*\[/;

// The server formula twin's math-function list, in declaration order. Every entry is a plain
// literal (unlike NOTATION_KEYWORDS' leading constant), so extraction is the simpler anchored
// scan; a rename of the anchor or a non-literal entry throws rather than matching nothing.
export function extractRustTemplateFunctions(rustTemplateSource) {
  const head = rustTemplateSource.match(FUNCTIONS_LIST_HEAD_RE);
  if (!head) {
    throw new Error("`NOTATION_FUNCTIONS` declaration not found in the template twin source");
  }
  const start = head.index + head[0].length;
  const end = rustTemplateSource.indexOf("];", start);
  if (end === -1) throw new Error("`NOTATION_FUNCTIONS` declaration is unterminated");
  return rustTemplateSource
    .slice(start, end)
    .split(",")
    .map((entry) => entry.trim())
    .filter((token) => token !== "")
    .map((token) => {
      const literal = token.match(/^"([a-z]+)"$/);
      if (!literal) {
        throw new Error(`unrecognized entry '${token}' in the template twin's NOTATION_FUNCTIONS`);
      }
      return literal[1];
    });
}

const FN_CALL_HEAD = "match name.as_str() {";
const FN_CATCH_ALL_MARKER = "unknown function";

// The math-function names the server's notation parser accepts, in declaration order, taken
// from the arm heads of `fn_call`'s match block — the runtime set a template scan must
// reserve (followed by `(`), or every function-calling roll breaks at resolution.
export function extractRustNotationFunctions(parserSource) {
  const fnAt = parserSource.indexOf("fn fn_call(");
  if (fnAt === -1) throw new Error("`fn fn_call` not found in the notation parser source");
  const matchAt = parserSource.indexOf(FN_CALL_HEAD, fnAt);
  if (matchAt === -1) throw new Error("`match name.as_str()` not found inside `fn fn_call`");
  let depth = 0;
  for (let i = matchAt + FN_CALL_HEAD.length - 1; i < parserSource.length; i++) {
    if (parserSource[i] === "{") depth++;
    else if (parserSource[i] === "}") {
      depth--;
      if (depth === 0) {
        const block = parserSource.slice(matchAt, i + 1);
        if (!block.includes(FN_CATCH_ALL_MARKER)) {
          throw new Error("the extracted fn_call match block is missing its catch-all arm");
        }
        return [...block.matchAll(/"([a-z]+)"[ \t]*=>/g)].map((m) => m[1]);
      }
    }
  }
  throw new Error("`match name.as_str()` block is unterminated");
}
