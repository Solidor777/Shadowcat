# Fix ui_state Clobber Race (Per-Slice Merge Writes) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate the whole-blob last-writer-wins race on `PUT /api/me/ui-state` (OPEN_BUGS.md: concurrent sessions of one user clobber each other's `ui_state` slices) by narrowing write granularity: the client persists only the changed `global`/per-world slice, and the server merges per top-level key inside one transaction.

**Architecture:** Server: `SqliteRepository::merge_ui_state(user, patch, max_bytes)` replaces `set_ui_state` — single-tx read+merge+write, each top-level patch key replaces the stored key wholesale EXCEPT `worlds`, whose entries each replace only `worlds.<id>`; the 64 KiB cap applies to the MERGED serialization. Route `put_ui_state` validates object-shape (body + `worlds`) and delegates. Client: `sessionState.svelte.ts` tracks dirty slices (`global` flag + world-id set) and `persist()` sends a `UiStatePatch` containing only dirty slices; debounce shape (leading-edge + trailing catch-up) unchanged. `GET /api/me/ui-state` unchanged.

**Tech Stack:** Rust (axum, sqlx/SQLite, serde_json), TypeScript (Svelte 5 shell package, Vitest), Playwright e2e.

## Model/Effort directives

- Dispatcher: mainline — this session owns the SDD loop (recorded 2026-07-31; user opted into SDD for this work; campaign context lives here).
- Implementer: `shadowcat-coder` (sonnet, effort **medium**). Escalation: `shadowcat-coder-opus` (opus, high) per CLAUDE.md §3 before any human escalation.
- Per-task review: `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` pair (effort **high** each). Escalation: the `-opus` twins when findings read shallow/uncertain.
- Final whole-branch review: `shadowcat-spec-reviewer-opus` + `shadowcat-code-reviewer-opus` (opus, high — fires once).

## Buddy-check directives

Standing rule: no task is buddy-check-flagged. Every task gates through the two-reviewer pair above (spec + code), which is this project's review checkpoint standard; escalate to the `-opus` twins on shallow/uncertain findings rather than convening a buddy check.

## Global Constraints

- **Reviewers have NO shell/write access (user directive).** Pre-generate every review diff to a file under the plan workspace; relay gate outputs (test/lint/typecheck results) verbatim in the review brief. Never commit or edit the tree while a reviewer subagent is running.
- **Merge semantics are stated in exactly ONE place per side** — server: `merge_ui_state`'s doc + body; client: `UiStatePatch`'s doc + `buildPatch()`. No second code path may re-derive them (never-fork rule). Parity is pinned by the server http merge test AND the client granularity tests asserting the same slice-preservation behavior.
- **Vitest does not typecheck** — every client task's verification includes `pnpm --filter @shadowcat/shell typecheck` in addition to the test run.
- **No data migrations**: `users.ui_state` storage shape is unchanged (single JSON column); `migrations/0001_init.sql` is untouched.
- **Wire route unchanged in name/method** (`PUT /api/me/ui-state`); only body semantics narrow from "replace whole blob" to "merge patch". Server and client ship together (pre-customer; no compat shim).
- **Local matrix replaces CI watch** (user directive): server `cargo test`/`cargo fmt --check`/`cargo clippy --all-targets -- -D warnings`; client `pnpm -r typecheck`, `pnpm -r test`, `pnpm lint`; e2e `pnpm --filter @shadowcat/core test:e2e` and `pnpm --filter @shadowcat/shell e2e`.
- **No debug code in commits** (`dbg!`, `println!`, `console.log`, `debugger`).
- Commit messages end with the standard Co-Authored-By / Claude-Session trailer used on this branch.

## Pre-existing work on this branch (disclosure to reviewers)

Tasks 1's implementation and Task 2's `api.ts` portion were written mainline in this session BEFORE the switch to SDD, are uncommitted in the working tree, and have passing targeted tests (`cargo test ui_state --lib`: 3/3). Task 1 commits that work verbatim and goes straight to the review pair; Task 2's implementer inherits the uncommitted `api.ts` diff and builds the rest.

---

### Task 1: Server — `merge_ui_state` + route semantics (commit existing work, then review)

**Files:**
- Modify: `src/server/src/data/sqlite.rs` (replace `set_ui_state` with `merge_ui_state`; replace test `ui_state_round_trips_and_defaults_to_none` with `ui_state_merges_per_top_level_key_and_per_world` + `ui_state_merge_caps_the_merged_result_not_the_patch` + helper `ui_state_of`)
- Modify: `src/server/src/http/routes.rs` (`put_ui_state` merge semantics + `worlds`-object validation; `MAX_UI_STATE_BYTES` doc)
- Modify: `src/server/src/http/mod.rs` (test `ui_state_get_put_round_trip_and_validation`: add worlds-only-patch-preserves-global assertions + non-object-`worlds` 422 assertion)

**Interfaces:**
- Produces: `pub async fn merge_ui_state(&self, user: Uuid, patch: &serde_json::Value, max_bytes: usize) -> Result<(), DataError>` on `SqliteRepository`. Semantics: single tx; each top-level key of `patch` replaces the stored key wholesale except `worlds`, whose entries each replace `worlds.<id>`; absent keys untouched; cap on merged serialization → `DataError::TooLarge` (maps to 422); unknown user → `NotFound`; non-object patch/`worlds` → `OpFailed` (route pre-validates to 422).
- `set_ui_state` is REMOVED (no remaining references — verified by grep).

**This task's implementation already exists in the working tree (see disclosure above).** Steps:

- [ ] **Step 1: Run the full server gate on the existing diff**

Run (from `src/server/`): `cargo test` (full suite), `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`
Expected: all green (baseline was 1470/0 before this diff; the diff replaces one test with two).

- [ ] **Step 2: Commit the server work only**

```bash
git add src/server/src/data/sqlite.rs src/server/src/http/routes.rs src/server/src/http/mod.rs
git commit -m "fix(server/http): ui-state writes merge per slice in one tx, ending whole-blob clobber"
```
(`src/client/shell/src/lib/api.ts` stays uncommitted — it belongs to Task 2.)

- [ ] **Step 3: Review pair** — generate the review package for this commit range; dispatch `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` with the diff file, this task section, and the relayed gate outputs. Findings enter the standard fix loop (fixes via `shadowcat-coder` dispatch, never mainline edits).

---

### Task 2: Client — dirty-slice tracking in `sessionState.svelte.ts`

**Files:**
- Modify: `src/client/shell/src/lib/api.ts` (ALREADY IN TREE, uncommitted: `UiStatePatch` interface + `putUiState(patch: UiStatePatch, opts)`)
- Modify: `src/client/shell/src/lib/sessionState.svelte.ts`
- Modify: `src/client/shell/src/lib/sessionState.test.ts`

**Interfaces:**
- Consumes: `putUiState(patch: UiStatePatch, opts?: { keepalive?: boolean })` from `api.ts`; server merge semantics from Task 1.
- Produces: unchanged public exports of `sessionState.svelte.ts` (`loadSessionState`, `getSessionState`, `setLastWorld`, `getPanelLayout`, `setPanelLayout`, `getChatRead`, `setChatRead`, `flushSessionState`, `flushOnUnload`) — internals now persist per-slice.

- [ ] **Step 1: Write the failing granularity tests** — replace the bodies of the two persistence-asserting tests and ADD two new tests in `sessionState.test.ts`:

```ts
test("a world-slice change persists ONLY that world's slice (no global, no other worlds)", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue({
    global: { locale: "en", lastWorld: null },
    worlds: { wOther: { chatRead: { general: 1 } } },
  });
  const put = vi.spyOn(api, "putUiState").mockResolvedValue();
  await loadSessionState();
  setPanelLayout("w1", { version: 1 });
  await flushSessionState();
  const patch = put.mock.calls.at(-1)?.[0];
  expect(patch?.worlds).toEqual({ w1: { panelLayout: { version: 1 } } });
  expect(patch?.global).toBeUndefined();
  expect(patch?.worlds?.wOther).toBeUndefined();
});

test("a global change persists ONLY the global slice", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue({
    global: { locale: "en", lastWorld: null },
    worlds: { w1: { panelLayout: { version: 1 } } },
  });
  const put = vi.spyOn(api, "putUiState").mockResolvedValue();
  await loadSessionState();
  setLastWorld("w2");
  await flushSessionState();
  const patch = put.mock.calls.at(-1)?.[0];
  expect(patch?.global?.lastWorld).toBe("w2");
  expect(patch?.worlds).toBeUndefined();
});
```

Also update the four existing persistence assertions to the patch shape (optional-chaining on `.global` / `.worlds`):
- `setLastWorld` test: `expect(put.mock.calls.at(-1)?.[0].global?.lastWorld).toBe("w2");`
- locale test: `expect(put.mock.calls.at(-1)?.[0].global?.locale).toBe("zz");`
- `setPanelLayout` test: `expect(put.mock.calls.at(-1)?.[0].worlds?.w1?.panelLayout).toBe(blob);`
- `setChatRead` test: `expect(put.mock.calls.at(-1)?.[0].worlds?.w1?.chatRead).toBe(blob);`
The `flushOnUnload` test's `objectContaining` assertion stays valid as written.

- [ ] **Step 2: Run tests to verify the new ones fail**

Run: `pnpm --filter @shadowcat/shell test`
Expected: the two new granularity tests FAIL (current code sends the whole blob, so `patch.global` is always present).

- [ ] **Step 3: Implement dirty-slice tracking** — in `sessionState.svelte.ts`, replace the persist internals (keep `COOLDOWN_MS`, the leading-edge + trailing-catch-up debounce shape, and the `loaded` guard):

```ts
import { getUiState, putUiState, type UiState, type UiStatePatch } from "./api";

// Dirty-slice tracking: persist() sends ONLY the slices marked since the last
// successful write. Write granularity is the concurrency control — concurrent
// sessions of one account contend only on slices both actually write, so a
// session can never revert a slice it did not touch (the server merges per
// top-level key / per world id).
const dirty = { global: false, worlds: new Set<string>() };

function buildPatch(): UiStatePatch {
  const patch: UiStatePatch = {};
  if (dirty.global) patch.global = state.global;
  if (dirty.worlds.size > 0) {
    patch.worlds = {};
    for (const id of dirty.worlds) {
      const w = state.worlds[id];
      if (w) patch.worlds[id] = w;
    }
  }
  return patch;
}

async function persist(): Promise<void> {
  const hadGlobal = dirty.global;
  const hadWorlds = [...dirty.worlds];
  const patch = buildPatch();
  dirty.global = false;
  dirty.worlds.clear();
  if (patch.global === undefined && patch.worlds === undefined) return;
  try {
    await putUiState(patch);
  } catch (e) {
    // Re-mark the lost slices so the next scheduled persist retries them.
    if (hadGlobal) dirty.global = true;
    for (const id of hadWorlds) dirty.worlds.add(id);
    logger.warn("ui_state persist failed", e);
  }
}
```

Mutators mark their slice dirty before scheduling (`schedulePersist()` itself is unchanged):
- `setLastWorld` and the locale-subscribe callback: `dirty.global = true; schedulePersist();`
- `setPanelLayout(world, …)` / `setChatRead(world, …)`: `dirty.worlds.add(world); schedulePersist();`

`flushSessionState` (test/teardown helper) keeps its shape (clear timer + `pendingDuringCooldown`, then `await persist()` — the new empty-patch early-return makes it a no-op when nothing is dirty).

`flushOnUnload` gates on dirty slices instead of `pendingDuringCooldown` and sends the patch:

```ts
/** Best-effort flush on page hide/unload: a change made during the cooldown is
 * otherwise only written by the trailing timer, which never fires if the tab
 * closes first. `keepalive` lets the PUT survive the unload. */
export function flushOnUnload(): void {
  if (!loaded || (!dirty.global && dirty.worlds.size === 0)) return;
  const patch = buildPatch();
  dirty.global = false;
  dirty.worlds.clear();
  pendingDuringCooldown = false;
  void putUiState(patch, { keepalive: true }).catch((e) => logger.warn("ui_state unload flush failed", e));
}
```

- [ ] **Step 4: Run tests + typecheck + lint to verify green**

Run: `pnpm --filter @shadowcat/shell test && pnpm --filter @shadowcat/shell typecheck && pnpm lint`
Expected: all PASS (including the updated existing tests).

- [ ] **Step 5: Commit**

```bash
git add src/client/shell/src/lib/api.ts src/client/shell/src/lib/sessionState.svelte.ts src/client/shell/src/lib/sessionState.test.ts
git commit -m "fix(shell): persist only dirty ui-state slices, matching the server's per-slice merge"
```

- [ ] **Step 6: Review pair** — same protocol as Task 1 Step 3 (diff file + task section + relayed gate outputs).

---

### Task 3: Verification matrix + documentation sync

**Files:**
- Modify: `docs/OPEN_BUGS.md` (remove the ui_state race entry)
- Modify: `docs/CLOSED_BUGS.md` (log the resolution: per-slice merge writes, commits from Tasks 1-2)
- Modify: `docs/POST_WORK_FINDINGS.md` (append a dated resolution note to the "panels reload test flaked" entry: the restore-assert failure mode is fixed by the per-slice merge; the render-ready class remains open with 2 members)
- Modify: `.claude/skills/shadowcat-codebase-client-shell/SKILL.md` (the `sessionState.svelte.ts` seam description: per-world slices now persist as `UiStatePatch` partial writes merged server-side per slice; whole-blob PUT no longer exists)

**Interfaces:**
- Consumes: Tasks 1-2 committed; the e2e suite's `panels.spec.ts` persistResponse predicate (`body.split('"assets:panel"').length - 1 >= 2` on the PUT payload) — with per-slice patches the dock-carrying PUT still contains the world's layout blob listing `assets:panel` in both `compact.order` and the expanded zone groups, so the predicate still matches; verify, don't assume.

- [ ] **Step 1: Full local matrix**

Run, in order (client build first — embed ordering):
1. `pnpm build`
2. from `src/server/`: `cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings`
3. `pnpm -r typecheck && pnpm -r test && pnpm lint`
4. `pnpm --filter @shadowcat/core test:e2e`
5. `pnpm --filter @shadowcat/shell e2e`
Expected: all green. The ui-e2e suite specifically exercises the fixed path (`panels.spec.ts` reload test). If the panels reload test fails at the persistResponse wait, the predicate broke — fix the SPEC's predicate to match the patch body (the product behavior is the spec here), re-run.

- [ ] **Step 2: Repeat the previously-flaky spec for confidence**

Run (from `src/client/shell/`): `npx playwright test e2e/panels.spec.ts` five times.
Expected: 5/5 green (pre-fix, the full-suite failure rate was ~2/3 under parallel load; the per-slice merge removes the interference mechanism).

- [ ] **Step 3: Documentation sync** — apply the four doc edits listed under Files. The OPEN_BUGS entry moves to CLOSED_BUGS (with commit refs and the fix shape); POST_WORK_FINDINGS gains the dated resolution note; the client-shell skill seam text updates per the reviewed skill-update gate.

- [ ] **Step 4: Commit**

```bash
git add docs/OPEN_BUGS.md docs/CLOSED_BUGS.md docs/POST_WORK_FINDINGS.md .claude/skills/shadowcat-codebase-client-shell/SKILL.md
git commit -m "docs: close the ui-state clobber bug; sync findings + client-shell skill seam"
```

- [ ] **Step 5: Review pair** — the spec reviewer additionally verifies the skill diff under the reviewed skill-update gate (accuracy, no omission/drift/broken pointers).

---

## Final Review

Whole-branch review (merge-base `main`..HEAD) by `shadowcat-spec-reviewer-opus` + `shadowcat-code-reviewer-opus` with the full-branch review package, the ledger's deferred minors, and this plan. Then merge `fix-ui-state-clobber` into main `--ff-only`, push, delete the branch (finishing-a-development-branch, autonomous-commit rules).
