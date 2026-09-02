# M15b — Asset Browser + Generic Document Move Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **This session runs Fable:** `mainline-plan-execution` replaces those skills per its own trigger rules.

**Goal:** Ship the GM asset browser module (replacing `@shadowcat/module-assets`) on top of a new fully-generic, GM-only `Operation::Move` document intent, with a multi-capable `AppContext.pickAsset` modal seam converting every asset-picking surface.

**Architecture:** A new `Move` variant on the document `Operation` enum rides the existing intent path (authz + Create-validity at the `apply_intent` chokepoint, trusted replay in `apply_command`, per-recipient egress in `filter_command`, ECS mirror in `scene`). The browser is one new module package composed of focused components around a shared `AssetBrowser` used by both the panel and the pick modal; picking is a ui-kit stable-ref bridge so modules never import each other.

**Tech Stack:** Rust (axum/sqlx/ts-rs), Svelte 5 runes, Zod, vitest, Playwright.

**Spec:** `docs/superpowers/specs/2026-08-30-m15b-asset-browser-design.md` (delta over `docs/superpowers/specs/2026-08-30-m15-asset-pipeline-browser-design.md` §4).

## Model/Effort directives

User-directed: plan written and executed **mainline on Fable** (`mainline-plan-execution`); no `sdd-*` dispatch ladder. Subagents at the executor's discretion per standard Fable rules; the shadowcat-codebase reviewer pair is used for review checkpoints.

## Buddy-check directives

User delegated buddy-checking to the executor's judgment. Directive: run ONE full-branch buddy check (superpowers:buddy-checking, two blind reviewers + brokered debate) at the end, before merge — the Move op touches document-layer authz/egress, the same risk class whose M15a buddy check surfaced nine real defects. Per-task review stays the inline enumerative check from mainline-plan-execution.

## Global Constraints

- Comments cite SYMBOLS, never files/lines; no ephemeral refs (milestones, specs, history) in code or code-facing strings (`docs/design/doc-sweep-truthfulness-rules.md` RULES 15/16; gates `pnpm lint:comments`, `check-skill-symbol-refs`).
- No lint suppressions, no `#[expect]`, no dead code; no file over 5,000 lines; Rust test bodies in sibling files (`pnpm lint:file-size`, `pnpm lint:inline-tests`).
- Schema changes edit `src/server/migrations/0001_init.sql` in place (no incremental migrations, no backfill). Delete a stale dev DB on checksum failure.
- ts-rs: edit Rust, regenerate via `cargo test`, commit `src/types/generated/*`; then mirror in the client Zod schema (drift guard enforces parity). Never hand-edit generated files.
- Never fork a decision: shared checks are routed through one symbol, never copied. Placement checks Move shares with Create must be extracted, not duplicated.
- Doc coverage gates are errors repo-wide: every new item documented (`missing_docs` denies in Rust; `pnpm lint:docs` / `lint:props` for TS/Svelte). New Rust modules carry the inner deny pair.
- Cross-platform: `std::path` only; responsive/touch UI (≥44px targets, viewport reflow).
- All strings through `t()`/locale catalogs; keys land in every shipped catalog (no cross-locale fallback).
- Safe deletion only (`trash`, never `rm`/`git rm` as sole step).
- Commit per green task with gate-chained commits (`<gate> && git commit -- <paths>`); `pnpm -r test` before any wire-type commit.
- Debug instrumentation never committed; `debug/dumps/` is the only dump sink.

---

## Phase 1 — Server: generic document Move

### Task 1: `Operation::Move` variant, invert, snapshot arm, ts-rs

**Files:**
- Modify: `src/server/src/data/command.rs` (enum + `invert`)
- Modify: `src/server/src/data/sqlite.rs` (the `OpSnapshot` builder fn near `apply_command`; stub arms the compiler forces)
- Modify: `src/server/src/data/permission.rs`, `src/server/src/scene/mod.rs` (compiler-forced arms — minimal fail-closed stubs, completed in Tasks 3–5)
- Test: `src/server/src/data/command` test module (sibling file, existing layout)

**Interfaces:**
- Produces: `Operation::Move { doc_id: Uuid, parent_id: Option<Uuid>, old_parent_id: Option<Uuid> }` — `parent_id` = target (None = top level), `old_parent_id` = OCC pre-image + invertibility. `invert()` swaps the two. ts-rs regenerated `Operation` type.

- [ ] **Step 1: Write failing tests** — in the command tests: `move_inverts_by_swapping_parents` (invert twice = identity), and a serde round-trip of a `Move` op through JSON (wire shape `{ "op": "move", ... }` matching the existing serde tagging of the enum — read the enum's serde attributes and match them exactly).
- [ ] **Step 2: Run to verify failure** — `cargo test -p shadowcat move_inverts` fails to compile (no variant).
- [ ] **Step 3: Add the variant** with doc comments (present-tense, symbol-cited): target/pre-image semantics, GM-only note citing `apply_intent`. Implement the `invert` arm. Add compiler-forced arms elsewhere as FAIL-CLOSED stubs: snapshot builder returns an `OpSnapshot` shaped like the Update arm's (captures `permissions_at_commit`/`owner_at_commit` from the post-image — read the Update arm at the builder and mirror its field sourcing); `filter_command` Move arm DROPS the op (temporary — Task 4 completes it); scene mirror arm no-ops (Task 5 completes it); `apply_intent`/`apply_command` arms return `DataError::OpFailed("move not yet handled")` (Tasks 2–3 complete them). Every stub gets its task's real behavior in this same plan — none survives the branch.
- [ ] **Step 4: Regenerate ts-rs** — `cargo test` (bindings regenerate), verify `src/types/generated/Operation.ts` gains the variant.
- [ ] **Step 5: Run tests** — `cargo test -p shadowcat` green; `cargo fmt --check`; `cargo clippy --all-targets -- -D warnings`.
- [ ] **Step 6: Commit** (server files + regenerated types).

### Task 2: `apply_intent` Move arm — authz, Create-validity, cycle walk, hooks

**Files:**
- Modify: `src/server/src/data/sqlite.rs` (Phase-1 arm + Phase-2 apply; extract the shared placement helper from the Create arm)
- Modify: `src/server/src/data/sqlite/assets.rs` (reuse `refresh_derived_tags_for_folder_subtree`; extend `check_asset_folder_parent` with the ancestor cycle walk for a move of an EXISTING folder)
- Test: `src/server/src/data/sqlite/tests/commands_and_intents.rs` (or a new `tests/moves.rs` beside it if the subject warrants — shared fixtures in `tests/mod.rs`)

**Interfaces:**
- Consumes: Task 1's variant.
- Produces: a committed Move — envelope `parent_id` rewritten, `updated_at` bumped, derived tags refreshed for a moved folder's asset subtree, no-op short-circuit, whole-batch rollback on rejection. A shared `check_parent_placement` helper used by BOTH Create and Move arms (extraction of the Create arm's containment + combatant-parent + `check_asset_folder_parent` block — never-fork).

- [ ] **Step 1: Write failing tests** (test names are behavior statements, no plan refs):
  - GM moves an `asset_folder` under another folder → `parent_id` updated, `updated_at` bumped.
  - Non-GM Move → `DataError::Forbidden`; whole batch rolls back (a sibling Create in the same intent is absent).
  - A GM capped by `gm_role` on the target document → `Forbidden` (authz = `WorldRole::Gm` AND the resolved `Access.all` short-circuit — a capped GM floor-resolves and fails; cite `resolve_access_world`).
  - Move to own descendant → rejected (cycle); move to self → rejected; direct + deep chain cases.
  - Move a `combatant` to a non-`combat` parent → rejected; to another `combat` → accepted (Create-validity).
  - Move a `combat` under any parent → rejected (`validate_containment` on the post-image).
  - Cross-world parent → rejected; missing parent → rejected; `parent_id: None` legal where Create allows it (folder to root).
  - OCC: `old_parent_id` mismatching the stored value → `Conflict`.
  - No-op move (target == current) → success, no `updated_at` bump, `normalized_ops` carries it (invertibility) but no side effects.
  - Folder move recomputes subtree assets' folder-segment derived tags (assert via `get_asset(...).derived_tags`).
  - Move an id addressed only as an embedded child → rejected (`Conflict`: document missing — embedded children have no top-level row; assert that outcome).
  - World-bundle round-trip of a moved tree: export after a folder move, import, assert `parent_id` + derived tags survive (spec §E).
- [ ] **Step 2: Run to verify failures** (arm still stubs `OpFailed`).
- [ ] **Step 3: Implement.** Extract `check_parent_placement(tx, doc, batch_folders, batch_combats)` from the Create arm; Create arm calls it (no behavior change — existing Create tests stay green). Phase-1 Move arm: load current doc (`Conflict` if missing) → `check_command_scope` → authz (`ctx.world_role == WorldRole::Gm` && `resolve_access_world(..).all`, `CombatTransition` origin exempt like other cap gates) → OCC `old_parent_id` vs stored → if no-op, mark and skip → build post-image with new `parent_id`, run `validate_containment` + `check_parent_placement` → for an `asset_folder` target run the ancestor cycle walk (extend `check_asset_folder_parent`: walk stored ancestors of the target parent; reject if the chain contains the moved id). Phase-2: `upsert_document` the post-image (bumps `updated_at` via the existing path), then for `asset_folder` call `refresh_derived_tags_for_folder_subtree`. Snapshot capture mirrors the Update arm's pre-capture (`pre_permissions`/`pre_owners`) so READ transitions resolve (Task 4).
- [ ] **Step 4: Run tests** — the new tests + the full `cargo test -p shadowcat` (Create-arm extraction must not shift any existing result).
- [ ] **Step 5: fmt/clippy, commit.**

### Task 3: `apply_command` Move arm (trusted replay)

**Files:**
- Modify: `src/server/src/data/sqlite.rs` (the `apply_command` op loop)
- Test: `src/server/src/data/sqlite/tests/commands_and_intents.rs`

**Interfaces:**
- Consumes: Tasks 1–2. Produces: replay/undo substrate parity — a Move replays without capability gates but WITH structural validation (scope, containment via the shared helper), like the Delete/Create arms' posture.

- [ ] **Step 1: Failing test** — `apply_command` with a Move op rewrites `parent_id` and refreshes a moved folder's subtree derived tags (parity with the intent path; both write paths share the recompute, mirroring the folder-Update precedent).
- [ ] **Step 2: Verify failure.**
- [ ] **Step 3: Implement** — same structural steps as Task 2's Phase-2 minus authz/OCC-rejection (trusted): load current, `check_command_scope`, apply post-image through `check_parent_placement` (structural integrity is not exempted by trust — same rationale as `validate_property_overrides` running here), upsert, folder-subtree tag refresh, push normalized op.
- [ ] **Step 4: Tests green; fmt/clippy.**
- [ ] **Step 5: Commit.**

### Task 4: `filter_command` Move egress + READ transitions

**Files:**
- Modify: `src/server/src/data/permission.rs` (replace Task 1's drop-stub)
- Test: the permission test module (sibling file, existing layout)

**Interfaces:**
- Consumes: `OpSnapshot` fields Task 1 captures. Produces: per-recipient Move delivery — op delivered iff commit-time AND current-time whole-document `cap::READ` admit it (the conjunction rule); READ transitions synthesize like the Update arm.

- [ ] **Step 1: VERIFY the parent-independence question the spec pins** — read `resolve_access`/`effective_role`: effective READ never consults `parent_id`/ancestors (parent-independent), so a Move cannot itself flip READ. Record the finding as a test, not prose: a recipient's view of a doc is identical before/after a Move (no synthetic Create/Delete arises from Move alone).
- [ ] **Step 2: Failing tests** — (a) recipient WITH READ receives the Move op verbatim; (b) recipient without commit-time READ gets the op dropped; (c) recipient without current-time READ gets it dropped (conjunction, mirroring the Update arm's two gates); (d) legacy all-`None` snapshot drops the op on replay (same posture as other arms).
- [ ] **Step 3: Implement** the arm mirroring the `Update` arm's whole-doc gating structure (commit-time gate from the snapshot + current-time gate), minus field-delta redaction (a Move carries only envelope placement — no content bands to strip; state that in the arm's doc comment).
- [ ] **Step 4: Tests green; fmt/clippy; commit.**

### Task 5: Scene ECS mirror + broadcast integration

**Files:**
- Modify: `src/server/src/scene/mod.rs` (the committed-op mirror — replace Task 1's no-op stub)
- Test: `src/server/src/scene/tests/ecs_and_footprints.rs`

**Interfaces:**
- Consumes: committed Move ops on the broadcast/replay path. Produces: ECS parity — a token/region Move re-derives scene membership (membership is DERIVED from the doc's own `parent_id`, per `Room`'s doc), so vision/footprints/visible-cells reflect the new scene and forget the old.

- [ ] **Step 1: Failing tests** — move a token doc between two scenes via the mirror: source scene's vision-source/footprint set loses it, destination gains it; a region move re-fields the destination scene. Mirror-input posture matches `MirrorInput::Committed` (log level `error!` on failure).
- [ ] **Step 2: Implement** — the mirror arm rewrites the mirrored doc's `parent_id` exactly as the DB did (through the store-equal path the other arms use — never a hand-rolled branch) and invalidates whatever per-scene caches a placement-affecting Update invalidates today (read the Update mirror's invalidation and route through the same symbols).
- [ ] **Step 3: Full `cargo test -p shadowcat`** — the scene suites are the regression net here.
- [ ] **Step 4: fmt/clippy; commit.**

### Task 6: Client wire mirror + optimistic Move

**Files:**
- Modify: the client core wire Zod mirror (Operation schema; exact file per Phase-2 preamble below)
- Modify: the client core store module (`applyOperation`)
- Test: colocated `*.test.ts` beside each
- `pnpm -r test` gate (shared wire type).

**Interfaces:**
- Consumes: regenerated `Operation` ts-rs type. Produces: `WireOperation` Move member (Zod-validated), `applyOperation` Move handling (rewrites `parent_id`, bumps nothing else), optimistic predict + rollback via the existing correlated-intent machinery (no new client API — `dispatchIntent` already takes ops).

- [ ] **Step 1: Failing tests** — Zod accepts the serde wire shape of Move (copy the exact JSON from a Task-1 Rust serde test's output; the two suites pin the same bytes); rejects a Move missing `doc_id`. Store: applying Move updates the doc's `parent_id` in both `DocumentStore` and `OptimisticClient`; a rejected intent rolls the parent back (drive `OptimisticClient`'s existing reject path).
- [ ] **Step 2: Implement** schema member + `applyOperation` arm (client mirror of the store-equal rule — one function, same as Create/Update/Delete handling).
- [ ] **Step 3: `pnpm -r test` + `pnpm -r typecheck`** (wire change: full repo).
- [ ] **Step 4: Commit.**

---

## Phase 2 — Client: pickAsset seam + browser module

**Preamble — exact seams (verified in tree):**
- Wire mirror: `src/client/core/src/wire.ts` — `WireOperation` union + `operationSchemaImpl` (`z.discriminatedUnion("op", [...])`; two-const pattern: unannotated impl + `z.ZodType<T>`-annotated export). Delete member: `z.object({ op: z.literal("delete"), doc: DocumentSchema })`.
- Store: `src/client/core/src/store.ts` `applyOperation(docs, op)` switch on `op.op`; `src/client/core/src/optimistic.ts` `OptimisticClient` (`applyIntent`, `reject`, `rebuildView`).
- Intent send: `worldSession.svelte.ts` `dispatchIntent` → `#optimistic.applyIntent(intentId, ops)` then `ws.send({ type: "intent", intent_id, ops })`.
- Default module list: `src/client/shell/src/App.svelte` (single `modules: [...]` array; `defaultModuleOrder.test.ts` pins it); shell `package.json` carries each `@shadowcat/module-*` dep.
- AppContext literal: `src/client/shell/src/lib/Table.svelte` `setAppContext({...})`; app-level hosts mounted beside the root `<Surface>`: `TemplateModalHost` (controller-in-ui-kit pattern: `templatesController.svelte.ts` holds `pending`, host renders `MergeConflictModal` — fixed-position `.modal-scrim`, `role="dialog"`, Escape-cancel) and `NotificationHost`.
- Panel gating: `panel: { gmOnly: true }` (precedent `game-settings:index.ts`, contribution id `"game-settings:panel"`, order 5). Old assets panel: order 1, id `"assets:panel"`.
- Virtualization: chat's `computeVisibleWindow(scrollTop, clientHeight, scrollHeight, totalCount, overscan)` in `src/modules/chat/src/channels.ts` is module-PRIVATE (not importable) — the browser gets its own grid-row variant with its own tests.
- i18n: first-party keys live in `src/client/ui-kit/src/locales/en.ts` (sole locale); `ctx.i18n.addMessages` is for external modules.
- Module scaffold template: `src/modules/assets/` (`package.json` deps `@shadowcat/{core,types,ui-kit}` workspace:\*, devDeps testing-library/jsdom/sass, scripts `typecheck`/`test`; `vitest.config.ts` jsdom + svelteTesting + `vitest.setup.ts`; plus `svelte.config.js`, `tsconfig.json`, `typedoc.json`).
- Asset REST already shipped: `queryAssets(world, q)` (comma-joined `tags`), `patchAsset`, `bulkPatchAssets`, `reconvertAsset`, `originalUrl`, `startChunkedUpload` (opts incl. `folderId`, `tags`, `onProgress`, `fetchImpl`), `AssetResolver.url(uuid, variant?)`, `onListingInvalidated`.
- Picker consumers: `scene-tools/AssetPicker.svelte` (sets `controller.selectedAsset`, `ToolRail` renders it GM-only for the place tool); `actors/VisualKindEditor.svelte` — ONE shared `{#snippet assetPicker(selected, onPick)}` with three call sites (face `assetId`/`f.asset` single-pick; frames append-pick into `anim.frames`; sheet `anim.sheetAsset` single-pick) fed by a `listAssets` effect into `assetList`.
- e2e: Playwright shell suite `src/client/shell/e2e/` (`assets.spec.ts` = upload/replace/delete via `launcher-item-assets:panel`, inline `PNG_1X1` fixture; `playwright.config.ts` fixed port 31999 — rerun alone if a parallel session holds it).

### Task 7: `AssetPickController` + `AppContext.pickAsset` + overlay surface

**Files:**
- Create: `src/client/ui-kit/src/assetPickController.svelte.ts`, `src/client/ui-kit/src/assetPickController.test.ts`
- Modify: `src/client/ui-kit/src/appContext.ts` (two fields + types), `src/client/ui-kit/src/index.ts` (exports), the `setAppContextForTest` fixture (default instances)
- Modify: `src/modules/core-ui/src/Layout.svelte` (+ its test) — add an app-overlay region hosting `<Surface contract="shadowcat.surface:overlay" />` (new singleton surface, rendered last so fixed-position content stacks above the grid)
- Modify: `src/client/shell/src/lib/Table.svelte` (construct controller, wire both fields)

**Interfaces:**
- Produces:
  ```ts
  export interface PickAssetOptions { kind?: "image" | "other"; tags?: string[]; multiple?: boolean }
  /** Pending pick request; `resolve` settles the requester's promise. */
  interface PendingPick { opts: PickAssetOptions; resolve: (ids: string[] | null) => void }
  /** Stable-ref, mutate-in-place (AppContext invariant). */
  export class AssetPickController {
    pending = $state<PendingPick | null>(null);
    /** Opens a pick; a second request cancels (resolves null) the first. */
    request(opts: PickAssetOptions = {}): Promise<string[] | null> {
      this.pending?.resolve(null);
      return new Promise((resolve) => { this.pending = { opts, resolve }; });
    }
    /** Settles the pending pick with the chosen ids (ordered) or null on cancel. */
    settle(ids: string[] | null): void { const p = this.pending; this.pending = null; p?.resolve(ids); }
  }
  ```
  AppContext gains `assetPick: AssetPickController` (module-facing: the overlay component reads `pending`, calls `settle`) and the convenience overloads:
  ```ts
  pickAsset(opts: PickAssetOptions & { multiple: true }): Promise<string[] | null>;
  pickAsset(opts?: PickAssetOptions): Promise<string | null>;
  ```
  Table wires `pickAsset` to `assetPick.request(opts)` mapped `multiple ? ids : (ids?.[0] ?? null)`.

- [ ] **Step 1: Failing controller tests** — request resolves on `settle(["a"])`; cancel resolves `null`; a second `request` while pending resolves the first with `null`; `settle` clears `pending` before resolving (re-entrant `request` from a resolve callback must not clobber).
- [ ] **Step 2: Implement** controller + AppContext fields + fixture defaults (`assetPick: new AssetPickController()`, `pickAsset` mapped over it — so existing tests gain no behavior).
- [ ] **Step 3: Overlay surface** — core-ui `Layout.svelte` renders the new singleton surface region; test asserts contributions to `shadowcat.surface:overlay` render. Table.svelte wiring.
- [ ] **Step 4:** `pnpm -r test && pnpm -r typecheck`; commit.

### Task 8: Module scaffold, GM panel registration, shell swap

**Files:**
- Create: `src/modules/asset-browser/` (scaffold copied from `src/modules/assets/`: `package.json` named `@shadowcat/module-asset-browser`, `vitest.config.ts`, `vitest.setup.ts`, `svelte.config.js`, `tsconfig.json`, `typedoc.json`), `src/index.ts`, `src/AssetBrowser.svelte` (skeleton: `mode: "manage" | "pick"` prop, three-region layout markup, renders a placeholder grid from `queryAssets`), `src/AssetBrowserPanel.svelte` (thin wrapper rendering `<AssetBrowser mode="manage" />` — the contribution component), `src/index.test.ts`
- Modify: `src/client/shell/src/App.svelte` (import swap: `assetBrowser` replaces `assets` in the modules array), `src/client/shell/package.json` (dep swap), `src/client/shell/src/lib/defaultModuleOrder.test.ts`
- Modify: `src/client/ui-kit/src/locales/en.ts` (`assetBrowser.*` keys: tab/title/upload/empty/error/searchName/regex/tags/kind/sort/folderRoot + keys added per later task)
- Modify: `pnpm-workspace.yaml` only if module globs are enumerated (they are workspace-globbed via `src/modules/*` — verify, no edit expected)

**Interfaces:**
- Produces: `export const assetBrowser: Module` — manifest `{ id: "asset-browser", version: "0.1.0", dependencies: { "core-ui": "^0.1.0" }, requires: [PANEL_CONTRACT], provides: [] }`; contribution `{ id: "asset-browser:panel", contract: PANEL_CONTRACT, order: 1, component: AssetBrowserPanel, panel: { icon: "🖼️", labelKey: "assetBrowser.tab", gmOnly: true } }` where `AssetBrowserPanel` wraps `<AssetBrowser mode="manage" />`. `AssetBrowser.svelte` props: `{ mode, initialFilters?: PickAssetOptions, onConfirm?: (ids: string[]) => void, onCancel?: () => void }`.

- [ ] **Step 1: Failing tests** — index registration test (contribution id, `gmOnly: true`); AssetBrowser renders empty-state under `setAppContextForTest` with mocked `queryAssets`.
- [ ] **Step 2: Implement scaffold + skeleton;** old `@shadowcat/module-assets` stays in the tree (its package tests keep passing) but leaves the shell's module list NOW — the launcher swap is one commit.
- [ ] **Step 3:** `pnpm -r test && pnpm -r typecheck && pnpm lint`; commit.

### Task 9: Filter bar + virtualized grid

**Files:**
- Create: `src/modules/asset-browser/src/{FilterBar.svelte,AssetGrid.svelte,windowing.ts}` + colocated `.test.ts` each
- Modify: `src/modules/asset-browser/src/AssetBrowser.svelte` (compose; owns the query state + refetch)

**Interfaces:**
- Produces: `windowing.ts` `computeGridWindow(scrollTop, clientHeight, scrollHeight, totalCount, columns, overscanRows?) -> { start, end }` (grid-row variant of chat's list pattern — chat's helper is module-private, so this is a sibling implementation with its own tests, not a fork of a shared decision). `FilterBar` emits a `FilterState { name, nameIsRegex, tags: string[], kind?: "image" | "other", sort: "name" | "created" | "size" }`; `AssetBrowser` maps it 1:1 onto `queryAssets` params (`name` vs `name_regex` exclusive on the toggle), debounced (leading-edge per the debounce memory rule), keyset `cursor` load-more on scroll-bottom. `AssetGrid` renders `?variant=thumb` tiles via `ctx.assets.url(id, "thumb")`, multi-select (click, ctrl-click toggle, shift-click range), `data-testid="asset-tile"`, min 44px touch targets, 2-column reflow under the 48rem `sizeClass()` axis.

- [ ] **Step 1: Failing tests** — windowing math (rows from columns, overscan clamp, empty list); FilterBar → params mapping incl. regex toggle exclusivity and tag chips; grid selection semantics (range/toggle); listing refetch on `onListingInvalidated` + `onAssetChanged` created/moved.
- [ ] **Step 2: Implement;** wire `AssetResolver.reconcile` on each page like the old panel did.
- [ ] **Step 3:** module tests + `pnpm --filter @shadowcat/module-asset-browser typecheck`; commit.

### Task 10: Folder tree (Move UI), delete dialog, drop-to-file

**Files:**
- Create: `src/modules/asset-browser/src/{FolderTree.svelte,FolderTree.test.ts,folderOps.ts,folderOps.test.ts}`
- Modify: `AssetBrowser.svelte` (tree region + selected-folder → query `folder`/`recursive`); `src/client/ui-kit/src/locales/en.ts` (folder keys incl. delete-dialog reparent/purge copy)

**Interfaces:**
- Consumes: `asset_folder` documents from `ctx.store` (reactive subscription), `dispatchIntent`, the Task-6 Move op, `patchAsset`/`bulkPatchAssets`, `DELETE /api/asset-folders/{id}?assets=` via a new `deleteAssetFolder(id, assets: "reparent" | "delete")` added to `src/client/core/src/asset-rest.ts` (+ test) — the route shipped without a client function.
- Produces: `folderOps.ts` pure helpers — `folderChildren(docs, parentId)`, `folderPathNames(docs, id)`, `buildMoveOp(docId, targetParentId, currentParentId): WireOperation` (`{ op: "move", doc_id, parent_id, old_parent_id }`), `isDescendant(docs, maybeAncestorId, id)` (client-side pre-check for drag targets; server remains authoritative). Tree UI: create (Create op with `AssetFolderEngine { sort }`), rename (Update op on `/name` with OCC old), drag-to-reparent (HTML5 DnD dispatching the Move op) AND an accessible "Move to…" control on each node (opens a target-folder picker; keyboard/touch path — drag-only is a cross-platform defect), delete → dialog offering *reparent assets* (default) / *purge* with explicit confirm; asset/multi-asset drop on a node → `patchAsset`/`bulkPatchAssets` `folder_id`. Mobile: below the 48rem `sizeClass()` axis the tree collapses to a toggleable drawer (spec §C reflow).

- [ ] **Step 1: Failing tests** — folderOps helpers (children ordering by `sort` then name; descendant detection; move-op shape); tree renders nested folders from a seeded store; "Move to…" dispatches the Move op with real OCC `old_parent_id`; delete dialog fires the right query param per choice; drop payload → bulk call.
- [ ] **Step 2: Implement.** Optimistic Move renders instantly via the store subscription (Task 6); rejection rolls back through `OptimisticClient.reject`.
- [ ] **Step 3:** tests + typecheck; commit.

### Task 11: Preview pane + bulk toolbar

**Files:**
- Create: `src/modules/asset-browser/src/{PreviewPane.svelte,PreviewPane.test.ts,BulkBar.svelte,BulkBar.test.ts}`
- Modify: `AssetBrowser.svelte`; locales.

**Interfaces:**
- Consumes: `patchAsset` (rename/tags/folder), `bulkPatchAssets`, `deleteAsset`, `reconvertAsset`, `originalUrl`, `ctx.assets.url(id, "preview")`.
- Produces: PreviewPane — `?variant=preview` image; metadata rows (dimensions, byte sizes, content types, `conversion_note`, `original_retained`); explicit-tag chip editor (add/remove → `patchAsset { tags }` with the full next set; derived tags rendered read-only, visually distinct); rename; "Download original" anchor (`originalUrl`, only when `original_retained`); Reconvert button (disabled when no original); Delete with confirm. BulkBar (shown when selection > 1): move-to-folder (reuses the Task-10 target picker), add/remove tags, delete — one `bulkPatchAssets`/looped `deleteAsset` call set.

- [ ] **Step 1: Failing tests** — tag editor emits the full replacement set; original affordances gated on `original_retained`; bulk calls carry all selected ids; delete confirm gates the call.
- [ ] **Step 2: Implement; Step 3: tests + typecheck; commit.**

### Task 12: Upload queue + drop-zones

**Files:**
- Create: `src/modules/asset-browser/src/{UploadQueue.svelte,UploadQueue.test.ts,uploadQueue.ts,uploadQueue.test.ts}`
- Modify: `AssetBrowser.svelte` (grid drop-zone + folder-node drop target + file-input fallback); locales.

**Interfaces:**
- Consumes: `startChunkedUpload(world, file, { folderId, tags, onProgress })`, `ChunkedUploadError` (`.partial` repair: surface the created asset, do not re-upload).
- Produces: `uploadQueue.ts` headless queue class (stable-ref, `$state` list of `{ file, folderId, sent, total, status: "queued" | "uploading" | "done" | "error", error?, partial? }`, sequential execution, `retry(i)`, `AbortSignal` cancel); `UploadQueue.svelte` renders per-file progress/error/retry. Drop on grid targets the currently-selected folder; drop on a folder node targets that node.

- [ ] **Step 1: Failing queue tests** under mocked `startChunkedUpload` — progress updates, error + retry, `.partial` surfaced as done-with-warning, abort.
- [ ] **Step 2: Implement; Step 3: tests + typecheck; commit.**

### Task 13: Pick mode + overlay contribution

**Files:**
- Create: `src/modules/asset-browser/src/{AssetPickOverlay.svelte,AssetPickOverlay.test.ts,PickConfirmBar.svelte}`
- Modify: `src/modules/asset-browser/src/index.ts` (second contribution into `shadowcat.surface:overlay`), `AssetBrowser.svelte` (pick-mode behavior), locales.

**Interfaces:**
- Consumes: `ctx.assetPick` (Task 7). Produces: `AssetPickOverlay` — renders nothing until `ctx.assetPick.pending`; then a fixed-position scrim + `role="dialog"` `aria-modal` panel (mirror `MergeConflictModal`'s scrim/Escape/focus pattern) embedding `<AssetBrowser mode="pick" initialFilters={pending.opts} onConfirm={(ids) => ctx.assetPick.settle(ids)} onCancel={() => ctx.assetPick.settle(null)} />`. Pick mode: mutation affordances hidden (no upload/delete/tag-edit/bulk/folder-mutations; tree + filters remain), single-pick confirms on double-click or the confirm bar, `multiple` keeps ordered selection (badges show pick order) with a confirm bar. Overlay is usable by ANY member (panel stays gmOnly; the overlay contribution is not gm-gated).

- [ ] **Step 1: Failing tests** — overlay hidden with no pending; renders on `request()`; confirm settles ids in pick order; Escape settles null; mutation affordances absent in pick mode (assert upload/delete controls not rendered); `kind`/`tags` from opts preset the filters.
- [ ] **Step 2: Implement; Step 3:** `pnpm -r test` (ui-kit + module + shell touched); commit.

### Task 14: Convert consumers

**Files:**
- Modify: `src/modules/scene-tools/src/AssetPicker.svelte` (+ its test) — add a "browse…" button: `const ids = await ctx.pickAsset({ kind: "image" }); if (ids) controller.selectedAsset = ids;` (single overload returns the id).
- Modify: `src/modules/actors/src/VisualKindEditor.svelte` (+ its test) — delete the `assetPicker` snippet, the `assetList` state and its `listAssets` effect; replace the three call sites with pick buttons: face/`f.asset`/sheet → `pickAsset({ kind: "image" })` writing the same state fields; frames → `pickAsset({ kind: "image", multiple: true })` REPLACING `anim.frames` wholesale with the ordered result (the picker's ordered multi-select supersedes append-one-at-a-time; the existing frame-list remove buttons stay). Completeness rules (`animSourceComplete`, `faceRowComplete`) untouched.
- Locale keys for the browse/pick buttons.

- [ ] **Step 1: Failing tests** — browse button calls `pickAsset` and applies the result (mock via `setAppContextForTest` override); VisualKindEditor frames flow writes the ordered array; cancel (null) leaves state untouched.
- [ ] **Step 2: Implement; Step 3:** `pnpm -r test`; commit.

### Task 15: Retire `@shadowcat/module-assets`

**Files:**
- Delete (via `trash`, git-add the removals): `src/modules/assets/` entire package
- Modify: `src/client/ui-kit/src/locales/en.ts` (remove now-unreferenced `assets.*` keys — grep first; keys still referenced by other modules stay), docs site per-module pages (`docs/site/` — replace the assets module page with an asset-browser page, update any index/nav listing), any straggler references (`grep -rn "module-assets\|@shadowcat/module-assets\|\"assets\"" src docs --include-glob` sweep; `defaultModuleOrder` already swapped in Task 8)

- [ ] **Step 1: Sweep** — enumerate every reference to the old package (workspace deps, docs, tests, e2e testids `launcher-item-assets:panel`).
- [ ] **Step 2: Delete + fix** — shell e2e `assets.spec.ts` retargets the new panel (`launcher-item-asset-browser:panel`) with its upload/replace/delete flow rewritten against the new UI (this spec is extended in Task 16).
- [ ] **Step 3:** `pnpm install` (lockfile), `pnpm -r test && pnpm -r typecheck && pnpm lint && pnpm lint:file-size && pnpm lint:comments`; commit.

### Task 16: e2e + full gate sweep

**Files:**
- Modify/Create: `src/client/shell/e2e/asset-browser.spec.ts` (supersedes `assets.spec.ts` per Task 15)

**Scenarios (one spec file, serial):**
1. Upload a >1-chunk file (9 MiB `Buffer.alloc` octet-stream via the drop-zone/file input), watch queue progress, then tag it in the preview pane and find it via a tag-chip filter (the parent design's carried e2e).
2. Small PNG upload → thumbnail appears (parity with the retired spec) → replace → delete.
3. Create two folders, move one under the other via the accessible "Move to…" control, assert the tree nesting updates (exercises the Move op end-to-end).

- [ ] **Step 1: Write the spec** (reuse `fixtures.ts` login/world helpers, `PNG_1X1` inline fixture pattern).
- [ ] **Step 2: Run** `pnpm --filter @shadowcat/shell e2e` ALONE (port 31999 is fixed; a parallel session's server poisons the run).
- [ ] **Step 3: Full gates** — `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test -p shadowcat`, `pnpm -r test`, `pnpm -r typecheck`, `pnpm lint`, `pnpm lint:docs`, `pnpm lint:props`, `pnpm lint:comments`, `pnpm lint:file-size`, `pnpm lint:inline-tests`, `pnpm docs:check-examples`, core `test:e2e`, shell `e2e`; commit.

### Task 17: Docs, skills, close-out

- [ ] **Step 1: `docs/design/ARCHITECTURE.md`** — document Move under the document-mutation invariants: GM-only + uncapped (`Access.all`), Create-validity via the shared placement helper, single chokepoint with per-type hooks, egress conjunction.
- [ ] **Step 2: Docs site** — wire-protocol page gains the Move op; module page swap verified (Task 15); `pnpm build:all` green (embed ordering: client build first).
- [ ] **Step 3: Skill-update gate** (plugin checkout `~/.claude/skills/shadowcat-codebase/`): `-assets` (folder move now exists — replace the "no route" gotcha; `pickAsset`/browser module seams), `-documents-permissions` (Move op, envelope `parent_id` no longer immutable-forever — "rewritten only by `Operation::Move`"), `-client-shell` (`pickAsset`/`assetPick`/overlay surface, module list swap), `-actors-tokens` (VisualKindEditor picking via `pickAsset`). Dispatch `shadowcat-codebase:shadowcat-spec-reviewer` on the skill diff; run `node scripts/check-skill-symbol-refs-cli.mjs` + `pnpm run test:scripts`; bump plugin `version`; commit + push in the plugin repo.
- [ ] **Step 4: Tracking docs** — `PLAN.md` (M15 entry closes; M14d note: panel conventions set here), `HISTORY.md` M15b entry, `TODO.md` (nothing expected; log any deferral surfaced in flight), memory state file update.
- [ ] **Step 5: Buddy check** (per the directives section) on the full branch diff; fold fixes; re-run gates.
- [ ] **Step 6: Merge + push** (milestone completion), `gh run watch` the CI matrix.
