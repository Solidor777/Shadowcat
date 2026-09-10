# M20 · Full default module suite — Implementation Plan

> **For agentic workers:** Execute task-by-task in order; each task's steps use checkbox
> (`- [ ]`) syntax. Written for a sonnet-class implementer with no conversation context —
> every path, symbol and test name below is exact; read the cited code before editing it.

**Goal:** the four missing default modules (`notes` + `tables` panels, `sheet-note` +
`sheet-table` sheets) over the M19 seams; aura/sound/VFX editing on the actor sheet; a shell-wide
toast on every rejected intent; a `canCreate` advisory mirror; `firstChannel`/`buildMoveOp`/
`EmissionEditor` hoisted to core/ui-kit; the sheet-driven Playwright flows for notes and tables.

**Architecture:** each new module imports only `@shadowcat/core`/`@shadowcat/ui-kit`/
`@shadowcat/types`; sheets follow the `ActorSheet`/`ItemSheet` pattern (`basePrefix` derivation,
`createSubscriber` bridge, raw-`old` OCC through `setField`); panels follow `ActorsPanel`
(live-search subscription, `openDocument`); the Welcome frame gains the caller's projected
world-level capabilities.

**Tech stack:** Svelte 5 (runes), TypeScript, Vitest + `@testing-library/svelte`, Rust (one
Welcome field + ts-rs regen), Playwright (written here, run by the dispatcher).

**Spec:** `docs/superpowers/specs/2026-09-10-m20-default-module-suite-design.md` — read it
first; §2 (shared seams), §3–§6 (the four modules), §7 (emitters), §9 (tests), §11 (M1–M14).

**Worktree:** `C:/Dev/Shadowcat-m20`, branch `m20-module-suite`. Tasks 1–6 need nothing from
M21; Task 7 is the merge-forward that brings `searchDocuments`' `docTypes`; Tasks 8+ depend on it.

## Execution directives

**Every dispatched agent's first prompt MUST contain this paragraph verbatim:**

> The iron rule is no deferrals of existing work, or new work as it comes up - we fix this now
> unless I give my EXPRESS authorization. The only exception is if a bug or to-do has a genuine
> blocker that is already logged in a milestone in PLAN.md that has not been started yet. Another
> iron clad is rule is that when faced with a design fork, determine the best long term shape in
> keeping with our plans and goals, and implement accordingly. You only need to ask me if the
> question "what is the best long term shape in keeping with our plans and goals?" is not able to
> answer the question. Churn is not a concern. This paragraph must be copied verbatim to any
> agents dispatched in this campaign.

…plus the reporting rule: a subagent delivers its report as the Agent tool result, via
SendMessage, or by writing a named file; the prompt states which. Opus is banned for every
dispatch in this campaign.

## Global constraints

Identical to `2026-09-10-m21-search-consolidation.md`'s Global constraints (suppressions, file
sizes, sibling tests, comment rules, `trash`, explicit-path commits, background long commands,
the full gate list, the browser suite DISPATCHER-ONLY). Additionally:

- **Sheet/panel discipline** (the sheets skill's hard invariants): read the OPTIMISTIC view
  `ctx.documents`, bridge it with `const subscribe = createSubscriber((update) =>
  ctx.documents.subscribe(update))` and call `subscribe()` as the FIRST statement of every
  `$derived.by` that reads `ctx.documents` directly; every write's `old` is the RAW stored value
  read through `getPointer(doc, path)`; a sheet's root is `<div role="dialog">`; no static
  `aria-label` on a per-item control inside an `{#each}`; every human-facing attribute is `t(…)`
  text.
- **New package skeleton** = copy `src/modules/sheet-item/`'s `package.json`, `tsconfig.json`,
  `typedoc.json`, `svelte.config.js`, `vitest.config.ts`, `vitest.setup.ts` (the setup guards its
  DOM patch on `typeof HTMLCanvasElement !== "undefined"` — keep that), rename the package, then
  `pnpm install` from the repo root (updates `pnpm-lock.yaml`; commit it with the package).
- **Every test file that never touches the DOM** opens with `// @vitest-environment node`.
- **i18n**: keys in `src/client/ui-kit/src/locales/en.ts`, grouped under `notes.`, `tables.`,
  `sheetNote.`, `sheetTable.`, `intent.rejected.`; reuse `sheets.title`/`sheets.close`/
  `sheets.missing` where the existing sheets do.

---

### Task 1: hoist `firstChannel` and `buildMoveOp` into `@shadowcat/core`

**Files:**
- Modify: `src/client/core/src/chat-docs.ts` (add `firstChannel(documents: ReadableDocuments):
  string | null` — the body moves VERBATIM from `src/modules/combat-tracker/src/model.ts` with
  its `ChannelRegistryShape` helper type; import `ReadableDocuments` from `./store`),
  `src/client/core/src/chat-docs.test.ts` (the `describe("firstChannel")` block moves from
  `src/modules/combat-tracker/src/model.test.ts`), `src/client/core/src/index.ts` (export).
- Create: `src/client/core/src/move-op.ts` (`buildMoveOp(docId, targetParentId,
  currentParentId): WireOperation` moved VERBATIM from `src/modules/asset-browser/src/folderOps.ts`,
  doc example's import becomes `@shadowcat/core`), `src/client/core/src/move-op.test.ts`
  (`// @vitest-environment node`; the `buildMoveOp` cases move from `folderOps.test.ts`).
- Modify: `src/modules/combat-tracker/src/model.ts` + `model.test.ts` (delete the copy; `CombatHeader.svelte`
  imports `firstChannel` from `@shadowcat/core`), `src/modules/asset-browser/src/folderOps.ts` +
  `folderOps.test.ts` + every importer of `buildMoveOp` in that module (`rg buildMoveOp
  src/modules/asset-browser`) now importing from `@shadowcat/core`,
  `src/modules/sheet-item/src/ItemSheet.svelte` (`roll` posts to `firstChannel(ctx.documents)`;
  the buttons are `disabled` while it is `null`; the doc comment on `roll` no longer claims a
  hardcoded channel), `src/modules/sheet-item/src/ItemSheet.test.ts` (seed a `channel-registry`
  doc with a `general` key in `storeWith`'s store so the existing roll assertion still expects
  `channel: "general"`; add a test that the button is disabled with no registry), `src/client/core/src/index.ts`.

- [ ] **Step 1:** move the tests first (they fail on the missing exports); then implement.
- [ ] **Step 2:** `pnpm -r typecheck`, `pnpm --filter @shadowcat/core test`, `pnpm --filter
  @shadowcat/module-combat-tracker test`, `pnpm --filter @shadowcat/module-asset-browser test`,
  `pnpm --filter @shadowcat/module-sheet-item test` PASS; `pnpm lint`, `pnpm lint:docs`,
  `pnpm lint:props`, `pnpm docs:check-examples` PASS.
- [ ] **Step 3:** `git commit -m "refactor(core): firstChannel and buildMoveOp become shared core helpers" -- src/client/core/ src/modules/combat-tracker/ src/modules/asset-browser/ src/modules/sheet-item/`

### Task 2: `EmissionEditor` → `@shadowcat/ui-kit`

**Files:**
- Move (via `git mv` is banned — create the new file with the identical content, then `trash` the
  old one and `git add` both paths): `src/modules/actors/src/EmissionEditor.svelte` →
  `src/client/ui-kit/src/EmissionEditor.svelte`; adjust its imports (`getAppContext` from
  `./appContext`, `listAssets` from `@shadowcat/core`, `t` via the context as today).
- Modify: `src/client/ui-kit/src/index.ts` (export `EmissionEditor`, beside
  `LightEmissionEditor`), `src/modules/actors/src/ActorsPanel.svelte` and
  `src/modules/actors/src/TokenEmissionControl.svelte` (import from `@shadowcat/ui-kit`),
  their tests (the `listAssets` mock comment stays true).
- Create: `src/client/ui-kit/src/EmissionEditor.test.ts` — none exists today. Cover: each of the
  three sections toggles from `null` to a default payload through `onAura`/`onSound`/`onVfx`,
  toggling off emits `null`, a field edit emits the replacement payload, the asset lists come
  from a mocked `listAssets` (mirror `TokenEmissionControl.test.ts`'s mock).

- [ ] **Step 1:** write the new test against the moved component; implement the move;
  `pnpm --filter @shadowcat/ui-kit test`, `pnpm --filter @shadowcat/module-actors test`,
  `pnpm -r typecheck`, `pnpm lint:docs`, `pnpm lint:props` PASS. `git status --short` shows
  exactly the intended delete.
- [ ] **Step 2:** `git commit -m "refactor(ui-kit): EmissionEditor moves out of the actors module" -- src/client/ui-kit/ src/modules/actors/`

### Task 3: rejected-intent toast

**Files:**
- Modify: `src/client/shell/src/lib/worldSession.svelte.ts` (`WorldSessionOpts.onReject?:
  (reason: WireRejectReason) => void` — export `type WireRejectReason =
  z.infer<typeof RejectReasonSchema>` from `wire.ts` if no named type exists; the `WsClient`
  handler `onReject: (id, reason) => { this.#optimistic.reject(id); this.opts.onReject?.(reason); }`),
  `src/client/shell/src/App.svelte` (`onReject: (reason) => notifications.push("warning",
  t(\`intent.rejected.${reason}\`))` — `notifications` from `@shadowcat/ui-kit`'s
  `notifications.svelte.ts`, `t` from ui-kit's i18n adapter, as `Table.svelte` already does),
  `src/client/ui-kit/src/locales/en.ts` (`intent.rejected.forbidden`, `intent.rejected.conflict`,
  `intent.rejected.invalid` — player-presentable sentences from spec §2.4),
  `src/client/shell/src/lib/worldSession.test.ts` (a `reject` frame → `onReject` called once with
  the reason AND `#optimistic.reject` still ran — assert the prediction rolled back), `src/client/core/src/index.ts`
  (export the type).

- [ ] **Step 1:** failing test; implement; `pnpm --filter @shadowcat/shell test`, `pnpm -r
  typecheck`, `pnpm lint:docs`/`props` PASS.
- [ ] **Step 2:** `git commit -m "feat(shell): every rejected intent surfaces as a notification" -- src/client/`

### Task 4: `role_capabilities` on Welcome + `canCreateDoc` + `AppContext.canCreate`

**Files:**
- Modify: `src/server/src/data/document.rs` (`pub struct RoleCapabilities { pub all:
  BTreeSet<String>, pub by_type: BTreeMap<String, BTreeSet<String>> }`, ts-rs exported to
  `../../types/generated/`, `#[serde(default)]` on both fields, doc + example),
  `src/server/src/data/permission.rs` (`pub fn project_role_caps_for(caps: &RoleCaps, role:
  WorldRole) -> RoleCapabilities` beside `project_grants_for`: `all = caps.all[role]` (or empty),
  `by_type = { t: caps.by_type[t][role] }` for every `t` where that role has an entry — nothing
  about other roles crosses; doc example proving a Player projection omits the Gm's entries),
  `src/server/src/data/permission/tests.rs` (or the module's sibling test file),
  `src/server/src/ws/protocol.rs` (`Welcome.role_capabilities: RoleCapabilities`, doc: "the
  connecting user's own world-level capabilities — the `core:create` policy `apply_intent`
  consults through `role_has` — projected for their role; advisory mirror"),
  `src/server/src/ws/conn.rs` (compute `project_role_caps_for(&world_defaults.role_caps,
  ctx.world_role)` beside `actor_grants` and send it), every `ServerMsg::Welcome { … }`
  construction site (`rg "ServerMsg::Welcome \{" src/server/src`), `src/types/index.ts` (export
  the generated type), `src/client/core/src/wire.ts` (`WireRoleCapabilities`, `WireWelcome.role_capabilities`,
  the Zod object with `.default` empty maps so a frame missing it still parses — mirror how
  `#[serde(default)]` fields are mirrored elsewhere in the file), `src/client/core/src/capabilities.ts`
  (`export function canCreateDoc(docType: string, role: WorldRole, roleCaps:
  WireRoleCapabilities): boolean` per spec §2.5; delete the module-header `TODO` and rewrite the
  header sentence that says no create mirror exists), `src/client/core/src/capabilities.test.ts`
  (GM ⇒ true; `all` grant ⇒ true; `by_type[docType]` ⇒ true; `by_type[other]` ⇒ false; player
  with nothing ⇒ false), `src/client/shell/src/lib/worldSession.svelte.ts` (`#roleCaps` set in
  `#onWelcome`; `canCreate(docType): boolean` beside `canEdit`), `src/client/shell/src/lib/worldSession.test.ts`,
  `src/client/ui-kit/src/appContext.ts` (`canCreate(docType: string): boolean` — doc: advisory
  mirror of the server's Create gate, `role_has`-shaped; the baseline-message exemption is not
  mirrored), `src/client/shell/src/lib/Table.svelte` (`canCreate: (t) => session.canCreate(t)`),
  `src/client/ui-kit/src/__fixtures__/appContextTest.ts` (`canCreate: over.canCreate ?? (() =>
  (over.role ?? "player") === "gm")` — read the fixture's existing `role` default first and match
  it), every OTHER `setAppContext(`/`AppContext` literal site (`rg "setAppContext\(|: AppContext =" src --type ts --type svelte -l`,
  excluding node_modules) gains the field, `src/modules/actors/src/ActorsPanel.svelte` (the create
  form's `{#if ctx.role === "gm"}` around the create `<form>` — locate it — becomes
  `{#if ctx.canCreate(ACTOR_DOC_TYPE)}`; the other GM-only controls (hide-name, ownership) stay
  role-gated, they are Update-path GM affordances), `src/modules/actors/src/ActorsPanel.test.ts`,
  `docs/site/protocol.md` (`welcome` row names `role_capabilities`).

- [ ] **Step 1:** failing tests (server projection; client `canCreateDoc`; `worldSession.canCreate`
  reads the Welcome; `ActorsPanel` shows the form for a player with a `by_type.actor` grant and
  hides it without); implement; `cargo test --all` (background + log) regenerates the bindings;
  `git diff --exit-code src/types/generated` FAILS as expected — stage them; `pnpm -r typecheck`,
  `pnpm -r test` (background + log), `pnpm lint:docs`/`props`, clippy (both invocations), fmt PASS.
- [ ] **Step 2:** `git commit -m "feat(permissions): Welcome carries the caller's world-level capabilities; canCreate advisory mirror" -- src/server/ src/types/ src/client/ src/modules/actors/ docs/site/protocol.md`

### Task 5: emitter editors on the actor sheet

**Files:**
- Modify: `src/modules/sheet-actor/src/ActorSheet.svelte` (after the carried-light block: three
  `EmissionEditor`s? — NO: `EmissionEditor` renders all three sections in ONE component (props
  `aura`/`sound`/`vfx` + three callbacks); render ONE `<EmissionEditor aura={engine.aura ?? null}
  sound={engine.sound ?? null} vfx={engine.vfx ?? null} onAura={(v) => setEngine("aura", v)}
  onSound={(v) => setEngine("sound", v)} onVfx={(v) => setEngine("vfx", v)} />` inside a
  `<fieldset disabled={readOnly}>` (the component has no `disabled` prop; if a fieldset does not
  disable its controls in the component's markup, add a `disabled?: boolean` prop to
  `EmissionEditor` threaded to every control — and cover it in the ui-kit test from Task 2);
  read `ActorEngine`'s field names for the three emissions (`rg "pub aura|pub sound|pub vfx"
  src/server/src/data/engine/`) — no GM gate (spec §7)), `src/modules/sheet-actor/src/ActorSheet.test.ts`
  (each callback dispatches `/engine/<field>` with the raw stored object as `old`; a toggle-off
  writes `null`; a read-only sheet renders the controls disabled), `docs/site/modules/sheet-actor.md`
  (Components/Contracts: emission editing).

- [ ] **Step 1:** failing tests; implement; `pnpm --filter @shadowcat/module-sheet-actor test`,
  `pnpm -r typecheck`, `pnpm lint:docs`/`props`/`aria-labels` PASS.
- [ ] **Step 2:** `git commit -m "feat(sheet-actor): edit an actor's aura, sound and VFX emissions on the sheet" -- src/modules/sheet-actor/ src/client/ui-kit/ docs/site/modules/sheet-actor.md`

### Task 6: `@shadowcat/module-sheet-note` and `@shadowcat/module-sheet-table`

**Files:**
- Create: `src/modules/sheet-note/` (skeleton per Global constraints; `src/index.ts` exporting
  `sheetNote: Module` with `provides: [{ contract: sheetContract(NOTE_DOC_TYPE), cardinality:
  "multi" }]` and `contribute({ id: "sheet-note:sheet", contract: sheetContract(NOTE_DOC_TYPE),
  component: NoteSheet, sheet: { priority: 0 } })`; `src/NoteSheet.svelte`; `src/NoteSheet.test.ts`;
  `src/index.test.ts` (registration, modelled on `ItemSheet.test.ts`'s first describe)).
- Create: `src/modules/sheet-table/` (same skeleton; `sheetTable`, `sheet-table:sheet`,
  `TableSheet.svelte`, `RowEditor.svelte`, `EntryEditor.svelte`, `rowOps.ts` — pure helpers
  `addRow(rows, draw)`, `removeRow(rows, i)`, `moveRow(rows, i, dir)`, `setRow(rows, i, row)`,
  `defaultEntry(kind)` with `// @vitest-environment node` tests in `rowOps.test.ts` — and the
  component tests).
- Modify: `src/client/shell/src/App.svelte` (import + module list, after `sheetItem`),
  `src/client/shell/package.json` (two workspace deps), `pnpm-lock.yaml` (via `pnpm install`),
  `src/client/shell/src/lib/defaultModuleOrder.test.ts` (no change needed unless it enumerates
  sheet modules — read it; it lists panel modules only), `src/client/ui-kit/src/locales/en.ts`
  (`sheetNote.*`: `title`, `untitled`, `edit`, `save`, `cancel`, `reload`, `changedRemotely`,
  `unrenderable`, `visibility`, `visibilityPrivate`, `visibilityShared`, `sort`, `children`,
  `newChild`, `upTo`; `sheetTable.*`: `title`, `description`, `draw`, `drawCount`, `drawRule`,
  `weighted`, `formula`, `notation`, `rows`, `addRow`, `removeRow`, `moveUp`, `moveDown`,
  `label`, `weight`, `rangeLo`, `rangeHi`, `results`, `addEntry`, `removeEntry`, `entryKind`,
  `kindText`, `kindDoc`, `kindImage`, `kindDraw`, `pickDoc`, `pickTable`, `pickImage`, `alt`,
  `count`, `noChannel`), `docs/site/modules/sheet-note.md`, `docs/site/modules/sheet-table.md`,
  `docs/site/modules/index.md` (two rows), `docs/site/.vitepress/config.mts` (two sidebar
  entries after `sheet-item`).

**`NoteSheet` behaviour** — spec §3 exactly. Anchors: `parseNoteBody`/`NOTE_DOC_TYPE`/
`buildNoteDoc` from `@shadowcat/core`; `SegmentList` from `@shadowcat/ui-kit` takes
`{ segments, channel }` — pass `channel={firstChannel(ctx.documents) ?? ""}` and render the list
inside a `<fieldset disabled={channel === ""}>`; the draft: `let draft = $state<string | null>(null)`
+ `let draftBase = $state("")`; Save → `setField(ctx, docId, \`${enginePrefix}/source\`, draftBase,
draft)`; the remote-change banner shows when `draft !== null && storedSource !== draftBase`;
Reload re-seeds both from the store. Visibility: `setField(ctx, docId, "/permissions/default",
doc.permissions.default, value)`. Children: `ctx.documents.query(NOTE_DOC_TYPE)` filtered by
`parent_id === docId`, sorted by `(engine.sort ?? 0, created_at)`; each row is a `<button>` whose
visible text is the child's name (no static aria-label). "New child note" (when
`ctx.canCreate(NOTE_DOC_TYPE)`) → `const child = buildNoteDoc(ctx.world, t("sheetNote.untitled"),
"", { parentId: docId, owner: ctx.selfId }); ctx.dispatchIntent([{ op: "create", doc: child }]);
ctx.openDocument({ docId: child.id })`. Test ids: `note-title`, `note-visibility`, `note-edit`,
`note-source`, `note-save`, `note-cancel`, `note-body`, `note-child`, `note-new-child`,
`note-sort`.

**`TableSheet` behaviour** — spec §4 exactly. `TableEngine`/`TableRow`/`TableEntry`/`DrawRule`/
`RowRange` types from `@shadowcat/core` (ts-rs re-exports); every rows mutation goes
`setField(ctx, docId, \`${enginePrefix}/rows\`, engine.rows, next)` where `next` comes from a
`rowOps` helper over a `structuredClone` of the stored array (never mutate the store's object);
draw rule: `setField(ctx, docId, \`${enginePrefix}/draw\`, engine.draw, next)`; description on
`change`. Draw: `ctx.chat.drawTable({ tableId: docId, channel, count }).catch((e) =>
ctx.notify(String(e instanceof Error ? e.message : e)))`. Pickers: the `doc` and `draw` entry
pickers reuse the composer's live-search pattern (`ctx.searchDocuments(q, { limit: 20 }, …)` and,
for tables, `{ limit: 20, docTypes: [TABLE_DOC_TYPE] }` — this option lands with the Task-7
merge-forward; until then pass `{ limit: 20 }` and filter hits by `doc_type === TABLE_DOC_TYPE`
with a `TODO:` comment naming the option to switch to; Task 8 removes the filter); the `image`
picker calls `ctx.pickAsset({ kind: "image" })`. Test ids: `table-name`, `table-draw`,
`table-draw-count`, `table-description`, `table-draw-rule`, `table-notation`, `table-add-row`,
`table-row` (per row), `row-label`, `row-weight`, `row-lo`, `row-hi`, `row-remove`, `row-up`,
`row-down`, `row-add-entry`, `entry-kind`, `entry-text`, `entry-label`, `entry-alt`,
`entry-count`, `entry-remove`, `entry-pick-doc`, `entry-pick-table`, `entry-pick-image`.

- [ ] **Step 1:** failing tests per spec §9 (every `NoteSheet.test.ts`/`TableSheet.test.ts`
  live-update case runs against a real `OptimisticClient` — construct one the way
  `ActorSheet.test.ts` or `GameSettingsPanel.test.ts` does; the draft-base test mutates the store
  between Edit and Save and asserts the dispatched `old` is the ORIGINAL source).
- [ ] **Step 2:** implement; `pnpm install`; `pnpm -r typecheck`, `pnpm --filter
  @shadowcat/module-sheet-note test`, `pnpm --filter @shadowcat/module-sheet-table test`,
  `pnpm --filter @shadowcat/shell test`, `pnpm lint`, `pnpm lint:docs`, `pnpm lint:props`,
  `pnpm lint:aria-labels`, `pnpm docs:check-examples`, `pnpm run check:svelte-runtime`,
  `pnpm build`, `pnpm docs:build:portal` PASS.
- [ ] **Step 3:** `git commit -m "feat(modules): note and table sheets" -- src/modules/sheet-note/ src/modules/sheet-table/ src/client/shell/ src/client/ui-kit/src/locales/en.ts pnpm-lock.yaml docs/site/`

### Task 7: merge-forward from `main` (dispatcher-gated)

Runs only after the dispatcher confirms M21 is on `main`. `git fetch origin && git merge
origin/main` in the worktree; resolve conflicts (expected: `App.svelte`'s module list,
`en.ts`, `appContext.ts`'s `searchDocuments` opts if both sides touched it, `HISTORY.md`); the
full gate list green; `git commit` (a merge commit carries its own message — append the trailer).

- [ ] **Step 1:** merge, resolve, gates, commit.

### Task 8: `@shadowcat/module-notes` and `@shadowcat/module-tables`

**Files:**
- Create: `src/modules/notes/` (skeleton; `notes: Module` — `manifest.id: "notes"`,
  `dependencies: { "core-ui": "^0.1.0" }`, `requires: [PANEL_CONTRACT]`, contributing
  `{ id: "notes:panel", contract: PANEL_CONTRACT, order: 3, component: NotesPanel, panel: { icon:
  "📓", labelKey: "notes.tab" } }`; `src/NotesPanel.svelte`, `src/NoteTree.svelte`,
  `src/tree.ts` (`// @vitest-environment node`-tested pure helpers: `buildNoteTree(notes:
  WireDocument[]): TreeNode[]` implementing spec §5's root promotion + `(sort, created_at)`
  ordering), tests).
- Create: `src/modules/tables/` (skeleton; `tables: Module`, `tables:panel`, order 3, icon
  `📋`, `labelKey: "tables.tab"`; `src/TablesPanel.svelte`, tests).
- Modify: `App.svelte` (imports + list — panels BEFORE the sheet modules, after `chatCard`),
  `src/client/shell/package.json`, `pnpm-lock.yaml`, `defaultModuleOrder.test.ts` (both
  register-lists gain `notes, tables`; the assertions stay: `chat:panel` first, both new panels
  launcher-closed), `en.ts` (`notes.*`: `tab`, `search`, `create`, `name`, `newNote`, `delete`,
  `moveTo`, `root`, `empty`, `expand`, `collapse`; `tables.*`: `tab`, `search`, `create`,
  `name`, `newTable`, `draw`, `delete`, `empty`), `docs/site/modules/notes.md`,
  `docs/site/modules/tables.md`, `docs/site/modules/index.md`, `docs/site/.vitepress/config.mts`
  (sidebar entries after `combat-tracker`), `src/modules/sheet-table/src/EntryEditor.svelte`
  (switch the table picker to `docTypes: [TABLE_DOC_TYPE]` and delete the interim client filter +
  its `TODO:`).

Panel behaviour per spec §5/§6. Live search: copy `ActorsPanel`'s `$effect` subscription block
(cancel guard, `.then` handle capture) with `docTypes: [NOTE_DOC_TYPE]` / `[TABLE_DOC_TYPE]`.
Delete: `ctx.dispatchIntent([{ op: "delete", doc }])` (the wire delete carries the whole doc —
see `ToolRail.svelte`'s delete). Move-to: `ctx.dispatchIntent([buildMoveOp(doc.id, target,
doc.parent_id ?? null)])`. Test ids: `notes-panel`, `notes-search`, `notes-name`, `notes-create`,
`note-row` (+ `data-note-id`), `note-open`, `note-delete`, `note-move-target`, `note-move`,
`note-toggle`; `tables-panel`, `tables-search`, `tables-name`, `tables-create`, `table-row`,
`table-open`, `table-quick-draw`, `table-delete`.

- [ ] **Step 1:** failing tests per spec §9 (tree helper cases incl. hidden-parent promotion;
  panel search sends the right `docTypes`; create builds the private note with `owner: selfId`;
  gating on `canCreate`/role/owner; Move-to payload).
- [ ] **Step 2:** implement; `pnpm install`; the same gate set as Task 6 Step 2.
- [ ] **Step 3:** `git commit -m "feat(modules): notes and tables panels" -- src/modules/notes/ src/modules/tables/ src/modules/sheet-table/ src/client/shell/ src/client/ui-kit/src/locales/en.ts pnpm-lock.yaml docs/site/`

### Task 9: Playwright specs (written here, RUN by the dispatcher)

**Files:**
- Create: `src/client/shell/e2e/notes.spec.ts`, `src/client/shell/e2e/tables.spec.ts` — spec
  §9's two scenarios, dual-session (`DUAL_SESSION_TIMEOUT_MS`, the GM+invited-player seating
  flow copied from `combat-tracker.spec.ts`: world create, account create via
  `createAccount`, invite, second context redeems, GM re-enters), panels opened through
  `launcher-trigger` + `launcher-item-notes:panel` / `launcher-item-tables:panel`, every control
  driven by the test ids above, chat cards located as `chat-media.spec.ts` does
  (`page.locator(".card").filter({ hasText: … })`; the roll tooltip trigger is the element
  `SegmentList` renders for a `table_draw` segment's roll — read `SegmentList.svelte` and assert
  on its ACTUAL markup: a GM card has it, a player card does not). Every behavioural claim in a
  spec comment must be verified against the component before it is written.

- [ ] **Step 1:** write both; `pnpm --filter @shadowcat/shell typecheck` PASS; `pnpm lint` PASS.
  Do NOT run the suite. Commit subjects say `(written; dispatcher runs)`.
- [ ] **Step 2:** `git commit -m "test(e2e): notes and tables sheet-driven browser specs (written; dispatcher runs)" -- src/client/shell/e2e/`

### Task 10: docs, HISTORY/PLAN, skills, hook map, full gates

**Files:**
- `docs/HISTORY.md` (`### M20 · Full default module suite ✅` under Phase 2, M19-entry style:
  branch, spec, every delivered symbol, decisions, coverage, the two Playwright specs' status as
  the dispatcher reports it), `docs/PLAN.md` (delete the M20 section), `docs/site/modules/
  sheet-item.md` (roll channel from `firstChannel`), `docs/site/modules/asset-browser.md`
  (`buildMoveOp` from core), `docs/site/modules/actors.md` (create gated by `canCreate`).
- Skills (plugin checkout `C:/Users/emper/.claude/skills/shadowcat-codebase/skills/`; edit only
  what you need, do NOT commit there): `shadowcat-codebase-sheets/SKILL.md` (replace the
  "Notes/tables sheets — Not yet built" section with the shipped sheets: contracts, the
  draft-base OCC rule, whole-array rows, `firstChannel`-gated roll/draw), `shadowcat-codebase-tables-notes/SKILL.md`
  (UI consumers), `shadowcat-codebase-client-shell/SKILL.md` (module list; `canCreate` +
  `role_capabilities`; the reject toast; `firstChannel`/`buildMoveOp` in core),
  `shadowcat-codebase-actors-tokens/SKILL.md` (`EmissionEditor` in ui-kit; the sheet's emission
  editing; `ActorsPanel` create gate), `shadowcat-codebase-documents-permissions/SKILL.md`
  (`project_role_caps_for`), `shadowcat-codebase-realtime-sync/SKILL.md` (the Welcome field),
  `hooks/codebase-skill-reminder.py` (`sheets` glob → `src/modules/sheet-(fallback|actor|item|note|table)/`;
  `tables-notes` gains `src/modules/notes/`, `src/modules/tables/`) and
  `hooks/test-codebase-skill-reminder.sh` (a `check` line per new glob with an ABSOLUTE
  Windows-style path, e.g. `C:/Dev/Shadowcat/src/modules/notes/src/NotesPanel.svelte`; run the
  script with `bash` from the plugin checkout root and paste its output).

- [ ] **Step 1:** edits; `node scripts/check-skill-symbol-refs-cli.mjs`, `node
  scripts/check-skill-api-refs-cli.mjs`, `pnpm run test:scripts` from the worktree — zero broken
  citations you introduced.
- [ ] **Step 2:** the FULL gate list (background the long ones) + `pnpm build:all` +
  `pnpm docs:check-rust-examples`; paste every result line.
- [ ] **Step 3:** `git commit -m "docs: default module suite — history, plan, module pages" -- docs/`
- [ ] **Step 4:** report: `STATUS`, commits, gate lines, the browser suite NOT RUN (both new
  specs + the existing suite), the plugin diff stat, deviations.
