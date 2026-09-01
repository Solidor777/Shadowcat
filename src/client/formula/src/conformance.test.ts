import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { evaluate } from "./evaluate";
import { resolveAll } from "./graph";
import { parseFormula, type Expr } from "./parser";
import { resolveNotationTemplate } from "./template";
import type { FormulaError, FormulaValue } from "./types";

/** Every dotted ref in `expr`, in source order, first occurrence only. A graph node must
 * fetch its dependencies through `get` OUTSIDE `evaluate`: `evaluate` guards its resolver
 * callback with a try/catch, which would swallow `resolveAll`'s internal restart signal. */
function collectRefs(expr: Expr, out: string[] = []): string[] {
  switch (expr.kind) {
    case "num":
      break;
    case "ref": {
      const key = expr.path.join(".");
      if (!out.includes(key)) out.push(key);
      break;
    }
    case "neg":
      collectRefs(expr.operand, out);
      break;
    case "bin":
      collectRefs(expr.left, out);
      collectRefs(expr.right, out);
      break;
    case "call":
      for (const a of expr.args) collectRefs(a, out);
      break;
  }
  return out;
}

/** One expression case: `refs` values are numbers or errors; a missing ref is `unknown-ref`. */
interface ExpressionCase {
  /** Unique case name, reported on failure. */
  name: string;
  /** Formula source text. */
  source: string;
  /** Resolver environment keyed by dotted path. */
  refs?: Record<string, FormulaValue>;
  /** The expected value or error. */
  expect: FormulaValue;
}
/** One graph case: every node is a formula source resolved through `resolveAll`. */
interface GraphCase {
  /** Unique case name. */
  name: string;
  /** Node key → formula source. */
  nodes: Record<string, string>;
  /** The keys handed to `resolveAll`. */
  roots: string[];
  /** Expected value per key (a subset of the result map). */
  expect: Record<string, FormulaValue>;
}
/** One template case: `bindings` values are numbers or errors; a path absent from the map
 * resolves as `unknown-ref`. */
interface TemplateCase {
  /** Unique case name, reported on failure. */
  name: string;
  /** Notation-template source text. */
  src: string;
  /** Stub-resolver environment keyed by the RAW-CASE dotted path (this grammar does not
   * lowercase what it offers the resolver). */
  bindings?: Record<string, FormulaValue>;
  /** The rewritten notation, or the error the scan ends in. */
  expect: { notation: string } | FormulaError;
}
/** The corpus file's shape. */
interface Corpus {
  /** Expression cases. */
  expressions: ExpressionCase[];
  /** Graph cases. */
  graphs: GraphCase[];
  /** Notation-template cases. */
  templates: TemplateCase[];
}

const corpus: Corpus = JSON.parse(
  readFileSync(new URL("./__fixtures__/conformance.json", import.meta.url), "utf8"),
);

const unknownRef = (key: string): FormulaError => ({
  error: "unknown-ref",
  detail: `unknown reference '${key}'`,
});

describe("conformance corpus (shared with the server twin)", () => {
  it("has unique case names", () => {
    const names = [...corpus.expressions, ...corpus.graphs, ...corpus.templates].map(
      (c) => c.name,
    );
    expect(new Set(names).size).toBe(names.length);
  });

  for (const c of corpus.expressions) {
    it(`expression: ${c.name}`, () => {
      const ast = parseFormula(c.source);
      if ("error" in ast) {
        expect(ast).toEqual(c.expect);
        return;
      }
      const refs = c.refs ?? {};
      const resolve = (path: string[]): FormulaValue => {
        const key = path.join(".");
        return key in refs ? refs[key] : unknownRef(key);
      };
      expect(evaluate(ast, resolve)).toEqual(c.expect);
    });
  }

  for (const g of corpus.graphs) {
    it(`graph: ${g.name}`, () => {
      const evalNode = (key: string, get: (dep: string) => FormulaValue): FormulaValue => {
        const source = g.nodes[key];
        if (source === undefined) return unknownRef(key);
        const ast = parseFormula(source);
        if ("error" in ast) return ast;
        const fetched = new Map<string, FormulaValue>();
        for (const dep of collectRefs(ast)) fetched.set(dep, get(dep));
        return evaluate(ast, (path) => fetched.get(path.join(".")) ?? unknownRef(path.join(".")));
      };
      const result = resolveAll(g.roots, evalNode);
      for (const [key, expected] of Object.entries(g.expect)) {
        expect(result.get(key), key).toEqual(expected);
      }
    });
  }

  for (const t of corpus.templates) {
    it(`template: ${t.name}`, () => {
      const bindings = t.bindings ?? {};
      const resolve = (path: string[]): FormulaValue => {
        const key = path.join(".");
        return key in bindings ? bindings[key] : unknownRef(key);
      };
      expect(resolveNotationTemplate(t.src, resolve)).toEqual(t.expect);
    });
  }
});
