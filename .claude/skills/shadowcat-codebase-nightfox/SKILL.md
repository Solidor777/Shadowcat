---
name: shadowcat-codebase-nightfox
description: "Use when touching `@shadowcat/formula` (the framework-neutral expression library: lexer/parser/evaluator, dependency-graph resolution, dice-notation-template mode), the Nightfox rules engine it feeds (the `nightfox-docs`/`contributions`/`resolve` modules), or the Nightfox sheets layer (`src/sheets/*` — the `sheet-model` module, StatRow/StatTable/ModifiersEditor, ActorSheet/ItemSheet/EffectSheet, the `nf-i18n` module). Two repos: the Shadowcat engine repo owns src/client/formula/, and the separate Nightfox module repo owns its own src/, nested for dev into a Shadowcat checkout at src/modules/nightfox/. Invoke shadowcat-codebase-core first."
---

# Shadowcat — Nightfox / `@shadowcat/formula`

Orientation for the shared formula library and the Nightfox rules engine consuming it. Two
repositories are in play and this skill ships into both, so every path below names its owner
explicitly: the **Shadowcat engine repo** owns the formula library, and the **Nightfox module
repo** owns the rules engine, sheets layer, and roll wire. This skill covers the whole surface
(library + rules engine + sheets + roll wire), extended in place rather than forked.

## Purpose

`@shadowcat/formula` (`src/client/formula/` **in the Shadowcat engine repo**) is a pure-TS,
zero-runtime-dependency expression library: text → tokens → AST → number, plus generic
cycle-guarded dependency-graph resolution and a dice-notation-template rewrite mode. It has
**zero Nightfox concepts** — no stat types, no modifier buckets, no `parent`/`base` vocabulary.
References are opaque dotted paths (`hp.max`) resolved entirely by a consumer-supplied callback;
Nightfox is the first consumer but any game system may use or replace this library. No Svelte in
its dependency closure — it is usable from server-side validators and other headless contexts,
not just the client.

Nightfox itself is a **standalone repository, separate from the Shadowcat engine repo** — its own
git history and its own public remote, which an agent never pushes to. It is nested a second time
into a Shadowcat checkout at `src/modules/nightfox/` purely so the pnpm workspace resolves
`@shadowcat/core`/`@shadowcat/formula` for dev. Every Nightfox source path below is relative to
the **Nightfox module repo's** own root (`src/...`), never to the Shadowcat engine repo's — do not
edit them from a Shadowcat working tree.

## The Nightfox rules engine

Headless, pure-function rules package: `system.stats`/`system.mechanics` data model (Zod
tier-1 validation) + a one-dependency-graph resolver + typed commutative modifier buckets +
`ITEM_DOC_TYPE`/`EFFECT_DOC_TYPE` semantics. Zero server change (client-semantics only);
zero Svelte/store dependency — sheets call these functions with docs they already
have.

- **The `nightfox-docs` module** (Nightfox repo) — the stat/mechanics data model and its fail-closed
  boundary. `Stat` is a discriminated union on `type`: `number` (`base` + optional `formula`/
  `roll`), `resource` (`current` + `maxBase`/`maxFormula`, `clampToMax` default `true`), `text`
  (`value: string`), `boolean` (`value: boolean`) — only `number`/`resource` are
  formula/modifier targets. `Modifier = { stat, op: "add"|"mulAdditive"|"mulCompound",
  value: string | number }` — magnitude is itself a formula or literal. `parseNightfox(doc)`
  returns `null` when BOTH `system.stats` and `system.mechanics` are absent (not a
  Nightfox-bearing doc) **or** when EITHER present side is malformed (fail-closed, never a
  partial parse) — an absent-but-not-both side defaults (`stats: {}` / `mechanics: {version:
  1}`). `validateStatKey`/`RESERVED_STAT_KEYS` reject `parent`/`base`/`current` plus every
  `@shadowcat/formula` function name (`FnName.min`/`FnName.max`/`FnName.floor`/`FnName.ceil`/
  `FnName.round`) and any dice-notation-keyword collision (imported from
  `@shadowcat/formula`'s `NOTATION_KEYWORDS`) — a key collides when its maximal `[a-z_]+` prefix
  is a notation keyword AND (the prefix is the whole key or the next char is a digit), so a key
  spelling `NOTATION_KEYWORDS.kh` exactly is rejected, and so is one spelling `NOTATION_KEYWORDS.kh`
  immediately followed by a digit; `damage`/`total` are not. Caps: `MAX_STATS`/`MAX_MODIFIERS = 128` per
  doc, `label` ≤ 64 chars, `text.value` ≤ 1024 chars, formula strings ≤ `@shadowcat/formula`'s
  `MAX_FORMULA_LENGTH`.
- **The `contributions` module** (Nightfox repo) — `collectNightfox(host)` walks the embed tree
  exactly (host → each `embedded` `ITEM_DOC_TYPE` child → that child's `embedded.effect`, plus host's own
  `embedded.effect`) and produces `ModifierContribution`s (`{ modId, carrierId, targetId,
  modifier }`) plus warnings. Targeting rule: an item's modifiers target the owning
  actor; an effect's modifiers target its host (actor-embedded → actor; item-embedded → the
  item; item-embedded **with `transfer: true`** → the item's owning actor). Active gating:
  `mechanics.active !== false` on the carrier itself AND the whole carrier chain — an inactive
  item suppresses its own modifiers **and** everything its embedded effects would otherwise
  contribute; an inactive effect suppresses only itself. Host-level `mechanics.modifiers` on an
  actor host is inert (`host-modifiers-inert` warning); on a standalone item/effect host
  (sheet-preview context) it's `dangling-modifiers`. A doc that fails `parseNightfox` is skipped
  entirely — it contributes nothing and does not appear in `Collected.docs`.
- **The `resolve` module** (Nightfox repo) — `resolveNightfox(host)` is **one** `@shadowcat/formula`
  `resolveAll` call over node keys `f:<docId>#<key>` (final value) and `c:<docId>#<key>`
  (resource `effectiveCurrent`), not three passes. Per stat: `base` → derived (the stat's own
  `formula`, else `base`/`maxBase`) → final (derived pushed through the bucket pipeline `final
  = (derived + Σadd) × (1 + ΣmulAdditive) × ΠmulCompound` — fixed stage order, commutative
  *within* each stage). **Canonical fold order is load-bearing, not cosmetic**: floating-point
  add/multiply is not associative, so contributions within a stage are folded in a fixed
  `(carrierId, modId)` sort order — the permutation battery (below) exists specifically to catch
  a regression here. Reference semantics: a bare reference resolves to the referenced
  stat's **final** value (cross-stat and cross-doc, so `attack = dex + str` sees a str-boosting
  belt); **exception 1**: a stat's own derived formula referencing its own key reads its
  own **base**, not final (layering, not recursion); **exception 2**: `base.<key>` always reads
  base from anywhere. A modifier magnitude referencing its own **target's** final value is a
  cycle → error (self-scaling composes via `mulAdditive` instead). Scope rules: bare
  identifiers in a **derived formula** resolve against the document that owns the formula;
  `parent.*` in a derived formula resolves against the embed parent (item-in-actor → actor,
  effect-in-item → item); bare identifiers in a **modifier magnitude** resolve against the
  modifier's **carrier**; `parent.*` in a magnitude resolves against the modifier's **target**
  (not the carrier) — a transferring effect's `parent.*` reaches the actor, not the item it
  rides on. Tolerance: a modifier targeting a missing stat or a `text`/`boolean` stat is
  inert + a `ResolveWarning` (community content mixing must never error the whole actor;
  resource targets always apply to `max`); a **formula** referencing a missing stat is a hard
  error value (fail-closed, visible chip) — the missing-ref tolerance is a modifier-only
  courtesy, not a general one. An errored magnitude poisons its target stat's final value rather
  than being silently dropped. `statRefResolver` exposes the same reference rules over an
  already-resolved doc for roll-template previews and sheet formula previews.
- **Nightfox's `index` module** (Nightfox repo) — the module's public barrel, re-exporting the full pure
  API surface above alongside a scaffolded manifest/`register()` (superseded by the real
  sheet registration below).

## The sheets layer

`src/sheets/` (Nightfox repo) — actor/item/effect sheets over the sheet registry
(`shadowcat.sheet:<doc_type>` contract, see `shadowcat-codebase-sheets`), all client-side, no
new wire frames.

- **The `sheet-model` module** — the read/write model shared by every sheet. `sheetView(top,
  systemPrefix)` ALWAYS resolves `resolveNightfox` from the TOP-LEVEL host document, then
  extracts the self doc's slice by id — resolving an embedded item/effect in isolation would
  drop transfer/`parent.*` flow (the scope rules above). The self doc is located by stripping the trailing
  `/system` off `systemPrefix` to get `basePrefix` — **the same `basePrefix` derivation
  documented in `shadowcat-codebase-sheets`'s `ActorSheet`/`ItemSheet` paragraph
  (`systemPrefix.replace(/\/system$/, "")`)**: `systemPrefix` (`/system` or
  `/embedded/<coll>/<i>/system`) is where `mechanics`/`stats` live, `basePrefix` is where
  `name`/`engine` live. **Confusing the two silently breaks OCC** — a `mechanics.*` field read
  via `basePrefix` instead of `systemPrefix` always resolves `undefined`, collapsing every
  write's `old` pre-image to `null`. No shipped test exercises the embedded (non-top-level) case
  that would catch this, so treat any new sheet or write helper touching
  `stats`/`mechanics` as needing this check first, and consider adding that regression test.
- **Map-CRUD idiom, every write helper (`addStat`/`editStatField`/`removeStat`/
  `setStatOrder`/`addModifier`/`editModifierField`/`removeModifier`/`setMechanicsFlag`)**: add =
  single-key `old:null`; edit = single-key raw-old; remove = whole-map `setField` replace with
  the current map as the pre-image (`set_pointer` cannot delete a key in place). Stat/modifier
  keys are spliced directly into a JSON pointer, so both `addStat` and `addModifier` guard
  against a key that would escape the intended pointer segment as defense-in-depth even though
  the calling UI is expected to have already surfaced Tier-1 validation — but the two guards are
  NOT the same mechanism: `addStat` calls the shared `validateStatKey` (rejects `/`-containing
  AND reserved words like `parent`/`base`/`current`/`min`/`max`), because a stat key is a
  user-chosen identifier subject to reserved-word/notation-collision rules (it can appear bare in
  a formula reference). `addModifier` uses its own simpler ad-hoc check
  (`id.length === 0 || id.includes("/")`), because a modifier id is an opaque generated UUID with
  no reserved-word semantics to protect — only pointer-injection safety matters for it.
- **Presentation-only order:** `StatTable`'s add-stat assigns `max-existing-order + 1`,
  not a count — a remove-then-add sequence with a naive count would collide orders. Order never
  feeds `resolveNightfox`; it is a sort key at render time only.
- **Permission-gating split — a Critical-class distinction, not interchangeable `readOnly`
  booleans.** A sheet's own `/system` writes (its own stats/modifiers/mechanics flags) gate on
  `core:write_fields`; a write to an EMBEDDED carrier's `/embedded/.../mechanics/<flag>`
  (item/effect active toggle, effect transfer toggle) gates on the DISTINCT
  `core:manage_embedded` capability. `ActorSheet` computes this per-carrier as `embedReadOnly`,
  never reusing the actor's own `core:write_fields`-derived `readOnly` for embedded controls;
  `ItemSheet` inherits the identical pattern (`effectReadOnly`) for its own
  embedded effects. `EffectSheet` has no embeds of its own (an effect is a leaf document) and
  needs no such split — its `readOnly` alone is correct and complete.
- **`nfT`/`NF_MESSAGES`** — chrome-translation helper: prefers the shell's `t`,
  falls back to a built-in English map when `t` echoes the key unchanged (the i18next/test
  "missing key" signal). Nightfox has no public seam to register its own i18n keys into the
  shell's translation catalog, so `NF_MESSAGES` is a permanent fallback, not a bootstrap
  scaffold — an out-of-tree module in the same position has the same gap. **Test-context
  gotcha:** `setAppContextForTest`'s default `t: (k) => k`
  identity-echo means `nfT` under test ALWAYS resolves through `NF_MESSAGES`, never through a
  real translation — a test asserting a raw i18n key can never pass against a correctly
  i18n-routed component; assert against `NF_MESSAGES["..."]` instead. User-authored stat
  labels/keys/values are DATA and never routed through `nfT`.
- **Registration** (Nightfox's `index` module) — `EFFECT_DOC_TYPE = "effect"` (no engine home yet:
  it is a Nightfox-defined document type, not one of the engine-owned 17); all three sheets contribute at `sheet: { priority:
  10 }`, above the generic sheets (0 / `-Infinity`) so a community sheet module can still outbid
  by priority.
- Component files: `StatRow`/`StatTable`/`ModifiersEditor` (shared
  editors — per-instance datalist ids via `$props.id()` to avoid collision across simultaneous
  instances), `ActorSheet`/`ItemSheet`/`EffectSheet` (per-doc-type sheets),
  the `format` module (value display + live formula-validation + warning chips, sharing `resolve`'s
  `isParseError`).

### The permutation invariant — a tested property, not a hope

`permutation.test` (Nightfox repo) is a 100-seed exact-equality battery over three
independently-toggled shuffle axes — embed-array order, record key insertion order (`stats` and
`modifiers` records together, one flag), and `order` field values — tested via four comparison
variants against the natural-order baseline: one per individual axis plus one with all three
axes combined. `resolveNightfox` output must deep-equal across all of them. This is the concrete
test the canonical fold order exists for; a failure here is a `resolve` bug, never a test to
loosen (`[[tests-yield-to-correct-code]]`).

## The roll wire

The `roll` module (Nightfox repo) — `buildStatRollContent(resolved: Map<string, ResolvedStat>, block:
NightfoxBlock, key: string): { content: string } | FormulaError` is the pure builder that turns a
resolved stat into chat content, posted verbatim through the existing chat seam
(`WsClient.sendChatMessage`/`SendMessage`). Zero new wire frames, zero server change — the
ingest boundary (`chat::rolls` caps/entropy/validation, see `shadowcat-codebase-chat`/
`shadowcat-codebase-dice`) remains the only security boundary; this builder is untrusted-input
producer, not consumer.

- **Pathway:** stat lookup in `block.stats[key]` → rollable-type gate (`number`/`resource` only;
  `text`/`boolean` or a missing key return a `FormulaError` instead of posting) → template =
  `stat.roll ?? key` (no authored template = a bare flat-value roll on the stat key itself) →
  `resolveNotationTemplate(template, statRefResolver(resolved, block))` (resolves every
  bare/dotted reference to its **final** resolved value per the existing resolver scope rules) →
  on success, `content = "<template> [[<notation>]]"`; on any resolver error (a referenced stat is
  itself errored, e.g. a broken `formula`), the builder returns that `FormulaError` and never
  posts.
- **Verbatim-copy rule:** the builder never rewrites, rounds, or otherwise normalizes the produced
  notation string — `resolveNotationTemplate` alone owns that (including the count-less
  `d<M>` → `1d<M>` normalization the dice-notation parser requires; the visible template text
  keeps the user's authored count-less `d<M>` spelling). A non-integer resolved value (e.g. a
  `formula` evaluating to `7 / 2`) is a `type` error
  from the builder rather than a silently-rounded roll — explicit rounding is required upstream.
- **One-embed-per-message constraint:** the builder emits exactly ONE inline `[[…]]` roll embed per
  message by construction, trivially satisfying the server's `MAX_INLINE_ROLLS=8` (`chat::rolls`).
  `MAX_MESSAGE_CHARS=4096` (the `chat` module) is NOT structurally guaranteed: `resolveNotationTemplate`
  caps only the pre-substitution template at `@shadowcat/formula`'s `MAX_FORMULA_LENGTH=512`, but
  each substituted identifier reference expands to `${value}[${originalText}]` with `value`
  unclamped up to `i32::MAX` — a pathological template packing many large-valued short-named
  identifier references can push the composed content over 4096 chars. This has no security
  consequence (the server's own length check still rejects the oversized message — no bypass, just
  a self-inflicted rejection for a pathological author), but do not assume the cap is unreachable.
  Never call the builder in a loop to compose a multi-roll message; that is an unenforced-by-this-
  function caller discipline, not a library-level cap.
- **Server-side prerequisite this pathway depends on:** the dice notation parser accepts a
  trailing `[label]` on ANY atomic factor (a bare `Const` or a `DiceGroup`), not only a dice group
  — required because `resolveNotationTemplate` substitutes a resolved identifier as a labeled
  constant (`value[name]`) even when the template has no dice group at all (e.g. `str`'s default
  flat roll → `4[str]`). **A NEGATIVE value is the exception and emits no label** — `(0 - N)`, an
  unlabeled parenthesized subtraction — so a negative stat contributes no `[label]` chip to the
  breakdown even though the total is unaffected. See `shadowcat-codebase-dice`'s
  `ConstTerm`/`labeled_consts` entries for the full mechanism — this skill only needs the
  one-sentence dependency plus that exception, not a duplicate description.
- **Differential proof, not just a unit test:** `roll-wire.e2e.test` (Nightfox repo,
  `test:e2e:roll-wire` script) spawns the REAL Rust `test_server` and sends every
  `buildStatRollContent` output shape through a real `WsClient`, asserting each survives the
  server's actual chat-ingest pipeline as an accepted `roll_embed` message with zero whispered
  `System` rejection notices, plus a sanity inversion (a deliberately-broken `[[1d]]` notation)
  proving the harness can actually detect a real rejection. A pure-unit test against the `roll`
  module alone never exercises the server's actual grammar, which is the gap this closes.

## The `@shadowcat/formula` graph-resolver contract (the `graph` module, Shadowcat engine repo)

Load-bearing for anyone writing a new `resolveAll.evalNode` consumer (Nightfox's `resolve` module is the first,
not the only, expected caller):

- **`resolveAll` is a pure function of the key set** — sorted-root traversal means the same set
  of requested keys always produces the same result regardless of call/iteration order. Traversing
  in the caller's key order instead makes the result order-dependent, which is exactly the kind of
  bug the Nightfox permutation battery is designed to surface one layer up.
- **Cycle-error detail names the lexicographically smallest cycle member** — a canonical,
  deterministic choice so two logically-identical graphs built in different key orders report
  byte-identical error details.
- **`resolveAll.evalNode` implementations MUST NOT wrap their own call(s) to the injected `get` in
  try/catch.** `resolveAll` drives evaluation via a restart-based trampoline keyed on an internal
  `NeedsDependency` signal thrown by `get`; swallowing it in a consumer's own try/catch breaks
  the trampoline and silently memoizes a wrong (partial) result. This is why Nightfox's
  `resolve` evaluator prefetches every reference path unwrapped before calling `evaluate()` —
  `evaluate`'s own `ref` case has a try/catch around a *different* concern (turning a malformed
  resolver return into a `FormulaError`) and must not be reused to catch the trampoline signal.
  The visiting/stack pairing invariant that makes this safe fails loudly (not silently) on
  violation.

## Key files & seams (`@shadowcat/formula`, Shadowcat engine repo)

- The `types` module — `FormulaError`/`FormulaErrorKind`/`FormulaValue`, `isFormulaError`, the four
  cap constants. Everything else imports from here.
- The `lexer` module → the `parser` module (`parseFormula` → `Expr` AST) → the `evaluate` module
  (`evaluate(expr, resolve)`) → the `graph` module (`resolveAll(keys, evalNode)`) →
  the `template` module (`resolveNotationTemplate`, `NOTATION_KEYWORDS`) — the five-stage pipeline in
  spec order.
- The `internal` module — shared trust-boundary helpers (`isWellFormedError`, `validateResolverOutput`,
  `finite`). **Not re-exported from `@shadowcat/formula`'s `index` module** — every injected-callback boundary (evaluate's
  `ref` case, graph's `resolveAll.evalNode` call, template's identifier resolver) validates a consumer
  callback's return value through these before trusting it as a `FormulaValue`.
- `@shadowcat/formula`'s `index` module — the only public entry point: types + caps + `parseFormula` + `evaluate` +
  `resolveAll` + `resolveNotationTemplate` + `NOTATION_KEYWORDS`.
- **Arithmetic semantics that surprise formula AUTHORS** (all in the `evaluate`/`lexer` modules): `/` is
  float division and `%` is JS TRUNCATED remainder, so `-7 % 2` is `-1`, not the floored `1` — and
  neither implicitly rounds, so a stat requiring an integer needs an explicit
  `FnName.floor`/`FnName.round` — and `FnName.round` is JS-native, meaning ties go toward
  +Infinity, NOT away from zero
  (`Math.round(-2.5) === -2`), which is a real difference for negative modifiers.
  Both `x / 0` and `x % 0` are `"div-zero"`, never `Infinity`/`NaN`; every arithmetic result is
  gated through `finite()`, so an overflow surfaces as `"non-finite"` rather than leaking
  `Infinity` downstream. **`.5` is not a numeric literal** — the lexer requires a leading digit and
  emits a bare `.` operator, so a leading-dot decimal is a parse error; write `0.5`. And
  `checkArity` runs at PARSE time only (the `parser` module), so an `Expr` hand-constructed against the
  public API bypasses arity checking entirely and degrades through `finite()` instead of erroring
  cleanly — build expressions with `parseFormula`, not by hand.

## Hard invariants

- **Error-value-only, fail-closed.** No function in this package ever throws on ANY input, and
  arithmetic never leaks `NaN`/`Infinity` — both become a `FormulaError` (`internal`'s
  `finite`). A consumer callback (`resolve`/`resolveAll.evalNode`) IS allowed to throw or return a malformed
  value; the library's own boundary code (`validateResolverOutput`) converts that into a
  `"resolver-error"` rather than propagating it. `FormulaErrorKind` is mirrored by hand in
  `FORMULA_ERROR_KINDS` (the `types` module) for runtime validation — adding a kind means updating BOTH the
  union and the array, with nothing else enforcing they stay in sync.
- **DoS caps, exact values:** `MAX_FORMULA_LENGTH=512`, `MAX_AST_NODES=256`,
  `MAX_PARSE_DEPTH=32` (counts true structural-nesting boundaries — parens, call args,
  unary-minus — NOT grammar-production depth; a flat `a+b+c+...` chain never trips it),
  `MAX_GRAPH_VISITS=2048` (charged once per newly discovered key in `resolveAll`).
- **`resolveAll`'s trampoline is O(1) JS-stack-depth by construction, not an implementation
  detail.** It restarts `resolveAll.evalNode` from scratch on an internal `NeedsDependency` throw rather than
  recursing, so graph depth never grows the call stack — required for constrained-stack mobile
  engines (project cross-platform invariant). Consumer `resolveAll.evalNode` bodies must NEVER wrap their own
  call(s) to the injected `get` in try/catch — that would swallow the internal signal driving the
  trampoline and silently memoize a wrong result. Documented in `graph`'s own JSDoc; treat any
  PR touching `resolveAll` or its consumers as needing that invariant re-verified.
- **Zero Nightfox vocabulary in this package.** If a change introduces a Nightfox-specific concept
  (stat, bucket, effect, etc.) into the Shadowcat engine repo's `src/client/formula/`, that is a
  layering violation — it belongs in the Nightfox module repo, not in `@shadowcat/formula`.
- **The grammar has no exponent notation.** `1e999` lexes as `num(1)` followed by `word("e999")` —
  a parse error, not a cap error. This is a deliberate grammar boundary, not a lexer defect; do
  not "fix" the lexer to accept exponents without a grammar change.
- **Identifiers are case-insensitive, normalized to lowercase; the library reserves no identifier
  names** (reserved-word/tier-1 validation is Nightfox's concern — shipped in
  `nightfox-docs`'s `RESERVED_STAT_KEYS`) — every consumer-facing guard belongs in the
  consumer, not here.

## Gotchas

- `internal`'s three helpers are the ONLY sanctioned way to cross a consumer-callback boundary.
  A gap in this pattern at one boundary reopens the same class of bug that the other injected-callback
  boundaries in the pipeline already guard against — treat any new injected-callback seam as
  needing the same validation, not a bespoke check.
- Arithmetic semantics (`/`, `%`, rounding, `finite()` gating, `.5`) → see the
  `@shadowcat/formula` arithmetic bullet under **Key files & seams** — stated once there, so the
  two copies cannot drift apart.
- Property/fuzz tests (`property.test` in the Shadowcat engine repo; `permutation.test` in the
  Nightfox module repo) use a hand-rolled seeded PRNG — do not add `fast-check` or any other new
  dependency to either package.
- **Nested for dev, the Nightfox repo is inside Shadowcat's gate perimeter.** `check-lint-allowances`
  walks each of its `ROOTS` recursively and skips only `SKIP_DIRS` (`node_modules`/`dist`/`target`/
  `.git`/`dist-docs`), so `pnpm lint:allowances` scans `src/modules/nightfox/**` and fails on a covered
  suppression there — even though Nightfox standalone has no ESLint and no lint script. Nesting is
  the only configuration Nightfox is developed in, so treat the gate as always applying to it.
- **Item-in-item nesting silently drops modifiers.** `contributions`'s embed walk is exactly
  host → items → each item's effects (+ host's own effects) — an item embedded inside another
  item is never visited, so its modifiers vanish with no warning. Item-in-item is not a supported
  shape; treat any fix here as a scope change, not a bugfix, and check whether the walk needs
  generalizing to arbitrary nesting depth or whether item-in-item stays disallowed.
- **`mechanics.active` is read doc_type-agnostically** on whatever document hosts it — the
  active-gating logic in `contributions` has no special case per `doc_type` (`ITEM_DOC_TYPE` vs
  `EFFECT_DOC_TYPE`), it just reads the field. Any future doc_type that reuses `system.mechanics` inherits
  active-gating for free; verify that's actually wanted before adding a new mechanics consumer.
- **`resolveNightfox` re-parses every doc on every call — there is no cross-call cache.** Fine at
  current call rates (sheet render, roll-button click); revisit if a future consumer wires it to
  a per-tick or per-frame path.
- **A literal `#` in a `docId` would collide with the `f:`/`c:` node-key scheme** (`f:<docId>#
  <key>`) — theoretical only, since server-issued document ids are UUIDs and can't contain `#`,
  but do not relax that assumption without re-deriving the node-key format.
- **`basePrefix`/`systemPrefix` confusion silently breaks OCC, not a crash.** See "The sheets
  layer" above — a `mechanics.*`/`stats.*` read through `basePrefix` resolves `undefined` and
  collapses the write's `old` pre-image to `null` with no error. It is the single highest-risk
  mistake for any new sheet touching `stats`/`mechanics`, and both `ItemSheet` and `EffectSheet`
  are shaped to avoid it.
- **`nfT` under `setAppContextForTest` never exercises real i18n** — its default `t: (k) => k`
  identity-echo means every `nfT` call resolves through `NF_MESSAGES`, not through a mocked
  translation catalog; a test asserting a raw key string can never pass against a component that
  correctly routes through `nfT`.
- **An embedded carrier's write capability is NOT the sheet's own `core:write_fields`.** Reusing a
  single `readOnly` for both a sheet's own fields and its embedded items/effects is a
  Critical-class permission bug, not a minor UX gap — always compute
  embedded-write gating from `core:manage_embedded` per carrier.

## Pointers

- **Generated API** — `/api/ts/modules/_shadowcat_formula.html` (TypeDoc — `@shadowcat/formula`,
  Shadowcat engine repo). The Nightfox rules engine, sheets layer, and roll wire live in the
  Nightfox module repo and have no page under the Shadowcat engine repo's `dist-docs/`. Produce
  with `pnpm build:all` from a Shadowcat checkout.
- `docs/design/ARCHITECTURE.md` §6 — the `system.stats`/`system.mechanics` reserved-directory
  premise as a durable engine invariant, independent of Nightfox as a specific system.
- `shadowcat-codebase-sheets` — the sheet registry (`shadowcat.sheet:<doc_type>` contract,
  `pickSheet`/`resolveDocRef`, the `basePrefix` derivation pattern) that Nightfox's sheets
  register into; read it alongside this skill for any sheet-registration work.
- This skill is scoped to the whole Nightfox surface — the formula library in the Shadowcat
  engine repo plus the rules engine, sheets and roll wire in the Nightfox module repo — not just
  the formula library. Any future Nightfox work should extend this skill rather than forking a
  new one, from either repo.
