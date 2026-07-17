---
name: shadowcat-codebase-nightfox
description: "Use when touching `@shadowcat/formula` (the framework-neutral expression library: lexer/parser/evaluator, dependency-graph resolution, dice-notation-template mode), the Nightfox rules engine it feeds (`nightfox-docs.ts`/`contributions.ts`/`resolve.ts`), or the Nightfox sheets layer (`src/sheets/*` — sheet-model.ts, StatRow/StatTable/ModifiersEditor, ActorSheet/ItemSheet/EffectSheet, nf-i18n.ts), external repo `C:\\Dev\\Nightfox`, nested for dev at `src/modules/nightfox/`. Covers src/client/formula/ (in-repo) and the Nightfox repo's src/ (out-of-repo, M13b+). Invoke shadowcat-codebase-core first."
---

# Shadowcat — Nightfox / `@shadowcat/formula`

Orientation for the shared formula library and the Nightfox rules engine consuming it (M13a
shipped in this repo; M13b + M13c shipped in the external Nightfox repo — this skill covers all
three, extended in-place rather than forking a new one per checkpoint). Spec:
`docs/superpowers/specs/2026-07-15-m13-nightfox-system-design.md` §§3-6 (decisions D2-D4, D7-D14).
Plans: `docs/superpowers/plans/2026-07-15-m13a-formula-library.md` (library),
`docs/superpowers/plans/2026-07-15-m13b-nightfox-headless-rules.md` (rules engine),
`docs/superpowers/plans/2026-07-16-m13c-nightfox-sheets.md` (sheets).

## Purpose

`@shadowcat/formula` (`src/client/formula/`, **in this repo**) is a pure-TS,
zero-runtime-dependency expression library: text → tokens → AST → number, plus generic
cycle-guarded dependency-graph resolution and a dice-notation-template rewrite mode. It has
**zero Nightfox concepts** — no stat types, no modifier buckets, no `parent`/`base` vocabulary.
References are opaque dotted paths (`hp.max`) resolved entirely by a consumer-supplied callback;
Nightfox is the first consumer but any game system may use or replace this library. No Svelte in
its dependency closure — it is usable from server-side validators and other headless contexts,
not just the client.

Nightfox itself (M13b) is a **standalone external repository** (`C:\Dev\Nightfox`, own git
history, never pushed by an agent — the user owns that remote), nested a second time into a
Shadowcat checkout at `src/modules/nightfox/` purely so the pnpm workspace resolves
`@shadowcat/core`/`@shadowcat/formula` for dev (M13-1's toolchain). All Nightfox source paths
below are Nightfox-repo-relative (`src/...` under `C:\Dev\Nightfox`), not in-tree Shadowcat
paths — never edit them from a Shadowcat-repo working tree.

## The Nightfox rules engine (M13b)

Headless, pure-function rules package: `system.stats`/`system.mechanics` data model (Zod
tier-1 validation) + a one-dependency-graph resolver + typed commutative modifier buckets +
`item`/`effect` semantics. Zero server change (client-semantics only, ARCHITECTURE §2 invariant
6); zero Svelte/store dependency — sheets (M13c) call these functions with docs they already
have.

- **`src/nightfox-docs.ts`** (Nightfox repo) — the stat/mechanics data model and its fail-closed
  boundary. `Stat` is a discriminated union on `type`: `number` (`base` + optional `formula`/
  `roll`), `resource` (`current` + `maxBase`/`maxFormula`, `clampToMax` default `true`), `text`
  (`value: string`), `boolean` (`value: boolean`) — only `number`/`resource` are
  formula/modifier targets (D7). `Modifier = { stat, op: "add"|"mulAdditive"|"mulCompound",
  value: string | number }` — magnitude is itself a formula or literal (D4). `parseNightfox(doc)`
  returns `null` when BOTH `system.stats` and `system.mechanics` are absent (not a
  Nightfox-bearing doc) **or** when EITHER present side is malformed (fail-closed, never a
  partial parse) — an absent-but-not-both side defaults (`stats: {}` / `mechanics: {version:
  1}`). `validateStatKey`/`RESERVED_STAT_KEYS` reject `parent`/`base`/`current`/`min`/`max`/
  `floor`/`ceil`/`round` plus any dice-notation-keyword collision (imported from
  `@shadowcat/formula`'s `NOTATION_KEYWORDS`) — a key collides when its maximal `[a-z_]+` prefix
  is a notation keyword AND (the prefix is the whole key or the next char is a digit), e.g.
  `d20`/`kh3` are rejected, `damage`/`total` are not. Caps: `MAX_STATS`/`MAX_MODIFIERS = 128` per
  doc, `label` ≤ 64 chars, `text.value` ≤ 1024 chars, formula strings ≤ `@shadowcat/formula`'s
  `MAX_FORMULA_LENGTH`.
- **`src/contributions.ts`** (Nightfox repo) — `collectNightfox(host)` walks the embed tree
  exactly (host → `embedded.item` → each item's `embedded.effect`, plus host's own
  `embedded.effect`) and produces `ModifierContribution`s (`{ modId, carrierId, targetId,
  modifier }`) plus warnings. Targeting per spec §5.3: an item's modifiers target the owning
  actor; an effect's modifiers target its host (actor-embedded → actor; item-embedded → the
  item; item-embedded **with `transfer: true`** → the item's owning actor). Active gating:
  `mechanics.active !== false` on the carrier itself AND the whole carrier chain — an inactive
  item suppresses its own modifiers **and** everything its embedded effects would otherwise
  contribute; an inactive effect suppresses only itself. Host-level `mechanics.modifiers` on an
  actor host is inert (`host-modifiers-inert` warning); on a standalone item/effect host
  (sheet-preview context) it's `dangling-modifiers`. A doc that fails `parseNightfox` is skipped
  entirely — it contributes nothing and does not appear in `Collected.docs`.
- **`src/resolve.ts`** (Nightfox repo) — `resolveNightfox(host)` is **one** `@shadowcat/formula`
  `resolveAll` call over node keys `f:<docId>#<key>` (final value) and `c:<docId>#<key>`
  (resource `effectiveCurrent`), not three passes. Per stat: `base` → `derived` (the stat's own
  `formula`, else `base`/`maxBase`) → `final` (derived pushed through the bucket pipeline `final
  = (derived + Σadd) × (1 + ΣmulAdditive) × ΠmulCompound` — fixed stage order, commutative
  *within* each stage). **Canonical fold order is load-bearing, not cosmetic**: floating-point
  add/multiply is not associative, so contributions within a stage are folded in a fixed
  `(carrierId, modId)` sort order — the permutation battery (below) exists specifically to catch
  a regression here. Reference semantics (§5.2): a bare reference resolves to the referenced
  stat's **final** value (cross-stat and cross-doc, so `attack = dex + str` sees a str-boosting
  belt); **exception 1 (D8)**: a stat's own derived formula referencing its own key reads its
  own **base**, not final (layering, not recursion); **exception 2**: `base.<key>` always reads
  base from anywhere. A modifier magnitude referencing its own **target's** final value is a
  cycle → error (self-scaling composes via `mulAdditive` instead). Scope rules (§5.3): bare
  identifiers in a **derived formula** resolve against the document that owns the formula;
  `parent.*` in a derived formula resolves against the embed parent (item-in-actor → actor,
  effect-in-item → item); bare identifiers in a **modifier magnitude** resolve against the
  modifier's **carrier**; `parent.*` in a magnitude resolves against the modifier's **target**
  (not the carrier) — a transferring effect's `parent.*` reaches the actor, not the item it
  rides on. Tolerance (§5.4): a modifier targeting a missing stat or a `text`/`boolean` stat is
  inert + a `ResolveWarning` (community content mixing must never error the whole actor;
  resource targets always apply to `max`); a **formula** referencing a missing stat is a hard
  error value (fail-closed, visible chip) — the missing-ref tolerance is a modifier-only
  courtesy, not a general one. An errored magnitude poisons its target stat's final value rather
  than being silently dropped. `statRefResolver` exposes the same reference rules over an
  already-resolved doc for roll-template previews (M13d) and sheet formula previews.
- **`src/index.ts`** (Nightfox repo) — the module's public barrel, re-exporting the full pure
  API surface above alongside the M13-1-scaffolded manifest/`register()` (superseded by real
  sheet registration in M13c, not touched by M13b).

## The sheets layer (M13c)

`src/sheets/` (Nightfox repo) — actor/item/effect sheets over the M12c sheet registry
(`shadowcat.sheet:<doc_type>` contract, see `shadowcat-codebase-sheets`), all client-side, no
new wire frames.

- **`sheet-model.ts`** — the read/write model shared by every sheet. `sheetView(top,
  systemPrefix)` ALWAYS resolves `resolveNightfox` from the TOP-LEVEL host document, then
  extracts the self doc's slice by id — resolving an embedded item/effect in isolation would
  drop transfer/`parent.*` flow (§5.3). The self doc is located by stripping the trailing
  `/system` off `systemPrefix` to get `basePrefix` — **the same M13-0 `basePrefix` derivation
  documented in `shadowcat-codebase-sheets`'s `ActorSheet`/`ItemSheet` paragraph
  (`systemPrefix.replace(/\/system$/, "")`)**: `systemPrefix` (`/system` or
  `/embedded/<coll>/<i>/system`) is where `mechanics`/`stats` live, `basePrefix` is where
  `name`/`engine` live. **Confusing the two silently breaks OCC** — a `mechanics.*` field read
  via `basePrefix` instead of `systemPrefix` always resolves `undefined`, collapsing every
  write's `old` pre-image to `null`. This exact bug was caught live in `ItemSheet` and avoided
  proactively in `EffectSheet` after the flag — treat any new sheet or write helper touching
  `stats`/`mechanics` as needing this check first.
- **Map-CRUD idiom (D11), every write helper (`addStat`/`editStatField`/`removeStat`/
  `setStatOrder`/`addModifier`/`editModifierField`/`removeModifier`/`setMechanicsFlag`)**: add =
  single-key `old:null`; edit = single-key raw-old; remove = whole-map `setField` replace with
  the current map as the pre-image (`set_pointer` cannot delete a key in place). Stat/modifier
  keys are spliced directly into a JSON pointer, so `addStat`/`addModifier` guard against a
  `/`-containing or reserved key (`validateStatKey`) as defense-in-depth even though the calling
  UI is expected to have already surfaced Tier-1 validation.
- **Presentation-only order (D12):** `StatTable`'s add-stat assigns `max-existing-order + 1`,
  not a count — a remove-then-add sequence with a naive count would collide orders. Order never
  feeds `resolveNightfox`; it's a `sort()` key at render time only.
- **Permission-gating split — a Critical-class distinction, not interchangeable `readOnly`
  booleans.** A sheet's own `/system` writes (its own stats/modifiers/mechanics flags) gate on
  `core:write_fields`; a write to an EMBEDDED carrier's `/embedded/.../mechanics/<flag>`
  (item/effect active toggle, effect transfer toggle) gates on the DISTINCT
  `core:manage_embedded` capability. `ActorSheet` computes this per-carrier as `embedReadOnly`,
  never reusing the actor's own `write_fields`-derived `readOnly` for embedded controls — caught
  as a Critical/Important in Task 7's pre-authorized buddy-check (2 blind reviewers, same finding
  independently); `ItemSheet`/`EffectSheet` inherit the identical pattern for their own embeds.
- **`nfT`/`NF_MESSAGES` (`nf-i18n.ts`)** — chrome-translation helper: prefers the shell's `t`,
  falls back to a built-in English map when `t` echoes the key unchanged (the i18next/test
  "missing key" signal). **Test-context gotcha:** `setAppContextForTest`'s default `t: (k) => k`
  identity-echo means `nfT` under test ALWAYS resolves through `NF_MESSAGES`, never through a
  real translation — a test asserting a raw i18n key can never pass against a correctly
  i18n-routed component; assert against `NF_MESSAGES["..."]` instead. User-authored stat
  labels/keys/values are DATA and never routed through `nfT`.
- **Registration (`index.ts`)** — `EFFECT_DOC_TYPE = "effect"` (D9; no engine home yet, filed as
  friction in `docs/POST_WORK_FINDINGS.md`); all three sheets contribute at `sheet: { priority:
  10 }`, above the generic sheets (0 / `-Infinity`) so a community sheet module can still outbid
  by priority (D10).
- Component files: `StatRow.svelte`/`StatTable.svelte`/`ModifiersEditor.svelte` (shared
  editors — per-instance datalist ids via `$props.id()` to avoid collision across simultaneous
  instances), `ActorSheet.svelte`/`ItemSheet.svelte`/`EffectSheet.svelte` (per-doc-type sheets),
  `format.ts` (value display + live formula-validation + warning chips, sharing `resolve.ts`'s
  `isParseError`).

### The permutation invariant (D3/D12) — a tested property, not a hope

`src/permutation.test.ts` (Nightfox repo) is a 100-seed exact-equality battery over three
independently-toggled shuffle axes — embed-array order, record key insertion order (`stats` and
`modifiers` records together, one flag), and `order` field values — tested via four comparison
variants against the natural-order baseline: one per individual axis plus one with all three
axes combined. `resolveNightfox` output must deep-equal across all of them. This is the concrete test the
canonical-fold-order fix exists for; a failure here is a Task-4/`resolve.ts` bug, never a test to
loosen (`[[tests-yield-to-correct-code]]`).

## The `@shadowcat/formula` graph-resolver contract (`src/graph.ts`, in this repo)

Load-bearing for anyone writing a new `evalNode` consumer (Nightfox's `resolve.ts` is the first,
not the only, expected caller):

- **`resolveAll` is a pure function of the key set** — sorted-root traversal means the same set
  of requested keys always produces the same result regardless of call/iteration order (fixed
  during M13b Task 5 review: an earlier version was order-dependent on the caller's key
  ordering, which is exactly the kind of bug the Nightfox permutation battery is designed to
  surface one layer up).
- **Cycle-error detail names the lexicographically smallest cycle member** — a canonical,
  deterministic choice so two logically-identical graphs built in different key orders report
  byte-identical error details.
- **`evalNode` implementations MUST NOT wrap their own call(s) to the injected `get` in
  try/catch.** `resolveAll` drives evaluation via a restart-based trampoline keyed on an internal
  `NeedsDependency` signal thrown by `get`; swallowing it in a consumer's own try/catch breaks
  the trampoline and silently memoizes a wrong (partial) result. This is why Nightfox's
  `resolve.ts` evaluator prefetches every reference path unwrapped before calling `evaluate()` —
  `evaluate`'s own `ref` case has a try/catch around a *different* concern (turning a malformed
  resolver return into a `FormulaError`) and must not be reused to catch the trampoline signal.
  The visiting/stack pairing invariant that makes this safe fails loudly (not silently) on
  violation as of the M13b Task 5 fix.

## Key files & seams (`@shadowcat/formula`, in this repo)

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
  names** (reserved-word/tier-1 validation is Nightfox's concern — shipped in
  `nightfox-docs.ts`'s `RESERVED_STAT_KEYS`, M13b) — every consumer-facing guard belongs in the
  consumer, not here.

## Gotchas

- `internal.ts`'s three helpers are the ONLY sanctioned way to cross a consumer-callback boundary.
  A prior task (buddy-check-caught) skipped this pattern at one boundary and reopened a bug
  already fixed twice elsewhere in the pipeline — treat any new injected-callback seam as
  needing the same validation, not a bespoke check.
- `/` is float division; `%` is JS truncated remainder; neither implicitly rounds.
- Property/fuzz tests (`property.test.ts` in this repo; `permutation.test.ts` in the Nightfox
  repo) use a hand-rolled seeded PRNG — do not add `fast-check` or any other new dependency to
  either package (Global Constraint).
- **Item-in-item nesting silently drops modifiers.** `contributions.ts`'s embed walk is exactly
  host → items → each item's effects (+ host's own effects) — an item embedded inside another
  item is never visited, so its modifiers vanish with no warning. Not yet a spec-covered case;
  treat any fix here as a scope change, not a bugfix, and check whether the walk needs
  generalizing to arbitrary nesting depth or whether item-in-item stays disallowed.
- **`mechanics.active` is read doc_type-agnostically** on whatever document hosts it — the
  active-gating logic in `contributions.ts` has no special case per `doc_type` (`item` vs
  `effect`), it just reads the field. Any future doc_type that reuses `system.mechanics` inherits
  active-gating for free; verify that's actually wanted before adding a new mechanics consumer.
- **`resolveNightfox` re-parses every doc on every call — there is no cross-call cache.** Fine at
  current call rates (sheet render, roll-button click); revisit if a future consumer wires it to
  a per-tick or per-frame path.
- **A literal `#` in a `docId` would collide with the `f:`/`c:` node-key scheme** (`f:<docId>#
  <key>`) — theoretical only, since server-issued document ids are UUIDs and can't contain `#`,
  but do not relax that assumption without re-deriving the node-key format.
- **`basePrefix`/`systemPrefix` confusion silently breaks OCC, not a crash.** See "The sheets
  layer" above — a `mechanics.*`/`stats.*` read through `basePrefix` resolves `undefined` and
  collapses the write's `old` pre-image to `null` with no error. Caught twice already in this
  checkpoint (`ItemSheet` live, `EffectSheet` proactively); treat as the single highest-risk
  mistake for any new sheet touching `stats`/`mechanics`.
- **`nfT` under `setAppContextForTest` never exercises real i18n** — its default `t: (k) => k`
  identity-echo means every `nfT` call resolves through `NF_MESSAGES`, not through a mocked
  translation catalog; a test asserting a raw key string can never pass against a component that
  correctly routes through `nfT`.
- **An embedded carrier's write capability is NOT the sheet's own `write_fields`.** Reusing a
  single `readOnly` for both a sheet's own fields and its embedded items/effects is a
  Critical-class permission bug (Task 7 buddy-check), not a minor UX gap — always compute
  embedded-write gating from `core:manage_embedded` per carrier.

## Pointers

- Spec: `docs/superpowers/specs/2026-07-15-m13-nightfox-system-design.md` §3 (formula grammar,
  caps, error model, template mode — `@shadowcat/formula`), §§4-5 (Nightfox data model + one-graph
  resolver, decisions D2-D4, D7-D9, D11-D14 — the rules engine), §6 (sheets — this skill's
  "sheets layer" section).
- Plans: `docs/superpowers/plans/2026-07-15-m13a-formula-library.md` (M13a, shipped, this repo);
  `docs/superpowers/plans/2026-07-15-m13b-nightfox-headless-rules.md` (M13b, shipped, Nightfox
  repo); `docs/superpowers/plans/2026-07-16-m13c-nightfox-sheets.md` (M13c, shipped, Nightfox
  repo).
- `docs/PLAN.md` M13 section — milestone chain (M13-0 → M13-1 → **M13a** → **M13b** → **M13c** →
  M13d → M13e → M13f) and which repo (this one vs. the external Nightfox repo) owns each step;
  the M13b/M13c done-entries there are the empirically-verified record of what shipped (task
  count, buddy-check findings, suite counts) — this skill is the orientation layer, not the
  changelog.
- `docs/design/ARCHITECTURE.md` §6 — the `system.stats`/`system.mechanics` reserved-directory
  premise (D13/D14) as a durable engine invariant, independent of Nightfox as a specific system.
- `shadowcat-codebase-sheets` — the M12c sheet registry (`shadowcat.sheet:<doc_type>` contract,
  `pickSheet`/`resolveDocRef`, the `basePrefix` derivation pattern) that M13c's sheets register
  into; read it alongside this skill for any sheet-registration work.
- `docs/POST_WORK_FINDINGS.md` — the external-module i18n-registration-seam gap and the
  `effect`-doc_type-has-no-engine-home gap, both filed at M13c.
- Once M13d lands (Nightfox repo, `src/`), extend this skill's Nightfox sections in place rather
  than splitting — this skill is scoped to the whole Nightfox surface (in-repo library +
  out-of-repo rules engine + sheets + rolls), not just the formula library.
