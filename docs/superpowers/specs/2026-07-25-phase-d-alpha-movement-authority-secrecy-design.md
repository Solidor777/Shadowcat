# Phase D-α — Movement authority & secrecy (design)

Date: 2026-07-25
Campaign: Phase-1 close-out (`docs/superpowers/specs/2026-07-24-phase1-closeout-campaign-design.md`)
Predecessors: Phase A (merged), Phase B (merged `15d0ee4`), Phase C (merged `9eb9add`)

## Scope & amendment note

The campaign spec's Phase D lists seven items (D1–D7). Exploration for this phase found three
additional items and one already-shipped item, so Phase D is **split in two**:

- **Phase D-α (this spec)** — movement authority & secrecy: **D10, D9, D8, D4**, in that order.
  All server-side movement authority, security-sensitive, one coherent restructure.
- **Phase D-β (later spec)** — movement & scene correctness: **D3, D1+D2, D7, D6, D5**.

Added items, none of them in the campaign spec:

- **D10 — wall secrecy axis** (security). `SceneEcs::move_walls(scene)` (`scene/mod.rs:1108`) has no
  viewer parameter, so a non-GM's route preview detours around `gm_only` walls, leaking their
  geometry through route shape. The same leak class M10g closed for secret regions.
- **D9 — player moves are request-only.** The standing user rule (memory
  `server-authoritative-movement-rule`, 2026-06-25) is that gated movement is request-only and
  server-executed. The select-tool drag still writes `/engine/x,y` directly
  (`src/modules/scene-tools/src/controller.svelte.ts:810-821`), violating three clauses of that rule.
  Closing it **eliminates the second movement gate** rather than making two gates agree.
- **D8 — GM gate-exemption unification.** `execute_move` enforces walls and impassable/arrest
  against GMs; `Room::publish` does not. The original M9 design spec
  (`2026-06-22-m9-walls-vision-fog-design.md:103`) grants GMs an "ignore walls" override, so
  `execute_move`'s enforcement is a regression against an approved spec, not a standing decision.

Already shipped, moved to D-β as verify-then-close: **D5** (edge-projected environment light) landed
2026-07-19 in `513aef8` + `e1156ae`; `POST_WORK_FINDINGS.md`'s "flat ambient / constraint-forced"
entry is stale.

User decisions governing this spec (recorded verbatim in intent):

1. GMs may make illegal moves. A GM placing a token in a space puts it in that space.
2. GMs may move with or without pathfinding. **Only players are forced to use pathfinding.**
3. Invisible walls can exist.
4. The footprint-in-mask consequence (wide tokens freeze where their body overlaps fog) is accepted.

## Cross-cutting invariants established here

**I1 — A GM bypasses every gameplay gate and no resource guard.** Gameplay gates: walls, vision
mask, impassable, arrest, footprint clearance. Resource/admissibility guards that stay unconditional
for every requester including GMs: `MAX_GATE_WALK_COORD`, `MAX_GATE_WALK_SAMPLES`, non-finite
refusal, scene-existence refusal, and `TokenEngine::validate`'s ingress coordinate bound. A future
"unification" that folds the DoS bounds into the GM exemption is a defect; this sentence exists to
prevent it.

**I2 — `execute_move` is the sole movement gate.** After D9 there is exactly one implementation of
the per-cell movement decision. The six-axis parity checklist in the scene-rendering skill
(cell indexing, traversal completeness, input admissibility, scene identity, `remove` semantics,
fail-open defaults) describes a fork that no longer exists. Any future second write path to a token
position must route through `execute_move`, never re-implement its gate.

**I3 — Wall secrecy is a two-value contract, mirroring `region_field`.** Authoritative for the
executor, per-requester for the router. Never a third mode; callers pass `None` for a GM.

**I4 — `route-admissible ⇔ gate-admissible` holds for non-GM movers only,** and modulo geometry the
router is not permitted to see: secret regions and `gm_only` walls both spring at execution. For a
GM the gate allows everything, so the equivalence is trivially satisfied.

**I5 — Vision and lighting keep the full wall set.** `sight_walls`/`light_walls` deliberately include
`gm_only` walls (`scene/mod.rs:199,939`, the M9b full-wall-set invariant): a wall you cannot see
still blocks your sight, which under-reveals and is correct. D10 changes the **routing** wall set
only. Do not "unify" these two wall sets.

---

## D10 — Wall secrecy axis

### Problem

`move_walls(&self, scene: Uuid) -> Vec<vision::Seg>` (`scene/mod.rs:1108`) returns every
`blocksMove` wall segment parented to the scene, with no requester filtering. Four consumers leak:

1. `pathfinding::find` → `cell_enterable` checks 1 and 3 (footprint-disc clearance, center-to-center
   `segments_cross`) — a route detours around an unseen wall.
2. `navmesh::build_navmesh` — inflates each wall into a capsule obstacle, so the navmesh itself
   encodes unseen geometry and every route through it detours.
3. `navmesh::clip_to_visible_mask` — its wall check truncates a route at an unseen wall.
4. `navmesh::los_smooth`'s `chord_ok` — refuses to straighten across an unseen wall.

Consumer 3's in-code justification ("walls are public geometry", a fidelity guarantee with no
confidentiality stake) is false once `gm_only` walls exist. The two-checks dichotomy documented there
must be re-derived, not merely re-worded: the mask check remains a secrecy gate, and the wall check
becomes a secrecy gate **as well** whenever the wall set is unfiltered.

### Two kinds of invisible wall — opposite treatment

| Kind | Document visibility | Router | Executor |
|---|---|---|---|
| `blocksSight:false` + `blocksMove:true` (invisible barrier) | public | **include** | include |
| `gm_only` wall document | hidden | **omit** | include (springs at execution) |

The first kind needs no change: the wall document is public, the player may legitimately learn "I
cannot walk there", and honouring it in the route is more honest than hiding it. Only document-level
secrecy filters.

### Design

`move_walls(&self, scene: Uuid, viewer: Option<Uuid>) -> Vec<vision::Seg>`, mirroring
`region_field`'s per-requester filter **exactly** (`scene/mod.rs`, the `viewer: Option<Uuid>` branch):

- `None` — authoritative: every `blocksMove` wall. Used by `execute_move` and by GM requesters.
- `Some(user)` — per-requester: a wall is included only when the requester can see the visibility
  tier declared on its `/engine` (`permissions.property_overrides.get("/engine")`, defaulting to
  `Visibility::All`), resolved through the same
  `resolve_access(user, WorldRole::Player, doc, effective_owner(doc, None))` + `can_see(tier)` call
  the region filter uses. A wall doc carries no actor link, so the no-join effective-owner resolution
  is exact — identical to the region case.

No new secrecy machinery: this is the same `resolve_access`/`property_overrides` mechanism that
already gates every document's egress, applied to a set the router consumes.

Threading: `SceneEcs::pathfind` computes the wall set once, above the engine dispatch, exactly as it
already does for `mask` and `region_field` — `move_walls(scene, if is_gm { None } else { Some(user) })`
— and passes the same slice into both engines. `navmesh_for`'s memo key must incorporate the
requester's wall set, since a per-requester mesh is no longer shared across requesters; keying on
`(scene, quantized footprint, is_filtered)` is insufficient because different players may see
different wall subsets. **Decision:** key the cache on `(scene, quantized footprint, digest)` where
`digest` is an order-independent hash of the included wall-document id set. This preserves the memo
for the common case where every player sees the same walls, and two requesters with identical wall
subsets correctly share one mesh. Whole-cache invalidation on any wall/scene mutation is unchanged
(over-invalidation is the safe direction).

### Tests

- A `gm_only` wall is absent from `move_walls(scene, Some(player))` and present in
  `move_walls(scene, None)`.
- A non-GM `pathfind` across a `gm_only` wall returns a route **through** it (the wall is invisible to
  the router), and `execute_move` over that same route stops at the wall — the spring-at-execution
  property, asserted end-to-end.
- A GM `pathfind` in the same scene detours around the wall (GM passes `None`).
- Fixture-construction precision, per the region precedent: mark the wall secret with
  `permissions.property_overrides.insert("/engine", Visibility::GmOnly)`. A test that instead sets
  `permissions.default = Access::None` proves nothing about this filter.
- Mutation check: flipping the filter to unconditional-include must fail the first two tests.

---

## D9 — Player moves are request-only; the second gate is deleted

### Problem

Three clauses of the standing movement rule are violated by the drag path:

- Request-only / server-sole-executor: a player's drag is a client-authored `/engine/x,y` write that
  the server validates, not a request the server executes.
- No optimistic render for gated moves: `dispatchIntent` renders the move immediately and rolls back
  on refusal — the rubber-banding the rule exists to prevent.
- Region-arrestable mid-route: `Room::publish` rejects a move **wholesale**; it cannot stop a token
  partway, so an arrest region cannot arrest a dragged token.

### Design — server

A non-GM `Operation::Update` whose changes touch a token's `/engine/x` or `/engine/y` is **refused**
(`DataError::Forbidden`) before any geometry work. Detection reuses `scene.token_move(doc_id, changes)`,
which already computes the committed post-image over the whole `/engine` band and therefore cannot be
evaded by a wholesale `/engine` write or duplicate change entries.

Refusal is strictly stricter than gating, so the **traversal** half of the non-GM gate block at
`ws/room.rs:220-…` is deleted: the M9a `blocks_move` wall gate, the `line_traversal`/supercover
traversed-cell computation, the per-cell mask membership test over a path, and the
coordinate-magnitude check (`TokenEngine::validate` covers ingress unconditionally; a point placement
needs finiteness only). `execute_move` becomes the sole implementation of the per-cell traversal
decision (**I2**), which is what collapses parity axes 1, 2 and 3 (per-cell decision, cell indexing,
traversal completeness).

The block's **point-placement** machinery is *retained and repointed*, not deleted — the Create gate
below needs it: the scene-existence refusal (parity axis 6 — an absent `scene_grid_sizes` entry must
still refuse rather than synthesize a 100-unit grid), the `MovementRestriction` dispatch, the
per-`(scene, leniency)` `visible_cache` memo, and the deferred `revealed_pending` / `get_explored`
async pass (the explored fetch still must not run under the `scene.read()` guard). What changes is
what it authorizes: a created token's position instead of a moved token's path.

GM drags are unchanged: no gate, direct write, token lands exactly where dropped (**user decision 1**).

`TokenEngine::validate`'s unconditional ingress coordinate bound is independent of this block and
stays — it is a resource guard (**I1**), and it is what keeps the deleted block's coordinate check
from being load-bearing.

### The Create gap

`ws/room.rs:239` leaves `Operation::Create` deliberately ungated, reasoning that "the create
capability is already a privileged grant". That reasoning does not hold: `data/document.rs:531-532`
asserts a world **can** grant `WorldRole::Player` `core:create` on `token`, so arbitrary player
placement is reachable by configuration. Placing a token in an unseen room is a fog bypass — the new
token's vision reveals the room — which is a strictly larger capability than the movement the same
player is forbidden from performing.

Non-GM token `Create` placement is therefore gated through the **same** mask predicate the movement
gate uses: the created position's cell must lie in the creator's mask for the scene's resolved
`MovementRestriction` (`Visible` ⇒ `visible_cells`; `Revealed` ⇒ `visible_cells ∪ explored`;
`Unrestricted` ⇒ ungated). GM exempt. Never a second mask computation — call the same accessor.
This is the [[path-prefix-authz-covers-ancestor-and-create]] shape: a gate scoped to `Update` while
`Create` reaches the same state is not a gate.

### Design — client

`sendMoves(delta)` (`controller.svelte.ts:810-821`) currently emits one batched `update` op per
selected token. It becomes role-branched:

- **GM** — unchanged: the batched `/engine/x,y` update, keeping the raw-stored-value `old` convention
  (`old: eng?.x ?? null`) that the field-level OCC check requires.
- **Non-GM** — one `moveRequest` **per selected token**, each routed to its own destination
  (`origin + delta`). `commitRoute`'s single-selection restriction (`controller.svelte.ts:494`) does
  not apply: `moveRequest` is per-token on the wire, the server gates each token independently, and
  partial outcomes (one token arrests, another completes) are correct and expected.

The drag gesture keeps visual feedback via a **preview overlay**, not an optimistic document write.
The token's own position animates only when the server's `MoveStream` arrives; that playback path
already exists from M2, so no new render path is introduced — the optimistic-write path is removed and
the existing authoritative-stream path takes over. This is the M10e-5 Task 7 conflict the standing
rule already identified.

### Tests

- A non-GM `Update` to `/engine/x` on a token is refused; the same op from a GM succeeds.
- A non-GM wholesale `/engine` write that changes position is refused (post-image detection).
- A non-GM `Update` to a non-position engine field (e.g. `/engine/rotation`) still succeeds — the
  refusal is scoped to position, not to token writes generally.
- A non-GM token `Create` outside the creator's mask is refused; inside it, succeeds; a GM's
  succeeds anywhere; an `Unrestricted` scene ungates it.
- Client: a non-GM drag of two selected tokens issues two `moveRequest`s and **zero** update ops; a
  GM drag issues one batched update and zero move requests.
- Client: a non-GM drag does not move the token's rendered position before a `MoveStream` arrives.

---

## D8 — GM gate-exemption unification

### Problem

`move_exec::execute_move`'s step 1 (`ecs.blocks_move`) is unconditional, and step 3 (impassable /
arrest) has no GM branch at all — `execute_move` receives only `restriction`, and folding a GM to
`Unrestricted` skips the mask (step 2) alone. So a GM's move request is wall-blocked, impassable-
blocked, and arrestable, while a GM's drag is none of those. `ws/room.rs:553-557` documents this as
intentional and instructs against re-granting the bypass; the M9 design spec grants it. The spec wins.

### Design

`execute_move` takes an explicit exemption input and a GM bypasses all three gameplay gates:
walls (step 1), mask (step 2, already), impassable and arrest (step 3). A GM's move lands exactly at
the requested destination — no truncation, `truncated: false` (**user decision 1**).

Because D9 leaves a single gate, this is a plain `is_gm` parameter with early-outs, not a shared
`MoveGateProfile` symbol; a shared symbol with one consumer is indirection without a second party to
keep honest. Should a second gate ever reappear, **I2** requires it to call `execute_move`, not to
re-derive the profile.

Terrain **cost accrual is independent of the exemption**: a GM still accumulates
`terrain_multiplier` per cell entry, so reported cost stays meaningful. Cost is information, not a
gate.

Resource guards stay unconditional for GMs (**I1**) — `gate_walk`'s `MAX_GATE_WALK_COORD` /
`MAX_GATE_WALK_SAMPLES`, non-finite refusal, and `MoveReject::SceneUnknown`.

### Router asymmetry, deliberate and documented

`pathfinding::cell_enterable` keeps its **unconditional** wall check, so a GM's route preview still
detours around walls. A route preview is a navigation aid; a GM who wants to cross a wall drags
directly (**user decision 2** — a GM may move without pathfinding). `route ⊆ gate-allowed` holds
trivially because the GM gate allows everything. This is an asymmetry, not the fork D8 closes, and
the spec says so explicitly so a later reader does not "fix" it.

### Tests

- A GM move request across a `blocksMove` wall completes to the requested destination, untruncated.
- A GM move request through an impassable region completes; through an arrest region completes without
  arrest.
- The same three moves by a non-GM are blocked/arrested as today (the non-GM behaviour is unchanged —
  assert it, so the exemption cannot be widened by accident).
- A GM move at `MAX_GATE_WALK_COORD + 1.0` is still refused (**I1** anti-drift).
- A GM's reported cost still reflects terrain multipliers.

### Doc sync

Invert the three stale statements, each citing M9 §5 as governing: `ws/room.rs:555`,
`docs/PLAN.md:345`, and `.claude/skills/shadowcat-codebase-scene-rendering/SKILL.md:802-804`.
`ws/room.rs:218` and `:1469` and the two M9 docs already state the rule correctly.

---

## D4 — Footprint-aware authoritative gate

### Problem

The router enforces full footprint clearance while the authoritative gate is center-based, a
divergence recorded in `POST_WORK_FINDINGS.md` ("Route stricter than the authoritative gate") and in
the scene-rendering skill's region gotcha. The user's decision is that the gate adopts the router's
predicate so route-admissible ⇔ gate-admissible (**I4**).

D9 shrinks this item: `Room::publish` no longer gates non-GM moves at all, so **no footprint is
needed there.** D4 touches `execute_move` and the router only.

### Footprint provenance — the blocker the campaign spec does not address

`Pathfind` carries a client-supplied `footprint_radius` and names **no token**
(`ws/protocol.rs:69-76`). The gate has a token and can derive; the router cannot. Without a shared
source, parity is unachievable — a client understating its footprint gets a preview the gate then
refuses.

Two parts:

1. **Server-side derivation.** `resolve_token_footprint(token) -> Option<f64>`, mirroring the client's
   `footprintRadius(eff)` (`src/client/core/src/actor.ts:177`) **exactly**, including the `0.4`
   fallback used when no effective actor resolves (`controller.svelte.ts:393`), resolved through the
   existing token→actor join precedence that `token_vision_floors` already implements. Pinned by a
   size-table parity test enumerating every `(shape, size)` pair and asserting equality with the
   client's values. This is the [[server-mirrors-client-resolver-semantics]] shape: verify against the
   client **source**, fail closed on degenerate input.
2. **`Pathfind` gains `token: Option<Uuid>`.** When present the server **derives** the footprint from
   that token and ignores any wire `footprint_radius`; the named token also serves as the existing
   non-GM presence proof, which strengthens that check rather than adding a second one. When absent,
   the wire radius is used and the result is an explicitly hypothetical preview with no parity claim.
   The measure tool already resolves its own footprint from the selected token, so real gameplay
   always takes the derived path.

This is a wire-protocol addition (ts-rs → Zod mirror + drift guard).

### Gate predicate

`execute_move`'s per-step check adopts, for non-GM movers only:

- **Wall clearance** — the token's bounding disc must clear every `blocksMove` segment
  (`point_segment_distance`), replacing the center-based `blocks_move` test. After D9 `execute_move`
  is the sole path for all non-GM movement, so its per-sample cost is the one that matters; the disc
  test is the same O(walls) order as the segment-cross test it replaces, so the change is not a
  complexity regression.
- **Mask membership over the footprint** — `footprint_cells(to) ∪ line_traversal(from, to)` must lie
  in the mask, the exact union `cell_enterable` applies. Both halves come from the resolved
  `GridShape`, never the free square functions (`pathfinding::footprint_cells`,
  `movement::supercover_cells`) — calling those reintroduces the square-on-hex defect Task 14e-7 fixed.
- **Regions over the footprint** — impassable and arrest become footprint-gated, matching the router.
  The center-cell-only asymmetry documented in the skill's gotcha is retired **by making the executor
  stricter**, which is the safe direction; the parity argument that gotcha protects is what **I4**
  now guarantees directly.

Accepted consequences (**user decision 4**): a wide token cannot move where any footprint cell is
unseen, so wide tokens freeze in corridors lit only along the centre; a wide token is arrested when
its footprint touches an arrest cell rather than when its center enters one. Both follow from ⇔ and
are the same shape as the documented, intended "dark scene freezes non-GM movement" outcome.

### Tests

- Round-trip parity, both directions, for a non-GM: a route `find` accepts is accepted step-for-step
  by `execute_move`, and a step `execute_move` refuses is absent from every route `find` returns.
  Exercised on **both** grid kinds and both movement models.
- Sub-0.5-cell footprint diagonal (the buddy-check P1 case) stays admissible on both sides.
- A wide token whose footprint overlaps an unseen cell is refused by the gate and absent from the
  route.
- Footprint derivation: server table equals client table for every `(shape, size)`; a `Pathfind`
  naming a token ignores a lying wire `footprint_radius`.
- GM is exempt from all three footprint checks.

---

## Out of scope (Phase D-β)

D3 (bounds unit + hex extent, incl. the `env_light_polys` hex defect), D1+D2 (cost model unification
and the continuous-cost unit bug — `PathResult.cost` is declared in cells at `ws/protocol.rs:288` but
the continuous engine reports scene units, so continuous route cost displays ~`cell`× too large),
D7 (`explored_fog` grid-kind **and** cell-size tagging + transactional re-index), D6 (lighting render
polish), D5 (verify + close).

## Verification gate

Client build precedes any `cargo` build (`rust-embed` compile-time `dist/` validation). Full gate:
`pnpm -r test`, `pnpm -r typecheck`, `pnpm lint`, `cargo test`, `cargo fmt`, `cargo clippy`.
Security-sensitive phase ⇒ two-reviewer pair (`shadowcat-spec-reviewer` + `shadowcat-code-reviewer`)
on the whole branch, plus the reviewed skill-update gate on
`shadowcat-codebase-scene-rendering` (the six-axis parity checklist, the wall-set invariants, the GM
exemption, and the region/footprint asymmetry gotchas all change).
