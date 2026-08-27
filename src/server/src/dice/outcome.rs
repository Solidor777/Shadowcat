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
    /// Expertise points allocated to this die by `eval::expertise`;
    /// 0 for every die when the roll has no expertise budget. Audit trail:
    /// `value` is the post-expertise face, `natural`/base `value` the pre-expertise one.
    #[serde(default)]
    pub expertise: i32,
    /// Tag copied from the producing `DiceGroup.label`; `None` if the
    /// group is unlabeled. Read by `RollOutcome::by_label`/`compare_labels`.
    #[serde(default)]
    pub label: Option<String>,
    /// Resolved symbols for a `Faces` die's drawn face; empty for `Numeric`.
    #[serde(default)]
    pub symbols: Vec<Symbol>,
    /// Whether the producing group's `DieKind` was ordered (`DieKind::is_ordered`) at
    /// construction time. `Numeric` is always `true`; a `Faces` die is `true`
    /// only if every face in its group had `value: Some`. `compare_labels` uses this to
    /// detect an unordered (symbolic) label — it cannot be inferred from `value` alone,
    /// since a genuine ordered value of `0` is indistinguishable from an unordered
    /// die's default-`0` fallback.
    #[serde(default = "default_ordered")]
    pub ordered: bool,
}

/// `serde(default)` fallback for `ordered`: a record deserialized without this
/// field on the wire had no unordered dice, so `true` (fully ordered)
/// preserves its actual shape.
fn default_ordered() -> bool {
    true
}

/// Fully-derived result. `total` is the primary output for Total mode; in
/// SuccessCount mode it still holds a reference kept-die sum, while
/// `successes`/`pass`/`margin` (all `None` in Total mode with no `difficulty`)
/// carry that mode's primary output. `tier_label`/`tier_value` classify `margin`
/// against `RollSpec`'s tier ladder; the crit/counter fields are SuccessCount-only
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
    /// unlabeled constant. `#[serde(default)]` supplies empty for a stored roll
    /// whose record carries no `labeled_consts` key.
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
    /// unordered (a symbolic group with no numeric value).
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
mod tests;
