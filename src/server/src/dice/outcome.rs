#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::dice::spec::{ConstTerm, DieId, DieKind, RollSpec, Symbol};

/// A single die's natural (RNG) result — the only nondeterministic artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawDie {
    /// Stable per-roll die id.
    pub id: DieId,
    /// The face space it was rolled from.
    pub kind: DieKind,
    /// The natural (unmodified) RNG face.
    pub natural: i32,
}

/// The RNG output for a whole roll. `dice` is the natural-face log; `records` is the
/// post-pipeline per-die result (filled by `roll`/`recalculate`); `next_id` hands out
/// stable ids so reroll/add ops never collide with existing dice.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawRoll {
    /// Natural-face log, in roll order.
    pub dice: Vec<RawDie>,
    /// Post-pipeline per-die results (filled by `roll`/`recalculate`).
    pub records: Vec<DieRecord>,
    /// Next fresh `DieId` (never reused within the roll).
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
        self.next_id = match self.next_id.checked_add(1) {
            Some(n) => n,
            None => {
                // Unreachable while the chat boundary's roll-record cap bounds any
                // one roll's die count well below `DieId::MAX`; guard is
                // defense-in-depth against an id collision that would otherwise
                // silently alias two distinct dice under the same id.
                debug_assert!(false, "RawRoll::next_id overflowed DieId::MAX");
                DieId::MAX
            }
        };
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
    /// The die's stable id (matches its `RawDie`).
    pub id: DieId,
    /// Index of the `Dice` AST node that produced this die, in left-to-right walk
    /// order. Lets Total-mode fold per-group without positional heuristics over a
    /// flattened record list (`eval::sum::evaluate_total`).
    pub group_index: usize,
    /// The original natural face.
    pub natural: i32,
    /// Post-modifier face (see the struct doc for penetrate's out-of-range case).
    pub value: i32,
    /// Survived keep/drop selection.
    pub kept: bool,
    /// This die triggered an explosion.
    pub exploded: bool,
    /// Immediately-preceding value for a rerolled die (see the struct doc).
    pub rerolled_from: Option<i32>,
    /// Crit-success event fired on this die.
    pub crit_success: bool,
    /// Crit-fail event fired on this die (can coexist with `crit_success`).
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
    /// Resolved symbols for a `Faces` die's drawn face (M11b-3); empty for `Numeric`.
    #[serde(default)]
    pub symbols: Vec<Symbol>,
    /// Whether the producing group's `DieKind` was ordered (`DieKind::is_ordered`) at
    /// construction time (M11b-3). `Numeric` is always `true`; a `Faces` die is `true`
    /// only if every face in its group had `value: Some`. `compare_labels` uses this to
    /// detect an unordered (symbolic) label — it cannot be inferred from `value` alone,
    /// since a genuine ordered value of `0` is indistinguishable from an unordered
    /// die's default-`0` fallback.
    #[serde(default = "default_ordered")]
    pub ordered: bool,
}

/// `serde(default)` fallback for `ordered`: pre-M11b-3 records (and any deserialized
/// data predating this field) had no unordered dice at all, so `true` preserves their
/// prior (fully-ordered) behavior.
fn default_ordered() -> bool {
    true
}

/// Fully-derived result. `total` is the primary output for Total mode; in
/// SuccessCount mode it still holds a reference kept-die sum, while
/// `successes`/`pass`/`margin` (all `None` in Total mode with no `difficulty`)
/// carry that mode's primary output. `tier_label`/`tier_value` classify `margin`
/// against the spec's tier ladder; the crit/counter fields are SuccessCount-only
/// aggregates (0 in Total mode).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollOutcome {
    /// Total-mode fold result; in SuccessCount mode, the reference sum of
    /// kept-die values (see the struct doc), never a fold.
    pub total: i64,
    /// Per-die results, AST left-to-right then roll order.
    pub records: Vec<DieRecord>,
    /// Net successes (SuccessCount mode only).
    pub successes: Option<i32>,
    /// Pass/fail against the margin reference, when one exists.
    pub pass: Option<bool>,
    /// Oriented margin against difficulty/required successes.
    pub margin: Option<i64>,
    /// Ladder rung label `margin` classified into.
    pub tier_label: Option<String>,
    /// Ladder rung numeric payload.
    pub tier_value: Option<i32>,
    /// Count of crit-success events across kept dice.
    pub crit_successes: i32,
    /// Count of crit-fail events across kept dice.
    pub crit_fails: i32,
    /// Sum of fired `CritSuccess::positive_counter` values.
    pub positive_counter: i32,
    /// Sum of fired `CritFail::negative_counter` values.
    pub negative_counter: i32,
    /// Per-symbol tallies over KEPT dice, computed unconditionally (independent
    /// of `SuccessRule`'s variant). Deterministic iteration order (`BTreeMap`).
    #[serde(default)]
    pub symbol_counts: BTreeMap<Symbol, i32>,
    /// Every labeled `Const` term in the expression, in AST left-to-right order
    /// (`eval::sum::evaluate_total`'s `collect_labeled_consts`). Total-mode only —
    /// display/provenance decoration for chat-embed rendering, mirroring how a
    /// labeled `DiceGroup`'s dice show up in `records`; a bare constant has no
    /// dice pool, so this is NEVER read by `by_label`/`compare_labels`. Always
    /// empty in SuccessCount mode (arithmetic is ignored there) and for an
    /// unlabeled constant. `#[serde(default)]`: absent on any roll persisted
    /// before this field existed.
    #[serde(default)]
    pub labeled_consts: Vec<ConstTerm>,
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
            if recs.is_empty() || recs.iter().any(|r| !r.ordered) {
                return None;
            }
            Some(recs.iter().filter(|r| r.kept).map(|r| r.value as i64).sum())
        };
        let sa = sum_of(a)?;
        let sb = sum_of(b)?;
        Some(sa.cmp(&sb))
    }
}

/// A complete roll: what was asked, what the RNG produced, what it means.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollResult {
    /// The canonical parameters the roll ran with.
    pub spec: RollSpec,
    /// The natural faces + per-die pipeline results.
    pub raws: RawRoll,
    /// The scored outcome.
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
        // the exact Daggerheart Hope/Fear headline case (design doc §3/§5): an
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
        // pre-M11d/pre-M13d stored RollOutcome shape (no such Rust-side test
        // existed; the chat_rolls back-compat test carries no RollOutcome).
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
}
