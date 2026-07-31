# Docs Sweep 8 — `@shadowcat/render` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development.
> **REQUIRED READING for every implementer and reviewer:** `docs/design/doc-sweep-truthfulness-rules.md`
> — hand it over BY PATH, do not paste it into a brief (pasting is how the rules drift between
> dispatches).

**Goal:** Drive `@shadowcat/render` from 339 `lint:docs` warnings to 0, then ratchet the package to
`error` in `eslint.docs.config.js`.

**Architecture:** Six content tasks grouped by subsystem cohesion, then a ratchet-and-ship task.
Same shape as Sweep 7 (client/core, 620→0).

**Tech Stack:** TypeScript, PixiJS, ESLint + `eslint-plugin-jsdoc`, Vitest.

## Global Constraints

- **Comment-only.** No runtime change in any content task. The only non-comment edit in this plan is
  `eslint.docs.config.js` in Task 7. Any other non-comment line is a defect.
- **Gate:** `npx eslint --config eslint.docs.config.js src/client/render/src/<file>` per file, and a
  whole-package sweep before each task reports. Task 7's ratchet fails on ANY file left above 0,
  including one not enumerated here — enumerate live counts, never trust this document's numbers.
- **Per-task gates:** scoped counts at 0, `pnpm -r typecheck`, `pnpm --filter @shadowcat/render test`,
  `pnpm docs:check-examples`, `pnpm lint`.
- **`@example` fences:** ```ts fences are EXTRACTED and typechecked by `pnpm docs:check-examples` —
  self-contained, import by workspace package name (`@shadowcat/render`), `declare const x: Type;`
  for unconstructible deps (a PixiJS `Application`, a WebGL context). Anything NOT exported from the
  package `index.ts` MUST use an UNTAGGED ``` fence.
- **Staging:** stage only the files you edited, by explicit path. Never `git add -A` — other agents
  may share the tree.
- **Report contract:** every report carries a CLAIMS TABLE (claim → enforcing `file:line`) and an
  explicit statement of what the pre-existing-prose re-scan covered ("none found" is acceptable only
  with a list; that verdict was wrong three times in Sweep 7).
- **Check the already-clean files too (Rule 10).** This task list is built from the warning census, so
  files already at 0 are invisible to it — and that is where stale claims hide, because nothing forces
  anyone to open them. When you document a symbol, grep the package for the same claim and check the
  **interface** the symbol implements (`types.ts` especially: it has zero warnings and no task owns
  it). Task 1 found the same false `serverNow` sentence in `token-animator.ts` AND `types.ts`; the
  latter would have survived the whole sweep, leaving the interface contradicting its implementation.

## Why this package is harder than client/core

`client/core` is headless logic. `render` owns the PixiJS layer, which means most claims are about
**observable visual/temporal behavior** rather than data flow — draw order, frame timing, teardown,
resource lifetime. Those are easy to state plausibly and hard to verify, which is exactly the profile
that produced Sweep 7's eight fix rounds. Two specific traps:

- **`pixi-backend.ts` and `backend.mock.ts` are a NEVER-FORK-A-DECISION pair.** The mock exists so
  tests exercise the real backend's contract. Any documented behavior must hold for BOTH, or the doc
  must say explicitly which one it describes and how they differ. This is the single highest-risk
  file pair in the sweep — treat a "the backend does X" sentence as a claim about two files.
- **Resource lifetime and teardown claims.** "Destroyed on scene change", "released when the layer
  unmounts", "pooled and reused" — each is a claim about a specific `destroy()`/`removeChild` call
  site. Cite it or do not write it.

---

### Task 1: `engine.ts` (79)

The render-layer API — the seam modules program against, so its docs are the most externally
load-bearing in the package.

- [ ] Enumerate live count. Document every symbol. Gates. Commit.

**Hot spots:** the layer registration/ordering contract (what guarantees a module gets about draw
order relative to engine-owned layers); the scene-swap lifecycle (what is torn down vs retained);
which operations are safe before first render. Verify each against the implementation, not the
neighbouring prose.

### Task 2: `pixi-backend.ts` (55) + `backend.mock.ts` (26) = 81

- [ ] Enumerate live counts. Document both. Gates. Commit.

**Hot spots:** the fork risk above. For every behavioral claim, check the mock actually mirrors it —
and where it deliberately does NOT (a no-op stub, a synchronous stand-in for an async GPU path), say
so. A mock documented as equivalent when it is an approximation misleads every test author who reads
it. Also: any claim about what the real backend does with GPU resources needs its `destroy()` site.

### Task 3: `geometry.ts` (35) + `grid.ts` (13) + `camera.ts` (8) = 56

- [ ] Enumerate live counts. Document all three. Gates. Commit.

**Hot spots:** hex metrics. Sweep 2b shipped a wrong claim here once — **hex `size` is the
CIRCUMRADIUS (outer radius), not across-flats**; verify against `Grid.ts` and the server's
`scene.rs`, which must agree. Also the square-vs-hex indexing split (the codebase's worst historical
defect class), and camera transform conventions (screen↔world direction, y-axis sign) — state the
direction explicitly, since an inverted transform doc is invisible until someone builds on it.

### Task 4: `token-view.ts` (24) + `token-animator.ts` (24) + `token-animation.ts` (6) = 54

- [ ] Enumerate live counts. Document all three. Gates. Commit.

**Hot spots:** animation completion/cancellation semantics. A known defect class here is that
**async-completion guards need object identity, not just a key** — a Pixi object recreated mid-flight
(kind-swap) defeats key-equality staleness checks. If the docs describe a staleness guard, state
precisely what it compares. Also: what happens to an in-flight animation on token deletion, scene
change, or a second move arriving mid-animation.

### Task 5: `lighting.ts` (15) + `fog-blend.ts` (11) + `compositor.ts` (8) + `layers.ts` (6) = 40

- [ ] Enumerate live counts. Document all four. Gates. Commit.

**Hot spots:** **fog is the secrecy gate and must fail CLOSED** — a client-side visibility gate must
hide everything on a missing or garbled signal. If a doc describes fog/vision behavior on absent
data, verify the code actually fails closed rather than defaulting to visible. Do not document a
fail-open default as if it were intentional without citing it. Also: blend-mode and compositing order
claims need their enforcing call site.

### Task 6: the remaining 8 small files (29)

`easing.ts` (7), `ping-view.ts` (7), `drawing-view.ts` (3), `region-view.ts` (3), `template-view.ts`
(3), `wall-view.ts` (3), `reconciler.ts` (2), `scene-scope.ts` (1).

- [ ] Enumerate live counts. Document all. **Run a whole-package sweep** — the package total must
  reach 0, including any file not listed here. Gates. Commit.

**Hot spots:** `reconciler.ts` — diff/patch semantics against the document store; state what triggers
a rebuild vs an in-place update. The `*-view.ts` files are thin, but check each one's claim about
which doc fields it reads.

### Task 7: Ratchet + verify + sync + ship

- [ ] Add `src/client/render/**/*.ts` to the ratcheted `error` block in `eslint.docs.config.js`
  (the block already exists from Sweep 7; keep its `files`/`ignores` symmetric with the warn block).
- [ ] **Mutation proof:** delete one doc comment from a render file → `pnpm lint:docs` exits nonzero
  naming that file → restore. A ratchet nobody proved may not be wired.
- [ ] **Manual scan for orphaned doc blocks** — a `*/` line immediately followed by `/**`. `lint:docs`
  CANNOT detect this (it gates tag presence; jsdoc/TypeDoc/hover bind to the nearest preceding
  block, so an appended block satisfies the linter while orphaning the richer one above it). Found in
  Sweep 7's `transport.ts` at 0 warnings throughout.
- [ ] Full local matrix (CI is not watchable; this run IS the gate): `pnpm -r typecheck`, `pnpm -r test`,
  `pnpm lint`, `pnpm lint:docs`, `pnpm docs:check-examples`, `pnpm build`, then server `cargo fmt --check`,
  `cargo clippy --all-targets -D warnings`, `cargo test` (build `dist/` BEFORE any cargo — `rust-embed`
  validates `../../dist/` at COMPILE time).
- [ ] Docs-sync: `docs/PLAN.md`. Log any deferral to `docs/TODO.md`; bugs (if any) to `docs/OPEN_BUGS.md`.
- [ ] **Reviewed skill-update gate:** update `shadowcat-codebase-scene-rendering` (and any other skill
  whose seam/invariant/gotcha this sweep touches), then dispatch `shadowcat-spec-reviewer` on the
  SKILL DIFF. If nothing needs updating, state that explicitly rather than skipping silently.
- [ ] Whole-branch review pair (no-shell, pre-generated diff), fix findings, merge `--ff-only`, push,
  delete branch, memory update.

---

## Deferred (logged, not dropped)

- Sweep 9: shell + ui-kit + formula (281). Sweeps 10–11: module packages (~530).
- Then: per-crate buddy-check convergence → final ratchet (crate-root deny,
  `treatValidationWarningsAsErrors`, docs lint merged into `eslint.config.js`) → skills
  documentation-reference pass.
- Sweep 9 must also document `worldSession.canEdit`'s `gm_role` caveat — see `docs/TODO.md`.
