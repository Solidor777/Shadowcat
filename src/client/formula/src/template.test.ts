import { describe, expect, it } from "vitest";
import { NOTATION_KEYWORDS, resolveNotationTemplate } from "./template";
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

// The two grammars reserve differently, and the doc on `NOTATION_KEYWORDS` states the split as the
// reason that list is public. Every case below derives its inputs from the list itself, so adding a
// keyword extends the coverage rather than leaving a silent hole.
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
    // `readAlphaPrefix` reads the maximal ALPHA run and stops at the first non-alpha
    // character, so a `NOTATION_KEYWORDS` member followed by a digit offers just that
    // member for the membership test and the trailing digit re-lexes as a notation atom
    // — the compound name never reaches the resolver. A consuming system's reserved-key
    // validation must reject this shape too, not just literal list members; a key
    // spelled this way is silently rewritten into an operator with no error on any path.
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
  it("the formula grammar reserves none of them: each parses as a reference", () => {
    for (const kw of NOTATION_KEYWORDS) {
      expect(parseFormula(kw), `'${kw}' did not parse as a reference`)
        .toEqual({ kind: "ref", path: [kw] });
    }
  });
});
