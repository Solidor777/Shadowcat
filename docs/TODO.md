# TODO — Deferred Work

Actionable, externally-logged deferrals. Bugs go in `OPEN_BUGS.md`, not here.

## Tooling
- TODO: Extend the ESLint gate to cover TypeScript. The M1 flat config (`eslint.config.js`) registers only `@eslint/js`, with no `typescript-eslint` parser and no `files` glob, so every `.ts` source is skipped and `pnpm lint` can pass green with lint errors present in TypeScript. Now actionable: M6a/M6b landed real client logic (`@shadowcat/core`). Add `typescript-eslint` and a `files: ["**/*.ts"]` block (scope-aware so the Svelte `ui` package keeps building).

## Client core
- TODO: `WsClient.search` issued while the transport is disconnected currently waits out `timeoutMs` (the `send` is a no-op, so no frame is ever answered). Fast-fail with an immediate rejection when there is no live transport, taking care not to reject a search issued in the brief window during initial `connect`. (M6c-1 final-review minor; in-flight searches are already drained on disconnect/stop.)

## Server / auth
- TODO: Offload Argon2 hashing/verification to a blocking thread (e.g. `tokio::task::spawn_blocking`) on the login and setup paths. Each verify burns ~tens of ms of CPU on an async worker; acceptable at current traffic, revisit before the server handles concurrent logins at scale.
- TODO: Periodically sweep expired rows from the `tower_sessions` table. Expired rows can never load (the store filters `expiry_date > now`), so this is housekeeping, not correctness — wire a sweep when session volume grows.

## Data layer
- TODO: `command::set_pointer` is set-only — an Update that conceptually removes a key writes `null` (key stays present as null) rather than removing it. `null` ≠ absent. Resolve removal semantics when the merge engine lands.
