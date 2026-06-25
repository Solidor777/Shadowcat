//! Server-authoritative grid A* pathfinder (M10e-6). Pure + headless: callers pass parsed inputs
//! (walls, mask, cell size, rule, footprint); this module owns no I/O and borrows no ECS.
//! Engine-owned geometry (ARCHITECTURE §6 exception); clean-room A* (Hart, Nilsson & Raphael 1968).
//!
//! INVARIANT (spec §13): the per-cell mask test consumes the SAME `visible_cells` set the M10e-4
//! movement gate uses — the route can never thread the unknown nor leak hidden geometry.

/// Grid diagonal-cost rule (from `world-settings.pathfinding.diagonalRule`). All four are the same
/// king-move graph; they differ only in diagonal cost + the admissible heuristic. `Alternating`
/// (PF1e/3.5 "5-10-5") costs diagonals 1,2,1,2… and so requires a parity bit in the search node.
// TODO: remove once the A* body consumes this enum.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagonalRule {
    Chebyshev,
    Manhattan,
    Euclidean,
    Alternating,
}

/// Parse the diagonal-rule string; unknown/missing ⇒ `Chebyshev` (mirrors the client
/// `DEFAULT_WORLD_SETTINGS.pathfinding.diagonalRule` in `scene-docs.ts`).
// TODO: remove once the A* body calls this.
#[allow(dead_code)]
pub fn parse_diagonal_rule(s: &str) -> DiagonalRule {
    match s {
        "manhattan" => DiagonalRule::Manhattan,
        "euclidean" => DiagonalRule::Euclidean,
        "alternating" => DiagonalRule::Alternating,
        _ => DiagonalRule::Chebyshev,
    }
}
