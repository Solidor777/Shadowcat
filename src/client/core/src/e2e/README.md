# Node↔Rust end-to-end tests

These suites (`*.e2e.test.ts`) drive the real `@shadowcat/core` client against
the real Rust `test_server` over a WebSocket, asserting behavior end to end
across the runtime boundary.

## Requirements

- The **Rust toolchain** (`cargo`) — the harness spawns
  `cargo run -p shadowcat --bin test_server`.
- Node 22+ (for `fetch` / `getSetCookie`).

## Running

```sh
pnpm --filter @shadowcat/core test:e2e
```

The default unit run (`pnpm --filter @shadowcat/core test`, and the repo-wide
`pnpm -r test`) **excludes** these via `vitest.config.ts`, so the no-Rust `web`
CI job stays green. The dedicated `e2e` CI job (Rust + Node) runs them; it
pre-builds the server so the in-test spawn does not pay the compile cost.

## What the harness provides

- `startTestServer(opts?)` — spawns the server, parses its `test_server:` address and
  `e2e-fixture:` JSON (world/doc/gm/player ids), returns `{ baseUrl, wsUrl,
  fixture, stop }`. `opts.modulesDir` (optional) passes `--modules-dir <path>`
  to the spawned binary, for tests exercising the installed-module pipeline
  (see `modules.e2e.test.ts`).
- `login(baseUrl, user, pw)` — `POST /api/login`, returns the session cookie.
