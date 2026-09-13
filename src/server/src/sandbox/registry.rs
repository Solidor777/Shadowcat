//! Compiles every enabled installed module's declared validators once, cached and
//! invalidated the same way `crate::modules::ModuleScanCache` invalidates (mtime-keyed).
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::BTreeMap;
use std::sync::Arc;

use dashmap::DashMap;
use uuid::Uuid;

use super::runtime::CompiledValidator;

/// Every installed module's compiled validator set, keyed by module id, then `doc_type`.
///
/// # Examples
///
/// ```
/// use shadowcat::sandbox::registry::ValidatorRegistry;
///
/// // A repository with no `modules_dir` wired holds an empty, accept-everything registry.
/// let registry = ValidatorRegistry::default();
/// assert!(registry.validator_for("example-module", "actor").is_none());
/// ```
#[derive(Default)]
pub struct ValidatorRegistry {
    /// `module_id` → `doc_type` → the compiled validator, only entries that compiled.
    compiled: BTreeMap<String, BTreeMap<String, CompiledValidator>>,
    /// Per-(world, module) consecutive-fault counter, shared with every `ValidatorRegistry` a
    /// `ValidatorRegistryCache` hands out across a rescan — the counter must survive a
    /// rescan that only changes WHICH modules are compiled, never reset a fault streak in
    /// progress.
    faults: Arc<DashMap<(Uuid, String), u32>>,
}

impl ValidatorRegistry {
    /// The compiled validator `module_id` declares for `doc_type`, if any and if it compiled.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::sandbox::registry::ValidatorRegistry;
    ///
    /// let registry = ValidatorRegistry::default();
    /// assert!(registry.validator_for("example-module", "actor").is_none());
    /// ```
    pub fn validator_for(&self, module_id: &str, doc_type: &str) -> Option<&CompiledValidator> {
        self.compiled.get(module_id).and_then(|m| m.get(doc_type))
    }

    /// Records one consecutive sandbox fault for `(world, module)`, returning the new streak
    /// length. Maintained entirely by `sandbox::validate_document` — this registry never
    /// faults on its own.
    pub(crate) fn record_fault(&self, world: Uuid, module: &str) -> u32 {
        let mut entry = self.faults.entry((world, module.to_string())).or_insert(0);
        *entry += 1;
        *entry
    }

    /// Resets `(world, module)`'s consecutive-fault counter to zero — called on that module's
    /// own non-`Fault` verdict, and externally once a caller finishes disabling the module
    /// (so a future re-enable starts clean).
    pub(crate) fn reset_faults(&self, world: Uuid, module: &str) {
        self.faults.remove(&(world, module.to_string()));
    }
}

#[cfg(test)]
impl ValidatorRegistry {
    /// Test-only constructor: a registry whose `validator_for` resolves exactly the given
    /// `(module_id, doc_type)` pairs to the given compiled validators, sharing a fresh
    /// fault-counter map.
    pub(crate) fn for_test(entries: Vec<(&str, &str, CompiledValidator)>) -> Self {
        Self::for_test_with_faults(entries, Arc::default())
    }

    /// As `for_test`, but sharing the given fault-counter map — lets a test drive two
    /// registries (e.g. a faulting one and an accepting one for the same module id) against
    /// the same underlying streak, mirroring how a rescan hands out a fresh
    /// `ValidatorRegistry` sharing the cache's one persistent counter.
    pub(crate) fn for_test_with_faults(
        entries: Vec<(&str, &str, CompiledValidator)>,
        faults: Arc<DashMap<(Uuid, String), u32>>,
    ) -> Self {
        let mut compiled: BTreeMap<String, BTreeMap<String, CompiledValidator>> = BTreeMap::new();
        for (module_id, doc_type, validator) in entries {
            compiled
                .entry(module_id.to_string())
                .or_default()
                .insert(doc_type.to_string(), validator);
        }
        Self { compiled, faults }
    }
}
