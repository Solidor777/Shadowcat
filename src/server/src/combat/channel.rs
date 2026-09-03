//! The `"combat"` derived channel: server-resolved resource numbers and movement budgets for
//! every combat/combatant `ctx` may read, computed through the SAME `combat::eval` derivations
//! the transitions and the movement-budget gate use. The client evaluates and stores nothing.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// The whole `"combat"` derived-channel payload: every combat `ctx` may read, sorted by id for a
/// stable fingerprint (the egress loop's change detection compares whole payloads).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct CombatsPayload {
    /// One entry per readable combat.
    pub combats: Vec<CombatView>,
}

/// One combat's resolved view: identity plus every combatant `ctx` may read, sorted by id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct CombatView {
    /// The combat document's id.
    pub id: Uuid,
    /// The scene this combat is bound to.
    pub scene_id: Uuid,
    /// Readable combatants, sorted by id.
    pub combatants: Vec<CombatantView>,
}

/// One combatant's resolved numbers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct CombatantView {
    /// The combatant document's id.
    pub id: Uuid,
    /// Every registry-key resolution `ctx` may see the `/engine/resources` pointer for; `None`
    /// when that band's tier is not visible to `ctx`.
    pub resources: Option<BTreeMap<String, ResolvedResourceView>>,
    /// The combat's movement resource converted to cells for this combatant, when resolvable;
    /// `None` for no movement resource, an unresolvable binding, or a hidden `resources` band.
    pub movement_cells: Option<f64>,
}

/// One resource's resolved numbers for one combatant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct ResolvedResourceView {
    /// Whether the registry binds this resource as a derived mirror or a tracked spend.
    pub binding: ResourceBindingKind,
    /// The resolved current value; `None` on an evaluation failure.
    pub current: Option<f64>,
    /// The resolved ceiling; `None` on an evaluation failure.
    pub max: Option<f64>,
    /// The formula-evaluation failure's detail, when resolution failed.
    pub error: Option<String>,
}

/// Discriminates `ResolvedResourceView`'s source binding kind, mirroring
/// `data::engine::combat::ResourceBinding`'s own wire tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum ResourceBindingKind {
    /// `ResourceBinding::Mirror`.
    Mirror,
    /// `ResourceBinding::Tracked`.
    Tracked,
}

#[cfg(test)]
mod tests;
