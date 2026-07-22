# Server-Side Hex-Grid Movement Support — Design

**Status:** approved 2026-07-22. Base: `phase1-cleanup-burndown` branch @ `8ca326a` (post Phase-1
cleanup burndown, Tasks 1-49). Standalone sub-project, not part of the burndown plan itself —
surfaced by a user-directed TODO.md audit that found "hex-grid pathfinding" mis-filed as a
feature-gated deferral when hex grid was explicitly in original scope
(`docs/PLAN.md`: "grid (square / hex)") and the client already renders/measures hex correctly.

**Goal:** give the server the same hex-grid movement authority it already has for square grids —
wall-blocking, vision-mask gating, region gating, and A* pathfinding — so a hex scene's server-side
movement gate is not silently square-shaped underneath a client that renders hex correctly.

**One-line frame:** the server's movement/vision engine is currently **square-grid-shaped by
hardcoded assumption**, not by design choice — every cell-geometry primitive (`movement.rs`'s
line-traversal, `pathfinding.rs`'s A*, `scene/mod.rs`'s visibility-mask cell iteration,
`regions.rs`'s rasterization) computes directly against square cell math. This design generalizes
those primitives behind a `GridShape` abstraction so a scene's `engine.grid.kind` (already
wire-typed, already read by the client, currently unconsulted server-side) selects square or hex
cell geometry once, and every downstream consumer works unchanged either way.

---

## 1. Context: what already exists, what doesn't

**Client (already correct, unchanged by this design):** `src/client/render/src/grid.ts`'s `Grid`
class already implements pointy-top axial hex coordinates (Red Blob Games convention:
`pixelToAxial`/`axialToPixel`, `axialRound`, cube-coordinate axial distance) alongside its
existing square implementation, selected by `GridSpec.kind: "square" | "hex"`. Hex movement has no
`DiagonalRule` analog — `distance()` returns the unconditional axial distance for hex, only square
selects among the 4 diagonal-cost rules.

**Server (square-only today, the gap this design closes):**
- `scene/movement.rs`'s `supercover_cells(a0, a1, cell) -> Option<BTreeSet<Cell>>` — the shared
  line-traversal primitive BOTH the M1 move executor (`move_exec.rs`'s `gate_walk`) and the A*
  router's mask check (`pathfinding.rs`'s `cell_enterable`) use to enumerate every cell a move
  segment crosses. Hardcoded square DDA (Amanatides–Woo).
- `scene/pathfinding.rs`'s `PathGrid`/`cell_enterable`/A* search — square cell-center math,
  square footprint-disc-vs-AABB overlap (`footprint_cells`), square 8-neighbor expansion, the
  4-way `DiagonalRule` cost model.
- `scene/mod.rs`'s `accumulate_visible_cells` (feeds both `player_lit_mask`'s secrecy egress AND
  `visible_cells`/`visible_cells_cached`'s movement gate) — iterates a rectangular `(i, j)`
  bounding-box loop over square cell indices to build the visibility mask.
- `scene/regions.rs`'s `rasterize(shape, cell) -> Option<Vec<Cell>>` — square cell-center
  containment test against a region's continuous-coordinate shape (Rect/Circle/Polygon).
- `data/engine/scene.rs`'s `Grid { kind: String, size: f64, distance: Option<GridDistance> }` is
  already wire-typed and deserialized server-side (`SceneEngine.grid`) — it is simply never
  consulted by any of the four modules above, which all assume square unconditionally.

**Why this is genuinely one project, not "add a hex pathfinder":** the router's mask predicate
must be a superset of the move-executor's gate (an existing, load-bearing invariant in this
codebase — a route the router approves that the executor then rejects is a correctness bug, not
just an inconvenience). Since the mask itself comes from `accumulate_visible_cells`, a hex-aware
router built against a still-square-shaped mask would be internally inconsistent. Vision/mask,
movement gate, and pathfinding must all move to hex-awareness together.

## 2. Decisions

| # | Decision |
|---|---|
| H1 | **A `GridShape` abstraction, not parallel square/hex modules.** `movement.rs`, `pathfinding.rs`, `scene/mod.rs`'s cell-iteration code, and `regions.rs`'s `rasterize` are refactored to work against an abstract grid-shape interface (a small closed enum or trait — implementation detail decided during planning) providing: cell-center point, point-to-cell test, neighbor enumeration, and a line-traversal ("supercover") primitive. One code path serves both grid kinds. |
| H2 | **Two concrete implementations: `SquareGrid` and `HexGrid`.** `SquareGrid` is today's exact square logic, ported into the abstraction with byte-identical behavior (H3 proves this). `HexGrid` is new: pointy-top axial `(q, r)`, mirroring `grid.ts`'s exact coordinate formulas and `size` = outer-radius convention — no new coordinate convention invented. |
| H3 | **Regression safety: frozen-fixture parity proof before cutover**, mirroring this codebase's M10f-2 precedent for a comparably risky internal refactor. `SquareGrid` is proven byte-identical to the current hardcoded square behavior via a frozen fixture battery (wall-gated moves, mask/vision cell sets, A* routes, region rasterization results — reusing/freezing existing square-grid test cases as the oracle) BEFORE any call site is cut over or the old hardcoded logic is deleted. `HexGrid` is only built and wired in once that parity is proven. |
| H4 | **Hex movement cost is uniform 1-per-step; no `DiagonalRule` analog.** Matches the client's unconditional hex axial distance. The A* router's cost function for `HexGrid` has no 4-way rule to select — this is a simplification relative to square, not an added dimension. |
| H5 | **No hex equivalent of the square diagonal-corner-tie bug class.** Hex has 6 uniform neighbors, no orthogonal/diagonal split — the entire failure mode `supercover_cells`'s closed corner-tie bug belongs to (a per-axis remaining-step-budget gate on the diagonal branch) has no hex analog to design around. |
| H6 | **Hex line-traversal ("supercover" equivalent) is a clean-room cube-coordinate interpolation + hex-round per sample**, the standard approach (Red Blob Games' cited algorithm), matching this codebase's existing citation convention for public-domain computational geometry (ARCHITECTURE §7). |
| H7 | **Footprint radius stays a scene-space distance**, not a per-axis cell count — already computed this way for square (`footprint_radius_cells * cell` → a scene-unit disc radius), so it carries over to `HexGrid` unchanged; only the disc-vs-cell overlap test (`footprint_cells`'s AABB check) needs a hex-shaped equivalent. |
| H8 | **Scene resolves its `GridShape` once per call site from `SceneEngine.grid.kind`**, defaulting to `SquareGrid` on any unrecognized/malformed `kind` (fail-closed toward the already-hardened, already-tested behavior — never silently treat an unrecognized grid kind as hex). |

## 3. Scope boundaries

- **In scope:** wall-blocking line-traversal, visibility-mask cell iteration (secrecy + movement
  gate), region rasterization, A* pathfinding — full server-side parity with what square grids
  already have, for hex scenes.
- **Out of scope:** any new grid kind beyond square/hex (e.g. offset-coordinate hex, flat-top
  orientation) — the client only supports pointy-top axial hex today, so the server matches that
  exactly, not a generalized N-orientation system. Client-side changes — `grid.ts`'s hex math is
  already correct and is not touched by this design. Any change to the diagonal-rule cost model
  itself for square grids (H4 only concerns hex's lack of an equivalent).

## 4. Testing

- **Server, square-parity phase (H3):** a frozen fixture battery proving `SquareGrid`-via-the-new-
  abstraction produces byte-identical results to the current hardcoded square code, across wall-
  gated moves, mask/vision cell sets, A* routes, and region rasterization. This is the gate before
  any call site cuts over.
- **Server, hex phase:** unit tests for `HexGrid`'s cell-center/point-to-cell/neighbor/traversal
  math (mirroring `grid.ts`'s existing hex unit tests where a direct comparison is meaningful —
  e.g. the axial-distance formula should agree with the client's for the same inputs), then
  integration tests mirroring the existing wall/mask/region-gated move and A*-route test suites,
  run against a hex scene instead of square.
- **Client:** no behavior change expected (hex rendering/distance is already correct); add an
  end-to-end test confirming a hex-scene move request now genuinely round-trips through real
  server-side gating instead of relying on the client's own (already-correct) math being the only
  thing standing between a player and an illegal hex move.

## 5. Open questions for the implementation plan

These are genuinely deferred to `writing-plans`, not unresolved design gaps:
- Exact Rust shape of the `GridShape` abstraction (trait vs. closed enum + match) — a
  planning-time implementation-ergonomics choice, not a design-level one; either satisfies every
  decision above.
- Whether `HexGrid`'s footprint-disc-vs-hex-cell overlap test needs a bespoke hex geometry routine
  or can reuse `point_segment_distance`-style primitives against a hex cell's 6 edges — a
  planning/implementation-time detail.
