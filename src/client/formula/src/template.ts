import { MAX_FORMULA_LENGTH, type FormulaError, type FormulaValue, isFormulaError } from "./types";
import { callResolver, validateResolverOutput } from "./internal";
import { isDigit, isWordChar, isWordStart } from "./chars";

/** The dice operator: both a `NOTATION_KEYWORDS` member and the one keyword whose
 * emission is rewritten — with no integer immediately before it a count of `1` is
 * synthesized, because the server's notation parser requires a count. Declared once and
 * used both as the list's first member and as `emitClaim`'s test, so the two cannot
 * disagree about which keyword that is. */
const DICE_OPERATOR = "d";

/** Identifier words that mean dice notation rather than a stat. Mirrors `P::modifiers`'s
 * keyword match, plus the dice operator; the array below is the only enumeration of the set
 * on this side of the language boundary. Neither language can read the other's declaration,
 * so `modifierParityDifference` reads both declarations and fails the script-test gate on
 * any difference.
 *
 * **This list is not the set of unsafe stat keys, and no list is.** The notation grammar
 * reserves more than these words, and what it reserves has no closed-form description — it is
 * negative space over `RECOGNIZERS`. `checkNotationKey` answers by running that chain and is
 * the only answer, so a consuming system's stat-key authoring validation calls it and must not
 * reimplement a rule over this list instead.
 *
 * **A collision is not reliably loud.** How a scan of a colliding key can end is enumerated
 * on `NotationKeyCheck` and nowhere else, so no second copy of that taxonomy can drift from
 * the type a consuming authoring UI actually branches on. */
export const NOTATION_KEYWORDS: readonly string[] =
  [DICE_OPERATOR, "kh", "kl", "dh", "dl", "r", "ro", "cs", "cf", "t", "e", "tr", "rs"];

const I32_MAX = 2147483647;

/** Which recognizer claimed a span of notation-template source — the vocabulary
 * `checkNotationKey` reports its claim structure in, a rejection aside.
 *
 * - `"label"` — a bracketed span.
 * - `"integer"` — a run of digits.
 * - `"keyword"` — an identifier-start run that is a `NOTATION_KEYWORDS` member when lowercased.
 * - `"identifier"` — a dotted reference span.
 * - `"literal"` — one character no recognizer claimed.
 *
 * What a consumer needs from the five is one distinction: `"identifier"` is the only kind whose
 * span reaches the consumer's resolver, and what each of the other four contributes to the
 * rewritten notation is `emitClaim`'s decision rather than a property of the kind.
 *
 * These five names are stable public vocabulary. An authoring UI branches on them to explain
 * a verdict, so they are committed to independently of how `RECOGNIZERS` is later split or
 * merged — a claim CATEGORY outlives the recognizer that currently produces it. */
export type NotationClaimKind = "label" | "integer" | "keyword" | "identifier" | "literal";

/** One recognizer's claim over a span of source, before any consumer resolution. */
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
   * recognizer try, or a `FormulaError` rejecting the whole SCAN it runs in — the template
   * under `resolveNotationTemplate`, the key alone under `checkNotationKey`. */
  readonly claim: (src: string, at: number) => string | FormulaError | null;
}

/** Reads the maximal run of identifier-START characters at `i` — the run
 * `claimNotationKeyword` tests for membership. The run's extent is `isWordStart`'s alone:
 * over `"kh_max"` it returns the whole six characters, over `"kh1"` only `"kh"` before the
 * digit stops it. What that extent means for whether the written key survives the grammar is
 * `NotationKeyCheck`'s to state, not this function's.
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

/** A bracketed label span, from `[` through the next `]`. An unterminated bracket rejects the
 * whole SCAN it is running in: the template under `resolveNotationTemplate`, the key alone
 * under `checkNotationKey`. A label's contents are never scanned for keywords or identifiers,
 * so an author-written label survives the rewrite whatever it spells; that follows from the
 * claim's EXTENT rather than from its place in the chain, because `[` is in no other
 * recognizer's start set and this recognizer's position is therefore unobservable. */
const claimLabelSpan: Recognizer = {
  kind: "label",
  claim: (src, at) => {
    if (src[at] !== "[") return null;
    const end = src.indexOf("]", at + 1);
    if (end === -1) return { error: "parse", detail: `unterminated '[' label at position ${at}` };
    return src.slice(at, end + 1);
  },
};

/** A maximal run of digits, starting at `at` while `isDigit` holds and declining (`null`)
 * otherwise. Its position in the chain is unobservable: a digit is in no other recognizer's
 * start set. Unlike `claimLabelSpan`, this recognizer has no rejecting branch — it only
 * declines or claims a slice. */
const claimIntegerRun: Recognizer = {
  kind: "integer",
  claim: (src, at) => {
    if (!isDigit(src[at])) return null;
    let j = at;
    while (j < src.length && isDigit(src[j])) j++;
    return src.slice(at, j);
  },
};

/** An identifier-start run that is a `NOTATION_KEYWORDS` member when lowercased —
 * `readKeywordRun` fixes the run's extent, and only the whole run is tested. Ordered BEFORE
 * `claimIdentifierSpan`, which is what makes the reserved set wider than the list, and the one
 * adjacency in the chain whose order is observable: the two share `isWordStart` as their start
 * set. */
const claimNotationKeyword: Recognizer = {
  kind: "keyword",
  claim: (src, at) => {
    if (!isWordStart(src[at])) return null;
    const run = readKeywordRun(src, at);
    return NOTATION_KEYWORDS.includes(run.toLowerCase()) ? run : null;
  },
};

/** A dotted reference span: an `isWordStart` run continued by `isWordChar`, joined by a `.` to
 * a further such run only when the character immediately after that `.` is itself an
 * identifier-start character — a `.` not followed by one is not crossed and the span ends
 * before it. Ordered LAST in the chain; only its position after `claimNotationKeyword` is
 * observable, since the two share `isWordStart` as their start set. */
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

/** The template grammar's recognizer chain, tried in this order at every position. Exactly ONE
 * pair's relative order is observable — `claimNotationKeyword` ahead of `claimIdentifierSpan`,
 * which share `isWordStart` as their start set. `claimLabelSpan`'s `[` and `claimIntegerRun`'s
 * digits are each in no other recognizer's start set, so those two positions are free and an
 * ordering claim about them would be unfalsifiable. Ordering is data here rather than nested
 * control flow precisely so the survival question is answerable by RUNNING the chain
 * (`checkNotationKey`) instead of by describing it. */
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
 * @returns The winning `Claim`, or a `FormulaError` rejecting the whole scan.
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
  // Trust-boundary validation: `resolve` is a consumer-supplied callback and is not
  // guaranteed to honor the `FormulaValue` contract (same boundary `evaluate`'s `ref`
  // case and the `evalNode` callback already cross via this shared helper).
  const value = validateResolverOutput(callResolver(path, resolve));
  if (isFormulaError(value)) return value;
  if (!Number.isInteger(value)) {
    return {
      error: "type",
      detail: `'${originalText}' = ${value}: roll templates require integers (use floor/round in the stat formula)`,
    };
  }
  // The cap is a MAGNITUDE test rather than a range test, so it is asymmetric about zero:
  // the most negative representable i32 (-2147483648) exceeds `I32_MAX` in magnitude and is
  // rejected as a cap error even though an i32 holds it. Deliberate — do not "fix" it into
  // a symmetric range check.
  if (Math.abs(value) > I32_MAX) {
    return { error: "cap", detail: `'${originalText}' = ${value}: out of i32 range` };
  }
  // A negative value emits the same labeled shape as a positive one, prefixed with a unary
  // minus, so the server parses `Expr::Neg` wrapping a labeled `Const` rather than two
  // unlabeled `Const`s: `collect_labeled_consts` recurses through `Expr::Neg` with a sign
  // flip and emits a `ConstTerm` for any `Const` carrying a label, so this form surfaces a
  // correctly-signed chip in the roll breakdown where `(0 - N)`'s two unlabeled consts would
  // not. Arithmetically identical to `(0 - N)` in every preceding context — the server's
  // recursive-descent grammar folds `x - (0 - N)` and `x - Neg(N)` to the same `x + N`.
  if (value < 0) return `-${-value}[${originalText}]`;
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

/** What the template grammar does to one written key — `checkNotationKey`'s result, and the
 * ONE enumeration of how a scan of that key can end. Each outcome below is a property of THAT
 * SCAN: what the recognizer chain claims over the text it is handed.
 *
 * The discriminator is TWO-LEVEL, and the first level is rejection. `rejects !== null` is one
 * outcome and excludes the other three, because `segments` then covers only the prefix claimed
 * ahead of the rejecting position and says nothing about the rest of the key — a rejected key
 * can carry claims of any shape, including the shape the SPLIT bullet describes. The other
 * three are properties of a value whose `rejects` is `null`, keyed on whether an
 * `"identifier"` claim survives, which is what decides whether any of the key reaches the
 * consumer's resolver. A claim COUNT decides nothing: a key claimed entirely as notation
 * carries as many claims as the grammar took to consume it and still reaches no resolver. So
 * read as a two-level discriminator these are TOTAL and mutually exclusive over the returned
 * value.
 *
 * - **A recognizer rejected.** The first and exclusive outcome. The scan in which the
 *   rejection occurs ends in that parse error instead of notation, and `rejects` is that
 *   verdict computed over the key ALONE.
 * - **No `"identifier"` claim among the segments, including none at all.** No span of the key
 *   reaches the consumer's identifier resolver and the scan returns no error on any path — the
 *   outcome no consumer data can make loud. The roll runs and the number changes. The key's
 *   text becomes notation rather than a reference, span by span as `emitClaim` decides; a
 *   reader wanting the emitted text for a given claim structure reads that function, since no
 *   count of rewritten spans holds — `"d.d"` emits `1d.1d`.
 * - **One `"identifier"` claim covering the whole key.** The `intact` verdict: the resolver is
 *   offered the path the author wrote, and nothing else is emitted for the key.
 * - **An `"identifier"` claim alongside others.** SPLIT: each identifier span is offered to
 *   the resolver on its own — paths the author never wrote — and every other span is emitted
 *   into the notation. Loud whenever the consumer holds no stat at one of those paths: that
 *   resolver's own unknown-reference error fails the scan. Silent only while every split path
 *   resolves. */
export interface NotationKeyCheck {
  /** `true` only when the whole key is claimed as ONE identifier span, which is the only
   * shape that reaches a consumer's identifier resolver as the author wrote it. The verdict
   * is about the key scanned from position ZERO and says nothing about what a template puts
   * in front of it: an intact key placed immediately after a digit run still has that run
   * emitted ahead of the substituted value, where the two concatenate into one number. */
  readonly intact: boolean;
  /** Every claim over the key, in order. On a rejection this holds only the PREFIX claimed
   * ahead of the rejecting position, which is why the three shape outcomes above are scoped to
   * `rejects === null`: read on a rejected value they answer about a prefix the consumer never
   * asked about. */
  readonly segments: readonly NotationKeySegment[];
  /** The error a recognizer rejected the key with — the REJECTED outcome above — or `null`
   * when no recognizer rejected it. */
  readonly rejects: FormulaError | null;
}

/** Answers whether a written key survives the notation-template grammar intact, by
 * RUNNING that grammar's recognizer chain over it — `claimAt`, the same chain
 * `resolveNotationTemplate` runs at every position, so within the grammar the two cannot
 * disagree about what claims a key. Two things sit outside the grammar and CAN part them: the
 * length cap named under Scope below, and `claimLabelSpan`'s extent.
 *
 * This is the authority a consuming system's stat-key authoring validation calls.
 * `NOTATION_KEYWORDS` membership is one of several ways a key fails and there is no closed
 * form for the rest, because surviving means being claimed by a recognizer no earlier
 * recognizer beat to the position. Reimplementing a rule instead of calling this is the
 * failure this function exists to end.
 *
 * Scope: the grammar only. `MAX_FORMULA_LENGTH` bounds a whole template rather than a key, so
 * it is not applied here, and no resolution is attempted — no resolver is accepted. A key past
 * that length is therefore scanned here and refused UNSCANNED by `resolveNotationTemplate`,
 * which returns its cap error before any recognizer runs.
 *
 * **The answer is over the key in ISOLATION, and one recognizer's extent is not key-local.**
 * `claimLabelSpan` scans FORWARD for a closing `]` through whatever source it is handed, so
 * a key carrying an unmatched `[` rejects here while a template that supplies a `]` further
 * along returns notation with no error at all, absorbing everything between the two
 * brackets as a label. A `rejects` verdict therefore says the key rejects ON ITS OWN, not
 * that every template containing it must. The two positions differ for the same reason: the
 * detail here counts from the start of the KEY, where a template's own error counts from
 * the start of the template.
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

/** Scans a template left to right through `claimAt`, emitting each claim's text via
 * `emitClaim` — which delegates to `substituteIdentifier` for an `"identifier"` claim and
 * decides the emitted text itself for every other kind. An unclaimed character always passes
 * through as a `"literal"` claim, so the scan never fails on unfamiliar input; the returned
 * `notation` is therefore NOT guaranteed to be text the server's parser accepts — e.g.
 * `resolveNotationTemplate("]", () => 7)` returns `{ notation: "]" }` unchanged.
 * INVARIANT: never throws; every failure path returns a FormulaError.
 *
 * `emitClaim` is not a pass-through for the non-identifier branch: it may prepend a synthesized
 * count. Every identifier substitution is labeled, positive or negative.
 *
 * Which recognizer claims a position is what decides whether an author's stat key survives as
 * a reference at all. How a key that loses ends is enumerated on `NotationKeyCheck`, so a
 * consuming system validates its keys by CALLING `checkNotationKey` — which runs this same
 * chain — rather than reasoning about the chain itself.
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
  // The scan's only carried state: whether the immediately preceding claim was an integer run.
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
