// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::path::Path;

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
    /// Author-declared id — presence-validated only, never trusted as a key.
    #[allow(dead_code)]
    id: String,
    /// Author-declared semver — presence-validated only.
    #[allow(dead_code)]
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

#[cfg(test)]
mod tests {
    use super::*;

    fn write_module(dir: &Path, folder: &str, json: &str) {
        let module_dir = dir.join(folder);
        std::fs::create_dir_all(&module_dir).unwrap();
        std::fs::write(module_dir.join("module.json"), json).unwrap();
    }

    #[test]
    fn missing_modules_dir_yields_empty() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        assert!(scan_installed_modules(&missing).is_empty());
    }

    #[test]
    fn discovers_a_valid_module_with_default_entry() {
        let dir = tempfile::tempdir().unwrap();
        write_module(
            dir.path(),
            "actors-plus",
            r#"{"id":"actors-plus","version":"1.0.0","engines":{"shadowcat":"^0.1.0"}}"#,
        );
        let found = scan_installed_modules(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "actors-plus");
        assert_eq!(found[0].entry_url, "/modules/actors-plus/index.js");
        assert_eq!(found[0].engines_shadowcat.as_deref(), Some("^0.1.0"));
        assert_eq!(found[0].manifest_json["id"], "actors-plus");
    }

    #[test]
    fn respects_an_entry_override() {
        let dir = tempfile::tempdir().unwrap();
        write_module(
            dir.path(),
            "custom-entry",
            r#"{"id":"custom-entry","version":"1.0.0","entry":"bundle/main.js"}"#,
        );
        let found = scan_installed_modules(dir.path());
        assert_eq!(found[0].entry_url, "/modules/custom-entry/bundle/main.js");
    }

    #[test]
    fn an_invalid_manifest_is_skipped_without_hiding_valid_siblings() {
        let dir = tempfile::tempdir().unwrap();
        write_module(dir.path(), "broken", r#"{"not-json"#); // malformed JSON
        write_module(
            dir.path(),
            "missing-fields",
            r#"{"name":"no id or version"}"#,
        );
        write_module(dir.path(), "good", r#"{"id":"good","version":"1.0.0"}"#);
        let found = scan_installed_modules(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "good");
    }

    #[test]
    fn a_folder_with_no_module_json_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("not-a-module")).unwrap();
        assert!(scan_installed_modules(dir.path()).is_empty());
    }

    #[test]
    fn unknown_manifest_fields_are_tolerated_forward_compatibly() {
        let dir = tempfile::tempdir().unwrap();
        write_module(
            dir.path(),
            "future",
            r#"{"id":"future","version":"1.0.0","someFutureField":{"nested":true}}"#,
        );
        let found = scan_installed_modules(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "future");
    }

    #[test]
    fn discovery_order_is_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        write_module(dir.path(), "zzz", r#"{"id":"zzz","version":"1.0.0"}"#);
        write_module(dir.path(), "aaa", r#"{"id":"aaa","version":"1.0.0"}"#);
        let found = scan_installed_modules(dir.path());
        assert_eq!(
            found.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["aaa", "zzz"]
        );
    }

    #[test]
    fn semver_wildcard_matches_anything() {
        assert!(semver_satisfies("9.9.9", "*"));
    }

    #[test]
    fn semver_exact_match() {
        assert!(semver_satisfies("1.2.3", "1.2.3"));
        assert!(!semver_satisfies("1.2.4", "1.2.3"));
    }

    #[test]
    fn semver_caret_allows_same_major_gte_patch_minor() {
        assert!(semver_satisfies("1.4.0", "^1.2.3"));
        assert!(!semver_satisfies("1.2.2", "^1.2.3"));
        assert!(!semver_satisfies("2.0.0", "^1.2.3"));
    }

    #[test]
    fn semver_caret_0_x_y_boundary_is_minor_when_major_is_zero() {
        assert!(semver_satisfies("0.2.5", "^0.2.3"));
        assert!(!semver_satisfies("0.3.0", "^0.2.3"));
    }

    #[test]
    fn semver_caret_0_0_y_boundary_is_patch_when_major_and_minor_are_zero() {
        assert!(semver_satisfies("0.0.3", "^0.0.3"));
        assert!(!semver_satisfies("0.0.4", "^0.0.3"));
    }

    #[test]
    fn semver_tilde_allows_same_major_minor_gte_patch() {
        assert!(semver_satisfies("1.2.9", "~1.2.3"));
        assert!(!semver_satisfies("1.3.0", "~1.2.3"));
    }

    #[test]
    fn semver_invalid_version_fails_closed() {
        assert!(!semver_satisfies("not-a-version", "*"));
    }

    #[test]
    fn engine_compat_ok_requires_the_engines_field() {
        let dir = tempfile::tempdir().unwrap();
        write_module(
            dir.path(),
            "no-engines",
            r#"{"id":"no-engines","version":"1.0.0"}"#,
        );
        let m = &scan_installed_modules(dir.path())[0];
        // A module with no declared compat range never enables (mandatory
        // going forward for the modules-folder pipeline).
        assert!(!engine_compat_ok(m));
    }

    #[test]
    fn engine_compat_ok_checks_the_running_server_version() {
        let dir = tempfile::tempdir().unwrap();
        write_module(
            dir.path(),
            "compatible",
            &format!(
                r#"{{"id":"compatible","version":"1.0.0","engines":{{"shadowcat":"^{}"}}}}"#,
                env!("CARGO_PKG_VERSION")
            ),
        );
        write_module(
            dir.path(),
            "incompatible",
            r#"{"id":"incompatible","version":"1.0.0","engines":{"shadowcat":"^99.0.0"}}"#,
        );
        let found = scan_installed_modules(dir.path());
        let compatible = found.iter().find(|m| m.id == "compatible").unwrap();
        let incompatible = found.iter().find(|m| m.id == "incompatible").unwrap();
        assert!(engine_compat_ok(compatible));
        assert!(!engine_compat_ok(incompatible));
    }
}
