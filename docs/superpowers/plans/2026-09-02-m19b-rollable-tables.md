# M19b · Rollable Tables — Implementation Plan

> **For agentic workers:** Execute task-by-task in order; each task's steps use checkbox
> (`- [ ]`) syntax. Written for a sonnet-class implementer with no conversation context —
> every path, symbol and test name below is exact; read the cited code before editing it.

**Goal:** `table` documents (weighted rows or a formula with ranges; rows yield text, doc links,
images, nested draws) drawn **on the server** via a `DrawTable` frame and posted to chat as
`Segment::TableDraw` roll embeds with GM-only `spec`/`raw`; the card renders draws (recursively)
and offers a Draw button on any doc link to a table.

**Architecture:** `data::engine::table` (typed engine band + `validate`), `tables::{draw,
handle_draw_table}` (new server module), `ClientMsg::DrawTable` → `ServerMsg::ChatError`
correlation, `chat::roll_property_overrides`, core `table-docs.ts` + `WsClient.drawTable`,
ui-kit `ChatApi.drawTable` + `SegmentList` `table_draw` renderer.

**Tech stack:** Rust, ts-rs (regenerated bindings committed), Svelte 5, Vitest.

**Spec:** `docs/superpowers/specs/2026-09-02-m19-tables-notes-chat-media-design.md` — §2.1, §2.3,
§3.1, §3.2, §3.5, §4, §5, §6, §7, §8, §11 (C2–C6, T1–T10). Read it first.

**Prerequisite:** M19a merged to `main` (`Segment::Image`, `chat::body`, `validate_audience`,
`validate_actor_owner`, ui-kit `SegmentList`).

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

…plus the reporting rule: a subagent delivers its report as the Agent tool result or writes it to
a named file; the prompt states which. Opus is banned for every dispatch in this campaign.

## Global constraints

Identical to `2026-09-02-m19a-chat-media.md`'s Global constraints (suppressions, file sizes,
sibling tests, comment rules, `trash`, explicit-path commits, the full gate list, build order).
Additionally: every ts-rs-exported struct change regenerates `src/types/generated/**` (run
`cargo test` from `src/server/`, commit the bindings in the same commit) and `src/types/index.ts`
gains the export lines.

---

### Task 1: `data::engine::table` — types + validation

**Files:**
- Create: `src/server/src/data/engine/table.rs`, `src/server/src/data/engine/table/tests.rs`
- Modify: `src/server/src/data/engine/mod.rs` (`pub mod table;`, re-exports, `TABLE_DOC_TYPE`,
  `is_engine_doc_type` + `normalize_engine` `"table"` arms with `validate`),
  `src/server/src/data/validation.rs` (`validate_containment`: a `table` has no `parent_id` and is
  never embedded — the `combat` arm shape), `src/server/src/data/validation/tests.rs`,
  `src/server/src/chat/rolls.rs` (`pub(crate) fn validate_table_formula(notation: &str) ->
  Result<(), RollError>`: `resolve_notation_template` with `NoHostResolver` (any reference ⇒
  `RollError::Reference`), `notation::parse` under `TABLE_PARSE_CONTEXT`, `validate_pre_roll`,
  then `matches!(spec.mode, Mode::SuccessCount(_))` ⇒ `RollError::TableNeedsTotal` (new variant +
  `Display` arm + no-debug-artifacts coverage); `pub(crate) const TABLE_PARSE_CONTEXT:
  ParseContext = ParseContext { mode: ModeKind::Total, direction: Direction::HighWins }` — confirm
  `ParseContext`'s fields are `pub` and `const`-constructible; if not, a `pub(crate) fn
  table_parse_context() -> ParseContext`), `src/types/index.ts` (exports).

**Interfaces:** exactly spec §2.1 (`TableEngine`, `DrawRule`, `TableRow`, `RowRange`,
`TableEntry`, the five `MAX_*` constants; `MAX_IMAGE_ALT_CHARS` imported from `chat`).
`TableEngine::validate(&self) -> Result<(), String>` implements every bullet of §2.1; the
`Weighted` sum bound reads `crate::chat::rolls::MAX_DIE_SIDES` (make it `pub(crate)` if it is
not) — never a copied literal.

- [ ] **Step 1:** failing tests in `table/tests.rs` — accept a valid weighted table; reject
  weight 0, a `range` under `Weighted`, sum > `MAX_DIE_SIDES` (mutation check: change the
  constant reference to a literal and confirm the parity test `weighted_sum_bound_is_the_chat_die_cap`
  fails), a missing `range` under `Formula`, `lo > hi`, overlapping ranges, a referencing formula
  (`1d20+str`), a SuccessCount formula (`5d10cs>=7`), > `MAX_TABLE_ROWS`, empty label, over-cap
  text/alt/description, `Draw.count` 0 or > `MAX_NESTED_DRAW_COUNT`; `normalize_engine("table")`
  round-trips; `validate_engine("item", Some(table_body))` still errors; containment tests.
- [ ] **Step 2:** implement; `cargo test -p shadowcat` PASS (bindings regenerate:
  `src/types/generated/engine/{TableEngine,DrawRule,TableRow,RowRange,TableEntry}.ts`); clippy; fmt.
- [ ] **Step 3:** `git commit -m "feat(engine): rollable table documents" -- src/server/src/data/ src/server/src/chat/rolls.rs src/types/`

### Task 2: `Segment::TableDraw` + `roll_property_overrides`

**Files:**
- Modify: `src/server/src/chat/mod.rs` (`Segment::TableDraw(TableDrawSegment)`,
  `pub struct TableDrawSegment`, `pub struct DrawnRow` per spec §2.3; `roll_embed_property_overrides`
  renamed `roll_property_overrides`, walking `RollEmbed` as today AND `TableDraw` recursively —
  a private `fn push_draw_overrides(prefix: &str, seg: &TableDrawSegment, out: &mut BTreeMap<…>)`
  emitting `{prefix}/spec`, `{prefix}/raw`, then `{prefix}/row/nested/{j}` per nested draw),
  `src/server/src/chat/tests.rs` (or split) — the existing overrides tests renamed.

- [ ] **Step 1:** failing test `table_draw_spec_and_raw_are_gm_only_at_every_depth` — a message
  whose content is one `TableDraw` with two nested draws yields exactly six `GmOnly` pointers;
  `filter_properties` for a Player strips all six and keeps `outcome`/`row.label`/`row.content`;
  a GM keeps them.
- [ ] **Step 2:** implement. `cargo test -p shadowcat chat` PASS.
- [ ] **Step 3:** `git commit -m "feat(chat): table-draw segment with GM-only roll state" -- src/server/src/chat/`

### Task 3: `tables` module — draw resolution

**Files:**
- Create: `src/server/src/tables/mod.rs`, `src/server/src/tables/draw.rs`,
  `src/server/src/tables/tests.rs` (+ `tables/draw/tests.rs` if the file grows)
- Modify: `src/server/src/lib.rs` or `main.rs` module tree (`pub mod tables;` beside `chat`/`combat`).

**Interfaces (spec §3.2):**
- `pub(crate) const MAX_TOP_LEVEL_DRAWS: u32 = 10; MAX_DRAWS_PER_REQUEST: usize = 64;
  MAX_DRAW_DEPTH: usize = 8;`
- `pub enum DrawTableError { RateLimited, UnknownChannel, UnknownRecipient, ActorNotSpeakable,
  Forbidden, NotFound, TooMany, TooDeep, Cycle, EmptyTable, MissingAsset, Roll(RollError),
  Data(DataError) }` + `impl From<SendMessageError>` + `[sec]`-classified `Display`
  (`Forbidden`/`NotFound`/`Data` ⇒ the same generic sentence `SendMessageError` uses; the rest
  player-presentable).
- `pub struct DrawTableRequestCtx<'a> { room: &'a Room, repo: &'a dyn Repository, ctx:
  &'a PermissionContext, rate: &'a PingRateLimiter, world_defaults: &'a WorldCapDefaults, now:
  i64, budget_per_min: usize }` — how `ws::conn` obtains `world_defaults` for
  `combat::handle_combat_intent` is the model.
- `pub async fn handle_draw_table(req: DrawTableRequestCtx<'_>, table_id: Uuid, channel: String,
  count: u32, actor_owner: Option<ActorOwnerRef>, audience: Audience) -> Result<Command,
  DrawTableError>` — the seven steps of §3.2 in order.
- `draw::DrawCtx<'a> { repo, ctx, world_defaults, policy: &ChatContentPolicy, world_id, chain:
  Vec<Uuid>, budget: usize }` and `pub(crate) async fn draw_table(cx: &mut DrawCtx<'_>, table_id:
  Uuid, depth: usize) -> Result<TableDrawSegment, DrawTableError>`; a `#[cfg(test)]`
  `draw_table_with_seed(.., seed: u64)` seam over `execute_roll_with_seed` so tests are
  deterministic (production `draw_table` uses `execute_roll`).
- Row selection helpers are pure and unit-tested on their own: `fn weighted_row(rows, total:
  i64) -> Option<usize>` (first row with cumulative weight ≥ total) and `fn ranged_row(rows,
  total) -> Option<usize>`.

- [ ] **Step 1:** failing tests (build tables through the `data::sqlite` test harness — `doc(perms,
  system)` + an `engine` body — with a real `create_user` owner): weighted selection at cumulative
  boundaries under a fixed seed; formula hit and miss (`row: None`); nested draw `count` fan-out
  and `row.nested` order; `Cycle` on self-reference and on A→B→A; `TooDeep` at depth 9;
  `TooMany` past 64 total; a nested table with `default: none` ⇒ `Forbidden` for a Player, ok for
  the GM; a `Draw` naming a non-table doc ⇒ `NotFound` generic; `MissingAsset`; `Text` rows
  sanitize under the world policy (plain-text world ⇒ `Text`, markdown world ⇒ `Html`); the
  posted message is `kind: Roll` with `audience` mapped by `build_message_doc`
  (Public/Whisper/GmOnly all three); rate-limit refusal; unknown channel; `Display`
  no-debug-artifacts over every variant; `handle_recalc_roll` on a draw's `roll_id` ⇒
  `RollNotFound`.
- [ ] **Step 2:** implement. `cargo test -p shadowcat tables chat` PASS; clippy (incl. the
  missing-docs pair); fmt.
- [ ] **Step 3:** `git commit -m "feat(tables): server-side table draws posted to chat" -- src/server/src/tables/ src/server/src/lib.rs`

### Task 4: wire — `ClientMsg::DrawTable` + `ws::conn` arm

**Files:**
- Modify: `src/server/src/ws/protocol.rs` (`DrawTable { request_id, table_id, channel,
  #[serde(default = "default_draw_count")] count: u32, #[serde(default)] actor_owner,
  #[serde(default)] audience }` with a doc comment naming `handle_draw_table` and the
  `ChatError` correlation), `src/server/src/ws/conn.rs` (the arm beside `RecalcRoll`: on `Err`
  ⇒ `ServerMsg::ChatError { request_id, message: e.to_string() }` to the sender only;
  `tracing::debug!` log), `src/server/src/ws/conn/tests/*.rs` (the chat-frame tests' home),
  `src/types/generated/ClientMsg.ts` (regenerated), `src/client/core/src/wire.ts` if the Zod
  mirror of `ClientMsg` exists there (check `rg "recalc_roll" src/client/core/src/wire.ts`), the
  wire drift-guard test.

- [ ] **Step 1:** failing tests — a `draw_table` frame from a Player on a readable table produces
  a broadcast `Event` creating a `message` doc whose content is `table_draw`; a refused draw emits
  `chat_error` with the correlated `request_id` to the sender only; a malformed frame is
  `BadMessage`.
- [ ] **Step 2:** implement; regenerate bindings; `cargo test -p shadowcat` PASS; `pnpm -r test`
  (drift guard) PASS.
- [ ] **Step 3:** `git commit -m "feat(ws): draw_table frame" -- src/server/src/ws/ src/types/ src/client/core/src/wire.ts`

### Task 5: client core — `table-docs.ts`, segment mirror, `drawTable`

**Files:**
- Create: `src/client/core/src/table-docs.ts`, `src/client/core/src/table-docs.test.ts`
- Modify: `src/client/core/src/chat-docs.ts` (`table_draw` arm: `TableDrawSegment` type +
  `tableDrawSegmentSchemaImpl` via `z.lazy` for `row.content` (`ChatSegmentSchema` array) and
  `row.nested`; `spec`/`raw` optional `.passthrough()` like `roll_embed`; `isKnownSegment` +
  fallback refusal), `chat-docs.test.ts`, `src/client/core/src/ws-client.ts` (`DrawTableOptions
  { tableId, channel, count?, actorOwner?, audience? }`; `drawTable(opts): Promise<void>` on the
  `chatPending` map exactly like `recalcRoll`), `ws-client.test.ts`, `src/client/core/src/index.ts`.

**`table-docs.ts`:** `TABLE_DOC_TYPE = "table"`; `buildTableDoc(worldId, name, engine:
TableEngine, id?)` → `WireDocument` via the `envelope` helper `buildActorDoc` uses (`system: {}`,
`permissions.default: "observer"`); re-export the ts-rs types from `@shadowcat/types`.

- [ ] **Step 1:** failing tests — builder shape; `table_draw` parses with two nesting levels and
  refuses a malformed nested segment (whole message ⇒ null); `drawTable` resolves after the
  window and rejects on a correlated `chat_error`.
- [ ] **Step 2:** implement. `pnpm -r test`, `pnpm -r typecheck`, `pnpm lint` PASS.
- [ ] **Step 3:** `git commit -m "feat(core): table documents, table-draw segments, drawTable" -- src/client/core/`

### Task 6: ui-kit + card rendering

**Files:**
- Modify: `src/client/ui-kit/src/appContext.ts` (`ChatApi.drawTable(opts: DrawTableOptions):
  Promise<void>` with the same doc shape as `recalc`), the shell wiring
  (`src/client/shell/src/**/Table.svelte` or wherever `ChatApi` is constructed — `rg "recalc:"
  src/client/shell/src`), `src/client/ui-kit/src/SegmentList.svelte` + `SegmentList.test.ts`
  (`table_draw` renderer per spec §4: name link presence-gated via `ctx.documents.get(table_id)`,
  roll chip + `RollTooltip`, row label, `row.content` through `SegmentList`, `row.nested`
  recursive with indentation, "no matching row" for `row: null`; `doc_link` whose target doc has
  `doc_type === "table"` renders a `Draw` button (`data-testid="table-draw-button"`) calling
  `ctx.chat.drawTable({ tableId, channel })` and surfacing a rejection inline like the roll
  button does), i18n keys (`chat.table.draw`, `chat.table.noRow`, `chat.table.nested`).
- Modify: `docs/site/modules/chat-card.md` (segment kinds gain `table_draw`; Draw affordance).

- [ ] **Step 1:** failing tests — renders a nested draw; the Draw button appears only when the
  doc link resolves to a table in the store; clicking calls `drawTable` with the message's channel.
- [ ] **Step 2:** implement. `pnpm -r test`, `pnpm -r typecheck`, `pnpm lint` PASS.
- [ ] **Step 3:** `git commit -m "feat(ui-kit,chat-card): render table draws; draw from a table link" -- src/client/ src/modules/chat-card/ docs/site/modules/chat-card.md`

### Task 7: e2e

**Files:**
- Create: `src/client/core/src/e2e/table-draw.e2e.test.ts` (spec §8: GM `Intent` creates
  table B (weighted, two rows) and table A (weighted, one row whose `results` holds `{ kind:
  "draw", table_id: B, count: 2 }`) via `buildTableDoc`; GM `drawTable({ tableId: A, channel:
  "general" })`; GM and player watchers both receive a `message` whose `content[0].kind ===
  "table_draw"` with `row.nested.length === 2`; the player's copy has no `spec`/`raw` at either
  depth, the GM's has both; a Player `drawTable` on a table created with `permissions.default:
  "none"` rejects with the generic wording; a table whose row draws itself rejects with the
  `Cycle` wording; a `Formula` table (`2d6`, ranges 2–6 / 7–12) yields an `outcome.total` inside
  the drawn row's range).

- [ ] **Step 1:** write; `pnpm --filter @shadowcat/core test:e2e` PASS (alone; 31999 free).
- [ ] **Step 2:** `git commit -m "test(e2e): nested table draws with per-recipient roll-state redaction" -- src/client/core/src/e2e/`

### Task 8: docs, skills, gates, review

- [ ] `docs/site/protocol.md`: `draw_table` row in the client→server catalog; a "Rollable
  tables" paragraph after the dice-reference paragraph (server-side draw, fixed Total context,
  READ on every table in a chain, `spec`/`raw` GM-only, not recalc-able).
- [ ] `docs/design/ARCHITECTURE.md` invariant 6 + §6: add `table` to the engine doc-type list and
  correct the count from `is_engine_doc_type`'s arms.
- [ ] Skills: **create** `~/.claude/skills/shadowcat-codebase/skills/shadowcat-codebase-tables-notes/SKILL.md`
  (Purpose / Key files & seams / Hard invariants / Gotchas / Pointers — tables half now; §11
  T1–T10 as invariants/gotchas), add it to core's Subsystem skills list and to
  `hooks/codebase-skill-reminder.py`'s `SUBSYSTEMS` map (`src/server/src/tables/`,
  `src/server/src/data/engine/table`, `src/client/core/src/table-docs`) with an absolute-path
  assertion in the hook self-test; amend `shadowcat-codebase-chat` (`TableDraw`,
  `roll_property_overrides`, `DrawTable`), `shadowcat-codebase-dice` (`validate_table_formula`,
  `TABLE_PARSE_CONTEXT`), `shadowcat-codebase-documents-permissions` (registry entry, containment),
  `shadowcat-codebase-client-shell` (`ChatApi.drawTable`). Symbol/api ref gates + `pnpm run
  test:scripts`; spec-reviewer on the skill diff; commit + push the plugin repo (explicit paths
  only).
- [ ] `docs/HISTORY.md` M19b entry; `docs/PLAN.md` marks M19b done.
- [ ] Full gate run; `git diff --exit-code src/types/generated` clean.
- [ ] Final two-reviewer branch review (`shadowcat-codebase:shadowcat-spec-reviewer` +
  `shadowcat-codebase:shadowcat-code-reviewer`, pre-generated diff). Address findings. Report to
  the dispatcher; the dispatcher merges.
