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
    let enough = descriptors_needed(&pre, &live);
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
    let budget = 1024 - descriptors_reserved();
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
    for soft in [descriptors_reserved(), FD_RESERVE, FD_RESERVE / 2, 0] {
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
    let (fitted_pre, fitted_live, _) =
        fit_to_descriptor_budget(Some(descriptors_reserved() + 20), pre, live);
    assert!(fitted_pre.per_ip_cap as usize <= fitted_pre.global_cap);
    assert!(fitted_live.max_live_per_peer as usize <= fitted_live.max_live);
}

#[test]
fn a_clamp_never_raises_a_cap_above_what_was_asked_for() {
    let (mut pre, mut live) = defaults();
    pre.global_cap = 8;
    live.max_live = 10;
    let (fitted_pre, fitted_live, _) =
        fit_to_descriptor_budget(Some(descriptors_reserved() + 4), pre, live);
    assert!(fitted_pre.global_cap <= 8);
    assert!(fitted_live.max_live <= 10);
}

/// The default caps' sum, which the reserved terms are added to.
fn default_caps_sum() -> u64 {
    let (pre, live) = defaults();
    pre.global_cap as u64 + live.max_live as u64
}

#[test]
fn the_need_is_the_caps_plus_every_reserved_term() {
    let (pre, live) = defaults();
    assert_eq!(
        descriptors_reserved(),
        FD_RESERVE + HANDOFF_QUEUE_CAP as u64 + STREAMED_FILE_PERMITS as u64
    );
    assert_eq!(
        descriptors_needed(&pre, &live),
        default_caps_sum() + descriptors_reserved()
    );
}

#[test]
fn the_fit_changes_nothing_at_exactly_the_need_and_clamps_one_below_it() {
    let (pre, live) = defaults();
    let needed = descriptors_needed(&pre, &live);
    assert!(fit_to_descriptor_budget(Some(needed), pre, live)
        .2
        .is_none());
    assert!(fit_to_descriptor_budget(Some(needed - 1), pre, live)
        .2
        .is_some());
}

#[test]
fn the_hand_off_queue_is_subtracted_from_the_budget() {
    let (pre, live) = defaults();
    // Backs every term except the queue: only the queue's subtraction can make this clamp.
    let soft = default_caps_sum() + FD_RESERVE + STREAMED_FILE_PERMITS as u64;
    let clamp = fit_to_descriptor_budget(Some(soft), pre, live).2;
    let clamp = clamp.expect("a limit short by the hand-off queue must clamp");
    assert_eq!(clamp.budget, soft - descriptors_reserved());
}

#[test]
fn the_streamed_file_permits_are_subtracted_from_the_budget() {
    let (pre, live) = defaults();
    // Backs every term except the streamed-file permits.
    let soft = default_caps_sum() + FD_RESERVE + HANDOFF_QUEUE_CAP as u64;
    let clamp = fit_to_descriptor_budget(Some(soft), pre, live).2;
    assert!(
        clamp.is_some(),
        "a limit short by the streamed-file permits must clamp"
    );
}

#[test]
fn the_raise_target_is_the_need_bounded_by_the_hard_limit_and_ceiling_and_never_below_the_soft() {
    // The soft limit already covers the need: unchanged, even with a far higher hard limit.
    assert_eq!(raise_target(4096, 65_536, None, 3_000), 4096);
    // The need is above the soft limit and below the hard limit: exactly the need.
    assert_eq!(raise_target(1024, 65_536, None, 3_000), 3_000);
    // The need is above the hard limit: the hard limit.
    assert_eq!(raise_target(1024, 2_048, None, 3_000), 2_048);
    // The ceiling is below the need: the ceiling.
    assert_eq!(raise_target(1024, 65_536, Some(2_500), 3_000), 2_500);
    // A ceiling below the current soft limit never lowers it.
    assert_eq!(raise_target(20_000, 65_536, Some(10_240), 30_000), 20_000);
    // An unlimited hard limit asks for the need, not for unlimited.
    assert_eq!(raise_target(1024, u64::MAX, None, 3_000), 3_000);
    assert_eq!(raise_target(1024, u64::MAX, Some(10_240), 3_000), 3_000);
}

/// The `(soft, hard)` pairs a write closure was called with.
type Calls = std::sync::Mutex<Vec<(u64, u64)>>;

#[test]
fn a_refused_raise_reports_the_unchanged_limit_and_the_fit_then_clamps_to_it() {
    let (pre, live) = defaults();
    let needed = descriptors_needed(&pre, &live);
    let calls = Calls::default();
    let limit = raise_with(
        || Some((1024, 1 << 20)),
        |soft, hard| {
            calls.lock().unwrap().push((soft, hard));
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        },
        None,
        needed,
        u64::MAX,
    );
    assert_eq!(*calls.lock().unwrap(), vec![(needed, 1 << 20)]);
    assert!(limit.problem.is_some());
    assert_eq!(limit.soft, Some(1024), "the read-back limit, not the ask");
    let clamp = fit_to_descriptor_budget(limit.soft, pre, live).2;
    assert!(clamp.is_some(), "a refused raise must constrain the caps");
}

#[test]
fn a_limit_that_already_covers_the_need_is_not_written() {
    let calls = Calls::default();
    let limit = raise_with(
        || Some((8192, 65_536)),
        |soft, hard| {
            calls.lock().unwrap().push((soft, hard));
            Ok(())
        },
        None,
        3_000,
        u64::MAX,
    );
    assert!(calls.lock().unwrap().is_empty());
    assert_eq!(limit.soft, Some(8192));
    assert!(limit.problem.is_none());
}

#[test]
fn a_raise_asks_for_the_need_and_keeps_the_hard_limit() {
    let state = std::sync::Mutex::new((1024u64, 65_536u64));
    let limit = raise_with(
        || Some(*state.lock().unwrap()),
        |soft, hard| {
            *state.lock().unwrap() = (soft, hard);
            Ok(())
        },
        None,
        3_000,
        u64::MAX,
    );
    assert_eq!(*state.lock().unwrap(), (3_000, 65_536));
    assert_eq!((limit.before, limit.soft), (Some(1024), Some(3_000)));
    assert!(limit.problem.is_none());
}

#[test]
fn an_unlimited_soft_limit_reports_no_limit() {
    let limit = raise_with(
        || Some((u64::MAX, u64::MAX)),
        |_, _| Ok(()),
        None,
        3_000,
        u64::MAX,
    );
    assert_eq!((limit.before, limit.soft), (None, None));
}

#[test]
fn an_unreadable_limit_reports_a_problem_and_no_soft_limit() {
    let limit = raise_with(|| None, |_, _| Ok(()), None, 3_000, u64::MAX);
    assert_eq!(limit.soft, None);
    assert!(limit.problem.is_some());
}

#[cfg(unix)]
#[test]
fn raising_reads_back_a_limit_that_is_at_least_the_one_it_started_from() {
    let (pre, live) = defaults();
    let needed = descriptors_needed(&pre, &live);
    let first = raise_nofile_limit(needed);
    let second = raise_nofile_limit(needed);
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
        let expected = raise_target(first.before.unwrap_or(soft), hard, ceiling, needed);
        assert_eq!(
            soft, expected,
            "an unrefused raise must reach the need (or the hard limit / platform ceiling)"
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
