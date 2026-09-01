use super::*;

/// An origin-anchored envelope of the given world-unit spans — the shape a SQUARE scene's
/// `GridShape::world_extent` produces. These fixtures exercise `env_light_polys`'s own
/// perimeter walk and refusals on literal pixel spans, where the anchor is incidental; the
/// negative-minimum case a hex scene produces is exercised by the fixture that names it.
fn origin_extent(w: f64, h: f64) -> WorldExtent {
    WorldExtent {
        min: (0.0, 0.0),
        max: (w, h),
    }
}

fn lamp() -> Light {
    Light {
        pos: (0.0, 0.0),
        elevation: 0.0,
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
fn cell_illumination_composes_additively_and_respects_occlusion() {
    let l = lamp(); // at origin, bright 2 / dim 6 cells, intensity 1, linear
                    // No env, cell at the light center, `world_units_per_cell` 100 → full + light tint.
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
    // Additive: env 0.6 + light 0.5 (4 cells → t=0.5 linear) sums to 1.1 and saturates at 1.0.
    let near = cell_illumination(
        (400.0, 0.0),
        0.6,
        0x0A0E1A,
        std::slice::from_ref(&l),
        &[vec![]],
        &open_env(),
        100.0,
    );
    assert_eq!(near.level, 1.0);
    // Tint: (0.6×0x0A0E1A + 0.5×0xFFEEAA) / 1.1 per channel = (121.36→121, 115.82→116, 91.45→91).
    assert_eq!(near.tint, 0x79745B);
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
fn two_overlapping_dim_lights_cross_the_bright_threshold() {
    // Two 0.4-intensity lights both covering the cell at full (inside bright radius): each alone
    // sits in the dim band (0.34 ≤ 0.4 < 0.67); their sum 0.8 crosses into bright (≥ 0.67).
    let a = Light {
        color: 0xFF0000,
        intensity: 0.4,
        ..lamp()
    };
    let b = Light {
        color: 0x0000FF,
        intensity: 0.4,
        ..lamp()
    };
    let single = cell_illumination(
        (0.0, 0.0),
        0.0,
        0,
        std::slice::from_ref(&a),
        &[vec![]],
        &[],
        100.0,
    );
    assert_eq!(single.level, 0.4);
    let bands = sorted_bands(default_bands());
    assert_eq!(bands[band_index(&bands, single.level)].name, "dim");
    let both = cell_illumination((0.0, 0.0), 0.0, 0, &[a, b], &[vec![], vec![]], &[], 100.0);
    assert!((both.level - 0.8).abs() < 1e-9);
    assert_eq!(bands[band_index(&bands, both.level)].name, "bright");
    // Equal weights → the red/blue mix is pure magenta.
    assert_eq!(both.tint, 0x800080);
}

#[test]
fn tint_mixes_by_illuminance_weight() {
    // A full-strength red light and a half-strength blue light: 2:1 mix → (170, 0, 85).
    // r = (255×1.0 + 0×0.5)/1.5 = 170, b = (0×1.0 + 255×0.5)/1.5 = 85.
    let red = Light {
        color: 0xFF0000,
        ..lamp()
    };
    let blue = Light {
        color: 0x0000FF,
        intensity: 0.5,
        ..lamp()
    };
    let c = cell_illumination(
        (0.0, 0.0),
        0.0,
        0,
        &[red, blue],
        &[vec![], vec![]],
        &[],
        100.0,
    );
    assert_eq!(c.tint, 0xAA0055);
}

#[test]
fn composition_saturates_at_full_level_without_skewing_hue() {
    // Two full-intensity lights sum to 2.0 but clamp to 1.0; the tint divisor is the
    // UNSATURATED sum, so saturation brightens without shifting the mix (still equal-weight).
    let a = Light {
        color: 0xFF0000,
        ..lamp()
    };
    let b = Light {
        color: 0x00FF00,
        ..lamp()
    };
    let c = cell_illumination((0.0, 0.0), 0.0, 0, &[a, b], &[vec![], vec![]], &[], 100.0);
    assert_eq!(c.level, 1.0);
    assert_eq!(c.tint, 0x808000);
}

#[test]
fn environment_and_placed_light_compose_additively() {
    // env 0.3 + light 0.5 (4 cells → t=0.5 linear) = 0.8, below saturation.
    let c = cell_illumination(
        (400.0, 0.0),
        0.3,
        0x0A0E1A,
        std::slice::from_ref(&lamp()),
        &[vec![]],
        &open_env(),
        100.0,
    );
    assert!((c.level - 0.8).abs() < 1e-9);
}

#[test]
fn zero_contributors_is_dark_and_untinted() {
    let c = cell_illumination((0.0, 0.0), 0.0, 0x0A0E1A, &[], &[], &[], 100.0);
    assert_eq!(c.level, 0.0);
    assert_eq!(c.tint, 0);
    // A NaN-intensity light is a per-source fail-closed case: contributes nothing (and must not
    // poison the running sum — `f64::clamp` panics on NaN).
    let nan = Light {
        intensity: f64::NAN,
        ..lamp()
    };
    let c = cell_illumination((0.0, 0.0), 0.0, 0, &[nan], &[vec![]], &[], 100.0);
    assert_eq!(c.level, 0.0);
    assert_eq!(c.tint, 0);
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
fn non_finite_distance_contributes_nothing_even_for_flat_falloff() {
    // A NaN/±inf distance must zero the contribution for EVERY curve — the `None` curve ignores
    // the taper parameter, so without the guard it would light every cell of a NaN-positioned
    // light (a scene-wide fail-open).
    for falloff in [Falloff::None, Falloff::Linear, Falloff::Quadratic] {
        let l = Light { falloff, ..lamp() };
        assert_eq!(light_illumination(&l, f64::NAN), 0.0, "{falloff:?}");
        assert_eq!(light_illumination(&l, f64::INFINITY), 0.0, "{falloff:?}");
    }
}

#[test]
fn missing_polygon_does_not_occlude_and_unoccluded_light_contributes() {
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
    // center; the dim light's reach (2 cells) does not extend here, so the bright light alone
    // sets the level and tint.
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
    let polys = env_light_polys(origin_extent(500.0, 500.0), 100.0, &[]);
    assert!(!polys.is_empty());
    for p in [(50.0, 50.0), (250.0, 250.0), (450.0, 450.0), (10.0, 490.0)] {
        assert!(env_lit(&polys, p), "open scene lights interior point {p:?}");
    }
}

#[test]
fn env_light_polys_open_scene_equals_global_illumination_at_the_sample_cap() {
    // A 10000 × 10000 rectangle at cell size 100: raw n = round(perimeter / cell) = 400,
    // clamped to MAX_ENV_LIGHT_SAMPLES (256) — confirm the clamp actually engages (one
    // polygon per sample).
    let polys = env_light_polys(origin_extent(10_000.0, 10_000.0), 100.0, &[]);
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
    let polys = env_light_polys(origin_extent(500.0, 500.0), 100.0, &walls);
    assert!(
        !env_lit(&polys, (250.0, 250.0)),
        "sealed interior is not env-lit"
    );
    assert!(env_lit(&polys, (50.0, 50.0)), "open exterior stays env-lit");
}

#[test]
fn env_light_polys_fail_closed_on_degenerate_bounds() {
    // Degenerate bounds/cell ⇒ empty ⇒ env reaches nothing (under-reveal).
    assert!(env_light_polys(origin_extent(0.0, 500.0), 100.0, &[]).is_empty());
    assert!(env_light_polys(origin_extent(500.0, f64::NAN), 100.0, &[]).is_empty());
    assert!(env_light_polys(origin_extent(500.0, 500.0), 0.0, &[]).is_empty());
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
