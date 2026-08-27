use super::*;
use crate::dice::spec::DieKind;

#[test]
fn raw_roll_allocates_monotonic_ids() {
    let mut r = RawRoll::default();
    let a = r.push(DieKind::Numeric { min: 1, max: 6 }, 4);
    let b = r.push(DieKind::Numeric { min: 1, max: 6 }, 2);
    assert_eq!(a, 0);
    assert_eq!(b, 1);
    assert_eq!(r.dice.len(), 2);
    assert_eq!(r.dice[0].natural, 4);
}

#[test]
#[should_panic(expected = "next_id overflowed")]
fn raw_roll_push_debug_asserts_on_next_id_overflow() {
    // Debug/test builds have debug_assertions on, so the guard's
    // debug_assert! fires before the saturating fallback would run;
    // this proves the guard actually triggers at the boundary rather
    // than silently wrapping.
    let mut r = RawRoll {
        next_id: DieId::MAX,
        ..Default::default()
    };
    r.push(DieKind::Numeric { min: 1, max: 6 }, 1);
}

fn labeled_record(label: &str, value: i32, kept: bool) -> DieRecord {
    labeled_record_ordered(label, value, kept, true)
}

fn labeled_record_ordered(label: &str, value: i32, kept: bool, ordered: bool) -> DieRecord {
    DieRecord {
        id: 0,
        group_index: 0,
        natural: value,
        value,
        kept,
        exploded: false,
        rerolled_from: None,
        crit_success: false,
        crit_fail: false,
        expertise: 0,
        label: Some(label.to_string()),
        symbols: vec![],
        ordered,
    }
}

#[test]
fn by_label_collects_only_matching_records() {
    let out = RollOutcome {
        total: 0,
        records: vec![
            labeled_record("Hope", 5, true),
            labeled_record("Fear", 3, true),
            labeled_record("Hope", 2, true),
        ],
        successes: None,
        pass: None,
        margin: None,
        tier_label: None,
        tier_value: None,
        crit_successes: 0,
        crit_fails: 0,
        positive_counter: 0,
        negative_counter: 0,
        symbol_counts: Default::default(),
        labeled_consts: vec![],
    };
    let hope: Vec<i32> = out.by_label("Hope").iter().map(|r| r.value).collect();
    assert_eq!(hope, vec![5, 2]);
    assert!(out.by_label("Nope").is_empty());
}

#[test]
fn compare_labels_orders_by_sum_of_kept_values() {
    use std::cmp::Ordering;
    let out = RollOutcome {
        total: 0,
        records: vec![
            labeled_record("Hope", 5, true),
            labeled_record("Hope", 1, false), // dropped: excluded from the sum
            labeled_record("Fear", 3, true),
        ],
        successes: None,
        pass: None,
        margin: None,
        tier_label: None,
        tier_value: None,
        crit_successes: 0,
        crit_fails: 0,
        positive_counter: 0,
        negative_counter: 0,
        symbol_counts: Default::default(),
        labeled_consts: vec![],
    };
    // Hope kept-sum = 5, Fear kept-sum = 3 -> Hope > Fear.
    assert_eq!(out.compare_labels("Hope", "Fear"), Some(Ordering::Greater));
    assert_eq!(out.compare_labels("Fear", "Hope"), Some(Ordering::Less));
    assert_eq!(out.compare_labels("Hope", "Missing"), None);
}

#[test]
fn compare_labels_returns_none_when_either_label_is_unordered() {
    // "Fear" is a symbolic (unordered) label — its records carry `ordered: false`,
    // the exact Daggerheart Hope/Fear headline case: an
    // unordered label has no well-defined sum, so `compare_labels` must return
    // `None`, not `Some(0)` from summing derived-0 `value`s.
    let out = RollOutcome {
        total: 0,
        records: vec![
            labeled_record_ordered("Hope", 5, true, true),
            labeled_record_ordered("Fear", 0, true, false),
        ],
        successes: None,
        pass: None,
        margin: None,
        tier_label: None,
        tier_value: None,
        crit_successes: 0,
        crit_fails: 0,
        positive_counter: 0,
        negative_counter: 0,
        symbol_counts: Default::default(),
        labeled_consts: vec![],
    };
    assert_eq!(out.compare_labels("Hope", "Fear"), None);
    assert_eq!(out.compare_labels("Fear", "Hope"), None);
}

#[test]
fn compare_labels_returns_none_when_label_mixes_ordered_and_unordered_groups() {
    // A label spanning two DiceGroups, one ordered and one unordered: a partial
    // pool with any unordered member has no well-defined sum either.
    let out = RollOutcome {
        total: 0,
        records: vec![
            labeled_record_ordered("Mixed", 5, true, true),
            labeled_record_ordered("Mixed", 0, true, false),
        ],
        successes: None,
        pass: None,
        margin: None,
        tier_label: None,
        tier_value: None,
        crit_successes: 0,
        crit_fails: 0,
        positive_counter: 0,
        negative_counter: 0,
        symbol_counts: Default::default(),
        labeled_consts: vec![],
    };
    assert_eq!(out.compare_labels("Mixed", "Mixed"), None);
}

#[test]
fn roll_outcome_missing_defaulted_keys_deserializes() {
    // Pins `#[serde(default)]` on labeled_consts + symbol_counts against a
    // stored RollOutcome JSON shape missing both fields.
    let j = serde_json::json!({
        "total": 7, "records": [], "successes": null, "pass": null,
        "margin": null, "tier_label": null, "tier_value": null,
        "crit_successes": 0, "crit_fails": 0,
        "positive_counter": 0, "negative_counter": 0
    });
    let out: super::RollOutcome = serde_json::from_value(j).unwrap();
    assert!(out.labeled_consts.is_empty());
    assert!(out.symbol_counts.is_empty());
}
