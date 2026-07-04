# M11b-1: Globals + Classification + Crit — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the `direction` global flip, refactor `Mode` into a data-carrying enum (`Total | SuccessCount`), build the shared margin→pass/tier classifier applied to both modes, add crit events + counters, and add the unified `t<N>` target notation — all on top of the stable M11a dice core.

**Architecture:** M11a's per-group pipeline (`eval::groups::resolve_group`) and the RNG stay sealed. This checkpoint reshapes the *spec* data model (Task 1), then layers evaluator behavior additively: a pure direction-aware classifier over each mode's margin (Task 2 for Total, Task 4 for SuccessCount), crit arithmetic in SuccessCount (Task 3), and an ambient-mode notation front-end for `t<N>` (Task 5). Property tests (Task 6) and the reviewed codebase-skill gate (Task 7) close it.

**Tech Stack:** Rust (the `shadowcat` server crate), `serde`, `proptest`. Pure library — no `ws`/`data`/`http` deps, no ts-rs bindings (M11d).

## Global Constraints

- **Design source of truth:** `docs/superpowers/specs/2026-07-04-m11b-system-rules-design.md`. This plan implements **M11b-1 only** (its §1 checkpoint table row 1). Expertise DP is **M11b-2**; labeled/custom-face dice are **M11b-3** — do NOT build them here.
- **`dist/` must exist before any `cargo` build** of the server (`rust-embed` validates `../../dist/` at compile time). If absent, run `pnpm --filter @shadowcat/shell build` from the repo root once. [[embed-dist-compile-ordering]]
- **All commands run from `src/server/`** unless stated. Crate is `shadowcat`.
- **Keep M11a sealed:** do not change the semantics of `eval/groups.rs`, `rng.rs`, or `recalc.rs`'s pipeline. Task 1 adds two defaulted fields to the `DieRecord` literals in `groups.rs`; that is the only permitted `groups.rs` edit.
- **Pure library:** no `#[derive(TS)]`/ts-rs on any new type. No `ws`/`data`/`http`/`scene` imports.
- **Per-task gate:** every task ends green under `cargo test -p shadowcat`, `cargo fmt --check`, and `cargo clippy -p shadowcat --all-targets -- -D warnings`.
- **Cross-platform:** no OS-specific code; this is arithmetic + data. No new dependencies.

## Model/Effort directives

- **Plan-writer:** mainline, Opus 4.8 (1M) / effort high (user directive: "no model switch, want to make sure we get it right").
- **Implementation:** `shadowcat-coder` (effort medium) per task, or mainline on the current model.
- **Review:** `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` two-reviewer pair per task (effort high); escalate to `-opus` twins if findings read shallow.

## Buddy-check directives

Pre-authorized `buddy-checking` (blind two-reviewer + debate) — dense, easy-to-get-subtly-wrong logic, matching the M11a track record where every buddy-checked pipeline task found real bugs:

- **Task 2 (classifier + direction-aware margin)** — the HighWins/LowWins margin orientation is subtle, and the asymmetry (Total margin flips with direction; SuccessCount margin does NOT) is exactly the kind of sign error a single reviewer misses. Buddy-check.
- **Task 3 (crit arithmetic)** — net-success clamping, `allow_negative`, and counters-as-separate-output are the M11a "dense arithmetic" bug class. Buddy-check.

All other tasks: standard two-reviewer gate. Fold the buddy-check offer into the execution handoff.

## File Structure

- `src/server/src/dice/spec.rs` — **Modify.** Add `Direction`, `Tier`, `CritSuccess`, `CritFail`, `TotalConfig`, `SuccessConfig`; rewrite `Mode` (data-carrying) and `RollSpec` (add `direction`, `mode` carries config; drop the top-level `success`/`required_successes`).
- `src/server/src/dice/outcome.rs` — **Modify.** `RollOutcome`: rename `net_margin`→`margin` (widen to `Option<i64>`), add `tier_label`/`tier_value`/`crit_successes`/`crit_fails`/`positive_counter`/`negative_counter`. `DieRecord`: add `crit_success`/`crit_fail` bools.
- `src/server/src/dice/eval/classify.rs` — **Create.** Pure `classify(margin, &tiers) -> Classification`; `oriented_margin` helper.
- `src/server/src/dice/eval/mod.rs` — **Modify.** `evaluate` dispatch matches the new `Mode`; extracts config + passes it.
- `src/server/src/dice/eval/sum.rs` — **Modify.** `evaluate_sum`→`evaluate_total(spec, cfg, raws)`; Task 2 adds Total classification.
- `src/server/src/dice/eval/success.rs` — **Modify.** `evaluate_success(spec, cfg, raws)`; Task 3 adds crit; Task 4 adds the shared classifier + tiers.
- `src/server/src/dice/eval/crit.rs` — **Create (Task 3).** Per-die crit scoring helper.
- `src/server/src/dice/notation/parser.rs` — **Modify.** Task 1: construct the new shape. Task 5: `parse(input, ctx)` + `t<N>`.
- `src/server/src/dice/notation/mod.rs` — **Modify (Task 5).** `ParseContext`, `ModeKind`; maybe a `ModeConflict` error.
- `src/server/src/dice/recalc.rs`, `eval/groups.rs`, `proptests.rs` — **Modify (Task 1, mechanical).** Migrate constructors / add defaulted fields.
- `src/server/src/dice/mod.rs` — **Modify (Task 1).** Re-export the new public types.

---

### Task 1: Data-model migration (Direction + data-carrying Mode; behavior preserved)

Reshape the spec + outcome data model to the full M11b-1 shape, migrate **every** construction site, and keep the entire suite green. **No behavior change** — `Total{difficulty:None,tiers:[]}` and `SuccessCount{…,tiers:[],crit_*:None}` reproduce exact M11a semantics. The existing test suite is the regression gate; add one serde round-trip test for the new shape.

**Files:**
- Modify: `src/server/src/dice/spec.rs`, `src/server/src/dice/outcome.rs`, `src/server/src/dice/eval/mod.rs`, `src/server/src/dice/eval/sum.rs`, `src/server/src/dice/eval/success.rs`, `src/server/src/dice/eval/groups.rs:32-40,172-180`, `src/server/src/dice/recalc.rs`, `src/server/src/dice/notation/parser.rs`, `src/server/src/dice/proptests.rs`, `src/server/src/dice/mod.rs`

**Interfaces:**
- Produces: the new `RollSpec`/`Mode`/`TotalConfig`/`SuccessConfig`/`Direction`/`Tier`/`CritSuccess`/`CritFail` types; `RollOutcome` with `margin: Option<i64>` + tier/crit/counter fields; `DieRecord` with `crit_success`/`crit_fail`. `evaluate(spec, raws)`, `evaluate_total(spec, cfg, raws)`, `evaluate_success(spec, cfg, raws)`.

- [ ] **Step 1: Add the new spec types** in `spec.rs` (keep `SuccessRule`, `Comparator`, `Expr`, `DiceGroup`, etc. unchanged):

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Direction {
    #[default]
    HighWins,
    LowWins,
}

/// One rung of a classification ladder, evaluated on an oriented margin
/// (higher = better). `margin_offset` is the threshold the margin must reach.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tier {
    pub margin_offset: i32,
    pub label: Option<String>,
    pub tier_value: Option<i32>,
}

/// A crit-success event (SuccessCount mode). Fires when a kept die's value
/// reaches `threshold` (direction-aware). Adds `extra_successes` beyond the
/// die's base success and `positive_counter` to the positive tally.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CritSuccess {
    pub threshold: i32,
    pub extra_successes: i32,
    pub positive_counter: i32,
}

/// A crit-fail event (SuccessCount mode). Fires when a kept die's value
/// reaches `threshold` (direction-aware). Subtracts `lost` from net successes
/// (clamped at 0 unless `allow_negative`) and adds `negative_counter`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CritFail {
    pub threshold: i32,
    pub lost: i32,
    pub negative_counter: i32,
    pub allow_negative: bool,
}

/// Total-mode config: fold the expression to a total, optionally classify it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TotalConfig {
    /// Margin reference; `None` ⇒ report the bare total, no classification.
    pub difficulty: Option<i32>,
    /// Ladder over `margin = oriented(total, difficulty)`. Empty ⇒ default 2-rung pass/fail.
    pub tiers: Vec<Tier>,
}

/// SuccessCount-mode config: count net successes across the pooled kept dice.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuccessConfig {
    /// Per-die target (comparator + threshold) — REQUIRED in this mode.
    pub success: SuccessRule,
    /// Margin reference; `None` ⇒ report the bare success count.
    pub required_successes: Option<i32>,
    /// Ladder over `margin = net_successes - required_successes`. Empty ⇒ default 2-rung.
    pub tiers: Vec<Tier>,
    pub crit_success: Option<CritSuccess>,
    pub crit_fail: Option<CritFail>,
    // NOTE: `expertise: u32` is deliberately absent — it is M11b-2.
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Total(TotalConfig),
    SuccessCount(SuccessConfig),
}
```

Rewrite `RollSpec` (remove the old `mode`/`success`/`required_successes` fields):

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollSpec {
    pub expr: Expr,
    pub direction: Direction,
    pub mode: Mode,
}
```

- [ ] **Step 2: Migrate `spec.rs`'s own test.** Replace `spec_serde_round_trips`'s constructor with the new shape and add a SuccessCount round-trip:

```rust
#[test]
fn spec_serde_round_trips() {
    let spec = RollSpec {
        expr: Expr::Bin {
            op: BinOp::Add,
            lhs: Box::new(Expr::Dice(DiceGroup {
                count: 2,
                kind: DieKind::Numeric { min: 1, max: 6 },
                modifiers: vec![],
            })),
            rhs: Box::new(Expr::Const(3)),
        },
        direction: Direction::HighWins,
        mode: Mode::Total(TotalConfig { difficulty: None, tiers: vec![] }),
    };
    let json = serde_json::to_string(&spec).unwrap();
    assert_eq!(spec, serde_json::from_str::<RollSpec>(&json).unwrap());
}

#[test]
fn success_config_serde_round_trips() {
    let spec = RollSpec {
        expr: Expr::Dice(DiceGroup { count: 5, kind: DieKind::Numeric { min: 1, max: 10 }, modifiers: vec![] }),
        direction: Direction::LowWins,
        mode: Mode::SuccessCount(SuccessConfig {
            success: SuccessRule { comp: Comparator::Lte, target: 4 },
            required_successes: Some(2),
            tiers: vec![Tier { margin_offset: 2, label: Some("Great".into()), tier_value: Some(1) }],
            crit_success: Some(CritSuccess { threshold: 1, extra_successes: 1, positive_counter: 1 }),
            crit_fail: None,
        }),
    };
    let json = serde_json::to_string(&spec).unwrap();
    assert_eq!(spec, serde_json::from_str::<RollSpec>(&json).unwrap());
}
```

- [ ] **Step 3: Extend `outcome.rs`.** In `RollOutcome`, rename `net_margin`→`margin` as `Option<i64>` and add the new fields; in `DieRecord` add the two crit flags:

```rust
pub struct RollOutcome {
    pub total: i64,
    pub records: Vec<DieRecord>,
    pub successes: Option<i32>,
    pub pass: Option<bool>,
    pub margin: Option<i64>,            // renamed from net_margin, widened
    pub tier_label: Option<String>,
    pub tier_value: Option<i32>,
    pub crit_successes: i32,
    pub crit_fails: i32,
    pub positive_counter: i32,
    pub negative_counter: i32,
}
// DieRecord: add after `rerolled_from`:
    pub crit_success: bool,
    pub crit_fail: bool,
```

- [ ] **Step 4: Add the two defaulted `DieRecord` fields at both construction sites in `groups.rs`.** In the base map (`groups.rs:32-40`) and in `push_extra` (`groups.rs:172-180`), append `crit_success: false, crit_fail: false,` to each `DieRecord { … }` literal. **No other `groups.rs` change.**

- [ ] **Step 5: Rewrite `eval/mod.rs::evaluate` dispatch:**

```rust
pub fn evaluate(spec: &RollSpec, raws: &RawRoll) -> RollOutcome {
    match &spec.mode {
        Mode::Total(cfg) => sum::evaluate_total(spec, cfg, raws),
        Mode::SuccessCount(cfg) => success::evaluate_success(spec, cfg, raws),
    }
}
```

Update the two `eval/mod.rs` tests: `mode: Mode::Sum, success: None, required_successes: None` → `direction: Direction::HighWins, mode: Mode::Total(TotalConfig { difficulty: None, tiers: vec![] })`. Update imports (`Direction`, `TotalConfig`).

- [ ] **Step 6: Update `eval/sum.rs`.** Rename `evaluate_sum`→`evaluate_total` with signature `pub fn evaluate_total(spec: &RollSpec, _cfg: &TotalConfig, raws: &RawRoll) -> RollOutcome` (cfg unused this task — Task 2 uses it). Body: fold as before, but construct the full `RollOutcome` with new fields defaulted:

```rust
pub fn evaluate_total(spec: &RollSpec, _cfg: &TotalConfig, raws: &RawRoll) -> RollOutcome {
    let mut next_group = 0usize;
    let total = fold(&spec.expr, raws, &mut next_group);
    RollOutcome {
        total,
        records: raws.records.clone(),
        successes: None,
        pass: None,
        margin: None,
        tier_label: None,
        tier_value: None,
        crit_successes: 0,
        crit_fails: 0,
        positive_counter: 0,
        negative_counter: 0,
    }
}
```

Migrate the 5 test constructors in `sum.rs` (each `mode: Mode::Sum, success: None, required_successes: None` → new Total shape; add `Direction`/`TotalConfig` imports).

- [ ] **Step 7: Update `eval/success.rs`.** Signature `pub fn evaluate_success(spec: &RollSpec, cfg: &SuccessConfig, raws: &RawRoll) -> RollOutcome` (drop `_ = spec` warning by using `spec` only if needed — for this task it is unused except to keep a uniform signature; prefix `_spec` if clippy warns). Read from `cfg`:

```rust
pub fn evaluate_success(_spec: &RollSpec, cfg: &SuccessConfig, raws: &RawRoll) -> RollOutcome {
    let successes = raws.records.iter()
        .filter(|r| r.kept && cfg.success.comp.test(r.value, cfg.success.target))
        .count() as i32;
    let total: i64 = raws.records.iter().filter(|r| r.kept).map(|r| r.value as i64).sum();
    let (pass, margin) = match cfg.required_successes {
        Some(req) => (Some(successes >= req), Some((successes - req) as i64)),
        None => (None, None),
    };
    RollOutcome {
        total, records: raws.records.clone(), successes: Some(successes),
        pass, margin, tier_label: None, tier_value: None,
        crit_successes: 0, crit_fails: 0, positive_counter: 0, negative_counter: 0,
    }
}
```

Migrate `success.rs`'s `pool()` helper + update its two tests (they now assert `out.margin` not `out.net_margin`; and set `required_successes` inside the `SuccessConfig`, not on the spec).

- [ ] **Step 8: Migrate `recalc.rs` + `proptests.rs` + `parser.rs` constructors.**
  - `recalc.rs`: `pool()`, `explode_spec_and_raws()`, `reroll_spec_and_raws()` — Sum specs → `direction: HighWins, mode: Mode::Total(TotalConfig{difficulty:None,tiers:vec![]})`; the `pool()` SuccessCount → new `SuccessConfig` shape. Update the `use` list (`Mode`, add `Direction`, `TotalConfig`, `SuccessConfig`; drop nothing still used).
  - `proptests.rs`: `simple_pool()`, `pool_with_modifiers()` → new `SuccessConfig` shape.
  - `parser.rs::parse`: build the new shape (keep the CURRENT mode-inference — `t<N>`/`ParseContext` is Task 5):

```rust
    let mode = match p.success {
        Some(rule) => Mode::SuccessCount(SuccessConfig {
            success: rule, required_successes: None, tiers: vec![],
            crit_success: None, crit_fail: None,
        }),
        None => Mode::Total(TotalConfig { difficulty: None, tiers: vec![] }),
    };
    Ok(RollSpec { expr, direction: Direction::HighWins, mode })
```

  Update `parser.rs`'s 3 affected tests: `parses_keep_highest_plus_const` + `parses_parentheses_and_mul` assert `Mode::Sum` → assert `matches!(spec.mode, Mode::Total(_))`; `parses_success_pool` destructures `Mode::SuccessCount(cfg)` and asserts `cfg.success == SuccessRule { comp: Gte, target: 7 }`.

- [ ] **Step 9: Update re-exports** in `dice/mod.rs` and `spec.rs`'s `pub use`: add `Direction, Tier, CritSuccess, CritFail, TotalConfig, SuccessConfig` to the `spec::{…}` re-export in `dice/mod.rs`.

- [ ] **Step 10: Build + full suite green.**

Run: `cargo test -p shadowcat dice`
Expected: PASS — all pre-existing dice tests + the two new serde tests. (Behavior is unchanged; any failure is a migration miss.)

Run: `cargo clippy -p shadowcat --all-targets -- -D warnings && cargo fmt --check`
Expected: clean.

- [ ] **Step 11: Commit**

```bash
git add src/server/src/dice
git commit -m "refactor(dice/m11b-1): data-carrying Mode + direction + outcome fields (behavior preserved)"
```

---

### Task 2: Direction-aware classifier + Total-mode difficulty/tiers

Build the pure classifier and wire it into Total mode. **BUDDY-CHECK.**

**Files:**
- Create: `src/server/src/dice/eval/classify.rs`
- Modify: `src/server/src/dice/eval/mod.rs` (add `pub mod classify;`), `src/server/src/dice/eval/sum.rs`

**Interfaces:**
- Produces: `classify::Classification { pass: Option<bool>, tier_label: Option<String>, tier_value: Option<i32> }`; `classify::classify(margin: i64, tiers: &[Tier]) -> Classification`; `classify::oriented_margin(direction: Direction, scalar: i64, reference: i64) -> i64`.
- Consumes (Task 4): the same `classify`/`oriented_margin` for SuccessCount.

- [ ] **Step 1: Write failing classifier tests** in `classify.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::dice::spec::{Direction, Tier};

    #[test]
    fn empty_ladder_is_pass_fail_at_zero_margin() {
        assert_eq!(classify(0, &[]).pass, Some(true));  // margin >= 0 -> pass
        assert_eq!(classify(-1, &[]).pass, Some(false)); // margin < 0 -> fail
        assert!(classify(3, &[]).tier_label.is_none());
    }

    #[test]
    fn ladder_picks_highest_rung_at_or_below_margin() {
        let tiers = vec![
            Tier { margin_offset: -10, label: Some("crit-fail".into()), tier_value: Some(0) },
            Tier { margin_offset: 0, label: Some("success".into()), tier_value: Some(2) },
            Tier { margin_offset: 10, label: Some("crit-success".into()), tier_value: Some(3) },
        ];
        assert_eq!(classify(5, &tiers).tier_value, Some(2));
        assert_eq!(classify(10, &tiers).tier_value, Some(3));
        assert_eq!(classify(-3, &tiers).tier_value, Some(0));
        assert!(classify(5, &tiers).pass.is_none(), "ladder result reports tier, not pass");
    }

    #[test]
    fn ladder_below_floor_fails_closed_to_lowest_rung() {
        let tiers = vec![
            Tier { margin_offset: 0, label: Some("ok".into()), tier_value: Some(1) },
            Tier { margin_offset: 5, label: Some("great".into()), tier_value: Some(2) },
        ];
        // margin below every offset -> lowest rung (min margin_offset), never "no tier".
        assert_eq!(classify(-100, &tiers).tier_value, Some(1));
    }

    #[test]
    fn ladder_order_independent() {
        let a = vec![
            Tier { margin_offset: 0, label: None, tier_value: Some(1) },
            Tier { margin_offset: 5, label: None, tier_value: Some(2) },
        ];
        let b = vec![a[1].clone(), a[0].clone()];
        assert_eq!(classify(6, &a).tier_value, classify(6, &b).tier_value);
    }

    #[test]
    fn oriented_margin_flips_with_direction() {
        assert_eq!(oriented_margin(Direction::HighWins, 15, 10), 5);
        assert_eq!(oriented_margin(Direction::LowWins, 8, 10), 2); // rolling under: lower is better
    }
}
```

Run: `cargo test -p shadowcat dice::eval::classify` → FAIL (module not found).

- [ ] **Step 2: Implement `classify.rs`:**

```rust
use crate::dice::spec::{Direction, Tier};

/// Classification of an oriented margin (higher = better). Mutually exclusive
/// outputs: a roll reports EITHER a `pass` (default 2-rung ladder) OR a `tier`
/// (custom ladder), never both.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Classification {
    pub pass: Option<bool>,
    pub tier_label: Option<String>,
    pub tier_value: Option<i32>,
}

/// Orient a scalar-vs-reference difference so "better" is always more positive.
/// HighWins: higher scalar is better. LowWins (roll-under): lower scalar is better.
pub fn oriented_margin(direction: Direction, scalar: i64, reference: i64) -> i64 {
    match direction {
        Direction::HighWins => scalar - reference,
        Direction::LowWins => reference - scalar,
    }
}

/// Classify `margin` against `tiers`. Empty ladder ⇒ default 2-rung pass/fail
/// (`pass = margin >= 0`). Non-empty ⇒ the highest rung with `margin_offset <=
/// margin`; if none match (margin below the floor), fail closed to the lowest
/// rung. Order-independent (no sorted precondition).
pub fn classify(margin: i64, tiers: &[Tier]) -> Classification {
    if tiers.is_empty() {
        return Classification { pass: Some(margin >= 0), tier_label: None, tier_value: None };
    }
    let chosen = tiers.iter()
        .filter(|t| (t.margin_offset as i64) <= margin)
        .max_by_key(|t| t.margin_offset)
        .or_else(|| tiers.iter().min_by_key(|t| t.margin_offset))
        .expect("tiers is non-empty");
    Classification { pass: None, tier_label: chosen.label.clone(), tier_value: chosen.tier_value }
}
```

Add `pub mod classify;` to `eval/mod.rs`.

Run: `cargo test -p shadowcat dice::eval::classify` → PASS.

- [ ] **Step 3: Write failing Total-classification tests** in `sum.rs` tests:

```rust
#[test]
fn total_no_difficulty_reports_bare_total() {
    let spec = RollSpec {
        expr: ng(1, 3, 3), // a fixed d3-min? use a const-like group; simpler: Const
        direction: Direction::HighWins,
        mode: Mode::Total(TotalConfig { difficulty: None, tiers: vec![] }),
    };
    let raws = roll(&spec, &mut NoiseRng::from_seed(7));
    let out = evaluate(&spec, &raws);
    assert!(out.pass.is_none());
    assert!(out.tier_label.is_none());
    assert!(out.margin.is_none());
}

#[test]
fn total_with_difficulty_sets_pass_by_direction() {
    // total of 1d1000-ish; use a constant expr so the total is deterministic.
    let spec_hi = RollSpec {
        expr: Expr::Const(12),
        direction: Direction::HighWins,
        mode: Mode::Total(TotalConfig { difficulty: Some(10), tiers: vec![] }),
    };
    let raws = roll(&spec_hi, &mut NoiseRng::from_seed(1));
    let out = evaluate(&spec_hi, &raws);
    assert_eq!(out.margin, Some(2));
    assert_eq!(out.pass, Some(true)); // 12 >= 10

    let spec_lo = RollSpec { direction: Direction::LowWins, ..spec_hi.clone() };
    let out_lo = evaluate(&spec_lo, &roll(&spec_lo, &mut NoiseRng::from_seed(1)));
    assert_eq!(out_lo.margin, Some(-2)); // roll-under: 12 vs 10 -> 10-12
    assert_eq!(out_lo.pass, Some(false));
}

#[test]
fn total_with_ladder_reports_tier() {
    let tiers = vec![
        Tier { margin_offset: 0, label: Some("hit".into()), tier_value: Some(1) },
        Tier { margin_offset: 5, label: Some("crit".into()), tier_value: Some(2) },
    ];
    let spec = RollSpec {
        expr: Expr::Const(17),
        direction: Direction::HighWins,
        mode: Mode::Total(TotalConfig { difficulty: Some(10), tiers }),
    };
    let out = evaluate(&spec, &roll(&spec, &mut NoiseRng::from_seed(1)));
    assert_eq!(out.tier_value, Some(2)); // margin 7 -> highest rung <= 7 is offset 5
    assert!(out.pass.is_none());
}
```

(Add `Tier`, `Direction`, `TotalConfig` to the `sum.rs` test imports.)

Run: `cargo test -p shadowcat dice::eval::sum` → the three new tests FAIL.

- [ ] **Step 4: Wire classification into `evaluate_total`:**

```rust
pub fn evaluate_total(spec: &RollSpec, cfg: &TotalConfig, raws: &RawRoll) -> RollOutcome {
    let mut next_group = 0usize;
    let total = fold(&spec.expr, raws, &mut next_group);
    let (pass, margin, tier_label, tier_value) = match cfg.difficulty {
        None => (None, None, None, None),
        Some(diff) => {
            let m = crate::dice::eval::classify::oriented_margin(spec.direction, total, diff as i64);
            let c = crate::dice::eval::classify::classify(m, &cfg.tiers);
            (c.pass, Some(m), c.tier_label, c.tier_value)
        }
    };
    RollOutcome {
        total, records: raws.records.clone(), successes: None,
        pass, margin, tier_label, tier_value,
        crit_successes: 0, crit_fails: 0, positive_counter: 0, negative_counter: 0,
    }
}
```

Run: `cargo test -p shadowcat dice` → PASS. Then clippy + fmt.

- [ ] **Step 5: Commit** — `git commit -m "feat(dice/m11b-1): direction-aware classifier + Total-mode difficulty/tiers"`

---

### Task 3: Crit events + counters (SuccessCount)

Add crit-success/crit-fail scoring, net successes, counters, and the `DieRecord` crit flags. **BUDDY-CHECK.**

**Files:**
- Create: `src/server/src/dice/eval/crit.rs`
- Modify: `src/server/src/dice/eval/mod.rs` (`pub mod crit;`), `src/server/src/dice/eval/success.rs`

**Interfaces:**
- Produces: `crit::DieCrit { is_success: bool, is_fail: bool, extra_successes: i32, lost: i32, positive_counter: i32, negative_counter: i32 }`; `crit::score_die(direction, value, cfg) -> DieCrit`.
- Consumes: `Direction`, `SuccessConfig` from Task 1.

- [ ] **Step 1: Write failing crit tests** in `crit.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::dice::spec::{CritFail, CritSuccess, Direction, SuccessConfig, SuccessRule, Comparator};

    fn cfg(cs: Option<CritSuccess>, cf: Option<CritFail>) -> SuccessConfig {
        SuccessConfig {
            success: SuccessRule { comp: Comparator::Gte, target: 7 },
            required_successes: None, tiers: vec![], crit_success: cs, crit_fail: cf,
        }
    }

    #[test]
    fn crit_success_at_or_above_threshold_highwins() {
        let c = cfg(Some(CritSuccess { threshold: 10, extra_successes: 2, positive_counter: 1 }), None);
        let hit = score_die(Direction::HighWins, 10, &c);
        assert!(hit.is_success && hit.extra_successes == 2 && hit.positive_counter == 1);
        assert!(!score_die(Direction::HighWins, 9, &c).is_success);
    }

    #[test]
    fn crit_thresholds_flip_under_lowwins() {
        // LowWins crit-success fires at/below threshold.
        let c = cfg(Some(CritSuccess { threshold: 1, extra_successes: 1, positive_counter: 0 }), None);
        assert!(score_die(Direction::LowWins, 1, &c).is_success);
        assert!(!score_die(Direction::LowWins, 2, &c).is_success);
    }

    #[test]
    fn crit_fail_reports_loss_and_negative_counter() {
        let c = cfg(None, Some(CritFail { threshold: 1, lost: 1, negative_counter: 1, allow_negative: false }));
        let f = score_die(Direction::HighWins, 1, &c);
        assert!(f.is_fail && f.lost == 1 && f.negative_counter == 1);
    }
}
```

Run: `cargo test -p shadowcat dice::eval::crit` → FAIL.

- [ ] **Step 2: Implement `crit.rs`:**

```rust
use crate::dice::spec::{Direction, SuccessConfig};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DieCrit {
    pub is_success: bool,
    pub is_fail: bool,
    pub extra_successes: i32,
    pub lost: i32,
    pub positive_counter: i32,
    pub negative_counter: i32,
}

/// HighWins: crit-success fires at value >= threshold, crit-fail at value <= threshold.
/// LowWins: both comparisons flip (crit-success at <= threshold, crit-fail at >= threshold).
fn reaches(direction: Direction, value: i32, threshold: i32, is_success_event: bool) -> bool {
    match (direction, is_success_event) {
        (Direction::HighWins, true) | (Direction::LowWins, false) => value >= threshold,
        (Direction::HighWins, false) | (Direction::LowWins, true) => value <= threshold,
    }
}

pub fn score_die(direction: Direction, value: i32, cfg: &SuccessConfig) -> DieCrit {
    let mut out = DieCrit::default();
    if let Some(cs) = &cfg.crit_success {
        if reaches(direction, value, cs.threshold, true) {
            out.is_success = true;
            out.extra_successes = cs.extra_successes;
            out.positive_counter = cs.positive_counter;
        }
    }
    if let Some(cf) = &cfg.crit_fail {
        if reaches(direction, value, cf.threshold, false) {
            out.is_fail = true;
            out.lost = cf.lost;
            out.negative_counter = cf.negative_counter;
        }
    }
    out
}
```

Add `pub mod crit;` to `eval/mod.rs`. Run the crit tests → PASS.

- [ ] **Step 3: Write failing SuccessCount integration tests** in `success.rs` (net successes, clamp, counters, flags):

```rust
#[test]
fn crit_success_adds_extra_and_counter() {
    // 1d10 with target>=7, crit_success at 10 (+1 extra, +1 pos counter).
    // Use a scripted-natural RawRoll to force a 10. Build raws via roll then assert
    // over a deterministic seed that yields a 10, OR construct raws directly.
    let cfg = SuccessConfig {
        success: SuccessRule { comp: Comparator::Gte, target: 7 },
        required_successes: None, tiers: vec![],
        crit_success: Some(CritSuccess { threshold: 10, extra_successes: 1, positive_counter: 1 }),
        crit_fail: None,
    };
    let spec = RollSpec { expr: Expr::Dice(DiceGroup { count: 1, kind: DieKind::Numeric { min: 10, max: 10 }, modifiers: vec![] }),
                          direction: Direction::HighWins, mode: Mode::SuccessCount(cfg) };
    let out = evaluate(&spec, &roll(&spec, &mut NoiseRng::from_seed(1)));
    assert_eq!(out.successes, Some(2)); // base 1 + crit extra 1
    assert_eq!(out.positive_counter, 1);
    assert_eq!(out.crit_successes, 1);
    assert!(out.records[0].crit_success);
}

#[test]
fn crit_fail_clamps_net_at_zero_unless_allowed() {
    // Single die at min=max=1: base success fails, crit_fail loses 1.
    let mk = |allow: bool| RollSpec {
        expr: Expr::Dice(DiceGroup { count: 1, kind: DieKind::Numeric { min: 1, max: 1 }, modifiers: vec![] }),
        direction: Direction::HighWins,
        mode: Mode::SuccessCount(SuccessConfig {
            success: SuccessRule { comp: Comparator::Gte, target: 7 },
            required_successes: None, tiers: vec![], crit_success: None,
            crit_fail: Some(CritFail { threshold: 1, lost: 1, negative_counter: 1, allow_negative: allow }),
        }),
    };
    let clamped = mk(false);
    let o1 = evaluate(&clamped, &roll(&clamped, &mut NoiseRng::from_seed(1)));
    assert_eq!(o1.successes, Some(0)); // 0 base - 1 lost, clamped at 0
    assert_eq!(o1.negative_counter, 1);
    assert_eq!(o1.crit_fails, 1);
    let neg = mk(true);
    let o2 = evaluate(&neg, &roll(&neg, &mut NoiseRng::from_seed(1)));
    assert_eq!(o2.successes, Some(-1)); // allow_negative
}
```

(Add imports: `CritSuccess`, `CritFail`, `Direction`, `Mode`, `SuccessConfig`, `Expr`, `DiceGroup`.)

Run → FAIL.

- [ ] **Step 4: Extend `evaluate_success`** to fold crit per kept die, set flags on a mutable records copy, and compute net + counters:

```rust
pub fn evaluate_success(spec: &RollSpec, cfg: &SuccessConfig, raws: &RawRoll) -> RollOutcome {
    use crate::dice::eval::crit::score_die;
    let mut records = raws.records.clone();
    let mut base = 0i32;
    let (mut extra, mut lost) = (0i32, 0i32);
    let (mut pos, mut neg) = (0i32, 0i32);
    let (mut crit_s, mut crit_f) = (0i32, 0i32);
    for r in records.iter_mut().filter(|r| r.kept) {
        if cfg.success.comp.test(r.value, cfg.success.target) { base += 1; }
        let dc = score_die(spec.direction, r.value, cfg);
        r.crit_success = dc.is_success;
        r.crit_fail = dc.is_fail;
        if dc.is_success { crit_s += 1; }
        if dc.is_fail { crit_f += 1; }
        extra += dc.extra_successes;
        lost += dc.lost;
        pos += dc.positive_counter;
        neg += dc.negative_counter;
    }
    let raw_net = base + extra - lost;
    let allow_neg = cfg.crit_fail.as_ref().map(|c| c.allow_negative).unwrap_or(false);
    let net = if allow_neg { raw_net } else { raw_net.max(0) };
    let total: i64 = records.iter().filter(|r| r.kept).map(|r| r.value as i64).sum();
    // NOTE: required_successes / tiers classification is Task 4; keep the M11a
    // pass/margin here for now (over `net`, not `base`).
    let (pass, margin) = match cfg.required_successes {
        Some(req) => (Some(net >= req), Some((net - req) as i64)),
        None => (None, None),
    };
    RollOutcome {
        total, records, successes: Some(net),
        pass, margin, tier_label: None, tier_value: None,
        crit_successes: crit_s, crit_fails: crit_f,
        positive_counter: pos, negative_counter: neg,
    }
}
```

Run: `cargo test -p shadowcat dice` → PASS (existing SuccessCount tests still hold: with no crit configured, `net == base`, `crit_*==0`). clippy + fmt.

- [ ] **Step 5: Commit** — `git commit -m "feat(dice/m11b-1): crit-success/crit-fail events, net successes, counters"`

---

### Task 4: SuccessCount classification via the shared classifier (tiers over net-success margin)

Replace the inline pass/margin in `evaluate_success` with the shared `classify`, so SuccessCount supports tier ladders ("succeed with N extra successes"). Margin here is **NOT** direction-flipped (more successes is always better — direction was already applied per-die).

**Files:** Modify `src/server/src/dice/eval/success.rs`.

- [ ] **Step 1: Write failing tests** in `success.rs`:

```rust
#[test]
fn successcount_tier_over_net_margin() {
    // required 1, tiers: offset 0 = "success", offset 2 = "success+2 extra".
    let tiers = vec![
        Tier { margin_offset: 0, label: Some("success".into()), tier_value: Some(1) },
        Tier { margin_offset: 2, label: Some("great".into()), tier_value: Some(2) },
    ];
    // 3 dice all min=max=10, target>=7 -> 3 successes, required 1 -> margin 2 -> "great".
    let spec = RollSpec {
        expr: Expr::Dice(DiceGroup { count: 3, kind: DieKind::Numeric { min: 10, max: 10 }, modifiers: vec![] }),
        direction: Direction::HighWins,
        mode: Mode::SuccessCount(SuccessConfig {
            success: SuccessRule { comp: Comparator::Gte, target: 7 },
            required_successes: Some(1), tiers, crit_success: None, crit_fail: None,
        }),
    };
    let out = evaluate(&spec, &roll(&spec, &mut NoiseRng::from_seed(1)));
    assert_eq!(out.margin, Some(2));
    assert_eq!(out.tier_value, Some(2));
    assert!(out.pass.is_none(), "custom ladder reports tier, not pass");
}
```

(The existing `required_successes_sets_pass_and_margin` test still asserts `pass`/`margin` for the **empty-ladder** case — keep it.)

Run → FAIL.

- [ ] **Step 2: Swap the inline pass/margin for the shared classifier** in `evaluate_success` (replace the `let (pass, margin) = …` block):

```rust
    use crate::dice::eval::classify::classify;
    let (pass, margin, tier_label, tier_value) = match cfg.required_successes {
        None => (None, None, None, None),
        Some(req) => {
            // Direction is NOT applied here: more successes is always better.
            let m = (net - req) as i64;
            let c = classify(m, &cfg.tiers);
            (c.pass, Some(m), c.tier_label, c.tier_value)
        }
    };
```

and populate `tier_label`/`tier_value` in the returned `RollOutcome`.

Run: `cargo test -p shadowcat dice` → PASS. clippy + fmt.

- [ ] **Step 3: Commit** — `git commit -m "feat(dice/m11b-1): SuccessCount tier classification over net-success margin"`

---

### Task 5: Notation — unified `t<N>` target + ambient `ParseContext`

Add the mode-agnostic `t<N>` target token and make mode + direction ambient (caller-supplied), per the design's notation pillar (§10).

**Files:**
- Modify: `src/server/src/dice/notation/mod.rs` (add `ParseContext`, `ModeKind`, `ModeConflict` error), `src/server/src/dice/notation/parser.rs`, `src/server/src/dice/mod.rs` (re-export `ParseContext`, `ModeKind`).

**Interfaces:**
- Produces: `ParseContext { mode: ModeKind, direction: Direction }` (`Default` = `Total`/`HighWins`); `ModeKind { Total, SuccessCount }`; `parse(input: &str, ctx: ParseContext) -> Result<RollSpec, ParseError>`.

- [ ] **Step 1: Add `ParseContext`/`ModeKind` + error** to `notation/mod.rs`:

```rust
use crate::dice::spec::Direction;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeKind { Total, SuccessCount }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseContext { pub mode: ModeKind, pub direction: Direction }

impl Default for ParseContext {
    fn default() -> Self { ParseContext { mode: ModeKind::Total, direction: Direction::HighWins } }
}
```

Add to `ParseError`: `/// A t<N> target and a cs/cf rule both set the per-die success rule.` — reuse the existing `DuplicateSuccessRule` for that collision (no new variant needed).

- [ ] **Step 2: Write failing parser tests** (in `parser.rs` tests; note the new 2-arg `parse`):

```rust
use crate::dice::notation::{ParseContext, ModeKind};
use crate::dice::spec::Direction;

#[test]
fn t_target_in_total_mode_sets_difficulty() {
    let spec = parse("1d20t10", ParseContext::default()).unwrap();
    match spec.mode { Mode::Total(c) => assert_eq!(c.difficulty, Some(10)), _ => panic!() }
}

#[test]
fn t_target_in_successcount_uses_direction_comparator() {
    let hi = parse("5d10t7", ParseContext { mode: ModeKind::SuccessCount, direction: Direction::HighWins }).unwrap();
    match hi.mode { Mode::SuccessCount(c) => assert_eq!(c.success, SuccessRule { comp: Comparator::Gte, target: 7 }), _ => panic!() }
    let lo = parse("5d10t7", ParseContext { mode: ModeKind::SuccessCount, direction: Direction::LowWins }).unwrap();
    match lo.mode { Mode::SuccessCount(c) => assert_eq!(c.success, SuccessRule { comp: Comparator::Lte, target: 7 }), _ => panic!() }
}

#[test]
fn cs_forces_successcount_even_under_total_ambient() {
    let spec = parse("5d10cs>=7", ParseContext { mode: ModeKind::Total, direction: Direction::HighWins }).unwrap();
    assert!(matches!(spec.mode, Mode::SuccessCount(_)));
}

#[test]
fn t_and_cs_collision_errors() {
    let e = parse("5d10t6cs>=7", ParseContext { mode: ModeKind::SuccessCount, direction: Direction::HighWins });
    assert!(matches!(e, Err(ParseError::DuplicateSuccessRule)));
}
```

Update **all** existing `parse("…")` call sites in `parser.rs` tests to `parse("…", ParseContext::default())`. Run → the pre-existing tests compile-fail until updated; the four new tests FAIL.

- [ ] **Step 3: Implement.** Add a `t` field to `P` and a `"t"` arm; thread `ctx`:

```rust
// struct P { toks, pos, success: Option<SuccessRule>, t_target: Option<i32> }
// in modifiers(), Ident arm:
    "t" => {
        let n = self.expect_int()?;
        if self.t_target.is_some() { return Err(ParseError::DuplicateSuccessRule); }
        self.t_target = Some(n);
    }
```

Rewrite `parse`:

```rust
pub fn parse(input: &str, ctx: ParseContext) -> Result<RollSpec, ParseError> {
    let toks = lex(input)?;
    if toks.is_empty() { return Err(ParseError::Empty); }
    let mut p = P { toks, pos: 0, success: None, t_target: None };
    let expr = p.expr()?;
    if p.pos != p.toks.len() { return Err(ParseError::Trailing(format!("{:?}", p.toks[p.pos]))); }

    // Explicit cs/cf forces SuccessCount; otherwise the ambient mode governs.
    let success_count = p.success.is_some() || ctx.mode == ModeKind::SuccessCount;
    let mode = if success_count {
        // Resolve the per-die success rule: cs/cf rule, else a t<N> target with a
        // direction-derived comparator. Both present => collision.
        let rule = match (p.success, p.t_target) {
            (Some(_), Some(_)) => return Err(ParseError::DuplicateSuccessRule),
            (Some(r), None) => r,
            (None, Some(t)) => SuccessRule {
                comp: match ctx.direction { Direction::HighWins => Comparator::Gte, Direction::LowWins => Comparator::Lte },
                target: t,
            },
            (None, None) => return Err(ParseError::Unexpected("SuccessCount mode requires a per-die target (t<N> or cs)".into())),
        };
        Mode::SuccessCount(SuccessConfig { success: rule, required_successes: None, tiers: vec![], crit_success: None, crit_fail: None })
    } else {
        Mode::Total(TotalConfig { difficulty: p.t_target, tiers: vec![] })
    };
    Ok(RollSpec { expr, direction: ctx.direction, mode })
}
```

Update the `use` list (add `ModeKind`, `ParseContext`, `Direction`, `TotalConfig`, `SuccessConfig`, `Mode`). Re-export `ParseContext`/`ModeKind` from `dice/mod.rs`.

Run: `cargo test -p shadowcat dice::notation` → PASS. Then `cargo test -p shadowcat dice` (no non-test caller of `parse` exists yet, so no other breakage). clippy + fmt.

- [ ] **Step 4: Commit** — `git commit -m "feat(dice/m11b-1): unified t<N> target notation + ambient ParseContext"`

---

### Task 6: Property tests — direction mirror symmetry + ambient-target invariance

**Files:** Modify `src/server/src/dice/proptests.rs`.

- [ ] **Step 1: Add the direction-flip mirror-symmetry property.** Flipping `direction` and mirroring every die face (`f → min+max−f`) must leave the SuccessCount net successes invariant when the per-die comparator and crit thresholds are mirrored too. Simplest robust form: build a symmetric pool where the target sits at the range midpoint so the mirror maps successes↔successes.

```rust
proptest! {
    #[test]
    fn direction_flip_mirrors_success_count(seed in any::<u64>(), count in 1u32..8) {
        // d10, HighWins target >=6 vs LowWins target <=5 are mirror images
        // (face f maps to 11-f; f>=6  <=>  11-f<=5). Same natural faces, so the
        // success COUNT must match across the mirrored spec pair.
        use crate::dice::spec::*;
        let hi = RollSpec {
            expr: Expr::Dice(DiceGroup { count, kind: DieKind::Numeric { min: 1, max: 10 }, modifiers: vec![] }),
            direction: Direction::HighWins,
            mode: Mode::SuccessCount(SuccessConfig { success: SuccessRule { comp: Comparator::Gte, target: 6 },
                required_successes: None, tiers: vec![], crit_success: None, crit_fail: None }),
        };
        let raws = roll(&hi, &mut NoiseRng::from_seed(seed));
        let hi_succ = evaluate(&hi, &raws).successes.unwrap();
        // Mirror the faces into a fresh raws and evaluate the LowWins spec.
        let mut mirrored = raws.clone();
        for d in mirrored.dice.iter_mut() { d.natural = 11 - d.natural; }
        for r in mirrored.records.iter_mut() { r.value = 11 - r.value; r.natural = 11 - r.natural; }
        let lo = RollSpec { direction: Direction::LowWins,
            mode: Mode::SuccessCount(SuccessConfig { success: SuccessRule { comp: Comparator::Lte, target: 5 },
                required_successes: None, tiers: vec![], crit_success: None, crit_fail: None }), ..hi.clone() };
        prop_assert_eq!(hi_succ, evaluate(&lo, &mirrored).successes.unwrap());
    }
}
```

- [ ] **Step 2: Add the ambient-target invariance property** (notation pillar): for a fixed expression + target, the `t<N>` value reaches the mode-appropriate field regardless of mode.

```rust
proptest! {
    #[test]
    fn t_target_reaches_mode_appropriate_field(sides in 2i32..20, target in 1i32..20) {
        use crate::dice::notation::{parse, ParseContext, ModeKind};
        use crate::dice::spec::{Mode, Direction};
        let s = format!("1d{sides}t{target}");
        let total = parse(&s, ParseContext { mode: ModeKind::Total, direction: Direction::HighWins }).unwrap();
        let sc = parse(&s, ParseContext { mode: ModeKind::SuccessCount, direction: Direction::HighWins }).unwrap();
        match total.mode { Mode::Total(c) => prop_assert_eq!(c.difficulty, Some(target)), _ => prop_assert!(false) }
        match sc.mode { Mode::SuccessCount(c) => prop_assert_eq!(c.success.target, target), _ => prop_assert!(false) }
    }
}
```

- [ ] **Step 3: Run** `cargo test -p shadowcat dice::proptests` → PASS. clippy + fmt.

- [ ] **Step 4: Commit** — `git commit -m "test(dice/m11b-1): direction mirror-symmetry + ambient-target property tests"`

---

### Task 7: Reviewed codebase-skill gate + docs sync

Per CLAUDE.md's reviewed skill-update gate (blocks completion at the doc-sync tier).

**Files:**
- Modify: `.claude/skills/shadowcat-codebase-dice/SKILL.md` (path per the skill's actual location), `docs/PLAN.md`.

- [ ] **Step 1: Update `shadowcat-codebase-dice`** to reflect: the data-carrying `Mode` (`Total | SuccessCount`) + `direction`; the shared `eval::classify` layer (margin → pass/tier, order-independent, direction-oriented for Total but NOT for SuccessCount); crit events (`eval::crit`, net-success clamp, counters as separate output, DieRecord crit flags); the `t<N>` + `ParseContext` ambient-mode notation. Note what's still deferred (expertise = b-2; labeled/custom-face = b-3).

- [ ] **Step 2: Update `docs/PLAN.md`** M11b entry: mark M11b-1 (globals + classification + crit) DONE with a one-line summary; leave b-2/b-3 pending.

- [ ] **Step 3: Dispatch `shadowcat-spec-reviewer`** on the skill diff to confirm it accurately captures the implemented change (no omission/drift/broken pointer). Fix any finding; re-confirm.

- [ ] **Step 4: Commit** — `git commit -m "docs(dice/m11b-1): update dice codebase skill + PLAN.md (reviewed gate)"`

---

## Self-review

- **Spec coverage:** direction (§4 → T1/T2/T3/T5), data-carrying Mode (§4 → T1), Sum≡Tiered/Total (§6 → T1/T2), shared classification + optional thresholds + tiers-both-modes (§5, §7 → T2/T4), crit events + counters (§7 → T3), `t<N>` unified notation + ambient mode + direction comparator (§10 → T5), RollOutcome final shape (§11 → T1 defines, T2/T3/T4 populate), property tests incl. mirror symmetry (§13 → T6), skill gate (§15 → T7). Expertise (§8) and die-models (§9) are explicitly out of b-1. **No gaps.**
- **Placeholder scan:** none — every step has concrete code/commands. (Task 1's mechanical migration is specified as an exact transformation rule + the full site list, not "similar to.")
- **Type consistency:** `evaluate_total(spec, cfg, raws)` / `evaluate_success(spec, cfg, raws)` consistent T1→T4; `classify(margin: i64, &[Tier])` / `oriented_margin(Direction, i64, i64)` consistent T2→T4; `score_die(direction, value, cfg)` T3; `parse(input, ParseContext)` T5; `margin: Option<i64>` throughout. RollOutcome field names (`crit_successes`, `crit_fails`, `positive_counter`, `negative_counter`, `tier_label`, `tier_value`) consistent across T1/T2/T3/T4.
