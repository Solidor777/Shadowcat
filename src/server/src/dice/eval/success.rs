use crate::dice::outcome::{RawRoll, RollOutcome};
use crate::dice::spec::{RollSpec, SuccessConfig};

/// Pool aggregation: count kept dice satisfying `cfg.success`. Optional
/// `required_successes` yields overall pass + margin (net hits). Pools ALL kept
/// records across every group — SuccessCount mode ignores the AST arithmetic.
/// `spec` is currently unused beyond keeping a uniform `evaluate` dispatch signature.
pub fn evaluate_success(_spec: &RollSpec, cfg: &SuccessConfig, raws: &RawRoll) -> RollOutcome {
    let successes = raws
        .records
        .iter()
        .filter(|r| r.kept && cfg.success.comp.test(r.value, cfg.success.target))
        .count() as i32;
    let total: i64 = raws
        .records
        .iter()
        .filter(|r| r.kept)
        .map(|r| r.value as i64)
        .sum();
    let (pass, margin) = match cfg.required_successes {
        Some(req) => (Some(successes >= req), Some((successes - req) as i64)),
        None => (None, None),
    };
    RollOutcome {
        total,
        records: raws.records.clone(),
        successes: Some(successes),
        pass,
        margin,
        tier_label: None,
        tier_value: None,
        crit_successes: 0,
        crit_fails: 0,
        positive_counter: 0,
        negative_counter: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dice::eval::roll;
    use crate::dice::rng::NoiseRng;
    use crate::dice::spec::{
        Comparator, DiceGroup, DieKind, Direction, Expr, Mode, RollSpec, SuccessRule,
    };

    fn pool(count: u32) -> RollSpec {
        RollSpec {
            expr: Expr::Dice(DiceGroup {
                count,
                kind: DieKind::Numeric { min: 1, max: 10 },
                modifiers: vec![],
            }),
            direction: Direction::HighWins,
            mode: Mode::SuccessCount(SuccessConfig {
                success: SuccessRule {
                    comp: Comparator::Gte,
                    target: 7,
                },
                required_successes: None,
                tiers: vec![],
                crit_success: None,
                crit_fail: None,
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
}
