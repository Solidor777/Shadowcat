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
