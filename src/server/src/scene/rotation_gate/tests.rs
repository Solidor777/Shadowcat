use super::*;

/// Every `(locked, rule, standing)` combination and the refusal it must produce.
#[test]
fn rotation_refusal_matrix() {
    use eng::FacingTurnRule::{Free, OwnTurn};
    use RotationRefusal::{Locked, NotYourTurn};
    use TurnStanding::{AwaitsTurn, HoldsTurn, Unconstrained};
    let cases = [
        (false, Free, Unconstrained, None),
        (false, Free, HoldsTurn, None),
        (false, Free, AwaitsTurn, None),
        (false, OwnTurn, Unconstrained, None),
        (false, OwnTurn, HoldsTurn, None),
        (false, OwnTurn, AwaitsTurn, Some(NotYourTurn)),
        (true, Free, Unconstrained, Some(Locked)),
        (true, Free, AwaitsTurn, Some(Locked)),
        (true, OwnTurn, HoldsTurn, Some(Locked)),
        (true, OwnTurn, AwaitsTurn, Some(Locked)),
    ];
    for (locked, rule, standing, want) in cases {
        assert_eq!(
            rotation_refusal(locked, rule, standing),
            want,
            "locked={locked} rule={rule:?} standing={standing:?}"
        );
    }
}

/// The lock half is the single decision: `rotation_refusal` under a rule that never refuses on
/// turn agrees with `lock_refusal` for both lock values, so a drift in either shows here.
#[test]
fn the_lock_half_and_the_full_decision_agree() {
    for locked in [false, true] {
        assert_eq!(
            rotation_refusal(
                locked,
                eng::FacingTurnRule::Free,
                TurnStanding::Unconstrained
            ),
            lock_refusal(locked)
        );
    }
}
