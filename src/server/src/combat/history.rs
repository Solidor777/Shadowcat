//! Turn-history record/restore seam. `transition::advance`/`start` call
//! through here so the rewind/redo transition lands as a body change to
//! these two functions, never a call-site change in `transition`.
//!
//! TODO: implement `append_record` to append a `TurnRecord` (bounded by
//! `MAX_TURN_HISTORY`) capturing every anchored effect and combatant at the
//! turn boundary.
//! TODO: implement `fast_forward` to short-circuit a transition by restoring
//! a cached history record instead of re-deriving it, when the history
//! cursor already covers the target turn.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::data::command::Operation;

use super::{CombatError, CombatSnapshot};

/// Appends a turn-boundary record to the combat's history log. Currently
/// emits no ops (see the module TODOs).
pub(crate) fn append_record(_snap: &CombatSnapshot, _ops: &mut Vec<Operation>) {}

/// Fast-forwards a redo when a cached history record already covers the
/// target turn, instead of re-deriving it through the ordinary transition
/// walk. Currently never fast-forwards (see the module TODOs).
pub(crate) fn fast_forward(_snap: &CombatSnapshot) -> Result<Option<Vec<Operation>>, CombatError> {
    Ok(None)
}
