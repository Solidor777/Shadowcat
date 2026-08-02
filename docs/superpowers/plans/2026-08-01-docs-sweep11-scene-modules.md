# Docs Sweep 11 — scene-adjacent modules Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development.
> **REQUIRED READING for every implementer and reviewer:** `docs/design/doc-sweep-truthfulness-rules.md`
> — hand it over BY PATH, never pasted into a brief (pasting is how the rules drift between
> dispatches).

**Goal:** Drive six module packages — `scene-tools` (107), `actors` (17), `scene-browser` (9),
`conditions` (8), `factions` (8), `stage` (8) — from 157 `lint:docs` warnings to 0, then ratchet all
six to `error` in `eslint.docs.config.js` (BOTH the `.ts` and the `.svelte` block).

**Architecture:** Three content tasks on `scene-tools/src/controller.svelte.ts` (934 lines, 101 of
the 157 warnings — the sweep is 68% one file), split by tool family; two tasks for the eleven small
files across the other five packages; then ratchet-and-ship. Same shape as Sweeps 7 (620→0),
8 (339→0), 9 (276→0), 10 (217→0).

**Tech Stack:** TypeScript, Svelte 5 runes (`.svelte.ts` rune modules), PixiJS via
`@shadowcat/render`, ESLint + `eslint-plugin-jsdoc`, Vitest.

## Global Constraints

- **Comment-only.** No runtime change in a content task. If you find a real defect, report it with
  reachability bounded — do not fix runtime code in a docs task. (Sweep 8 found a rendering bug, two
  docs-gate defects, and a sibling divergence exactly this way; Sweep 9 found a silent-no-op branch;
  Sweep 10 found a dead seam.) Correcting a **stale comment** is not a runtime change and IS in
  scope — see Rule 7.
- **Gate:** `npx eslint --config eslint.docs.config.js <file>` per file, plus a whole-package sweep
  before each task reports. The ratchet fails on ANY file left above 0, including one not enumerated
  here — enumerate live counts, never trust this document's numbers.
- **Per-task gates:** scoped counts at 0, `pnpm -r typecheck`, the package's own tests
  (`pnpm --filter @shadowcat/module-<pkg> test`), `pnpm docs:check-examples`, `pnpm lint`.
- **Reports:** write to `.superpowers/sdd/2026-08-01-docs-sweep11-scene-modules/task-N-report.md`
  and **`ls` it to confirm it exists before reporting back.** A Sweep-8 task reported writing one and
  had not; both reviewers lost their claims-table audit and that task's citation metrics are
  unrecoverable.
- **Staging:** stage only the files you edited, by explicit path. Never `git add -A`.
- **Reviewer dispatch contract (dispatcher-side, every task):** reviewers run read-only —
  Read/Grep/Glob/Skill, **no Write and no Bash** by standing user directive. So (a) pre-generate the
  diff to a file and hand over the path, and (b) **name the delivery channel explicitly: "send your
  findings back with SendMessage."** Omitting (b) makes a reviewer go idle having produced nothing
  recoverable — it happened on Task 1 despite the lesson already being in memory, which is why it
  is written down here where the next task boundary will surface it.
- **The Rule 7 re-scan covers EVERY comment in range, not just the JSDoc blocks.** Found on Task 2:
  the re-scan lists had been implicitly scoped to `/** */` blocks — the things `lint:docs` counts —
  while standalone inline `//` comments inside function bodies went un-enumerated. Those carry some
  of the load-bearing claims in this package (`requestRoute`'s arrest-marker and budget-rounding
  notes, `commitRoute`'s "no `movementModel` branch anywhere" claim, the `committing` field
  comment). Three spot-checks came back true, so nothing false is known to be hiding — but "none
  found" is worthless over a set nobody enumerated. **Inventory the inline comments explicitly and
  list them.** Same failure shape as Task 1's `regionShapePath` omission, one level down.
- **Report contract:** a CLAIMS TABLE (claim → enforcing `file:line`, **path-qualified**, Rule 13),
  an explicit list of what the pre-existing-prose re-scan covered ("none found" only with a list),
  and any bug found with its reachability bounded.

## `@example` fences: UNTAGGED, everywhere in this sweep

This is the single structural fact that separates Sweep 11 from Sweep 10, and getting it wrong
fails `pnpm docs:check-examples` at the import line.

**Each of these six packages exports exactly ONE symbol from its `index.ts`: the module manifest
const** (`sceneTools`, `actors`, `sceneBrowser`, `conditions`, `factions`, `stage`). Verified by
reading all six files in full. There are **no** re-exports — not `ToolController`, not
`makeSelectMoveTool`, not `topTokenAt`, not `seedFactionRegistryIfAbsent`, not one component. Panels
re-exported a wide surface and could use ```ts fences for most of its sweep; **here nothing outside
`index.ts` is reachable by workspace package name.**

Therefore: **every `@example` fence in this sweep is UNTAGGED** (``` … ```), following the
established private-function idiom — a one-line note that the symbol is module-private plus the call
shape. `src/modules/panels/src/PanelHost.svelte:52-58` is the reference example.

- ```ts fences are EXTRACTED and typechecked; untagged are not
  (`scripts/extract-ts-examples.mjs:23`). `.svelte` files are never scanned; `.ts` **and
  `.svelte.ts`** files are (`candidateFiles`, `scripts/extract-ts-examples.mjs:99`) — so
  `controller.svelte.ts`, `hit-test.ts` and `seed.ts` DO get their fences extracted if tagged.
- **Baseline is 332 examples** (`pnpm docs:check-examples`). This sweep must leave it at **exactly
  332.** A RISE means someone used a ```ts fence, which in these packages can only typecheck by
  scaffolding `declare const` stand-ins around an unimportable symbol — an example that demonstrates
  nothing. A DROP means something silently stopped being extracted. Either way: investigate, do not
  proceed.
- The manifest consts in `index.ts` are the one importable surface, but `require-jsdoc` does not gate
  a `const` — all six `index.ts` files are already at 0 and need no new example.

## What makes this sweep different from the last four

The client packages were a library, a renderer, a reactive shell, and a layout state machine. This
is **the client's authoring surface for server-authoritative state**, and every doc failure mode here
is a variant of one question: *who has authority, and does this code path merely predict it?*

- **Movement authority is ROLE-ASYMMETRIC, and the asymmetry is a security invariant.**
  `commitMoves` (`src/modules/scene-tools/src/controller.svelte.ts:856-890`) branches on `ctx.role`:
  a GM writes `/engine/x`,`/engine/y` directly via `dispatchIntent`; a **player's move is
  request-only** — per-token `pathfind` then `moveRequest`, and the token's rendered position
  advances only when the resulting `MoveStream` arrives. `previewMoves`
  (`controller.svelte.ts:835-848`) is drag feedback only: never a document write, never a move
  request. A doc that describes the drag as optimistically moving a player's token is not a wording
  slip — it contradicts [[server-authoritative-movement-rule]] and would invite someone to "restore"
  optimistic prediction on a gated path. State the branch, name both sides.
- **The client's `footprintRadius` is DISCARDED by the server at every production call site.**
  `ws/protocol.rs:113-117` and `ws/conn.rs:681-700`: when a `Pathfind` frame names a `token`, the
  server authorizes it (effective ownership + scene membership) and **derives** the footprint,
  ignoring the wire value entirely; a failed check returns the generic `PathError`, not a fallback to
  the wire value. The wire value is used **only** when `token` is absent. All three production call
  sites pass a token — `controller.svelte.ts:429` and `:558` (both gated on
  `tokenSelection.ids.size === 1`, so `selectedTokenId()` is never `undefined` there) and `:876`
  (passes `id` unconditionally). So `resolveFootprint()`/`footprintFor()` compute a value that
  **influences no production outcome.** Pre-existing prose at `controller.svelte.ts:402-403`
  ("Footprint radius of the single selected token (for pathfind clearance)") reads as though it does.
  This is a Rule 8 + Rule 7 item: document what the value IS, then say plainly that the server
  overrides it when a token is named, and cite `ws/conn.rs:681-700`. Do **not** silently delete the
  functions or call them dead code — they are live, and the `token`-absent contract still exists.
- **Client-side gating is ADVISORY; the server gates the DATA.** `ToolRail.svelte:65-79` already
  documents `gmOnly` precisely, including the non-obvious reason `place` must stay gated
  (`Room::publish`'s movement gate inspects `Operation::Update` only, so a `Create` carries an
  arbitrary initial position past it). **Preserve that framing; do not "improve" it.** But see
  Task 3's hot spot: one sentence in it needs verifying against the current `commitMoves`.
- **Every tool factory closes over `ctx` captured ONCE**, deliberately
  (`ToolRail.svelte:9-11`, with an explicit `svelte-ignore state_referenced_locally`).
  `makeSelectMoveTool` additionally binds `const sel = ctx.tokenSelection` at construction while
  `makeMeasureTool` reads `ctx.tokenSelection` live on each call. Both are correct given a stable
  context object — **say why in the docs rather than implying one form is safer**, and do not
  describe either as a reactive subscription. It is not one.
- **Two coalescing windows, both leading-edge, and only one has a trailing fire.**
  `DRAG_THROTTLE_MS` (`controller.svelte.ts:793`) is pure leading-edge. `ROUTE_PREVIEW_DEBOUNCE_MS`
  (`:101`) is leading-edge **plus** a deferred trailing fire (`schedulePendingRouteFire`/
  `firePendingRoute`, `:485-492`) that exists because a hover-only stop produces no further
  `onPointerMove`. "At most one request per window" is therefore **false** for the route preview —
  the deferred fire adds one after the window closes. Get this right per-constant;
  [[debounce-leading-edge-not-trailing-rearm]] is why the leading-edge half matters.
- **The `committing` flag is a stated invariant with an ordering requirement.**
  `controller.svelte.ts:513-517` spells it out: set TRUE before `seq` is captured, cleared ONLY by
  `finish()`, the reject handler, or `onDeactivate`. `onDeactivate` clears it *before* `clearRoute()`
  precisely so the bumped `pendingSeq` aborts an in-flight commit. Verify each clause still holds
  before restating it, and do not compress the ordering out.

---

### Task 1: `controller.svelte.ts` part A — module helpers, `ToolController`, and the
authoring tools (44)

Lines 1–325: `activeScene`, `footprintFor`, the `ToolController` class (constructor + `toggle`),
`makePlaceTool`, `hasExtent`, `makeWallTool`, `makeRegionTool`, `regionShapePath`,
`regionShapeGeometry`, `makePingTool`. 13 documentation sites.

- [ ] Enumerate live count. Document every symbol. Gates. Commit.

**Hot spots:**

- `activeScene`'s `?? 100` cell-size and `?? {perCell: 5, unit: "ft"}` distance defaults
  (`:74-75`). If you make ANY parity claim about the server, the only correct one is that
  `scene_grid_sizes` (`src/server/src/scene/mod.rs:1189`) is the server's **sole intentional
  defaulting source** and also defaults to 100. Do **not** claim the movement gates default — they
  refuse. **Cite the gates themselves:** `Room::publish`'s Create-placement scene-existence refusal
  (`src/server/src/ws/room.rs:333`) and `Room::execute_move`'s (`src/server/src/ws/room.rs:577-580`),
  both `DataError::Forbidden`. `scene/mod.rs:1319-1322` (`navmesh_for`) and `:1360-1362`
  (`pathfind`) are the **router**, not the gates — `scene/mod.rs:1355-1357` says so in as many
  words, so citing them for a claim about "the movement gates" is a Rule 1 adjacency defect on an
  otherwise true claim. The safest doc makes no parity claim at all; an uncited one is where the
  false claims live (Rule 2).
- `footprintFor` (`:84-88`) — the server-override fact above. Its existing comment's claim that
  module-level placement makes preview and commit "share one source" is true; verify it still is
  (both `makeMeasureTool` and `makeSelectMoveTool` must call it) before restating.
- `ToolController.toggle` — the mid-gesture-clear invariant: `onDeactivate` fires on the OUTGOING
  tool *before* `active` is updated, so the tool can still read state. Re-selecting the active tool
  clears to null (camera). Both halves matter.
- `makePlaceTool`'s actor-vs-asset precedence and the `link`/`instance` mode decision
  (`engine.prototype`), plus the deselect-after-place rule and its `keepAfterPlace` opt-out — state
  why a linked actor deselects and an instanced one does not.
- `makeWallTool`/`makeRegionTool` are **create-only by design** (no edit UI; a GM re-authors by
  delete+recreate). `makeRegionTool`'s existing comment says so and cites `makeWallTool` as the
  precedent — verify that is still accurate rather than repeating it.
- `regionShapePath` vs `regionShapeGeometry` — one produces PREVIEW points, the other the
  PERSISTED `engine.shape.points` layout, and they are deliberately different shapes for `circle`
  (a polygon ring vs `[cx, cy, r]`). Documenting them as interchangeable is a real defect. The
  persisted layout claim ("matches the server's region shape parser expectations") is load-bearing
  and currently uncited: **cite the server-side parser or drop the claim** (Rule 2/Rule 4).
- `makePingTool` broadcasts with no local echo — the server relays it back to all members including
  the sender. Say that; it explains the absence of a local-render path.

### Task 2: `controller.svelte.ts` part B — the measure tool (25)

Lines 327–661: `makeMeasureTool` and its inner functions `inRouteMode`, `tokenCenter`,
`resolveFootprint`, `selectedTokenId`, `requestRoute`, `clearPendingRouteTimer`, `firePendingRoute`,
`schedulePendingRouteFire`, `clearRoute`, `commitRoute`. 9 documentation sites, the densest
concurrency logic in the sweep.

- [ ] Enumerate live count. Document all. Gates. Commit.

**Hot spots:**

- **Route mode's three-way activation condition** (`:330-341`) is already stated as a numbered list.
  Verify each clause against `inRouteMode()` (`:385-391`) — note that the doc lists "there is an
  active scene" as clause 3 but `inRouteMode()` itself checks only `pathfind` and a
  single-token selection; the scene check lives at each call site. Say where each clause is
  actually enforced, or the list reads as one predicate that it is not.
- **Two independent staleness mechanisms, and they are not the same thing.** `pendingSeq` guards
  stale RESPONSES (last-write-wins). The debounce guards REQUEST volume. `requestRoute`'s comment
  and `:374-375` say this explicitly. A doc that merges them is wrong in a way that would let
  someone delete one.
- The deferred trailing fire (above): `firePendingRoute` re-checks `committing` and **re-resolves
  scene/start fresh** rather than trusting values captured at suppression time. That re-resolution
  is the point of the function — do not describe it as "fires the queued request."
- `commitRoute`'s `committing` ordering invariant (above), the `lastPreviewedPath` reuse path
  (avoids a second round-trip) versus the fallback pathfind, and the fact that **animation is
  broadcast-driven via `MoveStream` for all viewers — never animated from the `moveRequest` resolve
  value** (`:546-547`). The last is a correctness claim someone will get backwards.
- `scheduleTimeout`/`clearScheduledTimeout` injection: the existing `ToolContext` comment
  (`:36-42`) explains that a test faking `now` must also fake the timer or it arms a REAL timer that
  can fire during an unrelated later test. That is a hard-won constraint — preserve it verbatim in
  substance.
- `resolveFootprint` — apply the server-override framing from the Global section. `selectedTokenId`'s
  existing comment (`:411-413`) already has the correct framing; make the two consistent rather than
  leaving one saying the estimate is used and the other saying it is not.

### Task 3: `controller.svelte.ts` part C + `hit-test.ts` + `ToolRail.svelte` (38)

Lines 663–934 (`shapePath`, `makeDrawTool`, `templatePath`, `sizeDir`, `makeTemplateTool`,
`makeSelectMoveTool` + `centerOf`/`drawSelection`/`previewMoves`/`commitMoves`), plus
`hit-test.ts` (5) and `ToolRail.svelte` (1). Brings `scene-tools` to 0.

- [ ] Enumerate live counts. Document all. **Run a whole-package sweep** including `AssetPicker.svelte`
  and `index.ts` (both at 0 today — confirm, do not assume). Gates. Commit.

**Hot spots:**

- **The role branch in `commitMoves`** — the sweep's most important single doc. See the Global
  section. Cover both branches, the per-token (not batched) rationale, and why the `.catch()` drops
  the preview rather than swallowing: a refusal is the NORMAL outcome for a player, and
  `onMoveOutcome` at the Stage level is what surfaces it.
- **`ToolRail.svelte:67-69` needs verifying, not transcribing.** It says the three ungated tools each
  write "through a path the server already polices for a non-GM: select/move emits an optimistic
  `/engine/x,y` token Update (`Room::publish`'s wall + visibility-mask gate, then the permission
  check in `apply_intent`)". `controller.svelte.ts:856-890` branches on `ctx.role`. **Determine which
  path a NON-GM actually takes today and whether that sentence still describes it.** If it does not,
  it is stale pre-existing prose (Rule 7) — correct it, cite both branches, and report it as a
  finding. If it does, say what you checked. Do not edit the `place`-must-stay-gmOnly paragraph
  (`:72-79`) unless you can show it is wrong; it is the most carefully reasoned comment in the
  package.
- `drawSelection`'s shape agreement: a circle token gets an ellipse ring "so the ring, hit-test, and
  faction border agree on shape" — a three-way agreement claim (Rule 6). Either verify all three and
  cite each, or narrow the claim to what you checked.
- `hasExtent`/`sizeDir` degenerate-input handling: a pure click has no extent and persists nothing;
  a near-zero template drag falls back to one grid cell. Both are deliberate — say what would
  otherwise be written to the scene and event log.
- `shapePath` vs `templatePath` are siblings with different parameter conventions (two corners vs
  anchor+size+direction). **Rule 11 sibling audit:** `shapePath`, `templatePath`, `regionShapePath`,
  and `regionShapeGeometry` are four shape-builders in one file. Audit them as a set and report any
  divergence in closed/open handling or point layout that is not deliberate.
- `topTokenAt` (`hit-test.ts`): "topmost = last in document order (render z-order)" is a coupling
  claim between this file and the renderer — cite the render-layer ordering or narrow it.
  Rotation is ignored for picking, deliberately; degenerate boxes are skipped.
- `ToolRail.svelte`'s only warning is an `@example` on `toggleSnap`; its OCC `old`-value comment
  (`:55-59`) cites "controller.svelte.ts's sendMoves convention" — **there is no `sendMoves` symbol
  in that file today.** Verify and correct the reference (`commitMoves` is the likely intent) as a
  Rule 7 item.

### Task 4: `actors` (17) + `scene-browser` (9)

Five components, 26 warnings: `VisualKindEditor.svelte` (9), `FaceSwapPalette.svelte` (4),
`ActorsPanel.svelte` (2), `TokenOwnerControl.svelte` (2), `SceneBrowserPanel.svelte` (9).

- [ ] Enumerate live counts per file. Document all. Whole-package sweeps for both. Gates. Commit.

**Hot spots:**

- `TokenOwnerControl` writes `/owner`, which is gated by `cap::EDIT_PERMISSIONS` server-side, and
  token ownership is **EFFECTIVE** (a token's own `/owner`, else the linked actor's owner), resolved
  at authz time and never stamped. Any doc here that implies the control sets a stamped owner is
  wrong — see `shadowcat-codebase-actors-tokens` and cite the server rule rather than restating it
  from memory.
- `VisualKindEditor` authors the `TokenVisual` union. Document what each kind branch produces and
  which fields are cleared on a kind swap — a kind swap that leaves a stale sibling field is the
  failure mode [[async-completion-needs-object-identity-not-key]] came from.
- `SceneBrowserPanel`'s three verbs are genuinely different and get confused: **configure**
  (deep-links game-settings), **local view** (GM roam — changes only this client's viewed scene),
  and **activate** (sets the scene players render). `index.ts:5-7` states this correctly; make each
  handler's doc say which one it is and what scope the change has.
- Whether these panels' GM-gating is client-advisory: `scene-browser`'s contribution carries
  `panel: { gmOnly: true }` (`index.ts`). Same advisory framing as the tool rail — the panel
  registration hides the UI; the server gates the writes.

### Task 5: `stage` (8) + `conditions` (8) + `factions` (8)

Four files, 24 warnings: `Stage.svelte` (8), `ConditionsPanel.svelte` (8),
`FactionsPanel.svelte` (4), `seed.ts` (4).

- [ ] Enumerate live counts per file. Document all. Whole-package sweeps for all three. Gates. Commit.

**Hot spots:**

- `Stage.svelte` hosts the engine-owned PixiJS surface — the lifecycle is the doc. `wirePointer`
  (`:221`) takes `(engine, signal)`: the `AbortController` created in the `$effect` (`:79`) aborts
  every pointer/wheel listener on teardown *and on any `$effect` re-run*, so a stale listener set
  can never call into a destroyed engine. Say that, and say what the `disposed` flag at `:69`/`:83`
  guards — teardown racing the async backend init is a real, handled case
  ([[refactor-async-contribution-paint-timing]] is the adjacent failure mode).
- **`readColor(token, fallback)` (`:54`) takes a CSS CUSTOM PROPERTY name, not a game token.**
  `token` here means design token (`--surface-base`, `--grid-line`). It reads the computed `color`
  off a throwaway probe span because `getPropertyValue` returns the unresolved `var(...)` string for
  aliased custom properties — that indirection IS the reason the function exists. Name the two
  fallback triggers precisely: no `getComputedStyle`/no `host` (`:55`), and an unparseable
  `rgb()` result (`:63`).
- `applyGmView` (`:38`) covers three modes — `"all"`, `"fog"` (client-only full-fog preview) and
  `"as:<userId>"` (see-as-player, **server-gated to GMs**). The fog preview is client-only; the
  see-as re-subscription is the server-gated one. Do not describe them as the same kind of switch,
  and note it is re-applied after an `$effect` re-run (`:103`) so a non-default view survives.
- `seed.ts` carries the sweep's best pre-existing comment (`:3-8`, `:15-23`): the deterministic-id
  race between two GMs seeding one world, why the loser needs no conflict handling, and why
  `dispatchIntent` is fire-and-forget by design. **Verify each clause** — particularly
  "`envelope()` stamps `created_at`/`updated_at` via `Date.now()` per call" and the claim that the
  server's singleton create-gate is **doc_type-scoped, not id-scoped** — then preserve it. That
  second claim is checkable in one grep (`SINGLETON_DOC_TYPES`, `src/server/src/data/sqlite.rs`) and
  is exactly the kind of load-bearing detail Rule 2 exists for. `SEED` is a `const` and needs no
  doc; `seedFactionRegistryIfAbsent` needs `@param`×3 + `@example` (untagged).
- `ConditionsPanel`'s two roles in one component — the GM registry editor and the
  selection-driven toggle palette — plus the idempotent GM seed. `index.ts:4-7` says the module is
  **replaceable** by a game-system module; that is a modding-contract claim worth stating on the seed
  function, not just the manifest.
- `FactionsPanel` and `ConditionsPanel` are near-identical registry editors (**Rule 11 sibling
  audit**): audit them as a pair and report any divergence in seeding, OCC pre-image handling, or
  GM gating that is not deliberate.

### Task 6: Ratchet + verify + sync + ship

- [ ] Add all six packages to the ratcheted `error` **`.ts`** block and all six to the ratcheted
  **`.svelte`** block in `eslint.docs.config.js`. Both are required — a package with components
  ratchets in TWO places (one flat-config block cannot carry two parsers). Keep the two ignore lists
  byte-identical to the warn blocks'. Six packages × two blocks: **verify each of the twelve globs
  individually**, not the aggregate count.
- [ ] **Mutation proof, one per newly-ratcheted BLOCK** (`.ts` and `.svelte` separately): delete a
  doc comment → `pnpm lint:docs` exits nonzero naming that file → restore. Target a
  `FunctionDeclaration`, `MethodDefinition`, or `ClassDeclaration` — **`require-jsdoc` does NOT gate
  a `const` arrow function**, so a probe aimed at one returns a false green (this cost two wasted
  probes in Sweep 9). Note `makeSelectMoveTool`'s inner helpers are `const` arrows: probe a
  `function` declaration such as `activeScene` or `topTokenAt`.
- [ ] **Manual scan for orphaned doc blocks** (`*/` immediately followed by `/**`) — **repo-wide, not
  sweep-scoped.** `lint:docs` cannot detect this. Sweep 9's scoped scan found 1; widening it found 4,
  three of them inside already-ratcheted packages.
- [ ] **Inline-comment inventory pass over the six swept packages.** Every sweep so far has scoped
  its Rule 7 re-scan to `/** */` blocks, because those are what `lint:docs` counts — so standalone
  `//` comments inside function bodies have never been systematically re-verified in ANY sweep,
  including the nine already merged. Enumerate them here and check the load-bearing ones.
- [ ] **Promote the above to RULE 14** in `docs/design/doc-sweep-truthfulness-rules.md`: a green
  `lint:docs` scopes the eye to what it counts, and the re-scan inherits that scope silently. State
  it as its own rule rather than leaving it as this plan's footnote — Sweeps 12+ need it, and the
  nine merged sweeps are all exposed to it.
- [ ] **Citation audit, repo-scoped over the sweep's packages** — print every `file:line` citation in
  the branch's claims tables beside the line it actually lands on (Rule 13). Sweep 10's tool scanned
  ONE file and declared it clean while three bad citations survived in siblings: **a verification
  tool's scope is part of its correctness.**
- [ ] Full local matrix: `pnpm -r typecheck`, `pnpm -r test`, `pnpm lint`, `pnpm lint:docs`,
  `pnpm docs:check-examples` (**expect exactly 332**), `pnpm build`, then server
  `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, `cargo test` (build `dist/` BEFORE
  any cargo — embed ordering).
- [ ] **Check `git status` before merging** — nothing regenerated or stray.
- [ ] Docs-sync: `docs/PLAN.md`; log any deferral to `docs/TODO.md`. Also move
  `docs/design/doc-sweep-truthfulness-rules.md`'s `## Report contract` section (currently at `:211`,
  ahead of RULE 13 at `:217`) to the end of the file, so the rules read contiguously.
- [ ] **Reviewed skill-update gate:** update `shadowcat-codebase-scene-rendering` and
  `shadowcat-codebase-actors-tokens` (and any other skill whose seam/invariant/gotcha this sweep
  touches), then dispatch `shadowcat-spec-reviewer` on the SKILL DIFF. If nothing needs updating,
  state that explicitly.
- [ ] Whole-branch review pair, merge `--ff-only`, push, delete branch, memory update.

---

## Deferred (logged, not dropped)

- Sweep 12: the chat family and the rest (chat 51, chat-card 23, entry 22, settings 13, topbar 7,
  sheet-actor 5, assets 4, chat-composer 4, game-settings 3, sheet-item 3 = 135). Then per-crate
  buddy-check convergence → final ratchet (crate-root deny, `treatValidationWarningsAsErrors`, docs
  lint merged into `eslint.config.js`) → skills documentation-reference pass.
- **Two open design decisions awaiting the user** (both in `docs/TODO.md`, neither a sweep's call):
  Sweep 9's `TemplatesController.push` `/embedded` filter gap, and Sweep 10's `EngineAdapter.focus`
  seam — wire it up or delete it, and reconcile the `STAGE_ID` guard across both adapters either way.
- **Standing audit item, surfaced by Sweep 8:** nothing routinely checks *skills against code*. The
  reviewed skill-update gate compares a skill diff to the change that prompted it, so a skill can
  drift indefinitely with nothing noticing. Worth a dedicated pass during the closing phases.
