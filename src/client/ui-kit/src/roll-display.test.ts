import { describe, it, expect } from "vitest";
import { natExtreme } from "./roll-display";

describe("natExtreme", () => {
  const numeric = { Numeric: { min: 1, max: 20 } };

  it("reports max and min for the bounds of a Numeric die, and null between them", () => {
    expect(natExtreme({ natural: 20, kind: numeric })).toBe("max");
    expect(natExtreme({ natural: 1, kind: numeric })).toBe("min");
    expect(natExtreme({ natural: 10, kind: numeric })).toBeNull();
  });

  it("reports null for a Faces die and for an absent kind", () => {
    const faces = { Faces: { faces: [{ value: 1, symbols: [] }] } };
    expect(natExtreme({ natural: 1, kind: faces })).toBeNull();
    expect(natExtreme({ natural: 1, kind: null })).toBeNull();
    expect(natExtreme({ natural: 1 })).toBeNull();
  });

  it("reports max for a one-sided range", () => {
    expect(natExtreme({ natural: 1, kind: { Numeric: { min: 1, max: 1 } } })).toBe("max");
  });
});
