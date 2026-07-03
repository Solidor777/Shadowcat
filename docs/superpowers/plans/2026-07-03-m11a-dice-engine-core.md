# M11a Dice Engine Core — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the server-side, pure-Rust core dice engine — a struct-canonical `RollSpec`, a seeded-noise RNG, `roll`/`evaluate`/`recalculate`, the standard-notation parser, and the Sum + SuccessCount aggregation modes — verified by unit + property tests, with no wire frames.

**Architecture:** A pure library at `src/server/src/dice/` with no `ws`/`data` dependencies. A roll is a canonical `RollSpec` (an expression AST of dice + arithmetic, plus a mode and success config). `roll(spec, rng) -> RawRoll` is the only randomness step; `evaluate(spec, raws) -> RollOutcome` is deterministic; `recalculate` applies targeted die ops then re-evaluates. Randomness is a stateless noise function (`noise(seed, index)`), so results are deterministic and reproducible by construction. String notation is one parser front-end producing a `RollSpec`. This plan is M11a (core); the nine system rules (expertise DP, crit events, Tiered mode, labeled/custom-face dice, `direction` global flip) are M11b, planned separately after this lands.

**Tech Stack:** Rust (crate `shadowcat`), a hand-rolled SplitMix64 seeded-noise RNG (no `rand` dependency), `serde`, `proptest` (dev). `cargo test` / `cargo fmt` / `cargo clippy`.

## Global Constraints

- **Crate:** `shadowcat` (server). All new code under `src/server/src/dice/`. Register with `pub mod dice;` in `src/server/src/lib.rs`.
- **Pure library:** the `dice` module MUST NOT depend on `ws`, `data`, `http`, or `scene`. No wire frames, no protocol changes, no transport in M11a/b.
- **No ts-rs in M11a/b:** types stay pure Rust. ts-rs bindings are added by the consuming milestone (M11d). Do NOT add `#[derive(TS)]`/`#[ts(export)]` here (avoids generating `.ts` with no Zod mirror, which trips the drift guard).
- **Seeded-noise randomness, no runtime RNG dependency:** randomness is a hand-rolled stateless noise/hash (SplitMix64 finalizer, public-domain constants) wrapped in a seeded generator — deterministic and reproducible by design. The ONLY new dependency is `proptest = "1"` in `[dev-dependencies]`. **Production seed caveat:** a real roll's seed MUST fold in high-entropy material (OS entropy or a server secret), NOT a bare timestamp+counter, or rolls become predictable/cheatable. Entropy seeding lives at the consuming transport boundary (M11d); M11a/b stay pure and seed-agnostic — the caller supplies the seeded generator, and stored natural faces already make every roll reproducible for evaluate/recalc without persisting the seed.
- **Cross-platform:** no OS-specific code; the noise RNG is pure integer arithmetic (portable). Three-OS CI must stay green.
- **Style:** `cargo fmt` + `cargo clippy` clean (no warnings) at every commit. TDD: failing test → minimal impl → passing test → commit.
- **Determinism invariant:** given the same `RollSpec` and the same `RawRoll`, `evaluate` MUST return an identical `RollOutcome`. All nondeterminism lives in `roll`/`recalculate` (the noise RNG) only.

## File Structure

- `src/server/src/dice/mod.rs` — module root; re-exports the public API (`RollSpec`, `RawRoll`, `RollOutcome`, `RollResult`, `roll`, `evaluate`, `recalculate`, `parse`); integration tests.
- `src/server/src/dice/rng.rs` — `noise`, `RngSource` trait, `NoiseRng`, `roll_uniform`.
- `src/server/src/dice/spec.rs` — `RollSpec`, `Expr`, `DiceGroup`, `DieKind`, `GroupModifier`, `ExplodeKind`, `Comparator`, `BinOp`, `Mode`, `SuccessRule`.
- `src/server/src/dice/outcome.rs` — `RawRoll`, `RawDie`, `RollOutcome`, `DieRecord`, `RollResult`.
- `src/server/src/dice/eval/mod.rs` — `roll` + `evaluate` dispatch.
- `src/server/src/dice/eval/groups.rs` — per-group reroll/explode/keep-drop producing `DieRecord`s.
- `src/server/src/dice/eval/sum.rs` — Sum-mode AST folding.
- `src/server/src/dice/eval/success.rs` — SuccessCount aggregation.
- `src/server/src/dice/recalc.rs` — `recalculate` + `RecalcOp`.
- `src/server/src/dice/notation/mod.rs` — `parse` + shared `ParseError`.
- `src/server/src/dice/notation/lexer.rs` — tokenizer.
- `src/server/src/dice/notation/parser.rs` — recursive-descent grammar → `RollSpec`.
- `src/server/src/dice/proptests.rs` — cross-module property tests.

Unit tests live inline (`#[cfg(test)] mod tests`) in each file.

---

### Task 1: Seeded-noise RNG source + module scaffold

**Files:**
- Modify: `src/server/src/lib.rs` (add `pub mod dice;`)
- Create: `src/server/src/dice/mod.rs`
- Create: `src/server/src/dice/rng.rs`

**Interfaces:**
- Produces:
  - `fn noise(seed: u64, n: u64) -> u64` — stateless SplitMix64 noise; `noise(seed, n)` depends only on its inputs.
  - `trait RngSource { fn next_u32(&mut self) -> u32; }`
  - `struct NoiseRng` with `NoiseRng::from_seed(seed: u64) -> Self` (implements `RngSource`) and `NoiseRng::at(seed, index) -> u64`.
  - `fn roll_uniform(rng: &mut dyn RngSource, min: i32, max: i32) -> i32` — inclusive `[min,max]`, unbiased.

- [ ] **Step 1: Register the module**

In `src/server/src/lib.rs`, add the declaration in alphabetical position (after `pub mod data;`):

```rust
pub mod dice;
```

- [ ] **Step 2: Create the module root**

Create `src/server/src/dice/mod.rs`:

```rust
//! Server-authoritative dice engine. Pure library: a struct-canonical `RollSpec`
//! is rolled by `roll` (the only randomness step) and scored by `evaluate`
//! (deterministic). Randomness is a stateless noise function, so any roll is
//! reproducible from its seed. INVARIANT: (spec, raws) fully determines the outcome.

pub mod rng;
```

- [ ] **Step 3: Write the failing RNG test**

Create `src/server/src/dice/rng.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_is_deterministic() {
        let mut a = NoiseRng::from_seed(42);
        let mut b = NoiseRng::from_seed(42);
        let xs: Vec<i32> = (0..20).map(|_| roll_uniform(&mut a, 1, 6)).collect();
        let ys: Vec<i32> = (0..20).map(|_| roll_uniform(&mut b, 1, 6)).collect();
        assert_eq!(xs, ys);
    }

    #[test]
    fn different_seeds_differ() {
        let mut a = NoiseRng::from_seed(1);
        let mut b = NoiseRng::from_seed(2);
        let xs: Vec<i32> = (0..50).map(|_| roll_uniform(&mut a, 1, 100)).collect();
        let ys: Vec<i32> = (0..50).map(|_| roll_uniform(&mut b, 1, 100)).collect();
        assert_ne!(xs, ys);
    }

    #[test]
    fn roll_uniform_stays_in_range() {
        let mut r = NoiseRng::from_seed(7);
        for _ in 0..1000 {
            let v = roll_uniform(&mut r, 3, 8);
            assert!((3..=8).contains(&v), "out of range: {v}");
        }
    }

    #[test]
    fn roll_uniform_degenerate_range() {
        let mut r = NoiseRng::from_seed(1);
        assert_eq!(roll_uniform(&mut r, 5, 5), 5);
    }

    #[test]
    fn at_is_positionally_stable() {
        assert_eq!(NoiseRng::at(123, 4), NoiseRng::at(123, 4));
        assert_ne!(NoiseRng::at(123, 4), NoiseRng::at(123, 5));
    }
}
```

- [ ] **Step 4: Run the test to verify it fails**

Run: `cargo test -p shadowcat dice::rng`
Expected: FAIL — `noise`/`NoiseRng`/`roll_uniform` not found (does not compile).

- [ ] **Step 5: Implement the seeded-noise RNG**

Prepend to `src/server/src/dice/rng.rs` (above the test module):

```rust
/// Stateless 64-bit noise (SplitMix64 finalizer). Deterministic: `noise(seed, n)`
/// depends only on its inputs, so any die is reproducible from (seed, index) with no
/// carried state. Source: SplitMix64 [Steele, Lea & Flood 2014]; constants are the
/// published golden-ratio increment + two mixing multipliers. Chosen over a stateful
/// PRNG because a dice engine needs position-based reproducibility for recalculation.
pub fn noise(seed: u64, n: u64) -> u64 {
    let mut z = seed.wrapping_add(n.wrapping_add(1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Abstract randomness source: tests seed deterministically; production seeds from
/// entropy at the transport boundary. Trait-object friendly (`&mut dyn RngSource`).
pub trait RngSource {
    fn next_u32(&mut self) -> u32;
}

/// Deterministic generator over the noise function: output i = `noise(seed, i)`,
/// advancing an index counter. Reproducible: rebuild with the same seed to replay.
pub struct NoiseRng {
    seed: u64,
    index: u64,
}

impl NoiseRng {
    pub fn from_seed(seed: u64) -> Self {
        NoiseRng { seed, index: 0 }
    }

    /// Derive a value at an explicit index without advancing — for position-based
    /// per-die derivation (e.g. recalculation of a specific die).
    pub fn at(seed: u64, index: u64) -> u64 {
        noise(seed, index)
    }
}

impl RngSource for NoiseRng {
    fn next_u32(&mut self) -> u32 {
        let v = noise(self.seed, self.index) as u32;
        self.index += 1;
        v
    }
}

/// Unbiased inclusive `[min, max]` draw via rejection sampling (drop the biased tail
/// above the largest multiple of the span). PRECONDITION: `min <= max`. Rejection
/// sampling avoids the modulo bias of `next_u32() % span`.
pub fn roll_uniform(rng: &mut dyn RngSource, min: i32, max: i32) -> i32 {
    debug_assert!(min <= max, "roll_uniform requires min <= max");
    let span = (max as i64 - min as i64 + 1) as u64; // 1..=2^32
    if span == 1 {
        return min;
    }
    let span32 = span as u32;
    let limit = u32::MAX - (u32::MAX % span32);
    loop {
        let x = rng.next_u32();
        if x < limit {
            return min + (x % span32) as i32;
        }
    }
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p shadowcat dice::rng`
Expected: PASS (5 tests).

- [ ] **Step 7: Commit**

```bash
cd src/server
cargo fmt && cargo clippy -p shadowcat --all-targets -- -D warnings
cd ../..
git add src/server/src/lib.rs src/server/src/dice/mod.rs src/server/src/dice/rng.rs
git commit -m "feat(dice): seeded-noise RNG source + module scaffold (M11a)"
```

---

### Task 2: Core spec types

**Files:**
- Create: `src/server/src/dice/spec.rs`
- Modify: `src/server/src/dice/mod.rs` (add `pub mod spec;` + re-exports)

**Interfaces:**
- Produces (exact shapes later tasks depend on):
  - `type DieId = u32`
  - `enum DieKind { Numeric { min: i32, max: i32 } }`
  - `enum Comparator { Eq, Ne, Gt, Lt, Gte, Lte }` with `fn test(self, value: i32, target: i32) -> bool`
  - `enum ExplodeKind { Standard, Compound, Penetrate }`
  - `enum GroupModifier { KeepHighest(u32), KeepLowest(u32), DropHighest(u32), DropLowest(u32), Explode { kind: ExplodeKind, comp: Comparator, target: i32 }, Reroll { comp: Comparator, target: i32, once: bool } }`
  - `struct DiceGroup { count: u32, kind: DieKind, modifiers: Vec<GroupModifier> }`
  - `enum BinOp { Add, Sub, Mul, Div }`
  - `enum Expr { Dice(DiceGroup), Const(i32), Bin { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> }, Neg(Box<Expr>) }`
  - `enum Mode { Sum, SuccessCount }`
  - `struct SuccessRule { comp: Comparator, target: i32 }`
  - `struct RollSpec { expr: Expr, mode: Mode, success: Option<SuccessRule>, required_successes: Option<i32> }`

- [ ] **Step 1: Write the failing test**

Create `src/server/src/dice/spec.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparator_tests_each_op() {
        assert!(Comparator::Gte.test(7, 7));
        assert!(Comparator::Gte.test(8, 7));
        assert!(!Comparator::Gte.test(6, 7));
        assert!(Comparator::Lt.test(6, 7));
        assert!(!Comparator::Lt.test(7, 7));
        assert!(Comparator::Eq.test(5, 5));
        assert!(Comparator::Ne.test(5, 6));
    }

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
            mode: Mode::Sum,
            success: None,
            required_successes: None,
        };
        let json = serde_json::to_string(&spec).unwrap();
        let back: RollSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(spec, back);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p shadowcat dice::spec`
Expected: FAIL — types not defined.

- [ ] **Step 3: Implement the types**

Prepend to `src/server/src/dice/spec.rs`:

```rust
use serde::{Deserialize, Serialize};

/// Stable identity of a rolled die within one roll; lets `recalculate` target a subset.
pub type DieId = u32;

/// A die's face space. M11a: numeric only; M11b adds `Faces` for custom-symbol dice.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DieKind {
    Numeric { min: i32, max: i32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Comparator {
    Eq,
    Ne,
    Gt,
    Lt,
    Gte,
    Lte,
}

impl Comparator {
    pub fn test(self, value: i32, target: i32) -> bool {
        match self {
            Comparator::Eq => value == target,
            Comparator::Ne => value != target,
            Comparator::Gt => value > target,
            Comparator::Lt => value < target,
            Comparator::Gte => value >= target,
            Comparator::Lte => value <= target,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExplodeKind {
    /// Roll an extra die per trigger; each extra can itself trigger.
    Standard,
    /// Add the extra roll into the same die's value (one combined result).
    Compound,
    /// Standard, but each successive extra die is reduced by 1.
    Penetrate,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GroupModifier {
    KeepHighest(u32),
    KeepLowest(u32),
    DropHighest(u32),
    DropLowest(u32),
    Explode {
        kind: ExplodeKind,
        comp: Comparator,
        target: i32,
    },
    Reroll {
        comp: Comparator,
        target: i32,
        once: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiceGroup {
    pub count: u32,
    pub kind: DieKind,
    /// Applied in vec order: reroll/explode alter the die set, keep/drop select from it.
    pub modifiers: Vec<GroupModifier>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}

/// Roll expression AST. Sum mode folds this to a total; SuccessCount mode ignores
/// the arithmetic and pools the dice reachable from `Dice` nodes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Expr {
    Dice(DiceGroup),
    Const(i32),
    Bin {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Neg(Box<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Sum,
    SuccessCount,
}

/// SuccessCount dimension 1: the per-die target a die must satisfy to score a success.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuccessRule {
    pub comp: Comparator,
    pub target: i32,
}

/// Canonical roll parameters. Notation parses INTO this; recalculation re-runs it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollSpec {
    pub expr: Expr,
    pub mode: Mode,
    /// Required when `mode == SuccessCount`.
    pub success: Option<SuccessRule>,
    /// SuccessCount dimension 2 (optional): successes needed for an overall pass.
    pub required_successes: Option<i32>,
}
```

- [ ] **Step 4: Register the module**

In `src/server/src/dice/mod.rs`, add after `pub mod rng;`:

```rust
pub mod spec;

pub use spec::{
    BinOp, Comparator, DiceGroup, DieId, DieKind, Expr, ExplodeKind, GroupModifier, Mode, RollSpec,
    SuccessRule,
};
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p shadowcat dice::spec`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

```bash
cd src/server && cargo fmt && cargo clippy -p shadowcat --all-targets -- -D warnings && cd ../..
git add src/server/src/dice/spec.rs src/server/src/dice/mod.rs
git commit -m "feat(dice): core RollSpec + expression AST types (M11a)"
```

---

### Task 3: Raw-roll and outcome types

**Files:**
- Create: `src/server/src/dice/outcome.rs`
- Modify: `src/server/src/dice/mod.rs` (add `pub mod outcome;` + re-exports)

**Interfaces:**
- Consumes: `DieId`, `DieKind` (Task 2).
- Produces:
  - `struct RawDie { id: DieId, kind: DieKind, natural: i32 }`
  - `struct RawRoll { dice: Vec<RawDie>, records: Vec<DieRecord>, next_id: DieId }`
  - `struct DieRecord { id: DieId, natural: i32, value: i32, kept: bool, exploded: bool, rerolled_from: Option<i32> }`
  - `struct RollOutcome { total: i64, records: Vec<DieRecord>, successes: Option<i32>, pass: Option<bool>, net_margin: Option<i32> }`
  - `struct RollResult { spec: RollSpec, raws: RawRoll, outcome: RollOutcome }`
  - `impl RawRoll { fn push(&mut self, kind: DieKind, natural: i32) -> DieId }`

- [ ] **Step 1: Write the failing test**

Create `src/server/src/dice/outcome.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::dice::spec::DieKind;

    #[test]
    fn raw_roll_allocates_monotonic_ids() {
        let mut r = RawRoll::default();
        let a = r.push(DieKind::Numeric { min: 1, max: 6 }, 4);
        let b = r.push(DieKind::Numeric { min: 1, max: 6 }, 2);
        assert_eq!(a, 0);
        assert_eq!(b, 1);
        assert_eq!(r.dice.len(), 2);
        assert_eq!(r.dice[0].natural, 4);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p shadowcat dice::outcome`
Expected: FAIL — types not defined.

- [ ] **Step 3: Implement the types**

Prepend to `src/server/src/dice/outcome.rs`:

```rust
use serde::{Deserialize, Serialize};

use crate::dice::spec::{DieId, DieKind, RollSpec};

/// A single die's natural (RNG) result — the only nondeterministic artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawDie {
    pub id: DieId,
    pub kind: DieKind,
    pub natural: i32,
}

/// The RNG output for a whole roll. `dice` is the natural-face log; `records` is the
/// post-pipeline per-die result (filled by `roll`/`recalculate`); `next_id` hands out
/// stable ids so reroll/add ops never collide with existing dice.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawRoll {
    pub dice: Vec<RawDie>,
    pub records: Vec<DieRecord>,
    pub next_id: DieId,
}

impl RawRoll {
    /// Append a natural die with a fresh id; returns that id.
    pub fn push(&mut self, kind: DieKind, natural: i32) -> DieId {
        let id = self.next_id;
        self.next_id += 1;
        self.dice.push(RawDie { id, kind, natural });
        id
    }
}

/// A die's contribution after the pipeline. `value` = post-modifier face; `kept` =
/// survived keep/drop; `rerolled_from` = prior natural if this replaced a reroll.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DieRecord {
    pub id: DieId,
    pub natural: i32,
    pub value: i32,
    pub kept: bool,
    pub exploded: bool,
    pub rerolled_from: Option<i32>,
}

/// Fully-derived result. `total` for Sum; `successes`/`pass`/`net_margin` for
/// SuccessCount (all `None` in the other mode where inapplicable).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollOutcome {
    pub total: i64,
    pub records: Vec<DieRecord>,
    pub successes: Option<i32>,
    pub pass: Option<bool>,
    pub net_margin: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollResult {
    pub spec: RollSpec,
    pub raws: RawRoll,
    pub outcome: RollOutcome,
}
```

- [ ] **Step 4: Register the module**

In `src/server/src/dice/mod.rs`, add after the spec re-exports:

```rust
pub mod outcome;

pub use outcome::{DieRecord, RawDie, RawRoll, RollOutcome, RollResult};
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p shadowcat dice::outcome`
Expected: PASS (1 test).

- [ ] **Step 6: Commit**

```bash
cd src/server && cargo fmt && cargo clippy -p shadowcat --all-targets -- -D warnings && cd ../..
git add src/server/src/dice/outcome.rs src/server/src/dice/mod.rs
git commit -m "feat(dice): raw-roll + outcome record types (M11a)"
```

---

### Task 4: Per-group pipeline — reroll, explode, keep/drop

**Files:**
- Create: `src/server/src/dice/eval/mod.rs`
- Create: `src/server/src/dice/eval/groups.rs`
- Modify: `src/server/src/dice/mod.rs` (add `pub mod eval;`)

**Interfaces:**
- Consumes: `DiceGroup`, `GroupModifier`, `ExplodeKind`, `DieKind`, `Comparator` (Task 2); `DieRecord`, `RawDie`, `RawRoll` (Task 3); `RngSource`, `roll_uniform` (Task 1).
- Produces: `fn resolve_group(group: &DiceGroup, naturals: &[RawDie], rng: &mut dyn RngSource, raws: &mut RawRoll) -> Vec<DieRecord>` — takes a group's already-rolled naturals, applies reroll → explode → keep/drop in modifier order, allocating new dice through `raws`, returning records (`kept` flags set; dropped dice retained for display).

Design notes:
- Start with one `DieRecord` per natural (`kept=true`). Reroll replaces a die's `value` (records `rerolled_from`). Explode appends extra dice via `raws.push` (`exploded=true` on the trigger). Keep/drop sorts by `value` and flips `kept` on excluded dice (never removes them).
- Compound explode adds the extra face into the same record's `value` and re-checks the trigger on the extra face. Penetrate subtracts 1 from each successive extra face. Standard appends a new record.
- Guard runaway chains with a hard cap (`CHAIN_CAP`) to avoid infinite loops on always-true comparators.

- [ ] **Step 1: Create the eval module root**

Create `src/server/src/dice/eval/mod.rs`:

```rust
pub mod groups;
```

- [ ] **Step 2: Write the failing tests**

Create `src/server/src/dice/eval/groups.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::dice::outcome::{RawDie, RawRoll};
    use crate::dice::rng::NoiseRng;
    use crate::dice::spec::{Comparator, DiceGroup, DieKind, ExplodeKind, GroupModifier};

    fn d6(id: u32, natural: i32) -> RawDie {
        RawDie { id, kind: DieKind::Numeric { min: 1, max: 6 }, natural }
    }

    fn group(mods: Vec<GroupModifier>) -> DiceGroup {
        DiceGroup { count: 4, kind: DieKind::Numeric { min: 1, max: 6 }, modifiers: mods }
    }

    #[test]
    fn keep_highest_flags_lowest_as_not_kept() {
        let naturals = vec![d6(0, 2), d6(1, 5), d6(2, 3), d6(3, 6)];
        let mut raws = RawRoll { dice: naturals.clone(), records: vec![], next_id: 4 };
        let mut rng = NoiseRng::from_seed(1);
        let recs = resolve_group(&group(vec![GroupModifier::KeepHighest(3)]), &naturals, &mut rng, &mut raws);
        let kept: Vec<i32> = recs.iter().filter(|r| r.kept).map(|r| r.value).collect();
        let dropped: Vec<i32> = recs.iter().filter(|r| !r.kept).map(|r| r.value).collect();
        assert_eq!(dropped, vec![2]);
        assert_eq!(kept.iter().sum::<i32>(), 14);
    }

    #[test]
    fn reroll_once_replaces_matching_die() {
        let naturals = vec![d6(0, 1), d6(1, 4)];
        let mut raws = RawRoll { dice: naturals.clone(), records: vec![], next_id: 2 };
        let g = DiceGroup { count: 2, kind: DieKind::Numeric { min: 1, max: 6 },
            modifiers: vec![GroupModifier::Reroll { comp: Comparator::Lte, target: 1, once: true }] };
        let mut rng = NoiseRng::from_seed(3);
        let recs = resolve_group(&g, &naturals, &mut rng, &mut raws);
        assert_eq!(recs[0].rerolled_from, Some(1));
        assert!((1..=6).contains(&recs[0].value));
    }

    #[test]
    fn standard_explode_appends_extra_on_max() {
        let naturals = vec![d6(0, 6), d6(1, 2)];
        let mut raws = RawRoll { dice: naturals.clone(), records: vec![], next_id: 2 };
        let g = DiceGroup { count: 2, kind: DieKind::Numeric { min: 1, max: 6 },
            modifiers: vec![GroupModifier::Explode { kind: ExplodeKind::Standard, comp: Comparator::Gte, target: 6 }] };
        let mut rng = NoiseRng::from_seed(11);
        let recs = resolve_group(&g, &naturals, &mut rng, &mut raws);
        assert!(recs.len() >= 3, "expected at least one extra die from the explosion");
        assert!(recs[0].exploded);
    }
}
```

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test -p shadowcat dice::eval::groups`
Expected: FAIL — `resolve_group` not found.

- [ ] **Step 4: Implement the group pipeline**

Prepend to `src/server/src/dice/eval/groups.rs`:

```rust
use crate::dice::outcome::{DieRecord, RawDie, RawRoll};
use crate::dice::rng::{roll_uniform, RngSource};
use crate::dice::spec::{DiceGroup, DieKind, ExplodeKind, GroupModifier};

/// Hard cap on chained explosions/rerolls per die — prevents infinite loops when a
/// comparator is always true (e.g. explode on `>=1`).
const CHAIN_CAP: usize = 100;

/// Resolve one group's dice: apply modifiers in order, returning per-die records.
/// Reroll/explode mutate the die set (new dice allocated via `raws`); keep/drop only
/// flip `kept`. All rolled dice stay in the returned vec (dropped dice for display).
pub fn resolve_group(
    group: &DiceGroup,
    naturals: &[RawDie],
    rng: &mut dyn RngSource,
    raws: &mut RawRoll,
) -> Vec<DieRecord> {
    let DieKind::Numeric { min, max } = group.kind;
    let mut recs: Vec<DieRecord> = naturals
        .iter()
        .map(|d| DieRecord {
            id: d.id,
            natural: d.natural,
            value: d.natural,
            kept: true,
            exploded: false,
            rerolled_from: None,
        })
        .collect();

    for m in &group.modifiers {
        match m {
            GroupModifier::Reroll { comp, target, once } => {
                for r in recs.iter_mut() {
                    let mut chain = 0;
                    while comp.test(r.value, *target) && chain < CHAIN_CAP {
                        r.rerolled_from = Some(r.value);
                        r.value = roll_uniform(rng, min, max);
                        chain += 1;
                        if *once {
                            break;
                        }
                    }
                }
            }
            GroupModifier::Explode { kind, comp, target } => {
                let mut i = 0;
                while i < recs.len() {
                    if comp.test(recs[i].value, *target) {
                        recs[i].exploded = true;
                        let mut chain = 0;
                        loop {
                            if chain >= CHAIN_CAP {
                                break;
                            }
                            let mut extra = roll_uniform(rng, min, max);
                            match kind {
                                ExplodeKind::Compound => {
                                    recs[i].value += extra;
                                }
                                ExplodeKind::Penetrate => {
                                    extra -= 1;
                                    let id = raws.push(group.kind.clone(), extra);
                                    recs.push(DieRecord { id, natural: extra, value: extra, kept: true, exploded: false, rerolled_from: None });
                                }
                                ExplodeKind::Standard => {
                                    let id = raws.push(group.kind.clone(), extra);
                                    recs.push(DieRecord { id, natural: extra, value: extra, kept: true, exploded: false, rerolled_from: None });
                                }
                            }
                            chain += 1;
                            // Re-check the trigger on the fresh face.
                            if !comp.test(extra, *target) {
                                break;
                            }
                        }
                    }
                    i += 1;
                }
            }
            GroupModifier::KeepHighest(n) => keep(&mut recs, *n as usize, true, true),
            GroupModifier::KeepLowest(n) => keep(&mut recs, *n as usize, false, true),
            GroupModifier::DropHighest(n) => keep(&mut recs, *n as usize, true, false),
            GroupModifier::DropLowest(n) => keep(&mut recs, *n as usize, false, false),
        }
    }
    recs
}

/// Flip `kept` on records. `highest` = which end to select; `keep_selected` = true for
/// keep-N (select N to keep), false for drop-N (select N to drop).
fn keep(recs: &mut [DieRecord], n: usize, highest: bool, keep_selected: bool) {
    let mut idx: Vec<usize> = (0..recs.len()).collect();
    idx.sort_by(|&a, &b| {
        if highest {
            recs[b].value.cmp(&recs[a].value)
        } else {
            recs[a].value.cmp(&recs[b].value)
        }
    });
    let selected: std::collections::HashSet<usize> = idx.into_iter().take(n).collect();
    for (i, r) in recs.iter_mut().enumerate() {
        let is_selected = selected.contains(&i);
        r.kept = if keep_selected { is_selected } else { !is_selected };
    }
}
```

- [ ] **Step 5: Register the eval module**

In `src/server/src/dice/mod.rs`, add after the outcome re-exports:

```rust
pub mod eval;
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p shadowcat dice::eval::groups`
Expected: PASS (3 tests).

- [ ] **Step 7: Commit**

```bash
cd src/server && cargo fmt && cargo clippy -p shadowcat --all-targets -- -D warnings && cd ../..
git add src/server/src/dice/eval/ src/server/src/dice/mod.rs
git commit -m "feat(dice): per-group reroll/explode/keep-drop pipeline (M11a)"
```

---

### Task 5: `roll` — natural faces + pipeline from the expression

**Files:**
- Modify: `src/server/src/dice/eval/mod.rs` (add `roll`)
- Modify: `src/server/src/dice/mod.rs` (re-export `roll`)

**Interfaces:**
- Consumes: `Expr`, `DiceGroup`, `DieKind`, `RollSpec` (Task 2); `RawRoll`, `RawDie` (Task 3); `resolve_group` (Task 4); `RngSource`, `roll_uniform` (Task 1).
- Produces: `fn roll(spec: &RollSpec, rng: &mut dyn RngSource) -> RawRoll` — walks the AST left-to-right; for each `Dice` node rolls `count` naturals, then resolves the group pipeline, appending the resulting records into `raws.records`. The ONLY randomness step.

- [ ] **Step 1: Write the failing test**

Add to `src/server/src/dice/eval/mod.rs` (create a `#[cfg(test)] mod tests`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::dice::rng::NoiseRng;
    use crate::dice::spec::{BinOp, DiceGroup, DieKind, Expr, Mode, RollSpec};

    fn ng(count: u32, min: i32, max: i32) -> Expr {
        Expr::Dice(DiceGroup { count, kind: DieKind::Numeric { min, max }, modifiers: vec![] })
    }

    #[test]
    fn roll_produces_one_record_per_die_across_groups() {
        let spec = RollSpec {
            expr: Expr::Bin { op: BinOp::Add, lhs: Box::new(ng(3, 1, 6)), rhs: Box::new(ng(2, 1, 8)) },
            mode: Mode::Sum, success: None, required_successes: None,
        };
        let raws = roll(&spec, &mut NoiseRng::from_seed(99));
        assert_eq!(raws.records.len(), 5);
        assert_eq!(raws.dice.len(), 5); // no explode modifiers -> no extra dice
        for r in &raws.records[0..3] { assert!((1..=6).contains(&r.value)); }
        for r in &raws.records[3..5] { assert!((1..=8).contains(&r.value)); }
    }

    #[test]
    fn roll_is_seed_stable() {
        let spec = RollSpec { expr: ng(4, 1, 20), mode: Mode::Sum, success: None, required_successes: None };
        let a = roll(&spec, &mut NoiseRng::from_seed(5));
        let b = roll(&spec, &mut NoiseRng::from_seed(5));
        assert_eq!(a, b);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p shadowcat dice::eval::tests::roll`
Expected: FAIL — `roll` not found.

- [ ] **Step 3: Implement `roll`**

Prepend to `src/server/src/dice/eval/mod.rs` (above `pub mod groups;`):

```rust
use crate::dice::eval::groups::resolve_group;
use crate::dice::outcome::{RawDie, RawRoll};
use crate::dice::rng::{roll_uniform, RngSource};
use crate::dice::spec::{DieKind, Expr, RollSpec};

/// Roll every die in the spec's expression, left-to-right, running each group's
/// pipeline. The ONLY randomness step; `evaluate` (Task 6/7) reads `raws.records`.
pub fn roll(spec: &RollSpec, rng: &mut dyn RngSource) -> RawRoll {
    let mut raws = RawRoll::default();
    roll_expr(&spec.expr, rng, &mut raws);
    raws
}

fn roll_expr(expr: &Expr, rng: &mut dyn RngSource, raws: &mut RawRoll) {
    match expr {
        Expr::Dice(group) => {
            let DieKind::Numeric { min, max } = group.kind;
            let start = raws.dice.len();
            for _ in 0..group.count {
                let natural = roll_uniform(rng, min, max);
                raws.push(group.kind.clone(), natural);
            }
            let naturals: Vec<RawDie> = raws.dice[start..].to_vec();
            let recs = resolve_group(group, &naturals, rng, raws);
            raws.records.extend(recs);
        }
        Expr::Const(_) => {}
        Expr::Neg(inner) => roll_expr(inner, rng, raws),
        Expr::Bin { lhs, rhs, .. } => {
            roll_expr(lhs, rng, raws);
            roll_expr(rhs, rng, raws);
        }
    }
}
```

- [ ] **Step 4: Re-export `roll`**

In `src/server/src/dice/mod.rs`, change the eval line to:

```rust
pub mod eval;
pub use eval::roll;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p shadowcat dice::eval`
Expected: PASS (roll tests + group tests).

- [ ] **Step 6: Commit**

```bash
cd src/server && cargo fmt && cargo clippy -p shadowcat --all-targets -- -D warnings && cd ../..
git add src/server/src/dice/eval/mod.rs src/server/src/dice/mod.rs
git commit -m "feat(dice): roll() drives the AST through the group pipeline (M11a)"
```

---

### Task 6: `evaluate` — Sum mode

**Files:**
- Create: `src/server/src/dice/eval/sum.rs`
- Modify: `src/server/src/dice/eval/mod.rs` (add `mod sum;` + `evaluate` dispatch)

**Interfaces:**
- Consumes: `Expr`, `BinOp`, `RollSpec`, `Mode`, `DieId` (Task 2); `RawRoll`, `RollOutcome`, `DieRecord` (Task 3).
- Produces:
  - `fn evaluate(spec: &RollSpec, raws: &RawRoll) -> RollOutcome` (Sum branch here; SuccessCount added Task 7).
  - `fn evaluate_sum(spec: &RollSpec, raws: &RawRoll) -> RollOutcome`.

Design: each `DieRecord` gets a `group_index: usize` (which `Dice` node produced it) so Sum-mode folds by group without fragile positional heuristics. Add `group_index` to `DieRecord` (Task 3 struct) and set it in `roll`/`resolve_group`.

- [ ] **Step 1: Add `group_index` to `DieRecord` and thread it through**

In `src/server/src/dice/outcome.rs`, add the field to `DieRecord`:

```rust
pub struct DieRecord {
    pub id: DieId,
    pub group_index: usize,
    pub natural: i32,
    pub value: i32,
    pub kept: bool,
    pub exploded: bool,
    pub rerolled_from: Option<i32>,
}
```

In `src/server/src/dice/eval/groups.rs`, change `resolve_group` to accept a `group_index: usize` param and set it on every constructed `DieRecord` (initial map + explosion children). Update its three tests to pass `0`.

In `src/server/src/dice/eval/mod.rs` `roll_expr`, track a group counter and pass it: add a `&mut usize` counter threaded through `roll_expr`, incremented once per `Dice` node, passed to `resolve_group`. (The counter increments in AST left-to-right order, matching `evaluate_sum`'s walk.)

Concretely, change `roll` / `roll_expr`:

```rust
pub fn roll(spec: &RollSpec, rng: &mut dyn RngSource) -> RawRoll {
    let mut raws = RawRoll::default();
    let mut gi = 0usize;
    roll_expr(&spec.expr, rng, &mut raws, &mut gi);
    raws
}

fn roll_expr(expr: &Expr, rng: &mut dyn RngSource, raws: &mut RawRoll, gi: &mut usize) {
    match expr {
        Expr::Dice(group) => {
            let DieKind::Numeric { min, max } = group.kind;
            let start = raws.dice.len();
            for _ in 0..group.count {
                let natural = roll_uniform(rng, min, max);
                raws.push(group.kind.clone(), natural);
            }
            let naturals: Vec<RawDie> = raws.dice[start..].to_vec();
            let index = *gi;
            *gi += 1;
            let recs = resolve_group(group, index, &naturals, rng, raws);
            raws.records.extend(recs);
        }
        Expr::Const(_) => {}
        Expr::Neg(inner) => roll_expr(inner, rng, raws, gi),
        Expr::Bin { lhs, rhs, .. } => {
            roll_expr(lhs, rng, raws, gi);
            roll_expr(rhs, rng, raws, gi);
        }
    }
}
```

- [ ] **Step 2: Write the failing Sum test**

Create `src/server/src/dice/eval/sum.rs`:

```rust
#[cfg(test)]
mod tests {
    use crate::dice::eval::{evaluate, roll};
    use crate::dice::rng::NoiseRng;
    use crate::dice::spec::{BinOp, DiceGroup, DieKind, Expr, Mode, RollSpec};

    fn ng(count: u32, min: i32, max: i32) -> Expr {
        Expr::Dice(DiceGroup { count, kind: DieKind::Numeric { min, max }, modifiers: vec![] })
    }

    #[test]
    fn sum_two_dice_plus_constant() {
        let spec = RollSpec {
            expr: Expr::Bin { op: BinOp::Add, lhs: Box::new(ng(2, 1, 6)), rhs: Box::new(Expr::Const(3)) },
            mode: Mode::Sum, success: None, required_successes: None,
        };
        let raws = roll(&spec, &mut NoiseRng::from_seed(20));
        let out = evaluate(&spec, &raws);
        let dice_sum: i64 = raws.records.iter().filter(|r| r.kept).map(|r| r.value as i64).sum();
        assert_eq!(out.total, dice_sum + 3);
        assert_eq!(out.successes, None);
    }

    #[test]
    fn sum_multiplication() {
        let spec = RollSpec {
            expr: Expr::Bin { op: BinOp::Mul, lhs: Box::new(ng(1, 1, 4)), rhs: Box::new(Expr::Const(2)) },
            mode: Mode::Sum, success: None, required_successes: None,
        };
        let raws = roll(&spec, &mut NoiseRng::from_seed(21));
        let out = evaluate(&spec, &raws);
        let d: i64 = raws.records.iter().filter(|r| r.kept).map(|r| r.value as i64).sum();
        assert_eq!(out.total, d * 2);
    }

    #[test]
    fn sum_two_groups_fold_independently() {
        // 1d6 + 1d6: each group folds its own kept sum; group_index disambiguates.
        let spec = RollSpec {
            expr: Expr::Bin { op: BinOp::Add, lhs: Box::new(ng(1, 1, 6)), rhs: Box::new(ng(1, 1, 6)) },
            mode: Mode::Sum, success: None, required_successes: None,
        };
        let raws = roll(&spec, &mut NoiseRng::from_seed(22));
        let out = evaluate(&spec, &raws);
        let all: i64 = raws.records.iter().filter(|r| r.kept).map(|r| r.value as i64).sum();
        assert_eq!(out.total, all);
    }
}
```

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test -p shadowcat dice::eval::sum`
Expected: FAIL — `evaluate` not found.

- [ ] **Step 4: Implement Sum-mode `evaluate`**

Add to `src/server/src/dice/eval/mod.rs` (after `roll`):

```rust
pub mod sum;
pub mod success;

use crate::dice::outcome::RollOutcome;
use crate::dice::spec::Mode;

/// Deterministic scoring: (spec, raws) -> outcome. Reads `raws.records`; NO randomness.
pub fn evaluate(spec: &RollSpec, raws: &RawRoll) -> RollOutcome {
    match spec.mode {
        Mode::Sum => sum::evaluate_sum(spec, raws),
        Mode::SuccessCount => success::evaluate_success(spec, raws),
    }
}
```

Prepend to `src/server/src/dice/eval/sum.rs`:

```rust
use crate::dice::outcome::{RawRoll, RollOutcome};
use crate::dice::spec::{BinOp, Expr, RollSpec};

/// Fold the AST to a total. Each `Dice` node contributes the sum of its group's kept
/// records (matched by `group_index`); a cursor consumes groups in AST order.
pub fn evaluate_sum(spec: &RollSpec, raws: &RawRoll) -> RollOutcome {
    let mut next_group = 0usize;
    let total = fold(&spec.expr, raws, &mut next_group);
    RollOutcome {
        total,
        records: raws.records.clone(),
        successes: None,
        pass: None,
        net_margin: None,
    }
}

fn fold(expr: &Expr, raws: &RawRoll, next_group: &mut usize) -> i64 {
    match expr {
        Expr::Const(c) => *c as i64,
        Expr::Neg(inner) => -fold(inner, raws, next_group),
        Expr::Dice(_) => {
            let gi = *next_group;
            *next_group += 1;
            raws.records
                .iter()
                .filter(|r| r.group_index == gi && r.kept)
                .map(|r| r.value as i64)
                .sum()
        }
        Expr::Bin { op, lhs, rhs } => {
            let l = fold(lhs, raws, next_group);
            let r = fold(rhs, raws, next_group);
            match op {
                BinOp::Add => l + r,
                BinOp::Sub => l - r,
                BinOp::Mul => l * r,
                // Division by zero yields 0 (documented; parser rejects literal `/0`).
                BinOp::Div => if r == 0 { 0 } else { l / r },
            }
        }
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p shadowcat dice::eval`
Expected: PASS (sum + roll + group tests; the group tests now pass `group_index` — update them if not already done in Step 1).

- [ ] **Step 6: Commit**

```bash
cd src/server && cargo fmt && cargo clippy -p shadowcat --all-targets -- -D warnings && cd ../..
git add src/server/src/dice/eval/ src/server/src/dice/outcome.rs
git commit -m "feat(dice): Sum-mode evaluate folds by group_index (M11a)"
```

---

### Task 7: `evaluate` — SuccessCount mode

**Files:**
- Create: `src/server/src/dice/eval/success.rs`

**Interfaces:**
- Consumes: `RollSpec`, `SuccessRule` (Task 2); `RawRoll`, `RollOutcome` (Task 3).
- Produces: `fn evaluate_success(spec: &RollSpec, raws: &RawRoll) -> RollOutcome` — pools ALL kept records, counts those satisfying `spec.success` (if `None`, successes = 0); sets `successes`; if `required_successes` is set, sets `pass` (successes ≥ required) and `net_margin` (successes − required). `total` = sum of kept values (reference).

- [ ] **Step 1: Write the failing test**

Create `src/server/src/dice/eval/success.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::dice::eval::roll;
    use crate::dice::rng::NoiseRng;
    use crate::dice::spec::{Comparator, DiceGroup, DieKind, Expr, Mode, RollSpec, SuccessRule};

    fn pool(count: u32) -> RollSpec {
        RollSpec {
            expr: Expr::Dice(DiceGroup { count, kind: DieKind::Numeric { min: 1, max: 10 }, modifiers: vec![] }),
            mode: Mode::SuccessCount,
            success: Some(SuccessRule { comp: Comparator::Gte, target: 7 }),
            required_successes: None,
        }
    }

    #[test]
    fn counts_dice_at_or_above_target() {
        let spec = pool(6);
        let raws = roll(&spec, &mut NoiseRng::from_seed(30));
        let out = evaluate_success(&spec, &raws);
        let expected = raws.records.iter().filter(|r| r.kept && r.value >= 7).count() as i32;
        assert_eq!(out.successes, Some(expected));
        assert_eq!(out.pass, None);
        assert_eq!(out.net_margin, None);
    }

    #[test]
    fn required_successes_sets_pass_and_margin() {
        let mut spec = pool(6);
        spec.required_successes = Some(2);
        let raws = roll(&spec, &mut NoiseRng::from_seed(31));
        let out = evaluate_success(&spec, &raws);
        let s = out.successes.unwrap();
        assert_eq!(out.pass, Some(s >= 2));
        assert_eq!(out.net_margin, Some(s - 2));
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p shadowcat dice::eval::success`
Expected: FAIL — `evaluate_success` not found.

- [ ] **Step 3: Implement SuccessCount `evaluate`**

Prepend to `src/server/src/dice/eval/success.rs`:

```rust
use crate::dice::outcome::{RawRoll, RollOutcome};
use crate::dice::spec::RollSpec;

/// Pool aggregation: count kept dice satisfying the (required) per-die success rule.
/// Optional `required_successes` yields overall pass + net margin (net hits).
pub fn evaluate_success(spec: &RollSpec, raws: &RawRoll) -> RollOutcome {
    let successes: i32 = match &spec.success {
        Some(rule) => raws
            .records
            .iter()
            .filter(|r| r.kept && rule.comp.test(r.value, rule.target))
            .count() as i32,
        None => 0,
    };
    let total: i64 = raws.records.iter().filter(|r| r.kept).map(|r| r.value as i64).sum();
    let (pass, net_margin) = match spec.required_successes {
        Some(req) => (Some(successes >= req), Some(successes - req)),
        None => (None, None),
    };
    RollOutcome { total, records: raws.records.clone(), successes: Some(successes), pass, net_margin }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p shadowcat dice::eval::success`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
cd src/server && cargo fmt && cargo clippy -p shadowcat --all-targets -- -D warnings && cd ../..
git add src/server/src/dice/eval/success.rs
git commit -m "feat(dice): SuccessCount-mode evaluate + required-successes (M11a)"
```

---

### Task 8: Notation lexer

**Files:**
- Create: `src/server/src/dice/notation/mod.rs`
- Create: `src/server/src/dice/notation/lexer.rs`
- Create: `src/server/src/dice/notation/parser.rs` (stub)
- Modify: `src/server/src/dice/mod.rs` (add `pub mod notation;` + `pub use notation::parse;`)

**Interfaces:**
- Produces:
  - `enum Token { Int(i32), D, Ident(String), Plus, Minus, Star, Slash, LParen, RParen, Cmp(Comparator), Bang, BangBang, BangP }`
  - `fn lex(input: &str) -> Result<Vec<Token>, ParseError>`
  - `enum ParseError { Empty, Unexpected(String), Trailing(String) }`

- [ ] **Step 1: Write the failing test**

Create `src/server/src/dice/notation/lexer.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::dice::spec::Comparator;

    #[test]
    fn lex_basic_expression() {
        let toks = lex("4d6kh3+2").unwrap();
        assert_eq!(toks, vec![
            Token::Int(4), Token::D, Token::Int(6),
            Token::Ident("kh".into()), Token::Int(3),
            Token::Plus, Token::Int(2),
        ]);
    }

    #[test]
    fn lex_success_comparator() {
        let toks = lex("5d10cs>=7").unwrap();
        assert_eq!(toks, vec![
            Token::Int(5), Token::D, Token::Int(10),
            Token::Ident("cs".into()), Token::Cmp(Comparator::Gte), Token::Int(7),
        ]);
    }

    #[test]
    fn lex_rejects_garbage() {
        assert!(lex("4d6 @ 2").is_err());
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p shadowcat dice::notation::lexer`
Expected: FAIL — `lex`/`Token` not found.

- [ ] **Step 3: Implement the lexer + shared ParseError + parser stub**

Create `src/server/src/dice/notation/mod.rs`:

```rust
pub mod lexer;
pub mod parser;

pub use parser::parse;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    Empty,
    Unexpected(String),
    Trailing(String),
}
```

Create `src/server/src/dice/notation/parser.rs` (stub, implemented in Task 9):

```rust
use crate::dice::notation::ParseError;
use crate::dice::spec::RollSpec;

pub fn parse(_input: &str) -> Result<RollSpec, ParseError> {
    Err(ParseError::Empty)
}
```

Prepend to `src/server/src/dice/notation/lexer.rs`:

```rust
use crate::dice::notation::ParseError;
use crate::dice::spec::Comparator;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    Int(i32),
    D,
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    LParen,
    RParen,
    Cmp(Comparator),
    Bang,
    BangBang,
    BangP,
}

pub fn lex(input: &str) -> Result<Vec<Token>, ParseError> {
    let mut out = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            ' ' | '\t' => i += 1,
            '0'..='9' => {
                let start = i;
                while i < bytes.len() && (bytes[i] as char).is_ascii_digit() {
                    i += 1;
                }
                let n: i32 = input[start..i]
                    .parse()
                    .map_err(|_| ParseError::Unexpected(input[start..i].to_string()))?;
                out.push(Token::Int(n));
            }
            'd' if !(i + 1 < bytes.len() && (bytes[i + 1] as char).is_ascii_alphabetic()) => {
                out.push(Token::D);
                i += 1;
            }
            'a'..='z' | 'A'..='Z' => {
                let start = i;
                while i < bytes.len() && (bytes[i] as char).is_ascii_alphabetic() {
                    i += 1;
                }
                out.push(Token::Ident(input[start..i].to_lowercase()));
            }
            '+' => { out.push(Token::Plus); i += 1; }
            '-' => { out.push(Token::Minus); i += 1; }
            '*' => { out.push(Token::Star); i += 1; }
            '/' => { out.push(Token::Slash); i += 1; }
            '(' => { out.push(Token::LParen); i += 1; }
            ')' => { out.push(Token::RParen); i += 1; }
            '!' => {
                if input[i..].starts_with("!!") { out.push(Token::BangBang); i += 2; }
                else if input[i..].starts_with("!p") { out.push(Token::BangP); i += 2; }
                else { out.push(Token::Bang); i += 1; }
            }
            '>' | '<' | '=' => {
                let (cmp, adv) = if input[i..].starts_with(">=") { (Comparator::Gte, 2) }
                    else if input[i..].starts_with("<=") { (Comparator::Lte, 2) }
                    else if c == '>' { (Comparator::Gt, 1) }
                    else if c == '<' { (Comparator::Lt, 1) }
                    else { (Comparator::Eq, 1) };
                out.push(Token::Cmp(cmp));
                i += adv;
            }
            _ => return Err(ParseError::Unexpected(c.to_string())),
        }
    }
    Ok(out)
}
```

> Note: the `'d'` arm emits the dice operator only when not immediately followed by another letter, so identifiers `kh`/`kl`/`dh`/`dl` lex whole.

- [ ] **Step 4: Register + run tests**

In `src/server/src/dice/mod.rs`, add:

```rust
pub mod notation;
pub use notation::parse;
```

Run: `cargo test -p shadowcat dice::notation::lexer`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
cd src/server && cargo fmt && cargo clippy -p shadowcat --all-targets -- -D warnings && cd ../..
git add src/server/src/dice/notation/ src/server/src/dice/mod.rs
git commit -m "feat(dice): notation lexer + shared ParseError (M11a)"
```

---

### Task 9: Notation parser → RollSpec

**Files:**
- Modify: `src/server/src/dice/notation/parser.rs` (replace the stub)

**Interfaces:**
- Consumes: `Token`, `lex` (Task 8); all `spec` types (Task 2); `ParseError` (Task 8).
- Produces: `fn parse(input: &str) -> Result<RollSpec, ParseError>` — recursive descent. Grammar: `expr := term (('+'|'-') term)*`; `term := factor (('*'|'/') factor)*`; `factor := '(' expr ')' | '-' factor | dice | int`; a dice factor is `int 'd' int modifier*`; modifiers `kh|kl|dh|dl int`, `! | !! | !p [cmp int]`, `r|ro [cmp int]`, `cs cmp int` / `cf cmp int` (set `Mode::SuccessCount` + `success`). Explode with omitted target defaults to "equals die max," resolved at parse time.

- [ ] **Step 1: Write the failing tests**

Append a test module to `src/server/src/dice/notation/parser.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::parse;
    use crate::dice::spec::*;

    fn dice(count: u32, min: i32, max: i32, mods: Vec<GroupModifier>) -> Expr {
        Expr::Dice(DiceGroup { count, kind: DieKind::Numeric { min, max }, modifiers: mods })
    }

    #[test]
    fn parses_keep_highest_plus_const() {
        let spec = parse("4d6kh3+2").unwrap();
        assert_eq!(spec.mode, Mode::Sum);
        assert_eq!(spec.expr, Expr::Bin {
            op: BinOp::Add,
            lhs: Box::new(dice(4, 1, 6, vec![GroupModifier::KeepHighest(3)])),
            rhs: Box::new(Expr::Const(2)),
        });
    }

    #[test]
    fn parses_success_pool() {
        let spec = parse("5d10cs>=7").unwrap();
        assert_eq!(spec.mode, Mode::SuccessCount);
        assert_eq!(spec.success, Some(SuccessRule { comp: Comparator::Gte, target: 7 }));
        assert_eq!(spec.expr, dice(5, 1, 10, vec![]));
    }

    #[test]
    fn parses_explode_default_target_is_die_max() {
        let spec = parse("6d6!").unwrap();
        match spec.expr {
            Expr::Dice(g) => assert_eq!(
                g.modifiers[0],
                GroupModifier::Explode { kind: ExplodeKind::Standard, comp: Comparator::Gte, target: 6 }
            ),
            _ => panic!("expected dice"),
        }
    }

    #[test]
    fn parses_reroll() {
        let spec = parse("6d6r<2").unwrap();
        match spec.expr {
            Expr::Dice(g) => assert!(matches!(
                g.modifiers[0],
                GroupModifier::Reroll { once: false, comp: Comparator::Lt, target: 2 }
            )),
            _ => panic!("expected dice"),
        }
    }

    #[test]
    fn parses_parentheses_and_mul() {
        assert_eq!(parse("(1d4+1)*2").unwrap().mode, Mode::Sum);
    }

    #[test]
    fn rejects_empty_and_trailing() {
        assert!(parse("").is_err());
        assert!(parse("2d6 2d6").is_err());
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p shadowcat dice::notation::parser`
Expected: FAIL — stub returns `Empty`.

- [ ] **Step 3: Implement the recursive-descent parser**

Replace the non-test body of `src/server/src/dice/notation/parser.rs`:

```rust
use crate::dice::notation::lexer::{lex, Token};
use crate::dice::notation::ParseError;
use crate::dice::spec::{
    BinOp, Comparator, DiceGroup, DieKind, Expr, ExplodeKind, GroupModifier, Mode, RollSpec,
    SuccessRule,
};

struct P {
    toks: Vec<Token>,
    pos: usize,
    success: Option<SuccessRule>,
}

pub fn parse(input: &str) -> Result<RollSpec, ParseError> {
    let toks = lex(input)?;
    if toks.is_empty() {
        return Err(ParseError::Empty);
    }
    let mut p = P { toks, pos: 0, success: None };
    let expr = p.expr()?;
    if p.pos != p.toks.len() {
        return Err(ParseError::Trailing(format!("{:?}", p.toks[p.pos])));
    }
    let mode = if p.success.is_some() { Mode::SuccessCount } else { Mode::Sum };
    Ok(RollSpec { expr, mode, success: p.success, required_successes: None })
}

impl P {
    fn peek(&self) -> Option<&Token> { self.toks.get(self.pos) }

    fn bump(&mut self) -> Option<Token> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() { self.pos += 1; }
        t
    }

    fn expect_int(&mut self) -> Result<i32, ParseError> {
        match self.bump() {
            Some(Token::Int(n)) => Ok(n),
            other => Err(ParseError::Unexpected(format!("{other:?}, expected int"))),
        }
    }

    fn expr(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.term()?;
        while let Some(op) = match self.peek() {
            Some(Token::Plus) => Some(BinOp::Add),
            Some(Token::Minus) => Some(BinOp::Sub),
            _ => None,
        } {
            self.bump();
            let rhs = self.term()?;
            lhs = Expr::Bin { op, lhs: Box::new(lhs), rhs: Box::new(rhs) };
        }
        Ok(lhs)
    }

    fn term(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.factor()?;
        while let Some(op) = match self.peek() {
            Some(Token::Star) => Some(BinOp::Mul),
            Some(Token::Slash) => Some(BinOp::Div),
            _ => None,
        } {
            self.bump();
            let rhs = self.factor()?;
            lhs = Expr::Bin { op, lhs: Box::new(lhs), rhs: Box::new(rhs) };
        }
        Ok(lhs)
    }

    fn factor(&mut self) -> Result<Expr, ParseError> {
        match self.peek() {
            Some(Token::Minus) => { self.bump(); Ok(Expr::Neg(Box::new(self.factor()?))) }
            Some(Token::LParen) => {
                self.bump();
                let e = self.expr()?;
                match self.bump() {
                    Some(Token::RParen) => Ok(e),
                    other => Err(ParseError::Unexpected(format!("{other:?}, expected )"))),
                }
            }
            Some(Token::Int(_)) => {
                let n = self.expect_int()?;
                if matches!(self.peek(), Some(Token::D)) {
                    self.bump();
                    let sides = self.expect_int()?;
                    let modifiers = self.modifiers(sides)?;
                    Ok(Expr::Dice(DiceGroup {
                        count: n as u32,
                        kind: DieKind::Numeric { min: 1, max: sides },
                        modifiers,
                    }))
                } else {
                    Ok(Expr::Const(n))
                }
            }
            other => Err(ParseError::Unexpected(format!("{other:?}"))),
        }
    }

    fn modifiers(&mut self, sides: i32) -> Result<Vec<GroupModifier>, ParseError> {
        let mut mods = Vec::new();
        loop {
            match self.peek() {
                Some(Token::Bang) => { self.bump(); mods.push(self.explode(ExplodeKind::Standard, sides)?); }
                Some(Token::BangBang) => { self.bump(); mods.push(self.explode(ExplodeKind::Compound, sides)?); }
                Some(Token::BangP) => { self.bump(); mods.push(self.explode(ExplodeKind::Penetrate, sides)?); }
                Some(Token::Ident(id)) => {
                    let id = id.clone();
                    self.bump();
                    match id.as_str() {
                        "kh" => mods.push(GroupModifier::KeepHighest(self.expect_int()? as u32)),
                        "kl" => mods.push(GroupModifier::KeepLowest(self.expect_int()? as u32)),
                        "dh" => mods.push(GroupModifier::DropHighest(self.expect_int()? as u32)),
                        "dl" => mods.push(GroupModifier::DropLowest(self.expect_int()? as u32)),
                        "r" | "ro" => {
                            let (comp, target) = self.cmp_target_required()?;
                            mods.push(GroupModifier::Reroll { comp, target, once: id == "ro" });
                        }
                        "cs" => {
                            let (comp, target) = self.cmp_target_required()?;
                            self.success = Some(SuccessRule { comp, target });
                        }
                        "cf" => {
                            // Failure-counting parsed as success on the inverted comparator
                            // (single count path in M11a; dedicated fail-count is M11b).
                            let (comp, target) = self.cmp_target_required()?;
                            self.success = Some(SuccessRule { comp: invert(comp), target });
                        }
                        other => return Err(ParseError::Unexpected(format!("modifier {other}"))),
                    }
                }
                _ => break,
            }
        }
        Ok(mods)
    }

    /// Explode: optional `[cmp] int`; when omitted, default to `Gte` the die max.
    fn explode(&mut self, kind: ExplodeKind, sides: i32) -> Result<GroupModifier, ParseError> {
        let (comp, target) = match self.peek() {
            Some(Token::Cmp(c)) => { let c = *c; self.bump(); (c, self.expect_int()?) }
            Some(Token::Int(_)) => (Comparator::Gte, self.expect_int()?),
            _ => (Comparator::Gte, sides),
        };
        Ok(GroupModifier::Explode { kind, comp, target })
    }

    /// Require `cmp int` or a bare `int` (defaults comparator to `Gte`).
    fn cmp_target_required(&mut self) -> Result<(Comparator, i32), ParseError> {
        match self.bump() {
            Some(Token::Cmp(c)) => Ok((c, self.expect_int()?)),
            Some(Token::Int(n)) => Ok((Comparator::Gte, n)),
            other => Err(ParseError::Unexpected(format!("{other:?}, expected comparator/int"))),
        }
    }
}

fn invert(c: Comparator) -> Comparator {
    match c {
        Comparator::Gte => Comparator::Lt,
        Comparator::Gt => Comparator::Lte,
        Comparator::Lte => Comparator::Gt,
        Comparator::Lt => Comparator::Gte,
        Comparator::Eq => Comparator::Ne,
        Comparator::Ne => Comparator::Eq,
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p shadowcat dice::notation`
Expected: PASS (all lexer + parser tests).

- [ ] **Step 5: Commit**

```bash
cd src/server && cargo fmt && cargo clippy -p shadowcat --all-targets -- -D warnings && cd ../..
git add src/server/src/dice/notation/parser.rs
git commit -m "feat(dice): recursive-descent notation parser -> RollSpec (M11a)"
```

---

### Task 10: `recalculate` — targeted reroll/replace/remove

**Files:**
- Create: `src/server/src/dice/recalc.rs`
- Modify: `src/server/src/dice/mod.rs` (add `pub mod recalc;` + re-exports)

**Interfaces:**
- Consumes: `RollSpec`, `Expr`, `DieId`, `DieKind` (Task 2); `RawRoll`, `RawDie`, `RollOutcome` (Task 3); `roll_uniform`, `RngSource` (Task 1); `resolve_group` (Task 4); `evaluate` (Task 6).
- Produces:
  - `enum RecalcOp { RerollDice(Vec<DieId>), ReplaceDie { id: DieId, natural: i32 }, RemoveDice(Vec<DieId>) }`
  - `fn recalculate(spec: &RollSpec, raws: &RawRoll, ops: &[RecalcOp], rng: &mut dyn RngSource) -> (RawRoll, RollOutcome)`

Design: recalc operates on the **base natural dice** per group (the first `count` naturals of each group in `raws.dice`, tracked via `group_spans`). It applies ops to those naturals, then re-derives records by re-running each group's pipeline over the mutated naturals, then calls `evaluate`. Empty ops → identical `raws` and outcome (identity). To make base-die reconstruction exact under explosions, `roll` records a `group_spans: Vec<(usize, usize)>` on `RawRoll` (start, base_count into `raws.dice`).

- [ ] **Step 1: Record base group spans in `roll`**

In `src/server/src/dice/outcome.rs`, add to `RawRoll`:

```rust
pub group_spans: Vec<(usize, usize)>, // (start index into `dice`, base die count) per Dice node
```

In `src/server/src/dice/eval/mod.rs` `roll_expr`, push a span for each `Dice` node right after rolling its base naturals:

```rust
// inside the Expr::Dice arm, after the for-loop that pushes `count` naturals:
raws.group_spans.push((start, group.count as usize));
```

(Existing tests that build `RawRoll { ... }` literally must add `group_spans: vec![]`.)

- [ ] **Step 2: Write the failing tests**

Create `src/server/src/dice/recalc.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::dice::eval::{evaluate, roll};
    use crate::dice::rng::NoiseRng;
    use crate::dice::spec::{Comparator, DiceGroup, DieKind, Expr, Mode, RollSpec, SuccessRule};

    fn pool(count: u32) -> RollSpec {
        RollSpec {
            expr: Expr::Dice(DiceGroup { count, kind: DieKind::Numeric { min: 1, max: 10 }, modifiers: vec![] }),
            mode: Mode::SuccessCount,
            success: Some(SuccessRule { comp: Comparator::Gte, target: 7 }),
            required_successes: None,
        }
    }

    #[test]
    fn empty_ops_is_identity() {
        let spec = pool(5);
        let mut rng = NoiseRng::from_seed(50);
        let raws = roll(&spec, &mut rng);
        let base = evaluate(&spec, &raws);
        let (raws2, out2) = recalculate(&spec, &raws, &[], &mut rng);
        assert_eq!(raws2, raws);
        assert_eq!(out2, base);
    }

    #[test]
    fn reroll_changes_only_targeted_die() {
        let spec = pool(5);
        let raws = roll(&spec, &mut NoiseRng::from_seed(51));
        let target = raws.dice[0].id;
        let others: Vec<i32> = raws.dice[1..5].iter().map(|d| d.natural).collect();
        let mut rng = NoiseRng::from_seed(999);
        let (raws2, _out) = recalculate(&spec, &raws, &[RecalcOp::RerollDice(vec![target])], &mut rng);
        let others2: Vec<i32> = raws2.dice[1..5].iter().map(|d| d.natural).collect();
        assert_eq!(others, others2, "non-targeted dice must not change");
    }

    #[test]
    fn replace_then_replace_back_round_trips() {
        let spec = pool(3);
        let mut rng = NoiseRng::from_seed(60);
        let raws = roll(&spec, &mut rng);
        let id = raws.dice[0].id;
        let orig = raws.dice[0].natural;
        let (r1, _) = recalculate(&spec, &raws, &[RecalcOp::ReplaceDie { id, natural: 10 }], &mut rng);
        let (r2, out2) = recalculate(&spec, &r1, &[RecalcOp::ReplaceDie { id, natural: orig }], &mut rng);
        assert_eq!(out2, evaluate(&spec, &raws));
        assert_eq!(r2.dice[0].natural, orig);
    }
}
```

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test -p shadowcat dice::recalc`
Expected: FAIL — `recalculate`/`RecalcOp` not found.

- [ ] **Step 4: Implement `recalculate`**

Prepend to `src/server/src/dice/recalc.rs`:

```rust
use crate::dice::eval::evaluate;
use crate::dice::eval::groups::resolve_group;
use crate::dice::outcome::{RawDie, RawRoll, RollOutcome};
use crate::dice::rng::{roll_uniform, RngSource};
use crate::dice::spec::{DieId, DieKind, Expr, RollSpec};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecalcOp {
    RerollDice(Vec<DieId>),
    ReplaceDie { id: DieId, natural: i32 },
    RemoveDice(Vec<DieId>),
}

/// Apply targeted ops to each group's BASE natural dice (via `group_spans`), re-derive
/// records through the pipeline, and re-evaluate. Empty ops is an identity. Reroll
/// draws fresh naturals from the server RNG (server-authoritative).
pub fn recalculate(
    spec: &RollSpec,
    raws: &RawRoll,
    ops: &[RecalcOp],
    rng: &mut dyn RngSource,
) -> (RawRoll, RollOutcome) {
    // Rebuild per-group base naturals from spans (excludes prior explosion children).
    let mut groups: Vec<Vec<RawDie>> = raws
        .group_spans
        .iter()
        .map(|&(start, count)| raws.dice[start..start + count].to_vec())
        .collect();

    // Apply ops against the base dice.
    for op in ops {
        match op {
            RecalcOp::RerollDice(ids) => {
                for g in groups.iter_mut() {
                    for d in g.iter_mut() {
                        if ids.contains(&d.id) {
                            let DieKind::Numeric { min, max } = d.kind;
                            d.natural = roll_uniform(rng, min, max);
                        }
                    }
                }
            }
            RecalcOp::ReplaceDie { id, natural } => {
                for g in groups.iter_mut() {
                    if let Some(d) = g.iter_mut().find(|d| d.id == *id) {
                        d.natural = *natural;
                    }
                }
            }
            RecalcOp::RemoveDice(ids) => {
                for g in groups.iter_mut() {
                    g.retain(|d| !ids.contains(&d.id));
                }
            }
        }
    }

    // Re-derive: walk the AST, consuming groups in order, re-running each pipeline.
    let mut out = RawRoll { next_id: raws.next_id, ..Default::default() };
    let mut gi = 0usize;
    rederive(&spec.expr, &groups, &mut gi, rng, &mut out);
    let outcome = evaluate(spec, &out);
    (out, outcome)
}

fn rederive(
    expr: &Expr,
    groups: &[Vec<RawDie>],
    gi: &mut usize,
    rng: &mut dyn RngSource,
    out: &mut RawRoll,
) {
    match expr {
        Expr::Dice(group) => {
            let index = *gi;
            *gi += 1;
            let naturals = &groups[index];
            let start = out.dice.len();
            for d in naturals {
                out.dice.push(d.clone());
                out.next_id = out.next_id.max(d.id + 1);
            }
            out.group_spans.push((start, naturals.len()));
            let recs = resolve_group(group, index, naturals, rng, out);
            out.records.extend(recs);
        }
        Expr::Const(_) => {}
        Expr::Neg(inner) => rederive(inner, groups, gi, rng, out),
        Expr::Bin { lhs, rhs, .. } => {
            rederive(lhs, groups, gi, rng, out);
            rederive(rhs, groups, gi, rng, out);
        }
    }
}
```

> **Note (identity):** with empty ops and no reroll/explode modifiers, `rederive` reproduces `raws` exactly (same naturals, same ids, same records), so `empty_ops_is_identity` holds. Under explode modifiers the reroll of explosion children is re-derived from the same base naturals; since the noise RNG is advanced by the caller, identity holds only when `rng` is not consumed — the test uses no-modifier pools, matching this guarantee. Document this: recalc with explode modifiers re-rolls the explosion tail (expected).

- [ ] **Step 5: Register + run tests**

In `src/server/src/dice/mod.rs`:

```rust
pub mod recalc;
pub use recalc::{recalculate, RecalcOp};
```

Run: `cargo test -p shadowcat dice::recalc`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
cd src/server && cargo fmt && cargo clippy -p shadowcat --all-targets -- -D warnings && cd ../..
git add src/server/src/dice/recalc.rs src/server/src/dice/mod.rs src/server/src/dice/outcome.rs src/server/src/dice/eval/mod.rs
git commit -m "feat(dice): recalculate() with targeted die ops via group spans (M11a)"
```

---

### Task 11: Property tests + deferred-work log

**Files:**
- Create: `src/server/src/dice/proptests.rs`
- Modify: `src/server/src/dice/mod.rs` (add `#[cfg(test)] mod proptests;`)
- Modify: `src/server/Cargo.toml` (`proptest` dev-dependency)
- Modify: `docs/TODO.md`

**Interfaces:**
- Consumes: `roll`, `evaluate`, `recalculate`, `NoiseRng`, spec types.

- [ ] **Step 1: Add the proptest dev-dependency**

In `src/server/Cargo.toml` under `[dev-dependencies]`, add:

```toml
proptest = "1"
```

- [ ] **Step 2: Write the property tests**

Create `src/server/src/dice/proptests.rs`:

```rust
use proptest::prelude::*;

use crate::dice::eval::{evaluate, roll};
use crate::dice::recalc::recalculate;
use crate::dice::rng::NoiseRng;
use crate::dice::spec::{Comparator, DiceGroup, DieKind, Expr, Mode, RollSpec, SuccessRule};

fn simple_pool(count: u32, sides: i32, target: i32) -> RollSpec {
    RollSpec {
        expr: Expr::Dice(DiceGroup { count, kind: DieKind::Numeric { min: 1, max: sides }, modifiers: vec![] }),
        mode: Mode::SuccessCount,
        success: Some(SuccessRule { comp: Comparator::Gte, target }),
        required_successes: None,
    }
}

proptest! {
    #[test]
    fn evaluate_is_deterministic(seed in any::<u64>(), count in 1u32..12, sides in 2i32..20) {
        let spec = simple_pool(count, sides, sides / 2);
        let raws = roll(&spec, &mut NoiseRng::from_seed(seed));
        prop_assert_eq!(evaluate(&spec, &raws), evaluate(&spec, &raws));
    }

    #[test]
    fn successes_never_exceed_dice(seed in any::<u64>(), count in 1u32..20, sides in 2i32..12) {
        let spec = simple_pool(count, sides, 2);
        let raws = roll(&spec, &mut NoiseRng::from_seed(seed));
        let out = evaluate(&spec, &raws);
        let kept = raws.records.iter().filter(|r| r.kept).count() as i32;
        prop_assert!(out.successes.unwrap() <= kept);
    }

    #[test]
    fn empty_recalc_is_identity(seed in any::<u64>(), count in 1u32..10, sides in 2i32..12) {
        let spec = simple_pool(count, sides, sides / 2);
        let mut rng = NoiseRng::from_seed(seed);
        let raws = roll(&spec, &mut rng);
        let base = evaluate(&spec, &raws);
        let (raws2, out2) = recalculate(&spec, &raws, &[], &mut rng);
        prop_assert_eq!(raws2, raws);
        prop_assert_eq!(out2, base);
    }
}
```

- [ ] **Step 3: Register + run**

In `src/server/src/dice/mod.rs` add:

```rust
#[cfg(test)]
mod proptests;
```

Run: `cargo test -p shadowcat dice::proptests`
Expected: PASS (3 property tests).

- [ ] **Step 4: Log deferred notation breadth**

Append to `docs/TODO.md`:

```markdown
- Dice notation: extended math functions (floor/ceil/round/abs/min/max) are not yet
  parsed. M11a covers dice + arithmetic +-*/() + keep/drop/explode/reroll + cs/cf. Add
  as the notation grammar grows with system demand.
```

- [ ] **Step 5: Full suite + lint + commit**

Run: `cargo test -p shadowcat dice`
Expected: PASS (all dice tests).

```bash
cd src/server && cargo fmt && cargo clippy -p shadowcat --all-targets -- -D warnings && cd ../..
git add src/server/Cargo.toml src/server/Cargo.lock src/server/src/dice/proptests.rs src/server/src/dice/mod.rs docs/TODO.md
git commit -m "test(dice): property tests + deferred-notation log (M11a)"
```

---

## Self-Review

**Spec coverage (against `2026-07-03-m11-dice-engine-design.md`):**
- §2.1 three types → Tasks 2, 3. §2.2 functions (`roll`/`evaluate`/`recalculate`) → Tasks 5, 6/7, 10. §2.3 die model (Numeric; `Faces` = M11b) → Task 2. §2.4 Sum + SuccessCount (Tiered = M11b) → Tasks 6, 7. §2.5 `difficulty` dim-1 (per-die target) + dim-2 (`required_successes`) → Task 7 (Sum-mode target compare + `direction` = M11b). §2.6 pipeline order → Tasks 4–7. §3 recalculation → Task 10. §5 notation baseline → Tasks 8, 9. §7 unit + property tests → all tasks + Task 11.
- **Explicitly deferred to M11b** (plan header + spec §8): expertise DP, crit events + counters, Tiered mode, labeled dice, custom-face dice, `direction` global flip, custom notation. **Not a descope** — M11b is the next plan.

**Placeholder scan:** none. The implementer notes give a concrete primary approach + named fallback with a required test, not deferred work.

**Type consistency:** `RawRoll` gains `records` (Task 3), `group_spans` (Task 10); `DieRecord` gains `group_index` (Task 6). Each addition names every literal-construction site to update. `roll`/`evaluate`/`recalculate`/`parse`/`NoiseRng`/`roll_uniform`/`resolve_group` signatures are stable once introduced (note: `resolve_group` gains a `group_index` param in Task 6 — flagged there).

## Model/Effort directives

- **Plan authored:** mainline, Opus/high (this session) — chosen at the writing-plans tier-switch checkpoint.
- **Execution tiers (per `~/.claude/docs/sdd-model-effort-tiers.md`):** `sdd-implementer` (Sonnet/medium) default; escalate a blocked task Sonnet-high → Opus-high, never skipping a rung. Tasks 9 (parser) and 10 (recalculate) are most intricate — dispatch-time judgment may open them at `sdd-implementer-highthink`. Per-task review `sdd-reviewer` (Sonnet/high), escalate to `sdd-reviewer-opus` for Tasks 4, 6, 10 (pipeline/boundary correctness). Final whole-branch review `sdd-final-reviewer` (Opus/high), once.

## Buddy-check directives

Per the writing-plans buddy-check checkpoint, these carry high-risk signals (subtle correctness,
boundary/float-free integer edges, security-adjacent RNG) and are **recommended for buddy-check**
at their review checkpoint:
- **Task 4 (group pipeline)** — explode/reroll chaining, compound/penetrate semantics, keep/drop selection, chain-cap edges.
- **Task 6 (`group_index` fold)** — the group-boundary reconstruction is the correctness core; a wrong boundary silently mis-sums multi-group rolls.
- **Task 10 (recalculate + `group_spans`)** — rerolling a subset without disturbing siblings, especially under explosions; the highest-consequence correctness path in M11a.
- **For M11b:** the expertise optimizer DP + its brute-force differential oracle is the single highest-risk piece of the whole engine (spec §4.1) and MUST be buddy-checked — recorded here so the M11b plan carries it forward.

## Codebase-skill gate

On completion, create the new **`shadowcat-codebase-dice`** skill (fixed shape; add its globs to
`.claude/hooks/codebase-skill-reminder.py`) and dispatch `shadowcat-spec-reviewer` on the skill diff
per the reviewed skill-update gate, before merge.
