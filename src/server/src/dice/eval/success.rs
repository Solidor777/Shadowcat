use std::collections::BTreeMap;

use crate::dice::eval::classify;
use crate::dice::eval::crit;
use crate::dice::eval::expertise;
use crate::dice::outcome::{RawRoll, RollOutcome};
use crate::dice::spec::{RollSpec, SuccessConfig};

/// Pool aggregation: count kept dice satisfying `cfg.success`, then fold each kept
/// die's crit event (`cfg.crit_success`/`cfg.crit_fail`) into net successes and the
/// positive/negative counters. Pools ALL kept records across every group —
/// SuccessCount mode ignores the AST arithmetic. Net successes clamp at 0 unless
/// `cfg.crit_fail.allow_negative` opts out of the clamp.
///
/// `required_successes`/`tiers` classify over `net - req` via the shared
/// `eval::classify`. Unlike Total mode, this margin is NOT run through
/// `oriented_margin`/direction: more successes is always better, and `direction`
/// was already applied per-die inside `crit::score_die`.
pub fn evaluate_success(spec: &RollSpec, cfg: &SuccessConfig, raws: &RawRoll) -> RollOutcome {
    let mut records = raws.records.clone();
    if cfg.expertise > 0 {
        expertise::allocate(spec.direction, cfg, raws, &mut records);
    }
    let mut raw_net = 0i32;
    let (mut pos, mut neg) = (0i32, 0i32);
    let (mut crit_s, mut crit_f) = (0i32, 0i32);
    let mut symbol_counts: BTreeMap<crate::dice::spec::Symbol, i32> = BTreeMap::new();
    for r in records.iter_mut().filter(|r| r.kept) {
        for s in &r.symbols {
            *symbol_counts.entry(s.clone()).or_insert(0) += 1;
        }
        let scored = crit::score_die_net(spec.direction, cfg, r.value, &r.symbols);
        raw_net += scored.net();
        let dc = scored.crit;
        r.crit_success = dc.is_success;
        r.crit_fail = dc.is_fail;
        if dc.is_success {
            crit_s += 1;
        }
        if dc.is_fail {
            crit_f += 1;
        }
        pos += dc.positive_counter;
        neg += dc.negative_counter;
    }
    let allow_neg = cfg
        .crit_fail
        .as_ref()
        .map(|c| c.allow_negative)
        .unwrap_or(false);
    let net = if allow_neg { raw_net } else { raw_net.max(0) };
    let total: i64 = records
        .iter()
        .filter(|r| r.kept)
        .map(|r| r.value as i64)
        .sum();
    let (pass, margin, tier_label, tier_value) = match cfg.required_successes {
        None => (None, None, None, None),
        Some(req) => {
            // Direction is NOT applied here: more successes is always better.
            let m = (net - req) as i64;
            let c = classify::classify(m, &cfg.tiers);
            (c.pass, Some(m), c.tier_label, c.tier_value)
        }
    };
    RollOutcome {
        total,
        records,
        successes: Some(net),
        pass,
        margin,
        tier_label,
        tier_value,
        crit_successes: crit_s,
        crit_fails: crit_f,
        positive_counter: pos,
        negative_counter: neg,
        symbol_counts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dice::eval::{evaluate, roll};
    use crate::dice::outcome::{DieRecord, RawRoll};
    use crate::dice::rng::NoiseRng;
    use crate::dice::spec::{
        Comparator, DiceGroup, DieId, DieKind, Direction, Expr, Mode, RollSpec, SuccessRule,
    };

    fn pool(count: u32) -> RollSpec {
        RollSpec {
            expr: Expr::Dice(DiceGroup {
                label: None,
                count,
                kind: DieKind::Numeric { min: 1, max: 10 },
                modifiers: vec![],
            }),
            direction: Direction::HighWins,
            mode: Mode::SuccessCount(SuccessConfig {
                success: SuccessRule::Numeric {
                    comp: Comparator::Gte,
                    target: 7,
                },
                required_successes: None,
                tiers: vec![],
                crit_success: None,
                crit_fail: None,
                expertise: 0,
            }),
        }
    }

    fn cfg_of(spec: &RollSpec) -> &SuccessConfig {
        match &spec.mode {
            Mode::SuccessCount(cfg) => cfg,
            _ => panic!("expected SuccessCount mode"),
        }
    }

    #[test]
    fn crit_success_adds_extra_and_counter() {
        use crate::dice::spec::{CritSuccess, CritTrigger};
        // 1d10 with target>=7, crit_success at 10 (+1 extra, +1 pos counter).
        let cfg = SuccessConfig {
            success: SuccessRule::Numeric {
                comp: Comparator::Gte,
                target: 7,
            },
            required_successes: None,
            tiers: vec![],
            crit_success: Some(CritSuccess {
                trigger: CritTrigger::AtLeast(10),
                extra_successes: 1,
                positive_counter: 1,
            }),
            crit_fail: None,
            expertise: 0,
        };
        let spec = RollSpec {
            expr: Expr::Dice(DiceGroup {
                label: None,
                count: 1,
                kind: DieKind::Numeric { min: 10, max: 10 },
                modifiers: vec![],
            }),
            direction: Direction::HighWins,
            mode: Mode::SuccessCount(cfg),
        };
        let raws = roll(&spec, &mut NoiseRng::from_seed(1));
        let out = evaluate_success(&spec, cfg_of(&spec), &raws);
        assert_eq!(out.successes, Some(2)); // base 1 + crit extra 1
        assert_eq!(out.positive_counter, 1);
        assert_eq!(out.crit_successes, 1);
        assert!(out.records[0].crit_success);
    }

    #[test]
    fn crit_fail_clamps_net_at_zero_unless_allowed() {
        use crate::dice::spec::{CritFail, CritTrigger};
        // Single die at min=max=1: base success fails, crit_fail loses 1.
        let mk = |allow: bool| RollSpec {
            expr: Expr::Dice(DiceGroup {
                label: None,
                count: 1,
                kind: DieKind::Numeric { min: 1, max: 1 },
                modifiers: vec![],
            }),
            direction: Direction::HighWins,
            mode: Mode::SuccessCount(SuccessConfig {
                success: SuccessRule::Numeric {
                    comp: Comparator::Gte,
                    target: 7,
                },
                required_successes: None,
                tiers: vec![],
                crit_success: None,
                crit_fail: Some(CritFail {
                    trigger: CritTrigger::AtLeast(1),
                    lost: 1,
                    negative_counter: 1,
                    allow_negative: allow,
                }),
                expertise: 0,
            }),
        };
        let clamped = mk(false);
        let raws1 = roll(&clamped, &mut NoiseRng::from_seed(1));
        let o1 = evaluate_success(&clamped, cfg_of(&clamped), &raws1);
        assert_eq!(o1.successes, Some(0)); // 0 base - 1 lost, clamped at 0
        assert_eq!(o1.negative_counter, 1);
        assert_eq!(o1.crit_fails, 1);

        let neg = mk(true);
        let raws2 = roll(&neg, &mut NoiseRng::from_seed(1));
        let o2 = evaluate_success(&neg, cfg_of(&neg), &raws2);
        assert_eq!(o2.successes, Some(-1)); // allow_negative
    }

    #[test]
    fn counts_dice_at_or_above_target() {
        let spec = pool(6);
        let raws = roll(&spec, &mut NoiseRng::from_seed(30));
        let out = evaluate_success(&spec, cfg_of(&spec), &raws);
        let expected = raws
            .records
            .iter()
            .filter(|r| r.kept && r.value >= 7)
            .count() as i32;
        assert_eq!(out.successes, Some(expected));
        assert_eq!(out.pass, None);
        assert_eq!(out.margin, None);
    }

    #[test]
    fn required_successes_sets_pass_and_margin() {
        let mut spec = pool(6);
        if let Mode::SuccessCount(cfg) = &mut spec.mode {
            cfg.required_successes = Some(2);
        }
        let raws = roll(&spec, &mut NoiseRng::from_seed(31));
        let out = evaluate_success(&spec, cfg_of(&spec), &raws);
        let s = out.successes.unwrap();
        assert_eq!(out.pass, Some(s >= 2));
        assert_eq!(out.margin, Some((s - 2) as i64));
    }

    /// Builds a `RawRoll` whose `records` are hand-set (not RNG-derived) so the
    /// pool-level test can pin exact per-die values. `raws.dice` is populated in
    /// lockstep (`DieKind::Numeric { min: 1, max: 6 }`) so `expertise::allocate`'s
    /// `DieId -> (min, max)` bounds lookup resolves for every record.
    fn manual_raws(values: &[i32]) -> RawRoll {
        use crate::dice::outcome::RawDie;
        use crate::dice::spec::DieKind;
        let mut raws = RawRoll::default();
        for (i, &v) in values.iter().enumerate() {
            let id = i as DieId;
            raws.dice.push(RawDie {
                id,
                kind: DieKind::Numeric { min: 1, max: 6 },
                natural: v,
            });
            raws.records.push(DieRecord {
                id,
                group_index: 0,
                natural: v,
                value: v,
                kept: true,
                exploded: false,
                rerolled_from: None,
                crit_success: false,
                crit_fail: false,
                expertise: 0,
                label: None,
                symbols: vec![],
                ordered: true,
            });
        }
        raws.next_id = values.len() as DieId;
        raws
    }

    #[test]
    fn overlapping_crit_thresholds_fold_correctly_at_pool_level() {
        use crate::dice::spec::{CritFail, CritSuccess, CritTrigger};

        // cs fires at value >= 5 (+2 extra, +1 pos counter each).
        // cf fires at value <= 10 (-1 lost, +1 neg counter each), not allow_negative.
        // die0 = 7  -> both fire (overlap region [5, 10]).
        // die1 = 15 -> only cs fires (cf requires <= 10).
        // die2 = 2  -> only cf fires (cs requires >= 5).
        let cfg = SuccessConfig {
            success: SuccessRule::Numeric {
                comp: Comparator::Gte,
                target: 6,
            },
            required_successes: None,
            tiers: vec![],
            crit_success: Some(CritSuccess {
                trigger: CritTrigger::AtLeast(5),
                extra_successes: 2,
                positive_counter: 1,
            }),
            crit_fail: Some(CritFail {
                trigger: CritTrigger::AtLeast(10),
                lost: 1,
                negative_counter: 1,
                allow_negative: false,
            }),
            expertise: 0,
        };
        let spec = RollSpec {
            expr: Expr::Dice(DiceGroup {
                label: None,
                count: 3,
                kind: DieKind::Numeric { min: 1, max: 20 },
                modifiers: vec![],
            }),
            direction: Direction::HighWins,
            mode: Mode::SuccessCount(cfg.clone()),
        };
        let raws = manual_raws(&[7, 15, 2]);
        let out = evaluate_success(&spec, &cfg, &raws);

        // base successes (>= 6): die0 (7) and die1 (15) => 2.
        // extra: die0 + die1 both hit cs => 2 + 2 = 4.
        // lost: die0 + die2 both hit cf => 1 + 1 = 2.
        // net = 2 + 4 - 2 = 4 (already >= 0, clamp is a no-op here).
        assert_eq!(out.successes, Some(4));
        assert_eq!(out.crit_successes, 2);
        assert_eq!(out.crit_fails, 2);
        assert_eq!(out.positive_counter, 2);
        assert_eq!(out.negative_counter, 2);

        assert!(out.records[0].crit_success && out.records[0].crit_fail);
        assert!(out.records[1].crit_success && !out.records[1].crit_fail);
        assert!(!out.records[2].crit_success && out.records[2].crit_fail);
    }

    #[test]
    fn successcount_tier_over_net_margin() {
        use crate::dice::spec::Tier;
        // required 1, tiers: offset 0 = "success", offset 2 = "success+2 extra".
        let tiers = vec![
            Tier {
                margin_offset: 0,
                label: Some("success".into()),
                tier_value: Some(1),
            },
            Tier {
                margin_offset: 2,
                label: Some("great".into()),
                tier_value: Some(2),
            },
        ];
        // 3 dice all min=max=10, target>=7 -> 3 successes, required 1 -> margin 2 -> "great".
        let spec = RollSpec {
            expr: Expr::Dice(DiceGroup {
                label: None,
                count: 3,
                kind: DieKind::Numeric { min: 10, max: 10 },
                modifiers: vec![],
            }),
            direction: Direction::HighWins,
            mode: Mode::SuccessCount(SuccessConfig {
                success: SuccessRule::Numeric {
                    comp: Comparator::Gte,
                    target: 7,
                },
                required_successes: Some(1),
                tiers,
                crit_success: None,
                crit_fail: None,
                expertise: 0,
            }),
        };
        let out = evaluate(&spec, &roll(&spec, &mut NoiseRng::from_seed(1)));
        assert_eq!(out.margin, Some(2));
        assert_eq!(out.tier_value, Some(2));
        assert!(out.pass.is_none(), "custom ladder reports tier, not pass");
    }

    #[test]
    fn required_successes_pass_margin_use_net_not_base() {
        use crate::dice::spec::{CritSuccess, CritTrigger};
        // Active crit config: die0/die1 = 10 both hit cs (+2 extra each), no crit_fail.
        // base successes (>= 6) = 2; net = base(2) + extra(4) = 6.
        // If pass/margin were computed over `base` instead of `net`, pass would be
        // false (2 >= 3) and margin would be -1, diverging from the correct (true, 3).
        let cfg = SuccessConfig {
            success: SuccessRule::Numeric {
                comp: Comparator::Gte,
                target: 6,
            },
            required_successes: Some(3),
            tiers: vec![],
            crit_success: Some(CritSuccess {
                trigger: CritTrigger::AtLeast(10),
                extra_successes: 2,
                positive_counter: 0,
            }),
            crit_fail: None,
            expertise: 0,
        };
        let spec = RollSpec {
            expr: Expr::Dice(DiceGroup {
                label: None,
                count: 3,
                kind: DieKind::Numeric { min: 1, max: 20 },
                modifiers: vec![],
            }),
            direction: Direction::HighWins,
            mode: Mode::SuccessCount(cfg.clone()),
        };
        let raws = manual_raws(&[10, 10, 1]);
        let out = evaluate_success(&spec, &cfg, &raws);

        assert_eq!(out.successes, Some(6));
        assert_eq!(out.pass, Some(true));
        assert_eq!(out.margin, Some(3));
    }

    #[test]
    fn expertise_lifts_success_count_through_the_full_evaluate_path() {
        // 3 dice all at face 4 (min=max forces it), target >=5, expertise 2.
        // Optimal: two dice -> 5 => 2 successes. Runs through evaluate(), not allocate directly.
        let spec = RollSpec {
            expr: Expr::Dice(DiceGroup {
                label: None,
                count: 3,
                kind: DieKind::Numeric { min: 1, max: 6 },
                modifiers: vec![],
            }),
            direction: Direction::HighWins,
            mode: Mode::SuccessCount(SuccessConfig {
                success: SuccessRule::Numeric {
                    comp: Comparator::Gte,
                    target: 5,
                },
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
        use crate::dice::spec::Tier;
        // required 1; tier at offset 1 = "great". 2 dice at 4, target >=5, expertise 2
        // -> 2 successes -> margin (2-1)=1 -> "great". Without expertise: 0 successes.
        let spec = RollSpec {
            expr: Expr::Dice(DiceGroup {
                label: None,
                count: 2,
                kind: DieKind::Numeric { min: 1, max: 6 },
                modifiers: vec![],
            }),
            direction: Direction::HighWins,
            mode: Mode::SuccessCount(SuccessConfig {
                success: SuccessRule::Numeric {
                    comp: Comparator::Gte,
                    target: 5,
                },
                required_successes: Some(1),
                tiers: vec![
                    Tier {
                        margin_offset: 0,
                        label: Some("ok".into()),
                        tier_value: Some(1),
                    },
                    Tier {
                        margin_offset: 1,
                        label: Some("great".into()),
                        tier_value: Some(2),
                    },
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
        use crate::dice::spec::{CritSuccess, CritTrigger};
        // Mirror property (design §4/§8): flip direction + mirror every face
        // (f -> min+max-f) + mirror the success target and crit threshold -> identical
        // net successes and counters.
        let hi_cfg = SuccessConfig {
            success: SuccessRule::Numeric {
                comp: Comparator::Gte,
                target: 5,
            },
            required_successes: None,
            tiers: vec![],
            crit_success: Some(CritSuccess {
                trigger: CritTrigger::AtLeast(6),
                extra_successes: 1,
                positive_counter: 1,
            }),
            crit_fail: None,
            expertise: 3,
        };
        let hi = RollSpec {
            expr: Expr::Dice(DiceGroup {
                label: None,
                count: 4,
                kind: DieKind::Numeric { min: 1, max: 6 },
                modifiers: vec![],
            }),
            direction: Direction::HighWins,
            mode: Mode::SuccessCount(hi_cfg.clone()),
        };
        // Mirror faces about (min+max)=7: 2->5, 4->3, 5->2, 1->6.
        let hi_raws = manual_raws(&[2, 4, 5, 1]);
        let hi_out = evaluate_success(&hi, cfg_of(&hi), &hi_raws);

        let lo_cfg = SuccessConfig {
            success: SuccessRule::Numeric {
                comp: Comparator::Lte,
                target: 2,
            }, // 7 - 5
            crit_success: Some(CritSuccess {
                trigger: CritTrigger::AtLeast(1),
                extra_successes: 1,
                positive_counter: 1,
            }), // 7 - 6
            ..hi_cfg
        };
        let lo = RollSpec {
            expr: Expr::Dice(DiceGroup {
                label: None,
                count: 4,
                kind: DieKind::Numeric { min: 1, max: 6 },
                modifiers: vec![],
            }),
            direction: Direction::LowWins,
            mode: Mode::SuccessCount(lo_cfg),
        };
        let lo_raws = manual_raws(&[5, 3, 2, 6]); // 7 - each hi face
        let lo_out = evaluate_success(&lo, cfg_of(&lo), &lo_raws);

        assert_eq!(hi_out.successes, lo_out.successes);
        assert_eq!(hi_out.positive_counter, lo_out.positive_counter);
        assert_eq!(hi_out.negative_counter, lo_out.negative_counter);
    }

    #[test]
    fn has_symbol_success_rule_feeds_net_successes_through_evaluate() {
        use crate::dice::spec::Face;
        // 3 dice, each a 2-face symbolic die: face 0 = "blank", face 1 = "triumph".
        // Success rule: HasSymbol("triumph"). Force naturals 1,1,0 (2 triumphs, 1 blank).
        let faces = vec![
            Face {
                value: Some(0),
                symbols: vec!["blank".into()],
            },
            Face {
                value: Some(1),
                symbols: vec!["triumph".into()],
            },
        ];
        let spec = RollSpec {
            expr: Expr::Dice(DiceGroup {
                count: 3,
                kind: DieKind::Faces {
                    faces: faces.clone(),
                },
                modifiers: vec![],
                label: None,
            }),
            direction: Direction::HighWins,
            mode: Mode::SuccessCount(SuccessConfig {
                success: SuccessRule::HasSymbol("triumph".to_string()),
                required_successes: None,
                tiers: vec![],
                crit_success: None,
                crit_fail: None,
                expertise: 0,
            }),
        };
        // Build raws directly (bypassing RNG) with naturals selecting faces 1,1,0.
        let mut raws = RawRoll::default();
        for &idx in &[1i32, 1, 0] {
            raws.push(
                DieKind::Faces {
                    faces: faces.clone(),
                },
                idx,
            );
        }
        let recs = crate::dice::eval::groups::resolve_group(
            match &spec.expr {
                Expr::Dice(g) => g,
                _ => unreachable!(),
            },
            0,
            &raws.dice.clone(),
            &mut NoiseRng::from_seed(1),
            &mut raws,
        );
        raws.records = recs;
        raws.group_spans = vec![(0, 3)];
        let out = evaluate(&spec, &raws);
        assert_eq!(
            out.successes,
            Some(2),
            "2 of 3 dice show the triumph symbol"
        );
    }

    #[test]
    fn symbol_counts_tallies_kept_dice_unconditionally() {
        use crate::dice::spec::Face;
        // Numeric success rule (not HasSymbol) — symbol_counts must STILL populate,
        // independent of which SuccessRule variant is active.
        let faces = vec![
            Face {
                value: Some(0),
                symbols: vec!["advantage".into()],
            },
            Face {
                value: Some(1),
                symbols: vec!["triumph".into(), "advantage".into()],
            },
        ];
        let spec = RollSpec {
            expr: Expr::Dice(DiceGroup {
                count: 2,
                kind: DieKind::Faces {
                    faces: faces.clone(),
                },
                modifiers: vec![],
                label: None,
            }),
            direction: Direction::HighWins,
            mode: Mode::SuccessCount(SuccessConfig {
                success: SuccessRule::Numeric {
                    comp: Comparator::Gte,
                    target: 100,
                }, // never fires
                required_successes: None,
                tiers: vec![],
                crit_success: None,
                crit_fail: None,
                expertise: 0,
            }),
        };
        let mut raws = RawRoll::default();
        raws.push(
            DieKind::Faces {
                faces: faces.clone(),
            },
            0,
        );
        raws.push(
            DieKind::Faces {
                faces: faces.clone(),
            },
            1,
        );
        let recs = crate::dice::eval::groups::resolve_group(
            match &spec.expr {
                Expr::Dice(g) => g,
                _ => unreachable!(),
            },
            0,
            &raws.dice.clone(),
            &mut NoiseRng::from_seed(1),
            &mut raws,
        );
        raws.records = recs;
        raws.group_spans = vec![(0, 2)];
        let out = evaluate(&spec, &raws);
        assert_eq!(out.symbol_counts.get("advantage"), Some(&2));
        assert_eq!(out.symbol_counts.get("triumph"), Some(&1));
        assert_eq!(
            out.successes,
            Some(0),
            "numeric rule never fires — sanity check"
        );
    }

    #[test]
    fn triumph_symbol_crit_adds_success_and_positive_counter_end_to_end() {
        use crate::dice::spec::{CritSuccess, CritTrigger, Face};
        let faces = vec![
            Face {
                value: Some(0),
                symbols: vec!["blank".into()],
            },
            Face {
                value: Some(1),
                symbols: vec!["triumph".into()],
            },
        ];
        let cfg = SuccessConfig {
            success: SuccessRule::HasSymbol("triumph".to_string()),
            required_successes: None,
            tiers: vec![],
            crit_success: Some(CritSuccess {
                trigger: CritTrigger::HasSymbol("triumph".to_string()),
                extra_successes: 1,
                positive_counter: 1,
            }),
            crit_fail: None,
            expertise: 0,
        };
        let spec = RollSpec {
            expr: Expr::Dice(DiceGroup {
                count: 1,
                kind: DieKind::Faces {
                    faces: faces.clone(),
                },
                modifiers: vec![],
                label: None,
            }),
            direction: Direction::HighWins,
            mode: Mode::SuccessCount(cfg),
        };
        let mut raws = RawRoll::default();
        raws.push(
            DieKind::Faces {
                faces: faces.clone(),
            },
            1,
        ); // draws the "triumph" face
        let recs = crate::dice::eval::groups::resolve_group(
            match &spec.expr {
                Expr::Dice(g) => g,
                _ => unreachable!(),
            },
            0,
            &raws.dice.clone(),
            &mut NoiseRng::from_seed(1),
            &mut raws,
        );
        raws.records = recs;
        raws.group_spans = vec![(0, 1)];
        let out = evaluate(&spec, &raws);
        // base success (HasSymbol) = 1, crit extra = 1 -> net 2.
        assert_eq!(out.successes, Some(2));
        assert_eq!(out.positive_counter, 1);
        assert_eq!(out.crit_successes, 1);
    }
}
