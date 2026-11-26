use crate::dice::eval::{evaluate, roll};
use crate::dice::notation::{self, ModeKind, ParseContext};
use crate::dice::outcome::{DieRecord, RollOutcome};
use crate::dice::rng::NoiseRng;
use crate::dice::spec::Direction;

fn ctx(direction: Direction) -> ParseContext {
    ParseContext {
        mode: ModeKind::Total,
        direction,
    }
}

/// Rolls `formula` under `direction` and returns the outcome with the untouched rolled records.
fn run(formula: &str, direction: Direction, seed: u64) -> (RollOutcome, Vec<DieRecord>) {
    let spec = notation::parse(formula, ctx(direction)).unwrap();
    let raws = roll(&spec, &mut NoiseRng::from_seed(seed));
    (evaluate(&spec, &raws), raws.records)
}

/// Sum of the rolled values of group `gi`, read from the pre-evaluate records.
fn group_total(records: &[DieRecord], gi: usize) -> i64 {
    records
        .iter()
        .filter(|r| r.group_index == gi)
        .map(|r| r.value as i64)
        .sum()
}

/// Checks branch `winner` kept every die and branch `1 - winner` discarded all of them,
/// labelled with its own branch index.
fn assert_branch_split(out: &RollOutcome, winner: usize) {
    for r in &out.records {
        if r.group_index == winner {
            assert!(r.kept, "winning branch die was discarded: {r:?}");
            assert_eq!(r.best_of_branch, None);
        } else {
            assert!(!r.kept, "losing branch die stayed kept: {r:?}");
            assert_eq!(r.best_of_branch, Some(r.group_index as u32));
        }
    }
}

#[test]
fn high_wins_keeps_the_greater_branch_total() {
    let mut differing = 0;
    for seed in 0..40 {
        let (out, recs) = run("best(4d6, 4d6)", Direction::HighWins, seed);
        let (a, b) = (group_total(&recs, 0), group_total(&recs, 1));
        assert_eq!(out.total, a.max(b), "seed {seed}");
        if a != b {
            differing += 1;
            assert_branch_split(&out, if a > b { 0 } else { 1 });
        }
    }
    assert!(differing > 10, "test is vacuous: branches rarely differ");
}

#[test]
fn low_wins_keeps_the_smaller_branch_total() {
    let mut differing = 0;
    for seed in 0..40 {
        let (out, recs) = run("best(4d6, 4d6)", Direction::LowWins, seed);
        let (a, b) = (group_total(&recs, 0), group_total(&recs, 1));
        assert_eq!(out.total, a.min(b), "seed {seed}");
        if a != b {
            differing += 1;
            assert_branch_split(&out, if a < b { 0 } else { 1 });
        }
    }
    assert!(differing > 10, "test is vacuous: branches rarely differ");
}

#[test]
fn a_tie_keeps_the_leftmost_branch() {
    // `1d1` always rolls 1, so both branches tie under either direction.
    for direction in [Direction::HighWins, Direction::LowWins] {
        let (out, _) = run("best(1d1, 1d1, 1d1)", direction, 3);
        assert_eq!(out.total, 1);
        let kept: Vec<usize> = out
            .records
            .iter()
            .filter(|r| r.kept)
            .map(|r| r.group_index)
            .collect();
        assert_eq!(kept, vec![0]);
        let losers: Vec<_> = out
            .records
            .iter()
            .filter(|r| !r.kept)
            .map(|r| (r.group_index, r.best_of_branch))
            .collect();
        assert_eq!(losers, vec![(1, Some(1)), (2, Some(2))]);
    }
}

#[test]
fn best_of_folds_into_surrounding_arithmetic() {
    for seed in 0..10 {
        let (out, recs) = run("best(1d6, 1d6) + 3", Direction::HighWins, seed);
        let top = group_total(&recs, 0).max(group_total(&recs, 1));
        assert_eq!(out.total, top + 3, "seed {seed}");
    }
}

#[test]
fn evaluate_leaves_the_stored_roll_untouched_and_repeats_exactly() {
    let spec = notation::parse("best(4d6, 4d6)", ctx(Direction::HighWins)).unwrap();
    let raws = roll(&spec, &mut NoiseRng::from_seed(5));
    let first = evaluate(&spec, &raws);
    assert!(raws.records.iter().all(|r| r.kept));
    assert!(raws.records.iter().all(|r| r.best_of_branch.is_none()));
    assert_eq!(first, evaluate(&spec, &raws));
}

#[test]
fn a_branch_with_no_dice_competes_on_its_constant() {
    let (high, _) = run("best(5, 1d1)", Direction::HighWins, 1);
    assert_eq!(high.total, 5);
    assert!(high.records.iter().all(|r| !r.kept));
    let (low, _) = run("best(5, 1d1)", Direction::LowWins, 1);
    assert_eq!(low.total, 1);
    assert!(low.records.iter().all(|r| r.kept));
}

#[test]
fn only_the_winning_branch_contributes_labeled_constants() {
    let (high, _) = run("best(5[atk], 2[def])", Direction::HighWins, 1);
    assert_eq!(high.labeled_consts.len(), 1);
    assert_eq!(high.labeled_consts[0].label.as_deref(), Some("atk"));
    let (low, _) = run("best(5[atk], 2[def])", Direction::LowWins, 1);
    assert_eq!(low.labeled_consts.len(), 1);
    assert_eq!(low.labeled_consts[0].label.as_deref(), Some("def"));
}

#[test]
fn the_outermost_losing_branch_labels_a_die_discarded_twice() {
    // Outer branch 0 (the nested best, total 1) loses to the constant 5. Its inner loser
    // (group 1, inner index 1) is discarded again by the outer loss, and the outer index wins.
    let (out, _) = run("best(best(1d1, 1d1), 5)", Direction::HighWins, 1);
    assert_eq!(out.total, 5);
    for r in &out.records {
        assert!(!r.kept);
        assert_eq!(r.best_of_branch, Some(0), "group {}", r.group_index);
    }
}

#[test]
fn an_inner_loser_inside_a_winning_outer_branch_keeps_its_own_index() {
    let (out, _) = run("best(best(1d1, 1d1), 0)", Direction::HighWins, 1);
    assert_eq!(out.total, 1);
    let marks: Vec<_> = out
        .records
        .iter()
        .map(|r| (r.group_index, r.kept, r.best_of_branch))
        .collect();
    assert_eq!(marks, vec![(0, true, None), (1, false, Some(1))]);
}
