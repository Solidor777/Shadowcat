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
- The three-stage pipeline, in order: the `lexer` module (`tokenize` → `Tok`/`Op`) → the `parser`
  module (`parseFormula` → the `Expr` AST) → the `evaluate` module (`evaluate`). Each stage imports
  the one before it, so this is a real data flow.
- Two SIBLING entry points over the same value types — NOT later stages of that pipeline: the
  `graph` module (`resolveAll`) and the `template` module (`resolveNotationTemplate`,
  `NOTATION_KEYWORDS`). Each imports the `types` and `internal` modules and nothing else in this
  package, and each is driven by a consumer callback. Neither can call the pipeline, so wiring a
  graph node's or a template identifier's text through `parseFormula`/`evaluate` is the consumer's
  own callback body, never something this package does on its way through.
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

This package has no design document of its own, so an invariant here cites the architecture
document or a memory slug where one governs it, and otherwise names the TEST that pins it. A test
is the stronger referent for a library contract anyway: it fails when the invariant stops holding,
which no prose can do.

- **Error-value-only, fail-closed.** No function in this package throws on ANY input, and
  arithmetic never leaks `NaN`/`Infinity` — both become a `FormulaError` via `finite`. A consumer
  callback (`evaluate.resolve`, `resolveAll.evalNode`) IS allowed to throw or return a malformed
  value; `validateResolverOutput` converts that into a `"resolver-error"` rather than propagating
  it. `FormulaErrorKind` is mirrored by hand in `FORMULA_ERROR_KINDS` for runtime validation —
  adding a kind means updating BOTH, and the enforcement is ONE-DIRECTIONAL. The array's
  satisfies clause makes an entry outside the union a compile error; the reverse — a kind added
  to the union and omitted from the array — compiles clean and silently narrows what
  `isWellFormedError` accepts, so a consumer returning that kind gets it rewritten to
  `"resolver-error"`. That is the direction to check by hand. The
  callback half is [[injected-callback-boundary-must-validate-every-site]]; the no-throw half is
  pinned by `property.test`'s never-throws and never-NaN properties over random input.
- **DoS caps, exact values:** `MAX_FORMULA_LENGTH` 512, `MAX_AST_NODES` 256, `MAX_PARSE_DEPTH` 32
  (true structural-nesting boundaries — parens, call arguments, unary minus — NOT
  grammar-production depth, so a flat `a+b+c` chain never trips it), `MAX_GRAPH_VISITS` 2048
  (charged once per newly discovered key in `resolveAll`). No external record fixes any of them, so
  the tests are the source — but FOUR separate cases pin them, one per cap, and no single file
  stands for the rest. `types.test` asserts `MAX_FORMULA_LENGTH`'s value directly. The other three
  are pinned behaviourally, by a boundary case that spells its size as a LITERAL rather than
  deriving it from the constant: `parser.test` for `MAX_AST_NODES` and for `MAX_PARSE_DEPTH` (the
  exact size parses, one more caps — three constructs for the depth cap), and `graph.test` for
  `MAX_GRAPH_VISITS` (a chain of exactly that many distinct keys resolves, one key more caps, which
  also pins the bound as EXCEEDS rather than reaches). A bracket derived from the constant pins
  nothing: `graph.test`'s 2000-key and 5000-key chains admit every value between them.
- **`resolveAll`'s trampoline is O(1) JS-stack-depth by construction, not an implementation
  detail.** It restarts `resolveAll.evalNode` from scratch on an internal `NeedsDependency` throw
  rather than recursing, so graph depth never grows the call stack. Pinned by `graph.test`'s
  deep-chain case, which resolves a 2000-long chain and records the depth at which a recursive
  traversal of the same graph dies on a constrained stack. Motivation, not the constraint itself:
  the client must run on mobile browsers (`docs/design/ARCHITECTURE.md` §2 invariant 10), which
  requires that support but states no call-stack bound of its own. **A consumer's
  `resolveAll.evalNode` body must NEVER wrap its own call(s) to the injected getter in
  try/catch**: that swallows the signal driving the trampoline and silently
  memoizes a wrong, partial result. The `evaluate` module's own reference-case try/catch guards a
  DIFFERENT concern (turning a malformed resolver return into a `FormulaError`) and must not be
  reused to catch the trampoline signal — prefetch every reference path unwrapped before calling
  `evaluate` instead.
- **`resolveAll` is a pure function of the key set.** Sorted-root traversal means the same set of
  requested keys always produces the same result regardless of call or iteration order; traversing
  in the caller's key order instead makes the result order-dependent. Cycle-error detail names the
  lexicographically smallest cycle member, so two logically-identical graphs built in different key
  orders report byte-identical detail. Pinned by `graph.test`'s order-independence cases and
  `property.test`'s random-DAG property; no external record states it.
- **Zero game-system vocabulary in this package.** A change that introduces one consumer's concepts
  into `src/client/formula/` is a layering violation: it belongs in that consumer's own package.
  (`docs/design/ARCHITECTURE.md` §2 invariant 7, the framework-neutral public API, and invariant 6,
  which makes the opaque band the game system's own territory rather than the engine's.)
- **The grammar has no exponent notation.** `1e999` lexes as a number followed by a word — a parse
  error, not a cap error. A deliberate grammar boundary, not a lexer defect; do not "fix" the lexer
  to accept exponents without a grammar change. Pinned by `parser.test`'s exponent-notation case;
  the boundary is this package's own decision and no external record states it.
- **Identifiers are case-insensitive and normalized to lowercase, and the two grammars this package
  parses reserve DIFFERENTLY.** `parseFormula`'s grammar reserves no identifier names: a bare word
  is always a reference, whatever it spells, so reserved-word validation there is purely a
  consumer's concern. `resolveNotationTemplate`'s grammar reserves MORE than
  `NOTATION_KEYWORDS` lists. `readAlphaPrefix` reads the maximal LEADING alpha run, lowercases it,
  and tests THAT for membership, so the reserved set is every identifier whose leading alpha run,
  lowercased, is a member — which reaches two shapes beyond the literal list. A COMPOUND identifier
  collides whenever its alpha run stops early, i.e. a keyword followed by a digit: the keyword
  branch takes it and the remainder re-lexes as notation atoms (the same mechanism that lexes a
  dice atom, so it cannot be narrowed away). And matching is CASE-INSENSITIVE, so an upper- or
  mixed-case spelling collides as well as the lowercase one the list is written in. Any collision
  emits dice notation and never reaches the consumer's identifier resolver, so a stat key spelled
  `NOTATION_KEYWORDS.t` or `NOTATION_KEYWORDS.e` — or either of the two shapes above — is rewritten
  into a threshold or explode operator and the roll uses the operator, with no error on any path.
  That asymmetry is why the list is exported at all: it is what a consuming system's stat-key
  authoring validation rejects against, and that validation must reject the DERIVED set, not list
  membership alone. All three shapes are pinned by cases in `template.test` that derive from the
  list, so a keyword added there is covered without a second edit.

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
  FORMULA grammar reserves nothing (the notation-template grammar's separate reservation is the
  invariant above), so a consumer that skips a collision check gets an identifier resolving to a
  call instead of a reference. **The two name sets a consumer must reject against are NOT
  equally reachable**, and the asymmetry is the trap: `NOTATION_KEYWORDS` is exported from the
  `template` module, while `FN_NAMES` and its `FnName` mirror are module-private to `parser` and
  the barrel re-exports nothing it does not export. A consumer therefore cannot import the
  builtin-function names and has to mirror them. The RUNTIME value set is what is unimportable —
  the mirror is not unbindable: `Expr` is exported and its call arm declares `fn` as the same
  closed literal union, so a consumer can extract that union from the exported AST type and bind
  its own list to it at compile time in both directions (constrain the list to the union, and
  assert the union minus the list is empty). That is the drift guard a consumer-side duplicate
  needs — not a hand-maintained copy — and it needs no change to this package's public API. Do not
  export `FN_NAMES` to remove the fork without a ruling: that IS a public-API change.

## Pointers

- **Generated API** — `/api/ts/modules/_shadowcat_formula.html` (TypeDoc). Produce with
  `pnpm build:all`.
- Relationships: `graphify query "formula lexer parser evaluate graph resolver trampoline"`.
- `shadowcat-codebase-sheets` — the sheet registry a formula-driven system registers into.
- `shadowcat-codebase-dice` — the server-side dice engine; `resolveNotationTemplate` produces the
  notation string that engine then executes, and the two owe each other nothing else.
