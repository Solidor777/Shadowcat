# Server-Authoritative Movement — Design

**Status:** Approved (user, 2026-06-25). Supersedes the optimistic route-commit half of
M10e-5; reshapes the M10 movement roadmap. The M10e-5 **animation engine** is retained;
its optimistic route-commit is replaced by this model.

## 1. Motivation

Gated token movement can be rejected or halted for reasons the client cannot fully or
safely replicate — the per-(user,scene) **vision mask** and, later, **region effects**
(e.g. a trap that arrests movement mid-route). Optimistic client prediction of such moves
therefore causes **rubber-banding**: the client shows a move the server then refuses or
truncates. The M10e-5 buddy-check surfaced a concrete instance (P1): for sub-0.5-cell
footprints the client pathfinder accepted a diagonal step the server's supercover gate
rejected, and the optimistic animate-ahead walked the token to a refused goal before
snapping back.

**Rule (user-set, durable):** gated token moves are **request-only and
server-authoritative**. The client *requests* a move; the server is the sole executor and
gates each step; the client **does not render a move until the server has executed it**.
The pathfinder is **vision-gated** (an occluded cell is not a valid pathfinding cell), so a
previewed route can never include a step the server will reject. This deliberately reverses
optimistic prediction *for gated moves only* — non-gated contexts and all other document
mutations keep optimism. The accepted trade is input round-trip latency in exchange for
never showing a move that won't happen.

## 2. Core invariants

1. **Atomic game state.** A move changes the token's authoritative position **once**, from
   start directly to its **stop location** (the goal, or the interruption point — first
   blocked step or region-arrest). There are no intermediate authoritative positions. All
   game logic sees the token at a single well-defined location.
2. **Render-only animation.** The walking animation is cosmetic catch-up to an
   already-authoritative stop location. It carries no game state.
3. **Determinism / reload.** The authoritative position is the stop location the instant
   the move is **approved**. A reload (or resync) mid-animation resolves the token at its
   stop location with no animation. Move **duration** is deterministic (`path-distance ÷
   speed`), so the server and client agree on it without negotiation.
4. **Move lock.** Once approved, a move is committed: the token *will* reach the stop
   location and **cannot accept another move until it arrives**. The token carries a
   transient `moving` state for the (deterministic) animation duration; new move requests
   for a `moving` token are rejected by the server.
5. **Shared deterministic vision.** Vision is one algorithm. The server uses it as the
   authority for persistent secrecy and move approval; the client uses the *same* algorithm
   for render-time continuous vision. Because they agree by construction, client-side vision
   is not a divergence/cheat vector. (Mirrors the established
   "server-mirrors-client-resolver-semantics" rule.)

## 3. Move lifecycle

```
Plan (client)         Commit (client→server)     Execute (server)            Render (all clients)
─────────────         ──────────────────────     ────────────────            ────────────────────
vision-gated          MoveRequest{ token,        validate exact path step    MoveExecuted arrives:
pathfinder preview →  proposed cell-path }       by step vs walls+vision+     token doc position is
+ checkpoints                                    regions; find stop location; the stop location
                                                 atomically write token       (authoritative); animate
                                                 position = stop; set moving; the render-path (render-
                                                 broadcast MoveExecuted       only); moving lock clears
                                                                              after duration_ms
```

- **MoveRequest** `{ request_id, token_id, path: [cell,…] }` — the exact previewed
  cell-path (start … goal). The server treats it as a *proposal*, not a command.
- **Validation.** The server walks the proposed path. For each step it checks: the step
  crosses no `blocksMove` wall (M9a `blocks_move`), the step's supercover cells are all in
  the mover's vision mask (M10e-4 gate, including diagonal flankers), and no region arrests
  movement at that cell (§6). The **stop location** is the last cell reachable before the
  first failing step (or the goal if none fails). The path is bounded/sanitized
  (max length; each step adjacent to the previous; fail-closed on malformed input).
- **Atomic commit.** The server writes the token's `system.x/y` to the stop location
  through the normal document-command path, so the change **persists and resyncs** like any
  authoritative state. GM / unrestricted contexts skip the gate (move resolves at goal).
- **MoveExecuted** `{ token_id, stop: cell, render_path: [point,…], duration_ms, moving:
  true }` — a companion broadcast frame carrying the transient render metadata. The
  authoritative position is the document update; `MoveExecuted` is the cosmetic overlay the
  client correlates with it.

## 4. Render-path delivery & continuous client vision

This section describes the **target** (reached at M2). Delivery is staged across the
decomposition (§7): **M1 delivers the render-path only to the mover**; **M2** adds
observer-side delivery (per-observer clipping, then continuous client vision).

- **Client animation:** a client animates the token along its received `render_path` over
  `duration_ms` using the retained M10e-5 animation engine (`animateAlongPath`). The
  authoritative `setTarget(stop)` coincides with the path's final vertex, so it does not
  interrupt the walk.
- **M1 — mover-only render-path (no leak, no per-recipient machinery).** The executed
  `render_path` is returned to **the moving player only** (one-shot, like the `Pathfind`
  reply). Because the mover's route is vision-gated (within the mover's own vision by
  construction), no clipping is needed — the mover sees their whole walk. **Observers** see
  only the **atomic position update** (the token tweens straight to the stop location via the
  existing animator). No hidden cells leave the server.
- **M2 — observer render-path delivery (per-observer clip → continuous).** First, deliver the
  render-path to observers **server-clipped per recipient** (LOS against `blocksSight` for
  the observer's current vision) as **timing-tagged visible sub-segments**, so a **static
  observer** sees the correct **timed occlusion gap** behind a wall (no leak). Then move to
  **full `render_path` broadcast** to all scene players + **per-frame** shared deterministic
  LOS recompute from each token's **animated** position, adding the cases server-clipping
  cannot reach:
  - **observer moved first:** the whole move is visible (the observer's new sightline covers
    it);
  - **concurrent animations:** an observer who finishes its own move mid-animation picks the
    watched token up live as its own sightline opens.

## 5. Secrecy posture (scoped relaxation)

The server-side fog gate stays authoritative for **all persistent state** — resting token
positions, the explored map, GM-only documents — which is never delivered to a client that
cannot see it. **M1 introduces no relaxation** (it server-clips render-paths per observer).
The **only** relaxation, introduced at **M2**, is the **transient in-flight render-path**,
broadcast scene-wide and clipped client-side by the shared deterministic vision. It is
bounded (lives only for the animation duration, never written to persistent fog/explored)
and justified by the continuous-cross-animation-vision requirement, which fundamentally
needs the moving token's full trajectory on the client (server-side clipping cannot react to
a second client-paced animation). Determinism prevents *computation* cheating (the client
computes the same vision as the server); the accepted residue is that a modified client
could read the transient hidden-trajectory cells during the animation window.

## 6. Regions / trap arrest (hook now, system in M10g)

Step-validation consults regions: a region may **arrest** movement (set the stop location to
the arrest cell) or, later, modify cost. This design adds the **arrest hook** in the move
executor (a per-step "does a region stop the token here?" check) and the `moving`/atomic
semantics that make an early stop well-defined. The **region data model and effects system
remain M10g**; until then the hook has no registered arresting regions (no behavior change).

## 7. Decomposition (sequential checkpoints)

| # | Checkpoint | Deliverable |
|---|---|---|
| **M1** ✅ | Server-authoritative move execution | `MoveRequest`/`MoveExecuted` frames; exact-path step validation (walls + vision mask incl. diagonal flankers); atomic stop-location write via the document-command path; `moving` lock + server rejection of moves for a moving token; reload/resync resolves at stop location; render-path returned **to the mover only** (one-shot; mover animates their walk). Observers see the atomic position (straight tween) until M2. Client: request-only commit (rework the measure-tool route-commit) + animate the render-path via the kept animation engine. |
| **M2** ✅ | Observer render-path + continuous client vision | Deliver the render-path to observers — first **server-clipped per recipient** as timing-tagged visible sub-segments (static-observer occlusion gaps, no leak), then **full broadcast** + per-frame shared deterministic LOS recompute from each token's animated position (observer-moved-first + concurrent pick-up). |
| **M3** ✅ | Vision-gated pathfinder + region-arrest hook | Make the preview pathfinder vision-gated and deterministic-equal to server validation (closes buddy-check P1 at the root — router mask predicate ≥ gate supercover predicate, incl. diagonal flankers / sub-0.5 footprints); add the per-step region-arrest hook in the executor. Region system itself = M10g. |

**Dependency order:** M1 → M2 → M3. M1 is independently shippable (basic static-vision
clipping); M2 upgrades the render fidelity; M3 closes the preview/gate parity and seats the
region hook. Each is independently buddy-checked (M8/M9 cadence).

**Status: ALL THREE CHECKPOINTS DONE** (branch `m10e-5-movement-animation`, commits
`98bf191..fb8b7dd`). M3 landed via `docs/superpowers/plans/2026-07-01-m3-vision-gated-pathfinder.md`
+ `docs/superpowers/specs/2026-07-01-m3-vision-gated-pathfinder-design.md` — the router's
`cell_enterable` now unions `movement::supercover_cells(from, to, cell)` into its mask check
(the same primitive the M1 executor and the legacy `publish` gate use per step) alongside the
existing footprint-disc test, fails closed on a degenerate `None`, and carries a same-shaped inert
region-arrest stub mirroring the M1 executor's. This milestone (the server-authoritative-movement
redirect of M10e-5) is complete; M10f (continuous/Polyanya pathfinding) and M10g (weighted/
impassable regions) remain, unstarted.

## 8. Disposition of the M10e-5 branch (`m10e-5-movement-animation` @ fd344af)

- **Keep — animation engine** (Tasks 1–5, 8): duration/easing/interruptible, path-aware
  `TokenAnimator` + `setAnimation`/`animateAlongPath` seams + Stage config wiring. Retained
  as-is; it animates the authoritative render-path. (The `any-ahead`-ignore rule, built for
  the optimistic burst, becomes inert under request-only but is harmless; M2 may simplify
  it.)
- **Drop / rework — optimistic route-commit** (Task 7) and the **client-chaining gate test**
  (Task 9): replaced by `MoveRequest`/`MoveExecuted` + atomic state. The double-click
  route-commit interaction is reworked into a request-only commit in M1.
- **Resolved by redirect:** buddy-check P1/P4 (the diagonal-flanker rubber-band) — no
  optimistic animate-ahead means nothing to rubber-band, and M3's vision-gated pathfinder
  removes the divergence at its source.

The branch is **held, not merged.** The animation engine lands as part of M1 (the first
checkpoint to build on it) rather than as a standalone merge.

## 9. Open / deferred

- **Animated vision for light-bearing movers** (a moving light progressively revealing area
  during its walk) — deferred; under atomic state, vision is authoritatively recomputed for
  the final position. M2's continuous render-time vision approximates the observer side; a
  full sweeping-light render is a later enhancement.
- **Region cost/weighting** (vs arrest) — M10g.
- **Continuous (Polyanya) movement model** — M10f; this design is grid-stepped, consistent
  with the existing M10e pathfinder.
- **Move-queueing** (issuing the next move while one is in flight) — out of scope; the
  `moving` lock rejects it by design.

## 10. Decisions — confirmed (user, 2026-06-25)

1. Move initiation: **client sends the exact proposed cell-path** (preview + checkpoints);
   server validates + executes that path.
2. State is **atomic** — token is at its stop location from approval; animation is render-
   only; reload resolves at the stop location.
3. A token gains a **`moving` state** during animation and **cannot be re-moved until it
   arrives**; an approved move is deterministic and will complete.
4. Render-path recipients: **all players in the scene** (clipped client-side by vision).
5. **Vision updates with the animation** (continuous, render-time) — requires client-side
   deterministic vision; the full render-path on the client is the accepted, bounded leak.
6. **Determinism is an acceptable alternative to server control** (esp. vision): a shared
   deterministic algorithm the client may run locally, with the server authoritative for
   approval + persistent secrecy + final state.
