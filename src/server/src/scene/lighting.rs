//! Illumination field + gradation banding (M10e-2). Pure, engine-owned (ARCHITECTURE #6),
//! server-authoritative (#3). Clean-room: standard radial light falloff plus threshold banding of a
//! continuous [0,1] illumination field. No proprietary VTT/engine source consulted.
//!
//! Mirrors the client `light-gradation`/`light`/`vision-modes` shapes in scene-docs.ts; the server
//! stays structural-only (callers parse documents and pass these plain structs).

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
