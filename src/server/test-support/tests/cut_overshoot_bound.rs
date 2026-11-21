//! `assert_cut_overshoot_within_ceiling` fails a cut that lands more than `CUT_OVERSHOOT_MAX` past
//! its derived bound, and passes one at the ceiling or before the bound.

use std::time::Duration;

use shadowcat_test_support::measure::{assert_cut_overshoot_within_ceiling, CUT_OVERSHOOT_MAX};

#[test]
fn a_cut_at_the_ceiling_or_before_its_derived_bound_passes() {
    let derived = Duration::from_millis(1250);
    assert_cut_overshoot_within_ceiling("at the ceiling", derived + CUT_OVERSHOOT_MAX, derived);
    assert_cut_overshoot_within_ceiling("before the bound", Duration::from_millis(500), derived);
}

#[test]
#[should_panic(expected = "past the derived")]
fn a_cut_one_millisecond_past_the_ceiling_fails() {
    let derived = Duration::from_millis(1250);
    assert_cut_overshoot_within_ceiling(
        "past the ceiling",
        derived + CUT_OVERSHOOT_MAX + Duration::from_millis(1),
        derived,
    );
}
