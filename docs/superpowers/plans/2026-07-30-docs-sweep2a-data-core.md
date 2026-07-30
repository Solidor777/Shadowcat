# Docs Sweep 2a — Data Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: on a Fable-class model use
> `mainline-plan-execution`; on any other model use
> superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Document the server `data/` core (everything except `data/engine/`,
which is Sweep 2b): `document.rs`, `mod.rs`, `command.rs`, `permission.rs`,
`repository.rs`, `membership.rs`, `validation.rs`, `search.rs`, `asset.rs`,
`sqlite.rs` — measured backlog 185 items — then flip all ten files to deny.

**Architecture:** Same calibrated pattern as Sweep 1
(`docs/superpowers/plans/2026-07-30-docs-sweep1-server-ops.md` — its Global
Constraints, doctest policy, scoped-count gate, and per-item-class doc shapes
apply verbatim and are NOT restated here). Two mechanics are NEW in this sweep:

1. **ts-rs propagation.** Many `data/` types derive `TS`; Rust doc comments are
   emitted into `src/types/generated/*.ts` on regeneration (`cargo test`), and
   CI's "ts-rs bindings in sync" step diffs those files. Every task that touches
   a ts-rs type MUST run `cargo test`, inspect the regenerated bindings diff
   (doc-comment additions ONLY — any shape change means a mistake), and commit
   the regenerated `.ts` files WITH the task. Full `pnpm -r test` +
   `pnpm -r typecheck` run in the final task (wire-schema gate; comments should
   be inert to Zod but the gate proves it).
2. **Invariant truthfulness review.** This is the permissions subsystem —
   Sweep 1's final review caught a factually wrong doc; here a wrong doc could
   misstate a SECURITY invariant. Every doc claim about
   redaction/visibility/authz must be verified against the code path AND
   cross-checked against `shadowcat-codebase-documents-permissions`'s invariants
   and `docs/design/ARCHITECTURE.md` §2 while writing (redact-before-send,
   per-recipient filtering, owner-or-GM tier, three-band shape). Where a doc
   states an invariant, cite the enforcing function.

**Measured backlog (re-verify per task):** document.rs 75, sqlite.rs 37,
command.rs 22, mod.rs 20, permission.rs 10, search.rs 6, asset.rs 6,
repository.rs 4, validation.rs 3, membership.rs 2.

## Model/Effort directives

Authored + executed mainline (Fable 5, effort high) per the user's standing
directive. ONE final review at branch end: `shadowcat-spec-reviewer` +
`shadowcat-code-reviewer` pair, with the code reviewer explicitly briefed to
adversarially verify doc TRUTHFULNESS against code (the Sweep-1 lesson).

## Buddy-check directives

No high-risk signals (docs + lint attributes + regenerated doc-comment-only
bindings; zero behavior change). Standard final review only.

## Global Constraints

Sweep 1's Global Constraints apply verbatim (docs-only, comment rules, doctest
policy, scoped-count gate, per-task green: `cargo test` + `cargo fmt --all
--check` + `cargo clippy --all-targets -- -D warnings` from `src/server/`
subshell, feature branch `docs-sweep2a-data-core`, push only at completion).
Additions for this sweep:

- ts-rs mechanic (above): regenerated bindings diff inspected + committed per
  task; a diff hunk that is not purely doc comments blocks the task.
- Doctests for `SqliteRepository` methods and anything needing a live pool →
  `no_run` with a `# #[tokio::main]` wrapper (or runnable via
  `"sqlite::memory:"` where genuinely hermetic and fast — prefer runnable when
  the setup is ≤ a few lines, e.g. `connect` + one call).
- Security-invariant docs cite their enforcing function (e.g. "stripped in
  `filter_document_for` before transmission").

---

### Task 1: document.rs (75 items)

**Files:** Modify: `src/server/src/data/document.rs` (+ regenerated
`src/types/generated/*.ts`)

- [ ] **Step 1:** Scoped count for `document.rs` — expect 75; enumerate live.
- [ ] **Step 2:** Document all items. This file holds the three-band document
  model (`Document`, `PermissionSet`, visibility/`DocRole`/`WorldRole`,
  capability grants, `Scope`, schemas). Field docs state meaning + redaction
  behavior where applicable, with enforcing-function citations. Structs/enums
  deriving `TS`: remember the docs land in the generated TS too — write them to
  read correctly in BOTH references.
- [ ] **Step 3:** `# Examples` doctests on every fn/method in the file
  (constructors/predicates are typically pure → runnable).
- [ ] **Step 4:** `cargo test` (doctests + regen), inspect
  `git diff src/types/generated` = doc-comments only; scoped count 0; fmt;
  clippy `-D`.
- [ ] **Step 5:** Commit (`docs(server/data): document the document model`,
  including the regenerated bindings).

### Task 2: mod.rs (20) + command.rs (22)

**Files:** Modify: `src/server/src/data/mod.rs`,
`src/server/src/data/command.rs` (+ regenerated bindings)

- [ ] **Step 1:** Scoped counts — expect 20 + 22; enumerate live.
- [ ] **Step 2:** Document (command.rs: `Operation`/`FieldChange`/`WriteOrigin`
  — `remove` semantics ("genuine absence, distinct from null"), OCC pre-image
  discipline, intent correlation; mod.rs: module decls/re-exports/shared types).
- [ ] **Step 3:** Doctests on every fn (pure constructors runnable).
- [ ] **Step 4:** Gates + bindings-diff inspection as Task 1.
- [ ] **Step 5:** Commit.

### Task 3: permission.rs (10) + repository.rs (4) + membership.rs (2) + validation.rs (3)

**Files:** Modify: those four files (+ regenerated bindings if any derive TS)

- [ ] **Step 1:** Scoped counts — expect 10/4/2/3.
- [ ] **Step 2:** Document. permission.rs is the authz core: every predicate doc
  states the tier logic exactly (owner-or-GM never widens the GM boolean —
  cross-check [[ownerorgm-tier-no-widen]] via the documents-permissions skill)
  and cites callers. repository.rs: the `Repository` trait contract.
  validation.rs: structural-only `system`-band rules (never semantic).
- [ ] **Step 3:** Doctests (permission predicates with hand-built
  `PermissionSet` fixtures → runnable; trait methods → document on the trait,
  `no_run` examples via `SqliteRepository`).
- [ ] **Step 4:** Gates as Task 1. Test-harness gotcha: `doc(perms, system)`
  helper + owner-FK rules per [[server-test-doc-helper-and-owner-fk]] if any
  doctest needs a real repo — prefer NOT to; keep examples fixture-light.
- [ ] **Step 5:** Commit.

### Task 4: search.rs (6) + asset.rs (6)

**Files:** Modify: both (+ bindings if applicable)

- [ ] **Step 1:** Counts — expect 6 + 6.
- [ ] **Step 2:** Document. search.rs: visibility-partitioned FTS invariant
  (physically separate tables per visibility — cite
  [[search-index-must-be-visibility-partitioned]] rationale as a present
  constraint, not history). asset.rs: content-addressed store, ETag/version
  revalidation, commit-row-before-file-swap ordering as a present invariant.
- [ ] **Step 3:** Doctests per policy (pure helpers runnable; store ops no_run).
- [ ] **Step 4:** Gates.
- [ ] **Step 5:** Commit.

### Task 5: sqlite.rs (37)

**Files:** Modify: `src/server/src/data/sqlite.rs`

- [ ] **Step 1:** Count — expect 37. This file is very large (~7k lines incl.
  tests); the undocumented items are the target, not a restyle of existing docs.
- [ ] **Step 2:** Document the 37 (repository impl internals, apply-intent
  pipeline pieces, per-recipient egress helpers). Transactionality claims must
  be exact (two-query guards wrapped in one tx — [[two-query-guard-needs-tx]]).
- [ ] **Step 3:** Doctests on undocumented fns per policy (most are private or
  pool-bound → public-caller examples or `no_run`; do NOT add examples to
  `#[cfg(test)]` items — they're excluded).
- [ ] **Step 4:** Gates.
- [ ] **Step 5:** Commit.

### Task 6: Deny flip, full verification, docs-sync, ship

**Files:** Modify: the ten data-core files (inner deny attrs), `docs/PLAN.md`,
`.claude/skills/shadowcat-codebase-documents-permissions/SKILL.md`

- [ ] **Step 1:** Add the two inner deny attributes (Sweep-1 wording, no
  process-meta) to all ten files. `data/engine/*` stays un-flipped (Sweep 2b).
- [ ] **Step 2:** Ratchet-bite mutation proof on one file; restore (mind the
  Sweep-1 gotcha: restore via re-Edit, not `git restore`, if attrs are
  uncommitted).
- [ ] **Step 3:** Full gates: server cargo test/fmt/clippy `-D`; `pnpm -r test`
  + `pnpm -r typecheck` (wire-schema gate over regenerated bindings);
  `pnpm docs:build` green; all ten scoped counts 0.
- [ ] **Step 4:** Docs-sync: PLAN.md campaign entry (Sweep 2a complete, counts);
  documents-permissions skill Gotcha (ratchet live in data core, engine/ still
  open; doc-truthfulness rule). Dispatch `shadowcat-spec-reviewer` on the skill
  diff; fix findings.
- [ ] **Step 5:** Final branch review pair (per Model/Effort directives), fix
  Critical/Important, then merge `--ff-only` to main, push, `gh run watch`
  all-green, delete branch, update memory campaign state.

---

## Deferred (logged, not dropped)

- Sweep 2b: `data/engine/` (scene 76, token 39, geometry 32, registries 25 =
  172 items) — next plan after this ships.
- Then: ws/ (98+33+22+protocol), http/+auth/, scene/, chat/+dice/; client
  packages; modules. lib.rs crate-root deny = final ratchet.
