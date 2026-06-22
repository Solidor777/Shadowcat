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

## Client / intents
- TODO: Replay (or visibly block) optimistic intents issued while the world socket is disconnected. `WorldSession.dispatchIntent` drops a dispatch when `WsClient` has no transport (logged), avoiding an orphaned pending entry that would mis-correlate the FIFO confirm of the next echo; a reconnect does not replay the dropped action. Add a replay-on-resync queue (or a "reconnecting" UI block) when offline editing matters.

## Client / render
- TODO: Lerp token rotation along the shortest signed delta (`((b-a+540)%360)-180`) with a wrap-aware ε-settle, when M8d-2 adds rotation control. M8d-1's `TokenAnimator` lerps rotation as a raw scalar (350°→10° tweens the long way); cannot manifest until rotation is authorable. (Surfaced by the M8d-1 buddy check.)
- TODO: Select the active scene deterministically once multiple scene documents can exist (M8d scene authoring). `SceneReconciler` renders `store.query("scene")[0]` (insertion-order); with >1 scene this picks an arbitrary background. Add explicit active-scene selection with M8d. (Surfaced by the M8c-1 buddy check.)
- TODO: Add browser e2e asserting the scene **background** renders (Scene `system.background` → sprite). Blocked on a UI to set `scene.system.background` — scene management (background/dimensions/switching) is M12; M8d-2 has no authoring path for the background. The **token** render e2e is done (M8d-2 `stage.spec.ts` "place a token via the tool rail, then drag it" asserts `data-token-count`). Add the background assertion when M12 scene management lands.
