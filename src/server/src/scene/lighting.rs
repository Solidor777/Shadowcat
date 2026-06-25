//! Illumination field + gradation banding (M10e-2). Pure, engine-owned (ARCHITECTURE #6),
//! server-authoritative (#3). Clean-room: standard radial light falloff plus threshold banding of a
//! continuous [0,1] illumination field. No proprietary VTT/engine source consulted.
//!
//! Mirrors the client `light-gradation`/`light`/`vision-modes` shapes in scene-docs.ts; the server
//! stays structural-only (callers parse documents and pass these plain structs).

use crate::scene::vision::point_in_poly;
use crate::scene::vision::P;

/// Photometric falloff curve across the dim band `(bright_radius, dim_radius]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Falloff {
    /// Smooth linear taper from full intensity at the bright edge to 0 at the dim edge.
    Linear,
    /// Smooth quadratic taper (faster than linear).
    Quadratic,
    /// No gradient: a flat dim-band step (`0.5 × intensity`) — bright/dim radii feed the gradation
    /// bands directly (spec §5.4). With the default gradation this lands a unit-intensity light's
    /// dim band at 0.5 ∈ [dim 0.34, bright 0.67).
    None,
}

/// A placed light's photometric inputs. Radii are in GRID CELLS; `color` is packed `0xRRGGBB`.
/// Mirrors the client `LightSystem` (scene-docs.ts).
#[derive(Clone, Debug)]
pub struct Light {
    pub pos: P,
    pub color: u32,
    pub intensity: f64,
    pub bright_radius: f64,
    pub dim_radius: f64,
    pub falloff: Falloff,
    pub enabled: bool,
}

/// Illumination this light contributes at distance `dist_cells` (in CELLS), BEFORE occlusion.
/// Full `intensity` within `bright_radius`; tapers across `(bright_radius, dim_radius]` by the
/// curve; 0 beyond `dim_radius`. Disabled / non-finite / non-positive `dim_radius` ⇒ 0.
///
/// Returns a value in `[0, intensity]`. A caller composing multiple lights clamps the summed
/// result to `[0, 1]` before band lookup. `intensity` must be finite (the document→`Light` parser
/// clamps it to `[0, 1]`).
pub fn light_illumination(light: &Light, dist_cells: f64) -> f64 {
    if !light.enabled
        || !light.dim_radius.is_finite()
        || light.dim_radius <= 0.0
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

/// A named illumination band. `min_illumination` is the minimum [0,1] light level a cell must reach
/// to qualify for this band. Mirrors the client `GradationBand`.
#[derive(Clone, Debug, PartialEq)]
pub struct Band {
    pub name: String,
    /// INVARIANT: must be finite and in [0,1]; non-finite values are dropped by `sorted_bands`.
    pub min_illumination: f64,
}

/// Built-in three-band gradation (bright → dim → dark). Mirrors `DEFAULT_GRADATION` in scene-docs.ts.
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

/// A composed per-cell illumination result: a [0,1] `level` and a packed-RGB `tint` (the dominant
/// contributor's color; `0x000000` when only an unset environment contributes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellLight {
    pub level: f64,
    pub tint: u32,
}

/// Compose illumination at a cell center from a flat environment ambient plus each light, taking the
/// MAX contributor (no over-brightening, spec §6); `tint` follows the dominant contributor.
/// `lit_polys[k]` is `lights[k]`'s `blocksLight` visibility polygon — a light contributes only if the
/// cell center lies inside it (an EMPTY polygon means "no occluder computed" → never occludes).
/// `cell_size` is world units per cell (light radii are in cells, so distance is divided by it).
pub fn cell_illumination(
    center: P,
    env_intensity: f64,
    env_color: u32,
    lights: &[Light],
    lit_polys: &[Vec<P>],
    cell_size: f64,
) -> CellLight {
    let mut best = CellLight {
        level: env_intensity.clamp(0.0, 1.0),
        tint: env_color,
    };
    for (k, light) in lights.iter().enumerate() {
        // Occlusion: a non-empty polygon that excludes the cell center kills this light's reach here.
        if let Some(poly) = lit_polys.get(k) {
            if !poly.is_empty() && !point_in_poly(poly, center) {
                continue;
            }
        }
        let d = ((center.0 - light.pos.0).powi(2) + (center.1 - light.pos.1).powi(2)).sqrt();
        let dist_cells = if cell_size > 0.0 { d / cell_size } else { d };
        let level = light_illumination(light, dist_cells);
        if level > best.level {
            best = CellLight {
                level,
                tint: light.color,
            };
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lamp() -> Light {
        Light {
            pos: (0.0, 0.0),
            color: 0xFFEEAA,
            intensity: 1.0,
            bright_radius: 2.0,
            dim_radius: 6.0,
            falloff: Falloff::Linear,
            enabled: true,
        }
    }

    #[test]
    fn falloff_curves_and_radii() {
        let l = lamp();
        assert_eq!(light_illumination(&l, 0.0), 1.0); // center: full
        assert_eq!(light_illumination(&l, 2.0), 1.0); // bright edge: full (continuous)
        assert_eq!(light_illumination(&l, 7.0), 0.0); // beyond dim radius: dark
                                                      // Linear: halfway across (bright=2 → dim=6), dist=4 → t=0.5 → 0.5
        assert!((light_illumination(&l, 4.0) - 0.5).abs() < 1e-9);
        // Quadratic falls off faster than linear at the same distance.
        let q = Light {
            falloff: Falloff::Quadratic,
            ..lamp()
        };
        assert!(light_illumination(&q, 4.0) < light_illumination(&l, 4.0));
        // None: flat dim-band step across (bright, dim].
        let n = Light {
            falloff: Falloff::None,
            ..lamp()
        };
        assert!((light_illumination(&n, 4.0) - 0.5).abs() < 1e-9);
        assert_eq!(light_illumination(&n, 1.0), 1.0); // still full inside bright
                                                      // Disabled / zero dim radius contribute nothing.
        assert_eq!(
            light_illumination(
                &Light {
                    enabled: false,
                    ..lamp()
                },
                0.0
            ),
            0.0
        );
        assert_eq!(
            light_illumination(
                &Light {
                    dim_radius: 0.0,
                    ..lamp()
                },
                0.0
            ),
            0.0
        );
    }

    #[test]
    fn band_lookup_and_floor_are_fail_closed() {
        let bands = sorted_bands(default_bands());
        // brightest-first: bright(0.67) → dim(0.34) → dark(0.0)
        assert_eq!(bands[0].name, "bright");
        assert_eq!(band_index(&bands, 0.9), 0); // bright
        assert_eq!(band_index(&bands, 0.5), 1); // dim
        assert_eq!(band_index(&bands, 0.1), 2); // dark
                                                // floor_min: a normal-vision token (dim floor) needs >= 0.34; darkvision (dark) needs >= 0.0.
        assert_eq!(floor_min(&bands, "dim"), 0.34);
        assert_eq!(floor_min(&bands, "dark"), 0.0);
        // Unknown floor name → most restrictive (brightest band min) = under-reveal.
        assert_eq!(floor_min(&bands, "nonsense"), 0.67);
        // Empty input → defaults (never panics).
        assert_eq!(sorted_bands(vec![])[0].name, "bright");
    }

    #[test]
    fn fail_closed_on_degenerate_band_input() {
        // floor_min on an empty slice → the fail-closed maximum (1.0): nothing satisfies >= 1.0
        // except a fully-lit cell, so an unset gradation under-reveals.
        assert_eq!(floor_min(&[], "dim"), 1.0);
        // A non-finite band is dropped deterministically; an all-NaN input falls back to defaults.
        let nan = Band {
            name: "bad".into(),
            min_illumination: f64::NAN,
        };
        assert_eq!(sorted_bands(vec![nan])[0].name, "bright");
        // A finite band survives alongside a dropped NaN band.
        let mixed = sorted_bands(vec![
            Band {
                name: "bad".into(),
                min_illumination: f64::NAN,
            },
            Band {
                name: "ok".into(),
                min_illumination: 0.5,
            },
        ]);
        assert_eq!(mixed.len(), 1);
        assert_eq!(mixed[0].name, "ok");
    }

    #[test]
    fn cell_illumination_takes_max_and_respects_occlusion() {
        let l = lamp(); // at origin, bright 2 / dim 6 cells, intensity 1, linear
                        // No env, cell at the light center, cell_size 100 (world units per cell) → full + light tint.
        let c = cell_illumination(
            (0.0, 0.0),
            0.0,
            0x000000,
            std::slice::from_ref(&l),
            &[vec![]],
            100.0,
        );
        assert_eq!(c.level, 1.0);
        assert_eq!(c.tint, 0xFFEEAA);
        // Environment ambient alone when no light reaches: env wins, env tint.
        let far = cell_illumination(
            (10_000.0, 0.0),
            0.3,
            0x0A0E1A,
            std::slice::from_ref(&l),
            &[vec![]],
            100.0,
        );
        assert_eq!(far.level, 0.3);
        assert_eq!(far.tint, 0x0A0E1A);
        // Max-compose: a brighter env beats a dim faraway light contribution.
        let near = cell_illumination(
            (400.0, 0.0),
            0.6,
            0x0A0E1A,
            std::slice::from_ref(&l),
            &[vec![]],
            100.0,
        ); // 4 cells → 0.5
        assert_eq!(near.level, 0.6); // env 0.6 > light 0.5 (no over-brightening)
                                     // Occlusion: a light whose polygon excludes the cell contributes nothing.
        let occluded_poly = vec![(1000.0, 1000.0), (1001.0, 1000.0), (1001.0, 1001.0)]; // tiny, far away
        let occ = cell_illumination((0.0, 0.0), 0.0, 0x000000, &[l], &[occluded_poly], 100.0);
        assert_eq!(occ.level, 0.0); // cell center not inside the light's poly → dark
    }

    #[test]
    fn non_finite_dim_radius_contributes_nothing() {
        let l = Light {
            dim_radius: f64::NAN,
            ..lamp()
        };
        assert_eq!(light_illumination(&l, 0.0), 0.0);
        let i = Light {
            dim_radius: f64::INFINITY,
            ..lamp()
        };
        assert_eq!(light_illumination(&i, 1.0), 0.0);
    }
}
