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
/// survived keep/drop; `rerolled_from` = prior natural if this replaced a reroll.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DieRecord {
    pub id: DieId,
    pub natural: i32,
    pub value: i32,
    pub kept: bool,
    pub exploded: bool,
    pub rerolled_from: Option<i32>,
}

/// Fully-derived result. `total` for Sum; `successes`/`pass`/`net_margin` for
/// SuccessCount (all `None` in the other mode where inapplicable).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollOutcome {
    pub total: i64,
    pub records: Vec<DieRecord>,
    pub successes: Option<i32>,
    pub pass: Option<bool>,
    pub net_margin: Option<i32>,
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
}
