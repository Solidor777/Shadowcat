# Docs Sweep 5 — Scene Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: on a Fable-class model use
> `mainline-plan-execution`; otherwise superpowers:subagent-driven-development.
> Steps use checkbox (`- [ ]`) syntax.

**Goal:** Document `src/server/src/scene/` — measured backlog 126 (mod.rs 54,
regions.rs 25, pathfinding.rs 20, lighting.rs 10, vision.rs 7, explored.rs 5,
grid_shape.rs 3, navmesh.rs 1, move_exec.rs 1; movement.rs, move_stream.rs,
grid_shape_parity_tests.rs already clean) plus the 3-item stray `health.rs` at
the crate root — then flip the whole scene/ tree (all 12 files) + health.rs to
deny.

**Architecture:** Same calibrated pattern (prior sweep plans' Global
Constraints verbatim). Branch `docs-sweep5-scene`. Ship with the LOCAL matrix.
Reviews under the no-shell protocol (pre-generated diff + relayed evidence;
reviewers must not run `cargo test`).

**Truthfulness hot spots:** this subsystem is the one with the documented
stale-citation defect class — movement/traversal gate docs must cite
`move_exec::execute_move`/`gate_walk` via `SceneEcs::move_walls`, NEVER
`Room::publish` (the post-D9 authority); the fail-open cell-size default is
REMOVED at all sites (`scene_grid_sizes` is the sole defaulting source — an
absent entry returns `None`/empty, `region_field` returns `Option`, callers
refuse via let-else; do not document any 100.0 fallback); `MirrorInput::
{Committed, Proposed}` decides LOG LEVEL not mutation (error! committed /
debug! proposed — backwards is a defect); `mirror_field_change`/
`reapply_changes` wrap the shared `apply_field_change`, never hand-written
branches; hex grid size = outer/circumradius (the Sweep-2b Critical — do not
regress); pathfind preserves the mover's literal start point (6f3b3c6);
vision/fog egress claims must match the per-recipient clip sites in
`clip_move_stream`/`send_filtered`.

## Model/Effort directives

Mainline (Fable 5, effort high) per standing directive. No-shell final review
pair; fixes pre-merge.

## Buddy-check directives

No high-risk signals (docs + lint attrs only). Standard final review only.

---

### Task 1: scene/mod.rs (54)

- [ ] Enumerate live (expect 54); document every item — SceneEcs surface
  (actor/token tables, wall/region/light mirrors, `apply_op`/`mirror_field_change`/
  `reapply_changes`, `MirrorInput`), scene-cache seams (`scene_grid_sizes`,
  `region_field -> Option`, `player_lit_mask`, `visible_cells{,_cached}`,
  `navmesh_for`). Doctests per policy (pure/constructible runnable; ECS-bound
  ` ```text `). Gates (scoped count for mod.rs = 0; cargo test/fmt/clippy -D;
  bindings shape-check if any ts-rs type is touched); commit.

### Task 2: scene/{regions,pathfinding}.rs (45)

- [ ] Enumerate live (expect 25+20); document — region overlay/weighting
  structs + field sampling; both pathfind engines (grid A* vs continuous by
  movement model, per the M10 architecture), start-point preservation, reject
  variants (`SceneUnknown` mirrors `Degenerate`). Doctests per policy. Gates;
  commit.

### Task 3: scene/{lighting,vision,explored,grid_shape,navmesh,move_exec}.rs (27) + health.rs (3)

- [ ] Enumerate live (expect 10+7+5+3+1+1 and 3); document — lighting
  gradation/lit-mask, vision tiers + `can_see` egress coupling, explored-mask
  persistence, grid-shape math (hex size = circumradius; supercover line
  contract), navmesh (square-on-hex reachable), move_exec gate order, health
  endpoint. Doctests per policy. Gates; commit.

### Task 4: Deny flip + verify + sync + ship

- [ ] Inner deny pair in ALL 12 scene/ files (explored, grid_shape,
  grid_shape_parity_tests, lighting, mod, move_exec, move_stream, movement,
  navmesh, pathfinding, regions, vision — clean files get the attr too) AND
  health.rs. Mutation proof on mod.rs + one leaf file; restore via python.
  Full local matrix. Docs-sync: PLAN.md; scene-rendering skill ratchet Gotcha.
  No-shell review pair with pre-generated diff + relayed evidence; fix
  findings; merge `--ff-only`; push; delete branch; memory update.

---

## Deferred (logged, not dropped)

- Sweep 6+: chat/ (83) + dice/ (172), then client packages, then modules.
  Then buddy-check convergence → final ratchet → skills reference pass.
