# Docs Sweep 10 — panels Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development.
> **REQUIRED READING for every implementer and reviewer:** `docs/design/doc-sweep-truthfulness-rules.md`
> — hand it over BY PATH, never pasted into a brief (pasting is how the rules drift between
> dispatches).

**Goal:** Drive `@shadowcat/module-panels` from 217 `lint:docs` warnings to 0, then ratchet it to
`error` in `eslint.docs.config.js` (BOTH the `.ts` and the `.svelte` block).

**Architecture:** Three content tasks split by layer — production engine, pure layout core, and
controller+policy+components — then ratchet-and-ship. Same shape as Sweeps 7 (620→0), 8 (339→0),
and 9 (276→0). **This is the first sweep into `src/modules/`**, so it is also the first to prove
the module packages behave like the client packages under both gates.

**Tech Stack:** TypeScript, Svelte 5 runes, dockview-core 7.0.2 (exact pin), ESLint +
`eslint-plugin-jsdoc`, Vitest.

## Global Constraints

- **Comment-only.** No runtime change in a content task. If you find a real defect, report it with
  reachability bounded — do not fix runtime code in a docs task. (Sweep 8 found a rendering bug, two
  docs-gate defects, and a sibling divergence exactly this way; Sweep 9 found a silent-no-op branch.
  Each was fixed in its own commit, separate from the docs commit.)
- **Gate:** `npx eslint --config eslint.docs.config.js src/modules/panels/src/<file>` per file, plus
  a whole-package sweep before each task reports. The ratchet fails on ANY file left above 0,
  including one not enumerated here — enumerate live counts, never trust this document's numbers.
- **Per-task gates:** scoped counts at 0, `pnpm -r typecheck`, the package's own tests
  (`pnpm --filter @shadowcat/module-panels test`), `pnpm docs:check-examples`, `pnpm lint`.
- **`@example` fences:** ```ts fences are EXTRACTED and typechecked; untagged ``` are not.
  `src/modules` **is** in the extractor's roots (`scripts/extract-ts-examples.mjs:120`), so this
  applies here exactly as it did in the client packages. Self-contained, import by workspace package
  name (`@shadowcat/module-panels`), `declare const x: Type;` for unconstructible deps. Anything NOT
  exported from the package's `index.ts` MUST use an UNTAGGED fence. **`index.ts` re-exports a wide
  surface here** — `export * from "./layout/tree"` and `"./layout/persist"`, plus `EngineAdapter`,
  `FakeEngine`, `DockviewEngine`, `classifyDrop`/`STAGE_ID`/`DropSite`/`ClassifyResult`,
  `PanelsController`/`regsForRole`/`PanelsBridgeLike`/`PanelsControllerDeps`, and the components — so
  most of this sweep CAN use ```ts fences. `opForMenuCommand` and `MenuCommand` are notably NOT
  re-exported. Check the live export list per symbol rather than assuming either way.
  **Baseline is 291 examples.** A drop means something silently stopped being extracted —
  investigate rather than proceeding.
- **Reports:** write to `.superpowers/sdd/2026-08-01-docs-sweep10-panels/task-N-report.md`
  and **`ls` it to confirm it exists before reporting back.** A Sweep-8 task reported writing one and
  had not; both reviewers lost their claims-table audit and that task's citation metrics are
  unrecoverable.
- **Staging:** stage only the files you edited, by explicit path. Never `git add -A`.
- **Report contract:** a CLAIMS TABLE (claim → enforcing `file:line`), an explicit list of what the
  pre-existing-prose re-scan covered ("none found" only with a list), and any bug found with its
  reachability bounded.

## What makes this sweep different from the last three

The client packages were a library, a renderer, and a reactive shell. This is **an
intercept-and-redispatch state machine wrapped around a third-party widget**, and that shape
produces its own doc failure modes:

- **The engine never owns state — but the prose will want to say it does.** Every dockview gesture is
  intercepted, classified into a `LayoutOp`, and re-dispatched through the pure `applyOp` reducer.
  Any sentence of the form "dockview moves the panel" is backwards. State the direction explicitly
  and cite the interception site.
- **Vetoes are DOUBLE-layered on purpose.** `STAGE_ID` is vetoed in `policy.ts` AND as early-returns
  in the dockview wire. That is deliberate defense-in-depth, not redundancy to be "simplified" in a
  comment. Do not document either layer as the sole enforcement point, and do not imply one is
  vestigial. Confirm both still exist before describing them.
- **Two constants must stay numerically identical across two files by design.**
  `SHEET_CASCADE_BASE`/`STEP` (`layout/tree.ts`) and `REHYDRATE_FLOAT_BASE`/`STEP`
  (`controller.svelte.ts`) are deliberately NOT a shared import. This is a documented exception to
  the repo's never-fork-a-decision rule — **read both, verify they still match, and if they do not,
  that is a Critical finding, not a doc nuance.** Do not describe them as sharing a symbol.
- **The same-reference no-op contract is a `toBe` contract.** `applyOp` returning an unchanged layout
  MUST return the SAME object; callers and tests rely on reference identity. A doc that says
  "returns an equivalent layout" is wrong in a way that will get someone to add a `structuredClone`.
- **dockview-core is pinned EXACT and boundary-enforced.** `engine/dockview.ts` (+ its test) is the
  only file permitted to import it, enforced by `no-restricted-imports` in `eslint.config.js` — for
  `.ts` files only; in `.svelte` files the boundary holds by the `EngineAdapter` seam's design, not
  by a lint rule. Any claim about dockview's own behavior must be verified against the **vendored
  pinned source** in `node_modules/dockview-core`, never from recollection or its README
  ([[verify-crate-claims-against-vendored-source]] — the lesson generalizes past Rust).
- **Keep-mounted is the load-bearing invariant of `PanelHost.svelte`.** Panels hide via CSS and slot
  adoption (`appendChild`), never `{#if}`; pop-out RE-PARENTS the same mounted instance into a second
  same-heap `Window` rather than remounting. A doc implying remount/teardown is a correctness defect.

---

### Task 1: `engine/dockview.ts` (73)

The production engine adapter, 1182 lines, and the sweep's largest file. The single boundary to
dockview-core.

- [ ] Enumerate live count. Document every symbol. Gates. Commit.

**Hot spots:** the intercept-and-redispatch wiring (every engine event handler — say what it
classifies into and where it re-dispatches); the `STAGE_ID` early-return vetoes (the second of the
two layers — name the other); pop-out (`Window` creation, style adoption, same-heap constraint,
what happens on popup-blocked); `onNotice`; any place the adapter must reconcile engine state back
toward the tree after a rejected gesture. **Every claim about what dockview-core itself does needs a
citation into `node_modules/dockview-core` at the pinned 7.0.2 version.**

### Task 2: `layout/tree.ts` (41) + `layout/persist.ts` (30)

The pure layout core: the reducer and the persistence codec. 71 warnings, 747 lines.

- [ ] Enumerate live counts. Document all. Gates. Commit.

**Hot spots:** `applyOp`'s **same-reference no-op contract** (above) — state it on the reducer, and
on any op whose no-op case is non-obvious; `locate`/`prune`/`placeNewRegistrations`/
`placeFromPersistedLocation` (each is a placement decision with an ordering rule — get the precedence
right and cite it); `resizeFloating` vs `float` (in-place rect update vs detach-and-reinsert — the
distinction is the reason both exist); `SHEET_CASCADE_BASE`/`STEP` (cross-file parity, above);
`decodeLayout`'s TWO-way outcome — `reset: true` + a fresh `fallback()` on a blob failing
`isPanelLayoutV1`/`isReferentiallyConsistent`, else `reset: false` + `prune(normalized, known)`
(`persist.ts:141-143`). Pruning is NOT a third outcome: the accepted branch ALWAYS prunes, possibly
to a no-op, and `reset` reports decode-time rejection only — the file says so itself at
`persist.ts:124`. Do not write "three-way". Also the
`poppedOut` back-compat rule (absent normalizes to `[]` via `withPoppedOut`; present-but-malformed
fails the WHOLE blob — those two are opposite and easy to state backwards); the retained pre-prune
`source` layout and why late registrations need it.

### Task 3: `controller.svelte.ts` (26) + `engine/fake.ts` (25) + `engine/policy.ts` (9) + `PanelHost.svelte` (10) + `CompactSwitcher.svelte` (2) + `PanelMenu.svelte` (1)

73 warnings across the controller, the two non-production engine pieces, and the three components.

- [ ] Enumerate live counts. Document all. **Run a whole-package sweep.** Gates. Commit.

**Hot spots:** `regsForRole`'s `gmOnly` filter is **client-advisory only, NOT security** — say so
explicitly and point at the server-side gate; `syncRegistrations` never resets a saved layout;
`#rehydratePoppedOut()` runs at construction, converts persisted `poppedOut` to floating (never
re-opens a real popup — no user gesture at load), and **defers its notice past first mount** via
`#pendingNotice`/`flushPendingNotice()` because an `aria-live="polite"` region never announces
content present at its own initial render — that reason is the whole point of the deferral, so state
it; `policy.ts`'s `classifyDrop`/`opForMenuCommand` returning op-or-veto; `fake.ts` is BOTH a test
double and the bespoke production fallback (it degrades pop-out to a floating window — production
pop-out is dockview-only), so be precise about which behaviors are fidelity to dockview and which are
deliberate divergence; `PanelHost.svelte`'s keep-mounted/slot-adoption invariant and its lazy
controller construction + `PanelsBridge.bind()`.

**Rule 11 (sibling audit) applies to `fake.ts` vs `dockview.ts`**: they implement the same
`EngineAdapter` interface. Audit them as a set and flag any divergence beyond the documented pop-out
degradation explicitly — Sweep 8 found two real divergences this way among four sibling files, and
Sweep 9 found one among three selection stores.

### Task 4: Ratchet + verify + sync + ship

- [ ] Add `src/modules/panels/**/*.ts` to the ratcheted `error` `.ts` block AND
  `src/modules/panels/**/*.svelte` to the ratcheted `.svelte` block in `eslint.docs.config.js`.
  Both are required — a package with components ratchets in TWO places (one flat-config block cannot
  carry two parsers). Keep the two ignore lists byte-identical to the warn blocks'.
- [ ] **Mutation proof, one per newly-ratcheted BLOCK** (`.ts` and `.svelte` separately): delete a
  doc comment → `pnpm lint:docs` exits nonzero naming that file → restore. Target a
  `FunctionDeclaration`, `MethodDefinition`, or `ClassDeclaration` — **`require-jsdoc` does NOT gate
  a `const` arrow function**, so a probe aimed at one returns a false green (this cost two wasted
  probes in Sweep 9).
- [ ] **Manual scan for orphaned doc blocks** (`*/` immediately followed by `/**`) — **repo-wide, not
  sweep-scoped.** `lint:docs` cannot detect this. Sweep 9's scoped scan found 1; widening it found 4,
  three of them inside already-ratcheted packages.
- [ ] Full local matrix: `pnpm -r typecheck`, `pnpm -r test`, `pnpm lint`, `pnpm lint:docs`,
  `pnpm docs:check-examples`, `pnpm build`, then server `cargo fmt --check`, `cargo clippy
  --all-targets -D warnings`, `cargo test` (build `dist/` BEFORE any cargo — embed ordering).
- [ ] **Check `git status` before merging** — nothing regenerated or stray.
- [ ] Docs-sync: `docs/PLAN.md`; log any deferral to `docs/TODO.md`.
- [ ] **Reviewed skill-update gate:** update `shadowcat-codebase-panels` (and any other skill whose
  seam/invariant/gotcha this sweep touches), then dispatch `shadowcat-spec-reviewer` on the SKILL
  DIFF. If nothing needs updating, state that explicitly.
- [ ] Whole-branch review pair, merge `--ff-only`, push, delete branch, memory update.

---

## Deferred (logged, not dropped)

- Sweeps 11–12: scene-adjacent modules (~157) then chat/entry/settings/sheets/topbar/assets (~135).
  Then per-crate buddy-check convergence → final ratchet (crate-root deny,
  `treatValidationWarningsAsErrors`, docs lint merged into `eslint.config.js`) → skills
  documentation-reference pass.
- **Open design decision carried from Sweep 9** (`docs/TODO.md`): `TemplatesController.push`'s
  per-instance filter omits `/embedded`, so an affected instance's entire Update is refused and it
  stays stale. Two candidate fixes are logged; this is the user's call, not a sweep's.
- **Standing audit item, surfaced by Sweep 8:** nothing routinely checks *skills against code*. The
  reviewed skill-update gate compares a skill diff to the change that prompted it, so a skill can
  drift indefinitely with nothing noticing. Worth a dedicated pass during the closing phases.
