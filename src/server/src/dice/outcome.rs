use serde::{Deserialize, Serialize};

use crate::dice::spec::{DieId, DieKind, RollSpec};

/// A single die's natural (RNG) result — the only nondeterministic artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawDie {
    pub id: DieId,
    pub kind: DieKind,
    pub natural: i32,
}

/// The RNG output for a whole roll. `dice` is the natural-face log; `records` is the
/// post-pipeline per-die result (filled by `roll`/`recalculate`); `next_id` hands out
/// stable ids so reroll/add ops never collide with existing dice.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawRoll {
    pub dice: Vec<RawDie>,
    pub records: Vec<DieRecord>,
    pub next_id: DieId,
    /// Per-`Dice`-AST-node `(start, base_count)` into `dice`, in AST left-to-right
    /// order. Covers ONLY the base naturals rolled for that group (explosion/
    /// penetrate children pushed later live past the span). `recalculate` uses this
    /// to reconstruct each group's pre-pipeline naturals exactly, excluding any
    /// derived dice a prior roll/recalc appended.
    pub group_spans: Vec<(usize, usize)>,
}

impl RawRoll {
    /// Append a natural die with a fresh id; returns that id.
    pub fn push(&mut self, kind: DieKind, natural: i32) -> DieId {
        let id = self.next_id;
        self.next_id += 1;
        self.dice.push(RawDie { id, kind, natural });
        id
    }
}

/// A die's contribution after the pipeline. `value` = post-modifier face; `kept` =
/// survived keep/drop. `rerolled_from` holds the immediately-preceding value for a
/// chained (non-`once`) reroll — NOT necessarily the original natural roll; see
/// `natural` for that. Penetrate-produced records may fall outside `[min, max]` by
/// design (each successive extra die is reduced by 1, so a natural-min roll stores
/// a value one below `min`). `crit_success`/`crit_fail` are independent flags set
/// by `eval::success::evaluate_success` via `eval::crit::score_die`; BOTH can be
/// `true` on the same die under an overlapping-threshold `SuccessConfig` — see
/// `crit::score_die`'s doc comment for the rationale.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DieRecord {
    pub id: DieId,
    /// Index of the `Dice` AST node that produced this die, in left-to-right walk
    /// order. Lets Total-mode fold per-group without positional heuristics over a
    /// flattened record list (`eval::sum::evaluate_total`).
    pub group_index: usize,
    pub natural: i32,
    pub value: i32,
    pub kept: bool,
    pub exploded: bool,
    pub rerolled_from: Option<i32>,
    pub crit_success: bool,
    pub crit_fail: bool,
    /// Expertise points allocated to this die by `eval::expertise` (M11b-2);
    /// 0 for every die when the roll has no expertise budget. Audit trail:
    /// `value` is the post-expertise face, `natural`/base `value` the pre-expertise one.
    #[serde(default)]
    pub expertise: i32,
    /// Tag copied from the producing `DiceGroup.label` (M11b-3); `None` if the
    /// group is unlabeled. Read by `RollOutcome::by_label`/`compare_labels`.
    #[serde(default)]
    pub label: Option<String>,
}

/// Fully-derived result. `total` is the primary output for Total mode; in
/// SuccessCount mode it still holds a reference kept-die sum, while
/// `successes`/`pass`/`margin` (all `None` in Total mode with no `difficulty`)
/// carry that mode's primary output. `tier_label`/`tier_value` classify `margin`
/// against the spec's tier ladder; the crit/counter fields are SuccessCount-only
/// aggregates (0 in Total mode).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollOutcome {
    pub total: i64,
    pub records: Vec<DieRecord>,
    pub successes: Option<i32>,
    pub pass: Option<bool>,
    pub margin: Option<i64>,
    pub tier_label: Option<String>,
    pub tier_value: Option<i32>,
    pub crit_successes: i32,
    pub crit_fails: i32,
    pub positive_counter: i32,
    pub negative_counter: i32,
}

impl RollOutcome {
    /// All records (kept and dropped) carrying `label`, in roll order.
    pub fn by_label(&self, label: &str) -> Vec<&DieRecord> {
        self.records
            .iter()
            .filter(|r| r.label.as_deref() == Some(label))
            .collect()
    }

    /// Compares two labels by the sum of their KEPT records' `value`s.
    /// `None` if either label has no records, or either label's records are
    /// unordered (a symbolic group with no numeric value — M11b-3 §9).
    /// Direction-independent: purely "which summed higher."
    pub fn compare_labels(&self, a: &str, b: &str) -> Option<std::cmp::Ordering> {
        let sum_of = |label: &str| -> Option<i64> {
            let recs = self.by_label(label);
            if recs.is_empty() {
                return None;
            }
            Some(recs.iter().filter(|r| r.kept).map(|r| r.value as i64).sum())
        };
        let sa = sum_of(a)?;
        let sb = sum_of(b)?;
        Some(sa.cmp(&sb))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollResult {
    pub spec: RollSpec,
    pub raws: RawRoll,
    pub outcome: RollOutcome,
}

#[cfg(test)]
mod tests {
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

    fn labeled_record(label: &str, value: i32, kept: bool) -> DieRecord {
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
        };
        // Hope kept-sum = 5, Fear kept-sum = 3 -> Hope > Fear.
        assert_eq!(out.compare_labels("Hope", "Fear"), Some(Ordering::Greater));
        assert_eq!(out.compare_labels("Fear", "Hope"), Some(Ordering::Less));
        assert_eq!(out.compare_labels("Hope", "Missing"), None);
    }
}
