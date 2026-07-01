# M2 — Streamed Continuous Vision Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every scene viewer animates a server-authoritative token move with continuous, leak-free vision — the moving token tweens, occlusion sweeps with the animation, and the mover's reveal progresses along the walk — by streaming a server-precomputed, per-recipient vision trajectory the client plays back time-synced.

**Architecture:** At move-execute the server samples the legal path, raycasts the mover's vision at each sample against the **full** wall set, and broadcasts one `MoveStream` aux frame carrying the full position trajectory + the mover's vision trajectory. The egress strips it **per recipient** (mover keeps all; observer gets only the position samples their own vision admits, mover-vision nulled; fully-occluded → suppressed) before the socket write. The client plays it back on a server-aligned clock: tween token position between samples (hide across gaps), and the mover feeds the streamed vision polygons into the existing fog renderer (snap, then render-texture cross-fade). No client-side vision computation; no new dependency.

**Tech Stack:** Rust (server, `shadowcat` crate; ts-rs; tokio broadcast), Svelte 5 / TS (`@shadowcat/core`, render), PixiJS (fog render textures), Vitest, `cargo test`.

**Design doc:** `docs/superpowers/specs/2026-06-25-m2-streamed-continuous-vision-design.md` (read it first).

## Global Constraints

- Server crate is `shadowcat` (run `cargo test`/`fmt`/`clippy` from `src/server/`).
- **Build `dist/` before any server `cargo` build** — `rust-embed` validates `../../dist/` at compile time: run `pnpm build` first (`embed-dist-compile-ordering`).
- **ts-rs types are generated** — edit the Rust struct/enum, regenerate, then mirror in the client Zod schema (`src/client/core/src/wire.ts`); a drift guard enforces parity. Never hand-edit `src/types/generated/*.ts`.
- **Server-authoritative; client computes no vision** (ARCHITECTURE §2 invariant 3/4). The client renders only authoritative streamed polygons.
- **Fog is the secrecy gate — fail closed** (`fog-is-the-secrecy-gate-fail-closed`): a missing/garbled clip input hides more, never less; a fully-occluded move suppresses the frame.
- **No lock across await** in the new egress branch (mirror M1 `execute_move` / `handle_pathfind`): read ECS/vision under the guard, drop it, then await the sink.
- **Strip before transmission:** per-recipient clipping happens in the egress before the sink write; the full trajectory never reaches an observer's socket.
- **Atomic state is M1-final:** do not touch the position `Event`, `moving` lock, or `commit_ops_locked`. M2 adds only the cosmetic `MoveStream` overlay + client playback.
- Client logging via the project logger, never raw `console.log`; Rust diagnostics via `tracing`, never `println!`/`dbg!`.
- Cross-platform: `std::path` only; no OS-specific assumptions; responsive/touch unaffected.

## File Structure

- `src/server/src/ws/protocol.rs` — add `PosSample`, `VisionSample`, `ServerMsg::MoveStream`; keep `MoveExecuted` only if still used (Task 5 removes the M1 mover-only reply path → `MoveStream` replaces it; `MoveError` stays).
- `src/server/src/scene/move_stream.rs` (new) — pure, headless: path → time-tagged samples; arc-length→time; sampling cap. Unit-tested in isolation.
- `src/server/src/ws/room.rs` — `execute_move` returns the sampled trajectory + mover vision alongside the existing `MoveExecution`; a `MoveStream`-building helper.
- `src/server/src/ws/conn.rs` — `handle_move_request` broadcasts `MoveStream` via `broadcast_aux`; `egress_loop` gains a `MoveStream` per-recipient clip branch.
- `src/server/src/scene/mod.rs` — a `pub(crate)` accessor to raycast a user's vision polygons **from an arbitrary viewpoint set** (the moving token at `p_k` + the user's other owned tokens), reusing `sight_walls` + `vision::visibility_polygon`.
- `src/types/generated/*` — regenerated.
- `src/client/core/src/wire.ts` — Zod schema for `MoveStream`/`PosSample`/`VisionSample`.
- `src/client/core/src/ws-client.ts` — `move_stream` dispatch: resolve pending (mover) + emit `onMoveStream`; `MoveStream` TS interface.
- `src/client/render/src/token-animator.ts` (+ `token-view.ts`, `engine.ts`, `types.ts`) — play time-tagged samples with gaps over a server duration.
- `src/client/render/src/pixi-backend.ts` (+ `compositor.ts`, `engine.ts`) — feed a time-varying vision polygon into the fog (snap; then render-texture cross-fade).
- `src/client/ui-kit/src/sceneInteraction.ts`, `src/client/shell/src/lib/worldSession.svelte.ts`, `src/client/ui-kit/src/appContext.ts` — wire `onMoveStream` → host playback.
- `src/modules/scene-tools/src/controller.svelte.ts` — request-only commit no longer animates from the promise (playback is broadcast-driven).

---

### Task 1: Protocol — `PosSample` / `VisionSample` / `MoveStream`

**Files:**
- Modify: `src/server/src/ws/protocol.rs`
- Regenerate: `src/types/generated/*`
- Modify: `src/client/core/src/wire.ts`
- Test: `src/server/src/ws/protocol.rs` (`#[cfg(test)]`), `src/client/core/src/wire.test.ts`

**Interfaces:**
- Produces (Rust, ts-rs `#[derive(Serialize, Deserialize, TS)]`):
  ```rust
  pub struct PosSample { pub t_ms: f64, pub pos: [f64; 2] }
  pub struct VisionSample { pub t_ms: f64, pub polygons: Vec<Vec<[f64; 2]>> }
  // ServerMsg variant:
  MoveStream {
      request_id: Uuid,
      token_id: Uuid,
      mover: Uuid,
      scene: Uuid,
      start_server_ms: f64,
      duration_ms: f64,
      stop: [f64; 2],
      samples: Vec<PosSample>,
      mover_vision: Option<Vec<VisionSample>>,
  }
  ```
- Produces (TS): `MoveStream` wire shape (snake_case) parsed by the Zod schema; camelCase client interface added in Task 5.

- [ ] **Step 1: Write the failing protocol round-trip test** in `protocol.rs` tests: construct a `ServerMsg::MoveStream { … samples: vec![PosSample{t_ms:0.0,pos:[0.0,0.0]}], mover_vision: Some(vec![VisionSample{t_ms:0.0,polygons:vec![vec![[0.0,0.0],[1.0,0.0],[1.0,1.0]]]}]) … }`, `serde_json::to_string` then `from_str`, assert equality; assert the tag serializes as `"move_stream"`.

- [ ] **Step 2: Run to verify it fails**

Run: `cd src/server && cargo test ws::protocol`
Expected: FAIL (variant/structs not defined).

- [ ] **Step 3: Add the structs + variant** in `protocol.rs` with the derives above; doc-comment each field (present-tense, constraint-led). `mover_vision: None` means "observer/no sweep". Keep `MoveError`.

- [ ] **Step 4: Regenerate ts-rs + run the round-trip test**

Run: `cd src/server && cargo test` (ts-rs export runs in the test that writes `src/types/generated`), then `cargo test ws::protocol`.
Expected: PASS; `src/types/generated/ServerMsg.ts` now contains `MoveStream`, `PosSample.ts`, `VisionSample.ts`.

- [ ] **Step 5: Mirror the Zod schema** in `src/client/core/src/wire.ts`: add `moveStreamSchema` (discriminator `"move_stream"`) with `request_id`, `token_id`, `mover`, `scene` (uuid strings), `start_server_ms`, `duration_ms` (numbers), `stop` (tuple), `samples` (array of `{ t_ms, pos: tuple }`), `mover_vision` (nullable array of `{ t_ms, polygons: array of array of tuple }`). Add to the `ServerMsg` discriminated union.

- [ ] **Step 6: Write + run a wire.test.ts case** parsing a representative `move_stream` JSON; assert it validates and a malformed one (missing `samples`) rejects.

Run: `pnpm --filter @shadowcat/core test wire` and `pnpm -r typecheck`.
Expected: PASS; drift guard green.

- [ ] **Step 7: Commit**

```bash
git add src/server/src/ws/protocol.rs src/types/generated src/client/core/src/wire.ts src/client/core/src/wire.test.ts
git commit -m "feat(m2): MoveStream protocol — PosSample/VisionSample + Zod mirror"
```

---

### Task 2: Server path sampler + position trajectory broadcast

**Files:**
- Create: `src/server/src/scene/move_stream.rs`
- Modify: `src/server/src/scene/mod.rs` (`mod move_stream;`)
- Modify: `src/server/src/ws/room.rs` (extend `execute_move` to also return samples), `src/server/src/ws/conn.rs` (`handle_move_request` broadcasts `MoveStream`)
- Test: `src/server/src/scene/move_stream.rs` (`#[cfg(test)]`), `src/server/src/ws/conn.rs` integration test

**Interfaces:**
- Produces:
  ```rust
  // scene/move_stream.rs — pure, no I/O.
  /// Time-tag the legal render-path into position samples for client playback.
  /// `cell` > 0; `duration_ms` >= 0. ~SAMPLES_PER_CELL per cell, capped at MAX_VISION_SAMPLES.
  /// Always includes the first and last vertex; samples are strictly increasing in t_ms.
  pub(crate) fn sample_path(path: &[(f64,f64)], cell: f64, duration_ms: f64) -> Vec<PosSamplePt>;
  pub(crate) struct PosSamplePt { pub t_ms: f64, pub pos: (f64,f64) }
  pub(crate) const MAX_VISION_SAMPLES: usize = 96;
  pub(crate) const SAMPLES_PER_CELL: f64 = 3.0;
  ```
- Consumes: M1 `MoveExecution { stop, render_path, duration_ms }` (room.rs).
- Produces (room.rs): `execute_move` additionally returns `samples: Vec<PosSamplePt>` (extend `MoveExecution`).

- [ ] **Step 1: Write failing sampler unit tests** in `move_stream.rs`:
  - `straight_two_cell_path_samples_endpoints_and_interior`: `path=[(0,0),(100,0),(200,0)]`, `cell=100`, `dur=1000` → first `t_ms==0 pos==(0,0)`, last `t_ms==1000 pos==(200,0)`, count ≈ `2*3+1` within ±1, t_ms strictly increasing.
  - `cap_bounds_samples`: a very long path (`> MAX_VISION_SAMPLES/3` cells) returns `<= MAX_VISION_SAMPLES` samples, still endpoints exact.
  - `zero_progress_returns_single_sample`: `path=[(0,0)]` or `dur==0` → exactly one sample at `t_ms==0`.
  - `arc_length_time_mapping`: an L-route `[(0,0),(100,0),(100,100)]` → the corner vertex's `t_ms ≈ 500` (half the arc length).

- [ ] **Step 2: Run to verify they fail**

Run: `cd src/server && cargo test scene::move_stream`
Expected: FAIL (module missing).

- [ ] **Step 3: Implement `sample_path`**: compute cumulative segment arc-lengths; total length `L`; target sample count `n = min(MAX_VISION_SAMPLES, max(2, ceil(L/cell * SAMPLES_PER_CELL)))`; place `n` samples at equal arc-length steps (param `s_i = i/(n-1) * L`), map each onto the polyline (linear interp within the containing segment), `t_ms_i = s_i / L * duration_ms`. Guard `L==0` / single point → one sample `{0.0, path[0]}`. De-dup exact-equal consecutive t (defensive).

- [ ] **Step 4: Run sampler tests**

Run: `cd src/server && cargo test scene::move_stream`
Expected: PASS.

- [ ] **Step 5: Extend `execute_move`** (`room.rs`) to call `sample_path(&outcome.render_path, cell, duration_ms)` after computing `duration_ms`, and return the samples in `MoveExecution`. Zero-progress short-circuit returns `samples: vec![{0.0, start}]`.

- [ ] **Step 6: Broadcast `MoveStream` from `handle_move_request`** (`conn.rs`): on `Ok(exec)`, build `ServerMsg::MoveStream { request_id, token_id, mover: ctx.user_id, scene: scene_id, start_server_ms: now_millis() as f64, duration_ms: exec.duration_ms, stop: [exec.stop.0, exec.stop.1], samples: exec.samples.iter().map(|s| PosSample{t_ms:s.t_ms, pos:[s.pos.0,s.pos.1]}).collect(), mover_vision: None }` and `room.broadcast_aux(frame)`. Return **no** `etx` frame on success; on `Err` return `MoveError` to `etx` (unchanged). (`mover_vision` filled in Task 3; clipping in Task 4 — at this task the broadcast is unclipped/full, validated by the integration test below before Task 4 lands.)

- [ ] **Step 7: Write + run an integration test** (`conn.rs` tests, mirror `handle_move_request_executes_and_replies_to_requester`): drive `handle_move_request` for a mover, assert it returns no success `etx` frame and that a `broadcast_aux` `MoveStream` is observable on a second `room.subscribe()` receiver with `samples` non-empty terminating at the goal; a rejected move still yields `MoveError` to the requester.

Run: `cd src/server && cargo test ws::conn` and `cargo fmt && cargo clippy --all-targets -- -D warnings`.
Expected: PASS, clean.

- [ ] **Step 8: Commit**

```bash
git add src/server/src/scene/move_stream.rs src/server/src/scene/mod.rs src/server/src/ws/room.rs src/server/src/ws/conn.rs
git commit -m "feat(m2): path sampler + MoveStream broadcast (positions; vision/clip follow)"
```

---

### Task 3: Mover vision trajectory (full-wall raycast per sample)

**Files:**
- Modify: `src/server/src/scene/mod.rs` (new viewpoint-set vision accessor)
- Modify: `src/server/src/ws/room.rs` (compute `mover_vision` in `execute_move`), `src/server/src/ws/conn.rs` (pass it into the `MoveStream`)
- Test: `src/server/src/scene/mod.rs` tests

**Interfaces:**
- Produces:
  ```rust
  // scene/mod.rs — reuses sight_walls + vision::visibility_polygon (FULL wall set, incl. gm_only).
  /// Vision polygons for `user` in `scene` if the user's moving token were at `viewpoint`:
  /// the moving token's polygon from `viewpoint` unioned with the user's OTHER owned tokens'
  /// polygons (their committed positions). Scene-local. Empty when the user owns no token here.
  pub(crate) fn player_vision_polygons_at(
      &self, user: Uuid, scene: Uuid, moving_token: Uuid, viewpoint: (f64,f64),
  ) -> Vec<Vec<vision::P>>;
  ```
- Consumes: Task 2 `samples` (room.rs computes a `VisionSample` per sample position).

- [ ] **Step 1: Write failing tests** in `scene/mod.rs` tests:
  - `vision_at_grows_as_token_advances`: a scene with one `blocksSight` wall and a player token; `player_vision_polygons_at` at two viewpoints (near vs past the wall) yields different polygons (the post-wall viewpoint reveals area the near one occludes).
  - `vision_at_uses_full_wall_set`: a `gm_only`-permissioned `blocksSight` wall still occludes a non-GM's `player_vision_polygons_at` (mirror the M9b "gm_only wall still occludes" invariant).
  - `vision_at_empty_when_user_owns_no_token`: returns empty.

- [ ] **Step 2: Run to verify fail**

Run: `cd src/server && cargo test scene::tests` (or the new test names)
Expected: FAIL (method missing).

- [ ] **Step 3: Implement `player_vision_polygons_at`**: gather the user's owned tokens in `scene`; for `moving_token` use `viewpoint`, for others their committed `(x,y)`; for each, `let walls = self.sight_walls(scene); let bound = vision::bound_for(vp, &walls, VISION_BOUND_MARGIN); out.push(vision::visibility_polygon(vp, &walls, bound))`. (Same primitives as `player_vision_polygons`; `sight_walls` already includes gm_only.)

- [ ] **Step 4: Run the tests**

Run: `cd src/server && cargo test scene::`
Expected: PASS.

- [ ] **Step 5: Compute `mover_vision` in `execute_move`** (under the same ECS read used for the executor; drop the lock before commit): for each `samples[k]`, `let polys = scene.player_vision_polygons_at(ctx.user_id, scene_id, token, samples[k].pos);` → `VisionSamplePt { t_ms, polygons: polys }`. GM mover (`Unrestricted`, sees all) → `mover_vision = None` (no fog to sweep). Return `mover_vision: Option<Vec<VisionSamplePt>>` in `MoveExecution`.

- [ ] **Step 6: Thread into the `MoveStream`** (`conn.rs`): map `mover_vision` to `Vec<VisionSample>` (`polygons: Vec<Vec<[f64;2]>>`). Cap each polygon's vertex count consistent with the existing vision payload bound; if a sample exceeds it, keep within bound (fail-closed under-reveal).

- [ ] **Step 7: Run + lint**

Run: `cd src/server && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src/server/src/scene/mod.rs src/server/src/ws/room.rs src/server/src/ws/conn.rs
git commit -m "feat(m2): mover vision trajectory — full-wall raycast per path sample"
```

---

### Task 4: Per-recipient egress clip (the secrecy boundary)

**Files:**
- Modify: `src/server/src/ws/conn.rs` (`egress_loop` — new `MoveStream` branch)
- Test: `src/server/src/ws/conn.rs` integration tests

**Interfaces:**
- Consumes: the broadcast `MoveStream` (full `samples` + `mover_vision`), the connection's effective view-ctx, the recipient's cached/derived vision polygons.
- Produces: a per-recipient `MoveStream` written to the sink (or suppressed).

- [ ] **Step 1: Write failing integration tests** (mirror the egress/`enrich_vision_explored` test setup):
  - `mover_receives_full_stream`: the mover's egress passes all `samples` and keeps `mover_vision`.
  - `observer_behind_wall_gets_gap_or_suppressed`: an observer whose vision excludes the whole path receives **no** `MoveStream` (suppressed); an observer who sees only the first half receives `samples` truncated to the visible prefix with `mover_vision == None`.
  - `secret_wall_area_never_streamed`: an observer separated from the move by a `gm_only` wall is suppressed (the clip uses the observer's authoritative vision, which the secret wall bounds).

- [ ] **Step 2: Run to verify fail**

Run: `cd src/server && cargo test ws::conn`
Expected: FAIL (no clip branch — broadcast is currently full to all).

- [ ] **Step 3: Implement the `MoveStream` egress branch** in `egress_loop`'s `rx.recv()` arm, before `send_filtered`. Determine `effective_user` (own `ctx.user_id`; a scene_sub `as_user`/see-as target is out of scope here — use own ctx). If `effective_user == frame.mover` → forward the frame unchanged (full samples + `mover_vision`). Else: read the recipient's authoritative vision polygons for `frame.scene` (reuse the most recent `vision` scene-sub payload if present; else compute `player_vision_polygons(effective_user)` under the ECS read guard, then **drop the guard**); keep only `samples` whose `pos` is inside any polygon (`vision::point_in_poly`); set `mover_vision = None`. If the kept set is empty → **do not send** (suppress). Otherwise send the rewritten `MoveStream`. Fail closed: no derivable vision → empty → suppress. **No lock across await** (read polygons, drop guard, then `sink.send`).

- [ ] **Step 4: Run the tests**

Run: `cd src/server && cargo test ws::conn`
Expected: PASS.

- [ ] **Step 5: Verify the no-leak invariant explicitly** — add an assertion in `secret_wall_area_never_streamed` that the suppressed observer's sink received zero `move_stream` frames (not merely an empty-sample one).

Run: `cd src/server && cargo test ws::conn && cargo clippy --all-targets -- -D warnings`
Expected: PASS, clean.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/ws/conn.rs
git commit -m "feat(m2): per-recipient MoveStream egress clip — leak-free observer vision"
```

---

### Task 5: Client position playback (broadcast-driven animation)

**Files:**
- Modify: `src/client/core/src/ws-client.ts` (dispatch + `MoveStream` interface + `onMoveStream` seam), `src/client/core/src/index.ts` (export)
- Modify: `src/client/render/src/token-animator.ts`, `token-view.ts`, `engine.ts`, `types.ts` (play time-tagged samples with gaps)
- Modify: `src/client/ui-kit/src/sceneInteraction.ts`, `src/client/shell/src/lib/worldSession.svelte.ts`, `src/client/ui-kit/src/appContext.ts`, `src/modules/scene-tools/src/controller.svelte.ts`
- Test: `ws-client.test.ts`, `token-animator.test.ts`, `sceneInteraction.test.ts`

**Interfaces:**
- Produces (TS):
  ```ts
  interface MoveSample { tMs: number; pos: [number, number] }
  interface MoveStream {
    requestId: string; tokenId: string; mover: string; scene: string;
    startServerMs: number; durationMs: number; stop: [number, number];
    samples: MoveSample[];                 // visible samples (server-clipped)
    moverVision: { tMs: number; polygons: [number, number][][] }[] | null;
  }
  // WsClient: onMoveStream(cb: (s: MoveStream) => void): () => void
  // Animator seam: animateSamples(id, samples: {tMs;pos}[], durationMs, startServerMs)
  ```
- Consumes: the `move_stream` wire frame (Task 1); the existing time-sync (`server_time` offset already tracked by `WsClient`).

- [ ] **Step 1: Write failing ws-client test** (`ws-client.test.ts`): feed a `move_stream` frame whose `request_id` matches a pending `moveRequest` → the promise resolves AND a registered `onMoveStream` callback fires with camelCase fields. Feed a `move_stream` with an unknown `request_id` (observer) → no pending rejection, callback still fires.

- [ ] **Step 2: Run to verify fail**

Run: `pnpm --filter @shadowcat/core test ws-client`
Expected: FAIL.

- [ ] **Step 3: Implement the `move_stream` dispatch**: resolve+delete a matching pending entry (mover success signal); **always** invoke the `onMoveStream` listeners with the mapped `MoveStream`. Add `onMoveStream` registration (mirror existing listener seams) and the `MoveStream`/`MoveSample` interfaces; export from `index.ts`. `move_error` unchanged.

- [ ] **Step 4: Run ws-client test**

Run: `pnpm --filter @shadowcat/core test ws-client`
Expected: PASS.

- [ ] **Step 5: Write failing animator test** (`token-animator.test.ts`): `animateSamples("t1", [{tMs:0,pos:[0,0]},{tMs:500,pos:[300,0]}], 1000, <start>)` positions the token at `[0,0]` at clock 0 and interpolates toward `[300,0]` by `tMs`; a **gap** (samples `[{0,[0,0]},{800,[800,0]}]` with no sample in `(0,800)`) hides the token during the gap and shows it at the endpoints.

- [ ] **Step 6: Implement `animateSamples`** in `TokenAnimator` (+ forward through `token-view.ts`, `engine.ts`, `types.ts` `SceneToolHost`): drive position by the server clock (`startServerMs` + server-time offset); interpolate between adjacent samples by `tMs`; when the current clock falls in a span between two samples whose `tMs` gap exceeds the nominal interval (`durationMs / expectedSamples`, or a span flagged by absence), set token `visible=false`; settle at the last sample. Keep the M1 `animateAlongPath` for any remaining single-path callers, or re-express it via `animateSamples`.

- [ ] **Step 7: Run animator test**

Run: `pnpm --filter @shadowcat/render test token-animator`
Expected: PASS.

- [ ] **Step 8: Wire `onMoveStream` → host playback**: in `worldSession`/`appContext`, subscribe to `onMoveStream` and call `sceneInteraction.animateSamples(tokenId, samples, durationMs, startServerMs)` (add the bridge method, mirror `animateAlongPath`); `scene-tools` route-commit stops animating from the `moveRequest` promise (playback is now broadcast-driven; it still surfaces `MoveError`). Add a `sceneInteraction.test.ts` forward-through case.

- [ ] **Step 9: Run client suites + typecheck**

Run: `pnpm -r test` and `pnpm -r typecheck`
Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add src/client/core/src/ws-client.ts src/client/core/src/index.ts src/client/render/src src/client/ui-kit/src src/client/shell/src src/modules/scene-tools/src
git commit -m "feat(m2): client MoveStream playback — broadcast-driven, time-synced, gap-aware"
```

---

### Task 6: Client vision-sweep fog (snap)

**Files:**
- Modify: `src/client/render/src/engine.ts`, `pixi-backend.ts`, `compositor.ts`, `types.ts` (feed a time-varying vision polygon into the fog during a mover's animation)
- Test: `src/client/render/src/engine.test.ts` / `compositor.test.ts`

**Interfaces:**
- Consumes: `MoveStream.moverVision` (Task 5) + the animation clock.
- Produces: per-clock fog updates via the existing `compositor.setVisibility` / `PixiBackend.setVisibility` path (the existing polygon fog renderer — no new render layer).

- [ ] **Step 1: Write failing test** (`engine.test.ts`): given a mover `MoveStream` with two `moverVision` samples (small polygon → larger polygon), advancing the animation clock past the second sample's `tMs` applies the **larger** polygon to the fog (assert via the mock backend's recorded `setVisibility` `visible` polygon), and at animation end the fog reverts to the last `vision` subscription payload.

- [ ] **Step 2: Run to verify fail**

Run: `pnpm --filter @shadowcat/render test engine`
Expected: FAIL.

- [ ] **Step 3: Implement the snap feed**: when a mover-owned `MoveStream` plays, on each frame select the `moverVision` sample with the greatest `tMs <= clock` and feed its polygons as a `VisibilityInput{ mode:"masked", visible:<polys>, explored:<current> }` to the compositor; on completion, clear the override and re-apply the last derived `vision` payload (the existing `lastInput`/`renderVisibility` path). Observers' fog is untouched (they receive no `moverVision`). Guard: only the mover (frame.mover == own user) drives this.

- [ ] **Step 4: Run the test + typecheck**

Run: `pnpm --filter @shadowcat/render test && pnpm -r typecheck`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/client/render/src
git commit -m "feat(m2): mover vision-sweep fog (snap) — progressive reveal during animation"
```

---

### Task 7: Fog cross-fade (smoothness enhancement)

**Files:**
- Modify: `src/client/render/src/pixi-backend.ts` (render-texture cross-fade between consecutive vision samples)
- Test: `src/client/render/src/pixi-backend` unit or a layers test

**Interfaces:**
- Consumes: consecutive `moverVision` samples + inter-sample interval.
- Produces: a smoothly cross-faded fog (no polygon morphing).

- [ ] **Step 1: Write a failing/behavioral test**: assert that between two vision samples the backend blends (alpha of an outgoing fog texture decreases toward the next sample's texture) — assert via the mock backend recording two render-texture handles + a blend factor that advances `0→1` across the interval. (If a true GL assert isn't feasible in jsdom, assert the cross-fade controller's computed blend factor as a pure function of clock vs sample bounds.)

- [ ] **Step 2: Run to verify fail**

Run: `pnpm --filter @shadowcat/render test`
Expected: FAIL.

- [ ] **Step 3: Implement the cross-fade**: rasterize each vision sample's fog into a PixiJS `RenderTexture`; hold the current + next sample textures; alpha-blend by `(clock - tCur)/(tNext - tCur)` clamped `[0,1]`; snap forward when crossing a sample boundary. Pure blend-factor helper extracted for unit testing. Falls back to the Task-6 snap if render textures are unavailable. No new dependency.

- [ ] **Step 4: Run + typecheck**

Run: `pnpm --filter @shadowcat/render test && pnpm -r typecheck`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/client/render/src
git commit -m "feat(m2): fog cross-fade between vision samples — smooth sweep, no morphing"
```

---

### Task 8: Docs + reviewed skill-update gate

**Files:**
- Modify: `.claude/skills/shadowcat-codebase-scene-rendering/SKILL.md`, `.claude/skills/shadowcat-codebase-realtime-sync/SKILL.md`, `.claude/skills/shadowcat-codebase-client-shell/SKILL.md`
- Modify: `docs/PLAN.md` (M10e / M2 status)

- [ ] **Step 1: Update `shadowcat-codebase-scene-rendering`** — add the `MoveStream` streamed-vision seam: `scene/move_stream.rs` sampler, `player_vision_polygons_at`, the full-wall-set raycast, the per-recipient egress clip as the secrecy boundary, the client fog-sweep playback. Cite the design doc + the no-leak / fail-closed invariants.

- [ ] **Step 2: Update `shadowcat-codebase-realtime-sync`** — `MoveStream` is an aux broadcast frame (no seq, like `ScenePing`) with a **per-recipient egress transform** (mover full; observer clipped; suppressed when occluded); `MoveError` stays mover-only.

- [ ] **Step 3: Update `shadowcat-codebase-client-shell`** — `onMoveStream` seam, broadcast-driven playback, server-clock alignment.

- [ ] **Step 4: Update `docs/PLAN.md`** — mark M2 (streamed continuous vision) done; note the deferred live-concurrency follow-up (TODO).

- [ ] **Step 5: Dispatch `shadowcat-spec-reviewer`** on the skill diffs to confirm they accurately capture the implemented change (no omission/drift/broken pointer). Address findings.

- [ ] **Step 6: Commit**

```bash
git add .claude/skills docs/PLAN.md
git commit -m "docs(m2): sync codebase skills + PLAN — streamed continuous vision"
```

---

## Buddy-check directives

This branch is **security-sensitive** — the per-recipient egress clip (Task 4) *is* the secrecy
boundary, and the mover vision trajectory (Task 3) raycasts the full wall set (secret walls). Per
the project two-reviewer gate, after all tasks:

- Run a **whole-branch buddy-check** (`buddy-checking` skill: two independent blind reviewers →
  debate to convergence) over the M2 diff, with explicit focus on:
  1. **No leak:** an observer never receives a hidden position sample or the mover's vision; a
     fully/partly-occluded move is suppressed/truncated; `gm_only` walls bound observer clips.
  2. **Parity / no fork (§13):** observer clipping uses the recipient's authoritative vision;
     the mover trajectory uses the same `sight_walls` + `visibility_polygon` as `player_vision_polygons`.
  3. **No lock across await** in the egress branch; **atomic state (M1) untouched.**
  4. **Determinism:** client plays the server `duration_ms`/`start_server_ms`; invents no timing.
- The reviewed **skill-update gate** (Task 8) is part of completion (dispatch `shadowcat-spec-reviewer`).
- Do **not** push/merge — the integration gate is the full M10 milestone.

## Self-Review notes

- Spec coverage: design §3 (Tasks 2–6), §4 secrecy (Task 4 + buddy-check), §5 sampling (Task 2),
  §6 fog snap/cross-fade (Tasks 6–7), §3.2 protocol (Task 1), §9 decomposition = these 8 tasks,
  §8 concurrency deferred (TODO, not built). Covered.
- Type consistency: `PosSample{t_ms,pos}` / `VisionSample{t_ms,polygons}` / `MoveStream` fields are
  identical across Task 1 (Rust+Zod), Task 5 (TS camelCase `MoveSample`/`MoveStream`), Tasks 6–7.
- `sample_path`/`player_vision_polygons_at`/`animateSamples`/`onMoveStream` names are stable across
  the tasks that produce and consume them.
