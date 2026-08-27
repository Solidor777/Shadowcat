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
        label: None,
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
        label: None,
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
        label: None,
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
        label: None,
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
        label: None,
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
        label: None,
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
        label: None,
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
        label: None,
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
        label: None,
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
fn label_propagates_to_every_record_including_exploded_children() {
    // Same fixture shape as the group_index propagation test: a single die at
    // max that explodes once, so the label must reach BOTH the original
    // record and the pushed exploded child.
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
        label: Some("Hope".to_string()),
    };
    let mut rng = ScriptedRng::new(vec![face_x(6), face_x(3)]);
    let recs = resolve_group(&g, 0, &naturals, &mut rng, &mut raws);
    assert_eq!(recs.len(), 3, "1 original + 2 chained extras");
    assert!(
        recs.iter().all(|r| r.label.as_deref() == Some("Hope")),
        "label must propagate to the original AND every exploded child record"
    );
}

fn faces_group(faces: Vec<crate::dice::spec::Face>, count: u32) -> DiceGroup {
    DiceGroup {
        count,
        kind: DieKind::Faces { faces },
        modifiers: vec![],
        label: None,
    }
}

#[test]
fn faces_die_derives_value_and_symbols_from_index() {
    use crate::dice::spec::Face;
    let faces = vec![
        Face {
            value: Some(1),
            symbols: vec!["blank".into()],
        },
        Face {
            value: Some(3),
            symbols: vec!["success".into(), "triumph".into()],
        },
    ];
    // natural = 1 selects faces[1].
    let naturals = vec![RawDie {
        id: 0,
        kind: DieKind::Faces {
            faces: faces.clone(),
        },
        natural: 1,
    }];
    let mut raws = RawRoll {
        dice: naturals.clone(),
        records: vec![],
        next_id: 1,
        group_spans: vec![],
    };
    let mut rng = NoiseRng::from_seed(1);
    let recs = resolve_group(&faces_group(faces, 1), 0, &naturals, &mut rng, &mut raws);
    assert_eq!(recs[0].value, 3);
    assert_eq!(
        recs[0].symbols,
        vec!["success".to_string(), "triumph".to_string()]
    );
}

#[test]
fn faces_die_none_value_face_contributes_zero() {
    use crate::dice::spec::Face;
    let faces = vec![Face {
        value: None,
        symbols: vec!["blank".into()],
    }];
    let naturals = vec![RawDie {
        id: 0,
        kind: DieKind::Faces {
            faces: faces.clone(),
        },
        natural: 0,
    }];
    let mut raws = RawRoll {
        dice: naturals.clone(),
        records: vec![],
        next_id: 1,
        group_spans: vec![],
    };
    let mut rng = NoiseRng::from_seed(1);
    let recs = resolve_group(&faces_group(faces, 1), 0, &naturals, &mut rng, &mut raws);
    assert_eq!(
        recs[0].value, 0,
        "a None-value face contributes 0 numerically"
    );
    assert_eq!(recs[0].symbols, vec!["blank".to_string()]);
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
        label: None,
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

#[test]
fn unordered_faces_die_is_skipped_by_keep_highest() {
    use crate::dice::spec::Face;
    // Two unordered faces dice (value: None) mixed conceptually with keep-highest —
    // since ALL dice in this group are the same DieKind, this test exercises the
    // group-level gate: an unordered Faces group's keep/drop modifier must be a
    // no-op (every die stays kept), not a ranking-by-value(0) accident.
    let faces = vec![
        Face {
            value: None,
            symbols: vec!["a".into()],
        },
        Face {
            value: None,
            symbols: vec!["b".into()],
        },
    ];
    let naturals = vec![
        RawDie {
            id: 0,
            kind: DieKind::Faces {
                faces: faces.clone(),
            },
            natural: 0,
        },
        RawDie {
            id: 1,
            kind: DieKind::Faces {
                faces: faces.clone(),
            },
            natural: 1,
        },
    ];
    let mut raws = RawRoll {
        dice: naturals.clone(),
        records: vec![],
        next_id: 2,
        group_spans: vec![],
    };
    let g = DiceGroup {
        count: 2,
        kind: DieKind::Faces { faces },
        modifiers: vec![GroupModifier::KeepHighest(1)],
        label: None,
    };
    let mut rng = NoiseRng::from_seed(1);
    let recs = resolve_group(&g, 0, &naturals, &mut rng, &mut raws);
    assert!(
        recs.iter().all(|r| r.kept),
        "unordered Faces group must skip keep/drop entirely"
    );
}

#[test]
fn ordered_faces_die_participates_in_keep_highest_like_numeric() {
    use crate::dice::spec::Face;
    let faces = vec![
        Face {
            value: Some(1),
            symbols: vec![],
        },
        Face {
            value: Some(6),
            symbols: vec![],
        },
    ];
    let naturals = vec![
        RawDie {
            id: 0,
            kind: DieKind::Faces {
                faces: faces.clone(),
            },
            natural: 0,
        }, // value 1
        RawDie {
            id: 1,
            kind: DieKind::Faces {
                faces: faces.clone(),
            },
            natural: 1,
        }, // value 6
    ];
    let mut raws = RawRoll {
        dice: naturals.clone(),
        records: vec![],
        next_id: 2,
        group_spans: vec![],
    };
    let g = DiceGroup {
        count: 2,
        kind: DieKind::Faces { faces },
        modifiers: vec![GroupModifier::KeepHighest(1)],
        label: None,
    };
    let mut rng = NoiseRng::from_seed(1);
    let recs = resolve_group(&g, 0, &naturals, &mut rng, &mut raws);
    let kept: Vec<i32> = recs.iter().filter(|r| r.kept).map(|r| r.value).collect();
    assert_eq!(
        kept,
        vec![6],
        "ordered Faces group ranks exactly like Numeric"
    );
}

#[test]
fn explode_retrigger_uses_derived_value_not_raw_index_for_ordered_faces() {
    use crate::dice::spec::Face;
    // Faces where index does NOT track value monotonically: index 0 ->
    // value 6, index 1 -> value 1. A retrigger check against the raw
    // drawn INDEX (instead of the die's derived value) would test
    // `comp.test(0, 6)` on the first extra draw and wrongly stop the
    // chain, even though the die's actual configured value (6) satisfies
    // `Gte(6)` and the chain must continue.
    let faces = vec![
        Face {
            value: Some(6),
            symbols: vec![],
        },
        Face {
            value: Some(1),
            symbols: vec![],
        },
    ];
    let naturals = vec![RawDie {
        id: 0,
        kind: DieKind::Faces {
            faces: faces.clone(),
        },
        natural: 0, // value 6, triggers Gte(6)
    }];
    let mut raws = RawRoll {
        dice: naturals.clone(),
        records: vec![],
        next_id: 1,
        group_spans: vec![],
    };
    let g = DiceGroup {
        count: 1,
        kind: DieKind::Faces { faces },
        modifiers: vec![GroupModifier::Explode {
            kind: ExplodeKind::Standard,
            comp: Comparator::Gte,
            target: 6,
        }],
        label: None,
    };
    // Chain roll 1 draws index 0 (derived value 6 -> must retrigger).
    // Chain roll 2 draws index 1 (derived value 1 -> stops).
    let mut rng = ScriptedRng::new(vec![0, 1]);
    let recs = resolve_group(&g, 0, &naturals, &mut rng, &mut raws);
    assert_eq!(
        recs.len(),
        3,
        "chain must continue based on the die's derived value, not the raw drawn index"
    );
    assert_eq!(recs[1].value, 6);
    assert_eq!(recs[2].value, 1);
}
