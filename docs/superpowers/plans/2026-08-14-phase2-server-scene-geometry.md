# Phase 2 — Server: Scene Geometry, Movement, Vision — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop the server's scene geometry from *assuming* that one grid cell is one cell-size of
world distance. Derive both the bounds-to-world-extent conversion and the cells-to-world-distance
conversion from the scene's own `GridShape`, through two shared symbols every consumer reads;
close the remaining movement/vision/fog defects in the same crate.

**Architecture:** `GridShape` already owns every other piece of per-cell geometry (cell centres,
point-to-cell, supercover, footprint overlap, candidate enumeration). It does **not** yet own the
two scalar conversions between grid space and world space, so each consumer re-derives them from
the raw `cell` scalar — correct on square, wrong on pointy-top hex, where adjacent centres sit
`√3 · size` apart and a `w × h` block of hexes spans neither `w × size` nor `h × size`. Two new
trait members close that: `world_units_per_cell()` and `world_extent(bounds_cells)`. Every
consumer of those two quantities reads one of those two symbols; **no call site keeps its own
conversion.**

A second, separate confusion runs through the same crate: the scene's **grid kind** is a decision
re-derived from a raw `"hex"` string comparison at one site, and is absent from two identities that
depend on it — the persisted explored-fog blob and the visible-cells cache key. A resolved
`GridKind` becomes the single source, carried by `ResolvedScene` and readable from any `GridShape`
already in hand, and both identities carry it.

**Tech Stack:** Rust (server crate `shadowcat`; `serde`, `sqlx`, `polyanya`, `geo`, `glam`).

**Spec:** `docs/superpowers/specs/2026-08-13-debt-burndown-campaign-design.md`

**Ledger ids covered:** PW1, PW2, PW3, PW4, PW5, PW31, TD17, TD18, TD19, TD48 — plus NEW-6, NEW-7,
NEW-8, NEW-9, NEW-10 and NEW-11, discovered while verifying this plan against source (see the
Ledger Additions section).

---

## Global Constraints

- **The campaign directive in the spec's §1 is copied verbatim into every subagent's first prompt.**
- **Report channel, stated in every dispatch:** return the report as the Agent tool's result, or
  send it via SendMessage, or write it to a named document. Dispatches are launched **without** a
  `name` — naming an agent backgrounds it and its final text reaches nobody.
- **Per-item disposition.** Every task reports one line per ledger id it touches, naming what was
  done and the evidence. "Category complete" is not an accepted report shape.
- **No suppressions.** `#[allow(...)]` and `#[expect(...)]` are both forbidden, `dead_code`
  included. A symbol with no caller is not annotated — it is wired to a real consumer, scoped to
  the build that needs it (`#[cfg(test)]`), or deleted. If no fix exists, **stop and ask.**
- **Never predict test-runner output.** No step in this plan states which tests will fail or how
  many, and no implementer report may substitute a prediction for an observation. Steps say RUN,
  OBSERVE, RECORD. A predicted outcome written by someone who never ran it gets read downstream
  as a measured one.
- **Never state the expected conclusion above a step that asks the implementer to derive it.**
  Where a step asks for a derivation or a measurement, this plan states the QUESTION and the METHOD
  only; the plan's own reading, where it has one, is recorded in a *separate* step that runs
  **after** the derivation is written down, and a disagreement between the two is a finding to
  report rather than an error to reconcile.
- **Every test carries a discrimination line** — the production change that would make it fail,
  checked against the test **as written**, not asserted. The check is mechanical: name the edit,
  then confirm the test's own call path REACHES the edited code. A test that passes its input
  through a helper the task does not change cannot discriminate, however the line is worded.
- **No test assertion sits on an exact floating-point equality boundary.** Where a test brackets a
  threshold, both sides are placed a stated distance off it, so a one-ULP difference in a correct
  implementation cannot decide the outcome. A test whose expected value is the threshold itself is
  a flake, not a pin.
- **Comments cite symbols, never file names or line numbers**, and never name a milestone, task
  id, phase, sweep, date, repo-document pointer, or the code's own history. **This binds test
  names and test comments**: a name states the constraint it pins, never why it exists or what
  once went wrong; a comment says what the code does now, never what it "was" or "still" does.
  **Narration by allusion is the same violation** — "rather than a sum over the traversal",
  "instead of the old box", "no longer skips" all describe a previous implementation without
  naming it. State the present constraint and stop. The automated checker cannot see this, so it
  is a review obligation.
- **Enumerate, do not sample.** Where a task claims every call site is covered, the step runs the
  search **in the step**, lists every hit, and gives each hit its own disposition line. A
  same-type parameter rename is exactly where the compiler goes silent, so enumeration is the only
  check that a site was not missed.
- **A pre-existing fixture that fails on an ASSERTION is re-derived, not re-baselined.** Where
  this plan changes behaviour a fixture pins, the task states that fixture's protected intent and
  the re-derived value that preserves it. Adopting whatever the new code emits is forbidden. Any
  fixture failure this plan does NOT name is a stop-and-report — and because that rule turns an
  incomplete enumeration into a halt, every task that moves a fixture enumerates the candidate set
  **from source, in a step**, before re-deriving anything.
- **No migrations.** SQL schema changes edit `src/server/migrations/0001_init.sql` in place.
  **TD48 needs no schema change** — the grid kind rides inside the existing `cells` BLOB, and the
  alternative (a `grid_kind` column) is considered and rejected in Task 2.
- **Cross-platform:** `std::path` only; no hardcoded separators. This phase adds no path handling.
- **Verification (server):** from `src/server/`, `cargo fmt --check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo test`. Rustdoc doctests are CI-blocking, so
  a doctest on a symbol whose signature changes must be updated in the same step.
- **Branch:** `phase2-server-scene-geometry`, merged `--no-ff` to local `main` after the
  whole-branch review. No push until the sub-project completes. History is never rewritten.

## Model/Effort directives

- **Implementation:** `shadowcat-coder`, `effort: medium`. On BLOCKED, re-dispatch to
  `shadowcat-coder-opus` (`effort: high`) before escalating to the human.
- **Review:** `shadowcat-spec-reviewer` + `shadowcat-code-reviewer`, `effort: high`, as a pair at
  every task gate. On shallow or uncertain findings, re-dispatch the `-opus` twin.
- Reviewers have **no shell** by directive: pre-generate the task diff to a file and relay gate
  output to them.
- **Execution loop:** Opus 5 at effort high. This phase rewrites the conversion every
  movement, vision, lighting and routing decision is scaled by, and a wrong conversion produces
  geometry that looks plausible in every square-grid test.

## Buddy-check directives

A buddy check is two blind reviewers plus a brokered debate to convergence, replacing both
single-reviewer stages for that task.

- **Task 5 (the shared conversion symbols) is buddy-checked (PHASE = code).** This is the
  strongest candidate in the phase and the reason is structural, not severity: it is the one
  change where getting the conversion wrong produces **plausible-looking geometry**. Every
  square-grid test in the repo passes under *any* wrong hex formula, because `SquareGrid`'s two
  conversions are identity against the existing code; the only signal is hex-specific, and hex
  coverage is thin (that thinness is itself a ledger item in this phase, PW3). A reviewer reading
  the diff sees arithmetic that "looks like the hex formulas" and a green suite. Blind independent
  derivation of the two hex formulas from the pointy-top axial convention — rather than review of
  the author's — is what actually checks it. This task also re-derives the fixture set the
  pre-dispatch measurement names; each reviewer states, per fixture, whether the re-derived value
  preserves the assertion's protected intent or re-baselines it.
- **Task 6 (the remaining conversion sites) is buddy-checked (PHASE = code).** It carries a
  classification judgement — which uses of `cell` are *authored quantities*, which are *internal
  subdivision densities*, and which are *deliberately unconverted* — and getting that wrong
  changes a secrecy-relevant sampler or a gameplay quantity while every square test stays green.
  The two reviewers classify all enumerated sites independently before seeing the author's table.
- **Task 4 (the hex footprint predicate) is buddy-checked (PHASE = code).** It changes the
  predicate three gates and one secrecy post-filter share, and its failure direction is
  under-inclusion, which is invisible.
- **Task 2 (grid-kind identity) is buddy-checked (PHASE = code).** It changes a persisted byte
  format and a cache key on the movement/secrecy gate; a wrong fail-direction on either is silent.
- **Task 1 (PW31) takes the standard two-reviewer gate**, with a directed question: reviewers must
  state explicitly whether the change can *reduce* the emitted cell set on any input, since
  under-inclusion is the one direction that admits a forbidden move. A "no change observed" report
  is not an answer to that question.
- **Task 3 (the scan window) takes the standard two-reviewer gate**, with a directed question:
  reviewers must state explicitly whether any scan that fits under `MAX_CELLS_PER_POLYGON` today
  can have its candidate set reduced by this change, and must answer it from the code path rather
  than from the window's size.
- Tasks 7, 8, 9, 10, 11, 12 take the standard two-reviewer gate.
- A **whole-branch** review runs before merge regardless.

## Ledger Additions (spec §2.4)

Six items were found while verifying this plan against source. All are folded into this phase's
tasks; all are appended to the spec's §4.4 ledger in Task 12, in the same commit that records the
dispositions.

| Id | Item | Task |
|---|---|---|
| NEW-6 | `VisibilityInputsSnapshot` — the value-comparison key for `visible_cells_cached`, which sits directly on the movement/secrecy gate — carries `cell` (the size) but not the grid KIND, and `ResolvedScene` carries no kind either. Changing a live scene's `grid.kind` while its size is unchanged leaves the snapshot byte-identical, so a square-indexed mask is served for a now-hex scene. Same root as TD48: grid kind is absent from an identity that depends on it. | 2 |
| NEW-7 | `SceneEcs::resolved_vision_modes` and `SceneEcs::resolved_bands` treat "the config document is absent" and "the config document is present but its typed `engine` will not decode" as the same outcome, silently. `resolved_vision_modes`' own inline comment states the opposite intent — that a present document must not be replaced by the built-in seed — and the code does exactly that. Sibling shape to PW5, one layer up. | 9 |
| NEW-8 | The per-source candidate-cell scan behind `SceneEcs::player_lit_mask` and `accumulate_visible_cells`, and the per-polygon scan behind `ExploredSet::mark_polygons`, hand `cells_in_bounds` a bbox derived from the scene extent and treat its over-cap `None` as "skip this source" — an EMPTY mask, which under `MovementRestriction::Visible` refuses every move and ships no lit cells. The cell count is proportional to the authored scene area, so the cliff is reachable by authoring large bounds, and correcting the extent (PW1) multiplies that count by the square of the cell size. A total loss is the wrong degradation for a gate. | 3 |
| NEW-9 | `HexGrid::footprint_cells` decides overlap by `dist(cell_center, ctr) <= r_scene + inradius`, and its doc claims that is "an always-safe over-approximation (a hex overlapping the true disc boundary is never excluded)". It is not: a hex overlapping the disc near a VERTEX has its centre up to `r_scene + size` (the circumradius) away, and `size > inradius`. The predicate is therefore correct only when `ctr` is the anchor hex's own centre, and two production callers already pass an off-centre `ctr` — `navmesh::clip_to_visible_mask` and `navmesh::los_smooth`, both of which anchor at an arc-length sample point. On a hex scene that under-includes footprint cells in the fog clip, which LOOSENS the secrecy post-filter (over-inclusion would tighten it; under-inclusion is the fail-open direction). | 4 |
| NEW-10 | `data::engine::token::Size`'s `w`/`h` docs, and `TokenOverrides.size`'s doc, say "scene units". The live reading is GRID UNITS (cells): `SceneEcs::resolve_token_footprint` derives `hypot(w,h)/2` and compares it against `MAX_FOOTPRINT_CELLS`, and the client's `resolveTokenBox` multiplies `actor.size.w` BY the cell size to reach scene units. `TokenEngine.w`/`h` genuinely are scene units, so the two structs disagree while sharing a field name and a doc sentence. | 6 |
| NEW-11 | `compute_derived`'s `"vision"` arm carries a `TODO:` asking for the gradation bands `player_lit_mask` already resolved to be threaded through, to avoid a second resolve. Both callers read the SAME resolver (`SceneEcs::resolved_bands`), so no value can fork; the question is only cost, and cost is now settled by `engine_as_cached`. The marker is a live to-do in a file this phase edits and the campaign closes to-dos rather than carrying them. | 9 |

---

## File Structure

| File | Responsibility in this phase |
|---|---|
| `src/server/src/scene/grid_shape.rs` | Owns the three new `GridShape` members `world_units_per_cell`, `world_extent` and `kind`, and their per-impl bodies. Owns the corrected `HexGrid::footprint_cells` predicate. The single home of both conversions. |
| `src/server/src/scene/mod.rs` | Declares `GridKind` and the pure `grid_kind_from`; `ResolvedScene` gains `grid_kind`; `resolve_grid_shape_with_rule` and `resolve_scene` both read the pure helper. Adds `SceneEcs::scene_world_extent`. Converts the weighted-continuous cost, both vision-range `dist_cells` computations, the four `bound_for_scene` consumers, and both `lighting_inputs_from` call sites. Windows both candidate scans. Deletes `SceneEcs::blocks_move`. Hosts the new hex integration tests. |
| `src/server/src/scene/grid_shape_parity_tests.rs` | Two of its frozen parity fixtures author scene bounds in world-unit terms and are re-derived by Task 5. No production code lives here; it is in every fixture enumeration this phase runs. |
| `src/server/src/scene/navmesh.rs` | `build_navmesh` takes a world extent and a pre-converted footprint distance instead of deriving either. |
| `src/server/src/scene/lighting.rs` | `env_light_polys` takes a world extent instead of deriving one; `cell_illumination`'s light-radius divisor reads the shared symbol. |
| `src/server/src/scene/vision.rs` | `bound_for_scene`'s `scene_bounds` parameter becomes `scene_extent` (world units) with a corrected doc. Pure rename plus doc; no arithmetic change. |
| `src/server/src/scene/move_exec.rs` | The cell-membership footprint anchors at the true walk point (TD19). Five of its scene fixtures author bounds in world-unit terms and are re-derived by Task 5. |
| `src/server/src/scene/explored.rs` | Owns `clamp_scan_window`; `mark_polygons` windows its scan when the scan exceeds the cap; `to_bytes`/`from_bytes` carry a self-describing header stamped with the grid kind. |
| `src/server/src/data/engine/token.rs` | Corrects the actor-size unit docs (NEW-10). |
| `src/server/src/ws/room.rs` | Reuses the `is_gm` binding already in scope (TD17); passes the resolved grid kind at its two explored decode sites; the animation duration reads the shared symbol. |
| `src/server/src/ws/conn.rs` | Passes the resolved grid kind at its explored decode/encode sites. |
| `docs/OPEN_BUGS.md`, `docs/CLOSED_BUGS.md`, `docs/TODO.md`, `docs/POST_WORK_FINDINGS.md` | Tracker sync (Task 12). |
| `.claude/skills/shadowcat-codebase-scene-rendering/SKILL.md`, `.claude/.claude-plugin/plugin.json` | Reviewed skill-update gate + plugin bump (Task 12). |

**Grid conventions, verified against source** — both implementations, so the formulas below are
derivations rather than recollections:

- `SquareGrid { cell, rule }`. `cell_center(c) = ((c.0+0.5)*cell, (c.1+0.5)*cell)`. Cell `(i,j)`
  covers `[i*cell,(i+1)*cell) × [j*cell,(j+1)*cell)`. Adjacent centres are `cell` apart on an
  orthogonal step; a `w × h` block anchored at the origin spans exactly `(w*cell, h*cell)`.
- `HexGrid { size }`, pointy-top axial, `size` = **outer** radius (circumradius, centre to vertex).
  `axial_to_pixel(q,r) = (size*(√3*q + √3/2*r), size*(3/2*r))`. Therefore:
  - all six neighbours of a hex are `√3 * size` from its centre — `(1,0)` gives `(√3*size, 0)`;
    `(0,1)` gives `(√3/2*size, 3/2*size)`, whose length is `size*√(3/4 + 9/4) = √3*size`;
  - a pointy-top hex's half-extents are `√3/2 * size` in x (half the across-flats width, the
    INRADIUS) and `size` in y (the circumradius, centre to the top/bottom vertex);
  - the origin hex `(0,0)` is centred at the pixel origin, so an axial block anchored at `(0,0)`
    extends to `y = -size` and `x = -√3/2*size`, i.e. into NEGATIVE coordinates.

**Task order and why it is this order.**

- **Task 1 runs first**, per the spec's §5: an item whose cause may already have been fixed
  elsewhere is re-verified before any new work.
- **Task 2 (grid kind) runs second.** It consumes nothing from any other task, and until it lands
  the visibility cache's value-comparison key omits a value the mask depends on. Every later task
  in this phase changes mask geometry, so leaving it later leaves a window in which the mask
  depends on the grid kind while the key guarding it does not — a wider version of the very defect
  the task closes. The reorder is free.
- **Task 3 (scan windowing) runs before Task 5 (the extent conversion)** because Task 5 multiplies
  the scan's cell count by the square of the cell size, and Task 3 is what turns the resulting
  over-cap outcome from a total mask loss into a bounded subset; landing them the other way round
  leaves a commit in which a large scene has no mask.
- **Task 4 (the hex footprint predicate) runs before Task 11 (the executor's footprint anchor)**
  because Task 11 introduces an off-centre `ctr` at a site that currently only ever passes a cell
  centre, and Task 4 is what makes an off-centre `ctr` correct.
- **Task 6 runs after Task 5**, which produces the symbol it converts four further sites onto.
- **Task 7 runs after Tasks 4, 5 and 6**, because the hex scene it exercises runs through all
  three; **Task 8 runs after Task 5**, because it verifies the post-conversion environment light.
- **Task 5c (three comment/fixture classes) runs after Task 5b**, not because it depends on the
  envelope but because both edit the same test modules and two agents editing one module is the
  clobber hazard. It is comments and test-local constants only; no production behaviour changes, and
  a changed test outcome under it is a finding rather than something to accommodate.
- **Task 5b (the envelope) runs between Tasks 5 and 6**, and before Tasks 7 and 8. It changes the
  return type of the symbol Task 5 introduced, so running it after Task 6 would convert four further
  sites onto a signature that then changes under them, and Task 8's environment-light verification
  would be verifying a perimeter walk that this task moves. It is numbered `5b` rather than
  renumbering Tasks 6–12, whose numbers are already cited by dispatched briefs and ledger entries.

---

## Pre-dispatch measurement — which fixtures the extent conversion moves

**This section is run by the DISPATCHER, in a shell, BEFORE Task 5 is dispatched. It is not a step
inside Task 5, and it cannot be answered by reading.** Whether a fixture's assertions survive the
extent conversion depends on where its scene's vision bound lands relative to its wall geometry and
its asserted cells — a question about the interaction of four values, not about the source text of
any one of them. Task 5's own stop-and-report rule turns an incomplete list into a halt, so the
list is measured first and **recorded into Task 5 Step 9's table before the dispatch**. Step 9, not
Step 8: Step 8 is the step whose whole purpose is an enumeration the plan has not anchored, and
pasting a measured failure list above it would anchor exactly what it asks the implementer to
derive.

**Where in the phase it runs, and why the position is load-bearing.** On the phase branch with
Tasks 1–4 already committed, immediately before Task 5 is dispatched — not at the start of the
phase. Two mechanical reasons:

- Task 3 changes what an over-cap candidate scan does, and the extent conversion is what pushes a
  scan over the cap in the first place (it multiplies every scan's cell count). Measured before
  Task 3, a scan that goes over-cap yields an EMPTY mask; measured after it, a clamped subset.
  Those are different failure sets, and only the second is the one Task 5 lands into.
- The probe below reads `SceneEcs::resolve_grid_shape` for a hex scene's shape, and the grid-kind
  decision behind it is Task 2's.

**The probe applies the conversion on BOTH grid kinds, and covers all three consumer families.**
A square-restricted probe would have its blind spot exactly where this phase's risk is: `bounds ×
cell` IS `SquareGrid::world_extent`, so a probe built from that product measures nothing on hex,
where the real conversion is strictly larger on both axes. A hex fixture that moves would then be
absent from the recorded table and Task 5's halt rule would fire on a CORRECT implementation — the
same failure class this plan has already caught once. Nor is the hex case hypothetical here:
`hex_continuous_scene_docs` authors no bounds at all, so it falls back to
`DEFAULT_SCENE_BOUNDS_UNITS` and reaches `build_navmesh` through `SceneEcs::navmesh_for`, and
`move_exec::tests::scene_with_narrow_gap_and_wide_token` takes the grid kind as a parameter and is
instantiated for both. So the navmesh builder and the environment-light walk are probed too: those
two already compute `bounds × cell` themselves, which makes them square-INERT, and square-inert is
not inert.

- [ ] **M1: Enumerate every scene fixture that authors bounds, and the category that does not**

Run and record the full output:

```bash
cd /c/Dev/Shadowcat && git grep -n '"bounds"' -- src/server
cd /c/Dev/Shadowcat && git grep -n "DEFAULT_SCENE_BOUNDS_UNITS" -- src/server
```

Record, per hit, the enclosing fixture or test function name, the authored numbers, and the
scene's grid kind. Then record the second, uncounted category explicitly: **every scene fixture
that authors NO bounds falls back to `DEFAULT_SCENE_BOUNDS_UNITS`, which is in GRID units**, so its
extent changes from that literal pair to that pair's world conversion — the pair times the cell
size on square, and the strictly larger hex closed form on hex. Those fixtures do not appear in the
grep at all, which is why the probe measures them rather than this section enumerating them.

A grid kind recorded from a literal `"kind": "hex"` spelling is not the whole set: a fixture may
take the kind as a PARAMETER. `move_exec::tests::scene_with_narrow_gap_and_wide_token` is one such,
and naming it here is an input to the search rather than an example of it — search for the
parameterised shape as well, and give every fixture you find its own line.

- [ ] **M2: Apply the grid-kind-correct probe patch on a scratch branch**

```bash
cd /c/Dev/Shadowcat && git checkout -b phase2-extent-probe
```

The probe carries ONE expression of the conversion and routes every consumer through it. It does
**not** inline a second copy of the hex closed form beside Task 5's: a second copy that disagrees
produces a wrong measurement, and a scratch branch does not make a forked decision safe.

1. In `src/server/src/scene/grid_shape.rs`, add
   `fn world_extent(&self, bounds_cells: (f64, f64)) -> (f64, f64)` to the `GridShape` trait, with
   the two impl bodies **Task 5 Step 3 specifies, verbatim**. The module denies missing docs on
   private items as well as public ones, so give the trait member a one-line doc so it compiles;
   the real doc is Task 5's to write.
2. `scene::source_los_poly` — add a temporary `extent: (f64, f64)` parameter and pass it to
   `bound_for_scene` in place of `scene_bounds`. Update its two callers
   (`accumulate_visible_cells`, `player_lit_mask`'s per-source loop), both of which already hold a
   resolved shape and the settings, to pass `grid.world_extent(settings.bounds)` — read the real
   binding name at each site, since one is `grid` and one is `cell_grid`.
3. `SceneEcs::player_vision_polygons` — replace the `scene_bounds` binding with
   `self.resolve_grid_shape(scene, cell).world_extent(self.resolve_scene(scene).bounds)`, reading
   `cell` from `self.scene_grid_sizes()`.
4. `SceneEcs::player_vision_inputs` — the same, on BOTH the early return and the populated
   construction.
5. `SceneEcs::navmesh_for` — `build_navmesh` multiplies its `bounds` argument by `cell` internally,
   so hand it a PRE-DIVIDED value rather than changing its signature for a scratch run: compute
   `let (ex, ey) = self.resolve_grid_shape(scene, cell).world_extent(<the scene's resolved
   bounds>);` and pass `(ex / cell, ey / cell)` where the authored bounds went, so the product it
   computes is the extent. Read the real bounds expression at that site rather than assuming a
   binding name. Guard the division — when `cell` is not finite or not positive, pass the authored
   bounds unchanged, since both readings fail closed there and a `0.0 / 0.0` would measure a NaN
   path neither the current nor the landed code takes.
6. `SceneEcs::lighting_inputs` and `SceneEcs::visible_cells_cached` — the two callers of the
   associated `lighting_inputs_from`, which forwards its `bounds` to `env_light_polys`, which
   likewise multiplies by `cell_size` internally. Apply the same pre-divided value with the same
   guard at BOTH. Converting one and not the other forks the cached mask from the uncached one, and
   the measurement would then record a divergence of its own making.

This is a scratch measurement, not a draft of the change: it is never reviewed, never merged, and
the branch is deleted at M4.

**If Task 5's buddy check changes either hex formula, this measurement is void and is re-run**
against the corrected formula before Task 5's fixture re-derivation is finalised. A failure table
produced by a formula the phase then rejected is worse than no table, because the halt rule treats
it as complete.

- [ ] **M3: Run the whole server suite and record every failure**

```bash
cd /c/Dev/Shadowcat && pnpm build
cd /c/Dev/Shadowcat/src/server && cargo test 2>&1 | tee /c/Dev/Shadowcat/debug/dumps/extent-probe.txt
```

`pnpm build` first: `rust-embed` validates `dist/` at compile time, so a cargo build without it
fails for an unrelated reason and the measurement records nothing.

RUN, OBSERVE, RECORD. Produce a table with one row per failing test: its full path, its assertion
message, the fixture it reads its bounds from, and that fixture's grid kind. **Do not fix anything
on this branch.** The output of this step is the input to Task 5 Step 9 and to Task 5's own
stop-and-report rule: a fixture that fails during Task 5 and is absent from this table is a genuine
surprise and halts; one that is present is re-derived with its intent stated.

- [ ] **M4: Discard the probe branch and confirm the tree is clean**

```bash
cd /c/Dev/Shadowcat && git checkout . && git checkout phase2-server-scene-geometry && git branch -D phase2-extent-probe && git status --short
```

`git status --short` must print nothing. A probe left in the tree and a landed change produce
identical test output, so confirming the discard is part of the measurement, not housekeeping.
Delete `debug/dumps/extent-probe.txt` once its table has been copied into Task 5 Step 9; dumps are
ephemeral and are never committed.

---

### Task 1: PW31 — verify the corner-drift fix does not subsume it, then remove the drift at its cause

**Ledger id:** PW31. **Runs first**, per the spec's §5: an item whose cause may already have been
fixed elsewhere is re-verified before any new work, because patching a symptom whose cause was
fixed elsewhere is how a repeat failure gets hidden.

**Files:**
- Modify: `src/server/src/scene/movement.rs` (`supercover_cells`)
- Test: `src/server/src/scene/movement.rs` (`mod tests`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: no new public symbols. `supercover_cells` keeps its signature
  `pub fn supercover_cells(a0: (f64, f64), a1: (f64, f64), cell: f64) -> Option<BTreeSet<Cell>>`.

- [ ] **Step 1: Derive, from the code, whether the shipped fix subsumes the entry — and write the derivation down BEFORE reading Step 2**

Read `supercover_cells` in full. Answer these three questions in the task report, quoting the
symbols involved, and do not read Step 2 until all three answers are written:

1. What did the shipped corner-drift fix change? Name the bindings it introduced and the branch it
   gated.
2. What is the tie predicate itself — the exact expressions bound to `tol` and `tied` — and does
   the fix's gate appear inside either expression?
3. Therefore: can a near-corner **with steps still owed on both axes** still satisfy `tied` and
   emit both flanking cells?

- [ ] **Step 2: Compare your derivation against the plan's reading, and report any disagreement**

The plan's own reading, recorded here so Step 1 could not be anchored by it:

> The budget gate bounds *how often* the tie branch can fire; it does not change what `tied`
> detects. A mid-path near-corner with both budgets positive still takes the branch. The entry is
> not subsumed.

If your Step 1 answers agree, say so and proceed. **If they disagree, that disagreement is the
finding: stop and report it**, quoting both your derivation and this reading. Do not adjust either
one to match the other.

- [ ] **Step 3: Add the superset oracle and the staircase pin — inside the function's own span cap**

`supercover_cells` must always be a **superset** of the truly-crossed set; the entry is about it
being a **strict** superset on inputs that do not actually touch a lattice corner.

`supercover_cells`'s span guard is the bounding box of the two ENDPOINT CELLS, not the path
length: `span = (|ci-ei|+1) * (|cj-ej|+1)`, refused above `MAX_MOVE_CELLS = 1_000_000`. A diagonal
of `n` cells therefore has `span = (n+1)^2`, so `n` must satisfy `(n+1)^2 <= 1_000_000`, i.e.
`n <= 999`. Both tests below stay inside that bound; a test that exceeds it does not measure
over-detection, it measures the span guard, and `.expect` turns that into a panic.

`HexGrid`'s test module already carries this exact shape of oracle (`true_hexes_crossed`, a dense
midpoint sampler with an epsilon) and its own deterministic PRNG. Mirror that structure here
rather than inventing a second one; read it first.

Add to `mod tests` in `src/server/src/scene/movement.rs`:

```rust
    /// Cells the segment provably enters, by dense midpoint sampling. A SUBSET oracle: it can
    /// miss a cell the segment only grazes, so it may only ever be compared as
    /// `oracle ⊆ supercover`.
    fn densely_sampled_cells(a: (f64, f64), b: (f64, f64), cell: f64, n: usize) -> BTreeSet<Cell> {
        let mut out = BTreeSet::new();
        for k in 0..=n {
            let t = k as f64 / n as f64;
            let x = a.0 + (b.0 - a.0) * t;
            let y = a.1 + (b.1 - a.1) * t;
            out.insert(((x / cell).floor() as i32, (y / cell).floor() as i32));
        }
        out
    }

    #[test]
    fn supercover_never_omits_a_densely_sampled_cell() {
        // The safety direction: the emitted set may never LOSE a cell the segment demonstrably
        // enters. Endpoints stay within ±200 at cell 10, so the endpoint-cell bounding box is at
        // most 41×41 and the span guard cannot fire.
        // Discrimination: fails on any edit that drops a cell from the emitted set on any of the
        // 2000 sampled segments — including a narrowed corner tolerance, which is what makes this
        // the pin that a tolerance change is not free.
        let cell = 10.0;
        let mut seed = 0x5EED_1234_u64;
        let mut next = |lo: f64, hi: f64| -> f64 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            lo + ((seed >> 11) as f64 / (1u64 << 53) as f64) * (hi - lo)
        };
        for _ in 0..2000 {
            let a = (next(-200.0, 200.0), next(-200.0, 200.0));
            let b = (next(-200.0, 200.0), next(-200.0, 200.0));
            let sc = supercover_cells(a, b, cell).expect("finite endpoints inside the span cap");
            for c in densely_sampled_cells(a, b, cell, 4000) {
                assert!(sc.contains(&c), "{a:?}->{b:?} omits {c:?}");
            }
        }
    }

    #[test]
    fn a_corner_free_long_diagonal_emits_a_plain_staircase() {
        // A segment offset off the exact diagonal by a fraction of a cell crosses each vertical
        // grid line strictly before or after the matching horizontal one, so no step is a corner
        // crossing and the emitted set is a staircase: exactly one new cell per crossing, never a
        // flanking PAIR. `n = 900` keeps the endpoint-cell bounding box at 901×901 = 811_801,
        // inside `MAX_MOVE_CELLS`.
        //
        // Discrimination: the assertion is a COUNT, so it fails on any edit that emits a flanking
        // pair on a step whose two crossings are separated. It does NOT restate the traversal:
        // the expected count is `2n + 1` from the staircase geometry, and the accompanying
        // superset check below re-derives the same set from the dense-sample oracle.
        //
        // SCOPE, stated so this test is not read as more than it is: the two crossings on each
        // step of this construction are separated by `0.15/n` in the parametric variable, which is
        // many orders of magnitude above the tolerance `tol` evaluates to at these magnitudes. It
        // is therefore a REGRESSION PIN on the staircase property, not a reproducer of a tie
        // firing on separated crossings. A construction that reproduces that would need `tol` to
        // reach the crossing separation, which it does not at any coordinate magnitude the span
        // guard admits.
        let cell = 1.0;
        let n = 900_i32;
        let a = (0.25, 0.4);
        let b = (a.0 + n as f64, a.1 + n as f64);
        let sc = supercover_cells(a, b, cell).expect("within the span cap");
        assert_eq!(
            sc.len(),
            (2 * n + 1) as usize,
            "a corner-free long diagonal must produce a plain staircase, got {} cells",
            sc.len()
        );
        for c in densely_sampled_cells(a, b, cell, 20_000) {
            assert!(sc.contains(&c), "staircase omits {c:?}");
        }
    }
```

Run: `cd src/server && cargo test --lib scene::movement`

**RUN, OBSERVE, RECORD.** Do not state in advance which of the two tests passes. Record the exact
runner output for both in the task report. Three outcomes, each with a defined next move:

- Both pass ⇒ the over-inclusion is not reachable by this construction. **Widen the search once**:
  sweep `a` offsets over `k/64` for `k in 1..64` on both axes, at `n = 900` (the same span-cap
  bound applies to every sweep member — a sweep at a larger `n` measures the span guard, not the
  traversal), record the outcome, and proceed to Step 4 anyway. Step 4's change is a cause fix
  that stands on its own — it removes accumulated floating-point error from a comparison whose
  tolerance exists to absorb exactly that — independent of whether one reproducer was found.
- The staircase test fails ⇒ the over-inclusion is reproduced. Record the observed cell count and
  proceed to Step 4.
- The superset test fails ⇒ **stop and report immediately.** That is an under-inclusion, a
  security defect of a different and more serious class than PW31, and it is not this task's
  scope to fix silently.

- [ ] **Step 4: Remove the accumulation, which is the cause the tolerance exists for**

The tolerance is 64 ULPs *relative to the running magnitude of `t_max_i`/`t_max_j`*, and its own
comment states why: `t_max_i` and `t_max_j` are built by repeatedly **adding** `t_delta_i` /
`t_delta_j`, so each accumulates an independent error sum that "can far exceed `f64::EPSILON`".
The two values being compared are therefore drifting estimates of quantities that have exact
closed forms. Recompute them instead of accumulating.

In `src/server/src/scene/movement.rs`, delete the `t_delta_i` / `t_delta_j` bindings entirely
(they have no other use, and a suppression is not an option), and replace each of the three
`t_max_* += t_delta_*` updates with a recomputation from the cell just stepped into:

```rust
        let tol = (t_max_i.abs() + t_max_j.abs() + 1.0) * f64::EPSILON * 64.0;
        let tied = (t_max_i - t_max_j).abs() < tol;
        if tied && remaining_i > 0 && remaining_j > 0 {
            // Genuine corner crossing with path remaining on both axes: emit BOTH flanking
            // cells (supercover), then step diagonally.
            out.insert((ci + step_i, cj));
            out.insert((ci, cj + step_j));
            ci += step_i;
            cj += step_j;
            remaining_i -= 1;
            remaining_j -= 1;
            t_max_i = next_boundary(ci, step_i, x0, dx);
            t_max_j = next_boundary(cj, step_j, y0, dy);
        } else if remaining_j == 0 || (remaining_i > 0 && t_max_i < t_max_j) {
            // Either j has already arrived at ej (must not overshoot it), or i is genuinely the
            // next boundary crossed — step i alone.
            ci += step_i;
            remaining_i -= 1;
            t_max_i = next_boundary(ci, step_i, x0, dx);
        } else {
            // Either i has already arrived at ei, or j is the next boundary crossed.
            cj += step_j;
            remaining_j -= 1;
            t_max_j = next_boundary(cj, step_j, y0, dy);
        }
```

`next_boundary` is the closure already defined above the loop and already parameterised on the
current cell index, so this is the same quantity computed exactly rather than incrementally.

Replace the tolerance comment with one stating the present constraint. It must not describe an
alternative implementation, not even by allusion — no "rather than", no "instead of", no comparison
to a running sum:

```rust
        // Corner-crossing tolerance. `t_max_i`/`t_max_j` are each computed from the current cell
        // index by one subtraction and one division, so each carries that much rounding error;
        // the relative form scales the tolerance with the magnitude of the parametric values
        // being compared.
        //
        // When one component is INFINITY (axis-aligned move): INF - INF = NaN; NaN < any finite
        // is false, so no corner branch fires — correct, axis-aligned steps are single-axis.
        //
        // Safe failure direction: over-detecting a near-corner only over-includes flanking cells
        // (rejects a fine move), never under-includes (never lets a forbidden move through).
```

**Do not narrow `tol`.** The tolerance's job is to catch a true corner; shrinking it trades a
visible false reject for an invisible false accept, and the invisible one is the security defect.
The constant is CONSERVATIVE relative to the error it has to absorb, which is the correct state for
a gate tolerance and is not a reason to retune it. If Step 6's run shows over-inclusion surviving,
that is the finding to report, not an input to a smaller constant.

- [ ] **Step 5: Rewrite the one existing test comment the change falsifies**

`long_nonsymmetric_diagonal_corner_both_flankers_present`'s comment describes the accumulated
values at the corner (a stated difference of `3.33e-16` between two running sums) and attributes
the test's outcome to the relative tolerance absorbing that drift. After Step 4 both values are
computed exactly, so the comment describes arithmetic the function does not perform.

Replace that comment block with one stating the present constraint — what the segment is, which
corner it crosses, and which two flanking cells must be present — with no reference to
accumulation, to an absolute-epsilon alternative, or to a prior implementation:

```rust
    #[test]
    fn long_nonsymmetric_diagonal_corner_both_flankers_present() {
        // Segment (0,0)→(14,4) at cell = 1: dx = 14, dy = 4, so the step from cell (13,3) to
        // (14,4) is a genuine corner crossing — the segment reaches x = 14 and y = 4 at the same
        // parametric value. Supercover requires BOTH flanking cells there, not just the diagonal
        // pair: a thin line would let a move thread (14,3)/(13,4) unseen.
        // Discrimination: fails if the corner branch stops firing for a non-symmetric diagonal —
        // the two flanker assertions are the corner's signature and no staircase produces them.
        let c = supercover_cells((0.0, 0.0), (14.0, 4.0), 1.0).expect("within cap");
        assert!(c.contains(&(0, 0)), "start cell");
        assert!(c.contains(&(14, 4)), "end cell");
        assert!(c.contains(&(14, 3)), "flanker (14,3) at the lattice corner");
        assert!(c.contains(&(13, 4)), "flanker (13,4) at the lattice corner");
    }
```

Then re-read every other comment in `mod tests` and in `supercover_cells` itself; treat each as
stale until checked against the post-Step-4 body, and rewrite any that narrates accumulation —
including by allusion. Report each comment you changed and each you checked and left.

- [ ] **Step 6: Run the tests and the existing suite; answer the directed question**

Run: `cd src/server && cargo test --lib scene::movement && cargo test --lib scene::grid_shape && cargo test --lib scene::move_exec`

RUN, OBSERVE, RECORD. Report:
- the observed result of both new tests;
- the observed result of the pre-existing supercover and grid-shape parity suites, by name;
- an explicit statement, supported by the superset test's output, that no input was observed to
  lose a cell.

If any pre-existing test changes outcome, **stop and report** with its name and message.

- [ ] **Step 7: Mutation check — prove the tie branch is still load-bearing**

Temporarily change `tied` to the constant `false`, then run
`cd src/server && cargo test --lib scene::movement`.

Do not predict which assertions fail or how many — an `assert!` panic unwinds the whole `#[test]`
function it runs in, so two assertions in one test can never both be observed to fail in one run;
only the runner's actual output is evidence. Instead:

- Record the observed failing test names and messages.
- Confirm the mutation is detected **at all**. If the suite stays green, the corner branch is not
  covered by any test and that is the finding to report — stop rather than proceeding.
- Revert the edit, re-run, and confirm green.
- Confirm the revert landed by diffing the file against its pre-mutation state. A mutation that
  never took effect and a test that does not gate produce identical output.

- [ ] **Step 8: Run the full server gate**

Run: `cd src/server && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

- [ ] **Step 9: Commit**

```bash
git add src/server/src/scene/movement.rs
git commit -m "fix(scene/movement): compute supercover crossings exactly, not incrementally

The corner-tie tolerance scales with the magnitude of two parametric values
built by repeated addition, so each carried a sum of rounding error over the
whole traversal. Both are now recomputed from the current cell index, leaving
one subtraction and one division of error each. The tolerance is unchanged and
is now conservative for the error it absorbs: over-detection over-includes a
flanking cell, while under-detection would admit a move across an unseen one.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>" -- src/server/src/scene/movement.rs
```

---

### Task 2: TD48 + NEW-6 — the grid kind becomes part of every identity that depends on it

**Ledger ids:** TD48, NEW-6. **Buddy-checked (PHASE = code).**

**Why this runs second.** It consumes nothing and produces nothing any other task needs, so its
position is free — and every later task in this phase changes what the visibility mask contains.
Landing the kind on `ResolvedScene` first means the cache key guarding that mask carries the grid
kind for the whole of the rest of the phase, instead of the phase running with a key that omits a
value the mask depends on.

**Files:**
- Modify: `src/server/src/scene/mod.rs` (`GridKind`, `grid_kind_from`, `resolve_grid_kind`,
  `ResolvedScene`, `resolve_scene`, `resolve_grid_shape_with_rule`)
- Modify: `src/server/src/scene/grid_shape.rs` (`GridShape::kind` and both impls)
- Modify: `src/server/src/scene/explored.rs` (`to_bytes`, `from_bytes`)
- Modify: `src/server/src/ws/conn.rs`, `src/server/src/ws/room.rs` (the explored decode/encode
  sites)
- Test: `src/server/src/scene/explored.rs`, `src/server/src/scene/grid_shape.rs`,
  `src/server/src/scene/mod.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `pub enum GridKind { Square, Hex }` in `scene` (deriving `Debug, Clone, Copy, PartialEq, Eq`)
  - `scene::grid_kind_from(eng: Option<&eng::SceneEngine>) -> GridKind` (private, pure)
  - `SceneEcs::resolve_grid_kind(&self, scene: Uuid) -> GridKind` (`pub(crate)`)
  - `ResolvedScene.grid_kind: GridKind`
  - `GridShape::kind(&self) -> GridKind`
  - `ExploredSet::to_bytes(&self, kind: GridKind) -> Vec<u8>`
  - `ExploredSet::from_bytes(b: &[u8], kind: GridKind) -> Self`

  Task 3 consumes `GridShape` but not `kind`; no later task consumes any of these.

**Why the tag rides in the blob and not in a column.** A `grid_kind` column on `explored_fog`
would work and needs no migration (the baseline is edited in place). It is rejected because the
repository's contract for this row is *opaque bytes* — `get_explored`/`set_explored` traffic in
`Vec<u8>` and know nothing about cells — so a column would put geometry knowledge in the schema
and leave the byte format still unversioned. A self-describing blob puts the constraint where the
geometry lives, gives the format a version in the same move, and makes a headerless blob
detectable rather than silently reinterpretable.

**What this costs an existing world, stated rather than left to a reader.** Every fog blob already
persisted is headerless, so `from_bytes` refuses all of them and each affected scene's remembered
fog starts empty again. That is deliberate and it is the safe direction: an empty `ExploredSet`
under-reveals (a `Revealed`-restriction scene falls back to what is currently visible, and the fog
re-accumulates on the next vision recompute), whereas decoding a headerless blob would mean
guessing the coordinate system its indices are in — the exact reinterpretation this task exists to
make impossible. There is no upgrade path to write because there is nothing to preserve: fog memory
is derived data that regenerates from play, not authored content, and the project takes no data
migrations before customers. No conversion pass is written, and none is needed.

**Why `GridShape::kind` rather than a parallel map.** `ws::conn::enrich_vision_explored` already
receives two per-scene maps (`grid` sizes and `grid_shapes`) built under the ECS lock, and needs
the kind at a decode that happens after the lock is released. A third parallel map is three
per-scene values that must agree by convention — the forked-decision shape this codebase produces
most. A `kind()` on the shape makes them agree by construction: `resolve_grid_shape_with_rule`
constructs the shape FROM the resolved kind, so `shape.kind()` cannot disagree with
`resolve_grid_kind`. Step 2's parity test (`the_resolved_shape_reports_the_resolved_kind`) pins
that.

- [ ] **Step 1: Enumerate the existing explored-blob fixtures this changes, from source**

Two pre-existing tests in `src/server/src/scene/explored.rs` assert against the byte layout and
therefore move when the layout gains a header. Enumerate them rather than trusting this list:

```bash
cd /c/Dev/Shadowcat && git grep -n "to_bytes\|from_bytes" -- src/server
```

Record every hit with its enclosing function. Give each a disposition line. The two that fail on an
ASSERTION are re-derived, with the intent preserved:

| Fixture | Protected intent | Re-derivation |
|---|---|---|
| `round_trips_through_bytes_deterministically` | A marked set serializes to an exact, deterministic byte length and reads back equal. | `to_bytes(GridKind::Square)` / `from_bytes(&bytes, GridKind::Square)`; the length assertion becomes `EXPLORED_HEADER_LEN + set.len() * 8`. Both halves of the intent — exact length, round-trip equality — survive unchanged. |
| `from_bytes_drops_a_truncated_trailing_record` | A truncated trailing record is dropped and the records before it survive. | Build the blob through `to_bytes(GridKind::Square)` from a set containing the same one cell, append the same two garbage bytes, and assert the decode equals that set. Hand-assembling bare records would pin a format the encoder no longer writes. |

`hex_grid_marks_the_hex_axial_cell_containing_a_covered_center` touches no bytes and does not move.
**A third explored fixture failing on an assertion is a stop-and-report.**

- [ ] **Step 2: Write the failing tests**

Add to `mod tests` in `src/server/src/scene/explored.rs`:

```rust
    #[test]
    fn a_blob_written_under_one_grid_kind_does_not_decode_under_the_other() {
        // Discrimination: fails if the header is absent, ignored on read, or compared loosely —
        // the assertion is that the SAME bytes yield cells under one kind and none under the
        // other, which no format lacking the tag can satisfy.
        let mut set = ExploredSet::new();
        set.mark_polygons(
            &[vec![0.0, 0.0, 250.0, 0.0, 250.0, 250.0, 0.0, 250.0]],
            &sq(100.0),
            100.0,
        );
        let bytes = set.to_bytes(GridKind::Square);
        assert_eq!(ExploredSet::from_bytes(&bytes, GridKind::Square), set);
        assert!(
            ExploredSet::from_bytes(&bytes, GridKind::Hex).is_empty(),
            "square-indexed fog must not be reinterpreted as hex axial cells"
        );
    }

    #[test]
    fn a_headerless_blob_decodes_to_nothing() {
        // A blob with no header states no coordinate system for its records, so it is unusable
        // rather than assumed. Under-reveal is the safe direction for fog memory.
        // Discrimination: fails if `from_bytes` parses bare 8-byte records.
        let mut bare = (1_i32).to_le_bytes().to_vec();
        bare.extend_from_slice(&(2_i32).to_le_bytes());
        assert!(ExploredSet::from_bytes(&bare, GridKind::Square).is_empty());
    }

    #[test]
    fn a_hex_blob_round_trips_under_its_own_kind() {
        // Discrimination: fails if the header is written but the record payload is mis-offset,
        // which a square-only round-trip test would not catch.
        let g = HexGrid { size: 100.0 };
        let (cx, cy) = g.cell_center((1, 0));
        let poly = vec![
            cx - 10.0, cy - 10.0, cx + 10.0, cy - 10.0, cx + 10.0, cy + 10.0, cx - 10.0, cy + 10.0,
        ];
        let mut set = ExploredSet::new();
        set.mark_polygons(&[poly], &g, 100.0);
        let bytes = set.to_bytes(GridKind::Hex);
        assert_eq!(ExploredSet::from_bytes(&bytes, GridKind::Hex), set);
    }
```

Add to `mod tests` in `src/server/src/scene/grid_shape.rs`:

```rust
    #[test]
    fn each_shape_reports_its_own_kind() {
        // Discrimination: fails if either impl returns the other's kind, which would make the
        // explored blob's tag disagree with the geometry that wrote it.
        let sq = SquareGrid { cell: 100.0, rule: DiagonalRule::Chebyshev };
        let hx = HexGrid { size: 100.0 };
        assert_eq!(sq.kind(), crate::scene::GridKind::Square);
        assert_eq!(hx.kind(), crate::scene::GridKind::Hex);
    }
```

Add to `mod tests` in `src/server/src/scene/mod.rs`:

```rust
    #[test]
    fn the_resolved_shape_reports_the_resolved_kind() {
        // The three readers of the same decision — the shape a scene resolves to, the kind its
        // settings carry, and the ECS resolver — must not be able to disagree.
        // Discrimination: fails if `resolve_grid_shape_with_rule` stops constructing its shape
        // from `resolve_grid_kind`, or if `resolve_scene` stops reading the same pure helper,
        // which are the only ways the three can diverge. The unrecognised spelling pins the
        // fail-closed default in the same loop.
        for (engine, expect) in [
            (json!({ "grid": { "kind": "hex", "size": 50 }, "background": null }), GridKind::Hex),
            (json!({ "grid": { "kind": "square", "size": 50 }, "background": null }), GridKind::Square),
            (json!({ "grid": { "kind": "wobbly", "size": 50 }, "background": null }), GridKind::Square),
        ] {
            let ecs = SceneEcs::from_documents(vec![entity_doc_top_eng(10, "scene", engine)], 0);
            let scene = Uuid::from_u128(10);
            assert_eq!(ecs.resolve_grid_kind(scene), expect);
            assert_eq!(ecs.resolve_scene(scene).grid_kind, expect);
            assert_eq!(ecs.resolve_grid_shape(scene, 50.0).kind(), expect);
        }
    }

    #[test]
    fn changing_a_scenes_grid_kind_invalidates_the_cached_visibility_mask() {
        // The cache is value-comparison based, so any input the mask depends on must appear in
        // the snapshot. The grid kind decides every cell index in the mask.
        // Discrimination: fails if `ResolvedScene` carries no kind or the snapshot omits it,
        // because the snapshot then compares equal across the mutation and the stale mask is
        // returned. It cannot pass vacuously: the fixture asserts the two masks differ, so a
        // scene whose kind change produced no geometric difference fails the guard.
        // ... build a square scene with a lit token, call `visible_cells_cached`, mutate the
        // scene document's `/engine/grid/kind` to "hex" through `apply_op`, call again ...
        assert_ne!(before, after, "a grid-kind change must produce a different mask");
    }
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cd src/server && cargo test --lib scene::explored && cargo test --lib scene::grid_shape && cargo test --lib scene::tests::the_resolved_shape && cargo test --lib scene::tests::changing_a_scenes_grid_kind`

RUN, OBSERVE, RECORD.

- [ ] **Step 4: Declare `GridKind` and make one pure function the single source of the decision**

`GridKind` goes in `src/server/src/scene/mod.rs` beside `MovementModel` and `MovementRestriction`,
because `ResolvedScene`'s fields are public and `grid_shape` is `pub(crate)`:

```rust
/// A scene's cell geometry family, resolved from its `engine.grid.kind`. The single decision
/// behind which `GridShape` implementation a scene uses, which coordinate system its persisted
/// fog is indexed in, and which cached masks a change to it must invalidate. Anything other than
/// the hex spelling resolves to `Square` — the hardened default an absent or malformed scene
/// document falls back to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridKind {
    /// Axis-aligned square cells.
    Square,
    /// Pointy-top axial hexes.
    Hex,
}

/// The grid kind a decoded scene engine declares. Pure, so the two readers that each already hold
/// a decoded `SceneEngine` — `resolve_scene` and `SceneEcs::resolve_grid_kind` — read ONE
/// implementation of the comparison rather than repeating it, and neither pays a second decode to
/// reach it.
fn grid_kind_from(eng: Option<&eng::SceneEngine>) -> GridKind {
    if eng.map(|s| s.grid.kind.as_deref()) == Some(Some("hex")) {
        GridKind::Hex
    } else {
        GridKind::Square
    }
}
```

Read `eng::Grid`'s `kind` field before writing that comparison — if its type is not
`Option<String>`, follow the real one; the constraint is that exactly one expression performs the
comparison, not that it takes this exact form.

Add `grid_kind` to `ResolvedScene`:

```rust
    /// The scene's cell geometry family. Decides the `GridShape` implementation, the coordinate
    /// system of persisted explored fog, and — because it is part of `ResolvedScene` — the
    /// visibility cache's own invalidation key.
    pub grid_kind: GridKind,
```

`resolve_scene` already decodes the scene engine into `scene_eng` and binds `let s = scene_eng
.as_ref();`, so it sets the field from the value it holds — no second lookup and no second decode:

```rust
            grid_kind: grid_kind_from(s),
```

Add the ECS-level resolver for callers that hold only a scene id:

```rust
    /// The scene's `GridKind`, for a caller that holds no decoded scene engine. Performs exactly
    /// the lookup `resolve_grid_shape_with_rule` already performed inline, and defers the
    /// comparison to `grid_kind_from`, so the shape path pays nothing new and the decision has
    /// one implementation.
    ///
    /// Deliberately NOT routed through `resolve_scene`: that resolver reads the world-settings
    /// document and resolves the full settings set, a cost the shape path runs in per-scene and
    /// per-move loops (`scene_grid_shapes`, `pathfind`, `execute_move`) and does not need.
    pub(crate) fn resolve_grid_kind(&self, scene: Uuid) -> GridKind {
        let eng = self
            .index
            .get(&scene)
            .and_then(|&e| self.world.get::<&SceneEntity>(e).ok())
            .and_then(|c| self.engine_as_cached::<eng::SceneEngine>(scene, &c.doc));
        grid_kind_from(eng.as_ref())
    }
```

Then have `resolve_grid_shape_with_rule` read it, replacing its inline string comparison:

```rust
        match self.resolve_grid_kind(scene) {
            GridKind::Hex => Box::new(grid_shape::HexGrid { size: cell }),
            GridKind::Square => Box::new(grid_shape::SquareGrid { cell, rule }),
        }
```

Read the existing body of `resolve_grid_shape_with_rule` before writing `resolve_grid_kind` — its
lookup is the source of the one above, and if its real shape differs (borrow lifetimes, the field
type), follow the real one rather than this sketch. Update its doc comment, which describes the
`"hex"` string match it performed inline, to describe reading the resolved kind. Every
pre-existing `ResolvedScene` literal in tests gains the new field; if any test constructs one, the
compiler names it.

`VisibilityInputsSnapshot` needs **no new field** — it already stores `settings: ResolvedScene`,
which now carries the kind, and `ResolvedScene` derives `PartialEq`. State that explicitly in the
report rather than adding a redundant field, and let the new mask test prove it.

- [ ] **Step 5: Add `GridShape::kind`**

In `src/server/src/scene/grid_shape.rs`, add to the trait:

```rust
    /// This shape's geometry family. Lets any holder of a resolved shape reach the kind without a
    /// second per-scene map that could disagree with it: `resolve_grid_shape_with_rule` builds
    /// the shape FROM `SceneEcs::resolve_grid_kind`, so the two are the same decision by
    /// construction rather than by convention.
    fn kind(&self) -> GridKind;
```

with `fn kind(&self) -> GridKind { GridKind::Square }` on `SquareGrid` and
`fn kind(&self) -> GridKind { GridKind::Hex }` on `HexGrid`, and `use crate::scene::GridKind;` in
the module's imports.

- [ ] **Step 6: Give the explored blob a self-describing header**

In `src/server/src/scene/explored.rs`:

```rust
/// Magic prefix of a serialized `ExploredSet`. A blob without it states no coordinate system for
/// its records and is unusable rather than assumed.
const EXPLORED_MAGIC: [u8; 4] = *b"SCEF";

/// Serialization format version. A blob at any other version is not decoded.
const EXPLORED_VERSION: u8 = 1;

/// Header length: magic, version, grid-kind tag.
const EXPLORED_HEADER_LEN: usize = EXPLORED_MAGIC.len() + 2;

/// Grid-kind tag byte for `kind`.
fn kind_tag(kind: GridKind) -> u8 {
    match kind {
        GridKind::Square => 0,
        GridKind::Hex => 1,
    }
}
```

```rust
    /// Serialize as `SCEF`, a version byte, a grid-kind tag, then 8 bytes per cell (i32 i, i32 j,
    /// little-endian) in ascending order. `kind` is the grid family the cell indices are
    /// expressed in; `from_bytes` refuses a blob whose tag disagrees with the scene's current
    /// kind, because a square index and a hex axial index are different coordinate systems that
    /// share a representation.
    pub fn to_bytes(&self, kind: GridKind) -> Vec<u8> {
        let mut out = Vec::with_capacity(EXPLORED_HEADER_LEN + self.cells.len() * 8);
        out.extend_from_slice(&EXPLORED_MAGIC);
        out.push(EXPLORED_VERSION);
        out.push(kind_tag(kind));
        for &(i, j) in &self.cells {
            out.extend_from_slice(&i.to_le_bytes());
            out.extend_from_slice(&j.to_le_bytes());
        }
        out
    }

    /// Deserialize the `to_bytes` layout, refusing anything that is not this format at this
    /// version indexed in `kind`. Every refusal yields an EMPTY set: explored memory is
    /// best-effort and an empty set under-reveals, which is the safe direction for a fog gate. A
    /// trailing partial record is likewise dropped rather than erroring.
    pub fn from_bytes(b: &[u8], kind: GridKind) -> Self {
        if b.len() < EXPLORED_HEADER_LEN
            || b[..EXPLORED_MAGIC.len()] != EXPLORED_MAGIC
            || b[EXPLORED_MAGIC.len()] != EXPLORED_VERSION
            || b[EXPLORED_MAGIC.len() + 1] != kind_tag(kind)
        {
            return Self::default();
        }
        let mut cells = BTreeSet::new();
        for rec in b[EXPLORED_HEADER_LEN..].chunks_exact(8) {
            let i = i32::from_le_bytes([rec[0], rec[1], rec[2], rec[3]]);
            let j = i32::from_le_bytes([rec[4], rec[5], rec[6], rec[7]]);
            cells.insert((i, j));
        }
        Self { cells }
    }
```

Add `use crate::scene::GridKind;` to the module's imports.

- [ ] **Step 7: Convert every call site, enumerated**

Run and record the full output:

```bash
cd /c/Dev/Shadowcat && git grep -n "ExploredSet::from_bytes\|to_bytes()" -- src/server
```

The production set is **four decodes and one encode**, and each row below is a claim to verify
against that output before acting on it — including which module each handler lives in, because a
handler attributed to the wrong module sends the edit to the wrong read guard:

| Site | Where the kind comes from |
|---|---|
| `ws::conn::handle_pathfind`'s explored fetch — the `Pathfind` handler lives in `ws::conn`, not `ws::room` | It already takes a short read guard to read `resolve_scene(scene).movement_restriction` and drops it before the fetch. Capture the kind inside that SAME guard, from the `ResolvedScene` it is already resolving, and use the captured value at the decode. |
| `ws::conn::enrich_vision_explored`'s decode and its `set_explored` encode | It already resolves a per-scene `GridShape` (`grid_shapes`) and uses it for `mark_polygons`; read `shape.kind()`. Do NOT add a third per-scene map. |
| `ws::room::Room::publish`'s Create-placement `Revealed` branch | Easy to miss: the placement gate defers its explored fetch through `revealed_pending` and decodes AFTER the scene read guard is dropped. Capture the kind under that guard — the branch already resolves the scene for its restriction — and carry it in the deferred tuple alongside the mask. Resolving it at the fetch would re-acquire a lock across an await. |
| `ws::room::Room::execute_move`'s `Revealed` explored fetch | Capture under the read guard that already resolves `cell` and the settings, before it is dropped, for the same reason. |

Every test-only call site takes the kind its fixture is built with. Report the per-site
disposition; a count is not a disposition. **If the grep returns a production site not in this
table, or a row here does not resolve to a real symbol, that is the finding — report it before
editing.**

- [ ] **Step 8: Run the tests and the full suite**

Run: `cd src/server && cargo test`

RUN, OBSERVE, RECORD, including doctests — `ExploredSet::new`'s doctest is adjacent to the changed
methods, and any doctest calling `to_bytes`/`from_bytes` must be updated in the same step.

- [ ] **Step 9: Mutation check — prove all three guards are load-bearing**

Three mutations, run and reverted independently:

1. Make `from_bytes` ignore the kind tag (compare only magic and version).
2. Make `resolve_scene` hardcode `grid_kind: GridKind::Square`.
3. Make `HexGrid::kind` return `GridKind::Square`.

For each: run `cd src/server && cargo test --lib scene`, record the observed failing test names
and messages, revert, re-run, and confirm green plus a byte-identical diff against the
pre-mutation file. A mutation that leaves the suite green means that guard is unproven — report it
and stop rather than proceeding.

- [ ] **Step 10: Run the full server gate**

Run: `cd src/server && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

- [ ] **Step 11: Commit**

```bash
git add src/server/src/scene/mod.rs src/server/src/scene/grid_shape.rs src/server/src/scene/explored.rs src/server/src/ws/conn.rs src/server/src/ws/room.rs
git commit -m "fix(scene): make the grid kind part of the identities that depend on it

A stored fog blob carried bare cell indices with no statement of which
coordinate system they were in, so switching a live scene between square and
hex reinterpreted square indices as hex axial ones. The blob is now
self-describing and refuses a kind it was not written under; a blob predating
the header decodes empty, which under-reveals and re-accumulates from play.
The same omission ran through the visibility cache, whose value-comparison key
held the cell size but not the kind; the resolved kind now lives on the scene
settings the key already stores, one pure function answers the question for
both readers, and a resolved shape reports its own kind so no second per-scene
map can disagree with it.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>" -- src/server/src/scene/mod.rs src/server/src/scene/grid_shape.rs src/server/src/scene/explored.rs src/server/src/ws/conn.rs src/server/src/ws/room.rs
```

---

### Task 3: NEW-8 — an over-cap candidate scan degrades to a bounded window, never to nothing

**Ledger id:** NEW-8.

**The failure this removes.** Three sites hand `GridShape::cells_in_bounds` a pixel AABB and the
cap `explored::MAX_CELLS_PER_POLYGON`, and treat its `None` as "skip": `SceneEcs::player_lit_mask`
(`tracing::warn!("lit mask cell scan exceeds cap; skipping source")`), `accumulate_visible_cells`
(`"visible_cells scan exceeds cap; skipping source"`) and `ExploredSet::mark_polygons`
(`"explored cell scan over-cap or degenerate; skipping polygon"`). Skipping a source produces an
EMPTY mask for it. An empty mask is fail-closed in the sense that nothing is revealed, but under
`MovementRestriction::Visible` it also refuses every move and ships no lit cells — a total denial
rather than a degradation. Task 5 multiplies each of these scans' cell counts by the square of the
cell size, so the cliff moves from "absurd bounds" to "large bounds".

**The decision, and why it is this one.** Instead of dropping the source entirely, an over-cap scan
is clamped to a window centred on the source's own focus, so the source still contributes the
neighbourhood it can actually be asked about. The clamped set is a SUBSET of the unclamped one, so
every consumer's fail direction is unchanged (mask: fewer cells ⇒ fewer moves admitted and fewer
cells shipped; explored: fewer cells marked). Raising the cap was rejected: it moves the cliff
without removing it and raises the worst-case memory and time the cap exists to bound.

**The window is applied CONDITIONALLY, and that is load-bearing rather than an optimisation.** The
cap bounds a PRODUCT (`w × h` candidate cells); the window bounds a PER-AXIS DISTANCE from a focus
that is the source, not the box's centre. Those are different quantities, so sizing the window from
the cap does NOT make it inert below the cap. Worked case: a 1500 × 1500-cell scene at cell 100
enumerates about 2.26M candidates, comfortably under the 4M cap and scanned whole today; with its
source near the origin, a window of ±999 cells about that source would drop every cell past 999 on
each axis — out of the mask, and under `MovementRestriction::Visible` out of reach. That is a
silent reduction of player-visible area on a legitimately-authored scene.

`clamp_scan_window` therefore computes the candidate span FIRST, through the same
`GridShape::cell_bounds` + `saturating_mul` arithmetic `cells_in_bounds` itself uses, and returns
the box untouched whenever that span is within the cap. "Inert below the cap" is then a property of
the code — one comparison, testable — rather than an assumption about box shape. Step 3's first
test is exactly that property, at a box that is wider than the window and still under the cap.

**Files:**
- Modify: `src/server/src/scene/explored.rs` (`clamp_scan_window`, `mark_polygons`)
- Modify: `src/server/src/scene/mod.rs` (`player_lit_mask`'s scan, `accumulate_visible_cells`'s scan)
- Test: `src/server/src/scene/explored.rs` (`mod tests`), `src/server/src/scene/mod.rs` (`mod tests`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `explored::SCAN_WINDOW_HALF_CELLS: i64`
  - `explored::clamp_scan_window(grid: &dyn GridShape, focus: vision::P, min: vision::P, max: vision::P, cell: f64, max_cells: i64) -> (vision::P, vision::P)` (`pub(crate)`)

  No later task consumes either; the windowing is independent of the conversions.

- [ ] **Step 1: Measure the post-Task-5 scan size against the cap, and record the numbers**

This step produces measurements, not a decision — the decision is already made above. Record all
of it in the task report.

1. Derive, from `accumulate_visible_cells` and `SquareGrid::cell_bounds`, the number of candidate
   cells the scan enumerates for a square scene as a function of the authored bounds `(w, h)` in
   grid units, the cell size, and `VISION_BOUND_MARGIN`. Do the derivation for the CURRENT code
   (where `bound_for_scene` receives the raw grid-unit bounds) and for the post-Task-5 code (where
   it receives `w*cell, h*cell`), and state the ratio between them.
2. From that, state the authored `(w, h)` at which the post-Task-5 scan first exceeds
   `MAX_CELLS_PER_POLYGON`.
3. Write a `#[test]`-gated timing probe — NOT committed — that builds a wall-less all-bright
   square scene at authored bounds `(1000.0, 1000.0)` and cell `100.0`, calls `visible_cells` once,
   and prints the elapsed time and the mask length. Run it, record both numbers, then delete the
   probe. This is the cost of the corrected scan at a large-but-legal scene, and it is a number
   the report must carry rather than an assumption.

If the measured elapsed time at step 3 exceeds one second in a debug build, **report it as a
finding** alongside the number. Do not respond by changing the window size: the window's size is
fixed by the cap for the reason given above, and trading a cost figure for a smaller player-visible
radius is a scope decision that is not this task's to make.

- [ ] **Step 2: Enumerate the pre-existing over-cap fixture and re-derive it**

One pre-existing test asserts the total-loss outcome directly and therefore fails on an ASSERTION:

```bash
cd /c/Dev/Shadowcat && git grep -n "MAX_CELLS_PER_POLYGON\|exceeds_the_cell_cap" -- src/server
```

Record every hit with its enclosing function and a disposition. `grid_shape.rs`'s over-cap tests
call `cells_in_bounds` directly and do not move — the cap itself is unchanged. The one that moves:

| Fixture | Protected intent | Re-derivation |
|---|---|---|
| `explored::tests::skips_a_polygon_whose_bbox_exceeds_the_cell_cap` | An over-cap polygon does BOUNDED work: the enumeration never runs at the size the bbox implies. | Rename to `bounds_a_polygon_whose_bbox_exceeds_the_cell_cap_to_the_scan_window` and re-author the polygon as a long thin strip — `[(0,0), (9_000_000,0), (9_000_000,3), (0,3)]` at `cell_size` 1, a bbox of about 9M × 4 cells against the 4M cap — so the clamped scan is a few thousand cells rather than the window's full square. Assert: the mark count is positive, the cell at the bbox centre's own column is marked, and a cell far outside the window is not. Bounded work is preserved and is now asserted from both sides; "marks nothing" was the outcome, never the intent. |

**A second explored fixture failing on an assertion is a stop-and-report.**

- [ ] **Step 3: Write the failing tests**

Add to `mod tests` in `src/server/src/scene/explored.rs`:

```rust
    #[test]
    fn a_scan_wider_than_the_window_but_under_the_cap_is_returned_unchanged() {
        // The property the conditional application exists for: the cap bounds a PRODUCT while the
        // window bounds a PER-AXIS distance, so a box can reach far past the window on both axes
        // and still enumerate fewer cells than the cap allows. Such a box must not lose a single
        // candidate — its cells are in the mask today and a player can move to them.
        //
        // Discrimination: fails if the window is applied whenever the box is wider than it,
        // because the returned max would then be the window edge rather than the box edge. The
        // guard below keeps the test honest if `SCAN_WINDOW_HALF_CELLS` ever changes.
        let cell = 100.0;
        let g = sq(cell);
        let focus = (50.0, 50.0);
        let min = (-50.0, -50.0);
        let max = (150_000.0, 150_000.0); // 1502 × 1502 = 2_256_004 candidates, under the cap
        assert!(
            max.0 - focus.0 > SCAN_WINDOW_HALF_CELLS as f64 * cell,
            "fixture: the box must reach past the window, or the test proves nothing"
        );
        let (out_min, out_max) =
            clamp_scan_window(&g, focus, min, max, cell, MAX_CELLS_PER_POLYGON);
        assert_eq!((out_min, out_max), (min, max));
    }

    #[test]
    fn clamp_scan_window_bounds_a_scan_that_exceeds_the_cap() {
        // Discrimination: fails if the window is not centred on `focus`, if its half-extent is not
        // `SCAN_WINDOW_HALF_CELLS` cells, or if it expands rather than intersects — the low edges
        // already sit inside the window and must come back unchanged, while the high edges must
        // come back at the window.
        let cell = 100.0;
        let g = sq(cell);
        let focus = (50.0, 50.0);
        let half_px = SCAN_WINDOW_HALF_CELLS as f64 * cell;
        let (min, max) = ((-50.0, -50.0), (1.0e9, 1.0e9));
        let (out_min, out_max) =
            clamp_scan_window(&g, focus, min, max, cell, MAX_CELLS_PER_POLYGON);
        assert_eq!(out_min, min, "an edge already inside the window is untouched");
        assert_eq!(out_max, (focus.0 + half_px, focus.1 + half_px));
    }

    #[test]
    fn a_window_that_misses_the_scan_box_leaves_it_unchanged() {
        // The precondition `clamp_scan_window` states: `focus` lies inside the box. A focus far
        // outside it would otherwise produce min > max — an inverted rectangle that enumerates
        // nothing, which is the total loss this clamp exists to remove, reintroduced as a silent
        // empty result.
        // Discrimination: fails if the intersection is returned without the emptiness check.
        let cell = 100.0;
        let g = sq(cell);
        let (min, max) = ((0.0, 0.0), (1.0e9, 1.0e9));
        assert_eq!(
            clamp_scan_window(&g, (-1.0e8, -1.0e8), min, max, cell, MAX_CELLS_PER_POLYGON),
            (min, max)
        );
    }

    #[test]
    fn a_clamped_square_window_stays_inside_the_per_polygon_cap() {
        // The window exists so that `cells_in_bounds` cannot refuse it.
        // Discrimination: fails if `SCAN_WINDOW_HALF_CELLS` is raised such that
        // `(2*half + 1)^2 > MAX_CELLS_PER_POLYGON`.
        let side = 2 * SCAN_WINDOW_HALF_CELLS + 1;
        assert!(
            side * side <= MAX_CELLS_PER_POLYGON,
            "the window enumerates {} cells against a {MAX_CELLS_PER_POLYGON} cap",
            side * side
        );
    }

    #[test]
    fn a_clamped_hex_window_also_stays_inside_the_per_polygon_cap() {
        // Square is the denser of the two shapes per unit of pixel area only if hex's axial
        // preimage of the same pixel box enumerates fewer cells. That is a claim about
        // `HexGrid::cell_bounds`, so it is measured through that function rather than argued in
        // prose. Discrimination: fails if the axial padding or the preimage arithmetic changes
        // such that a clamped hex window can be refused by the cap.
        let size = 100.0;
        let g = HexGrid { size };
        let half_px = SCAN_WINDOW_HALF_CELLS as f64 * size;
        let (q0, r0, q1, r1) = g.cell_bounds((-half_px, -half_px), (half_px, half_px), size);
        let span = (q1 as i64 - q0 as i64 + 1) * (r1 as i64 - r0 as i64 + 1);
        assert!(
            span <= MAX_CELLS_PER_POLYGON,
            "a clamped hex window enumerates {span} cells against a {MAX_CELLS_PER_POLYGON} cap"
        );
    }

```

**`mark_polygons`' own over-cap coverage is Step 2's re-derived fixture, and is not duplicated
here.** An earlier shape of this step added
`an_oversized_polygon_marks_a_bounded_neighbourhood_rather_than_nothing` with the same
`[(0,0), (9_000_000,0), (9_000_000,3), (0,3)]` strip at `cell_size` 1 and the same three
assertions as
`bounds_a_polygon_whose_bbox_exceeds_the_cell_cap_to_the_scan_window`. Two tests with one fixture
and one assertion set are one test twice. **The retained one is Step 2's re-derived fixture** —
between a re-derived pre-existing pin and a new test that would pin the same thing, the pin that
also discharges "re-derive, never re-baseline" is the one to keep, and dropping it in favour of a
new name would re-baseline it under cover of adding coverage. Step 7's wiring mutation names it as
the `mark_polygons` call site's detector.

Add to `mod tests` in `src/server/src/scene/mod.rs`. The fixture's doc states a REQUIREMENT rather
than a measured fact, deliberately: the next step derives whether the scene meets it and halts on a
disagreement, and a doc asserting the answer would be the plan telling the implementer what the
derivation is supposed to produce. It also carries no reference to the derivation itself, because a
committed comment may not name a step, a task or any other thing outside the code.

```rust
    /// REQUIREMENT this scene has to satisfy, which is what every test reading it depends on: a
    /// single source's candidate scan must exceed `MAX_CELLS_PER_POLYGON` under BOTH readings of
    /// the authored bounds — the raw one, where `vision::bound_for_scene` compares the authored
    /// value against world coordinates, and the converted one, where it receives that value times
    /// the cell size. A scan under the cap never engages the clamp, and the assertions would then
    /// hold for a reason they do not name. The width is what supplies the over-cap product; the
    /// height is small so the CLAMPED scan is a few thousand cells and the tests run in a unit
    /// suite. Wall-less, all-bright, LOS off, one owned token at the origin cell, so the whole
    /// scan is a single source's.
    fn over_cap_scan_scene() -> (SceneEcs, Uuid, Uuid) {
        let user = Uuid::from_u128(7);
        let scene_id = Uuid::from_u128(10);
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        let scene = entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                    "bounds": { "width": 200_000_000.0, "height": 5.0 } }),
        );
        let mut ecs = SceneEcs::from_documents(vec![scene, tok], 0);
        ecs.set_world_settings_for_test(json!({
            "scene": {
                "losRestriction": false, "fog": true,
                "lightingEnabled": false, "lightMode": "environmentLight",
                "environment": { "color": "#ffffff", "intensity": 1.0 },
                "observerVision": false,
                "movementRestriction": "visible",
                "movementModel": "grid-stepped",
                "partialCellLeniency": false
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        }));
        (ecs, user, scene_id)
    }

    #[test]
    fn an_over_cap_visibility_scan_yields_a_bounded_mask_not_an_empty_one() {
        // Under `MovementRestriction::Visible` an empty mask refuses every move, so the
        // over-cap outcome must be a bounded neighbourhood of the source rather than nothing.
        // Discrimination: fails if `accumulate_visible_cells` hands the unclamped bbox to
        // `cells_in_bounds`, because the cap then returns `None` and the source is skipped,
        // leaving the mask empty. It cannot pass vacuously: the second assertion requires the
        // mask to STOP somewhere, so a scan that ignored the cap entirely also fails.
        let (ecs, user, scene) = over_cap_scan_scene();
        let mask = ecs.visible_cells(user, scene, false);
        assert!(mask.contains(&(0, 0)), "the source's own cell is visible");
        let outside = crate::scene::explored::SCAN_WINDOW_HALF_CELLS as i32 + 10;
        assert!(
            !mask.contains(&(outside, 0)),
            "a cell beyond the scan window is not in the mask"
        );
    }

    #[test]
    fn an_over_cap_lit_mask_scan_yields_a_bounded_cell_set_not_an_empty_one() {
        // The egress half of the same scan, which is a separate call site and would otherwise be
        // converted independently. Discrimination: identical to the mask test, applied to
        // `player_lit_mask`'s own scan.
        let (ecs, user, scene) = over_cap_scan_scene();
        let cells: std::collections::BTreeSet<(i32, i32)> = ecs
            .player_lit_mask(user)
            .into_iter()
            .filter(|s| s.scene == scene)
            .flat_map(|s| s.cells.into_iter().map(|(i, j, _b, _t, _h)| (i, j)))
            .collect();
        assert!(cells.contains(&(0, 0)), "the source's own cell is lit");
        let outside = crate::scene::explored::SCAN_WINDOW_HALF_CELLS as i32 + 10;
        assert!(
            !cells.contains(&(outside, 0)),
            "a cell beyond the scan window is not shipped"
        );
    }
```

- [ ] **Step 4: Run the tests to verify they fail, and record the over-cap arithmetic you observe**

Run: `cd src/server && cargo test --lib scene::explored && cargo test --lib scene::tests::an_over_cap`

RUN, OBSERVE, RECORD. The compiler is the first signal here (`clamp_scan_window` and
`SCAN_WINDOW_HALF_CELLS` do not exist yet); record its actual output rather than asserting what
it will say.

Then, before writing the implementation, derive and record for `over_cap_scan_scene` **under the
current code**: the rectangle `vision::bound_for_scene` produces, the per-axis cell counts
`SquareGrid::cell_bounds` derives from it, and their product against `MAX_CELLS_PER_POLYGON`. The
fixture's whole value is that this product exceeds the cap at this task's position in the phase, not
only after Task 5 — if your derivation says otherwise, that is the finding: **stop and report**,
because the two tests above would then be pinning behaviour the fixture does not reach.

- [ ] **Step 5: Add the window**

In `src/server/src/scene/explored.rs`, beside `MAX_CELLS_PER_POLYGON`:

```rust
/// Half-extent, in CELLS, of the window an over-cap candidate scan is clamped to.
///
/// Sized so the window itself can never be refused by `MAX_CELLS_PER_POLYGON`: a square window of
/// `2*HALF + 1` cells per side enumerates `(2*HALF + 1)^2` cells, and `HALF` is the largest value
/// keeping that product at or under the cap. Hex enumerates FEWER cells for the same pixel window
/// — the axial preimage of a pixel box is a sheared parallelogram whose integer bounding box is
/// smaller than the square index rectangle of the same box — so bounding the square case bounds
/// both, and `a_clamped_hex_window_also_stays_inside_the_per_polygon_cap` measures that through
/// `HexGrid::cell_bounds` rather than assuming it.
pub(crate) const SCAN_WINDOW_HALF_CELLS: i64 = 999;

/// Intersect a candidate-scan AABB with a window of `SCAN_WINDOW_HALF_CELLS` cells around `focus`,
/// but ONLY when the AABB's own candidate count exceeds `max_cells`.
///
/// An over-cap scan makes `GridShape::cells_in_bounds` return `None`, and every caller of that
/// primitive treats `None` as "skip this source/polygon" — an empty mask, which on the movement
/// gate refuses every move and on egress ships no cells. Clamping keeps such a scan enumerable, at
/// a bounded SUBSET of the unclamped candidate set: each caller's fail direction stays the
/// under-revealing one (fewer cells admitted, fewer cells shipped, fewer cells remembered), and
/// the outcome is a degradation the source survives rather than the source's whole contribution.
///
/// The span test is what keeps this from taking cells away from a scan that was never in trouble.
/// The cap bounds a PRODUCT of two cell counts; the window bounds a PER-AXIS distance from a focus
/// that sits wherever the source does, not at the box's centre. A box can therefore reach far
/// beyond the window on both axes and still enumerate fewer cells than the cap allows, and those
/// cells are in the mask a player moves through. So the span is computed first — through the same
/// `cell_bounds` + `saturating_mul` arithmetic `cells_in_bounds` applies — and a span within
/// `max_cells` returns `min`/`max` untouched.
///
/// PRECONDITION: `focus` lies inside `[min, max]`. All three callers satisfy it — a visibility
/// source sits inside its own LOS polygon's bbox, and `mark_polygons` uses that bbox's own centre.
/// A focus far enough outside that the window misses the box would otherwise yield `min > max`, an
/// inverted rectangle that enumerates nothing, so that case returns the box unchanged and lets the
/// callee's own cap decide.
///
/// Returns `min`/`max` unchanged for a degenerate `cell`, `focus` or box as well — the callee's
/// fail-closed `None` on a degenerate input is the correct outcome there and must not be masked.
pub(crate) fn clamp_scan_window(
    grid: &dyn GridShape,
    focus: vision::P,
    min: vision::P,
    max: vision::P,
    cell: f64,
    max_cells: i64,
) -> (vision::P, vision::P) {
    if !cell.is_finite()
        || cell <= 0.0
        || !focus.0.is_finite()
        || !focus.1.is_finite()
        || !min.0.is_finite()
        || !min.1.is_finite()
        || !max.0.is_finite()
        || !max.1.is_finite()
    {
        return (min, max);
    }
    let (i0, j0, i1, j1) = grid.cell_bounds(min, max, cell);
    let w = i1 as i64 - i0 as i64 + 1;
    let h = j1 as i64 - j0 as i64 + 1;
    if w.saturating_mul(h) <= max_cells {
        return (min, max);
    }
    let half_px = SCAN_WINDOW_HALF_CELLS as f64 * cell;
    let win_min = (min.0.max(focus.0 - half_px), min.1.max(focus.1 - half_px));
    let win_max = (max.0.min(focus.0 + half_px), max.1.min(focus.1 + half_px));
    if win_min.0 > win_max.0 || win_min.1 > win_max.1 {
        return (min, max);
    }
    (win_min, win_max)
}
```

Add `use crate::scene::vision;` to the module's imports if it is not already there; `GridShape` is
already in scope for `mark_polygons`' own signature.

In `mark_polygons`, the focus is the polygon's own bbox centre — the polygon is a visibility
polygon, so its centre is the region the caller is asking about. Replace the `cells_in_bounds`
call with:

```rust
            // Clamp before enumerating: a bbox whose candidate count exceeds the cap is
            // intersected with a window around its own centre, so the polygon marks a bounded
            // SUBSET of the cells that bbox covers. Fail direction: fewer cells remembered, which
            // under-reveals. `cells_in_bounds` still applies the cap, so a degenerate input fails
            // closed.
            let focus = ((minx + maxx) * 0.5, (miny + maxy) * 0.5);
            let (scan_min, scan_max) = clamp_scan_window(
                grid,
                focus,
                (minx, miny),
                (maxx, maxy),
                cell_size,
                MAX_CELLS_PER_POLYGON,
            );
            let Some(candidates) =
                grid.cells_in_bounds(scan_min, scan_max, cell_size, MAX_CELLS_PER_POLYGON)
            else {
                tracing::warn!("explored cell scan degenerate; skipping polygon");
                continue;
            };
```

and update `mark_polygons`' doc sentence about the over-cap case to state the present behaviour:

```rust
    /// A polygon whose bbox enumerates more than `MAX_CELLS_PER_POLYGON` candidate cells is
    /// clamped to a `SCAN_WINDOW_HALF_CELLS` window around that bbox's centre, marking a bounded
    /// subset; a bbox within the cap is enumerated whole. A DEGENERATE polygon
    /// (`cells_in_bounds` → `None`) is skipped (under-reveal) to bound the dispatch-path cost.
```

In `src/server/src/scene/mod.rs`, apply the same clamp at both vision scans, with `src.vp` as the
focus — the source's viewpoint is the centre of what that source can be asked about. In
`player_lit_mask`:

```rust
                // Clamp before enumerating: a scan whose candidate count exceeds the cap is
                // intersected with a window around this source's viewpoint, so the source
                // contributes a bounded SUBSET of its candidate cells. Fail direction: fewer cells
                // shipped, which under-reveals. A scan within the cap is enumerated whole.
                let (scan_min, scan_max) = crate::scene::explored::clamp_scan_window(
                    cell_grid.as_ref(),
                    src.vp,
                    (minx, miny),
                    (maxx, maxy),
                    cell,
                    crate::scene::explored::MAX_CELLS_PER_POLYGON,
                );
                let candidates = match cell_grid.cells_in_bounds(
                    scan_min,
                    scan_max,
                    cell,
                    crate::scene::explored::MAX_CELLS_PER_POLYGON,
                ) {
                    Some(c) => c,
                    None => {
                        tracing::warn!("lit mask cell scan degenerate; skipping source");
                        continue;
                    }
                };
```

and in `accumulate_visible_cells`, where the lenient pad is applied BEFORE the clamp so the
lenient candidate set stays a superset of the strict one under clamping as well:

```rust
        let pad_px = if lenient { cell } else { 0.0 };
        let min = (minx - pad_px, miny - pad_px);
        let max = (maxx + pad_px, maxy + pad_px);
        // Pad first, then clamp: clamping the padded box keeps the lenient candidate set a
        // superset of the strict one (equal when the clamp binds), which the strict/lenient
        // relationship depends on.
        let (min, max) = crate::scene::explored::clamp_scan_window(
            grid,
            src.vp,
            min,
            max,
            cell,
            crate::scene::explored::MAX_CELLS_PER_POLYGON,
        );
        let candidates = match grid.cells_in_bounds(
            min,
            max,
            cell,
            crate::scene::explored::MAX_CELLS_PER_POLYGON,
        ) {
            Some(c) => c,
            None => {
                tracing::warn!("visible_cells scan degenerate; skipping source");
                continue;
            }
        };
```

Read the real receiver expression at each site before editing — `cell_grid` and `grid` differ in
whether they are a `Box<dyn GridShape>` or a `&dyn GridShape`, and the call needs whichever
produces `&dyn GridShape`. Update the comment above each of the two vision scans that currently
explains the over-cap `None` as a skip, so it states the clamp-then-cap behaviour instead. Treat
every comment on a line you touch as stale until verified against the new code.

- [ ] **Step 6: Run the tests and the whole scene suite**

Run: `cd src/server && cargo test --lib scene`

RUN, OBSERVE, RECORD. Report the outcome of each new test and, separately, of every pre-existing
test in `scene::explored` and in `scene::tests` that exercises a mask or explored accumulation. The
clamp does not engage below the cap, so no fixture under the cap should move; **a pre-existing
fixture other than the one re-derived in Step 2 that changes outcome contradicts that and is a
stop-and-report**, not a fixup.

- [ ] **Step 7: Mutation check — prove the clamp is wired at all three sites, and that the span test is real**

Two mutation families, each run and reverted independently, each confirmed by a byte-identical
diff against the pre-mutation file after the revert.

1. **Wiring.** Make `clamp_scan_window` return `(min, max)` unconditionally, run
   `cd src/server && cargo test --lib scene::explored && cargo test --lib scene::tests::an_over_cap`,
   and record the observed failing test names and messages. The mutation must be observed to fail
   at least one test per call site: `bounds_a_polygon_whose_bbox_exceeds_the_cell_cap_to_the_scan_window`
   for `mark_polygons`, `an_over_cap_visibility_scan_yields_a_bounded_mask_not_an_empty_one` for
   `accumulate_visible_cells`, and
   `an_over_cap_lit_mask_scan_yields_a_bounded_cell_set_not_an_empty_one` for `player_lit_mask`.
   If any one of the three stays green, that site is unwired or uncovered and is the finding to
   report; stop rather than proceeding.
2. **Conditionality.** Delete the `if w.saturating_mul(h) <= max_cells { return (min, max); }`
   early return so the window applies always, run `cd src/server && cargo test --lib scene`, and
   record the observed failures. If the suite stays green, the under-cap property is unproven and
   that is the finding — the whole reason this clamp is conditional is that an unconditional one
   silently shrinks a legitimate mask, and a green suite would mean nothing detects that.

- [ ] **Step 8: Run the full server gate**

Run: `cd src/server && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

- [ ] **Step 9: Commit**

```bash
git add src/server/src/scene/explored.rs src/server/src/scene/mod.rs
git commit -m "fix(scene): clamp an over-cap candidate scan instead of dropping its source

The three per-source candidate scans treated the enumeration cap's refusal as
'skip this source', which yields an empty mask — under visible-restriction that
refuses every move and ships no lit cells, a total loss rather than a
degradation. A scan whose candidate count exceeds the cap is now intersected
with a window around its own focus; a scan within the cap is enumerated whole,
tested through the span rather than assumed from the window's size, because the
cap bounds a product of cell counts while the window bounds a per-axis distance.
The clamped set is a subset of the unclamped one, so each consumer's fail
direction is unchanged.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>" -- src/server/src/scene/explored.rs src/server/src/scene/mod.rs
```

---

### Task 4: NEW-9 — the hex footprint predicate stops depending on where its anchor sits

**Ledger id:** NEW-9. **Buddy-checked (PHASE = code).**

**The defect, and why it is live now.** `HexGrid::footprint_cells` includes a hex when
`dist(cell_center, ctr) <= r_scene + inradius`, and its doc calls that "an always-safe
over-approximation (a hex overlapping the true disc boundary is never excluded)". That claim holds
only while `ctr` is the anchor hex's own centre. For an off-centre `ctr`, a hex the disc reaches
near one of its VERTICES has its centre up to `r_scene + size` away, and `size` (the circumradius)
exceeds `inradius = √3/2 · size` — so the predicate drops it. Two production callers already pass
an off-centre `ctr`: `navmesh::clip_to_visible_mask` (`grid.footprint_cells(to_cell, s.pos, …)`)
and `navmesh::los_smooth`'s `chord_ok` (`grid.footprint_cells(to, s.pos, …)`), both anchored at an
arc-length sample point. On a hex scene that under-includes the cells a route's footprint must
have visible, which delays the fog clip's truncation.

**The direction, stated precisely because the two are easy to swap.** Over-inclusion TIGHTENS these
gates: more cells must be visible, so a route truncates earlier and a step is refused sooner.
Under-inclusion LOOSENS them: cells the token's body genuinely covers are never required to be
visible, so a route runs further into fog than the mask allows. This defect is the under-inclusion
direction, on a secrecy post-filter.

This task runs **before** Task 11, which introduces a third off-centre caller on the movement gate
itself.

**Files:**
- Modify: `src/server/src/scene/grid_shape.rs` (`HexGrid::footprint_cells`, a new private
  `HexGrid::distance_to_cell_polygon`)
- Test: `src/server/src/scene/grid_shape.rs` (`mod tests`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: no signature changes. `GridShape::footprint_cells` keeps
  `fn footprint_cells(&self, anchor: Cell, ctr: vision::P, r_scene: f64, cell: f64) -> Vec<Cell>`.
  `SquareGrid`'s implementation is untouched: its AABB-to-disc distance test is already exact and
  already independent of `anchor`, which is used there only as the empty-result fallback.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/server/src/scene/grid_shape.rs`:

```rust
    #[test]
    fn hex_footprint_reaches_a_cell_the_disc_touches_near_a_shared_vertex() {
        // A pointy-top hex of size 50 centred at the origin has a vertex at
        // (√3/2·50, -25) = (43.301, -25), shared with hexes (1,0) and (1,-1) — both of whose
        // centres sit exactly 50 (the circumradius) from that vertex. A disc of radius 2 centred
        // one unit inside the origin hex from that vertex overlaps all three by a clear margin,
        // so no assertion here sits on the overlap boundary.
        //
        // Discrimination: a predicate comparing the CELL CENTRE distance against
        // `r_scene + inradius` computes about 50.5 against a bound of 45.3 for both neighbours and
        // emits only the anchor. The assertion is the presence of the two neighbours, which no
        // centre-distance-against-inradius test can produce at this radius.
        let g = HexGrid { size: 50.0 };
        let half_x = 50.0 * 3.0_f64.sqrt() / 2.0;
        let vertex = (half_x, -25.0);
        // One unit from the vertex along the direction back to the origin hex's centre.
        let p = (vertex.0 - 0.866, vertex.1 + 0.5);
        assert_eq!(g.cell_of(p), (0, 0), "fixture: the sample sits in the origin hex");
        let cells = g.footprint_cells((0, 0), p, 2.0, 50.0);
        assert!(cells.contains(&(0, 0)), "the anchor hex, got {cells:?}");
        assert!(cells.contains(&(1, 0)), "the hex across the shared edge, got {cells:?}");
        assert!(cells.contains(&(1, -1)), "the third hex at that vertex, got {cells:?}");
    }

    #[test]
    fn hex_footprint_from_a_cell_centre_brackets_the_inradius_threshold() {
        // From a hex's own centre the nearest point of every neighbour is their shared edge, at
        // the inradius √3/2·size ≈ 43.301. A disc a clear 0.5 under that stays in one hex; a disc
        // a clear 0.5 over it reaches all six neighbours and nothing beyond (ring 2's nearest edge
        // is 2.598·size away). Both probes sit off the threshold, so a one-ULP difference in the
        // distance computation cannot decide either.
        //
        // Discrimination: fails if the threshold for a CENTRE-anchored disc moves in either
        // direction by more than half a unit — the case every existing hex parity fixture
        // exercises, and the property that makes this change inert for them.
        let g = HexGrid { size: 50.0 };
        let ctr = g.cell_center((0, 0));
        let inradius = 50.0 * 3.0_f64.sqrt() / 2.0;
        assert_eq!(g.footprint_cells((0, 0), ctr, inradius - 0.5, 50.0), vec![(0, 0)]);
        let over = g.footprint_cells((0, 0), ctr, inradius + 0.5, 50.0);
        assert_eq!(over.len(), 7, "the disc reaches all six neighbours, got {over:?}");
        assert!(over.contains(&(0, 0)) && over.contains(&(1, 0)) && over.contains(&(0, 1)));
    }

    #[test]
    fn hex_footprint_returns_the_anchor_when_the_disc_overlaps_nothing_else() {
        // The zero-radius guarantee the square implementation also makes: a point footprint
        // yields exactly the anchor. Discrimination: fails if the empty-result fallback is
        // dropped, or if the predicate admits a cell the disc does not reach.
        let g = HexGrid { size: 50.0 };
        let ctr = g.cell_center((2, -1));
        assert_eq!(g.footprint_cells((2, -1), ctr, 0.0, 50.0), vec![(2, -1)]);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src/server && cargo test --lib scene::grid_shape`

RUN, OBSERVE, RECORD.

- [ ] **Step 3: Replace the predicate with an exact hex-polygon distance**

In `src/server/src/scene/grid_shape.rs`, add a private helper to `impl HexGrid` (the inherent impl,
beside its other private helpers — not the trait impl):

```rust
    /// Distance from `p` to cell `c`'s hexagon: zero when `p` lies inside it, else the smallest
    /// distance to any of its six edges. Reads the SAME vertex ring `cell_vertices` supplies to
    /// the leniency corner test, so the footprint predicate and the corner sampler cannot
    /// disagree about a hex's geometry.
    fn distance_to_cell_polygon(&self, c: Cell, p: vision::P, cell: f64) -> f64 {
        let verts = self.cell_vertices(c, cell);
        if vision::point_in_poly(&verts, p) {
            return 0.0;
        }
        let mut best = f64::INFINITY;
        for k in 0..verts.len() {
            let a = verts[k];
            let b = verts[(k + 1) % verts.len()];
            best = best.min(vision::point_segment_distance(p, a, b));
        }
        best
    }
```

`vision::point_in_poly` and `vision::point_segment_distance` are both `pub(crate)`; add whatever
import the module needs beside its existing `vision` use.

Replace `impl GridShape for HexGrid`'s `footprint_cells` body and doc:

```rust
    /// Cells whose hex geometry the footprint disc (centre `ctr`, radius `r_scene`) overlaps.
    ///
    /// EXACT and INDEPENDENT of where `ctr` sits relative to `anchor`: a hex is included iff the
    /// distance from `ctr` to that hex's own polygon is at most `r_scene`. A centre-distance test
    /// against the inradius is not a safe substitute — a hex the disc reaches near one of its
    /// VERTICES has its centre up to `r_scene + size` away, and `size > √3/2·size` — and the
    /// callers that pass an arc-length sample point rather than a cell centre
    /// (`navmesh::clip_to_visible_mask`, `navmesh::los_smooth`) are exactly the ones that would
    /// lose those cells. Losing them LOOSENS the gates that read this set: a cell the token's body
    /// covers is then never required to be visible.
    ///
    /// Two cheap bounds settle most candidates without the polygon walk, both exact rather than
    /// approximate: a hex whose centre is within `r_scene + √3/2·size` necessarily overlaps
    /// (every hex contains its own inscribed disc of that radius), and a hex whose centre is
    /// beyond `r_scene + size` necessarily does not (every hex lies inside its circumscribed disc
    /// of that radius). Only the annulus between them needs the six edge distances.
    ///
    /// The scan is a hex-shaped ring neighbourhood of `anchor`, sized so it cannot miss a
    /// reachable hex: ring `k`'s centres are at least `1.5·size` per ring from `anchor`'s, `ctr`
    /// is at most `size` from `anchor`'s centre, and an overlapping hex's centre is at most
    /// `r_scene + size` from `ctr`. `anchor` is returned alone when nothing overlaps, mirroring
    /// the square implementation's zero-radius guarantee.
    fn footprint_cells(&self, anchor: Cell, ctr: vision::P, r_scene: f64, cell: f64) -> Vec<Cell> {
        let mut out = Vec::new();
        let r = r_scene.max(0.0);
        let inradius = self.size * 3.0_f64.sqrt() / 2.0;
        let ring_radius = ((r / (self.size * 1.5)).ceil() as i32).max(0) + 2;
        for dq in -ring_radius..=ring_radius {
            for dr in -ring_radius..=ring_radius {
                let ds = -dq - dr;
                if ds.abs() > ring_radius {
                    continue; // outside the hex-shaped scan region
                }
                let c = (anchor.0 + dq, anchor.1 + dr);
                let center = self.cell_center(c);
                let dx = center.0 - ctr.0;
                let dy = center.1 - ctr.1;
                let d_center = (dx * dx + dy * dy).sqrt();
                if d_center <= r + inradius {
                    out.push(c);
                } else if d_center <= r + self.size
                    && self.distance_to_cell_polygon(c, ctr, cell) <= r
                {
                    out.push(c);
                }
            }
        }
        if out.is_empty() {
            out.push(anchor);
        }
        out
    }
```

- [ ] **Step 4: Run the tests and every consumer's suite**

Run: `cd src/server && cargo test --lib scene::grid_shape && cargo test --lib scene::pathfinding && cargo test --lib scene::move_exec && cargo test --lib scene::navmesh && cargo test --lib scene`

RUN, OBSERVE, RECORD. Report the three new tests and, separately, every pre-existing test naming a
footprint or a hex parity property, by name and outcome.

The change is exact-equal to the previous predicate for a CENTRE-anchored disc in the six axial
directions and can only ADD cells in the vertex directions, so a centre-anchored fixture should not
move. **A pre-existing fixture that changes outcome is a stop-and-report**, with its name, its
message, and which of the two anchors it uses — a moved centre-anchored fixture would contradict
the equality above and is a finding, not a fixup.

- [ ] **Step 5: Mutation check — prove the polygon branch is load-bearing**

Temporarily delete the `else if` arm (leaving only the `d_center <= r + inradius` fast accept),
run `cd src/server && cargo test --lib scene::grid_shape`, record the observed failing test names
and messages, revert, re-run, and confirm green plus a byte-identical diff against the
pre-mutation file. If the suite stays green the vertex case is uncovered and that is the finding
to report; stop rather than proceeding.

- [ ] **Step 6: Run the full server gate**

Run: `cd src/server && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

- [ ] **Step 7: Commit**

```bash
git add src/server/src/scene/grid_shape.rs
git commit -m "fix(scene/grid_shape): measure a hex footprint against the hex, not its centre

The hex overlap test compared a cell centre's distance against the inradius,
which is correct only when the disc is centred on the anchor hex. A hex the
disc reaches near a vertex has its centre up to the circumradius away, so the
two callers that anchor at an arc-length sample point rather than a cell centre
lost those cells from the footprint the fog clip requires visible — the
direction that lets a route run further into fog than the mask allows. The
predicate now measures the distance to the hex polygon, with exact
inscribed/circumscribed bounds settling the common cases first.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>" -- src/server/src/scene/grid_shape.rs
```

---

### Task 5: PW1 + PW2 — one grid-derived extent, one grid-derived step distance

**Ledger ids:** PW1, PW2. **Buddy-checked (PHASE = code).**

**This task is dispatched only after the Pre-dispatch measurement section has been run and its
table pasted into Step 9.** Without it, Step 9's stop-and-report rule fires on a correct
implementation the first time a fixture nobody enumerated moves.

**The unit question is already settled and is not re-opened.** `ResolvedScene.bounds` is
`(width, height)` **in grid units**: `eng::SceneEngine.bounds`'s field doc says "Navmesh outer
rectangle in grid units", `ResolvedScene.bounds`' says "Scene dimensions (width, height) in grid
units", and the client carries no consumer that converts them — it only authors them. So
`build_navmesh` and `env_light_polys`, which multiply by `cell`, are reading the value correctly
and merely converting it wrongly on hex; `bound_for_scene`, which compares the value directly
against raw wall coordinates, is the consumer that is wrong about the unit.

**The square-grid consequence is deliberate and is not "no change by construction".** The
conversion is the identity only at cell size 1. At any other cell size `bound_for_scene`'s scene
contribution grows by a factor of `cell` on each axis, which widens the wall-less/sparsely-walled
vision bound, the movement-gate mask built from it, the lit cells shipped from it, and the
explored blob accumulated from those. That widening is the CORRECTION — the bound was comparing a
cell COUNT against wall coordinates. It reaches further than the fixtures that author bounds:
`DEFAULT_SCENE_BOUNDS_UNITS` is itself in grid units, so **every scene fixture that authors no
bounds at all** also moves, from a 100-world-unit extent to 100 cells' worth. The Pre-dispatch
measurement is what turns that from a category into a list.

**The token footprint radius is NOT converted, here or anywhere in this phase.** `world_extent`
and `world_units_per_cell` are the two conversions this task introduces, and the footprint radius
reads NEITHER: `navmesh_for` computes `footprint_radius_cells * cell`, exactly as `build_navmesh`
does today. The reason is that converting it changes GAME SEMANTICS rather than fixing a
mis-scaling: `resolve_token_footprint` derives the radius as a square block's half-diagonal
(`hypot(w,h)/2`), so a 1×1 token gets `0.707` — under the indexing scale that disc is `0.707·size`
on hex, short of the `0.866·size` inradius, and the token occupies exactly its own hex; multiplied
by `√3` it becomes `1.22·size`, past the inradius, and a medium creature would occupy seven hexes.
The model behind the value is a square block, and giving hex its own footprint model is a rules
decision with its own design pass, not a mechanical sweep. Task 6 documents that at the four sites
that hold it.

**Files:**
- Modify: `src/server/src/scene/grid_shape.rs` (the `GridShape` trait and both impls)
- Modify: `src/server/src/scene/navmesh.rs` (`build_navmesh`)
- Modify: `src/server/src/scene/lighting.rs` (`env_light_polys`)
- Modify: `src/server/src/scene/vision.rs` (`bound_for_scene` parameter name + doc)
- Modify: `src/server/src/scene/mod.rs` (`SceneEcs::scene_world_extent`, `navmesh_for`,
  `lighting_inputs`, `lighting_inputs_from`, `visible_cells_cached`, `player_vision_polygons`,
  `player_vision_inputs`, `VisionMoveInputs`, `source_los_poly` and its two callers, `pathfind`'s
  weighted-continuous cost conversion)
- Modify: `src/server/src/scene/grid_shape_parity_tests.rs`, `src/server/src/scene/move_exec.rs`
  (fixture bounds re-derivation only — no production change in either)
- Test: `src/server/src/scene/grid_shape.rs`, `src/server/src/scene/navmesh.rs`,
  `src/server/src/scene/lighting.rs`, `src/server/src/scene/mod.rs` (`mod tests` in each)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `GridShape::world_units_per_cell(&self) -> f64`
  - `GridShape::world_extent(&self, bounds_cells: (f64, f64)) -> (f64, f64)`
  - `SceneEcs::scene_world_extent(&self, scene: Uuid) -> (f64, f64)` (`pub(crate)`)
  - `navmesh::build_navmesh(extent: (f64, f64), footprint_scene: f64, walls: &[Seg]) -> Option<NavMesh>`
  - `lighting::env_light_polys(extent: (f64, f64), cell_size: f64, light_walls: &[Seg]) -> Vec<Vec<P>>`
    — only the first parameter's meaning changes; `cell_size` keeps both of its roles.
  - `SceneEcs::lighting_inputs_from`'s `bounds` parameter becomes `extent`.
  - `vision::bound_for_scene`'s `scene_bounds` parameter becomes `scene_extent`.

  Task 6 consumes `world_units_per_cell` at four further sites.

- [ ] **Step 1: Write the failing tests for the two conversions**

Add to `mod tests` in `src/server/src/scene/grid_shape.rs`:

```rust
    #[test]
    fn square_world_units_per_cell_is_the_cell_size() {
        // Discrimination: fails if the square implementation returns anything other than its own
        // `cell` — including a hex-shaped formula accidentally shared between the two impls.
        let g = SquareGrid { cell: 37.5, rule: DiagonalRule::Chebyshev };
        assert_eq!(g.world_units_per_cell(), 37.5);
    }

    #[test]
    fn hex_world_units_per_cell_equals_the_distance_between_adjacent_centers() {
        // Derived from the geometry, not from the implementation: the value must equal the
        // measured distance from a hex's centre to each of its six neighbours' centres.
        // Discrimination: fails for `size`, `1.5*size`, or any constant other than `√3*size`,
        // because the expectation is computed from `cell_center` rather than restated.
        let g = HexGrid { size: 50.0 };
        let origin = g.cell_center((0, 0));
        for (dq, dr) in [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)] {
            let n = g.cell_center((dq, dr));
            let d = ((n.0 - origin.0).powi(2) + (n.1 - origin.1).powi(2)).sqrt();
            assert!(
                (d - g.world_units_per_cell()).abs() < 1e-9,
                "neighbour ({dq},{dr}) sits {d} away, world_units_per_cell reports {}",
                g.world_units_per_cell()
            );
        }
    }

    #[test]
    fn square_world_extent_wholly_contains_every_cell_of_the_authored_block() {
        // Square's guarantee is a FULL cover: every cell's own rectangle lies inside the extent.
        // Discrimination: fails if the square extent gains a margin, loses a cell through a
        // `w - 1` term, or picks up a per-axis asymmetry — the containment check is per cell and
        // the closed form is asserted separately.
        let g = SquareGrid { cell: 20.0, rule: DiagonalRule::Chebyshev };
        let (w, h) = (8.0_f64, 5.0_f64);
        let (ex, ey) = g.world_extent((w, h));
        for i in 0..w as i32 {
            for j in 0..h as i32 {
                let c = g.cell_center((i, j));
                assert!(c.0 + 10.0 <= ex + 1e-9, "cell ({i},{j}) exceeds extent x {ex}");
                assert!(c.1 + 10.0 <= ey + 1e-9, "cell ({i},{j}) exceeds extent y {ey}");
                assert!(c.0 - 10.0 >= -1e-9 && c.1 - 10.0 >= -1e-9, "cell ({i},{j}) starts below the origin");
            }
        }
        assert_eq!((ex, ey), (160.0, 100.0));
    }

    #[test]
    fn hex_world_extent_contains_every_cell_centre_and_the_far_cells_vertices() {
        // Hex's guarantee is a CENTRE cover plus the extreme cell's far vertices — not a full
        // cover; the origin-side truncation is asserted by the next test.
        // Discrimination: fails for a `w*size`/`h*size` reading, for a per-axis pitch that omits
        // the axial shear, and for any formula that leaves the far cell's own far vertex outside
        // — every check is derived from `cell_center` plus the pointy-top half-extents.
        let g = HexGrid { size: 50.0 };
        let (w, h) = (9.0_f64, 7.0_f64);
        let (ex, ey) = g.world_extent((w, h));
        let half_x = 50.0 * 3.0_f64.sqrt() / 2.0;
        for q in 0..w as i32 {
            for r in 0..h as i32 {
                let c = g.cell_center((q, r));
                assert!(c.0 >= -1e-9 && c.1 >= -1e-9, "hex ({q},{r}) centre is below the origin");
                assert!(c.0 <= ex + 1e-9 && c.1 <= ey + 1e-9, "hex ({q},{r}) centre exceeds the extent");
            }
        }
        let far = g.cell_center((w as i32 - 1, h as i32 - 1));
        assert!(far.0 + half_x <= ex + 1e-9, "the far hex's right vertex exceeds extent x {ex}");
        assert!(far.1 + 50.0 <= ey + 1e-9, "the far hex's bottom vertex exceeds extent y {ey}");
    }

    #[test]
    fn hex_world_extent_leaves_the_origin_cells_negative_margin_outside() {
        // The truncation the extent's doc states, pinned so the doc cannot drift into claiming a
        // full cover: the origin hex is centred ON the origin, so its lower vertex sits at
        // `(0, -size)` and its left vertices at `x = -√3/2·size`, both outside an origin-anchored
        // rectangle. A consumer that treats the rectangle as the whole play area excludes them.
        // Discrimination: fails if `world_extent` ever starts returning a rectangle with a
        // negative origin, which would silently change what `(0,0)–extent` means for every
        // consumer that assumes the origin corner.
        let g = HexGrid { size: 50.0 };
        let (ex, ey) = g.world_extent((9.0, 7.0));
        let half_x = 50.0 * 3.0_f64.sqrt() / 2.0;
        let inside = |p: (f64, f64)| p.0 >= 0.0 && p.1 >= 0.0 && p.0 <= ex && p.1 <= ey;
        assert!(!inside((0.0, -50.0)), "the origin hex's lower vertex is outside");
        assert!(!inside((-half_x, -25.0)), "the origin hex's left vertex is outside");
        assert!(inside(g.cell_center((0, 0))), "the origin hex's centre is inside");
    }

    #[test]
    fn hex_world_extent_exceeds_the_bounds_size_product_on_both_axes() {
        // The axial shear makes a w×h block a rhombus, so its covering rectangle is wider than a
        // per-axis pitch product on both axes.
        // Discrimination: fails if the hex impl falls back to the square formula.
        let g = HexGrid { size: 50.0 };
        let (w, h) = (40.0_f64, 40.0_f64);
        let (ex, ey) = g.world_extent((w, h));
        assert!(ex > w * 50.0, "hex extent x {ex} must exceed {}", w * 50.0);
        assert!(ey > h * 50.0, "hex extent y {ey} must exceed {}", h * 50.0);
    }

    #[test]
    fn world_extent_is_positive_for_a_sub_single_cell_block() {
        // A fractional authored bound below one cell must not produce a negative or zero
        // rectangle through a `w - 1` term. Discrimination: fails if either impl subtracts one
        // cell without clamping.
        let sq = SquareGrid { cell: 20.0, rule: DiagonalRule::Chebyshev };
        let hx = HexGrid { size: 20.0 };
        for (ex, ey) in [sq.world_extent((0.25, 0.25)), hx.world_extent((0.25, 0.25))] {
            assert!(ex > 0.0 && ey > 0.0, "extent must stay positive, got ({ex}, {ey})");
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src/server && cargo test --lib scene::grid_shape`

RUN, OBSERVE, RECORD. The compiler is the first signal here (the trait members do not exist yet);
record its actual output rather than asserting what it will say.

- [ ] **Step 3: Add the two trait members and both implementations**

`grid_shape.rs` carries `#![deny(missing_docs)]` and
`#![deny(clippy::missing_docs_in_private_items)]`, so every new item needs a doc comment.

Add to the `GridShape` trait, after `cell_vertices`:

```rust
    /// World-space distance represented by ONE unit of grid distance on this shape — the
    /// distance between two adjacent cell centres.
    ///
    /// This is the conversion for a quantity a GM authors in CELLS that the engine must measure
    /// in world units: light bright/dim radii, a vision mode's `default_range`, animation speed,
    /// and the router's reported cost. Square returns its own cell size, so nothing changes
    /// there; pointy-top hex returns `√3 · size`, because all six axial neighbours sit that far
    /// from a hex's centre while `size` is only its circumradius.
    ///
    /// NOT the cell INDEXING scale. `cell_of`, `cell_center`, `cells_in_bounds`, `cell_bounds`,
    /// `footprint_cells` and `line_traversal` index against the shape's own stored scale and must
    /// never be re-scaled by this value. The two coincide on square, which is exactly why a site
    /// that confuses them stays invisible until a hex scene runs through it.
    ///
    /// NOT the token footprint scale either. A token's footprint radius is a square block's
    /// half-diagonal in cells, and its model is a square block, so it converts through the
    /// indexing scale — scaling it here would give a 1×1 token a disc past the hex inradius and
    /// make a medium creature occupy seven hexes, which is a rules change rather than a unit fix.
    fn world_units_per_cell(&self) -> f64;

    /// The origin-anchored world rectangle `(0,0)–result` for the authored index block
    /// `[0, bounds_cells.0) × [0, bounds_cells.1)`, whose units are GRID units.
    ///
    /// The guarantee differs by grid kind and consumers must not assume the stronger one:
    /// - **Square** — an exact COVER. Cell `(i,j)` occupies `[i·cell,(i+1)·cell)` per axis, so
    ///   the block occupies exactly `(w·cell, h·cell)` with no shear and no overhang.
    /// - **Hex** — a CENTRE cover. Every cell's centre lies inside, and the extreme cell
    ///   `(w-1, h-1)`'s far vertices lie inside, but the axial block is a rhombus whose origin
    ///   row reaches `y = -size` and whose origin cell reaches `x = -√3/2·size`; those margins
    ///   are OUTSIDE the rectangle. Claiming a full cover here would be false.
    ///
    /// What the hex truncation costs each consumer, since over-covering is NOT free for all of
    /// them:
    /// - `navmesh::build_navmesh` triangulates this rectangle, so a continuous position inside
    ///   the origin row's negative-y margin is off-mesh and routes as unreachable. Every cell
    ///   CENTRE — the only position a grid-snapped token occupies — is on-mesh.
    /// - `lighting::env_light_polys` walks this rectangle's perimeter, so the truncated margin
    ///   gets no boundary sample of its own and is lit only through neighbouring samples:
    ///   under-reveal.
    /// - `vision::bound_for_scene` unions this rectangle after clamping its low edges to `≤ 0`
    ///   and expanding by its own `margin`, so the truncation shows only where that margin is
    ///   smaller than the circumradius: under-reveal again.
    ///
    /// Growing the rectangle is not a free hedge in the other direction: the vision bound and the
    /// lit mask are BUILT from it rather than merely gated by it, so a larger rectangle widens
    /// what a player is told they can see, and the environment-light perimeter walk is not
    /// mask-gated at all.
    fn world_extent(&self, bounds_cells: (f64, f64)) -> (f64, f64);
```

Add to `impl GridShape for SquareGrid`:

```rust
    fn world_units_per_cell(&self) -> f64 {
        self.cell
    }

    fn world_extent(&self, bounds_cells: (f64, f64)) -> (f64, f64) {
        // Cell (i,j) covers [i*cell,(i+1)*cell) on each axis, so a w × h block anchored at the
        // origin spans exactly (w*cell, h*cell) with no shear and no overhang.
        let (w, h) = bounds_cells;
        (w.max(0.0) * self.cell, h.max(0.0) * self.cell)
    }
```

Add to `impl GridShape for HexGrid`:

```rust
    fn world_units_per_cell(&self) -> f64 {
        // Every axial neighbour is √3·size away: (1,0) → (√3·size, 0); (0,1) →
        // (√3/2·size, 3/2·size), whose length is size·√(3/4 + 9/4) = √3·size.
        self.size * 3.0_f64.sqrt()
    }

    fn world_extent(&self, bounds_cells: (f64, f64)) -> (f64, f64) {
        // The far corner of the axial block is cell (w-1, h-1): `axial_to_pixel` is monotone
        // increasing in q on x, and in r on BOTH axes (the shear), so that cell maximises each
        // coordinate. Add the pointy-top half-extents — √3/2·size across the flats (x) and the
        // circumradius `size` to a vertex (y) — so that hex's far vertices are inside.
        let (w, h) = bounds_cells;
        let qmax = (w - 1.0).max(0.0);
        let rmax = (h - 1.0).max(0.0);
        let sqrt3 = 3.0_f64.sqrt();
        let max_x = self.size * (sqrt3 * qmax + sqrt3 / 2.0 * rmax) + self.size * sqrt3 / 2.0;
        let max_y = self.size * 1.5 * rmax + self.size;
        (max_x, max_y)
    }
```

- [ ] **Step 4: Convert `build_navmesh` to take a world extent and a converted footprint distance**

`build_navmesh` currently receives `bounds` (grid units), `cell`, and `footprint_radius_cells`, and
derives both `(w_px, h_px)` and `footprint_scene` itself. Both derivations are conversions this
task centralises at the caller, and `cell` has no other use in that function — so it stops taking
`cell` at all. Removing the parameter is what makes it impossible to re-derive a conversion there.

Two guards live on the parameters being removed, and **both must be shown to survive rather than
assumed to**:

- `!cell.is_finite() || cell <= 0.0 → None`. A non-finite or non-positive `cell` produces a
  non-finite or non-positive `extent` from either `world_extent` impl, which the extent guard
  already refuses. Step 11 pins this at `navmesh_for`, which is now the level where `cell` enters.
- `!(0.0..=MAX_FOOTPRINT_CELLS).contains(&footprint_radius_cells) → None`. `navmesh_for` performs
  the identical check **before** computing its cache key, and must keep doing so; the comment on
  that check currently says it "mirrors `build_navmesh`'s guard exactly so the two stay
  consistent", which stops being true and must be rewritten to state that this is now the sole
  site of the radius-range refusal.

In `src/server/src/scene/navmesh.rs`, change the signature and the head of the body:

```rust
/// Build a footprint-inflated navmesh from a scene's world-unit `extent` (the rectangle
/// `(0,0)–extent`, produced by `GridShape::world_extent` from the scene's authored grid-unit
/// bounds) and its `blocksMove` wall segments, inflating each wall by `footprint_scene` (the
/// mover's footprint radius in world units — the radius in cells times the shape's INDEXING
/// scale, which is what a footprint is measured in). Fails closed (`None`) on: a
/// non-finite/non-positive or over-magnitude extent, a non-finite/negative/over-magnitude
/// `footprint_scene`, an obstacle count over `MAX_NAVMESH_OBSTACLE_SEGMENTS`, or a
/// triangulation/mesh-build failure — callers MUST treat `None` as "no navmesh" (the scene
/// reports `Unreachable`, never a silent all-pass). The radius-RANGE refusal
/// (`0.0..=MAX_FOOTPRINT_CELLS`) lives at the caller, which must apply it before its cache key is
/// computed; a degenerate `cell` reaches this function as a degenerate extent and is refused here.
pub(crate) fn build_navmesh(
    extent: (f64, f64),
    footprint_scene: f64,
    walls: &[Seg],
) -> Option<NavMesh> {
    let (w_px, h_px) = extent;
    if !w_px.is_finite() || !h_px.is_finite() || w_px <= 0.0 || h_px <= 0.0 {
        return None;
    }
    // `footprint_scene` is a single scene-wide value (not per-wall), so an oversized value here
    // fails the whole build rather than skipping one segment. Bounded before it reaches
    // `line.buffer(...)`: an out-of-range-but-finite value saturates to infinity in the `as f32`
    // cast below and panics inside `spade`'s triangulation.
    if !footprint_scene.is_finite() || footprint_scene < 0.0 {
        return None;
    }
    let footprint_scene = footprint_scene.max(0.01);
    if footprint_scene > MAX_NAVMESH_COORD {
        return None;
    }
    if walls.len() > MAX_NAVMESH_OBSTACLE_SEGMENTS {
        return None;
    }
    // Bound the extent's magnitude before the `as f32` cast below — see `MAX_NAVMESH_COORD`.
    if w_px.abs() > MAX_NAVMESH_COORD || h_px.abs() > MAX_NAVMESH_COORD {
        return None;
    }
```

Everything from `let outer = [...]` onward is unchanged. Delete the removed range check and the
`let (w_px, h_px) = (w * cell, h * cell);` line.

Read the `MAX_NAVMESH_COORD` doc-comment paragraphs that describe "derived `w_px`/`h_px`
scene-pixel bounds" and rewrite them to describe the values as they now arrive (a caller-supplied
world extent). Read the paragraph about a "tiny-but-finite `bounds` paired with an extreme `cell`"
and rewrite it in terms of `footprint_scene` arriving pre-multiplied. Treat every comment on a
line you touch as stale until verified against the new code.

Update `navmesh_for` in `src/server/src/scene/mod.rs`:

```rust
        let cell = self.scene_grid_sizes().get(&scene).copied()?;
        let grid = self.resolve_grid_shape(scene, cell);
        let extent = grid.world_extent(self.resolve_scene(scene).bounds);
        // The footprint radius is authored against the INDEXING scale (a square block's
        // half-diagonal in cells), not the per-cell world distance — see
        // `GridShape::world_units_per_cell`'s own note on why scaling it is a rules change.
        let footprint_scene = footprint_radius_cells * cell;
        let built = navmesh::build_navmesh(extent, footprint_scene, walls)?;
```

The `!(0.0..=MAX_FOOTPRINT_CELLS).contains(&footprint_radius_cells)` guard at the top of
`navmesh_for` is unchanged and stays where it is — it must run before the cache key is computed.
Rewrite only its trailing "Mirrors `build_navmesh`'s guard exactly" sentence to state that this is
the sole site of the range refusal and that `build_navmesh` refuses only on the converted
distance's magnitude.

Then enumerate every caller of `build_navmesh` and record the disposition of each. Run and record:

```bash
cd /c/Dev/Shadowcat && git grep -n "build_navmesh" -- src/server
```

Every test calling it with four arguments converts to the three-argument form by computing the
same extent and footprint distance the caller now computes. **Two of those tests exist to pin the
removed guards** — the degenerate-`cell` refusal and the radius-cap refusal. Do not delete them:
re-express each against `SceneEcs::navmesh_for`, which is the level those inputs now enter, and
name both in the report with their old and new call form. A guard whose test cannot be re-expressed
at either level is a stop-and-report.

- [ ] **Step 5: Convert `env_light_polys` and BOTH `lighting_inputs_from` call sites**

`env_light_polys` derives `(w, h)` from `bounds_grid * cell_size`, and separately uses `cell_size`
for its sample count and its bound margin. Split those roles explicitly.

In `src/server/src/scene/lighting.rs`:

```rust
/// Boundary-projected environment-light occlusion polygons. Environment light enters the scene
/// from OUTSIDE its boundary; a cell is lit iff an unobstructed line reaches it from some point on
/// the scene rectangle past the `blocksLight` walls. The rectangle perimeter is sampled, and each
/// sample's visibility polygon is computed with the SAME `vision::visibility_polygon` primitive
/// placed lights and vision use (never a second, forked occlusion computation). A cell is
/// environment-lit iff it lies inside ANY sample's polygon (composed by `env_lit`).
///
/// `extent` is the scene rectangle `(0,0)–extent` in WORLD units, produced by
/// `GridShape::world_extent` from the scene's authored grid-unit bounds. `cell_size` is the grid's
/// INDEXING scale and plays two roles that are both discretization, not measurement: it sets the
/// sample count (one per cell of perimeter, clamped to `[4, MAX_ENV_LIGHT_SAMPLES]`) and the
/// raycast bound's margin, so boundary samples sit strictly inside it. Sample count is a
/// convergence knob, not a secrecy one: the sampled union approaches the true boundary-reachable
/// set FROM BELOW, so a coarser count under-reveals and a finer one is strictly more faithful —
/// which is why the indexing scale, the smaller of the two scalars on hex, is the right input here
/// and `world_units_per_cell` is not.
///
/// Fail-closed: a non-finite or non-positive `extent` or `cell_size` ⇒ empty (environment reaches
/// nothing — under-reveal, never over-reveal). The boundary itself never occludes (only interior
/// `blocksLight` walls do): light enters freely across the scene edge.
pub fn env_light_polys(
    extent: (f64, f64),
    cell_size: f64,
    light_walls: &[vision::Seg],
) -> Vec<Vec<P>> {
    let (w, h) = extent;
    if !w.is_finite()
        || !h.is_finite()
        || w <= 0.0
        || h <= 0.0
        || !cell_size.is_finite()
        || cell_size <= 0.0
    {
        return Vec::new();
    }
    let perim = 2.0 * (w + h);
    let n = (perim / cell_size).round() as usize;
    let n = n.clamp(4, MAX_ENV_LIGHT_SAMPLES);
    let margin = cell_size.max(1.0);
    let bound = vision::Rect {
        minx: -margin,
        miny: -margin,
        maxx: w + margin,
        maxy: h + margin,
    };
    (0..n)
        .map(|i| {
            let d = (i as f64) / (n as f64) * perim;
            vision::visibility_polygon(perimeter_point(w, h, d), light_walls, bound)
        })
        .collect()
}
```

`perim / cell_size` is the sample count for the true perimeter on both grid kinds; on square it
equals `2 * (wg + hg)`, since `perim = 2·(wg·cell + hg·cell)`.

Convert the existing `env_light_polys_*` tests in this module: each passes grid-unit bounds and a
cell size, so each passes `bounds × cell` as the extent and the same cell size, leaving their scene
geometry numerically identical. **Do not change what those tests assert** — if any changes outcome,
stop and report; a behaviour change in an environment-occlusion test is not a mechanical fixup.

Now convert **both** `lighting_inputs_from` call sites. Run and record:

```bash
cd /c/Dev/Shadowcat && git grep -n "lighting_inputs_from" -- src/server
```

There are two, and the second compiles silently if missed because the parameter keeps its
`(f64, f64)` type. Give each a disposition line:

1. `SceneEcs::lighting_inputs` — the per-dispatch path used by `player_lit_mask` and
   `visible_cells`.
2. `SceneEcs::visible_cells_cached` — the MOVEMENT-GATE path, which calls
   `lighting_inputs_from` directly after a snapshot miss. Converting one and not the other forks
   the cached and uncached masks on environment-light occlusion.

Both hold `self`, `scene`, `settings` and `cell`, so both convert identically:

```rust
            self.resolve_grid_shape(scene, cell).world_extent(settings.bounds),
```

in place of `settings.bounds` (in `lighting_inputs` the local binding is `settings`; in
`visible_cells_cached` read the real local names before editing). Rename
`lighting_inputs_from`'s `bounds: (f64, f64)` parameter to `extent: (f64, f64)`, document it as
the world-unit scene rectangle produced by `GridShape::world_extent`, and pass it straight to
`env_light_polys`.

`VisibilityInputsSnapshot` needs no change for this, and — because Task 2 already landed
`grid_kind` on `ResolvedScene` — the snapshot is COMPLETE for this derivation: it stores `settings`
(carrying both `bounds` and `grid_kind`) and `cell`, which are the three inputs the extent depends
on. Confirm that against the struct as it stands and state the confirmation in the report; if
`grid_kind` is not on `ResolvedScene` when you read it, Task 2 did not land and this is a
stop-and-report.

- [ ] **Step 6: Convert the `bound_for_scene` consumers, which are the unit fork itself**

Add to `src/server/src/scene/mod.rs`, next to `resolve_grid_shape`:

```rust
    /// The scene's authored bounds converted to a world-unit rectangle through its own
    /// `GridShape` — the conversion every vision-bound consumer that does not already hold a
    /// resolved shape and settings reads, so the raw grid-unit value never reaches a comparison
    /// against world coordinates. Equal by construction to
    /// `resolve_grid_shape(scene, cell).world_extent(resolve_scene(scene).bounds)`, which is what
    /// the sites already holding both compute inline.
    ///
    /// `(0.0, 0.0)` when the scene has no live document: `scene_grid_sizes` carries an entry for
    /// every live scene, so an absent entry means the scene is gone and no extent may be
    /// synthesised. A zero extent contributes nothing to `vision::bound_for_scene`'s union,
    /// leaving the wall-derived bound — the under-reveal direction.
    pub(crate) fn scene_world_extent(&self, scene: Uuid) -> (f64, f64) {
        let Some(cell) = self.scene_grid_sizes().get(&scene).copied() else {
            return (0.0, 0.0);
        };
        self.resolve_grid_shape(scene, cell)
            .world_extent(self.resolve_scene(scene).bounds)
    }
```

Then rename the parameter in `src/server/src/scene/vision.rs` and correct its doc:

```rust
/// `bound_for`, unioned with the scene's own world-unit extent (`(0,0)` to `scene_extent`,
/// clamped to non-negative). `scene_extent` is in WORLD units — a caller passes
/// `GridShape::world_extent` of the scene's authored grid-unit bounds, never the raw bounds,
/// which are a cell COUNT and would otherwise be compared here against wall coordinates. A
/// wall-derived bound smaller than the scene's extent is grown to cover the whole scene instead,
/// so a wall-less (or near-wall-less) scene reveals its own full extent rather than a small
/// `margin` box around the viewpoint. A wall-derived bound that already exceeds the extent (e.g. a
/// wall placed beyond the authored bounds) is left unchanged: this only ever grows the bound.
pub fn bound_for_scene(viewpoint: P, walls: &[Seg], scene_extent: (f64, f64), margin: f64) -> Rect {
    let wall_bound = bound_for(viewpoint, walls, margin);
    let (width, height) = scene_extent;
```

The body below that line is unchanged.

Now enumerate the call sites. Run and record the full output **before and after**:

```bash
cd /c/Dev/Shadowcat && git grep -n "bound_for_scene" -- src/server
```

There are **four** production call sites, not three, and each gets its own disposition line:

1. `VisionMoveInputs::polygons_at` — reads the struct field. Rename the field
   `scene_bounds: (f64, f64)` to `scene_extent`, update its doc to say it is the world-unit
   rectangle, and follow the rename here.
2. `SceneEcs::player_vision_polygons` — replace
   `let scene_bounds = self.resolve_scene(scene).bounds;` with
   `let scene_extent = self.scene_world_extent(scene);` and pass `scene_extent`.
3. `SceneEcs::player_vision_inputs` — the same replacement; it fills the `VisionMoveInputs` field
   renamed in (1), on both the `has_owned == false` early return and the populated construction.
4. `source_los_poly` — rename its `scene_bounds: (f64, f64)` parameter to
   `scene_extent: (f64, f64)`, update its doc comment (which names `ResolvedScene.bounds` as the
   value it receives) to name the world extent, and update both of its callers —
   `accumulate_visible_cells` and `player_lit_mask`'s per-source loop — to pass the extent. Both
   already resolve a `GridShape` for the scene (`grid` / `cell_grid`) and hold `settings`, so each
   passes `grid.world_extent(settings.bounds)` locally rather than re-entering the ECS.

The remaining hits from that grep are doc comments and test comments naming the symbol; each must
be checked for a claim the rename or the unit change falsifies, and each gets a disposition line
too. One of them, on `hex_open_scene`, states the exact rectangle the fixture produces and is
re-derived in Step 9.

Add the parity pin, so the two ways of computing the extent cannot drift:

```rust
    #[test]
    fn scene_world_extent_agrees_with_the_shapes_own_conversion() {
        // Two call shapes exist — the ECS helper for callers holding only a scene id, and the
        // inline `grid.world_extent(settings.bounds)` for callers already holding both — and a
        // divergence between them would fork the vision bound from the lit mask.
        // Discrimination: fails if either shape starts reading a different bounds value or a
        // different shape, which is the only way the two can disagree.
        let (ecs, _user, scene) = hex_open_scene();
        let cell = 50.0;
        let inline = ecs
            .resolve_grid_shape(scene, cell)
            .world_extent(ecs.resolve_scene(scene).bounds);
        assert_eq!(ecs.scene_world_extent(scene), inline);
    }
```

- [ ] **Step 7: Convert the weighted-continuous cost (PW2)**

In `SceneEcs::pathfind`'s `Continuous` branch, the weighted sub-path converts `pathfinding::find`'s
cost from cells to scene units. Replace the multiplier and rewrite the comment:

```rust
                    // `find` reports cost in CELLS; the continuous engine reports SCENE UNITS
                    // (parity with the polyanya path below, which measures Euclidean length).
                    // The conversion is the shape's own per-cell world distance, not the cell
                    // size: on hex those differ by the √3 factor between a hex's circumradius
                    // and the distance to its neighbours.
                    let weighted = pathfinding::PathOutcome {
                        cost: weighted.cost * grid_shape.world_units_per_cell(),
                        ..weighted
                    };
```

`grid_shape` — not `euclid_shape` — is already the binding in scope for the smoother on the line
below, and the two are cell-identical by construction (the diagonal rule feeds only
`neighbors_with_cost`/`heuristic`), so either yields the same scalar; use `grid_shape` so one
binding serves both. Read the real binding names before editing and report if they differ.

- [ ] **Step 8: Enumerate every fixture that authors bounds, and derive the hex coordinates — BEFORE reading Step 9**

Two separate outputs, both written down before Step 9 is read.

**(a) The enumeration.** Run and record the full output:

```bash
cd /c/Dev/Shadowcat && git grep -n '"bounds"' -- src/server
cd /c/Dev/Shadowcat && git grep -n "DEFAULT_SCENE_BOUNDS_UNITS" -- src/server
```

For each hit, record the enclosing fixture or test function, the authored numbers, the scene's
authored grid size, the scene's grid KIND, and your classification into exactly one of the classes
below. **A fixture that takes its grid kind as a parameter gets one row per instantiation, not
one row per fixture** — the two rows can classify the same and still re-derive differently, because
the world rectangle a division preserves on square is not the rectangle it produces on hex. Find
those fixtures by searching for the parameterised shape as well as the literal `"kind": "hex"`
spelling; `move_exec::tests::scene_with_narrow_gap_and_wide_token` is a known instance and is an
input to that search, not a substitute for it.

- **W — authored in WORLD units.** The number is a world distance that happens to sit in a
  grid-unit field. The re-derivation divides it by the scene's cell size, which reproduces the same
  world rectangle exactly and leaves every dependant assertion untouched.
- **G — authored in GRID units already.** The number is correct in the documented unit, so it is
  NOT edited; the scene's extent legitimately grows by a factor of the cell size, and any dependant
  whose assertions move is re-derived on its own terms.
- **U — unaffected.** The test asserts `ResolvedScene.bounds` itself and never reaches a consumer
  of the extent.

Then record the category the grep cannot show: **every scene fixture that authors no bounds falls
back to `DEFAULT_SCENE_BOUNDS_UNITS`, which is in GRID units**, so its extent changes by a factor
of the cell size too. Do not attempt to enumerate those by reading — the Pre-dispatch measurement's
recorded table is their enumeration.

**(b) The hex derivation.** Compute each of these from `HexGrid::cell_center` and the
`world_extent` implementation added in Step 3, and write the values down:

1. `HexGrid { size: 50.0 }.world_extent((3.2, 3.0))`.
2. The rectangle `vision::bound_for_scene` yields for a source at `(0,0)` with no walls,
   `VISION_BOUND_MARGIN = 100`, and that extent.
3. The centre x of hexes `(2,0)`, `(3,0)`, `(4,0)` and `(5,0)` at `size = 50`.
4. The left-vertex x of hexes `(3,0)`, `(4,0)` and `(5,0)` — the centre minus the inradius.
5. Which of those hexes have their centre inside the rectangle from (2), and which have their
   centre outside but their left vertices inside.
6. For `move_exec::tests::scene_with_narrow_gap_and_wide_token`'s HEX instantiation (grid size
   100): the authored bounds pair its expressions evaluate to, that pair divided by the grid size,
   `HexGrid { size: 100.0 }.world_extent(...)` of the divided pair, and — for comparison — the
   rectangle that scene contributes to `bound_for_scene` today. State which of the two rectangles
   contains the other.

- [ ] **Step 9: Compare against the plan's values, then re-derive the fixtures**

The plan's own readings, recorded here so Step 8 could not be anchored by them.

**Its hex values:** `world_extent((3.2, 3.0))` = `(320.429…, 200.0)`; the bound rectangle is
`[-100, 320.429…] × [-100, 200]`; centres `(2,0) = 173.205`, `(3,0) = 259.808`,
`(4,0) = 346.410`, `(5,0) = 433.013`; left vertices `(3,0) = 216.506`, `(4,0) = 303.109`,
`(5,0) = 389.711`. So `(2,0)` and `(3,0)` are centre-inside; `(4,0)` is centre-outside with its
left vertices inside; `(5,0)` is outside on both.

**Its classification of the authored sites:**

| Site | Authored | Class | Re-derivation |
|---|---|---|---|
| `scene::tests::wall_less_large_scene_all_bright` (helper) | 500 × 500 @ cell 100 | W | → `5.0 × 5.0`. `world_extent` then returns `(500, 500)`, the value the scene contributed before. Its dependants are **five**, not four — `player_lit_mask_wall_less_scene_covers_full_bounds_not_a_degenerate_box`, `visible_cells_wall_less_scene_covers_full_bounds_not_a_degenerate_box`, `visible_cells_agrees_with_player_vision_polygons_bound_on_wall_less_scene`, `accumulate_visible_cells_routes_through_grid_shape_cell_center_not_hardcoded`, `player_lit_mask_routes_through_grid_shape_cell_center_not_hardcoded` — and all five keep every assertion unchanged, including the pinned `(-1..=4)²` cell sets. Rebuild this list from the helper's own call sites rather than reading it off here; a halt rule resting on an enumeration is only as good as the enumeration. |
| `scene::tests::wall_less_scene_gives_full_intrascene_vision_not_a_degenerate_box` (inline scene) | 500 × 500 @ cell 100 | W | → `5.0 × 5.0`. Protected intent: a wall-less scene reveals its own extent, not a `VISION_BOUND_MARGIN` box — `(490,490)` is visible although the wall-derived box stops at `105`. Assertion unchanged. |
| `scene::tests::wall_less_scene_vision_does_not_leak_beyond_its_own_bounds` | 500 × 500 @ cell 100 | W | → `5.0 × 5.0`. Protected intent: the grown bound is still BOUNDED — `(1000,1000)` is outside it. The extent stays `(500,500)`, so the assertion is unchanged. Moving the probe point instead would preserve the letter and drop the scale the test was written at. |
| `scene::tests::player_vision_polygons_and_player_vision_inputs_agree_on_wall_less_bound` | 500 × 500 @ cell 100 | W | → `5.0 × 5.0`. Protected intent: the two vision paths compute the identical bound for one scene. Assertion unchanged. |
| `scene::tests::scene_with_secret_wall_between_two_cells` | 400 × 400 @ cell 100 | W | → `4.0 × 4.0`. Protected intent: bounds wide enough that a detour around the wall's `y=100` endpoint exists. Same world rectangle, assertions unchanged. |
| `grid_shape_parity_tests::two_source_open_scene` | 500 × 500 @ cell 100 | W | → `5.0 × 5.0`. Protected intent: the two-source union is exactly `(-1..=4)²`. Same world rectangle, assertion unchanged. Its header comment says "bounds 500x500 grid units", which is false today and becomes true only after the edit; rewrite it to state the authored block and the world rectangle it produces at cell 100. |
| `grid_shape_parity_tests::lenient_corner_open_scene` | 520 × 520 @ cell 100 | W | → `5.2 × 5.2`. Protected intent: the derived rectangle `[-70,520]²`, from which the strict `[-1,4]²` and lenient `[-1,5]²` sets and the 13-cell corner ring are computed in the header comment. Same rectangle, every assertion and every derived number in that comment unchanged; only the sentence naming the authored bounds changes. |
| `move_exec::tests::scene_with_narrow_gap_and_wide_token`, **square instantiation** (`kind == "square"`, grid size 100; `start = (50,250)`, `goal = (450,250)`, so the authored pair is `850 × 650`) | `goal.0 + 400`, `row_y + 400` | W | → the same expressions divided by the scene's authored grid size, i.e. `8.5 × 6.5`. `SquareGrid::world_extent` then returns `(850, 650)` — the identical world rectangle, so every dependant assertion is unchanged. Protected intent: the play area covers the corridor and the gap with room to spare. |
| `move_exec::tests::scene_with_narrow_gap_and_wide_token`, **hex instantiation** (`kind == "hex"`, grid size 100; `start = hex_cell_center(0,2) = (173.205, 300)`, `goal = hex_cell_center(4,2) = (866.025, 300)`, so the authored pair is `1266.025 × 700`) | the same two expressions | W | → the same division, i.e. `12.66025 × 7.0`. The world rectangle does **not** survive here and must not be claimed to: `HexGrid { size: 100 }::world_extent((12.66025, 7.0))` is `(2625.83…, 1000.0)` against the `(1266.03, 700)` this fixture's scene contributes today — about `×2.07` on x and `×1.43` on y. Protected intent is unchanged (the play area covers the corridor and the gap), and it is preserved a fortiori by a larger rectangle. **Direction, so a hex-arm change is a named re-derivation rather than a surprise:** `bound_for_scene` UNIONS the wall bound with `(0,0)–extent` after clamping the low edges to `≤ 0`, so a larger extent yields a rectangle that strictly contains the current one, the LOS polygon grows with it, and the mask several of this fixture's dependants read can only GAIN cells — never lose one. A dependant that loses a cell contradicts that and is a stop-and-report. |
| `move_exec::tests::scene_with_wall_between_adjacent_cells_and_default_footprint` | 300 × 300 | W | → divided by the scene's authored grid size. |
| `move_exec::tests::scene_with_lit_center_line_only` | 300 × 300 | W | → divided by the scene's authored grid size. |
| `move_exec::tests::scene_with_open_lit_area` | 300 × 300 | W | → divided by the scene's authored grid size. |
| `move_exec::tests::scene_with_arrest_cell_beside_the_path_and_wide_token` | 300 × 300 | W | → divided by the scene's authored grid size. |
| `scene::tests::hex_open_scene` (helper) | 240 × 240 @ size 50 | W | Not divisible in whole cells: the hex extent is a shear-dependent function of the block, so preserving a chosen rectangle needs a fractional authored block. → `3.2 × 3.0`, giving the rectangle derived in Step 8. Its dependants pin specific hexes against that rectangle and are re-derived with it, below. Rewrite the helper's doc comment to state the new rectangle and how it is derived. |
| `scene::tests::env_lit_scene_with_room` | 6 × 6 @ cell 100 | G | **Not edited.** The authored value is already in the documented unit, so the scene's extent grows from a 6-unit square (which the wall-derived box swallowed entirely) to a 600-unit one, and the scanned/lit set grows with it. Whether any assertion moves is what the Pre-dispatch measurement answers. |
| `scene::tests::open_env_lit_scene` | 6 × 6 @ cell 100 | G | **Not edited**, same reasoning. |
| `scene::tests::resolve_scene_bounds_reads_authored_value` | 40 × 25 | U | Asserts `r.bounds` only. Untouched. |
| `scene::tests::resolve_scene_bounds_fail_closed_on_degenerate` | 0 × −5 | U | Asserts the degenerate fallback. Untouched. |

**A disagreement between your Step 8 output and either table is a finding to report**, not a value
to adopt from either side. Report it and stop before editing.

The three `hex_open_scene` dependants are re-derived against the rectangle above:

| Fixture | Protected intent | Re-derivation |
|---|---|---|
| `visible_cells_hex_excludes_cell_whose_center_is_outside_the_mask` | The REJECT direction: a hex whose centre AND whose nearest vertex are outside the rectangle is excluded under both strict and lenient sampling. | Hex `(4,0)` no longer serves — its left vertices are inside. Replace it with hex `(5,0)`, outside on both. The kept assertion that `(2,0)` is in the strict mask is unchanged. |
| `visible_cells_hex_lenient_includes_cell_whose_vertex_clips_the_mask` | The strict→lenient FLIP: a hex whose centre is outside but whose vertex is inside is excluded strictly and included leniently, proving the six-vertex hex geometry is wired. | Hex `(3,0)` no longer flips — its centre is now inside. Replace it with hex `(4,0)`. Update the doc comment's stated coordinates to the re-derived ones. |
| `hex_lenient_mask_lets_the_executor_enter_a_cell_the_strict_mask_stops_at` | The composed behaviour: the executor consumes the widened lenient mask, so the SAME move completes under leniency and truncates under strict sampling. | Its destination hex `(3,0)` is now strict-visible. Move the destination to `(4,0)` — the hex that flips under the re-derived rectangle — and update the two `cell_of(out.stop)` assertions to `(4, 0)`. |

**Now paste the Pre-dispatch measurement's recorded failure table here** and give every row a
disposition: a row whose fixture is class W is fixed by that fixture's division and needs nothing
further; a row whose fixture is class G, or which reads the no-bounds default, states its protected
intent and its re-derived assertion explicitly. **A fixture that fails during Step 12 and appears
in neither this table nor the measurement's is a stop-and-report** — that is a genuine surprise,
and it is the only remaining case the halt rule is for.

- [ ] **Step 10: Add the integration-level assertions**

Add to `mod tests` in `src/server/src/scene/mod.rs`:

```rust
    #[test]
    fn hex_continuous_navmesh_spans_the_authored_play_area() {
        // A hex scene authored 20 × 20 grid units at size 50 must route to a hex near the far
        // edge of that authored area. Hex (18,1)'s centre sits well beyond the product of the
        // authored bound and the cell size, so a rectangle built from that product excludes the
        // destination and the route reports unreachable.
        // Discrimination: fails if `world_extent` returns the bounds×size product on hex, because
        // the destination is derived from `cell_center`, not from the extent.
        let g = grid_shape::HexGrid { size: 50.0 };
        let docs = vec![entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "hex", "size": 50 }, "background": null,
                    "bounds": { "width": 20.0, "height": 20.0 },
                    "vision": { "movementModel": "continuous" } }),
        )];
        let mut ecs = SceneEcs::from_documents(docs, 0);
        ecs.set_world_settings_for_test(continuous_world_settings());
        let out = ecs
            .pathfind(
                RouteRequester { user: Uuid::from_u128(1), is_gm: true, explored: None },
                Uuid::from_u128(10),
                g.cell_center((1, 1)),
                &[g.cell_center((18, 1))],
                0.1,
            )
            .expect("a hex cell inside the authored bounds must be routable");
        assert!(out.path.len() >= 2, "route must reach the destination, got {:?}", out.path);
    }

    #[test]
    fn hex_continuous_weighted_cost_is_reported_in_scene_units() {
        // A terrain region flips the continuous dispatch to the weighted grid sub-path, whose
        // cost is converted from cells to scene units. On hex one grid step is √3·size scene
        // units, so the reported cost must be at least the straight-line distance between the
        // endpoints; a conversion through the size itself cannot reach that.
        // Discrimination: the expectation is bounded below by the straight-line distance between
        // the two endpoints, computed from `cell_center`, not from the router's own output.
        let g = grid_shape::HexGrid { size: 50.0 };
        let mut docs = vec![entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "hex", "size": 50 }, "background": null,
                    "bounds": { "width": 20.0, "height": 20.0 },
                    "vision": { "movementModel": "continuous" } }),
        )];
        // A terrain region well away from the route: present only to select the weighted path.
        docs.push(region_doc_top(
            13,
            10,
            "terrain",
            5.0,
            RegionRect { x0: 1200.0, y0: 600.0, x1: 1260.0, y1: 660.0 },
        ));
        let mut ecs = SceneEcs::from_documents(docs, 0);
        ecs.set_world_settings_for_test(continuous_world_settings());
        let a = g.cell_center((1, 1));
        let b = g.cell_center((10, 1));
        let straight = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        let out = ecs
            .pathfind(
                RouteRequester { user: Uuid::from_u128(1), is_gm: true, explored: None },
                Uuid::from_u128(10),
                a,
                &[b],
                0.1,
            )
            .expect("hex continuous weighted route");
        assert!(
            out.cost >= straight * 0.99,
            "cost {} must be at least the straight-line scene distance {straight}",
            out.cost
        );
    }
```

Read the existing `region_doc_top`, `entity_doc_top_eng`, `continuous_world_settings` and
`RegionRect` helpers in that module before using them, and match their real parameter order —
the snippets above show the shape, not a literal drop-in. If the terrain region does not select
the weighted sub-path, read the existing test in the module that asserts exactly that dispatch and
copy its fixture; adjust the fixture, never the assertion.

- [ ] **Step 11: Add the guard tests displaced from `build_navmesh`**

```rust
    #[test]
    fn navmesh_for_refuses_a_scene_whose_grid_size_is_degenerate() {
        // The degenerate-`cell` refusal, pinned at the level the cell size enters: a non-positive
        // grid size yields a non-positive extent, which `build_navmesh`'s extent guard refuses.
        // Discrimination: fails if a degenerate cell size ever produces a mesh rather than
        // `None`, which would let the continuous router run against a collapsed rectangle.
        let scene = entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "square", "size": 0.0 }, "background": null,
                    "bounds": { "width": 10.0, "height": 10.0 } }),
        );
        let ecs = SceneEcs::from_documents(vec![scene], 0);
        assert!(ecs.navmesh_for(Uuid::from_u128(10), 0.4, &[]).is_none());
    }
```

Read `scene_grid_sizes` before writing this: if it filters a non-positive authored size rather
than passing it through, use the value it does pass through for a degenerate scene and say so in
the report — the assertion to preserve is "a degenerate grid produces no mesh", whichever input
expresses it. The radius-cap refusal already has a `navmesh_for` test
(`navmesh_for_rejects_degenerate_radius_even_after_cache_primed_at_zero`); cite it in the report
rather than duplicating it.

- [ ] **Step 12: Run the tests and the whole scene suite**

Run: `cd src/server && cargo test --lib scene`

RUN, OBSERVE, RECORD. Report each new test's outcome and, separately, the outcome of every
pre-existing test in `scene::navmesh`, `scene::lighting`, `scene::grid_shape`, `scene::vision`,
`scene::grid_shape_parity_tests`, `scene::move_exec` and `scene::tests`. A signature change of this
reach produces mechanical call-site fallout; **a pre-existing test that needs a change to its
ASSERTIONS, and appears in neither Step 9's table nor the pasted measurement table, is not
mechanical fallout — stop and report it.**

- [ ] **Step 13: Mutation checks — prove the hex formulas are load-bearing**

Three mutations, run and reverted independently. For each: run
`cd src/server && cargo test --lib scene`, record the observed failing test names and messages,
revert, re-run, and confirm green plus a byte-identical diff against the pre-mutation file.

1. `HexGrid::world_units_per_cell` returns `self.size`.
2. `HexGrid::world_extent` returns `(w * self.size, h * self.size)`.
3. `SquareGrid::world_extent` returns `bounds_cells` unchanged (the pre-conversion reading), which
   is what every re-derived square fixture pins.

If any mutation leaves the suite green, the corresponding formula is unproven and that is the
finding to report — stop rather than proceeding.

- [ ] **Step 14: Run the full server gate**

Run: `cd src/server && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

- [ ] **Step 15: Commit**

```bash
git add src/server/src/scene/grid_shape.rs src/server/src/scene/navmesh.rs src/server/src/scene/lighting.rs src/server/src/scene/vision.rs src/server/src/scene/mod.rs src/server/src/scene/grid_shape_parity_tests.rs src/server/src/scene/move_exec.rs
git commit -m "fix(scene): derive world extent and per-cell distance from the grid shape

A scene's bounds are a cell COUNT and one grid step is not one cell size, so
both conversions now come from GridShape. The navmesh rectangle and the
environment-light perimeter take a world extent instead of deriving one; the
vision bound stops comparing a cell count against wall coordinates; the
weighted continuous route reports its cost in scene units. Square scenes widen
at any cell size other than one, which is the correction rather than a side
effect, and the fixtures that authored a world distance into a grid-unit field
are re-derived to the same world geometry. The token footprint radius keeps the
indexing scale: its model is a square block, and rescaling it would change what
a token occupies.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>" -- src/server/src/scene/grid_shape.rs src/server/src/scene/navmesh.rs src/server/src/scene/lighting.rs src/server/src/scene/vision.rs src/server/src/scene/mod.rs src/server/src/scene/grid_shape_parity_tests.rs src/server/src/scene/move_exec.rs
```

---

### Task 5b: `world_extent` returns the scene's true envelope, not a max corner

**Ledger ids:** owner ruling, escalated out of Task 5's buddy check. **Numbered `5b` deliberately:**
Tasks 6–12 are already dispatched-against and referenced by ledger entries and briefs; renumbering
them would falsify every one of those references.

**Depends on:** Task 5 (which introduced `GridShape::world_extent` and routed the three consumers
through it). Do not start before Task 5's fix round closes.

---

#### The defect

`GridShape::world_extent` returns a single `(f64, f64)` and every consumer reads it as the far
corner of a rectangle anchored at the origin. On square that is exactly right. On hex it is false,
and the trait's own doc already says so: a pointy-top hex block is not origin-anchored, because
axial cell `(0, 0)`'s centre sits at the origin and its own polygon extends half an inradius left
and one circumradius down. The block's true minimum is

```
min = ( -(√3/2)·size , -size )
```

Three consumers each handle that fact independently, and all three handle it the same wrong way —
they hardcode the origin:

| Consumer | How it anchors | What it costs |
|---|---|---|
| `navmesh::build_navmesh` | literal `glam::Vec2::new(0.0, 0.0)` as the outer rectangle's first vertex | axial row `r = 0`'s centres sit exactly ON the mesh's bottom edge; routability there depends on `polyanya::Layer::point_in_polygon` admitting an on-edge point — a third-party convention we do not control |
| `lighting::env_light_polys` | `perimeter_point(w, h, d)` walks from `(0,0)`; the raycast `Rect` runs `-margin` to `w + margin` | environment light enters at the centre row rather than at the block's real edge — under-reveal |
| `vision::bound_for_scene` | `minx: wall_bound.minx.min(0.0)`, `miny: …min(0.0)` | the scene's contribution to the bound starts at the origin, so under `los_restriction = false` the whole-box polygon misses the bottom half of row 0 |

**This is not the usual fork.** The three do not disagree with each other; they agree in being wrong
about the same thing. That is worse in one specific way: a fork is visible the moment two paths are
compared, whereas unanimous agreement on a false anchor reads as a settled convention. The remedy is
the same either way — one symbol returns the truth and nobody restates it.

**Direction of the behaviour change: it reveals cells that were authored, never more than were
authored.** The origin row's hexes are real members of the authored block. Covering their geometry
is not a hedge and is not the growth `world_extent`'s doc warns against when it rejects rounding the
authored bound up — that rejection stands untouched, because it is about inventing cells the GM did
not author. This task invents nothing; it stops truncating cells the GM did author.

---

**Files:**
- Modify: `src/server/src/scene/grid_shape.rs` — the `WorldExtent` type, the trait method, both impls, their tests
- Modify: `src/server/src/scene/navmesh.rs` — `build_navmesh`
- Modify: `src/server/src/scene/lighting.rs` — `env_light_polys`, `perimeter_point`
- Modify: `src/server/src/scene/vision.rs` — `bound_for_scene`
- Modify: `src/server/src/scene/mod.rs` — `SceneEcs::scene_world_extent`, `SceneEcs::player_vision_polygons`, `SceneEcs::player_vision_inputs`, `SceneEcs::navmesh_for`, `SceneEcs::lighting_inputs`, `SceneEcs::lighting_inputs_from`, `SceneEcs::visible_cells_cached`, `SceneEcs::player_lit_mask`, `accumulate_visible_cells`, `source_los_poly`, and their tests
- Modify: `src/server/src/scene/grid_shape_parity_tests.rs` — any parity test reading the extent

**Interfaces:**
- Produces: `WorldExtent { min: (f64, f64), max: (f64, f64) }` with `width()` and `height()`;
  `GridShape::world_extent(&self, bounds_cells: (f64, f64)) -> WorldExtent`. Task 6 consumes
  `world_units_per_cell`, which this task does not touch.
- Consumes: `normalize_bounds_cells`'s `Option` contract from Task 5's fix round — an unusable
  authored bound still yields the value the guards refuse.

---

- [ ] **Step 1: Enumerate every subject before changing anything**

Do NOT grep for a marker and call the result the worklist. Enumerate the bounded set of SUBJECTS and
give every one a row and a disposition, in the report:

1. **Every caller of `world_extent`** (production and test), read out of the source, one row each.
2. **Every test whose assertions encode an origin-anchored rectangle** — including tests that never
   name `world_extent`, e.g. one asserting a navmesh route's coordinates or an env-light sample's
   position. The axis to check is not "does it mention the symbol" but "would this assertion change
   if the hex rectangle gained a negative minimum".
3. **Every comment stating the rectangle is `(0,0)–extent`** in the five files above. There are
   several; the phrase varies, so read the comments rather than searching for that spelling.

Two named instances that MUST appear in your table with a disposition, because both become FALSE
under this change and neither will fail to compile:

- `hex_world_extent_leaves_the_origin_cells_negative_margin_outside` — its name and its assertion
  both state the negative margin is outside the rectangle. Under the envelope it is inside. This
  test inverts; it does not get deleted. The property worth pinning is that the envelope's minimum
  is exactly the origin cell's own lower-left extreme, and that square's minimum is exactly the
  origin.
- `hex_continuous_routes_along_axial_row_zero_including_the_mesh_corner`, and the `world_extent`
  doc clause that explains it. Both currently say row 0's centres are on-mesh *because the mesh's
  containment test admits an exactly-on-boundary point*. Under the envelope they are strictly
  interior and that explanation is false. Rewrite both to state the present fact — the row's centres
  sit one circumradius above the mesh's bottom edge — and rename the test so "mesh corner" does not
  survive as a claim about a point that is no longer the corner. **Keep the route assertions**: the
  test still pins that the row routes, and it now pins it without depending on a third-party
  convention. Say in your report which of its assertions changed and why.

These two are INPUTS, not examples. A disposition line for each is required.

- [ ] **Step 2: Write the failing test for the envelope itself**

In `grid_shape.rs`'s test module:

```rust
#[test]
fn each_shapes_envelope_starts_at_its_own_origin_cells_lower_left_extreme() {
    let size = 50.0_f64;
    let sq = SquareGrid { cell: size, diagonal_rule: DiagonalRule::Chebyshev };
    let hx = HexGrid { size };

    // Discrimination: fails if `world_extent` returns an origin-anchored rectangle on hex, or if
    // the square arm gains a spurious negative margin from a shared normalisation path.
    let s = sq.world_extent((8.0, 6.0));
    assert_eq!(s.min, (0.0, 0.0), "a square block's origin cell starts AT the origin");

    let h = hx.world_extent((8.0, 6.0));
    let (cx, cy) = hx.cell_center((0, 0));
    assert_eq!(cy, 0.0, "fixture guard: axial (0,0)'s centre is the origin row");
    assert!(
        (h.min.0 - (cx - 3.0_f64.sqrt() / 2.0 * size)).abs() < 1e-9,
        "the envelope's x minimum is the origin hex's own left inradius, got {}",
        h.min.0
    );
    assert!(
        (h.min.1 - (cy - size)).abs() < 1e-9,
        "the envelope's y minimum is the origin hex's own bottom circumradius, got {}",
        h.min.1
    );
    assert!(h.width() > 0.0 && h.height() > 0.0, "a positive block has a positive envelope");
}
```

- [ ] **Step 3: Run it and record the failure verbatim**

Run: `cargo test --manifest-path src/server/Cargo.toml each_shapes_envelope`
It cannot compile until Step 4 exists. Record what you observe; do not predict it.

- [ ] **Step 4: Introduce `WorldExtent` and change both impls**

```rust
/// A scene's world-unit envelope: the axis-aligned rectangle that contains every cell of the
/// authored integer block, as both corners rather than a far corner alone.
///
/// `min` is not the origin on every shape. A pointy-top hex block's origin cell is CENTRED on the
/// origin, so its own polygon reaches `-(√3/2)·size` in x and `-size` in y; a square block's origin
/// cell has its corner there, so `min` is the origin exactly. Consumers that triangulate, walk, or
/// bound this rectangle read both corners, which is why the type carries both — an anchor a caller
/// supplies itself is an anchor each caller can get wrong independently.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct WorldExtent {
    /// Lower-left corner in world units.
    pub(crate) min: (f64, f64),
    /// Upper-right corner in world units.
    pub(crate) max: (f64, f64),
}

impl WorldExtent {
    /// The envelope's x span. Zero or negative marks an envelope every consumer refuses.
    pub(crate) fn width(&self) -> f64 {
        self.max.0 - self.min.0
    }

    /// The envelope's y span. Zero or negative marks an envelope every consumer refuses.
    pub(crate) fn height(&self) -> f64 {
        self.max.1 - self.min.1
    }
}
```

Both impls keep their existing `max` closed forms unchanged — this task does not re-derive them; two
reviewers derived them independently and matched. What each gains is its `min`:

- `SquareGrid`: `min = (0.0, 0.0)`.
- `HexGrid`: `min = (-(√3/2)·size, -size)` — derived from the same half-extents the `max` formula
  already adds, so the two corners read one expression per axis rather than two.

**The refusal value becomes a zero-AREA envelope**, not a zero max corner: `normalize_bounds_cells`
returning `None` yields `WorldExtent { min: (0.0, 0.0), max: (0.0, 0.0) }` from both impls. Every
guard below refuses it on span, so the fail-closed behaviour Task 5's fix round pinned is preserved
by construction. Confirm that the non-finite parity test from that round still passes unchanged in
meaning, and update it to compare envelopes rather than corners.

- [ ] **Step 5: Run the new test and the full shape suite**

Run: `cargo test --manifest-path src/server/Cargo.toml grid_shape`

- [ ] **Step 6: Move each consumer onto both corners**

`navmesh::build_navmesh` — take `WorldExtent`. The outer rectangle's four vertices become the
envelope's corners rather than `(0,0)` and `extent`. Its refusal set must not weaken: refuse a
non-finite corner on either axis, a non-positive `width()` or `height()`, and an over-`MAX_NAVMESH_COORD`
magnitude on BOTH corners (today only the far corner is magnitude-checked — with a real minimum, a
corner that is finite but enormous reaches the `as f32` cast the check exists to protect).

`lighting::env_light_polys` — take `WorldExtent`. `perimeter_point` walks the envelope, so it needs
the minimum as well as the spans; the raycast `Rect` runs `min − margin` to `max + margin`. The
sample count stays `perimeter / cell_size`, which is now the envelope's perimeter. The doc's
fail-closed clause stays true and its `(0,0)–extent` phrasing does not.

`vision::bound_for_scene` — take `WorldExtent`. The two `.min(0.0)` clamps become clamps against the
envelope's minimum, and the two `.max(...)` against its maximum. **The `.max(0.0)` guards on
`scene_maxx`/`scene_maxy` are load-bearing and must be preserved in spirit**: a degenerate envelope
must not shrink the wall-derived bound. Preserve that by unioning, never by replacing.

`SceneEcs::scene_world_extent` and the eight sites in `mod.rs` — thread `WorldExtent` through.
`source_los_poly` and `accumulate_visible_cells` pass it along unchanged.

- [ ] **Step 7: Pin the behaviour change where it is observable**

Three tests, each asserting a consumer now covers the hex block's negative margin:

1. **Navmesh**: a hex continuous scene routes between two points BELOW `y = 0` but inside the origin
   row's hexes — impossible today, since the mesh starts at `y = 0`. Its discrimination line is the
   envelope's minimum; confirm by mutating `HexGrid::world_extent`'s `min` to `(0.0, 0.0)` and
   observing this test fail. Revert by `diff`, byte-identical, and re-run.
2. **Env light**: the hex sealed-interior fixture Task 5's fix round added gains a sibling asserting
   a cell in the origin row is environment-lit through the block's real bottom edge.
3. **LOS-off box**: a hex scene with `los_restriction = false` includes the origin row in the
   visible-cell mask. If it already does today (row 0's CENTRES are at `y = 0`, which the current
   box's edge admits), say so in your report and pin the property that actually changed instead of
   asserting one that did not — **do not manufacture a test that passes before and after**.

- [ ] **Step 8: Full gate**

Run from `src/server/`: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test`. Then from the repo root: `node scripts/check-comment-refs.mjs`.
`pnpm build` first if `dist/` is stale.

Report the test delta as `before → after` with the count of tests added and the count removed. **A
removed test needs a named reason.** Fixture-coordinate churn on hex is expected and is not a reason
to delete a test.

- [ ] **Step 9: Commit**

```bash
git commit -m "fix(scene): return the scene's true envelope, not an origin-anchored corner

A pointy-top hex block's origin cell is centred on the origin, so the block
reaches below and left of it. Three consumers each hardcoded the origin as the
rectangle's lower-left corner, truncating the origin row's geometry on the
navmesh, the environment-light perimeter walk, and the LOS-off bound.

world_extent returns both corners. The navmesh triangulates them, the perimeter
walk starts at the minimum, and the vision bound unions the envelope instead of
clamping to zero, so axial row zero's centres are strictly interior rather than
on the mesh boundary.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>" -- src/server/src/scene/grid_shape.rs src/server/src/scene/navmesh.rs src/server/src/scene/lighting.rs src/server/src/scene/vision.rs src/server/src/scene/mod.rs src/server/src/scene/grid_shape_parity_tests.rs
```

---

### Task 5c: three comment/fixture classes the scene subsystem carries, enumerated and closed

**Ledger ids:** surfaced by Task 5's fix rounds and held for their own task. **Numbered `5c` for the
same reason as `5b`:** Tasks 6–12 are already cited by dispatched briefs and ledger entries.

**Depends on:** Task 5 closing. Independent of Task 5b — it touches comments and test fixtures, not
`world_extent`'s signature — but run it after 5b to avoid two agents editing the same test module.

---

#### Why these are one task and not three

All three are the same shape: **a value or a reference that exists in two places, where the copy is
not checked against its source.** They are separated from Task 5 only because Task 5's fix rounds
would have had to sweep them while mid-way through a different subject, and a sweep inside a fix
round is how a class gets scoped to whatever the fix happened to touch.

They are explicitly NOT ruled acceptable by having been held. Each has a measured population and
none is grandfathered.

#### The method requirement, which is the point of the task

**Enumerate SUBJECTS, not markers.** Every prior attempt at these classes searched for a spelling —
`below`, `§`, `cell count` — and every one undercounted, the last by roughly sevenfold. A search
finds the shape you already imagined; the members that matter are the ones spelled differently.

So: for each file in scope, enumerate **every comment in the file**, one row each, and adjudicate it
against the criterion. Report the row count per file. A file's row count that is obviously below its
comment count is the tell that the enumeration was a search wearing an enumeration's clothes.

**Derive from the criterion's wording, never from examples of it.** A round-4 enumeration missed six
members because it enumerated three *forms* of its criterion and matched against those; the wording
covered all six. Enumerating the forms of a criterion narrows the criterion.

---

**Files:** every file under `src/server/src/scene/`, plus `src/server/src/data/engine/` for class C.

**Interfaces:** none. This task changes comments and test-local constants only. No production
behaviour changes; the test suite's pass/fail set must be identical before and after, and a changed
test outcome is a finding to report, not a thing to accommodate.

---

- [ ] **Step 1: Class A — positional references**

A comment that locates something by where it sits rather than by what it is: `below`, `above`,
`the loop below`, `placed ahead of`, `the line following`. **71 are measured to survive** — the
estimate rose from four to thirty to seventy-one across three successive counts, every one of them an
undercount, which is why this task enumerates rather than estimates.

**Classify, do not sweep.** Roughly 40 further lines match the same words while being nothing of the
kind: quantitative uses (`a bound below one cell`), temporal ones (`rejected later by`), and ordinary
prose. A pattern narrow enough to exclude them would exclude real members spelled differently, and
narrowing a detector hides what widening revealed — so keep the match broad and adjudicate every hit
by hand, recording the non-members and their reason alongside the members.

**The conversion is: keep the DESCRIPTIVE name, drop the POSITIONAL word.** "the integer-block loop
below" → "the integer-block loop". The name is what makes a reference findable; the position is the
part that rots, silently, on any reordering — and no gate catches it.

Where a reference has no descriptive name to fall back on, that is the finding: name the thing, or
state the constraint directly instead of pointing at it.

- [ ] **Step 2: Class B — section-style and unnamed-document pointers**

Comments carrying a bare `§N` or an unnamed spec reference — a pointer whose referent cannot be
identified from the code. **13 lines carrying 14 tokens are measured to survive**, 11 of them one
recurring section number.

These pass `check-comment-refs`, which is a fact about the gate's coverage and not a licence. State
the constraint the sentence is about; where the pointer carried nothing, delete the token and change
nothing else — inventing a plausible replacement constraint is the worst outcome available.

- [ ] **Step 3: Class C — configuration-only size restatements**

Around 14 test sites author a grid size in scene JSON and separately restate it as a literal, without
deriving coordinates from a shape. Their failure mode is loud rather than silent — the literal
assertions break — which is why they were separated from the drift class Task 5 closed.

**Add `grid_shape.rs`'s own 11 sites**, which appear in no earlier count because that file sat outside
the worklist that produced every other number here. Treat the totals above the same way: they are the
measured floor, not the answer.

**The shape is minimal: each fixture's authored size gets ONE expression within that test.** Do NOT
build a shared fixture constructor across these — they are diverse scenes, and forcing them through
one constructor would couple unrelated fixtures to make a comment true, which is worse than the
restatement.

- [ ] **Step 4: Verify no behaviour moved**

Run the full server gate and confirm the pass/fail set is unchanged: `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test`, then `node scripts/check-comment-refs.mjs`
from the repo root.

**A test that changes outcome under a comment-and-constant task is a finding.** It means a literal
this task replaced was load-bearing in a way nobody had recorded. Report it; do not adjust the test
to restore green.

- [ ] **Step 5: Make the class a gate, once its population is zero**

Add the positional-reference and unnamed-pointer patterns to `scripts/check-comment-refs.mjs` as
full-tree rules, in the same edit that empties their populations. **No baseline and no allowlist** —
a warn tier or a grandfathered set is an exemption spread across the whole codebase, and a
reported-but-passing violation is indistinguishable to a later reader from code that was checked.

Two constraints on the patterns, both learned from this task's own history:

- **Keep them broad and pay for it in review, never narrow them to silence a collision.** A false
  positive is visible and gets adjudicated; a false negative is invisible forever. Where a legitimate
  quantitative or temporal use collides, change the prose so the collision does not arise rather than
  carving the pattern around it.
- **The gate must print its active exemption count**, if it ends up with any. An uncounted exemption
  is a backdoor and a silent one is indistinguishable from a rule that does not apply.

Verify the gate is real before trusting it: introduce one violation of each new pattern, observe the
gate FAIL, and revert byte-identically by `diff`. A gate that does not gate and a clean tree produce
the same output.

- [ ] **Step 6: Report the measured populations**

Per class: the enumerated population, the number converted, and every member left unconverted with
its reason. **Do not report a class as "complete"** — report what was enumerated and what was done
with each member. A class whose population matches an earlier estimate exactly is worth a second
look; every earlier estimate here was an undercount.

- [ ] **Step 7: Commit**

```bash
git commit -m "docs(scene): replace positional and unnamed-document references, single-source fixture sizes

A positional reference rots on any reordering with nothing to catch it, and a
section pointer naming no document names nothing at all. Each is replaced by the
symbol or the constraint it was pointing at.

Test fixtures that authored a grid size twice now express it once, so a changed
size cannot leave a fixture configuring one scene and asserting about another.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>" -- src/server/src/scene src/server/src/data/engine
```

---

### Task 5d: close the unnamed-pointer class repo-wide and make its detector fatal

**Numbered `5d` for the same reason as `5b`/`5c`.** Split out of Task 5c's Step 5, which was correctly
held rather than decided by its implementer.

---

#### The ruling this task rests on, and the half it deliberately excludes

Task 5c wrote two new detector rules, verified both real, and found they fire on **505 sites
repo-wide, none in the files it enumerated**. It reverted the detector byte-identically and held.
That was right, and the reason is sharper than the site count:

- **481 of those are POSITIONAL references** (`the loop below`, `the guard above`). `RULE 15` as
  written bans *file names, paths, and line numbers*. It does not mention positional words inside a
  file. Extending it to cover them is defensible — "below" rots on any reordering exactly as a line
  number rots on any insertion, and no gate catches either — **but it is a rule EXTENSION, and
  widening a rule into a fatal repo-wide gate is the owner's call in the same way narrowing one is.**
  That question is raised, not answered here.
- **24 of those are unnamed-document pointers** (a bare `§N`, `per brief`, `per spec §3.2`). These are
  **already banned** — `RULE 16` names "unnamed spec references" explicitly, and the campaign's
  standing directive is that its gate is *a gate, never a ratchet*, applying retroactively with
  nothing grandfathered.

So this task closes the second class only. The positional class stays enumerated, unswept, and
explicitly **not** thereby ruled acceptable.

---

**Files:** wherever the enumeration lands — known to include `src/server/src/ws/`, `chat/`, `dice/`,
the client packages, `scripts/`, and the eslint configs — plus `scripts/check-comment-refs.mjs`.

---

- [ ] **Step 1: Enumerate the class, do not trust the count**

24 is a detector's output, not an enumeration. Four successive counts of the sibling class each
undercounted (4 → 30 → 71 → 79), every time because the subject set came from a filter rather than
from the domain.

Enumerate every comment in every file the detector flags **and in every sibling file in those
directories**, one row each, and adjudicate against `RULE 16`'s wording — not against the three forms
the detector happens to match. Report per-file row counts.

**Expect non-members and record them.** A `§` inside a string literal, a URL fragment, or a config
file with no symbols to cite is not a member. Adjudicate; do not carve the pattern around them.

- [ ] **Step 2: Convert each member**

State the constraint the sentence is about and drop the pointer. **Where the pointer carried nothing,
delete the token and change nothing else** — inventing a plausible replacement constraint is the
worst outcome available, worse than leaving the pointer.

Some of these sit in `scripts/` and eslint configs, which `RULE 16`'s carve-out exempts as
config/build files with no symbols to cite. Adjudicate each rather than assuming the carve-out covers
a whole directory.

- [ ] **Step 3: Fix the two items Task 5c reported and left**

Both are in the scene subsystem and squarely in scope; they were reported rather than swept because
they belong to different classes than the three that task enumerated.

- `SceneEcs`'s module-header comment carries a stale plan reference **and** a now-false claim — it says
  the pathfinding and animation fields are resolved in later checkpoints, and both resolvers exist.
  The pointer is a `RULE 16` violation; the false claim is worse and is the reason this is not
  cosmetic. State what the fields are and what resolves them.
- `FIXTURE_GRID_SIZE`'s doc argued in-code that the sites it named had "nothing to drift against" —
  they *were* the drift population. Task 5c rewrote it; verify the rewrite is true of the set as it
  now stands.

- [ ] **Step 4: Make the unnamed-pointer rule fatal, with its population at zero**

Add the rule to `scripts/check-comment-refs.mjs` in the same commit that empties it. **No baseline, no
allowlist, no warn tier** — a warn tier is an exemption spread across the codebase, and a
reported-but-passing violation is indistinguishable to a later reader from code that was checked.

If the rule needs an exemption at all, **it must print its active count** — an uncounted exemption is
a backdoor and a silent one is indistinguishable from a rule that does not apply.

**Verify the gate is real**: introduce one violation, observe the gate FAIL and name it, revert
byte-identically by `diff`, and re-run clean. A gate that does not gate and a clean tree produce
identical output.

**Do not add the positional rule.** Task 5c's verified rule source is preserved in its report for
whenever that question is answered.

- [ ] **Step 5: Full gate**

`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` from `src/server/`;
`pnpm -r test` if any client package was touched; `node scripts/check-comment-refs.mjs` from the root.

**A test that changes outcome under a comment-only task is a finding** — it means a pointer you
removed was load-bearing in a way nobody recorded. Report it; do not adjust the test to restore green.

- [ ] **Step 6: Commit**

Conventional-commits, imperative, stating the constraint and the consequence. No task ids, round
numbers, dates, or process references.

---

### Task 6: Every remaining authored-in-cells quantity reads the shared symbol; the rest are documented non-conversions

**Ledger ids:** PW1, PW2 (same root cause; these are the sites the two named symptoms did not
cover) and NEW-10. **Buddy-checked (PHASE = code).**

**The classification this task turns on.** The scalar `cell` plays three roles in this crate, and
they coincide on square, which is why a site that confuses them is invisible until a hex scene runs
through it.

- **Role A — the cell INDEXING scale.** `floor(p / cell)`, `cell_center`, `cells_in_bounds`,
  `cell_bounds`, `cell_vertices`, `footprint_cells`' own AABB arithmetic, `line_traversal`'s `cell`
  argument, `regions::rasterize`, `explored::mark_polygons`' `cell_size`. **These must not change.**
  `HexGrid` ignores the parameter for indexing entirely (its own `size` is the scale).
- **Role B — an authored quantity measured in cells.** A light's bright/dim radius, a vision mode's
  `default_range`, animation speed in cells/sec, the router's reported cost. **These convert
  through `world_units_per_cell`.**
- **Role C — an internal subdivision or sampling density.** `gate_walk`'s `(cheby / cell).ceil()`
  and `move_stream::sample_path`'s `total_len / cell * SAMPLES_PER_CELL`. **These must not change,
  and the reason is directional:** on hex `cell` is the *smaller* of the two scalars, so it produces
  *more* samples than `world_units_per_cell` would. `gate_walk` feeds the movement gate and
  `sample_path` feeds `clip_to_visible_mask` and `truncate_at_arrest`; coarsening either lets a
  sampled chord skip a cell, which is the fail-open direction. `env_light_polys`' sample count is
  also Role C but for the opposite reason — its sampled union approaches the true reachable set
  from below, so a finer count is strictly more faithful and no secrecy direction is at stake;
  Task 5 documents that at the symbol.

**A fourth category exists and is not Role B**: the **token footprint radius**, at
`move_exec::execute_move`'s `r_scene`, `pathfinding::cell_enterable`'s `r_scene`,
`navmesh::clip_to_visible_mask` and `navmesh::los_smooth`. It is authored in cells, so it looks
like Role B — but it is a square block's half-diagonal and the model behind it is a square block,
so converting it changes what a token OCCUPIES rather than fixing a mis-scaling (see Task 5's
preamble for the numbers). These four keep the indexing scale and gain a comment saying why.

**Files:**
- Modify: `src/server/src/scene/lighting.rs` (`cell_illumination`)
- Modify: `src/server/src/scene/mod.rs` (`player_lit_mask`'s `dist_cells` and its
  `cell_illumination` argument, `point_qualifies`)
- Modify: `src/server/src/scene/move_exec.rs`, `src/server/src/scene/pathfinding.rs`,
  `src/server/src/scene/navmesh.rs` (comments only — the four footprint sites)
- Modify: `src/server/src/ws/room.rs` (`Room::execute_move`'s animation duration)
- Modify: `src/server/src/data/engine/token.rs` (NEW-10 unit docs)
- Test: `src/server/src/scene/mod.rs`, `src/server/src/ws/room.rs`

**Interfaces:**
- Consumes: `GridShape::world_units_per_cell` from Task 5.
- Produces: `lighting::cell_illumination`'s `cell_size` parameter is renamed
  `world_units_per_cell`; `point_qualifies`' `cell: f64` parameter is renamed
  `world_units_per_cell: f64`. No other signature changes.

- [ ] **Step 1: Enumerate the sites from source and classify every hit BEFORE reading Step 2**

Run and record the full output in the task report:

```bash
cd /c/Dev/Shadowcat && git grep -n -- "/ cell\b\|\* cell\b\|/ cell_size\|\* cell_size\|/ self\.cell\|\* self\.cell" -- src/server/src/scene src/server/src/ws
```

Classify **every** hit as Role A, Role B, Role C or footprint-radius against the definitions above,
in a table, before editing anything and before reading Step 2. `Room::execute_move`'s
`duration_ms = (distance / cell) / speed_cells_per_sec * 1000.0` sits in
`src/server/src/ws/room.rs`, which the grep path above does cover; confirm it appears in your
output and flag it if it does not, because a path filter that silently drops a site is the failure
this enumeration exists to catch.

- [ ] **Step 2: Compare your classification against the plan's, and report any disagreement**

The plan's own Role B set, recorded here so Step 1 could not be anchored by it:

| Site | Quantity | Conversion |
|---|---|---|
| `lighting::cell_illumination` | light bright/dim radius, authored in cells | `d / world_units_per_cell` |
| `SceneEcs::player_lit_mask`'s per-cell loop | vision-mode `default_range`, authored in cells, plus its own `cell_illumination` argument | `d / world_units_per_cell` |
| `point_qualifies` | the same range on the shared per-point decision, plus its own `cell_illumination` argument | `d / world_units_per_cell` |
| `Room::execute_move` | animation speed in cells/sec | `distance / world_units_per_cell` |

and its footprint-radius set: `move_exec::execute_move`, `pathfinding::cell_enterable`,
`navmesh::clip_to_visible_mask`, `navmesh::los_smooth` — unconverted, commented.

A hit you classified as Role B that is on neither list, or a listed site your enumeration did not
find, is a discrepancy to REPORT rather than to reconcile silently.

- [ ] **Step 3: Write the failing tests**

All four exercise the real call path. Read the module's existing helpers first and follow their
construction; each snippet marks the fixture region that must be written against them. **Every
threshold below sits a clear half-cell off the value being tested**, so no assertion turns on a
one-ULP difference: a hex two grid steps away is exactly 2.0 grid steps from the source, and a
range of exactly `2.0` would make the comparison an equality on a computed float.

Add to `mod tests` in `src/server/src/scene/mod.rs`:

```rust
    #[test]
    fn a_hex_vision_range_is_measured_in_grid_steps() {
        // A vision mode with a 2.5-cell default range must reach the hex two grid steps away and
        // not the hex three steps away. On a pointy-top hex of size 50 those centres are 2·√3·50
        // and 3·√3·50 scene units out, i.e. 2.0 and 3.0 grid steps; dividing by the indexing scale
        // instead reports 3.46 and 5.20.
        //
        // Discrimination: the assertion is that (2,0) is IN the mask and (3,0) is OUT. Under the
        // indexing-scale divisor (2,0) reads as 3.46 cells and drops out, so the first assertion
        // fails; under any divisor more than 20% larger than √3·size, (3,0) reads as under 2.5
        // cells and joins the mask, so the second fails. The pair brackets the conversion from
        // both sides with half a cell of clearance on each, and the call path is `visible_cells`,
        // which is the production mask rather than a helper.
        // ... assemble a hex scene at size 50, all-bright, LOS off, with one owned token at hex
        // (0,0) whose resolved vision mode has `defaultRange: 2.5` — read the module's existing
        // bounded-range vision test for how a range-carrying mode is authored ...
        let mask = ecs.visible_cells(user, scene, false);
        assert!(mask.contains(&(2, 0)), "two grid steps is inside a 2.5-cell range, got {mask:?}");
        assert!(!mask.contains(&(3, 0)), "three grid steps is outside a 2.5-cell range");
    }

    #[test]
    fn a_hex_vision_range_bounds_the_lit_egress_the_same_way() {
        // `player_lit_mask` computes its own `dist_cells` rather than routing through
        // `point_qualifies`, so the range conversion has two independent homes and a test through
        // one proves nothing about the other. Under strict sampling the two masks must agree.
        //
        // Discrimination: fails if `player_lit_mask`'s divisor keeps the indexing scale, because
        // (2,0) then reads as 3.46 cells and is not shipped. Reuses the fixture above, so a
        // divergence between the gate and the egress shows up as one of the two tests failing.
        // ... same fixture as the test above ...
        let cells: std::collections::BTreeSet<(i32, i32)> = ecs
            .player_lit_mask(user)
            .into_iter()
            .filter(|s| s.scene == scene)
            .flat_map(|s| s.cells.into_iter().map(|(i, j, _b, _t, _h)| (i, j)))
            .collect();
        assert!(cells.contains(&(2, 0)), "two grid steps is inside a 2.5-cell range");
        assert!(!cells.contains(&(3, 0)), "three grid steps is outside a 2.5-cell range");
    }

    #[test]
    fn a_hex_light_radius_is_measured_in_grid_steps() {
        // A lamp with a 2.5-cell bright radius and a 3.5-cell dim radius must light the hex two
        // grid steps away and leave the hex four steps away dark. The distances are the same
        // 2.0/4.0 grid steps as above; the divisor is the only thing under test.
        //
        // Discrimination: fails whenever `cell_illumination` receives the indexing scale, because
        // 2 grid steps then read as 3.46 cells, which exceeds both radii and the cell reports
        // dark. Both masks are asserted because `cell_illumination` has two production callers —
        // `player_lit_mask`'s per-cell closure and `point_qualifies` — and converting one without
        // the other forks the gate from the egress.
        // ... assemble a hex scene at size 50 with lighting ENABLED and a light document at hex
        // (0,0) with bright radius 2.5 and dim radius 3.5, plus one owned token with unlimited
        // normal vision — read the module's existing lit-scene helper for the light document's
        // shape ...
        let cells: std::collections::BTreeSet<(i32, i32)> = ecs
            .player_lit_mask(user)
            .into_iter()
            .filter(|s| s.scene == scene)
            .flat_map(|s| s.cells.into_iter().map(|(i, j, _b, _t, _h)| (i, j)))
            .collect();
        assert!(cells.contains(&(2, 0)), "two grid steps is inside a 2.5-cell bright radius");
        assert!(!cells.contains(&(4, 0)), "four grid steps is beyond the 3.5-cell dim radius");
        let mask = ecs.visible_cells(user, scene, false);
        assert!(mask.contains(&(2, 0)), "the gate mask agrees with the egress mask");
        assert!(!mask.contains(&(4, 0)), "the gate mask agrees with the egress mask");
    }
```

Add to `mod tests` in `src/server/src/ws/room.rs`:

```rust
    #[tokio::test]
    async fn a_hex_move_animates_at_the_grid_step_rate() {
        // Animation speed is authored in cells per second, so one grid step at six cells per
        // second lasts 1000/6 ms whatever the grid kind. On a pointy-top hex of size 50 a step is
        // √3·50 ≈ 86.6 scene units.
        //
        // Discrimination: dividing the travelled distance by the indexing scale reports
        // (86.6/50)/6·1000 ≈ 288.7 ms for the same step, which is outside the tolerance below.
        // The expectation is derived from the authored SPEED and step count, not from the
        // distance the executor returns.
        // ... build a hex scene at size 50 with `speedCellsPerSec: 6`, one owned token at hex
        // (0,0), and move it one step to hex (1,0) — read the module's existing
        // `execute_move`-based tests for the handle construction ...
        let expected_ms = 1000.0 / 6.0;
        assert!(
            (out.duration_ms - expected_ms).abs() < 1.0,
            "one grid step at six cells per second lasts {expected_ms} ms, got {}",
            out.duration_ms
        );
    }
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cd src/server && cargo test --lib scene::tests::a_hex && cargo test --lib ws::room::tests::a_hex_move`

RUN, OBSERVE, RECORD.

- [ ] **Step 5: Convert the four Role B sites**

`lighting::cell_illumination` — rename the parameter, rewrite the doc line and the
`debug_assert!` message, then use it:

```rust
/// `world_units_per_cell` is the world distance one grid step represents
/// (`GridShape::world_units_per_cell`) — light radii are authored in cells, so distance is
/// divided by it. It is NOT the cell indexing scale; the two coincide on square and differ on
/// hex. CALLER PRECONDITION: it must be positive.
```

```rust
    debug_assert!(
        world_units_per_cell > 0.0,
        "INVARIANT: world_units_per_cell must be positive; light radii are authored in cells"
    );
```

```rust
        let dist_cells = if world_units_per_cell > 0.0 { d / world_units_per_cell } else { d };
```

Its two call sites in `scene/mod.rs` — `player_lit_mask`'s per-cell closure and
`point_qualifies` — each already hold a resolved `GridShape` or the converted scalar; pass
`grid.world_units_per_cell()` in place of `cell`. Enumerate them with
`git grep -n "cell_illumination" -- src/server` and give each hit a disposition line, including
the tests in `lighting.rs`: those pass a square fixture where the two scalars are equal, so their
argument's meaning changes while its value does not — say so per test rather than editing values.

`SceneEcs::player_lit_mask` — replace the divisor:

```rust
                    let dist_cells = (((cx - src.vp.0).powi(2) + (cy - src.vp.1).powi(2)).sqrt())
                        / cell_grid.world_units_per_cell();
```

`point_qualifies` — rename its `cell: f64` parameter to `world_units_per_cell: f64`, document it
as the shape-derived conversion, and use it for BOTH of its uses: the `dist_cells` divisor and the
`cell_illumination` argument it forwards. Enumerate its callers with
`git grep -n "point_qualifies" -- src/server` and record the list with a disposition each; there
are three, all inside `accumulate_visible_cells`, and every one must be converted, because
`point_qualifies` is the shared per-point decision behind both `visible_cells` and
`player_lit_mask` and a partially-converted set forks exactly the decision this crate must not
fork. `accumulate_visible_cells` holds `grid`, so each call passes
`grid.world_units_per_cell()`.

`Room::execute_move` (`src/server/src/ws/room.rs`) — the animation duration converts a world
distance into cells so the cells/sec speed applies. The `scene` read guard covering
`resolved_animation_speed` is still held at this line, so resolve the shape there:

```rust
            let speed_cells_per_sec = scene.resolved_animation_speed();
            // Animation speed is authored in cells/sec, so the travelled distance converts
            // through the scene shape's per-cell world distance, not its indexing scale.
            let world_per_cell = scene
                .resolve_grid_shape(token_scene, cell)
                .world_units_per_cell();
```

```rust
            duration_ms = if distance < 1e-9 {
                0.0
            } else {
                (distance / world_per_cell) / speed_cells_per_sec * 1000.0
            };
```

`sample_path`'s `cell` argument on the line below is **Role C and stays `cell`** — add nothing
there, and do not "harmonise" it.

**`MoveExecution.duration_ms`'s own doc states the formula this edit changes, and is named here
because nothing else in the step reaches it.** It reads "Animation duration in milliseconds
(distance / cell / speed \* 1000)", and the field declaration sits several hundred lines above the
computation in the same file — outside any sweep keyed on the lines this task touches, which is
exactly how a doc that restates a formula goes stale invisibly. Rewrite it to name the conversion
the code now performs:

```rust
    /// Animation duration in milliseconds: the travelled distance converted to grid steps through
    /// the scene shape's `GridShape::world_units_per_cell`, divided by the authored cells-per-second
    /// speed. Zero when `stop == start`.
```

Then search the same file for any other prose restating this formula and give each hit a
disposition line; a formula written down twice is only correct until one copy is edited.

- [ ] **Step 6: Comment the four footprint sites as deliberate non-conversions**

Each of these sits next to a converted site and will read as an oversight to the next person
otherwise. At `move_exec::execute_move`:

```rust
    // Constant for the whole walk: the footprint disc radius in world units, mirroring
    // `cell_enterable`'s `r_scene`. The radius is a square block's half-diagonal in cells and the
    // model behind it is a square block, so it converts through the INDEXING scale, not
    // `GridShape::world_units_per_cell` — scaling it would give a 1×1 token a disc past the hex
    // inradius and make a medium creature occupy seven hexes, which is a rules change rather than
    // a unit fix.
    let r_scene = footprint_radius_cells.max(0.0) * cell;
```

Put the same constraint, stated for the local context and without repeating the whole paragraph,
at `pathfinding::cell_enterable`, `navmesh::clip_to_visible_mask` and `navmesh::los_smooth`, each
pointing at `GridShape::world_units_per_cell`'s own note as the shared statement. Report the four
edits individually.

- [ ] **Step 7: Correct the actor-size unit docs (NEW-10)**

In `src/server/src/data/engine/token.rs`, the `Size` struct's `w`/`h` docs and
`TokenOverrides.size`'s doc say "scene units". The live reading is GRID UNITS:
`SceneEcs::resolve_token_footprint` derives `hypot(w,h)/2` and compares the result against
`MAX_FOOTPRINT_CELLS`, and the client's `resolveTokenBox` multiplies `actor.size.w` BY the cell
size to reach scene units. `TokenEngine.w`/`h` genuinely ARE scene units, so the two structs
disagree while sharing a field name and a doc sentence.

Verify both readings against source before editing — cite `resolve_token_footprint`'s comparison
and `resolveTokenBox`'s multiplication in the report — then correct only the actor-size docs:

```rust
    /// Width in GRID UNITS (cells): a medium creature is `1`. `resolve_token_footprint` derives
    /// the footprint radius from this directly and bounds it by `MAX_FOOTPRINT_CELLS`, and the
    /// client multiplies it by the scene's cell size to reach scene units. Not to be confused
    /// with `TokenEngine.w`, which is the token's rendered box in scene units.
    pub w: f64,
```

with the matching sentence on `h`, on `TokenOverrides.size`, and on `ActorEngine.size` if its own
doc carries the same claim. **Leave `TokenEngine.w`/`h` alone** — those are correct. If either
reading fails to hold when you check it, stop and report; a unit claim is not something to adjust
by assumption.

- [ ] **Step 8: Run the tests and the whole server suite**

**First, enumerate the fixtures this task's conversions can reach, so the halt rule below is aimed
at a complete list rather than at a blind spot.** The four converted sites are inert on
square by construction (`SquareGrid::world_units_per_cell` is its own cell size), so the only
fixtures that can move are hex ones that author a light radius, a vision-mode range, or an
animation speed. Run and record:

```bash
cd /c/Dev/Shadowcat && git grep -n '"kind": "hex"' -- src/server
cd /c/Dev/Shadowcat && git grep -n "brightRadius\|dimRadius\|defaultRange\|speedCellsPerSec" -- src/server
```

The first search does not close the set: a fixture may take its grid kind as a PARAMETER rather
than a literal, and `move_exec::tests::scene_with_narrow_gap_and_wide_token` is a known instance —
an input to the search, not a substitute for it. Intersect the two lists, give every hex fixture
reaching a converted quantity its own line stating which quantity it reaches, and record explicitly
if the intersection is empty. An empty intersection is a result; an unstated one is a gap.

Then run: `cd src/server && cargo test`

RUN, OBSERVE, RECORD, including the doctest section. Report the outcome of the four new tests and
of every pre-existing test in `scene::lighting`, `scene::pathfinding`, `scene::move_exec`,
`scene::navmesh` and `ws::room`. A pre-existing test needing an ASSERTION change is not mechanical
fallout — stop and report, and say whether the fixture appears in the intersection above.

- [ ] **Step 9: Mutation checks — prove each converted site is load-bearing**

**Five** mutations, run and reverted independently. For each: restore the indexing scale at that one
use, run `cd src/server && cargo test --lib scene::tests::a_hex && cargo test --lib ws::room::tests::a_hex_move`,
record the observed failing test names and messages, revert, re-run, and confirm green plus a
byte-identical diff against the pre-mutation file.

1. **a.** `point_qualifies`' `dist_cells` divisor takes `cell` again — its forwarded
   `cell_illumination` argument left converted.
1. **b.** `point_qualifies`' forwarded `cell_illumination` argument takes `cell` again — its
   `dist_cells` divisor left converted.
2. `player_lit_mask`'s `dist_cells` divisor takes `cell` again.
3. `player_lit_mask`'s `cell_illumination` argument takes `cell` again.
4. `Room::execute_move`'s duration divides by `cell` again.

**Why (1) is two mutations and not one.** `point_qualifies` has two independent uses of the same
scalar, and mutating both at once proves only that at least one of them is covered — a
half-conversion, which is the likelier mistake, would still be reported as detected. The two halves
are separable in this fixture set by construction: the vision-range fixture is all-bright, so the
forwarded illumination argument cannot decide anything there, and the light-radius fixture gives the
token unlimited vision, so the range divisor cannot decide anything there. Each half therefore has
its own detector, and the report names which test caught which.

Each mutation must be observed to fail at least one test. **A mutation that leaves the suite green
means that site is unwired or uncovered** — that is the finding to report; stop rather than
proceeding. This is the check the Step 10 grep cannot make: the grep proves the NON-conversions
stayed put, which is the complement of proving the conversions took effect, not a substitute for
it.

- [ ] **Step 10: Prove the non-conversions were not converted**

Run and record:

```bash
cd /c/Dev/Shadowcat && git grep -n "cheby / cell\|total_len / cell" -- src/server
cd /c/Dev/Shadowcat && git grep -n "footprint_radius_cells.max(0.0) \* cell\|footprint_radius_cells \* cell" -- src/server
```

Every hit from the first search must still divide by `cell`; every hit from the second must still
multiply by `cell`. Report each explicitly as unconverted, with its reason. This step exists
because "we converted everything" is precisely the report shape that loosens a sampler or silently
changes what a token occupies.

- [ ] **Step 11: Run the full server gate**

Run: `cd src/server && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

- [ ] **Step 12: Commit**

```bash
git add src/server/src/scene/lighting.rs src/server/src/scene/mod.rs src/server/src/scene/move_exec.rs src/server/src/scene/pathfinding.rs src/server/src/scene/navmesh.rs src/server/src/ws/room.rs src/server/src/data/engine/token.rs
git commit -m "fix(scene): scale authored cell quantities by the grid step distance

Light radii, vision ranges and animation speed are authored in cells, so each
converts through the shape's per-cell world distance rather than its indexing
scale. The two subdivision densities and the token footprint radius keep the
indexing scale deliberately and now say so: the densities are finer and
fail-closed there, and the footprint's model is a square block whose rescaling
would change what a token occupies. The actor size struct's unit doc is
corrected to the grid units both the server footprint derivation and the client
box computation read it as.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>" -- src/server/src/scene/lighting.rs src/server/src/scene/mod.rs src/server/src/scene/move_exec.rs src/server/src/scene/pathfinding.rs src/server/src/scene/navmesh.rs src/server/src/ws/room.rs src/server/src/data/engine/token.rs
```

---

### Task 6b: a placed light reaches what it was authored to reach

**Numbered `6b` for the same reason as `5b`.** Found during Task 6, pre-existing, live on both grid
shapes, and correctly not fixed inline — the fix widens a secrecy surface and moves every lit-mask
fixture, which earns its own review cycle rather than a fold-in.

---

#### The defect

`SceneEcs::lighting_inputs_from` builds each placed light's occlusion polygon by raycasting against
`vision::bound_for(light.pos, light_walls, VISION_BOUND_MARGIN)`. That bound grows around the lamp
and around nearby `blocksLight` wall ENDPOINTS — and around nothing else. With no such wall near the
lamp, the polygon is a box of roughly `VISION_BOUND_MARGIN` on a side. `cell_illumination` then
requires a cell's centre to lie inside that polygon, so **the occlusion polygon silently becomes a
hard range cap**, independent of the light's authored radii.

Demonstrated by a fixture already in the tree: `scene_with_lit_player_token` authors a dim radius of
6 cells at cell size 100 — 600 world units of intended reach — and cannot light past roughly 100.

**This is the same defect shape the `ceil` proposal was rejected for in the extent work: a setting
whose stored value no longer determines its effect.** That argument was decisive there and is
decisive here, which is why this is a bug rather than a tuning question.

#### The cap is worse than a cap: it is a reach that varies with unrelated authoring

Read `bound_for` before designing the fix. It seeds an AABB at the viewpoint and grows it over
**every** wall endpoint in the slice it is handed, then pads by `margin` — and
`SceneEcs::lighting_inputs_from` hands it `light_walls`, the whole scene's `blocksLight` set, not a
neighbourhood of the lamp. Three consequences follow, and the third is the one that makes this
unfixable by tuning:

1. On a wall-less scene the AABB is the lamp ± `margin` — a hard cap at roughly `VISION_BOUND_MARGIN`
   regardless of authored radius.
2. Adding a `blocksLight` wall **anywhere** in the scene grows that AABB, so it raises the cap for
   **every** lamp in the scene, including lamps on the far side of it.
3. Therefore a lamp's maximum reach is a function of where unrelated walls were placed elsewhere.
   Author a wall in a distant room and a lamp that was clipped starts reaching further; delete it and
   the lamp dims — with nothing near the lamp having changed.

**Do not describe this in the report as "capped at ~100".** A fixed cap reads as a tuning constant
someone can raise. This is a reach with no stable value at all, which is why the remedy is to make
the authored radius determine the bound rather than to enlarge `VISION_BOUND_MARGIN`.

**Raising `VISION_BOUND_MARGIN` is therefore not an acceptable fix** — it moves the cap without
making the stored radius determine the effect, leaving the same defect at a larger number and
leaving consequence 3 fully intact. If the work reaches a point where that looks like the answer,
that is a signal the fix has been mis-scoped; say so rather than adjusting the constant.

**Why nothing caught it.** The direction is under-reveal, and no test asserts a light reaching as far
as it was told to. Every lit-mask fixture either sits inside the cap or has walls near enough to grow
the bound past it. A test suite can only catch a cap it tries to exceed.

#### Direction and blast radius, stated before the work rather than discovered during it

The fix makes lights reach further, which grows `SceneEcs::player_lit_mask` and therefore the movement
gate's `visible_cells`. That is the over-reveal direction on two secrecy-bearing consumers — but it is
**correct** reveal: the cells becoming lit are the ones a GM authored the radius to light. The bug was
that the authored value was being silently ignored, not that the correct value is too generous.

Expect existing lit-mask fixtures to move. **A fixture whose expected set changes is evidence the fix
works, not a regression** — but each one must be re-derived and stated, never adjusted until green.

---

**Files:**
- Modify: `src/server/src/scene/vision.rs` (`bound_for`, or a light-specific sibling)
- Modify: `src/server/src/scene/mod.rs` (`SceneEcs::lighting_inputs_from`, its tests)
- Modify: `src/server/src/scene/lighting.rs` if the reach belongs there

**Interfaces:**
- Consumes: `GridShape::world_units_per_cell` — the authored radii are in cells, so the reach disc is
  a second per-cell-distance conversion and must use that symbol, not the indexing scale.
- Produces: no new public surface expected. If the fix needs one, say so before building it.

---

- [ ] **Step 1: Reproduce the cap before changing anything, and record it verbatim**

Write a test that authors a light with a radius reaching well past `VISION_BOUND_MARGIN` on an
otherwise wall-less scene, and asserts a cell inside the authored radius is lit. Run it. **Record the
observed failure verbatim.** Do not predict it.

Do this on **both** shapes. The implementer that found this reports it live on square; verify that
independently rather than inheriting the claim, because a square-only or hex-only cap would mean a
different mechanism than the one described above.

- [ ] **Step 2: Union the reach into the bound, never replace it**

The recommended shape, which mirrors the union-never-replace discipline already established for the
scene extent: grow the bound to contain the lamp's reach disc as well as the wall endpoints, so the
polygon can only get larger and occlusion still decides what inside it is actually lit.

**Do not clamp, substitute, or special-case.** A bound that replaces rather than unions can shrink,
and a shrinking bound on this path is an under-reveal defect of exactly the kind being fixed.

The reach is `max(bright_radius, dim_radius) × world_units_per_cell`, since both are authored in
cells. Handle a non-finite or negative authored radius the way the extent guards do — refuse to a
value the consumer already rejects, rather than inventing a fallback.

- [ ] **Step 3: Verify occlusion still occludes**

The whole risk of growing this bound is that it stops being an occlusion polygon and becomes a disc.
Pin that it does not: a `blocksLight` wall between the lamp and a cell inside the authored radius must
still leave that cell unlit. Witness required — a mutation that drops the occlusion must fail this
test and no other.

- [ ] **Step 4: Re-derive every moved fixture**

Enumerate every test whose expected lit set or visible-cell set changes, one row each, with the old
set, the new set, and **why the new one is right** — derived from the authored radius and the cell
size, not read back from the run. A fixture adjusted until it passed is the failure mode here.

State the movement-gate consequence separately from the fog consequence. They are different things,
they ride on the same mask, and reporting only the fog half is how a gate change gets ratified as a
fog change.

- [ ] **Step 5: Full gate**

`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` from `src/server/`;
`node scripts/check-comment-refs.mjs` from the root; `pnpm -r test` if anything crosses the
generated-type boundary.

- [ ] **Step 6: Commit**

Conventional-commits, imperative, stating the constraint and the consequence. No task ids, round
numbers, dates, or process references.

---

### Task 6c: the client measures a travelled distance in the same cells the server does

**Numbered `6c` for the same reason as `5b`.** Found during Task 6, pre-existing, client-side, and
correctly not fixed inline — it crosses the language boundary and needs a parity pin, which earns its
own review cycle.

---

#### The defect: the per-step distance decision is FORKED across the server and the client

The server converts a travelled world distance into grid steps through
`GridShape::world_units_per_cell` — `cell` on square, `size * √3` on hex — and uses it to compute the
authoritative `MoveExecution::duration_ms` in `Room`'s move path, whose own doc states that contract.

`TokenAnimator`'s `startAnim` makes the same decision independently and differently:

```ts
const cells = total / this.cfg.cellSize;
// duration: (cells / this.cfg.speedCellsPerSec) * 1000
```

`AnimationConfig.cellSize` is fed by `TokenView.pushAnimConfig` from `TokenView.setCellSize`, which
`RenderEngine.setGrid` sets from `GridSpec.size` — the scene document's authored `engine.grid.size`.
On hex that field is the **outer radius (circumradius)**, and the client's own `Grid.axialToPixel`
confirms adjacent centres are `√3 · size` apart. So on a hex grid the client divides by a value
`√3` too small, computes `√3×` too many cells, and animates `√3×` too slowly.

**This is the forked-decision class — the defect this codebase produces most.** Two paths documented
to agree on "how far is one cell" disagree on an input nobody checked, namely the grid shape.

**It is live in production.** `TokenView.reconcile` calls `TokenAnimator.setTarget` for every token
document on every sync, and `setTarget` starts the `startAnim` tween. Any position change arriving as
a document update rather than as a move-stream broadcast animates through this path. Verify the
reachability yourself before building anything — if you conclude it is unreachable, stop and report
that, because it changes the task.

**Why nothing caught it.** `animateSamples` plays back a server-supplied `durationMs` verbatim and
never reads `cellSize`, so the server-driven route path is correct and masks the fork on exactly the
journeys anyone would test. Every existing animator test uses cell size 100 on an implicit square
grid, where `world_units_per_cell == size` and the two conventions coincide.

---

**Files:**
- Modify: `src/client/render/src/grid.ts` (`Grid`)
- Modify: `src/client/render/src/token-animator.ts` (`AnimationConfig`, `TokenAnimator`)
- Modify: `src/client/render/src/token-view.ts` (`TokenView.setCellSize`, `TokenView.pushAnimConfig`)
- Modify: `src/client/render/src/engine.ts` (`RenderEngine.setGrid`) if the plumbing changes shape
- Test: the sibling `.test.ts` of each modified module

**Interfaces:**
- Produces: a new public `Grid` method returning the world distance between adjacent cell centres.
  Name it to match the server's `world_units_per_cell` in the client's casing so the two are greppable
  as one concept.
- Consumes: `GridSpec.kind` — the animator currently receives a bare number and cannot know the shape,
  which is the structural reason this forked at all.

---

- [ ] **Step 1: Reproduce the fork before changing anything, and record it verbatim**

Write a test that animates a token one hex step on a hex grid and asserts the duration equals one cell
at the configured speed. Run it. **Record the observed failure verbatim** — do not predict it, and do
not compute the expected number by re-deriving the formula you are about to fix.

- [ ] **Step 2: Give `Grid` the per-step distance, and make the animator consume it**

Add the method to `Grid` — `size` for square, `size * Math.sqrt(3)` for hex — with a doc comment
stating the invariant (every axial neighbour is `√3 · size` away) and why it is NOT the indexing
scale.

Then **rename `AnimationConfig.cellSize`** to name the quantity it actually needs. The rename is the
point, not cosmetic: the present name is why a reader supplied the indexing scale in good faith. Its
doc comment currently says "Pixels per grid cell (grid.size)" — that sentence must become false and be
rewritten, not left standing beside a new name.

**Do not compute `√3` inside the animator.** One shape-aware symbol, consumed everywhere — a second
site that knows about hexes is the same fork again in a new place.

- [ ] **Step 3: Pin the parity across the language boundary**

The two implementations cannot share a symbol, so pin them with a test that fails if either side
changes. Assert the client's per-step distance for a hex grid of a stated size equals the value the
server's `GridShape::world_units_per_cell` produces for the same size, with the constant written once
and both conventions derived from it.

**Witness required, on both sides:** mutate the client method to return `size` and confirm the parity
test fails; revert by `diff`. A test that passes because both sides are wrong the same way proves
nothing.

- [ ] **Step 4: Enumerate every consumer of the renamed field**

Grep the client for the old name and adjudicate every hit with a row and a disposition, including hits
that are correct as they stand. A rename falsifies prose that never enters the diff, so include
comments, not just code. State the count.

- [ ] **Step 5: Full gate**

`pnpm -r test`, `pnpm -r typecheck`, `pnpm lint` from the root; `node scripts/check-comment-refs.mjs`.
Run `cargo test` from `src/server/` too if you touched anything the server reads.

- [ ] **Step 6: Commit**

Conventional-commits, imperative, stating the constraint and the consequence. No task ids, round
numbers, dates, or process references. End with the `Co-Authored-By: Claude Opus 5
<noreply@anthropic.com>` trailer.

---

### Task 6d: one route cost, one unit, whatever engine produced it

**Numbered `6d` for the same reason as `5b`.** Found during Task 6. Server-and-client, and the fix
changes a number a GM reads off the screen, so it earns its own review cycle.

---

#### The defect: the wire field's stated unit is true of one producer and false of the other

`ServerMsg::PathResult`'s own doc comment states the contract for the whole field:

> total cost in cells (client multiplies `grid.distance.perCell`)

The grid A* router honours it — `PathOutcome.cost` is documented "Total weighted cost in cells", and
its tests pin one cell per orthogonal step. **The `Continuous` movement model does not.**
`SceneEcs::pathfind` deliberately rescales:

```rust
// `find` reports cost in CELLS; the continuous engine reports SCENE UNITS
// (parity with the polyanya path, which measures Euclidean length).
cost: weighted.cost * grid_shape.world_units_per_cell(),
```

and the pure-navmesh branch returns a Euclidean length in world units directly. `conn` forwards
`outcome.cost` to the wire unchanged, so the same field carries cells on one model and world units on
the other, under a doc comment that promises cells.

The client cannot tell. `makeMeasureTool`'s route branch always computes
`Math.round(result.cost * scene.perCell)` with `perCell` from `grid.distance.perCell`. On a
`Continuous` scene it multiplies an already-world-unit cost by the game-distance scale a second time.
At the common authoring of `size: 100`, `perCell: 5`, a five-cell move reports **2500 ft where it
should read 25 ft**.

**This is the forked-decision class again, and in its purest form:** two producers of one wire field
disagree about its unit, and the consumer has nothing to branch on. `GridStepped` is the default
movement model, which is why this has gone unseen.

**Why nothing caught it.** The one client test covering a continuous-movement scene stubs `pathfind`
with a fixed `cost: 2`, so it never exercises the server's unit switch, and it asserts nothing about
the budget label.

#### The design fork, and its ruling

Either add a unit discriminant to `PathResult` so the client branches, or make both engines report the
same unit.

**Ruled: one unit on the wire — cells — and convert at the boundary, not at the consumer.** The
never-fork rule is explicit that agreement must be structural rather than verified: a discriminant is
a second decision the client can get wrong, and it preserves two units where the field's contract
declares one. A fractional cell count is a perfectly meaningful continuous result. The internal
computation may keep working in world units for navmesh parity — that is a good reason for the
internal unit and no reason at all for the wire unit. Do not re-open this; implement it.

---

**Files:**
- Modify: `src/server/src/scene/mod.rs` (`SceneEcs::pathfind`, both branches)
- Modify: `src/server/src/scene/navmesh.rs` if the conversion belongs at its boundary
- Modify: `src/server/src/ws/protocol.rs` (`PathResult`'s doc, if its wording needs sharpening)
- Test: the server pathfind tests, and `src/modules/scene-tools/src/measure-tool.test.ts`

**Interfaces:**
- Produces: no wire SHAPE change — `cost` stays an `f64`. Its VALUE changes on continuous scenes,
  which is the point. If you find yourself adding a field, stop and report: that is the discriminant
  this task ruled against.

---

- [ ] **Step 1: Reproduce the double-multiply end to end, and record it verbatim**

Pin the server side first: a continuous-movement scene whose route covers a known number of cells must
report that number as `cost`. Run it, **record the observed failure verbatim** — it should show the
world-unit value, and that number is your evidence, not a prediction.

Then pin the client side with a test that does NOT stub the cost: feed `makeMeasureTool` a cost in the
unit the server will now send and assert the label. The existing continuous test's fixed `cost: 2`
stub is exactly why this survived; do not extend that stub, replace the coverage.

- [ ] **Step 1b: The same tool labels the same quantity two different ways — fix both branches**

`makeMeasureTool` has a second, non-route branch, taken whenever the user has no token selected, more
than one selected, or no pathfind available. It labels with
`String(ctx.scene.gridDistance(anchor, p))` — a raw whole-cell count with **no `perCell` multiply and
no unit suffix at all** — while the route branch three hundred lines earlier renders
`` `${budget} ${scene.unit}` ``.

So measuring five cells shows `25 ft` with a token selected and `5` without one. **This is the same
forked decision as the task's main subject, one level down**: two paths inside one tool disagreeing
about how a distance is expressed, and the fallback is the one a player without a selected token
actually hits.

Route the label through ONE function that takes a cell count and produces the labelled string, and
have both branches call it. Do not fix the fallback by copying the route branch's expression — a
second copy of the formula is the fork re-created, and it is what let these two drift apart.

The `⚠` arrest marker belongs to the route branch only; keep that distinction, and say in your report
how you kept it without giving the shared function a caller-specific flag.

- [ ] **Step 2: Convert once, at the boundary, in the direction that preserves the contract**

Make both branches of `SceneEcs::pathfind` yield a cost in cells. Delete the multiply; convert the
navmesh branch's Euclidean length by dividing by the shape's per-cell world distance.

**Use `GridShape::world_units_per_cell`** — the authored-distance conversion, since a route length is
a distance a GM authors expectations about. It is emphatically not the footprint radius conversion,
which the surrounding code is careful to keep distinct; do not follow that precedent here.

Guard the divisor the way the extent work guards its own: a non-finite or non-positive per-cell
distance must refuse rather than produce an infinity the client renders as a label.

- [ ] **Step 3: Make the contract un-forkable rather than merely correct**

Right now the only thing binding the two engines to one unit is a doc comment. Add a test that
exercises **both** movement models through the same assertion — same scene geometry, same expected
cell count — so a future change to either branch fails. Witness required: mutate one branch's
conversion and confirm the shared test fails; revert by `diff`.

- [ ] **Step 4: Re-derive every server test that asserts a continuous route cost**

**This step is larger than it looks and is the reason this task is not a two-line change.** A scan of
the scene tests finds numerous assertions on `PathOutcome.cost` at magnitudes only world units
explain — comparisons against `900.0`, `400.0`, `200.0`, and against derived lengths like a straight-
line `dist_to_goal` — alongside grid-model assertions at `2.0` and `0.0`. Every one of the former
moves when the unit changes.

Enumerate them, one row each: the test, its old expected value, its new expected value, and **why the
new one is right, derived from the cell count and the shape's per-cell distance** — never read back
from the run. A fixture adjusted until it passed is the failure mode here, and it is especially
tempting on this task because dividing by the cell size makes the "right" number obvious enough to
back-fill without thinking.

Note the tolerances too: several of these assert within an absolute epsilon (`< 5.0`, `< 3.0`) chosen
for world-unit magnitudes. Divided into cells those epsilons become enormous relative to the value,
and an assertion that cannot fail is worse than no assertion. Re-scale each tolerance and say so.

- [ ] **Step 4b: State the GM-visible consequence**

The measure tool's label changes on every continuous-movement scene. Say so plainly in the report,
with the before and after for one concrete authoring, derived from the cell count and `perCell` —
never read back from the run.

**Confirmed before this task was written: no gate consumes this value.** The sole production consumer
is the measure tool's label; there is no per-turn movement budget anywhere on the server. So this is a
display correction with no authz or secrecy dimension — but if you find a second consumer while
working, that conclusion changes and you must stop and report it rather than proceeding.

- [ ] **Step 5: Full gate**

`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` from `src/server/`;
`pnpm -r test` from the root; `node scripts/check-comment-refs.mjs`.

- [ ] **Step 6: Commit**

Conventional-commits, imperative, stating the constraint and the consequence. No task ids, round
numbers, dates, or process references. End with the `Co-Authored-By: Claude Opus 5
<noreply@anthropic.com>` trailer.

---

### Task 6e: the lighting and fog overlays land on the cells they describe

**Numbered `6e` for the same reason as `5b`.** Found during Task 6, client-side, and it is a
rendering-correctness defect on every hex scene.

---

#### The defect: axial indices painted at square positions

On a hex scene the server sends lit cells and explored cells as **axial `(q, r)`** — they come from
`HexGrid`'s `GridShape` implementation, whose `axial_to_pixel` places a cell at
`x = size·(√3q + √3/2·r)`, `y = size·1.5·r`.

Two client paths paint those indices as if they were square row/column:

- `PixiBackend.setLighting` — `const x = c.i * cellSize, y = c.j * cellSize`, then fills an
  axis-aligned `rect` of `cellSize` on a side.
- `cellsToRects`, which rasterizes the fog explored-memory layer — the identical
  `x = cells[k] * size, y = cells[k+1] * size`, emitting a square rect polygon per cell.

Neither consults the grid shape. Neither has one to consult: `LightingFrame` and the explored payload
carry a `cell` size and no kind.

**The correct math is already in the same package, on the same object.** `RenderEngine` holds a
`Grid` built from the scene's `GridSpec`, whose private `axialToPixel` is the exact mirror of the
server's — and it is used only for grid lines, snapping and measurement. So on a hex scene the grid
lines are right, the cursor snaps right, the currently-visible fog polygon is right (the server sends
raw raycast vertices, so no cell math is possible), and **the lighting overlay and the explored fog
are drawn at skewed square positions over correctly-drawn hexes.**

**Why nothing caught it.** These two paths take cell indices; every other fog path takes polygons.
The bug needs a hex scene AND an overlay to be visible at once, and the overlay tests all use square
fixtures where the two conventions coincide.

#### The design fork, and its ruling

**Ruled: one shape-aware symbol on `Grid`, consumed by both paths — not shape branches at the paint
sites.** This is the same ruling as Task 6c and for the same reason: a second site that knows how
hexes are laid out is the forked decision reappearing. `Grid` already owns the axial math privately;
promote what these paths need onto its public surface and have them ask for it.

A hex cell is not a rectangle, so the paint sites must move from `rect` to a filled polygon. Take the
corner geometry from `Grid` as well — `hexLines` already computes hex outlines, and a second corner
formula elsewhere is the same defect again.

---

**Files:**
- Modify: `src/client/render/src/grid.ts` (`Grid` — promote cell-centre and cell-corner geometry)
- Modify: `src/client/render/src/pixi-backend.ts` (`PixiBackend.setLighting`)
- Modify: `src/client/render/src/engine.ts` (`cellsToRects`, `RenderEngine.toVisibility`,
  `RenderEngine.toLighting`)
- Modify: `src/client/render/src/types.ts` / `lighting.ts` — the frame types must carry the shape
- Test: the sibling `.test.ts` of each

**Interfaces:**
- Consumes: `GridSpec.kind`, which these paths currently cannot see. Threading it is most of the work.
- Produces: public `Grid` geometry. **The convention is already set and is named here rather than
  pointed at**: `Grid.worldUnitsPerCell()` exists, returning `size` on square and `size * √3` on hex,
  chosen as the exact camelCase mirror of the server's `GridShape::world_units_per_cell` so the two
  are greppable as one concept across the language boundary. Name what you add the same way — a
  camelCase mirror of the server symbol for the same quantity, where one exists.
  **Do not add a second per-step-distance method**; that one is done. What is missing is cell
  POSITION and cell CORNER geometry, which `Grid` currently owns only privately.

---

- [ ] **Step 1: Reproduce the misalignment before changing anything**

Write a test on a hex fixture asserting the lighting overlay's painted position for a stated axial
cell equals that cell's true centre. Run it, **record the observed failure verbatim** — the gap
between the square and axial positions is the measurement, not a prediction.

Do the same for the explored-fog rasterization. Two paths, two reproductions; a single test covering
both would let one stay broken.

- [ ] **Step 2: Thread the shape to the paint sites**

The frame types carry `cell` and no kind, which is the structural cause. Add the shape, and prefer
carrying the resolved geometry over carrying a `kind` the paint site then branches on — a `kind`
field invites exactly the branch this task ruled against.

- [ ] **Step 3: Paint the cell, not a rectangle**

Both sites emit filled polygons from `Grid`'s corner geometry. Square must keep producing an identical
result to today: pin that with a test, so this change is provably shape-neutral on square scenes.

- [ ] **Step 4: Enumerate every remaining cell-index consumer in the render layer**

Both known sites are named above, and named sites have twice become the whole worklist on this branch.
Sweep `src/client/render/` for every conversion from a cell index to a position, give each a row and a
disposition — including the ones that are correct — and state the count. If you find a third, it is in
scope.

- [ ] **Step 5: Full gate**

`pnpm -r test`, `pnpm -r typecheck`, `pnpm lint` from the root; `node scripts/check-comment-refs.mjs`.
If any e2e fixture covers a hex scene's overlays, run it — this is a visual defect and the e2e layer
is where a visual regression would show.

- [ ] **Step 6: Commit**

Conventional-commits, imperative, stating the constraint and the consequence. No task ids, round
numbers, dates, or process references. End with the `Co-Authored-By: Claude Opus 5
<noreply@anthropic.com>` trailer.

---

### Task 6f: a vision mode's authored default range stops being inert

**Numbered `6f` for the same reason as `5b`.** Surfaced by Task 6's fix round, which corrected a doc
comment that had cited this field as a live member of the converting class. It is not live at all.

---

#### The defect: a GM-facing control whose stored value has no effect

`VisionMode::default_range` is written at exactly three sites, all inside
`SceneEcs::resolved_vision_modes` — one copying the authored value, two seeding the built-in `normal`
and `darkvision` modes — and is **read by nothing**. `SceneEcs::token_vision_floors` looks a mode up
solely for its `illumination_floor` and `render_hint`, and takes the range from
`VisionAssignment::range` unconditionally. That field is a plain `f64`, not an `Option`, so there is
structurally nowhere for a fallback to attach.

Meanwhile `GameSettingsPanel` renders a GM-only number input that patches
`/engine/modes/<id>/defaultRange` on the vision-modes document. It persists, it round-trips, it
validates, and it changes nothing on the table. The client's `SEED_VISION_MODES` seeds `darkvision`
with a default range of 12, which likewise reaches no mask.

**This is the same defect shape as the light-reach cap in Task 6b and as the rejected `ceil` proposal:
a setting whose stored value no longer determines its effect.** Two independent code comments in the
tree already state the field is dead, which makes this a documented-and-tolerated inertness rather
than an unknown.

#### The design fork, and its ruling

Two shapes are available: delete the field and its control, or make the assignment's range optional so
an omitted range inherits the mode's default.

**Ruled: make it live.** The question "what is the best long-term shape in keeping with our plans and
goals?" answers this one. A registry whose entries carry defaults that per-instance assignments may
override is the modular shape this platform is built around, and it is what the existing GM control
and the seeded `darkvision: 12` already promise. Deleting the field would remove an advertised
capability and force every assignment to restate a value its mode already defines. Do not re-open
this; implement it.

---

**Files:**
- Modify: `src/server/src/data/engine/token.rs` (`VisionAssignment`)
- Modify: `src/server/src/scene/mod.rs` (`SceneEcs::token_vision_floors`)
- Modify: `src/types/generated/**` — by REGENERATION, never by hand
- Modify: `src/client/core/src/actor.ts` and any client reader of a vision assignment's range
- Test: `src/server/src/scene/mod.rs` tests, plus the client tests covering those readers

**Interfaces:**
- Produces: `VisionAssignment::range` becomes optional on the wire.

**There is NO Zod schema to mirror for this type, and that is verified rather than assumed.**
`WireDocument.engine` is declared `z.unknown()` — the engine band carries no client-side structural
validation at all. `VisionAssignment` reaches the client purely as a ts-rs TYPE re-exported through
`scene-docs`, so regeneration propagates the change on its own and there is no hand-written schema
that can silently fail to follow.

**What that changes about your risk profile:** the usual "a typecheck cannot see a dropped Zod field"
warning does not apply here, but the opposite exposure does — because nothing validates this band at
runtime, a client reading `.range` as a bare number gets `undefined` at runtime with no schema error
to announce it. **Enumerate every client site that reads a vision assignment's range and adjudicate
each**, rather than relying on the typecheck to find them; a site that destructures or arithmetics
the value is the one that breaks quietly.

---

- [ ] **Step 1: Pin the current behaviour before changing it**

Write a test asserting that a token whose assignment omits a range gets the mode's default. Run it,
**record the failure verbatim.** Confirm by reading, not by assuming, that no fallback exists today.

- [ ] **Step 2: Make the range optional and resolve it against the mode**

`VisionAssignment::range` becomes `Option<f64>`; `token_vision_floors` resolves
`a.range.unwrap_or(vm.default_range)`. Both quantities are authored in CELLS — confirm that against
the surrounding conversion work before relying on it, and make sure the resolved value flows through
the same per-cell conversion the assignment's own range does today. A default that skips the
conversion the override receives is this phase's whole subject repeated.

**Serde note:** a missing key on an `Option` is never an error, so verify the omitted case actually
deserializes to `None` rather than being rejected by `deny_unknown_fields` or a required-field guard.

- [ ] **Step 3: Regenerate and mirror**

Regenerate the ts-rs bindings — never hand-edit them — and mirror the change in the client Zod schema.
Run `pnpm -r test`, not just a typecheck: a dropped Zod field is a runtime failure a typecheck cannot
see.

- [ ] **Step 4: Make the GM control's effect observable**

The control already writes the field. Verify end-to-end that authoring a mode default now changes a
token's vision floor when that token's assignment omits a range, and state which test proves it.

- [ ] **Step 5: Full gate**

`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` from `src/server/`;
`pnpm -r test` and `pnpm -r typecheck` from the root; `node scripts/check-comment-refs.mjs`.

Both dead-field comments — on the vision-range test helper and on the grid shape's conversion doc —
become false the moment this lands. Find them and rewrite them; a rename or a revival falsifies prose
that never enters the diff.

- [ ] **Step 6: Commit**

Conventional-commits, imperative, stating the constraint and the consequence. No task ids, round
numbers, dates, or process references. End with the `Co-Authored-By: Claude Opus 5
<noreply@anthropic.com>` trailer.

---

### Task 6g: a token occupies the hexes it is authored to occupy

**Numbered `6g` for the same reason as `5b`.** The largest of this group and the only one carrying an
owner ruling on semantics rather than a mechanical correction. Dispatch it LAST of the `6*` group —
it consumes the shape-aware `Grid` surface Tasks 6c and 6e establish.

---

#### The defect, measured rather than inferred

`resolveTokenBox` sizes a token's drawn footprint as `actor.size.w * cell` by
`actor.size.h * cell`, where `cell` is `sceneCellSize` — the scene's authored `grid.size`. On hex
that field is the cell's **circumradius**. Separately, `footprintRadius` reduces the same authored
size to a bounding-disc radius in grid units (`hypot(w,h)/2` for a square, `max(w,h)/2` for a circle),
which the server multiplies by the same `cell` to get `r_scene` for its collision checks.

For a 1×1 token on a hex grid of circumradius `size`:

| Quantity | Today | The hex it sits in |
|---|---|---|
| Drawn box | `size` × `size` | spans `√3·size` ≈ `1.73·size` wide, `2·size` tall |
| Collision radius | `0.707·size` | inradius `0.866·size`, circumradius `size` |

**Both are undersized, and by different factors** — so a token under-fills its hex visually while
also colliding as something smaller than the hex, and the two errors do not even agree with each
other. Gaps a hex would block stay passable.

`resolveTokenBox` is read by three separate concerns — the rendered box, `topTokenAt`'s hit test, and
`drawSelection`'s ring — so all three inherit it. Those are consumers, not independent defects; fixing
the source fixes them.

**The remedy is NOT a Role-A-to-Role-B substitution, and this is the trap.** Multiplying by
`world_units_per_cell` instead of `cell` yields a `√3·size` SQUARE: correct width, wrong height,
because a hex's height (`2·size`) and width (`√3·size`) are not in the same ratio. Any fix that
substitutes one scalar for another is wrong before it starts.

#### The owner's ruling — both halves, settled, do not re-open

1. **A token's authored `size` counts HEXES.** A 1×1 token fills the hex it occupies; an N-cell token
   spans N hexes. Its drawn geometry and its collision geometry both derive from the hex's own
   dimensions, never from a square approximation.
2. **One definition, both sides derive from it.** The client's drawn box and the server's collision
   footprint must come from a single resolved geometry rather than two formulas kept in agreement by
   review. This is the never-fork rule applied to the exact shape that produced the defect.

**Ruled by me, from the existing documented convention rather than by asking again: the collision
disc is the CIRCUMSCRIBING radius (`size` for one hex, i.e. `1.0` in cell units), not the inradius.**
`footprintRadius`'s own doc already states the convention — *"Conservative enclosure: a square uses
its half-diagonal, a circle its radius"* — and conservative enclosure of a hex is its circumradius.
It over-blocks slightly rather than under-blocking, which is the fail-closed direction for a movement
gate. Keep that sentence true by extending it, not by contradicting it.

#### Direction and blast radius, stated before the work

Tokens get bigger, both visually and in collision. Narrow gaps that a token previously slipped
through will now refuse. **That is the correction, not a regression** — but every movement fixture
whose route threads a gap may move, and each must be re-derived from the hex geometry and stated,
never adjusted until green.

---

**Files:**
- Modify: `src/client/core/src/actor.ts` (`resolveTokenBox`, `sceneCellSize`, `footprintRadius`)
- Modify: `src/client/core/src/scene-docs.ts` (`buildTokenFromActor`'s fallback)
- Modify: `src/modules/scene-tools/src/controller.svelte.ts` (`makePlaceTool`, both branches)
- Modify: `src/server/src/scene/pathfinding.rs` (`footprint_cells`, `cell_enterable`) and its callers
- Modify: the wire type carrying the resolved footprint, plus `src/types/generated/**` BY
  REGENERATION
- Test: the sibling tests of each, plus `src/modules/scene-tools/src/hit-test.ts`'s

**Interfaces:**
- Produces: a resolved footprint geometry carried on the wire rather than recomputed per side. State
  its shape in your report BEFORE building the consumers, since both languages bind to it.
- Consumes: the shape-aware `Grid` surface from Tasks 6c and 6e. If a hex corner or extent helper you
  need already exists there, use it; a second corner formula is this task's own defect re-created.

---

- [ ] **Step 1: Pin today's geometry before changing it, on both sides**

Write two failing tests: a client test asserting a 1×1 token's drawn box equals the hex's bounding
box, and a server test asserting a 1×1 token's collision disc equals the hex's circumradius. Run
both. **Record the observed failures verbatim** — the two wrong values ARE the measurement in the
table above, and I want them confirmed by a run rather than carried from my prose.

- [ ] **Step 2: Define the footprint once**

Build the single resolved geometry: given a token's shape, its authored size, and the scene's grid
shape, produce the drawn extent and the collision radius. **Square must come out byte-identical to
today** — pin that with a test before touching hex, so the change is provably hex-only.

- [ ] **Step 3: Carry it, do not recompute it**

The client renders what the resolved geometry says; the server collides with the same. Neither side
re-derives from `grid.size`. **If you find yourself writing a second expression that multiplies an
authored size by a grid scalar, stop** — that expression is the defect, wherever it appears.

Delete `sceneCellSize` if nothing legitimate still needs it. A helper whose only purpose was the
wrong conversion should not survive the fix as dead code.

- [ ] **Step 4: Witness the anti-fork property**

A test must fail if either side stops deriving from the shared definition. Mutate the client to size
from `grid.size` again and confirm it fails; revert by `diff`. Then mutate the server the same way and
confirm it fails too. **Two mutations, two observed failures** — a test that only catches one side
leaves the fork half-open.

- [ ] **Step 5: Re-derive every moved movement fixture**

Enumerate every test whose route, arrest point, or reachable set changes, one row each: old value,
new value, and why the new one is right, derived from the hex geometry. State separately whether any
moved fixture belongs to a SECRECY-bearing path (the movement gate, `visible_cells`) versus a
convenience path — they ride together and reporting only the convenience half is how a gate change
gets ratified as a rendering change.

- [ ] **Step 6: Full gate**

`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` from `src/server/`;
`pnpm -r test` and `pnpm -r typecheck` from the root; `node scripts/check-comment-refs.mjs`. Run the
e2e hex movement spec — this changes what fits through a gap, which is what that spec exists to catch.

- [ ] **Step 7: Commit**

Conventional-commits, imperative, stating the constraint and the consequence. No task ids, round
numbers, dates, or process references. End with the `Co-Authored-By: Claude Opus 5
<noreply@anthropic.com>` trailer.

---

### Task 7: PW3 — exercise the hex + continuous fog clip through the real dispatch, as a non-GM

**Ledger id:** PW3.

**Files:**
- Test: `src/server/src/scene/mod.rs` (`mod tests`)

**Interfaces:**
- Consumes: Task 4's hex footprint predicate and Tasks 5 and 6's conversions — the scene under
  test is hex, so all three are live in the path being exercised. This test is also the only
  end-to-end coverage of `clip_to_visible_mask`'s footprint predicate on hex, which is where
  Task 4's under-inclusion was reachable.
- Produces: no production symbols. Test-only.

**Why the existing coverage does not count.** The module's one hex + continuous integration test
routes with `RouteRequester { is_gm: true, .. }`. `pathfind` sets `mask = None` for a GM, and
`clip_to_visible_mask` returns before its per-sample check when `mask` is `None` and `walls` is
empty. So the hex + continuous branch's fog clip has no coverage through the dispatch path under a
real mask — only its square-grid siblings have.

- [ ] **Step 1: Read the fixtures this test must reuse**

Read, in `src/server/src/scene/mod.rs`'s test module:
- `hex_continuous_scene_docs` and `continuous_world_settings` — the hex + continuous scene shape;
- `hex_open_scene` (as re-derived in Task 5) — how a hex scene with a real owned token and a real
  mask is assembled, and the rectangle its bounds now produce;
- `visible_cells_hex_excludes_cell_whose_center_is_outside_the_mask` — how a non-GM hex mask is
  asserted.

The new test composes those fixtures rather than inventing a third; record in the report which
helpers you reused and any that had to be extended.

- [ ] **Step 2: Write the test**

```rust
    /// A non-GM route on a hex scene running the continuous engine is truncated at the edge of
    /// the requester's own visibility mask.
    ///
    /// Discrimination: the assertion is that the returned route's far end lies inside the mask
    /// while the requested destination lies outside it. It fails if `clip_to_visible_mask` is
    /// skipped for this dispatch, if the mask is built with square indices while the clip reads
    /// hex axial ones (the returned cell would not be a mask member), or if the requester is
    /// resolved as unrestricted. It cannot pass vacuously: the fixture guards below assert the
    /// destination is genuinely outside the mask and the start genuinely inside it, so an
    /// all-visible or empty mask fails a guard rather than the assertion.
    #[test]
    fn non_gm_hex_continuous_route_is_clipped_to_the_requesters_mask() {
        let g = grid_shape::HexGrid { size: 50.0 };
        // ... assemble a hex + continuous scene with an owned token for `user`, a bounded vision
        // range or lighting that leaves a reachable-but-unseen region beyond it, and
        // `movementRestriction: "visible"` ...
        let mask = ecs.visible_cells(user, scene, false);
        let start_cell = g.cell_of(start);
        let far_cell = g.cell_of(destination);
        assert!(mask.contains(&start_cell), "fixture: the start is visible");
        assert!(!mask.contains(&far_cell), "fixture: the destination is NOT visible");
        assert!(!mask.is_empty(), "fixture: the mask is non-empty, so the clip has work to do");

        let out = ecs
            .pathfind(
                RouteRequester { user, is_gm: false, explored: None },
                scene,
                start,
                &[destination],
                0.1,
            )
            .expect("a route into the visible region exists");
        let last = *out.path.last().expect("a non-empty route");
        assert_ne!(
            g.cell_of(last),
            far_cell,
            "the route must not reach a hex outside the requester's mask"
        );
        assert!(
            mask.contains(&g.cell_of(last)),
            "the route's far end must be a hex the requester can see, got {last:?}"
        );
    }
```

Fill in the fixture assembly from the helpers read in Step 1 — the snippet marks the one region
that must be written against the real helpers rather than guessed. Any bounded vision range the
fixture authors must sit a clear half-cell off the hex distances it separates, for the reason
Task 6 Step 3 states. **The fixture guards are not optional**: without them a mask that is empty
or all-visible would let the assertions pass while proving nothing.

- [ ] **Step 3: Run the test**

Run: `cd src/server && cargo test --lib scene::tests::non_gm_hex_continuous`

RUN, OBSERVE, RECORD. If the route is `Err(Unreachable)` rather than clipped, that is a real
result and not a fixture problem to paper over — record it, then determine which of the three
fail-closed paths produced it (`navmesh_for` returning `None`, `navmesh_find` finding no leg, or
the clip reducing the path below two points with a non-trivial raw route) and report which.

- [ ] **Step 4: Prove the test discriminates**

Temporarily make `clip_to_visible_mask` return its `outcome` argument unchanged at the top of the
function, run `cd src/server && cargo test --lib scene::tests::non_gm_hex_continuous`, record the
observed result, revert, re-run, and confirm green plus a byte-identical diff against the
pre-mutation file.

If the test stays green under that mutation, it is not exercising the clip and must be rewritten
before it is committed — that is the finding to report.

- [ ] **Step 5: Run the full server gate**

Run: `cd src/server && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

- [ ] **Step 6: Commit**

```bash
git add src/server/src/scene/mod.rs
git commit -m "test(scene): clip a non-GM hex continuous route against a real mask

The hex continuous dispatch is only routed as a GM elsewhere, where the mask is
absent and the fog clip returns early, so the branch that truncates an
any-angle hex route at the edge of a requester's visibility has no coverage
through the dispatch path — including the footprint predicate the clip applies
per sample.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>" -- src/server/src/scene/mod.rs
```

---

### Task 8: PW4 — verify the edge-projected environment light against source, and correct the record

**Ledger id:** PW4.

**This item's description in the spec is contradicted by the code.** The spec carries PW4 as
"environment light is flat ambient rather than edge-projected… the specified behaviour is
buildable" — i.e. as unbuilt work now unblocked. It is built. `lighting::env_light_polys` walks
the scene perimeter, casts a `vision::visibility_polygon` from each sample against the
`blocksLight` wall set, and `env_lit` admits the environment ambient at a cell only when that
cell lies inside some boundary sample's polygon; `lighting_inputs_from` wires it, and
`cell_illumination` consumes it as `env_polys`. `docs/POST_WORK_FINDINGS.md` still carries the
superseded entry stating the opposite.

This task **verifies rather than trusts** that reading, then corrects the record. The residual
work — the same conversion defect inside `env_light_polys` — is covered by Task 5, so this task
changes no scene code.

**Files:**
- Modify: `docs/POST_WORK_FINDINGS.md`

**Interfaces:**
- Consumes: Task 5's `env_light_polys` conversion (the verification runs against the post-Task-5
  code).
- Produces: no symbols.

- [ ] **Step 1: Verify the four properties the entry claims are missing**

Record each against real code, citing the symbols:

1. **Edge projection exists** — `env_light_polys` samples the perimeter of the scene rectangle
   (`perimeter_point`) rather than applying a scene-wide constant.
2. **It is occludable** — each sample's reachability is a `vision::visibility_polygon` cast
   against the `blocksLight` wall set, and `env_lit` requires the cell centre to lie inside one
   of them.
3. **It is wired** — `lighting_inputs_from` fills `LightingInputs.env_polys`, and
   `cell_illumination` gates the environment base on `env_lit(env_polys, center)`.
4. **It is exercised** — name the tests that cover it, from
   `git grep -n "env_light_polys\|env_lit\|open_env_lit_scene\|env_lit_scene_with_room" -- src/server`.

If any of the four does not hold, **stop and report**: this task's premise is then wrong and PW4
is real work, which is a scope question for the human and not something to build inline.

- [ ] **Step 1b: Answer whether the raycast margin can disclose anything outside the authored block**

`env_light_polys` builds its raycast bound as the scene envelope grown by a margin, so the
environment reaches cells OUTSIDE the authored block. Square has always had this; hex reaches parity
with it once the envelope carries a real minimum, and the change is measurable — one extra ring of
lit cells on the origin side of a hex scene.

Parity is not the question. The question the parity change surfaced, which nobody has answered, is:

> **Limb 1 — what a lit cell outside the block reveals.** Can a cell outside the authored block,
> being lit and reachable, expose the existence or the position of a document a GM placed outside
> the bounds?
>
> **Limb 2 — what a player can step onto there.** `visible_cells` and `visible_cells_cached` reach
> the same environment-light path, so the movement gate's mask carries the same ring. Under
> `movementRestriction: "visible"` a non-GM can enter it. Is a play area a player can leave by one
> ring the intended contract, on either shape?

Answer limb 2 as well as limb 1. The review that surfaced this found the fog consequence stated and
the gate consequence unstated, which is how a gate change gets ratified as a fog change.

Note what IS already established, so it is not re-derived: `gm_only` walls and regions are filtered
per recipient and never delivered, so no secret geometry reaches the ring. What is NOT established is
the non-secret case — a token, drawing or template a GM placed there is delivered and hidden only by
fog, so lighting the ring renders it. That half is derived from the fog model rather than tested, and
no server-side test can assert it, which is itself worth stating in the answer.

Answer it **from the code**, tracing what a client actually receives: whether a cell outside the
block can enter a player visible-cell mask, what an egressed mask discloses about a cell nothing
authored occupies, and whether any document outside the bounds becomes reachable through that path.
State the answer either way with the symbols that establish it.

**If the answer is that it can disclose**, stop and report. That is a live secrecy defect predating
this phase and applying equally to square, and its fix is a scope question rather than something to
build inside a documentation task. **If it cannot**, say so with the mechanism that prevents it, so
the margin stops being an open question the next reader has to re-derive.

- [ ] **Step 2: Correct the record**

In `docs/POST_WORK_FINDINGS.md`, replace the environment-light entry's `Status:` with a resolved
status stating what the code does now, citing symbols only — no milestone ids, no dates, no
history narration, and no reference to what the entry previously said. State:

- environment light is projected from the scene boundary and occluded by `blocksLight` walls,
  through `env_light_polys` / `env_lit` / `cell_illumination`;
- the scene rectangle it projects from is the shape-derived world extent
  (`GridShape::world_extent`), so the perimeter matches the authored play area on both grid kinds,
  and its sample count is a discretization density that converges to the true reachable set from
  below;
- the tests that hold it, by name.

Do not delete the entry: `POST_WORK_FINDINGS.md` is a living review record, and a resolved entry
with its evidence is what stops the same claim being re-derived.

- [ ] **Step 3: Commit**

Documentation-only; no server gate applies.

```bash
git add docs/POST_WORK_FINDINGS.md
git commit -m "docs(findings): record environment light as boundary-projected and occluded

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>" -- docs/POST_WORK_FINDINGS.md
```

---

### Task 9: PW5 + NEW-7 + NEW-11 — a config registry that will not decode is reported, not silently replaced

**Ledger ids:** PW5, NEW-7, NEW-11.

**This item's mechanism has moved since it was written, and the plan follows the code.** PW5 says
"a vision-mode entry missing its illumination floor is silently dropped with no diagnostic". That
per-entry drop no longer exists: `eng::VisionMode.illumination_floor` is a required
`Deserialize` field on a `deny_unknown_fields` struct, `validate_engine` round-trips
`VisionModesEngine` at ingress, and `resolved_vision_modes` inserts every decoded entry
unconditionally. A mode entry missing its floor is rejected at write time.

What survives is the same defect one layer up, and it is worse than the original: because the
decode is now whole-document, a single malformed entry makes `engine_as_cached` return `None`,
and `resolved_vision_modes` treats that identically to "no document at all" — silently seeding
the built-in registry over the GM's authored one. Its own inline comment states the opposite
intent. `resolved_bands` has the identical shape (NEW-7).

**One reviewer finding about `resolved_bands` is mistaken, and the plan does not implement it.**
The finding states that `resolved_bands`' doc claims a fail-closed three-band default while the
body yields an empty list. Traced: the body's `unwrap_or_default()` produces an empty `Vec<Band>`,
which it then passes to `lighting::sorted_bands`, whose first branch is
`if bands.is_empty() { return default_bands(); }` — and `default_bands()` returns the three bands
`bright`/`dim`/`dark`. The doc is therefore CORRECT and the fallback VALUE is the three-band
default, not an empty list. Step 2's second test
(`a_gradation_document_that_will_not_decode_still_resolves_to_the_default_bands`) pins that value;
a test asserting emptiness would fail, and changing the doc to match the finding would make it
false.

**Files:**
- Modify: `src/server/src/scene/mod.rs` (`warn_if_config_engine_undecodable`, `set_world_config`,
  `apply_op`'s `Create` config arms **and** its config-Update mirror, `resolved_vision_modes`,
  `resolved_bands`, `compute_derived`)
- Test: `src/server/src/scene/mod.rs` (`mod tests`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `scene::warn_if_config_engine_undecodable<T>(doc: &Document)` (private, free
  function). No signature changes: `resolved_vision_modes` keeps
  `pub fn resolved_vision_modes(&self) -> BTreeMap<String, VisionMode>` and `resolved_bands`
  keeps `pub fn resolved_bands(&self) -> Vec<Band>`.

**The policy, and why it is this one.** `data::engine::engine_of` already sets the project's
precedent for exactly this situation and states it explicitly: an absent engine is the normal case
and stays silent, while a present-but-undeserializable engine indicates schema drift between
ingress validation and the typed read and is logged **with its deserialization error** so it is
observable rather than silently masked. Both readers here adopt that policy rather than inventing a
third: **absent ⇒ silent fallback (unchanged); present-but-undecodable ⇒ log at `warn` naming the
document and the error, then the same fallback.** The fallback value is deliberately unchanged —
changing it would alter what a world resolves to under an already-abnormal condition, which is a
separate decision from making the condition visible.

**Where the diagnostic goes, and why not in the resolver.** `SceneEcs::token_vision_floors` calls
BOTH `resolved_vision_modes` and `resolved_bands`, and it runs once per token per visibility
recompute. A `warn!` inside either resolver would therefore emit once per token per frame for as
long as the document stays broken — a log flood that buries the very signal it exists to raise, and
a per-frame allocation on the vision path. The diagnostic belongs where the decode is ATTEMPTED
ONCE: everywhere a config document enters the ECS or its stored engine changes. One warning per
installed document version, which is the rate the condition actually changes at.

**There are SIX such call sites, not four, and the sixth pair is the one an assignment-shaped search
misses.** `SceneEcs::set_world_config` (room hydration) and `SceneEcs::apply_op`'s `Create` arms
assign the field, so a search for `self.gradation = ` finds them. `apply_op`'s `Update` arm does
not assign: it calls `Self::apply_config_update`, which takes `&mut Option<Document>` and mutates
the stored document IN PLACE through `reapply_changes` when the op names its id. That path installs
a new engine value into a config document without any assignment to the field, so a search keyed on
the assignment shape reports the enumeration complete while omitting it — and a document made
undecodable by an update is exactly the schema-drift case this diagnostic exists for.

- [ ] **Step 1: Enumerate the install sites BY EFFECT, from source**

Run and record the full output of all three:

```bash
cd /c/Dev/Shadowcat && git grep -n "self.gradation\|self.vision_modes" -- src/server
cd /c/Dev/Shadowcat && git grep -n "apply_config_update" -- src/server
cd /c/Dev/Shadowcat && git grep -n "resolved_vision_modes\|resolved_bands" -- src/server
```

The enumeration question is **"after this runs, can `self.gradation` / `self.vision_modes` hold a
document whose engine differs from what it held before?"** — not "where is the field assigned". A
search shaped like the answer you expect finds only the sites shaped that way. Report, per hit,
which of the three categories it falls in:

- **installs a document** (the diagnostic attaches here);
- **clears the slot** (the `Delete` arms set `None` — no document, nothing to decode, so no
  diagnostic; this is vacuous, not an exclusion);
- **reads the slot** (an accessor or a resolver — no diagnostic, per the rate argument above).

Then report every caller of the two resolvers with the loop it sits in, and confirm from that list
that `token_vision_floors` is per-token. If it is not, the rate argument above is wrong and that is
a finding to report rather than a premise to keep. **If your enumeration finds an install path not
in the six the next steps wire, report it before editing** — the mutation step's per-site
requirement is only as strong as this list.

- [ ] **Step 2: Write the failing tests**

Add to `mod tests` in `src/server/src/scene/mod.rs`. Read the module's existing
`set_world_config` / vision-modes test helpers first and follow their construction:

```rust
    #[test]
    fn a_vision_modes_document_that_will_not_decode_still_resolves_to_the_seed() {
        // The fallback VALUE is unchanged by this task; pinning it here is what makes the
        // logging change provably behaviour-preserving rather than merely believed to be.
        // Discrimination: fails if the undecodable branch is changed to return an empty
        // registry, or to propagate an error, either of which would change what a world
        // resolves to under an abnormal condition.
        let ecs = ecs_with_vision_modes_engine(json!({ "modes": { "normal": { "id": "normal" } } }));
        let modes = ecs.resolved_vision_modes();
        assert!(modes.contains_key("normal") && modes.contains_key("darkvision"));
    }

    #[test]
    fn a_gradation_document_that_will_not_decode_still_resolves_to_the_default_bands() {
        // Sibling shape to the vision-mode reader; the fallback value is likewise unchanged.
        // `resolved_bands` routes its empty result through `lighting::sorted_bands`, whose
        // empty-input branch returns `lighting::default_bands()` — so the fallback is the three
        // built-in bands, not an empty list.
        // Discrimination: fails if the undecodable branch stops returning the built-in bands, and
        // fails if `sorted_bands`' empty-input branch is removed — the names are asserted, not
        // just the count, so a different three-band set also fails.
        let ecs = ecs_with_gradation_engine(json!({ "bands": [ { "name": "dim" } ] }));
        let names: Vec<String> = ecs.resolved_bands().into_iter().map(|b| b.name).collect();
        assert_eq!(names, vec!["bright".to_string(), "dim".to_string(), "dark".to_string()]);
    }
```

`ecs_with_vision_modes_engine` / `ecs_with_gradation_engine` are helpers you write on top of the
module's existing config-document construction — read `set_world_config` and the fixtures that
already build a `vision-modes` or `light-gradation` document and reuse them rather than building
a document literal by hand.

Before writing the second test's expectation, read `lighting::default_bands` and confirm the three
names and their order (`sorted_bands` returns brightest-first). If they differ from the values
above, use the ones in the source and report the difference.

**The decodable-document property already has a pin; do not write a second one.**
`vision_modes_doc_is_respected_not_reseeded` installs an authored `vision-modes` document through
`set_world_config` and asserts its own mode is present while a seed-only key is absent — which is
exactly the "a document that decodes wins outright" property. Verify that against the test as it
stands and **cite it by name in the report** instead of adding a duplicate under a new name. If it
does not assert that property, say so and add the missing pin.

**Now the coverage that makes the production change verifiable at all.** Everything above pins
fallback VALUES, all of which this task leaves unchanged — so none of it can tell whether the
diagnostic is wired at any install site, for either document kind. That is the whole production
change. The module already carries the harness for it: `LevelCapture` and
`captured_levels(f) -> Vec<tracing::Level>` in this same test module run `f` under a thread-local
capturing subscriber. Build on it rather than adding a second subscriber:

```rust
    /// Count of `warn`-level events emitted while `f` runs, over the module's existing
    /// thread-local capture. A COUNT is sufficient here because each test below installs exactly
    /// one document, so the count is that one site's emission; the decodable control in the same
    /// test is what separates "wired to this site" from "warns whatever is installed".
    fn captured_warns(f: impl FnOnce()) -> usize {
        captured_levels(f)
            .into_iter()
            .filter(|l| *l == tracing::Level::WARN)
            .count()
    }

    /// A `light-gradation` document whose engine will not decode: `GradationBand` requires
    /// `minIllumination`.
    fn undecodable_gradation_doc(id: u128) -> Document {
        let mut d = doc(id, None, "light-gradation");
        d.engine = Some(json!({ "bands": [ { "name": "dim" } ] }));
        d
    }

    /// A `light-gradation` document whose engine decodes.
    fn decodable_gradation_doc(id: u128) -> Document {
        let mut d = doc(id, None, "light-gradation");
        d.engine = Some(json!({ "bands": [
            { "name": "bright", "minIllumination": 0.5 },
            { "name": "dark", "minIllumination": 0.0 }
        ] }));
        d
    }

    /// A `vision-modes` document whose engine will not decode: `VisionMode` requires
    /// `illuminationFloor`.
    fn undecodable_vision_modes_doc(id: u128) -> Document {
        let mut d = doc(id, None, "vision-modes");
        d.engine = Some(json!({ "modes": { "normal": { "id": "normal" } } }));
        d
    }

    /// A `vision-modes` document whose engine decodes.
    fn decodable_vision_modes_doc(id: u128) -> Document {
        let mut d = doc(id, None, "vision-modes");
        d.engine = Some(json!({ "modes": { "normal": {
            "id": "normal", "name": "Normal", "illuminationFloor": "dark", "defaultRange": 6
        } } }));
        d
    }

    #[test]
    fn an_undecodable_gradation_is_reported_where_world_config_installs_it() {
        // Discrimination: the first half fails if the gradation call at this install site is
        // absent; the second fails if the diagnostic fires for a document that decodes, which is
        // what separates "wired here" from "warns unconditionally".
        let mut ecs = SceneEcs::new();
        let n = captured_warns(|| {
            ecs.set_world_config(None, Some(undecodable_gradation_doc(101)), None)
        });
        assert_eq!(n, 1, "an undecodable gradation document is reported once where it installs");

        let mut ok = SceneEcs::new();
        let quiet = captured_warns(|| {
            ok.set_world_config(None, Some(decodable_gradation_doc(101)), None)
        });
        assert_eq!(quiet, 0, "a gradation document that decodes is not reported");
    }

    #[test]
    fn an_undecodable_vision_modes_doc_is_reported_where_world_config_installs_it() {
        // The sibling site for the other document kind: one type parameter apart, and a single
        // call covering both would leave one kind unreported with nothing to show it.
        // Discrimination: as above, applied to the vision-modes argument.
        let mut ecs = SceneEcs::new();
        let n = captured_warns(|| {
            ecs.set_world_config(None, None, Some(undecodable_vision_modes_doc(102)))
        });
        assert_eq!(n, 1, "an undecodable vision-modes document is reported once where it installs");

        let mut ok = SceneEcs::new();
        let quiet = captured_warns(|| {
            ok.set_world_config(None, None, Some(decodable_vision_modes_doc(102)))
        });
        assert_eq!(quiet, 0, "a vision-modes document that decodes is not reported");
    }

    #[test]
    fn an_undecodable_gradation_is_reported_where_a_create_installs_it() {
        // The write path, which is a different install site from room hydration and would
        // otherwise be wired by assumption. Discrimination: fails if the `Create` arm carries no
        // call for this document kind.
        let mut ecs = SceneEcs::new();
        let n = captured_warns(|| {
            ecs.apply_op(&Operation::Create { doc: undecodable_gradation_doc(101) })
        });
        assert_eq!(n, 1, "a created gradation document that will not decode is reported once");

        let mut ok = SceneEcs::new();
        let quiet = captured_warns(|| {
            ok.apply_op(&Operation::Create { doc: decodable_gradation_doc(101) })
        });
        assert_eq!(quiet, 0, "a created gradation document that decodes is not reported");
    }

    #[test]
    fn an_undecodable_vision_modes_doc_is_reported_where_a_create_installs_it() {
        // Discrimination: fails if the `Create` arm carries no call for this document kind.
        let mut ecs = SceneEcs::new();
        let n = captured_warns(|| {
            ecs.apply_op(&Operation::Create { doc: undecodable_vision_modes_doc(102) })
        });
        assert_eq!(n, 1, "a created vision-modes document that will not decode is reported once");

        let mut ok = SceneEcs::new();
        let quiet = captured_warns(|| {
            ok.apply_op(&Operation::Create { doc: decodable_vision_modes_doc(102) })
        });
        assert_eq!(quiet, 0, "a created vision-modes document that decodes is not reported");
    }

    #[test]
    fn an_update_that_makes_the_stored_gradation_undecodable_is_reported() {
        // The install path with no assignment: `apply_config_update` rewrites the STORED document
        // in place, so an engine can become undecodable without the field being assigned.
        // Discrimination: the first half fails if the diagnostic is attached only to the
        // assignment sites. The second half pins the id guard — an update naming another document
        // must not re-report a config document it did not touch, which is the per-op repetition
        // this task's own rate argument rejects.
        use crate::data::command::FieldChange;
        let mut ecs = SceneEcs::new();
        ecs.apply_op(&Operation::Create { doc: decodable_gradation_doc(101) });

        let break_it = Operation::Update {
            doc_id: Uuid::from_u128(101),
            changes: vec![FieldChange {
                remove: false,
                path: "/engine/bands".into(),
                old: json!(null),
                new: json!([ { "name": "dim" } ]),
            }],
        };
        let n = captured_warns(|| ecs.apply_op(&break_it));
        assert_eq!(n, 1, "an update that makes the stored gradation undecodable is reported once");

        let unrelated = Operation::Update {
            doc_id: Uuid::from_u128(999),
            changes: vec![FieldChange {
                remove: false,
                path: "/name".into(),
                old: json!(null),
                new: json!("unrelated"),
            }],
        };
        let quiet = captured_warns(|| ecs.apply_op(&unrelated));
        assert_eq!(quiet, 0, "an update naming another document does not re-report the stored one");
    }

    #[test]
    fn an_update_that_makes_the_stored_vision_modes_doc_undecodable_is_reported() {
        // Discrimination: as above, for the other document kind and the other mirror call.
        use crate::data::command::FieldChange;
        let mut ecs = SceneEcs::new();
        ecs.apply_op(&Operation::Create { doc: decodable_vision_modes_doc(102) });

        let break_it = Operation::Update {
            doc_id: Uuid::from_u128(102),
            changes: vec![FieldChange {
                remove: false,
                path: "/engine/modes".into(),
                old: json!(null),
                new: json!({ "normal": { "id": "normal" } }),
            }],
        };
        let n = captured_warns(|| ecs.apply_op(&break_it));
        assert_eq!(n, 1, "an update that makes the stored vision-modes doc undecodable is reported");

        let unrelated = Operation::Update {
            doc_id: Uuid::from_u128(999),
            changes: vec![FieldChange {
                remove: false,
                path: "/name".into(),
                old: json!(null),
                new: json!("unrelated"),
            }],
        };
        let quiet = captured_warns(|| ecs.apply_op(&unrelated));
        assert_eq!(quiet, 0, "an update naming another document does not re-report the stored one");
    }
```

Read `doc`, `SceneEcs::new`, `Operation` and `FieldChange` as the module already uses them and
follow the real shapes — the snippets show the structure, not a guaranteed drop-in. Record, as part
of this step, the observed level vector for each broken-document run: if any WARN other than the
diagnostic's is emitted on one of these paths, that is a finding to report, **not** a reason to
loosen the assertion from an exact count to "at least one".

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cd src/server && cargo test --lib scene::tests::a_vision_modes && cargo test --lib scene::tests::a_gradation && cargo test --lib scene::tests::an_undecodable && cargo test --lib scene::tests::an_update_that`

RUN, OBSERVE, RECORD. The two fallback-value tests may already hold against current behaviour —
that is information, not a problem; record which do and which do not before writing the
implementation. The six install-site tests cannot already hold, since the diagnostic does not
exist; record the compiler's and the runner's actual output rather than asserting what they will
say.

- [ ] **Step 4: Add the install-time diagnostic**

In `src/server/src/scene/mod.rs`, beside the other private free functions:

```rust
/// Report a world-config document whose typed `engine` is present but will not deserialize.
///
/// Called where a config document ENTERS the ECS or has its stored engine rewritten, never from a
/// resolver: `token_vision_floors` calls both config resolvers once per token per visibility
/// recompute, so a diagnostic inside a resolver emits once per token per frame for as long as the
/// document stays broken. An install site fires once per document version, which is the rate the
/// condition changes at. The update mirror's call is guarded on the op naming that document, for
/// the same reason.
///
/// Policy mirrors `data::engine::engine_of`: an ABSENT engine is the ordinary case for a fresh
/// world and stays silent; a PRESENT one that will not deserialize is schema drift between ingress
/// validation and the typed read, and is reported with its error. The decode here is deliberately
/// direct rather than through `engine_as_cached`, which discards the error this diagnostic exists
/// to carry.
fn warn_if_config_engine_undecodable<T: serde::de::DeserializeOwned>(doc: &Document) {
    let Some(v) = doc.engine.as_ref() else {
        return;
    };
    if let Err(e) = serde_json::from_value::<T>(v.clone()) {
        tracing::warn!(
            doc_id = %doc.id,
            doc_type = %doc.doc_type,
            error = %e,
            "world config engine failed to deserialize; the built-in default resolves in its place"
        );
    }
}
```

Call it at **all six** install sites — three install points × two document kinds — each with its own
type parameter. A single call covering "a config document" does not exist: the type parameter IS
the decode, so one call per kind is structural, not repetition.

In `SceneEcs::set_world_config`, before the assignments:

```rust
        if let Some(d) = gradation.as_ref() {
            warn_if_config_engine_undecodable::<eng::LightGradationEngine>(d);
        }
        if let Some(d) = vision_modes.as_ref() {
            warn_if_config_engine_undecodable::<eng::VisionModesEngine>(d);
        }
```

In `apply_op`'s `Create` arm, on the document being installed — read the real arms before editing,
since they assign `Some(doc.clone())` and the borrow shape decides whether the call goes before or
after the assignment.

In `apply_op`'s `Update` arm, **after** the `Self::apply_config_update` calls, because that is the
mutation whose result is being judged — and guarded on the op naming that document, because
otherwise every update to any document re-reports a config document it never touched, which is the
per-op repetition this task's own rate argument rejects:

```rust
                if let Some(d) = self.gradation.as_ref().filter(|d| d.id == *doc_id) {
                    warn_if_config_engine_undecodable::<eng::LightGradationEngine>(d);
                }
                if let Some(d) = self.vision_modes.as_ref().filter(|d| d.id == *doc_id) {
                    warn_if_config_engine_undecodable::<eng::VisionModesEngine>(d);
                }
```

`apply_config_update` takes `&mut Option<Document>` for the three singletons independently, so
these reads sit after all three of its calls and borrow `self` immutably. Read the real arm before
editing and follow its borrow shape.

- [ ] **Step 5: Correct the two resolver comments the code contradicts**

Neither resolver's body changes. What changes is the comment that claims an intent the code does
not achieve, and it must state the present constraint without narrating the previous one.

`resolved_vision_modes`' inline comment currently says the seed is used "only on the None
(absent/malformed) branch — a present doc's modes being all malformed must not silently replace a
GM-authored registry with the built-in seed", which describes a distinction the code does not draw.
Replace it with:

```rust
        // A present document that will not decode resolves to the built-in seed, exactly as an
        // absent one does. The two are distinguished at the install sites
        // (`warn_if_config_engine_undecodable`), which report the undecodable case once per
        // document version; this resolver runs per token per visibility recompute and stays
        // silent.
```

`resolved_bands` carries no equivalent claim; give it the matching sentence on its doc comment so
the two readers state the same policy:

```rust
    /// Resolved gradation bands, brightest-first. Fail-closed to the built-in three-band default
    /// (`lighting::sorted_bands` substitutes `lighting::default_bands` for an empty list), for an
    /// absent document and for a present one that will not decode alike; the undecodable case is
    /// reported at the install sites rather than here, because this resolver runs per token per
    /// visibility recompute.
```

- [ ] **Step 6: Derive the double-resolve marker's premise (NEW-11) — BEFORE reading Step 7**

`compute_derived`'s `"vision"` arm carries `// TODO: thread the bands player_lit_mask already
resolved to avoid this second resolve.` Answer these three questions from source, quoting the
expressions, and write all three answers into the task report before Step 7 is read:

1. Does `resolved_bands` decode through `engine_as_cached` or through a fresh
   `serde_json::from_value`? Quote the expression.
2. What does the second call therefore cost — a decode, or a cache hit plus the work in
   `sorted_bands`? Quote `sorted_bands`' body and state how many bands it sorts.
3. Do the two calls read the same function, so that no value can differ between them?

Then state, in one sentence and on your own answers alone, whether the marker describes a cost that
still exists.

- [ ] **Step 7: Compare against the plan's reading, then act on the derivation**

The plan's own reading, recorded here so Step 6 could not be anchored by it:

> The decode is cached by document value, the residual cost of the second call is a sort of a
> handful of bands, and both callers read the one resolver — so no value can fork and the cost the
> marker names is gone.

If your answers agree, act: the marker asks for a restructuring that buys nothing and costs shape,
because `player_lit_mask` returns per-scene `LitScene` values while the bands are a world-level
list, so threading them means widening a per-scene return type to carry a world-level value. Delete
the marker and leave one sentence stating the present constraint:

```rust
                // Both this payload and `player_lit_mask` read `resolved_bands`, so the band list
                // cannot differ between them; the decode behind it is cached by document value.
```

**If they disagree — in particular if you found the decode is not cached — stop and report**,
quoting your derivation and this reading. The marker would then be describing a real per-frame
decode, and removing it would be closing a to-do by deleting it, which is not what this step is
for. Do not adjust either reading to match the other.

- [ ] **Step 8: Run the tests and the scene suite**

Run: `cd src/server && cargo test --lib scene`

RUN, OBSERVE, RECORD.

- [ ] **Step 9: Mutation check — prove every install-site call is load-bearing**

The production change in this task is the diagnostic at six call sites and nothing else, so the
only evidence that it is wired is that removing each call is detected on its own.

Six mutations, run and reverted independently. For each: delete that ONE
`warn_if_config_engine_undecodable` call, run
`cd src/server && cargo test --lib scene::tests::an_undecodable && cargo test --lib scene::tests::an_update_that`,
record the observed failing test names and messages, restore the call, re-run, and confirm green
plus a byte-identical diff against the pre-mutation file.

1. `set_world_config`'s gradation call.
2. `set_world_config`'s vision-modes call.
3. `apply_op`'s `Create` gradation call.
4. `apply_op`'s `Create` vision-modes call.
5. `apply_op`'s update-mirror gradation call.
6. `apply_op`'s update-mirror vision-modes call.

Each must be observed to fail its own site's test and no other — a mutation that takes down two
tests means two sites share one call and one document kind or one path is unreported. **A mutation
that leaves the suite green means that site is unwired or uncovered**: that is the finding to
report, and it stops the task rather than being noted.

Then one seventh mutation for the id guard: delete the `.filter(|d| d.id == *doc_id)` from the
update-mirror gradation call so it reports on every update, run the same tests, and record the
result. A green suite there means the guard is unproven, and the guard is the whole difference
between one warning per document version and one per op.

- [ ] **Step 10: Run the full server gate**

Run: `cd src/server && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

- [ ] **Step 11: Commit**

```bash
git add src/server/src/scene/mod.rs
git commit -m "fix(scene): report a config registry that will not decode

The vision-mode and gradation readers treated an absent document and a
present, undecodable one as the same outcome, so an authored registry could
be replaced by the built-in seed with no signal — the opposite of what the
vision-mode reader's own comment claimed. The undecodable case is now reported
at every site that installs a config document or rewrites its stored engine —
including the update mirror, which changes an engine without assigning the
field — at once per document version rather than once per token per visibility
recompute, and carries the deserialization error the way the typed-read policy
elsewhere does. Both fallback values are unchanged and pinned by test, and each
site's call is held by its own test.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>" -- src/server/src/scene/mod.rs
```

---

### Task 10: TD17 + TD18 — reuse the role binding in scope; delete the caller-less predicate

**Ledger ids:** TD17, TD18. Two independent one-move changes in the same crate, committed
separately.

**Files:**
- Modify: `src/server/src/ws/room.rs` (`Room::execute_move`)
- Modify: `src/server/src/scene/mod.rs` (delete `SceneEcs::blocks_move` and its tests)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `SceneEcs::blocks_move` ceases to exist. Nothing consumes it after this task.

- [ ] **Step 1: TD17 — reuse `is_gm`**

`Room::execute_move` binds `is_gm = ctx.world_role == WorldRole::Gm` under the first scene read
guard and still holds that binding where `mover_vision` is computed, which re-derives the same
comparison. Replace the re-derivation:

```rust
            mover_vision = if is_gm {
```

Then read the comment above that line and update it if it explains the derivation rather than the
behaviour. Verify the binding is genuinely in scope and unshadowed by compiling, not by reading.

Run and record: `git grep -n "WorldRole::Gm" -- src/server/src/ws/room.rs`. Report each remaining
hit with its enclosing function and why it is not a duplicate of `is_gm` — "the count went down by
one" is not the report; the per-hit disposition is.

- [ ] **Step 2: Run the gate and commit TD17**

Run: `cd src/server && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

```bash
git add src/server/src/ws/room.rs
git commit -m "refactor(ws/room): reuse the resolved GM role in the move executor

The mover-vision branch re-derived the world-role comparison the gate had
already resolved and still held, so one request carried two independent
answers to the same question.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>" -- src/server/src/ws/room.rs
```

- [ ] **Step 3: TD18 — confirm there is no production caller, then delete**

The spec's §6 resolves this fork: delete. Its stated justification was that it is one home for
wall-crossing semantics, but the production path reads `move_walls` and `segments_cross` directly
— so it is a **second** home for that decision with no callers, which is worse than none.

Confirm before deleting. Run and record the full output:

```bash
cd /c/Dev/Shadowcat && git grep -n "blocks_move" -- src/server
```

Classify every hit: the `eng::WallEngine.blocks_move` field and the `wall.blocks_move != Some(true)`
filters are the wall document's own data and stay; `SceneEcs::blocks_move` and its tests go. If
any hit is a production call of `SceneEcs::blocks_move`, **stop and report** — the fork's premise
would then be false and deletion is not this task's call to make.

Delete `SceneEcs::blocks_move` and the tests that exist only to exercise it
(`blocks_move_geometry_scene_scoping_and_filters` and any sibling naming it). Then check the
comments that cite it as a reference — `SceneEcs::move_walls`' doc comment states an INVARIANT
about sharing a filter with it, and `move_exec::execute_move`'s parity doc calls `segments_cross`
"the primitive `blocks_move` wraps". Both must be rewritten to state the constraint without
naming a symbol that no longer exists. A dangling symbol reference is the rot the citation rule
exists to prevent.

- [ ] **Step 4: Run the gate and commit TD18**

Run: `cd src/server && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

Record the outcome. If `clippy` reports a newly-unused import or helper left behind by the
deletion, remove it — do not annotate it.

```bash
git add src/server/src/scene/mod.rs
git commit -m "refactor(scene): delete the caller-less wall-crossing predicate

Production resolves a wall crossing through move_walls and segments_cross
directly, so this predicate was a second home for that decision with nothing
reading it. The comments that cited it now state the constraint without
naming it.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>" -- src/server/src/scene/mod.rs
```

---

### Task 11: TD19 — one anchor for the executor's footprint

**Ledger id:** TD19.

**Files:**
- Modify: `src/server/src/scene/move_exec.rs` (`execute_move`)
- Test: `src/server/src/scene/move_exec.rs` (`mod tests`)

**Interfaces:**
- Consumes: Task 4's anchor-independent hex footprint predicate — this task creates a third
  production caller passing an off-centre `ctr`, and Task 4 is what makes that correct on hex.
  Also Task 6's `r_scene` comment on the same line region.
- Produces: no signature changes.

**The asymmetry, verified, and which way it fails.** Inside `execute_move`'s per-step loop the
wall gate measures `point_segment_distance(next, ...)` — the token's **true** position on the
dense walk — while the cell-membership gates (mask, impassable) measure
`footprint_cells(next_cell, fp_ctr, ...)` with `fp_ctr = grid.cell_center(next_cell)`. The two
coincide on grid input, where `gate_walk` is the identity and grid A* emits cell-centre vertices.
They do not coincide on a continuous route, where `next` is genuinely off-centre — and there the
cell-membership gates measure from a point the token is not at, which can omit a cell the token's
disc genuinely overlaps. That is the fail-open direction, on the gate rather than on the preview.

**The existing comment argues the opposite and must be answered, not deleted.** The comment above
`fp_ctr` states that anchoring at `next` "is degenerate whenever `next` lands exactly on a cell
boundary … where `footprint_cells`'s zero-distance-to-AABB test spuriously admits every cell
touching that corner, not just the cells the footprint actually occupies". Steps 1–2 derive whether
that claim holds against the predicates as written — on BOTH grid kinds, since Task 4 changed the
hex one — before anything is changed, and Step 6 replaces the comment with an answer either way.

**"All three agree" is not the claim this task makes.** `pathfinding::cell_enterable` anchors at
`cell_center(to)` at both of its uses and is NOT changed here, so two anchoring RULES remain. They
do not conflict, and the reason is specific: the grid router's own emitted vertices ARE cell
centres, and `gate_walk` is the identity on them, so on every input the grid router produces the
true point and the cell centre are the same value. What this task removes is the disagreement on
CONTINUOUS input, where the preview (`clip_to_visible_mask`, `los_smooth`) already anchors at the
sample point and the executor did not.

- [ ] **Step 1: Derive both predicates' behaviour at a boundary-exact centre BEFORE reading Step 2**

Two derivations, both written into the task report before Step 2 is read.

**(a) Square.** Read `pathfinding::footprint_cells` in full — it is the body behind
`SquareGrid::footprint_cells` — and answer, quoting the expressions:

1. For `ctr.0 = k * cell` exactly and `r_scene = 0.0`, what values do `i0` and `i1` take, and how
   many columns does the loop body therefore examine?
2. For the same `ctr.0` with `0 < r_scene < cell`, which columns does the loop examine, and what
   does `dx` evaluate to for each?
3. For every cell the loop ADMITS when `ctr` sits exactly on a four-way lattice corner with
   `r_scene > 0`, does the closed disc of radius `r_scene` about `ctr` meet that cell's rectangle
   in positive area? Answer per cell.
4. Therefore: is there any input for which the emitted set contains a cell the disc does not
   overlap?

**(b) Hex.** Read `HexGrid::footprint_cells` and `HexGrid::distance_to_cell_polygon` as Task 4
leaves them, and answer the analogous questions:

5. For `ctr` exactly on the shared EDGE between two hexes, what does `distance_to_cell_polygon`
   return for each of those two hexes, and which of them does the predicate admit at
   `r_scene = 0.0`? At `r_scene > 0`?
6. For `ctr` exactly on a VERTEX shared by three hexes, the same question for all three.
7. In each of those cases, does the closed disc of radius `r_scene` about `ctr` meet the admitted
   hex's polygon at all — and at `r_scene = 0`, does it meet it in positive AREA?
8. Therefore: is a boundary-exact `ctr` a degenerate input for the hex predicate, and if the
   emitted set is larger there than at a nearby interior point, which direction does that move the
   gates that read it?

- [ ] **Step 2: Compare against the plan's reading, and stop if they disagree**

The plan's own readings, recorded here so Step 1 could not be anchored by them.

**Square:**

> The loop bounds `i0..=i1` are themselves derived from `ctr ± r_scene`, so at `r_scene = 0` they
> collapse to the single column `floor(ctr.0 / cell)` and no flanking cell is examined at all. At
> `r_scene > 0` the two flanking columns ARE examined and both evaluate `dx = 0`, but a disc of
> positive radius centred on a cell boundary genuinely meets both adjacent cells in positive area,
> and at a four-way corner all four. The emitted set is therefore the set of cells the disc
> actually overlaps, and the spurious admission the comment describes is not reproducible from the
> predicate as written.

**Hex:**

> `distance_to_cell_polygon` returns exactly `0.0` for every hex whose polygon contains or touches
> `ctr`, so an edge-exact `ctr` admits both flanking hexes and a vertex-exact `ctr` admits all
> three, at any `r_scene >= 0`. At `r_scene > 0` that is simply correct — the disc meets each of
> them in positive area. At `r_scene = 0` exactly, the "disc" is a point lying on the shared
> boundary, so it touches each of those hexes without meeting any of them in positive area, and the
> emitted set is one or two cells larger than at a nearby interior point. That is an
> over-inclusion on a set of inputs of measure zero, and over-inclusion TIGHTENS every gate that
> reads it: more cells must be visible, more cells are checked for impassability. It is the safe
> direction and needs no correction.

If your derivations agree, say so and proceed to Step 3. **If either disagrees — if you find an
input where the emitted set contains a cell the disc does not even touch, or where the hex
predicate's boundary behaviour LOOSENS a gate — stop and report**, quoting the input and the cell.
That would make the existing comment correct, and this task's change would then need the predicate
corrected first, which is a scope question rather than something to work around inside this task.

- [ ] **Step 3: Derive the test's coordinates BEFORE reading Step 4**

The test in Step 5 needs a square scene at cell 100 with a 0.4-cell footprint, a horizontal
continuous step that stays strictly inside one cell row, and a destination at which the token's
disc reaches into the adjacent row while a disc centred on the destination CELL'S CENTRE does not.
Derive, from `pathfinding::footprint_cells` and `SquareGrid::cell_center`, and write down:

1. The disc radius in scene units for `footprint_radius_cells = 0.4` at cell 100.
2. A start and destination point on the same horizontal line inside row 1, chosen so the disc at
   the destination crosses the `y = 100` boundary while the step itself stays inside row 1. State
   the y coordinate you choose and the disc's y span at the destination.
3. `footprint_cells` evaluated at the destination anchored at the TRUE point: the exact cell set.
4. `footprint_cells` evaluated at the destination anchored at the destination cell's CENTRE: the
   exact cell set.
5. The two masks the test needs — one that contains (4) but not (3), and one that contains both.

- [ ] **Step 4: Compare against the plan's coordinates, and report any disagreement**

The plan's own values, recorded here so Step 3 could not be anchored by them: radius 40 scene
units; the walk runs from `(150,130)` to `(250,130)`, so the traversal cells are `(1,1)` and
`(2,1)` only; at the destination the disc spans `y ∈ [90,170]` and therefore reaches cell `(2,0)`;
measured from cell `(2,1)`'s centre `(250,150)` the same disc spans `y ∈ [110,190]` and never
leaves row 1. The first mask omits `(2,0)`; the second adds it.

A disagreement is a finding to report, not a value to adopt from either side.

- [ ] **Step 5: Write the failing test**

Add to `mod tests` in `src/server/src/scene/move_exec.rs`. Read the module's existing continuous
executor tests first (those passing an any-angle `path` rather than cell centres) and reuse their
fixture construction:

```rust
    #[test]
    fn an_off_center_step_gates_the_footprint_where_the_token_actually_is() {
        // A square scene at cell 100 with a 0.4-cell footprint. The token walks a horizontal step
        // that stays strictly inside row 1, but at the destination its disc reaches into row 0, so
        // a cell outside the first mask is under the token's body.
        //
        // Discrimination: the first half fails whenever the cell-membership footprint is anchored
        // at the destination cell's centre, because the emitted set is then confined to row 1,
        // every member is in the mask, and the step is admitted. The second half is the
        // anti-vacuity guard: adding that row-0 cell to the mask must admit the same move, so a
        // gate that refuses everything fails there rather than passing the first assertion for the
        // wrong reason.
        // ... assemble the scene, the token, the path and the two masks from the coordinates
        // derived in Step 3; read the module's existing continuous-executor fixture for
        // `MoveGateInputs` construction ...
        let refused = execute_move(&ecs, gate_without_row_zero, token, &path, false, 0.4)
            .expect("the walk itself is well formed");
        assert!(
            refused.truncated,
            "the token's disc overlaps a cell outside the mask, so the step is refused"
        );

        let admitted = execute_move(&ecs, gate_with_row_zero, token, &path, false, 0.4)
            .expect("the walk itself is well formed");
        assert!(
            !admitted.truncated,
            "with that cell visible the same step is legal"
        );
    }
```

Do **not** add a second test for the grid-input invariance claim: the module's frozen parity
fixture already pins whole-path grid outcomes at a granularity no per-step test improves on. Cite
it by its real name in Step 8's report instead.

- [ ] **Step 6: Run the test to verify it fails**

Run: `cd src/server && cargo test --lib scene::move_exec`

RUN, OBSERVE, RECORD.

- [ ] **Step 7: Write the implementation**

In `execute_move`'s per-step loop, delete the `fp_ctr` binding and anchor both cell-membership
calls at the true walk point:

```rust
        let next_cell = to_cell(next);
```

```rust
        if check_mask {
            let Some(mut cells) = grid.line_traversal(prev, next, cell) else {
                stopped_early = true;
                break;
            };
            cells.extend(grid.footprint_cells(next_cell, next, r_scene, cell));
            if !cells.iter().all(|c| visible.contains(c)) {
                stopped_early = true;
                break;
            }
        }
```

```rust
            if check_regions {
                let fp_cells = grid.footprint_cells(next_cell, next, r_scene, cell);
                if fp_cells.iter().any(|c| regions.is_impassable(*c)) {
                    stopped_early = true;
                    break;
                }
            }
```

Replace the `fp_ctr` comment block with one that answers the argument it made rather than dropping
it:

```rust
        // Every footprint test — the wall disc, the mask's cell membership, and the impassable
        // check — anchors at `next`, the token's actual position on the dense walk. The
        // continuous preview anchors the same way (`clip_to_visible_mask` and `los_smooth` both
        // pass the sample point), so preview and gate cannot disagree about where the token is.
        //
        // Anchoring at the destination cell's CENTRE instead is not equivalent, and the
        // difference is a fail-open: off-centre, the centre-anchored disc omits cells the token's
        // body genuinely overlaps, and those cells then never have to be visible.
        //
        // A boundary-exact sample is not a degenerate input on either shape. Square derives its
        // scan bounds from `ctr ± r_scene`, so a zero-radius footprint examines exactly one cell
        // and every extra cell a positive-radius disc picks up on a boundary is one it meets in
        // positive area. Hex measures distance to the hex polygon, which is zero for every hex
        // touching the point, so a boundary-exact `ctr` at zero radius admits the two or three
        // hexes meeting there — an over-inclusion that TIGHTENS this gate, since more cells must
        // then be visible and more are checked for impassability.
        //
        // `cell_enterable` keeps `cell_center` as its own anchor and is not affected: every vertex
        // the grid router emits IS a cell centre, and `gate_walk` is the identity on grid input,
        // so the two anchors evaluate to the same point on every input that router produces.
```

- [ ] **Step 8: Run the tests and the frozen parity fixtures**

Run: `cd src/server && cargo test --lib scene::move_exec && cargo test --lib scene::pathfinding && cargo test --lib scene::navmesh && cargo test --lib scene::grid_shape_parity_tests`

RUN, OBSERVE, RECORD. Report the frozen-parity tests by name with their observed outcome. **A
frozen fixture that needs editing is a stop-and-report, not a fixup**: those fixtures exist so a
grid-behaviour change cannot happen silently, and this change is claimed to be inert on grid
input. If one moves, the claim is wrong.

- [ ] **Step 9: Mutation check — prove the anchor is load-bearing**

Temporarily restore `let fp_ctr = grid.cell_center(next_cell);` and pass `fp_ctr` at both
`footprint_cells` calls, run `cd src/server && cargo test --lib scene::move_exec`, record the
observed failing test names and messages, revert, re-run, and confirm green plus a byte-identical
diff against the pre-mutation file. If the suite stays green the anchor is uncovered and that is
the finding to report; stop rather than proceeding.

- [ ] **Step 10: Run the full server gate**

Run: `cd src/server && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

- [ ] **Step 11: Commit**

```bash
git add src/server/src/scene/move_exec.rs
git commit -m "fix(scene/move_exec): anchor every footprint test at the token's position

The wall disc measured from the dense walk's actual point while the mask and
impassable checks measured from the destination cell's centre, so an
off-centre continuous step was gated as if the token stood where it does not,
and cells its body overlaps were never required to be visible. All three now
anchor at the walk point, matching the continuous preview's own anchoring. The
grid router keeps its cell-centre anchor, which agrees on every input it emits
because those vertices are cell centres.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>" -- src/server/src/scene/move_exec.rs
```

---

### Task 12: Phase closeout

**Files:**
- Modify: `docs/OPEN_BUGS.md`, `docs/CLOSED_BUGS.md`, `docs/TODO.md`,
  `docs/POST_WORK_FINDINGS.md`
- Modify: `docs/superpowers/specs/2026-08-13-debt-burndown-campaign-design.md` (ledger
  dispositions and the six new ids)
- Modify: `.claude/skills/shadowcat-codebase-scene-rendering/SKILL.md`
- Modify: `.claude/.claude-plugin/plugin.json`

- [ ] **Step 1: Promote and close the two bug-tier items**

PW1 and PW2 were promoted to bug status by the spec's triage. Add their resolution to
`docs/CLOSED_BUGS.md` in the file's existing entry style — symbols cited, never file names or
line numbers, no milestone ids, sweep markers, dates, or history narration. Cite
`GridShape::world_units_per_cell`, `GridShape::world_extent`, `SceneEcs::scene_world_extent`,
`build_navmesh`, `env_light_polys` and `bound_for_scene`. If either has an entry in
`docs/OPEN_BUGS.md`, remove it in the same commit.

- [ ] **Step 1a: Record the environment-light leak the envelope closed**

Add to `docs/CLOSED_BUGS.md`, in the file's existing entry style: `env_light_polys` sampled its
perimeter from a rectangle anchored at the origin, so on hex its bottom edge ran along `y = 0` —
**through** axial row 0 of the authored block rather than outside it. A boundary sample could
therefore land inside a `blocksLight`-sealed room on that row and project environment light into a
space the seal was supposed to keep dark. Sampling now walks the block's true envelope, so every
sample sits strictly outside it.

Cite `lighting::env_light_polys`, `lighting::perimeter_point`, and `grid_shape::WorldExtent`. Hex
only — a square block's minimum is the origin, so its walk was always outside. State that it was
unexercised: no fixture placed a sealed room on the origin row, which is why the leak survived to be
found by reasoning about the walk's geometry rather than by a failing test.

- [ ] **Step 2: Close the to-do entries**


Remove the TD17, TD18, TD19 and TD48 entries from `docs/TODO.md`.

**Do not touch the seven validly-blocked entries** the spec's §4.3 lists (TD34, TD35, TD36, TD41,
TD42, TD43, TD46) — TD35 and TD36 in particular are cost-comparability items in this very
subsystem and are blocked on an un-started `PLAN.md` milestone, not on this phase. Verify their
blocker text still names the blocking phase and report if it does not; do not rewrite it here.

- [ ] **Step 3: Record every per-item disposition**

In the spec's §4.4 and §4.2 tables, add a disposition line for **each** of PW1, PW2, PW3, PW4,
PW5, PW31, TD17, TD18, TD19, TD48 — every id in this phase's input list, whether the outcome was
a fix, a test, or a verification that the entry's stated mechanism no longer exists. State the
evidence (the symbols changed and the tests that hold them).

Append NEW-6, NEW-7, NEW-8, NEW-9, NEW-10 and NEW-11 to the §4.4 ledger with their one-line
statements and this phase as their assignment, per the spec's §2.4.

Two dispositions must record a **contradiction between the spec's entry and the source**, because
the spec's entries were written from earlier reviews:
- **PW4** — the described work was already built and shipped; this phase verified it and corrected
  the superseded finding rather than building it.
- **PW5** — the described per-entry drop no longer exists (ingress now rejects a mode entry
  missing its floor); the live defect is the whole-document decode failing over to the built-in
  seed with no diagnostic, which is what was fixed. Record with it that the fix covers **six**
  install sites, not the four an assignment-shaped search finds: the config-Update mirror rewrites
  a stored engine in place without assigning the field, and each of the six is held by its own
  test and its own mutation.

Two dispositions must record a **decision not to convert or not to restructure**:
- the token footprint radius stays on the indexing scale at all four of its sites, with the reason
  (its model is a square block, and rescaling it changes what a token occupies) and the consequence
  (hex footprints are a rules decision for a later design pass, not an open defect). State it as a
  recorded decision, not as deferred work;
- NEW-11's marker is closed by deriving that its premise no longer holds rather than by threading
  the value, with the derivation's three answers recorded.

One disposition must record a **user-visible consequence**: the explored-fog blob's new header
means every previously-persisted blob decodes empty, so remembered fog re-accumulates from play.
State the direction (under-reveal) and that no conversion pass exists because fog memory is derived
data.

- [ ] **Step 4: Update the subsystem skill (reviewed skill-update gate)**

**Method requirement, learned during this phase.** A skill claim can be stale in SUBSTANCE while the
symbol it is stale about never appears in the text. An implementer reported
`shadowcat-codebase-scene-rendering` untouched by the extent work because it never names
`GridShape::world_extent` — literally true, and the skill nonetheless documents `build_navmesh` with a
signature it has not had for some time and describes it as triangulating "the scene's bounds
rectangle", which is precisely the origin anchoring this phase removed. **Audit the skill's CLAIMS
against the code, never the skill's symbol list against the diff** — a grep for changed symbol names
reports clean on exactly the drift that matters most, because the staleness lives in a description
whose subject was renamed or restructured out of it.

Corrections this phase makes mandatory, beyond the additions listed after them:

- `build_navmesh`'s documented signature is wrong independently of this phase and must be corrected to
  what it takes now.
- The bounds-to-world conversion returns a `grid_shape::WorldExtent` carrying BOTH corners, and the
  minimum is the origin only on square — a pointy-top hex block reaches below and left of it. Any
  sentence describing a scene rectangle as anchored at the origin is wrong for half the shapes.
- `grid_shape::REFUSED_EXTENT` is the single zero-area refusal every extent guard rejects, and
  `SceneEcs::scene_world_extent_at` is the single body performing the conversion.
- `world_extent`'s guarantees hold over the INTEGER block, not over an arbitrary authored bound: a
  fractional bound leaves a partial column or row outside on a shape-dependent condition, which an
  executable membership rule and its sweep enforce rather than prose. The "CENTRE cover with a
  documented origin-side truncation" phrasing below is superseded and must not be carried forward.


`shadowcat-codebase-scene-rendering` gains, as Hard Invariants:
- the three-role distinction for the `cell` scalar — indexing scale, per-cell world distance,
  subdivision density — with `GridShape::world_units_per_cell` as the second, the two gate-facing
  densities named as deliberate non-conversions, and the token footprint radius named as a fourth
  category that stays on the indexing scale because its model is a square block;
- `GridShape::world_extent` as the only conversion from authored grid-unit bounds to a world
  rectangle, that `bound_for_scene` takes the converted value, that `DEFAULT_SCENE_BOUNDS_UNITS` is
  itself in grid units and therefore converts too, and that the hex result is a CENTRE cover with a
  documented origin-side truncation rather than a full cover;
- that a candidate scan whose cell count EXCEEDS `MAX_CELLS_PER_POLYGON` is clamped to a window
  around its focus rather than dropping its source, and that the clamp is applied **conditionally**
  — the span is computed first, so a scan within the cap is enumerated whole. State why the
  condition is load-bearing rather than an optimisation: the cap bounds a product of two cell
  counts while the window bounds a per-axis distance from a focus that is the source, so a box can
  reach far past the window on both axes and still fit under the cap, and clamping it would remove
  cells a player can currently move to;
- that `HexGrid::footprint_cells` measures distance to the hex polygon and is therefore valid for
  an off-centre anchor, which is what the continuous preview and the executor both pass, and that
  its failure directions are asymmetric — over-inclusion tightens the gates, under-inclusion
  loosens them;
- `GridKind` on `ResolvedScene` and `GridShape::kind` as the single grid-kind decision, and that
  it is what makes the visibility cache key and the explored blob kind-aware;
- the explored blob's self-describing header, its refuse-to-empty fail direction, and that a blob
  written before the header decodes empty by design;
- `execute_move`'s single footprint anchor, that it matches the continuous preview's, and that
  `cell_enterable` keeps a cell-centre anchor which agrees on every input the grid router emits;
- that the vision-mode and gradation resolvers stay silent by design and the undecodable-config
  diagnostic lives at the install sites — all six of them, including the config-Update mirror,
  which rewrites a stored engine without assigning the field and is therefore invisible to a search
  keyed on the assignment — because those resolvers run per token per visibility recompute.

Also correct the statements this phase falsified: the skill describes `blocks_move` as a primitive
`move_exec` parity is stated against, and describes `resolve_scene`'s `bounds` as feeding consumers
directly. Enumerate the skill's affected claims with a search for the symbols this phase changed;
a claim that is now false is a defect whether or not the diff touched it.

This is the reviewed skill-update gate: dispatch `shadowcat-spec-reviewer` to confirm the skill
diff accurately captures the change, with no omission, drift, or broken pointer.

- [ ] **Step 5: Bump the plugin version**

Increment `version` in `.claude/.claude-plugin/plugin.json`. A directory-sourced plugin serves a
cached snapshot, so without the bump the skill edit reaches no consuming repo and a stale copy is
indistinguishable from a current one. Report to the user that
`claude plugin marketplace update shadowcat` then
`claude plugin update shadowcat-codebase@shadowcat --scope project` must be run in each consuming
repo — a shell action outside this task, and one that fails naming the plugin rather than the
resolution if the qualified form is not used.

- [ ] **Step 6: Refresh the knowledge graph**

Run: `graphify update .`

- [ ] **Step 7: Run the full gate on both toolchains**

Run: `pnpm build && pnpm -r test && pnpm -r typecheck && pnpm lint && pnpm lint:allowances && pnpm lint:comments`
Run: `cd src/server && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

The client build must precede any cargo build — the embed validates `dist/` at compile time.
This phase changes no client source, but `lint:comments` and `lint:allowances` are repo-wide
gates and the tracker and skill edits above are in their scope.

RUN, OBSERVE, RECORD every command's outcome. No phase reports green without the command output.

- [ ] **Step 8: Confirm no measurement artifact survives**

Run: `cd /c/Dev/Shadowcat && git status --short && git branch --list "phase2-extent-probe" && ls debug/dumps`

The probe branch must not exist and `debug/dumps` must hold no extent-probe file. A probe left
behind is indistinguishable from landed code the next time someone measures.

- [ ] **Step 9: Commit and merge**

```bash
git add docs .claude
git commit -m "docs(phase2): sync trackers, skill, and plugin version

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>" -- docs .claude
git checkout main
git merge --no-ff phase2-server-scene-geometry
```

Merge only after the whole-branch review returns clean. Do not push — the push gate is the full
sub-project.

---

## Self-Review

**Spec coverage.** Every id in the phase's fixed input list has a task and a disposition:

| Id | Task | Disposition shape |
|---|---|---|
| PW1 | 5 | Fixed. One shared `world_extent`; every consumer converted; every fixture the pre-dispatch measurement names re-derived. |
| PW2 | 5, 6 | Fixed. One shared `world_units_per_cell`; the continuous cost and four sibling sites converted. |
| PW3 | 7 | Test added, mutation-proven against the clip. |
| PW4 | 8 | **Verified already built**; the superseded finding corrected. Its residual conversion defect is fixed in Task 5. |
| PW5 | 9 | **Mechanism moved**; the live silent-fallback defect fixed, with its sibling, at the install sites. |
| PW31 | 1 | Verified not subsumed; the drift the tolerance exists for removed at its cause. |
| TD17 | 10 | Fixed. |
| TD18 | 10 | Deleted, per §6, after confirming no production caller. |
| TD19 | 11 | Fixed to the anchor the continuous preview already uses, after verifying the counter-argument its own comment records on both grid kinds. |
| TD48 | 2 | Fixed, with NEW-6, through one resolved `GridKind`. |
| NEW-6 | 2 | Fixed by `ResolvedScene.grid_kind`; no snapshot field added. |
| NEW-7 | 9 | Fixed with PW5, same policy, at all six install sites, each held by its own test and its own mutation. |
| NEW-8 | 3 | Fixed: an over-cap scan clamps rather than dropping its source, and the clamp is inert below the cap by code rather than by argument. |
| NEW-9 | 4 | Fixed: the hex footprint predicate measures distance to the hex polygon. |
| NEW-10 | 6 | Fixed: the actor-size unit docs corrected to grid units. |
| NEW-11 | 9 | Closed by derivation: both callers read one resolver and the decode is cached, so the marker's premise no longer holds; the marker is replaced by the constraint. |

The spec's §5 requirements for this phase are all present: PW1/PW2 fixed as **one** change through
shared symbols with no call site keeping its own conversion (Tasks 5, 6); the unit question
resolved before the conversion, per §6, restated from evidence rather than assumed (Task 5's
preamble); PW3's non-GM end-to-end test in the same phase as the conversion (Task 7); PW4 treated
as unblocked by the existence of scene bounds (Task 8 — and found already built); PW31 re-verified
**before** any new work (Task 1 runs first). §6's TD18 fork is applied, not re-litigated.

**Placeholder scan.** No "TBD", no "add appropriate error handling", no "similar to Task N". Every
elided region is a fixture assembly that must be written against the module's real helpers, is
marked in place with a `// ...` line stating what it must build, and is named here — a search for
`// ...` in this document returns exactly these seven, plus the two occurrences inside this
paragraph:

| Location | What must be read first |
|---|---|
| Task 2 Step 2, the cached-mask test | the module's `apply_op` scene-mutation and `visible_cells_cached` test helpers |
| Task 6 Step 3, the hex vision-range test | the module's existing bounded-range vision test, for how a range-carrying mode is authored |
| Task 6 Step 3, the lit-egress range test | the same fixture as the test above |
| Task 6 Step 3, the hex light-radius test | the module's existing lit-scene helper, for the light document's shape |
| Task 6 Step 3, the animation-duration test | `ws::room`'s existing `execute_move`-based tests, for the handle construction |
| Task 7 Step 2 | `hex_continuous_scene_docs`, `hex_open_scene`, `continuous_world_settings`, and the existing non-GM hex mask test (Step 1 requires reading them before writing) |
| Task 11 Step 5 | the module's existing continuous-executor fixture, for `MoveGateInputs` construction, plus the coordinates derived in Step 3 |

Nine further steps instruct the implementer to read surrounding code rather than trust a literal,
each saying so explicitly and giving the shape: `HexGrid`'s existing oracle and PRNG (Task 1
Step 3), `resolve_grid_shape_with_rule`'s real lookup body and `eng::Grid`'s `kind` field type
(Task 2 Step 4), the real receiver expression at each clamp call site (Task 3 Step 5),
`scene_grid_sizes`' handling of a non-positive size (Task 5 Step 11), the
`region_doc_top`/`entity_doc_top_eng` fixture signatures (Task 5 Step 10), `point_qualifies`' and
`cell_illumination`'s caller enumerations (Task 6 Step 5), `resolve_token_footprint`'s and
`resolveTokenBox`'s unit readings (Task 6 Step 7), `lighting::default_bands` and the
config-document test helpers (Task 9 Step 2), and `apply_op`'s real config arms (Task 9 Step 4).
Each is a verification instruction, not a gap.

**Type consistency.** Every symbol this plan introduces is defined once and used with the same
name and arity everywhere:
- `GridShape::world_units_per_cell` and `GridShape::world_extent` — defined in Task 5 Step 3; used
  at Task 5's `navmesh_for`, `lighting_inputs`, `visible_cells_cached`, `scene_world_extent`,
  `source_los_poly`'s two callers and `pathfind`, and at Task 6's four Role B sites. Task 6 Step 1
  requires that list be rebuilt from a source search rather than copied from here.
- `GridShape::kind` — defined in Task 2 Step 5; used at `enrich_vision_explored` and in two tests.
- `GridKind` — variants `Square`/`Hex` in its definition, in `grid_kind_from`, `resolve_grid_kind`,
  `kind_tag`, `resolve_grid_shape_with_rule`, `resolve_scene`, both `GridShape::kind` impls, and
  every test that names them.
- `grid_kind_from(Option<&eng::SceneEngine>) -> GridKind` — defined in Task 2 Step 4; called by
  `resolve_scene` and `resolve_grid_kind`, the only two readers.
- `ExploredSet::to_bytes`/`from_bytes` — new signatures stated once in Task 2's Interfaces block
  and used at all five production sites plus the test sites Step 7 enumerates.
- `explored::SCAN_WINDOW_HALF_CELLS` and `explored::clamp_scan_window` — defined in Task 3 Step 5;
  `clamp_scan_window` takes six parameters in its definition, at all three call sites
  (`mark_polygons`, `player_lit_mask`, `accumulate_visible_cells`) and in the three tests that call
  it directly, each passing a `&dyn GridShape` first and `MAX_CELLS_PER_POLYGON` last. Two further
  tests read only `SCAN_WINDOW_HALF_CELLS` (the square and hex cap bounds), and the remaining
  coverage reaches the function through `mark_polygons` and the two mask paths rather than calling
  it.
- `HexGrid::distance_to_cell_polygon` — defined in Task 4 Step 3; called only from
  `HexGrid::footprint_cells`.
- `build_navmesh`'s three-argument signature — stated once in Task 5 Step 4 and used at its single
  production caller; its test callers are enumerated in the same step.
- `SceneEcs::scene_world_extent` returns `(f64, f64)` in its definition and at its two call sites.
- `warn_if_config_engine_undecodable::<T>(&Document)` — defined in Task 9 Step 4; called **six**
  times (two document kinds × three install sites: room hydration, the `Create` arm, and the
  config-Update mirror), instantiated at `eng::LightGradationEngine` and `eng::VisionModesEngine`,
  both of which `resolved_bands`/`resolved_vision_modes` already name. Six and not four because the
  update mirror rewrites a stored engine without assigning the field, so it is invisible to a
  search shaped like an assignment; Task 9 Step 1 enumerates by effect for that reason.
- `captured_warns(impl FnOnce()) -> usize` — defined in Task 9 Step 2 on the module's existing
  `captured_levels`; called twice in each of Task 9's six install-site tests. The four
  config-document fixtures it takes (`undecodable_gradation_doc`, `decodable_gradation_doc`,
  `undecodable_vision_modes_doc`, `decodable_vision_modes_doc`) each take a `u128` and return a
  `Document`, and are defined once in that same step.
- `lighting::cell_illumination`'s and `point_qualifies`' renamed parameters keep their `f64` type,
  which is precisely why Task 6 Step 5 enumerates their callers from source rather than trusting
  the compiler.

**Test discrimination.** Every test in this plan names the production edit that makes it fail AND
reaches that edit through its own call path. Five carry an explicit anti-vacuity guard because the
assertion alone could pass on a degenerate fixture: Task 7's mask guards (an empty or all-visible
mask fails the guard, not the assertion), Task 11's paired refused/admitted assertions (a gate that
refuses everything fails the second), Task 2's `assert_ne!` on the two masks (a kind change
producing no geometric difference fails the guard), and Task 3's two window tests — the under-cap
one guards that the box genuinely reaches past the window, and the over-cap pair brackets from both
sides so neither an unclamped nor an over-clamped scan passes.

No assertion in this plan sits on a floating-point equality boundary. The two places where one
could — Task 4's centre-anchored inradius bracket and Task 6's hex range and radius brackets —
place both probes a stated half-unit or half-cell off the threshold, and each test's comment says
so, because a hex two grid steps out is exactly 2.0 grid steps and a threshold of 2.0 would make
the comparison an equality between computed doubles.

Three tests are deliberately *not* written the way an earlier draft had them, and the reason is
recorded so it is not undone: a test that passes an already-converted value into a helper the task
does not change cannot discriminate, however its comment is worded. Task 6's light-radius and
vision-range tests therefore run through `player_lit_mask` and `visible_cells` rather than calling
`cell_illumination` or `footprint_cells` directly, and its animation test asserts a duration
derived from the authored speed rather than from the distance the executor returns.

**Mutation coverage.** Every task that changes production behaviour carries a mutation step, and
each names the per-site detection it requires rather than "the suite goes red": Task 1 (the tie
branch), Task 2 (three guards), Task 3 (two families — wiring at all three sites, and the
conditionality itself), Task 4 (the polygon branch), Task 5 (three formulas), Task 6 (**five**
mutations over four converted sites — `point_qualifies`' two independent uses of the scalar are
mutated separately, because mutating both at once cannot distinguish a full conversion from a half
one), Task 7 (the clip), Task 9 (six install-site calls, one per site, plus the update mirror's id
guard), Task 11 (the anchor). Task 6 additionally carries a grep proving the four non-conversions
stayed put; that is the complement of the mutation step, not a substitute for it, and the plan says
so at the step. Tasks 8, 10 and 12 change no production behaviour — a record correction, a binding
reuse plus a deletion the compiler proves, and the closeout.

Task 9 is in the first list and not the second, which is the accounting an earlier draft of this
review got wrong by placing it in neither. It DOES change production behaviour: its only
production edit is the diagnostic, its three fallback-value tests all pin values the task leaves
unchanged, and a per-site mutation step is therefore the only thing that can show the edit landed
at all. Its fallback-value tests remain necessary for the opposite reason — they are what makes the
change provably behaviour-preserving rather than believed to be.

**No predicted outputs.** No step states which tests will fail or how many. Every verification step
says RUN, OBSERVE, RECORD, and every mutation check carries the unwinding caveat plus the
revert-and-diff confirmation that a mutation which never landed and a gate that does not gate
produce identical output. Task 1's staircase test carries an explicit SCOPE paragraph stating what
it does *not* reproduce, so it cannot be read downstream as a reproducer.

**No anchored derivations or measurements.** Seven steps ask the implementer to derive or measure
something the plan also has a reading of. In each, the question and the method come first, the
plan's reading is quarantined in a following step, and a disagreement is a stop-and-report rather
than a value to reconcile: Task 1 Steps 1–2 (whether the shipped fix subsumes PW31), Task 3 Step 4
(whether the over-cap fixture is over-cap at this task's position — no plan-side number at all),
Task 5 Steps 8–9 (the fixture classification and the hex coordinates, the last of which is the
dual-kind fixture's own hex extent), Task 6 Steps 1–2 (the role classification), Task 9 Steps 6–7
(the double-resolve premise), and Task 11 Steps 1–2 (the boundary degeneracy, on both grid kinds)
and Steps 3–4 (the executor test's coordinates). Task 3
Step 1's cost measurement has no plan-side number either — it states the derivation to perform and
the threshold at which a result becomes a finding. The Pre-dispatch measurement section likewise
states the probe and the recording format and supplies no expected failure list.

