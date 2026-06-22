# TODO — Deferred Work

Actionable, externally-logged deferrals. Bugs go in `OPEN_BUGS.md`, not here.

## Server / auth
- TODO: Periodically sweep expired rows from the `tower_sessions` table. Expired rows can never load (the store filters `expiry_date > now`), so this is housekeeping, not correctness — wire a sweep when session volume grows.

## Server / assets
- TODO: Rate-limit the asset replace endpoint (`POST /assets/{uuid}/replace`). It streams a full new file like upload but is currently only GM-gated + magic-byte validated (per the M8b spec, which scopes the per-user rate limit to upload only). A GM can replace-loop a near-cap file unbounded. Update the M8b spec to extend the tiered rate limit to replace, then add the same `upload_rate.check`/`refund` guard the upload handler uses. (Surfaced by the M8b-1 buddy check; spec-compliant today, hardening deferred.)

## Data layer
- TODO: Purge `explored_fog` rows on world/scene/user deletion. The M9c table denormalizes `world_id` for a world-scoped purge, but no deletion path consumes it yet (worlds aren't deletable; scene deletion goes through the `apply_intent` document cascade, which doesn't touch `explored_fog`). Orphaned rows are harmless (reads key on the exact never-reused `(scene_id, user_id)` UUIDs) but accumulate unboundedly over a server's lifetime. Wire a `DELETE FROM explored_fog WHERE world_id = ?` (and a per-scene purge into the scene-delete cascade) when world/scene deletion lands; index `world_id` then. (Surfaced by the M9c-1 buddy check.)
- TODO: `command::set_pointer` is set-only — an Update that conceptually removes a key writes `null` (key stays present as null) rather than removing it. `null` ≠ absent. Resolve removal semantics when the merge engine lands.

## Client / UI
- TODO: Extend `reconcileTopology` beyond presence-by-`module_id` to flag version and `provides`/`requires` mismatches for modules present on both sides (a stale local build providing a contract the world no longer declares currently reconciles silently). Land with module management / hard topology enforcement.
- TODO: Resolve multi-provider conflict policy for `singleton` surface contracts in the UI contribution architecture — when two modules provide the same `singleton` contract (e.g. both claim "the sidebar"), decide the winner (load order, explicit priority, or user selection) instead of the current deterministic loud-fail. Design once a real second provider exists to validate the semantics; the contract model already carries the `singleton`/`multi` cardinality marker the policy slots into.
- TODO: Add capability version negotiation to contract-based module dependencies (`requires`) — match a required contract against a provider by version range, not presence alone. Deferred until multiple providers of a contract exist at differing versions.

## Client / intents
- TODO: Replay (or visibly block) optimistic intents issued while the world socket is disconnected. `WorldSession.dispatchIntent` drops a dispatch when `WsClient` has no transport (logged), avoiding an orphaned pending entry that would mis-correlate the FIFO confirm of the next echo; a reconnect does not replay the dropped action. Add a replay-on-resync queue (or a "reconnecting" UI block) when offline editing matters.

## Server / ws
- TODO: Make the ping rate limit per-user (on `AppState`) instead of per-connection. `conn.rs`'s `ScenePing` limiter is a per-connection sliding window (30/min) that resets on reconnect, so a user with N concurrent sockets gets N×30/min — a weaker abuse backstop than the per-user `UploadRateLimiter`. Accepted as a defensible choice for a transient cosmetic ping (membership-gated, silent drop, best-effort relay); upgrade to a per-user `PingRateLimiter` on `AppState` if ping abuse becomes a concern. (Deviation from the M8d-3b plan Task 4, surfaced by the buddy check.)

## Client / render
- TODO: Lerp token rotation along the shortest signed delta (`((b-a+540)%360)-180`) with a wrap-aware ε-settle, when M8d-2 adds rotation control. M8d-1's `TokenAnimator` lerps rotation as a raw scalar (350°→10° tweens the long way); cannot manifest until rotation is authorable. (Surfaced by the M8d-1 buddy check.)
- TODO: Select the active scene deterministically once multiple scene documents can exist (M8d scene authoring). `SceneReconciler` renders `store.query("scene")[0]` (insertion-order); with >1 scene this picks an arbitrary background. Add explicit active-scene selection with M8d. (Surfaced by the M8c-1 buddy check.)
- TODO: Add browser e2e asserting the scene **background** renders (Scene `system.background` → sprite). Blocked on a UI to set `scene.system.background` — scene management (background/dimensions/switching) is M12; M8d-2 has no authoring path for the background. The **token** render e2e is done (M8d-2 `stage.spec.ts` "place a token via the tool rail, then drag it" asserts `data-token-count`). Add the background assertion when M12 scene management lands.
- TODO: Give a wall-less scene full intrascene vision instead of the degenerate viewpoint-bound box. M9b's `player_vision_polygons` bounds a wall-less scene to a viewpoint±margin box (leak-safe under-reveal, but a player in an open scene sees only a small square). A payload-level `mode:"all"` shortcut is NOT viable — it clears fog globally and would reveal a *different* walled active scene (cross-scene leak). The fix needs a per-scene vision mode (or a scene-extent so the wall-less polygon can cover the whole scene), which lands with M9c persistent fog / M12 multi-scene. (Surfaced by the M9b buddy check.)
- TODO: Switch M9b's vision active-scene filter from `store.query("scene")[0]` to the real active-scene selector when it lands (same M12 dependency as the active-scene-selection TODO above). `engine.toVisibility` filters per-token vision polygons by the active scene id to prevent a cross-scene fog hole; today the active scene is `query("scene")[0]` (insertion-order, correct for single-scene M8d). The moment >1 scene + scene-switching exists, this filter must follow the chosen active scene or it cuts the wrong (or no) holes. (Surfaced by the M9b buddy check.)
