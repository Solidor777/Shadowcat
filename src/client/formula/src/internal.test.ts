import { describe, expect, it } from "vitest";
import { isWellFormedError, validateResolverOutput } from "./internal";

describe("isWellFormedError", () => {
  it("accepts a well-formed FormulaError with a real FormulaErrorKind", () => {
    expect(isWellFormedError({ error: "div-zero", detail: "x / 0" })).toBe(true);
  });
  it("rejects an error tag outside the FormulaErrorKind union", () => {
    expect(isWellFormedError({ error: "bogus-kind", detail: "x" })).toBe(false);
  });
  it("rejects non-string detail", () => {
    expect(isWellFormedError({ error: "div-zero", detail: 5 })).toBe(false);
  });
});

describe("validateResolverOutput", () => {
  it("coerces an invalid error kind to a synthetic resolver-error", () => {
    expect(validateResolverOutput({ error: "bogus-kind", detail: "x" })).toMatchObject({
      error: "resolver-error",
    });
  });
  it("passes a genuinely well-formed FormulaError through unchanged", () => {
    const err = { error: "cycle", detail: "a -> b -> a" };
    expect(validateResolverOutput(err)).toEqual(err);
  });
});
