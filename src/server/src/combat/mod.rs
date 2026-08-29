//! The server-owned combat clock: loads a `CombatSnapshot`, runs a pure
//! transition (`transition`), and hands ONE command's ops to `Room`.
//! INVARIANT: nothing here evaluates a `Formula::Text`; every transition reads
//! only resolved numbers/flags and skips what is unresolved.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

pub mod effects;
pub mod history;
pub mod ops;
pub mod snapshot;
pub mod transition;

#[cfg(test)]
mod tests;

pub use effects::collect_effects;
pub use snapshot::{load_snapshot, CombatSnapshot, Combatant};
pub use transition::{
    advance, end, pause, rebuild_order, resource, roll, sort, start, ResourceOp, RollPost,
};

/// Why a combat intent was refused. `Display` yields ONE wording for every
/// variant that could disclose a hidden combatant (`NotFound`, `Forbidden`,
/// `NotRunning`, `Data`): "combat rejected".
#[derive(Debug, thiserror::Error)]
pub enum CombatError {
    /// Combat, combatant, resource or host not found (or not readable).
    #[error("combat rejected")]
    NotFound,
    /// The actor may not perform this intent.
    #[error("combat rejected")]
    Forbidden,
    /// The combat is not active.
    #[error("combat rejected")]
    NotRunning,
    /// Nothing to advance/rewind (empty order, first record).
    #[error("nothing to do")]
    Empty,
    /// Rewind refused: at the first record or past the retained history.
    #[error("cannot rewind further")]
    Unrewindable,
    /// A roll failed its caps/parse.
    #[error("{0}")]
    Roll(#[from] crate::chat::rolls::RollError),
    /// Repository failure.
    #[error("combat rejected")]
    Data(#[from] crate::data::DataError),
}
