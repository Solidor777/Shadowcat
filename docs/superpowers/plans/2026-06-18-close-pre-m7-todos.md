# Close Pre-M7 TODOs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the four actionable pre-M7 TODOs — ESLint TypeScript coverage, fast-fail disconnected search, throwing-handler isolation in `handleFrame`, and Argon2 `spawn_blocking` offload — leaving #5 and #6 as documented deferrals.

**Architecture:** Four independent fixes across the JS toolchain (`eslint.config.js`), the client core (`src/client/core/src/ws-client.ts`), and the Rust auth layer (`src/server/src/auth/`). Each is self-contained, test-first, and committed separately. Tasks 2–3 build on the linter gate established in Task 1.

**Tech Stack:** ESLint 9 flat config + `typescript-eslint`, TypeScript 5.6, Vitest 4, Rust + Tokio 1.52 + `argon2`.

## Global Constraints

- Cross-platform: no OS-specific paths or shell builtins; verified by the CI matrix (ubuntu/macos/windows). Copied verbatim from project `CLAUDE.md`.
- No debug code in commits: no `console.log`/`dbg!`/`println!` instrumentation; diagnostics go through `tracing`/the project logger.
- Remove each closed TODO line from `docs/TODO.md` in the same commit that closes it.
- One commit per task. Local CI green (`pnpm lint`, `pnpm -r test`, `cargo test`) before each commit. Do not push (push only on full-milestone completion).
- `tokio` server features already include `rt-multi-thread` (enables `spawn_blocking`); no `Cargo.toml` feature change needed.

---

### Task 1: ESLint TypeScript coverage

**Files:**
- Modify: `package.json:13-23` (add `typescript-eslint` dev dependency)
- Modify: `eslint.config.js` (add TS-scoped config block)
- Modify: `docs/TODO.md:5-6` (remove the Tooling TODO)
- Fix inline: any violations surfaced in `src/client/core/src/*.ts`, `src/types/*.ts`

**Interfaces:**
- Consumes: nothing.
- Produces: a `pnpm lint` gate that parses and lints `**/*.ts` with `typescript-eslint` recommended rules. Tasks 2–3 rely on this gate being green.

- [ ] **Step 1: Install typescript-eslint**

Run: `pnpm add -D -w typescript-eslint`
Expected: `typescript-eslint` appears under root `devDependencies`; `pnpm-lock.yaml` updates.

- [ ] **Step 2: Add the TS-scoped flat-config block**

Edit `eslint.config.js` to:

```js
import js from "@eslint/js";
import tseslint from "typescript-eslint";

export default [
  js.configs.recommended,
  ...tseslint.config({
    files: ["**/*.ts"],
    extends: [tseslint.configs.recommended],
  }),
  {
    ignores: ["dist/", "node_modules/", "**/*.svelte", "src/types/generated/"],
  },
  {
    // Transitional browser-side auth pages served by the server bundle.
    files: ["src/server/static/**/*.js"],
    languageOptions: {
      globals: {
        fetch: "readonly",
        FormData: "readonly",
        document: "readonly",
        window: "readonly",
      },
    },
  },
];
```

Rationale: `tseslint.config({ files: ["**/*.ts"], ... })` scopes the TS parser and rules to `.ts` only, leaving the `.js` server-static files on espree. Non-type-checked `recommended` preset — no `parserOptions.project` plumbing. Global `ignores` preserved so the Svelte `ui` package and ts-rs–generated types stay excluded.

- [ ] **Step 3: Verify the gate actually parses TypeScript**

Temporarily append `const _gateCheck: any = 1; void _gateCheck;` to `src/client/core/src/ws-client.ts`.
Run: `pnpm lint`
Expected: a `@typescript-eslint/no-explicit-any` error on that line — proves `.ts` is now linted (previously it would pass green).
Then delete the temporary line.

- [ ] **Step 4: Run lint and enumerate real violations**

Run: `pnpm lint`
Expected: either clean, or a finite list of `@typescript-eslint/*` violations in `src/client/core/src/*.ts` / `src/types/*.ts`.

- [ ] **Step 5: Fix each violation inline**

For each reported violation, fix the source (e.g. remove an unused var, narrow an `any`, add the missing case). Do NOT add blanket `eslint-disable` comments. If a specific rule is genuinely wrong for this codebase, STOP and surface it to the user with the rule name and the offending code rather than disabling it. If the volume is unexpectedly large (>~15 violations), STOP and report the count before proceeding.

- [ ] **Step 6: Confirm lint and tests are green**

Run: `pnpm lint && pnpm -r test`
Expected: both PASS.

- [ ] **Step 7: Remove the TODO and commit**

Delete the `## Tooling` TODO block (`docs/TODO.md` lines 5–6, both the heading and its bullet).

```bash
git add package.json pnpm-lock.yaml eslint.config.js docs/TODO.md src/
git commit -m "build: lint TypeScript via typescript-eslint (closes Tooling TODO)"
```

---

### Task 2: Fast-fail disconnected search

**Files:**
- Modify: `src/client/core/src/ws-client.ts:223-244` (`search`) and `:252-288` (`subscribeSearch`)
- Test: `src/client/core/src/ws-client.test.ts` (append two cases inside the existing `describe("WsClient", ...)`)
- Modify: `docs/TODO.md` (remove the first Client-core TODO)

**Interfaces:**
- Consumes: `this.transport` (private, `Transport | null`), already maintained by `open`/`stop`/`handleClose`.
- Produces: `search`/`subscribeSearch` reject synchronously with `Error("not connected")` when no live transport.

- [ ] **Step 1: Write the failing tests**

Append to `src/client/core/src/ws-client.test.ts`, before the closing `});` of the `describe`:

```ts
it("search rejects immediately when there is no live transport", async () => {
  const client = new WsClient({
    connect: () => Promise.resolve({ send: () => {}, close: () => {} }),
    handlers: noop,
  });
  // Not started → transport is null. A long timeout would otherwise hang.
  await expect(
    client.search("x", { timeoutMs: 60_000 }),
  ).rejects.toThrow(/not connected/i);
});

it("subscribeSearch rejects immediately when there is no live transport", async () => {
  const client = new WsClient({
    connect: () => Promise.resolve({ send: () => {}, close: () => {} }),
    handlers: noop,
  });
  await expect(
    client.subscribeSearch("x", { timeoutMs: 60_000 }, () => {}),
  ).rejects.toThrow(/not connected/i);
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `pnpm --filter @shadowcat/core test -- ws-client`
Expected: both new cases FAIL by timing out / not rejecting with "not connected".

- [ ] **Step 3: Add the guard to `search`**

In `ws-client.ts`, make `search`'s returned promise reject up-front when disconnected. Replace the body of the `return new Promise(...)` so it begins with:

```ts
return new Promise<SearchPage>((resolve, reject) => {
  if (!this.transport) {
    reject(new Error("not connected"));
    return;
  }
  const timer = setTimeout(() => {
    this.pending.delete(request_id);
    reject(new Error("search request timeout"));
  }, timeoutMs);
  this.pending.set(request_id, { resolve, reject, timer });
  this.send({
    type: "search",
    request_id,
    query,
    limit: opts.limit ?? 20,
    cursor: opts.cursor,
    subscribe: false,
  });
});
```

- [ ] **Step 4: Add the guard to `subscribeSearch`**

In `subscribeSearch`, the subscription must not be registered when disconnected. Move the `this.subscriptions.set(...)` to AFTER the guard. Replace the `this.subscriptions.set(request_id, onUpdate);` line and the start of the promise so it reads:

```ts
return new Promise<SubscriptionHandle>((resolve, reject) => {
  if (!this.transport) {
    reject(new Error("not connected"));
    return;
  }
  this.subscriptions.set(request_id, onUpdate);
  const timer = setTimeout(() => {
    this.pending.delete(request_id);
    this.subscriptions.delete(request_id);
    reject(new Error("subscribe request timeout"));
  }, timeoutMs);
  // ...unchanged pending.set + send below...
```

Delete the original `this.subscriptions.set(request_id, onUpdate);` that preceded the `return new Promise`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `pnpm --filter @shadowcat/core test -- ws-client`
Expected: all WsClient tests PASS, including the two new cases and the existing `search rejects on timeout` / `stop() rejects in-flight searches`.

- [ ] **Step 6: Lint, remove the TODO, commit**

Run: `pnpm lint`
Expected: PASS.
Remove the first bullet under `## Client core` in `docs/TODO.md` (the `WsClient.search`/`subscribeSearch` disconnected-timeout item).

```bash
git add src/client/core/src/ws-client.ts src/client/core/src/ws-client.test.ts docs/TODO.md
git commit -m "fix(client): fast-fail search/subscribeSearch when disconnected (closes Client-core TODO)"
```

---

### Task 3: Isolate throwing consumer handlers in `handleFrame`

**Files:**
- Modify: `src/client/core/src/ws-client.ts` (`handleFrame` consumer-callback sites; the `subscribeSearch` resolve wrapper; add a private `safeEmit`)
- Test: `src/client/core/src/ws-client.test.ts` (append one case)
- Modify: `docs/TODO.md` (remove the second Client-core TODO)

**Interfaces:**
- Consumes: `this.opts.handlers.onError` (optional).
- Produces: every synchronous consumer-callback dispatch in the message pump is wrapped so a throw routes to `onError` and never propagates out of `handleFrame`.

- [ ] **Step 1: Write the failing test**

Append to `src/client/core/src/ws-client.test.ts`, before the closing `});`:

```ts
it("a throwing onUpdate is surfaced, not thrown into the socket loop", async () => {
  const sent: string[] = [];
  let onMessage: (d: string) => void = () => {};
  const errors: unknown[] = [];
  const client = new WsClient({
    connect: (h) => {
      onMessage = h.onMessage;
      return Promise.resolve({ send: (d) => sent.push(d), close: () => {} });
    },
    handlers: { onCommand: () => {}, onError: (e) => errors.push(e) },
  });
  await client.start();
  let calls = 0;
  const p = client.subscribeSearch("dragon", { limit: 5 }, () => {
    calls += 1;
    if (calls === 1) throw new Error("handler boom");
  });
  const req = JSON.parse(sent.find((s) => JSON.parse(s).type === "search")!);
  // Initial result fires onUpdate, which throws. The throw must not prevent the
  // subscription promise from resolving, and must be routed to onError.
  onMessage(
    JSON.stringify({ type: "search_result", request_id: req.request_id, hits: [], next_cursor: null }),
  );
  const handle = await p;
  expect(calls).toBe(1);
  expect(errors).toHaveLength(1);
  // The subscription is still live: a later update still dispatches.
  onMessage(JSON.stringify({ type: "search_update", request_id: req.request_id, hits: [] }));
  expect(calls).toBe(2);
  handle.unsubscribe();
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `pnpm --filter @shadowcat/core test -- ws-client`
Expected: FAIL — the throwing `onUpdate` propagates out of the synchronous `onMessage(...)` call (or `await p` never resolves).

- [ ] **Step 3: Add the `safeEmit` helper**

In `ws-client.ts`, add a private method (place it next to `failPending`):

```ts
/** Run a consumer callback in isolation: a throw is routed to `onError` and
 * never propagates into the socket message pump. A throw from `onError`
 * itself is swallowed so the pump cannot die. */
private safeEmit(fn: () => void): void {
  try {
    fn();
  } catch (err) {
    try {
      this.opts.handlers.onError?.(err);
    } catch {
      // onError must not break the pump; ignore its failure.
    }
  }
}
```

- [ ] **Step 4: Wrap the four synchronous consumer-callback sites**

In `handleFrame`, wrap `onWelcome` and `onReject`:

```ts
case "welcome":
  this.serverOffsetMs = msg.server_time - this.now();
  this.safeEmit(() => this.opts.handlers.onWelcome?.(msg));
  if (msg.current_seq >= this.nextExpected) {
    this.send({ type: "resync_request", from_seq: this.nextExpected });
  }
  break;
```

```ts
case "reject":
  this.safeEmit(() => this.opts.handlers.onReject?.(msg.intent_id, msg.reason));
  break;
```

In the `search_update` case:

```ts
case "search_update": {
  const handler = this.subscriptions.get(msg.request_id);
  if (handler) this.safeEmit(() => handler(msg.hits));
  break;
}
```

In `subscribeSearch`'s pending resolve wrapper, guard the initial `onUpdate` so a throw cannot block `resolve(handle)`:

```ts
resolve: (page) => {
  this.safeEmit(() => onUpdate(page.hits));
  resolve({
    unsubscribe: () => {
      this.subscriptions.delete(request_id);
      this.send({ type: "unsubscribe", request_id });
    },
  });
},
```

Note: `onCommand` is already isolated in `applyEvent` (try/catch → `onError`); leave it as-is. Protocol logic (the `resync_request` send, `nextExpected`/`serverOffsetMs` updates) stays OUTSIDE `safeEmit`.

- [ ] **Step 5: Run the test to verify it passes**

Run: `pnpm --filter @shadowcat/core test -- ws-client`
Expected: all WsClient tests PASS, including the new case and the existing `subscribeSearch fires onUpdate...` / `a throwing onCommand is surfaced...`.

- [ ] **Step 6: Lint, remove the TODO, commit**

Run: `pnpm lint`
Expected: PASS.
Remove the second bullet under `## Client core` in `docs/TODO.md` (the `handleFrame` try/catch item). The `## Client core` heading now has no bullets — delete the heading too.

```bash
git add src/client/core/src/ws-client.ts src/client/core/src/ws-client.test.ts docs/TODO.md
git commit -m "fix(client): isolate throwing consumer handlers in handleFrame (closes Client-core TODO)"
```

---

### Task 4: Offload Argon2 to spawn_blocking

**Files:**
- Modify: `src/server/src/auth/password.rs` (add async wrappers + tests)
- Modify: `src/server/src/http/routes.rs:100` (login uses async verify)
- Modify: `src/server/src/auth/setup.rs:27` (`create_admin` uses async hash)
- Modify: `docs/TODO.md` (remove the first Server/auth TODO)

**Interfaces:**
- Consumes: existing sync `hash_password(&str) -> Result<String, argon2::password_hash::Error>` and `verify_password(&str, &str) -> bool`.
- Produces:
  - `pub async fn hash_password_async(plain: String) -> Result<String, argon2::password_hash::Error>`
  - `pub async fn verify_password_async(plain: String, phc: String) -> bool`

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` in `src/server/src/auth/password.rs`:

```rust
#[tokio::test]
async fn async_hash_then_async_verify_roundtrips() {
    let hash = hash_password_async("correct horse".to_owned())
        .await
        .expect("hash");
    assert!(verify_password_async("correct horse".to_owned(), hash.clone()).await);
    assert!(!verify_password_async("wrong horse".to_owned(), hash).await);
}

#[tokio::test]
async fn async_verify_false_on_unparseable_phc() {
    assert!(!verify_password_async("x".to_owned(), "not-a-phc-string".to_owned()).await);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p shadowcat auth::password`
Expected: FAIL to COMPILE — `hash_password_async` / `verify_password_async` not found.

- [ ] **Step 3: Add the async wrappers**

In `src/server/src/auth/password.rs`, after `verify_password`:

```rust
/// Async wrapper: runs the CPU-bound Argon2 hash on a blocking thread so the
/// async worker is not stalled for the ~tens of ms each hash costs. Owned
/// `String` because `spawn_blocking` requires a `'static` closure.
pub async fn hash_password_async(plain: String) -> Result<String, argon2::password_hash::Error> {
    tokio::task::spawn_blocking(move || hash_password(&plain))
        .await
        .map_err(|_| argon2::password_hash::Error::Crypto)?
}

/// Async wrapper for the CPU-bound verify. A `spawn_blocking` join failure
/// (panic) is treated as a verification failure — the safe default on the auth
/// path. Owned `String`s for the `'static` closure.
pub async fn verify_password_async(plain: String, phc: String) -> bool {
    tokio::task::spawn_blocking(move || verify_password(&plain, &phc))
        .await
        .unwrap_or(false)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p shadowcat auth::password`
Expected: PASS — including the existing sync tests and the two new async tests.

- [ ] **Step 5: Wire the login path to the async verify**

In `src/server/src/http/routes.rs`, change the import on line 12 to include the async fn, then replace line 100. The import becomes:

```rust
use crate::auth::password::{hash_password, verify_password_async};
```

Replace line 100:

```rust
    let verified = verify_password_async(body.password, verify_target).await;
```

`body.password` is moved here (it has no later use); `verify_target` is already an owned `String`. `verify_password` (sync) is still referenced by `anti_enumeration_phc` indirectly? No — `anti_enumeration_phc` calls `hash_password`. Confirm `verify_password` (sync) has no remaining caller in `routes.rs`; if the `use` previously imported `verify_password`, drop it from the import (it stays defined in `password.rs` for unit tests).

- [ ] **Step 6: Wire the admin-create path to the async hash**

In `src/server/src/auth/setup.rs`, change the import on line 5 and the call on line 27. Import:

```rust
use crate::auth::password::hash_password_async;
```

Replace line 27:

```rust
    let hash = hash_password_async(password.to_owned())
        .await
        .map_err(|_| AppError::Internal)?;
```

(`anti_enumeration_phc` in `routes.rs` keeps the sync `hash_password` — it is a one-time `OnceLock` init in a sync context and must not become async.)

- [ ] **Step 7: Run the full server test suite**

Run: `cargo test -p shadowcat`
Expected: PASS — login, setup/bootstrap, and capability/ws suites all green through the async hashing path.

- [ ] **Step 8: Remove the TODO and commit**

Remove the first bullet under `## Server / auth` in `docs/TODO.md` (the Argon2 `spawn_blocking` item). Leave the session-sweep bullet (#5).

```bash
git add src/server/src/auth/password.rs src/server/src/http/routes.rs src/server/src/auth/setup.rs docs/TODO.md
git commit -m "perf(auth): offload Argon2 hash/verify to spawn_blocking (closes Server/auth TODO)"
```

---

### Final verification

- [ ] **Step 1: Full local CI**

Run: `pnpm lint && pnpm -r test && cargo test -p shadowcat`
Expected: all PASS.

- [ ] **Step 2: Confirm TODO.md final state**

`docs/TODO.md` retains exactly two items: the `## Server / auth` session-sweep bullet (#5) and the `## Data layer` `set_pointer` bullet (#6). The `## Tooling` and `## Client core` sections are gone.

- [ ] **Step 3: Update graphify**

Run: `graphify update .`
Expected: graph reflects the new `safeEmit` / async-wrapper symbols.

## Deferrals (NOT in scope — remain in TODO.md)

- **#5 session sweep** — premature ("when session volume grows").
- **#6 `set_pointer` removal semantics** — blocked on the unbuilt merge engine.
