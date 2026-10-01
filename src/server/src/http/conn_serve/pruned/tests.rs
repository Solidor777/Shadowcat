//! The registry's length bound and its survival rule, measured at every insert.

use super::*;

/// The most entries a registry may hold when `live` items are alive.
fn bound(live: usize) -> usize {
    (2 * live).max(PRUNE_FLOOR) + 1
}

/// Inserts `cycles` short-lived items next to `held` long-lived ones and returns the registry, the
/// long-lived items, and the longest the registry was after any insert.
fn churn(held: usize, cycles: usize) -> (Pruned<u32>, Vec<Arc<u32>>, usize) {
    let mut registry = Pruned::new();
    let long_lived: Vec<Arc<u32>> = (0..held).map(|i| Arc::new(i as u32)).collect();
    let mut peak = 0;
    for item in &long_lived {
        registry.push(Arc::downgrade(item));
        peak = peak.max(registry.len());
    }
    for _ in 0..cycles {
        let short = Arc::new(u32::MAX);
        registry.push(Arc::downgrade(&short));
        peak = peak.max(registry.len());
    }
    (registry, long_lived, peak)
}

#[test]
fn a_constant_live_count_bounds_the_registry_at_every_insert() {
    for held in [0, 3, 200] {
        let (registry, _long_lived, peak) = churn(held, 100_000);
        assert!(
            peak <= bound(held),
            "{held} live: the registry reached {peak} entries, over {}",
            bound(held)
        );
        assert!(registry.len() <= bound(held));
    }
}

#[test]
fn a_long_lived_entry_survives_every_prune() {
    let (registry, long_lived, _peak) = churn(3, 100_000);
    let live = registry.live();
    assert_eq!(live.len(), 3, "a prune dropped a live entry");
    for item in &long_lived {
        assert!(
            live.iter().any(|l| Arc::ptr_eq(l, item)),
            "a long-lived entry was lost"
        );
    }
}
