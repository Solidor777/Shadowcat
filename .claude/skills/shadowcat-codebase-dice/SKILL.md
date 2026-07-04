---
name: shadowcat-codebase-dice
description: "Use when touching Shadowcat's dice engine: RollSpec/Expr AST, the seeded-noise RNG, roll/evaluate/recalculate, the per-group reroll/explode/keep-drop pipeline, group_index-based Sum folding, SuccessCount aggregation, group_spans-based recalculation, or the dice notation lexer/parser. Covers src/server/src/dice/. Invoke shadowcat-codebase-core first."
---

# Shadowcat — Dice Engine

Orientation for the server-authoritative dice engine: a pure Rust library with no wire/ws
coupling yet (M11a). M11b (expertise DP, crit events, Tiered mode, labeled/custom-face dice,
`direction` global flip) and M11d (transport/wire integration) build on this foundation later.

## Purpose

A roll is a canonical `RollSpec` (an `Expr` AST of dice groups + arithmetic, plus a `Mode` and
success config). `roll(spec, rng) -> RawRoll` is the **only** randomness step; `evaluate(spec,
raws) -> RollOutcome` is deterministic; `recalculate` applies targeted die ops then re-derives.
Randomness is a stateless noise function keyed by `(seed, index)`, so results are reproducible by
construction without persisting the seed — only the resulting `RawRoll` (natural faces) needs to
be stored. String notation (`4d6kh3+2`, `5d10cs>=7`) is one parser front-end producing a
`RollSpec`; the canonical struct form is what every other function operates on.

## Key files & seams

- `src/server/src/dice/rng.rs` — `noise(seed, n) -> u64` (hand-rolled SplitMix64 finalizer, **no
  `rand` dependency** — a deliberate user preference for determinism-by-construction).
  `RngSource` trait (`next_u32`) + `NoiseRng` (stateful sequential generator). `roll_uniform`
  (unbiased inclusive-range draw via rejection sampling) is the **only** place raw entropy becomes
  a die face — has a hard guard against the exact-2^32-span panic and a documented-conservative
  (not maximally tight) rejection threshold. `NoiseRng::at(seed, index)` is a pure `(seed,index)`
  function with **no correspondence to a sequential `next_u32()` walk once any rejection has
  occurred** — do not use it to "replay the k-th die" of a real roll; currently unused by any
  consumer.
- `src/server/src/dice/spec.rs` — the canonical AST: `DieKind::Numeric{min,max}` (M11a numeric
  only; `Faces` for custom-symbol dice is M11b), `Comparator`+`test`, `ExplodeKind`
  (Standard/Compound/Penetrate), `GroupModifier`, `DiceGroup{count,kind,modifiers}` (**modifiers
  apply in Vec order — caller-controlled**, e.g. reroll-then-keep vs keep-then-reroll are
  different specs), `Expr` (Dice/Const/Bin/Neg), `Mode` (Sum/SuccessCount), `SuccessRule`,
  `RollSpec`.
- `src/server/src/dice/outcome.rs` — `RawDie`, `RawRoll` (`dice`, `records`, `next_id`,
  `group_spans`), `DieRecord` (`id`, `group_index`, `natural`, `value`, `kept`, `exploded`,
  `rerolled_from`), `RollOutcome`, `RollResult`.
- `src/server/src/dice/eval/groups.rs` — `resolve_group(group, group_index, naturals, rng, raws)
  -> Vec<DieRecord>`: the per-group pipeline (reroll → explode → keep/drop, in modifier-Vec
  order). `push_extra` is the single construction site for an exploded/penetrated child
  `DieRecord`. `CHAIN_CAP = 100` bounds chained explosions/rerolls per die.
- `src/server/src/dice/eval/mod.rs` — `roll(spec, rng) -> RawRoll` (walks `Expr` left-to-right,
  the ONLY randomness entry point) and `evaluate(spec, raws) -> RollOutcome` (dispatches
  `Mode::Sum` → `sum::evaluate_sum`, `Mode::SuccessCount` → `success::evaluate_success`).
- `src/server/src/dice/eval/sum.rs` — folds the AST to a total by matching `DieRecord.group_index`
  against an AST-order cursor (`fold`); **the group-boundary reconstruction is the correctness
  core** — a wrong boundary silently mis-sums a multi-group roll.
- `src/server/src/dice/eval/success.rs` — pools **all kept records across every group** (ignores
  `group_index`/AST arithmetic entirely — this is the defining difference from Sum mode), counts
  against `spec.success`, sets `pass`/`net_margin` when `required_successes` is set. `total` is
  still populated as a reference kept-sum even in this mode.
- `src/server/src/dice/recalc.rs` — `recalculate(spec, raws, ops, rng) -> (RawRoll, RollOutcome)`
  + `RecalcOp{RerollDice, ReplaceDie, RemoveDice}`. Reconstructs each group's **base naturals only**
  from `RawRoll.group_spans` (excludes explosion/penetrate children by design), applies ops, then
  `rederive`s by re-running `resolve_group` over the mutated naturals in AST order.
- `src/server/src/dice/notation/{mod.rs,lexer.rs,parser.rs}` — `lex`/`Token`/`ParseError` +
  `parse(input) -> Result<RollSpec, ParseError>` (recursive descent: `expr := term (('+'|'-')
  term)*`; `term := factor (('*'|'/') factor)*`; `factor := '(' expr ')' | '-' factor | dice |
  int`). The lexer is **case-insensitive on the `d` dice operator** and enforces **ASCII-only
  input** as an explicit precondition (not an accident of the byte-as-char cast it uses
  internally). The parser rejects a second `cs`/`cf` in one spec (`ParseError::DuplicateSuccessRule`
  — `success: Option<SuccessRule>` is shared parser state, not per-group) and rejects
  `sides < 1` (`ParseError::InvalidDieSides`) before ever constructing a `DieKind::Numeric`.

## Hard invariants

- **`roll` is the ONLY randomness step; `evaluate` is pure/deterministic.** Given the same
  `(spec, raws)`, `evaluate` MUST return an identical `RollOutcome`. This is what makes a stored
  `RawRoll` (natural faces only, no seed) fully reproducible.
- **`resolve_group`'s outer Explode loop must never re-scan a die it (or a sibling die's own
  chain) already pushed** — snapshot the pool length (`initial_len`) before the pass; the inner
  chain loop is the sole mechanism that extends any one die's own chain. Violating this caused a
  real 41GB-memory-blowup bug (fixed in this branch's Task 4 buddy-check) and would silently
  falsify `CHAIN_CAP`'s bound.
- **An Explode/Reroll retrigger check must test the RAW rolled face, never a post-modifier value**
  (Penetrate's `-1` must apply only to the stored `value`, not to what gates whether the chain
  continues) — checking the decremented value silently truncates Penetrate chains to length 1.
- **Reroll and Explode must skip `!kept` dice.** Since modifiers apply in Vec order, a
  Drop-then-Reroll sequence is legal and must not mutate an already-dropped die.
- **`resolve_group`/`push_extra` must stamp every produced `DieRecord` (including exploded/
  penetrated children) with the CALLER-SUPPLIED `group_index`** — `eval::sum::fold`'s per-group
  folding depends on every record in a group carrying that group's own index, never a stale/wrong
  one.
- **`recalculate` targets ONLY base naturals (via `group_spans`), never explosion/penetrate
  children** — a `RecalcOp` naming a non-base id silently no-ops (documented on `RecalcOp`, not an
  error). **`recalculate` is NOT a no-op-diff snapshot for a group with an Explode/Reroll
  modifier**: `rederive` re-triggers the full modifier pipeline fresh against (possibly-unchanged)
  base naturals, so an UNTARGETED sibling die in the same group can get a brand-new explosion/
  reroll tail across a recalc call even though its own `natural` never changed. This is
  intentional/plan-approved (not a bug) — see `recalc.rs`'s doc comment and the pinning tests in
  `recalc.rs`'s test module (`explosion_tail_for_untouched_sibling_changes_across_recalc` et al.)
  for the exact proven behavior. Base-die `natural` values ARE stable across recalc; derived
  records for exploding/rerolling dice are NOT.
- **Every `DieKind::Numeric` construction from untrusted/parsed input must validate `sides >= 1`
  before construction** — `rng::roll_uniform` only `debug_assert!`s this (unsafe in release). The
  notation parser enforces it (`ParseError::InvalidDieSides`); any FUTURE wire-facing `RollSpec`
  construction path (M11d) bypassing the parser needs the identical guard independently.
- **Pure library — `dice` must never depend on `ws`/`data`/`http`/`scene`.** No wire frames, no
  `#[derive(TS)]`/ts-rs bindings in M11a/b (that's M11d's job, once real consumers exist).

## Gotchas

- **The plan text and the real code can drift** — this module was built through several
  buddy-check fix rounds that changed `resolve_group`'s signature (added `group_index`), added
  `push_extra`, and added `kept`-flag gating, all AFTER the plan's own Task 6/9/10 example code was
  written. Always read the actual current file before assuming a plan snippet's exact signature —
  three separate tasks in this milestone needed adaptation for this reason.
- **No per-roll dice-count cap exists yet.** `DiceGroup.count: u32` is unbounded; `RawRoll::next_id`
  (a `u32` counter) has no overflow guard; `eval::sum::fold`'s `i64` total sum is theoretically
  overflowable for a pathological `count`. All tracked in `docs/TODO.md` "Server / dice (M11a)",
  deferred to whatever cap Task 9/M11d establishes at the transport boundary — not yet built.
- **`cf` (fail-count) is a single-count-path approximation in M11a** — it inverts the comparator on
  the SAME `SuccessRule` field `cs` uses, not a dedicated fail-counter. Dedicated fail-count
  semantics are M11b.
- **`ParseError` messages are raw `{:?}` Debug output** (e.g. `"Some(BangP), expected int"`) — not
  yet player-presentable. Needs a `Display` impl for `Token` before M11c/d surfaces parse errors
  directly in a chat UI (logged in `docs/TODO.md`).
- **Buddy-check track record in this module**: all three of the plan's pre-approved buddy-check
  tasks (Task 4 group pipeline, Task 6 group_index fold, Task 10 recalculate) found and fixed real
  Critical/Important bugs — this is dense, easy-to-get-subtly-wrong pipeline logic; treat any
  future change to `eval/groups.rs`, `eval/sum.rs`, or `recalc.rs` as buddy-check-worthy by default.

## Pointers

- Rationale: `docs/superpowers/specs/2026-07-03-m11-dice-engine-design.md` (full design, M11a+b
  scope split); `docs/superpowers/plans/2026-07-03-m11a-dice-engine-core.md` (task-by-task
  implementation plan + Buddy-check/Model-effort directives).
- Deferred work: `docs/TODO.md` "Server / dice (M11a)" section (dice-count cap, serde-defaults,
  `Token` Display impl — several already marked RESOLVED as later tasks closed them).
- M11 milestone context (dice + chat, parallel to the M10 movement/vision track): memory
  `m11-dice-chat-resume` in the project's auto-memory.
