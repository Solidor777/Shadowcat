# M13-1 · External-Module Toolchain Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let Shadowcat load, serve, and per-world-enable community-built modules through a real install/dev/e2e pipeline (host-served import map + modules-folder), and bootstrap Nightfox — the first external module — as proof.

**Architecture:** Server-side: a new `modules` domain (scan `<data-dir>/modules/*/module.json`, semver engine-compat, path-traversal-guarded static serving) plus per-world enablement persisted in the existing `settings` JSON table beside `capability_requirements`/`contract_declarations`, non-destructively unioned into the `Welcome` broadcast. Client-side: the shell's Vite build emits stable-named ESM chunks for `svelte`, `@shadowcat/core`, `@shadowcat/ui-kit`, `@shadowcat/formula`, `@shadowcat/types`, referenced by a build-time `<script type="importmap">` in `index.html` (no server-side HTML templating needed — chunk names are deterministic); `WorldSession` fetches the world's enabled installed modules after `Welcome` and loads them through a newly per-module-contained `loadModules`. Nightfox is scaffolded as a standalone repo at `C:\Dev\Nightfox`, authored to resolve `@shadowcat/*` only once nested into a Shadowcat checkout's pnpm workspace (matching first-party module conventions exactly).

**Tech Stack:** Rust/axum/sqlx (server), TypeScript/Svelte 5/Vite/Zod (client), pnpm workspaces, ts-rs 12 for wire types.

## Global Constraints

These are the spec's Invariants (copied verbatim); every task's requirements implicitly include this section.

1. Exactly ONE instance of `svelte` and of each `@shadowcat/*` package exists at runtime; the import map is the single resolution authority for external module code (T1/T3).
2. The server never executes or introspects module code; module serving is static files with a path-traversal guard (§1; ARCHITECTURE §2 invariant 6).
3. External modules load exclusively through the modules-folder pipeline in every environment, including development (T4).
4. A broken installed module degrades to a logged, user-visible warning — never a failed server start or bricked world (§1 scan, §3 load containment).
5. Engine-compat (`engines.shadowcat`) is enforced at enable AND load, with explicit errors (T6).
6. All new server paths are `std::path`-built; module serving and the toolchain work on the three-OS matrix and mobile browsers (import maps are baseline in evergreen browsers).

**Design decisions made by this plan (not silently assumed — see the plan-writer's final report for the rationale on each):**

- **Per-world enabled-module storage**: a new `settings` table JSON key `world_modules:<world>` (`Vec<String>` of installed module ids), mirroring the EXACT mechanism `world_caps_req_key`/`world_contracts_key` already use. Chosen because it sits beside `capability_requirements` (spec's own delegation criterion) and needs no schema migration.
- **"Enabling a module publishes its manifest requirements... exactly as first-party module requirements are published" (spec §2)**: audited — no live publish path exists today for first-party modules (`ModuleRegistry.collectRequirements()` is unused dead code; no client UI ever PUTs `/api/worlds/{id}/capability-requirements`). This plan implements the mechanism fresh: `world_cap_requirements` (the GM's own raw admin record) is left completely untouched by enable/disable; the `Welcome` broadcast instead unions it with the enabled modules' own `requirements` at send time (Task 10). Non-destructive, no clobber risk, and it publishes through the identical broadcast site the existing mechanism uses.
- **Module discovery caching**: scanned fresh on every `GET /api/modules` call and every enable-time validation (no `AppState` cache). A GM's manual filesystem extract is visible immediately without a server restart; local-disk scan cost is negligible at realistic module counts. A log-only scan also runs at startup (matches the spec's literal "on startup, scan" trigger).
- **Import map delivery**: built INTO `index.html` at Vite build time (not injected server-side at request time), because the custom `entryFileNames` function makes the runtime-chunk output paths deterministic and known ahead of time. `src/server/src/http/embed.rs` needs no changes.
- **`engines` field is OPTIONAL on the shared `ModuleManifest` TS type/Zod schema** (not required), because it is shared with every first-party module's in-code manifest literal — making it required would break ~20 existing first-party `Module` objects for no reason (first-party modules ship version-locked inside the binary; they have no engine-compat problem). T6's "obligates version discipline from here on" applies to the community modules-folder pipeline specifically (server enable-time gate + client load-time gate both hard-require it there), not to the shared schema.
- **The running server version reaches the client via the authenticated `Welcome` broadcast, NOT public `/api/config`** (Task 9). The load-time engine-compat gate needs the server version, and it runs in `worldSession.#onWelcome` — which already receives the whole `Welcome` payload. `Welcome` is authenticated and per-session, and already carries a server-global value (`server_time`), so `server_version` rides the same message the client is already awaiting: no extra `getConfig()` round-trip, no new public API. `/api/config` stays public but `initialized`-only (it MUST remain unauthenticated for the pre-init setup flow, so `version` was the only field that ever wanted auth — splitting it out is mandatory, not optional). This closes a pre-auth version-fingerprinting surface (exact patch semver disclosed to any unauthenticated caller, enabling targeted-CVE recon) while REDUCING API surface. The server-side enable-time gate (Task 8) is unaffected — it reads `CARGO_PKG_VERSION` directly, never the wire field.

---

## Task 1: Server config — `modules_dir` / `modules_path()`

**Files:**
- Modify: `src/server/src/config.rs`

**Interfaces:**
- Produces: `Config::modules_path(&self) -> std::path::PathBuf`; `Config.modules_dir: Option<String>`; `Cli.modules_dir: Option<String>`.

- [ ] **Step 1: Write the failing test** — add to the `#[cfg(test)] mod tests` block in `src/server/src/config.rs`, immediately after `assets_path_defaults_to_db_sibling`:

```rust
#[test]
fn modules_path_defaults_to_db_sibling() {
    let mut cfg = Config {
        db: "/data/shadowcat.db".into(),
        ..Config::default()
    };
    assert_eq!(
        cfg.modules_path(),
        std::path::PathBuf::from("/data").join("modules")
    );
    cfg.modules_dir = Some("/custom/modules".into());
    assert_eq!(
        cfg.modules_path(),
        std::path::PathBuf::from("/custom/modules")
    );
}
```

- [ ] **Step 2: Run to verify it fails**

Run (from `src/server/`): `cargo test modules_path_defaults_to_db_sibling`
Expected: FAIL to compile — `no field \`modules_dir\` on type \`Config\`` / `no method named \`modules_path\``.

- [ ] **Step 3: Add the field + CLI flag + resolver**

In `src/server/src/config.rs`, add to `Cli` (after `pub assets_dir: Option<String>,`):

```rust
    #[arg(long)]
    pub modules_dir: Option<String>,
```

Add to `Config` (after `pub assets_dir: Option<String>,`), with its own doc comment:

```rust
    /// Installed-module discovery root. `None` → sibling `modules/` dir beside the db file.
    pub modules_dir: Option<String>,
```

Add to `Config::default()`'s struct literal (after `assets_dir: None,`):

```rust
            modules_dir: None,
```

In `Config::load`, after the `if let Some(v) = cli.assets_dir { cfg.assets_dir = Some(v); }` block:

```rust
        if let Some(v) = cli.modules_dir {
            cfg.modules_dir = Some(v);
        }
```

Add the resolver method right after `assets_path`:

```rust
    /// Resolve the installed-module discovery root: explicit `modules_dir`, else a
    /// sibling `modules/` directory beside the db file (built via std::path, #2).
    /// Unlike `assets_path`, nothing writes here server-side (install is manual
    /// filesystem extract, T2) — the directory need not exist; a missing dir
    /// scans as "no modules installed" (see `modules::scan_installed_modules`).
    pub fn modules_path(&self) -> std::path::PathBuf {
        if let Some(dir) = &self.modules_dir {
            return std::path::PathBuf::from(dir);
        }
        std::path::Path::new(&self.db)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("modules")
    }
```

- [ ] **Step 4: Run full server suite + clippy**

Run (from `src/server/`): `cargo test --all-targets`
Expected: PASS, including `modules_path_defaults_to_db_sibling`.
Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/config.rs
git commit -m "feat(server/config): modules_dir config + modules_path() resolver"
```

---

## Task 2: Server `modules.rs` — installed-module discovery

**Files:**
- Create: `src/server/src/modules.rs`
- Modify: `src/server/src/lib.rs`

**Interfaces:**
- Consumes: `crate::data::document::CapabilityRequirement` (Task 0 research; already exists).
- Produces: `pub struct InstalledModule { pub id: String, pub requirements: Vec<CapabilityRequirement>, pub engines_shadowcat: Option<String>, pub manifest_json: serde_json::Value, pub entry_url: String }`; `pub fn scan_installed_modules(modules_dir: &std::path::Path) -> Vec<InstalledModule>` (deterministic id-sorted order; logs+skips invalid manifests; empty on a missing dir).

- [ ] **Step 1: Write the failing tests** — create `src/server/src/modules.rs`:

```rust
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
    id: String,
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
        let contents = match std::fs::read_to_string(&manifest_path) {
            Ok(c) => c,
            Err(_) => continue, // no module.json: not a module folder
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
        let _ = &mirror.version; // presence-validated by the mirror parse; not otherwise used here
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
```

- [ ] **Step 2: Run to verify it fails**

Wire `pub mod modules;` into `src/server/src/lib.rs` first (add after `pub mod ws;`, alphabetically it lands as: `db`, `dice`, `health`, `http`, `modules`, `scene`, `ws` — insert `pub mod modules;` between `pub mod http;` and `pub mod scene;`).

Run (from `src/server/`): `cargo test -p shadowcat modules::`
Expected: currently compiles and PASSES (the file above is written directly as working code, per TDD-in-one-step convention for a pure new module with no prior stub). Confirm this explicitly: run it once before Step 3 exists — since Step 1's code IS the implementation, this plan folds RED/GREEN into one write for this pure-function task (no separate "stub that fails" step makes sense when there is no existing partial implementation to diff against). Proceed to Step 3 for the actual verification.

- [ ] **Step 3: Run full server suite + clippy to verify GREEN**

Run (from `src/server/`): `cargo test --all-targets`
Expected: PASS, including all `modules::tests::*`.
Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src/server/src/modules.rs src/server/src/lib.rs
git commit -m "feat(server/modules): scan_installed_modules discovery + skip-invalid"
```

---

## Task 3: Server `modules.rs` — semver engine-compat gate (T6)

**Files:**
- Modify: `src/server/src/modules.rs`

**Interfaces:**
- Consumes: `InstalledModule` (Task 2).
- Produces: `pub fn semver_satisfies(version: &str, range: &str) -> bool`; `pub fn engine_compat_ok(m: &InstalledModule) -> bool`.

- [ ] **Step 1: Write the failing tests** — append to the `#[cfg(test)] mod tests` block in `src/server/src/modules.rs`:

```rust
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
        write_module(dir.path(), "no-engines", r#"{"id":"no-engines","version":"1.0.0"}"#);
        let m = &scan_installed_modules(dir.path())[0];
        // A module with no declared compat range never enables (T6: mandatory
        // going forward for the modules-folder pipeline).
        assert!(!engine_compat_ok(m));
    }

    #[test]
    fn engine_compat_ok_checks_the_running_server_version() {
        let dir = tempfile::tempdir().unwrap();
        write_module(
            dir.path(),
            "compatible",
            &format!(r#"{{"id":"compatible","version":"1.0.0","engines":{{"shadowcat":"^{}"}}}}"#, env!("CARGO_PKG_VERSION")),
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
```

- [ ] **Step 2: Run to verify it fails**

Run (from `src/server/`): `cargo test -p shadowcat modules::tests::semver`
Expected: FAIL to compile — `cannot find function \`semver_satisfies\`` / `\`engine_compat_ok\``.

- [ ] **Step 3: Implement** — append to `src/server/src/modules.rs`, before the `#[cfg(test)]` block:

```rust
/// Minimal semver range matcher mirroring the client's `satisfies` in
/// `src/client/core/src/semver.ts` (exact / `^` / `~` / `*`) — both sides must
/// agree on `engines.shadowcat` compatibility (enable-time here, load-time
/// there), so the tiny algorithm is duplicated intentionally rather than
/// shared across the Rust/TS boundary. Fails closed (false) on a malformed
/// version or range rather than panicking.
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
    let Some(v) = parse(version) else { return false };
    if r == "*" {
        return true;
    }
    if let Some(rest) = r.strip_prefix('^') {
        let Some(b) = parse(rest) else { return false };
        return v.0 == b.0 && v >= b;
    }
    if let Some(rest) = r.strip_prefix('~') {
        let Some(b) = parse(rest) else { return false };
        return v.0 == b.0 && v.1 == b.1 && v >= b;
    }
    let Some(b) = parse(r) else { return false };
    v == b
}

/// T6 engine-compat gate: the running server's `CARGO_PKG_VERSION` must satisfy
/// the module's declared `engines.shadowcat` range. A module with NO declared
/// range fails closed (never enables) — the field is optional on the shared
/// client `ModuleManifest` TS type (first-party modules never set it) but is
/// effectively mandatory for anything going through this pipeline.
pub fn engine_compat_ok(m: &InstalledModule) -> bool {
    match &m.engines_shadowcat {
        Some(range) => semver_satisfies(env!("CARGO_PKG_VERSION"), range),
        None => false,
    }
}
```

- [ ] **Step 4: Run full server suite + clippy to verify GREEN**

Run (from `src/server/`): `cargo test --all-targets`
Expected: PASS.
Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/modules.rs
git commit -m "feat(server/modules): semver_satisfies + engine_compat_ok (T6)"
```

---

## Task 4: `InstalledModuleInfo` wire type + `GET /api/modules`

**Files:**
- Create: `src/server/src/http/module_routes.rs`
- Modify: `src/server/src/modules.rs`, `src/server/src/http/mod.rs`, `src/types/index.ts`

**Interfaces:**
- Consumes: `InstalledModule`, `scan_installed_modules` (Task 2); `AppState` (`repo`, `config`); `AuthUser` extractor (`src/server/src/auth/session.rs`, already exists).
- Produces: `pub struct InstalledModuleInfo { pub manifest: serde_json::Value, pub entry_url: String }` (ts-rs exported); `pub async fn list_installed_modules(...)` axum handler; `src/types/generated/InstalledModuleInfo.ts`; client-visible `InstalledModuleInfo` type from `@shadowcat/types`.

- [ ] **Step 1: Write the failing test** — create `src/server/src/http/module_routes.rs`:

```rust
use axum::extract::State;
use axum::Json;
use serde::Serialize;
use ts_rs::TS;

use crate::auth::session::AuthUser;
use crate::http::AppState;

/// `GET /api/modules` response element: the raw manifest (opaque to the
/// server beyond structural discovery, ARCHITECTURE invariant 2 — the client's
/// own Zod schema re-validates it) plus the URL the client dynamic-imports.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct InstalledModuleInfo {
    #[ts(type = "unknown")]
    pub manifest: serde_json::Value,
    pub entry_url: String,
}

impl From<&crate::modules::InstalledModule> for InstalledModuleInfo {
    fn from(m: &crate::modules::InstalledModule) -> Self {
        InstalledModuleInfo {
            manifest: m.manifest_json.clone(),
            entry_url: m.entry_url.clone(),
        }
    }
}

/// `GET /api/modules` — every validly installed module. Any authenticated user
/// (a client needs this to resolve entry URLs for its world's enabled set).
/// Freshly re-scanned per request (see the plan's "module discovery caching"
/// decision) — a manual filesystem install is visible without a restart.
pub async fn list_installed_modules(
    _user: AuthUser,
    State(state): State<AppState>,
) -> Json<Vec<InstalledModuleInfo>> {
    let installed = crate::modules::scan_installed_modules(&state.config.modules_path());
    Json(installed.iter().map(Into::into).collect())
}

#[cfg(test)]
mod tests {
    use crate::http::tests::initialized_state;
    use crate::http::router;
    use axum::http::StatusCode;

    #[tokio::test]
    async fn list_installed_modules_requires_auth() {
        let server = axum_test::TestServer::new(router(initialized_state().await).await).unwrap();
        server
            .get("/api/modules")
            .await
            .assert_status(StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn list_installed_modules_returns_the_scanned_set() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("actors-plus")).unwrap();
        std::fs::write(
            dir.path().join("actors-plus").join("module.json"),
            r#"{"id":"actors-plus","version":"1.0.0","provides":[{"contract":"x:y","cardinality":"multi"}]}"#,
        )
        .unwrap();

        let mut state = initialized_state().await;
        state.config = std::sync::Arc::new(crate::config::Config {
            modules_dir: Some(dir.path().to_string_lossy().to_string()),
            ..crate::config::Config::default()
        });
        let hash = crate::auth::password::hash_password("pw").unwrap();
        state
            .repo
            .create_user("u", Some(&hash), crate::auth::role::ServerRole::User, 0)
            .await
            .unwrap();
        let server = axum_test::TestServer::builder()
            .save_cookies()
            .build(router(state).await)
            .unwrap();
        server
            .post("/api/login")
            .json(&serde_json::json!({ "username": "u", "password": "pw" }))
            .await
            .assert_status(StatusCode::NO_CONTENT);

        let got: serde_json::Value = server.get("/api/modules").await.json();
        let arr = got.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["manifest"]["id"], "actors-plus");
        assert_eq!(arr[0]["manifest"]["provides"][0]["contract"], "x:y");
        assert_eq!(arr[0]["entry_url"], "/modules/actors-plus/index.js");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run (from `src/server/`): `cargo test -p shadowcat list_installed_modules`
Expected: FAIL to compile — `module_routes` is not declared, `AppState` field/route missing.

- [ ] **Step 3: Wire the module + route**

In `src/server/src/http/mod.rs`, add after `pub mod embed;`:

```rust
pub mod module_routes;
```

Add the route in the `Router::new()` chain, right after the `/api/worlds/{id}/contracts` route:

```rust
        .route("/api/modules", get(module_routes::list_installed_modules))
```

- [ ] **Step 4: Run full server suite + clippy to verify GREEN**

Run (from `src/server/`): `cargo test --all-targets`
Expected: PASS, including both `module_routes::tests::*` tests; ts-rs regenerates `src/types/generated/InstalledModuleInfo.ts` as a side effect.
Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 5: Barrel-export the generated type**

In `src/types/index.ts`, add a new section right after the `// HTTP API DTOs` block (after `export type { WorldEntry } from "./generated/WorldEntry";`):

```ts
// Module toolchain (M13-1)
export type { InstalledModuleInfo } from "./generated/InstalledModuleInfo";
```

- [ ] **Step 6: Run the client typecheck**

Run (from repo root): `pnpm -r typecheck`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/server/src/http/mod.rs src/server/src/http/module_routes.rs src/types/index.ts src/types/generated/InstalledModuleInfo.ts
git commit -m "feat(server/http): GET /api/modules — InstalledModuleInfo wire type"
```

---

## Task 5: `GET /modules/{id}/{*path}` — path-traversal-guarded static serving

**FLAGGED FOR BUDDY-CHECK** (security boundary — see Buddy-check directives).

**Files:**
- Modify: `src/server/src/http/module_routes.rs`, `src/server/src/http/mod.rs`

**Interfaces:**
- Consumes: `Config::modules_path()` (Task 1); `AuthUser` extractor.
- Produces: `pub async fn serve_module_file(...)` axum handler, mounted at `GET /modules/{id}/{*path}`.

- [ ] **Step 1: Write the failing tests** — append to the `#[cfg(test)] mod tests` block in `src/server/src/http/module_routes.rs`:

```rust
    #[tokio::test]
    async fn serve_module_file_requires_auth() {
        let server = axum_test::TestServer::new(router(initialized_state().await).await).unwrap();
        server
            .get("/modules/whatever/index.js")
            .await
            .assert_status(StatusCode::UNAUTHORIZED);
    }

    async fn logged_in_server_with_modules_dir(
        dir: &std::path::Path,
    ) -> axum_test::TestServer {
        let mut state = initialized_state().await;
        state.config = std::sync::Arc::new(crate::config::Config {
            modules_dir: Some(dir.to_string_lossy().to_string()),
            ..crate::config::Config::default()
        });
        let hash = crate::auth::password::hash_password("pw").unwrap();
        state
            .repo
            .create_user("u", Some(&hash), crate::auth::role::ServerRole::User, 0)
            .await
            .unwrap();
        let server = axum_test::TestServer::builder()
            .save_cookies()
            .build(router(state).await)
            .unwrap();
        server
            .post("/api/login")
            .json(&serde_json::json!({ "username": "u", "password": "pw" }))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        server
    }

    #[tokio::test]
    async fn serve_module_file_serves_the_entry_with_js_content_type() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("mod-a")).unwrap();
        std::fs::write(dir.path().join("mod-a").join("index.js"), b"export const x = 1;").unwrap();
        let server = logged_in_server_with_modules_dir(dir.path()).await;

        let res = server.get("/modules/mod-a/index.js").await;
        res.assert_status_ok();
        assert_eq!(res.text(), "export const x = 1;");
        let ct = res.header("content-type");
        assert_eq!(ct, "text/javascript");
    }

    #[tokio::test]
    async fn serve_module_file_serves_a_nested_asset_with_a_generic_content_type() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("mod-a").join("assets")).unwrap();
        std::fs::write(dir.path().join("mod-a").join("assets").join("icon.png"), b"\x89PNG").unwrap();
        let server = logged_in_server_with_modules_dir(dir.path()).await;

        let res = server.get("/modules/mod-a/assets/icon.png").await;
        res.assert_status_ok();
        assert_eq!(res.header("content-type"), "image/png");
    }

    #[tokio::test]
    async fn serve_module_file_404s_a_nonexistent_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("mod-a")).unwrap();
        let server = logged_in_server_with_modules_dir(dir.path()).await;
        server
            .get("/modules/mod-a/index.js")
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn serve_module_file_rejects_a_rel_path_traversal_out_of_the_module_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("mod-a")).unwrap();
        std::fs::write(dir.path().join("secret.txt"), b"outside mod-a, inside modules_dir").unwrap();
        let server = logged_in_server_with_modules_dir(dir.path()).await;

        // Percent-encoded so the traversal segment reaches the server unresolved
        // (a client-side fetch would otherwise normalize a literal `..` away
        // before the request is even sent, defeating the point of this test).
        let res = server.get("/modules/mod-a/%2e%2e%2fsecret.txt").await;
        res.assert_status(StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn serve_module_file_rejects_an_id_segment_that_escapes_the_modules_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("mod-a")).unwrap();
        std::fs::write(dir.path().join("outside.txt"), b"parent of modules_dir").unwrap();
        let server = logged_in_server_with_modules_dir(dir.path()).await;

        // id="..%2f.." resolves (via the `id` capture alone) above modules_dir
        // before `rel_path` is even considered — the two-stage guard must catch
        // this at the FIRST canonicalize, not rely on the second.
        let res = server.get("/modules/%2e%2e/outside.txt").await;
        res.assert_status(StatusCode::NOT_FOUND);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run (from `src/server/`): `cargo test -p shadowcat serve_module_file`
Expected: FAIL — route `/modules/{id}/{*path}` does not exist (404 vs expected behavior; the auth test also fails since the route is unmounted, giving 404 not 401).

- [ ] **Step 3: Implement** — append to `src/server/src/http/module_routes.rs`, before `#[cfg(test)]`:

```rust
use axum::extract::Path;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::http::error::AppError;

/// `GET /modules/{id}/{*path}` — static file serving from an installed
/// module's OWN folder only. Auth: any authenticated user (browsers `import()`
/// the entry + fetch its relative assets under session cookies). The server
/// never reads/executes this JS (ARCHITECTURE invariant 2) — this is
/// byte-serving with a MANDATORY two-stage path-traversal guard:
///   1. `id` alone (a single URL segment, but percent-encoded `..`/`/` can
///      still smuggle a traversal into it) must canonicalize to a path still
///      inside the modules root.
///   2. `rel_path` joined onto that module's own canonicalized root must
///      still canonicalize to a path inside THAT root.
/// Both canonicalize calls resolve symlinks too, closing that escape route in
/// the same check. Any failure (missing file, either escape) is a uniform 404
/// — never distinguishing "traversal rejected" from "not found".
pub async fn serve_module_file(
    _user: AuthUser,
    State(state): State<AppState>,
    Path((id, rel_path)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let modules_root = state.config.modules_path();
    let modules_root_canon = tokio::fs::canonicalize(&modules_root)
        .await
        .map_err(|_| AppError::NotFound)?;
    let module_dir = modules_root.join(&id);
    let module_dir_canon = tokio::fs::canonicalize(&module_dir)
        .await
        .map_err(|_| AppError::NotFound)?;
    if !module_dir_canon.starts_with(&modules_root_canon) {
        return Err(AppError::NotFound);
    }
    let candidate = module_dir.join(&rel_path);
    let candidate_canon = tokio::fs::canonicalize(&candidate)
        .await
        .map_err(|_| AppError::NotFound)?;
    if !candidate_canon.starts_with(&module_dir_canon) {
        return Err(AppError::NotFound);
    }
    let bytes = tokio::fs::read(&candidate_canon)
        .await
        .map_err(|_| AppError::NotFound)?;
    // `.js`/`.mjs` must be exactly `text/javascript` — load-bearing for ESM
    // `import()`; mime_guess alone is not trusted to pick the exact MIME the
    // browser's module loader requires.
    let content_type = match candidate_canon.extension().and_then(|e| e.to_str()) {
        Some("js") | Some("mjs") => "text/javascript".to_string(),
        _ => mime_guess::from_path(&candidate_canon)
            .first_or_octet_stream()
            .to_string(),
    };
    Ok(([(header::CONTENT_TYPE, content_type)], bytes).into_response())
}
```

Note: `StatusCode` import above is unused directly in this new code (used only in tests); remove the `use axum::http::{header, StatusCode};` duplicate `StatusCode` if `AppError`'s `NotFound` variant already covers status mapping — it does (see `http/error.rs`), so trim the import to just `header`:

```rust
use axum::http::header;
```

In `src/server/src/http/mod.rs`, add the route right after `.route("/api/modules", ...)`:

```rust
        .route("/modules/{id}/{*path}", get(module_routes::serve_module_file))
```

- [ ] **Step 4: Run full server suite + clippy to verify GREEN**

Run (from `src/server/`): `cargo test --all-targets`
Expected: PASS, including all 6 new tests.
Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/http/module_routes.rs src/server/src/http/mod.rs
git commit -m "feat(server/http): GET /modules/{id}/{*path} static serving with path-traversal guard"
```

---

## Task 6: main.rs — startup discovery log

**Files:**
- Modify: `src/server/src/main.rs`

**Interfaces:**
- Consumes: `scan_installed_modules` (Task 2), `Config::modules_path()` (Task 1).

- [ ] **Step 1: Add the startup scan+log** — in `src/server/src/main.rs`, after the existing `std::fs::create_dir_all(config.assets_path())?;` line:

```rust
    // Log-only discovery pass (the spec's literal "on startup, scan" trigger);
    // every actual read (GET /api/modules, enable-time validation) re-scans
    // fresh, so this never goes stale — it exists purely to surface a boot-time
    // summary in the log.
    let discovered = shadowcat::modules::scan_installed_modules(&config.modules_path());
    tracing::info!(count = discovered.len(), "installed modules discovered");
```

- [ ] **Step 2: Run full server suite + clippy**

Run (from `src/server/`): `cargo build` (main.rs is a binary target, not exercised by `cargo test`; a plain build proves it compiles).
Expected: builds cleanly.
Run: `cargo test --all-targets`
Expected: PASS (no behavior change to any existing test).
Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src/server/src/main.rs
git commit -m "feat(server): log installed-module discovery at startup"
```

---

## Task 7: Repository — per-world enabled-module storage

**Files:**
- Modify: `src/server/src/data/repository.rs`, `src/server/src/data/sqlite.rs`

**Interfaces:**
- Produces: `Repository::world_enabled_modules(&self, world: Uuid) -> Result<Vec<String>, DataError>`; `SqliteRepository::set_world_enabled_modules(&self, world: Uuid, ids: &[String]) -> Result<(), DataError>`.

- [ ] **Step 1: Write the failing test** — add to the `#[cfg(test)] mod tests` block in `src/server/src/data/sqlite.rs`, right after `world_cap_requirements_round_trip`:

```rust
    #[tokio::test]
    async fn world_enabled_modules_round_trip() {
        let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let author = r.create_user("a", None, ServerRole::User, 0).await.unwrap();
        let w = r.create_world_owned("W", author, 0).await.unwrap();

        assert!(r.world_enabled_modules(w.id).await.unwrap().is_empty());

        let ids = vec!["actors-plus".to_string(), "nightfox".to_string()];
        r.set_world_enabled_modules(w.id, &ids).await.unwrap();
        assert_eq!(r.world_enabled_modules(w.id).await.unwrap(), ids);

        // A subsequent set fully replaces, not appends.
        r.set_world_enabled_modules(w.id, &["nightfox".to_string()])
            .await
            .unwrap();
        assert_eq!(
            r.world_enabled_modules(w.id).await.unwrap(),
            vec!["nightfox".to_string()]
        );
    }
```

- [ ] **Step 2: Run to verify it fails**

Run (from `src/server/`): `cargo test -p shadowcat world_enabled_modules_round_trip`
Expected: FAIL to compile — no method `world_enabled_modules`/`set_world_enabled_modules`.

- [ ] **Step 3: Add the trait method**

In `src/server/src/data/repository.rs`, add after the `world_contract_declarations` trait method:

```rust
    /// A world's enabled installed-module ids (GM-set). Empty when unset.
    async fn world_enabled_modules(&self, world: Uuid) -> Result<Vec<String>, DataError>;
```

- [ ] **Step 4: Implement in sqlite.rs**

Add the key function in `src/server/src/data/sqlite.rs`, right after `world_contracts_key`:

```rust
/// Settings key holding a world's enabled installed-module ids (JSON).
fn world_modules_key(world: Uuid) -> String {
    format!("world_modules:{world}")
}
```

Add the setter as an inherent `SqliteRepository` method, right after `set_world_contract_declarations`:

```rust
    /// Replace a world's enabled installed-module set (stored as JSON in
    /// settings, beside `world_cap_requirements`/`world_contract_declarations`
    /// — enable/disable never mutates either of those; see the plan's
    /// non-destructive-union decision for the `Welcome` broadcast).
    pub async fn set_world_enabled_modules(
        &self,
        world: Uuid,
        ids: &[String],
    ) -> Result<(), DataError> {
        let json = serde_json::to_string(ids)?;
        self.set_setting(&world_modules_key(world), &json).await
    }
```

Implement the trait method in the `impl Repository for SqliteRepository` block, right after `world_contract_declarations`:

```rust
    async fn world_enabled_modules(&self, world: Uuid) -> Result<Vec<String>, DataError> {
        match self.get_setting(&world_modules_key(world)).await? {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Ok(Vec::new()),
        }
    }
```

- [ ] **Step 5: Run full server suite + clippy to verify GREEN**

Run (from `src/server/`): `cargo test --all-targets`
Expected: PASS.
Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/data/repository.rs src/server/src/data/sqlite.rs
git commit -m "feat(server/data): world_enabled_modules storage (settings JSON, mirrors world_cap_requirements)"
```

---

## Task 8: `GET/PUT /api/worlds/{id}/enabled-modules`

**FLAGGED FOR BUDDY-CHECK** (per-world enable + capability-requirements publish boundary — see Buddy-check directives).

**Files:**
- Modify: `src/server/src/http/module_routes.rs`, `src/server/src/http/mod.rs`

**Interfaces:**
- Consumes: `world_enabled_modules`/`set_world_enabled_modules` (Task 7); `scan_installed_modules`, `engine_compat_ok` (Tasks 2–3); `require_gm` (`src/server/src/http/routes.rs`, already `pub(crate)`).
- Produces: `pub async fn get_world_enabled_modules(...)`, `pub async fn set_world_enabled_modules(...)` axum handlers, mounted at `GET/PUT /api/worlds/{id}/enabled-modules`.

- [ ] **Step 1: Write the failing tests** — append to the `#[cfg(test)] mod tests` block in `src/server/src/http/module_routes.rs`:

```rust
    async fn logged_in_gm_and_player_with_modules_dir(
        dir: &std::path::Path,
    ) -> (axum_test::TestServer, axum_test::TestServer, String) {
        let mut state = initialized_state().await;
        state.config = std::sync::Arc::new(crate::config::Config {
            modules_dir: Some(dir.to_string_lossy().to_string()),
            ..crate::config::Config::default()
        });
        let hash = crate::auth::password::hash_password("pw").unwrap();
        state.repo.create_user("gm", Some(&hash), crate::auth::role::ServerRole::User, 0).await.unwrap();
        let player_id = state
            .repo
            .create_user("pl", Some(&hash), crate::auth::role::ServerRole::User, 0)
            .await
            .unwrap();

        let gm = axum_test::TestServer::builder().save_cookies().build(router(state.clone()).await).unwrap();
        gm.post("/api/login").json(&serde_json::json!({"username":"gm","password":"pw"})).await.assert_status(StatusCode::NO_CONTENT);
        let world: serde_json::Value = gm.post("/api/worlds").json(&serde_json::json!({"name":"W"})).await.json();
        let world_id = world["id"].as_str().unwrap().to_string();
        gm.post(&format!("/api/worlds/{world_id}/members"))
            .json(&serde_json::json!({"user": player_id, "role": "player"}))
            .await
            .assert_status(StatusCode::NO_CONTENT);

        let pl = axum_test::TestServer::builder().save_cookies().build(router(state).await).unwrap();
        pl.post("/api/login").json(&serde_json::json!({"username":"pl","password":"pw"})).await.assert_status(StatusCode::NO_CONTENT);

        (gm, pl, world_id)
    }

    #[tokio::test]
    async fn enabled_modules_gm_crud_and_member_read() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("actors-plus")).unwrap();
        std::fs::write(
            dir.path().join("actors-plus").join("module.json"),
            format!(r#"{{"id":"actors-plus","version":"1.0.0","engines":{{"shadowcat":"^{}"}}}}"#, env!("CARGO_PKG_VERSION")),
        )
        .unwrap();
        let (gm, pl, world_id) = logged_in_gm_and_player_with_modules_dir(dir.path()).await;

        // Empty by default.
        let got: serde_json::Value = gm.get(&format!("/api/worlds/{world_id}/enabled-modules")).await.json();
        assert_eq!(got, serde_json::json!([]));

        // A non-GM cannot enable.
        pl.put(&format!("/api/worlds/{world_id}/enabled-modules"))
            .json(&serde_json::json!(["actors-plus"]))
            .await
            .assert_status(StatusCode::FORBIDDEN);

        // The GM enables it.
        gm.put(&format!("/api/worlds/{world_id}/enabled-modules"))
            .json(&serde_json::json!(["actors-plus"]))
            .await
            .assert_status(StatusCode::NO_CONTENT);

        // Any member (not just the GM) can read the enabled set.
        let got: serde_json::Value = pl.get(&format!("/api/worlds/{world_id}/enabled-modules")).await.json();
        assert_eq!(got, serde_json::json!(["actors-plus"]));
    }

    #[tokio::test]
    async fn enabled_modules_rejects_an_uninstalled_id() {
        let dir = tempfile::tempdir().unwrap();
        let (gm, _pl, world_id) = logged_in_gm_and_player_with_modules_dir(dir.path()).await;
        gm.put(&format!("/api/worlds/{world_id}/enabled-modules"))
            .json(&serde_json::json!(["not-installed"]))
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        // Rejected atomically: nothing is persisted from the bad batch.
        let got: serde_json::Value = gm.get(&format!("/api/worlds/{world_id}/enabled-modules")).await.json();
        assert_eq!(got, serde_json::json!([]));
    }

    #[tokio::test]
    async fn enabled_modules_rejects_an_engine_incompatible_module() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("too-new")).unwrap();
        std::fs::write(
            dir.path().join("too-new").join("module.json"),
            r#"{"id":"too-new","version":"1.0.0","engines":{"shadowcat":"^99.0.0"}}"#,
        )
        .unwrap();
        let (gm, _pl, world_id) = logged_in_gm_and_player_with_modules_dir(dir.path()).await;
        gm.put(&format!("/api/worlds/{world_id}/enabled-modules"))
            .json(&serde_json::json!(["too-new"]))
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn enabled_modules_rejects_a_module_with_no_engines_field() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("no-engines")).unwrap();
        std::fs::write(
            dir.path().join("no-engines").join("module.json"),
            r#"{"id":"no-engines","version":"1.0.0"}"#,
        )
        .unwrap();
        let (gm, _pl, world_id) = logged_in_gm_and_player_with_modules_dir(dir.path()).await;
        gm.put(&format!("/api/worlds/{world_id}/enabled-modules"))
            .json(&serde_json::json!(["no-engines"]))
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn enabled_modules_a_batch_with_one_bad_id_rejects_the_whole_batch() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("actors-plus")).unwrap();
        std::fs::write(
            dir.path().join("actors-plus").join("module.json"),
            format!(r#"{{"id":"actors-plus","version":"1.0.0","engines":{{"shadowcat":"^{}"}}}}"#, env!("CARGO_PKG_VERSION")),
        )
        .unwrap();
        let (gm, _pl, world_id) = logged_in_gm_and_player_with_modules_dir(dir.path()).await;
        gm.put(&format!("/api/worlds/{world_id}/enabled-modules"))
            .json(&serde_json::json!(["actors-plus", "ghost"]))
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        let got: serde_json::Value = gm.get(&format!("/api/worlds/{world_id}/enabled-modules")).await.json();
        assert_eq!(got, serde_json::json!([]), "a valid id in a rejected batch must not partially apply");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run (from `src/server/`): `cargo test -p shadowcat enabled_modules_`
Expected: FAIL — route `/api/worlds/{id}/enabled-modules` does not exist.

- [ ] **Step 3: Implement** — append to `src/server/src/http/module_routes.rs`, before `#[cfg(test)]`:

```rust
use axum::extract::Path as AxumPath; // disambiguated below where both single- and tuple-Path extractors appear in this file
use uuid::Uuid;

use crate::http::routes::require_gm;

/// Upper bound on a world's enabled-module set. Parsed on every read/write and
/// broadcast (via the `Welcome`-time merge) — far above any realistic install.
const MAX_ENABLED_MODULES: usize = 256;

/// A world's enabled installed-module ids. Any member (needed at join to load
/// the enabled set) — mirrors `list_members`'s any-member-may-read stance.
pub async fn get_world_enabled_modules(
    user: AuthUser,
    State(state): State<AppState>,
    AxumPath(world): AxumPath<Uuid>,
) -> Result<Json<Vec<String>>, AppError> {
    state
        .repo
        .permission_context(world, user.id, user.role)
        .await?;
    Ok(Json(state.repo.world_enabled_modules(world).await?))
}

/// Replace a world's enabled installed-module set. GM/admin only. Every id
/// must name a currently-installed, validly-manifested module whose
/// `engines.shadowcat` range is satisfied by the running server version (T6) —
/// enabling a version-incompatible or unknown module is rejected outright,
/// atomically (never partially applied).
pub async fn set_world_enabled_modules(
    user: AuthUser,
    State(state): State<AppState>,
    AxumPath(world): AxumPath<Uuid>,
    Json(ids): Json<Vec<String>>,
) -> Result<StatusCode, AppError> {
    require_gm(&state, &user, world).await?;
    if ids.len() > MAX_ENABLED_MODULES {
        return Err(AppError::Unprocessable(format!(
            "too many enabled modules (max {MAX_ENABLED_MODULES})"
        )));
    }
    let installed = crate::modules::scan_installed_modules(&state.config.modules_path());
    for id in &ids {
        let Some(m) = installed.iter().find(|m| &m.id == id) else {
            return Err(AppError::Unprocessable(format!(
                "module '{id}' is not installed"
            )));
        };
        if !crate::modules::engine_compat_ok(m) {
            return Err(AppError::Unprocessable(format!(
                "module '{id}' is incompatible with this server version (requires shadowcat {})",
                m.engines_shadowcat.as_deref().unwrap_or("(missing engines.shadowcat)")
            )));
        }
    }
    state.repo.set_world_enabled_modules(world, &ids).await?;
    Ok(StatusCode::NO_CONTENT)
}
```

Fix the earlier `Path` import collision: this file's `serve_module_file` handler (Task 5) already `use axum::extract::Path;` for its tuple extraction. Replace that import line with the shared, unambiguous one — at the top of `src/server/src/http/module_routes.rs`, change:

```rust
use axum::extract::Path;
```

to:

```rust
use axum::extract::Path as AxumPath;
```

and update `serve_module_file`'s signature to use `AxumPath` instead of `Path` (both usages — remove the duplicate `use axum::extract::Path as AxumPath;` line added above in this task's snippet, since it now lives at the top of the file once):

```rust
pub async fn serve_module_file(
    _user: AuthUser,
    State(state): State<AppState>,
    AxumPath((id, rel_path)): AxumPath<(String, String)>,
) -> Result<Response, AppError> {
```

In `src/server/src/http/mod.rs`, add the route right after `/modules/{id}/{*path}`:

```rust
        .route(
            "/api/worlds/{id}/enabled-modules",
            get(module_routes::get_world_enabled_modules).put(module_routes::set_world_enabled_modules),
        )
```

- [ ] **Step 4: Run full server suite + clippy to verify GREEN**

Run (from `src/server/`): `cargo test --all-targets`
Expected: PASS, including all 5 new tests.
Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/http/module_routes.rs src/server/src/http/mod.rs
git commit -m "feat(server/http): GET/PUT /api/worlds/{id}/enabled-modules (T6 gated, atomic)"
```

---

## Task 9: `Welcome.server_version` — deliver the running server version over the authenticated join broadcast

Delivers the running server's semver to the client's load-time engine-compat gate (Task 12/15) through the authenticated, per-session `Welcome` message it already receives — NOT the public `/api/config` endpoint (see the "running server version reaches the client via `Welcome`" design decision above for the security/round-trip rationale). `/api/config` is left untouched (public, `initialized`-only). The server-side enable-time gate (Task 8) is unaffected — it reads `CARGO_PKG_VERSION` directly.

**Files:**
- Modify: `src/server/src/ws/protocol.rs`, `src/server/src/ws/conn.rs`, `src/client/core/src/wire.ts`

**Interfaces:**
- Produces: `ServerMsg::Welcome` gains `server_version: String` (regenerated `src/types/generated/ServerMsg.ts`); the client Zod `ServerMsgSchema` welcome variant + `WireWelcome` type gain `server_version: string`. Consumed by Task 15 (`worldSession.#onWelcome`'s `w.server_version`).

Note (Task 10 interplay): Task 10 also edits the `ServerMsg::Welcome { ... }` construction site in `conn.rs` (it changes the `world_reqs` source). Task 9 adds a `server_version:` line to that same literal. The two edits touch different fields of the same struct literal and compose without conflict; Task 9 lands first.

- [ ] **Step 1: Write the failing test** — extend the existing `welcome_carries_caps_role_and_requirements` test in `src/server/src/ws/protocol.rs`'s `#[cfg(test)] mod tests` block: add a `server_version` field to the `ServerMsg::Welcome { ... }` constructor and assert it serializes. The constructor becomes:

```rust
        let w = ServerMsg::Welcome {
            world: Uuid::from_u128(1),
            current_seq: 0,
            server_time: 0,
            server_version: "0.0.0-test".to_string(),
            world_default_grants: CapabilityGrants::default(),
            user_role: WorldRole::Player,
            capability_requirements: Vec::new(),
            contract_declarations: Vec::new(),
        };
```

and add, alongside the existing assertions:

```rust
        assert_eq!(json["server_version"], "0.0.0-test");
```

- [ ] **Step 2: Run to verify it fails**

Run (from `src/server/`): `cargo test -p shadowcat welcome_carries_caps_role_and_requirements`
Expected: FAIL to compile — `ServerMsg::Welcome` has no `server_version` field.

- [ ] **Step 3: Implement** — in `src/server/src/ws/protocol.rs`, add the field to the `Welcome` variant (right after `server_time`):

```rust
        server_time: i64,
        /// The running server's semver (`CARGO_PKG_VERSION`). The client's
        /// load-time engine-compat gate checks each external module's
        /// `engines.shadowcat` range against this; delivered here (authenticated,
        /// per-session) rather than on public `/api/config` to avoid disclosing
        /// the exact build to unauthenticated callers.
        server_version: String,
```

Then set it at the construction site in `src/server/src/ws/conn.rs` (the `ServerMsg::Welcome { ... }` literal, right after `server_time: now_millis(),`):

```rust
            server_time: now_millis(),
            server_version: env!("CARGO_PKG_VERSION").to_string(),
```

- [ ] **Step 4: Run full server suite + clippy to verify GREEN**

Run (from `src/server/`): `cargo test --all-targets`
Expected: PASS; ts-rs regenerates `src/types/generated/ServerMsg.ts` (now with `server_version: string`) as a side effect.
Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 5: Mirror in the client Zod schema** — in `src/client/core/src/wire.ts`, add the field to the `welcome` object of `ServerMsgSchema` (right after `server_time: int,`):

```ts
    server_time: int,
    server_version: z.string(),
```

- [ ] **Step 6: Run the client suites**

Run (from repo root): `pnpm --filter @shadowcat/core test`
Expected: PASS (the wire round-trip/drift-guard tests accept the new field; a Welcome fixture missing `server_version` would fail the schema, confirming the mirror is enforced).
Run: `pnpm -r typecheck`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/server/src/ws/protocol.rs src/server/src/ws/conn.rs src/types/generated/ServerMsg.ts src/client/core/src/wire.ts
git commit -m "feat(server/ws): carry the running server version in the Welcome broadcast (T6 client gate source)"
```

---

## Task 10: `Welcome` broadcast — union enabled modules' `requirements`

**FLAGGED FOR BUDDY-CHECK** (second half of the per-world enable + capability-requirements publish boundary — see Buddy-check directives).

**Files:**
- Modify: `src/server/src/ws/conn.rs`

**Interfaces:**
- Consumes: `Repository::world_cap_requirements`, `world_enabled_modules` (Task 7); `scan_installed_modules` (Task 2); `Config::modules_path()` (Task 1).
- Produces: `egress_loop`'s new final parameter `modules_dir: std::path::PathBuf`; a new private `welcome_capability_requirements(...)` helper.

- [ ] **Step 1: Write the failing test** — add to the `#[cfg(test)] mod tests` block in `src/server/src/ws/conn.rs`, right after `egress_lag_triggers_resync_and_converges`:

```rust
    #[tokio::test]
    async fn welcome_unions_enabled_modules_requirements_with_gm_authored_ones() {
        use crate::data::document::CapabilityRequirement;

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("actors-plus")).unwrap();
        std::fs::write(
            dir.path().join("actors-plus").join("module.json"),
            r#"{"id":"actors-plus","version":"1.0.0","requirements":[{"path_prefix":"/system/plus","caps":["plus:write"]}]}"#,
        )
        .unwrap();

        let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
        let gm = repo.create_user("gm", None, ServerRole::User, 0).await.unwrap();
        let world = repo.create_world_owned("W", gm, 0).await.unwrap();
        // A GM-authored requirement, unrelated to any module.
        repo.set_world_cap_requirements(
            world.id,
            &[CapabilityRequirement {
                path_prefix: "/system/vision".into(),
                caps: ["dnd5e:gm_vision".to_string()].into_iter().collect(),
            }],
        )
        .await
        .unwrap();

        // With nothing enabled, only the GM-authored requirement is published.
        let reqs = welcome_capability_requirements(repo.as_ref(), world.id, dir.path()).await;
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].path_prefix, "/system/vision");

        // Enabling the module adds its requirement WITHOUT removing the GM's own.
        repo.set_world_enabled_modules(world.id, &["actors-plus".to_string()])
            .await
            .unwrap();
        let reqs = welcome_capability_requirements(repo.as_ref(), world.id, dir.path()).await;
        assert_eq!(reqs.len(), 2);
        assert!(reqs.iter().any(|r| r.path_prefix == "/system/vision"));
        assert!(reqs.iter().any(|r| r.path_prefix == "/system/plus"));

        // world_cap_requirements itself is never mutated by this — the raw GM
        // record still holds exactly its one original entry.
        assert_eq!(repo.world_cap_requirements(world.id).await.unwrap().len(), 1);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run (from `src/server/`): `cargo test -p shadowcat welcome_unions_enabled_modules_requirements`
Expected: FAIL to compile — no function `welcome_capability_requirements`.

- [ ] **Step 3: Implement the helper**

In `src/server/src/ws/conn.rs`, add right before `async fn egress_loop<S>(`:

```rust
/// Union `world_reqs` (GM-authored, unchanged) with the `requirements`
/// declared by each of the world's currently ENABLED installed modules (M13-1
/// §2 — "enabling a module publishes its manifest requirements through the
/// capability machinery"). Non-destructive: `world_cap_requirements` itself is
/// NEVER mutated by enable/disable; this union is recomputed fresh on every
/// `Welcome`, so a mid-session enable/disable takes effect on the affected
/// world's next (re)connect, exactly like a `world_cap_requirements` edit
/// already does today.
async fn welcome_capability_requirements(
    repo: &dyn Repository,
    world_id: Uuid,
    modules_dir: &std::path::Path,
) -> Vec<crate::data::document::CapabilityRequirement> {
    let mut out = match repo.world_cap_requirements(world_id).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(world = %world_id, error = %e, "capability requirements unreadable; sending empty");
            Vec::new()
        }
    };
    let enabled = match repo.world_enabled_modules(world_id).await {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(world = %world_id, error = %e, "enabled modules unreadable; skipping module-published requirements");
            Vec::new()
        }
    };
    if !enabled.is_empty() {
        let installed = crate::modules::scan_installed_modules(modules_dir);
        for id in &enabled {
            if let Some(m) = installed.iter().find(|m| &m.id == id) {
                out.extend(m.requirements.iter().cloned());
            }
        }
    }
    out
}
```

- [ ] **Step 4: Thread `modules_dir` through `egress_loop` and its call sites**

In `src/server/src/ws/conn.rs`, change the `egress_loop` signature (add the new final parameter):

```rust
async fn egress_loop<S>(
    mut sink: S,
    mut rx: tokio::sync::broadcast::Receiver<Arc<ServerMsg>>,
    mut erx: mpsc::Receiver<Egress>,
    room: Arc<Room>,
    repo: Arc<SqliteRepository>,
    ctx: PermissionContext,
    current_seq: i64,
    modules_dir: std::path::PathBuf,
) where
    S: Sink<Message> + Unpin,
{
```

Replace the existing `world_reqs` assignment inside `egress_loop`:

```rust
    let world_reqs = match repo.world_cap_requirements(world_id).await {
        Ok(r) => r,
        Err(e) => {
            // Fail open for the advisory client copy only; server-side
            // enforcement reads requirements freshly per intent and fails closed.
            tracing::warn!(world = %world_id, error = %e, "capability requirements unreadable; sending empty");
            Vec::new()
        }
    };
```

with:

```rust
    // Fail open for the advisory client copy only; server-side enforcement
    // reads requirements freshly per intent and fails closed.
    let world_reqs = welcome_capability_requirements(repo.as_ref(), world_id, &modules_dir).await;
```

Update the production call site in `handle_socket` (right after `let egress_repo = repo.clone();`):

```rust
    let egress_room = room.clone();
    let egress_repo = repo.clone();
    let modules_dir = state.config.modules_path();
    let mut egress = tokio::spawn(egress_loop(
        sink,
        rx,
        erx,
        egress_room,
        egress_repo,
        ctx,
        current_seq,
        modules_dir,
    ));
```

Update the test call site inside `egress_lag_triggers_resync_and_converges` — the existing `tokio::spawn(egress_loop(...))` call — add a nonexistent-path final argument (the test does not care about module-published requirements; a missing dir scans as empty per Task 2):

```rust
        let egress = tokio::spawn(egress_loop(
            sink,
            rx,
            erx,
            room.clone(),
            repo.clone(),
            ctx,
            current_seq,
            std::path::PathBuf::from("nonexistent-test-modules-dir-for-egress-lag-test"),
        ));
```

- [ ] **Step 5: Run full server suite + clippy to verify GREEN**

Run (from `src/server/`): `cargo test --all-targets`
Expected: PASS, including `welcome_unions_enabled_modules_requirements_with_gm_authored_ones` and the unmodified `egress_lag_triggers_resync_and_converges`.
Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/ws/conn.rs
git commit -m "feat(server/ws): union enabled modules' requirements into the Welcome broadcast (non-destructive)"
```

---

## Task 11: Client core — optional `engines` field on `ModuleManifest`

**Files:**
- Modify: `src/client/core/src/manifest.ts`, `src/client/core/src/manifest.test.ts` (create if it does not already exist — check first), `src/client/core/src/index.ts`

**Interfaces:**
- Produces: `export interface ModuleEngines { shadowcat: string }`; `ModuleManifest.engines?: ModuleEngines`; `ManifestSchema` accepts/validates it.

- [ ] **Step 0: Check for an existing manifest test file**

Run: `Glob src/client/core/src/manifest.test.ts` — if it exists, add the new test into it; if not, create it fresh with just this one test plus a minimal existing-behavior smoke test so the file is self-sufficient.

- [ ] **Step 1: Write the failing test** — in `src/client/core/src/manifest.test.ts` (create if absent):

```ts
import { expect, test } from "vitest";
import { parseManifest } from "./manifest";

test("a manifest with no engines field parses (first-party modules never set it)", () => {
  const m = parseManifest({ id: "a", version: "1.0.0", dependencies: {} });
  expect(m.engines).toBeUndefined();
});

test("a manifest with a valid engines.shadowcat range parses", () => {
  const m = parseManifest({
    id: "a",
    version: "1.0.0",
    dependencies: {},
    engines: { shadowcat: "^0.1.0" },
  });
  expect(m.engines?.shadowcat).toBe("^0.1.0");
});

test("an empty engines.shadowcat string is rejected", () => {
  expect(() =>
    parseManifest({
      id: "a",
      version: "1.0.0",
      dependencies: {},
      engines: { shadowcat: "" },
    }),
  ).toThrow();
});
```

- [ ] **Step 2: Run to verify it fails**

Run (from repo root): `pnpm --filter @shadowcat/core test -- manifest`
Expected: FAIL — `m.engines` type error at compile/typecheck, or the third test's `toThrow()` does not throw (an unknown `engines` key is currently silently accepted/ignored by Zod's default non-strict object schema, so it PASSES parse today with no validation — confirms the RED state either way).

- [ ] **Step 3: Implement** — in `src/client/core/src/manifest.ts`:

Add the new type, right after `ContractDeclaration`:

```ts
/** Minimal engine-compat gate (T6, M13-1). Optional on the shared manifest
 * shape (first-party modules never set it — they ship version-locked inside
 * the binary); the modules-folder install/enable/load pipeline treats a
 * missing or unsatisfied range as a hard reject for community modules
 * specifically (see `loader.ts`'s `checkEngineCompat` and the server's
 * `engine_compat_ok`). */
export interface ModuleEngines {
  shadowcat: string;
}
```

Add the field to `ModuleManifest` (after `requires?: string[];`):

```ts
  engines?: ModuleEngines;
```

Add the Zod schema, right after `const CapRequirementSchema = ...;`:

```ts
const ModuleEnginesSchema = z.object({ shadowcat: z.string().min(1) });
```

Add the field to `ManifestSchema`'s object (after `requires: z.array(z.string()).optional(),`):

```ts
  engines: ModuleEnginesSchema.optional(),
```

- [ ] **Step 4: Export the new type**

In `src/client/core/src/index.ts`, extend the existing manifest type export line:

```ts
export type {
  ModuleManifest,
  ModuleEngines,
  CapRequirement,
  HookDecl,
  ContractProvide,
  ContractDeclaration,
} from "./manifest";
```

- [ ] **Step 5: Run full core suite + typecheck to verify GREEN**

Run (from repo root): `pnpm --filter @shadowcat/core test`
Expected: PASS.
Run: `pnpm -r typecheck`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/client/core/src/manifest.ts src/client/core/src/manifest.test.ts src/client/core/src/index.ts
git commit -m "feat(core/manifest): optional engines.shadowcat field (T6)"
```

---

## Task 12: Client core — contained `loadModules` + engine-compat check

**Files:**
- Modify: `src/client/core/src/loader.ts`, `src/client/core/src/loader.test.ts`

**Interfaces:**
- Consumes: `satisfies` (`src/client/core/src/semver.ts`, already exists); `ModuleEngines` (Task 11).
- Produces: `loadModules(...): Promise<ModuleLoadResult>` (was `Promise<void>`, threw on any entry's failure); `export interface ModuleLoadFailure { id: string; entry: string; error: string }`; `export interface ModuleLoadResult { loaded: string[]; failed: ModuleLoadFailure[] }`; `loadModules`'s new optional `shadowcatVersion?: string` option.

- [ ] **Step 1: Rewrite the failing tests** — replace the full contents of `src/client/core/src/loader.test.ts`:

```ts
import { expect, test, vi } from "vitest";
import { loadModules } from "./loader";
import { ModuleRegistry, type Module } from "./modules";
import { HookBus } from "./hooks";
import { ServiceRegistry } from "./services";
import { MiddlewareChain } from "./middleware";
import { DocumentStore } from "./store";
import { OptimisticClient } from "./optimistic";
import { ContributionRegistry } from "./contributions";
import { silentLogger } from "./logger";

function registry() {
  return new ModuleRegistry({
    hooks: new HookBus(silentLogger),
    services: new ServiceRegistry(),
    middleware: new MiddlewareChain(),
    store: new DocumentStore(),
    client: new OptimisticClient("self"),
    logger: silentLogger,
    contributions: new ContributionRegistry(),
  });
}

const mod: Module = {
  manifest: { id: "a", version: "1.0.0", dependencies: {} },
  register: vi.fn(),
};

test("loadModules imports entries, adds them to the registry, and reports loaded ids", async () => {
  const r = registry();
  const importFn = vi.fn(async () => ({ default: mod }));
  const result = await loadModules({
    entries: [{ manifest: mod.manifest, entry: "./a.js" }],
    importFn,
    registry: r,
  });
  expect(importFn).toHaveBeenCalledWith("./a.js");
  expect(r.list().map((m) => m.id)).toEqual(["a"]);
  expect(result.loaded).toEqual(["a"]);
  expect(result.failed).toEqual([]);
});

test("a namespace export (no default) is accepted", async () => {
  const r = registry();
  const result = await loadModules({
    entries: [{ manifest: mod.manifest, entry: "./a.js" }],
    importFn: async () => mod,
    registry: r,
  });
  expect(r.list()).toHaveLength(1);
  expect(result.loaded).toEqual(["a"]);
});

test("a manifest id mismatch is contained per-module, not thrown", async () => {
  const r = registry();
  const result = await loadModules({
    entries: [
      { manifest: { id: "declared", version: "1.0.0", dependencies: {} }, entry: "./a.js" },
    ],
    importFn: async () => mod, // module's own id is "a"
    registry: r,
  });
  expect(result.loaded).toEqual([]);
  expect(result.failed).toHaveLength(1);
  expect(result.failed[0].id).toBe("declared");
  expect(result.failed[0].error).toMatch(/id/i);
  expect(r.list()).toHaveLength(0);
});

test("one failing entry does not block a later valid one", async () => {
  const r = registry();
  const good: Module = {
    manifest: { id: "b", version: "1.0.0", dependencies: {} },
    register: vi.fn(),
  };
  const result = await loadModules({
    entries: [
      { manifest: { id: "declared", version: "1.0.0", dependencies: {} }, entry: "./a.js" },
      { manifest: good.manifest, entry: "./b.js" },
    ],
    importFn: async (entry) => (entry === "./a.js" ? mod : good),
    registry: r,
  });
  expect(result.loaded).toEqual(["b"]);
  expect(result.failed.map((f) => f.id)).toEqual(["declared"]);
});

test("an engine-compat mismatch is contained and reported", async () => {
  const r = registry();
  const incompatible: Module = {
    manifest: { id: "c", version: "1.0.0", dependencies: {}, engines: { shadowcat: "^2.0.0" } },
    register: vi.fn(),
  };
  const result = await loadModules({
    entries: [{ manifest: incompatible.manifest, entry: "./c.js" }],
    importFn: async () => incompatible,
    registry: r,
    shadowcatVersion: "1.0.0",
  });
  expect(result.loaded).toEqual([]);
  expect(result.failed).toHaveLength(1);
  expect(result.failed[0].id).toBe("c");
  expect(result.failed[0].error).toMatch(/shadowcat/i);
});

test("shadowcatVersion is optional: compat is skipped entirely when omitted", async () => {
  const r = registry();
  const withRange: Module = {
    manifest: { id: "d", version: "1.0.0", dependencies: {}, engines: { shadowcat: "^99.0.0" } },
    register: vi.fn(),
  };
  const result = await loadModules({
    entries: [{ manifest: withRange.manifest, entry: "./d.js" }],
    importFn: async () => withRange,
    registry: r,
  });
  expect(result.loaded).toEqual(["d"]);
});

test("a manifest with no engines field always passes compat, even when shadowcatVersion is given", async () => {
  const r = registry();
  const result = await loadModules({
    entries: [{ manifest: mod.manifest, entry: "./a.js" }],
    importFn: async () => mod,
    registry: r,
    shadowcatVersion: "1.0.0",
  });
  expect(result.loaded).toEqual(["a"]);
});
```

- [ ] **Step 2: Run to verify it fails**

Run (from repo root): `pnpm --filter @shadowcat/core test -- loader`
Expected: FAIL — `result.loaded`/`result.failed` are `undefined` (current `loadModules` resolves `void`); the id-mismatch test's `rejects.toThrow` pattern is gone, replaced by non-throwing assertions that fail against the current throwing implementation.

- [ ] **Step 3: Implement** — replace the full contents of `src/client/core/src/loader.ts`:

```ts
// Thin delivery adapter: turns discovered (manifest, entry) pairs into Module
// objects via an injectable importFn and hands them to the registry. Discovery
// (filesystem in Node, fetch in the browser) is the host's job; the adapter
// stays environment-neutral so a future sandboxed delivery is another importFn.
// Every entry loads in isolation: a parse/compat/import/id-mismatch failure on
// one entry never aborts the batch — a broken community module must degrade to
// a reported failure, never brick every other module in the load list (M13-1 §3).
import { ModuleRegistry, type Module } from "./modules";
import { parseManifest, type ModuleManifest } from "./manifest";
import { satisfies } from "./semver";

export type ImportFn = (entry: string) => Promise<{ default: Module } | Module>;

export interface ModuleEntry {
  manifest: ModuleManifest;
  entry: string;
}

/** One entry that failed to load, with its declared id and the failure reason. */
export interface ModuleLoadFailure {
  id: string;
  entry: string;
  error: string;
}

export interface ModuleLoadResult {
  /** Module ids successfully imported and added to the registry. */
  loaded: string[];
  /** Entries that failed at any stage (manifest parse, engine compat, import, id mismatch). */
  failed: ModuleLoadFailure[];
}

function normalize(imported: { default: Module } | Module): Module {
  return "default" in imported && (imported as { default: Module }).default
    ? (imported as { default: Module }).default
    : (imported as Module);
}

/** Throws when `manifest.engines.shadowcat` is set and `shadowcatVersion` does
 * not satisfy it. A missing `engines.shadowcat` is NOT an error here — the
 * field is optional on the shared manifest shape (first-party modules never
 * set it); the modules-folder pipeline's enable/load gate is what makes it
 * effectively required for community modules (T6). */
function checkEngineCompat(manifest: ModuleManifest, shadowcatVersion: string): void {
  const range = manifest.engines?.shadowcat;
  if (!range) return;
  if (!satisfies(shadowcatVersion, range)) {
    throw new Error(
      `module ${manifest.id} requires shadowcat ${range}, running ${shadowcatVersion}`,
    );
  }
}

export async function loadModules(opts: {
  entries: ModuleEntry[];
  importFn: ImportFn;
  registry: ModuleRegistry;
  /** When provided, each entry's `engines.shadowcat` (if declared) is checked
   * against this version before import (T6 load-time gate). */
  shadowcatVersion?: string;
}): Promise<ModuleLoadResult> {
  const loaded: string[] = [];
  const failed: ModuleLoadFailure[] = [];
  for (const { manifest, entry } of opts.entries) {
    try {
      // Validates the *discovered* manifest; ModuleRegistry.add re-parses the
      // module's *own* manifest. Two distinct sources, bridged by the id check
      // below — both parses are intentional.
      parseManifest(manifest);
      if (opts.shadowcatVersion) checkEngineCompat(manifest, opts.shadowcatVersion);
      const module = normalize(await opts.importFn(entry));
      if (module.manifest.id !== manifest.id) {
        throw new Error(
          `module at ${entry} declares id ${module.manifest.id}, manifest says ${manifest.id}`,
        );
      }
      opts.registry.add(module);
      loaded.push(manifest.id);
    } catch (e) {
      failed.push({
        id: manifest.id,
        entry,
        error: e instanceof Error ? e.message : String(e),
      });
    }
  }
  return { loaded, failed };
}
```

- [ ] **Step 4: Export the new types**

In `src/client/core/src/index.ts`, extend the existing loader export line:

```ts
export { loadModules } from "./loader";
export type { ImportFn, ModuleEntry, ModuleLoadFailure, ModuleLoadResult } from "./loader";
```

- [ ] **Step 5: Run full core suite + typecheck to verify GREEN**

Run (from repo root): `pnpm --filter @shadowcat/core test`
Expected: PASS, all 7 loader tests.
Run: `pnpm -r typecheck`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/client/core/src/loader.ts src/client/core/src/loader.test.ts src/client/core/src/index.ts
git commit -m "feat(core/loader): per-module load containment + T6 engine-compat gate"
```

---

## Task 13: Client core — `module-rest.ts`

**Files:**
- Create: `src/client/core/src/module-rest.ts`, `src/client/core/src/module-rest.test.ts`
- Modify: `src/client/core/src/index.ts`

**Interfaces:**
- Consumes: `InstalledModuleInfo` (`@shadowcat/types`, Task 4).
- Produces: `listInstalledModules(): Promise<InstalledModuleInfo[]>`; `getEnabledModules(world: string): Promise<string[]>`; `setEnabledModules(world: string, ids: string[]): Promise<void>`.

- [ ] **Step 1: Write the failing tests** — create `src/client/core/src/module-rest.test.ts`:

```ts
import { expect, test, vi, afterEach } from "vitest";
import { listInstalledModules, getEnabledModules, setEnabledModules } from "./module-rest";

afterEach(() => {
  vi.unstubAllGlobals();
});

test("listInstalledModules GETs /api/modules and returns the parsed array", async () => {
  const fetchMock = vi.fn().mockResolvedValue({
    ok: true,
    json: async () => [{ manifest: { id: "a" }, entry_url: "/modules/a/index.js" }],
  });
  vi.stubGlobal("fetch", fetchMock);
  const got = await listInstalledModules();
  expect(fetchMock).toHaveBeenCalledWith("/api/modules", expect.any(Object));
  expect(got).toEqual([{ manifest: { id: "a" }, entry_url: "/modules/a/index.js" }]);
});

test("listInstalledModules throws on a non-ok response", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: false, status: 401 }));
  await expect(listInstalledModules()).rejects.toThrow(/401/);
});

test("getEnabledModules GETs the world's enabled-modules endpoint", async () => {
  const fetchMock = vi.fn().mockResolvedValue({ ok: true, json: async () => ["a", "b"] });
  vi.stubGlobal("fetch", fetchMock);
  const got = await getEnabledModules("w1");
  expect(fetchMock).toHaveBeenCalledWith("/api/worlds/w1/enabled-modules", expect.any(Object));
  expect(got).toEqual(["a", "b"]);
});

test("setEnabledModules PUTs the ids as a JSON body", async () => {
  const fetchMock = vi.fn().mockResolvedValue({ ok: true });
  vi.stubGlobal("fetch", fetchMock);
  await setEnabledModules("w1", ["a", "b"]);
  expect(fetchMock).toHaveBeenCalledWith(
    "/api/worlds/w1/enabled-modules",
    expect.objectContaining({
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(["a", "b"]),
    }),
  );
});

test("setEnabledModules throws on a non-ok response", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: false, status: 422 }));
  await expect(setEnabledModules("w1", ["a"])).rejects.toThrow(/422/);
});
```

- [ ] **Step 2: Run to verify it fails**

Run (from repo root): `pnpm --filter @shadowcat/core test -- module-rest`
Expected: FAIL — `Cannot find module './module-rest'`.

- [ ] **Step 3: Implement** — create `src/client/core/src/module-rest.ts`:

```ts
import type { InstalledModuleInfo } from "@shadowcat/types";

// Client-side module-toolchain REST, beside asset-rest.ts: the installed-module
// discovery + per-world enablement contract with the server. Framework-neutral
// (no Svelte in core's closure, invariant #7) — shared by the settings module's
// GM management UI and the world session's external-module load path.

/** Every validly installed module the server discovered under its modules folder. */
export async function listInstalledModules(): Promise<InstalledModuleInfo[]> {
  const res = await fetch("/api/modules", { headers: { accept: "application/json" } });
  if (!res.ok) throw new Error(`list installed modules failed: ${res.status}`);
  return (await res.json()) as InstalledModuleInfo[];
}

/** A world's enabled installed-module ids. Any world member may read this
 * (needed at join to load the enabled set). */
export async function getEnabledModules(world: string): Promise<string[]> {
  const res = await fetch(`/api/worlds/${world}/enabled-modules`, {
    headers: { accept: "application/json" },
  });
  if (!res.ok) throw new Error(`get enabled modules failed: ${res.status}`);
  return (await res.json()) as string[];
}

/** Replace a world's enabled installed-module set. GM/admin only server-side
 * (a non-GM caller gets a 403, surfaced via the thrown error). */
export async function setEnabledModules(world: string, ids: string[]): Promise<void> {
  const res = await fetch(`/api/worlds/${world}/enabled-modules`, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(ids),
  });
  if (!res.ok) throw new Error(`set enabled modules failed: ${res.status}`);
}
```

- [ ] **Step 4: Export from the barrel**

In `src/client/core/src/index.ts`, add after the `asset-rest` export line:

```ts
export { listInstalledModules, getEnabledModules, setEnabledModules } from "./module-rest";
```

- [ ] **Step 5: Run full core suite + typecheck to verify GREEN**

Run (from repo root): `pnpm --filter @shadowcat/core test`
Expected: PASS, all 5 module-rest tests.
Run: `pnpm -r typecheck`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/client/core/src/module-rest.ts src/client/core/src/module-rest.test.ts src/client/core/src/index.ts
git commit -m "feat(core): module-rest.ts — installed-module discovery + per-world enable REST"
```

---

## Task 14: Shell — shared-runtime ESM chunks + build-time import map

**Files:**
- Modify: `src/client/shell/vite.config.ts`, `src/client/shell/index.html`, `src/client/shell/package.json`
- Create: `src/client/shell/src/lib/importMap.test.ts`

**Interfaces:**
- Consumes: `@shadowcat/formula` package (M13a, parallel track — see Step 0 guard).
- Produces: deterministic build output `dist/runtime/{svelte,svelte-internal-client,svelte-internal-disclose-version,svelte-reactivity,shadowcat-core,shadowcat-ui-kit,shadowcat-formula,shadowcat-types}.js`; a `<script type="importmap">` in `dist/index.html` mapping each bare specifier (and `svelte/` subpaths actually used in this codebase) to its chunk.

- [ ] **Step 0: Guard — verify `@shadowcat/formula` exists before proceeding**

Run: `Glob src/client/formula/package.json`
Expected: a match. If this package does not yet exist (M13a has not landed on this branch), STOP this task and report the blocker — do not fabricate a package. (See the plan-writer's final report: this is a known cross-plan coordination dependency, not something to route around.)

- [ ] **Step 1: Write the failing test** — create `src/client/shell/src/lib/importMap.test.ts`:

```ts
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { describe, it, expect } from "vitest";

// .../src/client/shell/src/lib/importMap.test.ts -> repo root is five levels up.
const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../../../..");
const distDir = path.join(repoRoot, "dist");

const RUNTIME_CHUNKS = [
  "svelte",
  "svelte-internal-client",
  "svelte-internal-disclose-version",
  "svelte-reactivity",
  "shadowcat-core",
  "shadowcat-ui-kit",
  "shadowcat-formula",
  "shadowcat-types",
];

describe("shared-runtime import map (build output)", () => {
  // Mirrors embed.rs's `dist_built()` self-skip: this test only means anything
  // after `pnpm --filter @shadowcat/shell build` has run.
  if (!existsSync(path.join(distDir, "index.html"))) {
    it.skip("dist/ not built — run `pnpm --filter @shadowcat/shell build` first", () => {});
    return;
  }

  it("emits a stable-named chunk for every shared runtime", () => {
    for (const name of RUNTIME_CHUNKS) {
      const file = path.join(distDir, "runtime", `${name}.js`);
      expect(existsSync(file), `expected ${file} to exist`).toBe(true);
    }
  });

  it("index.html carries an import map pointing every bare specifier at its chunk", () => {
    const html = readFileSync(path.join(distDir, "index.html"), "utf-8");
    const match = /<script type="importmap">([\s\S]*?)<\/script>/.exec(html);
    expect(match, "no <script type=\"importmap\"> found in dist/index.html").not.toBeNull();
    const map = JSON.parse(match![1]) as { imports: Record<string, string> };
    expect(map.imports["svelte"]).toBe("/runtime/svelte.js");
    expect(map.imports["svelte/internal/client"]).toBe("/runtime/svelte-internal-client.js");
    expect(map.imports["svelte/internal/disclose-version"]).toBe(
      "/runtime/svelte-internal-disclose-version.js",
    );
    expect(map.imports["svelte/reactivity"]).toBe("/runtime/svelte-reactivity.js");
    expect(map.imports["@shadowcat/core"]).toBe("/runtime/shadowcat-core.js");
    expect(map.imports["@shadowcat/ui-kit"]).toBe("/runtime/shadowcat-ui-kit.js");
    expect(map.imports["@shadowcat/formula"]).toBe("/runtime/shadowcat-formula.js");
    expect(map.imports["@shadowcat/types"]).toBe("/runtime/shadowcat-types.js");

    // The import map must precede the app's own module entry script (§3:
    // "injected before any module script executes").
    const mapIdx = html.indexOf('<script type="importmap">');
    const appIdx = html.indexOf('<script type="module"');
    expect(mapIdx).toBeGreaterThanOrEqual(0);
    expect(appIdx).toBeGreaterThan(mapIdx);
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run (from repo root): `pnpm --filter @shadowcat/shell test -- importMap`
Expected: the test self-skips (`dist/` not yet built with the new config) — confirm the skip message prints, proving the guard itself works. This IS the RED state for this task: nothing meaningful can assert until Step 3's build.

- [ ] **Step 3: Add `@shadowcat/formula` as a shell dependency**

In `src/client/shell/package.json`, add to `dependencies` (alphabetically, right after `"@shadowcat/core": "workspace:*",`):

```json
    "@shadowcat/formula": "workspace:*",
```

Run (from repo root): `pnpm install`
Expected: resolves cleanly (workspace link).

- [ ] **Step 4: Implement the Vite config**

Replace the full contents of `src/client/shell/vite.config.ts`:

```ts
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Dev: the SPA is served by Vite; /api and /ws proxy to the Rust server so
// `vite dev` runs against a real backend. SHADOWCAT_SERVER overrides the target.
const target = process.env.SHADOWCAT_SERVER ?? "http://127.0.0.1:30000";

// Shared-runtime ESM entry chunks (M13-1 T1/T3): each bare specifier below is
// ALSO a genuine Rollup entry point in this same multi-entry build, so Rollup's
// standard entry-sharing dedup makes the app's own first-party bundle AND any
// future external module import the SAME runtime instance — never a second
// copy. Chunk filenames are forced stable (`runtime/<name>.js`, no content
// hash) so `index.html`'s import map below can reference them at build time.
// This set is the empirically-used surface in THIS codebase (grep for
// `from "svelte` across src/): `svelte` (user imports), `svelte/reactivity`
// (SvelteMap, widely used), plus the two internal subpaths every compiled
// Svelte 5 component imports regardless of author code
// (`svelte/internal/client`, `svelte/internal/disclose-version`). A module
// author introducing a NEW svelte/* subpath (e.g. `svelte/store`,
// `svelte/transition`) needs this list extended — see
// docs/design/module-authoring.md.
const RUNTIME_ENTRIES: Record<string, string> = {
  svelte: "svelte",
  "svelte-internal-client": "svelte/internal/client",
  "svelte-internal-disclose-version": "svelte/internal/disclose-version",
  "svelte-reactivity": "svelte/reactivity",
  "shadowcat-core": "@shadowcat/core",
  "shadowcat-ui-kit": "@shadowcat/ui-kit",
  "shadowcat-formula": "@shadowcat/formula",
  "shadowcat-types": "@shadowcat/types",
};

export default defineConfig({
  plugins: [svelte()],
  build: {
    outDir: "../../../dist",
    emptyOutDir: true,
    rollupOptions: {
      input: {
        main: fileURLToPath(new URL("./index.html", import.meta.url)),
        ...RUNTIME_ENTRIES,
      },
      output: {
        entryFileNames: (chunk) =>
          chunk.name && chunk.name in RUNTIME_ENTRIES
            ? `runtime/${chunk.name}.js`
            : "assets/[name]-[hash].js",
      },
    },
  },
  server: {
    proxy: {
      "/api": { target, changeOrigin: true },
      "/ws": { target, ws: true, changeOrigin: true },
    },
  },
});
```

- [ ] **Step 5: Add the build-time import map to `index.html`**

In `src/client/shell/index.html`, add the `<script type="importmap">` in `<head>`, BEFORE the closing `</head>` tag (after the existing `<title>` line):

```html
    <title>shadowcat</title>
    <script type="importmap">
      {
        "imports": {
          "svelte": "/runtime/svelte.js",
          "svelte/internal/client": "/runtime/svelte-internal-client.js",
          "svelte/internal/disclose-version": "/runtime/svelte-internal-disclose-version.js",
          "svelte/reactivity": "/runtime/svelte-reactivity.js",
          "@shadowcat/core": "/runtime/shadowcat-core.js",
          "@shadowcat/ui-kit": "/runtime/shadowcat-ui-kit.js",
          "@shadowcat/formula": "/runtime/shadowcat-formula.js",
          "@shadowcat/types": "/runtime/shadowcat-types.js"
        }
      }
    </script>
```

- [ ] **Step 6: Build and verify**

Run (from repo root): `pnpm --filter @shadowcat/shell build`
Expected: succeeds; `dist/runtime/*.js` exist for all 8 entries; `dist/index.html` contains the import map before the app's module script tag.
Run: `pnpm --filter @shadowcat/shell test -- importMap`
Expected: PASS (both tests now exercise real output).

- [ ] **Step 7: Run the full shell suite + typecheck**

Run (from repo root): `pnpm --filter @shadowcat/shell test`
Expected: PASS (unrelated tests unaffected).
Run: `pnpm -r typecheck`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src/client/shell/vite.config.ts src/client/shell/index.html src/client/shell/package.json src/client/shell/src/lib/importMap.test.ts pnpm-lock.yaml
git commit -m "feat(shell): shared-runtime ESM chunks + build-time import map (T1/T3)"
```

---

## Task 15: `worldSession.svelte.ts` — load enabled external modules after Welcome

**Files:**
- Modify: `src/client/shell/src/lib/worldSession.svelte.ts`, `src/client/shell/src/lib/worldSession.test.ts`

**Interfaces:**
- Consumes: `loadModules`, `type ModuleEntry`, `type ModuleManifest`, `listInstalledModules`, `getEnabledModules` (`@shadowcat/core`); the server version from the `Welcome` payload (`w.server_version`, Task 9) — NOT a `getConfig()` fetch.
- Produces: `WorldSession`'s new private `#loadExternalModules(world: string, serverVersion: string): Promise<void>`, invoked once per session inside the existing `#bootstrapped` guard.

- [ ] **Step 1: Extend the test mocks + write the failing tests**

In `src/client/shell/src/lib/worldSession.test.ts`, replace the existing `vi.mock("./api", ...)` block:

```ts
// The members fetch hits the network; stub it (safe default, overridable per
// test). The server version now arrives on the Welcome payload, not a fetch —
// ensure this file's mock Welcome message (built in `mockConnect`) includes a
// `server_version: "0.1.0"` field, since the `WireWelcome` schema now requires it.
vi.mock("./api", async (importActual) => {
  const actual = await importActual<typeof import("./api")>();
  return {
    ...actual,
    listWorldMembers: vi.fn().mockResolvedValue([]),
  };
});

// The external-module discovery fetches (installed set + a world's enabled
// set) also hit the network; default to "nothing enabled" so the 25+ existing
// Welcome-flow tests below are unaffected (empty enabled set → #loadExternalModules
// returns before ever calling loadModules/import()).
vi.mock("@shadowcat/core", async (importActual) => {
  const actual = await importActual<typeof import("@shadowcat/core")>();
  return {
    ...actual,
    listInstalledModules: vi.fn().mockResolvedValue([]),
    getEnabledModules: vi.fn().mockResolvedValue([]),
  };
});
```

Add new tests at the end of the file:

```ts
test("Welcome warns (but still enters the world) when an enabled id is not installed", async () => {
  const core = await import("@shadowcat/core");
  vi.mocked(core.getEnabledModules).mockResolvedValueOnce(["missing-mod"]);
  vi.mocked(core.listInstalledModules).mockResolvedValueOnce([]);
  const warnings: unknown[][] = [];
  const logger = { ...silentLogger, warn: (...args: unknown[]) => warnings.push(args) };
  const session = new WorldSession({
    selfId: "u1",
    connect: mockConnect(),
    modules: [coreUiStub],
    logger,
  });
  await session.enter("w1");
  await vi.waitFor(() => expect(session.role).toBe("player"));
  await vi.waitFor(() =>
    expect(warnings.some((a) => String(a[0]).includes("missing-mod"))).toBe(true),
  );
});

test("Welcome degrades gracefully when external module discovery fails (network error)", async () => {
  const core = await import("@shadowcat/core");
  vi.mocked(core.getEnabledModules).mockRejectedValueOnce(new Error("network down"));
  const warnings: unknown[][] = [];
  const logger = { ...silentLogger, warn: (...args: unknown[]) => warnings.push(args) };
  const session = new WorldSession({
    selfId: "u1",
    connect: mockConnect(),
    modules: [coreUiStub],
    logger,
  });
  await session.enter("w1");
  // The session still enters the world normally despite the discovery failure.
  await vi.waitFor(() => expect(session.role).toBe("player"));
  await vi.waitFor(() =>
    expect(
      warnings.some((a) => String(a[0]).includes("external module discovery failed")),
    ).toBe(true),
  );
});

test("an enabled set with nothing to load never calls listInstalledModules a second time on reconnect", async () => {
  const core = await import("@shadowcat/core");
  const session = new WorldSession({
    selfId: "u1",
    connect: mockConnect(2), // Welcome delivered twice (reconnect)
    modules: [coreUiStub],
    logger: silentLogger,
  });
  await session.enter("w1");
  await vi.waitFor(() => expect(session.role).toBe("player"));
  await vi.waitFor(() => expect(vi.mocked(core.getEnabledModules)).toHaveBeenCalledTimes(1));
  await Promise.resolve();
  // External-module loading is bootstrap-once, exactly like core-ui activation.
  expect(vi.mocked(core.getEnabledModules)).toHaveBeenCalledTimes(1);
});
```

Add the `silentLogger` import to the top of the file (extend the existing `@shadowcat/core` import list):

```ts
import {
  ContributionRegistry,
  silentLogger,
  buildTokenDoc,
  buildActorDoc,
  buildWorldSettingsDoc,
  buildSceneDoc,
  DEFAULT_WORLD_SETTINGS,
  type Connect,
  type WireDocument,
} from "@shadowcat/core";
```

- [ ] **Step 2: Run to verify it fails**

Run (from repo root): `pnpm --filter @shadowcat/shell test -- worldSession`
Expected: FAIL — the three new tests fail (no `#loadExternalModules` call exists yet, so `getEnabledModules`/`listInstalledModules` are never invoked and no warning is ever logged); the pre-existing 25 tests still PASS (the mocks default to a no-op empty set, proving no regression even before the feature exists).

- [ ] **Step 3: Implement** — in `src/client/shell/src/lib/worldSession.svelte.ts`:

Extend the `@shadowcat/core` import list (add after `type WireSearchHit,`):

```ts
  loadModules,
  type ModuleEntry,
```

Add the `listInstalledModules`/`getEnabledModules` import right after the `@shadowcat/core` import block:

```ts
import { listInstalledModules, getEnabledModules } from "@shadowcat/core";
```

In `#onWelcome`, change the bootstrap block:

```ts
      if (!this.#bootstrapped) {
        this.#bootstrapped = true;
        for (const m of this.opts.modules) this.#modules.add(m);
        await this.#modules.activate();
      }
```

to (threading the server version straight off the Welcome payload `w`):

```ts
      if (!this.#bootstrapped) {
        this.#bootstrapped = true;
        for (const m of this.opts.modules) this.#modules.add(m);
        await this.#modules.activate();
        await this.#loadExternalModules(w.world, w.server_version);
      }
```

Add the new private method right after `#onWelcome`'s closing brace:

```ts
  /** Fetch the world's enabled installed-module set + their (manifest,
   * entry_url) pairs and load them through the shared, per-module-contained
   * loader (M13-1 §3). Runs exactly once per WorldSession (called only inside
   * the `#bootstrapped` guard) — external modules never hot-reload across a
   * reconnect within one session (no hot unload, M13-1 §2); "next client load
   * of that world" means a fresh WorldSession (page load / re-enter), not a
   * WS reconnect. A discovery-level failure (network, malformed response)
   * degrades to a logged warning; the session still enters the world with
   * only its first-party modules active — a broken pipeline must never brick
   * a world (invariant 4). */
  async #loadExternalModules(world: string, serverVersion: string): Promise<void> {
    try {
      const [enabledIds, installed] = await Promise.all([
        getEnabledModules(world),
        listInstalledModules(),
      ]);
      const byId = new Map<string, (typeof installed)[number]>();
      for (const info of installed) {
        const id = (info.manifest as { id?: unknown }).id;
        if (typeof id === "string") byId.set(id, info);
      }
      const entries: ModuleEntry[] = [];
      for (const id of enabledIds) {
        const info = byId.get(id);
        if (!info) {
          this.#logger.warn(`enabled module ${id} is not installed; skipping`);
          continue;
        }
        entries.push({
          manifest: info.manifest as import("@shadowcat/core").ModuleManifest,
          entry: info.entry_url,
        });
      }
      if (entries.length === 0) return;
      const result = await loadModules({
        entries,
        importFn: (url) => import(/* @vite-ignore */ url),
        registry: this.#modules,
        shadowcatVersion: serverVersion,
      });
      for (const f of result.failed) {
        this.#logger.warn(`external module ${f.id} (${f.entry}) failed to load: ${f.error}`);
      }
      if (result.loaded.length > 0) await this.#modules.activate();
    } catch (e) {
      this.#logger.warn("external module discovery failed", e);
    }
  }
```

- [ ] **Step 4: Run full shell suite + typecheck to verify GREEN**

Run (from repo root): `pnpm --filter @shadowcat/shell test`
Expected: PASS, all existing + 3 new tests (28 total in this file).
Run: `pnpm -r typecheck`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/client/shell/src/lib/worldSession.svelte.ts src/client/shell/src/lib/worldSession.test.ts
git commit -m "feat(shell/worldSession): load a world's enabled external modules after Welcome"
```

---

## Task 16: Settings module — GM installed-module management UI

**Files:**
- Create: `src/modules/settings/src/ModuleManager.svelte`, `src/modules/settings/src/ModuleManager.test.ts`
- Modify: `src/modules/settings/src/Settings.svelte`, `src/client/ui-kit/src/locales/en.ts`

**Interfaces:**
- Consumes: `listInstalledModules`, `getEnabledModules`, `setEnabledModules`, `type InstalledModuleInfo` (`@shadowcat/core`); `getAppContext` (`@shadowcat/ui-kit`).

- [ ] **Step 1: Write the failing test** — create `src/modules/settings/src/ModuleManager.test.ts`:

```ts
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import ModuleManager from "./ModuleManager.svelte";

vi.mock("@shadowcat/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@shadowcat/core")>();
  return {
    ...actual,
    listInstalledModules: vi.fn().mockResolvedValue([
      { manifest: { id: "nightfox" }, entry_url: "/modules/nightfox/index.js" },
    ]),
    getEnabledModules: vi.fn().mockResolvedValue([]),
    setEnabledModules: vi.fn().mockResolvedValue(undefined),
  };
});

describe("ModuleManager", () => {
  it("lists installed modules and lets the GM toggle + save an enabled set", async () => {
    const { setEnabledModules } = await import("@shadowcat/core");
    render(ModuleManager, { context: setAppContextForTest({ world: "w1", role: "gm" }) });

    const checkbox = await screen.findByLabelText("nightfox");
    expect((checkbox as HTMLInputElement).checked).toBe(false);

    await fireEvent.click(checkbox);
    expect((checkbox as HTMLInputElement).checked).toBe(true);

    await fireEvent.click(screen.getByText("settings.modules.save"));
    await vi.waitFor(() => expect(vi.mocked(setEnabledModules)).toHaveBeenCalledWith("w1", ["nightfox"]));
  });

  it("shows an empty state when nothing is installed", async () => {
    const { listInstalledModules } = await import("@shadowcat/core");
    vi.mocked(listInstalledModules).mockResolvedValueOnce([]);
    render(ModuleManager, { context: setAppContextForTest({ world: "w1", role: "gm" }) });
    expect(await screen.findByText("settings.modules.empty")).toBeInTheDocument();
  });

  it("shows an error message when discovery fails", async () => {
    const { listInstalledModules } = await import("@shadowcat/core");
    vi.mocked(listInstalledModules).mockRejectedValueOnce(new Error("boom"));
    render(ModuleManager, { context: setAppContextForTest({ world: "w1", role: "gm" }) });
    expect(await screen.findByText("settings.modules.error")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run (from repo root): `pnpm --filter @shadowcat/module-settings test -- ModuleManager`
Expected: FAIL — `Cannot find module './ModuleManager.svelte'`.

- [ ] **Step 3: Implement the i18n keys**

In `src/client/ui-kit/src/locales/en.ts`, add after `"settings.language": "Language",`:

```ts
  "settings.modules.title": "Installed modules",
  "settings.modules.loading": "Loading modules…",
  "settings.modules.empty": "No modules installed.",
  "settings.modules.save": "Save",
  "settings.modules.error": "Module operation failed: {message}",
```

- [ ] **Step 4: Implement the component** — create `src/modules/settings/src/ModuleManager.svelte`:

```svelte
<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import {
    listInstalledModules,
    getEnabledModules,
    setEnabledModules,
    type InstalledModuleInfo,
  } from "@shadowcat/core";

  const { world, t } = getAppContext();

  let installed = $state<InstalledModuleInfo[]>([]);
  let enabled = $state<Set<string>>(new Set());
  let loaded = $state(false);
  let saving = $state(false);
  let error = $state<string | null>(null);

  function manifestId(info: InstalledModuleInfo): string {
    const id = (info.manifest as { id?: unknown }).id;
    return typeof id === "string" ? id : "(unknown)";
  }

  async function load(): Promise<void> {
    error = null;
    try {
      const [inst, en] = await Promise.all([listInstalledModules(), getEnabledModules(world)]);
      installed = inst;
      enabled = new Set(en);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loaded = true;
    }
  }
  load();

  function toggle(id: string): void {
    const next = new Set(enabled);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    enabled = next;
  }

  async function save(): Promise<void> {
    saving = true;
    error = null;
    try {
      await setEnabledModules(world, [...enabled]);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      saving = false;
    }
  }
</script>

<section class="module-manager">
  <h3>{t("settings.modules.title")}</h3>
  {#if !loaded}
    <p>{t("settings.modules.loading")}</p>
  {:else if installed.length === 0}
    <p>{t("settings.modules.empty")}</p>
  {:else}
    <ul>
      {#each installed as info (manifestId(info))}
        <li>
          <label>
            <input
              type="checkbox"
              aria-label={manifestId(info)}
              checked={enabled.has(manifestId(info))}
              onchange={() => toggle(manifestId(info))}
            />
            {manifestId(info)}
          </label>
        </li>
      {/each}
    </ul>
    <button onclick={save} disabled={saving}>{t("settings.modules.save")}</button>
  {/if}
  {#if error}
    <p class="error">{t("settings.modules.error", { message: error })}</p>
  {/if}
</section>

<style lang="scss">
  .module-manager {
    display: grid;
    gap: var(--space-2);
  }
  .error {
    color: var(--danger);
  }
</style>
```

- [ ] **Step 5: Wire it into Settings.svelte, GM-only**

Replace the full contents of `src/modules/settings/src/Settings.svelte`:

```svelte
<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import { i18n, locale } from "@shadowcat/ui-kit";
  import ModuleManager from "./ModuleManager.svelte";

  const { role, t, leaveWorld, logout } = getAppContext();
  async function doLogout() {
    await logout();
  }
</script>

<section class="panel">
  <h2>{t("settings.title")}</h2>
  <p>{t("settings.role", { role })}</p>
  <label>{t("settings.language")}
    <select value={locale()} onchange={(e) => i18n.setLocale(e.currentTarget.value)}>
      {#each i18n.locales as loc (loc)}<option value={loc}>{loc}</option>{/each}
    </select>
  </label>
  {#if role === "gm"}
    <ModuleManager />
  {/if}
  <button onclick={leaveWorld}>{t("settings.leaveWorld")}</button>
  <button onclick={doLogout}>{t("settings.logout")}</button>
</section>

<style lang="scss">
  .panel {
    padding: var(--space-4);
    display: grid;
    gap: var(--space-3);
  }
  .panel p {
    color: var(--text-muted);
    margin: 0;
  }
</style>
```

- [ ] **Step 6: Run full settings-module suite + typecheck to verify GREEN**

Run (from repo root): `pnpm --filter @shadowcat/module-settings test`
Expected: PASS, all 3 new ModuleManager tests + the existing `index.test.ts`.
Run: `pnpm -r typecheck`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/modules/settings/src/ModuleManager.svelte src/modules/settings/src/ModuleManager.test.ts src/modules/settings/src/Settings.svelte src/client/ui-kit/src/locales/en.ts
git commit -m "feat(settings): GM installed-module enable/disable UI"
```

---

## Task 17: `docs/design/module-authoring.md` — module build toolchain guide

**Files:**
- Create: `docs/design/module-authoring.md`

**Interfaces:** none (documentation only).

- [ ] **Step 1: Write the guide**

```markdown
# Authoring an External Shadowcat Module

This guide covers the build toolchain and dev workflow for a module built
OUTSIDE the Shadowcat repository (its own git repo, its own release cycle).
Nightfox (`C:\Dev\Nightfox` in this checkout's development environment) is the
reference implementation — copy its file layout for a new module.

## Manifest (`module.json`)

Same shape as the client `ModuleManifest`
(`src/client/core/src/manifest.ts`), plus one field every community module
MUST set:

```json
{
  "id": "your-module-id",
  "version": "0.1.0",
  "engines": { "shadowcat": "^0.1.0" },
  "dependencies": {},
  "capabilities": [],
  "requirements": [],
  "provides": [],
  "requires": []
}
```

- `engines.shadowcat` is a semver range (exact / `^` / `~` / `*`), checked
  against the running server's version at both enable time (GM toggles it on
  in a world) and load time (a client actually imports it). Missing this field
  = the module can never be enabled.
- `entry` (optional, default `"index.js"`) overrides the built entry file name
  relative to the module's install folder.
- `requirements` (declarative path-prefix → capability rules) are unioned into
  the world's broadcast `capability_requirements` for every world where the
  module is enabled — no separate publish step.

## Build config (Vite)

A module builds as an ES library with every engine package left external —
the host (Shadowcat's shell) supplies exactly one instance of each at runtime:

```ts
// vite.config.ts
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  plugins: [svelte()],
  build: {
    lib: { entry: "src/index.ts", formats: ["es"], fileName: () => "index.js" },
    rollupOptions: {
      external: ["svelte", /^svelte\//, "@shadowcat/core", "@shadowcat/ui-kit", "@shadowcat/formula", "@shadowcat/types"],
    },
  },
});
```

Output = `dist/index.js` (+ any chunks/assets your module itself splits) plus
your authored `module.json`, copied through unchanged. See Nightfox's
`scripts/copy-manifest.mjs` for the copy step.

## Dev flow (parity: never statically bundled, even in dev)

1. Clone a Shadowcat checkout. Clone your module's repo into
   `src/modules/<your-id>/` inside it — the pnpm workspace glob
   (`src/modules/*`) resolves `@shadowcat/*` and TS config with zero extra
   setup. Add `src/modules/<your-id>/` to the checkout's `.git/info/exclude`
   (git cannot pattern-match "a directory that is its own nested repo").
2. `pnpm --filter <your-id> dev` — a watch build whose output lands in
   `<data-dir>/modules/<your-id>/` (point it there via the
   `SHADOWCAT_MODULES_DIR` env var your `vite.config.ts` reads, matching
   Nightfox's template).
3. Run the Shadowcat dev server; log in as GM; open Settings → Installed
   modules; enable your module in a dev world; reload. Your module ALWAYS
   loads through the real modules-folder → server → import-map path, never a
   static import — matching production exactly.

## Testing

- **Unit tests** run in your module's own repo with vitest, against
  workspace-resolved `@shadowcat/*` packages (only available once nested into
  a checkout, per step 1 above — a module repo cloned standalone cannot
  `pnpm install` its `@shadowcat/*` deps).
- **e2e access**: a Node script in your repo can drive the real Shadowcat
  `test_server` binary end to end (install → discover → enable → serve),
  without a browser. See Nightfox's `e2e/run-e2e.mjs` for a complete,
  copy-pasteable template: it builds your module, stages its output as an
  installed module, spawns `test_server --modules-dir <staged-dir>`, logs in,
  and asserts the full HTTP surface (`GET /api/modules`, `PUT
  .../enabled-modules`, and the static entry serve).

## Known limits (M13-1)

- No upload/install UI — install is manual folder extraction into
  `<data-dir>/modules/<module-id>/`.
- No sandboxing — an installed module is admin-trusted client code, the same
  trust tier as the server binary itself.
- No hot enable/disable — a change takes effect on the affected client's next
  load of that world (page reload / re-enter), not instantly for an
  already-open session.
```

- [ ] **Step 2: Commit**

```bash
git add docs/design/module-authoring.md
git commit -m "docs: module authoring guide (build toolchain + dev flow + e2e access)"
```

---

## Task 18: Bootstrap the Nightfox repository

**Files (all under `C:\Dev\Nightfox`, a NEW standalone git repository — never a Shadowcat repo file):**
- Create: `C:\Dev\Nightfox\module.json`
- Create: `C:\Dev\Nightfox\package.json`
- Create: `C:\Dev\Nightfox\vite.config.ts`
- Create: `C:\Dev\Nightfox\tsconfig.json`
- Create: `C:\Dev\Nightfox\svelte.config.js`
- Create: `C:\Dev\Nightfox\vitest.config.ts`
- Create: `C:\Dev\Nightfox\vitest.setup.ts`
- Create: `C:\Dev\Nightfox\scripts\copy-manifest.mjs`
- Create: `C:\Dev\Nightfox\src\index.ts`
- Create: `C:\Dev\Nightfox\src\Hello.svelte`
- Create: `C:\Dev\Nightfox\src\index.test.ts`
- Create: `C:\Dev\Nightfox\.gitignore`
- Create: `C:\Dev\Nightfox\LICENSE`
- Create: `C:\Dev\Nightfox\README.md`

**Interfaces:** none consumed from this checkout (standalone repo). Produces the `nightfox: Module` export other tasks' e2e work depends on structurally (id `"nightfox"`, `engines.shadowcat: "^0.1.0"`, entry `dist/index.js`).

This task is a single atomic scaffold deliverable — a reviewer approves or rejects the whole bootstrap as one unit, not file-by-file (writing-plans Task Right-Sizing). No TDD cycle applies (there is no prior code to fail against); each step below both writes and structurally verifies one file group.

- [ ] **Step 1: Create the directory and initialize git**

Run: `mkdir "C:\Dev\Nightfox"` (verify `C:\Dev` exists first; if not, `mkdir "C:\Dev"` then the above).
Run: `cd "C:\Dev\Nightfox" && git init`
Expected: a fresh git repo, no commits yet. **Never push this repo** — the user owns the GitHub remote and push.

- [ ] **Step 2: `module.json`**

```json
{
  "id": "nightfox",
  "version": "0.1.0",
  "name": "Nightfox",
  "dependencies": {},
  "engines": { "shadowcat": "^0.1.0" },
  "capabilities": [],
  "requirements": [],
  "provides": [{ "contract": "nightfox.example:hello", "cardinality": "singleton" }],
  "requires": []
}
```

- [ ] **Step 3: `package.json`**

```json
{
  "name": "nightfox",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite build --watch",
    "build": "vite build && node ./scripts/copy-manifest.mjs",
    "typecheck": "svelte-check --tsconfig ./tsconfig.json",
    "test": "vitest run",
    "test:e2e": "node ./e2e/run-e2e.mjs"
  },
  "dependencies": {
    "@shadowcat/core": "workspace:*",
    "@shadowcat/ui-kit": "workspace:*",
    "@shadowcat/formula": "workspace:*",
    "@shadowcat/types": "workspace:*"
  },
  "devDependencies": {
    "@sveltejs/vite-plugin-svelte": "^7.0.0",
    "@testing-library/svelte": "^5.3.1",
    "jsdom": "^29.1.1",
    "svelte": "^5.56.0",
    "svelte-check": "^4.0.0",
    "typescript": "^5.6.0",
    "vite": "^8.0.0",
    "vitest": "^4.1.9"
  }
}
```

Note (documented, not executed by this task): `pnpm install` in this package only resolves once nested into a Shadowcat checkout's pnpm workspace at `src/modules/nightfox/` — the `workspace:*` specifiers have no meaning standalone. This is intentional (T5/T4 dev/prod parity); see the README written in Step 13.

- [ ] **Step 4: `vite.config.ts`**

```ts
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Library build: no HTML entry, ESM-only output. `svelte`, every `svelte/*`
// subpath, and every `@shadowcat/*` package stay external — the host
// (Shadowcat's shell) supplies them at runtime via its import map (M13-1
// T1/T3); bundling any of them here would duplicate the Svelte runtime
// instance and break context/reactivity across the module boundary.
const outDir = process.env.SHADOWCAT_MODULES_DIR ?? "../modules-dev/nightfox";

export default defineConfig({
  plugins: [svelte()],
  build: {
    outDir,
    emptyOutDir: false, // module.json is copied in separately; never wipe it
    lib: {
      entry: "src/index.ts",
      formats: ["es"],
      fileName: () => "index.js",
    },
    rollupOptions: {
      external: [
        "svelte",
        /^svelte\//,
        "@shadowcat/core",
        "@shadowcat/ui-kit",
        "@shadowcat/formula",
        "@shadowcat/types",
      ],
    },
  },
});
```

- [ ] **Step 5: `tsconfig.json`** (authored for its post-nesting location, `src/modules/nightfox/`, per T5 — inert until nested, documented in the README):

```json
{
  "extends": "../../../tsconfig.base.json",
  "compilerOptions": { "types": ["svelte"] },
  "include": ["src/**/*.ts", "src/**/*.svelte"]
}
```

- [ ] **Step 6: `svelte.config.js`**

```js
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

export default { preprocess: vitePreprocess() };
```

- [ ] **Step 7: `vitest.config.ts`**

```ts
import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { svelteTesting } from "@testing-library/svelte/vite";

export default defineConfig({
  plugins: [svelte(), svelteTesting()],
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./vitest.setup.ts"],
    include: ["src/**/*.test.ts"],
  },
});
```

- [ ] **Step 8: `vitest.setup.ts`**

```ts
// jsdom lacks ResizeObserver; stub it so Svelte component init completes under tests.
if (typeof globalThis.ResizeObserver === "undefined") {
  globalThis.ResizeObserver = class {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  } as unknown as typeof ResizeObserver;
}
```

- [ ] **Step 9: `scripts/copy-manifest.mjs`**

```js
import { copyFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const outDir = process.env.SHADOWCAT_MODULES_DIR ?? path.join(root, "..", "modules-dev", "nightfox");
copyFileSync(path.join(root, "module.json"), path.join(outDir, "module.json"));
console.log(`copied module.json -> ${outDir}`);
```

- [ ] **Step 10: `src/Hello.svelte`**

```svelte
<script lang="ts">
  // Trivial contributed surface: proves a module built external to the
  // Shadowcat repo compiles, loads through the modules-folder pipeline, and
  // renders using the shared (never-bundled) Svelte runtime.
</script>

<p class="nightfox-hello">Nightfox says hello.</p>

<style>
  .nightfox-hello {
    padding: 0.5rem;
  }
</style>
```

- [ ] **Step 11: `src/index.ts`**

```ts
import type { Module } from "@shadowcat/core";
import Hello from "./Hello.svelte";

/** Nightfox's module entry: identity + registration. A real game system
 * module owns the opaque `system` document band exclusively (ARCHITECTURE §2
 * invariant 6); this template ships only a trivial contributed surface to
 * prove the install → enable → load pipeline end to end. */
export const nightfox: Module = {
  manifest: {
    id: "nightfox",
    version: "0.1.0",
    dependencies: {},
    engines: { shadowcat: "^0.1.0" },
    capabilities: [],
    requirements: [],
    provides: [{ contract: "nightfox.example:hello", cardinality: "singleton" }],
    requires: [],
  },
  register(ctx) {
    ctx.contributions.contribute({
      id: "nightfox:hello",
      contract: "nightfox.example:hello",
      order: 0,
      component: Hello,
    });
    ctx.logger.debug("nightfox module registered");
  },
};

export default nightfox;
```

- [ ] **Step 12: `src/index.test.ts`**

```ts
import { describe, it, expect } from "vitest";
import { ContributionRegistry } from "@shadowcat/core";
import { nightfox } from "./index";

describe("nightfox module", () => {
  it("declares its identity and engine-compat range", () => {
    expect(nightfox.manifest.id).toBe("nightfox");
    expect(nightfox.manifest.engines?.shadowcat).toBe("^0.1.0");
  });

  it("contributes its trivial surface on register", () => {
    const contributions = new ContributionRegistry();
    nightfox.register({
      contributions,
      logger: { debug() {}, warn() {}, error() {} },
    } as never);
    const list = contributions.contributionsFor("nightfox.example:hello");
    expect(list).toHaveLength(1);
    expect(list[0].id).toBe("nightfox:hello");
  });
});
```

- [ ] **Step 13: `.gitignore`, `LICENSE`, `README.md`**

`.gitignore`:

```
node_modules/
dist/
```

`LICENSE` (MIT, matching the engine repo's own license — CLAUDE.md's permissive-only invariant):

```
MIT License

Copyright (c) 2026 Nightfox contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

`README.md`:

```markdown
# Nightfox

A game-system module for [Shadowcat](https://github.com/) — an external
module, built and released independently of the engine.

## Development flow (required — read before `pnpm install`)

This repo's `package.json` resolves `@shadowcat/core`, `@shadowcat/ui-kit`,
`@shadowcat/formula`, and `@shadowcat/types` as `workspace:*` — they only
resolve inside a Shadowcat checkout's pnpm workspace. **This repo cannot
`pnpm install` standalone.** Every build/test/dev command below runs from
inside a nested clone:

1. Clone a Shadowcat checkout somewhere.
2. Clone THIS repo a second time into `<shadowcat-checkout>/src/modules/nightfox/`
   (the pnpm workspace glob `src/modules/*` picks it up with zero extra
   config).
3. Add `src/modules/nightfox/` to the checkout's `.git/info/exclude` (git
   cannot pattern-match "a directory that is its own nested repo", so a plain
   `.gitignore` entry in the checkout would not suffice and is not used).
4. From the CHECKOUT root: `pnpm install`.
5. `pnpm --filter nightfox dev` — a watch build whose output lands in
   `<data-dir>/modules/nightfox/` (override via the `SHADOWCAT_MODULES_DIR`
   env var; see `vite.config.ts`).
6. Run the Shadowcat dev server; log in as GM; enable "nightfox" for a dev
   world in Settings → Installed modules; iterate. The module loads through
   the REAL install path every time — Nightfox is never statically imported by
   the shell, even in development (dev/prod parity, M13-1 T4).

## Testing

- `pnpm --filter nightfox test` — unit tests (vitest), workspace-resolved.
- `pnpm --filter nightfox typecheck` — svelte-check.
- `pnpm --filter nightfox test:e2e` — drives a REAL Shadowcat `test_server`
  binary over HTTP: builds this module, stages it as an installed module,
  spawns the server, logs in, and asserts install → discover → enable →
  static-serve end to end. Requires `SHADOWCAT_PATH` (path to a Shadowcat
  checkout with a built `cargo build --bin test_server`) — see
  `e2e/run-e2e.mjs`.

See `docs/design/module-authoring.md` in the Shadowcat repo for the full
toolchain guide this template implements.

## License

MIT — see `LICENSE`.
```

- [ ] **Step 13b: `.claude/CLAUDE.md`** — the Nightfox project's own agent scope (spec §5 / D16; the per-project memory directory at `~/.claude/projects/C--Dev-Nightfox/memory/` is created automatically by the first agent session, not by this bootstrap). Create `C:\Dev\Nightfox\.claude\CLAUDE.md`:

```markdown
# Nightfox — Agent Instructions

## Project
Nightfox is a generic first-party-quality game system module for the Shadowcat
virtual tabletop, developed OUT-OF-TREE as the first real consumer of Shadowcat's
community-module pipeline (Shadowcat D16). It ships per-document configurable
stats with formulas, derived stats, rolls to chat, item→actor stat modification,
and document-parent templates.

* **Core Stack:** TypeScript, Svelte 5 (Runes), Vite library build, Vitest.
* **Engine packages** (`svelte`, `@shadowcat/*`) are build-time EXTERNALS resolved
  at runtime by the host import map — never bundled (single-instance invariant).
* **Dev flow:** this repo is INERT standalone. Clone it into a Shadowcat
  checkout at `src/modules/nightfox/` (see README) so the pnpm workspace
  resolves engine packages; `pnpm --filter nightfox dev` watch-builds into the
  runtime modules folder; the module always loads through the real install
  pipeline, even in dev (parity invariant).
* **API friction with Shadowcat** is filed into the Shadowcat repo's
  `docs/POST_WORK_FINDINGS.md` as a cross-repo API bug report, not worked
  around silently.
```

Note: unlike the Shadowcat repo (where CLAUDE.md is git-ignored/local-only), this file IS committed — it is the external repo's canonical project instructions and travels with every clone.

- [ ] **Step 14: `.github/workflows/ci.yml`** — CI pipeline shape (3-OS matrix; spec §5 explicitly accepts the pipeline SHAPE now, full green not required this checkpoint):

```yaml
name: CI

on:
  push:
  pull_request:

# Nightfox resolves @shadowcat/* only inside a Shadowcat checkout's pnpm
# workspace (see README "Development flow"); this job nests the checked-out
# commit into a sibling Shadowcat checkout before installing, mirroring local
# dev exactly. `SHADOWCAT_REPO` is a repository variable (Settings ->
# Secrets and variables -> Actions -> Variables) set once the engine repo has
# a real remote — never hardcoded here, same category as a secret.
jobs:
  test:
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v6
        with:
          path: nightfox-checkout
      - uses: actions/checkout@v6
        with:
          repository: ${{ vars.SHADOWCAT_REPO }}
          path: shadowcat-checkout
      - name: Nest this checkout into the Shadowcat workspace
        shell: bash
        run: |
          rm -rf shadowcat-checkout/src/modules/nightfox
          cp -r nightfox-checkout shadowcat-checkout/src/modules/nightfox
      - uses: pnpm/action-setup@v6
        with:
          version: 9
      - uses: actions/setup-node@v6
        with:
          node-version: 22
          cache: pnpm
          cache-dependency-path: shadowcat-checkout/pnpm-lock.yaml
      - working-directory: shadowcat-checkout
        run: pnpm install --no-frozen-lockfile
      - working-directory: shadowcat-checkout
        run: pnpm --filter nightfox typecheck
      - working-directory: shadowcat-checkout
        run: pnpm --filter nightfox test
      - working-directory: shadowcat-checkout
        run: pnpm --filter nightfox build
```

- [ ] **Step 15: Verify structural completeness (no build/test run — see the plan's design decision: this repo is inert standalone by design)**

Run: `ls -la "C:\Dev\Nightfox"` and confirm every file from Steps 2–14 exists.
Run: `cd "C:\Dev\Nightfox" && git status`
Expected: all the above files listed as untracked (nothing committed yet).

- [ ] **Step 16: Commit (inside the Nightfox repo, NOT the Shadowcat repo)**

```bash
cd "C:\Dev\Nightfox"
git add module.json package.json vite.config.ts tsconfig.json svelte.config.js vitest.config.ts vitest.setup.ts scripts/copy-manifest.mjs src/index.ts src/Hello.svelte src/index.test.ts .gitignore LICENSE README.md .github/workflows/ci.yml
git commit -m "Bootstrap Nightfox: module.json, build toolchain, trivial hello module, CI stub"
```

**Never run `git push` here** — the user creates the GitHub remote and pushes.

---

## Task 19: `test_server --modules-dir` + Shadowcat-internal module e2e test

**Files:**
- Modify: `src/server/src/bin/test_server.rs`, `src/client/core/src/e2e/server-process.ts`, `src/client/core/src/e2e/README.md`
- Create: `src/client/core/src/e2e/modules.e2e.test.ts`

**Interfaces:**
- Produces: `test_server --modules-dir <path>` CLI flag; `startTestServer(opts?: { modulesDir?: string }): Promise<TestServer>` (was `startTestServer(): Promise<TestServer>`).

- [ ] **Step 1: Write the failing e2e test** — create `src/client/core/src/e2e/modules.e2e.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { mkdtempSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { startTestServer, login } from "./server-process";

describe("module toolchain e2e", () => {
  it("discovers an installed module, enables it per-world, and serves its entry through the path-traversal-guarded static route", async () => {
    const modulesDir = mkdtempSync(path.join(tmpdir(), "shadowcat-modules-"));
    const modDir = path.join(modulesDir, "fixture-mod");
    mkdirSync(modDir, { recursive: true });
    writeFileSync(
      path.join(modDir, "module.json"),
      JSON.stringify({
        id: "fixture-mod",
        version: "1.0.0",
        dependencies: {},
        engines: { shadowcat: "^0.1.0" },
      }),
    );
    writeFileSync(
      path.join(modDir, "index.js"),
      "export default { manifest: { id: 'fixture-mod', version: '1.0.0', dependencies: {} }, register() {} };\n",
    );
    // A file OUTSIDE fixture-mod/ but inside modulesDir — the traversal
    // assertion below proves the guard, not just a 404-on-missing-file.
    writeFileSync(path.join(modulesDir, "secret.txt"), "should-not-be-served");

    const server = await startTestServer({ modulesDir });
    try {
      const cookie = await login(server.baseUrl, "gm", "pw");

      const list = (await fetch(`${server.baseUrl}/api/modules`, {
        headers: { cookie },
      }).then((r) => r.json())) as Array<{ manifest: { id: string }; entry_url: string }>;
      const found = list.find((m) => m.manifest.id === "fixture-mod");
      expect(found).toBeDefined();
      expect(found!.entry_url).toBe("/modules/fixture-mod/index.js");

      const entryRes = await fetch(`${server.baseUrl}${found!.entry_url}`, {
        headers: { cookie },
      });
      expect(entryRes.status).toBe(200);
      expect(entryRes.headers.get("content-type")).toContain("text/javascript");
      expect(await entryRes.text()).toContain("fixture-mod");

      // Path traversal (percent-encoded so it is not client-side-normalized
      // away before the request is even sent) is rejected.
      const traversal = await fetch(
        `${server.baseUrl}/modules/fixture-mod/%2e%2e%2fsecret.txt`,
        { headers: { cookie } },
      );
      expect(traversal.status).toBe(404);

      const enable = await fetch(
        `${server.baseUrl}/api/worlds/${server.fixture.world}/enabled-modules`,
        {
          method: "PUT",
          headers: { "content-type": "application/json", cookie },
          body: JSON.stringify(["fixture-mod"]),
        },
      );
      expect(enable.status).toBe(204);

      const enabled = (await fetch(
        `${server.baseUrl}/api/worlds/${server.fixture.world}/enabled-modules`,
        { headers: { cookie } },
      ).then((r) => r.json())) as string[];
      expect(enabled).toEqual(["fixture-mod"]);

      const badEnable = await fetch(
        `${server.baseUrl}/api/worlds/${server.fixture.world}/enabled-modules`,
        {
          method: "PUT",
          headers: { "content-type": "application/json", cookie },
          body: JSON.stringify(["not-a-real-module"]),
        },
      );
      expect(badEnable.status).toBe(422);
    } finally {
      server.stop();
    }
  }, 30_000);
});
```

- [ ] **Step 2: Run to verify it fails**

Run (from repo root): `pnpm --filter @shadowcat/core test:e2e -- modules`
Expected: FAIL — `startTestServer` does not accept an `opts` argument (TS error) / the server never receives `--modules-dir` so the module isn't discovered (404s / empty list).

- [ ] **Step 3: Add the CLI flag to `test_server.rs`**

In `src/server/src/bin/test_server.rs`, add the import and args struct (right after the existing `use` block, before `#[tokio::main]`):

```rust
use clap::Parser;

/// `--modules-dir <path>`: overrides the modules folder the embedded router
/// scans/serves from (default: none installed). Lets the Node<->Rust e2e
/// harness — and an external module repo's own smoke script (see
/// `docs/design/module-authoring.md`) — point a fresh `test_server` at a
/// fixture-populated temp folder without touching the hardcoded in-memory
/// fixture data below.
#[derive(Parser, Debug, Default)]
struct Args {
    #[arg(long)]
    modules_dir: Option<String>,
}
```

Change the `AppState` construction at the end of `main()` — replace:

```rust
    let state = AppState {
        repo,
        config: Arc::new(Config::default()),
        setup_token: None,
        initialized: Arc::new(AtomicBool::new(true)),
        ws: shadowcat::ws::WsState::new(),
        upload_rate: Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
    };
```

with:

```rust
    let args = Args::parse();
    let mut config = Config::default();
    if let Some(dir) = args.modules_dir {
        config.modules_dir = Some(dir);
    }
    let state = AppState {
        repo,
        config: Arc::new(config),
        setup_token: None,
        initialized: Arc::new(AtomicBool::new(true)),
        ws: shadowcat::ws::WsState::new(),
        upload_rate: Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
    };
```

- [ ] **Step 4: Extend `startTestServer` to pass `--modules-dir`**

In `src/client/core/src/e2e/server-process.ts`, change the `startTestServer` signature and spawn call:

```ts
export async function startTestServer(opts: { modulesDir?: string } = {}): Promise<TestServer> {
  const isWindows = process.platform === "win32";
  const exe = path.join(repoRoot, "target", "debug", isWindows ? "test_server.exe" : "test_server");

  // Build first (fast if already built; the CI job pre-builds). `shell` lets
  // Windows resolve `cargo` via PATHEXT. Building separately means the long-lived
  // process is the binary itself, not a cargo wrapper with a grandchild.
  const build = spawnSync("cargo", ["build", "-p", "shadowcat", "--bin", "test_server"], {
    cwd: repoRoot,
    stdio: "inherit",
    shell: isWindows,
  });
  if (build.status !== 0) throw new Error(`cargo build test_server failed (${build.status})`);

  const args = opts.modulesDir ? ["--modules-dir", opts.modulesDir] : [];
  const proc: ChildProcess = spawn(exe, args, {
    cwd: repoRoot,
    stdio: ["ignore", "pipe", "inherit"],
  });
```

(The rest of the function is unchanged.)

- [ ] **Step 5: Update the e2e README**

In `src/client/core/src/e2e/README.md`, extend the "What the harness provides" bullet for `startTestServer`:

```markdown
- `startTestServer(opts?)` — spawns the server, parses its `test_server:` address and
  `e2e-fixture:` JSON (world/doc/gm/player ids), returns `{ baseUrl, wsUrl,
  fixture, stop }`. `opts.modulesDir` (optional) passes `--modules-dir <path>`
  to the spawned binary, for tests exercising the installed-module pipeline
  (see `modules.e2e.test.ts`).
```

- [ ] **Step 6: Run the full server suite + clippy, then the e2e test**

Run (from `src/server/`): `cargo build -p shadowcat --bin test_server`
Expected: builds cleanly.
Run: `cargo test --all-targets`
Expected: PASS (no other server test touches `test_server.rs`, which is a `[[bin]]` target excluded from `cargo test`'s unit-test discovery beyond compiling).
Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.
Run (from repo root): `pnpm --filter @shadowcat/core test:e2e -- modules`
Expected: PASS.
Run: `pnpm --filter @shadowcat/core test:e2e` (full e2e suite)
Expected: PASS — no regression to the existing e2e tests (`capabilities`, `ingress-rejection`, `live-search`, `name-privacy`, `search`), since `startTestServer()` with no args behaves identically to before (`opts.modulesDir` is `undefined` → no `--modules-dir` flag → `Config::default()`'s `modules_dir: None` → empty modules dir, same as today).

- [ ] **Step 7: Commit**

```bash
git add src/server/src/bin/test_server.rs src/client/core/src/e2e/server-process.ts src/client/core/src/e2e/README.md src/client/core/src/e2e/modules.e2e.test.ts
git commit -m "feat(e2e): test_server --modules-dir + module toolchain e2e (discover/enable/serve/traversal)"
```

---

## Task 20: Nightfox's own e2e smoke script

**Files (all under `C:\Dev\Nightfox`):**
- Create: `C:\Dev\Nightfox\e2e\run-e2e.mjs`

**Interfaces:**
- Consumes: `test_server --modules-dir` (Task 19); Nightfox's own `pnpm build` output (Task 18's `vite.config.ts` + `copy-manifest.mjs`).

This is the spec §4 "e2e access" deliverable proper: a script an external module repo runs against the checkout, reimplemented standalone (it cannot import Shadowcat's internal `server-process.ts` across a repo boundary) but mirroring its exact spawn/parse contract for consistency.

- [ ] **Step 1: Write `e2e/run-e2e.mjs`**

```js
// Node<->Rust smoke e2e run from OUTSIDE the Shadowcat repo: builds this
// module, stages its output as an installed module, spawns the checkout's
// test_server pointed at that staging dir, and asserts the installed-module
// pipeline end to end over plain HTTP (no browser — mirrors the scope of
// Shadowcat's own src/client/core/src/e2e/*.e2e.test.ts suite).
//
// Requires SHADOWCAT_PATH (a Shadowcat checkout with `cargo build --bin
// test_server` runnable, i.e. this Nightfox checkout must ALSO be nested at
// <SHADOWCAT_PATH>/src/modules/nightfox/ per the README's dev flow — `pnpm
// --filter nightfox build` must have already run so dist/ exists).
import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, cpSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const shadowcatPath = process.env.SHADOWCAT_PATH;
if (!shadowcatPath) {
  console.error("SHADOWCAT_PATH is required: path to a Shadowcat checkout with test_server buildable.");
  process.exit(1);
}
const distDir = path.join(root, "dist");
if (!existsSync(path.join(distDir, "index.js")) || !existsSync(path.join(distDir, "module.json"))) {
  console.error(`Nightfox is not built. Run \`pnpm --filter nightfox build\` first (looked in ${distDir}).`);
  process.exit(1);
}

const modulesDir = mkdtempSync(path.join(tmpdir(), "nightfox-e2e-modules-"));
const stagedModuleDir = path.join(modulesDir, "nightfox");
cpSync(distDir, stagedModuleDir, { recursive: true });

const isWindows = process.platform === "win32";
const exe = path.join(shadowcatPath, "target", "debug", isWindows ? "test_server.exe" : "test_server");
const build = spawnSync("cargo", ["build", "-p", "shadowcat", "--bin", "test_server"], {
  cwd: shadowcatPath,
  stdio: "inherit",
  shell: isWindows,
});
if (build.status !== 0) {
  console.error(`cargo build test_server failed (${build.status})`);
  process.exit(1);
}

const proc = spawn(exe, ["--modules-dir", modulesDir], { cwd: shadowcatPath, stdio: ["ignore", "pipe", "inherit"] });

let baseUrl = "";
let fixture = null;
await new Promise((resolve, reject) => {
  const timer = setTimeout(() => reject(new Error("test_server did not start within 20s")), 20_000);
  let buf = "";
  proc.stdout.on("data", (chunk) => {
    buf += chunk.toString();
    const addr = /test_server: (http:\/\/[\d.:]+)/.exec(buf);
    if (addr) baseUrl = addr[1];
    const fx = /e2e-fixture: (\{.*\})/.exec(buf);
    if (fx) fixture = JSON.parse(fx[1]);
    if (baseUrl && fixture) {
      clearTimeout(timer);
      resolve();
    }
  });
  proc.on("error", reject);
  proc.on("exit", (code) => reject(new Error(`test_server exited early (code ${code})`)));
});

function stop() {
  if (proc.pid === undefined) return;
  if (isWindows) spawnSync("taskkill", ["/pid", String(proc.pid), "/T", "/F"], { stdio: "ignore" });
  else proc.kill("SIGKILL");
}

function assert(cond, message) {
  if (!cond) {
    stop();
    console.error(`FAIL: ${message}`);
    process.exit(1);
  }
}

try {
  const loginRes = await fetch(`${baseUrl}/api/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ username: "gm", password: "pw" }),
  });
  assert(loginRes.ok, `login failed: ${loginRes.status}`);
  const cookie = loginRes.headers.getSetCookie()[0].split(";")[0];

  const list = await fetch(`${baseUrl}/api/modules`, { headers: { cookie } }).then((r) => r.json());
  const found = list.find((m) => m.manifest.id === "nightfox");
  assert(found, "nightfox not found in GET /api/modules");
  assert(found.entry_url === "/modules/nightfox/index.js", `unexpected entry_url: ${found.entry_url}`);

  const entryRes = await fetch(`${baseUrl}${found.entry_url}`, { headers: { cookie } });
  assert(entryRes.ok, `entry fetch failed: ${entryRes.status}`);
  assert(
    (entryRes.headers.get("content-type") ?? "").includes("text/javascript"),
    `unexpected content-type: ${entryRes.headers.get("content-type")}`,
  );

  const enableRes = await fetch(`${baseUrl}/api/worlds/${fixture.world}/enabled-modules`, {
    method: "PUT",
    headers: { "content-type": "application/json", cookie },
    body: JSON.stringify(["nightfox"]),
  });
  assert(enableRes.status === 204, `enable failed: ${enableRes.status}`);

  const enabled = await fetch(`${baseUrl}/api/worlds/${fixture.world}/enabled-modules`, {
    headers: { cookie },
  }).then((r) => r.json());
  assert(JSON.stringify(enabled) === JSON.stringify(["nightfox"]), `enabled set mismatch: ${JSON.stringify(enabled)}`);

  console.log("PASS: nightfox smoke e2e (discover, serve, enable) — install -> enable -> load pipeline verified over HTTP.");
} finally {
  stop();
}
```

- [ ] **Step 2: Structural verification (no execution — the plan-writer cannot build Nightfox standalone; see the plan's design decision)**

Run: `ls "C:\Dev\Nightfox\e2e\run-e2e.mjs"`
Expected: file exists.

A future execution session (once nested per the README and built) runs it via:

```bash
SHADOWCAT_PATH="C:\Dev\Shadowcat" pnpm --filter nightfox test:e2e
```
Expected: `PASS: nightfox smoke e2e ...` printed, exit code 0.

- [ ] **Step 3: Commit (inside the Nightfox repo)**

```bash
cd "C:\Dev\Nightfox"
git add e2e/run-e2e.mjs
git commit -m "feat(e2e): standalone smoke script (discover/serve/enable) against a Shadowcat checkout's test_server"
```

**Never run `git push` here.**

---

## Task 21: Documentation sync + skill updates

**Files:**
- Modify: `docs/PLAN.md`, `docs/TODO.md`
- Modify (reviewed skill-update gate): the `client-shell` and `realtime-sync`/`core` `shadowcat-codebase-*` skills whose seams this checkpoint changes
- Create (if the plan-writer judges a new subsystem skill is warranted — see Step 3): `shadowcat-codebase-module-toolchain` skill

- [ ] **Step 1: Update `docs/PLAN.md`**

Add an entry marking M13-1 complete, following the file's existing milestone-entry format (read the file first to match its exact style before editing — do not guess the format).

- [ ] **Step 2: Log out-of-scope items to `docs/TODO.md`**

Add these four entries (deferred by the spec's "Out of scope" section, verbatim source):

```markdown
- Module upload/install UI (M13-1 T2) — install stays manual-extract into `<data-dir>/modules/<id>/`.
- Sandboxing/permissions for installed module JS (M13-1 T2) — modules are admin-trusted, same tier as the server binary.
- Hot enable/disable of installed modules without a client reload (M13-1 §2).
- Module marketplace/registry, signing, or update channels (M13-1 §2).
```

- [ ] **Step 3: Reviewed skill-update gate**

Read `.claude/skills/shadowcat-codebase-client-shell/SKILL.md` and the realtime/core-facing skill covering `worldSession.svelte.ts`/`@shadowcat/core`'s loader+registry (likely `shadowcat-codebase-realtime-sync` or `shadowcat-codebase-core` — check `.claude/skills/` for the exact name/glob coverage before editing). Update whichever skill(s) own:
- `worldSession.svelte.ts`'s new external-module-loading seam (fetch enabled set → `loadModules` → activate, bootstrap-once).
- `loader.ts`'s new per-module-contained, non-throwing `loadModules` contract (a signature change from `Promise<void>` to `Promise<ModuleLoadResult>` — any skill documenting the loader's old throw-on-first-failure behavior is now stale and must be corrected, not just appended to).
- The new server-side `modules` domain (discovery, path-traversal-guarded static serving, per-world enablement, the `Welcome`-time capability-requirements union) — if no existing skill's glob covers `src/server/src/modules.rs` / `src/server/src/http/module_routes.rs`, create `shadowcat-codebase-module-toolchain` (fixed shape per the project's skill template) and add its globs to the scoped `Edit|Write` activation hook.

Then dispatch `shadowcat-spec-reviewer` on the skill diff(s) to confirm each accurately captures the change (no omission, drift, or broken pointer) — this review is itself part of completing this task, per the project's reviewed skill-update gate. If a skill needs no edit (a touched area has no skill covering it and does not warrant a new one), state that explicitly rather than silently skipping.

- [ ] **Step 4: Commit**

```bash
git add docs/PLAN.md docs/TODO.md .claude/skills
git commit -m "docs(m13-1): mark checkpoint complete, log deferred items, sync codebase skills"
```

---

## Buddy-check directives

**Flagged tasks (buddy-check replaces both review stages):**
- **Task 5** — `GET /modules/{id}/{*path}` static serving + path-traversal guard. Security boundary: the two-stage canonicalize-and-verify design (guarding BOTH the `id` segment and the `rel_path` segment independently) needs adversarial verification against percent-encoding, symlink escapes, and Windows-specific path quirks (UNC `\\?\` prefixes from `canonicalize`, case-insensitive filesystem semantics) beyond what the plan's own tests exercise.
- **Task 8 + Task 10 together** — the per-world enable + capability-requirements publish mechanism (`PUT /api/worlds/{id}/enabled-modules`'s atomic T6 validation, and the non-destructive `Welcome`-time union with GM-authored `world_cap_requirements`). Security-adjacent: an error here either under-grants (a legitimately-enabled module's declared requirements silently missing from the broadcast) or over-grants (a disabled/uninstalled module's requirements leaking into the union). Review both tasks as one unit — Task 10 is the second half of the mechanism Task 8's validation gate protects.

- **Task 14** (shared-runtime ESM chunks + import map) — FLAGGED (session decision 2026-07-16, user asleep; upgraded because invariant 1 — exactly one Svelte/`@shadowcat/*` instance at runtime — is load-bearing and the plan's `importMap.test.ts` only verifies build-output STRUCTURE, not runtime dedup; a stale/incomplete `svelte/*` subpath enumeration would silently degrade to duplicated Svelte instances rather than a build failure).

**Risk-signal tasks left at the standard two-reviewer gate (session decision, user asleep — revisit if desired):**
- **Task 15** (`worldSession.svelte.ts` external-module loading) — touches the `#onWelcome` hot path shared by all 28 existing tests in `worldSession.test.ts`; existing coverage is strong and the whole-branch buddy-check backstops it.
- **Task 18/20** (Nightfox bootstrap + its e2e script) — genuinely unexecutable/unverifiable from within this checkout (standalone repo, `workspace:*` deps that cannot resolve until nested); the real verification is the first nested run, which happens before the checkpoint closes.

**Customary whole-branch buddy-check before the checkpoint merge** applies regardless of the above, per project convention.

---

## Self-Review

**1. Spec coverage:**
- §1 (runtime modules folder + discovery, `GET /api/modules`, static serving, path-traversal guard, server never reads/executes module JS) → Tasks 1, 2, 4, 5.
- §2 (per-world enablement, settings-UI extension, requirements publish via capability machinery, engine-compat at enable AND load) → Tasks 7, 8, 10, 16.
- §3 (import map + shared ESM chunks, `worldSession` load-after-Welcome via `loadModules`, per-module load containment, first-party modules unchanged) → Tasks 11, 12, 13, 14, 15.
- §4 (module template deliverable, dev flow parity, unit tests, e2e access script + Nightfox smoke e2e) → Tasks 17, 18 (template = Nightfox's own files), 19, 20.
- §5 (Nightfox bootstrap: module.json, build config, tsconfig, vitest, CI stub 3-OS, README dev flow, own `.claude` scope) → Task 18 (`.claude` scope = Step 13b).
- Invariants 1–6 → covered across Tasks 5 (2, 6), 8/10 (5), 14 (1, 6), 15 (4, 3).
- T1–T6 decisions table → T1/T3 (Task 14), T2 (Tasks 5, 8, 16 — no upload UI, logged to TODO in Task 21), T4 (Task 15's bootstrap-once + Task 18's README), T5 (Task 18's file authoring for the nested location), T6 (Tasks 3, 8, 11, 12).
- Out-of-scope list → Task 21 Step 2.

**Gap found by the plan-writer and resolved at plan review (2026-07-16):** spec §5's "own `.claude` scope" sub-bullet had no covering step; Task 18 Step 13b now creates `C:\Dev\Nightfox\.claude\CLAUDE.md` (committed, unlike Shadowcat's local-only CLAUDE.md); the per-project memory directory is agent-runtime-created, not a bootstrap artifact.

**2. Placeholder scan:** searched every task for "TBD"/"fill in"/"similar to Task N"/bare prose-only steps. The one borderline case — `${{ vars.SHADOWCAT_REPO }}` in Task 18's CI stub — is a legitimate externally-supplied CI variable (the actual GitHub org/repo is not yet known; Shadowcat itself has no pushed remote in this session's git status), documented inline as such, not a code "TBD". No other instances found.

**3. Type consistency:** verified across tasks — `InstalledModule`/`InstalledModuleInfo` (Task 2 → 4 → 8 → 10 → 13 → 15 → 19) share field names throughout (`id`, `requirements`, `engines_shadowcat`, `manifest_json`/`manifest`, `entry_url`). `ModuleLoadResult`/`ModuleLoadFailure` (Task 12) field names (`loaded`, `failed`, `id`, `entry`, `error`) match their Task 15 consumer exactly. `ModuleEngines`/`engines.shadowcat` naming is consistent across Tasks 2, 3, 11, 12, 18. `egress_loop`'s new `modules_dir` parameter (Task 10) is threaded through both its call sites in the same task. `Config.modules_dir`/`modules_path()` (Task 1) is consumed identically in Tasks 4, 5, 6, 8, 10, 19.

---

Plan complete and saved to `docs/superpowers/plans/2026-07-16-m13-1-external-module-toolchain.md`. Two execution options:

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints

**Which approach?**
