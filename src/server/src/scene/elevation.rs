//! The elevation model (2.5D, single-level): tokens and lights carry an elevation
//! above the ground plane (0 = grounded), and a wall optionally carries the elevation
//! band its occlusion applies to. Elevation only ever FILTERS which walls occlude a
//! sight/light source — the star-shaped raycast pipeline itself is untouched, and
//! sight/light ranges stay 2D (horizontal). Environment ambient light is the one
//! exception: it keeps the full wall set at every elevation (walls always shadow
//! sky-light, or daylight would flood interiors).

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two crate-level deny attributes this module declares.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::data::engine as eng;

/// The ground plane: the elevation a token or light sits at when it authors none.
pub(crate) const GROUND: f64 = 0.0;

/// Read an optional authored elevation (`TokenEngine::elevation`,
/// `LightEngine::elevation`): absent/null reads as ground, and a non-finite
/// stored value is clamped to ground as defense in depth (unreachable through
/// JSON, but a reader must never propagate NaN into the occlusion pipeline).
pub(crate) fn elevation_or_ground(v: Option<f64>) -> f64 {
    match v {
        Some(e) if e.is_finite() => e,
        _ => GROUND,
    }
}

/// Whether a wall with elevation band `band` occludes a sight/light source at
/// elevation `e`: the wall occludes iff `bottom ≤ e ≤ top`, an absent end is
/// unbounded, and an absent band occludes every elevation. Fail-closed: a
/// malformed interval (`bottom > top`) or a non-finite authored endpoint occludes
/// EVERYTHING (the pre-elevation behavior), so a corrupt wall never opens a
/// sightline the scene did not have.
pub(crate) fn wall_occludes(band: Option<&eng::WallElevation>, e: f64) -> bool {
    let Some(b) = band else { return true };
    if b.bottom.is_some_and(|v| !v.is_finite()) || b.top.is_some_and(|v| !v.is_finite()) {
        return true;
    }
    let lo = b.bottom.unwrap_or(f64::NEG_INFINITY);
    let hi = b.top.unwrap_or(f64::INFINITY);
    if lo > hi {
        return true;
    }
    lo <= e && e <= hi
}

#[cfg(test)]
mod tests;
