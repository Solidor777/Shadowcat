# M9a — Walls + Server-Authoritative Movement-Blocking: Plan

> **For agentic workers:** execute with **`mainline-plan-execution`** (inline enumerative
> per-task spec check + ONE dispatched final branch review; buddy-check it). Checkbox steps.

**Goal:** A **wall** scene-entity (`doc_type:"wall"`, a segment + `blocksSight`/`blocksMove`),
rendered for the GM, drawn with a wall tool; and **server-authoritative movement-blocking** —
a token position-update intent that crosses a `blocksMove` wall is **rejected** by the server
(the client rolls back via the M6 optimistic path). No vision yet (M9b).

**Spec:** [`../specs/2026-06-22-m9-walls-vision-fog-design.md`](../specs/2026-06-22-m9-walls-vision-fog-design.md)
§4 (walls), §5 (server-authoritative movement), §10 (decisions, confirmed).

**Key design decision (server collision hook).** The geometric check needs the scene's wall set,
which lives in the per-world `SceneEcs` on `Room` — but authorization runs in `repo.apply_intent`,
which has no ECS access. So the collision check runs in **`Room::publish`, BEFORE `apply_intent`**:
it reads the token's **authoritative** current position from the ECS, forms the move segment
(committed pos → requested `new`), and rejects pre-write (no seq consumed) if it crosses a
`blocksMove` wall and the actor is not a GM. This keeps movement-collision **engine-owned** (on
`Room`, not the repo/module) — the second geometric exception to ARCHITECTURE #6 (after vision).

**Walls hydrate for free:** walls are scene entities (`parent_id` = scene), so `SceneEcs` already
hydrates them as `SceneEntity{doc}` — the collision query just filters `doc_type=="wall"`.

## Global constraints
- **#1/#3:** a token move stays an optimistic intent; the *rejection* is the server's authority
  (the move is exempt from being trusted, not from the optimistic round-trip).
- **#5/#6:** walls are documents; the per-world ECS hydrates them; the server's only semantic read
  of `system` is this geometric check (the documented exception).
- **#7 clean-room:** segment-intersection from public computational-geometry (cite the source); no
  proprietary VTT/engine source.
- **No raw `console.log`/`println!`; commit per task; push only at the M9a milestone end (CI green).**

---

### Task 1: server geometry + `SceneEcs::blocks_move`

**Files:** `src/server/src/scene/mod.rs` (a `blocks_move` query + a pure `segments_cross` helper);
tests in the same module.

- Pure `segments_cross(a0, a1, b0, b1) -> bool` — do segments `a0→a1` and `b0→b1` properly
  intersect? Orientation/sign-of-cross-product test. *Source: standard segment-intersection
  (CLRS "Determining whether two segments intersect" / orientation method).*
- `SceneEcs::blocks_move(&self, token_id: Uuid, new_x: f64, new_y: f64) -> bool`:
  - find the token `SceneEntity` by id; read its committed `system.x`,`system.y` (the segment
    start) + its `parent_id` (the scene); return false if not a token / missing coords.
  - move segment = `(x,y) → (new_x,new_y)`; query all `SceneEntity` with `doc_type=="wall"` AND
    `parent_id == scene` AND `system.blocksMove == true`; read each `system.seg.{x1,y1,x2,y2}`;
    return true if the move segment crosses any.
- Helper to read an `f64`/`bool` from a doc's opaque `system` via JSON pointer.
- [ ] Test: `segments_cross` truth table (crossing, parallel, collinear-disjoint, touching-endpoint,
  T-junction); `blocks_move` — a token whose move crosses a `blocksMove` wall → true; a move that
  misses → false; a `blocksMove:false` wall → false; a wall in another scene → false.
- [ ] Implement. `cargo test -p shadowcat --lib scene`. **Commit:** `feat(m9a): SceneEcs wall collision query + segment-intersection`

---

### Task 2: `Room::publish` movement-collision hook + reject

**Files:** `src/server/src/ws/room.rs` (`publish`); tests in the same module + an integration test
(mirror `ws_convergence.rs` / the apply_intent reject tests).

- In `publish`, **before** `repo.apply_intent`: if `ctx.world_role != Gm`, for each `Operation::Update`
  whose `changes` touch `/system/x` or `/system/y`, compute the requested `(new_x,new_y)` (from the
  change `new`, falling back to the ECS current for the unchanged axis) and call
  `self.scene.read().await.blocks_move(doc_id, new_x, new_y)`; if true → return
  `Err(DataError::Forbidden)` (maps to `RejectReason::Forbidden` → client rollback). GM moves skip
  the check (the "ignore walls" override, §5).
- The check runs before the write, so a blocked move consumes no seq and applies nothing.
- [ ] Test (integration): a scene + a `blocksMove` wall + a token; a **player** move crossing the wall
  → `publish` returns `Err(Forbidden)` and no event is logged (`authoritative_seqs` unchanged); a
  **GM** move crossing → allowed (event logged); a clear move → applied. Mirror `repo_with_world` +
  the existing publish tests.
- [ ] Implement. `cargo test -p shadowcat`. **Commit:** `feat(m9a): server-authoritative movement-blocking in Room::publish`

---

### Task 3: ARCHITECTURE.md #6 amendment

**Files:** `docs/design/ARCHITECTURE.md`.
- Amend the "server runs no semantic logic except vision" rule to add **movement-collision** as the
  second engine-owned geometric exception (server-authoritative, not module code, reads wall + token
  geometry from the scene ECS). Cross-reference M9 §2/§5.
- [ ] **Commit:** `docs(m9a): amend ARCHITECTURE #6 — movement-collision is engine-owned geometry`

---

### Task 4: client wall model + `WallView` reconciler

**Files:** `src/client/render/src/wall-view.ts` + `index.ts`; `engine.ts` (construct + reconcile);
test `wall-view.test.ts`, `engine.test.ts`.
- `wall.system` (client-owned): `{ seg: { x1, y1, x2, y2 }, blocksSight: boolean, blocksMove: boolean }`.
- `WallView(documents, backend)`: diff `query("wall")` → `setShape(id, { layer: "walls", points:
  [x1,y1,x2,y2], closed: false, stroke: { color: <wall color>, width: 4 }, fill: null })` /
  `removeShape`. Reads the optimistic view ([[render-from-optimistic-view]]). Engine reconciles it
  alongside drawings/templates.
- [ ] Test: a wall doc → a stroked segment in the `walls` layer; delete → removed; engine renders an
  existing wall on start. Implement. **Commit:** `feat(m9a): WallView reconciler (render walls)`

---

### Task 5: wall tool + tool rail

**Files:** `src/client/ui/src/modules/scene-tools/controller.svelte.ts` (`makeWallTool`,
`ToolId += "wall"`), `ToolRail.svelte` (wall button), `en.ts`; test `wall-tool.test.ts`.
- `makeWallTool(ctx)`: down → anchor = `snap(p)` (claim if a scene is active, else false); move →
  `previewOverlay([{ points:[a.x,a.y, snap(p).x, snap(p).y], closed:false, stroke, fill:null }])`;
  up → if extent ≥ ~1, `dispatchIntent(create buildSceneEntityDoc(world, scene, "wall", { seg:{x1:a.x,
  y1:a.y, x2:snap(p).x, y2:snap(p).y}, blocksSight:true, blocksMove:true }))`; `clearOverlay`.
- Wall tool button in the rail (GM-gated like the rest). i18n `tools.wall`.
- [ ] Test: a wall drag previews + persists a `wall` doc (parent=scene, seg, both flags true); a
  no-extent click persists nothing. Implement. **Commit:** `feat(m9a): wall tool + tool-rail button`

---

### Task 6: e2e + final verification

**Files:** `e2e/stage.spec.ts` (GM draws a wall → it renders, via a `data-wall-count` Stage signal).
- Stage: `host.dataset.wallCount = String(documents.query("wall").length)` in `onDocs`.
- e2e: as GM (the admin), activate Wall, drag on the canvas → assert `data-wall-count` = `"1"`.
  (Movement-rejection requires a *player* session the Playwright harness doesn't establish — it is
  covered by the Task 2 server integration test; note this.)
- [ ] `pnpm -r typecheck`/`test`/`lint`; `pnpm --filter @shadowcat/ui e2e`; `cargo test --all`/`clippy`/`fmt`.
- [ ] `grep pixi.js src/client/render/src` → only `pixi-backend.ts`.
- [ ] **Commit:** `feat(m9a): wall-render e2e + Stage wall-count signal`

---

## Buddy-check directive
M9a introduces the **first server-side semantic geometry** (movement-collision — a new ARCHITECTURE #6
exception) on the authoritative write path, which all of M9 builds on. **Run a buddy-check at the
final branch review**, focusing on: the segment-intersection correctness (collinear/touching/T-junction
edge cases), the collision running **before** `apply_intent` (no seq consumed on reject; authoritative
old-pos from the ECS not the client's `old`), the per-op detection (only token x/y updates; partial-axis
moves), the GM bypass, the `blocksMove`-only filter (a `blocksSight`-only wall must NOT block movement),
and the client wall reconcile + tool.

## Spec coverage self-check
- §4 wall doc model + hydration + render + tool → Tasks 1 (hydration-free), 4, 5.
- §5 server-authoritative movement-blocking (reject-on-cross, GM override, no validation of coords
  beyond the geometric gate) → Tasks 1, 2.
- §2 ARCHITECTURE #6 amendment → Task 3.
- §9 testability (headless Rust geometry + collision + reject path; TS reconciler/tool; GL e2e) → all.
- **Deferred to M9b/M9c (correctly absent):** vision raycasting, the `vision` SceneDerived channel,
  fog, GM vision mode, per-recipient hidden (`gm_only`) walls (a permission refinement; M9a walls are
  observer-visible). Client pre-clamp (advisory UX) — optional, deferred (the server is the gate).
