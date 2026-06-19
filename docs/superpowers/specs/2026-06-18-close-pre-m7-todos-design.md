# Close Pre-M7 TODOs — Design

Closes four actionable TODOs before the M7 milestone. `OPEN_BUGS.md` is empty;
these are the only outstanding items. Two TODOs (#5 session sweep, #6
`set_pointer` removal semantics) remain documented deferrals — see "Deferrals".

## Scope

| # | Area | Item |
|---|------|------|
| 1 | Tooling | ESLint TypeScript coverage |
| 2 | Client core | Fast-fail `search`/`subscribeSearch` when disconnected |
| 3 | Client core | Isolate throwing consumer handlers in `handleFrame` |
| 4 | Server/auth | Offload Argon2 hash/verify to `spawn_blocking` |

## #1 — ESLint TypeScript coverage

`eslint.config.js` loads only `@eslint/js` recommended (espree parser); `.ts`
sources are skipped, so `pnpm lint` passes green with TypeScript lint errors
present in the M6a/b/c client code (`@shadowcat/core`, `@shadowcat/types`).

**Change:** add `typescript-eslint` dev dependency; append the non-type-checked
`tseslint.configs.recommended` preset (applies to `**/*.ts`). Preserve the
existing `ignores` (`**/*.svelte`, `src/types/generated/`) so the Svelte `ui`
package and ts-rs–generated types stay excluded. Non-type-checked preset chosen
to avoid per-package `parserOptions.project` plumbing; type-checked rules are a
later opt-in.

**Risk:** enabling the parser surfaces pre-existing violations in
`src/client/core/src/*.ts` and `src/types/*.ts`. Fix violations inline. Surface
(do not mass-disable) any rule that is genuinely wrong for this codebase or an
unexpectedly large volume.

## #2 — Fast-fail disconnected search

`search` and `subscribeSearch` call `this.send()`, a no-op when
`transport === null`; the returned promise then waits out `timeoutMs`.

**Change:** synchronous guard at the top of each method — if `!this.transport`,
reject immediately with `Error("not connected")`; in `subscribeSearch`, do not
register the subscription. Correct in every null state: pre-start, post-`stop`,
and mid-reconnect-backoff are all states the current socket will never answer,
and reconnect does not replay search frames. `start()` awaits `open()`, so a
started client has a non-null transport — no "connecting window" false reject.

## #3 — Isolate throwing consumer handlers

`onCommand` is already guarded (`applyEvent` → `onError`). `onWelcome`,
`onReject`, and `onUpdate` (the `search_update` case and the synchronous initial
`onUpdate(page.hits)` inside `subscribeSearch`'s resolve wrapper) run unguarded
inside the message pump; a throw propagates through the transport `onMessage`
and breaks the socket.

**Change:** add a private `safeEmit(fn)` that runs the consumer callback in
try/catch and routes a throw to `onError`, swallowing a throw from `onError`
itself so the pump cannot die. Wrap exactly those four consumer-callback sites.
Protocol logic (resync sends, `nextExpected`/`serverOffsetMs` updates) stays
outside the guard, so a throwing handler never desyncs ordering.

## #4 — Offload Argon2 to spawn_blocking

`verify_password` (login, `routes.rs`) and `hash_password` (admin create,
`setup.rs`) run ~tens-of-ms CPU on the async worker. `anti_enumeration_phc()`
also hashes but is one-time, sync, and `OnceLock`-bound — leave it.

**Change:** keep sync `hash_password`/`verify_password` as the pure CPU
primitives (tests and the sync `OnceLock` need them). Add `async` wrappers
taking owned `String`s, running the primitive in `tokio::task::spawn_blocking`:

- `verify_password_async` → `unwrap_or(false)` on `JoinError` (panic ⇒ failed
  auth, the safe default).
- `hash_password_async` → propagate `JoinError` as the existing `Internal`
  error.

Login uses `verify_password_async`; `create_admin` uses `hash_password_async`.

## Deferrals (documented, not closed)

- **#5 session sweep** — premature ("when session volume grows"); stays in
  `TODO.md`.
- **#6 `set_pointer` removal semantics** — blocked on the unbuilt merge engine;
  stays in `TODO.md`.

## Sequence & verification

1. **#1** first — establish the gate, fix existing violations.
2. **#2, #3** — land clean under the new linter.
3. **#4** — Rust, independent.

Tests: a disconnected-reject test (#2), a throwing-handler-does-not-break-pump
test (#3), existing Argon2 tests still green through the async path (#4). Full
`pnpm lint` + `cargo test` + the e2e search suites green before each commit.
One commit per item; remove each closed item from `TODO.md` in its commit.
