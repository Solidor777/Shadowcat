//! Illumination field + gradation banding. Pure, engine-owned, server-authoritative.
//! Clean-room: standard radial light falloff plus threshold banding of a
//! continuous `[0,1]` illumination field. No proprietary VTT/engine source consulted.
//!
//! Mirrors the client `light-gradation`/`light`/`vision-modes` shapes in the `scene-docs` module; the server
//! stays structural-only (callers parse documents and pass these plain structs).

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::scene::grid_shape::WorldExtent;
use crate::scene::vision;
use crate::scene::vision::point_in_poly;
use crate::scene::vision::P;

/// DoS bound on the number of boundary edge-samples projected as environment-light sources
/// (`env_light_polys`). Matches the project's fail-closed-bound convention (cf.
/// `pathfinding::MAX_PATH_NODES`): a scene whose perimeter would demand more samples is sampled
/// coarsely rather than casting an unbounded number of visibility polygons.
pub const MAX_ENV_LIGHT_SAMPLES: usize = 256;

/// Photometric falloff curve across the dim band `(bright_radius, dim_radius]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Falloff {
    /// Smooth linear taper from full intensity at the bright edge to 0 at the dim edge.
    Linear,
    /// Smooth quadratic taper (faster than linear).
    Quadratic,
    /// No gradient: a flat dim-band step (`0.5 × intensity`) — bright/dim radii feed the gradation
    /// bands directly. With the default gradation this lands a unit-intensity light's
    /// dim band at 0.5 ∈ [dim 0.34, bright 0.67).
    None,
}

/// A light source's photometric inputs. Radii are in GRID CELLS; `color` is packed `0xRRGGBB`.
/// The value fields mirror `LightEmission` (used by both standalone `LightEngine` docs and
/// token-carried emissions); `pos` is the source's scene-unit position (the document's `x`/`y`,
/// or the carrying token's live position).
#[derive(Clone, Debug, PartialEq)]
pub struct Light {
    /// Position in scene units.
    pub pos: P,
    /// Packed `0xRRGGBB` tint contribution.
    pub color: u32,
    /// Peak illumination level within `bright_radius`.
    pub intensity: f64,
    /// Full-intensity radius, grid cells.
    pub bright_radius: f64,
    /// Outer taper radius, grid cells; the taper spans
    /// `(bright_radius, dim_radius]` (not validated against `bright_radius`).
    pub dim_radius: f64,
    /// Taper curve across `(bright_radius, dim_radius]`.
    pub falloff: Falloff,
    /// Disabled lights contribute nothing (kept for cache-key stability).
    pub enabled: bool,
}

/// Illumination this light contributes at distance `dist_cells` (in CELLS), BEFORE occlusion.
/// Full `intensity` within `bright_radius`; tapers across `(bright_radius, dim_radius]` by the
/// curve; 0 beyond `dim_radius`. Disabled / non-finite / non-positive `dim_radius` ⇒ 0, and a
/// non-finite `dist_cells` ⇒ 0 too — without that guard the `Falloff::None` arm (which ignores
/// the taper parameter) would return a finite, positive level for EVERY cell of a NaN-positioned
/// light.
///
/// Returns a value in `[0, intensity]`. A caller composing multiple lights clamps the summed
/// result to `[0, 1]` before band lookup. `intensity` must be finite (the document→`Light` parser
/// clamps it to `[0, 1]`).
pub fn light_illumination(light: &Light, dist_cells: f64) -> f64 {
    if !light.enabled
        || !light.dim_radius.is_finite()
        || light.dim_radius <= 0.0
        || !dist_cells.is_finite()
        || dist_cells > light.dim_radius
    {
        return 0.0;
    }
    if dist_cells <= light.bright_radius {
        return light.intensity;
    }
    let span = (light.dim_radius - light.bright_radius).max(1e-9);
    let t = ((light.dim_radius - dist_cells) / span).clamp(0.0, 1.0); // 1 at bright edge → 0 at dim edge
    let f = match light.falloff {
        Falloff::None => 0.5,
        Falloff::Linear => t,
        Falloff::Quadratic => t * t,
    };
    light.intensity * f
}

/// A named illumination band. `min_illumination` is the minimum `[0,1]` light level a cell must reach
/// to qualify for this band. Mirrors the client `GradationBand`.
#[derive(Clone, Debug, PartialEq)]
pub struct Band {
    /// Band name (matched against `VisionMode::illumination_floor`).
    pub name: String,
    /// INVARIANT: must be finite and in `[0,1]`; non-finite values are dropped by `sorted_bands`.
    pub min_illumination: f64,
}

/// Built-in three-band gradation (bright → dim → dark). Mirrors `DEFAULT_GRADATION` in the `scene-docs` module.
pub fn default_bands() -> Vec<Band> {
    vec![
        Band {
            name: "bright".into(),
            min_illumination: 0.67,
        },
        Band {
            name: "dim".into(),
            min_illumination: 0.34,
        },
        Band {
            name: "dark".into(),
            min_illumination: 0.0,
        },
    ]
}

/// Bands sorted brightest-first (descending `min_illumination`). Non-finite bands are dropped
/// before sorting. Fail-closed: empty input (or all-non-finite) → defaults.
pub fn sorted_bands(mut bands: Vec<Band>) -> Vec<Band> {
    bands.retain(|b| b.min_illumination.is_finite());
    if bands.is_empty() {
        return default_bands();
    }
    bands.sort_by(|a, b| {
        b.min_illumination
            .partial_cmp(&a.min_illumination)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    bands
}

/// Index (brightest=0) of the band a given illumination falls into.
/// `bands` MUST be non-empty and brightest-first (always true for `sorted_bands` output).
/// Clamps to the darkest band if nothing matched (defensive; the darkest floor is normally 0.0).
pub fn band_index(bands: &[Band], illumination: f64) -> usize {
    debug_assert!(
        !bands.is_empty(),
        "INVARIANT: bands must be non-empty; call sorted_bands first"
    );
    for (i, b) in bands.iter().enumerate() {
        if illumination >= b.min_illumination {
            return i;
        }
    }
    bands.len().saturating_sub(1)
}

/// Minimum illumination to perceive a cell at the named floor band. A token whose vision floor is
/// `floor_name` perceives a cell iff `illumination >= floor_min`. Fail-closed: an unknown floor
/// resolves to the brightest band's min (most restrictive → under-reveal).
pub fn floor_min(bands: &[Band], floor_name: &str) -> f64 {
    bands
        .iter()
        .find(|b| b.name == floor_name)
        .map(|b| b.min_illumination)
        .unwrap_or_else(|| bands.first().map(|b| b.min_illumination).unwrap_or(1.0))
}

/// A composed per-cell illumination result: a `[0,1]` `level` (the saturated sum of every
/// contributor's level) and a packed-RGB `tint` (the illuminance-weighted mix of the
/// contributors' colors; `0x000000` when nothing contributes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellLight {
    /// Composed illumination level, `[0, 1]` — `clamp01(Σ contributor levels)`.
    pub level: f64,
    /// Illuminance-weighted mix of the contributors' colors, packed `0xRRGGBB`:
    /// `Σ(levelᵢ × colorᵢ) / Σ(levelᵢ)` per channel over the UNSATURATED sum; `0x000000` when no
    /// source contributes.
    pub tint: u32,
}

/// Accumulate one contributor's packed `0xRRGGBB` color into a per-channel running weighted sum.
fn add_tint(acc: &mut [f64; 3], color: u32, weight: f64) {
    acc[0] += f64::from((color >> 16) & 0xFF) * weight;
    acc[1] += f64::from((color >> 8) & 0xFF) * weight;
    acc[2] += f64::from(color & 0xFF) * weight;
}

/// Maps arc-length `d` along the perimeter of the envelope `extent` to a boundary point, walking
/// `top → right → bottom → left` from `extent.min`. `d` is wrapped into `[0, 2(w+h))`, `w`/`h`
/// being the envelope's own spans. The walk starts at the envelope's MINIMUM rather than the
/// origin, which is where a hex block's boundary actually is.
fn perimeter_point(extent: WorldExtent, d: f64) -> P {
    let (w, h) = (extent.width(), extent.height());
    let (x0, y0) = extent.min;
    let perim = 2.0 * (w + h);
    let d = d.rem_euclid(perim);
    if d < w {
        (x0 + d, y0)
    } else if d < w + h {
        (x0 + w, y0 + (d - w))
    } else if d < 2.0 * w + h {
        (x0 + w - (d - (w + h)), y0 + h)
    } else {
        (x0, y0 + h - (d - (2.0 * w + h)))
    }
}

/// Boundary-projected environment-light occlusion polygons. Environment light enters the scene
/// from OUTSIDE its boundary; a cell is lit iff an unobstructed line reaches it from some point on
/// the scene rectangle past the `blocksLight` walls. The rectangle perimeter is sampled, and each
/// sample's visibility polygon is computed with the SAME `vision::visibility_polygon` primitive
/// placed lights and vision use (never a second, forked occlusion computation). A cell is
/// environment-lit iff it lies inside ANY sample's polygon (composed by `env_lit`).
///
/// `extent` is the scene's WORLD-unit envelope, produced by `GridShape::world_extent` from the
/// scene's authored grid-unit bounds; its `min` is the origin only on square, so the walk is
/// anchored on it rather than on the origin. `cell_size` is the grid's
/// INDEXING scale and plays two roles that are both discretization, not measurement: it sets the
/// sample count (one per indexing unit of perimeter, clamped to `[4, MAX_ENV_LIGHT_SAMPLES]` —
/// on hex the indexing unit is the circumradius, so that is about 1.73 samples per cell pitch) and the
/// raycast bound's margin, so boundary samples sit strictly inside it. Sample count is a
/// convergence knob, not a secrecy one: the sampled union UNDER-APPROXIMATES the true
/// boundary-reachable set, so a coarser count under-reveals and a finer one is strictly more faithful —
/// which is why the indexing scale, the smaller of the two scalars on hex, is the right input here
/// and `world_units_per_cell` is not.
///
/// Fail-closed: a non-finite corner, a non-positive `width()`/`height()`, or a non-finite or
/// non-positive `cell_size` ⇒ empty (environment reaches
/// nothing — under-reveal, never over-reveal). The boundary itself never occludes (only interior
/// `blocksLight` walls do): light enters freely across the scene edge.
pub(crate) fn env_light_polys(
    extent: WorldExtent,
    cell_size: f64,
    light_walls: &[vision::Seg],
) -> Vec<Vec<P>> {
    let (w, h) = (extent.width(), extent.height());
    if !extent.min.0.is_finite()
        || !extent.min.1.is_finite()
        || !extent.max.0.is_finite()
        || !extent.max.1.is_finite()
        || w <= 0.0
        || h <= 0.0
        || !cell_size.is_finite()
        || cell_size <= 0.0
    {
        return Vec::new();
    }
    let perim = 2.0 * (w + h);
    let n = (perim / cell_size).round() as usize;
    let n = n.clamp(4, MAX_ENV_LIGHT_SAMPLES);
    let margin = cell_size.max(1.0);
    let bound = vision::Rect {
        minx: extent.min.0 - margin,
        miny: extent.min.1 - margin,
        maxx: extent.max.0 + margin,
        maxy: extent.max.1 + margin,
    };
    (0..n)
        .map(|i| {
            let d = (i as f64) / (n as f64) * perim;
            vision::visibility_polygon(perimeter_point(extent, d), light_walls, bound)
        })
        .collect()
}

/// Whether the environment reaches `center`: true iff `center` lies inside some boundary sample's
/// visibility polygon. Fail-closed: an EMPTY `env_polys` (no boundary reachability computed) ⇒
/// false (environment does not reach — under-reveal). Distinct from a placed light's per-`k` empty
/// `lit_polys` entry (which fail-OPENS): there, "no occluder computed for this light" leaves the
/// light unoccluded; here, the whole environment source is gated by the polygon SET, so an empty
/// set means the source could not be projected at all and must not leak illumination.
fn env_lit(env_polys: &[Vec<P>], center: P) -> bool {
    !env_polys.is_empty() && env_polys.iter().any(|poly| point_in_poly(poly, center))
}

/// Compose illumination at a cell center by ADDITIVE SUPERPOSITION with saturation: every
/// admitted contributor (the boundary-projected environment ambient plus each unoccluded light)
/// adds its level into one sum, and `level` is that sum clamped to `[0,1]` — two dim lights
/// genuinely brighten their overlap. `tint` is the illuminance-weighted color mix
/// (`Σ(levelᵢ × colorᵢ) / Σ(levelᵢ)` per channel, divided by the UNSATURATED sum so saturation
/// brightens without skewing hue).
/// `lit_polys[k]` is `lights[k]`'s `blocksLight` visibility polygon — a light contributes only if the
/// cell center lies inside it (an EMPTY polygon means "no occluder computed" → never occludes).
/// `env_polys` are the scene-boundary visibility polygons (`env_light_polys`): the environment
/// ambient reaches this cell only if it is `env_lit` (inside some boundary polygon), so a
/// `blocksLight`-sealed interior receives no ambient. Environment occlusion is strictly
/// NARROWING: an empty `env_polys` or a cell outside every polygon contributes 0, never negative,
/// so occlusion can only shrink the composed field, never grow it.
/// `world_units_per_cell` is the world distance one grid step represents
/// (`GridShape::world_units_per_cell`) — light radii are authored in cells, so distance is
/// divided by it. It is NOT the cell indexing scale; the two coincide on square and differ on
/// hex. CALLER PRECONDITION: it must be positive — a non-positive value is a caller error; the
/// upstream caller guards it (a release-build fallback avoids division-by-zero but the value is wrong).
/// `env_intensity` must be finite; the document→settings resolver clamps it to `[0,1]`.
/// Per-source fail-closed directions are preserved under composition: a disabled/non-finite/
/// occluded source contributes exactly 0, and a zero-contributor cell is dark with tint
/// `0x000000`.
///
/// Note on the tint channel: the weighted mix means a contributor's hue is visible in every
/// shared cell even when a brighter source dominates the LEVEL — the per-cell tint in the
/// `vision` payload is the only place this reaches, and it is display metadata, not a position
/// or identity disclosure.
pub fn cell_illumination(
    center: P,
    env_intensity: f64,
    env_color: u32,
    lights: &[Light],
    lit_polys: &[Vec<P>],
    env_polys: &[Vec<P>],
    world_units_per_cell: f64,
) -> CellLight {
    debug_assert!(
        world_units_per_cell > 0.0,
        "INVARIANT: world_units_per_cell must be positive; light radii are authored in cells"
    );
    debug_assert!(env_intensity.is_finite(), "env_intensity must be finite");
    let mut total = 0.0_f64;
    let mut tint_sum = [0.0_f64; 3];
    // Environment ambient is boundary-projected and blocksLight-occluded (see `env_polys`).
    if env_intensity > 0.0 && env_lit(env_polys, center) {
        let level = env_intensity.clamp(0.0, 1.0);
        total += level;
        add_tint(&mut tint_sum, env_color, level);
    }
    for (k, light) in lights.iter().enumerate() {
        // Occlusion: a non-empty polygon that excludes the cell center kills this light's reach here.
        if let Some(poly) = lit_polys.get(k) {
            if !poly.is_empty() && !point_in_poly(poly, center) {
                continue;
            }
        }
        let d = ((center.0 - light.pos.0).powi(2) + (center.1 - light.pos.1).powi(2)).sqrt();
        let dist_cells = if world_units_per_cell > 0.0 {
            d / world_units_per_cell
        } else {
            d
        };
        let level = light_illumination(light, dist_cells);
        // Non-finite or zero levels contribute nothing (fail-closed per source); a NaN reaching
        // the running sum would also poison the saturation clamp below, which panics on NaN.
        if level.is_finite() && level > 0.0 {
            total += level;
            add_tint(&mut tint_sum, light.color, level);
        }
    }
    if total <= 0.0 {
        return CellLight {
            level: 0.0,
            tint: 0,
        };
    }
    let channel = |idx: usize| -> u32 { (tint_sum[idx] / total).round().clamp(0.0, 255.0) as u32 };
    CellLight {
        level: total.clamp(0.0, 1.0),
        tint: (channel(0) << 16) | (channel(1) << 8) | channel(2),
    }
}

#[cfg(test)]
mod tests;
