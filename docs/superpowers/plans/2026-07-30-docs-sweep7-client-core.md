# Docs Sweep 7 — @shadowcat/core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: on a Fable-class model use
> `mainline-plan-execution`; otherwise superpowers:subagent-driven-development.
> Steps use checkbox (`- [ ]`) syntax.

**Goal:** Document `src/client/core/` — measured backlog 617 `lint:docs`
warnings (scene-docs 119, ws-client 72, merge 70, templates 52, actor 45,
user-rest 27, store 23, capabilities 21, optimistic 18, contributions 16,
modules 16, hooks 15, i18n 14, mock-server 14, asset-rest 13, sheets 13,
chat-docs 11, module-rest 8, assets 7, e2e/server-process 6, services 6,
loader 5, manifest 4, middleware 4, topology 4, index 3, semver 3, transport
3, wire 3, logger 2) — then flip `client/core` from warn to ERROR in
`eslint.docs.config.js` (the TS ratchet mechanism; first package flipped).

**Architecture:** First TS-side sweep — mechanics differ from the Rust
sweeps: TSDoc comments; `@example` fences are ```ts and are EXTRACTED AND
TYPECHECKED by `scripts/extract-ts-examples.mjs` (`pnpm docs:check-examples`,
CI-blocking) so every example must import/compile against the workspace;
enforcement lives in `eslint.docs.config.js` per-package severity, not inner
attrs. Branch `docs-sweep7-client-core`. Per-task gates: scoped `lint:docs`
count for the task's files → 0, `pnpm -r typecheck`, `pnpm --filter
@shadowcat/core test`, `pnpm docs:check-examples`. Ship with the LOCAL
matrix. Reviews under the no-shell protocol.

**Truthfulness hot spots:** client mirrors must be documented AS mirrors with
the server symbol named — store.ts `applyOperation` mirrors server
`command.rs` pointer ops (null-INTERMEDIATE-as-absent parity, leaf null !=
absent); scene-docs.ts grid `size` = outer radius/circumradius,
`DEFAULT_SCENE_BOUNDS` parity with `DEFAULT_SCENE_BOUNDS_UNITS`; optimistic
view is what the canvas renders (`AppContext.documents`, appliedSeq
watermark); merge.ts provenance-based 3-way rules stated from code;
actor.ts effective-owner mirror equals server `effective_owner` exactly;
capabilities fail-closed defaults; NEVER document a wire field, event name,
or marker from memory — quote the Zod schema/enforcing line.

## Model/Effort directives

Mainline (Fable 5, effort high) per standing directive. No-shell final review
pair; fixes pre-merge.

## Buddy-check directives

No high-risk signals (docs + lint severity only). Standard final review only.

---

### Task 1: scene-docs.ts (119)

- [ ] Enumerate live (expect 119); document every export — scene/token/wall/
  region/light doc shapes, grid types (size = circumradius), resolved-settings
  mirrors, `DEFAULT_SCENE_BOUNDS`. @example on functions (typecheck-clean).
  Gates; commit.

### Task 2: ws-client.ts (72) + merge.ts (70)

- [ ] Enumerate live; document — WS client lifecycle/frame surface (quote the
  Zod frame schemas), reconnect/resync semantics; merge.ts provenance 3-way
  rules. Gates; commit.

### Task 3: templates.ts (52) + actor.ts (45)

- [ ] Enumerate live; document — MergeBase stamp/pull/push/revert model,
  actor/token effective-owner mirror (server-exact). Gates; commit.

### Task 4: user-rest (27) + store (23) + capabilities (21) + optimistic (18)

- [ ] Enumerate live; document — REST wrappers with authz-behavior notes,
  DocumentStore/applyOperation pointer-op parity, capability resolution
  fail-closed defaults, OptimisticClient queue/rollback. Gates; commit.

### Task 5: contributions (16) + modules (16) + hooks (15) + i18n (14) + mock-server (14) + asset-rest (13) + sheets (13)

- [ ] Enumerate live; document. Gates; commit.

### Task 6: remaining 14 small files (69)

- [ ] chat-docs 11, module-rest 8, assets 7, e2e/server-process 6, services 6,
  loader 5, manifest 4, middleware 4, topology 4, index 3, semver 3,
  transport 3, wire 3, logger 2. Gates; commit.

### Task 7: Severity flip + verify + sync + ship

- [ ] Flip `client/core` file globs from `warn` to `error` in
  `eslint.docs.config.js` (first per-package TS ratchet). Mutation proof:
  delete one doc comment → `pnpm lint:docs` exits nonzero citing the file →
  restore via python. Full local matrix. Docs-sync: PLAN.md; client-shell (or
  realtime-sync for ws-client) skill ratchet Gotcha. No-shell review pair
  with pre-generated diff + relayed evidence; fix findings; merge `--ff-only`;
  push; delete branch; memory update.

---

## Deferred (logged, not dropped)

- Sweep 8: client/render (339). Sweep 9: shell+ui-kit+formula (281).
  Sweeps 10-11: module packages (~530). Then buddy-check convergence → final
  ratchet → skills reference pass.
