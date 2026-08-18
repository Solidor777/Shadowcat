import { describe, expect, it } from "vitest";
import { NOTATION_KEYWORDS, checkNotationKey, resolveNotationTemplate } from "./template";
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
    // path the author never wrote, with no error on any path.
    for (const kw of NOTATION_KEYWORDS) expectDottedSplit(kw);
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
// pin the shapes four rounds of review each described differently, and the last case pins
// the two answers to each other so the checker cannot drift from the rewrite.
describe("checkNotationKey", () => {
  /** Runs the REWRITE over a bare key and reports whether the key survived whole: the
   * resolver was asked for exactly the key the author wrote, and the emitted notation is
   * exactly that one substitution. This is the observable consequence `intact` predicts. */
  const rewriteKeepsKeyWhole = (key: string): boolean => {
    const seen: string[] = [];
    const result = resolveNotationTemplate(key, (path) => {
      seen.push(path.join("."));
      return 7;
    });
    if (!("notation" in result)) return false;
    return seen.length === 1 && seen[0] === key && result.notation === `7[${key}]`;
  };

  /** Every key shape the review rounds surfaced, plus the safe shapes they contrast with.
   * Keyword-derived entries come from `NOTATION_KEYWORDS` itself, so a keyword added there
   * is covered without a second edit. */
  const CORPUS: readonly string[] = [
    ...NOTATION_KEYWORDS,
    ...NOTATION_KEYWORDS.map((kw) => `${kw}1`),
    ...NOTATION_KEYWORDS.map((kw) => kw.toUpperCase()),
    ...NOTATION_KEYWORDS.map((kw) => `${kw}.max`),
    "hp", "hp.max", "a.b.c", "str_mod", "_x", "x1", "HP.Max", "hp.d", "damage", "total",
    "2hp", "0hp", "hp.2max", "hp max", "hp-max", "hp+max", "hp[max", "hp[max]", "",
  ];

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

  it("a key that rejects the template reports the error rather than a split", () => {
    const check = checkNotationKey("hp[max");
    expect(check.intact).toBe(false);
    expect(check.rejects).toMatchObject({ error: "parse" });
    expect(resolveNotationTemplate("hp[max", () => 7)).toMatchObject({ error: "parse" });
  });

  it("an ordinary key survives whole, dotted or not, in any case", () => {
    for (const key of ["hp", "hp.max", "a.b.c", "str_mod", "_x", "x1", "HP.Max"]) {
      expect(checkNotationKey(key), key).toMatchObject({ intact: true, rejects: null });
      expect(checkNotationKey(key).segments.map((s) => s.text), key).toEqual([key]);
    }
    expect(checkNotationKey("")).toEqual({ intact: false, segments: [], rejects: null });
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
