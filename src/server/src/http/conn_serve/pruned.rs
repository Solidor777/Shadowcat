//! This module is a registry of weak references that frees the entries of dropped items.
//!
//! The algorithm is amortised pruning. An insert first checks whether the vector has grown past
//! `max(2 * live_at_last_prune, PRUNE_FLOOR)` and, if so, retains only the entries that still
//! upgrade and records that live count. A prune costs O(len) and is paid for by the more than
//! `len / 2` inserts since the previous prune, so an insert is amortised O(1); the vector never
//! exceeds `max(2 * live_at_last_prune, PRUNE_FLOOR) + 1` entries, whatever the insert rate.
//! Chosen over pruning on every insert (O(len) each) and over never pruning (length grows with
//! the number of items ever created, which for a per-request registry is unbounded).
//!
//! INVARIANT: a prune removes only entries whose item has been dropped, so a long-lived item
//! stays registered through every prune.

use std::sync::{Arc, Weak};

/// The length below which a registry never prunes.
pub(super) const PRUNE_FLOOR: usize = 64;

/// Weak references to live items, pruned on insert. See the module documentation.
pub(super) struct Pruned<T> {
    /// The registered references, live and dead.
    items: Vec<Weak<T>>,
    /// How many entries were live after the most recent prune.
    live_at_last_prune: usize,
}

impl<T> Pruned<T> {
    /// An empty registry.
    pub(super) fn new() -> Self {
        Self {
            items: Vec::new(),
            live_at_last_prune: 0,
        }
    }

    /// Registers `item`, first pruning dead entries when the vector has outgrown its bound.
    pub(super) fn push(&mut self, item: Weak<T>) {
        if self.items.len() > (2 * self.live_at_last_prune).max(PRUNE_FLOOR) {
            self.items.retain(|w| w.strong_count() > 0);
            self.live_at_last_prune = self.items.len();
        }
        self.items.push(item);
    }

    /// The items that are still alive.
    pub(super) fn live(&self) -> Vec<Arc<T>> {
        self.items.iter().filter_map(Weak::upgrade).collect()
    }

    /// The number of registered references, live and dead.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.items.len()
    }
}

#[cfg(test)]
mod tests;
