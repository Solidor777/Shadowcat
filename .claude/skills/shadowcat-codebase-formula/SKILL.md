---
name: shadowcat-codebase-formula
description: "Use when touching `@shadowcat/formula` (src/client/formula/) — the framework-neutral expression library: the `lexer`/`parser`/`evaluate` pipeline, `resolveAll`'s cycle-guarded dependency-graph resolution and its restart trampoline, the dice-notation-template rewrite (`resolveNotationTemplate`/`NOTATION_KEYWORDS`), the `types` module's error kinds and DoS caps, or the `internal` module's consumer-callback trust boundary. Invoke shadowcat-codebase-core first; for the sheet layer that consumes formulas invoke shadowcat-codebase-sheets."
---

# Shadowcat — `@shadowcat/formula`

Orientation+index for Shadowcat's expression library. Points INTO graphify, `docs/design/`, and
memory rather than restating them.

## Purpose

`@shadowcat/formula` (`src/client/formula/`) is a pure-TS, zero-runtime-dependency expression
library: text → tokens → AST → number, plus generic cycle-guarded dependency-graph resolution and
a dice-notation-template rewrite mode. It carries **no game-system vocabulary** — no stats, no
modifier buckets, no document shape. References are opaque dotted paths resolved entirely by a
consumer-supplied callback, so any game system may use it, extend it, or replace it. **No Svelte
in its dependency closure**, so it is usable from headless contexts, not just the client. It is
one of the shell's `RUNTIME_ENTRIES`, so a consuming module gets the same single instance the
engine holds.

## Key files & seams

- The `types` module — `FormulaError`/`FormulaErrorKind`/`FormulaValue`, `isFormulaError`, and the
  four cap constants. Everything else imports from here.
- The five-stage pipeline, in order: the `lexer` module (`tokenize` → `Tok`/`Op`) → the `parser`
  module (`parseFormula` → the `Expr` AST) → the `evaluate` module (`evaluate`) → the `graph`
  module (`resolveAll`) → the `template` module (`resolveNotationTemplate`, `NOTATION_KEYWORDS`).
- The `internal` module — the shared trust-boundary helpers `isWellFormedError`,
  `validateResolverOutput` and `finite`. **Deliberately not re-exported from the `index` module**:
  every injected-callback boundary (the `evaluate` module's reference case, `resolveAll`'s call to
  `resolveAll.evalNode`, the `template` module's identifier resolver) validates a consumer
  callback's return through these before trusting it as a `FormulaValue`.
- The `index` module — the only public entry point, re-exporting `types`, `parser`, `evaluate`,
  `graph` and `template`.

**Arithmetic semantics that surprise formula AUTHORS** (the `evaluate` and `lexer` modules): `/` is
float division and `%` is JS TRUNCATED remainder, so `-7 % 2` is `-1`, not the floored `1`; neither
implicitly rounds, so a value requiring an integer needs an explicit `FnName.floor`/`FnName.round`,
and `FnName.round` is JS-native, meaning ties go toward positive infinity rather than away from
zero — a real difference for negative operands. Both `x / 0` and `x % 0` are a `"div-zero"` error,
never `Infinity`/`NaN`; every arithmetic result is gated through `finite`, so an overflow surfaces
as `"non-finite"` instead of leaking downstream. **A leading-dot decimal is not a numeric
literal** — `tokenize` requires a leading digit and emits a bare `.` operator instead, so `.5` is a
parse error; write `0.5`. And `checkArity` runs at PARSE time only, so an `Expr` hand-constructed
against the public API bypasses arity checking entirely and degrades through `finite` rather than
erroring cleanly — build expressions with `parseFormula`, never by hand.

## Hard invariants

- **Error-value-only, fail-closed.** No function in this package throws on ANY input, and
  arithmetic never leaks `NaN`/`Infinity` — both become a `FormulaError` via `finite`. A consumer
  callback (`evaluate.resolve`, `resolveAll.evalNode`) IS allowed to throw or return a malformed
  value; `validateResolverOutput` converts that into a `"resolver-error"` rather than propagating
  it. `FormulaErrorKind` is mirrored by hand in `FORMULA_ERROR_KINDS` for runtime validation —
  adding a kind means updating BOTH, with nothing else enforcing they stay in sync.
- **DoS caps, exact values:** `MAX_FORMULA_LENGTH` 512, `MAX_AST_NODES` 256, `MAX_PARSE_DEPTH` 32
  (true structural-nesting boundaries — parens, call arguments, unary minus — NOT
  grammar-production depth, so a flat `a+b+c` chain never trips it), `MAX_GRAPH_VISITS` 2048
  (charged once per newly discovered key in `resolveAll`).
- **`resolveAll`'s trampoline is O(1) JS-stack-depth by construction, not an implementation
  detail.** It restarts `resolveAll.evalNode` from scratch on an internal `NeedsDependency` throw
  rather than recursing, so graph depth never grows the call stack — required for
  constrained-stack mobile engines (`docs/design/ARCHITECTURE.md` §2, the cross-platform
  invariant). **A consumer's `resolveAll.evalNode` body must NEVER wrap its own call(s) to the
  injected getter in try/catch**: that swallows the signal driving the trampoline and silently
  memoizes a wrong, partial result. The `evaluate` module's own reference-case try/catch guards a
  DIFFERENT concern (turning a malformed resolver return into a `FormulaError`) and must not be
  reused to catch the trampoline signal — prefetch every reference path unwrapped before calling
  `evaluate` instead.
- **`resolveAll` is a pure function of the key set.** Sorted-root traversal means the same set of
  requested keys always produces the same result regardless of call or iteration order; traversing
  in the caller's key order instead makes the result order-dependent. Cycle-error detail names the
  lexicographically smallest cycle member, so two logically-identical graphs built in different key
  orders report byte-identical detail.
- **Zero game-system vocabulary in this package.** A change that introduces one consumer's concepts
  into `src/client/formula/` is a layering violation: it belongs in that consumer's own package.
- **The grammar has no exponent notation.** `1e999` lexes as a number followed by a word — a parse
  error, not a cap error. A deliberate grammar boundary, not a lexer defect; do not "fix" the lexer
  to accept exponents without a grammar change.
- **Identifiers are case-insensitive, normalized to lowercase, and the library reserves no
  identifier names.** Reserved-word validation is a consumer's concern, and every consumer-facing
  guard belongs in the consumer.

## Gotchas

- The `internal` module's three helpers are the ONLY sanctioned way to cross a consumer-callback
  boundary. A gap at one boundary reopens the class of bug the others already guard against
  [[injected-callback-boundary-must-validate-every-site]] — treat any NEW injected-callback seam as
  needing the same validation, never a bespoke check.
- Arithmetic semantics (`/`, `%`, rounding, `finite` gating, the leading-dot decimal) are stated
  ONCE, under **Key files & seams** above, so two copies cannot drift apart.
- `property.test` uses a hand-rolled seeded PRNG — do not add `fast-check` or any other new
  dependency to this package.
- **A consumer that reuses the library's function names as data keys collides silently.** The
  library reserves nothing, so a consumer that skips a collision check gets an identifier resolving
  to a call instead of a reference. **The two name sets a consumer must reject against are NOT
  equally reachable**, and the asymmetry is the trap: `NOTATION_KEYWORDS` is exported from the
  `template` module, while `FN_NAMES` and its `FnName` mirror are module-private to `parser` and
  the barrel re-exports nothing it does not export. A consumer therefore cannot import the
  builtin-function names and has to mirror them — which is a forked decision by construction, since
  the copy has nothing binding it to the original. Treat any consumer-side duplicate as needing its
  own drift guard, and do not export `FN_NAMES` to remove the fork without a ruling: that is a
  public-API change.

## Pointers

- **Generated API** — `/api/ts/modules/_shadowcat_formula.html` (TypeDoc). Produce with
  `pnpm build:all`.
- Relationships: `graphify query "formula lexer parser evaluate graph resolver trampoline"`.
- `shadowcat-codebase-sheets` — the sheet registry a formula-driven system registers into.
- `shadowcat-codebase-dice` — the server-side dice engine; `resolveNotationTemplate` produces the
  notation string that engine then executes, and the two owe each other nothing else.
