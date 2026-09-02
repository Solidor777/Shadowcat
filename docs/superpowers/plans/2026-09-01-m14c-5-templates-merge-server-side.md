# M14c-5 Templates Merge Server-Side — Implementation Plan

> **For agentic workers:** Execute task-by-task in order; each task's steps use checkbox
> (`- [ ]`) syntax. THIS RUN: executed by the Kimi session running the M14c campaign, mainline in
> worktree `C:/Dev/Shadowcat-m14c5` (branch `m14c-5-templates-merge`).

**Goal:** The server owns template merging — `MergePull`/`MergePush`/`MergeRevert` intents compute
the 3-way merge server-side, return conflict sets for human review, and derive authorization
against the actual computed Update — and `Document.base` becomes a server-owned,
engine-tree-validated snapshot instead of an opaque client-writable blob.

**Architecture:** New `merge` module in the server crate (behavioural twin of `@shadowcat/core`'s
merge computation, corpus-pinned by a fixture GENERATED from the TS engine at migration time); the
three intents ride the `CombatRoll` request/reply pattern; merge writes commit under a new
`WriteOrigin::TemplateMerge`; the client controller becomes an intent sender and the TS merge
computation is deleted once the Rust twin passes the corpus.

**Tech Stack:** Rust (server crate), ts-rs (wire types change — regenerate bindings), Vitest
(corpus generation rides the core package's vitest, which executes TS natively), node scripts.

**Spec:** `docs/superpowers/specs/2026-09-01-m14c-5-templates-merge-server-side-design.md`
(decisions T1–T8; read it first).

## Execution directives

Kimi session, not Fable — `mainline-plan-execution` does not apply. Dispatch coder/explore
subagents freely for well-scoped tasks. **Every dispatched agent's first prompt MUST contain this
paragraph verbatim:**

> The iron rule is no deferrals, of existing work, or new work as it comes up - we fix this now
> unless I give my EXPRESS authorization. The only exception is if a bug or to-do has a genuine
> blocker that is already logged in a milestone in PLAN.md that has not been started yet. Another
> iron clad is rule is that when faced with a design fork, determine the best long term shape in
> keeping with our plans and goals, and implement accordingly. You only need to ask me if the
> question "what is the best long term shape in keeping with our plans and goals?" is not able to
> answer the question. Churn is not a concern. This paragraph must be copied verbatim to any
> agents dispatched in this campaign.

…plus the reporting rule: a subagent must deliver its report as the Agent tool result OR write it
to a named document; state which in the prompt.

## Buddy-check directives

Pre-authorized by the user ("You may use buddy checking as seems appropriate", carried from the
M14c-2 session). Run buddy-checking (two blind reviewers + brokered debate) at:
1. After Task 2 — the Rust merge twin + corpus (Tasks 1–2 diff).
2. After Task 4 — base ownership + intents (Tasks 3–4 diff).
3. Final: two-reviewer branch review before merge.

## Global Constraints

- No lint suppressions of any kind (`#[allow]`, `#[expect]`, `eslint-disable`, `@ts-ignore`); no
  file-size allowlist entries — split instead (soft 5,000 / hard 10,000 lines).
- Rust test bodies in sibling files (`pnpm lint:inline-tests`); comments cite symbols, never
  files/lines; no milestone ids, spec pointers, dates, or history narration in code comments or
  test names (`check-comment-refs`).
- `dist/` must exist before any cargo build (already built in this worktree).
- Deletions via `trash`, never `rm`/`Remove-Item`; commits always `git commit -- <paths>`.
- Doc gates at completion: `cargo test`/`clippy`/`fmt`, `pnpm -r test`, `pnpm -r typecheck`,
  `pnpm lint`, `pnpm lint:docs`, `pnpm lint:props`, `pnpm lint:comments`, `pnpm lint:file-size`,
  `pnpm lint:inline-tests`, `pnpm docs:check-examples`, skill checkers
  (`node scripts/check-skill-symbol-refs-cli.mjs`, `pnpm run test:scripts`).
- Wire shapes CHANGE here (new ClientMsg/ServerMsg variants + ts-rs types) — regenerate and
  commit `src/types/generated/` bindings in the same commit as the Rust type that produces them.
- CI clippy runs a NEWER toolchain than a stale local install — the M14c-4 merge red-flagged on
  clippy 1.98's `question_mark` lint. Local stable is now 1.98.0; keep it updated before gates.

---

### Task 1: merge conformance corpus — generated from the TS engine

**Files:**
- Create: `src/client/core/src/__fixtures__/merge-conformance.json`
- Create: `src/client/core/src/mergeConformance.test.ts` (dual-mode: default = assert the TS
  engine reproduces the fixture exactly; `MERGE_CORPUS_GENERATE=1` = regenerate the fixture from
  the TS engine's live output and write it)

**Interfaces:**
- Case shape (one per case):
  `{ "name": string, "base": MergeBaseJson, "parent": WireDocJson, "child": WireDocJson,
     "expect": { "mergedBands": …, "conflicts": […] } }` for pull cases;
  `{ …, "kind": "revert", "expect": { "update": <the WireOperation planToUpdate emits> } }` for
  revert cases. `WireDocJson` is a full envelope with deterministic ids (`"t1"`, `"c1"`… — the
  generator walks the output and normalizes any `crypto.randomUUID()` from `restampSubtree` to
  deterministic placeholders like `"restamped-1"` in template-add order, so the fixture is
  reproducible).
- Case matrix (spec §4): leaf set/delete both sides, same-result non-conflict, ancestor/
  descendant overlap, array wholesale win + array conflict, token placement exclusions,
  embedded: instance-added kept / base-missing fail-safe / template-added restamped /
  template-deleted unchanged-dropped / template-deleted changed-conflicted, nested embedded
  recursion, revert's drop-local-additions, `planToUpdate` emission (changed bands only; the
  absent-vs-`[]` `null` rule; `/base` = template snapshot), every `parentKind` shape.

**Steps:**
- [x] Write the case matrix + dual-mode harness; run with `MERGE_CORPUS_GENERATE=1` to emit the
  fixture; run again normally to confirm assertion mode passes.
- [x] Commit fixture + harness together (the TS engine still exists; the corpus is its measured
  behaviour).

---

### Task 2: Rust `merge` module (behavioural twin, corpus-pinned)

**Files:**
- Create: `src/server/src/merge/mod.rs`, `tree.rs`, `embedded.rs`, `bands.rs`, `plan.rs`
  (unit layout per spec §3)
- Create: sibling test files `src/server/src/merge/tests/*.rs` (unit tests) and
  `src/server/src/merge/tests/conformance.rs` (corpus runner)
- Modify: `src/server/src/lib.rs` (`pub mod merge;`)

**Interfaces:**
- `merge::MergeConflict { path, base, parent, child, parent_kind }` — `Serialize/Deserialize/TS`,
  `#[ts(export, export_to = "../../types/generated/")]`, serde camelCase (`parentKind`) to match
  the wire shape the modal consumes today.
- `merge::bands::{MergeBase, EmbeddedBaseChild, MergeBands}` (serde + TS), `snapshot_base`,
  `placement_exclusions`, `is_placement_excluded`.
- `merge::plan::{merge3, compute_pull, compute_revert, plan_to_update, apply_resolutions}`.
- All operate on `serde_json::Value` / `data::document::Document`; deterministic-id normalization
  is the CORPUS RUNNER's job (Rust restamp also mints uuids; the runner normalizes both sides the
  same way before comparing).

**Steps:**
- [x] Port unit-by-unit in the c-1 style (reader can diff against `merge.ts`/`templates.ts`).
  Clone-at-crossing discipline per the TS comments (aliasing cases are in the corpus).
- [x] `deep_equal` numeric comparison matches TS `===` on f64; key-order-independent objects;
  positional arrays.
- [x] Corpus runner green (`cargo test -p shadowcat merge`).
- [x] `cargo fmt` + `cargo clippy --all-targets` clean; commit.

**→ Buddy checkpoint 1 here (Tasks 1–2 diff).**

---

### Task 3: `Document.base` ownership + engine-tree validation

**Files:**
- Modify: `src/server/src/data/validation.rs` (`validate_engine_tree` walks `base`; shape-check
  `MergeBase` recursively; normalize each `engine` band via the owning doc_type — root's own,
  embedded base children correlated to live children by `sourceId`, unresolvable → shape-only)
- Modify: `src/server/src/data/permission.rs` (`/base` leaves the client-writable set:
  `required_cap_for_path` no longer maps it — split the write-side band list from
  `REDACTABLE_BANDS`, which egress still needs; rewrite the field's doc comments)
- Modify: `src/server/src/data/command.rs` (`WriteOrigin::TemplateMerge` — per-op capability
  gates waived; scope/size/engine/containment/schema/OCC all run; `is_server_authored`)
- Modify: `src/server/src/data/sqlite.rs` (Create branch derives `base`: instance →
  `snapshot_base` of the validated doc, discarding any client-supplied value; non-instance →
  `None`; BEFORE validation so the derived value is what's validated)
- Modify: `src/server/src/data/document.rs` (`base` field doc rewrite — server-owned,
  server-derived, engine-tree-validated; drop "opaque"/"NEVER interprets")
- Tests: sibling tests for validation + permission + create-derivation; ws-level tests land in
  Task 4's suite.

**Steps:**
- [x] Validation walk + normalization (mirror `validate_engine_tree`'s engine recursion).
- [x] Capability carve-out + `WriteOrigin::TemplateMerge` + Create-time derivation.
- [x] Sweep: every in-repo writer of `/base` (world seeds, test fixtures, chat/combat helpers)
  still compiles/passes; re-fixture any test that wrote `/base` as a client.
- [x] Legacy-row test: a stored stale-schema `base` still READS (validation is ingest-time only).
- [x] Gates (fmt/clippy/test); commit.

---

### Task 4: the three intents + repository query + wire types

**Files:**
- Modify: `src/server/src/ws/protocol.rs` (`ClientMsg::MergePull/MergePush/MergeRevert`,
  `ServerMsg::MergeResult/MergeError`, `MergeOutcome`/`PushInstanceOutcome`/`MergeErrorKind` —
  all ts-rs exported; reply docs cross-reference the `CombatRoll` pattern)
- Regenerate: `src/types/generated/*.ts` (same commit)
- Modify: `src/server/src/data/repository.rs` + `src/server/src/data/sqlite.rs`
  (`instances_of(world_id, template_id)` — `json_extract` on the stored `source`)
- Modify: `src/server/src/ws/conn.rs` (dispatch joins the combat-intent match at
  `conn.rs:671`'s arm) and a new `src/server/src/ws/conn/merge_intents.rs` handler module
- Create: `src/server/src/ws/conn/tests/merge_intents.rs`

**Interfaces (spec §5):** the intent/result/error shapes exactly as specified; `MergeErrorKind`
= `NotFound | NotAnInstance | Forbidden | StaleResolutions(MergeOutcome) |
UnknownResolution(MergeOutcome)` (+ shared validation/IO pass-through); push outcome entries
`{ instance_id, name, status: Applied | Conflicts(_) | Excluded }` with `name` = pusher-visible
display name.

**Steps:**
- [x] Wire types + ts-rs regeneration.
- [x] `instances_of` + visibility filter (the requester's redacted view decides "visible";
      replicate the client's store-scoped reach per T4).
- [x] Handlers: compute → derive authorization per T4 (owner-or-GM gate; per-path derivation
      against the actual computed Update using the same capability predicate `apply_intent`'s
      per-op gate uses) → commit authorized Updates under `WriteOrigin::TemplateMerge` → reply.
      Resolutions path: recompute, strict subset check, apply; stale/unknown → error carrying
      the fresh outcome.
- [x] WS tests per spec §8's list.
- [x] Gates; commit.

**→ Buddy checkpoint 2 here (Tasks 3–4 diff).**

---

### Task 5: client re-plumb + TS computation deletion

**Files:**
- Modify: `src/client/ui-kit/src/templatesController.svelte.ts` (pull/push/revert send intents
  with fresh `request_id`; `pending` keyed by `request_id`; `MergeResult` opens the modal /
  reports exclusions; `StaleResolutions`/`UnknownResolution` re-open with the fresh set)
- Modify: `src/client/shell/src/lib/Table.svelte` (route `MergeResult`/`MergeError` to the
  controller, alongside the existing server-message routing)
- Modify: `src/client/ui-kit/src/mergeConflict.ts` + `MergeConflictModal.svelte` (consume the
  generated `MergeConflict`; group `label` from the push outcome's `name`)
- Delete (trash): `merge3`, `merge3Tree`, `takeTemplate`, `applyResolutions`, `computePull`,
  `computeRevert`, `planToUpdate` and their now-dead internal helpers from
  `src/client/core/src/merge.ts`/`templates.ts`; the corpus assertion mode in
  `mergeConformance.test.ts` (the corpus JSON STAYS as the Rust suite's fixture); the deleted
  functions' coverage in `merge.test.ts`/`templates.test.ts`. KEEP: `structuralDiff`,
  `deepEqual`, `deletePointer` (check other consumers first), `isPlacementExcluded`,
  `placementExclusions`, `snapshotBase`, `stampInstance`, `restampSubtree`, `findInstances`,
  `syncState`, and the non-wire types.
- Modify: `src/client/core/src/index.ts` (export surface after deletion — check community-facing
  breakage and note it in HISTORY)
- Tests: re-fixture `templatesController.svelte.test.ts` (fake socket answering `MergeResult`),
  `TemplateControls.test.ts`, modal tests.

**Steps:**
- [ ] Controller/shell/modal re-plumb against generated types.
- [ ] Drop the `/base` leg from the advisory gates: Task 3's `/base` capability removal makes
      `canPull`'s `canEdit(child, "/base")` leg (and `#canApplyUpdate`'s coverage of the `/base`
      change) unsatisfiable for non-GM effective owners, which would hide pull/revert/push from
      exactly the users the server now authorizes. Remove both, mirroring the server's
      `TemplateMerge` exemption of the whole-band `/base` refresh; update
      `templatesController.svelte.ts`'s `canPull` comment (the "WRITE_FIELDS (base/system) ∪
      MANAGE_EMBEDDED" cap-union description) and the controller tests that mock `canEdit`.
- [ ] Deletion pass (verify zero remaining imports of the deleted symbols repo-wide first).
- [ ] Client gates (`pnpm -r test`, `pnpm -r typecheck`, lint family); commit.

---

### Task 6: docs, skills, gates, final review, merge

**Files:**
- Rewrite: the `shadowcat-codebase-templates` skill (plugin repo
  `~/.claude/skills/shadowcat-codebase` — server-owned merge, intents, base contract; the
  "server never merges" Purpose line inverts)
- Modify: `shadowcat-codebase-documents-permissions` skill (`Document.base` entry)
- Modify: `docs/site/protocol.md` (the three intents + result/error shapes)
- Modify: `docs/PLAN.md` (M14c-5 DONE) + `docs/HISTORY.md` (delivery entry: corpus case count,
  the base-ownership fork's rationale, the TS-deletion evidence)
- Sweep: `docs/POST_WORK_FINDINGS.md` per campaign convention.

**Steps:**
- [ ] Skill rewrites in the plugin repo; commit + push there (only this campaign's files — other
  agents have in-flight edits in that repo).
- [ ] Docs updates committed on the branch.
- [ ] Full gate suite (Global Constraints list) green.
- [ ] Final two-reviewer buddy review of the whole branch; fold findings.
- [ ] Merge to main via temp detached worktree (`git merge --no-ff`), push, `gh run watch` to
  green, remove temp worktree.
