use crate::dice::spec::{Direction, Tier};

/// Classification of an oriented margin (higher = better). Mutually exclusive
/// outputs: a roll reports EITHER a `pass` (default 2-rung ladder) OR a `tier`
/// (custom ladder), never both.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Classification {
    pub pass: Option<bool>,
    pub tier_label: Option<String>,
    pub tier_value: Option<i32>,
}

/// Orient a scalar-vs-reference difference so "better" is always more positive.
/// HighWins: higher scalar is better. LowWins (roll-under): lower scalar is better.
pub fn oriented_margin(direction: Direction, scalar: i64, reference: i64) -> i64 {
    match direction {
        Direction::HighWins => scalar - reference,
        Direction::LowWins => reference - scalar,
    }
}

/// Classify `margin` against `tiers`. Empty ladder => default 2-rung pass/fail
/// (`pass = margin >= 0`). Non-empty => the highest rung with `margin_offset <=
/// margin`; if none match (margin below the floor), fail closed to the lowest
/// rung. Order-independent (no sorted precondition). Well-formed ladders use
/// unique `margin_offset`s; a duplicate offset ties on `max_by_key`/
/// `min_by_key`'s last-element semantics, so which duplicate wins depends on
/// caller-supplied vec order.
pub fn classify(margin: i64, tiers: &[Tier]) -> Classification {
    if tiers.is_empty() {
        return Classification {
            pass: Some(margin >= 0),
            tier_label: None,
            tier_value: None,
        };
    }
    let chosen = tiers
        .iter()
        .filter(|t| (t.margin_offset as i64) <= margin)
        .max_by_key(|t| t.margin_offset)
        .or_else(|| tiers.iter().min_by_key(|t| t.margin_offset))
        .expect("tiers is non-empty");
    Classification {
        pass: None,
        tier_label: chosen.label.clone(),
        tier_value: chosen.tier_value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_ladder_is_pass_fail_at_zero_margin() {
        assert_eq!(classify(0, &[]).pass, Some(true)); // margin >= 0 -> pass
        assert_eq!(classify(-1, &[]).pass, Some(false)); // margin < 0 -> fail
        assert!(classify(3, &[]).tier_label.is_none());
    }

    #[test]
    fn ladder_picks_highest_rung_at_or_below_margin() {
        let tiers = vec![
            Tier {
                margin_offset: -10,
                label: Some("crit-fail".into()),
                tier_value: Some(0),
            },
            Tier {
                margin_offset: 0,
                label: Some("success".into()),
                tier_value: Some(2),
            },
            Tier {
                margin_offset: 10,
                label: Some("crit-success".into()),
                tier_value: Some(3),
            },
        ];
        assert_eq!(classify(5, &tiers).tier_value, Some(2));
        assert_eq!(classify(10, &tiers).tier_value, Some(3));
        assert_eq!(classify(-3, &tiers).tier_value, Some(0));
        assert!(
            classify(5, &tiers).pass.is_none(),
            "ladder result reports tier, not pass"
        );
    }

    #[test]
    fn ladder_below_floor_fails_closed_to_lowest_rung() {
        let tiers = vec![
            Tier {
                margin_offset: 0,
                label: Some("ok".into()),
                tier_value: Some(1),
            },
            Tier {
                margin_offset: 5,
                label: Some("great".into()),
                tier_value: Some(2),
            },
        ];
        // margin below every offset -> lowest rung (min margin_offset), never "no tier".
        assert_eq!(classify(-100, &tiers).tier_value, Some(1));
    }

    #[test]
    fn ladder_order_independent() {
        let a = vec![
            Tier {
                margin_offset: 0,
                label: None,
                tier_value: Some(1),
            },
            Tier {
                margin_offset: 5,
                label: None,
                tier_value: Some(2),
            },
        ];
        let b = vec![a[1].clone(), a[0].clone()];
        assert_eq!(classify(6, &a).tier_value, classify(6, &b).tier_value);
    }

    #[test]
    fn oriented_margin_flips_with_direction() {
        assert_eq!(oriented_margin(Direction::HighWins, 15, 10), 5);
        assert_eq!(oriented_margin(Direction::LowWins, 8, 10), 2); // rolling under: lower is better
    }
}
