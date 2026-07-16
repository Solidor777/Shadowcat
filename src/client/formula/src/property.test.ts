import { describe, expect, it } from "vitest";
import { parseFormula } from "./parser";
import { evaluate } from "./evaluate";
import { resolveAll } from "./graph";
import { resolveNotationTemplate } from "./template";
import { isFormulaError, type FormulaError, type FormulaValue } from "./types";

/** SplitMix32-style seeded PRNG — deterministic across runs, no runtime dep. */
function rng(seed: number) {
  let s = seed >>> 0;
  return () => {
    s = (s + 0x9e3779b9) >>> 0;
    let z = s;
    z = Math.imul(z ^ (z >>> 16), 0x21f0aaad);
    z = Math.imul(z ^ (z >>> 15), 0x735a2d97);
    return ((z ^ (z >>> 15)) >>> 0) / 2 ** 32;
  };
}

/** Fisher-Yates shuffle driven by the same seeded PRNG, for the key-order-independence
 * property below — never mutates the input array. */
function shuffle(arr: readonly string[], r: () => number): string[] {
  const a = arr.slice();
  for (let i = a.length - 1; i > 0; i--) {
    const j = Math.floor(r() * (i + 1));
    [a[i], a[j]] = [a[j], a[i]];
  }
  return a;
}

function mapToObj(m: Map<string, FormulaValue>): Record<string, FormulaValue> {
  const o: Record<string, FormulaValue> = {};
  for (const [k, v] of m) o[k] = v;
  return o;
}

describe("formula properties", () => {
  it("never throws on arbitrary input (parse + template)", () => {
    const r = rng(1);
    const alphabet = "dexkh0123456789 +-*/%().,[]_abz!<>=";
    for (let i = 0; i < 1000; i++) {
      const len = Math.floor(r() * 60);
      let s = "";
      for (let j = 0; j < len; j++) s += alphabet[Math.floor(r() * alphabet.length)];
      expect(() => parseFormula(s)).not.toThrow();
      expect(() => resolveNotationTemplate(s, () => 1)).not.toThrow();
    }
  });

  it("evaluates any successfully-parsed random formula to a number or error, never NaN", () => {
    const r = rng(2);
    for (let i = 0; i < 500; i++) {
      const src = randomExpr(r, 0);
      const ast = parseFormula(src);
      if (isFormulaError(ast as never)) continue;
      const v = evaluate(ast as never, () => Math.floor(r() * 10) - 3);
      if (!isFormulaError(v)) expect(Number.isFinite(v)).toBe(true);
    }
  });

  it("resolveAll is key-order independent (random DAGs)", () => {
    // 20-node DAG over add/mul: node i (i >= 2) depends on two strictly-earlier
    // nodes (j < i, k < i), guaranteeing acyclicity by construction; the rest
    // are leaves. Resolved with 3 different key-order shuffles; the Nightfox
    // permutation invariant (spec D3/D12) requires identical results.
    const r = rng(3);
    const n = 20;
    type Node =
      | { kind: "leaf"; value: number }
      | { kind: "op"; op: "add" | "mul"; a: number; b: number };
    const nodes: Node[] = [];
    for (let i = 0; i < n; i++) {
      if (i < 2 || r() < 0.2) {
        nodes.push({ kind: "leaf", value: Math.floor(r() * 10) + 1 });
      } else {
        nodes.push({
          kind: "op",
          op: r() < 0.5 ? "add" : "mul",
          a: Math.floor(r() * i),
          b: Math.floor(r() * i),
        });
      }
    }
    const keys = Array.from({ length: n }, (_, i) => String(i));
    const evalNode = (key: string, get: (dep: string) => FormulaValue): FormulaValue => {
      const node = nodes[Number(key)];
      if (node.kind === "leaf") return node.value;
      const av = get(String(node.a));
      if (isFormulaError(av)) return av;
      const bv = get(String(node.b));
      if (isFormulaError(bv)) return bv;
      return node.op === "add" ? av + bv : av * bv;
    };

    const shuffled1 = shuffle(keys, r);
    const shuffled2 = shuffle(keys, r);
    const shuffled3 = shuffle(keys, r);
    const result1 = mapToObj(resolveAll(shuffled1, evalNode));
    const result2 = mapToObj(resolveAll(shuffled2, evalNode));
    const result3 = mapToObj(resolveAll(shuffled3, evalNode));

    expect(result2).toEqual(result1);
    expect(result3).toEqual(result1);
  });

  it("random cycles always terminate with cycle errors", () => {
    // Functional graph: every node has exactly one outgoing edge to a random
    // node (including itself). A total function over a finite domain always
    // reaches a repeated state when iterated, so every node's dependency
    // chain is guaranteed to reach a cycle; evalNode forwards `get`'s error
    // unchanged, so every key resolves to the same {error:"cycle"} value.
    // Termination itself is guarded by the test/suite timeout, not an assertion.
    const r = rng(4);
    const n = 15;
    const deps: number[] = [];
    for (let i = 0; i < n; i++) deps.push(Math.floor(r() * n));
    const keys = Array.from({ length: n }, (_, i) => String(i));
    const evalNode = (key: string, get: (dep: string) => FormulaValue): FormulaValue => {
      const dep = deps[Number(key)];
      const v = get(String(dep));
      if (isFormulaError(v)) return v;
      return v + 1;
    };

    const result = resolveAll(keys, evalNode);
    expect(result.size).toBe(n);
    for (const key of keys) {
      const v = result.get(key)!;
      expect(isFormulaError(v)).toBe(true);
      expect((v as FormulaError).error).toBe("cycle");
    }
  });
});

function randomExpr(r: () => number, depth: number): string {
  if (depth > 4 || r() < 0.3) return r() < 0.5 ? String(Math.floor(r() * 20)) : "x";
  const ops = ["+", "-", "*", "/", "%"];
  const a = randomExpr(r, depth + 1);
  const b = randomExpr(r, depth + 1);
  if (r() < 0.2) return `floor(${a})`;
  if (r() < 0.2) return `min(${a}, ${b})`;
  return `(${a} ${ops[Math.floor(r() * ops.length)]} ${b})`;
}
