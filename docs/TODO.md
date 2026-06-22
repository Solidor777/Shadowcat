# TODO — Deferred Work

Actionable, externally-logged deferrals. Bugs go in `OPEN_BUGS.md`, not here.

## Server / auth
- TODO: Periodically sweep expired rows from the `tower_sessions` table. Expired rows can never load (the store filters `expiry_date > now`), so this is housekeeping, not correctness — wire a sweep when session volume grows.

## Server / assets
- TODO: Rate-limit the asset replace endpoint (`POST /assets/{uuid}/replace`). It streams a full new file like upload but is currently only GM-gated + magic-byte validated (per the M8b spec, which scopes the per-user rate limit to upload only). A GM can replace-loop a near-cap file unbounded. Update the M8b spec to extend the tiered rate limit to replace, then add the same `upload_rate.check`/`refund` guard the upload handler uses. (Surfaced by the M8b-1 buddy check; spec-compliant today, hardening deferred.)

## Data layer
- TODO: `command::set_pointer` is set-only — an Update that conceptually removes a key writes `null` (key stays present as null) rather than removing it. `null` ≠ absent. Resolve removal semantics when the merge engine lands.

## Client / UI
- TODO: Extend `reconcileTopology` beyond presence-by-`module_id` to flag version and `provides`/`requires` mismatches for modules present on both sides (a stale local build providing a contract the world no longer declares currently reconciles silently). Land with module management / hard topology enforcement.
- TODO: Resolve multi-provider conflict policy for `singleton` surface contracts in the UI contribution architecture — when two modules provide the same `singleton` contract (e.g. both claim "the sidebar"), decide the winner (load order, explicit priority, or user selection) instead of the current deterministic loud-fail. Design once a real second provider exists to validate the semantics; the contract model already carries the `singleton`/`multi` cardinality marker the policy slots into.
- TODO: Add capability version negotiation to contract-based module dependencies (`requires`) — match a required contract against a provider by version range, not presence alone. Deferred until multiple providers of a contract exist at differing versions.

## Client / render
- TODO: Add a browser e2e asserting the scene **background** renders (Scene `system.background` → sprite). M8c-1's `SceneReconciler` is unit-tested against a mock backend, but the live render assertion needs a Scene document with a background, which needs the scene-authoring UI that lands in M8d. Add the assertion to the Playwright stage smoke once scenes can be authored. (M8c design spec §9 lists "background renders" in the c-1 smoke; deferred to M8d per the user-approved decomposition, not a descope.)
