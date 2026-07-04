use crate::dice::outcome::{RawRoll, RollOutcome};
use crate::dice::spec::RollSpec;

// Placeholder so `eval::evaluate`'s Sum/SuccessCount dispatcher compiles.
// TODO: implement SuccessCount aggregation — count kept dice satisfying the
// per-die success rule, and set pass/net_margin when required_successes is set.
pub fn evaluate_success(_spec: &RollSpec, raws: &RawRoll) -> RollOutcome {
    RollOutcome {
        total: 0,
        records: raws.records.clone(),
        successes: None,
        pass: None,
        net_margin: None,
    }
}
