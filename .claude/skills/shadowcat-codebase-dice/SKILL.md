---
name: shadowcat-codebase-dice
description: "Use when touching Shadowcat's dice engine: RollSpec/Expr AST, the seeded-noise RNG, roll/evaluate/recalculate, the per-group reroll/explode/keep-drop pipeline, group_index-based Total folding, SuccessCount aggregation, group_spans-based recalculation, the shared classify/crit layers, or the dice notation lexer/parser. Covers src/server/src/dice/. Invoke shadowcat-codebase-core first."
---

# Shadowcat — Dice Engine

Orientation for the server-authoritative dice engine: a pure Rust library with no wire/ws
coupling yet. M11a shipped the core evaluator (RNG, per-group pipeline, Sum/SuccessCount modes,
notation, recalculate). **M11b-1 shipped** the `direction` global flip, a data-carrying `Mode`,
the shared classification layer, crit events, and unified `t<N>` notation (all detailed below).
Still deferred: expertise DP (**M11b-2**) and labeled/custom-face dice (**M11b-3**). M11d
(transport/wire integration) builds on all of this later.

## Purpose

A roll is a canonical `RollSpec` (an `Expr` AST of dice groups + arithmetic, a `direction`, and a
`Mode` carrying its own config). `roll(spec, rng) -> RawRoll` is the **only** randomness step;
`evaluate(spec, raws) -> RollOutcome` is deterministic; `recalculate` applies targeted die ops then
re-derives. Randomness is a stateless noise function keyed by `(seed, index)`, so results are
reproducible by construction without persisting the seed — only the resulting `RawRoll` (natural
faces) needs to be stored. String notation (`4d6kh3+2`, `5d10cs>=7`, `1d20t15`) is one parser
front-end producing a `RollSpec`; the canonical struct form is what every other function operates
on.

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
- `src/server/src/dice/spec.rs` — the canonical AST: `DieKind::Numeric{min,max}` (numeric only;
  `Faces` for custom-symbol dice is M11b-3), `Comparator`+`test`, `ExplodeKind`
  (Standard/Compound/Penetrate), `GroupModifier`, `DiceGroup{count,kind,modifiers}` (**modifiers
  apply in Vec order — caller-controlled**, e.g. reroll-then-keep vs keep-then-reroll are
  different specs), `Expr` (Dice/Const/Bin/Neg). `Direction` (`HighWins`/`LowWins`, `#[default]
  HighWins`) is a global flip on `RollSpec::direction` orienting every margin/tier/crit
  computation. `Tier{margin_offset, label, tier_value}` is one rung of a classification ladder,
  shared by both modes. `CritSuccess{threshold, extra_successes, positive_counter}` /
  `CritFail{threshold, lost, negative_counter, allow_negative}` are SuccessCount-only crit-event
  configs. `TotalConfig{difficulty: Option<i32>, tiers: Vec<Tier>}` — Sum and Tiered are now ONE
  mode: empty `tiers` = default pass/fail at `margin >= 0`; non-empty = a custom ladder.
  `SuccessConfig{success: SuccessRule, required_successes: Option<i32>, tiers: Vec<Tier>,
  crit_success: Option<CritSuccess>, crit_fail: Option<CritFail>}` — `expertise: u32` is
  deliberately absent (M11b-2). `Mode` is now **data-carrying**: `Total(TotalConfig) |
  SuccessCount(SuccessConfig)` (replaces M11a's unit `Mode::Sum | Mode::SuccessCount`).
  `RollSpec{expr, direction: Direction, mode: Mode}`.
- `src/server/src/dice/outcome.rs` — `RawDie`, `RawRoll` (`dice`, `records`, `next_id`,
  `group_spans`), `DieRecord` (`id`, `group_index`, `natural`, `value`, `kept`, `exploded`,
  `rerolled_from`, `crit_success: bool`, `crit_fail: bool`). `RollOutcome`'s final shape: `total:
  i64`, `records`, `successes: Option<i32>`, `pass: Option<bool>`, `margin: Option<i64>` (renamed
  + widened from M11a's `net_margin`), `tier_label: Option<String>`, `tier_value: Option<i32>`,
  `crit_successes: i32`, `crit_fails: i32`, `positive_counter: i32`, `negative_counter: i32` (all
  0/None in Total mode with no `difficulty`, or in SuccessCount with no crit config). `RollResult`.
- `src/server/src/dice/eval/groups.rs` — `resolve_group(group, group_index, naturals, rng, raws)
  -> Vec<DieRecord>`: the per-group pipeline (reroll → explode → keep/drop, in modifier-Vec
  order). `push_extra` is the single construction site for an exploded/penetrated child
  `DieRecord`. `CHAIN_CAP = 100` bounds chained explosions/rerolls per die.
- `src/server/src/dice/eval/mod.rs` — `roll(spec, rng) -> RawRoll` (walks `Expr` left-to-right,
  the ONLY randomness entry point) and `evaluate(spec, raws) -> RollOutcome` (dispatches
  `Mode::Total(cfg)` → `sum::evaluate_total`, `Mode::SuccessCount(cfg)` →
  `success::evaluate_success`).
- `src/server/src/dice/eval/classify.rs` — the shared classification layer used by BOTH modes.
  `oriented_margin(direction, scalar, reference) -> i64` flips a margin so "better" is always more
  positive (`HighWins: scalar - reference`; `LowWins: reference - scalar`) — used ONLY by Total
  mode. `classify(margin: i64, tiers: &[Tier]) -> Classification{pass, tier_label, tier_value}`:
  empty `tiers` => default 2-rung pass/fail at `margin >= 0`; non-empty => the highest rung with
  `margin_offset <= margin`, fail-closed to the lowest rung if below every offset (order-
  independent, no sorted precondition).
- `src/server/src/dice/eval/crit.rs` — `DieCrit{is_success, is_fail, extra_successes, lost,
  positive_counter, negative_counter}` + `score_die(direction, value, cfg: &SuccessConfig) ->
  DieCrit`: scores one kept die against `cfg.crit_success`/`cfg.crit_fail` independently
  (`reaches()` flips both comparisons under `LowWins`). Both `is_success` and `is_fail` CAN be
  `true` on the same die under an overlapping-threshold config — intentional, tested
  (`overlapping_thresholds_fire_both_crit_success_and_crit_fail`), not a bug.
- `src/server/src/dice/eval/sum.rs` — `evaluate_total(spec, cfg: &TotalConfig, raws) ->
  RollOutcome`: folds the AST to a total by matching `DieRecord.group_index` against an AST-order
  cursor (`fold`); **the group-boundary reconstruction is the correctness core** — a wrong
  boundary silently mis-sums a multi-group roll. If `cfg.difficulty` is set, classifies via
  `oriented_margin` + `classify::classify`; otherwise reports a bare total (`pass`/`margin`/
  `tier_*` all `None`).
- `src/server/src/dice/eval/success.rs` — `evaluate_success(spec, cfg: &SuccessConfig, raws) ->
  RollOutcome`: pools **all kept records across every group** (ignores `group_index`/AST
  arithmetic entirely — the defining difference from Total mode), counts base successes against
  `cfg.success`, then folds each kept die's `crit::score_die` result into net successes and the
  positive/negative counters (counters are a SEPARATE output, never folded into `successes`). Net
  = `base + extra_successes - lost`, clamped at 0 unless `cfg.crit_fail.allow_negative` opts out.
  If `cfg.required_successes` is set, classifies `net - required` via the SAME shared
  `eval::classify::classify` Total uses — but **NEVER runs it through `oriented_margin`**: more
  successes is always better, and `direction` was already applied per-die inside `crit::score_die`.
  This asymmetry (Total margins are direction-flipped; SuccessCount margins are a plain
  subtraction) is load-bearing — a future change that "fixes" SuccessCount to also call
  `oriented_margin` would double-apply direction and silently invert every LowWins SuccessCount
  roll's pass/tier result.
- `src/server/src/dice/recalc.rs` — `recalculate(spec, raws, ops, rng) -> (RawRoll, RollOutcome)`
  + `RecalcOp{RerollDice, ReplaceDie, RemoveDice}`. Reconstructs each group's **base naturals only**
  from `RawRoll.group_spans` (excludes explosion/penetrate children by design), applies ops, then
  `rederive`s by re-running `resolve_group` over the mutated naturals in AST order.
- `src/server/src/dice/notation/{mod.rs,lexer.rs,parser.rs}` — `lex`/`Token`/`ParseError` +
  `parse(input: &str, ctx: ParseContext) -> Result<RollSpec, ParseError>` (recursive descent:
  `expr := term (('+'|'-') term)*`; `term := factor (('*'|'/') factor)*`; `factor := '(' expr ')'
  | '-' factor | dice | int`). `ParseContext{mode: ModeKind, direction: Direction}` is caller-
  supplied ambient state the notation string does not itself encode: `mode` (`ModeKind::Total |
  SuccessCount`) resolves a bare `t<N>` target's `Mode` when the notation has no explicit `cs`/
  `cf`; `direction` resolves `t<N>`'s comparator under SuccessCount-ambient context (`HighWins` ->
  `Gte`, `LowWins` -> `Lte` — the composer never specifies the comparator via `t`) and seeds
  `RollSpec::direction`. Under Total-ambient context, `t<N>` resolves to `TotalConfig.difficulty`
  instead. Explicit `cs`/`cf` in the notation always forces `SuccessCount` regardless of the
  ambient `mode`. A `t<N>` + explicit `cs`/`cf` together is a collision —
  `ParseError::DuplicateSuccessRule` (shared parser state: `success`/`t_target` are one `RollSpec`,
  not per-`DiceGroup`). SuccessCount with NEITHER a `cs`/`cf` rule nor a `t<N>` target is a hard
  parse error. The lexer is **case-insensitive on the `d` dice operator** and enforces
  **ASCII-only input** as an explicit precondition (not an accident of the byte-as-char cast it
  uses internally). The parser also rejects `sides < 1` (`ParseError::InvalidDieSides`) before ever
  constructing a `DieKind::Numeric`.

## Hard invariants

- **`roll` is the ONLY randomness step; `evaluate` is pure/deterministic.** Given the same
  `(spec, raws)`, `evaluate` MUST return an identical `RollOutcome`. This is what makes a stored
  `RawRoll` (natural faces only, no seed) fully reproducible.
- **`oriented_margin` applies to Total mode ONLY; SuccessCount's margin is a plain subtraction,
  NEVER direction-flipped.** SuccessCount's per-die direction sensitivity is already baked in by
  `crit::score_die`/the success-rule comparator resolved at parse time; flipping the pooled margin
  again would double-apply direction. Any future change touching `success.rs`'s margin
  computation must preserve this asymmetry.
- **A `RollOutcome` reports EITHER `pass` (default 2-rung classification) OR a `tier`
  (`tier_label`/`tier_value`, custom ladder), never both** — `classify::classify` enforces this at
  the source; both modes rely on it unmodified.
- **`resolve_group`'s outer Explode loop must never re-scan a die it (or a sibling die's own
  chain) already pushed** — snapshot the pool length (`initial_len`) before the pass; the inner
  chain loop is the sole mechanism that extends any one die's own chain. Violating this caused a
  real 41GB-memory-blowup bug (fixed in M11a's Task 4 buddy-check) and would silently falsify
  `CHAIN_CAP`'s bound.
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
  `#[derive(TS)]`/ts-rs bindings yet (that's M11d's job, once real consumers exist).

## Gotchas

- **The plan text and the real code can drift** — this module was built through several
  buddy-check fix rounds that changed signatures after the plan's own example code was written.
  Always read the actual current file before assuming a plan snippet's exact signature.
- **No per-roll dice-count cap exists yet.** `DiceGroup.count: u32` is unbounded; `RawRoll::next_id`
  (a `u32` counter) has no overflow guard; `eval::sum::fold`'s `i64` total sum is theoretically
  overflowable for a pathological `count`. All tracked in `docs/TODO.md` "Server / dice (M11a)",
  deferred to whatever cap Task 9/M11d establishes at the transport boundary — not yet built.
- **`ParseError` messages are raw `{:?}` Debug output** (e.g. `"Some(BangP), expected int"`) — not
  yet player-presentable. Needs a `Display` impl for `Token` before M11c/d surfaces parse errors
  directly in a chat UI (logged in `docs/TODO.md`).
- **The notation-level `cs`/`cf` tokens and `SuccessConfig.crit_success`/`crit_fail` are two
  unrelated mechanisms that happen to share initials.** `cs`/`cf` in a dice-notation string set
  the ordinary per-die `SuccessRule` (or its M11a-era inverted-comparator `cf` approximation);
  they do NOT construct a `CritSuccess`/`CritFail` struct. Today, crit events are configurable
  only by authoring a `RollSpec`/`SuccessConfig` directly — no notation syntax exposes them yet.
- **Expertise DP (M11b-2) is the highest-risk piece of the whole engine** — the design's own
  standing directive is buddy-check + differential-oracle verification against a brute-force
  reference before merge. Do not treat it as routine pipeline work when it lands.
- **Buddy-check track record in this module**: all three of M11a's plan's pre-approved
  buddy-check tasks (group pipeline, group_index fold, recalculate) found and fixed real
  Critical/Important bugs — this is dense, easy-to-get-subtly-wrong pipeline logic; treat any
  future change to `eval/groups.rs`, `eval/sum.rs`, `eval/success.rs`, `eval/classify.rs`,
  `eval/crit.rs`, or `recalc.rs` as buddy-check-worthy by default.

## Pointers

- Rationale: `docs/superpowers/specs/2026-07-04-m11b-system-rules-design.md` (M11b full design:
  expertise/crit/tiers/labeled+custom-face dice/direction); `docs/superpowers/specs/2026-07-03-m11-dice-engine-design.md`
  (original M11a+b scope split); `docs/superpowers/plans/2026-07-04-m11b-1-globals-classification-crit.md`
  (this checkpoint's task-by-task plan); `docs/superpowers/plans/2026-07-03-m11a-dice-engine-core.md`
  (M11a implementation plan + Buddy-check/Model-effort directives).
- Deferred work: `docs/TODO.md` "Server / dice (M11a)" section (dice-count cap, serde-defaults,
  `Token` Display impl — several already marked RESOLVED as later tasks closed them). Expertise DP
  = M11b-2 (not yet started); labeled dice + custom-face/symbolic dice = M11b-3 (not yet started).
- M11 milestone context (dice + chat, parallel to the M10 movement/vision track): memory
  `m11-dice-chat-resume` in the project's auto-memory.
