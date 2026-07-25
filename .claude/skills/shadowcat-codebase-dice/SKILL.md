---
name: shadowcat-codebase-dice
description: "Use when touching Shadowcat's dice engine: RollSpec/Expr AST, the seeded-noise RNG, roll/evaluate/recalculate, the per-group reroll/explode/keep-drop pipeline, group_index-based Total folding, SuccessCount aggregation, group_spans-based recalculation, the shared classify/crit layers, the dice notation lexer/parser, or the chat wire boundary that executes untrusted notation (caps/entropy/validate in chat/rolls.rs — co-owned with shadowcat-codebase-chat). Covers src/server/src/dice/. Invoke shadowcat-codebase-core first."
---

# Shadowcat — Dice Engine

Orientation for the server-authoritative dice engine: a pure Rust library (still no ws/data/http
imports) whose ONLY untrusted entry is the notation parser, now wired to chat. M11a shipped the
core evaluator (RNG, per-group pipeline, Sum/SuccessCount modes, notation, recalculate).
**M11b-1 shipped** the `direction` global flip, a data-carrying `Mode`, the shared
classification layer, crit events, and unified `t<N>` notation. **M11b-2 shipped** the
expertise-point DP allocator (buddy-checked, differential-oracle-verified) and `e<N>` notation.
**M11b-3 shipped** labeled dice, custom-face (symbolic) dice, the `is_ordered` pipeline gate,
`SuccessRule`/`CritTrigger` as enums, `symbol_counts`, and Numeric-only expertise — **M11b
fully DONE**. **M11d-2 shipped the transport boundary**: `chat/rolls.rs` (OUTSIDE this crate —
see `shadowcat-codebase-chat`) executes untrusted notation at chat ingest behind caps
(`MAX_ROLL_DICE=100`, `MAX_ROLL_RECORDS=1000`, `MAX_EXPERTISE=100` — true DP worst case is
records-bounded, ~1000·100² ≈ 1e7 ops — `MAX_DIE_SIDES=10_000`, `MAX_INLINE_ROLLS=8`),
per-roll OS-entropy seeds (`Uuid::new_v4` fold), and the first production `DieKind::validate()`
caller; this crate gained the matching hardening — `#[serde(default)]` on every optional
`RollSpec`-reachable field, `RawRoll::push` `checked_add` id guard, saturating arithmetic in
`eval::sum::fold` (per-group sum AND every `Expr::Bin` Add/Sub/Mul arm — unbounded `Const`
terms/`*` chains were deterministically overflowable with zero dice, a buddy-check Critical),
and player-presentable `Display` for `ParseError`/`Token` (surfaced via chat System notices).
The recursive-descent parser has NO depth counter — callers rely on their input-length cap
(documented on `struct P`; chat's `MAX_MESSAGE_CHARS=4096` ≈ 2k nesting levels, safe on all
three target OSes' default stacks). Ambient `ParseContext` for chat rolls comes from the
world's `dice-settings` config doc (`chat/settings.rs::resolve_dice_context`, fail-closed
Total/HighWins, GM-authored in `module-game-settings`'s Dice section).

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
- `src/server/src/dice/spec.rs` — the canonical AST: `DieKind::Numeric{min,max}` |
  `Faces{faces: Vec<Face>}` (M11b-3, custom/symbolic dice). `Face{value: Option<i32>, symbols:
  Vec<Symbol>}` (`Symbol = String`, an opaque system-assigned tag, e.g. Genesys "triumph");
  `value: None` means the face has no numeric meaning, only symbols. `DieKind::validate() ->
  Result<(), DieKindError>` rejects `Faces{faces: []}` (`DieKindError::EmptyFaces`) — called in
  production by `chat/rolls.rs::validate_pre_roll` on every parsed group (M11d-2).
  `recalc::RecalcOp::ReplaceDie` onto a `Faces` die is bounds-checked at `recalc.rs` (Phase A): an
  out-of-range `natural` (negative or `>= faces.len()`) is silently ignored rather than written,
  matching an unknown `id`'s existing no-op semantics — closes the index-out-of-bounds panic
  surface `face_value_and_symbols` would otherwise hit. `DieKind::is_ordered()`: `Numeric` always
  `true`; `Faces` is
  `true` iff EVERY face has `value: Some` — a single unordered face makes the whole die unrankable
  against a valued sibling. `Comparator`+`test` (`#[derive(Default)] #[default] Gte`), `ExplodeKind`
  (Standard/Compound/Penetrate), `GroupModifier`, `DiceGroup{count, kind, modifiers, label:
  Option<String>}` (**modifiers apply in Vec order — caller-controlled**, e.g. reroll-then-keep vs
  keep-then-reroll are different specs; `label` is an M11b-3 tag propagated onto every `DieRecord`
  this group produces, including exploded/penetrated children — read by
  `RollOutcome::by_label`/`compare_labels`, orthogonal to mode; duplicate labels across groups are
  NOT an error, they pool under `by_label`), `Expr` (Dice(DiceGroup)/Const(ConstTerm)/Bin/Neg).
  `ConstTerm{value: i32, label: Option<String>}` mirrors `DiceGroup`'s label field onto a bare
  constant — the parser's label-consumption (`take_label()`) applies to EITHER atomic factor
  (a `DiceGroup` or a `Const`), not only dice groups; a labeled constant is Total/Sum-mode
  display-only provenance (surfaced via `RollOutcome.labeled_consts`, see below) and never feeds
  `by_label`/`compare_labels` (SuccessCount dice-pool comparison has no pool for a bare constant
  to join). `Direction`
  (`HighWins`/`LowWins`, `#[default] HighWins`) is a global flip on `RollSpec::direction` orienting
  every margin/tier/crit computation. `Tier{margin_offset, label, tier_value}` is one rung of a
  classification ladder, shared by both modes. `CritTrigger` (M11b-3, replaces M11b-1's bare
  `threshold: i32`): `AtLeast(i32)` (direction-aware, byte-for-byte the old bare-threshold logic)
  | `HasSymbol(Symbol)` (direction-**insensitive** — presence/absence has no "better end" to flip).
  `CritSuccess{trigger: CritTrigger, extra_successes, positive_counter}` /
  `CritFail{trigger: CritTrigger, lost, negative_counter, allow_negative}` are SuccessCount-only
  crit-event configs. `TotalConfig{difficulty: Option<i32>, tiers: Vec<Tier>}` — Sum and Tiered are
  now ONE mode: empty `tiers` = default pass/fail at `margin >= 0`; non-empty = a custom ladder.
  `SuccessRule` (M11b-3, was a struct, now an enum): `Numeric{comp: Comparator, target: i32}`
  (`#[default]` via a HAND-WRITTEN `impl Default` — `derive(Default)`'s `#[default]` attribute only
  targets a fieldless variant, `Numeric` carries fields) | `HasSymbol(Symbol)`.
  `SuccessConfig{success: SuccessRule, required_successes: Option<i32>, tiers: Vec<Tier>,
  crit_success: Option<CritSuccess>, crit_fail: Option<CritFail>, expertise: u32}` — the
  per-roll expertise-point budget (0 = disabled). `Mode` is **data-carrying**:
  `Total(TotalConfig) | SuccessCount(SuccessConfig)` (replaces M11a's unit `Mode::Sum |
  Mode::SuccessCount`). `RollSpec{expr, direction: Direction, mode: Mode}`.
- `src/server/src/dice/outcome.rs` — `RawDie`, `RawRoll` (`dice`, `records`, `next_id`,
  `group_spans`), `DieRecord` (`id`, `group_index`, `natural`, `value`, `kept`, `exploded`,
  `rerolled_from`, `crit_success: bool`, `crit_fail: bool`, `expertise: i32`, `label:
  Option<String>`, `symbols: Vec<Symbol>`, `ordered: bool`). `expertise` is the audit trail of
  points spent adjusting this die's `value` (0 if none/not applicable). `label` (M11b-3) is copied
  from the producing `DiceGroup.label`, `None` if unlabeled. `symbols` (M11b-3) is the resolved
  symbols for a `Faces` die's drawn face; always empty for `Numeric`. `ordered` (M11b-3, added in a
  post-ship final-review fix) is a per-record snapshot of the producing group's
  `DieKind::is_ordered()` at construction time, stamped at every `DieRecord` construction site
  (mirrors `label`/`symbols` propagation exactly, including exploded/penetrated children); `#[serde
  (default = "default_ordered")]` defaults deserialized/legacy records to `true`. It exists solely
  so `compare_labels` can detect an unordered (symbolic) label — this can NOT be inferred from
  `value` alone, since a genuine ordered value of `0` is indistinguishable from an unordered die's
  derived-`0` fallback. `RollOutcome`'s final shape: `total: i64`, `records`, `successes:
  Option<i32>`, `pass: Option<bool>`, `margin: Option<i64>` (renamed + widened from M11a's
  `net_margin`), `tier_label: Option<String>`, `tier_value: Option<i32>`, `crit_successes: i32`,
  `crit_fails: i32`, `positive_counter: i32`, `negative_counter: i32`, `symbol_counts:
  BTreeMap<Symbol, i32>` (M11b-3 — per-symbol tallies over KEPT dice, computed UNCONDITIONALLY
  inside `evaluate_success`'s per-die loop regardless of which `SuccessRule` variant is active;
  empty in Total mode), `labeled_consts: Vec<ConstTerm>` (M13d — every labeled bare `Const`
  reachable in the expression, Total-mode only; `#[serde(default)]` so pre-M13d stored messages
  still deserialize; `evaluate_success` always sets this to `Vec::new()` since SuccessCount
  ignores all AST arithmetic; display-only — NOT read by `by_label`/`compare_labels`; the
  chat wire mirror (`chat-docs.ts` Zod schema) and `MessageCard.svelte` render a labeled const's
  raw `value` the same way a labeled `DiceGroup`'s die faces are shown, including under an
  enclosing `Neg`/`Mul` where the displayed value does NOT reflect the operator — same
  precedent as `DieRecord`'s raw face values ignoring an enclosing sign) (all 0/None/empty in
  Total mode with no `difficulty`, or in SuccessCount with no crit config).
  `RollOutcome::by_label(&self, label: &str) -> Vec<&DieRecord>` (M11b-3)
  returns all records — kept AND dropped — carrying that label, in roll order.
  `RollOutcome::compare_labels(&self, a, b) -> Option<Ordering>` (M11b-3) compares two labels by
  the sum of their KEPT records' `value`s; `None` iff either label has zero matching records at
  all, OR either label's matching records include ANY with `ordered: false` (a mixed
  ordered+unordered pool under one label also has no well-defined sum) — an all-dropped-but-
  ordered label still yields `Some(0)`, since the sum-of-kept is simply empty, not missing.
  `RollResult`.
- `src/server/src/dice/eval/groups.rs` — `resolve_group(group, group_index, naturals, rng, raws)
  -> Vec<DieRecord>`: the per-group pipeline (reroll → explode → keep/drop, in modifier-Vec
  order). `face_value_and_symbols(kind, natural) -> (i32, Vec<Symbol>)` derives a die's
  `(value, symbols)`: `Numeric` passes `natural` straight through; `Faces` treats `natural` as a
  face INDEX and looks up `faces[natural]` (`None`-value face contributes `0` numerically). The
  ENTIRE modifier loop is gated by `let ordered = group.kind.is_ordered(); if !ordered { continue;
  }` (M11b-3) — an unordered `Faces` group (any face with `value: None`) skips reroll/explode/
  keep/drop entirely for every modifier, fail-closed. A shared `redraw` closure dispatches the RNG
  draw per `DieKind` (a numeric face for `Numeric`, a face INDEX via `roll_uniform(rng, 0,
  faces.len()-1)` for `Faces`). Explode's `Compound`/`Penetrate` arms are guarded by `matches!
  (group.kind, DieKind::Numeric { .. })` — restricted to `Numeric` only; an ordered `Faces` die
  falls through to the shared push-new-die path (Standard-style) instead, since "add"/"−1" have no
  defined meaning over an arbitrary face-list. The Explode chain's retrigger check tests the
  die's DERIVED value (via `face_value_and_symbols`), never the raw redrawn index/face — for
  `Numeric` this is identical to the raw face (a pure pass-through), but for an ordered `Faces` die
  whose index doesn't track value monotonically, checking the raw index would misfire (fixed
  during Task 6, a real bug). `push_extra` is the single construction site for a Penetrate child
  `DieRecord` (takes an `ordered: bool` param — reached only from the Numeric-guarded Penetrate
  arm, always `true`); the inlined Standard-explode/Faces-fallback arm (reached only inside the
  `!ordered { continue }`-gated modifier loop, so always `true` too) constructs its own `DieRecord`
  directly. `CHAIN_CAP = 100` bounds chained explosions/rerolls per die.
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
  positive_counter, negative_counter}` + `score_die(direction, value, symbols: &[Symbol], cfg:
  &SuccessConfig) -> DieCrit`: scores one kept die against `cfg.crit_success`/`cfg.crit_fail`
  independently via the shared `reaches(direction, value, symbols, trigger, is_success_event)`
  helper (M11b-3, replaces the old bare-comparator check) — `CritTrigger::AtLeast` flips under
  `LowWins` exactly as the old bare `threshold` did; `CritTrigger::HasSymbol` is direction-
  INSENSITIVE (`reaches`'s `HasSymbol` arm never reads `direction`). Both `is_success` and
  `is_fail` CAN be `true` on the same die under an overlapping-threshold OR overlapping-symbol
  config — intentional, tested (`overlapping_thresholds_fire_both_crit_success_and_crit_fail`,
  `overlapping_symbol_triggers_on_same_symbol_fire_both_crit_success_and_crit_fail`), not a bug.
  `DieScore{base_success: bool, crit: DieCrit}` + `.net() -> i32` (M11b-3, `eval::crit::
  score_die_net`) is the CENTRALIZED per-die net-success formula (`base + extra_successes −
  lost`): `score_die_net(direction, cfg, value, symbols) -> DieScore` computes both `cfg.success`'s
  base-success test AND `score_die`'s crit result together — shared by `success.rs`'s main pooling
  loop, `expertise.rs`'s `allocate` (the `fixed` term, see below), and `expertise.rs`'s test-only
  `score_pool` helper. `expertise.rs`'s `die_values` deliberately still inlines its own narrower
  version (a different shape — scoring a synthetic single-step candidate mid-DP, symbols always
  empty since expertise only ever adjusts Numeric dice).
- `src/server/src/dice/eval/expertise.rs` — the value-mutating pre-pass `allocate(direction,
  cfg: &SuccessConfig, raws: &RawRoll, records: &mut [DieRecord])`, called by
  `eval::success::evaluate_success` only when `cfg.expertise > 0`, BEFORE base-success counting
  (b-1's counting logic itself is unmodified/sealed). `adjust(direction, value, min, max, k)`
  moves a face up to `k` steps toward "better," stopping at the die's better-end bound
  (`max`/`min`) — provably preserves `adjust(_, v, _, _, 0) == v` for every `v`, INCLUDING values
  outside `[min,max]` (a Compound die's `value > max`, a Penetrate child's `value < min`), so an
  out-of-range face is never dragged back across a bound even at zero spend. `die_values(...) ->
  Vec<(i32,i32)>` builds the per-die `v_i(k)` table for `k in 0..=e`: each entry is
  `(net_i, counter_i)` from moving the face `k` steps then re-scoring via `crit::score_die` +
  `cfg.success`. `run_dp(dies, e, better) -> (Vec<u32>, (i32,i32))` is a bounded-knapsack DP,
  `O(N·E²)`, over an injected `better` ordering; ties break toward the SMALLEST `k` at each die,
  and backtracking runs from the LAST die outward so points concentrate on the earliest dice
  whenever spending is actually needed (R3 lowest-index-first). `allocate` runs `run_dp` up to
  TWICE (R1 two-pass clamp handling): pass 1 maximizes raw lexicographic `(net, counter)`; if
  `allow_negative` is unset and the achieved net is `< 1`, every allocation clamps to net 0 so a
  second counter-only pass replaces it (the all-failed-region fallback). Both passes mutate only
  the chosen dice's `value` (adjusted face) and `expertise` (points spent). **M11b-3 restricts
  `allocate`'s contributing-dice set to `Numeric` dice only** — the bounds map (`DieId ->
  (min,max)`) is built via `filter_map` over `raws.dice`, mapping only `DieKind::Numeric` entries;
  a kept `Faces` die (ordered or not) is excluded, since `adjust`'s "+1 toward better within
  [min,max]" has no defined meaning over an arbitrary face-list. A kept-but-excluded `Faces` die's
  own (unchangeable) success/crit contribution is folded in as a constant `fixed: i32` term (via
  `crit::score_die_net`) — **this fixes a real bug found during buddy-check**: the two-pass
  clamp-decision branch checks `allow_neg || net + fixed >= 1`, not just the DP's own Numeric-only
  `net`, because the pool's TRUE clamped net includes any fixed contribution from an excluded
  Faces die that independently satisfies success/crit rules; using only the partial `net` answers
  a different question than `evaluate_success` will actually score. `fixed` is a constant additive
  shift across every candidate allocation, so it never changes either DP pass's own argmax — only
  the pass-choice threshold needs it.
- `src/server/src/dice/eval/sum.rs` — `evaluate_total(spec, cfg: &TotalConfig, raws) ->
  RollOutcome`: folds the AST to a total by matching `DieRecord.group_index` against an AST-order
  cursor (`fold`); **the group-boundary reconstruction is the correctness core** — a wrong
  boundary silently mis-sums a multi-group roll. If `cfg.difficulty` is set, classifies via
  `oriented_margin` + `classify::classify`; otherwise reports a bare total (`pass`/`margin`/
  `tier_*` all `None`).
- `src/server/src/dice/eval/success.rs` — `evaluate_success(spec, cfg: &SuccessConfig, raws) ->
  RollOutcome`: if `cfg.expertise > 0`, first runs `eval::expertise::allocate` over a cloned
  `records` to mutate chosen dice's `value`/`expertise`, THEN pools **all kept records across
  every group** (ignores `group_index`/AST
  arithmetic entirely — the defining difference from Total mode), counts base successes against
  `cfg.success` (via the shared `crit::score_die_net`, M11b-3), then folds each kept die's crit
  result into net successes and the positive/negative counters (counters are a SEPARATE output,
  never folded into `successes`). The same per-die loop also tallies `symbol_counts` (M11b-3)
  UNCONDITIONALLY over every kept die's `symbols` — independent of which `SuccessRule` variant is
  active, so a Numeric-rule roll on symbolic dice still populates `symbol_counts`. Net
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
  `rederive`s by re-running `resolve_group` over the mutated naturals in AST order. `RecalcOp::
  RerollDice`'s redraw formula is `DieKind`-dispatched (M11b-3): a fresh numeric face for
  `Numeric`, a fresh face INDEX via `roll_uniform(rng, 0, faces.len()-1)` for `Faces` — mirrors the
  same formula used at `groups.rs`'s two other draw sites. `ReplaceDie`/`RemoveDice` needed no
  `Faces` change (both are `DieKind`-agnostic, operating only on `natural`/id).
- `src/server/src/dice/notation/{mod.rs,lexer.rs,parser.rs}` — `lex`/`Token`/`ParseError` +
  `parse(input: &str, ctx: ParseContext) -> Result<RollSpec, ParseError>` (recursive descent:
  `expr := term (('+'|'-') term)*`; `term := factor (('*'|'/') factor)*`; `factor := '(' expr ')'
  | '-' factor | dice | int`). `ParseContext{mode: ModeKind, direction: Direction}` is caller-
  supplied ambient state the notation string does not itself encode: `mode` (`ModeKind::Total |
  SuccessCount`) resolves a bare `t<N>` target's `Mode` when the notation has no explicit `cs`/
  `cf`; `direction` resolves `t<N>`'s comparator under SuccessCount-ambient context (`HighWins` ->
  `Gte`, `LowWins` -> `Lte` — the composer never specifies the comparator via `t`) and seeds
  `RollSpec::direction`. Under Total-ambient context, `t<N>` resolves to `TotalConfig.difficulty`
  instead. The parser's internal state (`struct P`, not `ParseContext`) also carries an
  `expertise: Option<u32>` roll-level scratch field set by an `e<N>` token (no dedicated lexer
  token — the alphabetic-run arm emits `Ident("e")`, the parser's `modifiers` arm reads the
  following int, the same function that handles `kh`/`cs`/`t`); a duplicate `e<N>` is
  `ParseError::DuplicateExpertise`. `expertise` is only consumed when the FINAL resolved mode is
  `SuccessCount(SuccessConfig{expertise, ..})` — if the notation instead resolves to `Total`
  (e.g. `t<N>` under Total-ambient context with no `cs`/`cf`), any parsed `e<N>` value is silently
  dropped, never an error. Explicit `cs`/`cf` in the notation always forces `SuccessCount` regardless of the
  ambient `mode`. A `t<N>` + explicit `cs`/`cf` together is a collision —
  `ParseError::DuplicateSuccessRule` (shared parser state: `success`/`t_target` are one `RollSpec`,
  not per-`DiceGroup`). SuccessCount with NEITHER a `cs`/`cf` rule nor a `t<N>` target is a hard
  parse error. The lexer is **case-insensitive on the `d` dice operator** and enforces
  **ASCII-only input** as an explicit precondition (not an accident of the byte-as-char cast it
  uses internally). The parser also rejects `sides < 1` (`ParseError::InvalidDieSides`) before ever
  constructing a `DieKind::Numeric`. **`[label]` notation (M11b-3)**: the lexer's `[` arm scans to
  the closing `]`, rejecting any byte that is neither `is_ascii_graphic()` (33-126) nor space
  (`ParseError::InvalidLabelChar`, catches control bytes and DEL/0x7F), an unterminated bracket
  (`ParseError::UnterminatedLabel`), or an all-whitespace/empty body after `.trim()`
  (`ParseError::EmptyLabel`); the trimmed, case-PRESERVING string becomes `Token::Label`. The
  parser's `factor` arm reads an optional trailing `Token::Label` onto the just-built `DiceGroup`
  (after its modifiers) — a per-group, not per-spec, field. Duplicate labels across different
  groups in the same notation string are NOT a parse error (they pool under `by_label`
  intentionally); only a duplicate `e<N>`/`t<N>`/`cs`/`cf` (shared roll-level state) errors.
  **(M13d)** label-consumption is a shared `take_label()` helper applied after EITHER atomic
  factor — a `DiceGroup` or a bare `Const` — not the `Dice` branch alone; a label immediately
  after a parenthesized/compound sub-expression is still correctly rejected as trailing input
  (the generalization is scoped to atomic factors only, not the whole grammar). This closed a
  real bug: `@shadowcat/formula`'s `resolveNotationTemplate` substitutes every resolved
  identifier as a labeled constant (`value[name]`) even with no dice roll present, and the
  parser previously rejected any such label not immediately adjacent to a dice group.

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
- **An Explode retrigger check on an ordered `Faces` die must test the die's DERIVED value
  (`face_value_and_symbols`), never the raw drawn face INDEX** (M11b-3) — a face-list where index
  doesn't track value monotonically (e.g. index 0 -> value 6, index 1 -> value 1) would otherwise
  test the comparator against the wrong number and silently truncate the chain. Fixed during Task
  6 as a real bug; for `Numeric` this is a no-op distinction since `face_value_and_symbols` is a
  pure pass-through there.
- **Reroll and Explode must skip `!kept` dice.** Since modifiers apply in Vec order, a
  Drop-then-Reroll sequence is legal and must not mutate an already-dropped die.
- **`resolve_group`'s ENTIRE modifier loop must be gated by `DieKind::is_ordered()`** (M11b-3) — an
  unordered `Faces` group (any face with `value: None`) has no rankable value, so every
  value-reading modifier (reroll/explode-by-comparator, keep/drop) must be a no-op for that group,
  not a ranking-by-`0`-default accident. `Numeric` is always ordered; a `Faces` group is ordered
  iff EVERY face has `value: Some`.
- **Expertise's `allocate` must restrict its contributing-dice set to `Numeric` dice only, folding
  any excluded kept `Faces` die's contribution in as a constant `fixed` term on the pass-choice
  threshold** (M11b-3) — `adjust`'s face-move has no defined meaning over a `Faces` die's arbitrary
  face-list, but a kept-but-excluded `Faces` die's own (unchangeable) success/crit score still
  counts toward the TRUE pool-wide net that decides whether the two-pass clamp fork triggers.
  Checking only the DP's own Numeric-only partial `net` (omitting `fixed`) answers a different
  question than `evaluate_success` will actually score — a real bug found and fixed via
  buddy-check. `fixed` never needs to enter the DP's own per-allocation comparisons (a constant
  shift never changes an argmax), only the pass-choice threshold.
- **`resolve_group`/`push_extra` must stamp every produced `DieRecord` (including exploded/
  penetrated children) with the CALLER-SUPPLIED `group_index`** — `eval::sum::fold`'s per-group
  folding depends on every record in a group carrying that group's own index, never a stale/wrong
  one.
- **`compare_labels` must treat ANY unordered record under a label as making that label's sum
  undefined** (`DieRecord.ordered: bool`, M11b-3) — checking only `recs.is_empty()` (the absent
  case) is insufficient; an unordered symbolic label's records derive `value` via
  `face_value_and_symbols`'s `unwrap_or(0)`, so omitting the ordered check silently returns
  `Some(0)`/`Some(sum)` instead of `None` for the exact Daggerheart Hope/Fear headline case the
  design doc calls out. A label spanning multiple `DiceGroup`s where even one group is unordered
  must also yield `None` for that label (a partial pool with any unordered member has no
  well-defined sum) — this was a real gap that shipped past Task 2 (written before Task 6
  introduced `is_ordered`) and was caught only by the whole-branch final review, not any
  per-task check.
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
- **Pure library — `dice` must never depend on `ws`/`data`/`http`/`scene`.** Still NO wire
  frames and NO `#[derive(TS)]`/ts-rs bindings even after M11d-2: roll outcomes ride the
  opaque chat `system` body (`Segment::RollEmbed{formula, outcome}`) and the client mirrors
  them by hand in `chat-docs.ts` Zod (`RollOutcomeSchema`/`DieRecordSchema`) — a shape change
  to `RollOutcome`/`DieRecord` MUST update that mirror, not regenerate a binding. All
  transport policy (caps, entropy, settings, error surfacing) lives in `chat/rolls.rs`, never
  here.
- **Expertise optimizes the CLAMPED (visible) net successes, with a counter-max fallback in the
  all-failed region.** `eval::expertise::allocate` maximizes raw lexicographic `(net, counter)`
  first; only when that raw net is `< 1` AND `allow_negative` is unset (every allocation clamps to
  net 0, so successes tie) does it re-run the DP with counters as the sole objective. A future
  change must preserve this two-pass fork, not just always maximize raw net.
- **`adjust` preserves `v_i(0) = value` for out-of-range (Compound/Penetrate) faces.** A naive
  `clamp(value ± k, min, max)` would drag an already-out-of-range die (a Compound's `value > max`,
  a Penetrate child's `value < min`) back across the bound even at `k = 0`; `adjust` instead moves
  by `k.min((bound - value).max(0))`, which is a no-op whenever the die is already past its
  better-end bound.
- **The expertise DP allocation is deterministic and oracle-verified.** `run_dp`'s tie-break
  (smallest `k` wins per die, backtrack from the last die) is pinned against a brute-force
  reference (`oracle` in `expertise.rs`'s test module) over a 4000-case deterministic pseudo-random
  corpus varying direction/target/crit config/`allow_negative`/e/n — both the objective value AND
  the exact per-die allocation must match. Any future change to the tie-break or the DP recurrence
  must re-run this oracle test, not just check the objective value.

## Gotchas

- **The plan text and the real code can drift** — this module was built through several
  buddy-check fix rounds that changed signatures after the plan's own example code was written.
  Always read the actual current file before assuming a plan snippet's exact signature.
- **The crate's OWN types stay uncapped by design — the caps live at the transport boundary**
  (M11d-2): `DiceGroup.count` is still an unbounded `u32` inside the pure library; anything
  reaching `roll()`/`evaluate()` from untrusted input MUST come through
  `chat/rolls.rs::execute_roll`/`validate_formula` (the cap walk + `DieKind::validate()`
  caller). A future second transport must reuse or replicate that boundary, never call
  `notation::parse` + `roll` bare. Overflow is defense-in-depth-guarded crate-side
  (`RawRoll::push` checked id increment; saturating folds in `eval/sum.rs` incl. every
  `Expr::Bin` arm).
- **`ParseError`/`Token` implement player-presentable `Display`** (M11d-2) — chat System
  notices surface them directly; a new variant MUST get a clean `Display` arm (pinned by the
  no-debug-artifacts test iterating every variant), never a `{:?}` payload.
- **The notation-level `cs`/`cf` tokens and `SuccessConfig.crit_success`/`crit_fail` are two
  unrelated mechanisms that happen to share initials.** `cs`/`cf` in a dice-notation string set
  the ordinary per-die `SuccessRule` (or its M11a-era inverted-comparator `cf` approximation);
  they do NOT construct a `CritSuccess`/`CritFail` struct. Today, crit events are configurable
  only by authoring a `RollSpec`/`SuccessConfig` directly — no notation syntax exposes them yet.
- **Expertise DP (M11b-2) was the highest-risk piece of the whole engine** — it shipped only after
  buddy-check + differential-oracle verification against a brute-force reference (see the Hard
  invariants entry above). Treat any future change to `eval/expertise.rs` as buddy-check-worthy by
  default, same tier as `eval/groups.rs`/`eval/sum.rs`/`eval/success.rs` below.
- **`e<N>` is roll-level and silently discarded under Total mode.** Mirrors the existing `t<N>`-
  vs-mode gotcha: `e<N>` sets the parser's internal `struct P.expertise` scratch field, but that
  value is only ever read into `SuccessConfig.expertise` when the resolved `Mode` is
  `SuccessCount`. A notation string like
  `4d6t10e3` under Total-ambient context (`t<N>` resolves to `TotalConfig.difficulty`, not a
  success target) parses successfully and simply drops the `e3` — no `ParseError`, no warning.
- **Buddy-check track record in this module**: all three of M11a's plan's pre-approved
  buddy-check tasks (group pipeline, group_index fold, recalculate) found and fixed real
  Critical/Important bugs — this is dense, easy-to-get-subtly-wrong pipeline logic; treat any
  future change to `eval/groups.rs`, `eval/sum.rs`, `eval/success.rs`, `eval/classify.rs`,
  `eval/crit.rs`, `eval/expertise.rs`, or `recalc.rs` as buddy-check-worthy by default. M11b-3's
  own pre-approved buddy-check (Task 9, reopening the sealed crit-scoring path for `CritTrigger`)
  and the independent Task 11 review (expertise's Numeric-only restriction) each found and fixed
  a real bug too — see the `fixed`-term and derived-value-retrigger Hard invariants above.
- **`DieKind::validate()` is enforced at the wire boundary, not inside `roll()`** (M11d-2):
  `chat/rolls.rs::validate_pre_roll` calls it per parsed group before any rolling, so an
  empty-`faces` die can no longer arrive via chat (notation still can't construct `Faces`
  anyway). The crate itself remains unvalidated by design — any future non-chat caller that
  hand-builds a `RollSpec` must run the same validation. `ReplaceDie`-onto-`Faces` is separately
  bounds-checked inside `recalc.rs` itself (Phase A, see the `spec.rs` entry above), so it needed
  no wire-boundary gate.
- **`validate_tiers` (`chat/rolls.rs`, Phase A) guards `SuccessConfig`/`TotalConfig.tiers`
  uniqueness at the wire boundary**, ahead of any untrusted construction path existing —
  `classify::classify`'s `max_by_key`/`min_by_key` tie on a duplicate `margin_offset` is
  caller-order-dependent (documented on `classify.rs`), so a malformed ladder with a repeated
  offset would otherwise resolve nondeterministically. `validate_pre_roll` calls it on every
  parsed spec's tiers; `RollError::DuplicateTierOffset(i32)` is the player-presentable rejection.
  Notation still cannot author a non-empty ladder today (`parser.rs` emits `tiers: vec![]`), so
  this guard arms the boundary before the construction path exists, mirroring the
  `DieKind::validate()` precedent above.
- **`compare_labels` returns `Some(0)`, not `None`, for an all-dropped-but-ordered label.** `None`
  means the label has ZERO matching records at all, OR at least one matching record is unordered
  (see the Hard invariants entry above); a label whose records all exist, are all ordered, but are
  all `kept: false` still sums to `0` over an empty kept-subset, which is `Some(0)` — distinct from
  "label doesn't exist" or "label is unordered."

## Pointers

- Rationale: `docs/superpowers/specs/2026-07-04-m11b-system-rules-design.md` (M11b full design:
  expertise/crit/tiers/labeled+custom-face dice/direction); `docs/superpowers/specs/2026-07-03-m11-dice-engine-design.md`
  (original M11a+b scope split); `docs/superpowers/specs/2026-07-07-m11b-3-labeled-custom-face-dice-design.md`
  (M11b-3 design); `docs/superpowers/plans/2026-07-04-m11b-1-globals-classification-crit.md`
  (M11b-1 task-by-task plan); `docs/superpowers/plans/2026-07-04-m11b-2-expertise-dp.md` (M11b-2
  plan); `docs/superpowers/plans/2026-07-07-m11b-3-labeled-custom-face-dice.md` (M11b-3 plan,
  including its buddy-check directive for Task 9); `docs/superpowers/plans/2026-07-03-m11a-dice-engine-core.md`
  (M11a implementation plan + Buddy-check/Model-effort directives).
- Deferred work: `docs/TODO.md` "Server / dice (M11a)" section (dice-count cap, serde-defaults,
  `Token` Display impl — several already marked RESOLVED as later tasks closed them), a dedicated
  `SuccessConfig.expertise` bounding entry (M11b-2, unbounded `E` is an `O(N·E²)` DoS/wrap-around
  vector), and the `DieKind::Faces` empty-face-list panic-surface gap (M11b-3) — all three deferred
  to the M11d untrusted-transport boundary alongside the dice-count cap. Expertise DP = M11b-2;
  labeled + custom-face dice = M11b-3. **M11b is now fully shipped** (M11a + M11b-1 + M11b-2 +
  M11b-3); M11c (chat core)/M11d (transport, default display modules) remain.
- M11 milestone context (dice + chat, parallel to the M10 movement/vision track): memory
  `m11-dice-chat-resume` in the project's auto-memory.
