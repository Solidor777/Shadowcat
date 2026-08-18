import { MAX_FORMULA_LENGTH, type FormulaError, type FormulaValue, isFormulaError } from "./types";
import { validateResolverOutput } from "./internal";
import { isDigit, isWordChar, isWordStart } from "./chars";

/** The dice operator: both a `NOTATION_KEYWORDS` member and the one keyword whose
 * emission is rewritten — with no integer immediately before it a count of `1` is
 * synthesized, because the server's notation parser requires a count. Declared once and
 * used both as the list's first member and as `emitClaim`'s test, so the two cannot
 * disagree about which keyword that is. */
const DICE_OPERATOR = "d";

/** Identifier words that mean dice notation rather than a stat. Mirrors `P::modifiers`'s
 * keyword match (kh/kl/dh/dl/r/ro/cs/cf/t/e) plus the dice operator.
 *
 * **This list is not the set of unsafe stat keys, and no list is.** The notation grammar
 * reserves more than these words, and what it reserves is negative space over an ordered
 * chain of recognizers (`RECOGNIZERS`): a key survives exactly when one
 * `claimIdentifierSpan` claim covers all of it. That set has no closed-form description;
 * the chain is its definition. So a consuming system's stat-key authoring validation calls
 * `checkNotationKey`, which runs the chain, and must not reimplement a rule over this list
 * instead. Every collision is silent on every path: the colliding text is rewritten into
 * notation and the consumer's identifier resolver is never asked about it. */
export const NOTATION_KEYWORDS: readonly string[] =
  [DICE_OPERATOR, "kh", "kl", "dh", "dl", "r", "ro", "cs", "cf", "t", "e"];

const I32_MAX = 2147483647;

/** Which recognizer claimed a span of notation-template source. The grammar is an ordered
 * chain (`RECOGNIZERS`) plus a total fallthrough, and this names the outcome of that
 * ordering — the whole of what `checkNotationKey` reports, since a key survives only when
 * one `"identifier"` claim covers it.
 *
 * - `"label"` — a bracketed span, emitted verbatim.
 * - `"integer"` — a run of digits, emitted verbatim.
 * - `"keyword"` — an identifier-start run that is a `NOTATION_KEYWORDS` member when
 *   lowercased, emitted as notation.
 * - `"identifier"` — a dotted reference span, handed to the consumer's resolver and
 *   replaced by the resolved value.
 * - `"literal"` — one character no recognizer claimed, emitted verbatim. */
export type NotationClaimKind = "label" | "integer" | "keyword" | "identifier" | "literal";

/** One recognizer's claim over a span of source, before any consumer resolution.
 * Recognition carries NO state: what claims a position depends only on the source and the
 * position. The scan's carry is consumed by `emitClaim` alone. */
interface Claim {
  /** Which recognizer claimed the span. */
  readonly kind: NotationClaimKind;
  /** The exact source slice claimed — its length is what the scan advances by. */
  readonly text: string;
}

/** A named member of the ordered recognizer chain. */
interface Recognizer {
  /** The `NotationClaimKind` this recognizer produces on a hit. */
  readonly kind: NotationClaimKind;
  /** Attempts a claim at `at`, as a pure function of the source and the position:
   * returns the claimed source slice (never empty), `null` to decline and let the next
   * recognizer try, or a `FormulaError` rejecting the whole template. */
  readonly claim: (src: string, at: number) => string | FormulaError | null;
}

/** Reads the maximal run of identifier-START characters at `i` — the run
 * `claimNotationKeyword` tests for membership. Stops at the first character outside
 * `isWordStart`, which includes both a digit and a dot.
 * @param src The template source text.
 * @param i Index to start scanning from.
 * @returns The run, empty when `src[i]` is not an identifier-start character.
 * @example
 * ```
 * // not part of the public `@shadowcat/formula` surface — this helper is not exported.
 * readKeywordRun("kh3", 0); // "kh"
 * ```
 */
function readKeywordRun(src: string, i: number): string {
  let j = i;
  while (j < src.length && isWordStart(src[j])) j++;
  return src.slice(i, j);
}

/** Recognizer 1: a bracketed label span, from `[` through the next `]`, emitted verbatim
 * so an author-written label survives the rewrite. An unterminated bracket rejects the
 * whole template. Ordered FIRST, so a label's contents are never scanned for keywords or
 * identifiers. */
const claimLabelSpan: Recognizer = {
  kind: "label",
  claim: (src, at) => {
    if (src[at] !== "[") return null;
    const end = src.indexOf("]", at + 1);
    if (end === -1) return { error: "parse", detail: `unterminated '[' label at position ${at}` };
    return src.slice(at, end + 1);
  },
};

/** Recognizer 2: a maximal run of digits, emitted verbatim as a notation count/sides
 * literal. Ordered BEFORE the identifier span, which is what costs a key whose first
 * character is a digit that digit: the run is emitted into the notation stream and the
 * remainder is claimed separately. */
const claimIntegerRun: Recognizer = {
  kind: "integer",
  claim: (src, at) => {
    if (!isDigit(src[at])) return null;
    let j = at;
    while (j < src.length && isDigit(src[j])) j++;
    return src.slice(at, j);
  },
};

/** Recognizer 3: an identifier-start run that is a `NOTATION_KEYWORDS` member when
 * lowercased. Only the run is tested, and `readKeywordRun` stops at the first character
 * outside `isWordStart`, so whatever follows the run is claimed by later iterations on its
 * own terms. Ordered BEFORE the identifier span, which is what makes the reserved set
 * wider than the list. */
const claimNotationKeyword: Recognizer = {
  kind: "keyword",
  claim: (src, at) => {
    if (!isWordStart(src[at])) return null;
    const run = readKeywordRun(src, at);
    return NOTATION_KEYWORDS.includes(run.toLowerCase()) ? run : null;
  },
};

/** Recognizer 4: a dotted reference span — `isWordChar` characters, then any number of
 * `.`-joined segments, each of which must itself begin with an identifier-start
 * character. A dot NOT followed by such a character ends the span, splitting what the
 * author wrote into separate references. The only recognizer whose emission reaches the
 * consumer's resolver. */
const claimIdentifierSpan: Recognizer = {
  kind: "identifier",
  claim: (src, at) => {
    if (!isWordStart(src[at])) return null;
    let j = at;
    while (j < src.length && isWordChar(src[j])) j++;
    while (j < src.length && src[j] === "." && j + 1 < src.length && isWordStart(src[j + 1])) {
      let k = j + 1;
      while (k < src.length && isWordChar(src[k])) k++;
      j = k;
    }
    return src.slice(at, j);
  },
};

/** The template grammar's recognizer chain, tried in this order at every position. The
 * ORDER is the grammar: each recognizer claims only what no earlier one took, so "does
 * this key survive" means "does `claimIdentifierSpan` get all of it". Ordering is data
 * here rather than nested control flow precisely so that question is answerable by
 * RUNNING the chain (`checkNotationKey`) instead of by describing it. */
const RECOGNIZERS: readonly Recognizer[] = [
  claimLabelSpan,
  claimIntegerRun,
  claimNotationKeyword,
  claimIdentifierSpan,
];

/** Runs the recognizer chain at one position. Total: when every recognizer declines, one
 * character passes through as a `"literal"` claim, so the scan always advances.
 * @param src The template source text.
 * @param at Index to claim at; must be within `src`.
 * @returns The winning `Claim`, or a `FormulaError` rejecting the whole template.
 * @example
 * ```
 * // not part of the public `@shadowcat/formula` surface — this helper is not exported.
 * claimAt("kh3", 0); // { kind: "keyword", text: "kh" }
 * ```
 */
function claimAt(src: string, at: number): Claim | FormulaError {
  for (const recognizer of RECOGNIZERS) {
    const text = recognizer.claim(src, at);
    if (text === null) continue;
    if (typeof text !== "string") return text;
    return { kind: recognizer.kind, text };
  }
  return { kind: "literal", text: src[at] };
}

/** Turns a `Claim` into the text it contributes to the rewritten notation. The ONLY stage
 * that reads the scan's carried state or calls the consumer's resolver; recognition does
 * neither.
 * @param claim The winning claim at the current position.
 * @param prevWasInt Whether the immediately preceding claim was an integer run — read only
 * by the dice-operator normalization, and passed explicitly rather than reached for.
 * @param resolve Consumer callback resolving a dotted ref path to a `FormulaValue`.
 * @returns The emitted text, or a `FormulaError` from identifier resolution.
 * @example
 * ```
 * // not part of the public `@shadowcat/formula` surface — this helper is not exported.
 * emitClaim({ kind: "keyword", text: "d" }, false, () => 0); // "1d"
 * ```
 */
function emitClaim(
  claim: Claim,
  prevWasInt: boolean,
  resolve: (path: string[]) => FormulaValue,
): string | FormulaError {
  if (claim.kind === "identifier") return substituteIdentifier(claim.text, resolve);
  if (claim.kind === "keyword" && claim.text.toLowerCase() === DICE_OPERATOR && !prevWasInt) {
    return `1${claim.text}`;
  }
  return claim.text;
}

/** Resolves a `.`-joined identifier path (e.g. "hp.max") to a labeled substitution.
 * INVARIANT: never throws — resolver faults propagate as ref-error/type/cap FormulaErrors.
 * @param originalText The dotted identifier as it appeared in the template (e.g. `"hp.max"`).
 * @param resolve Consumer callback resolving the dotted path to a `FormulaValue`. May throw;
 * a thrown value is caught and converted to `"resolver-error"` rather than propagating.
 * @returns The labeled substitution text on success (see the negative-value note below),
 * or a `FormulaError`. Raised here: `"resolver-error"` (the resolver threw, or returned a
 * value that is neither a number nor a well-formed error), `"type"` (a non-integer resolved
 * value — roll templates require integers), and `"cap"` (magnitude exceeds `i32::MAX`). NOT
 * an exhaustive list of what can come back, by two separate routes: a well-formed `FormulaError`
 * returned BY the resolver passes through verbatim (so any `FormulaErrorKind` the consumer
 * produces — `"unknown-ref"`, `"cycle"`, … — can surface here unchanged), and a resolver
 * returning a non-finite NUMBER (type-legal: `FormulaValue = number | FormulaError`) is converted
 * to `"non-finite"` by `validateResolverOutput`'s `finite()` check rather than passed through.
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
  // guaranteed to honor the `FormulaValue` contract (same boundary `evaluate`'s `ref`
  // case and the `evalNode` callback already cross via this shared helper).
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
  // an i32. This asymmetry is intentional — do not "fix" it into a symmetric range check.
  if (Math.abs(value) > I32_MAX) {
    return { error: "cap", detail: `'${originalText}' = ${value}: out of i32 range` };
  }
  // A negative value is emitted as an unlabeled parenthesized subtraction; a
  // positive one as a labeled constant (below). There is NO arithmetic reason
  // for the asymmetry: `(0 - N)` and `-N` denote the same number, and the
  // server's recursive-descent grammar evaluates either to the same total in
  // every preceding context — `x - (0 - N)` and `x - Neg(N)` both fold to
  // `x + N`.
  //
  // It does have one observable consequence, in the roll breakdown rather than
  // the total: `collect_labeled_consts` emits
  // a ConstTerm only for a `Const` carrying a label, and it RECURSES through
  // `Expr::Neg` — so a labeled `-N[label]` would still contribute a signed chip,
  // while this form's two unlabeled `Const`s contribute none. A negative
  // substitution therefore shows no `[label]` chip in the breakdown.
  // TODO: decide whether that is intended; if the chip is wanted, emit `-N[originalText]`
  // instead (arithmetically identical per the fold above).
  if (value < 0) return `(0 - ${-value})`;
  return `${value}[${originalText}]`;
}

/** One span of a written key, as the template grammar claims it. */
export interface NotationKeySegment {
  /** Which recognizer claimed the span. */
  readonly kind: NotationClaimKind;
  /** The claimed text. */
  readonly text: string;
  /** Index within the key where the span starts. */
  readonly at: number;
}

/** What the template grammar does to one written key — `checkNotationKey`'s result. */
export interface NotationKeyCheck {
  /** `true` only when the whole key is claimed as ONE identifier span, which is the only
   * shape that reaches a consumer's identifier resolver as the author wrote it. */
  readonly intact: boolean;
  /** Every claim over the key, in order, stopping at `rejects` when one is set. Two or
   * more segments means the key is SPLIT: each `"identifier"` segment resolves separately
   * and every other segment is emitted into the notation, with no error on any path. */
  readonly segments: readonly NotationKeySegment[];
  /** Set when a recognizer rejected the key outright, in which case any template
   * containing it returns this error instead of notation; `null` otherwise. */
  readonly rejects: FormulaError | null;
}

/** Answers whether a written key survives the notation-template grammar intact, by
 * RUNNING that grammar's recognizer chain over it — `claimAt`, the same chain
 * `resolveNotationTemplate` runs at every position, so the two cannot disagree about what
 * claims a key.
 *
 * This is the authority a consuming system's stat-key authoring validation calls.
 * `NOTATION_KEYWORDS` membership is one of several ways a key fails and there is no closed
 * form for the rest, because surviving means being claimed by a recognizer no earlier
 * recognizer beat to the position. Reimplementing a rule instead of calling this is the
 * failure this function exists to end.
 *
 * Scope: the grammar only. `MAX_FORMULA_LENGTH` bounds a whole template rather than a key,
 * so it is not applied here, and no resolution is attempted — no resolver is accepted.
 * @param key The stat key as an author would write it.
 * @returns An `intact` verdict plus the claim structure behind it: which recognizer took
 * each span, and where. A bare boolean cannot tell an authoring UI what went wrong, and
 * returning the recognizers themselves would pin this module's internals into the public
 * API — segments are the smallest shape that explains the verdict without doing either.
 * @example
 * ```ts
 * import { checkNotationKey } from "@shadowcat/formula";
 *
 * checkNotationKey("hp.max").intact; // true
 * checkNotationKey("kh.max").intact; // false — "kh" is claimed as dice notation
 * checkNotationKey("2hp").segments;  // [{ kind: "integer", ... }, { kind: "identifier", ... }]
 * ```
 */
export function checkNotationKey(key: string): NotationKeyCheck {
  const segments: NotationKeySegment[] = [];
  let at = 0;
  while (at < key.length) {
    const claim = claimAt(key, at);
    if (!("kind" in claim)) return { intact: false, segments, rejects: claim };
    segments.push({ kind: claim.kind, text: claim.text, at });
    at += claim.text.length;
  }
  const intact = segments.length === 1 && segments[0].kind === "identifier";
  return { intact, segments, rejects: null };
}

/** Rewrites a dice-notation template: identifiers resolve to labeled constants, existing
 * dice-notation atoms (and label spans) pass through untouched.
 * INVARIANT: never throws; every failure path returns a FormulaError.
 *
 * Scanning is `RECOGNIZERS` tried in order at each position (`claimAt`), then `emitClaim`
 * on the winner. Which recognizer claims a position is what decides whether an author's
 * stat key survives as a reference at all, and a key that loses is rewritten into notation
 * with no error on any path — so a consuming system validates its keys with
 * `checkNotationKey`, which runs this same chain.
 * @param src Template text, e.g. `"1d20 + str"` — a mix of dice-notation atoms
 * (numbers, the dice operator, `NOTATION_KEYWORDS` modifiers, bracketed label spans)
 * and dotted identifier references.
 * @param resolve Consumer callback resolving a dotted ref path to a `FormulaValue`.
 * @returns The rewritten notation string on success, or a `FormulaError` —
 * `"cap"` (template exceeds `MAX_FORMULA_LENGTH`), `"parse"` (unterminated
 * label bracket), or any error `substituteIdentifier` returns for a referenced identifier.
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
): {
  /** The rewritten notation string, ready to post to `chat::rolls`. */
  notation: string;
} | FormulaError {
  if (src.length > MAX_FORMULA_LENGTH) {
    return { error: "cap", detail: `template exceeds ${MAX_FORMULA_LENGTH} characters` };
  }

  let out = "";
  let i = 0;
  // The scan's only carried state: whether the immediately preceding claim was an integer
  // run. Read by `emitClaim` alone, and passed to it explicitly.
  let prevWasInt = false;

  while (i < src.length) {
    const claim = claimAt(src, i);
    if (!("kind" in claim)) return claim;
    const emitted = emitClaim(claim, prevWasInt, resolve);
    if (typeof emitted !== "string") return emitted;
    out += emitted;
    i += claim.text.length;
    prevWasInt = claim.kind === "integer";
  }

  return { notation: out };
}
