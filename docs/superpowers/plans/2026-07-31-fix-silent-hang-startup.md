# Fix Silent-Hang Startup Paths Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the four OPEN_BUGS "Silent-hang startup paths" (2026-07-31 render-ready audit): no Welcome watchdog; unbounded/unretried boot fetches; no WS connect timeout; the `#bootstrapped` latch caching a failed module activation.

**Architecture:** All timeouts live where the awaiting code already lives (never a new layer): the Welcome watchdog goes INSIDE `WsClient` (which owns transports, the welcome frame, and reconnect machinery — an unwelcomed connection becomes indistinguishable from a dropped one, and the existing backoff self-heals); the connect timeout goes inside `webSocketConnect` (with a settled-guard that also stops pre-open `close` events from double-scheduling reconnects); boot-fetch bounds go in `api.ts`'s helpers with a small bounded-retry wrapper used by `App.svelte`'s `boot()`; the activation latch in `WorldSession` splits into add-once + activated-on-success flags so a thrown activation is re-attempted on the next Welcome.

**Tech Stack:** TypeScript only — `@shadowcat/core` (ws-client, transport) + `@shadowcat/shell` (api, App boot, WorldSession). Vitest; no server changes.

## Model/Effort directives

- Dispatcher: mainline — this session owns the SDD loop (recorded 2026-07-31, same ruling as the ui-state plan).
- Implementer: `shadowcat-coder` (sonnet, effort **medium**); escalation `shadowcat-coder-opus`.
- Per-task review: `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` (effort high); `-opus` twins on shallow/uncertain findings.
- Final whole-branch review: `shadowcat-spec-reviewer-opus` + `shadowcat-code-reviewer-opus`.

## Buddy-check directives

Standing rule: no task buddy-check-flagged; the two-reviewer pair is the checkpoint standard, `-opus` twins are the escalation lever.

## Global Constraints

- **Reviewers have NO shell/write access**: pre-generate review diffs to files; relay gate outputs verbatim; never edit/commit while a reviewer runs.
- **Timeout values follow the file's own convention**: `ws-client.ts` correlated requests use 10s — the welcome watchdog and connect timeout use 10s; HTTP fetch bounds use 15s (matches the chat error window's outer bound). Every new timeout is a named constant or a documented `opts` default, never an inline literal at the use site.
- **Vitest does not typecheck** — every task's verification includes the package's `typecheck` script.
- **No behavior change to the happy path**: a Welcome inside 10s, a connect inside 10s, and green fetches must behave byte-identically to today (no new states, no new UI).
- **No debug code**; comments present-tense; timers must be identity-guarded (a stale timer firing after its transport/attempt was superseded must no-op — capture the object reference at arm time, compare at fire time).
- **Local matrix replaces CI watch** (final task).
- Commit messages end with the standard Co-Authored-By / Claude-Session trailer used on this repo.

---

### Task 1: `@shadowcat/core` — Welcome watchdog in `WsClient` + connect timeout / settled-guard in `webSocketConnect`

**Files:**
- Modify: `src/client/core/src/ws-client.ts`
- Modify: `src/client/core/src/transport.ts`
- Test: `src/client/core/src/ws-client.test.ts` (existing file; add cases), `src/client/core/src/transport.test.ts` (create if absent)

**Interfaces:**
- Produces: `WsClientOptions.welcomeTimeoutMs?: number` (default `10_000`); `webSocketConnect(url: string, connectTimeoutMs = 10_000)`. No other public-surface change.

- [ ] **Step 1: Write the failing watchdog tests** in `ws-client.test.ts` (the file's existing mock-connect + injectable `sleep`/`now` patterns apply; use vi fake timers as the existing timeout tests do):

```ts
test("a connection that never receives Welcome is closed and reconnected after welcomeTimeoutMs", async () => {
  // mock connect that resolves a transport but never delivers a welcome frame;
  // assert: after advancing 10_000ms, the transport's close() was called and a
  // second connect attempt was made (reconnect machinery engaged).
});

test("a Welcome inside the window disarms the watchdog (no spurious close)", async () => {
  // deliver welcome at t=100ms; advance past 10_000ms; assert the transport
  // was never closed and no reconnect was scheduled.
});

test("the watchdog re-arms on reconnect and a stale timer from a superseded transport no-ops", async () => {
  // first connection times out (reconnect); second connection welcomes quickly;
  // advance well past both windows; assert exactly one close (the first) —
  // the first attempt's timer must not close the second transport.
});
```

- [ ] **Step 2: Run to verify they fail** — `pnpm --filter @shadowcat/core test` → the three new tests FAIL (no watchdog exists).

- [ ] **Step 3: Implement the watchdog** in `ws-client.ts`:

```ts
// In WsClientOptions:
/** Ms to wait for the server's Welcome after a transport opens before the
 * connection is treated as dead (closed → normal reconnect/backoff). The
 * browser's socket `open` fires at HTTP 101, BEFORE the server's Welcome
 * preamble, so "open but never welcomed" is otherwise an unbounded silent
 * wait no reconnect machinery can see. Same 10s convention as the
 * correlated-request timeouts below. */
welcomeTimeoutMs?: number;

// In WsClient:
private readonly welcomeTimeoutMs: number;         // ctor: opts.welcomeTimeoutMs ?? 10_000
private welcomeTimer: ReturnType<typeof setTimeout> | null = null;

private armWelcomeWatchdog(): void {
  this.clearWelcomeWatchdog();
  // Identity guard: close only the transport this timer was armed for — a
  // stale timer surviving into a successor connection must no-op.
  const armed = this.transport;
  this.welcomeTimer = setTimeout(() => {
    this.welcomeTimer = null;
    if (this.running_ && armed !== null && this.transport === armed) {
      // Treat as a dead link: close() fires the transport's onClose →
      // handleClose → failPending + scheduleReconnect. Self-healing.
      armed.close();
    }
  }, this.welcomeTimeoutMs);
}

private clearWelcomeWatchdog(): void {
  if (this.welcomeTimer !== null) {
    clearTimeout(this.welcomeTimer);
    this.welcomeTimer = null;
  }
}
```
Call sites: `open()` success path (right after `this.transport = ...; this.reconnectAttempt = 0;`) → `this.armWelcomeWatchdog()`; `handleFrame` `case "welcome":` first statement → `this.clearWelcomeWatchdog()`; `stop()` and `handleClose()` → `this.clearWelcomeWatchdog()`.

- [ ] **Step 4: Write the failing transport tests** in `transport.test.ts` with a stubbed `globalThis.WebSocket` class (record instances; expose `fireOpen()/fireError()/fireClose()/fireMessage(d)`; record `close()` calls):

```ts
test("webSocketConnect rejects and closes the socket after connectTimeoutMs with no open", async () => { /* fake timers; advance 10_000 */ });
test("a pre-open close/error does NOT invoke handlers.onClose (no double reconnect signal)", async () => { /* fireError then fireClose before open; assert onClose never called and the promise rejected once */ });
test("post-open close invokes handlers.onClose exactly once", async () => { /* fireOpen then fireClose */ });
```

- [ ] **Step 5: Run to verify they fail**, then **implement** in `transport.ts`:

```ts
/** A `Connect` backed by the platform global `WebSocket` (browser / Node 22+).
 * Cookies are sent automatically by the browser; Node test/integration code that
 * needs a cookie header supplies its own `Connect` instead. `connectTimeoutMs`
 * bounds the handshake: a TCP-accepted-but-never-upgraded socket otherwise
 * never settles this promise, and the caller's reconnect machinery is
 * unreachable behind the unsettled await. Handlers attach semantically AFTER
 * open: pre-open close/error only reject (they must not leak into onClose —
 * the caller's open() failure path already schedules the reconnect, and a
 * pre-open onClose would double-schedule it). */
export function webSocketConnect(url: string, connectTimeoutMs = 10_000): Connect {
  return (handlers) =>
    new Promise<Transport>((resolve, reject) => {
      const ws = new WebSocket(url);
      let opened = false;
      const timer = setTimeout(() => {
        if (!opened) {
          reject(new Error("websocket connect timeout"));
          ws.close();
        }
      }, connectTimeoutMs);
      ws.addEventListener("open", () => {
        opened = true;
        clearTimeout(timer);
        resolve({
          send: (data) => ws.send(data),
          close: () => ws.close(),
        });
      });
      ws.addEventListener("message", (ev: MessageEvent) => {
        handlers.onMessage(
          typeof ev.data === "string" ? ev.data : String(ev.data),
        );
      });
      ws.addEventListener("close", () => {
        clearTimeout(timer);
        if (opened) handlers.onClose();
      });
      ws.addEventListener("error", () => {
        if (!opened) {
          clearTimeout(timer);
          reject(new Error("websocket error"));
        }
        // Post-open errors are followed by `close`; onClose handles teardown.
      });
    });
}
```
(`message` needs no `opened` guard — frames cannot arrive pre-101.)

- [ ] **Step 6: Green + gates** — `pnpm --filter @shadowcat/core test && pnpm --filter @shadowcat/core typecheck && pnpm lint`.

- [ ] **Step 7: Commit** — `fix(core/ws): welcome watchdog + bounded connect handshake; pre-open close no longer leaks to onClose`

- [ ] **Step 8: Review pair** (diff file + this section + relayed gates).

---

### Task 2: `@shadowcat/shell` — bounded boot fetches with retry + activation latch split

**Files:**
- Modify: `src/client/shell/src/lib/api.ts`
- Modify: `src/client/shell/src/App.svelte`
- Modify: `src/client/shell/src/lib/worldSession.svelte.ts`
- Test: `src/client/shell/src/lib/api.test.ts`, `src/client/shell/src/lib/worldSession.test.ts`

**Interfaces:**
- Consumes: nothing new from Task 1 (shell already passes `webSocketConnect(wsUrl)`; the new parameter defaults).
- Produces: `api.ts` exports `withRetry<T>(fn: () => Promise<T>, attempts?: number, delays?: number[]): Promise<T>`; all fetch helpers gain a bounded `AbortSignal`. `WorldSession`'s public surface unchanged.

- [ ] **Step 1: Failing api tests** (`api.test.ts`, existing mockFetch patterns):

```ts
test("getJson passes a bounded AbortSignal to fetch", async () => {
  // mock fetch capturing init; assert init.signal is an AbortSignal.
});
test("withRetry retries the configured attempts then rethrows the last error", async () => {
  // fn rejects twice then resolves → resolves with 3 attempts (fake timers for delays);
  // fn always rejects → rejects after `attempts` calls, not more.
});
```

- [ ] **Step 2: Implement in `api.ts`:**

```ts
/** Bound on every session/boot fetch. A hung backend request otherwise pins the
 * SPA on its current route forever (the boot chain has no other timeout). */
const FETCH_TIMEOUT_MS = 15_000;
```
Add `signal: AbortSignal.timeout(FETCH_TIMEOUT_MS)` to the `fetch` init in `getJson`, `postJson`, `getMe`, and `putUiState` (spread alongside `keepalive` there). Add:

```ts
/** Bounded retry for the boot chain: transient backend blips (restart, single
 * 5xx) must not permanently strand the SPA on the login/worlds route — the
 * pre-fix behavior degraded on the FIRST failure with no retry and no error
 * surface. Delays are flat values, not a policy knob (YAGNI). */
export async function withRetry<T>(
  fn: () => Promise<T>,
  attempts = 3,
  delays: number[] = [500, 1500],
): Promise<T> {
  let lastErr: unknown;
  for (let i = 0; i < attempts; i++) {
    try {
      return await fn();
    } catch (e) {
      lastErr = e;
      if (i < attempts - 1) await new Promise((r) => setTimeout(r, delays[Math.min(i, delays.length - 1)]));
    }
  }
  throw lastErr;
}
```

- [ ] **Step 3: Use it in `App.svelte`'s `boot()`** — wrap the three boot awaits, nothing else changes (the degrade branches stay):

```ts
me = await withRetry(() => getMe());
// ...
const ui = await withRetry(() => loadSessionState()); // applies the saved locale
// ...
const worlds = await withRetry(() => listWorlds());
```
(`onAuthenticated` stays un-retried: it is user-interactive; the user can re-click.)

- [ ] **Step 4: Failing worldSession test** (`worldSession.test.ts`, injectable `logger` + mock connect already used there): construct a session whose module set makes `#modules.activate()` THROW on the first Welcome (two test modules with mutually-circular `requires`/`provides` contract declarations — `topoSort` throws on the cycle), then deliver a second Welcome:

```ts
test("a failed first activation is re-attempted on the next Welcome (no permanent latch)", async () => {
  // logger spy: "world session welcome handling failed" logged on BOTH welcomes.
  // Pre-fix behavior: second Welcome short-circuits the #bootstrapped guard and
  // logs nothing — the failure is latched for the session's life.
});
```

- [ ] **Step 5: Implement the latch split** in `worldSession.svelte.ts` — replace `#bootstrapped` (and update its doc comment):

```ts
/** In-world bootstrap guards. Modules are ADDED exactly once per session
 * (re-adding would duplicate registrations), but `#activated` latches only on
 * a SUCCESSFUL activation — a thrown activation (e.g. a contract cycle) is
 * re-attempted on the next Welcome instead of being cached for the session's
 * life with every Surface silently empty. `ModuleRegistry.activate` is
 * incremental (activates only not-yet-active modules), so a retry never
 * double-activates. */
#modulesAdded = false;
#activated = false;
```
In `#onWelcome`, replace the `if (!this.#bootstrapped) { ... }` block:

```ts
if (!this.#activated) {
  if (!this.#modulesAdded) {
    this.#modulesAdded = true;
    for (const m of this.opts.modules) this.#modules.add(m);
  }
  await this.#modules.activate();
  this.#activated = true;
  await this.#loadExternalModules(w.world, w.server_version);
}
```
(`#loadExternalModules` keeps its once-per-session semantics: it now runs after the FIRST SUCCESSFUL activation — its own doc comment's "exactly once" contract is preserved because `#activated` latches immediately before it.)

- [ ] **Step 6: Green + gates** — `pnpm --filter @shadowcat/shell test && pnpm --filter @shadowcat/shell typecheck && pnpm lint && pnpm -r test` (the core package's contract tests must still pass against the shell's usage).

- [ ] **Step 7: Commit** — `fix(shell): bounded+retried boot fetches; activation failure no longer latches`

- [ ] **Step 8: Review pair.**

---

### Task 3: Verification matrix + documentation sync

**Files:**
- Modify: `docs/OPEN_BUGS.md` (remove the four "Silent-hang startup paths" entries; back to `_No open bugs._`)
- Modify: `docs/CLOSED_BUGS.md` (one new section recording all four resolutions with commit refs)
- Modify: `.claude/skills/shadowcat-codebase-realtime-sync/SKILL.md` (ws-client seam: welcome watchdog + connect timeout are now part of the reconnect contract)
- Modify: `.claude/skills/shadowcat-codebase-client-shell/SKILL.md` (boot seam: bounded/retried fetches; WorldSession activation-latch semantics)

**Steps:**

- [ ] **Step 1: Full local matrix** — `pnpm build`; from `src/server/`: `cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings` (no server diff — this pins nothing regressed via generated types); `pnpm -r typecheck && pnpm -r test && pnpm lint`; `pnpm --filter @shadowcat/core test:e2e`; full `pnpm --filter @shadowcat/shell e2e` (budgets now honest: 120s test / 15s expect). A ui-state/persistence or welcome-path assert failure is a REGRESSION and blocks; adjudicate any other single flake per the POST_WORK_FINDINGS isolated-re-run protocol.
- [ ] **Step 2: Doc sync** per Files, following each file's existing conventions; the two skill edits go through the reviewed skill-update gate (spec reviewer verifies them against the shipped code).
- [ ] **Step 3: Commit** — `docs: close the silent-hang startup bugs; sync realtime-sync + client-shell skills`
- [ ] **Step 4: Review pair** (spec reviewer explicitly covers the skill-update gate).

---

### Task 4: Regression fix — escalating Welcome-watchdog window

Added 2026-07-31 after Task 3's reviews. Dispatcher evidence: on a CALM machine (0% load), 3/3 full-suite runs fail `panels.spec.ts:18` at a `toHaveAttribute` (render-ready) wait at ~32-41s — deterministic in the 6-worker suite, green in isolation, and the same suite was 16/16 pre-branch. Mechanism hypothesis: the fixed 10s Welcome watchdog kills connections whose server preamble is merely SLOW under parallel-startup contention (single DB connection + per-connect fs scan); each kill discards queue position and re-enters the preamble from the back — the watchdog converts slow-but-progressing into never-completes. The plan's fixed-window watchdog spec was the defect; this task supersedes it.

**Files:**
- Modify: `src/client/core/src/ws-client.ts` (+ its test file)
- Modify: `.claude/skills/shadowcat-codebase-realtime-sync/SKILL.md` (watchdog contract: escalating window)
- Modify: `docs/CLOSED_BUGS.md` (watchdog bug entry gains the escalation note + this task's commit)
- Modify: `docs/POST_WORK_FINDINGS.md` (entry: the Task-3 gate-run panels failure was THIS regression, discovered by the review pair's blocking + dispatcher 3x calm-machine runs; corrects the task report's contention adjudication and its misquoted blocking criterion)

**Step 1 — falsify the hypothesis first (systematic-debugging):** on the current HEAD, temporarily set the `welcomeTimeoutMs` default to `60_000`, rebuild (`pnpm build` + `pnpm --filter @shadowcat/shell e2e:build` as needed), run the full shell e2e once. Expected: 16/16 (confirms the watchdog is the mechanism). Revert the temporary value. If the suite still fails, STOP — report BLOCKED with the output (the mechanism is something else in this branch; do not proceed to Step 2).

**Step 2 — failing test:** in `ws-client.test.ts`, add: "consecutive unwelcomed connections escalate the watchdog window and a Welcome resets it" — connection 1 unwelcomed closes at the base window; connection 2's window is 2x base (assert no close occurs between 1x and just-under-2x, then close at 2x); after a connection that DOES receive Welcome, the next connection's window is back to 1x base.

**Step 3 — implement:** in `WsClient`, track `private consecutiveUnwelcomed = 0`. `armWelcomeWatchdog()` computes `const window = this.welcomeTimeoutMs * 2 ** Math.min(this.consecutiveUnwelcomed, 3);` (cap 8x = 80s at the default base). The watchdog's fire path increments `consecutiveUnwelcomed` before closing; `handleFrame`'s welcome case resets it to 0 (alongside the existing clear). `stop()` also resets it. Update `welcomeTimeoutMs`'s doc comment: it is the BASE window; consecutive unwelcomed connections double it (cap 8x) so a slow-but-progressing server preamble under load is tolerated instead of amplified (each kill discards the connection's queue position server-side), while a truly hung link still self-heals at the base window on the first occurrence.

**Step 4 — gates:** `pnpm --filter @shadowcat/core test && pnpm --filter @shadowcat/core typecheck && pnpm lint && pnpm -r test`; rebuild; then the regression gate: full `pnpm --filter @shadowcat/shell e2e` THREE times — all 3 must be 16/16 (the pre-branch suite achieved 16/16 in ~42s on a calm machine; match it).

**Step 5 — docs** per Files (the POST_WORK_FINDINGS entry names the real criterion from Task 3's brief and states the corrected adjudication), **Step 6 — commit** (`fix(core/ws): welcome watchdog escalates its window under consecutive unwelcomed connections`), **Step 7 — review pair**.

## Final Review

Whole-branch review (merge-base `main`..HEAD) by the `-opus` twins with the full-branch package, this plan, and the ledger's deferred minors. Then `--ff-only` merge to main, push, delete branch.
