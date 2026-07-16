import { describe, expect, it } from "vitest";
import { resolveAll } from "./graph";
import type { FormulaValue } from "./types";

describe("resolveAll", () => {
  const nodes: Record<string, (get: (k: string) => FormulaValue) => FormulaValue> = {
    base: () => 2,
    a: (get) => { const b = get("base"); return typeof b === "number" ? b + 1 : b; },
    b: (get) => { const a = get("a"); return typeof a === "number" ? a * 2 : a; },
  };
  const evalNode = (k: string, get: (d: string) => FormulaValue) =>
    nodes[k] ? nodes[k](get) : ({ error: "unknown-ref", detail: k } as FormulaValue);

  it("resolves through dependencies", () => {
    const r = resolveAll(["b", "a", "base"], evalNode);
    expect(r.get("b")).toBe(6);
  });
  it("is order-independent", () => {
    const r1 = resolveAll(["base", "a", "b"], evalNode);
    const r2 = resolveAll(["b", "base", "a"], evalNode);
    expect(r1).toEqual(r2);
  });
  it("marks every cycle participant errored, never hangs", () => {
    const cyc = (k: string, get: (d: string) => FormulaValue): FormulaValue =>
      k === "x" ? get("y") : k === "y" ? get("x") : { error: "unknown-ref", detail: k };
    const r = resolveAll(["x", "y"], cyc);
    expect(r.get("x")).toMatchObject({ error: "cycle" });
    expect(r.get("y")).toMatchObject({ error: "cycle" });
  });
  it("caps total visits", () => {
    // a chain longer than MAX_GRAPH_VISITS trips the cap error, not a hang
    const chain = (k: string, get: (d: string) => FormulaValue): FormulaValue => {
      const n = Number(k.slice(1));
      return n <= 0 ? 0 : get(`n${n - 1}`);
    };
    const r = resolveAll(["n5000"], chain);
    expect(r.get("n5000")).toMatchObject({ error: "cap" });
  });
});
