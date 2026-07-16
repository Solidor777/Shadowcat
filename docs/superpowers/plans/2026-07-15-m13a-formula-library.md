# M13a · `@shadowcat/formula` — Shared Formula Library Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A framework-neutral, dependency-free TS expression library — lexer, parser, evaluator with injected resolver, generic cycle-guarded graph resolution, and a dice-notation-template mode — per spec §3 (`docs/superpowers/specs/2026-07-15-m13-nightfox-system-design.md`).

**Architecture:** Pure functions over an `Expr` AST. All failures are marked **error values** (never throws, never NaN leakage). References are opaque dotted paths resolved by an injected callback — the library contains ZERO Nightfox concepts (no stat types, no buckets, no `parent`/`base` vocabulary). Consumers (Nightfox first) define what names mean; other systems may replace this library entirely.

**Tech Stack:** TypeScript, Vitest. **No runtime dependencies** (not even zod). No Svelte in the dependency closure.

## Global Constraints

- New package at `src/client/formula/`, name `@shadowcat/formula` (pnpm workspace glob `src/client/*` picks it up automatically — no workspace-file change).
- Zero runtime deps; devDeps limited to what `@shadowcat/core` already uses (`@types/node`).
- Every failure path returns a `FormulaError` value; library code never throws on any input and never returns `NaN`/`Infinity`.
- Caps (exact values, spec §3.2): `MAX_FORMULA_LENGTH = 512`, `MAX_AST_NODES = 256`, `MAX_PARSE_DEPTH = 32`, `MAX_GRAPH_VISITS = 2048`.
- Identifiers are matched case-insensitively and normalized to lowercase; the library reserves NO identifier names (spec §3.1 — reserved-word policy is Nightfox tier-1 validation, M13b).
- `/` is float division; `%` is JS truncated remainder; no implicit rounding anywhere; any non-finite arithmetic result is an error value.
- Comments follow project rules (present-tense constraints; no history/process meta).
- Property/fuzz tests use a hand-rolled seeded PRNG in the test file — do NOT add fast-check or any new dependency.
- Commit per task once green; run `pnpm --filter @shadowcat/formula test` + `typecheck` before each commit.

---

### Task 1: Package scaffold + core types

**Files:**
- Create: `src/client/formula/package.json`
- Create: `src/client/formula/tsconfig.json` (copy `src/client/core/tsconfig.json` verbatim)
- Create: `src/client/formula/src/types.ts`
- Create: `src/client/formula/src/index.ts`
- Test: `src/client/formula/src/types.test.ts`

**Interfaces:**
- Produces: `FormulaError { error: FormulaErrorKind; detail: string }`, `FormulaValue = number | FormulaError`, `isFormulaError(v)`, cap constants. Every later task imports from `./types`.

- [ ] **Step 1: Write package.json** (mirrors `@shadowcat/core`'s shape, no deps):

```json
{
  "name": "@shadowcat/formula",
  "version": "0.0.0",
  "private": true,
  "type": "module",
  "main": "src/index.ts",
  "devDependencies": {
    "@types/node": "^22.0.0"
  },
  "scripts": {
    "typecheck": "tsc --noEmit",
    "test": "vitest run"
  }
}
```

- [ ] **Step 2: Copy tsconfig** from `src/client/core/tsconfig.json` unchanged, then run `pnpm install` at repo root (links the new workspace package). Expected: lockfile gains `@shadowcat/formula`.

- [ ] **Step 3: Write the failing test** (`src/client/formula/src/types.test.ts`):

```ts
import { describe, expect, it } from "vitest";
import { isFormulaError, MAX_FORMULA_LENGTH, type FormulaValue } from "./types";

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
});
```

- [ ] **Step 4: Run to verify it fails** — `pnpm --filter @shadowcat/formula test`. Expected: FAIL (module `./types` not found).

- [ ] **Step 5: Implement `src/types.ts`:**

```ts
/** Marked failure value. INVARIANT: library functions return these — they never throw
 * and never emit NaN/Infinity (spec §3.2 fail-closed error values). */
export type FormulaErrorKind =
  | "parse"        // source text does not lex/parse
  | "unknown-ref"  // resolver had no value for a reference (consumer-originated)
  | "type"         // non-numeric operand (consumer-originated, e.g. text stat)
  | "div-zero"     // x / 0 or x % 0
  | "non-finite"   // arithmetic overflowed to Infinity/NaN
  | "cycle"        // reference cycle in graph resolution
  | "cap"          // a DoS bound tripped
  | "ref-error";   // referenced value was itself an error (propagation wrapper)

export interface FormulaError {
  readonly error: FormulaErrorKind;
  /** Player-presentable, e.g. "unexpected '?' at position 4". Never internal dumps. */
  readonly detail: string;
}

export type FormulaValue = number | FormulaError;

export function isFormulaError(v: FormulaValue): v is FormulaError {
  return typeof v !== "number";
}

export const MAX_FORMULA_LENGTH = 512;
export const MAX_AST_NODES = 256;
export const MAX_PARSE_DEPTH = 32;
export const MAX_GRAPH_VISITS = 2048;
```

`src/index.ts` barrel: `export * from "./types";`

- [ ] **Step 6: Run tests (PASS), typecheck, commit** — `git add src/client/formula pnpm-lock.yaml && git commit -m "feat(formula): scaffold @shadowcat/formula package + error-value model"`

---

### Task 2: Lexer

**Files:**
- Create: `src/client/formula/src/lexer.ts`
- Test: `src/client/formula/src/lexer.test.ts`

**Interfaces:**
- Produces: `tokenize(src: string): Tok[] | FormulaError` with
  `Tok = { kind: "num", value: number, pos: number } | { kind: "word", value: string, pos: number } | { kind: "op", value: "+"|"-"|"*"|"/"|"%"|"("|")"|","|".", pos: number }`.
  Words are lowercased. Enforces `MAX_FORMULA_LENGTH`.

- [ ] **Step 1: Write the failing tests:**

```ts
import { describe, expect, it } from "vitest";
import { tokenize } from "./lexer";
import { isFormulaError } from "./types";

describe("tokenize", () => {
  it("lexes numbers, words, dots, and operators", () => {
    expect(tokenize("floor(Parent.dex / 2) + 1.5")).toEqual([
      { kind: "word", value: "floor", pos: 0 },
      { kind: "op", value: "(", pos: 5 },
      { kind: "word", value: "parent", pos: 6 },
      { kind: "op", value: ".", pos: 12 },
      { kind: "word", value: "dex", pos: 13 },
      { kind: "op", value: "/", pos: 17 },
      { kind: "num", value: 2, pos: 19 },
      { kind: "op", value: ")", pos: 20 },
      { kind: "op", value: "+", pos: 22 },
      { kind: "num", value: 1.5, pos: 24 },
    ]);
  });
  it("rejects unknown characters as a parse error value", () => {
    const r = tokenize("dex ? 2");
    expect(isFormulaError(r as never)).toBe(true);
  });
  it("rejects over-length sources (cap)", () => {
    const r = tokenize("1+".repeat(300) + "1");
    expect((r as { error: string }).error).toBe("cap");
  });
});
```

- [ ] **Step 2: Run — expect FAIL** (`./lexer` not found).

- [ ] **Step 3: Implement.** Single left-to-right scan: skip whitespace; `[0-9]` → number (digits, optional `.` + digits; a second `.` in one number is a parse error); `[a-zA-Z_]` → word `[a-zA-Z0-9_]*`, lowercased; single-char operators from the `op` set (`.` is an operator token — the parser assembles ref paths); anything else → `{ error: "parse", detail: "unexpected '<ch>' at position <pos>" }`. Length check first: `src.length > MAX_FORMULA_LENGTH` → `{ error: "cap", detail: "formula exceeds 512 characters" }`.

- [ ] **Step 4: Run tests (PASS). Step 5: Commit** — `feat(formula): lexer`

---

### Task 3: Parser → AST

**Files:**
- Create: `src/client/formula/src/parser.ts`
- Test: `src/client/formula/src/parser.test.ts`
- Modify: `src/client/formula/src/index.ts` (re-export)

**Interfaces:**
- Produces:

```ts
export type Expr =
  | { kind: "num"; value: number }
  | { kind: "ref"; path: string[] }   // e.g. ["parent","dex"] — opaque to the library
  | { kind: "neg"; operand: Expr }
  | { kind: "bin"; op: "+" | "-" | "*" | "/" | "%"; left: Expr; right: Expr }
  | { kind: "call"; fn: "min" | "max" | "floor" | "ceil" | "round"; args: Expr[] };
export function parseFormula(src: string): Expr | FormulaError;
```

- [ ] **Step 1: Write the failing tests** (representative; the implementer adds the sad-path table):

```ts
import { describe, expect, it } from "vitest";
import { parseFormula } from "./parser";

describe("parseFormula", () => {
  it("parses precedence: unary minus > mul > add", () => {
    expect(parseFormula("1 + 2 * -dex")).toEqual({
      kind: "bin", op: "+",
      left: { kind: "num", value: 1 },
      right: { kind: "bin", op: "*", left: { kind: "num", value: 2 },
               right: { kind: "neg", operand: { kind: "ref", path: ["dex"] } } },
    });
  });
  it("parses dotted refs and calls", () => {
    expect(parseFormula("min(hp.max, floor(parent.str / 2))")).toEqual({
      kind: "call", fn: "min", args: [
        { kind: "ref", path: ["hp", "max"] },
        { kind: "call", fn: "floor", args: [
          { kind: "bin", op: "/", left: { kind: "ref", path: ["parent", "str"] },
            right: { kind: "num", value: 2 } } ] },
      ],
    });
  });
  it("a word before '(' must be a known function", () => {
    expect(parseFormula("dex(1)")).toMatchObject({ error: "parse" });
  });
  it("enforces arity: floor/ceil/round take 1 arg, min/max take >= 1", () => {
    expect(parseFormula("floor(1, 2)")).toMatchObject({ error: "parse" });
    expect(parseFormula("min()")).toMatchObject({ error: "parse" });
  });
  it("caps AST size and nesting depth", () => {
    expect(parseFormula("1" + "+1".repeat(200))).toMatchObject({ error: "cap" });
    expect(parseFormula("(".repeat(40) + "1" + ")".repeat(40))).toMatchObject({ error: "cap" });
  });
  it("rejects trailing garbage", () => {
    expect(parseFormula("1 + 2 3")).toMatchObject({ error: "parse" });
  });
});
```

- [ ] **Step 2: Run — expect FAIL. Step 3: Implement** recursive descent over `tokenize` output: `additive := multiplicative (("+"|"-") multiplicative)*`; `multiplicative := unary (("*"|"/"|"%") unary)*`; `unary := "-" unary | primary`; `primary := num | "(" additive ")" | word ("(" args ")")? ("." word)*`. A word followed by `(` must be one of the five function names (else parse error); otherwise the word starts a ref path extended by `.` + word segments (a dotted segment is never a call). Track a node counter (`MAX_AST_NODES` → cap error) and recursion depth (`MAX_PARSE_DEPTH` → cap error). After `additive`, any remaining token is a parse error. The grammar has no exponent notation: `1e999` lexes as `num(1)` + `word("e999")` and fails as a trailing-input parse error, not a cap error.

- [ ] **Step 4: Run tests (PASS). Step 5: typecheck + commit** — `feat(formula): recursive-descent parser + Expr AST with caps`

---

### Task 4: Evaluator

**Files:**
- Create: `src/client/formula/src/evaluate.ts`
- Test: `src/client/formula/src/evaluate.test.ts`
- Modify: `src/client/formula/src/index.ts`

**Interfaces:**
- Consumes: `Expr`, `FormulaValue` (Tasks 1/3).
- Produces: `evaluate(expr: Expr, resolve: (path: string[]) => FormulaValue): FormulaValue`. Resolver errors propagate unchanged (spec §5.2 — a reference to an errored value poisons the expression). Booleans are the CONSUMER's job: resolvers return 1/0, never booleans.

- [ ] **Step 1: Write the failing tests:**

```ts
import { describe, expect, it } from "vitest";
import { parseFormula } from "./parser";
import { evaluate } from "./evaluate";
import type { Expr, FormulaValue } from "./types";

const ast = (src: string) => parseFormula(src) as Expr;
const env = (vals: Record<string, FormulaValue>) => (path: string[]): FormulaValue =>
  vals[path.join(".")] ?? { error: "unknown-ref", detail: `unknown reference '${path.join(".")}'` };

describe("evaluate", () => {
  it("computes arithmetic with resolver-supplied refs", () => {
    expect(evaluate(ast("floor(parent.str / 2) + dex"), env({ "parent.str": 15, dex: 3 }))).toBe(10);
  });
  it("float division, explicit rounding only", () => {
    expect(evaluate(ast("7 / 2"), env({}))).toBe(3.5);
    expect(evaluate(ast("round(7 / 2)"), env({}))).toBe(4);
  });
  it("division and mod by zero are error values", () => {
    expect(evaluate(ast("1 / dex"), env({ dex: 0 }))).toMatchObject({ error: "div-zero" });
    expect(evaluate(ast("1 % 0"), env({}))).toMatchObject({ error: "div-zero" });
  });
  it("propagates resolver errors unchanged", () => {
    const cyc: FormulaValue = { error: "cycle", detail: "dex -> str -> dex" };
    expect(evaluate(ast("dex + 1"), env({ dex: cyc }))).toEqual(cyc);
  });
  it("unknown refs are error values", () => {
    expect(evaluate(ast("ghost + 1"), env({}))).toMatchObject({ error: "unknown-ref" });
  });
  it("non-finite results are error values", () => {
    // Digit-run literals: the grammar has no exponent notation ("1e308" would lex as num(1)+word).
    const big = "1" + "0".repeat(160); // 1e160 as a pure digit run; product overflows f64
    expect(evaluate(ast(`${big} * ${big}`), env({}))).toMatchObject({ error: "non-finite" });
  });
  it("min/max n-ary", () => {
    expect(evaluate(ast("max(1, dex, 2)"), env({ dex: 9 }))).toBe(9);
  });
});
```

- [ ] **Step 2: Run — expect FAIL. Step 3: Implement** — structural recursion; left-to-right operand evaluation returning the FIRST error encountered; every `bin`/`neg`/`call` result passes a `Number.isFinite` gate (else `non-finite` error). Depth is already bounded by `MAX_PARSE_DEPTH`, so evaluation recursion needs no extra cap.

- [ ] **Step 4: Run tests (PASS). Step 5: Commit** — `feat(formula): evaluator with injected resolver + fail-closed error values`

---

### Task 5: Generic graph resolution (`resolveAll`)

**Files:**
- Create: `src/client/formula/src/graph.ts`
- Test: `src/client/formula/src/graph.test.ts`
- Modify: `src/client/formula/src/index.ts`

**Interfaces:**
- Produces:

```ts
/** Memoized lazy resolution over named nodes. Dependencies are discovered
 * dynamically: evalNode calls get(depKey) and cycles are detected via the
 * in-progress stack. Every node on a cycle resolves to {error:"cycle"}.
 * INVARIANT: result is independent of key iteration order (consumers rely on
 * this for the Nightfox permutation invariant, spec D3/D12). */
export function resolveAll(
  keys: readonly string[],
  evalNode: (key: string, get: (dep: string) => FormulaValue) => FormulaValue,
): Map<string, FormulaValue>;
```

- [ ] **Step 1: Write the failing tests:**

```ts
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
```

- [ ] **Step 2: Run — expect FAIL. Step 3: Implement** — memo `Map<string, FormulaValue>`, `visiting: Set<string>`, visit counter. `get(dep)`: memo hit → return; `visiting.has(dep)` → `{error:"cycle", detail:"reference cycle involving '<dep>'"}` (do NOT memoize the cycle error for `dep` here — let `dep`'s own evaluation produce and memoize its error so each participant independently reports); counter > `MAX_GRAPH_VISITS` → cap error; else push to `visiting`, call `evalNode`, memoize, pop. Iterative-safe recursion depth: JS stack depth = graph depth, bounded by `MAX_GRAPH_VISITS`; acceptable because visits cap at 2048 — document this bound.

- [ ] **Step 4: Run (PASS). Step 5: Commit** — `feat(formula): cycle-guarded memoized graph resolution`

---

### Task 6: Notation-template mode

**Files:**
- Create: `src/client/formula/src/template.ts`
- Test: `src/client/formula/src/template.test.ts`
- Modify: `src/client/formula/src/index.ts`

**Interfaces:**
- Produces:

```ts
/** Identifier words whose leading-alpha prefix means dice notation, not a stat.
 * Mirrors src/server/src/dice/notation/parser.rs keyword match (kh/kl/dh/dl/r/ro/cs/cf/t/e)
 * plus the 'd' dice operator. M13b's authoring validation imports this list. */
export const NOTATION_KEYWORDS: readonly string[] =
  ["d", "kh", "kl", "dh", "dl", "r", "ro", "cs", "cf", "t", "e"];
export function resolveNotationTemplate(
  src: string,
  resolve: (path: string[]) => FormulaValue,
): { notation: string } | FormulaError;
```

- [ ] **Step 1: Write the failing tests:**

```ts
import { describe, expect, it } from "vitest";
import { resolveNotationTemplate } from "./template";
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
      .toEqual({ notation: "d20 + (0 - 2)" });
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
});
```

- [ ] **Step 2: Run — expect FAIL. Step 3: Implement** slice-based rewriting: scan `src` left-to-right copying verbatim, EXCEPT: (a) `[` … `]` label spans copied verbatim (unterminated `[` → parse error); (b) at an alpha char, read the maximal `[a-z_]+` prefix P (case-insensitive): if `NOTATION_KEYWORDS.includes(P)` → keyword — copied through, with ONE normalization: a `d` keyword whose previous emitted token is not an integer gets `1` prefixed (`d20` → `1d20`; the server parser requires a count before `d`, but `d20+dex` is the canonical authoring form); digits after a keyword lex on their own; else extend through `[a-z0-9_]*` (+ optional `.`-joined segments, each segment same word rule) → identifier span: resolve; error → return it; `!Number.isInteger(v)` → `{error:"type", detail:"'<name>' = <v>: roll templates require integers (use floor/round in the stat formula)"}`; `Math.abs(v) > 2147483647` → cap error; `v < 0` → emit `(0 - ${-v})`; else emit `${v}[${originalText}]`. Length cap on `src` first (`MAX_FORMULA_LENGTH`).

- [ ] **Step 4: Run (PASS). Step 5: Commit** — `feat(formula): dice-notation template mode with notation-first lexing`

---

### Task 7: Property/fuzz + caps battery

**Files:**
- Test: `src/client/formula/src/property.test.ts`

**Interfaces:** Consumes everything; produces confidence only.

- [ ] **Step 1: Write the suite** (seeded SplitMix32-style PRNG in-file; no new deps). Four properties, each ≥ 500 iterations, fixed seed for determinism:

```ts
import { describe, expect, it } from "vitest";
import { parseFormula } from "./parser";
import { evaluate } from "./evaluate";
import { resolveAll } from "./graph";
import { resolveNotationTemplate } from "./template";
import { isFormulaError, type FormulaValue } from "./types";

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
      const src = randomExpr(r, 0); // generator below
      const ast = parseFormula(src);
      if (isFormulaError(ast as never)) continue;
      const v = evaluate(ast as never, () => Math.floor(r() * 10) - 3);
      if (!isFormulaError(v)) expect(Number.isFinite(v)).toBe(true);
    }
  });
  it("resolveAll is key-order independent (random DAGs)", () => {
    /* build a random 20-node DAG of add/mul nodes over the PRNG; resolve with
       3 different shuffles of the key list; expect deep-equal result maps */
  });
  it("random cycles always terminate with cycle errors", () => {
    /* random functional graph (each node depends on one random node) — guaranteed
       to contain a cycle reachable from some key; assert no hang (test timeout is
       the guard) and every key resolves to number | error */
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
```

The two sketched bodies (DAG order-independence, cycle termination) must be fully implemented — the comments above describe the exact construction; the implementer writes the ~15 lines each following them.

- [ ] **Step 2: Run — all green** (they test existing code; any failure is a real Task 2–6 bug — fix the source, not the property).
- [ ] **Step 3: Commit** — `test(formula): seeded property/fuzz battery`

---

### Task 8: Barrel, gates, docs, codebase skill

**Files:**
- Modify: `src/client/formula/src/index.ts` (final export surface: types, `parseFormula`, `evaluate`, `resolveAll`, `resolveNotationTemplate`, `NOTATION_KEYWORDS`, caps)
- Modify: `docs/PLAN.md` (M13 section: mark M13a done with a one-paragraph summary per house style)
- Create: `.claude/skills/shadowcat-codebase-nightfox/SKILL.md` (fixed shape: Purpose / Key files & seams / Hard invariants / Gotchas / Pointers; covers `@shadowcat/formula` now, extended by M13b/M13d)
- Modify: `.claude/hooks/codebase-skill-reminder.py` (add `src/client/formula/**` + `src/modules/nightfox*/**` globs to the SUBSYSTEMS map)

- [ ] **Step 1:** Finalize barrel; run the full gate: `pnpm -r typecheck && pnpm -r test && pnpm lint`. Expected: all green.
- [ ] **Step 2:** Write the codebase skill (link the spec; invariants: error-value model, zero-Nightfox-concepts boundary, caps, notation-first template lexing) and register its globs in the hook.
- [ ] **Step 3:** Update `docs/PLAN.md`. Per the reviewed skill-update gate, the skill diff is reviewed by `shadowcat-spec-reviewer` before the checkpoint closes.
- [ ] **Step 4:** Commit — `docs(m13a): PLAN sync + shadowcat-codebase-nightfox skill`

---

## Model/Effort directives

- Plan authored mainline on Fable 5, effort high (user directive 2026-07-15; tier-switch checkpoint outcome).
- Execution: **subagent-driven-development** — implementers `shadowcat-coder` (sonnet, `effort: medium`), reviewers `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` (`effort: high`), `-opus` twins on BLOCKED/shallow findings (project CLAUDE.md tiering).
- **Execution gated on M12 completion** (user decision 2026-07-15) — the M12 session shares this working tree; when execution starts, run on a per-checkpoint branch in a git worktree.

## Buddy-check directives

- **Pre-authorized task-level buddy-checks: Task 3 (parser), Task 4 (evaluator), Task 6 (notation-template)** — dense algorithmic cores (M11a precedent: every such buddy-check found real Criticals).
- Standard two-reviewer gates on all other tasks; customary whole-branch buddy-check before the checkpoint merge.
