# M13-1 · External-Module Toolchain — Design

**Status:** Approved by user 2026-07-16.
**Purpose (D16):** make Shadowcat consumable by out-of-tree module projects through the real
community pipeline, with Nightfox as the first external consumer. This checkpoint delivers the
distribution/install/load mechanism and bootstraps the Nightfox repository; M13b/M13c/M13d then
re-target the Nightfox repo.

## Decisions

| # | Decision |
|---|---|
| T1 | **No engine libraries in module builds.** A module artifact never bundles `svelte` or any `@shadowcat/*` package — they are build-time externals resolved at runtime by the host (user decision 2026-07-16). Redundancy aside, sharing is a correctness invariant: two Svelte 5 runtime instances break context/reactivity across the module boundary; two `@shadowcat/core` instances break `instanceof`/registry identity. |
| T2 | **Install = extract into the runtime modules folder; enable from the UI** (user decision 2026-07-16). No upload UI in M13-1 (deferred to TODO.md). No sandboxing in M13-1: installed modules are admin-trusted client code, the same trust tier as the server binary. |
| T3 | **Runtime linkage = browser import map + host-served ESM chunks** (Approach A, user-approved). The shell build emits stable ESM entry chunks for the shared runtimes; served HTML carries an import map pointing bare specifiers at those chunks; module entries are loaded with dynamic `import(url)` through the existing M6b loader. |
| T4 | **Dev/prod parity** (user-approved). Nightfox (and any external module) is ALWAYS loaded through the modules-folder → server → import-map path, even during development (watch build). External modules are never statically imported by the shell. First-party modules stay statically bundled, unchanged. |
| T5 | **Nightfox keeps its own GitHub repository (D16 stands), cloned INTO a local Shadowcat checkout for development** (user decision 2026-07-16): `src/modules/nightfox/` inside the checkout, where the pnpm workspace glob resolves engine packages with zero config. |
| T6 | **`module.json` gains a minimal engine-compat field now**: `engines: { shadowcat: "<semver range>" }`, checked at load with a clear error (user-approved with design). Retrofitting a compat gate after third-party modules exist is much harder; this obligates version discipline on every Shadowcat release from here on. |

## 1. Runtime modules folder + server discovery

- Installed modules live at `<data-dir>/modules/<module-id>/`, sibling to the database. All
  paths built with `std::path` (cross-platform invariant).
- Each installed module contains:
  - `module.json` — the manifest. Same shape as the client `ModuleManifest`
    (`src/client/core/src/manifest.ts`), plus the new `engines` field (T6). Validated
    server-side by a serde mirror struct (`deny_unknown_fields` NOT used here — the manifest is
    community-authored; unknown keys are ignored for forward compatibility, mirroring the Zod
    schema's behavior).
  - `index.js` — pre-built ESM entry (path may be overridden by an optional `entry` field in
    `module.json`, default `index.js`).
  - optional static assets referenced relatively by the module.
- Server behavior (new, in `src/server`):
  - On startup, scan `<data-dir>/modules/*/module.json`; parse + validate each. Invalid
    manifests are logged (warn) and skipped — one broken module must not prevent startup or
    hide the others.
  - `GET /api/modules` → JSON list of `{ manifest, entry_url }` for every validly installed
    module. Auth: any authenticated user (clients need it to load enabled modules).
  - `GET /modules/<id>/<path...>` → static file serving from that module's folder ONLY.
    **Path-traversal guard is mandatory**: canonicalize and verify the resolved path is inside
    `<data-dir>/modules/<id>/`; reject `..`, absolute paths, and symlink escapes. Correct
    `Content-Type` for `.js` (`text/javascript`) is load-bearing for ESM imports.
  - The server never reads, executes, or introspects module JS (ARCHITECTURE §2 invariant 6 —
    structural authority only).

## 2. Per-world enablement

- The GM enables/disables installed modules per world from the settings UI (extends the
  existing settings module).
- The enabled set persists server-side per world (storage: whatever the existing per-world
  config mechanism is — a world config document or a DB column; plan-writer picks the one that
  matches the M6b `capability_requirements` storage, which this sits beside).
- Enabling a module publishes its manifest `requirements` (path-prefix → caps) through the
  existing M6b capability machinery, exactly as first-party module requirements are published.
- On world join, the client receives (or fetches) the world's enabled module list; only enabled
  modules are loaded. Disabling takes effect on next client load of that world (no hot unload —
  out of scope).
- The engine-compat check (T6): at enable time AND at load time, `engines.shadowcat` is checked
  against the running server version; mismatch → clear error (enable rejected / module skipped
  with a visible warning), never a silent failure.

## 3. Client loading (import map + loader)

- The shell build (`src/client/shell`, Vite) additionally emits **stable ESM entry chunks** for
  the shared runtimes: `svelte` (incl. `svelte/internal/client` and other subpaths the compiler
  emits), `@shadowcat/core`, `@shadowcat/ui-kit`, `@shadowcat/formula`, and the generated types
  package. Chunk filenames must be deterministic/discoverable so the server can inject the map.
- The served HTML (embedded via rust-embed) carries a `<script type="importmap">` mapping those
  bare specifiers (and required subpath patterns) to the host chunk URLs. The import map must be
  injected before any module script executes.
- `worldSession` (shell): after welcome/enabled-list, fetch `(manifest, entry_url)` pairs for
  enabled external modules and run them through the existing `loadModules`
  (`src/client/core/src/loader.ts`) with `importFn = (url) => import(/* @vite-ignore */ url)`.
  The existing manifest-id ↔ module-id mismatch check stays as the identity gate.
- Load failures (fetch error, import error, engine-compat mismatch, Zod-invalid manifest) are
  contained per module: log through the project logger + surface a non-blocking UI warning;
  the session continues without that module. A broken community module must never brick a world.
- First-party modules remain statically imported in `App.svelte` — no behavior change.

## 4. Module build toolchain + dev workflow

- **Module template** (deliverable, lives in the Shadowcat repo as `docs/` guide + a template
  used to bootstrap Nightfox): Vite library build with the Svelte plugin, `build.lib` ESM-only
  output, and `rollupOptions.external` matching `svelte`, `svelte/*`, and `@shadowcat/*`.
  Output = `dist/` containing `index.js` (+ chunks/assets) and the authored `module.json`
  copied through.
- **Dev flow (T4/T5):**
  1. Clone a Shadowcat checkout; clone the module repo into `src/modules/<id>/` (the pnpm
     workspace glob `src/modules/*` picks it up: engine package resolution, TS config, vitest —
     zero extra config). Nested external clones are kept out of Shadowcat's git status by
     adding the folder to the checkout's `.git/info/exclude` (git cannot pattern-match
     "directories that are their own repo"); the Nightfox bootstrap README documents this
     step.
  2. `pnpm --filter <module> dev` = watch build whose output lands in
     `<data-dir>/modules/<id>/` (target dir configurable via env/flag; the template wires it).
  3. Run the Shadowcat dev server; enable the module in a dev world; iterate. The module loads
     through the REAL install path every time (parity).
- **Unit tests** run in the module repo with vitest against workspace-resolved engine packages.
- **e2e access:** a documented script lets the module repo run integration tests against the
  checkout's existing `test_server` + e2e harness (real wire, real permissions, real load
  path). Deliverable = the script + a passing Nightfox smoke e2e (module installs, enables,
  loads, contributes a trivial surface).

## 5. Nightfox repository bootstrap (first deliverable)

- Scaffold the Nightfox repo LOCALLY (own project folder, own git repo — never pushed by the
  agent; the user creates the GitHub remote and pushes):
  - `module.json` (`id: "nightfox"`, `engines.shadowcat` range, empty
    capabilities/requirements to start),
  - template build config (Vite + Svelte + externals), tsconfig, vitest setup,
  - CI stub (three-OS matrix per the cross-platform directive — module unit tests don't need
    OS-specific behavior, but the pipeline shape is established from day one),
  - README documenting the clone-into-checkout dev flow (§4).
- Nightfox gets its own `.claude` scope + memory directory (per D16) — created at bootstrap.
- API friction discovered while building Nightfox externally is filed into Shadowcat's
  `POST_WORK_FINDINGS.md` as cross-repo API bug reports.

## Out of scope (logged to TODO.md at plan completion)

- Module upload/install UI (T2) — install stays manual-extract.
- Sandboxing/permissions for module JS (T2) — modules are admin-trusted.
- Hot enable/disable without reload (§2).
- Module marketplace/registry, signing, or update channels.

## Invariants (load-bearing)

1. Exactly ONE instance of `svelte` and of each `@shadowcat/*` package exists at runtime; the
   import map is the single resolution authority for external module code (T1/T3).
2. The server never executes or introspects module code; module serving is static files with a
   path-traversal guard (§1; ARCHITECTURE §2 invariant 6).
3. External modules load exclusively through the modules-folder pipeline in every environment,
   including development (T4).
4. A broken installed module degrades to a logged, user-visible warning — never a failed server
   start or bricked world (§1 scan, §3 load containment).
5. Engine-compat (`engines.shadowcat`) is enforced at enable AND load, with explicit errors (T6).
6. All new server paths are `std::path`-built; module serving and the toolchain work on the
   three-OS matrix and mobile browsers (import maps are baseline in evergreen browsers).
