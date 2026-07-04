use crate::dice::eval::evaluate;
use crate::dice::eval::groups::resolve_group;
use crate::dice::outcome::{RawDie, RawRoll, RollOutcome};
use crate::dice::rng::{roll_uniform, RngSource};
use crate::dice::spec::{DieId, DieKind, Expr, RollSpec};

/// A targeted mutation of a roll's BASE natural dice, applied by `recalculate`
/// before re-deriving the pipeline. Operates on dice ids assigned by `roll` (or a
/// prior `recalculate`) — ids are stable across recalculation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecalcOp {
    /// Draw a fresh natural (via `rng`) for each targeted die.
    RerollDice(Vec<DieId>),
    /// Force a specific natural onto one die (e.g. GM override, undo).
    ReplaceDie { id: DieId, natural: i32 },
    /// Drop targeted dice from their group's base naturals entirely.
    RemoveDice(Vec<DieId>),
}

/// Apply targeted ops to each group's BASE natural dice (reconstructed from
/// `raws.group_spans`, which excludes any prior explosion/penetrate children), then
/// re-derive every group's records by re-running `resolve_group` over the mutated
/// naturals in AST order, then `evaluate`. Empty `ops` is an identity: `rederive`
/// replays the exact same naturals/ids through the exact same pipeline, so with no
/// reroll/explode modifiers `recalculate(spec, raws, &[], rng) == (raws.clone(),
/// evaluate(spec, raws))`. Reroll draws from `rng` (server-authoritative: the caller
/// supplies a seeded/entropy-backed source, never a client-provided face).
pub fn recalculate(
    spec: &RollSpec,
    raws: &RawRoll,
    ops: &[RecalcOp],
    rng: &mut dyn RngSource,
) -> (RawRoll, RollOutcome) {
    // Rebuild per-group base naturals from spans (excludes explosion/penetrate
    // children pushed past the span during the original roll/recalc).
    let mut groups: Vec<Vec<RawDie>> = raws
        .group_spans
        .iter()
        .map(|&(start, count)| raws.dice[start..start + count].to_vec())
        .collect();

    // Apply ops against the base dice only.
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

    // Re-derive: walk the AST, consuming groups in order, re-running each pipeline
    // over the (possibly mutated) base naturals. Fresh `RawRoll` — `next_id` carries
    // forward so any new dice (rerolled? no; only explosion children) never collide
    // with ids already handed out.
    let mut out = RawRoll {
        next_id: raws.next_id,
        ..Default::default()
    };
    let mut group_index = 0usize;
    rederive(&spec.expr, &groups, &mut group_index, rng, &mut out);
    let outcome = evaluate(spec, &out);
    (out, outcome)
}

/// Mirrors `eval::roll_expr`'s AST walk, but sources each `Dice` node's naturals
/// from `groups[index]` (already ops-mutated) instead of rolling fresh dice, then
/// re-runs `resolve_group` — the SAME group_index-aware entry point `roll` uses —
/// so `group_index` stamping, explosion re-triggering, and keep/drop all behave
/// identically to a fresh roll over these naturals.
fn rederive(
    expr: &Expr,
    groups: &[Vec<RawDie>],
    group_index: &mut usize,
    rng: &mut dyn RngSource,
    out: &mut RawRoll,
) {
    match expr {
        Expr::Dice(group) => {
            let index = *group_index;
            *group_index += 1;
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
        Expr::Neg(inner) => rederive(inner, groups, group_index, rng, out),
        Expr::Bin { lhs, rhs, .. } => {
            rederive(lhs, groups, group_index, rng, out);
            rederive(rhs, groups, group_index, rng, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dice::eval::{evaluate, roll};
    use crate::dice::rng::NoiseRng;
    use crate::dice::spec::{Comparator, DiceGroup, DieKind, Expr, Mode, RollSpec, SuccessRule};

    fn pool(count: u32) -> RollSpec {
        RollSpec {
            expr: Expr::Dice(DiceGroup {
                count,
                kind: DieKind::Numeric { min: 1, max: 10 },
                modifiers: vec![],
            }),
            mode: Mode::SuccessCount,
            success: Some(SuccessRule {
                comp: Comparator::Gte,
                target: 7,
            }),
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
        let (raws2, _out) = recalculate(
            &spec,
            &raws,
            &[RecalcOp::RerollDice(vec![target])],
            &mut rng,
        );
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
        let (r1, _) = recalculate(
            &spec,
            &raws,
            &[RecalcOp::ReplaceDie { id, natural: 10 }],
            &mut rng,
        );
        let (r2, out2) = recalculate(
            &spec,
            &r1,
            &[RecalcOp::ReplaceDie { id, natural: orig }],
            &mut rng,
        );
        assert_eq!(out2, evaluate(&spec, &raws));
        assert_eq!(r2.dice[0].natural, orig);
    }
}
