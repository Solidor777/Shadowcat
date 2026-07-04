use crate::dice::outcome::{DieRecord, RawDie, RawRoll};
use crate::dice::rng::{roll_uniform, RngSource};
use crate::dice::spec::{DiceGroup, DieKind, ExplodeKind, GroupModifier};

/// Hard cap on chained explosions/rerolls per die — prevents infinite loops when a
/// comparator is always true (e.g. explode on `>=1`).
const CHAIN_CAP: usize = 100;

/// Resolve one group's dice: apply modifiers in order, returning per-die records.
/// Reroll/explode mutate the die set (new dice allocated via `raws`); keep/drop only
/// flip `kept`. All rolled dice stay in the returned vec (dropped dice for display).
///
/// CONTRACT: Reroll and Explode only act on currently-`kept` dice. Since modifiers
/// apply in vec order and keep/drop can precede reroll/explode in the same group,
/// a die already dropped by an earlier `KeepHighest`/`KeepLowest`/`DropHighest`/
/// `DropLowest` must not be rerolled or exploded by a later modifier.
///
/// `group_index` identifies which `Dice` AST node this call is resolving (assigned
/// by the caller in AST left-to-right order); it is stamped onto every `DieRecord`
/// produced here, including exploded/penetrated children, so Total-mode evaluation
/// can fold per-group without positional heuristics.
pub fn resolve_group(
    group: &DiceGroup,
    group_index: usize,
    naturals: &[RawDie],
    rng: &mut dyn RngSource,
    raws: &mut RawRoll,
) -> Vec<DieRecord> {
    let DieKind::Numeric { min, max } = group.kind;
    let mut recs: Vec<DieRecord> = naturals
        .iter()
        .map(|d| DieRecord {
            id: d.id,
            group_index,
            natural: d.natural,
            value: d.natural,
            kept: true,
            exploded: false,
            rerolled_from: None,
            crit_success: false,
            crit_fail: false,
            expertise: 0,
        })
        .collect();

    for m in &group.modifiers {
        match m {
            GroupModifier::Reroll { comp, target, once } => {
                for r in recs.iter_mut() {
                    // Dropped dice are not live participants in later modifiers.
                    if !r.kept {
                        continue;
                    }
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
                // Snapshot the pool length before this pass: the outer loop only
                // trigger-scans dice that existed when the modifier started. The
                // inner chain loop below is the SOLE mechanism that extends any
                // given die's own chain (it rechecks the trigger on the fresh
                // face itself); a child it pushes must never be independently
                // re-scanned by the outer loop as if it were a fresh, un-exploded
                // die — that double-applies CHAIN_CAP per pushed child and can
                // balloon without bound when the comparator is often-true.
                let initial_len = recs.len();
                let mut i = 0;
                while i < initial_len {
                    // Dropped dice are not live participants in later modifiers.
                    if recs[i].kept && comp.test(recs[i].value, *target) {
                        recs[i].exploded = true;
                        let mut chain = 0;
                        loop {
                            if chain >= CHAIN_CAP {
                                break;
                            }
                            let extra = roll_uniform(rng, min, max);
                            match kind {
                                ExplodeKind::Compound => {
                                    recs[i].value += extra;
                                }
                                ExplodeKind::Penetrate => {
                                    // Penetrate: the chain-continuation recheck below
                                    // uses the RAW roll `extra` (mirroring Compound),
                                    // not the decremented value — otherwise a
                                    // `Gte(max)` chain could never extend past one
                                    // extra die, since `max - 1` never satisfies
                                    // `Gte(max)` even when the die keeps rolling max.
                                    // `natural` records the true RNG face for the
                                    // audit trail; only the stored `value` (used for
                                    // scoring/display) takes the -1 penalty, and may
                                    // therefore land below `min` by design.
                                    let value = extra - 1;
                                    push_extra(
                                        &mut recs,
                                        raws,
                                        group.kind.clone(),
                                        group_index,
                                        extra,
                                        value,
                                    );
                                }
                                ExplodeKind::Standard => {
                                    push_extra(
                                        &mut recs,
                                        raws,
                                        group.kind.clone(),
                                        group_index,
                                        extra,
                                        extra,
                                    );
                                }
                            }
                            chain += 1;
                            // Re-check the trigger on the fresh (raw) face.
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
        r.kept = if keep_selected {
            is_selected
        } else {
            !is_selected
        };
    }
}

/// Push one exploded/penetrated extra die into both the raw log and the per-die
/// record vec. `natural` is the true RNG face; `value` is the post-modifier
/// stored value, which for Penetrate differs from `natural` by the -1 penalty.
fn push_extra(
    recs: &mut Vec<DieRecord>,
    raws: &mut RawRoll,
    kind: DieKind,
    group_index: usize,
    natural: i32,
    value: i32,
) {
    let id = raws.push(kind, natural);
    recs.push(DieRecord {
        id,
        group_index,
        natural,
        value,
        kept: true,
        exploded: false,
        rerolled_from: None,
        crit_success: false,
        crit_fail: false,
        expertise: 0,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dice::outcome::{RawDie, RawRoll};
    use crate::dice::rng::NoiseRng;
    use crate::dice::spec::{Comparator, DiceGroup, DieKind, ExplodeKind, GroupModifier};

    fn d6(id: u32, natural: i32) -> RawDie {
        RawDie {
            id,
            kind: DieKind::Numeric { min: 1, max: 6 },
            natural,
        }
    }

    fn group(mods: Vec<GroupModifier>) -> DiceGroup {
        DiceGroup {
            count: 4,
            kind: DieKind::Numeric { min: 1, max: 6 },
            modifiers: mods,
        }
    }

    /// Test-only deterministic `RngSource`: replays a scripted sequence of
    /// `next_u32()` results in call order. Panics if the script is exhausted,
    /// surfacing a miscalibrated test immediately rather than reading garbage.
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
    /// every small `x` used by these scripts (`limit == u32::MAX - u32::MAX % 6`,
    /// far above any single-digit `x`). So scripting `x` directly scripts the face.
    fn face_x(face: i32) -> u32 {
        (face - 1) as u32
    }

    #[test]
    fn keep_highest_flags_lowest_as_not_kept() {
        let naturals = vec![d6(0, 2), d6(1, 5), d6(2, 3), d6(3, 6)];
        let mut raws = RawRoll {
            dice: naturals.clone(),
            records: vec![],
            next_id: 4,
            group_spans: vec![],
        };
        let mut rng = NoiseRng::from_seed(1);
        let recs = resolve_group(
            &group(vec![GroupModifier::KeepHighest(3)]),
            0,
            &naturals,
            &mut rng,
            &mut raws,
        );
        let kept: Vec<i32> = recs.iter().filter(|r| r.kept).map(|r| r.value).collect();
        let dropped: Vec<i32> = recs.iter().filter(|r| !r.kept).map(|r| r.value).collect();
        assert_eq!(dropped, vec![2]);
        assert_eq!(kept.iter().sum::<i32>(), 14);
    }

    #[test]
    fn reroll_once_replaces_matching_die() {
        let naturals = vec![d6(0, 1), d6(1, 4)];
        let mut raws = RawRoll {
            dice: naturals.clone(),
            records: vec![],
            next_id: 2,
            group_spans: vec![],
        };
        let g = DiceGroup {
            count: 2,
            kind: DieKind::Numeric { min: 1, max: 6 },
            modifiers: vec![GroupModifier::Reroll {
                comp: Comparator::Lte,
                target: 1,
                once: true,
            }],
        };
        let mut rng = NoiseRng::from_seed(3);
        let recs = resolve_group(&g, 0, &naturals, &mut rng, &mut raws);
        assert_eq!(recs[0].rerolled_from, Some(1));
        assert!((1..=6).contains(&recs[0].value));
    }

    #[test]
    fn standard_explode_appends_extra_on_max() {
        let naturals = vec![d6(0, 6), d6(1, 2)];
        let mut raws = RawRoll {
            dice: naturals.clone(),
            records: vec![],
            next_id: 2,
            group_spans: vec![],
        };
        let g = DiceGroup {
            count: 2,
            kind: DieKind::Numeric { min: 1, max: 6 },
            modifiers: vec![GroupModifier::Explode {
                kind: ExplodeKind::Standard,
                comp: Comparator::Gte,
                target: 6,
            }],
        };
        let mut rng = NoiseRng::from_seed(11);
        let recs = resolve_group(&g, 0, &naturals, &mut rng, &mut raws);
        // Exact count, not a loose lower bound: the outer loop trigger-scans only
        // the two original dice. Die 0 (a 6) triggers; its own chain (the sole
        // mechanism extending it) rolls until a non-6 face appears. Die 1 (a 2)
        // never triggers. `NoiseRng::from_seed(11)` deterministically produces
        // two more 6s before a non-6 (verified by running this test), so the
        // chain appends exactly two extra dice: 2 originals + 2 extras = 4.
        assert_eq!(
            recs.len(),
            4,
            "expected exactly two extra dice from the explosion chain"
        );
        assert!(recs[0].exploded);
        assert!(!recs[1].exploded);
    }

    #[test]
    fn explode_outer_loop_does_not_re_explode_pushed_children() {
        // A single die starts at 6 (max, triggers Gte(6)). Its own chain (the
        // inner loop) rolls a second 6 (still triggers, chain continues) then a 3
        // (stops). The outer loop scans only the original die (snapshotted
        // `initial_len == 1` before the pass begins); it must not revisit the
        // pushed 6 and independently explode it with its own fresh CHAIN_CAP
        // budget, which would inflate the record count and, on an often-true
        // comparator, grow without bound.
        let naturals = vec![d6(0, 6)];
        let mut raws = RawRoll {
            dice: naturals.clone(),
            records: vec![],
            next_id: 1,
            group_spans: vec![],
        };
        let g = DiceGroup {
            count: 1,
            kind: DieKind::Numeric { min: 1, max: 6 },
            modifiers: vec![GroupModifier::Explode {
                kind: ExplodeKind::Standard,
                comp: Comparator::Gte,
                target: 6,
            }],
        };
        // Chain roll 1 = face 6 (continues the chain), chain roll 2 = face 3
        // (stops it). If the outer loop revisits the pushed face-6 die, it
        // demands a third scripted value and panics (exhausted) instead of
        // reaching this assertion.
        let mut rng = ScriptedRng::new(vec![face_x(6), face_x(3)]);
        let recs = resolve_group(&g, 0, &naturals, &mut rng, &mut raws);
        // 1 original + 2 chained extras = 3.
        assert_eq!(recs.len(), 3);
        assert!(recs[0].exploded);
        assert!(!recs[1].exploded);
        assert!(!recs[2].exploded);
        assert_eq!(recs[1].value, 6);
        assert_eq!(recs[2].value, 3);
    }

    #[test]
    fn penetrate_retrigger_uses_raw_face_not_decremented_value() {
        // Penetrate's chain-continuation recheck runs against the RAW rolled
        // face, before the -1 penalty is applied. With `Gte(6)` on a d6, a die
        // rolling max every time must keep chaining; checking the decremented
        // value (max - 1 == 5) would end the chain after exactly one extra die
        // even though the underlying rolls keep hitting max.
        let naturals = vec![d6(0, 6)];
        let mut raws = RawRoll {
            dice: naturals.clone(),
            records: vec![],
            next_id: 1,
            group_spans: vec![],
        };
        let g = DiceGroup {
            count: 1,
            kind: DieKind::Numeric { min: 1, max: 6 },
            modifiers: vec![GroupModifier::Explode {
                kind: ExplodeKind::Penetrate,
                comp: Comparator::Gte,
                target: 6,
            }],
        };
        // Raw roll 1 = face 6 (raw check passes, chain continues), raw roll 2 =
        // face 3 (raw check fails, chain stops).
        let mut rng = ScriptedRng::new(vec![face_x(6), face_x(3)]);
        let recs = resolve_group(&g, 0, &naturals, &mut rng, &mut raws);
        // 1 original + 2 extras = 3: the chain extends past a single extra die,
        // which a decremented-value recheck against `Gte(max)` could never do.
        assert_eq!(recs.len(), 3);
        // Extra 1: raw natural 6, stored value 6 - 1 = 5.
        assert_eq!(recs[1].natural, 6);
        assert_eq!(recs[1].value, 5);
        // Extra 2: raw natural 3, stored value 3 - 1 = 2.
        assert_eq!(recs[2].natural, 3);
        assert_eq!(recs[2].value, 2);
    }

    #[test]
    fn dropped_die_is_not_rerolled_by_a_later_modifier() {
        // Modifiers act in vec order; a DropLowest ahead of a Reroll in the same
        // group's modifiers Vec must not let the Reroll pass touch the
        // already-dropped die.
        let naturals = vec![d6(0, 1), d6(1, 5)];
        let mut raws = RawRoll {
            dice: naturals.clone(),
            records: vec![],
            next_id: 2,
            group_spans: vec![],
        };
        let g = DiceGroup {
            count: 2,
            kind: DieKind::Numeric { min: 1, max: 6 },
            modifiers: vec![
                GroupModifier::DropLowest(1), // drops die 0 (value 1)
                GroupModifier::Reroll {
                    comp: Comparator::Eq,
                    target: 1,
                    once: true,
                },
            ],
        };
        let mut rng = NoiseRng::from_seed(5);
        let recs = resolve_group(&g, 0, &naturals, &mut rng, &mut raws);
        assert!(!recs[0].kept, "die 0 should remain dropped");
        assert_eq!(
            recs[0].value, 1,
            "dropped die's value must be untouched by the later Reroll"
        );
        assert_eq!(recs[0].rerolled_from, None);
    }

    #[test]
    fn dropped_die_is_not_exploded_by_a_later_modifier() {
        // Same contract, exercised on Explode instead of Reroll.
        let naturals = vec![d6(0, 6), d6(1, 2)];
        let mut raws = RawRoll {
            dice: naturals.clone(),
            records: vec![],
            next_id: 2,
            group_spans: vec![],
        };
        let g = DiceGroup {
            count: 2,
            kind: DieKind::Numeric { min: 1, max: 6 },
            modifiers: vec![
                GroupModifier::DropHighest(1), // drops die 0 (value 6)
                GroupModifier::Explode {
                    kind: ExplodeKind::Standard,
                    comp: Comparator::Gte,
                    target: 6,
                },
            ],
        };
        let mut rng = NoiseRng::from_seed(5);
        let recs = resolve_group(&g, 0, &naturals, &mut rng, &mut raws);
        assert!(!recs[0].kept, "die 0 should remain dropped");
        assert!(
            !recs[0].exploded,
            "dropped die must not be exploded by a later modifier"
        );
        assert_eq!(recs.len(), 2, "no extra die should be appended");
    }

    #[test]
    fn reroll_chain_iterates_multiple_times_and_terminates() {
        // Non-`once` reroll chaining at least twice, using a scripted RNG so the
        // sequence (continue, continue, stop) is exact rather than seed-hunted.
        let naturals = vec![d6(0, 1)];
        let mut raws = RawRoll {
            dice: naturals.clone(),
            records: vec![],
            next_id: 1,
            group_spans: vec![],
        };
        let g = DiceGroup {
            count: 1,
            kind: DieKind::Numeric { min: 1, max: 6 },
            modifiers: vec![GroupModifier::Reroll {
                comp: Comparator::Lte,
                target: 2,
                once: false,
            }],
        };
        // Reroll 1: face 2 (<=2, chain continues). Reroll 2: face 5 (>2, stops).
        let mut rng = ScriptedRng::new(vec![face_x(2), face_x(5)]);
        let recs = resolve_group(&g, 0, &naturals, &mut rng, &mut raws);
        assert_eq!(recs.len(), 1);
        assert_eq!(
            recs[0].value, 5,
            "chain must terminate on the first non-matching face"
        );
        // `rerolled_from` holds the immediately-preceding value (2, the first
        // reroll's result), not the original natural (1) — see the doc comment
        // on `DieRecord::rerolled_from`.
        assert_eq!(recs[0].rerolled_from, Some(2));
        // `natural` is untouched by rerolling — it is the original RNG result.
        assert_eq!(recs[0].natural, 1);
    }

    #[test]
    fn group_index_propagates_to_every_record_including_exploded_children() {
        // Non-zero group_index (2): proves the parameter reaches EVERY returned
        // record — both the initial per-natural map and every extra die pushed
        // via `push_extra` during the explosion chain — not just the naturals.
        let naturals = vec![d6(0, 6)];
        let mut raws = RawRoll {
            dice: naturals.clone(),
            records: vec![],
            next_id: 1,
            group_spans: vec![],
        };
        let g = DiceGroup {
            count: 1,
            kind: DieKind::Numeric { min: 1, max: 6 },
            modifiers: vec![GroupModifier::Explode {
                kind: ExplodeKind::Standard,
                comp: Comparator::Gte,
                target: 6,
            }],
        };
        // Chain roll 1 = face 6 (continues), chain roll 2 = face 3 (stops):
        // forces exactly one exploded child in addition to the original die.
        let mut rng = ScriptedRng::new(vec![face_x(6), face_x(3)]);
        let recs = resolve_group(&g, 2, &naturals, &mut rng, &mut raws);
        assert_eq!(recs.len(), 3, "1 original + 2 chained extras");
        assert!(
            recs.iter().all(|r| r.group_index == 2),
            "group_index must propagate to the original AND every exploded child record"
        );
    }

    #[test]
    fn penetrate_can_produce_a_value_below_min() {
        // Penetrate's -1 penalty is a deliberate house-rule departure from the
        // implicit [min, max] assumption. A natural-min extra die (face 1 on a
        // d6) stores value 0, one below min.
        let naturals = vec![d6(0, 6)];
        let mut raws = RawRoll {
            dice: naturals.clone(),
            records: vec![],
            next_id: 1,
            group_spans: vec![],
        };
        let g = DiceGroup {
            count: 1,
            kind: DieKind::Numeric { min: 1, max: 6 },
            modifiers: vec![GroupModifier::Explode {
                kind: ExplodeKind::Penetrate,
                comp: Comparator::Gte,
                target: 6,
            }],
        };
        // Raw roll = face 1 (min): the raw check against Gte(6) fails, so the
        // chain stops after exactly this one extra die.
        let mut rng = ScriptedRng::new(vec![face_x(1)]);
        let recs = resolve_group(&g, 0, &naturals, &mut rng, &mut raws);
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[1].natural, 1, "natural preserves the true rolled face");
        assert_eq!(
            recs[1].value, 0,
            "Penetrate's -1 penalty may land below min by design"
        );
    }
}
