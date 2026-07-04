# M11b System Rules — Design

**Status:** Approved design (brainstorm complete), pre-plan.
**Scope:** M11b only — the nine system rules layered on the M11a dice-engine core.
**Builds on:** M11a (merged, stable). Parent cross-cutting design:
[`2026-07-03-m11-dice-engine-design.md`](2026-07-03-m11-dice-engine-design.md).
**Consumer:** M11d wires dice to transport/chat and adds ts-rs bindings; **not** in scope here.

This document supersedes several framing choices in the cross-cutting spec where they conflict
with long-term durability + notation usability. Each supersession is consented (the user
explicitly directed "churn is not a concern; long-term durability and usability is — make the best
choice") and is called out in §3 so the deviation is auditable, per the project's strict
spec-adherence discipline.

## 1. Build decomposition

M11b bundles six rule families of very different risk and blast radius. Following the M10f
precedent (decompose a phase-sized milestone into sequenced checkpoints, isolate the risky piece
for buddy-check), M11b builds as **three sequenced checkpoints**, each its own plan → build →
merge → codebase-skill gate:

| Checkpoint | Contents | Risk |
|---|---|---|
| **M11b-1** | `direction` global flip; the `Mode` data-model refactor (§4); the shared classification layer — `difficulty` / `required_successes` + `tiers` + pass/tier logic (§5, §6); crit events + counters (§7) | Medium — pervasive but mechanical |
| **M11b-2** | Expertise DP (§8) — the provably-optimal allocator, in isolation | **Highest in the whole engine** — differential oracle + buddy-check mandatory |
| **M11b-3** | Die-model rules: labeled dice + custom-face (symbolic) dice (§9) | Low / Medium — orthogonal to modes |

Notation extensions (§10) land alongside the checkpoint that introduces each construct
(`t<N>`/tiers with b-1, `4d6e3` expertise with b-2, `[label]`/face-lists with b-3).

## 2. What M11a already ships (the stable base this extends)

The canonical types are stable, buddy-checked, and merged. M11b extends them; it does not
re-litigate them.

```rust
// spec.rs (M11a — current)
enum DieKind { Numeric { min: i32, max: i32 } }
enum Mode { Sum, SuccessCount }                       // unit enum, Copy
struct SuccessRule { comp: Comparator, target: i32 }
struct RollSpec {
    expr: Expr,
    mode: Mode,
    success: Option<SuccessRule>,      // "required when SuccessCount" — a runtime convention
    required_successes: Option<i32>,
}
// outcome.rs (M11a — current)
struct RollOutcome { total: i64, records: Vec<DieRecord>,
                     successes: Option<i32>, pass: Option<bool>, net_margin: Option<i32> }
struct DieRecord { id, group_index, natural, value, kept, exploded, rerolled_from }
```

`roll` (only RNG step), `evaluate` (pure), `recalculate` (targeted ops → re-derive) are unchanged
in contract; M11b changes what `evaluate` reads and emits, not the three-function shape.

## 3. Consented deviations from the cross-cutting spec

1. **`Mode` becomes a data-carrying enum** (§4), not the parent spec's unit enum with parallel
   optional top-level fields. Rationale: make illegal states unrepresentable; stop the
   bag-of-optionals growth as rules are added.
2. **A single shared `difficulty` field does *not* drive all three modes** (parent §2.5). In
   Total mode `difficulty` is a total-level reference; in SuccessCount the analogous knob is a
   *per-die comparator+threshold* (`SuccessRule`) — a different shape that needs its comparator.
   Forcing one `i32` loses information. "Difficulty/target" is unified at the **notation + UI**
   layer (§10), not as one struct field.
3. **Sum and Tiered are one mode** (`Mode::Total`), not two (parent §2.4's three-mode table).
   Sum is the degenerate ladder-less case of tiering a total. See §6.
4. **Tiering is a mode-orthogonal classification**, applied to *both* modes over a
   mode-appropriate margin — not a Total-only feature. See §5.

The parent spec's architecture invariants (server-authoritative, struct-canonical, reproducible
without a seed, pure library / no wire frames in M11a-b, differential-oracle for expertise) are
**unchanged and binding**.

## 4. Foundational: `direction` global + data-carrying `Mode`

```rust
struct RollSpec {
    expr: Expr,
    direction: Direction,      // NEW — global, default HighWins; read by all modes + expertise
    mode: Mode,                // now carries each mode's own configuration
}

enum Direction { HighWins, LowWins }        // default HighWins

enum Mode {
    Total(TotalConfig),                     // subsumes former Sum AND Tiered (§6)
    SuccessCount(SuccessConfig),
}
```

`Mode` is no longer `Copy` (holds `Vec<Tier>`); it is `Clone`. `evaluate` dispatches by matching
the variant. `roll` and `recalculate` are structurally unaffected (they walk `Expr`, which is
mode-independent).

**`direction` — the global mirror.** One `Direction` flips, consistently, every comparison that
has a "better" direction:

- the success comparator's default sense (`t<N>` → `>=` under HighWins, `<=` under LowWins);
- crit-threshold defaults (HighWins → `crit_success` at die-max, `crit_fail` at die-min; LowWins
  swaps them);
- expertise's "toward better" adjustment direction (§8);
- the classification margin sign (`margin` computed so that "better" is always more-positive).

WFRP (roll-under) is exactly `direction: LowWins` with no other special-casing.
**Property (b-1):** flipping `direction` and mirroring every die face (`f → min+max−f`) leaves the
classified outcome (pass / tier / net successes) invariant.

## 5. Shared classification: thresholds are always optional; tiers apply to both modes

Both modes produce a **scalar** and (optionally) a **margin**, then feed one shared classifier.
This is the structural core of the design.

| | Total mode | SuccessCount mode |
|---|---|---|
| scalar | `total` (fold the `Expr`) | net successes = base + crit-extras − crit-losses |
| reference | `difficulty: Option<i32>` | `required_successes: Option<i32>` |
| **margin** | `total − difficulty` | `net_successes − required_successes` |

**One shared classifier** — a pure function of `(margin, direction, tiers)`:

- reference `None` → **no classification**; report the raw scalar only
  (`2d6+3` = a bare total; a bare pool = a bare success count). `pass`/`tier` both `None`.
- reference `Some`, `tiers` empty → **default 2-rung ladder**: `pass = Some(margin ≥ 0)`
  (direction-aware), `tier` `None`. This is "success or failure."
- reference `Some`, `tiers` non-empty → `tier` = the highest rung with `margin_offset ≤ margin`;
  `pass` `None` (the tier is the result; a custom ladder assigns its own success semantics via
  `tier_value`/`label`, which the system interprets). The two classifier outputs are mutually
  exclusive: a roll reports *either* a `pass` *or* a `tier`, never both.

So `tiers: Vec<Tier>` lives in **both** configs; "Tiered" is not a mode. "Succeeding with 2 extra
successes" is a SuccessCount tier at `margin_offset = 2`, exactly parallel to a Total-mode
degrees-of-success ladder.

```rust
struct TotalConfig {
    difficulty: Option<i32>,        // reference; None ⇒ bare total
    tiers: Vec<Tier>,               // ladder over (total − difficulty)
}

struct SuccessConfig {
    success: SuccessRule,           // per-die target (comparator+threshold) — REQUIRED (no longer Option)
    required_successes: Option<i32>,// reference; None ⇒ bare success count
    tiers: Vec<Tier>,               // ladder over (net_successes − required_successes)
    crit_success: Option<CritSuccess>,   // §7
    crit_fail:    Option<CritFail>,      // §7
    expertise:    u32,                   // §8 (0 = off)
}

struct Tier {
    margin_offset: i32,             // threshold on margin
    label: Option<String>,
    tier_value: Option<i32>,        // e.g. degrees-of-success 0..3
}
```

The reference is **semantically named per mode** (`difficulty` vs `required_successes`) — they are
genuinely different quantities; only the `tiers` field name and the classifier *code* are shared.

**Ladder validation** (construction-time): `tiers` sorted ascending by `margin_offset`; the lowest
rung is the floor so *every* margin maps to exactly one tier (fail-closed to the lowest rung,
never "no tier"). Whole-roll tiering — one scalar → one tier (matches PF2e degrees of success:
one check → one degree); no per-die tiering.

## 6. Sum ≡ Tiered (why `Mode::Total` subsumes both)

Tiered already aggregates the total identically to Sum, then interprets it. The three familiar
shapes are one mechanism (fold → total → optionally classify):

- **Plain total** (`2d6+3`, damage) = `Total { difficulty: None, tiers: [] }`.
- **Hit/miss vs a target** (`1d20 t10` vs AC) = `Total { difficulty: Some(10), tiers: [] }` →
  the default 2-rung ladder → `pass`.
- **Degrees of success** (PF2e) = `Total { difficulty: Some(dc), tiers: [crit-fail … crit-success] }`.

Even Sum's would-be "crit band" is just a ladder rung. There is no case requiring distinct code
paths. `eval::sum::fold` (M11a) is retained as the Total-mode aggregation. "Sum" / "Tiered"
survive only as human-facing **system presets** (ladder-less vs laddered) over `Mode::Total`.
Canonical variant name: **`Total`** (it always yields a total; classification is optional).

## 7. Crit events (rules 2–5)

Two structs in `SuccessConfig`, each carrying a success delta *and* a counter delta:

```rust
struct CritSuccess { threshold: i32,     // default die-max (HighWins) / die-min (LowWins)
                     extra_successes: i32, // default 1
                     positive_counter: i32 } // default 0 — rule 4
struct CritFail    { threshold: i32,     // default die-min (HighWins) / die-max (LowWins)
                     lost: i32,           // default 1
                     negative_counter: i32,  // default 0 — rule 5
                     allow_negative: bool }  // default false
```

**Semantics.** A kept die reaching `crit_success.threshold` (direction-aware) adds
`extra_successes` beyond its base success and `positive_counter` to the positive tally; symmetric
for `crit_fail`. **Net successes** = base + Σ extras − Σ losses, clamped at 0 unless
`allow_negative`. **Counters are a separate output** — never folded into the success count.

Net successes feed the SuccessCount **margin** (§5), so crit extras can push the roll into a
higher success tier — the layers compose without special-casing.

## 8. Expertise DP (M11b-2 — highest-risk)

`expertise: u32` points, each adjusting **one ordered-numeric die's face by +1 toward "better"**
(direction-aware), clamped to `[min,max]`; points may stack on one die. The engine finds the
**provably-optimal** allocation under a lexicographic objective — never a greedy heuristic.

**Objective (lexicographic):** (1) maximize **net successes** (the §7 net formula — expertise is
crit-aware); (2) tie-break on maximum **net counters** (positive − negative). Carried as a
`(net_successes, net_counters)` pair compared lexically.

**Per-die value function** `v_i(k)` for `k` in `0..=expertise`: move the face
`f' = clamp(natural ± k, min, max)`, score that single die (success 0/1, crit deltas, counters) →
a `(successes, counters)` pair. `v_i` is nondecreasing in `k`, with jumps at the success and crit
thresholds, flat once clamped (further points on a maxed die are worth 0).

**DP:**
```
dp[j] = best (net_successes, net_counters) with ≤ j points over dice processed so far
for each contributing die i:
    dp'[j] = max over k in 0..=j of  dp[j-k] ⊕ v_i(k)      // ⊕ sums the pair; max is lexicographic
answer = dp[expertise]; backtrack argmax k per die for the audit trail
```

**Complexity: `O(N · E²)`** (`N` contributing dice, `E = expertise`). `E` is single digits and `N`
dozens in every real system → trivially cheap. We do **not** chase an `O(N·E)` convex-hull variant
(the value function is not guaranteed concave once two thresholds interact, and the speedup buys
nothing at these sizes). The parent spec's optimistic "`O(N·expertise)`-class" is corrected here to
`O(N·E²)`.

**Placement — resolving a spec tension.** The cross-cutting spec is internally inconsistent: §2.6
orders expertise *before* keep/drop, but §4's composition rule treats keep/drop as fixed and
allocates expertise among the contributing dice. We adopt the **composition-rule** reading (simpler;
these features essentially never co-occur — Age of Sigmar uses pools, not keep/drop):

- Expertise runs as a **new eval stage after `resolve_group` and after SuccessCount pools the kept
  dice across all groups, and before success/crit counting** — over the pooled **kept,
  ordered-numeric** dice. M11a's per-group `resolve_group` stays sealed. Lives in `eval::expertise`
  (called by `eval::success`).
- **Symbolic dice (`DieKind::Faces` with `value: None`) are excluded** — no ordering, no "+1".
- Expertise **mutates the die's `value`** (downstream counting sees the adjusted face) **and**
  records the per-die points in a new `DieRecord.expertise: i32` field (auditable — "rolled 4,
  +1 expertise → 5").

**Differential oracle (mandatory).** A brute-force **exhaustive** allocator (enumerate every
distribution of `E` points across `N` dice via stars-and-bars — feasible only because `E`,`N` are
tiny in tests; score each; take the lexicographic max) is the test oracle. Property test:
`DP == oracle` across a large random corpus (varying ranges, thresholds, crit configs, direction,
`expertise` 0..~6). Plus: expertise result ≥ greedy baseline, ≤ theoretical max, and the
direction-flip mirror symmetry. **b-2's plan pre-authorizes buddy-check on the DP task** (parent
spec §4.1 flags this as the single highest-risk piece of the whole engine; standing directive).

## 9. Die-model rules (M11b-3)

**Labeled dice (rule 6).** `label: Option<String>` on `DiceGroup`, propagated onto each
`DieRecord` it produces. `RollOutcome` exposes `by_label(&str) -> Vec<&DieRecord>` and a
comparison helper (which labeled group rolled higher). Engine exposes labeled results; the *system*
computes meaning (Daggerheart Hope/Fear). Any mode (orthogonal). Notation `1d12[Hope] + 1d12[Fear]`.

**Custom-face (symbolic) dice (rule 9).** Scope: **tallies + a contains-symbol predicate** (netting/
cancellation is left to the system to compute from exposed tallies — engine exposes, system decides).

```rust
enum DieKind {
    Numeric { min: i32, max: i32 },     // unchanged
    Faces   { faces: Vec<Face> },       // NEW
}
struct Face { value: Option<i32>, symbols: Vec<Symbol> }
type Symbol = String;                   // opaque tag; system assigns meaning
```

- **RNG:** a `Faces` die draws a uniform face **index** `0..faces.len()`; `RawDie.natural` stores
  that index (for `Numeric` it stays the face value — same field reused). The outcome derives
  `value`/`symbols` by looking the index up in `kind.faces`.
- **SuccessCount over symbols:** the per-die success/crit test extends to a **symbol predicate**
  ("face contains symbol X") alongside the numeric comparator. `RollOutcome` gains per-symbol
  tallies (`symbol_counts: BTreeMap<Symbol, i32>` over kept dice).
- **Guards:** a `Faces` die with `value: None` is unordered → excluded from expertise and
  keep/drop-highest/lowest (which require ordered numeric); those stages skip it, fail-closed. A
  `Faces` die *with* values participates numerically where ordering is defined.

## 10. Notation — one target vocabulary across all modes (pillar)

**Hard requirement:** a composer writing `1d20 t10` means *"roll a d20, target number 10"* and it
means the same thing regardless of mode. Composers never learn a per-mode word for "the target
number." The struct stays honest per-mode; the notation front-end lowers a unified vocabulary to
the active mode's field:

| Notation | Total mode | SuccessCount mode |
|---|---|---|
| `1d20 t10` | `TotalConfig.difficulty = Some(10)` | `SuccessConfig.success = {comp, target: 10}` |
| meaning | total vs 10 | each die vs 10 |

- The **comparator for `t<N>` comes from `direction`**, not the composer (HighWins ⇒ `>=`,
  LowWins ⇒ `<=`). So `1d20 t10` in a roll-under system means "succeed if ≤ 10" with no new syntax.
- **Mode is ambient.** `t<N>` is mode-agnostic, so mode is supplied to `parse` as caller/system
  context (Pathfinder → Total+ladder, WFRP → SuccessCount+LowWins, generic → Total). The identical
  string yields the correct per-mode config for the active system.
- The rpg-dice-roller explicit forms (`cs>=7` / `cf<=3`) remain available for SuccessCount when a
  composer wants an explicit comparator; `t<N>` is the shorthand that *also* covers Total (where
  `cs`/`cf` never applied). Exact reconciliation of `t` vs `cs`/`cf` when both appear, and any
  optional explicit mode-override token, is pinned down in the b-1 plan; the *pillar* (unified
  target vocabulary, direction-derived comparator, ambient mode) is locked here.
- Struct-heavy specs (full tier ladders, custom face-lists) that don't fit linear notation are
  configured **struct-directly** by systems (parent §5). Expertise `4d6e3`; labels `1d12[Hope]`.

**Property (b-1):** for a fixed dice expression + target, switching only the ambient mode never
changes what "the target" *is* — only how it's used.

## 11. `RollOutcome` — final shape

All additions default-empty so a mode that doesn't use a field is unaffected. `margin` generalizes
M11a's `net_margin` (now populated by both modes per §5).

```rust
struct RollOutcome {
    total: i64,                         // always (Total scalar; SuccessCount reference kept-sum)
    records: Vec<DieRecord>,
    successes: Option<i32>,             // SuccessCount net successes
    margin: Option<i32>,                // §5 classified margin (both modes)
    pass: Option<bool>,                 // default 2-rung classifier result
    tier_label: Option<String>,         // ladder result
    tier_value: Option<i32>,
    crit_successes: i32, crit_fails: i32,
    positive_counter: i32, negative_counter: i32,
    symbol_counts: BTreeMap<Symbol, i32>,
    // by_label(&str) provided as a method over `records`
}

struct DieRecord {  // M11a fields + :
    // …id, group_index, natural, value, kept, exploded, rerolled_from,
    crit_success: bool, crit_fail: bool,
    expertise: i32,                     // points allocated (§8)
    label: Option<String>,             // §9
    symbols: Vec<Symbol>,              // resolved for Faces dice; empty for Numeric
}
```

## 12. Evaluation pipeline (updated)

Per group (`resolve_group`, unchanged M11a): reroll → explode → keep/drop.
Then per mode:

- **Total:** `eval::sum::fold` → `total` → classify `(total − difficulty, tiers)` (§5).
- **SuccessCount:** pool kept dice across groups → **expertise DP** (§8, adjusts values) → count
  base successes + crit deltas + counters (§7) + symbol tallies (§9) → net successes → classify
  `(net_successes − required_successes, tiers)` (§5).

All comparisons read `direction` (§4).

## 13. Testing strategy

Pure library, exhaustive unit + property tests (no wire frames). Per checkpoint:

- **b-1:** unit tests per classifier branch (none/pass-fail/tier) in both modes; direction-flip
  mirror-symmetry property; crit net-success/counter arithmetic; ambient-mode notation invariance
  (§10 property); `t<N>` → correct per-mode field.
- **b-2:** the differential oracle (§8) — the highest-value test in M11b; greedy-baseline and
  theoretical-max bounds; expertise↔crit↔tier composition (maximizing net successes maximizes the
  reached tier).
- **b-3:** labeled `by_label`/comparison; symbolic face-index RNG uniformity; contains-symbol
  predicate + `symbol_counts`; expertise/keep-drop exclusion guard for unordered faces.

Existing M11a property tests (successes ≤ dice count except crit extras; recalc empty-ops identity;
replace-then-replace-back round-trip) are extended to cover the new fields.

## 14. Deferred (unchanged from parent §9)

- ts-rs bindings + wire frames → M11d (pure library until a real consumer exists).
- Arbitrary-code modder roll handlers (declarative config is the extension surface).
- GIF / animated dice; any client-side eval engine.

## 15. Codebase-skill gate

Each checkpoint updates `shadowcat-codebase-dice` (new mode model, classification layer, crit,
expertise stage, symbolic/labeled dice) and is reviewed by `shadowcat-spec-reviewer` as part of the
reviewed skill-update gate before merge — same tier as the doc-sync gate.
