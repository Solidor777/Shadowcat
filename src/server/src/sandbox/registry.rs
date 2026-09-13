//! Compiles every enabled installed module's declared validators once, cached and
//! invalidated the same way `crate::modules::ModuleScanCache` invalidates (mtime-keyed).
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use dashmap::DashMap;
use uuid::Uuid;

use super::runtime::CompiledValidator;

/// A module's declared validators as one scan outcome: every entry that compiled, plus the
/// first compile diagnostic if any failed (one bad `.wasm` never blocks the module's other
/// validators — each declared entry is independent), recorded per-module for
/// `ModuleManager`'s load-status display.
///
/// # Examples
///
/// ```
/// use shadowcat::sandbox::registry::ModuleLoadResult;
///
/// let result = ModuleLoadResult::default();
/// assert!(result.by_doc_type.is_empty());
/// assert!(result.load_error.is_none());
/// ```
#[derive(Debug, Clone, Default)]
pub struct ModuleLoadResult {
    /// `doc_type -> compiled validator`, only entries that compiled successfully.
    pub by_doc_type: BTreeMap<String, CompiledValidator>,
    /// The FIRST compile failure encountered for this module's declared validators, if any.
    pub load_error: Option<String>,
}

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
    /// `module_id` → that module's scan outcome (compiled validators + load diagnostic).
    by_module: BTreeMap<String, ModuleLoadResult>,
    /// Per-(world, module) consecutive-fault counter, shared with the `ValidatorRegistryCache`
    /// that produced this registry (and every other registry that cache has ever produced) —
    /// see `ValidatorRegistryCache::faults`'s own doc for why continuity survives a rescan.
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
        self.by_module.get(module_id)?.by_doc_type.get(doc_type)
    }

    /// This module's compile diagnostic, if its declared validators failed to load — shown by
    /// `GET /api/modules`'s `validator_load_error`.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::sandbox::registry::ValidatorRegistry;
    ///
    /// let registry = ValidatorRegistry::default();
    /// assert!(registry.load_error_for("example-module").is_none());
    /// ```
    pub fn load_error_for(&self, module_id: &str) -> Option<&str> {
        self.by_module
            .get(module_id)
            .and_then(|r| r.load_error.as_deref())
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
    /// own non-`Fault` verdict, and externally (via `ValidatorRegistryCache::reset_faults`)
    /// once a caller finishes disabling the module (so a future re-enable starts clean).
    pub(crate) fn reset_faults(&self, world: Uuid, module: &str) {
        self.faults.remove(&(world, module.to_string()));
    }

    /// Compile every declared validator of every module in `installed` (already scanned by
    /// the caller — `ValidatorRegistryCache::get_or_scan` walks the modules dir exactly ONCE
    /// per cache miss and shares the result between this compile pass and its own mtime
    /// bookkeeping). A module whose `.wasm` fails to read or fails to compile as a
    /// valid WASM module gets `load_error: Some(..)` and NO entries — the module itself still
    /// scans/loads normally (fail-open discovery, the same posture `scan_installed_modules`
    /// already takes for a malformed manifest). `faults` is handed through unchanged from the
    /// `ValidatorRegistryCache` that calls this, so a rescan never resets an in-flight streak.
    fn scan(
        installed: &[crate::modules::InstalledModule],
        modules_dir: &Path,
        faults: Arc<DashMap<(Uuid, String), u32>>,
    ) -> Self {
        let mut by_module = BTreeMap::new();
        for installed in installed {
            if installed.validators.is_empty() {
                continue;
            }
            let module_dir = modules_dir.join(&installed.id);
            let mut result = ModuleLoadResult::default();
            for decl in &installed.validators {
                match compile_one(&module_dir, &installed.id, decl) {
                    Ok(compiled) => {
                        result.by_doc_type.insert(decl.doc_type.clone(), compiled);
                    }
                    Err(e) => {
                        tracing::warn!(module = %installed.id, doc_type = %decl.doc_type, error = %e, "validator failed to load");
                        if result.load_error.is_none() {
                            result.load_error = Some(e);
                        }
                    }
                }
            }
            by_module.insert(installed.id.clone(), result);
        }
        Self { by_module, faults }
    }
}

#[cfg(test)]
impl ValidatorRegistry {
    /// Test-only constructor: a registry whose `validator_for` resolves exactly the given
    /// `(module_id, doc_type)` pairs to the given compiled validators, sharing a fresh
    /// fault-counter map — the same shape `ValidatorRegistryCache::get_or_scan` builds from a
    /// real scan, without touching the filesystem.
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
        let mut by_module: BTreeMap<String, ModuleLoadResult> = BTreeMap::new();
        for (module_id, doc_type, validator) in entries {
            by_module
                .entry(module_id.to_string())
                .or_default()
                .by_doc_type
                .insert(doc_type.to_string(), validator);
        }
        Self { by_module, faults }
    }
}

/// Reads, traversal-checks and compiles one `ValidatorDecl` under `module_id` — the INSTALLED
/// module's own id (`installed.id` in `ValidatorRegistry::scan`'s loop), never `decl.doc_type`:
/// `doc_type` names only which document type this validator judges, and two different modules
/// may both declare a validator for the SAME `doc_type`, so `doc_type` alone cannot identify
/// which module a fault belongs to. `MAX_WASM_BYTES` (4 MiB) bounds compile time; the traversal
/// check is the SAME `is_strictly_within` boundary `http::module_routes::serve_module_file`
/// enforces at request time, applied here at scan time since this reads the file directly
/// rather than serving it.
fn compile_one(
    module_dir: &Path,
    module_id: &str,
    decl: &crate::modules::ValidatorDecl,
) -> Result<CompiledValidator, String> {
    const MAX_WASM_BYTES: u64 = 4 * 1024 * 1024;
    let module_dir_canon = std::fs::canonicalize(module_dir).map_err(|e| e.to_string())?;
    let candidate = module_dir.join(&decl.wasm);
    let candidate_canon = std::fs::canonicalize(&candidate).map_err(|e| e.to_string())?;
    if !crate::http::module_routes::is_strictly_within(&candidate_canon, &module_dir_canon) {
        return Err("wasm path escapes the module's own folder".to_string());
    }
    let meta = std::fs::metadata(&candidate_canon).map_err(|e| e.to_string())?;
    if meta.len() > MAX_WASM_BYTES {
        return Err(format!("validator wasm exceeds {MAX_WASM_BYTES} bytes"));
    }
    let bytes = std::fs::read(&candidate_canon).map_err(|e| e.to_string())?;
    CompiledValidator::compile(module_id, &bytes).map_err(|e| e.to_string())
}

/// Caches `ValidatorRegistry::scan`'s result the same way `crate::modules::ModuleScanCache`
/// caches a manifest scan: invalidated by the modules directory's own mtime, each cached
/// module's `module.json` mtime, AND each declared validator's `.wasm` file mtime (an
/// in-place wasm swap — the natural update channel for an already-installed module — changes
/// no manifest and bumps no directory mtime, so without the third signal a swapped validator
/// would serve its stale compiled form until restart). One instance per `SqliteRepository`,
/// NOT shared with `ModuleScanCache` — the caches invalidate on overlapping signals but hold
/// structurally different payloads (raw manifests vs compiled `wasmi` modules) and are read
/// from different layers (`ws`/`http` vs `data`).
///
/// # Examples
///
/// ```
/// use shadowcat::sandbox::registry::ValidatorRegistryCache;
///
/// let cache = ValidatorRegistryCache::default();
/// let registry = cache.get_or_scan(std::path::Path::new("no-such-modules-dir"));
/// assert!(registry.validator_for("example-module", "actor").is_none());
/// ```
#[derive(Default)]
pub struct ValidatorRegistryCache {
    /// The current cached scan, if any — cloned out under the lock, validated and replaced
    /// outside it (the same lock discipline `crate::modules::ModuleScanCache` documents).
    entry: Mutex<Option<Arc<CachedEntry>>>,
    /// Per-(world, module) consecutive-fault counter, OUTLIVING any single scan — a rescan
    /// (a module installed/removed/changed) rebuilds `entry`'s compiled-module map but must
    /// never reset an in-flight fault streak, so this lives on the cache itself and is handed,
    /// by reference, to every `ValidatorRegistry` `get_or_scan` produces.
    faults: Arc<DashMap<(Uuid, String), u32>>,
}

/// One cached scan plus the mtimes it was valid against.
struct CachedEntry {
    /// `modules_dir`'s own mtime at scan time.
    dir_mtime: std::time::SystemTime,
    /// Each cached module's id -> its `module.json`'s mtime at scan time.
    manifest_mtimes: BTreeMap<String, std::time::SystemTime>,
    /// Every declared validator's `.wasm` path -> its mtime at scan time (see
    /// `ValidatorRegistryCache`'s doc for the in-place-swap case this catches).
    wasm_mtimes: BTreeMap<std::path::PathBuf, std::time::SystemTime>,
    /// The scan result itself.
    registry: Arc<ValidatorRegistry>,
}

impl ValidatorRegistryCache {
    /// Clears `(world, module)`'s consecutive-fault counter — a cheap, synchronous `DashMap`
    /// removal, safe to call directly from an async context without `spawn_blocking`. Called
    /// after a caller finishes disabling a faulting module, so a future re-enable starts clean.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::sandbox::registry::ValidatorRegistryCache;
    ///
    /// // Resetting a module with no recorded faults is a no-op.
    /// ValidatorRegistryCache::default().reset_faults(uuid::Uuid::nil(), "example-module");
    /// ```
    pub fn reset_faults(&self, world: Uuid, module: &str) {
        self.faults.remove(&(world, module.to_string()));
    }

    /// Returns the cached registry if `modules_dir`'s own mtime, every cached module's
    /// `module.json` mtime, and every declared validator's `.wasm` mtime are unchanged since
    /// the cache was populated; otherwise recompiles (`ValidatorRegistry::scan`) and replaces
    /// the cache. Blocking filesystem I/O — call only from within `spawn_blocking`.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::sandbox::registry::ValidatorRegistryCache;
    ///
    /// let cache = ValidatorRegistryCache::default();
    /// let registry = cache.get_or_scan(std::path::Path::new("no-such-modules-dir"));
    /// assert!(registry.validator_for("anything", "item").is_none());
    /// ```
    pub fn get_or_scan(&self, modules_dir: &Path) -> Arc<ValidatorRegistry> {
        let dir_mtime = std::fs::metadata(modules_dir)
            .and_then(|m| m.modified())
            .ok();
        let current = self
            .entry
            .lock()
            .expect("validator registry cache mutex poisoned")
            .clone();
        if let (Some(cached), Some(dir_mtime)) = (current.as_ref(), dir_mtime) {
            if cached.dir_mtime == dir_mtime
                && cached.manifest_mtimes.iter().all(|(id, mtime)| {
                    std::fs::metadata(modules_dir.join(id).join("module.json"))
                        .and_then(|m| m.modified())
                        .ok()
                        == Some(*mtime)
                })
                && cached.wasm_mtimes.iter().all(|(path, mtime)| {
                    std::fs::metadata(path).and_then(|m| m.modified()).ok() == Some(*mtime)
                })
            {
                return cached.registry.clone();
            }
        }
        // ONE directory walk per cache miss, shared between the compile pass and the
        // mtime bookkeeping below.
        let installed = crate::modules::scan_installed_modules(modules_dir);
        let registry = Arc::new(ValidatorRegistry::scan(
            &installed,
            modules_dir,
            self.faults.clone(),
        ));
        let manifest_mtimes = installed
            .iter()
            .filter_map(|m| {
                std::fs::metadata(modules_dir.join(&m.id).join("module.json"))
                    .and_then(|meta| meta.modified())
                    .ok()
                    .map(|mt| (m.id.clone(), mt))
            })
            .collect();
        let wasm_mtimes = installed
            .iter()
            .flat_map(|m| {
                m.validators
                    .iter()
                    .map(move |d| modules_dir.join(&m.id).join(&d.wasm))
            })
            .filter_map(|p| {
                std::fs::metadata(&p)
                    .and_then(|meta| meta.modified())
                    .ok()
                    .map(|mt| (p, mt))
            })
            .collect();
        // A missing/unreadable modules_dir yields no dir_mtime; fall back to UNIX_EPOCH so the
        // cache is still populated but never spuriously "matches" a later, real directory at
        // the same path (mirroring `crate::modules::ModuleScanCache`'s own fallback).
        let fresh = Arc::new(CachedEntry {
            dir_mtime: dir_mtime.unwrap_or(std::time::SystemTime::UNIX_EPOCH),
            manifest_mtimes,
            wasm_mtimes,
            registry: registry.clone(),
        });
        *self
            .entry
            .lock()
            .expect("validator registry cache mutex poisoned") = Some(fresh);
        registry
    }
}

#[cfg(test)]
mod tests;
