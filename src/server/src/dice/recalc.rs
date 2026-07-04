use crate::dice::eval::evaluate;
use crate::dice::eval::groups::resolve_group;
use crate::dice::outcome::{RawDie, RawRoll, RollOutcome};
use crate::dice::rng::{roll_uniform, RngSource};
use crate::dice::spec::{DieId, DieKind, Expr, RollSpec};

/// A targeted mutation of a roll's BASE natural dice, applied by `recalculate`
/// before re-deriving the pipeline. Operates on dice ids assigned by `roll` (or a
/// prior `recalculate`) — ids are stable across recalculation. An id naming an
/// explosion/penetrate child (never part of a group's base span, see
/// `RawRoll::group_spans`), or any id not present in the current base set, is
/// silently ignored rather than treated as an error.
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
    use crate::dice::eval::groups::resolve_group;
    use crate::dice::eval::{evaluate, roll};
    use crate::dice::rng::NoiseRng;
    use crate::dice::spec::{
        Comparator, DiceGroup, DieKind, ExplodeKind, Expr, GroupModifier, Mode, RollSpec,
        SuccessRule,
    };

    /// Test-only deterministic `RngSource`: replays a scripted sequence of
    /// `next_u32()` results in call order. Panics if the script is exhausted,
    /// surfacing a miscalibrated test immediately rather than reading garbage.
    /// Mirrors the identical stub in `eval::groups::tests` (private per-module,
    /// so duplicated rather than shared).
    struct ScriptedRng {
        values: std::vec::IntoIter<u32>,
    }

    impl ScriptedRng {
        fn new(values: Vec<u32>) -> Self {
            ScriptedRng {
                values: values.into_iter(),
            }
        }
    }

    impl RngSource for ScriptedRng {
        fn next_u32(&mut self) -> u32 {
            self.values
                .next()
                .expect("ScriptedRng exhausted: script too short for this test")
        }
    }

    /// `roll_uniform` on a d6 (min=1, max=6, span=6) maps a raw `next_u32()` value
    /// `x` to face `1 + x % 6` whenever `x` is below the rejection limit — true for
    /// every small `x` used by these scripts. So scripting `x` directly scripts
    /// the face.
    fn face_x(face: i32) -> u32 {
        (face - 1) as u32
    }

    fn d6(id: u32, natural: i32) -> RawDie {
        RawDie {
            id,
            kind: DieKind::Numeric { min: 1, max: 6 },
            natural,
        }
    }

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

    /// A 2-die Explode group: die 0 is the ONLY id named by `ops`; die 1 is never
    /// named but independently satisfies the Explode trigger on every resolution.
    fn explode_spec_and_raws() -> (RollSpec, RawRoll) {
        let group = DiceGroup {
            count: 2,
            kind: DieKind::Numeric { min: 1, max: 6 },
            modifiers: vec![GroupModifier::Explode {
                kind: ExplodeKind::Standard,
                comp: Comparator::Gte,
                target: 6,
            }],
        };
        let spec = RollSpec {
            expr: Expr::Dice(group.clone()),
            mode: Mode::Sum,
            success: None,
            required_successes: None,
        };
        // die 0 = 3 (below the Gte(6) trigger, target of the op below); die 1 = 6
        // (at the trigger, and NEVER named by any op in these tests).
        let naturals = vec![d6(0, 3), d6(1, 6)];
        let mut raws = RawRoll {
            dice: naturals.clone(),
            records: vec![],
            next_id: 2,
            group_spans: vec![(0, 2)],
        };
        // Establish the "before" derived state directly via `resolve_group` (the
        // same entry point `roll`/`rederive` use), so it reflects one real
        // resolution of this group rather than a hand-authored fixture.
        let mut rng_before = ScriptedRng::new(vec![face_x(3)]); // die1's 1 extra: face 3, non-triggering, chain stops
        let recs = resolve_group(&group, 0, &naturals, &mut rng_before, &mut raws);
        raws.records = recs;
        (spec, raws)
    }

    #[test]
    fn explosion_tail_for_untouched_sibling_changes_across_recalc() {
        // Finding (a): `ops` names ONLY die 0. Die 1 (never named) independently
        // satisfies the group's Explode trigger every time `resolve_group` runs,
        // and `recalculate` unconditionally re-runs `resolve_group` fresh over
        // base naturals for the WHOLE group on every call — so die 1's exploded
        // tail is expected to change across a recalc call even though die 1 was
        // never touched by `ops`.
        let (spec, raws_before) = explode_spec_and_raws();
        // Sanity on the "before" state: die 1's single explosion pass produced
        // exactly 1 extra die (base 2 + 1 extra = 3).
        assert_eq!(
            raws_before.dice.len(),
            3,
            "before: base 2 + 1 exploded extra"
        );
        assert_eq!(raws_before.dice[1].natural, 6);

        // Recalculate targeting ONLY die 0 (RerollDice), with a rng script that
        // gives die 1's re-triggered explosion chain a DIFFERENT tail: two
        // extras instead of one (face 6 continues the chain, face 2 stops it).
        let mut rng_after = ScriptedRng::new(vec![
            face_x(4), // die 0's reroll draw (op-driven, irrelevant to die 1)
            face_x(6), // die 1's 1st extra: still >= 6, chain continues
            face_x(2), // die 1's 2nd extra: < 6, chain stops
        ]);
        let (raws_after, _out) = recalculate(
            &spec,
            &raws_before,
            &[RecalcOp::RerollDice(vec![0])],
            &mut rng_after,
        );

        // Property that DOES hold: die 1's BASE NATURAL is untouched (`ops`
        // never named it, and `recalculate` only mutates ids present in `ops`).
        assert_eq!(
            raws_after.dice[1].natural, 6,
            "untouched die's base natural must stay stable across recalc"
        );

        // Property that does NOT hold: die 1's DERIVED exploded-child count.
        // before = 1 extra (3 dice total), after = 2 extras (4 dice total) —
        // an exact, asserted difference, not a loose bound.
        assert_eq!(raws_before.dice.len(), 3);
        assert_eq!(
            raws_after.dice.len(),
            4,
            "untouched die's exploded-child count changed across recalc, \
             even though ops never named it — this is the documented (not a bug) behavior"
        );
    }

    /// A 2-die Reroll(non-once) group: die 0 is the ONLY id named by `ops`
    /// (via `ReplaceDie`, which draws no rng); die 1 is never named but
    /// independently satisfies the chain-continuing Reroll trigger.
    fn reroll_spec_and_raws() -> (RollSpec, RawRoll) {
        let group = DiceGroup {
            count: 2,
            kind: DieKind::Numeric { min: 1, max: 6 },
            modifiers: vec![GroupModifier::Reroll {
                comp: Comparator::Lte,
                target: 2,
                once: false,
            }],
        };
        let spec = RollSpec {
            expr: Expr::Dice(group.clone()),
            mode: Mode::Sum,
            success: None,
            required_successes: None,
        };
        // die 0 = 5 (above the Lte(2) trigger; target of a ReplaceDie op below,
        // which draws no rng, so it never perturbs die 1's draws). die 1 = 1
        // (at/under the trigger, and NEVER named by any op in these tests).
        let naturals = vec![d6(0, 5), d6(1, 1)];
        let mut raws = RawRoll {
            dice: naturals.clone(),
            records: vec![],
            next_id: 2,
            group_spans: vec![(0, 2)],
        };
        // die1's chain: draw1 = face 2 (<=2, continue), draw2 = face 5 (>2, stop).
        let mut rng_before = ScriptedRng::new(vec![face_x(2), face_x(5)]);
        let recs = resolve_group(&group, 0, &naturals, &mut rng_before, &mut raws);
        raws.records = recs;
        (spec, raws)
    }

    #[test]
    fn reroll_chain_for_untouched_sibling_changes_across_recalc() {
        // Finding (b): analogous to (a) but for a non-`once` Reroll modifier —
        // a chain-continuing reroll. `ops` names ONLY die 0 (via `ReplaceDie`,
        // which mutates a base natural WITHOUT drawing from `rng`). Die 1 is
        // never named but resolves through a fresh reroll chain on every call.
        let (spec, raws_before) = reroll_spec_and_raws();
        assert_eq!(raws_before.dice.len(), 2, "Reroll never appends dice");
        assert_eq!(raws_before.dice[1].natural, 1);
        assert_eq!(
            raws_before.records[1].value, 5,
            "before: die 1's chain settles on the 2nd draw (face 5)"
        );

        // Recalculate targeting ONLY die 0 (ReplaceDie, no rng draw), with a rng
        // script that gives die 1's re-triggered reroll chain a longer tail and
        // a DIFFERENT final value: 3 draws (1, 2, 6) instead of 2 (2, 5).
        let mut rng_after = ScriptedRng::new(vec![
            face_x(1), // die 1's 1st reroll: <=2, chain continues
            face_x(2), // die 1's 2nd reroll: <=2, chain continues
            face_x(6), // die 1's 3rd reroll: >2, chain stops
        ]);
        let (raws_after, out_after) = recalculate(
            &spec,
            &raws_before,
            &[RecalcOp::ReplaceDie { id: 0, natural: 9 }],
            &mut rng_after,
        );

        // Property that DOES hold: die 1's BASE NATURAL is untouched.
        assert_eq!(
            raws_after.dice[1].natural, 1,
            "untouched die's base natural must stay stable across recalc"
        );

        // Property that does NOT hold: die 1's DERIVED chain result. before = 5
        // (settled after 2 draws), after = 6 (settled after 3 draws) — an exact
        // asserted difference in the final derived `value`, not a loose bound.
        let die1_after = out_after
            .records
            .iter()
            .find(|r| r.id == 1)
            .expect("die 1 present in the re-derived records");
        assert_eq!(
            die1_after.value, 6,
            "untouched die's chain-continuing reroll result changed across recalc, \
             even though ops never named it — this is the documented (not a bug) behavior"
        );
        assert_ne!(die1_after.value, raws_before.records[1].value);
    }

    #[test]
    fn untouched_die_natural_stability_vs_derived_instability_is_explicit() {
        // Finding (c): the actual subtlety is that TWO different properties are
        // in play for a die never named by `ops`, and they do NOT move
        // together:
        //   1. base natural (RawDie.natural / RawRoll.dice[i].natural) — STABLE
        //      across recalc; only an op naming that exact id can change it.
        //   2. derived record (DieRecord.value / exploded-child count/ids) —
        //      UNSTABLE across recalc; `resolve_group` re-derives it fresh
        //      every call from the (stable) base natural, so any live
        //      trigger (Explode/Reroll) re-fires with new rng draws.
        // This test drives TWO recalc calls back-to-back over the explode
        // fixture and asserts property 1 holds across BOTH calls while
        // property 2 differs between at least one pair of the three states
        // (original, after call 1, after call 2).
        let (spec, raws0) = explode_spec_and_raws();
        assert_eq!(raws0.dice.len(), 3, "original: base 2 + 1 exploded extra");

        let mut rng1 = ScriptedRng::new(vec![
            face_x(4), // die 0's reroll draw
            face_x(3), // die 1's re-triggered chain: 1 extra (face 3, non-triggering)
        ]);
        let (raws1, _) = recalculate(&spec, &raws0, &[RecalcOp::RerollDice(vec![0])], &mut rng1);

        let mut rng2 = ScriptedRng::new(vec![
            face_x(2), // die 0's reroll draw
            face_x(6), // die 1's re-triggered chain, extra 1: continues
            face_x(6), // die 1's re-triggered chain, extra 2: continues
            face_x(1), // die 1's re-triggered chain, extra 3: stops
        ]);
        let (raws2, _) = recalculate(&spec, &raws1, &[RecalcOp::RerollDice(vec![0])], &mut rng2);

        // Property 1 (STABLE): die 1's base natural is identical across all
        // three states — original roll, after recalc 1, after recalc 2.
        assert_eq!(raws0.dice[1].natural, 6);
        assert_eq!(raws1.dice[1].natural, 6);
        assert_eq!(raws2.dice[1].natural, 6);

        // Property 2 (UNSTABLE): die 1's exploded-child count differs across
        // the three states: 1 extra, then 1 extra again (call 1's script
        // happens to also yield exactly 1), then 3 extras (call 2's script).
        // Total dice per group = base 2 + extras.
        assert_eq!(raws0.dice.len(), 3, "original: 1 extra");
        assert_eq!(raws1.dice.len(), 3, "recalc 1: coincidentally also 1 extra");
        assert_eq!(
            raws2.dice.len(),
            5,
            "recalc 2: 3 extras — proves the tail is NOT pinned to the original's \
             shape even though die 1's base natural never moved"
        );
    }
}
