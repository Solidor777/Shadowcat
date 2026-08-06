//! Illumination field + gradation banding. Pure, engine-owned, server-authoritative.
//! Clean-room: standard radial light falloff plus threshold banding of a
//! continuous [0,1] illumination field. No proprietary VTT/engine source consulted.
//!
//! Mirrors the client `light-gradation`/`light`/`vision-modes` shapes in the `scene-docs` module; the server
//! stays structural-only (callers parse documents and pass these plain structs).

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

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

/// A placed light's photometric inputs. Radii are in GRID CELLS; `color` is packed `0xRRGGBB`.
/// Mirrors the client `LightEngine`.
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
    /// Band name (matched against `VisionMode::illumination_floor`).
    pub name: String,
    /// INVARIANT: must be finite and in [0,1]; non-finite values are dropped by `sorted_bands`.
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

/// A composed per-cell illumination result: a [0,1] `level` and a packed-RGB `tint` (the dominant
/// contributor's color; `0x000000` when only an unset environment contributes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellLight {
    /// Composed illumination level, `[0, 1]`.
    pub level: f64,
    /// Dominant contributor's packed `0xRRGGBB` color.
    pub tint: u32,
}

/// Maps arc-length `d` along the perimeter of the rectangle `(0,0)–(w,h)` to a boundary point,
/// walking `top → right → bottom → left`. `d` is wrapped into `[0, 2(w+h))`.
fn perimeter_point(w: f64, h: f64, d: f64) -> P {
    let perim = 2.0 * (w + h);
    let d = d.rem_euclid(perim);
    if d < w {
        (d, 0.0)
    } else if d < w + h {
        (w, d - w)
    } else if d < 2.0 * w + h {
        (w - (d - (w + h)), h)
    } else {
        (0.0, h - (d - (2.0 * w + h)))
    }
}

/// Boundary-projected environment-light occlusion polygons. Environment light enters the scene
/// from OUTSIDE its boundary; a cell is lit iff an unobstructed line reaches it from some point on
/// the scene-bounds rectangle past the `blocksLight` walls. The rectangle perimeter is sampled at
/// ~one point per grid-unit (clamped to `[4, MAX_ENV_LIGHT_SAMPLES]`), and each sample's
/// visibility polygon is computed with the SAME `vision::visibility_polygon` primitive placed
/// lights and vision use (never a second, forked occlusion computation). A cell is environment-lit
/// iff it lies inside ANY sample's polygon (composed by `env_lit`).
///
/// `bounds_grid` is the scene bounds in GRID units (`ResolvedScene.bounds`); `cell_size` is scene
/// units per cell, so the rectangle in scene units is `(0,0)–(width×cell, height×cell)`. The
/// raycast bound is that rectangle expanded by one cell so boundary samples sit strictly inside it.
/// Fail-closed: non-finite or non-positive bounds/`cell_size` ⇒ empty (environment reaches
/// nothing — under-reveal, never over-reveal). The boundary itself never occludes (only interior
/// `blocksLight` walls do): light enters freely across the scene edge.
pub fn env_light_polys(
    bounds_grid: (f64, f64),
    cell_size: f64,
    light_walls: &[vision::Seg],
) -> Vec<Vec<P>> {
    let (wg, hg) = bounds_grid;
    if !wg.is_finite()
        || !hg.is_finite()
        || wg <= 0.0
        || hg <= 0.0
        || !cell_size.is_finite()
        || cell_size <= 0.0
    {
        return Vec::new();
    }
    let w = wg * cell_size;
    let h = hg * cell_size;
    let n = (2.0 * (wg + hg)).round() as usize;
    let n = n.clamp(4, MAX_ENV_LIGHT_SAMPLES);
    let margin = cell_size.max(1.0);
    let bound = vision::Rect {
        minx: -margin,
        miny: -margin,
        maxx: w + margin,
        maxy: h + margin,
    };
    let perim = 2.0 * (w + h);
    (0..n)
        .map(|i| {
            let d = (i as f64) / (n as f64) * perim;
            vision::visibility_polygon(perimeter_point(w, h, d), light_walls, bound)
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

/// Compose illumination at a cell center from a boundary-projected environment ambient plus each
/// light, taking the MAX contributor (no over-brightening); `tint` follows the dominant
/// contributor.
/// `lit_polys[k]` is `lights[k]`'s `blocksLight` visibility polygon — a light contributes only if the
/// cell center lies inside it (an EMPTY polygon means "no occluder computed" → never occludes).
/// `env_polys` are the scene-boundary visibility polygons (`env_light_polys`): the environment
/// ambient reaches this cell only if it is `env_lit` (inside some boundary polygon), so a
/// `blocksLight`-sealed interior receives no ambient. This is strictly NARROWING: the occluded
/// environment base is `0 ≤ env_intensity`, so the composed `level` is `≤` the pre-occlusion
/// flat-floor level at every cell — visibility can only shrink, never widen.
/// `cell_size` is world units per cell (light radii are in cells, so distance is divided by it);
/// CALLER PRECONDITION: `cell_size` must be positive — a non-positive value is a caller error; the
/// upstream caller guards it (a release-build fallback avoids division-by-zero but the value is wrong).
/// `env_intensity` must be finite; the document→settings resolver clamps it to `[0,1]`.
/// Tie-break: ties (equal `level`) keep the earlier contributor — environment beats all lights at
/// equal level, and a lower-index light beats a higher-index one.
pub fn cell_illumination(
    center: P,
    env_intensity: f64,
    env_color: u32,
    lights: &[Light],
    lit_polys: &[Vec<P>],
    env_polys: &[Vec<P>],
    cell_size: f64,
) -> CellLight {
    debug_assert!(
        cell_size > 0.0,
        "INVARIANT: cell_size must be positive; light radii are in cells"
    );
    debug_assert!(env_intensity.is_finite(), "env_intensity must be finite");
    // Environment ambient is now boundary-projected and blocksLight-occluded (see `env_polys`).
    let env_reaches = env_intensity > 0.0 && env_lit(env_polys, center);
    let mut best = CellLight {
        level: if env_reaches {
            env_intensity.clamp(0.0, 1.0)
        } else {
            0.0
        },
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

    /// A single boundary polygon covering the whole plane: an open scene where the environment
    /// reaches every cell (the pre-occlusion flat-floor behavior, for tests that don't exercise
    /// env occlusion).
    fn open_env() -> Vec<Vec<P>> {
        vec![vec![
            (-1.0e9, -1.0e9),
            (1.0e9, -1.0e9),
            (1.0e9, 1.0e9),
            (-1.0e9, 1.0e9),
        ]]
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
            &[],
            100.0,
        );
        assert_eq!(c.level, 1.0);
        assert_eq!(c.tint, 0xFFEEAA);
        // Environment ambient alone when no light reaches (open scene → env reaches): env wins.
        let far = cell_illumination(
            (10_000.0, 0.0),
            0.3,
            0x0A0E1A,
            std::slice::from_ref(&l),
            &[vec![]],
            &open_env(),
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
            &open_env(),
            100.0,
        ); // 4 cells → 0.5
        assert_eq!(near.level, 0.6); // env 0.6 > light 0.5 (no over-brightening)
                                     // Occlusion: a light whose polygon excludes the cell contributes nothing.
        let occluded_poly = vec![(1000.0, 1000.0), (1001.0, 1000.0), (1001.0, 1001.0)]; // tiny, far away
        let occ = cell_illumination(
            (0.0, 0.0),
            0.0,
            0x000000,
            &[l],
            &[occluded_poly],
            &[],
            100.0,
        );
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

    #[test]
    fn missing_polygon_does_not_occlude_and_brighter_light_wins() {
        let dim = Light {
            intensity: 0.4,
            bright_radius: 1.0,
            dim_radius: 2.0,
            ..lamp()
        };
        let bright = Light {
            pos: (400.0, 0.0),
            color: 0x00FF00,
            intensity: 1.0,
            bright_radius: 3.0,
            dim_radius: 6.0,
            ..lamp()
        };
        // Two lights, only ONE polygon provided → the second light has no entry in lit_polys
        // (index past end) → fail-open: it still contributes. The cell sits at the bright light's
        // center, so the brighter light wins the MAX compose and its tint is taken.
        let c = cell_illumination(
            (400.0, 0.0),
            0.0,
            0x000000,
            &[dim, bright],
            &[vec![]],
            &[],
            100.0,
        );
        assert_eq!(c.level, 1.0);
        assert_eq!(c.tint, 0x00FF00);
    }

    #[test]
    fn env_light_polys_open_scene_reaches_every_interior_cell() {
        // No walls: every boundary sample sees the whole scene, so every interior point is env-lit.
        let polys = env_light_polys((5.0, 5.0), 100.0, &[]);
        assert!(!polys.is_empty());
        for p in [(50.0, 50.0), (250.0, 250.0), (450.0, 450.0), (10.0, 490.0)] {
            assert!(env_lit(&polys, p), "open scene lights interior point {p:?}");
        }
    }

    #[test]
    fn env_light_polys_open_scene_equals_global_illumination_at_the_sample_cap() {
        // bounds (100,100): raw n = round(2*(100+100)) = 400, clamped to MAX_ENV_LIGHT_SAMPLES
        // (256) — confirm the clamp actually engages (one polygon per sample).
        let polys = env_light_polys((100.0, 100.0), 100.0, &[]);
        assert_eq!(polys.len(), MAX_ENV_LIGHT_SAMPLES);
        // No walls: even capped at 256 samples over a 40000-unit perimeter, every interior and
        // near-boundary cell is still reached — the wall-less-equals-global-illumination
        // equivalence holds at the cap, not just for small/typical scenes.
        for p in [
            (5000.0, 5000.0), // center
            (100.0, 100.0),
            (9900.0, 9900.0),
            (100.0, 9900.0),
            (9900.0, 100.0),
            (50.0, 5000.0), // near the left edge, mid-height
            (5000.0, 50.0), // near the top edge, mid-width
        ] {
            assert!(
                env_lit(&polys, p),
                "open scene at the cap lights point {p:?}"
            );
        }
    }

    #[test]
    fn env_light_polys_seal_a_blocks_light_box_interior() {
        // A closed 4-wall box around (250,250) spanning (200,200)–(300,300): no exterior boundary
        // sample can see inside, so the interior is not env-lit; the open exterior still is.
        let walls = vec![
            vision::Seg {
                a: (200.0, 200.0),
                b: (300.0, 200.0),
            },
            vision::Seg {
                a: (300.0, 200.0),
                b: (300.0, 300.0),
            },
            vision::Seg {
                a: (300.0, 300.0),
                b: (200.0, 300.0),
            },
            vision::Seg {
                a: (200.0, 300.0),
                b: (200.0, 200.0),
            },
        ];
        let polys = env_light_polys((5.0, 5.0), 100.0, &walls);
        assert!(
            !env_lit(&polys, (250.0, 250.0)),
            "sealed interior is not env-lit"
        );
        assert!(env_lit(&polys, (50.0, 50.0)), "open exterior stays env-lit");
    }

    #[test]
    fn env_light_polys_fail_closed_on_degenerate_bounds() {
        // Degenerate bounds/cell ⇒ empty ⇒ env reaches nothing (under-reveal).
        assert!(env_light_polys((0.0, 5.0), 100.0, &[]).is_empty());
        assert!(env_light_polys((5.0, f64::NAN), 100.0, &[]).is_empty());
        assert!(env_light_polys((5.0, 5.0), 0.0, &[]).is_empty());
        assert!(
            !env_lit(&[], (10.0, 10.0)),
            "empty env_polys is fail-closed (dark)"
        );
    }

    #[test]
    fn cell_illumination_occludes_environment_outside_the_boundary_polys() {
        // env_polys that exclude the cell → no ambient; a covering set → full ambient. No lights.
        let excluding = vec![vec![(1000.0, 1000.0), (1001.0, 1000.0), (1001.0, 1001.0)]];
        let dark = cell_illumination((0.0, 0.0), 0.5, 0x0A0E1A, &[], &[], &excluding, 100.0);
        assert_eq!(
            dark.level, 0.0,
            "cell outside every boundary poly gets no ambient"
        );
        let lit = cell_illumination((0.0, 0.0), 0.5, 0x0A0E1A, &[], &[], &open_env(), 100.0);
        assert_eq!(
            lit.level, 0.5,
            "cell inside a boundary poly gets the full ambient"
        );
    }
}
