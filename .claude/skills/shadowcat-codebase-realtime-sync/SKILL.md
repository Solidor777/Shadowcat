---
name: shadowcat-codebase-realtime-sync
description: "Use when touching Shadowcat realtime: WebSocket transport, per-world rooms, broadcast/egress, sequence numbers + resync, the client document store and optimistic/rollback, sessions/auth, user accounts (admin-created) and world invite/accept seating, or live search. Covers src/server/src/{ws,http,auth} + src/client/core store. Invoke shadowcat-codebase-core first."
---

# Shadowcat — Realtime & Sync

Orientation for the realtime transport (ws/http/auth) and the client store with optimistic
application + rollback.

## Purpose

The client sends intents over a WebSocket; the server validates, applies, and broadcasts ordered
events to per-world rooms. Every broadcast carries a per-world monotonic sequence number; clients
detect gaps and resync from a bounded event buffer or a snapshot. The client may apply intents
optimistically and roll back on divergence.

## Key files & seams

- `ws::room` — `Room` (per-world), `RingBuffer` (time/size-bounded event buffer)
  + `range_from(from_seq)` for gap resync, `subscribe() -> (Receiver, seq)`, `current_seq()`,
  `broadcast_aux()` (out-of-band), `RoomRegistry`. `get_or_create` cold-hydrates the scene ECS:
  scene entities (`query_scene_entities`) **plus** the world config-docs
  `world-settings`/`light-gradation`/`vision-modes` + actors (`query_documents`), seeded via
  `SceneEcs::set_world_config`/`set_actors`; the live `apply_op` path keeps the side-tables current.
  - `Room::commit_ops_locked(repo, ctx, ops, ts)` (`pub(crate)`) — gate-free authoritative write
    tail (apply_intent → ECS-hydrate → ring/seq → broadcast Event → stats). Extracted from
    `publish`; PRECONDITION: caller MUST already hold `publish_guard`. Non-reentrant — do NOT
    re-acquire `publish_guard` inside (tokio `Mutex` would deadlock). Both `publish` and
    `execute_move` call this as their commit step.
  - `Room::execute_move(repo, ctx, scene_id, token, path, ts)` — server-authoritative token move.
    **`scene_id` IS NOT TRUSTED and selects nothing.** The gate's scene is DERIVED from the token
    via `SceneEcs::token_move(token, &[])`, and every gate input keys on that; the request's
    `scene_id` is only checked for agreement and refused on mismatch (redundant defense-in-depth).
    Taking it as input was a `[sec]` **Critical** — a total bypass of the wall + visibility gate,
    with authorization fully intact — so any NEW move-like or routing frame added here must derive
    the same way. A frame naming no token (`Pathfind`) must instead prove presence. Full invariant +
    the failure it prevents: `shadowcat-codebase-scene-rendering`, the derive-from-token INVARIANT.
    Acquires `publish_guard` at the TOP and HOLDS it across the entire validate→commit critical
    section (mirrors `publish` atomicity). Scene read locks are scoped and dropped before the
    `get_explored().await` (no lock across await); `publish_guard` (tokio `Mutex`) is intentionally
    held across awaits. Calls `move_exec::execute_move` (pure, lock-free), then `commit_ops_locked`
    (single acquisition, no re-entry). Atomic single position write (`/system/x` + `/system/y`
    OCC pre-image ops). Returns `MoveExecution { scene, stop, render_path, duration_ms, samples,
    mover_vision, cost }` —
    `scene` is the DERIVED scene, and it is what `MoveStream.scene` is stamped from, so the
    per-recipient egress clip and the client's viewed-scene filter cannot key on a client value.
  - `moving: Mutex<HashMap<Uuid, i64>>` — per-token moving lock: token → move-end epoch-ms. Lazy
    expiry (no timer); absent or expired entry allows the move. Updated after each successful commit
    (still inside `publish_guard`). In-memory only — cleared on server restart (move state is derived,
    not durable).
- `ws::protocol` — client/server message frames; `ServerMsg`, `event_seq()`.
- `ws::conn` — per-connection loop + egress; `ws::time` — server time source +
  client offset calibration (exists before its consumer, per ARCHITECTURE §2 invariant 2).
  `send_filtered` (Phase C: now takes `room: &Room`, not just `repo`/`ctx`) is where per-recipient
  redaction actually happens: only the `Event` branch carries document data, so only it is
  redacted — every other frame (including `MoveStream`, clipped separately by `clip_move_stream`;
  see the invariant below) passes through unredacted via `send_filtered`. The `Event` branch's
  shape: `load_update_docs` awaits the Update pre-images ONCE, before any lock is taken (no lock
  across await); then a short `room.scene()` read guard wraps ONLY the synchronous
  `permission::filter_command(command, ctx, world_defaults, &current, |id| ecs.actor(id))` call —
  the room's in-memory `SceneEcs` actor table is the egress-side owner join, so this never touches
  the pool (the same short-read-guard discipline `clip_move_stream` uses for vision). See
  `shadowcat-codebase-documents-permissions` for `filter_command`'s own internals and the other two
  owner-join sources (`list_documents`'s batched prefetch, `effective_owner_of` on single-doc
  routes/search).
- `http::routes` (+ the `http` module root) — HTTP routes (login, assets, embed).
- `auth::session` — `SqlxSqliteStore` (DB-backed sessions), `spawn_session_sweep`
  (also GCs `world_invites` rows via `DELETE FROM world_invites WHERE expires_at <= ?`, bound to
  `now_ms - INVITE_GC_GRACE_MS` (30-day grace, not the raw expiry) — an expired-but-recent invite
  survives a while post-expiry for audit/support lookup before GC actually removes it; same timer
  as session sweep, no dedicated invite timer),
  `SessionUser`/`AuthUser`/`AdminUser`; `auth::password`, `auth::role`.
- `http::throttle` (Phase A) — `AuthThrottle` (`check(key, now_ms, per_min) ->
  bool`, sliding 60s window, `Mutex<HashMap<String, Vec<i64>>>`), shared by the two
  Argon2-verifying endpoints (`/api/login`, `POST /api/invites/accept`). **INVARIANT (no
  enumeration oracle):** identity keys (`login:u:<username>`, `invite:u:<uuid>`) count attempts
  against unknown identities exactly like known ones — the throttle must never distinguish an
  existing identity from a nonexistent one, mirroring the routes' own constant-verify
  anti-enumeration property. Keys are opaque strings the CALLER composes
  (`login:u:<>`/`login:ip:<>`/`invite:u:<>`/`invite:ip:<>`); budgets are per-identity AND per-IP
  (`LOGIN_PER_MIN_PER_IDENTITY=10`/`LOGIN_PER_MIN_PER_IP=30`,
  `INVITE_PER_MIN_PER_ACCOUNT=10`/`INVITE_PER_MIN_PER_IP=30`) so identity-rotating stuffing from
  one address is bounded too. `MAX_TRACKED_KEYS=65_536` caps the map; at capacity, expired keys
  are swept first, and if still full a NEW key FAILS CLOSED (throttled) rather than evicting live
  state. `ClientIp` (Phase A axum extractor) is infallible: `Some` under real `ConnectInfo`,
  `None` under the axum-test mock transport (IP throttling degrades to identity-only there, never
  a 500) — `AppState.auth_throttle: Arc<AuthThrottle>`. All four budgets are config-tunable
  (`Config.login_per_min_per_identity`/`login_per_min_per_ip`/`invite_per_min_per_account`/
  `invite_per_min_per_ip`, `None` → the constants above, env/TOML-layered like every other optional
  `Config` field) — the shell e2e suite relaxes them via `playwright.config.ts`'s `webServer.env`
  so its many same-identity logins across specs can't trip the default budget.
- **Accounts + world seating.** `POST`/`GET /api/users` are admin-only, gated by the **`AdminUser`
  extractor** — the chokepoint for every server-tier gate. **INVARIANT: never build a server-tier
  gate on world role.** `permission_context` maps `ServerRole::Admin → WorldRole::Gm`, so any
  world-role check is satisfied by an ordinary GM; `AdminUser` reads the session's `ServerRole`
  directly and, being an extractor, rejects BEFORE body deserialization. `create_user_unique` is a
  single guarded `NOCASE` INSERT (clean 409, never a 500 or a check-then-act pair), and the ASCII
  username policy is applied at every insertion path including `/api/setup`/`bootstrap_admin`.
- **`auth::invite` + the world-invite repository methods** — a GM mints an invite
  for their own world; the invitee redeems it from their OWN session (`POST /api/invites/accept`
  with the code in the BODY, never the URL — a path-borne credential reaches the tower-http trace
  span, browser history, `Referer`, and proxy logs). Two invariants: **(1) redemption failures are
  UNIFORM** — invalid, expired, revoked and already-consumed are indistinguishable in status, body
  AND verify count (exactly one Argon2 verify per path, using a dummy PHC when no row matches,
  mirroring `anti_enumeration_phc`); a distinguishable path relocates the username oracle rather
  than closing it, and the property is pinned by a `#[cfg(test)]` verify counter, not by timing.
  **(2) Consume is a SINGLE guarded `UPDATE … RETURNING`** with every lifecycle predicate in the
  WHERE, seating sharing its transaction, so concurrent redemption cannot double-seat.
  **Why seating is by invite and not by name:** `add_member`-by-username returned 404-vs-204 and so
  leaked username existence to any authenticated account (`create_world` requires only `AuthUser`,
  so anyone can become a GM) — contradicting the constant-time verify `/api/login` already pays to
  hide exactly that. A uniform 204 would NOT have closed it, because seating-on-hit stays
  observable via `list_members`. Naming a target is the disclosure; the invite removes the naming.
  NOTE this is about the by-NAME path only: `add_member` survives (GM-gated, by user ID, 404 on an
  unknown id) — it is naming a user by a guessable identifier that was removed, not membership writes.
- **Deletion & eviction (Phase B).** `ServerMsg::Evicted { user: Option<Uuid> }` is the terminal
  out-of-band frame: `None` addresses every connection in a room (world deletion, broadcast on the
  removed room), `Some(id)` addresses one user's connections across ALL rooms
  (`RoomRegistry::evict_user`, account deletion). The egress loop delivers the frame, sends a
  protocol Close, and terminates; targeting keys on the server-resolved `ctx.user_id`, mirroring
  the `MoveStream` per-recipient precedent. `RoomRegistry` carries a deletion tombstone
  (`begin_delete` removes the room + blocks `get_or_create` re-creation until `finish_delete`,
  which must run on success AND failure paths; post-insert, `get_or_create` re-checks the
  tombstone AND re-verifies the world row — a delete can complete entirely inside the hydration
  window, lifting the tombstone before the re-check, so only row absence refuses that ghost). **INVARIANT: eviction is load-bearing, not cosmetic** —
  `permission_context` resolves once per connection, so a revoked membership/account is never
  re-checked on a live socket. **INVARIANT: user deletion revokes sessions INSIDE its delete
  transaction** (`json_extract(data, '$.data.user.id')` on `tower_sessions` — no user_id column):
  `AuthUser` trusts the session record without re-reading `users`, so a surviving row would keep a
  deleted account fully authenticated until cookie expiry. Client side: `WsClient` treats
  `evicted` as terminal (`stop()` — no reconnect) and surfaces `onEvicted`, which the shell routes
  to `leaveWorld()`.
- `WsClient` — client WS connection + resync. **Welcome watchdog +
  connection-generation guard (silent-hang-startup fix):** `open()` arms a `welcomeTimeoutMs`
  watchdog (default 10s, `opts.welcomeTimeoutMs`) once `opts.connect` resolves; an open-but-
  unwelcomed transport is closed into the normal `scheduleReconnect` path instead of hanging on
  "Connecting…" forever (the browser's socket `open` fires at HTTP 101, BEFORE the server's
  Welcome preamble — see `handle_socket`'s per-connect DB round trips + blocking
  `scan_installed_modules` scan). Every `open()` attempt is tagged with a monotonically
  increasing `connGeneration`; `handleFrame` ignores a `"welcome"` OR `"resync_end"` frame whose
  generation doesn't match the CURRENT connection before acting (clearing the watchdog, setting
  `serverOffsetMs`, emitting `onWelcome`; resp. advancing `nextExpected` + emitting
  `onResyncComplete`) — this closes the reintroduction where a frame already queued as a message
  task when the watchdog fired could still arrive after reconnect and incorrectly act on (or
  disarm) the successor connection's state. **`armWelcomeWatchdog` itself also guards against
  out-of-order delivery**: a `welcomedGeneration` field (set inside the `"welcome"` case, after the
  generation check) makes arming a no-op when the CURRENT generation is already welcomed — covers a
  `Connect` implementation that delivers Welcome via a microtask BEFORE its own promise resolves
  (the continuation that calls `armWelcomeWatchdog` runs strictly after), which would otherwise arm
  a watchdog nothing will ever disarm. **`reconnectAttempt` resets on WELCOME, not on socket
  `open`** — the reset lives in the `"welcome"` case (after the generation guard), not in
  `open()`: a server that accepts the socket but never sends Welcome must keep backing off
  (`scheduleReconnect`'s exponential-with-full-jitter delay) on every watchdog-close/reconnect
  cycle instead of retrying at the base delay forever, which would amplify load against exactly the
  degraded server the watchdog exists to escape.
- `webSocketConnect(url, connectTimeoutMs = 10_000)`: bounds
  the handshake so an accepted-but-never-upgraded socket settles (rejects + closes) instead of
  leaving `WsClient`'s `scheduleReconnect` path unreachable behind an unsettled connect promise. A single
  shared `settled` flag (distinct from `opened`) guards ALL THREE settling paths (timeout, error,
  open) — a `connectTimeoutMs` expiry and an already-queued `open` event landing in the same tick
  no longer lets `opened` flip true AFTER the promise already rejected, which would otherwise let a
  later `close` event ALSO reach `handlers.onClose` (double-scheduling a reconnect on top of the
  promise-rejection path, and the orphan transport's own close nulling `WsClient.transport` out
  from under the live one). The `open` listener discards the socket (`ws.close()`, no resolve) when
  `settled` is already true instead of completing the handshake.
- `DocumentStore implements ReadableDocuments` (authoritative,
  rollback base).
- `OptimisticClient implements ReadableDocuments` (the
  optimistic view the UI/canvas render).

## Hard invariants

- **Ordered, recoverable realtime** (ARCHITECTURE §2 invariant 2): every broadcast carries a per-world
  monotonic seq from an atomic counter; clients gap-detect and resync from the `RingBuffer` or a
  full snapshot.
- **Optimistic with rollback** (ARCHITECTURE §2 invariant 3): `OptimisticClient` applies locally tagged with
  an intent id; the server confirmation reconciles; divergence rolls back to `DocumentStore`.
  `appliedSeq` is identical across the two so the derived watermark holds
  [[render-from-optimistic-view]].
- **Socket-buffer backpressure is non-portable** — `SO_SNDBUF`/`SO_RCVBUF` are advisory; test the
  generic egress sink with a credit-gated `Sink`, not real-socket TCP backpressure
  [[socket-buffer-backpressure-nonportable]].
- **Debounce on the leading edge, arm only when idle** (or cap max staleness) — re-arming on every
  event starves under load [[debounce-leading-edge-not-trailing-rearm]].
- **Check-then-act across two pool queries needs one transaction** [[two-query-guard-needs-tx]].

## Gotchas

- **`WsClient.open()` does not re-check `running_` after its connect await** — a `stop()` call
  during a pending connect can leave an adopted-but-unwatched transport assigned to
  `this.transport`.
- **Docs-ratchet is live on the whole `ws/` tree AND the `http/` + `auth/` trees:**
  every file in all three trees carries `#![deny(missing_docs)]` +
  `#![deny(clippy::missing_docs_in_private_items)]` — a new undocumented item fails the 3-OS CI
  clippy step, and `ws::protocol` doc comments flow into the generated `ServerMsg`/`ClientMsg` TS
  types (regenerate + commit bindings with any change; the docs site's protocol page links these
  types). Route-handler docs in `http::routes` cite their authz gate (`require_gm`/`AuthUser`/
  `AdminUser`/`permission_context`) and the 404-uniform existence-hiding contract — keep those
  citations true when changing a route's gating.

- **Permissions filter every broadcast per recipient** — hidden fields are stripped before
  transmission (see `shadowcat-codebase-documents-permissions`), never sent-then-hidden.
- **Live search rides the broadcast** as top-N subscriptions over the same egress
  [[m6c-2-live-search]].
- **One-shot correlated request pairs** (`Search`→`SearchResult`/`SearchError`;
  `Pathfind`→`PathResult`/`PathError`) route replies to the requesting connection only (never
  broadcast); correlated by `request_id` via the `pending` map in `WsClient`. See
  `WsClient` and `ws::protocol`. **Chat ops
  (`SendMessage`/`EditMessage`/`DeleteMessage`) also carry `request_id` but are ASYMMETRIC**: only
  a REJECTION replies (`ServerMsg::ChatError`, sender-only), while success is confirmed by the
  broadcast `Event` echo. They use a separate `chatPending` map whose timer resolves
  (success-assumed) rather than rejects on timeout — see `shadowcat-codebase-chat`.
- **`ScenePing` is gated by `scene_ping_permitted` (Phase A), not by scene
  selection.** Unlike `MoveRequest`/`handle_pathfind` (which SELECT server state and so must
  derive-from-token, per the never-fork table in `shadowcat-codebase-core`), `ScenePing` relays
  only the client-supplied `scene`/`x`/`y` to the room via `broadcast_aux` — there is no server
  state to substitute against.
  The guard instead answers "may this sender ping into this scene at all": the doc must exist, be
  `doc_type == "scene"`, resolve to THIS room's `world_id` via `world_of`
  (`shadowcat-codebase-documents-permissions`), and grant the sender `cap::READ`. Deliberately
  ADMITS a token-less spectator (weaker than `handle_pathfind`'s controls-a-token gate) since a
  READ-only viewer legitimately pings a scene they're watching. Denial is a SILENT drop at the
  call site — no error frame, no behavior split — because any distinguishable response would leak
  scene existence to a non-reader. Rate-limited independently (30/min/user via `ping_rate`,
  checked BEFORE the authz lookup so an over-budget sender never pays a doc read).
- **MoveRequest → MoveStream (broadcast):** `MoveStream` is an **aux broadcast frame** — sent
  via `Room::broadcast_aux` like `ScenePing`, carrying NO seq number (it is cosmetic playback data,
  not an authoritative document event; it never touches the `RingBuffer`/gap-resync path).
  `MoveRequest` is still a one-shot correlated pair for the mover's promise (resolves on the
  matching `move_stream` frame via `pending` map), but `MoveStream` is broadcast to ALL scene
  viewers, not just the mover — the **per-recipient egress transform** (mover full incl.
  `moverVision`; observer clipped to their own visible samples with `moverVision: null`; suppressed
  entirely — zero frames — when the recipient's vision admits none of the move) is where the
  leak-free secrecy boundary lives (`egress_loop`'s dedicated `MoveStream` branch, detailed in
  `shadowcat-codebase-scene-rendering`). `MoveError` remains mover-only, always generic (no path
  geometry / vision state disclosed — no-geometry-leak invariant).
  `handle_move_request` dispatches `Room::execute_move`, then broadcasts `MoveStream` to the scene.
  Client animation is driven by `TokenAnimator.animateSamples` (time-tagged playback, catch-up on
  late arrival, gap/occlusion detection: gap threshold = `minConsecutiveDelta × 1.5` where
  `minConsecutiveDelta` is the minimum positive inter-sample interval across all consecutive pairs;
  Infinity for < 3 samples — no interior gap detectable). `animateSamples` cancels any competing
  ease-to-stop `anim` entry (handles Event-before-MoveStream ordering); `setTarget` is a no-op
  while `samplesAnim` is live (handles MoveStream-before-Event ordering). Wired end-to-end:
  `WsClient.onMoveStream` → `worldSession` → `SceneInteractionBridge.animateSamples` →
  `RenderEngine` → `TokenView` / `TokenAnimator`. `onMoveStream` listeners survive reconnects
  (NOT cleared in `failPending`).
- **Gated moves are request-only + server-executed:** the client sends
  `MoveRequest` and waits; the server validates, executes, and broadcasts `MoveStream`. The client
  MUST NOT apply an optimistic position update for a gated move. The atomic position `Event` (from
  `commit_ops_locked`) is the authoritative document update; the `MoveStream.samples` drive
  cosmetic animation for all scene viewers. The `moveRequest` promise resolves on success (the
  `MoveStream` frame) but the animation is broadcast-driven — no local `animateAlongPath` call
  on the mover side.

## Pointers

- Rationale: `docs/design/ARCHITECTURE.md` §2 (invariants 1-4) + §3 (tokio/axum/sqlx/argon2).
- Relationships:
  `graphify query "websocket room broadcast egress optimistic rollback store session auth"`.
- History: [[m6a-client-core]], [[m6c-1-search]], [[m6c-2-live-search]].
