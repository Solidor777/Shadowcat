# M1 — Project Infrastructure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `mainline-plan-execution` to implement this plan task-by-task (per user-scope workflow guidance, which replaces `superpowers:subagent-driven-development` / `superpowers:executing-plans` for this project). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the `src/`-rooted Cargo + pnpm monorepo with a minimal Rust server, the SQLite-only data target, the Rust→TS type pipeline, a Vite/Svelte client build, and CI that enforces all of it.

**Architecture:** One repo, two build halves. A Rust workspace under `src/server/` compiles to a single binary; a pnpm/Vite workspace under `src/client/` + `src/types/` builds static assets to `dist/`. `ts-rs` generates TypeScript types from Rust structs into `src/types/generated/` (committed, CI-verified in sync). No HTTP, no UI logic, no game features yet — this milestone proves the toolchain end-to-end.

**Tech Stack:** Rust (Cargo workspace, tokio 1.52, sqlx 0.9 + SQLite, ts-rs 12, serde), TypeScript 5, Svelte 5, Vite 8, pnpm, Vitest, GitHub Actions.

## Global Constraints

Copied from `docs/design/ARCHITECTURE.md`. Every task's requirements implicitly include these.

- **Permissive licenses only:** MIT / Apache-2.0 / BSD / zlib / MPL-2.0. No GPL / AGPL / SSPL / proprietary in runtime or required toolchain.
- **SQLite-only data target.** No Postgres. No Tantivy, no zstd, no blake3 in M1.
- **Single binary.** The client bundle is embedded into the server binary (rust-embed) in a later milestone; M1 only produces the two build halves and the `dist/` output. Build-time tools (pnpm, Vite) never ship.
- **Source resides under `src/`:** `src/server/`, `src/client/{core,ui}/`, `src/modules/`, `src/types/`. Build output goes to `dist/`. Config/manifests live at the repo root.
- **Headless core is Svelte-free:** `@shadowcat/core` must have no Svelte runtime in its dependency closure (invariant 7).
- **Release profile:** `opt-level = "z"`.
- **Type pipeline is CI-enforced:** generated TS under `src/types/generated/` must match a fresh `cargo test` run (no drift).
- Rust toolchain: stable. Node: 22+. pnpm: 9+.

---

## File Structure

```
/                              repo root
  Cargo.toml                   Rust workspace (members = ["src/server"]) + [profile.release]
  rust-toolchain.toml          pin stable + rustfmt/clippy
  pnpm-workspace.yaml          packages: src/types, src/client/*, src/modules/*
  package.json                 root dev tooling + aggregate scripts
  pnpm-lock.yaml               committed lockfile
  tsconfig.base.json           shared strict TS config
  eslint.config.js             flat ESLint config
  .gitignore                   extended (node_modules, /dist)
  .github/workflows/ci.yml     Rust + web CI
  dist/                        Vite output (gitignored)
  src/
    server/
      Cargo.toml               bin+lib crate "shadowcat"
      src/main.rs              prints health snapshot
      src/lib.rs               module declarations
      src/health.rs            HealthStatus (ts-rs export source)
      src/db.rs                SQLite pool open + smoke test
    types/
      package.json             @shadowcat/types
      tsconfig.json
      index.ts                 re-exports generated/
      generated/HealthStatus.ts  ts-rs output (committed)
    client/
      core/
        package.json           @shadowcat/core (Svelte-free)
        tsconfig.json
        src/index.ts           isHealthy(status)
        src/index.test.ts      vitest
      ui/
        package.json           @shadowcat/ui (Vite + Svelte 5)
        vite.config.ts         outDir ../../../dist
        svelte.config.js
        tsconfig.json
        index.html
        src/main.ts
        src/App.svelte
    modules/
      .gitkeep                 placeholder
```

**Responsibility boundaries:** `src/server` owns all Rust; `src/types` is the generated-type bridge (Rust→TS, no hand-written logic); `src/client/core` is framework-neutral client logic (consumes `@shadowcat/types`, never imports Svelte); `src/client/ui` is the Svelte default UI and the only Vite build. Root files are config only.

---

## Task 1: Workspace scaffold

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`, `pnpm-workspace.yaml`, `package.json`, `tsconfig.base.json`, `src/modules/.gitkeep`
- Modify: `.gitignore`
- Remove: the empty `source/` directory

**Interfaces:**
- Produces: the Cargo workspace (member `src/server`, added in Task 2 — declared here), the pnpm workspace, root scripts `typecheck`/`test`/`lint`/`build`, the shared `tsconfig.base.json`, and the `opt-level="z"` release profile.

- [ ] **Step 1: Remove the empty `source/` dir and create the module placeholder**

```bash
rmdir source 2>/dev/null || true
mkdir -p src/modules
printf '' > src/modules/.gitkeep
```

- [ ] **Step 2: Write `Cargo.toml` (workspace root)**

```toml
[workspace]
resolver = "2"
members = ["src/server"]

[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
strip = true
panic = "abort"
```

- [ ] **Step 3: Write `rust-toolchain.toml`**

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

- [ ] **Step 4: Write `pnpm-workspace.yaml`**

```yaml
packages:
  - "src/types"
  - "src/client/*"
  - "src/modules/*"
```

- [ ] **Step 5: Write root `package.json`**

```json
{
  "name": "shadowcat",
  "version": "0.0.0",
  "private": true,
  "type": "module",
  "packageManager": "pnpm@9",
  "scripts": {
    "typecheck": "pnpm -r typecheck",
    "test": "pnpm -r test",
    "lint": "eslint .",
    "build": "pnpm --filter @shadowcat/ui build"
  },
  "devDependencies": {
    "typescript": "^5.6.0",
    "vite": "^8.0.0",
    "svelte": "^5.56.0",
    "@sveltejs/vite-plugin-svelte": "^5.0.0",
    "svelte-check": "^4.0.0",
    "vitest": "^2.0.0",
    "eslint": "^9.0.0",
    "@eslint/js": "^9.0.0",
    "prettier": "^3.0.0"
  }
}
```

Note: exact dependency versions are resolved by pnpm. If a peer-dependency error appears (e.g., the Svelte plugin vs. Vite 8), accept pnpm's suggested compatible version and proceed.

- [ ] **Step 6: Write `tsconfig.base.json`**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "verbatimModuleSyntax": true,
    "isolatedModules": true
  }
}
```

- [ ] **Step 7: Extend `.gitignore`**

Append these lines (the file already ignores `target`, `debug`, and dumps):

```gitignore
# Node
node_modules/

# Client build output
/dist/

# TS incremental build info
*.tsbuildinfo
```

- [ ] **Step 8: Verify the workspaces resolve and install root tooling**

Run:
```bash
cargo metadata --format-version 1 >/dev/null && echo CARGO_OK
pnpm install
```
Expected: `CARGO_OK` printed; `pnpm install` completes, creating `node_modules/` and `pnpm-lock.yaml`. (No workspace packages exist yet, so pnpm installs only root devDependencies.)

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml rust-toolchain.toml pnpm-workspace.yaml package.json tsconfig.base.json pnpm-lock.yaml .gitignore src/modules/.gitkeep
git commit -m "chore(m1): scaffold src/ monorepo workspaces and root tooling"
```

---

## Task 2: Rust server crate + HealthStatus

**Files:**
- Create: `src/server/Cargo.toml`, `src/server/src/lib.rs`, `src/server/src/main.rs`, `src/server/src/health.rs`

**Interfaces:**
- Produces: `shadowcat::health::HealthStatus { status: String, db_connected: bool }` with `HealthStatus::ok(db_connected: bool) -> HealthStatus`. Consumed by Task 4 (ts-rs export) and Task 3 (binary wiring).

- [ ] **Step 1: Write `src/server/Cargo.toml`**

```toml
[package]
name = "shadowcat"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = { version = "1.52", features = ["macros", "rt-multi-thread"] }
sqlx = { version = "0.9", default-features = false, features = ["runtime-tokio", "sqlite", "macros"] }
serde = { version = "1", features = ["derive"] }
ts-rs = "12"
```

- [ ] **Step 2: Write the failing test in `src/server/src/health.rs`**

```rust
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Server health snapshot shared with the client via the ts-rs type pipeline.
/// INVARIANT: the TS mirror in src/types/generated must be regenerated whenever
/// this struct changes (CI enforces sync).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../types/generated/")]
pub struct HealthStatus {
    pub status: String,
    pub db_connected: bool,
}

impl HealthStatus {
    pub fn ok(db_connected: bool) -> Self {
        Self { status: "ok".to_string(), db_connected }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_reports_ok_status_and_passes_through_db_flag() {
        let s = HealthStatus::ok(true);
        assert_eq!(s.status, "ok");
        assert!(s.db_connected);
    }
}
```

- [ ] **Step 3: Write `src/server/src/lib.rs`**

```rust
pub mod health;
```

- [ ] **Step 4: Run the test to verify it fails to compile/link first time, then passes after lib wiring**

Run:
```bash
cargo test -p shadowcat health::tests::ok_reports_ok_status_and_passes_through_db_flag
```
Expected: PASS (the implementation is included in the same step set; if the crate did not yet declare `pub mod health`, the failure would be `unresolved module`). Confirm green.

- [ ] **Step 5: Write `src/server/src/main.rs`**

```rust
fn main() {
    let status = shadowcat::health::HealthStatus::ok(false);
    println!("shadowcat {} (db_connected={})", status.status, status.db_connected);
}
```

- [ ] **Step 6: Verify the binary builds and runs**

Run:
```bash
cargo run -p shadowcat
```
Expected: prints `shadowcat ok (db_connected=false)`.

- [ ] **Step 7: Commit**

```bash
git add src/server/Cargo.toml src/server/src/lib.rs src/server/src/main.rs src/server/src/health.rs Cargo.lock
git commit -m "feat(m1): minimal server crate with HealthStatus"
```

---

## Task 3: SQLite data target smoke test

**Files:**
- Create: `src/server/src/db.rs`
- Modify: `src/server/src/lib.rs`

**Interfaces:**
- Consumes: nothing from prior tasks.
- Produces: `shadowcat::db::open_pool(url: &str) -> Result<sqlx::SqlitePool, sqlx::Error>`.

- [ ] **Step 1: Write the failing test in `src/server/src/db.rs`**

```rust
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

/// Opens a SQLite connection pool. `"sqlite::memory:"` yields an ephemeral
/// in-process database — used here to prove the SQLite-only target wires up.
pub async fn open_pool(url: &str) -> Result<SqlitePool, sqlx::Error> {
    SqlitePoolOptions::new().max_connections(1).connect(url).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_pool_answers_select_one() {
        let pool = open_pool("sqlite::memory:").await.expect("open pool");
        let row: (i64,) = sqlx::query_as("SELECT 1")
            .fetch_one(&pool)
            .await
            .expect("query");
        assert_eq!(row.0, 1);
    }
}
```

- [ ] **Step 2: Declare the module in `src/server/src/lib.rs`**

```rust
pub mod db;
pub mod health;
```

- [ ] **Step 3: Run the test to verify it passes**

Run:
```bash
cargo test -p shadowcat db::tests::in_memory_pool_answers_select_one
```
Expected: PASS. (This confirms sqlx + the SQLite driver + the tokio runtime are correctly wired.)

- [ ] **Step 4: Commit**

```bash
git add src/server/src/db.rs src/server/src/lib.rs Cargo.lock
git commit -m "feat(m1): SQLite pool open + in-memory smoke test"
```

---

## Task 4: ts-rs type pipeline (Rust → TS)

**Files:**
- Create: `src/types/package.json`, `src/types/tsconfig.json`, `src/types/index.ts`, `src/types/generated/HealthStatus.ts` (generated)

**Interfaces:**
- Consumes: `shadowcat::health::HealthStatus` (Task 2), which carries `#[ts(export, export_to = "../types/generated/")]`.
- Produces: the `@shadowcat/types` package exporting the TS type `HealthStatus = { status: string, db_connected: boolean }`. Consumed by Task 5.

- [ ] **Step 1: Generate the TS bindings**

`#[ts(export)]` makes `cargo test` emit the binding. Run:
```bash
cargo test -p shadowcat
ls src/types/generated/HealthStatus.ts
```
Expected: `cargo test` passes and `src/types/generated/HealthStatus.ts` now exists. Its contents are ts-rs–authored, approximately:

```ts
// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.

export type HealthStatus = { status: string, db_connected: boolean, };
```

Commit whatever `cargo test` actually produces — do not hand-edit it.

- [ ] **Step 2: Write `src/types/package.json`**

```json
{
  "name": "@shadowcat/types",
  "version": "0.0.0",
  "private": true,
  "type": "module",
  "main": "index.ts",
  "types": "index.ts",
  "scripts": {
    "typecheck": "tsc --noEmit",
    "test": "echo \"@shadowcat/types: no tests\" && exit 0"
  }
}
```

- [ ] **Step 3: Write `src/types/tsconfig.json`**

```json
{
  "extends": "../../tsconfig.base.json",
  "include": ["**/*.ts"]
}
```

- [ ] **Step 4: Write `src/types/index.ts`**

```ts
export type { HealthStatus } from "./generated/HealthStatus";
```

- [ ] **Step 5: Install and typecheck the new package**

Run:
```bash
pnpm install
pnpm --filter @shadowcat/types typecheck
```
Expected: install updates the lockfile; typecheck passes with no errors.

- [ ] **Step 6: Verify the sync check works**

Run:
```bash
cargo test -p shadowcat
git diff --exit-code src/types/generated
```
Expected: exit code 0 (no diff) — committed bindings match a fresh generation. This is the exact check CI runs in Task 7.

- [ ] **Step 7: Commit**

```bash
git add src/types/package.json src/types/tsconfig.json src/types/index.ts src/types/generated/HealthStatus.ts pnpm-lock.yaml
git commit -m "feat(m1): ts-rs Rust->TS type pipeline with @shadowcat/types"
```

---

## Task 5: Svelte-free `@shadowcat/core` package

**Files:**
- Create: `src/client/core/package.json`, `src/client/core/tsconfig.json`, `src/client/core/src/index.ts`, `src/client/core/src/index.test.ts`

**Interfaces:**
- Consumes: `HealthStatus` from `@shadowcat/types` (Task 4).
- Produces: `isHealthy(status: HealthStatus): boolean`. Proves the Rust→TS type flows into client logic, and that the core package has no Svelte dependency.

- [ ] **Step 1: Write `src/client/core/package.json` (note: no `svelte` dependency)**

```json
{
  "name": "@shadowcat/core",
  "version": "0.0.0",
  "private": true,
  "type": "module",
  "main": "src/index.ts",
  "dependencies": {
    "@shadowcat/types": "workspace:*"
  },
  "scripts": {
    "typecheck": "tsc --noEmit",
    "test": "vitest run"
  }
}
```

- [ ] **Step 2: Write `src/client/core/tsconfig.json`**

```json
{
  "extends": "../../../tsconfig.base.json",
  "include": ["src/**/*.ts"]
}
```

- [ ] **Step 3: Write the failing test in `src/client/core/src/index.test.ts`**

```ts
import { describe, it, expect } from "vitest";
import { isHealthy } from "./index";

describe("isHealthy", () => {
  it("is true only when status is ok and the db is connected", () => {
    expect(isHealthy({ status: "ok", db_connected: true })).toBe(true);
    expect(isHealthy({ status: "ok", db_connected: false })).toBe(false);
    expect(isHealthy({ status: "down", db_connected: true })).toBe(false);
  });
});
```

- [ ] **Step 4: Run the test to verify it fails**

Run:
```bash
pnpm install
pnpm --filter @shadowcat/core test
```
Expected: FAIL — `isHealthy` is not exported from `./index` (module has no such export / file missing).

- [ ] **Step 5: Write `src/client/core/src/index.ts`**

```ts
import type { HealthStatus } from "@shadowcat/types";

/** Returns true when the server reports itself healthy with a live database. */
export function isHealthy(status: HealthStatus): boolean {
  return status.status === "ok" && status.db_connected;
}
```

- [ ] **Step 6: Run the test to verify it passes, and typecheck**

Run:
```bash
pnpm --filter @shadowcat/core test
pnpm --filter @shadowcat/core typecheck
```
Expected: test PASS; typecheck clean.

- [ ] **Step 7: Verify the core package is Svelte-free**

Run:
```bash
pnpm --filter @shadowcat/core why svelte || echo "SVELTE_ABSENT"
```
Expected: prints `SVELTE_ABSENT` (or pnpm reports no `svelte` in the package's dependency closure). If Svelte appears, a dependency was added in error — remove it.

- [ ] **Step 8: Commit**

```bash
git add src/client/core pnpm-lock.yaml
git commit -m "feat(m1): Svelte-free @shadowcat/core consuming generated types"
```

---

## Task 6: `@shadowcat/ui` Vite + Svelte 5 build to `dist/`

**Files:**
- Create: `src/client/ui/package.json`, `src/client/ui/vite.config.ts`, `src/client/ui/svelte.config.js`, `src/client/ui/tsconfig.json`, `src/client/ui/index.html`, `src/client/ui/src/main.ts`, `src/client/ui/src/App.svelte`

**Interfaces:**
- Consumes: `@shadowcat/core` (Task 5) — wired as a dependency to prove the core is consumable from the UI.
- Produces: a Vite build emitting `dist/index.html` + assets at the repo root.

- [ ] **Step 1: Write `src/client/ui/package.json`**

```json
{
  "name": "@shadowcat/ui",
  "version": "0.0.0",
  "private": true,
  "type": "module",
  "dependencies": {
    "@shadowcat/core": "workspace:*"
  },
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "typecheck": "svelte-check --tsconfig ./tsconfig.json",
    "test": "echo \"@shadowcat/ui: no tests\" && exit 0"
  }
}
```

- [ ] **Step 2: Write `src/client/ui/svelte.config.js`**

```js
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

export default { preprocess: vitePreprocess() };
```

- [ ] **Step 3: Write `src/client/ui/vite.config.ts`**

```ts
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// outDir resolves from src/client/ui to the repo-root dist/.
export default defineConfig({
  plugins: [svelte()],
  build: { outDir: "../../../dist", emptyOutDir: true },
});
```

- [ ] **Step 4: Write `src/client/ui/tsconfig.json`**

```json
{
  "extends": "../../../tsconfig.base.json",
  "compilerOptions": { "types": ["svelte"] },
  "include": ["src/**/*.ts", "src/**/*.svelte"]
}
```

- [ ] **Step 5: Write `src/client/ui/index.html`**

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>shadowcat</title>
  </head>
  <body>
    <div id="app"></div>
    <script type="module" src="/src/main.ts"></script>
  </body>
</html>
```

- [ ] **Step 6: Write `src/client/ui/src/App.svelte`**

```svelte
<script lang="ts">
  const title = "shadowcat";
</script>

<main>
  <h1>{title}</h1>
</main>
```

- [ ] **Step 7: Write `src/client/ui/src/main.ts`**

```ts
import { mount } from "svelte";
import App from "./App.svelte";

const app = mount(App, { target: document.getElementById("app")! });

export default app;
```

- [ ] **Step 8: Install, typecheck, and build**

Run:
```bash
pnpm install
pnpm --filter @shadowcat/ui typecheck
pnpm --filter @shadowcat/ui build
test -f dist/index.html && echo DIST_OK
```
Expected: typecheck clean; build succeeds; `DIST_OK` printed (`dist/index.html` exists). `dist/` is gitignored.

- [ ] **Step 9: Commit**

```bash
git add src/client/ui pnpm-lock.yaml
git commit -m "feat(m1): @shadowcat/ui Vite+Svelte5 build to dist/"
```

---

## Task 7: CI pipeline + lint config

**Files:**
- Create: `eslint.config.js`, `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: every prior task (the CI runs their verification commands).
- Produces: a green CI on push/PR enforcing fmt, clippy, Rust tests, ts-rs sync, release-build size budget, TS typecheck/test/lint, and the client build.

- [ ] **Step 1: Write `eslint.config.js` (flat config)**

```js
import js from "@eslint/js";

export default [
  js.configs.recommended,
  {
    ignores: ["dist/", "node_modules/", "**/*.svelte", "src/types/generated/"],
  },
];
```

- [ ] **Step 2: Verify lint passes locally**

Run:
```bash
pnpm lint
```
Expected: ESLint completes with no errors. (Svelte files and generated types are excluded for M1; broader linting lands when there is real client logic.)

- [ ] **Step 3: Write `.github/workflows/ci.yml`**

```yaml
name: CI

on:
  push:
  pull_request:

jobs:
  rust:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - name: Format
        run: cargo fmt --all -- --check
      - name: Clippy
        run: cargo clippy --all-targets -- -D warnings
      - name: Test (emits ts-rs bindings)
        run: cargo test --all
      - name: ts-rs bindings in sync
        run: git diff --exit-code src/types/generated
      - name: Release build
        run: cargo build --release
      - name: Binary size budget
        run: |
          size=$(stat -c%s target/release/shadowcat)
          echo "release binary size: ${size} bytes"
          # 60 MiB guardrail; tighten as the binary's real baseline settles.
          test "${size}" -lt 62914560

  web:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
        with:
          version: 9
      - uses: actions/setup-node@v4
        with:
          node-version: 22
          cache: pnpm
      - run: pnpm install --frozen-lockfile
      - run: pnpm -r typecheck
      - run: pnpm -r test
      - run: pnpm lint
      - run: pnpm --filter @shadowcat/ui build
```

- [ ] **Step 4: Reproduce the full CI locally before committing**

Run each CI command and confirm all pass:
```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all
git diff --exit-code src/types/generated
cargo build --release
pnpm -r typecheck
pnpm -r test
pnpm lint
pnpm --filter @shadowcat/ui build
```
Expected: every command exits 0. (`cargo-bloat` is the local introspection tool for the size budget — `cargo install cargo-bloat` then `cargo bloat --release --crates` — but CI enforces the budget with the `stat` check above to avoid a network install in CI.)

- [ ] **Step 5: Commit**

```bash
git add eslint.config.js .github/workflows/ci.yml pnpm-lock.yaml
git commit -m "ci(m1): enforce rust + web checks, ts-rs sync, and size budget"
```

- [ ] **Step 6: Push and confirm CI is green**

```bash
git push origin main
gh run watch
```
Expected: both `rust` and `web` jobs pass. If red, fix-forward from the topmost failing step and re-push.

---

## Self-Review

**1. Spec coverage (M1 deliverables from `docs/PLAN.md`):**
- Monorepo under `src/` (Cargo + pnpm), Vite, rename `source/` → Task 1. ✓
- CI: Rust tests, TS typecheck, lint, size budget → Task 7. ✓
- ts-rs pipeline, CI-enforced sync → Tasks 4, 7. ✓
- SQLite-only target → Tasks 2, 3 (sqlx sqlite; no Postgres). ✓
- Release `opt-level="z"` → Task 1 (`[profile.release]`). ✓
- Excludes Postgres/Tantivy/zstd/blake3 → none added. ✓
- Svelte-free core (invariant 7) → Task 5 (no svelte dep + Step 7 check). ✓

**2. Placeholder scan:** No TBD/TODO/"handle errors"/"similar to". Every code step shows complete content. The one generated file (Task 4 `HealthStatus.ts`) is explicitly "commit what `cargo test` produces, do not hand-edit" — correct for a generated artifact, not a placeholder.

**3. Type consistency:** Rust `HealthStatus { status: String, db_connected: bool }` (Task 2) → ts-rs `HealthStatus = { status: string, db_connected: boolean }` (Task 4) → consumed by `isHealthy(status: HealthStatus)` (Task 5) using `status.status` and `status.db_connected`. Names match across all three. `open_pool` signature consistent between definition and test (Task 3).

**Notes for the executor:**
- Dependency versions use caret ranges; if pnpm reports a peer-dependency conflict (notably `@sveltejs/vite-plugin-svelte` vs Vite 8), accept the resolver's compatible version.
- `cargo fmt` before each Rust commit to keep the `fmt --check` CI step green.
- This milestone establishes no public API, no auth/crypto/concurrency/determinism/unsafe code, and no save format — it is reversible scaffolding (no buddy-check-level risk signals).
