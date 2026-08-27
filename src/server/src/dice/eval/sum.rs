#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::dice::eval::classify;
use crate::dice::outcome::{RawRoll, RollOutcome};
use crate::dice::spec::{BinOp, ConstTerm, Expr, FnName, RollSpec, TotalConfig};

/// Fold the AST to a total. Each `Dice` node contributes the sum of its group's kept
/// records (matched by `group_index`); a cursor consumes groups in AST order. If
/// `cfg.difficulty` is set, classifies `total` (oriented by `spec.direction`) against
/// `cfg.tiers` into `margin`/`pass`/`tier_label`/`tier_value`; otherwise reports a bare total.
pub fn evaluate_total(spec: &RollSpec, cfg: &TotalConfig, raws: &RawRoll) -> RollOutcome {
    let mut next_group = 0usize;
    let total = fold(&spec.expr, raws, &mut next_group);
    let (pass, margin, tier_label, tier_value) = match cfg.difficulty {
        None => (None, None, None, None),
        Some(diff) => {
            let m = classify::oriented_margin(spec.direction, total, diff as i64);
            let c = classify::classify(m, &cfg.tiers);
            (c.pass, Some(m), c.tier_label, c.tier_value)
        }
    };
    let mut labeled_consts = Vec::new();
    collect_labeled_consts(&spec.expr, 1, &mut labeled_consts);
    RollOutcome {
        total,
        records: raws.records.clone(),
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
        labeled_consts,
    }
}

/// Collects every labeled `Const` term, in AST left-to-right order, for
/// chat-embed display (`RollOutcome::labeled_consts`), carrying the term's
/// EFFECTIVE additive sign: negation (`Neg`) and the right side of `Sub` flip
/// it, so `-3[dex]` and `1d20 - 3[dex]` both display -3. Multiplicative
/// context (`Mul`/`Div`) is NOT folded in — a `2 * 3[dex]` displays its
/// literal 3, mirroring how a `DieRecord`'s raw face is shown regardless of
/// the arithmetic around its group. Total mode only; SuccessCount ignores the
/// arithmetic entirely.
fn collect_labeled_consts(expr: &Expr, sign: i32, out: &mut Vec<ConstTerm>) {
    match expr {
        Expr::Const(c) => {
            if c.label.is_some() {
                out.push(ConstTerm {
                    // saturating_neg: i32::MIN has no i32 negation.
                    value: if sign < 0 {
                        c.value.saturating_neg()
                    } else {
                        c.value
                    },
                    label: c.label.clone(),
                });
            }
        }
        Expr::Dice(_) => {}
        Expr::Neg(inner) => collect_labeled_consts(inner, -sign, out),
        Expr::Bin {
            op: BinOp::Sub,
            lhs,
            rhs,
        } => {
            collect_labeled_consts(lhs, sign, out);
            collect_labeled_consts(rhs, -sign, out);
        }
        Expr::Bin { lhs, rhs, .. } => {
            collect_labeled_consts(lhs, sign, out);
            collect_labeled_consts(rhs, sign, out);
        }
        Expr::Call { args, .. } => {
            for arg in args {
                collect_labeled_consts(arg, sign, out);
            }
        }
    }
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
/// `Bin` uses, generalized to N children) then applying `apply_fn`.
fn fold(expr: &Expr, raws: &RawRoll, next_group: &mut usize) -> i64 {
    match expr {
        Expr::Const(c) => c.value as i64,
        // `i64::MIN.checked_neg()` is `None` (its magnitude has no positive
        // i64 representation); saturate to `i64::MAX` instead of the raw `-`
        // negation, which is checked-overflow (panics) even in release.
        Expr::Neg(inner) => fold(inner, raws, next_group)
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
            let l = fold(lhs, raws, next_group);
            let r = fold(rhs, raws, next_group);
            match op {
                BinOp::Add => add_saturating(l, r),
                BinOp::Sub => sub_saturating(l, r),
                BinOp::Mul => mul_saturating(l, r),
                // Division by zero yields 0 — the ONLY guard; the parser
                // accepts a literal `/0`, so this branch is reachable from
                // untrusted notation.
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
                vals.push(fold(arg, raws, next_group));
            }
            apply_fn(*name, &vals)
        }
    }
}

/// Applies a math function to its already-`fold`ed argument values. Indexes
/// defensively (`unwrap_or(0)` via the local `a` closure, mirroring `fold`'s own
/// Div-by-zero-to-0 convention) rather than panicking on an argument count that
/// disagrees with `name.arity()` — unreachable from `dice::notation::parser`-constructed
/// input (arity is checked at parse time there), but this crate's own types stay
/// unvalidated by design for a hand-constructed `RollSpec`.
/// `Floor`/`Ceil`/`Round` are true integer no-ops (`fold` never produces a fractional
/// value for any other `Expr` variant — `BinOp::Div` truncates toward zero rather than
/// producing a fraction), NOT a round-trip through `f64`: an `i64 -> f64` cast itself
/// loses precision above `f64`'s 53-bit mantissa (~9e15, well under `i64::MAX`), which
/// would silently corrupt a large already-integer input before any rounding function
/// ever ran. These three functions exist for functional parity with
/// `@shadowcat/formula`'s own function set; if a future change gives this grammar a
/// true fractional value, their implementation must be revisited then.
fn apply_fn(name: FnName, args: &[i64]) -> i64 {
    let a = |i: usize| args.get(i).copied().unwrap_or(0);
    match name {
        FnName::Floor | FnName::Ceil | FnName::Round => a(0),
        FnName::Abs => a(0).saturating_abs(),
        FnName::Min => a(0).min(a(1)),
        FnName::Max => a(0).max(a(1)),
    }
}

#[cfg(test)]
mod tests;
