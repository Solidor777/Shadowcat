# M14c-1 · Server Formula Engine + Invariant 6 — Design

**Status:** Approved (brainstorm 2026-08-30). First of six M14c sub-projects (§1). Amends the
[M14 design](2026-08-28-m14-combat-tracker-design.md) D4 and the
[M14b design](2026-08-28-m14b-combat-clock-design.md) B5 — where they disagree with this spec,
this spec wins; each carries a pointer.

## 0. The correction this spec exists for

ARCHITECTURE §2 invariant 6 ("opaque `system` band; the server never interprets it") was read as
"the server never evaluates a formula". M14a/b were built on that reading: `Formula::Text` is
stored verbatim, the client's formula library resolves it and writes numbers, and the server
skips any effect or resource whose number is missing. The user overturned the reading on
2026-08-30:

> "I can understand why the engine never interprets the system band, but not why the server
> never should." … "The system is normally assumed to run on the server." … "By default,
> everything should run on the server. The client just makes requests of the server, and the
> server answers."

The invariant's **intent** is that system *semantics* — what `speed` means, how modifiers stack
— are defined by system modules, not the engine. Evaluating an engine-owned grammar over numeric
`system` leaves a formula names touches no semantics; the dice engine already has that standing.
Client-side resolution created a "GM's browser must be open for the clock to work" dependency
and a multi-writer race for nothing.

A whole-codebase audit (server, client, tests, docs/skills/memory — 156 findings, consolidated
in Appendix A) found the same misreading in five subsystems. Nothing is deferred; the work is
decomposed into the six sub-projects of §1, built in order.

## 1. Decomposition (the M14c umbrella)

| # | Sub-project | Contents |
|---|---|---|
| **M14c-1** (this spec) | Server formula engine + invariant 6 | Rust twin of `@shadowcat/formula`; shared conformance corpus; engine reference resolver over `system` leaves; `Formula::Text` grammar-checked at ingress; ARCHITECTURE/PLAN/HISTORY/guide/skill/memory rewrite. |
| M14c-2 | Combat resolution server-side | `Mirror` implemented; `Tracked` seeded to `max`; text recoveries evaluated; `Duration.remaining` and lifecycle flags server-derived; egress rule for resolved scalars; `Hard` route-preview clamp inside `Pathfind`. |
| M14c-3 | World-config authority | `create_world` seeds every config singleton; `system-defaults` read server-side from the installed system package; engine defaults become the definition (client constants mirror them); client seed/upsert helpers deleted. |
| M14c-4 | Dice references + chat channel | Reference production in the notation grammar; roll frames carry an actor binding; `resolveNotationTemplate` preview-only; `MessageEngine.channel` validated at ingest. |
| M14c-5 | Templates merge server-side | `MergePull`/`MergePush`/`MergeRevert` intents; conflict set returned for human review; `Document.base` under engine-tree validation. |
| M14c-6 | Combat client seams | `AppContext.combat`, `CoreHooks` first entries + delta-derived emission, `Warn` overage label. |

Each sub-project gets its own spec → plan → execute cycle. M14c-2 through -5 consume this
spec's evaluator; -6 consumes -2's final shapes.

## 2. Decisions

| # | Decision |
|---|---|
| F1 | **The server evaluates the engine's formula grammar.** A `formula` module in the server crate is an exact behavioural twin of `@shadowcat/formula`: same lexer, grammar, AST, evaluation rules, error kinds and `detail` strings, and caps. The TS package stays as the client's authoring/preview evaluator; neither is "the mirror" — both conform to one corpus (F3). |
| F2 | **Vocabulary-free.** Reference resolution is a trait; the evaluator assigns no meaning to a path (M13 D1/D13 preserved). The engine's default resolver (F4) reads a path literally from the `system` root. |
| F3 | **One conformance corpus, run by both suites.** A JSON fixture under the TS package holds expression and graph cases; the vitest suite and a Rust test both assert exact equality (value, or error kind + `detail`). Drift on either side fails a test. |
| F4 | **Default reference semantics = literal path into `system`.** `a.b.c` reads `/system/a/b/c`; a JSON number is the value; absent → `unknown-ref`; any other JSON type → `type`. No `stats` convention is baked in: a system that keeps variables under `system.stats` (M13 D13) references `stats.<key>.<leaf>` and persists the leaf it wants the engine to read (the directive: derived values are persisted leaves). |
| F5 | **`Formula::Text` must parse at ingress.** `Formula::validate` runs the server parser; a parse or cap error rejects the write like any malformed engine value. The storage-only `MAX_FORMULA_CHARS` is replaced by the shared `MAX_FORMULA_LENGTH` (512 UTF-16 code units, counted identically on both sides). |
| F6 | **Invariant 6 is rewritten to say what it means** (§6): the server never interprets what a `system` value *means* and runs no third-party code; it evaluates the engine's own grammar over `system` leaves a formula names and interprets declarative data it owns the grammar of (M13f's formulation). "System logic runs on the client, GM-authoritative" is retired: by default computation runs on the server; the client requests. |
| F7 | **The dice notation grammar is untouched here.** It is integer-only (`i64`), so its `floor/ceil/round` are identity and its "not required to agree with `@shadowcat/formula`" note is not a defect — the note is reworded to state the integer-domain reason. Formula references inside notation are M14c-4. |

## 3. The `formula` module

`src/server/src/formula/` — `pub mod formula` in `lib.rs`, beside `dice`. Five units mirroring
the TS package one-to-one so a reader can diff them:

| Unit | Twin of | Contents |
|---|---|---|
| `types` | `types.ts` | `FormulaErrorKind` (nine variants, serde `kebab-case` to the TS tags), `FormulaError { error, detail }`, `FormulaValue = Result<f64, FormulaError>`-shaped enum, the four caps `MAX_FORMULA_LENGTH = 512`, `MAX_AST_NODES = 256`, `MAX_PARSE_DEPTH = 32`, `MAX_GRAPH_VISITS = 2048`. |
| `lexer` | `lexer.ts` + `chars.ts` | `tokenize(&str) -> Result<Vec<Tok>, FormulaError>`. Length cap counted as `src.encode_utf16().count()` (the TS `.length`). Whitespace ` \t\n\r`. Numeric literal: ASCII digits, one optional `.` that must be followed by a digit (else `parse` "unexpected '.' at position N"); parsed value must be finite (else `cap` "numeric literal at position N is out of range"). Identifier: `[A-Za-z_][A-Za-z0-9_]*`, lowercased. Operators `+ - * / % ( ) , .`. Any other character → `parse` "unexpected '<char>' at position N", where `<char>` is the full code point and N the UTF-16 offset. |
| `parser` | `parser.ts` | `parse(&str) -> Result<Expr, FormulaError>`. Grammar: `additive := multiplicative (('+'\|'-') multiplicative)*`; `multiplicative := unary (('*'\|'/'\|'%') unary)*`; `unary := '-' unary \| primary`; `primary := num \| '(' additive ')' \| word ('(' args ')')? ('.' word)*`. A word before `(` must be `min\|max\|floor\|ceil\|round` (else `parse`); arity: `min`/`max` ≥ 1, others exactly 1, checked at parse time with the TS `detail` wording. Node cap charged once per constructed node (`cap` "formula exceeds 256 AST nodes"); depth cap advances only at parens, calls and unary minus (`cap` at 32). Trailing tokens are a `parse` error. `Expr = Num(f64) \| Ref(Vec<String>) \| Neg(Box) \| Bin { op, left, right } \| Call { fn, args }`. |
| `evaluate` | `evaluate.ts` + `internal.ts` | `evaluate(&Expr, &dyn Resolve) -> FormulaValue`. Left-to-right, first error wins. `/` float division; `%` truncated remainder (Rust `%` on `f64` already is); `x/0`, `x%0` → `div-zero` "division by zero ('/')"/"('%')"; every arithmetic result through `finite()` → `non-finite` "arithmetic result is not finite (<n>)" (the `<n>` rendered as JS would: `Infinity`, `-Infinity`, `NaN`). `min`/`max` over all args; `floor`/`ceil` standard; **`round` = JS `Math.round`** (ties toward +∞: `round(-2.5) = -2`), implemented explicitly, never `f64::round`. `Resolve::resolve(&self, path: &[String]) -> FormulaValue`; a resolver's return value passes through the same well-formedness gate as the TS `validateResolverOutput` (a non-finite number → `non-finite`; `resolver-error` is the TS-only case of a malformed non-number return, which the typed `Resolve` trait cannot produce). Rust resolvers cannot throw, so the TS "resolver threw" branch has no twin; the corpus never asserts it. |
| `graph` | `graph.ts` | `resolve_all(keys, eval_node) -> BTreeMap<String, FormulaValue>`: memoized, roots iterated in sorted order, cycle detection over the active path with the detail naming the lexicographically smallest cycle member ("reference cycle involving '<k>'"), visit cap charged once per newly discovered key (`cap` "graph resolution exceeded visit cap"). The TS restart trampoline exists to bound JS stack depth; the Rust twin achieves the same contract with an explicit heap stack and must produce identical results for every corpus case — the corpus, not the mechanism, is the contract. |

**Never panics.** Every failure is a `FormulaError` value. A proptest (precedent:
`dice/proptests.rs`) feeds random sources, random resolver outputs (numbers, non-finite numbers,
errors) and random graphs, asserting no panic and no non-finite success value.

## 4. The engine's default resolver

`formula::SystemLeafResolver<'a>(&'a Document)` implements `Resolve`:

- `path` → JSON pointer `/system/<seg0>/<seg1>/…` (segments are already lowercase identifiers,
  so no escaping is needed; a segment cannot contain `/` or `~`).
- Number leaf → `Ok(n)` (finite by JSON construction).
- Missing at any depth (including traversing through a non-object) → `unknown-ref` with detail
  `unknown reference 'a.b.c'` (the wording the TS package's own test resolver uses, and the
  wording the conformance corpus asserts for a missing ref).
- Present but not a number (string, bool, object, array, null) → `type` with detail
  `'a.b.c' is not a number`.

This is the resolver M14c-2 hands the combat transition for a combatant's actor document
(token-embedded copy or linked actor — that choice is -2's). Nothing else in -1 calls it;
-1 ships it with its own tests because it is the engine's one reference-semantics decision.

## 5. Ingress: `Formula`

`data::engine::combat::Formula::validate(at)`:

- `Number(n)`: finite (unchanged).
- `Text(t)`: `formula::parse(&t)` must succeed. A `parse` error rejects with
  `"{at}: {detail}"`; a `cap` error likewise. The empty-string check is subsumed (the parser
  reports "expected expression at position 0"). `MAX_FORMULA_CHARS` is deleted; the length cap is
  the parser's own `MAX_FORMULA_LENGTH`.

`ResourceRegistryEngine::validate` and `EffectEngine::validate` already route every `Formula`
through `validate`, so no new call sites. The client's authoring UI shows the identical error from
the TS parser before the write is sent (F3 guarantees the two agree).

## 6. Documentation

### 6.1 ARCHITECTURE §2 invariant 6 (replacement text for bullets 2–3)

- **`system`** — opaque, system-defined JSONB body. The server's authority over what a value
  *means* is nil: it performs no semantic or mechanical validation of system content and never
  interprets it. Its authority over *structure* is total (size caps, JSON validity,
  `deny_unknown_fields`, declared shape schemas, permissions). This is the band game-system
  modules own.
- **The server evaluates the engine's own grammars over `system` data a system names, and
  interprets declarative data it owns the grammar of** — the formula language
  (`@shadowcat/formula` and its server twin) over numeric `system` leaves a formula references,
  dice notation, the M13f schema registry. None of that is interpreting system semantics: the
  server reads `system.stats.speed.final` because a formula told it to, and knows nothing about
  what speed is. It never executes third-party code.
- **By default, computation runs on the server; the client requests and the server answers.**
  System semantics are defined by system modules, but the system is assumed to run against the
  server: a derived value a system wants the engine to act on (a resource ceiling, a duration, a
  lifecycle flag) is a persisted `system` leaf the server evaluates from, never a number a
  browser computed and wrote. Client-side computation is the exception and needs a reason —
  presentation, input capture, optimistic prediction. The cooperative-play trust model still
  governs *what a GM may author*; it no longer makes a GM's client the authority for anything
  the engine derives.

### 6.2 Other prose

- ARCHITECTURE §4 sandbox row: the "seam in place" column no longer cites "client-side
  GM-authoritative model"; it cites "engine-grammar evaluation is server-side and needs no
  sandbox; untrusted *code* execution remains deferred". §5 Deno bullet: the rationale becomes
  the single-binary and weak-sandbox points alone. §6 "Derived values are computed, never
  stored" is scoped to live template inheritance (its M2 meaning); a new sentence states the
  persisted-leaf rule. §6 four-tier chain: `resolveSettingProvenance` is named as a display
  mirror of the server resolver, not a co-equal resolver.
- PLAN.md: M14 section replaced with the §1 table; the Phase-3 parking line separates
  "sandboxed validators for untrusted code" (still parked) from "server-side formula
  evaluation" (delivered here).
- HISTORY.md: M13 exclusions and M14b's "server never evaluates" paragraphs gain a one-line
  superseding pointer to this spec (append-only convention; no rewriting of the record).
- M14 spec D4 and M14b spec B5/§4.2: "*Amended (M14c-1)*" pointers.
- `docs/site/guides/creating-a-system.md`: band table row and the "your client code is the only
  semantic authority" sentence rewritten per 6.1.
- Skills (plugin checkout, reviewed skill-update gate): `shadowcat-codebase-core` three-band
  bullet; `shadowcat-codebase-documents-permissions` purpose line; `shadowcat-codebase-formula`
  gains the server twin, the corpus and the `SystemLeafResolver`; `shadowcat-codebase-combat`'s
  "server never evaluates" hard invariant becomes "the server evaluates; consumer wiring lands in
  M14c-2 — until then transitions still skip unresolved values" (truthful interim state).
- Memory `server-mirrors-client-resolver-semantics`: method kept (verify against the source,
  fail closed, deterministic containers); premise inverted (the server is the definition; a
  client copy is preview).

## 7. Testing

- Unit suites per module, each case mirrored from the TS suite it twins.
- Conformance corpus (`src/client/formula/src/__fixtures__/conformance.json`): expression cases
  `{ source, refs: { "<a.b>": number | error }, expect }` and graph cases
  `{ nodes: { key: source }, roots, expect: { key: value } }`. Consumed by
  `src/client/formula/src/conformance.test.ts` and `src/server/src/formula/tests/conformance.rs`
  (relative path, the `move_clip/tests.rs` precedent). Seeded from every existing TS test case
  plus the Rust-specific edge cases (`round` ties, `%` sign, non-finite rendering, UTF-16 length,
  astral-character error message). Sabotage verified once each way (flip an expected value → both
  suites fail; restore → empty diff) and recorded in the plan, not kept.
- Proptest: never panics, never a non-finite success.
- `SystemLeafResolver`: number / missing at each depth / each non-number JSON type.
- Ingress: `Formula::Text("1 +")`, `"("`, a 513-unit string, an astral-character source →
  rejected with the parser's wording; `"speed * 2"`, `"max(stats.hp.final, 1)"` → accepted.
- Doc gates: `pnpm lint:docs`, `node scripts/check-skill-symbol-refs-cli.mjs`,
  `pnpm run test:scripts`.

## 8. Security

The evaluator is a DoS surface on the server now: the four caps bound lexing, AST size, parse
depth and graph work; `Formula::Text` is capped at ingress, so no stored formula can exceed them.
It reads unredacted documents — that is fine in -1 (nothing consumes its output yet) and is the
reason M14c-2 owns an egress rule for resolved scalars (a number derived from `gm_only` leaves
defaults to trusted-only disclosure, the whole-move-scalar rule).

## 9. Coordination

M15a is in flight on its own worktree (`m15a-asset-pipeline`). -1 touches no file M15a does
except the shared docs (`PLAN.md`, `HISTORY.md`, skills); those merge as text.

## Appendix A — audit findings, by group → sub-project

| Group | Root | Sub-project |
|---|---|---|
| 1 | Combat formula resolution client-side: `Formula` "never evaluates Text", `MAX_FORMULA_CHARS`, `CombatantResource` "client writes", `Mirror` unimplemented, `Tracked` never seeded, `Duration.remaining`/`EffectLifecycle.resolved` `Option` client-writes, `recover` no-op on text, `effects::tick`/`expire_by_policy` skip-on-`None`, movement gate over a browser-written ceiling, the M14 D4 / M14b B5 chain, combat SKILL hard invariant, tests `text_recoveries_apply_nothing_server_side` / `…skips_unresolved` / `…never_touched_by_a_boundary…`, fixtures with empty `system` bands, generated-type comments. | -1 (type/ingress/docs), -2 (behaviour) |
| 2 | `system-defaults` written by the GM's client: `Module.systemDefaults`, `systemDefaultsUpsertOps`, `WorldSession.#onWelcome`, `SystemDefaultsEngine` rustdoc, M14b B1/§3.3, client-shell SKILL. | -3 |
| 3 | Engine defaults declared client-authoritative: `DEFAULT_WORLD_SETTINGS` "client stays the authoritative source", `WorldSettingsEngine::default` "MUST equal the client's", `SEED_VISION_MODES`/`DEFAULT_GRADATION`, server test messages "mirrors client default", plans m10e-4/m10e-6/m13-0; `create_world` seeds no config documents; game-settings five-singleton seed; registry seeds; reset-to-default writes a client-resolved literal. | -3 |
| 4 | Dice notation references resolved client-side (`resolveNotationTemplate`, `checkNotationKey`), `CombatRoll.notation`/`SendMessage` carrying pre-substituted literals, `dice::Round` note; `MessageEngine.channel` unvalidated but selecting a `ParseContext`. | -4 (note: -1 F7) |
| 5 | Invariant 6 text and its propagation (ARCHITECTURE §2/§4/§5/§6, PLAN Phase 3, HISTORY M13/M14b, `creating-a-system.md`, core/documents-permissions/formula/combat skills, memory `server-mirrors-client-resolver-semantics`). | -1 |
| 6 | Templates 3-way merge client-only ("server never merges", M13e E10), `Document.base` opaque even for `engine`-shaped content. | -5 |

Legitimately client-side, confirmed by the audit and unchanged: footprints (already
server-resolved; the model for the rest), `resolveTokenActor`/`EffectiveActor` (presentation
projection), `canEdit` and the capability mirrors (advisory), sheet edit paths (authoring input),
`move_clip` vision-sample parity (rendering playback), display-only faction/condition registry
content, module-code non-execution, doc-link/speak-as labels.
