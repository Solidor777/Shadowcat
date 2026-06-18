# TODO — Deferred Work

Actionable, externally-logged deferrals. Bugs go in `OPEN_BUGS.md`, not here.

## Tooling
- TODO: Extend the ESLint gate to cover TypeScript. The M1 flat config (`eslint.config.js`) registers only `@eslint/js`, with no `typescript-eslint` parser and no `files` glob, so every `.ts` source is skipped and `pnpm lint` can pass green with lint errors present in TypeScript. Now actionable: M6a landed real client logic (`@shadowcat/core`). Add `typescript-eslint` and a `files: ["**/*.ts"]` block (scope-aware so the Svelte `ui` package keeps building).
- TODO: Node↔Rust client/server end-to-end test. M6a's WS-client integration tests run against a TypeScript protocol mock (the `web` CI job has no Rust toolchain). Add a harness — local and/or a combined CI job — that builds the Rust `test_server` and drives the real `@shadowcat/core` WS client against it, asserting convergence against the authoritative `world_events` log.

## Server / auth
- TODO: Offload Argon2 hashing/verification to a blocking thread (e.g. `tokio::task::spawn_blocking`) on the login and setup paths. Each verify burns ~tens of ms of CPU on an async worker; acceptable at current traffic, revisit before the server handles concurrent logins at scale.
- TODO: Periodically sweep expired rows from the `tower_sessions` table. Expired rows can never load (the store filters `expiry_date > now`), so this is housekeeping, not correctness — wire a sweep when session volume grows.

## Data layer
- TODO: `command::set_pointer` is set-only — an Update that conceptually removes a key writes `null` (key stays present as null) rather than removing it. `null` ≠ absent. Resolve removal semantics when the merge engine lands.
