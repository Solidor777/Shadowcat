# M1 — Server-Authoritative Move Execution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make gated token moves server-authoritative and atomic — the client sends a `MoveRequest` (the exact previewed cell-path), the server validates it step-by-step, atomically commits the token to its stop location, returns a render-path to the mover for the cosmetic walk, and locks the token until it arrives — replacing M10e-5's optimistic client-chained route-commit.

**Architecture:** A new `MoveRequest`/`MoveExecuted` protocol pair. A pure `scene/move_exec.rs` executor validates the proposed path against `blocks_move` + the per-(user,scene) visible mask (supercover, incl. diagonal flankers) + a region-arrest hook, yielding the stop cell + render-path + the atomic position ops. `Room::publish`'s commit tail is extracted into `commit_ops` so the executor can write `start→stop` atomically **without** re-running the straight-line move gate (it has already gated each step). The mover receives `MoveExecuted` one-shot (like `Pathfind`); observers see only the atomic `Event`. A per-room `moving` lock rejects new requests for an in-flight token. Client: the measure-tool route-commit sends a `MoveRequest` and animates the returned render-path via the retained M10e-5 animation engine; the optimistic dispatch + collinear-run chaining is removed.

**Tech Stack:** Rust (`shadowcat` crate; tokio, serde, ts-rs, hecs), TypeScript, Svelte 5, Vitest.

## Global Constraints

- **Server crate is `shadowcat`** (NOT `shadowcat-server`).
- **Atomic game state:** a move changes the token's authoritative position exactly once (start → stop location). No intermediate authoritative positions. Reload/resync resolves the token at its stop location. (Spec §2.)
- **Render-only animation:** the render-path is cosmetic; it carries no game state. (Spec §2.)
- **Move lock:** once approved, the token *will* reach the stop location and cannot accept another move until it arrives (deterministic duration). New `MoveRequest`s for a `moving` token are rejected. (Spec §2.4.)
- **M1 render-path is mover-only.** No per-recipient clipping, no full broadcast (that is M2). Observers see the atomic position only. (Spec §4, §7.)
- **The executor gates each step itself; the atomic write must NOT re-run the straight-line supercover gate** (a routed `start→stop` may cross a wall the path avoided). (Spec §3.)
- **Vision mask reuse:** validation reuses the EXACT M10e-4 gate primitives — `blocks_move`, `visible_cells(user, scene, lenient)`, `supercover_cells` — never a fork. The move gate and the egress secrecy mask are the same mask (§13 invariant). GM / `Unrestricted` skip the mask check.
- **`dist/` must be built before any server `cargo` build** (rust-embed validates `../../dist/`): run `pnpm --filter @shadowcat/ui build` before `cargo test`.
- **Cross-platform** (macOS/Linux/Windows server; desktop + touch browsers). **No `console.log`/`dbg!`**; diagnostics via `tracing`/the project logger.
- **ts-rs:** protocol types are exported to `src/types/generated/`; the generation step is part of the protocol task.
- **Region system stays M10g.** M1 adds only the per-step arrest *hook* (no registered arresting regions → no behavior change).

---

## File Structure

**Server:**
- **Modify** `src/server/src/ws/protocol.rs` — add `ClientMsg::MoveRequest` + `ServerMsg::MoveExecuted`.
- **Create** `src/server/src/scene/move_exec.rs` — pure `execute_move` path validation → stop + render-path + ops + the region-arrest hook trait point.
- **Modify** `src/server/src/scene/mod.rs` — `pub mod move_exec;` + any `pub(crate)` exposure the executor needs (reuses existing `blocks_move`, `visible_cells`, `token_move`, `scene_grid_sizes`, `resolve_scene`).
- **Modify** `src/server/src/ws/room.rs` — extract `commit_ops` from `publish`; add `execute_move`; add the `moving` lock state + helpers.
- **Modify** `src/server/src/ws/conn.rs` — `MoveRequest` match arm + `handle_move_request`.
- **Generated** `src/types/generated/ClientMsg.ts`, `ServerMsg.ts` (ts-rs).

**Client:**
- **Modify** `src/client/core/src/ws-client.ts` — `moveRequest(...)` correlated request + `MoveExecuted` handling; `MoveExecuted` result type.
- **Modify** `src/client/ui-kit/src/appContext.ts` — `moveRequest` seam.
- **Modify** `src/client/shell/src/lib/worldSession.svelte.ts` — wire `moveRequest` through AppContext.
- **Modify** `src/modules/scene-tools/src/controller.svelte.ts` — rework `commitRoute` to send `MoveRequest` + animate the returned render-path; drop optimistic dispatch + `collinearRuns`.
- **Remove** `src/modules/scene-tools/src/path-runs.ts` + `path-runs.test.ts` (dead under the new model).
- **Revert** `src/server/src/ws/room.rs` test `route_commits_as_chained_runs_around_a_wall` (tested the removed client-chaining).

---

### Task 1: Protocol frames — `MoveRequest` / `MoveExecuted`

**Files:**
- Modify: `src/server/src/ws/protocol.rs`
- Test: `src/server/src/ws/protocol.rs` (in-module `#[cfg(test)]`)
- Generated: `src/types/generated/ClientMsg.ts`, `ServerMsg.ts`

**Interfaces:**
- Produces:
  - `ClientMsg::MoveRequest { request_id: Uuid, scene: Uuid, token_id: Uuid, path: Vec<[f64; 2]> }` — `path` is the exact previewed cell-center scene points (start … goal).
  - `ServerMsg::MoveExecuted { request_id: Uuid, token_id: Uuid, stop: [f64; 2], render_path: Vec<[f64; 2]>, duration_ms: f64 }`.
  - `ServerMsg::MoveError { request_id: Uuid, message: String }` — request rejected (token moving / not a token / malformed).

**Design notes:** Mirror the existing `Pathfind` (ClientMsg) and `PathResult`/`PathError` (ServerMsg) variants exactly — same `#[serde(rename_all = "snake_case")]` / `#[serde(tag = ...)]` convention, same `#[derive(..., TS)]` + `#[ts(export, export_to = "...")]` attributes the sibling variants use. Read the `Pathfind`/`PathResult` definitions first and copy their attribute shape.

- [ ] **Step 1: Write the failing test** (append to protocol.rs tests, mirroring the existing `Pathfind` round-trip test):

```rust
#[test]
fn move_request_and_executed_round_trip() {
    let req = ClientMsg::MoveRequest {
        request_id: Uuid::from_u128(1),
        scene: Uuid::from_u128(2),
        token_id: Uuid::from_u128(3),
        path: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0]],
    };
    let wire = serde_json::to_string(&req).unwrap();
    let back: ClientMsg = serde_json::from_str(&wire).unwrap();
    assert!(matches!(back, ClientMsg::MoveRequest { .. }));

    let ok = ServerMsg::MoveExecuted {
        request_id: Uuid::from_u128(1),
        token_id: Uuid::from_u128(3),
        stop: [100.0, 100.0],
        render_path: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0]],
        duration_ms: 500.0,
    };
    let w2 = serde_json::to_string(&ok).unwrap();
    assert!(w2.contains("move_executed"));
    let err = ServerMsg::MoveError { request_id: Uuid::from_u128(1), message: "token is moving".into() };
    assert!(serde_json::to_string(&err).unwrap().contains("move_error"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat move_request_and_executed_round_trip`
Expected: FAIL — variants not defined.

- [ ] **Step 3: Add the variants** to `ClientMsg` and `ServerMsg`, copying the exact attribute style of the neighbouring `Pathfind`/`PathResult` variants (snake_case tags `move_request` / `move_executed` / `move_error`; `Vec<[f64; 2]>` for points as `PathResult` uses for `path`).

- [ ] **Step 4: Regenerate ts-rs bindings + verify pass**

Run: `cargo test -p shadowcat export_bindings` (or the project's ts-rs export test/command — check how `Pathfind` types are generated; the `#[ts(export)]` tests emit the `.ts` files). Then `cargo test -p shadowcat move_request_and_executed_round_trip`.
Expected: PASS; `src/types/generated/ClientMsg.ts` now has `MoveRequest`, `ServerMsg.ts` has `MoveExecuted`/`MoveError` (snake_case, `[number, number][]` for points — verify parity with `PathResult`).

- [ ] **Step 5: Commit**

```bash
git add src/server/src/ws/protocol.rs src/types/generated/ClientMsg.ts src/types/generated/ServerMsg.ts
git commit -m "feat(m1): MoveRequest/MoveExecuted/MoveError protocol frames"
```

---

### Task 2: Move executor — `scene/move_exec.rs`

**Files:**
- Create: `src/server/src/scene/move_exec.rs`
- Modify: `src/server/src/scene/mod.rs` (add `pub mod move_exec;`)
- Test: `src/server/src/scene/move_exec.rs` (in-module `#[cfg(test)]`)

**Interfaces:**
- Consumes (existing on `SceneEcs`): `token_move`-style position read; `blocks_move(scene, a0, a1)`; `resolve_scene(scene)` → `{ movement_restriction, partial_cell_leniency }`; `visible_cells(user, scene, lenient)` → `BTreeSet<(i32,i32)>`; `scene_grid_sizes()` → cell size; `crate::scene::movement::supercover_cells(a0, a1, cell)`.
- Produces:
  ```rust
  pub struct MoveOutcome {
      pub stop: (f64, f64),            // scene coords of the stop cell center
      pub render_path: Vec<(f64, f64)>,// start..=stop, the legal prefix actually walked
      pub truncated: bool,             // true if the move stopped before the requested goal
  }
  pub enum MoveReject { NotAToken, EmptyPath, TooLong, Degenerate }
  pub fn execute_move(
      ecs: &SceneEcs, user: Uuid, scene: Uuid, token: Uuid, path: &[(f64, f64)],
  ) -> Result<MoveOutcome, MoveReject>;
  ```

**Design notes (binding):**
- `path[0]` MUST equal the token's current committed position (within EPSILON); else `MoveReject::Degenerate` (fail closed — the client must request from the token's real position).
- Bounds: `path.len()` ≤ `MAX_MOVE_PATH` (= 256); each consecutive pair must be grid-adjacent (king-step: `|di| ≤ 1 && |dj| ≤ 1`, computed from cell size); else `TooLong`/`Degenerate`. Fail closed.
- Walk the path step by step from `path[0]`. For each step `prev → next`:
  1. `blocks_move(scene, prev, next)` → if blocked, STOP at `prev`.
  2. For a non-GM, non-`Unrestricted` mover: `supercover_cells(prev, next, cell)` must all be in `visible_cells(user, scene, lenient)` (∪ explored for `Revealed` — see note); else STOP at `prev`. (Mirror the M10e-4 gate exactly; `lenient = partial_cell_leniency`.)
  3. **Region-arrest hook:** `region_arrests(ecs, scene, next)` (Task-2 stub returning `false`; M3/M10g implement) → if it arrests, STOP at `next` (the token enters the arrest cell, then halts).
  - The first failing step truncates: `stop = prev` (or `next` for an arrest), `render_path = path[0..=stopIndex]`, `truncated = true`.
- If all steps pass: `stop = path.last()`, `render_path = path.to_vec()`, `truncated = false`.
- GM and `Unrestricted` skip the mask check (step 2) but STILL honor `blocks_move` (step 1) and the arrest hook (step 3). (GM is exempt from the M10e-4 *mask* gate, not from walls — mirror `Room::publish`.)
- `Revealed` mode: this task validates against `visible_cells` only and leaves the explored-union to the room layer (explored is async/DB-backed and must not be fetched under a sync ECS borrow). The executor takes the already-resolved visible set... see Step 3 — to keep the executor pure and lock-free, pass the precomputed `visible: &BTreeSet<(i32,i32)>` in rather than calling `visible_cells` inside (the room computes it off the read lock, mirroring `publish`'s `visible_cache`). Revise the signature to:
  ```rust
  pub fn execute_move(
      ecs: &SceneEcs, scene: Uuid, token: Uuid, path: &[(f64,f64)],
      restriction: MovementRestriction, visible: &BTreeSet<(i32,i32)>, cell: f64,
  ) -> Result<MoveOutcome, MoveReject>;
  ```
  (The room resolves `restriction`, `visible`, `cell` exactly as `publish` does, then calls this.) GM-ness is folded into `restriction == Unrestricted` by the caller (the room passes `Unrestricted` for a GM, mirroring `publish`'s gate skip).

- [ ] **Step 1: Write the failing tests**

```rust
// src/server/src/scene/move_exec.rs  (#[cfg(test)] mod tests)
// Build a SceneEcs with a scene (grid 100), a token at (0,0), and a blocksMove wall
// between (0,0)->(0,100) but NOT around an L. (Reuse the from_documents test helper +
// the wall-doc shape used by scene/mod.rs movement tests.)
use std::collections::BTreeSet;
use crate::scene::MovementRestriction;

#[test]
fn full_clear_path_reaches_goal() {
    let ecs = /* scene + token@ (0,0), no walls */;
    let visible: BTreeSet<(i32,i32)> = (0..3).flat_map(|i| (0..3).map(move |j| (i,j))).collect();
    let out = execute_move(&ecs, scene, token, &[(0.0,0.0),(100.0,0.0),(100.0,100.0)],
        MovementRestriction::Visible, &visible, 100.0).unwrap();
    assert_eq!(out.stop, (100.0,100.0));
    assert_eq!(out.render_path.len(), 3);
    assert!(!out.truncated);
}

#[test]
fn wall_truncates_at_last_legal_cell() {
    let ecs = /* wall blocks (100,0)->(100,100) */;
    let visible: BTreeSet<(i32,i32)> = /* all cells visible */;
    let out = execute_move(&ecs, scene, token, &[(0.0,0.0),(100.0,0.0),(100.0,100.0)],
        MovementRestriction::Visible, &visible, 100.0).unwrap();
    assert_eq!(out.stop, (100.0,0.0)); // stopped before the wall
    assert!(out.truncated);
    assert_eq!(out.render_path, vec![(0.0,0.0),(100.0,0.0)]);
}

#[test]
fn unseen_cell_truncates_under_visible_restriction() {
    let ecs = /* no walls */;
    let mut visible: BTreeSet<(i32,i32)> = BTreeSet::new();
    visible.insert((0,0)); visible.insert((1,0)); // (1,1) NOT visible
    let out = execute_move(&ecs, scene, token, &[(0.0,0.0),(100.0,0.0),(100.0,100.0)],
        MovementRestriction::Visible, &visible, 100.0).unwrap();
    assert_eq!(out.stop, (100.0,0.0));
    assert!(out.truncated);
}

#[test]
fn unrestricted_ignores_mask_but_not_walls() {
    let ecs = /* wall blocks (100,0)->(100,100) */;
    let empty: BTreeSet<(i32,i32)> = BTreeSet::new();
    let out = execute_move(&ecs, scene, token, &[(0.0,0.0),(100.0,0.0),(100.0,100.0)],
        MovementRestriction::Unrestricted, &empty, 100.0).unwrap();
    assert_eq!(out.stop, (100.0,0.0)); // mask ignored, wall still stops it
}

#[test]
fn rejects_path_not_starting_at_token() {
    let ecs = /* token @ (0,0) */;
    let v: BTreeSet<(i32,i32)> = BTreeSet::new();
    assert!(matches!(
        execute_move(&ecs, scene, token, &[(500.0,0.0),(600.0,0.0)], MovementRestriction::Unrestricted, &v, 100.0),
        Err(MoveReject::Degenerate)));
}

#[test]
fn rejects_overlong_or_nonadjacent_path() {
    let ecs = /* token @ (0,0) */;
    let v: BTreeSet<(i32,i32)> = BTreeSet::new();
    // non-adjacent jump
    assert!(matches!(
        execute_move(&ecs, scene, token, &[(0.0,0.0),(500.0,0.0)], MovementRestriction::Unrestricted, &v, 100.0),
        Err(MoveReject::Degenerate)));
}
```

> The `/* … */` placeholders are scene-construction; build them with the existing `SceneEcs::from_documents` + the wall-doc JSON shape used by `scene/mod.rs`'s `token_move`/`blocks_move` tests (read those first and reuse verbatim). The assertions are the binding contract.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat move_exec`
Expected: FAIL — `execute_move` not defined.

- [ ] **Step 3: Implement `execute_move`** in `move_exec.rs`:

```rust
use std::collections::BTreeSet;
use uuid::Uuid;
use crate::scene::{SceneEcs, MovementRestriction, movement::supercover_cells};

const MAX_MOVE_PATH: usize = 256;
const EPS: f64 = 1e-6;

pub struct MoveOutcome { pub stop: (f64,f64), pub render_path: Vec<(f64,f64)>, pub truncated: bool }
pub enum MoveReject { NotAToken, EmptyPath, TooLong, Degenerate }

/// Region-arrest hook (M3/M10g). Returns true if a region halts a token entering `cell`.
fn region_arrests(_ecs: &SceneEcs, _scene: Uuid, _cell: (f64,f64)) -> bool { false }

pub fn execute_move(
    ecs: &SceneEcs, scene: Uuid, token: Uuid, path: &[(f64,f64)],
    restriction: MovementRestriction, visible: &BTreeSet<(i32,i32)>, cell: f64,
) -> Result<MoveOutcome, MoveReject> {
    if path.len() < 2 { return Err(MoveReject::EmptyPath); }
    if path.len() > MAX_MOVE_PATH { return Err(MoveReject::TooLong); }
    if !cell.is_finite() || cell <= 0.0 { return Err(MoveReject::Degenerate); }
    if path.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) { return Err(MoveReject::Degenerate); }
    // path[0] must be the token's current committed position.
    let cur = ecs.token_position(token).ok_or(MoveReject::NotAToken)?; // see note below
    if (cur.0 - path[0].0).abs() > EPS || (cur.1 - path[0].1).abs() > EPS {
        return Err(MoveReject::Degenerate);
    }
    let to_cell = |p: (f64,f64)| -> (i32,i32) { ((p.0 / cell).floor() as i32, (p.1 / cell).floor() as i32) };
    let mask = !matches!(restriction, MovementRestriction::Unrestricted);
    let lenient = matches!(restriction, MovementRestriction::Visible | MovementRestriction::Revealed);
    let mut stop_index = 0usize;
    for i in 1..path.len() {
        let prev = path[i-1]; let next = path[i];
        // adjacency guard (king-step)
        let (pc, nc) = (to_cell(prev), to_cell(next));
        if (pc.0 - nc.0).abs() > 1 || (pc.1 - nc.1).abs() > 1 { return Err(MoveReject::Degenerate); }
        // 1. wall
        if ecs.blocks_move(scene, prev, next) { break; }
        // 2. vision mask (supercover incl. diagonal flankers)
        if mask {
            let Some(cells) = supercover_cells(prev, next, cell) else { break; };
            let _ = lenient; // supercover already corner-inclusive per movement.rs; lenient affects sampling upstream
            if !cells.iter().all(|c| visible.contains(c)) { break; }
        }
        // 3. region-arrest hook
        if region_arrests(ecs, scene, next) { stop_index = i; break; }
        stop_index = i;
    }
    let render_path = path[0..=stop_index].to_vec();
    Ok(MoveOutcome { stop: path[stop_index], render_path, truncated: stop_index + 1 < path.len() })
}
```

> Add a small `pub(crate) fn token_position(&self, token: Uuid) -> Option<(f64,f64)>` to `SceneEcs` (read `/system/x`,`/system/y` from the token entity — factor out of the existing `token_move` which already reads `cx,cy`). If `visible_cells`'s `lenient` actually changes the mask *cells* (it does — corner sampling), the room passes the matching `visible` set; `execute_move` does not re-sample (it consumes the precomputed set), so `lenient` is informational here. Confirm against `movement.rs`/`mod.rs` that `supercover_cells` + the passed `visible` reproduce the M10e-4 decision exactly (parity is the §13 invariant).

- [ ] **Step 4: Run tests + clippy**

Run: `cargo test -p shadowcat move_exec && cargo clippy -p shadowcat --all-targets -- -D warnings`
Expected: PASS, clean.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/scene/move_exec.rs src/server/src/scene/mod.rs
git commit -m "feat(m1): move executor — per-step path validation -> stop + render-path (reuses M10e-4 gate primitives)"
```

---

### Task 3: Extract `commit_ops` from `Room::publish`

**Files:**
- Modify: `src/server/src/ws/room.rs`
- Test: `src/server/src/ws/room.rs` (existing publish tests must stay green; add one for `commit_ops` directly)

**Interfaces:**
- Produces: `async fn commit_ops(&self, repo: &dyn Repository, ctx: &PermissionContext, ops: Vec<Operation>, ts: i64) -> Result<Command, DataError>` — the apply_intent → ECS-hydrate → ring → broadcast tail of `publish`, with NO move gate. `publish` calls it after its gate.

**Design notes:** Pure refactor. Move the body of `publish` from `let cmd = repo.apply_intent(...)` (room.rs:279) through the broadcast + stats lines into `commit_ops`. `publish` keeps the gate (lines ~177–278) then `return self.commit_ops(repo, ctx, ops, ts).await;`. `commit_ops` acquires the same `publish_guard` serialization that `publish` holds — verify the guard is acquired once and not double-acquired (if `publish` already holds it, factor the guard so both the gate and `commit_ops` run under a single acquisition; the simplest shape is `publish` acquires the guard, runs the gate, then inlines the same tail — i.e. `commit_ops` is a *private helper that assumes the guard is held*, named `commit_ops_locked`, and `execute_move` (Task 4) acquires the guard then calls it). Read the top of `publish` to see where `publish_guard` is acquired and preserve single-acquisition.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn commit_ops_writes_and_broadcasts_without_gating() {
    let (repo, world_id, ctx) = repo_with_world().await;
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
    let (mut rx, _) = room.subscribe();
    // A create op committed via commit_ops broadcasts an Event and bumps seq.
    let op = /* a create op for a scene doc, mirroring publish_allocates_seq_buffers_and_broadcasts */;
    let cmd = room.commit_ops(repo.as_ref(), &ctx, vec![op], now_millis()).await.unwrap();
    assert_eq!(room.current_seq(), cmd.seq);
    assert!(matches!(&*rx.recv().await.unwrap(), ServerMsg::Event { .. }));
}
```

> Reuse the op/fixture shape from the existing `publish_allocates_seq_buffers_and_broadcasts` test (read it first).

- [ ] **Step 2: Run test to verify it fails** (`commit_ops` not defined)

Run: `cargo test -p shadowcat commit_ops_writes_and_broadcasts`
Expected: FAIL.

- [ ] **Step 3: Refactor** — extract the tail into `commit_ops` (or `commit_ops_locked` per the guard note), have `publish` call it, expose `commit_ops` at the visibility Task 4 needs.

- [ ] **Step 4: Run the FULL publish/movement test suite to prove behavior-preserving**

Run: `cargo test -p shadowcat ws::room && cargo test -p shadowcat commit_ops_writes_and_broadcasts`
Expected: PASS — all existing publish/movement-gate tests (`publish_*`, `movement_*`) unchanged + the new test green.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/ws/room.rs
git commit -m "refactor(m1): extract commit_ops tail from publish (gate-free authoritative write)"
```

---

### Task 4: `Room::execute_move` + the `moving` lock

**Files:**
- Modify: `src/server/src/ws/room.rs`
- Test: `src/server/src/ws/room.rs`

**Interfaces:**
- Consumes: `commit_ops` (Task 3); `move_exec::execute_move` (Task 2).
- Produces:
  ```rust
  pub struct MoveExecution { pub stop: (f64,f64), pub render_path: Vec<(f64,f64)>, pub duration_ms: f64 }
  pub async fn execute_move(
      &self, repo: &dyn Repository, ctx: &PermissionContext,
      scene: Uuid, token: Uuid, path: Vec<(f64,f64)>, ts: i64,
  ) -> Result<MoveExecution, DataError>;
  ```
  `DataError::Forbidden` when the token is currently `moving`, not a token, or the request is malformed.

**Design notes:**
- **Moving lock:** add `moving: Mutex<HashMap<Uuid, i64>>` to `Room` (token → move-end epoch-ms). On `execute_move`: lock, if `now < end` for this token → `Err(Forbidden)`; else proceed. After a successful commit, insert `token → now + duration_ms`. A request whose entry is expired (or absent) is allowed. (Lazy expiry — no timer. Reload has no in-memory lock, consistent with atomic state.)
- **Resolve gate inputs off the read lock** exactly like `publish`: take `scene.read()`, resolve `restriction`/`cell`/`visible_cells(user, scene, lenient)` and the token start, DROP the guard, then call the pure `execute_move`. For `Revealed`, union `repo.get_explored(...)` into `visible` AFTER dropping the guard (mirror `publish`'s `revealed_pending`/`explored_cache`; no lock across await).
- **Atomic write:** build the position ops `[{update token /system/x → stop.0}, {/system/y → stop.1}]` and `commit_ops(...)` them (NO gate — the executor already validated each step). If `stop == start` (zero-progress, fully blocked) skip the write (no-op move) but still return a `MoveExecution` with `render_path = [start]`, `duration_ms = 0`.
- **Duration:** `duration_ms = (render_path scene-distance / cell) / speed_cells_per_sec * 1000`, where `speed_cells_per_sec` comes from the world-settings `animation` (resolve via the ECS config side-table, same source `resolve_scene` uses; default 6). Distance = sum of segment lengths.
- GM detection: mirror `publish` — a GM passes `Unrestricted` to the executor (mask skipped).

- [ ] **Step 1: Write failing tests**

```rust
#[tokio::test]
async fn execute_move_commits_stop_and_returns_render_path() {
    let h = movement_scene("visible", /*with_light=*/ true).await; // reuse the M10e-4 harness
    // a player move along a clear lit path: token ends at stop, render_path returned.
    let res = h.room.execute_move(h.repo.as_ref(), &h.player, h.scene, h.token,
        vec![h.start, h.lit_goal], now_millis()).await.unwrap();
    assert_eq!(res.render_path.last().copied(), Some(res.stop));
    // committed position == stop
    assert_eq!(h.committed_pos(h.token).await, res.stop);
}

#[tokio::test]
async fn execute_move_rejects_a_moving_token() {
    let h = movement_scene("unrestricted", false).await;
    let _ = h.room.execute_move(h.repo.as_ref(), &h.player, h.scene, h.token,
        vec![h.start, h.adj], now_millis()).await.unwrap();
    // immediately requesting again (token still "moving") is Forbidden
    let again = h.room.execute_move(h.repo.as_ref(), &h.player, h.scene, h.token,
        vec![h.adj, h.adj2], now_millis()).await;
    assert!(matches!(again, Err(DataError::Forbidden)));
}

#[tokio::test]
async fn execute_move_truncates_at_a_wall_atomically() {
    let h = movement_scene_with_wall().await; // wall blocks the 2nd step
    let res = h.room.execute_move(h.repo.as_ref(), &h.player, h.scene, h.token,
        vec![h.start, h.corner, h.beyond_wall], now_millis()).await.unwrap();
    assert_eq!(res.stop, h.corner); // stopped before the wall
    assert_eq!(h.committed_pos(h.token).await, h.corner); // atomic: committed at stop
}
```

> Build on the existing `movement_scene(...)`/`MovementHandle` helpers (room.rs movement tests). Add `committed_pos`, `lit_goal`, `adj`, `corner`, etc. as the harness needs (mirror existing fields).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat execute_move`
Expected: FAIL — `execute_move` not defined.

- [ ] **Step 3: Implement** `Room::execute_move` + the `moving` field per the design notes.

- [ ] **Step 4: Run tests + the full suite + clippy/fmt**

Run: `cargo test -p shadowcat && cargo clippy -p shadowcat --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS, clean.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/ws/room.rs
git commit -m "feat(m1): Room::execute_move — atomic stop write + render-path + moving lock"
```

---

### Task 5: `conn.rs` — `MoveRequest` handler

**Files:**
- Modify: `src/server/src/ws/conn.rs`
- Test: `src/server/src/ws/conn.rs` (extract a testable `handle_move_request` free fn, like `handle_pathfind`)

**Interfaces:**
- Consumes: `Room::execute_move` (Task 4).
- Produces: a `MoveRequest` match arm in `handle_socket` that resolves the room, calls `execute_move`, and replies `MoveExecuted` to the requester's `etx` (one-shot — the atomic `Event` already broadcasts the position to everyone), or `MoveError` on `Err`.

**Design notes:** Mirror the `Pathfind` arm (conn.rs:334–349) and `handle_pathfind` (conn.rs:376) exactly: build the reply frame off a free function, send via `etx.send(Egress::Frame(Arc::new(frame)))`. The `MoveExecuted` reply carries `render_path` (mover-only per M1). Convert `(f64,f64)` tuples to `[f64;2]` for the wire. On `execute_move` `Err(Forbidden)` → `ServerMsg::MoveError { request_id, message: "move rejected" }` (generic — no geometry leak).

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn handle_move_request_executes_and_replies_to_requester() {
    let h = /* room + player ctx + token, reuse handle_pathfind test setup */;
    let frame = handle_move_request(&h.room, h.repo.as_ref(), &h.player, h.scene, h.token,
        vec![[0.0,0.0],[100.0,0.0]], Uuid::from_u128(7)).await;
    match frame {
        ServerMsg::MoveExecuted { request_id, render_path, .. } => {
            assert_eq!(request_id, Uuid::from_u128(7));
            assert!(render_path.len() >= 1);
        }
        other => panic!("expected MoveExecuted, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run test to verify it fails** (`handle_move_request` not defined)

Run: `cargo test -p shadowcat handle_move_request`
Expected: FAIL.

- [ ] **Step 3: Implement** `handle_move_request` + the match arm, mirroring `handle_pathfind`.

- [ ] **Step 4: Run tests + build client for the full server suite**

Run: `pnpm --filter @shadowcat/ui build && cargo test -p shadowcat && cargo clippy -p shadowcat --all-targets -- -D warnings`
Expected: PASS, clean.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/ws/conn.rs
git commit -m "feat(m1): conn MoveRequest handler — execute + reply MoveExecuted to requester"
```

---

### Task 6: Client `WsClient.moveRequest` + AppContext seam

**Files:**
- Modify: `src/client/core/src/ws-client.ts`
- Modify: `src/client/ui-kit/src/appContext.ts`
- Modify: `src/client/shell/src/lib/worldSession.svelte.ts`
- Test: `src/client/core/src/ws-client.test.ts`

**Interfaces:**
- Produces:
  - `interface MoveExecuted { tokenId: string; stop: [number,number]; renderPath: [number,number][]; durationMs: number }`
  - `WsClient.moveRequest(scene: string, tokenId: string, path: [number,number][]): Promise<MoveExecuted>` — a correlated request mirroring `pathfind` (pending-map by `request_id`; resolves on `move_executed`, rejects on `move_error`/timeout).
  - `AppContext.moveRequest(scene, tokenId, path): Promise<MoveExecuted>`.

**Design notes:** Mirror `WsClient.pathfind` (ws-client.ts) exactly — same correlated-request/pending-map/timeout machinery, same wire field names (snake_case: `move_request`, `token_id`, `request_id`; reply `move_executed` with `render_path`, `duration_ms`). Pure transport mirror; no movement logic in the client.

- [ ] **Step 1: Write the failing tests** (mirror the `pathfind` resolve/reject tests in ws-client.test.ts):

```ts
test("moveRequest resolves on move_executed", async () => {
  const { client, server } = makeWsHarness(); // reuse the pathfind test harness
  const p = client.moveRequest("scene1", "tok1", [[0,0],[100,0]]);
  const sent = JSON.parse(server.lastSent());
  expect(sent.type).toBe("move_request");
  server.deliver({ type: "move_executed", request_id: sent.request_id, token_id: "tok1",
    stop: [100,0], render_path: [[0,0],[100,0]], duration_ms: 200 });
  await expect(p).resolves.toEqual({ tokenId: "tok1", stop: [100,0], renderPath: [[0,0],[100,0]], durationMs: 200 });
});

test("moveRequest rejects on move_error", async () => {
  const { client, server } = makeWsHarness();
  const p = client.moveRequest("scene1", "tok1", [[0,0],[100,0]]);
  const sent = JSON.parse(server.lastSent());
  server.deliver({ type: "move_error", request_id: sent.request_id, message: "move rejected" });
  await expect(p).rejects.toThrow();
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/core test -- ws-client`
Expected: FAIL — `moveRequest` not defined.

- [ ] **Step 3: Implement** `moveRequest` (mirror `pathfind`), the `MoveExecuted` type, the `move_executed`/`move_error` wire handling, the `AppContext.moveRequest` seam, and the `worldSession` wiring (mirror how `pathfind` threads through). Update any AppContext test fixtures/stubs that must satisfy the new member.

- [ ] **Step 4: Run tests + typecheck (all packages — the AppContext member addition ripples)**

Run: `pnpm --filter @shadowcat/core test && pnpm -r typecheck`
Expected: PASS; 0 type errors (add `moveRequest` to every AppContext fake, as `pathfind` was).

- [ ] **Step 5: Commit**

```bash
git add src/client/core/src/ws-client.ts src/client/ui-kit/src/appContext.ts src/client/shell/src/lib/worldSession.svelte.ts src/client/core/src/ws-client.test.ts
git commit -m "feat(m1): WsClient.moveRequest + AppContext seam (transport mirror of pathfind)"
```

---

### Task 7: Rework the route-commit to request-only

**Files:**
- Modify: `src/modules/scene-tools/src/controller.svelte.ts` (`makeMeasureTool` / `commitRoute`)
- Test: `src/modules/scene-tools/src/measure-tool.test.ts`

**Interfaces:**
- Consumes: `ctx.moveRequest` (Task 6); `ctx.scene.animateAlongPath` (M10e-5).

**Design notes:** Replace the optimistic-commit body. On a double-click commit (keep the existing double-click detection + `committing` flag + epoch guard — they still apply to the async `moveRequest`):
- `commitRoute(goal)` requests a route preview path as today (`ctx.pathfind` for the proposed cell-path), then calls `ctx.moveRequest(scene.id, tokenId, proposedPath)`.
- On resolve (`MoveExecuted`): `ctx.scene.animateAlongPath(tokenId, result.renderPath)` — the cosmetic walk along the server-validated render-path. The authoritative position arrives via the normal store Event (token → stop) and the animator's any-ahead rule recognizes it. **No `dispatchIntent`, no `collinearRuns`, no per-run chaining.**
- On reject/`MoveError`: `clearRoute()` (no move).
- Keep the `committing` flag (suppress re-entry during the in-flight request) + the stale-resolve guard from M10e-5.
- Simpler path: the commit may send `ctx.moveRequest` directly with the previewed path the route-preview already produced (avoid a second pathfind) — store the last previewed `PathResult.path` and send it. If none previewed yet, do one `pathfind` then `moveRequest`.

- [ ] **Step 1: Write/adjust the failing tests** — replace the M10e-5 commit test. The double-click commit now calls `moveRequest` (not `dispatchIntent`) and animates `renderPath`:

```ts
test("double-click commits via moveRequest and animates the returned render-path", async () => {
  const moves: Array<{ tokenId: string; path: [number,number][] }> = [];
  const animated: Array<{ id: string; path: [number,number][] }> = [];
  const moveRequest = async (_s: string, tokenId: string, path: [number,number][]) => {
    moves.push({ tokenId, path });
    return { tokenId, stop: path.at(-1)!, renderPath: path, durationMs: 300 };
  };
  const { ctx, now } = seedRouteCtx({
    pathfind: async () => ({ path: [[0,0],[100,0],[100,100]] as [number,number][], cost: 2 }),
    moveRequest,
    animateAlongPath: (id, path) => animated.push({ id, path }),
    tokenAt: { id: "tok1", x: 0, y: 0 },
  });
  const tool = makeMeasureTool(ctx);
  tool.onPointerDown({ x: 100, y: 100 }, ev()); tool.onPointerUp(ev?.());
  now.advance(100);
  tool.onPointerDown({ x: 100, y: 100 }, ev()); tool.onPointerUp(ev?.());
  await drain();
  expect(moves).toEqual([{ tokenId: "tok1", path: [[0,0],[100,0],[100,100]] }]);
  expect(animated).toEqual([{ id: "tok1", path: [[0,0],[100,0],[100,100]] }]);
});
```

> Extend `seedRouteCtx` with a `moveRequest` stub. Remove the old `dispatchIntent`-chaining assertions.

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/module-scene-tools test -- measure-tool`
Expected: FAIL — commit still uses the old path.

- [ ] **Step 3: Implement** the request-only `commitRoute`; delete the `collinearRuns` import + the per-run dispatch loop.

- [ ] **Step 4: Run tests + typecheck**

Run: `pnpm --filter @shadowcat/module-scene-tools test && pnpm --filter @shadowcat/module-scene-tools typecheck`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/modules/scene-tools/src/controller.svelte.ts src/modules/scene-tools/src/measure-tool.test.ts
git commit -m "feat(m1): route-commit is request-only — MoveRequest + animate server render-path"
```

---

### Task 8: Remove dead M10e-5 chaining artifacts

**Files:**
- Remove: `src/modules/scene-tools/src/path-runs.ts`, `src/modules/scene-tools/src/path-runs.test.ts`
- Modify: `src/server/src/ws/room.rs` — remove the `route_commits_as_chained_runs_around_a_wall` test (it locked the client-chaining invariant that no longer exists; `Task 4`'s `execute_move_truncates_at_a_wall_atomically` is its successor).

**Design notes:** `collinearRuns` and the gate-chaining test were built for the optimistic client-chained commit, which Task 7 removed. Deleting them keeps the tree honest (no dead code, no test asserting a dropped design). Use `git rm` (preserves history per the immutable-history rule — deletion is a normal commit, not a rewrite).

- [ ] **Step 1: Remove the files + test**

```bash
git rm src/modules/scene-tools/src/path-runs.ts src/modules/scene-tools/src/path-runs.test.ts
```
Delete the `route_commits_as_chained_runs_around_a_wall` test fn from `room.rs`.

- [ ] **Step 2: Verify nothing references the removed symbols**

Run: `grep -rn "collinearRuns\|path-runs\|route_commits_as_chained" src/` — expect no hits.

- [ ] **Step 3: Run the full client + server suites**

Run: `pnpm -r test && pnpm -r typecheck && pnpm --filter @shadowcat/ui build && cargo test -p shadowcat`
Expected: all green.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "chore(m1): remove dead client-chaining artifacts (collinearRuns + gate-chaining test)"
```

---

## Verification (end of plan, before review)

- [ ] `pnpm -r test && pnpm -r typecheck` — client green, no type errors.
- [ ] `pnpm --filter @shadowcat/ui build && cargo test -p shadowcat` — server green.
- [ ] `cargo clippy -p shadowcat --all-targets -- -D warnings && cargo fmt --check`.
- [ ] Manual reasoning: a routed move around a wall commits **atomically** to the goal (one position write), the mover animates the full render-path, an observer sees the token at the stop location; a move into darkness/behind a wall **truncates** at the last legal cell (committed there); a second move on an in-flight token is **rejected**; a mid-animation reload shows the token at its stop location.

---

## Self-Review (completed during authoring)

- **Spec coverage:** §3 lifecycle → Tasks 1,4,5,7; atomic write → Tasks 3,4; exact-path step validation (walls+mask+supercover flankers) → Task 2; `moving` lock + reject → Task 4; reload-resolves-at-stop → atomic write (Task 4, no in-memory state needed on reload); mover-only render-path → Tasks 4,5,7; region-arrest hook stub → Task 2; M10e-5 disposition (drop optimistic route-commit) → Tasks 7,8. M2/M3 explicitly out of scope.
- **Type consistency:** `execute_move` signature is the room-facing one (with `restriction`/`visible`/`cell`) in Tasks 2/4; `MoveOutcome`/`MoveExecution`/`MoveExecuted` distinct (pure outcome → room result → wire frame); `renderPath`/`render_path` naming matches the protocol/ts-rs boundary; `moveRequest(scene, tokenId, path)` identical client-side across Tasks 6/7.
- **Placeholders:** scene-construction in tests is marked `/* … */` with explicit instruction to reuse the existing `from_documents`/`movement_scene` harness verbatim; all logic code is concrete.

---

## Buddy-check directives

**High-risk signals (multiple plan-time categories):** this plan touches **security boundaries** (the move executor reuses the M10e-4 vision mask — the secrecy gate; a fork or off-by-one re-introduces a movement-into-fog leak), **wide-blast-radius infrastructure** (the `commit_ops` refactor of the core authoritative write path), **public protocol contracts** (new `MoveRequest`/`MoveExecuted` frames + ts-rs), and **concurrency** (the `moving` lock + the off-read-lock resolve-then-await pattern that M10e-4 had to get right to avoid lock-across-await). The atomic-write-bypasses-the-gate design is correct only if the executor's per-step validation is exactly the gate's — that equivalence is the load-bearing claim.

**Directive:** run a **whole-branch buddy-check** (two reviewers — `shadowcat-spec-reviewer` spec-lens + `shadowcat-code-reviewer` code-lens, blind round 1 → debate to convergence) after all tasks, scoped to Tasks 2, 3, 4 (the executor/mask-parity, the commit refactor's behavior-preservation, and the lock/await discipline). Per-task gates use the two-reviewer pair as in the M10e-5 cadence. Fold a buddy-check offer into the execution handoff.
