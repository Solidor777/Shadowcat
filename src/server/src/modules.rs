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
    #[allow(dead_code)]
    id: String,
    #[allow(dead_code)]
    version: String,
    #[serde(default)]
    requirements: Vec<CapabilityRequirement>,
    #[serde(default)]
    engines: ModuleEngines,
    #[serde(default = "default_entry")]
    entry: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ModuleEngines {
    shadowcat: Option<String>,
}

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
    /// from (and cross-checked against, client-side, exactly as `loader.ts`
    /// already cross-checks discovery-id vs the module's own declared id).
    pub id: String,
    pub requirements: Vec<CapabilityRequirement>,
    pub engines_shadowcat: Option<String>,
    pub manifest_json: serde_json::Value,
    pub entry_url: String,
}

/// Scan `<modules_dir>/*/module.json`, parse + validate each. An invalid
/// manifest (missing/malformed `id`/`version`, or malformed JSON) is logged
/// (warn) and skipped — one broken module must not prevent startup or hide the
/// others (ARCHITECTURE invariant 6: server authority over a community-authored
/// body is structural only; fail-open on discovery is this plan's own Global
/// Constraint 4, not itself an ARCHITECTURE invariant). A missing `modules_dir`
/// (nothing installed yet) yields an empty list, not an error. Deterministic
/// id-sorted order.
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
        write_module(dir.path(), "missing-fields", r#"{"name":"no id or version"}"#);
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
        assert_eq!(found.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["aaa", "zzz"]);
    }
}
