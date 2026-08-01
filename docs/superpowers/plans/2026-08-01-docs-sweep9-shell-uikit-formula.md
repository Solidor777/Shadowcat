# Docs Sweep 9 — shell + ui-kit + formula Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development.
> **REQUIRED READING for every implementer and reviewer:** `docs/design/doc-sweep-truthfulness-rules.md`
> — hand it over BY PATH, never pasted into a brief (pasting is how the rules drift between
> dispatches).

**Goal:** Drive `@shadowcat/shell`, `@shadowcat/ui-kit`, and `@shadowcat/formula` from 276
`lint:docs` warnings to 0, then ratchet all three to `error` in `eslint.docs.config.js`.

**Architecture:** Four content tasks split by package and by size, then ratchet-and-ship. Same shape
as Sweeps 7 (620→0) and 8 (339→0).

**Tech Stack:** TypeScript, Svelte 5 runes, ESLint + `eslint-plugin-jsdoc`, Vitest.

## Global Constraints

- **Comment-only.** No runtime change in a content task. If you find a real defect, report it with
  reachability bounded — do not fix runtime code in a docs task. (Sweep 8 found a rendering bug, two
  docs-gate defects, and a sibling divergence exactly this way; each was fixed in its own commit.)
- **Gate:** `npx eslint --config eslint.docs.config.js src/client/<pkg>/src/<file>` per file, plus a
  whole-package sweep before each task reports. The ratchet fails on ANY file left above 0, including
  one not enumerated here — enumerate live counts, never trust this document's numbers.
- **Per-task gates:** scoped counts at 0, `pnpm -r typecheck`, the package's own tests,
  `pnpm docs:check-examples`, `pnpm lint`.
- **`@example` fences:** ```ts fences are EXTRACTED and typechecked; untagged ``` are not.
  Self-contained, import by workspace package name, `declare const x: Type;` for unconstructible
  deps. Anything NOT exported from the package's `index.ts` MUST use an UNTAGGED fence.
  **Baseline is 286 examples.** A drop means something silently stopped being extracted —
  investigate rather than proceeding.
- **Reports:** write to `.superpowers/sdd/2026-08-01-docs-sweep9-shell-uikit-formula/task-N-report.md`
  and **`ls` it to confirm it exists before reporting back.** A Sweep-8 task reported writing one and
  had not; both reviewers lost their claims-table audit and that task's citation metrics are
  unrecoverable.
- **Staging:** stage only the files you edited, by explicit path. Never `git add -A`.
- **Report contract:** a CLAIMS TABLE (claim → enforcing `file:line`), an explicit list of what the
  pre-existing-prose re-scan covered ("none found" only with a list), and any bug found with its
  reachability bounded.

## What makes this sweep different from the last two

`client/core` was headless logic; `client/render` was PixiJS. This sweep is **the reactive layer**:
Svelte 5 runes, session lifecycle, and a parser. Three consequences:

- **Reactivity claims are hard to verify and easy to state plausibly.** "Re-runs when X changes",
  "tracked by the effect", "stable across re-renders" are each claims about a specific rune's
  dependency graph. This codebase has already been bitten repeatedly here: a `setContext` value must
  be a stable in-place-mutated ref, a `$derived` reading a plain-callback store freezes at mount, and
  a config-doc seed `$effect` needs `createSubscriber`+`subscribe()`. Cite the mechanism or don't
  claim it.
- **`.svelte` files are in scope** (`App.svelte`, `SystemTreeEditor.svelte`, `MergeConflictModal.svelte`)
  and are linted by a separate config block. They have no `index.ts` export surface — use untagged
  fences.
- **`formula` is a grammar.** Every token, precedence, and error-case claim must be quoted from the
  lexer/parser, not recalled. A sibling parser in this repo shipped a doc that restated a *pre-fix*
  rule as current; treat any remembered grammar rule as suspect.

---

### Task 1: `worldSession.svelte.ts` (72)

The single largest file in the sweep, and the WS-session seam the whole client hangs off.

- [ ] Enumerate live count. Document every symbol. Gates. Commit.

**Mandatory fix — a carry-forward from Sweep 7.** `canEdit` (~`:152-161`) is the **live client-side
GM-bypass gate**: it returns `true` for `role === "gm"` before resolving any capabilities, and it is
unaware of `permissions.gm_role`. The server's GM bypass is conditional — a doc carrying
`gm_role: Some(role)` floors even a GM to ordinary `DocRole` resolution
(`data/permission.rs`'s `effective_role`/`resolve_access`). So a GM's client-side write affordances
can over-permit on a `gm_role`-capped document. **Document the caveat here.** See `docs/TODO.md` for
the full entry. Note deliberately: `canWritePath`'s own `isGm` branch has no production caller, which
is why the caveat belongs on `canEdit` and not there — do not move it.

**Other hot spots:** `dispatchIntent`'s ordering (`applyIntent` runs BEFORE `ws.send` — this is the
structural fact that makes the optimistic view observable pre-server, and other packages' docs now
cite it); the Welcome/reconnect lifecycle; `#requirements` mixing GM-authored world caps with
module-declared manifest requirements (the existing comment says module-only requirements make this
gate STRICTER than the server — verify that still holds).

### Task 2: shell remainder (50)

`sessionState.svelte.ts` (20), `api.ts` (14), `bootResolution.ts` (5), `App.svelte` (4),
`route.svelte.ts` (4), `reactiveStore.svelte.ts` (3).

- [ ] Enumerate live counts. Document. Gates. Commit.

**Hot spots:** `sessionState`'s dirty-slice tracking — per-LEAF-KEY granularity, and `flushOnUnload`
must re-mark on failure (both were real bugs); `api.ts`'s timeout/retry contract; `bootResolution`'s
rule that a world route in the URL always wins and `lastWorld` seeds only bare loads (a deep-link
regression came from getting this backwards); `reactiveStore`'s subscription bridge.

### Task 3: `ui-kit` — all 16 files (93)

Largest: `i18n.svelte.ts` (6), `SystemTreeEditor.svelte` (5), `actorSelection.svelte.ts` (5),
`tokenSelection.svelte.ts` (5), plus twelve smaller.

- [ ] Enumerate live counts. Document all. **Run a whole-package sweep.** Gates. Commit.

**Hot spots:** `appContext.ts` — a `setContext` value must be a **stable in-place-mutated** ref, never
a reassigned `$state`; the three selection stores are a near-identical sibling set (**Rule 11: audit
them as a set and flag any divergence explicitly** — Sweep 8 found two divergences this way among four
sibling view files); `i18n`'s missing-key and fallback behavior (fail-open vs fail-closed stated
backwards is Important); the `__fixtures__/` files are test scaffolding — be precise about what they
do and do NOT emulate.

### Task 4: `formula` — all 7 files (61)

`parser.ts` (15), `template.ts` (15), `evaluate.ts` (10), `internal.ts` (9), `lexer.ts` (6),
`graph.ts` (5), `types.ts` (1).

- [ ] Enumerate live counts. Document all. **Run a whole-package sweep.** Gates. Commit.

**Hot spots:** every grammar claim quoted from the lexer/parser rather than recalled — token spellings,
precedence, associativity, and which inputs are rejected. State error behavior precisely: does a bad
formula throw, return null, or produce a partial AST? `graph.ts`'s dependency/cycle handling is a
Rule-2 non-obvious claim. `evaluate.ts`'s numeric semantics (division by zero, overflow, coercion)
each need their enforcing line.

### Task 5: Ratchet + verify + sync + ship

- [ ] Add `src/client/shell/**/*.ts`, `src/client/ui-kit/**/*.ts`, `src/client/formula/**/*.ts` to the
  ratcheted `error` block in `eslint.docs.config.js`. **Decide explicitly whether `.svelte` files can
  join** — they are linted by a separate block; if they cannot be ratcheted the same way, say so in
  the commit rather than silently omitting them.
- [ ] **Mutation proof** per newly-ratcheted package: delete a doc comment → `pnpm lint:docs` exits
  nonzero naming that file → restore.
- [ ] **Manual scan for orphaned doc blocks** (`*/` immediately followed by `/**`). `lint:docs` cannot
  detect this — it binds to the nearest preceding block, so an appended block satisfies the linter
  while orphaning the richer one above it.
- [ ] Full local matrix: `pnpm -r typecheck`, `pnpm -r test`, `pnpm lint`, `pnpm lint:docs`,
  `pnpm docs:check-examples`, `pnpm build`, then server `cargo fmt --check`, `cargo clippy
  --all-targets -D warnings`, `cargo test` (build `dist/` BEFORE any cargo — embed ordering).
- [ ] **Check `git status` before merging.** Sweep 8 nearly shipped three regenerated ts-rs files
  uncommitted; a Rust doc-comment edit propagates into `src/types/generated`.
- [ ] Docs-sync: `docs/PLAN.md`; remove the `gm_role` entry from `docs/TODO.md` once Task 1 documents
  it; log any deferral.
- [ ] **Reviewed skill-update gate:** update `shadowcat-codebase-client-shell` (and any other skill
  whose seam/invariant/gotcha this sweep touches), then dispatch `shadowcat-spec-reviewer` on the
  SKILL DIFF. If nothing needs updating, state that explicitly.
- [ ] Whole-branch review pair, merge `--ff-only`, push, delete branch, memory update.

---

## Deferred (logged, not dropped)

- Sweeps 10–11: module packages (~530, 3–4 groups). Then per-crate buddy-check convergence → final
  ratchet (crate-root deny, `treatValidationWarningsAsErrors`, docs lint merged into
  `eslint.config.js`) → skills documentation-reference pass.
- **Standing audit item, surfaced by Sweep 8:** nothing routinely checks *skills against code*. The
  reviewed skill-update gate compares a skill diff to the change that prompted it, so a skill can
  drift indefinitely with nothing noticing — Sweep 8 found `CORE_LAYERS` indices each off by one
  purely by accident. Worth a dedicated pass during the closing phases.
