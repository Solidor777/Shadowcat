//! This module scores a roll in `Mode::Total` by folding its expression to a total.
//!
//! `evaluate_total` is the entry point. Addition, subtraction and multiplication in the fold
//! saturate at the `i64` bounds, and an overflowing per-group sum saturates too (and fails a
//! `debug_assert!`).

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::dice::eval::classify;
use crate::dice::outcome::{RawRoll, RollOutcome};
use crate::dice::spec::{BinOp, ConstTerm, Direction, Expr, FnName, RollSpec, TotalConfig};

/// Fold the AST to a total. Each `Dice` node contributes the sum of its group's kept
/// records (matched by `group_index`); a cursor consumes groups in AST order. If
/// `cfg.difficulty` is set, classifies `total` (oriented by `spec.direction`) against
/// `cfg.tiers` into `margin`/`pass`/`tier_label`/`tier_value`; otherwise reports a bare total.
///
/// # Examples
///
/// ```
/// use shadowcat::dice::eval::roll;
/// use shadowcat::dice::eval::sum::evaluate_total;
/// use shadowcat::dice::notation::{parse, ParseContext};
/// use shadowcat::dice::rng::NoiseRng;
/// use shadowcat::dice::spec::Mode;
///
/// let spec = parse("2d6+3", ParseContext::default()).unwrap();
/// let mut rng = NoiseRng::from_seed(1);
/// let raws = roll(&spec, &mut rng);
/// let Mode::Total(cfg) = &spec.mode else { unreachable!() };
/// let outcome = evaluate_total(&spec, cfg, &raws);
/// // 2d6 kept-die sum plus the +3 constant; each die is in [1,6].
/// assert!((5..=15).contains(&outcome.total));
/// assert_eq!(outcome.pass, None); // no difficulty set
/// ```
pub fn evaluate_total(spec: &RollSpec, cfg: &TotalConfig, raws: &RawRoll) -> RollOutcome {
    let mut next_group = 0usize;
    let mut state = FoldState {
        direction: spec.direction,
        labeled_consts: Vec::new(),
        losers: Vec::new(),
    };
    let total = fold(&spec.expr, raws, &mut next_group, 1, &mut state);
    let (pass, margin, tier_label, tier_value) = match cfg.difficulty {
        None => (None, None, None, None),
        Some(diff) => {
            let m = classify::oriented_margin(spec.direction, total, diff as i64);
            let c = classify::classify(m, &cfg.tiers);
            (c.pass, Some(m), c.tier_label, c.tier_value)
        }
    };
    // Losing `best(...)` branches are marked on this clone, never on `raws`, so `evaluate`
    // stays pure over the stored roll. `losers` lists inner calls before the calls enclosing
    // them, so the outermost losing branch's index is the one left on a die.
    let mut records = raws.records.clone();
    for (range, branch) in &state.losers {
        for rec in records
            .iter_mut()
            .filter(|r| range.contains(&r.group_index))
        {
            rec.kept = false;
            rec.best_of_branch = Some(*branch);
        }
    }
    RollOutcome {
        total,
        records,
        successes: None,
        pass,
        margin,
        tier_label,
        tier_value,
        crit_successes: 0,
        crit_fails: 0,
        positive_counter: 0,
        negative_counter: 0,
        symbol_counts: Default::default(),
        labeled_consts: state.labeled_consts,
    }
}

/// Mutable state `fold` threads through the AST besides the group cursor.
struct FoldState {
    /// `RollSpec::direction`, which decides which `best(...)` branch wins.
    direction: Direction,
    /// Every labeled `Const` of the WINNING path, in AST left-to-right order, carrying its
    /// effective additive sign (see `fold`'s `sign`). Becomes `RollOutcome::labeled_consts`.
    labeled_consts: Vec<ConstTerm>,
    /// `(group-index range, branch index)` of each losing `best(...)` branch.
    losers: Vec<(std::ops::Range<usize>, u32)>,
}

/// Accumulates via `checked_add`, saturating at `i64::MAX`/`MIN` instead of
/// panicking on overflow. Unreachable while the chat boundary's roll-record
/// cap bounds a group's kept-die count well below what it takes to overflow
/// `i64`; guard is defense-in-depth against a pathological dice-group count.
fn checked_add_saturating(acc: i64, next: i64) -> i64 {
    match acc.checked_add(next) {
        Some(v) => v,
        None => {
            debug_assert!(false, "dice group total overflowed i64");
            if next >= 0 {
                i64::MAX
            } else {
                i64::MIN
            }
        }
    }
}

/// Saturating `Add`/`Sub`/`Mul` for `Expr::Bin` folds. Unlike
/// `checked_add_saturating` above (whose overflow is unreachable given the
/// chat boundary's per-group record cap), a pure-`Const` arithmetic chain
/// (`2000000000*2000000000*3`) has NO dice groups at all -- `walk_groups`
/// counts zero, so `chat::rolls::MAX_ROLL_DICE`/`chat::rolls::MAX_ROLL_RECORDS` caps
/// never see it -- and a chain of `Mul`-combined dice groups (`1d10000 *
/// 1d10000 * ...`) can overflow `i64` even within those caps. Overflow here
/// is genuinely reachable, so these saturate silently (no `debug_assert!`).
/// `l + r`, saturating at the i64 rails instead of overflowing.
fn add_saturating(l: i64, r: i64) -> i64 {
    l.checked_add(r)
        .unwrap_or(if r >= 0 { i64::MAX } else { i64::MIN })
}

/// `l - r`, saturating at the i64 rails.
fn sub_saturating(l: i64, r: i64) -> i64 {
    l.checked_sub(r)
        .unwrap_or(if r <= 0 { i64::MAX } else { i64::MIN })
}

/// `l * r`, saturating toward the sign-correct rail.
fn mul_saturating(l: i64, r: i64) -> i64 {
    l.checked_mul(r).unwrap_or(if (l >= 0) == (r >= 0) {
        i64::MAX
    } else {
        i64::MIN
    })
}

/// Recursive Total-mode fold: consts as-is, dice groups as their kept-record
/// sums (consumed left-to-right via `next_group`), operators saturating, `Call`
/// nodes folding each argument in left-to-right order (the same cursor-threading
/// `Bin` uses, generalized to N children) then applying `apply_fn`. A `BestOf` node folds
/// every branch the same way, then yields the branch total that is best under
/// `FoldState::direction` (ties go to the leftmost) and records the other branches'
/// group-index ranges in `FoldState::losers`.
///
/// `sign` is the EFFECTIVE additive sign of the subtree, used only for the labeled-const
/// chips (`RollOutcome::labeled_consts`): negation (`Neg`) and the right side of `Sub` flip it,
/// so `-3[dex]` and `1d20 - 3[dex]` both display -3. Multiplicative context (`Mul`/`Div`) is NOT
/// folded in: a `2 * 3[dex]` displays its literal 3, mirroring how a `DieRecord`'s raw face is
/// shown regardless of the arithmetic around its group. Only the winning `BestOf` branch
/// contributes chips.
fn fold(
    expr: &Expr,
    raws: &RawRoll,
    next_group: &mut usize,
    sign: i32,
    state: &mut FoldState,
) -> i64 {
    match expr {
        Expr::Const(c) => {
            if c.label.is_some() {
                state.labeled_consts.push(ConstTerm {
                    // saturating_neg: i32::MIN has no i32 negation.
                    value: if sign < 0 {
                        c.value.saturating_neg()
                    } else {
                        c.value
                    },
                    label: c.label.clone(),
                });
            }
            c.value as i64
        }
        // `i64::MIN.checked_neg()` is `None` (its magnitude has no positive
        // i64 representation); saturate to `i64::MAX` instead of the raw `-`
        // negation, which is checked-overflow (panics) even in release.
        Expr::Neg(inner) => fold(inner, raws, next_group, -sign, state)
            .checked_neg()
            .unwrap_or(i64::MAX),
        Expr::Dice(_) => {
            let gi = *next_group;
            *next_group += 1;
            raws.records
                .iter()
                .filter(|r| r.group_index == gi && r.kept)
                .map(|r| r.value as i64)
                .fold(0i64, checked_add_saturating)
        }
        Expr::Bin { op, lhs, rhs } => {
            let l = fold(lhs, raws, next_group, sign, state);
            let rhs_sign = if matches!(op, BinOp::Sub) {
                -sign
            } else {
                sign
            };
            let r = fold(rhs, raws, next_group, rhs_sign, state);
            match op {
                BinOp::Add => add_saturating(l, r),
                BinOp::Sub => sub_saturating(l, r),
                BinOp::Mul => mul_saturating(l, r),
                // Division by zero yields 0. The parser accepts a literal
                // `/0`, so this branch is reachable from untrusted notation.
                BinOp::Div => {
                    if r == 0 {
                        0
                    } else {
                        // `i64::MIN / -1` is the one division that overflows i64
                        // (its magnitude has no positive i64 representation) --
                        // Rust's `/` is checked-overflow even in release, which
                        // would panic and abort the whole process under
                        // panic=abort. Reachable from untrusted chat input via a
                        // pure-const chain (`mul_saturating` can produce an exact
                        // `i64::MIN`, and `-1` comes from `Expr::Neg`), with zero
                        // dice groups so the roll-count caps never see it.
                        l.checked_div(r).unwrap_or(i64::MAX)
                    }
                }
            }
        }
        Expr::Call { name, args } => {
            let mut vals = Vec::with_capacity(args.len());
            for arg in args {
                vals.push(fold(arg, raws, next_group, sign, state));
            }
            apply_fn(*name, &vals)
        }
        Expr::BestOf(args) => {
            let mut totals = Vec::with_capacity(args.len());
            let mut ranges = Vec::with_capacity(args.len());
            let mut chips = Vec::with_capacity(args.len());
            for arg in args {
                let start = *next_group;
                let outer_chips = std::mem::take(&mut state.labeled_consts);
                totals.push(fold(arg, raws, next_group, sign, state));
                chips.push(std::mem::replace(&mut state.labeled_consts, outer_chips));
                ranges.push(start..*next_group);
            }
            let mut winner = 0usize;
            for (i, total) in totals.iter().enumerate().skip(1) {
                let better = match state.direction {
                    Direction::HighWins => *total > totals[winner],
                    Direction::LowWins => *total < totals[winner],
                };
                if better {
                    winner = i;
                }
            }
            for (i, range) in ranges.into_iter().enumerate() {
                if i != winner {
                    state.losers.push((range, i as u32));
                }
            }
            state.labeled_consts.append(&mut chips.swap_remove(winner));
            totals[winner]
        }
    }
}

/// Applies a math function to its already-`fold`ed argument values. Indexes
/// defensively (`unwrap_or(0)` via the local `a` closure, mirroring `fold`'s own
/// Div-by-zero-to-0 convention) rather than panicking on an argument count that
/// disagrees with `name.accepts_arg_count` — unreachable from `dice::notation::parser`-
/// constructed input (arity is checked at parse time there), but this crate's own types
/// stay unvalidated by design for a hand-constructed `RollSpec`.
/// `Floor`/`Ceil`/`Round` are true integer no-ops (`fold` never produces a fractional
/// value for any other `Expr` variant — `BinOp::Div` truncates toward zero rather than
/// producing a fraction), NOT a round-trip through `f64`: an `i64 -> f64` cast itself
/// loses precision above `f64`'s 53-bit mantissa (~9e15, well under `i64::MAX`), which
/// would silently corrupt a large already-integer input before any rounding function
/// ever ran. These three functions exist so notation and the engine formula grammar
/// (`crate::formula`) accept the same function names.
/// `Min`/`Max` are variadic (`FnName::accepts_arg_count` admits any count `>= 2`): both fold
/// over every element of `args` rather than indexing two fixed slots, so a 3+ argument call
/// is folded the same way as a 2-argument call. An empty slice (unreachable from the
/// parser, since even the minimum admissible count is 2) defensively yields 0, matching
/// `fold`'s Div-by-zero-to-0 convention.
fn apply_fn(name: FnName, args: &[i64]) -> i64 {
    let a = |i: usize| args.get(i).copied().unwrap_or(0);
    match name {
        FnName::Floor | FnName::Ceil | FnName::Round => a(0),
        FnName::Abs => a(0).saturating_abs(),
        FnName::Min => args.iter().copied().reduce(i64::min).unwrap_or(0),
        FnName::Max => args.iter().copied().reduce(i64::max).unwrap_or(0),
    }
}

#[cfg(test)]
mod best_of;
#[cfg(test)]
mod tests;
