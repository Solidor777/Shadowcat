# M9 — Walls + Vision + Fog: Architecture Design Spec

> Status: **DRAFT for review** (authored autonomously for morning review per the
> "brainstorm M9" directive). A **cross-cutting architecture pass** over M9 — like
> the M8 parent spec — fixing the load-bearing decisions and decomposing into
> sub-milestones (M9a–M9c), each of which later gets its own brainstorm→spec→plan.
>
> M9's vision architecture was **already brainstormed in M8 §6.3** (pulled forward
> to drive the M8c render-layer API). The render seam is built: the M8c `Compositor`
> owns a mask slot + a `VisibilityInput { visible, explored? }` API (M8 shipped the
> identity case), and the `SceneDerived` channel (subscribe/computed-at-seq/coalesced,
> per-recipient) is built and proven by the M8 identity spike. **M9 fills that seam
> with real walls, server raycasting, a fog shader, and persistent fog.** This spec
> refines §6.3 and adds the wall/movement/fog-storage decisions §6.3 left open.

## 1. Goal

Walls that block sight (and optionally movement); server-authoritative per-player
vision computed by raycasting against walls; a three-state fog overlay (unexplored /
explored-but-not-visible / visible) with **persistent** explored memory; and a GM
vision mode. Geometric vision only — no lighting/illumination coupling (Phase 2).

## 2. Constraints inherited (cited inline)

- **#1/#4** Server-authoritative, per-recipient. A hidden wall or a region a player
  can't see is **never transmitted** to that player, not sent-then-hidden — vision
  filtering is *which derived data + which wall docs a recipient receives*.
- **#3 (vision exemption)** Vision is **server-authoritative without client
  prediction** — explicitly exempt from the optimistic path. A token move is still
  optimistic (M6); the *vision recompute it triggers* is server-authoritative and
  arrives via `SceneDerived` after the move's event (computed-at-seq).
- **#5** Walls are documents; the per-world ECS (M8a) hydrates them as vector
  segments; vision is derived runtime state (ephemeral, recomputed), never a document.
- **#6 (server structural-only)** The server runs no semantic game logic — **except**
  the engine-owned **geometric** systems: vision/fog **and movement-collision** (§5).
  Both are engine-owned (not module/system code), server-authoritative (#3), and read
  the same ECS-hydrated wall segments + token positions. (ARCHITECTURE.md #6 currently
  names only vision as the exception; **amend it to include movement-collision when M9a
  lands.**)
- **#7 (clean-room, ARCHITECTURE §7)** Raycasting / visibility-polygon / fog
  techniques come from **public computational-geometry literature only** — no
  proprietary VTT/engine source; no proprietary names in code. `geo` for polygon ops.
- **#10** Fog renders correctly on desktop + mobile; the fog shader is a 2D PixiJS
  filter (not a 3D pipeline — the `realtime-rendering` skill remains N/A).

## 3. Dependencies

- **M8a** (scene-entity docs + per-world hecs + `SceneDerived` egress) — walls are a
  new `doc_type`; vision is a derived system on the egress channel.
- **M8c** (render foundation: `Compositor` mask slot + `VisibilityInput` + the
  `subscribeScene` client plumbing) — M9 swaps real polygons for the identity case and
  adds the engine-owned fog shader behind the existing seam (zero API change, §6.3).
- **Scene lifecycle** (see [[scene-lifecycle-gap]]) — walls + vision are **per-scene**;
  M9 needs an active scene + scene creation, the same prerequisite M8d-2 surfaced.
  **M9 assumes that gap is resolved first.**
- **M8d** (the wall *tool* reuses M8d-2's interaction/tool API + drawing-entity
  pattern; movement-blocking integrates the M8d-2 move tool).

## 4. Walls (M9a)

**Decision: a wall is a scene-entity document** (`doc_type:"wall"`, `parent_id` =
scene, the uniform M8 §4 pattern), `system` body (client/engine-owned, server
structural-only):

```jsonc
// wall.system
{ "seg": { "x1": 0, "y1": 0, "x2": 100, "y2": 0 },  // a line segment, scene coords
  "blocksSight": true,
  "blocksMove": true }
```

- **Minimal in M9:** a wall is one segment + two booleans. **Deferred** (later
  milestones, not M9): doors (open/closed/secret/locked state), one-way/directional
  walls, terrain/sound walls, windows (see-but-not-move). The segment+flags shape
  admits these later as added `system` keys with no model change.
- **Hydration (M8a):** walls hydrate into the per-world hecs world as vector-segment
  components, the wall set the raycaster queries. Per-recipient (#4): a `gm_only` wall
  is not transmitted to players (it still blocks *their* vision because the **server**
  raycasts against the full wall set; players only never receive the wall *doc*).
- **Wall tool (M9a, client):** a `scene-tools` tool (M8d-2 module) — draw a wall
  segment (snap to grid/endpoints) → create a wall doc (optimistic). GM-gated.
- **Per-recipient wall visibility** is just document permission filtering (#4) — no new
  machinery; reuses M5's `PermissionContext`.

## 5. Movement blocking (M9a) — server-authoritative

**Decision (user 2026-06-22): movement-blocking is server-authoritative.** A token
drag is a **request**; the server executes it or denies it. On a token position-update
intent (the M8d-2 move tool's optimistic intent touching `/system/x`,`/system/y`), the
server intersects the move path against the scene's `blocksMove` wall segments — already
in the server's ECS for vision — and **rejects** the intent if it crosses (the M6
optimistic path rolls the client back). If denied, there is no movement.

- **Why server, not client:** a stray/modified-client drag past a wall must never
  produce an authoritative position behind it. Combined with server-authoritative
  vision (which recomputes only on *confirmed* moves), this guarantees a rejected move
  reveals **nothing** — no secret leak. This is the load-bearing reason (a peek can ruin
  the game), and it makes movement-collision a second engine-owned geometric exception
  to #6 (§2), reusing the vision ECS data.
- **Client UX nicety (advisory, not the authority):** the move tool MAY pre-clamp the
  drag against `blocksMove` walls client-side to avoid a snap-back flicker, but the
  **server is the gate**; the client pre-check is never trusted.
- **GM override:** a GM-only "ignore walls" move bypasses the server check (server
  honors the GM's authority), so the GM can reposition freely.

## 6. Vision (M9b) — server raycasting → SceneDerived → client mask

Per M8 §6.3, mostly determined:

1. **Server raycast** (engine-owned derived ECS system, Rust): for each player, take
   their controlled tokens' positions; cast a **visibility polygon** against the wall
   set (clean-room angular-sweep: rays to each wall endpoint ± ε, sorted by angle,
   nearest-hit per ray → polygon — *Source: standard 2D visibility-polygon / "ray
   casting to endpoints" computational-geometry technique*); **`geo`-union** the
   per-token polygons into one **per-player** visible polygon. Server-authoritative,
   exempt from optimism (#3).
2. **Dispatch** over the M8a/M8c **`SceneDerived` channel** — a real `vision` channel
   (vs M8's debug `identity`): per-recipient, leading-edge coalesced, carrying the
   **computed-at-seq** watermark so the client applies the mask only after the
   token-move event it derives from (the M8c-2 watermark guard already enforces this).
   **Payload = polygon geometry (D-V1)** — compact, resolution-independent; the client
   rasterizes. Recomputed fresh on resync (never replayed), per §5.
3. **Client render:** the `subscribeScene("vision", …)` consumer (M8c-2 plumbing) maps
   the polygon payload → `VisibilityInput.visible` → the M8c `Compositor` (replacing
   the identity empty-array). The **engine-owned fog shader** (the M8c mask slot's
   real consumer) masks the fog-affected layers.
- **Recompute triggers:** a token move/create/delete or a wall change in a scene
  re-runs that scene's vision for affected players (coalesced). This is the first real
  `SceneDerived` consumer — the M8 identity spike proved the transport end-to-end.

## 7. Fog (M9c) — persistent, three-state

Per M8 §6.3 D-V2/D-V3:

- **Three states** composited by the engine-owned fog shader: **unexplored** = black,
  **explored-but-not-visible** = dimmed, **visible** = clear. The M8c
  `VisibilityInput { visible, explored }` already carries both masks; M9c populates
  `explored` and ships the shader.
- **Persistent, server-authoritative, per-(scene, player) (D-V2):** explored area is
  path-dependent (not recomputable from current positions) → the **server stores +
  accumulates** it (union each new `visible` into the player's `explored` for that
  scene) and dispatches it. Consistent across a player's devices.
- **Storage shape (user 2026-06-22): BOTH modes, GM-configurable per scene, defaulting
  to the cell bitmap.**
  - **Default — sparse per-(scene,player) explored-cell set** (a "visited cells"
    bitmap): **bounded** (O(explored area), no growth on revisit), trivially accumulated
    (mark cells touched by `visible`), cheap to store/diff/dispatch, robust. Boundary is
    cell-quantized — fine for the *dimmed* memory layer. Sparse-by-cell-coord so it needs
    no scene dimensions.
  - **Opt-in — accumulated polygon union** (`geo::union` of every `visible`): crisp
    explored edges, no resolution choice; cost is unbounded vertex growth + union
    robustness over a long session. For GMs who want pixel-crisp explored memory and
    accept the cost.
  - The *live* `visible` mask is always a precise polygon (crisp current-vision edge);
    the GM setting only changes the *persisted explored* representation. The fog shader
    consumes both modes identically (it rasterizes whatever explored mask it's given).
- **GM vision mode (D-V3):** the GM is authoritative and receives everything (all
  walls, full scene); GM fog is a **client-side toggle** — "see all" (no mask) /
  "see as player X" (apply that player's visible+explored masks the GM also receives).
  No extra server path.

## 8. Decomposition

Dependency order **M9a → M9b → M9c** (build walls before vision; vision before fog):

- **M9a — Walls + server movement-blocking** *(docs + tool + server collision)*: the
  wall `doc_type` + hydration into the ECS wall set; the wall-draw tool (scene-tools);
  **server-authoritative** validation of token position-update intents against
  `blocksMove` walls (reject-on-cross → M6 rollback), with an optional client pre-clamp
  + GM override. No vision yet. Headless-testable (wall hydration; server-side
  segment-intersection collision math; reject path).
- **M9b — Server vision + live visibility mask** *(the core)*: the Rust raycaster +
  visibility-polygon + `geo`-union; the `vision` `SceneDerived` channel (per-recipient,
  coalesced, computed-at-seq); the client maps polygons → the M8c `Compositor`; the
  engine-owned fog shader's **two-state** form (visible vs not-visible, no persistence
  yet). Server raycasting is the heaviest unit — its own headless Rust tests
  (visibility-polygon correctness against known wall configs).
- **M9c — Persistent fog + GM vision mode**: per-(scene,player) explored-cell storage +
  accumulation + dispatch; the **three-state** fog shader; the GM see-all / see-as-player
  toggle. Closes M9.

Each sub-milestone: its own brainstorm→spec→plan→execute, buddy-checked per the M8
pattern.

## 9. Testability + provenance

- **Headless (Rust):** visibility-polygon correctness (known wall layouts → expected
  polygons), `geo`-union, per-recipient filtering, explored-cell accumulation,
  coalescing/computed-at-seq on the `vision` channel (reuse the M8a SceneDerived test
  harness). **Headless (TS):** the polygon→`VisibilityInput` mapping; client movement
  collision math.
- **Playwright (GL):** a player sees walls occlude vision; explored area dims; the GM
  toggle. Real fog shader in headless chromium.
- **Clean-room (#7 / ARCHITECTURE §7):** every vision/fog technique cites public
  computational-geometry literature; no proprietary VTT/engine source is consulted or
  named. Recorded per-file as the M8 render work did.

## 10. Decisions — **CONFIRMED (user 2026-06-22)**

1. **Movement-blocking is SERVER-authoritative** (§5) — drags are requests the server
   executes or denies; a denied move never produces an authoritative position (no secret
   leak). Second engine-owned geometric exception to #6. Client pre-clamp is an advisory
   UX nicety only; GM override bypasses.
2. **Persisted explored fog = BOTH modes, GM-configurable, default = sparse cell set**
   (§7); polygon-union opt-in. Live `visible` always a precise polygon.
3. **Wall = segment + `blocksSight`/`blocksMove`** in M9; doors / one-way / windows /
   sound deferred. **Confirmed.**
4. **`vision` is a new `SceneDerived` channel** reusing the M8c plumbing. **Confirmed.**
5. **Decomposition M9a/M9b/M9c.** **Confirmed.**
6. **M9 depends on the scene lifecycle** (M8d §15, approved) landing first.

## 11. Out of scope (PLAN M9 exclusions + deferrals)

- Photometric / illumination coupling (light sources, brightness) — Phase 2.
- Darkvision / tremorsense / height / elevation senses — Phase 2.
- Web-Worker optimistic/predicted vision — explicitly excluded (vision stays
  server-authoritative, #3).
- Doors / one-way walls / windows / sound walls / terrain (later).
- Multi-level / 3D vision; dynamic lighting; weather/atmosphere (Phase 3).
