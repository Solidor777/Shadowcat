# M11b-2 Expertise DP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a provably-optimal expertise-point allocator to the SuccessCount dice mode — a bounded dynamic program that maximizes the visible (clamped) net successes, tie-breaking on net counters, proven correct against a brute-force differential oracle.

**Architecture:** Expertise is a value-mutating **pre-pass** (`eval::expertise::allocate`) that runs inside `eval::success::evaluate_success` after the kept dice are pooled and before the existing success/crit counting loop. The DP chooses each kept die's point count `k`, mutates that die's `value` to the adjusted face, and records `k` in `DieRecord.expertise`; the sealed M11b-1 counting loop then reproduces the DP's optimum from the mutated faces with no change. Notation `4d6e3` is lowered to a roll-level `SuccessConfig.expertise` in the parser (no lexer change — the existing identifier arm already tokenizes `e` + int).

**Tech Stack:** Rust (Cargo), `serde`. Pure library — no `ws`/`data`/`http`/`scene` dependency, no ts-rs bindings (deferred to M11d). Tests are `#[cfg(test)]` unit + property tests in-module.

## Global Constraints

- **Pure `dice` library** — `eval::expertise` must not depend on `ws`/`data`/`http`/`scene`; no wire frames, no `#[derive(TS)]`. (ARCHITECTURE §2; dice skill "Hard invariants".)
- **`roll` is the only randomness step; `evaluate` is pure/deterministic** — given the same `(spec, raws)`, `evaluate` MUST return an identical `RollOutcome`. Expertise runs inside `evaluate` (no RNG); its allocation must be a deterministic function of `(spec, raws)`. (dice skill "Hard invariants".)
- **Server tests require `dist/` built first** — `rust-embed` validates `../../dist/` at compile time, so `cargo test` on the server crate needs the client built once. Run `pnpm build` from the repo root before the first `cargo test`. ([[embed-dist-compile-ordering]])
- **`oriented_margin` applies to Total mode ONLY; SuccessCount's margin is never direction-flipped** — expertise lives entirely in SuccessCount; direction is applied per-die (via the face adjustment + `crit::score_die` + the parse-time comparator), never to the pooled margin. Do not add any `oriented_margin` call in this checkpoint.
- **Run cargo commands from `src/server/`.** Test filter form: `cargo test dice::eval::expertise` (module) or `cargo test dice::eval::expertise::tests::<name>` (single test).
- **Rust edition / style:** `cargo fmt` clean, `cargo clippy` warning-free before each commit.

---

## File Structure

- **Create** `src/server/src/dice/eval/expertise.rs` — the allocator: `adjust` (toward-better-bounded face move), `die_values` (per-die value function `v_i(k)`), `run_dp` (the bounded knapsack DP), `allocate` (clamp handling + backtrack + mutate), and the differential oracle + property tests. One responsibility: optimal expertise allocation.
- **Modify** `src/server/src/dice/spec.rs` — add `expertise: u32` to `SuccessConfig`.
- **Modify** `src/server/src/dice/outcome.rs` — add `expertise: i32` to `DieRecord`.
- **Modify** `src/server/src/dice/eval/groups.rs` — set `expertise: 0` at the two `DieRecord` construction sites (`resolve_group` initial map + `push_extra`).
- **Modify** `src/server/src/dice/eval/mod.rs` — register `pub mod expertise;`.
- **Modify** `src/server/src/dice/eval/success.rs` — call `expertise::allocate` before the counting loop; add integration tests.
- **Modify** `src/server/src/dice/notation/parser.rs` — collect roll-level `expertise`, lower per R4.
- **Modify** `src/server/src/dice/notation/mod.rs` — add `ParseError::DuplicateExpertise`.
- **Modify** `src/server/src/dice/notation/lexer.rs` — **no code change**; add one test documenting that `4d6e3` already lexes to `Ident("e") + Int`.

---

## Model/Effort directives

- **Plan authored** mainline on **Opus 4.8 / effort high** per the user's explicit "no model switch" directive at plan-writing time (already the recommended plan-writing tier; no dispatch to `sdd-plan-writer-*`).
- **Execution tiers** (per project CLAUDE.md "Model/Effort Tiering"): implementation via `shadowcat-coder` (Sonnet 5 / effort medium); each review checkpoint via the `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` pair (effort high). Escalate a BLOCKED/uncertain result to the `-opus` twin before the human. **Task 3 escalates to `shadowcat-coder-opus` on any BLOCKED**, given it is the highest-risk task in the engine.

## Buddy-check directives

- **Task 3 (the DP + differential oracle) is pre-authorized for a full `buddy-checking` pass** — two independent blind reviewers + brokered debate — per the standing §8/§4.1 directive naming expertise the single highest-risk piece of the whole dice engine. This is not optional for Task 3.
- The buddy-check must specifically verify: (a) the two-pass clamp handling (§R1) matches the oracle in the all-failed region; (b) the R3 lowest-index-first tie-break is what the DP actually produces (not just claimed); (c) the oracle itself is a faithful brute force (a buggy oracle gives false confidence — see [[fix-confirmation-catches-real-gaps]] and the M11b-1 Task 6 lesson where a green property test still didn't close the gap).
- All other tasks follow the standard two-reviewer gate. Treat any later change to `eval/expertise.rs` as buddy-check-worthy by default (dice skill "Gotchas").

---

## Task 1: Struct fields — `SuccessConfig.expertise` + `DieRecord.expertise`

**Files:**
- Modify: `src/server/src/dice/spec.rs` (SuccessConfig ~150-161)
- Modify: `src/server/src/dice/outcome.rs` (DieRecord ~48-62)
- Modify: `src/server/src/dice/eval/groups.rs` (`resolve_group` ~32-42, `push_extra` ~174-184)
- Test: the existing suite recompiles green (no behavior change)

**Interfaces:**
- Produces: `SuccessConfig.expertise: u32` (0 = off); `DieRecord.expertise: i32` (points allocated; 0 when none). Both `#[serde(default)]` so a missing field deserializes to 0 (design §11 "additions default-empty").

- [ ] **Step 1: Add `expertise: u32` to `SuccessConfig`**

In `src/server/src/dice/spec.rs`, replace the `// NOTE: ... M11b-2` comment line inside `SuccessConfig` with the field:

```rust
    pub crit_success: Option<CritSuccess>,
    pub crit_fail: Option<CritFail>,
    /// Expertise budget (M11b-2): points distributed across the pooled kept dice
    /// to maximize net successes (tie-break net counters). 0 = off.
    #[serde(default)]
    pub expertise: u32,
}
```

- [ ] **Step 2: Add `expertise: i32` to `DieRecord`**

In `src/server/src/dice/outcome.rs`, add the field at the end of `DieRecord` (after `crit_fail`):

```rust
    pub crit_success: bool,
    pub crit_fail: bool,
    /// Expertise points allocated to this die by `eval::expertise` (M11b-2);
    /// 0 for every die when the roll has no expertise budget. Audit trail:
    /// `value` is the post-expertise face, `natural`/base `value` the pre-expertise one.
    #[serde(default)]
    pub expertise: i32,
}
```

- [ ] **Step 3: Set `expertise: 0` at every `DieRecord` construction site**

In `src/server/src/dice/eval/groups.rs`, add `expertise: 0,` to both `DieRecord { .. }` literals — the `resolve_group` initial `.map(|d| DieRecord { .. })` (after `crit_fail: false,`) and `push_extra`'s `recs.push(DieRecord { .. })` (after `crit_fail: false,`).

- [ ] **Step 4: Compile and fix every remaining literal the compiler flags**

Run: `cargo build` (from `src/server/`)
Expected: compile errors listing each remaining `SuccessConfig`/`DieRecord` literal missing the new field — in `spec.rs` tests, `eval/crit.rs` tests (`cfg` helper), `eval/success.rs` tests (`pool`, `manual_raws`, and the inline configs), and `notation/parser.rs` (`Mode::SuccessCount(SuccessConfig { .. })` production site — add `expertise: 0,` there for now; Task 5 wires it to the parsed value).
Add `expertise: 0,` to each flagged `SuccessConfig` literal and `expertise: 0,` to each flagged `DieRecord` literal until `cargo build` is clean.

- [ ] **Step 5: Run the full suite to confirm no behavior change**

Run: `cargo test dice`
Expected: PASS — every pre-existing dice test still green (the new fields default to 0 and are not yet read).

- [ ] **Step 6: Commit**

```bash
cargo fmt && cargo clippy --all-targets
git add src/server/src/dice/spec.rs src/server/src/dice/outcome.rs src/server/src/dice/eval/groups.rs src/server/src/dice/eval/crit.rs src/server/src/dice/eval/success.rs src/server/src/dice/notation/parser.rs
git commit -m "feat(dice/m11b-2): add expertise fields to SuccessConfig and DieRecord"
```

---

## Task 2: The face-adjust + per-die value function

**Files:**
- Create: `src/server/src/dice/eval/expertise.rs`
- Modify: `src/server/src/dice/eval/mod.rs` (add `pub mod expertise;`)
- Test: `src/server/src/dice/eval/expertise.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `SuccessConfig` (Task 1), `crit::score_die(Direction, i32, &SuccessConfig) -> DieCrit`, `Comparator::test(i32, i32) -> bool`, `Direction`.
- Produces (private to the module):
  - `fn adjust(direction: Direction, value: i32, min: i32, max: i32, k: u32) -> i32` — move `value` toward "better" by up to `k` steps, bounded by the die's better-end face, never moving worse. **Preserves `adjust(_, v, _, _, 0) == v` for ALL `v`**, including `v` outside `[min,max]` (Compound over-max, Penetrate below-min).
  - `fn die_values(direction: Direction, cfg: &SuccessConfig, value: i32, min: i32, max: i32, e: u32) -> Vec<(i32, i32)>` — `v[k] = (net_i(k), counter_i(k))` for `k in 0..=e`, where `net_i = base_success + crit_extra − crit_lost` and `counter_i = positive_counter − negative_counter`.

- [ ] **Step 1: Register the module**

In `src/server/src/dice/eval/mod.rs`, add `pub mod expertise;` in the module list (alphabetical, between `crit` and `groups`):

```rust
pub mod classify;
pub mod crit;
pub mod expertise;
pub mod groups;
pub mod success;
pub mod sum;
```

- [ ] **Step 2: Write the failing tests for `adjust`**

Create `src/server/src/dice/eval/expertise.rs` with the module skeleton and the `adjust` tests:

```rust
use crate::dice::eval::crit;
use crate::dice::spec::{Direction, SuccessConfig};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dice::spec::{Comparator, CritFail, CritSuccess, SuccessRule};

    #[test]
    fn adjust_preserves_value_at_zero_points_for_all_ranges() {
        // In-range, over-max (Compound), below-min (Penetrate) — k=0 is a no-op each way.
        for &v in &[4, 15, 0, 1, 6] {
            assert_eq!(adjust(Direction::HighWins, v, 1, 6, 0), v);
            assert_eq!(adjust(Direction::LowWins, v, 1, 6, 0), v);
        }
    }

    #[test]
    fn adjust_moves_up_toward_max_under_highwins() {
        assert_eq!(adjust(Direction::HighWins, 4, 1, 6, 1), 5);
        assert_eq!(adjust(Direction::HighWins, 4, 1, 6, 3), 6); // capped at max
        assert_eq!(adjust(Direction::HighWins, 4, 1, 6, 9), 6); // over-budget still capped
    }

    #[test]
    fn adjust_moves_down_toward_min_under_lowwins() {
        assert_eq!(adjust(Direction::LowWins, 4, 1, 6, 1), 3);
        assert_eq!(adjust(Direction::LowWins, 4, 1, 6, 5), 1); // floored at min
    }

    #[test]
    fn adjust_never_pulls_an_out_of_range_value_back_across_a_bound() {
        // Compound over-max under HighWins: already above the max ceiling, no up-move.
        assert_eq!(adjust(Direction::HighWins, 15, 1, 6, 3), 15);
        // Penetrate below-min under LowWins: already below the min floor, no down-move.
        assert_eq!(adjust(Direction::LowWins, 0, 1, 6, 3), 0);
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test dice::eval::expertise::tests::adjust`
Expected: compile error — `adjust` not defined.

- [ ] **Step 4: Implement `adjust`**

Add above the `#[cfg(test)]` module:

```rust
/// Move `value` toward "better" (higher under `HighWins`, lower under `LowWins`)
/// by up to `k` steps, stopping at the die's better-end face (`max` / `min`).
///
/// INVARIANT: `adjust(_, v, _, _, 0) == v` for every `v`, INCLUDING values outside
/// `[min, max]`. A naive `clamp(v ± k, min, max)` would violate this — a Compound
/// die (`value > max`) would be pulled down to `max`, a Penetrate die (`value < min`)
/// pushed up to `min`, even at `k == 0`. The `max(0, headroom)` term makes the move
/// zero when the die is already at/past its better-end bound, so an out-of-range face
/// is never dragged back across a bound; expertise only ever moves a die toward better.
fn adjust(direction: Direction, value: i32, min: i32, max: i32, k: u32) -> i32 {
    let k = k as i32;
    match direction {
        Direction::HighWins => value + k.min((max - value).max(0)),
        Direction::LowWins => value - k.min((value - min).max(0)),
    }
}
```

- [ ] **Step 5: Run the `adjust` tests to verify they pass**

Run: `cargo test dice::eval::expertise::tests::adjust`
Expected: PASS (4 tests).

- [ ] **Step 6: Write the failing tests for `die_values`**

Add to the `mod tests` block:

```rust
    fn cfg(cs: Option<CritSuccess>, cf: Option<CritFail>) -> SuccessConfig {
        SuccessConfig {
            success: SuccessRule { comp: Comparator::Gte, target: 5 },
            required_successes: None,
            tiers: vec![],
            crit_success: cs,
            crit_fail: cf,
            expertise: 0,
        }
    }

    #[test]
    fn die_values_zero_point_scores_current_face() {
        // value 4, target >=5: not a success at k=0; +1 gets to 5 (success).
        let c = cfg(None, None);
        let v = die_values(Direction::HighWins, &c, 4, 1, 6, 2);
        assert_eq!(v[0], (0, 0)); // face 4: no success, no crit
        assert_eq!(v[1], (1, 0)); // face 5: base success
        assert_eq!(v[2], (1, 0)); // face 6: still one base success (no crit configured)
    }

    #[test]
    fn die_values_folds_crit_extra_and_counters() {
        // crit_success at 6: +2 extra successes, +1 positive counter.
        let c = cfg(
            Some(CritSuccess { threshold: 6, extra_successes: 2, positive_counter: 1 }),
            None,
        );
        let v = die_values(Direction::HighWins, &c, 4, 1, 6, 2);
        assert_eq!(v[0], (0, 0)); // face 4
        assert_eq!(v[1], (1, 0)); // face 5: base success only
        assert_eq!(v[2], (3, 1)); // face 6: base 1 + crit extra 2 = 3 net; +1 counter
    }

    #[test]
    fn die_values_lowwins_moves_toward_low_target() {
        // LowWins, success at <=2 (comp set by caller/parse). face 4 -> down.
        let mut c = cfg(None, None);
        c.success = SuccessRule { comp: Comparator::Lte, target: 2 };
        let v = die_values(Direction::LowWins, &c, 4, 1, 6, 3);
        assert_eq!(v[0], (0, 0)); // face 4: > 2, no success
        assert_eq!(v[1], (0, 0)); // face 3: still > 2
        assert_eq!(v[2], (1, 0)); // face 2: success
        assert_eq!(v[3], (1, 0)); // face 1: success
    }
```

- [ ] **Step 7: Run to verify failure**

Run: `cargo test dice::eval::expertise::tests::die_values`
Expected: compile error — `die_values` not defined.

- [ ] **Step 8: Implement `die_values`**

Add below `adjust`:

```rust
/// Per-die value function `v_i(k)` for `k in 0..=e`: move the face `k` steps toward
/// better, then score that single face — base success (0/1) + crit deltas — as a
/// `(net_i, counter_i)` pair. `net_i = base + extra_successes − lost`;
/// `counter_i = positive_counter − negative_counter`. Direction is applied ONCE here
/// (via the face move + the parse-time success comparator + `crit::score_die`); the
/// DP over these pairs must not reapply it.
fn die_values(
    direction: Direction,
    cfg: &SuccessConfig,
    value: i32,
    min: i32,
    max: i32,
    e: u32,
) -> Vec<(i32, i32)> {
    (0..=e)
        .map(|k| {
            let f = adjust(direction, value, min, max, k);
            let base = i32::from(cfg.success.comp.test(f, cfg.success.target));
            let dc = crit::score_die(direction, f, cfg);
            (base + dc.extra_successes - dc.lost, dc.positive_counter - dc.negative_counter)
        })
        .collect()
}
```

- [ ] **Step 9: Run all Task 2 tests**

Run: `cargo test dice::eval::expertise`
Expected: PASS (7 tests).

- [ ] **Step 10: Commit**

```bash
cargo fmt && cargo clippy --all-targets
git add src/server/src/dice/eval/expertise.rs src/server/src/dice/eval/mod.rs
git commit -m "feat(dice/m11b-2): expertise face-adjust and per-die value function"
```

---

## Task 3: The DP allocator + differential oracle — BUDDY-CHECK

**Files:**
- Modify: `src/server/src/dice/eval/expertise.rs`
- Test: `src/server/src/dice/eval/expertise.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `adjust`, `die_values` (Task 2), `RawRoll`, `DieRecord`, `DieId`, `DieKind`, `SuccessConfig`, `Direction`.
- Produces:
  - `pub fn allocate(direction: Direction, cfg: &SuccessConfig, raws: &RawRoll, records: &mut [DieRecord])` — mutate each chosen kept die's `value` (adjusted face) and `expertise` (points spent) to the provably-optimal allocation. No-op if `cfg.expertise == 0`. Consumed by Task 4.
  - `fn run_dp(dies: &[Vec<(i32, i32)>], e: u32, better: impl Fn((i32,i32),(i32,i32)) -> bool) -> (Vec<u32>, (i32,i32))` — the bounded knapsack DP; returns per-die points (aligned with `dies`) and the achieved `(net_sum, counter_sum)`.

**Objective (R1, R3).** Maximize the **visible** outcome: primary key = **clamped** net successes (`net` if `allow_negative`, else `max(net, 0)`), secondary = net counters. Among optimal allocations, the canonical choice is **lowest-index-first** (points concentrate on the earliest dice) — the DP produces this by preferring the smallest `k` per die on ascending scan and backtracking from the last die.

- [ ] **Step 1: Write the failing test for `run_dp` (lexicographic core)**

Add to `mod tests`:

```rust
    #[test]
    fn run_dp_lexicographic_prefers_net_then_counters_then_low_index() {
        let lex = |a: (i32, i32), b: (i32, i32)| a.0 > b.0 || (a.0 == b.0 && a.1 > b.1);
        // Two identical dice, each: k0=(0,0), k1=(1,0). One point, either die gives +1 net.
        let dies = vec![vec![(0, 0), (1, 0)], vec![(0, 0), (1, 0)]];
        let (alloc, achieved) = run_dp(&dies, 1, lex);
        assert_eq!(achieved, (1, 0));
        // Lowest-index-first: the point lands on die 0, not die 1.
        assert_eq!(alloc, vec![1, 0]);
    }

    #[test]
    fn run_dp_allocates_the_more_valuable_die() {
        let lex = |a: (i32, i32), b: (i32, i32)| a.0 > b.0 || (a.0 == b.0 && a.1 > b.1);
        // die0: one point -> +1 net. die1: one point -> +2 net. Budget 1 -> spend on die1.
        let dies = vec![vec![(0, 0), (1, 0)], vec![(0, 0), (2, 0)]];
        let (alloc, achieved) = run_dp(&dies, 1, lex);
        assert_eq!(achieved, (2, 0));
        assert_eq!(alloc, vec![0, 1]);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test dice::eval::expertise::tests::run_dp`
Expected: compile error — `run_dp` not defined.

- [ ] **Step 3: Implement `run_dp`**

Add to `expertise.rs` (above the test module):

```rust
fn add(a: (i32, i32), b: (i32, i32)) -> (i32, i32) {
    (a.0 + b.0, a.1 + b.1)
}

/// Bounded knapsack DP over `dies` (each a `v[k]` table, `k in 0..=e`) with budget
/// `e`, choosing the allocation maximal under the total order `better`.
///
/// `dp[j]` = best `(net,counter)` using ≤ `j` points over the dice processed so far;
/// `dp'[j] = max_k dp[j-k] ⊕ v_i(k)`. O(N·E²). Deterministic. Tie-break: the inner
/// loop scans `k` ascending and replaces only on STRICTLY `better`, so the smallest
/// `k` wins a tie at each die; backtracking runs from the LAST die, where later dice
/// take the smallest optimal `k` (passing budget down), so points concentrate on the
/// earliest dice — the R3 lowest-index-first canonical allocation.
fn run_dp(
    dies: &[Vec<(i32, i32)>],
    e: u32,
    better: impl Fn((i32, i32), (i32, i32)) -> bool,
) -> (Vec<u32>, (i32, i32)) {
    let e = e as usize;
    let n = dies.len();
    let mut dp = vec![(0i32, 0i32); e + 1];
    let mut choice = vec![vec![0u32; e + 1]; n];
    for i in 0..n {
        let vi = &dies[i];
        let prev = dp.clone();
        for j in 0..=e {
            let mut best_pair = add(prev[j], vi[0]); // k = 0
            let mut best_k = 0u32;
            for k in 1..=j {
                let cand = add(prev[j - k], vi[k]);
                if better(cand, best_pair) {
                    best_pair = cand;
                    best_k = k as u32;
                }
            }
            dp[j] = best_pair;
            choice[i][j] = best_k;
        }
    }
    let mut alloc = vec![0u32; n];
    let mut budget = e;
    for i in (0..n).rev() {
        let k = choice[i][budget];
        alloc[i] = k;
        budget -= k as usize;
    }
    (alloc, dp[e])
}
```

- [ ] **Step 4: Run the `run_dp` tests to verify they pass**

Run: `cargo test dice::eval::expertise::tests::run_dp`
Expected: PASS (2 tests).

- [ ] **Step 5: Write the failing tests for `allocate` (targeted cases)**

Add to `mod tests` — first two small helpers, then the case tests:

```rust
    use crate::dice::outcome::{DieRecord, RawDie, RawRoll};
    use crate::dice::spec::{DieId, DieKind};

    /// Build a RawRoll whose kept records have the given values, all d(min..=max).
    fn raws_of(values: &[i32], min: i32, max: i32) -> RawRoll {
        let mut raws = RawRoll::default();
        for (i, &v) in values.iter().enumerate() {
            let id = i as DieId;
            raws.dice.push(RawDie { id, kind: DieKind::Numeric { min, max }, natural: v });
            raws.records.push(DieRecord {
                id, group_index: 0, natural: v, value: v, kept: true,
                exploded: false, rerolled_from: None,
                crit_success: false, crit_fail: false, expertise: 0,
            });
        }
        raws.next_id = values.len() as DieId;
        raws
    }

    /// Re-score a pool exactly as `evaluate_success` will: clamped net + counters.
    fn score_pool(direction: Direction, cfg: &SuccessConfig, records: &[DieRecord]) -> (i32, i32) {
        let (mut base, mut extra, mut lost, mut pos, mut neg) = (0, 0, 0, 0, 0);
        for r in records.iter().filter(|r| r.kept) {
            base += i32::from(cfg.success.comp.test(r.value, cfg.success.target));
            let dc = crit::score_die(direction, r.value, cfg);
            extra += dc.extra_successes;
            lost += dc.lost;
            pos += dc.positive_counter;
            neg += dc.negative_counter;
        }
        let raw = base + extra - lost;
        let allow_neg = cfg.crit_fail.as_ref().map(|c| c.allow_negative).unwrap_or(false);
        (if allow_neg { raw } else { raw.max(0) }, pos - neg)
    }

    #[test]
    fn allocate_zero_budget_is_a_noop() {
        let c = cfg(None, None); // expertise: 0
        let mut raws = raws_of(&[4, 4, 4], 1, 6);
        let before = raws.records.clone();
        allocate(Direction::HighWins, &c, &raws.clone(), &mut raws.records);
        assert_eq!(raws.records, before);
    }

    #[test]
    fn allocate_spends_points_to_reach_the_success_target() {
        // Three dice at face 4, target >=5, 2 expertise points. Optimal: bump two
        // dice to 5 -> 2 successes (each bump costs 1). value/expertise recorded.
        let mut c = cfg(None, None);
        c.expertise = 2;
        let raws = raws_of(&[4, 4, 4], 1, 6);
        let mut records = raws.records.clone();
        allocate(Direction::HighWins, &c, &raws, &mut records);
        assert_eq!(score_pool(Direction::HighWins, &c, &records), (2, 0));
        // Lowest-index-first: dice 0 and 1 get the points, die 2 untouched.
        assert_eq!(records[0].expertise, 1);
        assert_eq!(records[1].expertise, 1);
        assert_eq!(records[2].expertise, 0);
        assert_eq!(records[0].value, 5);
        assert_eq!(records[2].value, 4);
    }

    #[test]
    fn allocate_all_failed_region_maximizes_counters() {
        // R1 fork: not allow_negative, and no allocation can reach 1 net success, so
        // every allocation clamps to net 0 -> maximize counters instead.
        // Two dice at face 1 on a d6, target >=5 (unreachable with 1 point each here),
        // crit_fail at <=1: lost 1, negative_counter 1. crit_success at 6: +0 success,
        // +1 positive_counter (extra_successes 0 so it can't lift net above 0).
        let c = SuccessConfig {
            success: SuccessRule { comp: Comparator::Gte, target: 5 },
            required_successes: None,
            tiers: vec![],
            crit_success: Some(CritSuccess { threshold: 6, extra_successes: 0, positive_counter: 1 }),
            crit_fail: Some(CritFail { threshold: 1, lost: 1, negative_counter: 1, allow_negative: false }),
            expertise: 5,
        };
        let raws = raws_of(&[1, 1], 1, 6);
        let mut records = raws.records.clone();
        allocate(Direction::HighWins, &c, &raws, &mut records);
        // Clamped net stays 0; counters maximized: move both dice to 6 (each leaves the
        // crit_fail region -> +1 net counter apiece from removing the -1, and lands on
        // crit_success -> +1 positive_counter apiece). score_pool nets: base 0,
        // pos 2, neg 0 -> (0, 2).
        assert_eq!(score_pool(Direction::HighWins, &c, &records), (0, 2));
    }
```

- [ ] **Step 6: Run to verify failure**

Run: `cargo test dice::eval::expertise::tests::allocate`
Expected: compile error — `allocate` not defined.

- [ ] **Step 7: Implement `allocate`**

Add to `expertise.rs`. Note the imports at the top of the file must grow:

```rust
use std::collections::HashMap;

use crate::dice::eval::crit;
use crate::dice::outcome::{DieRecord, RawRoll};
use crate::dice::spec::{Direction, DieId, DieKind, SuccessConfig};
```

Then the function:

```rust
/// Distribute `cfg.expertise` points across the pooled kept dice to maximize the
/// VISIBLE outcome: primary key = clamped net successes (`max(net,0)` unless
/// `allow_negative`), secondary = net counters (§R1). Mutates each chosen die's
/// `value` (adjusted face) and `expertise` (points spent). No-op if budget is 0 or
/// there are no kept dice.
///
/// Two-pass clamp handling: the lexicographic pass maximizes RAW `(net, counter)`.
/// If `allow_negative` OR the achieved net ≥ 1, that pass equals the clamped-lex
/// optimum (clamp is identity there). Otherwise every allocation clamps to net 0, so
/// successes tie and the objective degenerates to pure counter-maximization — a second
/// pass with the first key dropped. Both passes use the same R3 tie-break, so the
/// result matches the brute-force oracle exactly.
pub fn allocate(direction: Direction, cfg: &SuccessConfig, raws: &RawRoll, records: &mut [DieRecord]) {
    let e = cfg.expertise;
    if e == 0 {
        return;
    }
    // Bounds per die: every record id maps to a RawDie carrying its Numeric kind.
    let bounds: HashMap<DieId, (i32, i32)> = raws
        .dice
        .iter()
        .map(|d| {
            let DieKind::Numeric { min, max } = d.kind;
            (d.id, (min, max))
        })
        .collect();
    // Contributing dice = the pooled kept dice, in record order.
    let kept: Vec<usize> = records
        .iter()
        .enumerate()
        .filter(|(_, r)| r.kept)
        .map(|(i, _)| i)
        .collect();
    if kept.is_empty() {
        return;
    }
    let dies: Vec<Vec<(i32, i32)>> = kept
        .iter()
        .map(|&idx| {
            let (min, max) = bounds[&records[idx].id];
            die_values(direction, cfg, records[idx].value, min, max, e)
        })
        .collect();

    let lex = |a: (i32, i32), b: (i32, i32)| a.0 > b.0 || (a.0 == b.0 && a.1 > b.1);
    let (lex_alloc, (net, _)) = run_dp(&dies, e, lex);
    let allow_neg = cfg.crit_fail.as_ref().map(|c| c.allow_negative).unwrap_or(false);
    let alloc = if allow_neg || net >= 1 {
        lex_alloc
    } else {
        // All-failed region: successes all clamp to 0; maximize counters only.
        let counter_only = |a: (i32, i32), b: (i32, i32)| a.1 > b.1;
        run_dp(&dies, e, counter_only).0
    };

    for (pos, &idx) in kept.iter().enumerate() {
        let k = alloc[pos];
        let (min, max) = bounds[&records[idx].id];
        records[idx].value = adjust(direction, records[idx].value, min, max, k);
        records[idx].expertise = k as i32;
    }
}
```

- [ ] **Step 8: Run the `allocate` case tests**

Run: `cargo test dice::eval::expertise::tests::allocate`
Expected: PASS (3 tests).

- [ ] **Step 9: Write the differential oracle + property test**

Add to `mod tests`. The oracle enumerates every point distribution (stars-and-bars) and takes the clamped-lex max, matching `allocate`'s objective and canonical tie-break:

```rust
    /// Every distribution of `e` points across `n` dice (compositions into n nonneg
    /// parts). Feasible only because e, n are tiny — the whole point of the oracle.
    fn compositions(n: usize, e: u32) -> Vec<Vec<u32>> {
        if n == 0 {
            return if e == 0 { vec![vec![]] } else { vec![] };
        }
        let mut out = Vec::new();
        for first in 0..=e {
            for mut rest in compositions(n - 1, e - first) {
                let mut one = vec![first];
                one.append(&mut rest);
                out.push(one);
            }
        }
        out
    }

    /// Brute-force optimum: for each allocation, apply `adjust` to a copy of each die's
    /// value, re-score (clamped net + counters), take the clamped-lex max; among optima,
    /// the lowest-index-first allocation (lexicographically-largest k-vector).
    fn oracle(
        direction: Direction,
        cfg: &SuccessConfig,
        records: &[DieRecord],
        bounds: &std::collections::HashMap<DieId, (i32, i32)>,
    ) -> ((i32, i32), Vec<u32>) {
        let kept: Vec<usize> = records
            .iter()
            .enumerate()
            .filter(|(_, r)| r.kept)
            .map(|(i, _)| i)
            .collect();
        let mut best: Option<((i32, i32), Vec<u32>)> = None;
        for alloc in compositions(kept.len(), cfg.expertise) {
            let mut copy = records.to_vec();
            for (pos, &idx) in kept.iter().enumerate() {
                let (min, max) = bounds[&copy[idx].id];
                copy[idx].value = adjust(direction, copy[idx].value, min, max, alloc[pos]);
            }
            let score = score_pool(direction, cfg, &copy);
            let take = match &best {
                None => true,
                Some((bs, ba)) => {
                    score.0 > bs.0
                        || (score.0 == bs.0 && score.1 > bs.1)
                        // tie on objective -> lowest-index-first = lexicographically-largest k-vector
                        || (score == *bs && alloc > *ba)
                }
            };
            if take {
                best = Some((score, alloc));
            }
        }
        best.expect("at least the all-zero allocation exists")
    }

    #[test]
    fn dp_matches_brute_force_oracle_over_a_random_corpus() {
        // Deterministic pseudo-random corpus (no RNG dependency): a SplitMix walk over
        // a fixed seed, varying ranges, targets, crit configs, direction, allow_negative,
        // e in 0..=6, n in 1..=5. Objective-value equality is the load-bearing property;
        // allocation equality pins the canonical tie-break.
        let mut s: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = s;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        let mut pick = |lo: i32, hi: i32| lo + (next() % ((hi - lo + 1) as u64)) as i32;

        for _ in 0..4000 {
            let min = 1;
            let max = pick(2, 12);
            let direction = if next() % 2 == 0 { Direction::HighWins } else { Direction::LowWins };
            let comp = if direction == Direction::HighWins { Comparator::Gte } else { Comparator::Lte };
            let target = pick(min, max);
            let cs = if next() % 2 == 0 {
                Some(CritSuccess {
                    threshold: pick(min, max),
                    extra_successes: pick(-1, 3), // include a perverse negative
                    positive_counter: pick(-2, 2),
                })
            } else {
                None
            };
            let cf = if next() % 2 == 0 {
                Some(CritFail {
                    threshold: pick(min, max),
                    lost: pick(-1, 3),
                    negative_counter: pick(-2, 2),
                    allow_negative: next() % 2 == 0,
                })
            } else {
                None
            };
            let cfg = SuccessConfig {
                success: SuccessRule { comp, target },
                required_successes: None,
                tiers: vec![],
                crit_success: cs,
                crit_fail: cf,
                expertise: (next() % 7) as u32, // 0..=6
            };
            let n = 1 + (next() % 5) as usize; // 1..=5
            let values: Vec<i32> = (0..n).map(|_| pick(min, max)).collect();
            let raws = raws_of(&values, min, max);
            let bounds: std::collections::HashMap<DieId, (i32, i32)> = raws
                .dice
                .iter()
                .map(|d| {
                    let DieKind::Numeric { min, max } = d.kind;
                    (d.id, (min, max))
                })
                .collect();

            let mut records = raws.records.clone();
            allocate(direction, &cfg, &raws, &mut records);
            let dp_score = score_pool(direction, &cfg, &records);
            let (oracle_score, oracle_alloc) = oracle(direction, &cfg, &raws.records, &bounds);

            assert_eq!(dp_score, oracle_score, "objective mismatch: cfg={cfg:?} values={values:?}");

            // Allocation equality: compare the DP's recorded per-die k against the oracle.
            let dp_alloc: Vec<u32> = records
                .iter()
                .filter(|r| r.kept)
                .map(|r| r.expertise as u32)
                .collect();
            assert_eq!(dp_alloc, oracle_alloc, "allocation mismatch: cfg={cfg:?} values={values:?}");
        }
    }

    #[test]
    fn expertise_result_is_between_greedy_and_max_bounds() {
        // Sanity bounds: optimal net >= the no-expertise net, and <= the net with every
        // die individually maxed toward better (an unreachable upper bound when budget
        // is limited, but never exceeded).
        let mut c = cfg(Some(CritSuccess { threshold: 6, extra_successes: 1, positive_counter: 0 }), None);
        c.expertise = 3;
        let raws = raws_of(&[3, 4, 5, 2], 1, 6);
        let base = score_pool(Direction::HighWins, &c, &raws.records).0;
        let mut records = raws.records.clone();
        allocate(Direction::HighWins, &c, &raws, &mut records);
        let got = score_pool(Direction::HighWins, &c, &records).0;
        let mut all_max = raws.records.clone();
        for r in all_max.iter_mut() {
            r.value = 6;
        }
        let ceiling = score_pool(Direction::HighWins, &c, &all_max).0;
        assert!(got >= base, "expertise never worsens the outcome");
        assert!(got <= ceiling, "expertise never beats every-die-maxed");
    }
```

- [ ] **Step 10: Run the oracle + bounds tests**

Run: `cargo test dice::eval::expertise`
Expected: PASS (all Task 2 + Task 3 tests, including `dp_matches_brute_force_oracle_over_a_random_corpus`).

- [ ] **Step 11: Commit**

```bash
cargo fmt && cargo clippy --all-targets
git add src/server/src/dice/eval/expertise.rs
git commit -m "feat(dice/m11b-2): expertise DP allocator with brute-force differential oracle"
```

- [ ] **Step 12: Buddy-check gate (MANDATORY for this task)**

Run the `buddy-checking` skill over the Task 3 diff (`eval/expertise.rs`). Reviewers must independently verify: (a) the two-pass clamp split equals the oracle in the all-failed region (`net <= 0`, not `allow_negative`); (b) the DP's backtrack genuinely yields lowest-index-first (do not trust the doc comment — trace it or add a targeted case); (c) the oracle is a faithful brute force (enumeration complete, tie-break identical). Record the outcome; resolve every Critical/Important before Task 4.

---

## Task 4: Wire expertise into `evaluate_success`

**Files:**
- Modify: `src/server/src/dice/eval/success.rs` (`evaluate_success` ~16-22; imports ~1-4)
- Test: `src/server/src/dice/eval/success.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `expertise::allocate` (Task 3).
- Produces: `evaluate_success` applies the expertise pre-pass so every downstream field (`successes`, `margin`, `pass`, tiers, counters, `total`) reflects the optimal allocation. No signature change.

- [ ] **Step 1: Write the failing integration tests**

Add to `success.rs`'s `mod tests`:

```rust
    #[test]
    fn expertise_lifts_success_count_through_the_full_evaluate_path() {
        use crate::dice::spec::CritSuccess;
        // 3 dice all at face 4 (min=max forces it), target >=5, expertise 2.
        // Optimal: two dice -> 5 => 2 successes. Runs through evaluate(), not allocate directly.
        let spec = RollSpec {
            expr: Expr::Dice(DiceGroup {
                count: 3,
                kind: DieKind::Numeric { min: 1, max: 6 },
                modifiers: vec![],
            }),
            direction: Direction::HighWins,
            mode: Mode::SuccessCount(SuccessConfig {
                success: SuccessRule { comp: Comparator::Gte, target: 5 },
                required_successes: None,
                tiers: vec![],
                crit_success: None,
                crit_fail: None,
                expertise: 2,
            }),
        };
        // manual_raws pins the faces to 4 so the allocation is determinate.
        let raws = manual_raws(&[4, 4, 4]);
        let out = evaluate_success(&spec, cfg_of(&spec), &raws);
        assert_eq!(out.successes, Some(2));
        // Audit trail: two dice show +1 expertise and an adjusted value of 5.
        let bumped: Vec<&DieRecord> = out.records.iter().filter(|r| r.expertise == 1).collect();
        assert_eq!(bumped.len(), 2);
        assert!(bumped.iter().all(|r| r.value == 5));
    }

    #[test]
    fn expertise_can_push_net_across_a_required_successes_tier() {
        use crate::dice::spec::{CritSuccess, Tier};
        // required 1; tier at offset 1 = "great". 2 dice at 4, target >=5, expertise 2
        // -> 2 successes -> margin (2-1)=1 -> "great". Without expertise: 0 successes.
        let spec = RollSpec {
            expr: Expr::Dice(DiceGroup {
                count: 2,
                kind: DieKind::Numeric { min: 1, max: 6 },
                modifiers: vec![],
            }),
            direction: Direction::HighWins,
            mode: Mode::SuccessCount(SuccessConfig {
                success: SuccessRule { comp: Comparator::Gte, target: 5 },
                required_successes: Some(1),
                tiers: vec![
                    Tier { margin_offset: 0, label: Some("ok".into()), tier_value: Some(1) },
                    Tier { margin_offset: 1, label: Some("great".into()), tier_value: Some(2) },
                ],
                crit_success: None,
                crit_fail: None,
                expertise: 2,
            }),
        };
        let raws = manual_raws(&[4, 4]);
        let out = evaluate_success(&spec, cfg_of(&spec), &raws);
        assert_eq!(out.successes, Some(2));
        assert_eq!(out.margin, Some(1));
        assert_eq!(out.tier_value, Some(2));
    }

    #[test]
    fn expertise_direction_mirror_symmetry() {
        use crate::dice::spec::CritSuccess;
        // Mirror property (design §4/§8): flip direction + mirror every face
        // (f -> min+max-f) + mirror the success target and crit threshold -> identical
        // net successes and counters.
        let hi_cfg = SuccessConfig {
            success: SuccessRule { comp: Comparator::Gte, target: 5 },
            required_successes: None,
            tiers: vec![],
            crit_success: Some(CritSuccess { threshold: 6, extra_successes: 1, positive_counter: 1 }),
            crit_fail: None,
            expertise: 3,
        };
        let hi = RollSpec {
            expr: Expr::Dice(DiceGroup { count: 4, kind: DieKind::Numeric { min: 1, max: 6 }, modifiers: vec![] }),
            direction: Direction::HighWins,
            mode: Mode::SuccessCount(hi_cfg.clone()),
        };
        // Mirror faces about (min+max)=7: 2->5, 4->3, 5->2, 1->6.
        let hi_raws = manual_raws(&[2, 4, 5, 1]);
        let hi_out = evaluate_success(&hi, cfg_of(&hi), &hi_raws);

        let lo_cfg = SuccessConfig {
            success: SuccessRule { comp: Comparator::Lte, target: 2 }, // 7 - 5
            crit_success: Some(CritSuccess { threshold: 1, extra_successes: 1, positive_counter: 1 }), // 7 - 6
            ..hi_cfg
        };
        let lo = RollSpec {
            expr: Expr::Dice(DiceGroup { count: 4, kind: DieKind::Numeric { min: 1, max: 6 }, modifiers: vec![] }),
            direction: Direction::LowWins,
            mode: Mode::SuccessCount(lo_cfg),
        };
        let lo_raws = manual_raws(&[5, 3, 2, 6]); // 7 - each hi face
        let lo_out = evaluate_success(&lo, cfg_of(&lo), &lo_raws);

        assert_eq!(hi_out.successes, lo_out.successes);
        assert_eq!(hi_out.positive_counter, lo_out.positive_counter);
        assert_eq!(hi_out.negative_counter, lo_out.negative_counter);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test dice::eval::success::tests::expertise`
Expected: FAIL — expertise not applied (`successes` is `Some(0)`, no `expertise == 1` records).

- [ ] **Step 3: Wire the pre-pass into `evaluate_success`**

In `src/server/src/dice/eval/success.rs`, add the import (with the existing `use crate::dice::eval::...` lines):

```rust
use crate::dice::eval::expertise;
```

Then, in `evaluate_success`, insert the call immediately after the `records` clone and before the counting loop:

```rust
pub fn evaluate_success(spec: &RollSpec, cfg: &SuccessConfig, raws: &RawRoll) -> RollOutcome {
    let mut records = raws.records.clone();
    if cfg.expertise > 0 {
        expertise::allocate(spec.direction, cfg, raws, &mut records);
    }
    let mut base = 0i32;
    // ... unchanged counting loop over records.iter_mut().filter(|r| r.kept) ...
```

- [ ] **Step 4: Run the integration tests to verify they pass**

Run: `cargo test dice::eval::success`
Expected: PASS (existing SuccessCount tests + the three new expertise tests).

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets
git add src/server/src/dice/eval/success.rs
git commit -m "feat(dice/m11b-2): apply expertise pre-pass in evaluate_success"
```

---

## Task 5: Notation `e<N>`

**Files:**
- Modify: `src/server/src/dice/notation/mod.rs` (add `ParseError::DuplicateExpertise`)
- Modify: `src/server/src/dice/notation/parser.rs` (`P` struct, `parse`, `modifiers`)
- Test: `src/server/src/dice/notation/parser.rs` (`mod tests`) and `src/server/src/dice/notation/lexer.rs` (`mod tests`)

**Interfaces:**
- Consumes: `SuccessConfig.expertise` (Task 1).
- Produces: `e<N>` sets roll-level `expertise = N` when the resolved mode is SuccessCount; silently discarded (never an error) when the mode is Total; a duplicate `e<N>` is `ParseError::DuplicateExpertise`.

- [ ] **Step 1: Add the lexer documentation test (no code change)**

In `src/server/src/dice/notation/lexer.rs`'s `mod tests`, add:

```rust
    #[test]
    fn lex_expertise_uses_the_identifier_arm() {
        // `4d6e3` needs no dedicated token: the alphabetic-run arm emits Ident("e")
        // and the digits become Int(3). The parser recognizes Ident("e") as expertise.
        let toks = lex("4d6e3").unwrap();
        assert_eq!(
            toks,
            vec![
                Token::Int(4),
                Token::D,
                Token::Int(6),
                Token::Ident("e".into()),
                Token::Int(3),
            ]
        );
    }
```

Run: `cargo test dice::notation::lexer::tests::lex_expertise`
Expected: PASS (documents existing behavior; confirms no lexer change needed).

- [ ] **Step 2: Add the `DuplicateExpertise` error variant**

In `src/server/src/dice/notation/mod.rs`, add to `ParseError`:

```rust
    /// A second `e<N>` expertise token appeared in one roll. `expertise` is shared
    /// roll-level parser state (one `RollSpec`), so a silent overwrite would lose one.
    DuplicateExpertise,
```

- [ ] **Step 3: Write the failing parser tests**

In `src/server/src/dice/notation/parser.rs`'s `mod tests`, add:

```rust
    #[test]
    fn e_token_sets_expertise_under_successcount() {
        let spec = parse(
            "4d6t5e3",
            ParseContext { mode: ModeKind::SuccessCount, direction: Direction::HighWins },
        )
        .unwrap();
        match spec.mode {
            Mode::SuccessCount(c) => assert_eq!(c.expertise, 3),
            other => panic!("expected SuccessCount, got {other:?}"),
        }
    }

    #[test]
    fn e_token_is_discarded_under_total_ambient_without_error() {
        // R4: a stray e<N> where the mode can't use it must NOT fail the roll.
        let spec = parse("1d20t10e3", ParseContext::default()).unwrap(); // ambient Total
        match spec.mode {
            Mode::Total(c) => assert_eq!(c.difficulty, Some(10)),
            other => panic!("expected Total, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_e_token_errors() {
        let e = parse(
            "4d6t5e3e4",
            ParseContext { mode: ModeKind::SuccessCount, direction: Direction::HighWins },
        );
        assert!(matches!(e, Err(ParseError::DuplicateExpertise)));
    }
```

- [ ] **Step 4: Run to verify failure**

Run: `cargo test dice::notation::parser::tests::e_token`
Expected: FAIL — `e` parsed as an unknown modifier (`ParseError::Unexpected("modifier e")`).

- [ ] **Step 5: Add the `expertise` field to the parser state**

In `parser.rs`, add to `struct P`:

```rust
struct P {
    toks: Vec<Token>,
    pos: usize,
    success: Option<SuccessRule>,
    t_target: Option<i32>,
    /// Roll-level expertise budget from an `e<N>` token (design §8/R4). Shared state,
    /// not per-`DiceGroup`; applied only when the resolved mode is SuccessCount.
    expertise: Option<u32>,
}
```

And initialize it in `parse`:

```rust
    let mut p = P {
        toks,
        pos: 0,
        success: None,
        t_target: None,
        expertise: None,
    };
```

- [ ] **Step 6: Recognize `e<N>` in `modifiers`**

In `parser.rs`'s `modifiers`, add an arm to the `match id.as_str()` block (next to `"t"`):

```rust
                        "e" => {
                            let n = self.expect_int()?;
                            if self.expertise.is_some() {
                                return Err(ParseError::DuplicateExpertise);
                            }
                            self.expertise = Some(n as u32);
                        }
```

- [ ] **Step 7: Lower expertise into the SuccessCount config; discard under Total**

In `parse`, set the field when building `Mode::SuccessCount` and simply do not read `p.expertise` in the `Mode::Total` branch (R4 discard). The SuccessCount construction becomes:

```rust
        Mode::SuccessCount(SuccessConfig {
            success: rule,
            required_successes: None,
            tiers: vec![],
            crit_success: None,
            crit_fail: None,
            expertise: p.expertise.unwrap_or(0),
        })
```

(The `Mode::Total(TotalConfig { .. })` branch is unchanged — `p.expertise` is ignored there, silently discarded.)

- [ ] **Step 8: Run the parser tests to verify they pass**

Run: `cargo test dice::notation`
Expected: PASS (existing notation tests + the three new `e` tests + the lexer doc test).

- [ ] **Step 9: Commit**

```bash
cargo fmt && cargo clippy --all-targets
git add src/server/src/dice/notation/mod.rs src/server/src/dice/notation/parser.rs src/server/src/dice/notation/lexer.rs
git commit -m "feat(dice/m11b-2): parse e<N> expertise notation (roll-level, mode-lenient)"
```

---

## Task 6: Codebase-skill gate + deferred-work log

**Files:**
- Modify: `C:\Dev\Shadowcat\.claude\skills\shadowcat-codebase-dice\SKILL.md`
- Modify: `src/server/../docs/TODO.md` (repo `docs/TODO.md`)

**Interfaces:** none (documentation).

- [ ] **Step 1: Log the deferred expertise cap in `docs/TODO.md`**

In `docs/TODO.md`, under the "Server / dice" section (next to the dice-count cap item), add:

```markdown
- Bound `SuccessConfig.expertise` (u32) at the M11d untrusted-transport boundary,
  alongside the per-roll dice-count cap: `eval::expertise::allocate` is `O(N·E²)`, so
  an unbounded `E` from an untrusted `RollSpec` is a DoS vector. Pure-library b-2 stays
  cap-agnostic by design.
```

- [ ] **Step 2: Update `shadowcat-codebase-dice`**

In the dice skill's "Key files & seams", add an `eval::expertise` entry describing: the value-mutating pre-pass called by `eval::success` when `cfg.expertise > 0`; `adjust`/`die_values`/`run_dp`/`allocate`; the R1 two-pass clamp handling; the R3 lowest-index-first tie-break; `SuccessConfig.expertise: u32` + `DieRecord.expertise: i32`. In "Hard invariants", add: expertise optimizes the CLAMPED (visible) net successes with a counter-max fallback in the all-failed region; `adjust` preserves `v_i(0) = value` for out-of-range (Compound/Penetrate) faces; the DP allocation is deterministic and oracle-verified. In "Gotchas", note `e<N>` is roll-level and silently discarded under Total mode. Move expertise from "deferred (M11b-2)" to shipped. Update the "Purpose"/intro line that says M11b-2 is still deferred.

- [ ] **Step 3: Spec-reviewer confirmation of the skill diff**

Dispatch `shadowcat-spec-reviewer` on the `shadowcat-codebase-dice` diff to confirm it accurately captures the implemented change (no omission, drift, or broken pointer), per the reviewed skill-update gate. Resolve findings before merge.

- [ ] **Step 4: Run the whole dice suite once more**

Run: `cargo test dice`
Expected: PASS (all tasks).

- [ ] **Step 5: Commit**

```bash
git add docs/TODO.md .claude/skills/shadowcat-codebase-dice/SKILL.md
git commit -m "docs(dice/m11b-2): update dice skill + TODO for expertise DP"
```

---

## Self-Review

**1. Spec coverage** (design §-by-§):
- §2 placement (value-mutating pre-pass; b-1 counting sealed) → Task 4 (call site) + Task 3 (allocate mutates value/expertise only).
- §3 R1 (clamped objective + counter fallback) → Task 3 `allocate` two-pass + `allocate_all_failed_region_maximizes_counters` + oracle.
- §3 R2 (adjust from `value`) → Task 2 `adjust` + `adjust_preserves_value_at_zero_points_for_all_ranges` (refined beyond the doc's `clamp(...,min,max)` to preserve out-of-range faces — see note below).
- §3 R3 (lowest-index-first determinism) → Task 3 `run_dp` tie-break + `run_dp_lexicographic_prefers_net_then_counters_then_low_index` + oracle allocation equality.
- §3 R4 (`e<N>` roll-level, mode-lenient) → Task 5 (all three tests).
- §4 DP (`O(N·E²)`, value function, three-case clamp) → Tasks 2-3.
- §5 struct/pipeline changes → Task 1 + Task 4 + Task 5.
- §6 differential oracle + bounds + direction-mirror + penetrate + determinism → Task 3 (`dp_matches_brute_force_oracle...`, `expertise_result_is_between_greedy_and_max_bounds`) + Task 4 (`expertise_direction_mirror_symmetry`). Penetrate/out-of-range baseline is pinned in Task 2's `adjust` tests (the record-level penetrate value flows through `adjust` unchanged at k=0).
- §7 deferrals (E cap → M11d) → Task 6 Step 1.
- §8 codebase-skill gate → Task 6.

**2. Placeholder scan:** No "TBD"/"handle edge cases"/"similar to Task N" — every code step shows complete code. Construction-site sweeps (Task 1 Step 4) are compiler-driven with the exact field to add.

**3. Type consistency:** `allocate(direction, cfg, raws, records)` signature identical in Task 3 (def), Task 4 (call). `run_dp`/`die_values`/`adjust` signatures match across Tasks 2-3. `SuccessConfig.expertise: u32` / `DieRecord.expertise: i32` consistent (Task 1 → all). `ParseError::DuplicateExpertise` defined in Task 5 Step 2, used Step 6/Step 3-test. `score_pool`/`raws_of`/`oracle`/`compositions` are test-only helpers defined in Task 3 before their use.

**Note — a refinement of design §R2 discovered while grounding the plan:** the design doc phrases the face move as `clamp(value ± k, min, max)`. Taken literally, that corrupts any die whose `value` is outside `[min,max]` — a Compound die (`value > max`) or a Penetrate child (`value < min`) — by dragging it back across a bound even at `k = 0`, violating `v_i(0) = value`. Task 2's `adjust` uses a toward-better-bounded move (`value + k.min((max−value).max(0))` and its LowWins mirror) that provably preserves `v_i(0) = value` for all inputs while still capping at the die's better-end face. This honors R2's intent ("adjust from `value`, toward better, bounded") more precisely than the doc's clamp phrasing; it is called out here and in Task 2's invariant comment.
