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
                                    recs.push(DieRecord {
                                        id,
                                        natural: extra,
                                        value: extra,
                                        kept: true,
                                        exploded: false,
                                        rerolled_from: None,
                                    });
                                }
                                ExplodeKind::Standard => {
                                    let id = raws.push(group.kind.clone(), extra);
                                    recs.push(DieRecord {
                                        id,
                                        natural: extra,
                                        value: extra,
                                        kept: true,
                                        exploded: false,
                                        rerolled_from: None,
                                    });
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
        r.kept = if keep_selected {
            is_selected
        } else {
            !is_selected
        };
    }
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

    #[test]
    fn keep_highest_flags_lowest_as_not_kept() {
        let naturals = vec![d6(0, 2), d6(1, 5), d6(2, 3), d6(3, 6)];
        let mut raws = RawRoll {
            dice: naturals.clone(),
            records: vec![],
            next_id: 4,
        };
        let mut rng = NoiseRng::from_seed(1);
        let recs = resolve_group(
            &group(vec![GroupModifier::KeepHighest(3)]),
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
        let recs = resolve_group(&g, &naturals, &mut rng, &mut raws);
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
        let recs = resolve_group(&g, &naturals, &mut rng, &mut raws);
        assert!(
            recs.len() >= 3,
            "expected at least one extra die from the explosion"
        );
        assert!(recs[0].exploded);
    }
}
