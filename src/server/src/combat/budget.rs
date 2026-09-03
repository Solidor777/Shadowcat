//! The ONE resolution of "this combatant's movement budget, in cells".
//! Read by the movement-budget gate (`ws::room::resolve_budget`, and
//! through it the route-preview clamp `ws::conn::handle_pathfind`) and by
//! the `"combat"` derived channel (`SceneEcs::resolved_combats`). A second
//! derivation at any of those sites is the forked-decision class: one copy
//! gating on `ResourceBinding::Tracked` while the other prices a `Mirror`
//! binding as a number leaves a GM who names a Mirror-bound key as
//! `movement.resource` seeing a budget in the tracker while the gate refuses
//! every move under `Enforcement::Hard`.

use super::eval::resolved_resource;
use crate::data::document::Document;
use crate::data::engine::combat::{Interpretation, ResourceBinding};

/// Everything one movement-budget resolution reads, gathered by the caller
/// under whatever guard it already holds (the ECS read guard for the gate and
/// the channel) so this module never touches the ECS itself.
pub(crate) struct BudgetInputs<'a> {
    /// The registry's binding for `CombatEngine.movement.resource`, or `None`
    /// when there is no registry document or no such key — unresolvable.
    pub binding: Option<&'a ResourceBinding>,
    /// The combatant's stored `current` for that resource; `None` = untouched,
    /// which reads as full (the evaluated `max` — the lazy-full rule).
    pub stored: Option<f64>,
    /// The combatant's formula-host document
    /// (`SceneEcs::combatant_formula_host`).
    pub host: Option<&'a Document>,
    /// `CombatEngine.movement.interpretation`.
    pub interpretation: Interpretation,
    /// The scene's `grid.distance.per_cell`, or `None` when the scene authors
    /// no distance scale — unresolvable under `Interpretation::PerCell`.
    pub per_cell: Option<f64>,
}

/// A resolved movement budget: the evaluated `current` and the conversion
/// that folds the interpretation into one multiplier, so the cell ceiling
/// (`current / cost_to_resource`) and the post-move decrement
/// (`MoveOutcome.cost * cost_to_resource`) share one conversion.
pub(crate) struct MovementBudget {
    /// The combatant's current value for the resource (stored, else the
    /// evaluated full).
    pub current: f64,
    /// `MoveOutcome.cost` (cells) → resource units: the scene's `per_cell`
    /// distance under `PerCell`, or `1.0` under `Spaces`.
    pub cost_to_resource: f64,
}

impl MovementBudget {
    /// The budget in cells, through `resource_cells`.
    pub(crate) fn cells(&self) -> f64 {
        resource_cells(self.current, self.cost_to_resource)
    }
}

/// The one division a movement-budget number performs: `current` divided by
/// the per-cell (or per-space) conversion rate. The `Hard` truncation
/// ceiling, the route-preview number (`PathResult.budget_cells`) and the
/// channel's `movement_cells` all reach it through `MovementBudget::cells`,
/// so no two of them can drift apart by a rounding or unit difference.
pub(crate) fn resource_cells(current: f64, cost_to_resource: f64) -> f64 {
    current / cost_to_resource
}

/// Resolves one movement budget, or `None` when it cannot be resolved: no
/// binding, a `Mirror` binding (a spend cannot decrement a derived value, and
/// the server never writes the `system` band), a formula-evaluation failure,
/// or a missing `per_cell` scale under `Interpretation::PerCell`. The numbers
/// come from `combat::eval::resolved_resource` over the combatant's formula
/// host — the SAME derivation the combat transitions use.
pub(crate) fn resolve_movement_budget(inputs: &BudgetInputs<'_>) -> Option<MovementBudget> {
    let tracked = match inputs.binding? {
        ResourceBinding::Mirror { .. } => return None,
        tracked @ ResourceBinding::Tracked { .. } => tracked,
    };
    let nums = resolved_resource(tracked, inputs.stored, inputs.host).ok()?;
    let cost_to_resource = match inputs.interpretation {
        Interpretation::PerCell => inputs.per_cell?,
        Interpretation::Spaces => 1.0,
    };
    Some(MovementBudget {
        current: nums.current,
        cost_to_resource,
    })
}
