# TODO — Deferred Work

Actionable, externally-logged deferrals. Bugs go in `OPEN_BUGS.md`, not here.

## Client core
- TODO: Wrap consumer-handler dispatch in `WsClient.handleFrame` in a try/catch so a throwing handler (`onCommand`/`onReject`/`onWelcome`/`onUpdate`) cannot break the socket message pump. Currently a throwing callback propagates out through the transport `onMessage`. (M6c-2 final-review minor; pre-existing exposure, surfaced by the consumer-supplied `subscribeSearch` `onUpdate`.)

## Server / auth
- TODO: Offload Argon2 hashing/verification to a blocking thread (e.g. `tokio::task::spawn_blocking`) on the login and setup paths. Each verify burns ~tens of ms of CPU on an async worker; acceptable at current traffic, revisit before the server handles concurrent logins at scale.
- TODO: Periodically sweep expired rows from the `tower_sessions` table. Expired rows can never load (the store filters `expiry_date > now`), so this is housekeeping, not correctness — wire a sweep when session volume grows.

## Data layer
- TODO: `command::set_pointer` is set-only — an Update that conceptually removes a key writes `null` (key stays present as null) rather than removing it. `null` ≠ absent. Resolve removal semantics when the merge engine lands.
