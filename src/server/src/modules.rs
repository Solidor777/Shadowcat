// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;

use crate::data::document::CapabilityRequirement;

/// Minimal fields the server reads from a community `module.json`.
/// `deny_unknown_fields` is NOT used: the manifest is community-authored and
/// carries client-only fields (name, dependencies, capabilities, hooks,
/// provides, requires) the server never interprets — unknown keys are
/// forward-compatible no-ops, mirroring the client Zod schema's tolerance.
#[derive(Debug, Clone, Deserialize)]
struct ModuleManifestMirror {
    // Deserialization presence-validates id/version (a manifest missing either
    // fails parse and is skipped); the fields themselves are never read past
    // that point, since `InstalledModule::id` is the folder name, not this one.
    /// Author-declared id. Checked against the folder name at load and reported on mismatch; the
    /// folder name remains the key everywhere downstream.
    id: String,
    /// Author-declared semver. Recorded in the load-time diagnostic; engine compatibility is
    /// decided by `engines.shadowcat`, never by this.
    version: String,
    /// Declarative path-prefix → capability rules, unioned into the world's
    /// broadcast `capability_requirements` (advisory to the client only).
    #[serde(default)]
    requirements: Vec<CapabilityRequirement>,
    /// Engine-compat declaration; absent = the module can never be enabled.
    #[serde(default)]
    engines: ModuleEngines,
    /// Built entry file name relative to the install folder; default `index.js`.
    #[serde(default = "default_entry")]
    entry: String,
}

/// The `engines` object of a community `module.json`.
#[derive(Debug, Clone, Default, Deserialize)]
struct ModuleEngines {
    /// Semver range the running server version must satisfy (exact / `^` / `~` / `*`).
    shadowcat: Option<String>,
}

/// Serde default for `ModuleManifestMirror::entry`.
///
/// # Examples
///
/// ```text
/// default_entry() == "index.js"
/// ```
fn default_entry() -> String {
    "index.js".into()
}

/// One validly-discovered installed module: typed fields the server needs
/// (id, requirements, engine-compat range) plus the raw manifest JSON — served
/// byte-for-byte at `GET /api/modules` so the client's own Zod schema sees
/// every field a community author declared (dependencies, hooks, provides,
/// requires, ...), not just the subset this mirror extracts.
#[derive(Debug, Clone)]
pub struct InstalledModule {
    /// The install folder name — the routing id (`/modules/<id>/...`), distinct
    /// from (and cross-checked against, client-side, exactly as `loadModules`
    /// already cross-checks discovery-id vs the module's own declared id).
    pub id: String,
    /// The manifest's declarative capability rules (advisory; unioned into the
    /// world's broadcast `capability_requirements` where the module is enabled).
    pub requirements: Vec<CapabilityRequirement>,
    /// Declared `engines.shadowcat` range; `None` fails the enable gate closed.
    pub engines_shadowcat: Option<String>,
    /// The raw `module.json`, served byte-for-byte at `GET /api/modules`.
    pub manifest_json: serde_json::Value,
    /// Served entry URL: `/modules/<folder-id>/<entry>`.
    pub entry_url: String,
}

/// Scan `<modules_dir>/*/module.json`, parse + validate each. An invalid
/// manifest (missing/malformed `id`/`version`, or malformed JSON) is logged
/// (warn) and skipped — one broken module must not prevent startup or hide the
/// others. This fail-open-on-discovery behavior is a deliberate design choice,
/// separate from the server's purely structural authority over a
/// community-authored manifest body (`manifest_json` is served byte-for-byte,
/// never semantically interpreted). A missing `modules_dir`
/// (nothing installed yet) yields an empty list, not an error. Deterministic
/// id-sorted order.
///
/// # Examples
///
/// ```
/// use shadowcat::modules::scan_installed_modules;
///
/// let none = scan_installed_modules(std::path::Path::new("no-such-modules-dir"));
/// assert!(none.is_empty());
/// ```
pub fn scan_installed_modules(modules_dir: &Path) -> Vec<InstalledModule> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(modules_dir) {
        Ok(e) => e,
        Err(_) => return out, // absent modules dir = no modules installed
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let manifest_path = dir.join("module.json");
        if !manifest_path.is_file() {
            continue; // no module.json: not a module folder
        }
        let contents = match std::fs::read_to_string(&manifest_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(path = %manifest_path.display(), error = %e, "module.json exists but could not be read; skipping");
                continue;
            }
        };
        let raw: serde_json::Value = match serde_json::from_str(&contents) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(dir = %dir.display(), error = %e, "module.json is not valid JSON; skipping");
                continue;
            }
        };
        let mirror: ModuleManifestMirror = match serde_json::from_value(raw.clone()) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(dir = %dir.display(), error = %e, "module.json failed validation; skipping");
                continue;
            }
        };
        let folder_id = match dir.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        // The folder name is the module's identity everywhere downstream, so a manifest declaring
        // a different id is not merely cosmetic: every reference an author writes against their
        // own declared id resolves to nothing. Reported rather than rejected, because the declared
        // id has never been authoritative and refusing to load would break modules that work.
        if mirror.id != folder_id {
            tracing::warn!(
                dir = %dir.display(),
                declared = %mirror.id,
                folder = %folder_id,
                "module.json declares an id that is not the folder name; the folder name is used"
            );
        }
        // The declared version is not authoritative for anything the server decides — engine
        // compatibility is gated by `engines.shadowcat`, not by this — but it is the only record
        // of what an operator actually has installed, and a version mismatch against what they
        // believe they deployed is otherwise invisible at runtime.
        tracing::debug!(id = %folder_id, version = %mirror.version, "module loaded");
        let entry_url = format!("/modules/{folder_id}/{}", mirror.entry);
        out.push(InstalledModule {
            id: folder_id,
            requirements: mirror.requirements,
            engines_shadowcat: mirror.engines.shadowcat,
            manifest_json: raw,
            entry_url,
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// Minimal semver range matcher mirroring the client's `satisfies`
/// (exact / `^` / `~` / `*`) — both sides must
/// agree on `engines.shadowcat` compatibility (enable-time here, load-time
/// there), so the tiny algorithm is duplicated intentionally rather than
/// shared across the Rust/TS boundary. Fails closed (false) on a malformed
/// version or range rather than panicking.
///
/// # Examples
///
/// ```
/// use shadowcat::modules::semver_satisfies;
///
/// assert!(semver_satisfies("0.1.4", "^0.1.0")); // 0.x line: minor is breaking
/// assert!(!semver_satisfies("0.2.0", "^0.1.0"));
/// assert!(semver_satisfies("1.9.0", "~1.9.0"));
/// assert!(!semver_satisfies("not-semver", "*")); // "*" still needs a valid version
/// ```
pub fn semver_satisfies(version: &str, range: &str) -> bool {
    fn parse(v: &str) -> Option<(u64, u64, u64)> {
        let mut parts = v.trim().split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some((major, minor, patch))
    }
    let r = range.trim();
    let Some(v) = parse(version) else {
        return false;
    };
    if r == "*" {
        return true;
    }
    if let Some(rest) = r.strip_prefix('^') {
        let Some(b) = parse(rest) else { return false };
        // Caret's upper bound is set by the LEFTMOST non-zero component
        // (npm-semver semantics): major>0 -> next major is breaking;
        // major==0 with minor>0 -> next minor is breaking (0.x.y line);
        // major==0 and minor==0 -> next patch is breaking (0.0.y line).
        if b.0 > 0 {
            return v.0 == b.0 && v >= b;
        }
        if b.1 > 0 {
            return v.0 == b.0 && v.1 == b.1 && v >= b;
        }
        return v.0 == b.0 && v.1 == b.1 && v.2 == b.2;
    }
    if let Some(rest) = r.strip_prefix('~') {
        let Some(b) = parse(rest) else { return false };
        return v.0 == b.0 && v.1 == b.1 && v >= b;
    }
    let Some(b) = parse(r) else { return false };
    v == b
}

/// Engine-compat gate: the running server's `CARGO_PKG_VERSION` must satisfy
/// the module's declared `engines.shadowcat` range. A module with NO declared
/// range fails closed (never enables) — the field is optional on the shared
/// client `ModuleManifest` TS type (first-party modules never set it) but is
/// effectively mandatory for anything going through this pipeline.
///
/// # Examples
///
/// ```
/// use shadowcat::modules::{engine_compat_ok, InstalledModule};
///
/// let m = InstalledModule {
///     id: "example-mod".into(),
///     requirements: vec![],
///     engines_shadowcat: None, // no declared range
///     manifest_json: serde_json::json!({}),
///     entry_url: "/modules/example-mod/index.js".into(),
/// };
/// assert!(!engine_compat_ok(&m)); // fails closed without engines.shadowcat
/// assert!(engine_compat_ok(&InstalledModule { engines_shadowcat: Some("*".into()), ..m }));
/// ```
pub fn engine_compat_ok(m: &InstalledModule) -> bool {
    match &m.engines_shadowcat {
        Some(range) => semver_satisfies(env!("CARGO_PKG_VERSION"), range),
        None => false,
    }
}

/// Caches `scan_installed_modules`'s result, invalidated by comparing the
/// modules directory's own mtime (bumped by the OS on any entry add/remove —
/// operators install/uninstall modules by dropping/removing folders directly
/// on disk; there is no server-side install route to hook a cache
/// invalidation into) AND each cached module's `module.json` mtime (catches
/// an in-place manifest edit to an already-installed module, which does NOT
/// change the parent directory's own mtime). One instance shared across every
/// WS connection on the server (see `crate::ws::WsState::module_scan_cache`).
#[derive(Default)]
pub struct ModuleScanCache {
    /// The current cached scan, if any. Stored behind an `Arc` so
    /// `get_or_scan` can clone the current entry out under the lock and run
    /// its freshness check and any rescan entirely outside it.
    entry: std::sync::Mutex<Option<Arc<CachedScan>>>,
}

/// One cached scan result plus the mtimes it was valid against.
struct CachedScan {
    /// `modules_dir`'s own mtime at scan time.
    dir_mtime: std::time::SystemTime,
    /// Each cached module's id -> its `module.json`'s mtime at scan time.
    manifest_mtimes: std::collections::BTreeMap<String, std::time::SystemTime>,
    /// The scan result itself.
    modules: Vec<InstalledModule>,
}

impl ModuleScanCache {
    /// An empty cache (one per server).
    ///
    /// # Examples
    ///
    /// ```
    /// let cache = shadowcat::modules::ModuleScanCache::new();
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the cached scan if `modules_dir` and every cached module's
    /// `module.json` mtime are unchanged since the cache was populated;
    /// otherwise rescans (`scan_installed_modules`) and replaces the cache.
    /// Blocking filesystem I/O — call only from within `spawn_blocking`.
    ///
    /// # Examples
    ///
    /// ```
    /// let cache = shadowcat::modules::ModuleScanCache::new();
    /// let modules = cache.get_or_scan(std::path::Path::new("no-such-modules-dir"));
    /// assert!(modules.is_empty());
    /// ```
    pub fn get_or_scan(&self, modules_dir: &Path) -> Vec<InstalledModule> {
        let dir_mtime = std::fs::metadata(modules_dir)
            .and_then(|m| m.modified())
            .ok();

        // Snapshot the current entry; the lock is held only for this clone,
        // never across the freshness check's stat() loop or a rescan — both
        // run below with no lock held, so one caller's rescan never blocks
        // another caller's cache-hit validation.
        let current = self
            .entry
            .lock()
            .expect("module scan cache mutex poisoned")
            .clone();
        if let (Some(cached), Some(dir_mtime)) = (current.as_ref(), dir_mtime) {
            if cached.dir_mtime == dir_mtime
                && cached
                    .manifest_mtimes
                    .iter()
                    .all(|(id, mtime)| Self::manifest_mtime(modules_dir, id) == Some(*mtime))
            {
                return cached.modules.clone();
            }
        }
        let modules = scan_installed_modules(modules_dir);
        let manifest_mtimes = modules
            .iter()
            .filter_map(|m| Self::manifest_mtime(modules_dir, &m.id).map(|mt| (m.id.clone(), mt)))
            .collect();
        // A missing/unreadable modules_dir yields no dir_mtime; fall back to
        // UNIX_EPOCH so the cache is still populated (empty modules list) but
        // never spuriously "matches" a later, real directory at the same path
        // (a real mtime is never UNIX_EPOCH in practice).
        let dir_mtime = dir_mtime.unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let fresh = Arc::new(CachedScan {
            dir_mtime,
            manifest_mtimes,
            modules: modules.clone(),
        });
        // Lock taken again only to swap in the fresh entry. Two concurrent
        // misses may each independently rescan and each write their own
        // fresh entry here — last writer wins, matching `LinkPreviewCache`'s
        // miss-path semantics: both scans observed the same or a
        // monotonically later on-disk state, so either result is valid.
        *self.entry.lock().expect("module scan cache mutex poisoned") = Some(fresh);
        modules
    }

    /// `<modules_dir>/<id>/module.json`'s mtime, or `None` if it can't be stat'd.
    fn manifest_mtime(modules_dir: &Path, id: &str) -> Option<std::time::SystemTime> {
        std::fs::metadata(modules_dir.join(id).join("module.json"))
            .and_then(|m| m.modified())
            .ok()
    }
}

#[cfg(test)]
mod tests;
