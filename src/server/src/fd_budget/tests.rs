//! The clamp is a pure function of the soft limit and the desired caps; the raise touches the
//! process and is only checked for what it guarantees.

use super::*;

/// The default caps of both stages.
fn defaults() -> (Limits, ConnLimits) {
    (Limits::default(), ConnLimits::default())
}

#[test]
fn a_limit_that_backs_both_caps_changes_nothing() {
    let (pre, live) = defaults();
    let enough = FD_RESERVE + pre.global_cap as u64 + live.max_live as u64;
    let (fitted_pre, fitted_live, clamp) = fit_to_descriptor_budget(Some(enough), pre, live);
    assert!(clamp.is_none());
    assert_eq!(fitted_pre.global_cap, pre.global_cap);
    assert_eq!(fitted_live.max_live, live.max_live);
}

#[test]
fn an_unbounded_limit_changes_nothing() {
    let (pre, live) = defaults();
    let (fitted_pre, fitted_live, clamp) = fit_to_descriptor_budget(None, pre, live);
    assert!(clamp.is_none());
    assert_eq!(fitted_pre.global_cap, pre.global_cap);
    assert_eq!(fitted_live.max_live, live.max_live);
}

#[test]
fn a_hard_limit_below_the_caps_fits_their_sum_inside_the_budget_keeping_the_ratio() {
    let (pre, live) = defaults();
    // The common Unix default soft limit.
    let (fitted_pre, fitted_live, clamp) = fit_to_descriptor_budget(Some(1024), pre, live);
    let clamp = clamp.expect("1024 descriptors cannot back 3072 connections");
    let budget = 1024 - FD_RESERVE;
    assert_eq!(clamp.budget, budget);
    assert!(
        (fitted_pre.global_cap + fitted_live.max_live) as u64 <= budget,
        "the fitted caps promise more descriptors than the budget: {} + {} > {budget}",
        fitted_pre.global_cap,
        fitted_live.max_live
    );
    assert!(
        fitted_live.max_live > fitted_pre.global_cap,
        "the live stage keeps the larger share"
    );
    assert_eq!(
        (clamp.pre_handoff_before, clamp.live_before),
        (pre.global_cap, live.max_live)
    );
    assert_eq!(
        (clamp.pre_handoff_after, clamp.live_after),
        (fitted_pre.global_cap, fitted_live.max_live)
    );
    assert!(!clamp.floors_exceed_budget);
}

#[test]
fn a_limit_at_or_below_the_reserve_gets_the_floors_and_says_so() {
    let (pre, live) = defaults();
    for soft in [FD_RESERVE, FD_RESERVE / 2, 0] {
        let (fitted_pre, fitted_live, clamp) = fit_to_descriptor_budget(Some(soft), pre, live);
        let clamp = clamp.expect("a limit inside the reserve must clamp");
        assert_eq!(fitted_pre.global_cap, MIN_PRE_HANDOFF_CAP, "soft {soft}");
        assert_eq!(fitted_live.max_live, MIN_LIVE_CAP, "soft {soft}");
        assert!(clamp.floors_exceed_budget, "soft {soft}");
        assert_eq!(clamp.budget, 0, "soft {soft}");
    }
}

#[test]
fn per_peer_caps_never_exceed_their_global_cap() {
    let (pre, live) = defaults();
    let (fitted_pre, fitted_live, _) = fit_to_descriptor_budget(Some(FD_RESERVE + 20), pre, live);
    assert!(fitted_pre.per_ip_cap as usize <= fitted_pre.global_cap);
    assert!(fitted_live.max_live_per_peer as usize <= fitted_live.max_live);
}

#[test]
fn a_clamp_never_raises_a_cap_above_what_was_asked_for() {
    let (mut pre, mut live) = defaults();
    pre.global_cap = 8;
    live.max_live = 10;
    let (fitted_pre, fitted_live, _) = fit_to_descriptor_budget(Some(FD_RESERVE + 4), pre, live);
    assert!(fitted_pre.global_cap <= 8);
    assert!(fitted_live.max_live <= 10);
}

#[test]
fn the_raise_target_is_the_hard_limit_bounded_by_the_ceiling_and_never_below_the_soft_limit() {
    assert_eq!(raise_target(1024, 65_536, None), 65_536);
    assert_eq!(raise_target(1024, u64::MAX, Some(10_240)), 10_240);
    assert_eq!(raise_target(20_000, 65_536, Some(10_240)), 20_000);
    assert_eq!(raise_target(1024, 1024, None), 1024);
}

#[cfg(unix)]
#[test]
fn raising_reads_back_a_limit_that_is_at_least_the_one_it_started_from() {
    let first = raise_nofile_limit();
    let second = raise_nofile_limit();
    assert_eq!(first.soft, second.soft, "the raise is idempotent");
    if let (Some(before), Some(after)) = (first.before, first.soft) {
        assert!(
            after >= before,
            "the raise lowered the limit: {before} -> {after}"
        );
    }
    let (soft, hard) = read_nofile().expect("getrlimit must work in a test process");
    assert!(soft <= hard);
    if first.problem.is_none() {
        let ceiling = platform_ceiling();
        let expected = raise_target(first.before.unwrap_or(soft), hard, ceiling);
        assert_eq!(
            soft, expected,
            "an unrefused raise must reach the hard limit (or the platform ceiling)"
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn the_macos_ceiling_is_queried_and_positive() {
    let ceiling = platform_ceiling().expect("kern.maxfilesperproc must be readable on macOS");
    assert!(
        ceiling >= 256,
        "implausible kern.maxfilesperproc: {ceiling}"
    );
}
