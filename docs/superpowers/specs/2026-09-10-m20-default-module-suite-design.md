# M20 · Full default module suite — Design

**Status:** Drafted 2026-09-10 (Phase-2 completion campaign; designed without a brainstorming
dialogue — every fork is decided under "what is the best long-term shape in keeping with our plans
and goals?" and recorded in §11 with the alternatives considered). Consumes M12c (sheet registry),
M14d (tracker), M15b (asset browser, `Operation::Move`, `pickAsset`), M18 (emission component
model + `EmissionEditor`), M19 (`table`/`note` doc types, `SegmentList`, `buildTableDoc`/
`buildNoteDoc`/`parseNoteBody`, `ChatApi.drawTable`) and M21 (`searchDocuments` `docTypes`).
Closes Phase 2's module-suite milestone.

## 0. Where the codebase already is

Reconnaissance against `main` (`826b9b15`):

- **Two of PLAN.md's four named modules already shipped.** The combat tracker UI is
  `@shadowcat/module-combat-tracker` (M14d); the asset browser UI is
  `@shadowcat/module-asset-browser` (M15b). Nothing about either is re-done here.
- **Emitter editors exist for creation and for tokens, not for an existing actor.**
  `EmissionEditor.svelte` (aura/sound/VFX; `src/modules/actors/src/`) drives the actors panel's
  CREATE form (`pendingAura`/`pendingSound`/`pendingVfx`) and `TokenEmissionControl` (per-token
  `/engine/overrides`); `LightEmissionEditor` (`@shadowcat/ui-kit`) drives `/engine/light` on the
  actor sheet and `TokenLightControl`. **No writer of an actor's `/engine/aura`, `/engine/sound`
  or `/engine/vfx` exists after creation** — `ActorSheet` edits only `light`. An actor's emissions
  are frozen at the create form.
- **Tables and notes have no UI at all.** No `shadowcat.sheet:note`/`shadowcat.sheet:table`
  provider (a note or table opens in `FallbackSheet`, which shows its empty `system` band), and no
  panel creates, lists, or opens one — `buildNoteDoc`/`buildTableDoc`/`parseNoteBody` have only
  tests and e2e suites as callers; `ChatApi.drawTable`'s only caller is the chat card's Draw
  button on a `doc_link` segment.
- **A rejected intent is silent.** `WorldSession`'s `onReject` handler calls
  `OptimisticClient.reject(id)` and nothing else, so an OCC `conflict`, a `forbidden` write or an
  `invalid` engine body rolls the optimistic view back with no user-visible signal. Every sheet
  built on `setField` inherits that.
- **The client cannot mirror `core:create`.** `WorldSession.canEdit` mirrors the Update gate via
  `resolveCaps`/`canWritePath` over the Welcome's `world_default_grants`, but the Welcome does not
  carry the world's `role_caps` (`WorldCapDefaults.role_caps`, the `core:create` policy
  `apply_intent` consults through `role_has`), and `capabilities.ts` carries a `TODO` for exactly
  this mirror. Panels therefore gate creation on `ctx.role === "gm"`, hiding the control from a
  player the GM has granted `core:create`.
- **Two shared decisions are module-local copies.** `firstChannel(documents)` lives in
  `module-combat-tracker`'s `model.ts` while `ItemSheet.roll` hardcodes `"general"`; `buildMoveOp`
  lives in `module-asset-browser`'s `folderOps.ts`. Each new consumer (a note tree, a table sheet's
  Draw) would fork them.
- M19's design (§8) left the sheet-driven UI flows for tables and notes to this milestone; no
  Playwright spec exercises either.

## 1. Goals / non-goals

**Goals**

1. Four new first-party modules over the M19 seams, each importing only `@shadowcat/core`/
   `@shadowcat/ui-kit`/`@shadowcat/types` and each independently replaceable:
   `@shadowcat/module-notes` (panel), `@shadowcat/module-tables` (panel),
   `@shadowcat/module-sheet-note`, `@shadowcat/module-sheet-table`.
2. Emitter editing completed: `EmissionEditor` hoisted to `@shadowcat/ui-kit`; the actor sheet
   edits `aura`/`sound`/`vfx` beside `light`.
3. Rejected-intent feedback shell-wide: every `reject` frame surfaces through the notification
   sink `AppContext.notify` already uses.
4. A `canCreate(docType)` advisory mirror of the server's `core:create` gate, fed by a Welcome
   projection of the caller's own world-level capabilities — closing `capabilities.ts`'s TODO.
5. `firstChannel` and `buildMoveOp` hoisted into `@shadowcat/core`; every consumer reads the one
   symbol.
6. The sheet-driven browser flows M19 deferred: `notes.spec.ts` and `tables.spec.ts`.

**Non-goals** (each a decision, §11)

- No WYSIWYG/rich-text editor — the note body is server-derived; the client edits markdown and
  renders the derived body.
- No drag-and-drop tree reorder; ordering is the `sort` field plus a Move-to control.
- No per-row visibility, table import/export, or a global search panel.
- No emitter PLAYBACK (Phase 3 by design); no change to the emission component model.
- No change to `SheetHost`/`SheetsController`/`pickSheet` — the registry is consumed, not
  extended.

## 2. Shared seams (core / ui-kit / shell)

### 2.1 `firstChannel` → `@shadowcat/core` (`chat-docs.ts`)

`export function firstChannel(documents: ReadableDocuments): string | null` moves verbatim from
`module-combat-tracker`'s `model.ts` (which imports it from core; its own copy and test move
with it). Returns the first key of the world's `channel-registry` singleton, `null` before the
registry has arrived. **No `"general"` literal survives in any module**: `ItemSheet.roll` posts to
`firstChannel(ctx.documents)` and its roll buttons are disabled while it is `null` — the
registry is server-seeded (`ChannelRegistryEngine::seed` holds `general`), so `null` is only the
pre-resync window, and a disabled button is the honest state there (§11 M9).

### 2.2 `buildMoveOp` → `@shadowcat/core` (`move-op.ts`)

`export function buildMoveOp(docId: string, targetParentId: string | null, currentParentId:
string | null): WireOperation` moves verbatim (signature unchanged — the caller supplies the TRUE
current parent as the OCC pre-image) from `module-asset-browser`'s `folderOps.ts` (which imports
it; its `describe` block moves from `folderOps.test.ts` to core's `move-op.test.ts`). Consumers:
the folder tree (unchanged behaviour) and the notes panel's Move-to.

### 2.3 `EmissionEditor` → `@shadowcat/ui-kit`

`EmissionEditor.svelte` and `EmissionEditor.test.ts` move from `src/modules/actors/src/` to
`src/client/ui-kit/src/`, exported from ui-kit's index beside `LightEmissionEditor`. Its
`listAssets` call already resolves from core. `ActorsPanel` and `TokenEmissionControl` import it
from ui-kit; the actors module's own test mocks move to the ui-kit test. Props and behaviour are
unchanged — a pure move, proven by the moved test suite passing without edits beyond import paths.

### 2.4 Rejected-intent feedback (shell)

`WorldSession`'s `WsClient` handler `onReject: (id, reason) => …` gains one line beside
`this.#optimistic.reject(id)`: a notification through the same sink `Table.svelte` wires
`AppContext.notify` to (`notifications.push("warning", text)`). `WorldSession` receives the sink as
a constructor dependency (`onReject?: (reason: RejectReason) => void` in its opts, wired by
`App.svelte` to `notifications.push` with the resolved i18n text), keeping `WorldSession` free of
Svelte. Text: `t("intent.rejected.<reason>")` — three keys (`forbidden`, `conflict`, `invalid`),
player-presentable ("The server refused that edit: you may not change this." / "…someone else
changed it first — reload the field and try again." / "…the value was invalid."). Unit test: a
`reject` frame produces exactly one push with the mapped key's text; the rollback still happens.

### 2.5 `canCreate(docType)` — the Create-gate mirror

- Wire: `ServerMsg::Welcome` gains `role_capabilities: RoleCapabilities { all: BTreeSet<String>,
  by_type: BTreeMap<String, BTreeSet<String>> }` — the CONNECTING user's own world-level
  capabilities, projected from `WorldCapDefaults.role_caps` for `ctx.world_role` by a new
  `data::permission::project_role_caps_for(&role_caps, role)` beside `project_grants_for` (the
  same projection rule: nothing about other roles crosses). ts-rs exported; Zod mirror; the
  `docs/site/protocol.md` `welcome` row names it.
- `@shadowcat/core` `capabilities.ts`: `export function canCreateDoc(docType: string, role:
  WorldRole, roleCaps: RoleCapabilities): boolean` mirroring `apply_intent`'s Create arm:
  `role === "gm"` ⇒ true; else `roleCaps.all` or `roleCaps.by_type[docType]` contains
  `"core:create"`. The baseline-message exemption is not mirrored (no UI creates a `message`
  document); the doc comment says so. The `TODO` is deleted.
- `WorldSession.canCreate(docType)` reads the Welcome's `role_capabilities` through
  `canCreateDoc`; `AppContext.canCreate(docType: string): boolean` beside `canEdit`, advisory
  (the server re-checks at `apply_intent`); every `setAppContext` fixture defaults it to
  `() => false`... EXCEPT that a fixture whose `role` is `"gm"` defaults it to `() => true`, so
  existing GM-flow tests keep their affordances. `ActorsPanel`'s create form and the two new
  panels gate on it.

## 3. `@shadowcat/module-sheet-note` — `NoteSheet.svelte`

Registers `sheetContract(NOTE_DOC_TYPE)` at priority 0 (id `sheet-note:sheet`). Standard sheet
props (`docId`, `systemPrefix`, `close`); `basePrefix`/`enginePrefix`/`namePrefix` derived by
stripping the trailing `/system` (the sheets pattern); every `$derived.by` reading
`ctx.documents` calls the `createSubscriber` bridge first; every write carries the RAW stored value
as OCC `old`. Root `<div role="dialog">`.

- **Header:** title input over `namePrefix` (`setField` on change), close.
- **Visibility** (rendered when `ctx.canEdit(doc, "/permissions/default")`): a `<select>` —
  `private` (`permissions.default: "none"`) / `shared` (`"observer"`) — writing
  `/permissions/default` with the raw current value as `old`. This is the note's whole-document
  audience; the author's own `Owner` entry in `users` is untouched, so sharing never demotes the
  author.
- **Body:** `parseNoteBody(doc)` rendered through `SegmentList` with `channel =
  firstChannel(ctx.documents)` (the roll-button target; the body is rendered read-only when
  `null` — buttons disabled, as in `ItemSheet`); `null` from `parseNoteBody` renders
  `t("sheetNote.unrenderable")`.
- **Editor** (rendered when `ctx.canEdit(doc, enginePrefix + "/source")`): "Edit" opens a
  `<textarea>` seeded from the stored `source` and captures that value as the draft BASE; "Save"
  dispatches ONE `setField(ctx, docId, enginePrefix + "/source", base, draft)` and closes the
  editor; "Cancel" discards. The base — not the live stored value at save time — is the OCC
  pre-image on purpose (§11 M3): a concurrent edit makes the server refuse with `conflict`, the
  shell toast (§2.4) reports it, the draft is retained so the author can re-open (re-seeding from
  the new stored value is offered by a "Reload" action that discards the draft) and merge by hand.
  A remote change while the editor is open is shown as a banner (`sheetNote.changedRemotely`).
- **Sort:** a number input over `enginePrefix + "/sort"`.
- **Tree context:** a parent link ("Up to <parent name>") when `doc.parent_id` resolves in the
  store; a children list — `ctx.documents.query(NOTE_DOC_TYPE)` filtered `parent_id === docId`,
  ordered by (`engine.sort`, `created_at`) — each row `ctx.openDocument({ docId })`; a "New child
  note" control (when `ctx.canCreate(NOTE_DOC_TYPE)`) dispatching
  `buildNoteDoc(ctx.world, t("sheetNote.untitled"), "", { parentId: docId, owner: ctx.selfId })`
  as a Create and then opening it.

## 4. `@shadowcat/module-sheet-table` — `TableSheet.svelte`, `RowEditor.svelte`, `EntryEditor.svelte`

Registers `sheetContract(TABLE_DOC_TYPE)` at priority 0 (id `sheet-table:sheet`). Same prop/
derivation/bridge/OCC discipline as §3. `readOnly = !ctx.canEdit(doc, enginePrefix)`.

- **Header:** name input (`namePrefix`), close, **Draw**: count input (default 1) + button →
  `ctx.chat.drawTable({ tableId: docId, channel, count })` with `channel = firstChannel(...)`
  (disabled while `null`). The promise's rejection (the server's player-presentable
  `DrawTableError` text — over-cap count, empty table, cycle, missing asset, rate limit) goes to
  `ctx.notify(reason)`; the count input carries no client-side maximum — the server's
  `MAX_TOP_LEVEL_DRAWS` is the definition and its refusal is surfaced verbatim (§11 M12).
- **Description:** textarea over `enginePrefix + "/description"`, committed on `change`.
- **Draw rule:** `<select>` weighted / formula; formula reveals a notation input; a change writes
  the WHOLE `enginePrefix + "/draw"` object (`{ kind: "weighted" }` or `{ kind: "formula",
  notation }`) with the raw stored object as `old`.
- **Rows** (`RowEditor` per row): add row (appends `{ weight: 1, range: null, label: "",
  results: [] }`; under `formula`, `range: { lo: 1, hi: 1 }`), remove, move up/down; per row a
  label input, a weight number input (`min=1`), and — under `formula` only — `lo`/`hi` inputs;
  `EntryEditor` per result entry with a kind select (`text` / `doc` / `image` / `draw`) and
  per-kind fields: `text` → textarea; `doc` → label input + a document picker (a live
  `ctx.searchDocuments(q, { limit: 20 })` list, the composer's `@doc` pattern; picking sets
  `target: { kind: "doc", doc_id }` — token targets are not offered, §11 M5); `image` →
  `ctx.pickAsset({ kind: "image" })` + alt input; `draw` → a table picker
  (`ctx.searchDocuments(q, { limit: 20, docTypes: [TABLE_DOC_TYPE] })`) + count input. Every
  rows edit writes the WHOLE `enginePrefix + "/rows"` array with the raw stored array as `old`,
  committed on `change` (blur/select), never on `input` (§11 M4). Server-side `TableEngine::validate`
  refusals (weight 0, overlapping ranges, bad notation, caps) roll back and surface through the
  §2.4 toast; the sheet adds no second validator.

## 5. `@shadowcat/module-notes` — `NotesPanel.svelte`, `NoteTree.svelte`

`notes:panel` under `PANEL_CONTRACT`, `order: 3`, icon `📓`, `labelKey: "notes.tab"`,
launcher-closed, NOT `gmOnly` (players read shared notes and author private ones when granted).
Requires `PANEL_CONTRACT`; depends on `core-ui ^0.1.0`.

- **Tree:** every `note` in `ctx.documents` (the recipient's own redacted view — presence implies
  READ); roots are notes whose `parent_id` is `null` OR names a document absent from the store (a
  child whose parent this recipient cannot read is promoted to root, never hidden — a readable
  note is always reachable); siblings ordered by (`engine.sort`, `created_at`); expand/collapse per
  node; a row click opens the sheet (`ctx.openDocument({ docId })`).
- **Search:** a live search box → `ctx.searchDocuments(q, { limit: 20, docTypes: [NOTE_DOC_TYPE]
  })` (the `ActorsPanel` subscription pattern, cancel-guarded); a non-empty query replaces the tree
  with the flat hit list.
- **Create** (when `ctx.canCreate(NOTE_DOC_TYPE)`): name input + "New note" →
  `buildNoteDoc(ctx.world, name, "", { owner: ctx.selfId })` Create, then open. Private by
  default; the sheet's visibility control shares it.
- **Row actions:** Delete (rendered when GM, or the note's `owner === ctx.selfId`) →
  `{ op: "delete", doc }`; Move-to (GM only — `Operation::Move` is GM-only server-side) → a select
  of every other visible note plus "root" → `buildMoveOp(doc, parentId)`; the server's
  `check_note_parent`/`check_move_acyclic` refuse a bad target and the toast reports it.

## 6. `@shadowcat/module-tables` — `TablesPanel.svelte`

`tables:panel` under `PANEL_CONTRACT`, `order: 3`, icon `📋`, `labelKey: "tables.tab"`,
launcher-closed, NOT `gmOnly` (a player draws from a table they can read).

- **List:** every `table` in `ctx.documents`, ordered by name; live search with `docTypes:
  [TABLE_DOC_TYPE]` replacing the list while non-empty.
- **Row:** name → open sheet; **Draw** (count 1, `firstChannel`, rejection → `ctx.notify`);
  Delete (GM, or owner).
- **Create** (when `ctx.canCreate(TABLE_DOC_TYPE)`): name → `buildTableDoc(ctx.world, name,
  { draw: { kind: "weighted" }, rows: [], description: "" })` Create, then open — a table with no
  rows is valid at ingress (`TableEngine::validate` bounds row count, it does not require one) and
  refuses to draw (`EmptyTable`), which the toast reports.

## 7. Emitter editors on the actor sheet

`ActorSheet` renders three `EmissionEditor`s bound to `engine.aura`/`engine.sound`/`engine.vfx`,
each committing through the sheet's existing `setEngine(field, next)` (whole object; `null`
clears — the same wholesale-override shape `TokenOverrides` uses), `disabled={readOnly}` with no
GM gate (emissions follow standard write rules — M18's amended rule; `light` alone keeps its GM
gate because carried light is a vision input). Unit tests: each editor's commit dispatches an
Update of the right path with the raw stored object as `old`; clear writes `null`.

## 8. Shell wiring, i18n, docs, skills

- `App.svelte`'s module list gains `notes`, `tables`, `sheetNote`, `sheetTable`;
  `src/client/shell/package.json` gains the four workspace deps; `defaultModuleOrder.test.ts`'s
  pinned list gains the two panels (both launcher-closed, so `chat:panel` stays the first and only
  order-0 contribution).
- i18n keys in `src/client/ui-kit/src/locales/en.ts`: `notes.*`, `tables.*`, `sheetNote.*`,
  `sheetTable.*`, `intent.rejected.*`. Every `aria-label`/`placeholder`/`title` is `t(...)` text.
- Docs site: `docs/site/modules/{notes,tables,sheet-note,sheet-table}.md` (the sheet-item page
  shape: Purpose / Contributions / Components / Contracts & seams / Pointers), the index table and
  the sidebar (`docs/site/.vitepress/config.mts`); `docs/site/modules/actors.md` (emission editing
  on the sheet), `sheet-actor.md`, `sheet-item.md` (roll channel), `asset-browser.md` (no
  behaviour change; `buildMoveOp` now from core); `protocol.md` (`welcome` row); `docs/HISTORY.md`
  M20 entry; `docs/PLAN.md` M20 entry removed. `docs/TODO.md` unchanged (the `capabilities.ts`
  TODO was in code, not the backlog).
- Skills (plugin checkout, reviewed skill-update gate): `sheets` (the "Notes/tables sheets — not
  yet built" section becomes the description of the two shipped sheets; the write-site facts:
  draft-base OCC, whole-array rows), `tables-notes` (the UI consumers of `buildTableDoc`/
  `buildNoteDoc`/`parseNoteBody`/`drawTable`), `client-shell` (module list, `canCreate`,
  `role_capabilities`, the reject toast, `firstChannel`/`buildMoveOp` in core),
  `actors-tokens` (`EmissionEditor`'s new home; the sheet's emission editors),
  `documents-permissions` (`project_role_caps_for`), `realtime-sync` (the Welcome field). Hook
  map (`hooks/codebase-skill-reminder.py` `SUBSYSTEMS`): the `sheets` glob becomes
  `src/modules/sheet-(fallback|actor|item|note|table)/`; `tables-notes` gains
  `src/modules/notes/` and `src/modules/tables/`; both with absolute-path assertions in the hook's
  self-test.

## 9. Tests

Unit (Vitest, jsdom; every sheet/panel test runs against a REAL `OptimisticClient` for the
live-update cases, never the plain-store fixture alone):
- `NoteSheet.test.ts` — renders the parsed body through `SegmentList`; Edit → Save dispatches
  exactly one Update whose `old` is the draft BASE (assert with a store mutated between Edit and
  Save); Cancel dispatches nothing; visibility select writes `/permissions/default` with the raw
  old; children listed and ordered; "New child note" builds a private note owned by `selfId` with
  `parentId`; read-only for a non-writer; a remote edit re-renders (the frozen-at-mount trap);
  `null` body → unrenderable text.
- `TableSheet.test.ts`/`RowEditor.test.ts`/`EntryEditor.test.ts` — each edit writes the whole
  `rows` array with the raw old; draw-rule switch writes the whole `draw` object; range inputs only
  under formula; Draw calls `chat.drawTable` with `firstChannel` and the count; a rejected draw
  notifies with the reason; disabled Draw while no channel; entry kind switch clears the other
  kind's fields; doc/table pickers call `searchDocuments` with the right `docTypes`.
- `NotesPanel.test.ts`/`NoteTree.test.ts` — tree shape, hidden-parent root promotion, sibling
  ordering, search sends `docTypes: ["note"]` and replaces the tree, create builds the private note,
  Delete/Move-to gating and the `buildMoveOp` payload.
- `TablesPanel.test.ts` — list, search, create shape, Draw, gating.
- `ActorSheet.test.ts` — the three emission editors' commits and clears.
- ui-kit `EmissionEditor.test.ts` (moved, unchanged assertions).
- core `chat-docs.test.ts` (`firstChannel`), `move-op.test.ts` (`buildMoveOp`),
  `capabilities.test.ts` (`canCreateDoc`: GM, `all`, `by_type`, neither).
- shell `worldSession.test.ts` (reject → one notification per frame, rollback still runs;
  `canCreate` reads the Welcome projection), `defaultModuleOrder.test.ts`.
- server: `project_role_caps_for` (only the caller's role crosses; both `all` and `by_type`),
  the Welcome field's ts-rs shape.

Playwright (`src/client/shell/e2e`, dual-session under `DUAL_SESSION_TIMEOUT_MS`, no stage
gestures — panels and sheets only; test ids on every driven control):
- `notes.spec.ts` — the GM opens the notes panel from the launcher, creates "Session 1", the
  sheet opens; Edit, type markdown with `**bold**` and `[[roll:1d6|Luck]]`, Save; the body shows a
  `<strong>` and a roll button. The invited player (second context) opens their notes panel: empty.
  The GM sets visibility to shared; the player's panel now lists the note; opening it shows the
  body with NO textarea/Edit control; the player clicks the roll button and the chat panel shows a
  roll card. The GM creates a child note from the sheet; the player's tree shows it under its parent
  only after the GM shares it too.
- `tables.spec.ts` — the GM opens the tables panel, creates "Loot", the sheet opens; adds two rows
  with labels through the row editor; Draw; both the GM's and the player's chat panels show a
  `table_draw` card carrying one of the two labels; the GM's card exposes the roll tooltip trigger,
  the player's does not (no `spec`); the panel's quick Draw posts a second card.
Both specs are written by the implementer and RUN ONLY by the dispatcher (port 31999 is
dispatcher-serialized); a spec is not done until it has been observed to pass.

## 10. Build order

One branch (`m20-module-suite`), in this order — the first five need nothing from M21:
1. Core/ui-kit hoists: `firstChannel`, `buildMoveOp`, `EmissionEditor`; consumers switched.
2. Reject toast + `role_capabilities`/`canCreateDoc`/`canCreate`.
3. Actor sheet emission editors.
4. `sheet-note`, `sheet-table` packages (+ shell wiring, i18n, docs pages).
5. Merge-forward from `main` once M21 has landed (brings `docTypes`).
6. `notes`, `tables` panels.
7. Playwright specs, skills, HISTORY/PLAN.

## 11. Decision log

| # | Fork | Decision | Alternatives and why they lose |
|---|---|---|---|
| M1 | Package split | four packages: one panel per doc family + one sheet per doc_type | a single "journal" module couples two doc families a downstream system may want to replace separately; the actors/factions/conditions + sheet-actor/sheet-item precedent is one panel per family, one sheet per type |
| M2 | Note editing surface | markdown textarea + the server-derived body rendered through `SegmentList` | a WYSIWYG editor needs a second HTML renderer — a forked `{@html}` sink, the exact defect M19's C4 removed |
| M3 | OCC pre-image for a note save | the draft's BASE (captured at Edit) | the live stored value at save time would silently overwrite a concurrent edit; the base makes the server refuse and the toast explain |
| M4 | Rows write discipline | whole `/engine/rows` array on `change` | per-leaf writes cannot add/remove/reorder rows (`set_pointer` never resizes); per-keystroke writes churn OCC against a concurrent editor |
| M5 | Table entry targets | `doc` targets only | a table row naming a placed token is meaningless once the token moves scene or dies; `DocLinkTarget` keeps the `token` variant for chat |
| M6 | Rejection feedback | one shell-wide toast on every `reject` frame | per-sheet handling forks the decision across every sheet and misses every panel |
| M7 | Create affordance | `canCreate` mirror over a Welcome projection of the caller's role caps | gating on `role === "gm"` hides the control from a granted player — the audience private notes exist for; showing it unconditionally invites refusals |
| M8 | `EmissionEditor` location | `@shadowcat/ui-kit` | a sheet module cannot import `module-actors` (seam rule); `LightEmissionEditor` set the precedent |
| M9 | Roll/draw channel | `firstChannel` from core, nullable, buttons disabled on `null` | a `"general"` literal in any module forks the channel decision; the registry is server-seeded, so `null` is only the pre-resync window |
| M10 | M21 dependency | panels wait for the merge-forward; sheets do not | client-side type filtering reproduces the `ActorsPanel` under-fill defect the M21 filter removes |
| M11 | Panel visibility | neither panel `gmOnly` | players own private notes and draw from shared tables; the server gates every write anyway |
| M12 | Draw count cap on the client | none; the server's refusal is surfaced | a client constant mirroring `MAX_TOP_LEVEL_DRAWS` is a second statement of the cap with no fixture pinning it |
| M13 | Hidden-parent notes | promoted to root in the tree | hiding a readable note because its ancestor is private makes shared child notes unreachable |
| M14 | Note visibility control | `permissions.default` none/observer | a per-user share list is a permissions editor (a later, general surface); default is the audience switch the M19 design named |

## 12. Open questions for the user

None. Every fork above resolves under "best long-term shape in keeping with our plans and goals".
