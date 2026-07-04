# M11 Dice Engine — Design

**Status:** Approved design (brainstorm complete), pre-plan.
**Scope:** M11a + M11b (the whole dice engine — core + the nine system rules) as one spec.
**Chat (M11c/d) is out of scope here** — it is the consumer that later wires dice to transport.

## 1. Context & decomposition

M11 as scoped by the user is phase-sized: two large subsystems where one depends on the other.

- **Dice** — a self-contained evaluation engine (data model + evaluator + RNG + recalculation)
  plus nine system-specific rule families that must be *aware of each other*.
- **Chat** — a document/transport/enrichment/display stack whose *roll integration* depends on
  dice, but whose core (doc model, channels, sanitization) does not.

Agreed decomposition (each sub-milestone: its own spec → plan → build):

| Sub | Title | Depends on |
|-----|-------|-----------|
| **M11a** | Dice engine core (Rust) | — |
| **M11b** | System rule extensions | M11a |
| **M11c** | Chat core (headless) | — (parallel-able) |
| **M11d** | Default chat display module | M11a/b, M11c |

This document covers **M11a + M11b only**. It also refines the roadmap: M12's "chat panel" line
is superseded — the baseline chat display module lands in M11d.

### Decided architecture (foundational, not re-litigated below)

1. **Server-side Rust, authoritative.** RNG and evaluation live server-side. Rolls are
   request→execute→broadcast, **never optimistic** (a VTT cannot trust client rolls). This mirrors
   the gated-movement precedent. Resolves the tension between "custom roll handlers for modders"
   and the two hard invariants (*server-authoritative*; *server runs no third-party code*): the
   engine is a declarative evaluator, and modders/systems configure it via **data**, not code.
2. **Struct-canonical.** The canonical roll is a serializable `RollSpec`. String notation is one
   parser front-end that produces a `RollSpec`. Storage, recalculation, and modder config all
   operate on the struct.
3. **Reproducible without a seed.** `RollResult` retains every individual die's raw face value and
   a stable identity; that is sufficient to reproduce and to recalculate a subset.
4. **Notation parity target:** the rpg-dice-roller (GreenImp, MIT) grammar as the standard
   baseline, **clean-room reimplemented in Rust**, extended with custom notation for our rules.
5. **Verification:** M11a/b ship as a **pure library verified by exhaustive unit + property
   tests** — no wire frames. Rolls first surface end-to-end in M11c/d as chat messages. No
   throwaway transport is built.

## 2. Core data model

The user's mental model ("parameters as a struct, rolling and calculating as functions") maps to
three types and three pure functions.

### 2.1 The three types

- **`RollSpec`** — the canonical, serializable *parameters* object. Everything a roll *is*,
  independent of what it rolled: dice groups, arithmetic, keep/drop, exploding/reroll rules, the
  aggregation mode, the two global modifiers (`direction`, `difficulty`), and any enabled system
  rules. Notation parses *into* this; systems/modders configure *this*; this is stored and
  re-run by recalculation.
- **`RawRoll`** — the RNG outputs only: each die's natural rolled face, tagged with a stable
  `DieId`, its `DieKind`, and an optional `label`. The **sole** nondeterministic artifact.
- **`RollOutcome`** — fully derived: per-die final values/flags, total, net successes, crit flags,
  tier, counters, symbol tallies, labeled-die lookups.

`RollResult = { spec: RollSpec, raws: RawRoll, outcome: RollOutcome }`. `RollSpec`, `RawRoll`, and
`RollOutcome` get **ts-rs** bindings so the client can render an outcome. The client never rolls.

### 2.2 The pure functions

```
roll(spec, rng)              -> raws               // the ONLY RNG step
evaluate(spec, raws)         -> outcome            // pure, deterministic
recalculate(spec, raws, ops) -> (raws', outcome')  // ops reroll/replace a subset, then re-evaluate
```

### 2.3 Unified die model

Numeric and symbolic dice under one type:

- `DieKind::Numeric { min, max }` — `d6` = `{1, 6}`; also covers fudge / arbitrary ranges.
- `DieKind::Faces { faces: Vec<Face> }` — explicit face list. Each `Face { value: Option<i32>,
  symbols: Vec<Symbol> }` carries an optional ordinal value **and** a set of arbitrary resource
  tags. Star Wars Shatterpoint custom dice are pure-symbol faces (`value: None`).

`Numeric` is conceptually sugar for contiguous valued faces; it is retained as a compact,
range-friendly form for the common case.

**Ordering constraint:** expertise and keep/drop-highest/lowest require **ordered numeric dice** —
a pure-symbol face has no "+1" and no ordering. Symbolic dice still participate in success-counting
via symbol predicates (§4.5).

### 2.4 Aggregation modes

A `RollSpec` selects exactly one mode. This is the spine the nine rules hang off.

| Mode | Produces | Mode-specific rules |
|------|----------|--------------------|
| **Sum** | numeric total (+ optional target/crit bands) | standard math / keep / drop / explode |
| **SuccessCount** | net success count (+ optional pass/fail & net margin) + ± counters | expertise, crit_success / crit_fail, per-die target (required) + `required_successes` (optional) |
| **Tiered** | tier label + tier count | tier ladder (margin vs difficulty) |

**Labeled dice** (§4.3) and **custom-face dice** (§4.5) are **orthogonal** — they work in any mode.

### 2.5 Global modifiers (read by all three modes)

- **`direction: HighWins | LowWins`** (default `HighWins`). One flag flips every comparison:
  success comparator, crit-threshold defaults, expertise direction, tier-margin sign, and Sum-mode
  target comparison. WFRP (roll-under) sets `LowWins`.
- **`difficulty: Option<i32>`** — optional in all modes, applied per mode:
  - **Sum:** `total` compared against `difficulty` (roll-under/over, direction-aware); drives
    Sum-mode success/crit bands. Omit → plain total.
  - **SuccessCount:** difficulty has **two dimensions** — (1) the **per-die target** (the number a
    die must roll to score a success), which is **required** in this mode, and (2) the **required
    success count** (`required_successes`, the number of successes the pool needs to pass overall),
    which is **optional**. Dimension 1 is the global `difficulty` field applied per-die; dimension
    2 is a SuccessCount-only field. When `required_successes` is present, `RollOutcome` reports an
    overall **pass/fail** (net successes ≥ required) and a **net margin** (net successes − required,
    i.e. net hits); when absent, the outcome is just the success count.
  - **Tiered:** the ladder reference point; `margin = total − difficulty` (sign flips under
    `LowWins`).

### 2.6 Evaluation pipeline (order matters)

1. Roll natural faces (`roll`, consumes RNG).
2. Apply reroll / explode rules (may add or replace dice; consumes RNG).
3. Expertise DP allocation (§4.1) — crit/tier-aware; sets each die's expertise delta.
4. Keep / drop selection (highest/lowest).
5. Arithmetic modifiers (per-die and total).
6. Aggregate per mode, reading `direction` and `difficulty`.
7. Emit `RollOutcome`.

**Documented composition rule:** expertise optimizes over the kept/contributing set. Expertise and
keep/drop rarely co-occur (Age of Sigmar uses pools, not keep/drop); when both are present,
keep/drop selection is treated as fixed and expertise is allocated among the contributing dice.

## 3. Recalculation

Because `RawRoll` holds every die's natural face under a stable `DieId`, and `RollSpec` is
retained, recalculation is pure:

```
recalculate(spec, raws, ops) -> (raws', outcome')
```

`ops` is a list of **targeted** operations:

- `RerollDice([DieId])` — draw fresh faces for exactly these dice from the server RNG.
- `ReplaceDie(DieId, Face)` — set a specific face (e.g. GM override).
- `AddDice([DieSpec])` / `RemoveDice([DieId])`.

Everything else re-runs `evaluate`. The user's "reroll a number of dice equal to the failures"
flow is: the caller inspects `outcome` for failed `DieId`s → issues `RerollDice(thoseIds)`. **The
engine never decides policy** (which/when to reroll) — it executes targeted ops and recomputes.
Each die carries a reroll provenance chain, so rerolls stay auditable, and new faces always come
from the server RNG (server-authoritative).

## 4. The nine system rules

Every rule is a **declarative field** on `RollSpec`, not code. They are independent and they
stack. This is the modder/system extension surface.

| # | Rule (system) | Mode | Field / behavior |
|---|---------------|------|------------------|
| 1 | Expertise (Age of Sigmar) | SuccessCount | `expertise: u32` — see §4.1 |
| 2 | Extra success on crit | SuccessCount | `crit_success { threshold, extra_successes }` |
| 3 | Lost success on crit-fail | SuccessCount | `crit_fail { threshold, lost, allow_negative }` |
| 4 | Extra + counter on crit | SuccessCount | `crit_success.positive_counter` |
| 5 | Extra − counter on crit-fail | SuccessCount | `crit_fail.negative_counter` |
| 6 | Labeled dice (Daggerheart) | any | per-die `label` + outcome lookup helpers |
| 7 | Tiered successes (Pathfinder) | Tiered | `tiers` ladder |
| 8 | Roll-lower-for-success (WFRP) | all | `direction: LowWins` (global, §2.5) |
| 9 | Custom-face dice (Shatterpoint) | any | `DieKind::Faces` + symbol predicates |

Detail subsections below are numbered contiguously; the "(rule N)" tag maps each back to the
table. Rule 8 needs no subsection — it is the global `direction` flag, covered in §2.5.

### 4.1 Expertise optimizer — rule 1 (highest-risk piece)

`expertise: u32` points, each adjusting one ordered-numeric die's result by 1 toward "better"
(up for `HighWins`, down for `LowWins`), clamped to the die's face range.

**Objective — lexicographic (approved):**
1. Primary: maximize **net successes** = base successes + crit-success extras − crit-fail losses.
2. Tie-break: among all equally-max-success allocations, prefer the best **net counters**
   (positive − negative).

**Algorithm — provably optimal, not heuristic.** Each die's contribution is a step function of the
points allocated to it (nondecreasing, with jumps at the success and crit thresholds). Compute
`value(die, k)` for `k` in `0..=expertise`, then a bounded DP distributes the `expertise` points
across dice for the maximum total value under the lexicographic objective. `O(N · expertise)`-class
— exact and cheap. The lexicographic tie-break is carried as a `(successes, counters)` pair
compared lexically inside the DP.

### 4.2 Crit events — rules 2–5

Rules 2–5 collapse into **two structs**, each carrying its success delta *and* its counter delta,
so "extra success on crit" and "extra + counter on crit" are two fields of one crit event:

```
crit_success { threshold = die-max*, extra_successes = 1, positive_counter = 0 }
crit_fail    { threshold = die-min*, lost = 1, negative_counter = 0, allow_negative = false }
```

(*defaults flip under `LowWins`: crit_success→die-min, crit_fail→die-max.) A die at/beyond
`crit_success.threshold` contributes `extra_successes` beyond its base success and adds
`positive_counter` to the positive tally; symmetrically for `crit_fail`. Net successes clamp at 0
unless `crit_fail.allow_negative`. Counters are a **separate output** from successes.

### 4.3 Labeled dice — rule 6

Any die (or die group) may carry a `label`. `RollOutcome` exposes `by_label(l) -> [DieRoll]` and
comparison helpers (which labeled die is higher). The engine exposes labeled results and
comparisons; the *system* computes meaning (Daggerheart Hope/Fear). Notation: `1d12[Hope] +
1d12[Fear]`.

### 4.4 Tiered successes — rule 7

`tiers`: an ordered ladder of `{ margin_offset, label: Option<String>, tier_value: Option<i32> }`
evaluated on `margin = total − difficulty` (sign flips under `LowWins`). Supports labels and/or
tier counts (both). Example Pathfinder-like ladder: crit-fail at −10, fail `< 0`, success `≥ 0`,
crit-success at `+10` — fully configurable.

### 4.5 Custom-face dice — rule 9

`DieKind::Faces`. The result is the symbols on the rolled faces. In SuccessCount mode, success and
crit predicates are defined over **symbol sets** (e.g. "face contains ⚔" = success) rather than a
numeric compare; `RollOutcome` aggregates symbol tallies. Expertise does not apply (unordered).

## 5. Notation

Standard grammar = the rpg-dice-roller superset, clean-room reimplemented in Rust:
`kh`/`kl`/`dh`/`dl`, `!`/`!!`/`!p` (explode/compound/penetrate), `r`/`ro` (reroll/reroll-once),
`cs>=`/`cf<=` (success/failure counting), sorting, math functions.

Custom extensions use bracket/suffix syntax — labels `1d12[Hope]`, expertise `4d6e3`, difficulty,
`direction`. **Struct-heavy specs** (full tier ladders, custom face-lists) that do not fit linear
notation are configured **struct-directly** by systems; notation covers the common inline cases,
and the struct is always canonical.

## 6. File layout

`src/server/src/dice/` in the `shadowcat` crate — pure, **no `ws` / `data` dependencies**:

```
dice/
  mod.rs          // public API: RollSpec, RollResult, roll(), evaluate(), recalculate()
  spec.rs         // RollSpec + die model + rule fields (ts-rs)
  outcome.rs      // RollOutcome + per-die records (ts-rs)
  notation/       // tokenizer + grammar → RollSpec
  eval/           // pipeline stages: reroll/explode, expertise DP, keep-drop, aggregate
  rng.rs          // RngSource trait (CSPRNG in prod, seedable in tests)
```

## 7. Testing strategy

The pure-library choice is justified by testability:

- **Unit tests** per rule and per notation construct.
- **Property tests** (`proptest`) over random specs + seeds:
  - successes never exceed dice count (except via crit extras);
  - expertise result ≥ greedy baseline and ≤ theoretical max;
  - `direction` flip is a mirror symmetry;
  - recalc with empty `ops` is identity;
  - replace-a-die then replace-it-back round-trips.
- **Differential oracle for the expertise DP** — a brute-force exhaustive optimal allocator as the
  oracle, asserted equal to the DP across small random cases (the movement-executor oracle pattern
  applied to the highest-risk piece).

## 8. Milestone split (one spec, sequential build)

- **M11a** — data model, RNG, notation parser, **Sum + SuccessCount** modes, standard rules,
  recalculation. All three types + both pure functions land here.
- **M11b** — the nine system rules: expertise DP, `crit_success`/`crit_fail` + counters, **Tiered**
  mode + ladders, labeled dice, custom-face dice, `direction` global flip, custom notation
  extensions.

## 9. Deferred (noted, not built)

- Arbitrary-code modder roll handlers — declarative config is the v1 extension surface; code
  handlers would break server-authority.
- GIF / animated dice.
- Any client-side preview/eval engine — the client only renders server outcomes.

## 10. Codebase-skill gate

M11 opens a subsystem no existing `shadowcat-codebase-*` skill covers. Per CLAUDE.md, a new
**`shadowcat-codebase-dice`** skill must be created (fixed shape; globs added to the activation
hook) and reviewed as part of the skill-update gate before merge.
