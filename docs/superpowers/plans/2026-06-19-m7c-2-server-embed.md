# M7c-2 — Server Embed Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: this project executes plans with
> the **mainline-plan-execution** skill (inline, per-task spec-compliance check +
> a single final branch review) — NOT subagent-driven-development or
> executing-plans. Steps use checkbox (`- [ ]`) syntax for tracking.

## Buddy-check directives

Touches the `init_gate` (a pre-auth request gate) and the binary's served-asset
boundary. Final review is a **single fresh-context review** (chosen at handoff);
escalate to buddy-check only if the diff unexpectedly grows into auth-enforcement
logic. Focus review on: nothing reachable pre-init that shouldn't be, and the
embed/build-ordering correctness.

**Goal:** Make the single binary serve the Vite `dist/` SPA: flip `embed.rs` from
`static/` to `dist/`, re-home the PWA/favicon assets into the client build, remove
the now-obsolete `init_gate`, retire `src/server/static/`, wire client→server
build ordering in CI, and run the Playwright entry-flow smoke against the binary.

**Architecture:** `rust_embed` embeds `../../dist/` (repo-root `dist/`, relative to
`src/server/`) at release-build time and reads it from disk in debug. The SPA uses
hash routing, so `/` → `index.html` and named assets are all `static_handler`
needs (no SPA path-fallback). `init_gate` is removed: the SPA routes setup-vs-login
via `/api/config`, `/api/setup` self-gates on `initialized`, and every other
`/api/*` requires a session (impossible pre-init).

**Tech Stack:** Rust, axum, rust-embed, Vite, Playwright, GitHub Actions.

## Global Constraints

- The binary genuinely needs the client bundle: a release build / embed test
  requires `dist/` to exist. CI builds the client before the server; locally,
  build the client first (`pnpm --filter @shadowcat/ui build`). Embed tests
  **self-skip when `dist/` is absent** (debug rust-embed reads from disk) so local
  `cargo test` without a client build still passes; CI builds `dist/` so they run.
- Hash routing → no server SPA-fallback; `static_handler`'s `/`→`index.html` +
  named-asset behavior is unchanged.
- Cross-platform: the CI client-build step runs on all three rust-matrix OSes.
- TDD where it applies (Rust tests); some steps are config/asset moves verified by
  build + the embed tests.
- Commands: `cargo test --bin shadowcat`-style does not apply (lib tests);
  server tests run via `cargo test -p shadowcat`. Client build:
  `pnpm --filter @shadowcat/ui build` (emits repo-root `dist/`).

---

### Task 1: Re-home PWA/favicon assets into the client build

**Files:**
- Create: `src/client/ui/public/{favicon.ico,favicon-16.png,favicon-32.png,apple-touch-icon.png,icon-192.png,icon-512.png,site.webmanifest}` (moved from `src/server/static/`)
- Modify: `src/client/ui/index.html` (favicon/manifest links; drop the dev icon path)

**Interfaces:** Produces a `dist/` (after `vite build`) containing `index.html` +
the favicon/PWA assets at the web root.

- [ ] **Step 1: Move the assets into the client public dir**

```bash
mkdir -p src/client/ui/public
git mv src/server/static/favicon.ico src/server/static/favicon-16.png \
       src/server/static/favicon-32.png src/server/static/apple-touch-icon.png \
       src/server/static/icon-192.png src/server/static/icon-512.png \
       src/server/static/site.webmanifest src/client/ui/public/
```

(Vite copies `public/` verbatim to `dist/` at the web root.)

- [ ] **Step 2: Reference them from the SPA index**

Replace `src/client/ui/index.html`:

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <link rel="icon" href="/favicon.ico" sizes="any" />
    <link rel="icon" type="image/png" sizes="32x32" href="/favicon-32.png" />
    <link rel="icon" type="image/png" sizes="16x16" href="/favicon-16.png" />
    <link rel="apple-touch-icon" href="/apple-touch-icon.png" />
    <link rel="manifest" href="/site.webmanifest" />
    <title>shadowcat</title>
  </head>
  <body>
    <div id="app"></div>
    <script type="module" src="/src/main.ts"></script>
  </body>
</html>
```

- [ ] **Step 3: Build the client and verify dist contents**

Run: `pnpm --filter @shadowcat/ui build`
Then: `ls dist/ dist/assets/`
Expected: `dist/index.html`, `dist/favicon.ico`, `dist/site.webmanifest`, the other
icons, and `dist/assets/*.js`/`*.css`.

- [ ] **Step 4: Commit**

```bash
git add src/client/ui/public src/client/ui/index.html
git commit -m "build(ui): re-home favicon/PWA assets into the client public dir"
```

---

### Task 2: Flip `embed.rs` to `dist/` + retire `static/`

**Files:**
- Modify: `src/server/src/http/embed.rs` (folder path + tests)
- Delete: `src/server/static/` (remaining files: `auth.js`, `index.html`,
  `login.html`, `setup.html`, `styles.css`)

**Interfaces:** `static_handler` serves the Vite `dist/` bundle; signature
unchanged.

- [ ] **Step 1: Repoint the embed folder**

In `src/server/src/http/embed.rs`, change the `RustEmbed` folder (repo-root
`dist/`, relative to `src/server/`):

```rust
/// Embedded client bundle. Embeds the Vite build output (`dist/` at the repo
/// root) into the binary. In debug, rust-embed reads from disk at runtime; a
/// release build embeds at compile time, so `dist/` must exist for `cargo build
/// --release` (CI builds the client first).
#[derive(rust_embed::RustEmbed)]
#[folder = "../../dist/"]
struct StaticAssets;
```

- [ ] **Step 2: Update the embed tests (self-skip when dist absent)**

Replace the `#[cfg(test)] mod tests` in `embed.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::StaticAssets;
    use crate::http::router;
    use crate::http::tests::initialized_state;

    /// The SPA bundle is a build artifact; when `dist/` has not been built these
    /// tests self-skip so local `cargo test` (no client build) still passes. CI
    /// builds the client first, so they run there.
    fn dist_built() -> bool {
        StaticAssets::get("index.html").is_some()
    }

    #[tokio::test]
    async fn serves_the_spa_index_and_assets() {
        if !dist_built() {
            eprintln!("skipping: dist/ not built (run `pnpm --filter @shadowcat/ui build`)");
            return;
        }
        let server = axum_test::TestServer::new(router(initialized_state().await).await).unwrap();

        let root = server.get("/").await;
        root.assert_status_ok();
        // The Vite SPA index mounts into #app and loads a module script.
        assert!(root.text().contains("id=\"app\""));

        // A known public asset is served from dist/.
        server.get("/favicon.ico").await.assert_status_ok();

        let missing = server.get("/does-not-exist").await;
        missing.assert_status_not_found();
    }
}
```

- [ ] **Step 3: Delete the retired static bundle**

```bash
git rm src/server/static/auth.js src/server/static/index.html \
       src/server/static/login.html src/server/static/setup.html \
       src/server/static/styles.css
```

(The favicon/PWA files already moved in Task 1; `src/server/static/` is now empty
and removed.)

- [ ] **Step 4: Build client, then run the embed test**

Run: `pnpm --filter @shadowcat/ui build && cargo test serves_the_spa_index_and_assets`
Expected: PASS (with `dist/` built).

- [ ] **Step 5: Commit**

```bash
git add src/server/src/http/embed.rs
git commit -m "feat(server): serve the Vite dist/ SPA; retire static bundle"
```

---

### Task 3: Remove the obsolete `init_gate`

**Files:**
- Modify: `src/server/src/http/middleware.rs` (remove `init_gate`)
- Modify: `src/server/src/http/mod.rs` (remove the middleware layer; update tests)

**Interfaces:** No more pre-init redirect; the SPA + endpoint self-gating cover it.

- [ ] **Step 1: Remove the middleware layer**

In `src/server/src/http/mod.rs` `router()`, delete the line:

```rust
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::init_gate,
        ))
```

- [ ] **Step 2: Remove the `init_gate` function**

In `src/server/src/http/middleware.rs`, delete the `init_gate` fn (and now-unused
imports). If the file becomes empty, delete it and drop `pub mod middleware;` from
`src/server/src/http/mod.rs`.

- [ ] **Step 3: Update the affected tests**

In `src/server/src/http/mod.rs` tests, `setup_creates_admin_then_closes` asserted
the pre-init redirect (gate behavior). Drop the `/`-serving assertions (they tested
the removed gate) and keep the setup→close→login API flow:

```rust
    #[tokio::test]
    async fn setup_creates_admin_then_closes() {
        let server = fresh_server().await;

        let setup = server
            .post("/api/setup")
            .json(&serde_json::json!({ "username": "admin", "password": "pw-admin" }))
            .await;
        setup.assert_status(axum::http::StatusCode::NO_CONTENT);

        // Now initialized: a second setup is a conflict.
        server
            .post("/api/setup")
            .json(&serde_json::json!({ "username": "x", "password": "y" }))
            .await
            .assert_status(axum::http::StatusCode::CONFLICT);

        // The created admin can log in.
        server
            .post("/api/login")
            .json(&serde_json::json!({ "username": "admin", "password": "pw-admin" }))
            .await
            .assert_status(axum::http::StatusCode::NO_CONTENT);
    }
```

In `headless_bootstrap_closes_setup_and_allows_login`, remove the
`server.get("/").await.assert_status_ok()` line (it depended on the gate / a served
index; `/` serving is covered by the embed test). Keep the setup-409 + login
assertions.

- [ ] **Step 4: Run the server suite**

Run: `cargo test -p shadowcat`
Expected: PASS (the embed test self-skips without `dist/`; everything else green).

- [ ] **Step 5: Commit**

```bash
git add src/server/src/http/middleware.rs src/server/src/http/mod.rs
git commit -m "feat(server): remove obsolete init_gate (SPA + endpoint self-gating cover it)"
```

---

### Task 4: CI — client→server build ordering

**Files:**
- Modify: `.github/workflows/ci.yml` (build the client in the `rust` job before
  cargo; add the Playwright job — Task 5)

**Interfaces:** The `rust` matrix job produces `dist/` before `cargo test` /
`cargo build --release`, so the embed test runs and the release build embeds.

- [ ] **Step 1: Add a client-build step to the `rust` job**

In `.github/workflows/ci.yml`, in `jobs.rust.steps`, after `checkout` and before
the Rust toolchain/cargo steps, add Node + pnpm + the client build:

```yaml
      - uses: pnpm/action-setup@v6
        with:
          version: 9
      - uses: actions/setup-node@v6
        with:
          node-version: 22
          cache: pnpm
      - run: pnpm install --frozen-lockfile
      - name: Build client bundle (embedded by the server)
        run: pnpm --filter @shadowcat/ui build
```

(Order: this runs before `cargo test --all` and `cargo build --release`, so `dist/`
exists for the embed test and the release embedding.)

- [ ] **Step 2: Verify the workflow is valid**

Run: a YAML lint / `git diff` review of `.github/workflows/ci.yml`.
Expected: the `rust` job builds the client before any cargo step; the `web` job is
unchanged.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: build the client bundle before the server embeds it"
```

---

### Task 5: Playwright entry-flow smoke against the binary

**Files:**
- Modify: `src/client/ui/package.json` (add `@playwright/test`; `e2e` script)
- Create: `src/client/ui/playwright.config.ts`, `src/client/ui/e2e/global-setup.ts`,
  `src/client/ui/e2e/entry-flow.spec.ts`
- Modify: `.github/workflows/ci.yml` (a Playwright job)

**Interfaces:** The built binary serves the SPA + `/api` on one origin; Playwright
drives the browser against it. A global-setup spawns the binary (reusing the
`server-process.ts` pattern) with an admin seeded and the setup window closed.

- [ ] **Step 1: Add the dependency + script**

Run: `pnpm --filter @shadowcat/ui add -D @playwright/test`
Add to `src/client/ui/package.json` scripts: `"e2e": "playwright test"`.

- [ ] **Step 2: Global setup — spawn the binary**

`src/client/ui/e2e/global-setup.ts`:

```ts
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

// Build + spawn the main `shadowcat` binary (which embeds the SPA + serves /api)
// with an admin seeded and the setup window off, on a fixed loopback port. The
// binary is the real served artifact — the faithful e2e target.
const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../../..");
let proc: ChildProcess | undefined;

export default async function globalSetup(): Promise<() => void> {
  const isWindows = process.platform === "win32";
  // Client bundle must exist for the (release) embed; build it.
  run("pnpm", ["--filter", "@shadowcat/ui", "build"]);
  run("cargo", ["build", "-p", "shadowcat", "--bin", "shadowcat"], isWindows);

  const exe = path.join(repoRoot, "target", "debug", isWindows ? "shadowcat.exe" : "shadowcat");
  proc = spawn(exe, [], {
    cwd: repoRoot,
    stdio: ["ignore", "inherit", "inherit"],
    env: {
      ...process.env,
      SHADOWCAT_BIND: "127.0.0.1:31999",
      SHADOWCAT_ADMIN_USER: "ops",
      SHADOWCAT_ADMIN_PASSWORD: "pw-boot",
      SHADOWCAT_SETUP_TOKEN: "off",
      SHADOWCAT_DB: ":memory:",
    },
  });
  await waitForHealth("http://127.0.0.1:31999/health");
  return () => {
    if (proc?.pid === undefined) return;
    if (isWindows) spawnSync("taskkill", ["/pid", String(proc.pid), "/T", "/F"]);
    else proc.kill("SIGKILL");
  };
}

function run(cmd: string, args: string[], shell = false): void {
  const r = spawnSync(cmd, args, { cwd: repoRoot, stdio: "inherit", shell });
  if (r.status !== 0) throw new Error(`${cmd} ${args.join(" ")} failed (${r.status})`);
}

async function waitForHealth(url: string): Promise<void> {
  for (let i = 0; i < 100; i++) {
    try {
      if ((await fetch(url)).ok) return;
    } catch {
      /* not up yet */
    }
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error(`server did not become healthy at ${url}`);
}
```

> Verify against `src/server/src/config.rs` that `SHADOWCAT_ADMIN_USER`/
> `_PASSWORD`/`SETUP_TOKEN`/`BIND`/`DB` are the real env keys
> (`Env::prefixed("SHADOWCAT_")` over the `Config` fields `admin_user`,
> `admin_password`, `setup_token`, `bind`, `db`). Confirm the bootstrap seeds the
> admin from `admin_user`/`admin_password` (it does — `bootstrap_admin`) and that
> `:memory:` is an acceptable `db`. Adjust keys if they differ.

- [ ] **Step 3: Playwright config**

`src/client/ui/playwright.config.ts`:

```ts
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  globalSetup: "./e2e/global-setup.ts",
  use: { baseURL: "http://127.0.0.1:31999" },
});
```

- [ ] **Step 4: The smoke spec**

`src/client/ui/e2e/entry-flow.spec.ts`:

```ts
import { test, expect } from "@playwright/test";

test("login → world-select → enter table (served by the binary)", async ({ page }) => {
  await page.goto("/");
  // The SPA boots, sees an initialized server, routes to Login.
  await page.getByLabel("Username").fill("ops");
  await page.getByLabel("Password").fill("pw-boot");
  await page.getByRole("button", { name: "Log in" }).click();

  await expect(page.getByText("Your worlds")).toBeVisible();
  await page.getByLabel("New world name").fill("Smoke World");
  await page.getByRole("button", { name: "Create world" }).click();

  // Entering a world reaches the table shell (the stage placeholder).
  await expect(page.getByText("Scene rendering arrives in M8.")).toBeVisible();
});
```

- [ ] **Step 5: Run it locally**

Run: `pnpm --filter @shadowcat/ui exec playwright install --with-deps chromium`
Then: `pnpm --filter @shadowcat/ui exec playwright test`
Expected: PASS (global-setup builds + spawns the binary; the smoke walks the flow).

- [ ] **Step 6: Add the CI job**

In `.github/workflows/ci.yml`, add a `ui-e2e` job (both toolchains, like `e2e`):

```yaml
  ui-e2e:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - uses: dtolnay/rust-toolchain@stable
      - uses: pnpm/action-setup@v6
        with:
          version: 9
      - uses: actions/setup-node@v6
        with:
          node-version: 22
          cache: pnpm
      - run: pnpm install --frozen-lockfile
      - run: pnpm --filter @shadowcat/ui exec playwright install --with-deps chromium
      - run: pnpm --filter @shadowcat/ui exec playwright test
```

- [ ] **Step 7: Commit**

```bash
git add src/client/ui/package.json src/client/ui/playwright.config.ts \
        src/client/ui/e2e .github/workflows/ci.yml pnpm-lock.yaml
git commit -m "test(ui): Playwright entry-flow smoke against the binary"
```

---

### Task 6: Full green

- [ ] **Step 1: Client build + server suite**

Run: `pnpm --filter @shadowcat/ui build && cargo test -p shadowcat`
Expected: PASS — embed test runs (dist built), all server tests green.

- [ ] **Step 2: Clippy + client typecheck/tests**

Run: `cargo clippy -p shadowcat --all-targets -- -D warnings`
Then: `pnpm --filter @shadowcat/ui test && pnpm --filter @shadowcat/ui typecheck`
Expected: clean / PASS.

- [ ] **Step 3: Release build embeds the SPA**

Run: `cargo build --release` (with `dist/` built)
Expected: succeeds; the release binary embeds the SPA.

---

## Self-Review

**Spec coverage (spec §10, §11):**
- `embed.rs` `static/`→`dist/` (§10) → Task 2. ✓
- `init_gate` rework — **resolved as removal** (cleaner; SPA + endpoint
  self-gating cover it) (§10) → Task 3. ✓
- Retire `src/server/static/` (§10) → Tasks 1 (assets moved) + 2 (rest deleted). ✓
- Client→server build ordering (§10) → Task 4. ✓
- Embed tests (§10) → Task 2. ✓
- Playwright smoke against the binary (§11, moved from M7c-1) → Task 5. ✓
- **Discovered, not in §10:** favicon/PWA assets must move into the client build
  (Task 1) — else the served SPA loses its icons/manifest.

**Placeholder scan:** No TBD/TODO. The global-setup env-key "verify against
config.rs" note is a reconciliation instruction, not a placeholder.

**Type/behavior consistency:** the embed folder `../../dist/` matches the Vite
`outDir` (`../../../dist` from `src/client/ui` = repo-root `dist/`); the Playwright
`baseURL` port matches the global-setup `SHADOWCAT_BIND`; `init_gate` removal is
reflected in both the router and the tests.

## Decisions resolved during planning (flag for review)

1. **`init_gate` → removed**, not reworked. The SPA routes setup-vs-login via
   `/api/config`; `/api/setup` self-gates on `initialized`; other `/api/*` need a
   session (impossible pre-init). Removal is simpler and fully correct.
2. **CI build ordering → client build added to the `rust` matrix job** (Node on all
   three OSes). Alternative considered: build `dist/` once and pass it as an
   artifact to the rust job — saves two redundant builds but adds cross-job
   orchestration; per-runner build chosen for simplicity.
3. **Favicon/PWA assets → `src/client/ui/public/`** so Vite carries them into
   `dist/` (Task 1).

## Out of scope (M7d)

Theming (3-tier SCSS tokens + dark theme), i18n (`t()`), and session-restore
(lastWorld/active-tab/locale persistence via the M7a `ui_state` blob). M7c-2 ends
M7c: the binary serves the running, navigable (minimally-styled) SPA.
