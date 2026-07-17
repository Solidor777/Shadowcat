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
  it("reports an order-independent canonical cycle detail (smallest member) regardless of entry order", () => {
    // 3-cycle a -> b -> c -> a. The FULL result (kind AND detail) must be
    // independent of key order, per this module's documented invariant: detail
    // names the lexicographically smallest cycle member ('a'), not whichever
    // member the traversal happened to re-enter first.
    const cyc = (k: string, get: (d: string) => FormulaValue): FormulaValue =>
      k === "a" ? get("b") : k === "b" ? get("c") : k === "c" ? get("a") : { error: "unknown-ref", detail: k };
    const forward = resolveAll(["a", "b", "c"], cyc);
    const reverse = resolveAll(["c", "b", "a"], cyc);
    expect(forward).toEqual(reverse);
    expect(forward.get("a")).toEqual({ error: "cycle", detail: "reference cycle involving 'a'" });
    expect(reverse.get("c")).toEqual({ error: "cycle", detail: "reference cycle involving 'a'" });
  });
  it("a short-circuitable cycle resolves identically regardless of entry order (error KIND, not just detail)", () => {
    // s1 requests s2 (the dependency edge exists) but its own value
    // short-circuits to unknown-ref without needing s2; s2's value is
    // get(s1). Structurally s1<->s2 is a cycle. Path-dependent handling
    // would give s2 = cycle when entered from s1 (s1 still mid-evaluation
    // on the active path) but s2 = unknown-ref when entered from s2 (the
    // trampoline completes s1 first, s2 reads the memo) — sorted-root
    // iteration makes the outcome a pure function of the key set.
    const shortCircuit = (k: string, get: (d: string) => FormulaValue): FormulaValue => {
      if (k === "s1") {
        get("s2");
        return { error: "unknown-ref", detail: "u missing" };
      }
      if (k === "s2") return get("s1");
      return { error: "unknown-ref", detail: k };
    };
    const a = resolveAll(["s1", "s2"], shortCircuit);
    const b = resolveAll(["s2", "s1"], shortCircuit);
    expect(a.get("s1")).toEqual(b.get("s1"));
    expect(a.get("s2")).toEqual(b.get("s2"));
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

  it("turns a throwing evalNode into a resolver-error, never propagates", () => {
    const throwing = (): FormulaValue => {
      throw new Error("boom");
    };
    const r = resolveAll(["x"], throwing);
    expect(r.get("x")).toMatchObject({ error: "resolver-error" });
  });

  it("an evalNode's thrown exception message never leaks into detail (player-presentable field)", () => {
    const throwing = (): FormulaValue => {
      throw new Error("SECRET_INTERNAL_DETAIL");
    };
    const r = resolveAll(["x"], throwing);
    const result = r.get("x");
    expect(result).toMatchObject({ error: "resolver-error" });
    expect((result as { detail: string }).detail).not.toContain("SECRET_INTERNAL_DETAIL");
  });

  it("turns a malformed evalNode return value into a resolver-error", () => {
    // returns a value that is neither a finite number nor a well-formed FormulaError
    const malformed = () => "not a FormulaValue" as unknown as FormulaValue;
    const r = resolveAll(["x"], malformed);
    expect(r.get("x")).toMatchObject({ error: "resolver-error" });
  });

  it("resolves a long non-cyclic chain (depth under the visit cap) without a stack overflow", () => {
    // Depth 2000 stays under MAX_GRAPH_VISITS (2048) so the cap never trips —
    // this proves resolution succeeds regardless of chain depth (would have
    // been recursion-unsafe on a constrained JS stack before the iterative
    // rewrite: node --stack-size=200 threw RangeError at depth 770).
    const chain = (k: string, get: (d: string) => FormulaValue): FormulaValue => {
      const n = Number(k.slice(1));
      return n <= 0 ? 0 : get(`n${n - 1}`);
    };
    const r = resolveAll(["n2000"], chain);
    expect(r.get("n2000")).toBe(0);
  });

  it("evaluates a shared (diamond) dependency exactly once across two entry points", () => {
    let dCalls = 0;
    const diamondNodes: Record<string, (get: (k: string) => FormulaValue) => FormulaValue> = {
      d: () => {
        dCalls += 1;
        return 4;
      },
      a: (get) => {
        const d = get("d");
        return typeof d === "number" ? d + 1 : d;
      },
      c: (get) => {
        const d = get("d");
        return typeof d === "number" ? d + 2 : d;
      },
    };
    const evalDiamond = (k: string, get: (d: string) => FormulaValue) =>
      diamondNodes[k] ? diamondNodes[k](get) : ({ error: "unknown-ref", detail: k } as FormulaValue);

    const r = resolveAll(["a", "c"], evalDiamond);
    expect(r.get("a")).toBe(5);
    expect(r.get("c")).toBe(6);
    expect(dCalls).toBe(1);
  });
});
