---
name: shadowcat-codebase-nightfox
description: "Use when touching `@shadowcat/formula` (the framework-neutral expression library: lexer/parser/evaluator, dependency-graph resolution, dice-notation-template mode) or the in-repo Nightfox surface it feeds (`src/modules/nightfox*`, once M13b+ land). Covers src/client/formula/. Invoke shadowcat-codebase-core first."
---

# Shadowcat — Nightfox / `@shadowcat/formula`

Orientation for the shared formula library underlying Nightfox (M13a shipped; M13b/M13d extend
this skill in-place rather than forking a new one). Spec:
`docs/superpowers/specs/2026-07-15-m13-nightfox-system-design.md` §3. Plan:
`docs/superpowers/plans/2026-07-15-m13a-formula-library.md`.

## Purpose

`@shadowcat/formula` (`src/client/formula/`) is a pure-TS, zero-runtime-dependency expression
library: text → tokens → AST → number, plus generic cycle-guarded dependency-graph resolution
and a dice-notation-template rewrite mode. It has **zero Nightfox concepts** — no stat types, no
modifier buckets, no `parent`/`base` vocabulary. References are opaque dotted paths (`hp.max`)
resolved entirely by a consumer-supplied callback; Nightfox (external repo, M13b+) is the first
consumer but any game system may use or replace this library. No Svelte in its dependency
closure — it is usable from server-side validators and other headless contexts, not just the
client.

## Key files & seams

- `src/types.ts` — `FormulaError`/`FormulaErrorKind`/`FormulaValue`, `isFormulaError`, the four
  cap constants. Everything else imports from here.
- `src/lexer.ts` → `src/parser.ts` (`parseFormula` → `Expr` AST) → `src/evaluate.ts`
  (`evaluate(expr, resolve)`) → `src/graph.ts` (`resolveAll(keys, evalNode)`) →
  `src/template.ts` (`resolveNotationTemplate`, `NOTATION_KEYWORDS`) — the five-stage pipeline in
  spec order.
- `src/internal.ts` — shared trust-boundary helpers (`isWellFormedError`, `validateResolverOutput`,
  `finite`). **Not re-exported from `index.ts`** — every injected-callback boundary (evaluate's
  `ref` case, graph's `evalNode` call, template's identifier resolver) validates a consumer
  callback's return value through these before trusting it as a `FormulaValue`.
- `src/index.ts` — the only public entry point: types + caps + `parseFormula` + `evaluate` +
  `resolveAll` + `resolveNotationTemplate` + `NOTATION_KEYWORDS`.

## Hard invariants

- **Error-value-only, fail-closed.** No function in this package ever throws on ANY input, and
  arithmetic never leaks `NaN`/`Infinity` — both become a `FormulaError` (`internal.ts`'s
  `finite`). A consumer callback (`resolve`/`evalNode`) IS allowed to throw or return a malformed
  value; the library's own boundary code (`validateResolverOutput`) converts that into a
  `"resolver-error"` rather than propagating it. `FormulaErrorKind` is mirrored by hand in
  `FORMULA_ERROR_KINDS` (types.ts) for runtime validation — adding a kind means updating BOTH the
  union and the array, with nothing else enforcing they stay in sync.
- **DoS caps, exact values (spec §3.2):** `MAX_FORMULA_LENGTH=512`, `MAX_AST_NODES=256`,
  `MAX_PARSE_DEPTH=32` (counts true structural-nesting boundaries — parens, call args,
  unary-minus — NOT grammar-production depth; a flat `a+b+c+...` chain never trips it),
  `MAX_GRAPH_VISITS=2048` (charged once per newly discovered key in `resolveAll`).
- **`resolveAll`'s trampoline is O(1) JS-stack-depth by construction, not an implementation
  detail.** It restarts `evalNode` from scratch on an internal `NeedsDependency` throw rather than
  recursing, so graph depth never grows the call stack — required for constrained-stack mobile
  engines (project cross-platform invariant). Consumer `evalNode` bodies must NEVER wrap their own
  call(s) to the injected `get` in try/catch — that would swallow the internal signal driving the
  trampoline and silently memoize a wrong result. Documented in `graph.ts`'s own JSDoc; treat any
  PR touching `resolveAll` or its consumers as needing that invariant re-verified.
- **Zero Nightfox vocabulary in this package.** If a change introduces a Nightfox-specific concept
  (stat, bucket, effect, etc.) into `src/client/formula/`, that is a layering violation — it
  belongs in the Nightfox repo (M13b+), not here.
- **The grammar has no exponent notation.** `1e999` lexes as `num(1)` followed by `word("e999")` —
  a parse error, not a cap error. This was a real spec-text bug found and fixed twice during
  planning; do not "fix" the lexer to accept exponents without a spec change.
- **Identifiers are case-insensitive, normalized to lowercase; the library reserves no identifier
  names** (reserved-word/tier-1 validation is Nightfox's concern, M13b) — every consumer-facing
  guard belongs in the consumer, not here.

## Gotchas

- `internal.ts`'s three helpers are the ONLY sanctioned way to cross a consumer-callback boundary.
  A prior task (buddy-check-caught) skipped this pattern at one boundary and reopened a bug
  already fixed twice elsewhere in the pipeline — treat any new injected-callback seam as
  needing the same validation, not a bespoke check.
- `/` is float division; `%` is JS truncated remainder; neither implicitly rounds.
- Property/fuzz tests (`property.test.ts`) use a hand-rolled seeded PRNG — do not add `fast-check`
  or any other new dependency to this package (Global Constraint, still binding for M13b/d work
  that extends it).

## Pointers

- Spec: `docs/superpowers/specs/2026-07-15-m13-nightfox-system-design.md` §3 (grammar, caps,
  error model, template mode).
- Plan (M13a, shipped): `docs/superpowers/plans/2026-07-15-m13a-formula-library.md`.
- `docs/PLAN.md` M13 section — milestone chain (M13-0 → M13-1 → **M13a** → M13b → M13c → M13d →
  M13e → M13f) and which repo (this one vs. the external Nightfox repo) owns each step.
- Once M13b/M13d land in-repo (`src/modules/nightfox*`), extend this skill's Key files/Gotchas
  sections in place rather than splitting — this skill is scoped to the whole Nightfox surface
  that lives in THIS repo, not just the formula library.
