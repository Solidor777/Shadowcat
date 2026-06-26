# M2 — Streamed Continuous Vision (Server-Authoritative, Leak-Free) — Design

**Status:** Approved (user, 2026-06-25). Implements **M2** of the parent design
`2026-06-25-server-authoritative-movement-design.md`, replacing its §4/§7 "server-clip then
accept-the-leak" staging with a **server-precomputed, per-recipient vision trajectory** that
is **strictly leak-free**. Supersedes the discarded M2a "server-clip `VisibleSegment`" and
"broadcast-full-path + client raycaster" drafts.

Builds on **M1** (server-authoritative move execution; branch `m10e-5-movement-animation`
@ 248b484). M1 delivers the executed render-path **to the mover only** (one-shot reply);
observers see only the atomic stop. M2 makes **every** scene viewer animate the move with
**continuous vision** — the moving token tweens, occlusion sweeps with the animation, and the
mover's own reveal progresses along the walk — with **no token-path or wall geometry ever
leaving the server beyond what each recipient may see**.

## 1. Goals (both required — user, 2026-06-25)

1. **Smooth motion + correct occlusion.** A watched token tweens smoothly along its route and
   is smoothly hidden/revealed at walls and darkness as the watcher's vision dictates.
2. **Continuous vision sweep.** Visibility recomputes *along* the animation: the mover's reveal
   progresses cell-by-cell as they walk; a watcher's occlusion of the moving token updates
   continuously; (live cross-animation pickup is a documented v1 limitation — §8).

## 2. Why server-streamed (the rejected alternative)

Token positions are **not** redacted by vision server-side — `filter_command` redacts by the
capability model (`cap::READ` / `see_gm_only`), so token `/system/x,y` reach every scene
member and the **client fog is the sole secrecy gate** ([[fog-is-the-secrecy-gate-fail-closed]]).
A *client-computed* continuous vision would therefore have to recompute fog locally — but the
server raycasts the **full** wall set including `gm_only` sight walls the client never receives
("a `gm_only` wall the player never receives still occludes"). A client recompute lacks those
walls and would transiently reveal secret-walled areas, **and** would require broadcasting the
full render-path (leak) for observers to clip locally.

**Server-precomputed streaming avoids both.** The server computes vision against the full wall
set (secret walls occlude correctly) and clips each recipient to only what they may see (no
path leak). The client renders authoritative polygons and **computes no vision**. This is
*strictly better* secrecy than the parent spec, which had accepted a transient leak — that leak
is now **gone**.

**Dependencies:** none added. Server reuses `vision::visibility_polygon`; client reuses the
existing polygon fog renderer + time-sync (`TimePing`/`server_time`) + the M10e-5
`TokenAnimator`. Fog *tweening* (§6) uses PixiJS render-texture cross-fade — no morphing of
topologically-divergent polygons, no new package.

## 3. Core architecture — precompute at execute, play back time-synced

State is **atomic** (M1): the move commits to its stop the instant it is approved, and vision
is a deterministic function of position. So the server computes the **entire vision trajectory
once, at execute time**, and the client plays it back on a server-aligned clock. **No per-move
server timer loop.**

### 3.1 Server (extends `Room::execute_move` / `handle_move_request`)

After M1 computes the legal `render_path`, `stop`, `duration_ms` and commits the atomic
position `Event` (unchanged):

1. **Sample the path** into `(t_k, p_k)` for `k = 0..N` (§5 sampling). `t_k ∈ [0, duration_ms]`
   from constant-speed arc-length; `p_0 = start`, `p_N = stop`.
2. **Mover vision trajectory:** for the moving viewer (the mover; a GM see-as target is handled
   by that connection's own egress view-ctx), raycast `player_vision_polygons`-equivalent at
   each `p_k` **against the full wall set** → `vision_k` (the moving token's swept polygon
   unioned with the mover's other owned tokens' static polygons). This is the sweep.
3. **Broadcast** one `MoveStream` aux frame (via `broadcast_aux`) carrying the **full** position
   trajectory + the mover id + the mover vision trajectory + `stop`/`duration_ms`/`request_id`/
   `start_server_ms`. The full data lives only in-process; the egress strips it per recipient
   (§3.3) before any socket write — identical to how `Event`/`vision` are masked.

### 3.2 Wire protocol (`ws/protocol.rs`, ts-rs → generated TS → Zod mirror)

```
PosSample    { t_ms: f64, pos: [f64; 2] }
VisionSample { t_ms: f64, polygons: Vec<Vec<[f64; 2]>> }   // scene-local vision polygons at t

ServerMsg::MoveStream {
    request_id:      Uuid,                  // mover correlation (resolves the moveRequest promise)
    token_id:        Uuid,                  // the moved token
    mover:           Uuid,                  // user_id of the mover (egress: full vs clipped)
    scene:           Uuid,                  // active scene (polygons are scene-local)
    start_server_ms: f64,                   // server clock at animation start (sync/catch-up)
    duration_ms:     f64,                   // deterministic total (M1)
    stop:            [f64; 2],              // authoritative final cell (also via the position Event)
    samples:         Vec<PosSample>,        // time-tagged positions (egress clips per recipient)
    mover_vision:    Option<Vec<VisionSample>>,  // mover-only sweep; egress nulls for observers
}
```

`MoveError { request_id, message }` — unchanged, mover-only via `etx` (a rejected move did not
happen; nothing to stream).

### 3.3 Per-recipient egress transform (`conn.rs` `egress_loop`, dedicated `MoveStream` branch)

The discriminator is the connection's **effective view-user** — its own `ctx.user_id`, or the
GM see-as target. For each connection, before the sink write:
- **`mover_vision`:** kept **iff** effective-view-user `== frame.mover` (the mover, or a GM
  viewing as the mover). Nulled for everyone else — the mover's sightlines are disclosed to no
  one else. (A full-vision GM observer has no fog to sweep anyway; nulling is correct.)
- **`samples`:** **clip** to those whose `pos` is visible to the effective view-user — point-in-
  poly against that user's **cached authoritative vision polygons** (reuse their `vision`
  scene-subscription payload; recompute via `player_vision_polygons` only if absent). The mover
  and a full-vision GM both fall out of this clip with **all** samples (everything is in their
  vision), so no special case is needed. If no sample is visible ⇒ **suppress the frame** (the
  recipient learns nothing of a move it cannot see).
- **Fail closed:** no cached/derivable vision ⇒ clip to empty ⇒ frame suppressed. A point on a
  polygon boundary counts as visible (over-include is safe *only* within the recipient's OWN
  vision — never widened beyond it).

No lock across await: read the recipient vision under the ECS guard (or from cache), drop the
guard, clip, send.

### 3.4 Client playback (`@shadowcat/core` + render)

`MoveStream` is an **unsolicited broadcast**. The mover additionally resolves its pending
`moveRequest` promise on `request_id`; all recipients drive playback:
- **Clock:** align `start_server_ms` to local time via the existing time-sync; play `t ∈
  [0, duration_ms]`.
- **Position:** interpolate the token between `samples` by `t_ms` (smooth tween via the
  retained `TokenAnimator`); **hide the token across gaps** (consecutive samples > one sample
  interval apart ⇒ occluded span). Settle at `stop` when the position `Event` lands.
- **Fog (mover only):** feed the existing fog renderer the `mover_vision` polygon for the
  current `t` (snap to the latest sample ≤ `t` at the sample rate; §6 adds cross-fade). Observers'
  fog is their existing static vision (unchanged during the move).
- **Catch-up:** if playback falls behind the server-aligned clock (frame hitch), jump to the
  sample for the current `t`. At/after `duration_ms`, authoritative resting state (vision
  subscription + committed position) reasserts.

## 4. Secrecy posture (improves on the parent spec)

Fully server-authoritative; **no leak introduced, the parent spec's accepted leak removed**:
- **Secret (`gm_only`) walls** occlude correctly — the server raycasts the full wall set; the
  client never computes vision and never receives secret walls.
- **No render-path leak** — an observer receives only the position samples their own vision
  admits (per-recipient egress clip); hidden path portions never leave the server.
- **Mover sightlines** (`mover_vision`) go only to the mover (+ GM see-as), nulled for observers.
- **Fail closed** ([[fog-is-the-secrecy-gate-fail-closed]]): missing vision ⇒ empty clip ⇒
  suppressed frame; the moving token, like every token, is gated by authoritative fog.

## 5. Sampling

- **Distance-based** along the path: target ≈ 3 samples per grid cell (≈ `cell/3` spacing),
  **always** including `start`, `stop`, and the times the path crosses cell boundaries (where
  supercover membership — hence occlusion — can change).
- **Times** from cumulative arc-length: `t_k = (len_k / total_len) · duration_ms`.
- **Hard cap** `MAX_VISION_SAMPLES` (96): longer moves reduce density to stay under the cap
  (coarser sweep, never unbounded raycasts). Bounds the mover's raycast count per move.
- **Zero-progress / single-cell** moves: one sample (`start`); no sweep; the position `Event`
  settles. (Mirrors M1's zero-progress short-circuit.)

## 6. Fog rendering — snap, then cross-fade (the "tween if it works")

Polygon **morphing** between topologically-divergent vision polygons is intractable; do **not**
attempt it. Two stages:
1. **Snap (baseline):** set the fog polygon to the latest vision sample ≤ `t`. At ≈ 3 samples/
   cell and typical speeds the fog updates at ~15–30 Hz — reads as continuous.
2. **Cross-fade (enhancement):** rasterize each sample's fog to a PixiJS **render texture** and
   alpha cross-fade between consecutive samples over the inter-sample interval — smooth without
   morphing, no new dependency. Built last; the snap baseline ships first.

## 7. Performance guards

- Mover raycasts bounded by `MAX_VISION_SAMPLES` per move.
- Observer egress clip is point-in-poly per sample against a **cached** polygon (no egress
  raycast on the common path); recompute only if the cache is absent.
- `MoveStream` is rare (per move) — the egress branch is off the steady-state hot path.
- Suppress `MoveStream` entirely for zero-progress moves.
- Cap `VisionSample.polygons` vertex count consistent with the existing vision payload bounds.

## 8. Known v1 limitation — live concurrency

Each move's per-recipient clip is computed at **its** execute time against the recipient's
then-current vision. Two tokens moving simultaneously do **not** reveal each other mid-walk if a
watcher's vision opens *after* the clip (the watcher never received the now-visible samples). It
reconciles at the stop + the next `vision` rebroadcast. Fully-live concurrency would require
real-time per-recipient streaming (a per-move server loop); **deferred** (the parent spec
treated concurrent pickup as advanced). No correctness/secrecy impact — only a missed transient
reveal.

## 9. Decomposition (one M2 plan; SDD tasks — fresh `shadowcat-coder` + per-task
`shadowcat-spec-reviewer` + `shadowcat-code-reviewer`; whole-branch buddy-check at end)

| # | Task | Deliverable |
|---|------|-------------|
| 1 | Protocol + types | `PosSample`/`VisionSample`/`MoveStream` (ts-rs) + generated TS + Zod mirror + protocol tests. `MoveError` unchanged. |
| 2 | Server sampling + position trajectory | Path→`(t_k, p_k)` sampler (§5) in a pure, unit-tested module; `execute_move`/`handle_move_request` build + `broadcast_aux` a `MoveStream` with full `samples`, `mover_vision: None` for now. Integration: a second connection receives a (full, unclipped at this task) `MoveStream`; rejection still `MoveError` mover-only. |
| 3 | Mover vision trajectory | Raycast `vision_k` at each `p_k` (full wall set); fill `mover_vision`. Unit tests: sweep grows as the token advances; bounded by `MAX_VISION_SAMPLES`. |
| 4 | Per-recipient egress clip | `egress_loop` `MoveStream` branch: mover full + keep `mover_vision`; observer clip `samples` to visible + null `mover_vision`; suppress when empty; fail-closed; no lock across await. Integration: observer behind a wall gets a gap / suppressed; mover gets full; secret-wall area never streamed to a non-owner. |
| 5 | Client position playback | `ws-client` `MoveStream` broadcast handling (mover resolves `request_id` + all dispatch `onMoveStream`); time-synced clock; `TokenAnimator` plays `samples` with gaps; settle at `stop`. vitest + typecheck. |
| 6 | Client vision-sweep fog (snap) | Mover feeds `mover_vision` into the fog renderer per `t` (snap); observers unchanged. Tests: fog polygon advances with the clock; reverts to subscription vision at end. |
| 7 | Fog cross-fade (enhancement) | Render-texture cross-fade between consecutive vision samples (§6). Visual-smoothness; gated behind the snap baseline so it is independently revertible. |
| 8 | Docs + skill gate | Update `shadowcat-codebase-{scene-rendering,realtime-sync,client-shell}` skills + PLAN.md M10e; `shadowcat-spec-reviewer` confirms the skill diffs. |

## 10. Invariants this checkpoint must hold

- **Server-authoritative vision; client computes none** (ARCHITECTURE §2 invariant 3/4). The
  client renders only authoritative streamed polygons.
- **No fork of the per-cell/secrecy decision (§13):** observer clipping uses the recipient's
  authoritative server vision; the mover trajectory uses the full-wall-set raycaster.
- **Fog is the secrecy gate — fail closed** ([[fog-is-the-secrecy-gate-fail-closed]]).
- **Atomic state unchanged:** M2 adds only the cosmetic `MoveStream` overlay + client playback;
  the position `Event`, `moving` lock, and `commit_ops_locked` path are M1-final.
- **Deterministic duration / time-sync:** the client plays the server `duration_ms` on a
  server-aligned clock; it never invents its own timing (parent spec §3).
- **No lock across await** in the new egress branch (mirrors M1 / `handle_pathfind`).
- **Strip before transmission:** per-recipient clipping happens in egress before the sink write.

## 11. Deferred (NOT in this checkpoint)

- **Live cross-animation concurrency** (§8) — real-time per-recipient streaming.
- **Animated vision for light-bearing movers** (a moving light sweeping reveal) — parent spec §9;
  `mover_vision` is LOS∩lit-consistent at each sample but does not model a light that itself moves
  beyond the carrier token.
