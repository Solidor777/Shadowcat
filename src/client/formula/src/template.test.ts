import { describe, expect, it } from "vitest";
import {
  NOTATION_KEYWORDS,
  checkNotationKey,
  resolveNotationTemplate,
  type NotationKeyCheck,
  type NotationKeySegment,
} from "./template";
import { parseFormula } from "./parser";
import type { FormulaValue } from "./types";

const env = (vals: Record<string, FormulaValue>) => (path: string[]): FormulaValue =>
  vals[path.join(".")] ?? { error: "unknown-ref", detail: path.join(".") };

describe("resolveNotationTemplate", () => {
  it("substitutes identifiers as labeled constants, leaves notation untouched", () => {
    expect(resolveNotationTemplate("2d20kh1 + dex", env({ dex: 3 })))
      .toEqual({ notation: "2d20kh1 + 3[dex]" });
  });
  it("notation atoms win over identifier lexing (d20, kh3, e5)", () => {
    expect(resolveNotationTemplate("2d20kh3+e5", env({}))).toEqual({ notation: "2d20kh3+e5" });
  });
  it("bare 'd' with no preceding count is normalized to '1d' (server parser expects a count)", () => {
    expect(resolveNotationTemplate("d20 + dex", env({ dex: 3 }))).toEqual({ notation: "1d20 + 3[dex]" });
    expect(resolveNotationTemplate("2d20", env({}))).toEqual({ notation: "2d20" });
  });
  it("a word that merely STARTS with a keyword letter is a stat (dex, damage, total)", () => {
    expect(resolveNotationTemplate("d6 + damage", env({ damage: 2 })))
      .toEqual({ notation: "1d6 + 2[damage]" });
  });
  it("dotted refs substitute with the full path as label", () => {
    expect(resolveNotationTemplate("d20 + hp.max", env({ "hp.max": 12 })))
      .toEqual({ notation: "1d20 + 12[hp.max]" });
  });
  it("existing [labels] pass through verbatim, even containing keywords", () => {
    expect(resolveNotationTemplate("2d6[kh fire] + str", env({ str: 1 })))
      .toEqual({ notation: "2d6[kh fire] + 1[str]" });
  });
  it("negative values emit parenthesized zero-minus form (no label)", () => {
    expect(resolveNotationTemplate("d20 + mod", env({ mod: -2 })))
      .toEqual({ notation: "1d20 + (0 - 2)" });
  });
  it("non-integer values are a type error (explicit rounding required)", () => {
    expect(resolveNotationTemplate("d20 + mod", env({ mod: 2.5 })))
      .toMatchObject({ error: "type" });
  });
  it("errored refs fail the whole template", () => {
    expect(resolveNotationTemplate("d20 + ghost", env({}))).toMatchObject({ error: "unknown-ref" });
  });
  it("out-of-i32-range values are a cap error", () => {
    expect(resolveNotationTemplate("d20 + big", env({ big: 2 ** 31 }))).toMatchObject({ error: "cap" });
  });
  it("a word that merely STARTS with a keyword letter is a stat (total)", () => {
    expect(resolveNotationTemplate("total + 1", env({ total: 5 })))
      .toEqual({ notation: "5[total] + 1" });
  });
  it("i32 boundary is intentionally asymmetric: max passes, min is rejected as cap", () => {
    expect(resolveNotationTemplate("d20 + posBig", env({ posBig: 2147483647 })))
      .toEqual({ notation: "1d20 + 2147483647[posBig]" });
    expect(resolveNotationTemplate("d20 + negBig", env({ negBig: -2147483648 })))
      .toMatchObject({ error: "cap" });
  });
  it("a label between an integer and a bare 'd' does not carry the count", () => {
    // The scan's carry is set by an integer run and by nothing else, so any claim between
    // the integer and the dice operator clears it and the operator is normalized to a
    // synthesized count of 1 — the label is emitted verbatim between the two.
    expect(resolveNotationTemplate("2[fire]d6", env({}))).toEqual({ notation: "2[fire]1d6" });
    expect(resolveNotationTemplate("2[fire]d", env({}))).toEqual({ notation: "2[fire]1d" });
    expect(resolveNotationTemplate("2d6", env({}))).toEqual({ notation: "2d6" });
  });
  it("unterminated '[' label is a parse error", () => {
    expect(resolveNotationTemplate("d20 + [oops", env({}))).toMatchObject({ error: "parse" });
  });

  describe("malformed/faulting resolver returns (trust boundary)", () => {
    it("resolver throws -> resolver-error", () => {
      const throwing = (): FormulaValue => {
        throw new Error("boom");
      };
      expect(resolveNotationTemplate("d20 + oops", throwing)).toMatchObject({ error: "resolver-error" });
    });
    it("resolver returns undefined -> resolver-error, not silently passed through", () => {
      const undef = (() => undefined) as unknown as (path: string[]) => FormulaValue;
      expect(resolveNotationTemplate("d20 + oops", undef)).toMatchObject({ error: "resolver-error" });
    });
    it("resolver returns a raw string -> resolver-error, not spliced into the notation", () => {
      const stringy = (() => "haxxor") as unknown as (path: string[]) => FormulaValue;
      const result = resolveNotationTemplate("d20 + oops", stringy);
      expect(result).toMatchObject({ error: "resolver-error" });
      expect(JSON.stringify(result)).not.toContain("haxxor");
    });
    it("resolver returns NaN -> non-finite, not resolver-error", () => {
      const nan = (() => NaN) as unknown as (path: string[]) => FormulaValue;
      expect(resolveNotationTemplate("d20 + oops", nan)).toMatchObject({ error: "non-finite" });
    });
    it("resolver returns Infinity -> non-finite, not resolver-error", () => {
      const inf = (() => Infinity) as unknown as (path: string[]) => FormulaValue;
      expect(resolveNotationTemplate("d20 + oops", inf)).toMatchObject({ error: "non-finite" });
    });
  });
});

// The two grammars reserve differently. Every case below derives its inputs from
// `NOTATION_KEYWORDS` itself, so adding a keyword extends the coverage rather than leaving a
// silent hole — but list membership is only one of the ways a key is taken, and
// `checkNotationKey` rather than this list is what a consuming system validates against.
describe("the reservation split between the two grammars", () => {
  /** A resolver recording every dotted path it is asked for, so a case can assert the
   * template grammar reserved a word BEFORE the consumer's identifier resolver saw it. */
  const spyResolve = () => {
    const seen: string[] = [];
    const resolve = (path: string[]): FormulaValue => {
      seen.push(path.join("."));
      return 7;
    };
    return { seen, resolve };
  };

  /** Asserts `word` is consumed by the template grammar's keyword branch: the resolver is
   * never consulted and no `[word]` label is emitted for it. */
  const expectReserved = (word: string) => {
    const { seen, resolve } = spyResolve();
    const result = resolveNotationTemplate(`2d20 + ${word}`, resolve);
    expect(seen, `'${word}' reached the identifier resolver`).toEqual([]);
    expect(JSON.stringify(result), `'${word}' substituted as a stat`).not.toContain(`[${word}]`);
  };

  it("every NOTATION_KEYWORDS member is reserved by the template grammar (resolver never consulted)", () => {
    for (const kw of NOTATION_KEYWORDS) expectReserved(kw);
  });

  it("the reserved set is LARGER than the list: a compound identifier whose leading alpha run is a keyword collides", () => {
    // `readKeywordRun` reads the maximal identifier-start run and stops at the first
    // character outside it, so a `NOTATION_KEYWORDS` member followed by a digit offers just
    // that member for the membership test and the trailing digit re-lexes as a notation
    // atom — the compound name never reaches the resolver, and is rewritten into an operator
    // with no error on any path.
    for (const kw of NOTATION_KEYWORDS) expectReserved(`${kw}1`);
    expect(resolveNotationTemplate("2d20 + t1", () => 7)).toEqual({ notation: "2d20 + t1" });
  });

  it("the reserved set is LARGER than the list: matching is case-insensitive, so any case spelling collides", () => {
    // The alpha run is lowercased before the membership test, so the list's lowercase
    // spelling is the only one written down but every case spelling is reserved. The
    // emitted text keeps the ORIGINAL case — the rewrite is a reservation, not a
    // normalization.
    for (const kw of NOTATION_KEYWORDS) expectReserved(kw.toUpperCase());
    expect(resolveNotationTemplate("2d20Kh1", () => 7)).toEqual({ notation: "2d20Kh1" });
  });
  /** Asserts a DOTTED reference whose FIRST SEGMENT is `word` is split at the dot: the
   * keyword branch consumes the leading segment, so the identifier-span logic below it is
   * never entered and the resolver is offered the REMAINDER alone — never the path the
   * author wrote. */
  const expectDottedSplit = (word: string) => {
    const { seen, resolve } = spyResolve();
    const written = `${word}.max`;
    const result = resolveNotationTemplate(`2d20 + ${written}`, resolve);
    expect(seen, `'${written}' did not split at the dot`).toEqual(["max"]);
    expect(JSON.stringify(result), `'${written}' resolved as a whole path`)
      .not.toContain(`[${written}]`);
  };

  it("the reserved set is LARGER than the list: a DOTTED path whose first segment is a keyword is split at the dot", () => {
    // `readKeywordRun` stops at the first character outside `isWordStart`, and a DOT is
    // outside it. So a dotted reference offers only its first segment for the membership
    // test; on a hit the keyword branch emits notation and continues, the dot falls
    // through as a literal, and the remainder re-lexes as a reference of its own. A
    // dotted path is this library's canonical reference form, which makes this the
    // likeliest collision shape in practice — and the consumer's resolver is asked for a
    // path the author never wrote, so whether anything is reported is the consumer's
    // answer for that fragment rather than a property of the split.
    for (const kw of NOTATION_KEYWORDS) expectDottedSplit(kw);
  });

  it("the dotted split is LOUD whenever the consumer holds no stat at the remainder", () => {
    // The split is silent only while the remainder happens to resolve. A consuming system
    // whose stat sits at the path the author WROTE has nothing at the remainder, answers
    // its own unknown-reference error, and that error fails the whole template — the
    // common real-world shape, and the one a spy resolver answering every path hides.
    for (const kw of NOTATION_KEYWORDS) {
      const written = `${kw}.max`;
      expect(resolveNotationTemplate(`2d20 + ${written}`, env({ [written]: 7 })), written)
        .toEqual({ error: "unknown-ref", detail: "max" });
    }
  });

  it("the dotted split emits the leading segment verbatim, and synthesizes a count for the dice keyword", () => {
    // Two exact rewrites, because the dice keyword's output differs: with no integer
    // immediately before it the keyword branch normalizes it to a `1`-prefixed count,
    // so the emitted text gains a character the author never wrote.
    expect(resolveNotationTemplate("2d20 + kh.max", env({ max: 7 })))
      .toEqual({ notation: "2d20 + kh.7[max]" });
    expect(resolveNotationTemplate("2d20 + d.max", env({ max: 7 })))
      .toEqual({ notation: "2d20 + 1d.7[max]" });
  });

  it("hands the resolver a RAW-CASE path, where the formula grammar hands it a lowercased one", () => {
    // Only the membership probe is lowercased. A non-colliding identifier reaches
    // `substituteIdentifier`, which splits the raw slice — so the path offered here keeps the
    // author's case, and so does the emitted label. `tokenize` lowercases every word token it
    // emits, so the same reference read through `parseFormula` arrives lowercased instead.
    // A consumer keying its resolver on lowercase therefore resolves one grammar and misses the
    // other, and the miss surfaces as its OWN unknown-reference error.
    const { seen, resolve } = spyResolve();
    expect(resolveNotationTemplate("2d20 + HP.Max", resolve))
      .toEqual({ notation: "2d20 + 7[HP.Max]" });
    expect(seen).toEqual(["HP.Max"]);
    expect(parseFormula("HP.Max")).toEqual({ kind: "ref", path: ["hp", "max"] });
  });

  it("the formula grammar reserves none of them: each parses as a reference", () => {
    for (const kw of NOTATION_KEYWORDS) {
      expect(parseFormula(kw), `'${kw}' did not parse as a reference`)
        .toEqual({ kind: "ref", path: [kw] });
    }
  });
});

// `checkNotationKey` is the authority a consuming system's stat-key authoring validation
// calls, and it answers by RUNNING the same recognizer chain the rewrite runs. These cases
// pin the shapes this grammar takes apart — keyword runs, leading and embedded digits, the
// dot edges, brackets, non-word characters — against the ordinary keys they contrast with,
// and the last case pins the two answers to each other so the checker cannot drift from the
// rewrite.
describe("checkNotationKey", () => {
  /** Runs the REWRITE over a bare key with a resolver answering every path, reporting the
   * paths it was asked for and what the REWRITE produced — the resolver's own return is the
   * same number every time and carries no information. Answering everything is deliberate: it
   * makes a case observe the SHAPE the grammar imposed rather than the consumer's own
   * unknown-reference error, which would mask the paths the split actually offered. */
  const rewriteKey = (key: string): { seen: string[]; result: ReturnType<typeof resolveNotationTemplate> } => {
    const seen: string[] = [];
    const result = resolveNotationTemplate(key, (path) => {
      seen.push(path.join("."));
      return 7;
    });
    return { seen, result };
  };

  /** Reports whether the key survived the rewrite whole: the resolver was asked for exactly
   * the key the author wrote, and the emitted notation is exactly that one substitution.
   * This is the observable consequence `intact` predicts. */
  const rewriteKeepsKeyWhole = (key: string): boolean => {
    const { seen, result } = rewriteKey(key);
    if (!("notation" in result)) return false;
    return seen.length === 1 && seen[0] === key && result.notation === `7[${key}]`;
  };

  /** Every key shape this grammar can claim as something other than one identifier span,
   * beside the ordinary keys that survive whole. Keyword-derived entries come from
   * `NOTATION_KEYWORDS` itself, so a keyword added there is covered without a second
   * edit. */
  const CORPUS: readonly string[] = [
    ...NOTATION_KEYWORDS,
    ...NOTATION_KEYWORDS.map((kw) => `${kw}1`),
    ...NOTATION_KEYWORDS.map((kw) => kw.toUpperCase()),
    ...NOTATION_KEYWORDS.map((kw) => `${kw}.max`),
    "hp", "hp.max", "a.b.c", "str_mod", "_x", "x1", "HP.Max", "hp.d", "damage", "total",
    "kh_max", "2hp", "0hp", "42", "hp.2max", "hp max", "hp-max", "hp+max", "hp[max",
    "2hp[max", "hp[max]", "hp.", ".hp", "hp..max", "hp]max", "[hp]", "-", "",
  ];

  /** The states `NotationKeyCheck` enumerates, one predicate each, read straight off a
   * returned value — including the `intact` FIELD, which is read rather than recomputed, so a
   * change to how the checker sets it moves the classification here instead of being masked by
   * a second copy of its rule. Rejection is the first-level discriminator and the three shape
   * predicates all guard on it. The shape discriminator is whether an `"identifier"` claim
   * SURVIVES, never how many claims there are: a key the grammar consumed entirely as notation
   * carries as many claims as the grammar took to consume it — one for a bare keyword, none at
   * all for the empty key — and reaches no resolver either way. */
  const STATES: readonly { readonly name: string; readonly holds: (check: NotationKeyCheck) => boolean }[] = [
    { name: "rejected", holds: (check) => check.rejects !== null },
    {
      name: "no surviving identifier",
      holds: (check) =>
        check.rejects === null && !check.segments.some((seg) => seg.kind === "identifier"),
    },
    { name: "intact", holds: (check) => check.rejects === null && check.intact },
    {
      name: "split",
      holds: (check) =>
        check.rejects === null
        && check.segments.some((seg) => seg.kind === "identifier")
        && !check.intact,
    },
  ];

  /** The dice operator, read off the list rather than respelled: its own declaration is both
   * `NOTATION_KEYWORDS`' first member and `emitClaim`'s test, so reading the first member is
   * reading that declaration. A literal here would be a second spelling of it. */
  const DICE_OPERATOR = NOTATION_KEYWORDS[0];

  /** The notation the silent state predicts for a key: every claimed span verbatim, save the
   * ONE span the emission rewrites — a `DICE_OPERATOR` keyword with no integer run immediately
   * before it gains a synthesized count of `1`. This is a deliberate second statement of
   * `emitClaim`'s only rewrite, and it is the assertion that makes "emitted verbatim" testable
   * rather than asserted: any further rewrite the emitter grows fails here. */
  const emittedVerbatim = (segments: readonly NotationKeySegment[]): string =>
    segments
      .map((seg, idx) => {
        const countPrecedes = idx > 0 && segments[idx - 1].kind === "integer";
        const isDiceOperator = seg.kind === "keyword" && seg.text.toLowerCase() === DICE_OPERATOR;
        return isDiceOperator && !countPrecedes ? `1${seg.text}` : seg.text;
      })
      .join("");

  /** Classifies one key from what `checkNotationKey` RETURNS, then checks the consequence that
   * classification claims against what `resolveNotationTemplate` observably does to the same
   * key. Returns one line per violation — an empty array is agreement — so the caller asserts
   * over the whole key set at once instead of stopping at the first, and so the hand-written
   * corpus and the generated sweep below share ONE statement of the contract rather than two
   * that can drift.
   *
   * The oracle is the rewrite, never a predicate written beside the states: a predicate set
   * authored by the same hand as the taxonomy agrees with it by construction. What this is NOT
   * independent of is the recognizer chain — `checkNotationKey` and the rewrite both run
   * `claimAt`, so a defect INSIDE a recognizer moves both answers together and passes here.
   * What it is independent of is the prose and the hand-written case list. */
  const consequenceViolations = (key: string): string[] => {
    const check = checkNotationKey(key);
    const matched = STATES.filter((state) => state.holds(check)).map((state) => state.name);
    const kinds = JSON.stringify(check.segments.map((seg) => seg.kind));
    const where = `${JSON.stringify(key)} kinds=${kinds}`;
    if (matched.length !== 1) {
      return [`${where}: ${matched.length} states hold ${JSON.stringify(matched)}, expected exactly 1`];
    }
    const [state] = matched;
    const { seen, result } = rewriteKey(key);
    const at = `${where} (${state})`;
    if (state === "rejected") {
      // Same scan, same origin for the position: the rewrite is run over the bare key here, so
      // the two errors must be identical rather than merely both parse errors.
      return JSON.stringify(result) === JSON.stringify(check.rejects)
        ? []
        : [`${at}: rewrite returned ${JSON.stringify(result)}, rejects=${JSON.stringify(check.rejects)}`];
    }
    if (!("notation" in result)) return [`${at}: the rewrite failed with ${JSON.stringify(result)}`];
    const bad: string[] = [];
    if (state === "no surviving identifier") {
      if (seen.length > 0) bad.push(`${at}: the resolver was asked for ${JSON.stringify(seen)}`);
      const expected = emittedVerbatim(check.segments);
      if (result.notation !== expected) {
        bad.push(`${at}: emitted ${JSON.stringify(result.notation)}, expected ${JSON.stringify(expected)}`);
      }
    }
    if (state === "intact" && (seen.length !== 1 || seen[0] !== key)) {
      bad.push(`${at}: the resolver was asked for ${JSON.stringify(seen)}, expected the key once`);
    }
    if (state === "split") {
      if (seen.length === 0) bad.push(`${at}: the resolver was asked for nothing`);
      if (seen.includes(key)) bad.push(`${at}: the resolver was asked for the key as written`);
    }
    return bad;
  };

  it("a bare keyword never survives, whatever case it is written in", () => {
    for (const kw of NOTATION_KEYWORDS) {
      expect(checkNotationKey(kw), kw).toMatchObject({ intact: false });
      expect(checkNotationKey(kw.toUpperCase()), kw).toMatchObject({ intact: false });
    }
  });

  it("a keyword followed by a digit never survives, and the digit is claimed separately", () => {
    for (const kw of NOTATION_KEYWORDS) {
      const check = checkNotationKey(`${kw}1`);
      expect(check.intact, `${kw}1`).toBe(false);
      expect(check.segments.map((s) => s.kind), `${kw}1`).toEqual(["keyword", "integer"]);
    }
  });

  it("a dotted path whose FIRST segment is a keyword splits at the dot", () => {
    for (const kw of NOTATION_KEYWORDS) {
      const check = checkNotationKey(`${kw}.max`);
      expect(check.intact, `${kw}.max`).toBe(false);
      expect(check.segments.map((s) => s.kind), `${kw}.max`).toEqual(["keyword", "literal", "identifier"]);
    }
  });

  it("a keyword AFTER a dot is harmless: reservation applies where a claim starts", () => {
    expect(checkNotationKey("hp.d")).toMatchObject({ intact: true });
    expect(checkNotationKey("hp.kh")).toMatchObject({ intact: true });
  });

  it("a character the identifier span does not accept splits the key silently", () => {
    for (const key of ["hp-max", "hp+max", "hp max"]) {
      const check = checkNotationKey(key);
      expect(check.intact, key).toBe(false);
      expect(check.segments.map((s) => s.kind), key).toEqual(["identifier", "literal", "identifier"]);
    }
  });

  it("a key beginning with a digit loses the digit INTO the substituted value", () => {
    // The integer run is emitted ahead of the substitution and concatenates with it, so
    // the roll silently totals a different number rather than failing on a bad reference:
    // `2hp` with hp = 7 rolls 27, not 7 and not an error.
    expect(checkNotationKey("2hp")).toMatchObject({ intact: false });
    expect(checkNotationKey("2hp").segments).toEqual([
      { kind: "integer", text: "2", at: 0 },
      { kind: "identifier", text: "hp", at: 1 },
    ]);
    expect(resolveNotationTemplate("2hp", () => 7)).toEqual({ notation: "27[hp]" });
    expect(resolveNotationTemplate("12hp", () => 7)).toEqual({ notation: "127[hp]" });
  });

  it("a dotted segment beginning with a digit splits, where the formula grammar errors", () => {
    // Same written key, two grammars, two failure modes: the template grammar splits it
    // silently and the formula grammar refuses to parse it. Testing one says nothing
    // about the other.
    expect(checkNotationKey("hp.2max")).toMatchObject({ intact: false });
    expect(checkNotationKey("hp.2max").segments.map((s) => s.kind))
      .toEqual(["identifier", "literal", "integer", "identifier"]);
    expect(resolveNotationTemplate("hp.2max", () => 7)).toEqual({ notation: "7[hp].27[max]" });
    expect(parseFormula("hp.2max")).toMatchObject({ error: "parse" });
  });

  it("a dot with no identifier-start character after it is a literal, wherever it sits", () => {
    // The identifier span crosses a dot only when an identifier-start character follows it,
    // so a trailing, leading or doubled dot drops through to the literal fallthrough one
    // character at a time and the key is split around it.
    expect(checkNotationKey("hp.").segments.map((s) => s.kind)).toEqual(["identifier", "literal"]);
    expect(checkNotationKey(".hp").segments.map((s) => s.kind)).toEqual(["literal", "identifier"]);
    expect(checkNotationKey("hp..max").segments.map((s) => s.kind))
      .toEqual(["identifier", "literal", "literal", "identifier"]);
  });

  it("a closed bracket run is claimed as a label, never as part of the author's key", () => {
    // Nothing but the label recognizer claims a `[`, and its claim covers the closing
    // bracket, so a bracketed run is taken whole and its contents are never scanned — the
    // only way a key yields a `"label"` claim.
    expect(checkNotationKey("hp[max]").segments).toEqual([
      { kind: "identifier", text: "hp", at: 0 },
      { kind: "label", text: "[max]", at: 2 },
    ]);
    expect(checkNotationKey("[hp]").segments).toEqual([{ kind: "label", text: "[hp]", at: 0 }]);
  });

  it("a key that rejects the template reports the error rather than a split", () => {
    const check = checkNotationKey("hp[max");
    expect(check.intact).toBe(false);
    expect(check.rejects).toMatchObject({ error: "parse" });
    expect(resolveNotationTemplate("hp[max", () => 7)).toMatchObject({ error: "parse" });
  });

  it("a rejection is answered over the key ALONE: a later ']' in a template absorbs the rest", () => {
    // `claimLabelSpan` scans forward for a closing bracket through whatever source it is
    // handed, which is the one recognizer whose extent is not key-local. So a key holding
    // an unmatched `[` rejects on its own, while a template supplying a `]` further along
    // runs and swallows the text between the brackets as a label — and the positions in
    // the two errors are counted from different origins.
    expect(checkNotationKey("hp[max").rejects)
      .toEqual({ error: "parse", detail: "unterminated '[' label at position 2" });
    expect(resolveNotationTemplate("2d20 + hp[max", () => 7))
      .toEqual({ error: "parse", detail: "unterminated '[' label at position 9" });
    expect(resolveNotationTemplate("hp[max + 1d6] + str", () => 7))
      .toEqual({ notation: "7[hp][max + 1d6] + 7[str]" });
  });

  it("an ordinary key survives whole, dotted or not, in any case", () => {
    // `kh_max` is safe although `kh1` is not: `_` is an identifier-start character, so the
    // keyword run swallows it and the run tested for membership is the whole key.
    for (const key of ["hp", "hp.max", "a.b.c", "str_mod", "_x", "x1", "HP.Max", "kh_max"]) {
      expect(checkNotationKey(key), key).toMatchObject({ intact: true, rejects: null });
      expect(checkNotationKey(key).segments.map((s) => s.text), key).toEqual([key]);
    }
    expect(checkNotationKey("")).toEqual({ intact: false, segments: [], rejects: null });
  });

  it("each state predicts what the rewrite does to the key it classifies", () => {
    // The states are worth nothing as a partition alone; each names a consequence, and this
    // is what ties them to observable behaviour. The silent state is the load-bearing one:
    // its defining property is that no `"identifier"` claim is among the segments, so no span
    // reaches the consumer's resolver whatever claimed the key instead.
    expect(CORPUS.flatMap(consequenceViolations)).toEqual([]);
  });

  /** Every letter any `NOTATION_KEYWORDS` member is spelled with, derived from the list rather
   * than listed again, so a keyword added there widens the sweep's alphabet with no second
   * edit here. */
  const KEYWORD_LETTERS: readonly string[] = [...new Set(NOTATION_KEYWORDS.join(""))].sort();

  /** The sweep's alphabet. Reachability is the floor it has to clear — every recognizer and
   * the literal fallthrough must be reachable — and past that floor each member is here
   * because some branch is unobservable without it:
   * - every keyword letter, so every `NOTATION_KEYWORDS` member, every proper prefix of one
   *   and every near-miss spelling is generated rather than listed;
   * - the dice operator in UPPER case, because `claimNotationKeyword` lowercases the run
   *   before its membership probe and `emitClaim` lowercases before synthesizing a count, and
   *   neither lowercasing is observable from a lowercase-only alphabet;
   * - a letter no keyword is spelled with, so a run that cannot collide exists;
   * - a digit, reaching `claimIntegerRun` and the mid-identifier `isWordChar` branch;
   * - a dot, the segment join `claimIdentifierSpan` crosses only before an identifier start;
   * - an underscore: an identifier-start character that is not a letter, so it CONTINUES a
   *   keyword run where a digit ends one;
   * - both brackets, reaching `claimLabelSpan` terminated and unterminated — the closing one
   *   is claimed by no recognizer on its own and is what decides which of the two the opening
   *   one is;
   * - a hyphen, claimed by no recognizer at any position, reaching the literal fallthrough. */
  const ALPHABET: readonly string[] = [
    ...KEYWORD_LETTERS, DICE_OPERATOR.toUpperCase(), "a", "1", ".", "_", "[", "]", "-",
  ];

  /** The longest generated key. Four characters is what reaches the multi-claim shapes the
   * hand-written cases were written for — a keyword run followed by a digit, an identifier
   * followed by an unclosed bracket, a dotted path, a count ahead of a dice operator — with at
   * least one character to spare on each. */
  const SWEEP_LENGTH = 4;

  /** Every string over `ALPHABET` from the empty key up to `SWEEP_LENGTH`, in full. Exhaustive
   * over a bounded space rather than sampled from an unbounded one, so a clean run is a proof
   * about that space and not evidence about a draw from it. */
  const sweepKeys = (): string[] => {
    const keys: string[] = [""];
    let frontier: string[] = [""];
    for (let length = 1; length <= SWEEP_LENGTH; length++) {
      const next: string[] = [];
      for (const prefix of frontier) for (const ch of ALPHABET) next.push(prefix + ch);
      for (const key of next) keys.push(key);
      frontier = next;
    }
    return keys;
  };

  it("every generated key over a bounded alphabet lands in one state whose consequence holds", () => {
    // The hand-written cases above are one author's list of shapes, and so is the corpus, and
    // so would be any predicate set written beside them: a case nobody thought of is by
    // definition absent from all three. This enumerates keys MECHANICALLY instead and checks
    // each against what the rewrite observably does, which is what makes a shape nobody
    // listed reachable at all.
    const keys = sweepKeys();
    // Positive control on the GENERATOR: an alphabet and a length are worth only what they
    // reach, so name one key of each multi-claim shape the length was chosen for and require
    // the generated set to hold it. A generator quietly producing fewer shapes would
    // otherwise report a clean sweep over a space missing them.
    for (const shape of ["kh1", "h[", "h.h", "1d", "D", "a-a", "[]", "_1", "hh]"]) {
      expect(keys.includes(shape), `the sweep never generated ${JSON.stringify(shape)}`).toBe(true);
    }
    const violations = keys.flatMap(consequenceViolations);
    expect(
      violations.slice(0, 10),
      `${violations.length} of ${keys.length} generated keys broke their state's consequence`,
    ).toEqual([]);
  });

  it("the verdict matches what the rewrite actually does to the key, shape for shape", () => {
    // The anti-drift pin: `checkNotationKey` and `resolveNotationTemplate` reach their
    // answers through the same recognizer chain, so a change to one must move the other.
    // A checker that restated the rule instead would pass its own cases and disagree here.
    for (const key of CORPUS) {
      expect(checkNotationKey(key).intact, key).toBe(rewriteKeepsKeyWhole(key));
    }
  });
});
