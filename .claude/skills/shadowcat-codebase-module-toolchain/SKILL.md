---
name: shadowcat-codebase-module-toolchain
description: "Use when touching the external/community module toolchain: server-side installed-module discovery + path-traversal-guarded static serving + per-world enablement (src/server/src/modules.rs, http/module_routes.rs, config.rs modules_dir), the engine-compat semver gate, the Welcome server_version + capability-requirements union, or the client consumption path (core loader.ts/modules.ts/module-rest.ts/manifest.ts engines, shell import-map single-instance build + worldSession external-module loading, settings ModuleManager UI). Covers out-of-tree modules (the Nightfox reference repo), the authoring guide docs/site/guides/creating-a-module.md (docs/design/module-authoring.md is a pointer stub), and the examples/* scaffold packages. Invoke shadowcat-codebase-core first; for the shell/AppContext seams invoke shadowcat-codebase-client-shell."
---

# Shadowcat — External Module Toolchain

Orientation for how a module built OUTSIDE the engine repo (own git repo, own release cycle) is
installed, discovered, served, enabled, and loaded — the M13-1 pipeline. Orientation+index only:
points INTO graphify, `docs/design/`, and memory.

## Purpose

An installed module lives at `<data-dir>/modules/<folder-id>/` as a `module.json` manifest plus a
pre-built ESM bundle. The **server discovers and serves it as static files but NEVER executes any
module code** (ARCHITECTURE §2 invariant 6). A GM enables a module per-world; the client shell
supplies exactly one runtime instance of `svelte`/`@shadowcat/*` via an import map, fetches the
enabled set after `Welcome`, dynamically imports each enabled module through the real
modules-folder → server → import-map path (identical in dev and prod), and activates it through the
existing M6b `ModuleRegistry`.

## Key files & seams

**Server (authoritative, never runs module code):**
- `src/server/src/modules.rs` — `scan_installed_modules(dir)` walks `<dir>/*/module.json`, parse +
  validate each (invalid `id`/`version`/JSON → warn + SKIP, never blocks startup or hides siblings).
  `InstalledModule { id, requirements, engines_shadowcat, manifest_json, entry_url }` where **`id`
  is the install FOLDER name, not the author-declared manifest id**. `entry` (module.json field,
  `default_entry` = `index.js`) computes `entry_url` = `/modules/<folder>/<entry>`.
  `semver_satisfies` (exact/`^`/`~`/`*`, caret-0.x leftmost-non-zero fix) + `engine_compat_ok`
  (**fail-closed**: missing `engines.shadowcat` → reject).
- `src/server/src/http/module_routes.rs` — `InstalledModuleInfo { id (folder id), manifest,
  entry_url }` (ts-rs → `src/types/generated/`); `list_installed_modules` (`GET /api/modules`,
  any-auth); `serve_module_file` (`GET /modules/{id}/{*path}` — two-stage canonicalize +
  `is_strictly_within` proper-descendant check, guards BOTH the `id` segment and the `*path`
  segment, rejects path==root equality); `set_world_enabled_modules`/get (`PUT/GET
  /api/worlds/{id}/enabled-modules`, `require_gm`, atomic validate-all + dedup, `MAX_ENABLED_MODULES`).
- `src/server/src/config.rs` — `Config.modules_dir: Option<String>` + `modules_path()`; the
  `test_server --modules-dir` flag (`bin/test_server.rs`) sets it for e2e.
- `src/server/src/ws/{conn.rs,protocol.rs}` — `ServerMsg::Welcome.server_version`
  (`env!("CARGO_PKG_VERSION")`); `welcome_capability_requirements` non-destructively UNIONs the GM's
  `world_cap_requirements` with each `engine_compat_ok` enabled module's `requirements`.

**Client core (framework-neutral):**
- `src/client/core/src/loader.ts` — `loadModules(...) → Promise<ModuleLoadResult { loaded, failed }>`
  — **per-module contained, NON-throwing** (a single module's import/compat failure no longer aborts
  the batch); `checkEngineCompat`; fail-closed when `opts.shadowcatVersion` is absent.
- `src/client/core/src/modules.ts` — `ModuleRegistry.activate()` is per-module isolated: a throwing
  `register()` or a singleton-contract collision is logged + skipped, the topo loop continues, the
  first provider stays sole-active. **Rollback-on-throw:** a `register()` throw mid-registration is
  caught, then `activate()` calls the module's own `unload(id)` to roll back any partial side
  effects (hooks/services/middleware/contributions) it already registered before throwing — safe by
  construction since `r.active` is still `false` at the catch point (`activeDependentsOf(id)` is
  therefore always empty, topoSort guarantees no dependent activates before its dependency). The
  `unload(id)` call is itself wrapped in its own try/catch (logged, not propagated) so a SECOND
  throw during rollback can't abort the whole activation loop — modules ordered after the failing
  one still activate.
- `src/client/core/src/module-rest.ts` — `listInstalledModules` / `getEnabledModules` /
  `setEnabledModules` REST wrappers (consume `InstalledModuleInfo` via unchecked cast — no Zod).
- `src/client/core/src/manifest.ts` — `engines?: ModuleEngines` (optional; first-party modules never
  set it, community modules MUST); `requirements` are advisory. `src/client/core/src/semver.ts` —
  caret-0.x fix mirror of the server.

**Client shell:**
- `src/client/shell/vite.config.ts` — `RUNTIME_ENTRIES` multi-entry (svelte, svelte/internal/client,
  svelte/internal/disclose-version, svelte/reactivity, @shadowcat/{core,ui-kit,formula,types}) →
  stable `runtime/<name>.js` chunks + **`preserveEntrySignatures: "strict"`**; `index.html` import
  map maps each bare specifier to its chunk. `RUNTIME_ENTRIES` is exported (not duplicated) for the
  CI guard below.
- `scripts/check-svelte-runtime-entries.mjs` — a build-time CI guard scanning all client/module
  source for `svelte/*` bare-specifier imports, failing if any resolve to a subpath NOT present in
  `vite.config.ts`'s (exported) `RUNTIME_ENTRIES` — catches the "import map serves a FIXED
  svelte-subpath set" gotcha below at build time instead of a runtime `SyntaxError`. Wired into
  `.github/workflows/ci.yml`'s web job + `package.json`'s `check:svelte-runtime` script. Its
  CLI-entry-point detection uses `pathToFileURL(...).href` (not a raw `file://${argv[1]}` string
  compare, which never matches on Windows — wrong scheme/separator/drive-letter handling).
- `src/client/shell/src/lib/worldSession.svelte.ts` — `#loadExternalModules(world, serverVersion)`
  sourced from `w.server_version`; fetch enabled set → `loadModules` → activate; keyed on `info.id`.
- `src/modules/settings/src/ModuleManager.svelte` — GM installed-module management UI; toggle/save
  keyed on the canonical folder `info.id` (manifest id is display-only).

**Out-of-tree reference + guide:** the Nightfox repo (its own git repo, nested into a checkout at
`src/modules/nightfox/` for dev, never bundled statically even in dev). The authoring guide lives
in the docs site: `docs/site/guides/creating-a-module.md` (`docs/design/module-authoring.md` is a
pointer stub to it). Two in-repo CI-built worked examples double as copyable scaffolds:
`examples/module-initiative-tracker/` (panel + document read/write) and `examples/system-minimal/`
(sheet takeover + formula rules) — workspace members, so `pnpm -r test/typecheck` and the web CI
job's example-build step keep them green; the guides code-import their sources region-by-region.

## Hard invariants

- **The server NEVER executes installed module code** — it only discovers + serves it as static
  bytes (ARCHITECTURE §2 invariant 6). Authority over the `system` band stays structural.
- **Exactly one runtime instance** of `svelte`/`svelte/*`/`@shadowcat/*` (Global Constraint 1) —
  requires `preserveEntrySignatures: "strict"` so runtime chunks export real API names, verified by
  a test that IMPORTS each chunk (not just checks existence) [[build-artifact-tests-must-consume-not-just-exist]].
- **The enabled-module set is keyed on the install FOLDER id, never the manifest id** — the server
  controls folder names; author-declared manifest ids can collide and are untrusted as the key. Both
  client consumers (ModuleManager, worldSession) MUST key on the wire `InstalledModuleInfo.id`.
- **Engine-compat is fail-closed** (missing/unsatisfied `engines.shadowcat` → reject) at BOTH enable
  time and load time.
- **Module `requirements` are advisory to the client only** — unioned into the world's broadcast
  `capability_requirements`, but NEVER server-enforced at `apply_intent` (server authority stays with
  the GM's `world_cap_requirements`). A future explicit "GM adopts a module's requirements into the
  world policy" mechanism could make them enforced; until then advisory-only is the contract.
- **Path-traversal guard rejects equality, not just prefix** — a two-stage canonicalize must treat
  the modules root as a strict ancestor of both the `id` folder and the served file.

## Gotchas

- **`entry` is a `module.json` field read by the server scanner** (`modules.rs`, default
  `index.js`), NOT part of the client `ModuleManifest` Zod shape — declare it in module.json only.
- **The import map serves a FIXED svelte-subpath set** — a module importing a subpath the host does
  not serve (`svelte/store`, `svelte/transition`, …) hard-fails with a runtime `SyntaxError`; adding
  one is a host change (`RUNTIME_ENTRIES` + import map), not a module change. See the
  creating-a-module guide (`docs/site/guides/creating-a-module.md`).
  `scripts/check-svelte-runtime-entries.mjs` (above) catches an unserved subpath import at CI time.
- **`loadModules`'s contract CHANGED** from `Promise<void>` throw-on-first-failure to the contained
  `ModuleLoadResult`; any doc describing the old throw behavior is stale.
- **Adding a required field to `Welcome`** (e.g. `server_version`) breaks untyped frame fixtures in
  every package — gate with `pnpm -r test`, not a single filter [[shared-wire-schema-change-needs-full-repo-test]].
- **`InstalledModuleInfo` is ts-rs generated** — edit the Rust struct, regenerate, never hand-edit
  the `.ts`.
- **HTTP path-traversal tests via `axum_test`/`fetch` are vacuous for bare dot-segments.**
  `axum_test::TestServer` builds URLs through the `url` crate's `Url::set_path`, which applies WHATWG
  dot-segment normalization CLIENT-SIDE before the request is sent — a segment that EXACTLY matches
  `.`/`..`/`%2e`/`%2e%2e` (and case variants) is collapsed/popped before it can reach the router or
  `serve_module_file`'s guard. A dot-segment test therefore proves nothing: confirmed —
  `serve_module_file_rejects_an_id_segment_that_escapes_the_modules_root` (`http/module_routes.rs`)
  still PASSES against a deliberately-reverted, vulnerable guard. A NON-exact-match segment (e.g.
  `%2e%2e%2fsecret.txt` as one combined segment) is NOT normalized and DOES reach the handler intact.
  Write such tests as (a) a pure unit test of the containment predicate, (b) a symlink/alias HTTP
  repro (`module_routes.rs`'s `self-link`-style test), or (c) an encoded segment embedded in a longer
  non-exact-match string.
- **Scope deliberately excluded from M13-1** (manual/admin-trusted tier): no module upload/install UI
  (install stays manual-extract into `<data-dir>/modules/<id>/`); no sandboxing/permissions for
  installed module JS (modules are admin-trusted, same tier as the server binary); no hot
  enable/disable without a client reload; no marketplace/registry, signing, or update channels.

## Pointers

- Rationale: `docs/design/ARCHITECTURE.md` §2 invariant 6 (server runs no third-party code) +
  Global Constraint 1 (single instance); `docs/site/guides/creating-a-module.md` (authoring
  toolchain — `docs/design/module-authoring.md` is a pointer stub); `docs/PLAN.md` M13 (M13-1
  DONE entry).
- Relationships: `graphify query "installed module discovery serve enable engine-compat import map loader"`.
- Lessons: [[build-artifact-tests-must-consume-not-just-exist]],
  [[shared-wire-schema-change-needs-full-repo-test]], [[injected-callback-boundary-must-validate-every-site]].
