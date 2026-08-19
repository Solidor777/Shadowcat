import { describe, expect, it } from "vitest";
import {
  FORMULA_ERROR_KINDS,
  isFormulaError,
  MAX_FORMULA_LENGTH,
  type FormulaErrorKind,
  type FormulaValue,
} from "./types";

describe("formula value model", () => {
  it("discriminates numbers from error values", () => {
    const ok: FormulaValue = 3;
    const bad: FormulaValue = { error: "parse", detail: "unexpected '?'" };
    expect(isFormulaError(ok)).toBe(false);
    expect(isFormulaError(bad)).toBe(true);
  });
  it("exposes caps", () => {
    expect(MAX_FORMULA_LENGTH).toBe(512);
  });
  // The union and its runtime mirror are enforced in OPPOSITE directions. `FORMULA_ERROR_KINDS`'
  // own `satisfies` clause rejects an array entry the union does not declare; nothing in the
  // source rejects a union member the array omits.
  //
  // Omitting a kind the library ALREADY emits is caught elsewhere by consequence, since
  // `validateResolverOutput` then rejects that kind and the behaviour tests exercising it fail.
  // What nothing catches is a kind added to the union that no code emits YET: the pair is already
  // inconsistent, every test still passes, and the failure surfaces only once something starts
  // emitting it — as a boundary fault rather than as the real error.
  //
  // This closes that case without changing how the public type is DECLARED: the exhaustive record
  // makes adding a union member a COMPILE error here until it is listed, and the assertion then
  // requires the runtime array to carry it too.
  it("mirrors every kind the union declares, in the direction the source cannot enforce", () => {
    const everyKind: Record<FormulaErrorKind, true> = {
      "parse": true,
      "unknown-ref": true,
      "type": true,
      "div-zero": true,
      "non-finite": true,
      "cycle": true,
      "cap": true,
      "ref-error": true,
      "resolver-error": true,
    };
    expect([...FORMULA_ERROR_KINDS].sort()).toEqual(Object.keys(everyKind).sort());
  });
});
