# Pre-M10 Cleanup — Design

**Date:** 2026-06-22
**Goal:** Close every fixable bug / follow-up surfaced through M9 before starting
M10. Drains `POST_WORK_FINDINGS.md` (the `Needs triage` entries) and `TODO.md`
of everything that does **not** depend on unbuilt infra. Items blocked on the
merge engine, module management, M12 multi-scene/active-scene, rotation
authoring, or world/scene deletion stay deferred and are explicitly out of scope.

No deployments and no users exist yet, so there is **no migration or
backward-compatibility constraint** on any data shape changed here.

## Scope — 11 items

Sequenced so dependencies land first: **#10 → #9** (create gate builds on the
type-scoped resolver), the rest independent.

Two items are security/secrecy-touching and are **buddy-checked** before merge:
**#3** (embedded redaction) and **#11** (offline replay / convergence).

---

### #1 — by-id document routes leak existence (403 vs 404)

**Problem.** `GET/PATCH/DELETE /api/documents/{id}` resolve the document's world
and call `permission_context()` (`sqlite.rs:317`), which returns
`DataError::Forbidden` → `AppError::Forbidden` → **403** for a non-member —
distinguishable from the **404** a nonexistent id returns. The *in-world but
unreadable* case already collapses to 404 (`get_document` `routes.rs:434`). Only
the non-member case leaks. Document UUIDs are unguessable, so impact is low; the
fix is a uniform authz surface.

**Fix.** On the three **by-id** routes only, map the non-member `Forbidden` to
`NotFound`:
- `get_document` (`routes.rs:417`): `.map_err` the `permission_context` call so
  `DataError::Forbidden → AppError::NotFound`.
- `patch_document` (`routes.rs:445`) / `delete_document` (`routes.rs:469`): these
  resolve membership inside `write_ops`. Resolve `permission_context` explicitly
  before `write_ops` and map `Forbidden → NotFound` (or thread a "by-id" flag into
  `write_ops` that performs the same mapping). The plan picks the lower-churn
  mechanism.

**Unchanged.** World-scoped routes (`list_documents` `routes.rs:393`,
`list_members` `routes.rs:332`, asset `serve`/`replace`) keep returning 403 for a
non-member — the world id is known/supplied there, so 403 leaks nothing and is the
correct membership-denial response.

**Tests.** Non-member receives 404 on GET/PATCH/DELETE by-id; a member-without-read
still 404s (regression); a world-scoped non-member route still 403s (guard against
over-broad mapping).

---

### #2 — `validate_system_size` ignores embedded children

**Problem.** `validate_system_size` (`data/validation.rs:8`) measures only
`doc.system`. Embedded children are stored inline in the parent JSON, so a
Create/Update with a large `embedded` tree bypasses the 256 KiB
(`MAX_SYSTEM_BYTES`) opaque-body cap. Bounded in practice by axum's ~2 MB JSON
limit and the WS frame cap, so not unbounded — but unenforced.

**Fix.** Recurse the same per-body cap into every embedded descendant's `system`:
walk `doc.embedded` (and their `embedded`, depth-bounded by the existing
self-FK/visited-set discipline used elsewhere), calling the size check on each
child's `system`. Each individual body must be ≤ `MAX_SYSTEM_BYTES`.

**Tests.** Oversized embedded child rejected with `TooLarge`; small embedded tree
passes; deeply nested oversized grandchild rejected.

---

### #3 — embedded children's `GmOnly` overrides not redacted (security; **buddy-check**)

**Problem.** `filter_properties` (`data/permission.rs:201`) strips only the parent
document's `property_overrides`. An embedded child carrying its own
`property_overrides: {"/system/x": "gm_only"}` is delivered to players
**unredacted**.

**Fix.** `filter_properties` recurses into `doc.embedded`: for each child, apply
the child's own `property_overrides` to the child's body (same `strip_pointer`
subtree semantics), then reassemble. Because both the Create and Delete arms of
`filter_command` (`permission.rs:241`, `:249`) call `filter_properties`, and the
REST `get_document`/`list_documents` paths call it too, this single change covers
egress and REST reads.

**Related sub-case (flag for the buddy-check, scope TBD by reviewer).** The
`Update` arm of `filter_command` (`permission.rs:258`) redacts changes using only
the **parent** document's `gm_only` set (`redact_change`). An `Update` whose path
writes into `/embedded/<child>/system/...` is redacted against parent overrides,
not the child's. The finding is scoped to `filter_properties`; whether the Update
path must also honor embedded-child overrides is a buddy-check decision. If in
scope, extend the `Update` arm to resolve the target embedded child's overrides
for `/embedded/...` paths.

**Tests.** Player view of a parent with a GM-only embedded child property omits
that property (via `filter_properties` directly, and via a Create broadcast through
`filter_command`); GM sees it; nested-grandchild override honored.

---

### #4 — no protection against removing/demoting the last GM

**Problem.** `set_role` (`sqlite.rs:223`) and `remove_member` (`sqlite.rs:242`)
let a world's only GM be demoted or removed, leaving the world manageable only by a
server admin. Availability footgun, not a security defect.

**Fix.** Before mutating, count current GMs
(`SELECT COUNT(*) FROM world_members WHERE world_id=? AND role='Gm'`, role encoded
as the stored JSON string, consistent with existing binds). Reject with
`DataError::Conflict(..)` → `AppError::Conflict` → **409** when:
- `remove_member`: target is currently GM **and** GM count == 1.
- `set_role`: new role ≠ GM **and** target is currently GM **and** GM count == 1.

Admin-recovery remains the escape hatch (server admins are GM on every world via
`permission_context` `sqlite.rs:324`), so a world is never permanently orphaned;
the guard only stops the accidental self-lockout. Reuses the existing
`Conflict(String)` variant — no new error type.

**Tests.** Demote/remove the sole GM → 409; with two GMs either succeeds; removing
a non-GM member is unaffected.

---

### #5 — asset `replace` endpoint not rate-limited

**Problem.** `replace` (`assets.rs:263`) streams a full new file like `upload`
but is only GM-gated + magic-byte validated. The per-user tiered
`UploadRateLimiter` guards `upload` only (`assets.rs:168`), so a GM can
replace-loop a near-cap file unbounded.

**Fix.** Apply the identical guard `upload` uses: after `require_gm`, capture
`now`, `state.upload_rate.check(user.id, now, effective_rate_per_min(role))` →
`TooManyRequests` on reject; wrap the fallible work (stream + DB + rename) so any
error path calls `state.upload_rate.refund(user.id, now)`. The limiter is per-user
across upload **and** replace (shared `upload_rate`), which is the intended cap on
total write volume. Update the M8b spec note (`TODO.md` + the M8b plan's
rate-limit scope line) to record that the tier now covers replace.

**Tests.** Replace-loop past the tier cap → 429; a failed replace refunds the hit
(next replace allowed).

---

### #6 — ping rate limit is per-connection, not per-user

**Problem.** `conn.rs`'s `ScenePing` limiter (`conn.rs:229`) is a per-connection
sliding window (30/min) that resets on reconnect, so a user with N sockets gets
N×30/min — weaker than the per-user `UploadRateLimiter`.

**Fix.** Add a per-user `PingRateLimiter` on `AppState` mirroring
`UploadRateLimiter` (`Mutex<HashMap<Uuid, Vec<i64>>>`, `check`/window-prune; no
`refund` needed — a ping is fire-and-forget). The `conn.rs` `ScenePing` arm calls
`state.ping_rate.check(user_id, now, 30)` instead of the local `ping_times`
window; over-budget pings are silently dropped as today. Pings remain
membership-gated and best-effort.

**Tests.** A user's ping budget is shared across two connections (the per-user cap
holds after a simulated reconnect); under budget relays.

---

### #7 — `slow_reader_recovers_via_resync` doesn't deterministically hit `Lagged`

**Problem.** The test (`tests/ws_convergence.rs:327`) floods 400 events to a
non-reading client to force a broadcast `Lagged` → resync, but the OS TCP buffer
may absorb all frames so egress never lags. It still asserts convergence (valid),
but is not a reliable regression guard for the lag path specifically.
`BROADCAST_CAPACITY=256` (`ws/room.rs:22`) is a private const with no test
override. `RoomStats.lagged_drops` (`room.rs:80`) is already incremented on
`RecvError::Lagged` (`conn.rs:621`).

**Fix (determinism via injectable capacity + metric assertion).**
- Thread broadcast capacity as a `Room::new` parameter (or a field on the WS
  config) defaulting to 256 — production unchanged.
- The test constructs a room/harness with a **tiny** capacity (e.g. 8) and floods
  past it against a socket the client provably does not read during the flood,
  guaranteeing the egress task stalls on the full TCP buffer and the broadcast ring
  overflows.
- Add a harness accessor for `lagged_drops` (mirrors `authoritative_seqs()`,
  in-process — no admin HTTP call) and assert `lagged_drops > 0`, then assert
  convergence as today.

This makes the Lagged path fire deterministically while keeping the convergence
assertion. (Alternative considered: assert `lagged_drops` without shrinking
capacity — rejected, still TCP-timing-dependent.)

**Tests.** The test itself is the deliverable; it must fail if the egress stops
incrementing `lagged_drops` or if convergence regresses.

---

### #10 — capability world defaults are not `doc_type`-scoped (additive)

**Problem.** `world_cap_defaults` stores one `CapabilityGrants` per world applied
to all doc types (`sqlite.rs:1180`, stored as JSON under `world_caps:{world_id}`).
The spec (§7.2) allows per-`doc_type` scoping.

**Fix.** Introduce a `WorldCapDefaults { all: CapabilityGrants, by_type:
BTreeMap<String, CapabilityGrants> }` stored shape. No users exist, so the stored
form is simply replaced — **no compat deser needed**. Resolution merges `all` +
`by_type[doc_type]` for a given doc's type. Update the call sites that know the
type:
- `list_documents` (`routes.rs:403`, has `q.r#type`),
- `get_document` (`routes.rs:432`, has `doc.doc_type`),
- `filter_command` per-op (`permission.rs`, each op's doc carries `doc_type`),
- `apply_intent` per-op (`sqlite.rs:866`).

Add a `world_cap_defaults_for(world, doc_type)` repo method returning the merged
grants; keep/adjust the setter to store the `{all, by_type}` shape. The grant
**projection** to clients (`project_grants_for`, `permission.rs:188`) must project
the type-relevant merged grants (or the full structure with other users' per-user
grants still stripped) — the plan specifies which, preserving the "never leak other
users' UUIDs" invariant.

**Tests.** A `by_type["token"]` grant applies to tokens but not actors; `all`
applies to every type; merge is union; client projection still drops other users.

---

### #9 — `core:create` world authorization (additive; builds on #10)

**Decision (user):** **GM-only create by default; widen via grant.** No
compatibility concern (no users).

**Problem.** Creation is not gated by `core:create` (`apply_intent` Create arm,
`sqlite.rs:846`, checks `WRITE_FIELDS` + declared requirements, never `CREATE`).
The `core:create` constant exists (`permission.rs:21`) but is dead.

**Mechanism note.** At Create time there is **no document**, hence no `DocRole` to
resolve against. `core:create` is therefore a **world-level** capability tied to the
actor — it cannot use the per-doc `by_role` floor. It resolves from the world's
capability defaults' **user-keyed** grants (doc-type-aware via #10's
`by_type[doc_type].by_user` ∪ `all.by_user`).

**Fix.** In the `apply_intent` Create arm, after the existing checks:
- GM / server admin (`world_role == Gm`, i.e. `access.all`): always allowed.
- Otherwise: allowed only if the world's merged defaults grant the actor
  `core:create` for the new doc's `doc_type` (user-keyed). Else reject with
  `DataError::Forbidden` → **403** (no target id exists, so the #1 existence-leak
  concern does not apply — 403 is correct for a world-scoped capability denial).

Role-based create-widening (e.g. "all players may create tokens") is **not** in
scope — it would need a WorldRole-keyed world grant, a new structure. User-keyed
widening satisfies "grant to widen" for now; the larger model can land with the
capability-management milestone. Logged to `TODO.md`.

**Tests.** Non-GM create rejected by default (403); non-GM create allowed after a
user-keyed `core:create` world grant for that type; GM create always allowed;
a grant for type A does not authorize creating type B.

---

### #8 — see-as picker labels by short id (client)

**Problem.** `Stage.svelte` (`modules/core-ui/panels/Stage.svelte:195`) labels the
GM see-as-player picker "See as <first 8 of uuid>" — it derives candidates from
distinct token `owner`s (`:111`) with no username source. `Welcome` carries only
the actor's own role; no members/usernames reach the client.

**Fix.**
- **Server.** Extend `list_members` (`sqlite.rs:251`) to JOIN `users` →
  `(user_id, username, role)`; add `username` to `MemberEntry` (`routes.rs:326`).
  Endpoint stays GM-gated (`require_gm`) — see-as is GM-only, so this is sufficient
  and leaks no membership to players.
- **Client.** Add `listWorldMembers(worldId)` to `api.ts` (returns
  `{ user, username, role }[]`). `WorldSession` fetches members after Welcome
  **only when the actor is GM** (the endpoint 403s players), stores a
  `userId → username` map, and exposes it via `AppContext`. `Stage.svelte` labels
  each option by username, falling back to the short id when a member is absent from
  the map (e.g. a token owned by a since-removed user). Also list members who own
  no token, per the TODO.

**Tests.** Server: `list_members` returns usernames; endpoint 403s a non-GM.
Client/e2e or unit: picker renders usernames given a members map; falls back to
short id for an unknown owner. (e2e wiring per existing `stage.spec.ts` patterns.)

---

### #11 — replay optimistic intents issued while disconnected (client; **buddy-check**)

**Decision (user):** Optimistic + enqueue while reconnecting; **flush in FIFO order
after resync so the client provably re-converges (no permanent out-of-sync)**;
reject when the session is stopped.

**Problem.** `WorldSession.dispatchIntent`
(`lib/worldSession.svelte.ts:89`) drops a dispatch when `WsClient` is not connected,
to avoid an orphaned optimistic pending entry that would mis-correlate the FIFO
confirm of the next echo. A reconnect does not replay the dropped action.

**Fix.**
- **Dispatch.** When `running` but transport is null (reconnecting):
  `applyIntent` optimistically (immediate UI feedback) **and** enqueue
  `{ intentId, ops }` on a `WorldSession` replay queue. Because *every* intent
  during the offline window queues, the optimistic FIFO order equals the eventual
  send order — preserving the confirm-correlation contract the current drop
  protects. When **not** `running` (session stopped / fatal close): reject at
  dispatch (drop + log; optional user-visible notice). No reconnect is coming, so
  replay is meaningless.
- **Reconnect / resync.** `WsClient` already sends `resync_request` on Welcome and
  marks `resync_end` (`ws-client.ts:219`, `:228`), and the `OptimisticClient`
  rebases predicted ops onto each authoritative update. After `resync_end` brings
  authoritative state current (and after scene re-subscription,
  `worldSession.svelte.ts:206`), **flush the replay queue in FIFO order** through
  the normal `#ws.send` path. Queued intents then receive server confirm/reject
  normally; the optimistic layer's rebase guarantees the optimistic view and
  authoritative state **converge** — the user's "resolve differences so we don't
  remain out of sync" requirement.

**Convergence is the contract.** A long outage may produce optimistic predictions
that visibly snap to authoritative on resync; that is acceptable. What must hold:
after resync + flush, no permanent divergence, and FIFO confirm correlation is
never broken.

**Buddy-check focus.** (a) FIFO correlation across the offline→flush boundary;
(b) the rebase interaction when authoritative resync updates arrive *before* the
queued intents are sent (pending-but-unsent optimistic entries during resync);
(c) reject-when-stopped path leaves no orphaned pending entry.

**Tests.** Unit/e2e: an intent dispatched while transport is null is applied
optimistically and not lost; on reconnect it is sent and the final state matches a
control where the same intent was sent online; dispatch while stopped is rejected
with no orphaned pending entry; two offline intents flush in dispatch order.

---

## Out of scope (stays deferred — do not build the blocking infra here)

- **M12 multi-scene / active-scene:** deterministic active-scene selection,
  scene-background e2e, wall-less-scene full vision, M9b vision active-scene filter
  switch, `explored_fog` purge on scene/world deletion, caption font-size token.
- **Merge engine:** `set_pointer` removal semantics (`null` ≠ absent).
- **Module management / 2nd contract provider:** `reconcileTopology` version/
  provides mismatches, singleton multi-provider policy, capability version
  negotiation.
- **Rotation authoring:** token-rotation shortest-delta lerp.
- **Housekeeping until volume:** `tower_sessions` expired-row sweep.
- **Accepted (no action):** `core:delete` GM-only floor; grants targeting
  `DocRole::None`; Update-to-since-deleted dropped on replay; replay redaction not
  point-in-time; the saturated-lagged-WS CI convergence latency (needs a
  constrained-CPU repro before touching `conn.rs`, not a blind fix); M8c-2
  canvas-chrome re-audit (resolved).

## Documentation sync (final step, per project rules)

On completion: remove the closed `Needs triage` entries from
`POST_WORK_FINDINGS.md` and the closed deferrals from `TODO.md`; add the two new
deferrals logged above (role-based create-widening; embedded-Update redaction if
the buddy-check rules it out of this pass) to `TODO.md`; update the M8b rate-limit
spec note. No `OPEN_BUGS.md` entries (empty).
