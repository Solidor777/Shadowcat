use crate::dice::outcome::{RawRoll, RollOutcome};
use crate::dice::spec::RollSpec;

/// Pool aggregation: count kept dice satisfying the (required) per-die success rule.
/// Optional `required_successes` yields overall pass + net margin (net hits). Pools
/// ALL kept records across every group — SuccessCount mode ignores the AST arithmetic.
pub fn evaluate_success(spec: &RollSpec, raws: &RawRoll) -> RollOutcome {
    let successes: i32 = match &spec.success {
        Some(rule) => raws
            .records
            .iter()
            .filter(|r| r.kept && rule.comp.test(r.value, rule.target))
            .count() as i32,
        None => 0,
    };
    let total: i64 = raws
        .records
        .iter()
        .filter(|r| r.kept)
        .map(|r| r.value as i64)
        .sum();
    let (pass, net_margin) = match spec.required_successes {
        Some(req) => (Some(successes >= req), Some(successes - req)),
        None => (None, None),
    };
    RollOutcome {
        total,
        records: raws.records.clone(),
        successes: Some(successes),
        pass,
        net_margin,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dice::eval::roll;
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
    fn counts_dice_at_or_above_target() {
        let spec = pool(6);
        let raws = roll(&spec, &mut NoiseRng::from_seed(30));
        let out = evaluate_success(&spec, &raws);
        let expected = raws
            .records
            .iter()
            .filter(|r| r.kept && r.value >= 7)
            .count() as i32;
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
