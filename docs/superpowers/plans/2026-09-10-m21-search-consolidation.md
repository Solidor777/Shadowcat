# M21 · Search consolidation — Implementation Plan

> **For agentic workers:** Execute task-by-task in order; each task's steps use checkbox
> (`- [ ]`) syntax. Written for a sonnet-class implementer with no conversation context —
> every path, symbol and test name below is exact; read the cited code before editing it.

**Goal:** one FTS5 backend for every searchable thing. Documents index the text a reader sees
(an engine-aware projection replaces the engine-band leaf sweep), the `search` frame and
`Repository::search` accept a server-side `doc_types` filter, and assets join FTS through a
trigger-maintained `assets_fts` table queried by a `q` parameter that replaces the LIKE substring.

**Architecture:** `data::engine::search_text` (exhaustive per-type projection) +
`chat::segments_search_text` (the one segment-list text extraction) feed `data::search::
index_content`; both FTS document tables gain `doc_type UNINDEXED`; `assets_fts` is maintained by
SQL triggers on `assets` and `asset_tags`; `build_match` is the single MATCH sanitizer for
documents AND assets.

**Tech stack:** Rust (sqlx/SQLite FTS5, ts-rs regenerated), TypeScript (Zod mirror, Svelte 5
module), Vitest, the Node↔Rust e2e harness.

**Spec:** `docs/superpowers/specs/2026-09-10-m21-search-consolidation-design.md` — read it first;
§2 (index content), §3 (frame/repository/route), §5 (tests), §10 (decisions D1–D10).

**Worktree:** `C:/Dev/Shadowcat-m21`, branch `m21-search`, cut from `main` `826b9b15`.
`pnpm install` and `pnpm build` have been run there.

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

- No lint suppressions (`#[allow]`, `#[expect]`, `eslint-disable`, `@ts-ignore`); no file-size
  allowlist entries (split instead); Rust test bodies in sibling files (`pnpm lint:inline-tests`).
- Comments cite symbols, never files/lines; no milestone ids, spec pointers, dates, or history
  narration in code comments, `assert!` messages or test names (`pnpm lint:comments`).
- `pnpm build` before any cargo command (rust-embed validates `dist/`). Never two cargo commands
  concurrently in one worktree. Commands that may exceed five minutes (`cargo test --all`,
  `pnpm -r test`, `pnpm build:all`) run in the background with output to a log you poll.
- Deletions via `trash`, never `rm`/`Remove-Item`/`git rm`; commits always `git add <paths>` +
  `git commit -m "…" -- <paths>` (the `-m` before `--`); commit trailer per the campaign brief.
- Full gate list (run before claiming green, paste the real result lines in the report):
  `pnpm build`, `pnpm -r typecheck`, `pnpm -r test`, `pnpm lint`, `pnpm lint:docs`,
  `pnpm lint:props`, `pnpm lint:comments`, `pnpm lint:allowances`, `pnpm lint:file-size`,
  `pnpm lint:inline-tests`, `pnpm lint:aria-labels`, `pnpm lint:gate-manifest`,
  `pnpm lint:settings-privacy`, `pnpm docs:check-examples`, `pnpm run test:scripts`,
  `pnpm run check:svelte-runtime`, `pnpm --filter "shadowcat-example-*" build`; from
  `src/server/`: `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo clippy --all-targets -- -D missing-docs -D clippy::missing-docs-in-private-items`,
  `cargo test --all`, then `git diff --exit-code src/types/generated`; e2e:
  `pnpm --filter @shadowcat/core test:e2e` (background + log). The browser suite
  (`pnpm --filter @shadowcat/shell e2e`) is DISPATCHER-ONLY: never run it, never wait on port
  31999; report it NOT RUN.
- `src/server/src/data/sqlite.rs` is 2,276 lines and `src/server/src/ws/conn.rs` is over 2,100;
  neither task below adds more than a few dozen lines to either. `src/server/src/chat/mod.rs` and
  `src/server/src/chat/tests.rs` are large — the new segment projection and its tests go in a NEW
  sibling module (`chat/search_text.rs` + `chat/search_text/tests.rs`), not into `mod.rs`.
- Every public item needs a doc comment with a runnable ` ```rust ` example
  (`pnpm docs:check-rust-examples` is a push gate; `-D missing-docs` is a CI gate).
- A developer SQLite file predating this branch's `0001_init.sql` edit fails the sqlx checksum:
  delete the dev DB file (via `trash`) and restart — never add a migration file.

---

### Task 1: `chat::segments_search_text` — the one segment-list text projection

**Files:**
- Create: `src/server/src/chat/search_text.rs`, `src/server/src/chat/search_text/tests.rs`
- Modify: `src/server/src/chat/mod.rs` (`mod search_text;` + `pub use search_text::segments_search_text;`)

**Interface:** `pub fn segments_search_text(segments: &[Segment]) -> String` — space-joined
reader-facing text per spec §2.3. Read `Segment`'s definition in `chat/mod.rs` (variants `Text`,
`Html`, `RollEmbed`, `RollButton`, `LinkPreview`, `OEmbed(OEmbedSegment)`, `DocLink`, `Image`,
`TableDraw(TableDrawSegment)`) and `TableDrawSegment`/`DrawnRow`/`OEmbedSegment` for exact field
names. Rules: `Html` → `strip_tags_and_decode(&sanitized_html)` (remove every `<…>` run —
ammonia has escaped every literal `<` in text, state that on the helper's doc; then decode
`&amp;`/`&lt;`/`&gt;`/`&quot;`/`&#39;` and numeric `&#NNN;`/`&#xHH;`); `RollEmbed` → `formula`
ONLY (the struct has NO `label` field), never `outcome`/`roll_id`/`spec`/`raw`/`recalc_history`; `RollButton` → `label` + `formula`;
`LinkPreview` → its title/description/url text fields; `OEmbed` → title and author/provider
name fields, never ids; `DocLink` → `label`; `Image` → `alt`; `TableDraw` → `table_name` +
(when `row` is `Some`) the row's `label`, `segments_search_text(&row.content)` and every
`nested` draw recursively, never `formula`/`spec`/`raw`/`roll_id`. A private
`fn push_text(out: &mut String, s: &str)` appends with a single separating space so the output
never carries doubled spaces or a leading space.

- [ ] **Step 1:** failing tests in `chat/search_text/tests.rs` — one test per variant asserting
  the contributed words are present AND the excluded ones absent (`"roll_embed"`, a `spec` die
  size that appears nowhere else, an asset UUID, `"strong"` for an `Html` segment containing
  `<strong>bold</strong>`); an entity test (`&amp;` → `&`, `&#39;` → `'`, `&#x27;` → `'`); a
  nested `TableDraw` test (a grandchild draw's row label is present; its `formula` is not); an
  empty-list test (`""`).
- [ ] **Step 2:** implement. `cargo test -p shadowcat chat::search_text` PASS; clippy; fmt.
- [ ] **Step 3:** `git commit -m "feat(chat): reader-facing text projection over a segment list" -- src/server/src/chat/`

### Task 2: `data::engine::search_text` + `index_content` projection

**Files:**
- Modify: `src/server/src/data/engine/mod.rs` (`pub fn search_text(doc_type: &str, engine:
  &serde_json::Value) -> Option<String>`), `src/server/src/data/engine/tests.rs` (or the module's
  existing sibling test file — locate with `rg "mod tests" src/server/src/data/engine/mod.rs`),
  `src/server/src/data/search.rs` (`index_content`), `src/server/src/data/search/tests.rs`.

**Interface (spec §2.2):** an explicit `match doc_type` arm for EVERY name `is_engine_doc_type`
accepts (read that function; today 26 names) — `Some(text)` for each, `None` in the `_` arm
(non-engine type). Text-bearing arms: `"actor"` → `ActorEngine.display_name` (the Rust field —
`displayName` is only its serde wire name); `"note"` →
`chat::segments_search_text(&NoteEngine.body)`; `"table"` → `description` then per row `label`,
`TableEntry::Text.text`, `TableEntry::Doc.label`, `TableEntry::Image.alt`; `"message"` →
`chat::segments_search_text(&MessageEngine.content)`. Each deserializes its typed struct with
`serde_json::from_value` and yields `Some(String::new())` on failure (write-path posture: never
fail the index write; doc-comment it). Every other registered type → `Some(String::new())` — but
FIRST enumerate every `String` field across all engine structs (`rg "pub .*: String" src/server/src/data/engine/`)
and classify each as display text vs id/kind/notation/url in your report; any display-text field
found (a drawing label, a region caption) joins its type's arm and is named in the report.

`index_content(doc)` becomes: `name` (when present) + `search_text(&doc.doc_type, engine)`
(when the engine band is present and the projection is `Some`) + `collect_leaves(system)`.
`doc_type` is no longer pushed (spec D1). `index_content_public` is unchanged. Update both
functions' doc comments and doc examples (they must keep passing `cargo test --doc`).

- [ ] **Step 1:** failing tests — in `data/engine/tests` (or a new sibling): the exhaustiveness
  pin (iterate a list of every registered doc_type name — build it from the same names
  `is_engine_doc_type` matches; if no such list exists as data, add
  `pub(crate) const ENGINE_DOC_TYPES: &[&str]` in `data::engine` and make `is_engine_doc_type`
  read it, so the list and the predicate cannot drift — and assert `search_text(name,
  &json!({}))` is `Some` for each and `search_text("item", &json!({}))` is `None`); per-arm tests
  building a real `NoteEngine`/`TableEngine`/`MessageEngine`/`ActorEngine` value. In
  `data/search/tests.rs`: rewrite `extracts_string_and_number_leaves_and_doc_type` to assert
  `doc_type` is ABSENT; add `note_indexes_rendered_body_not_source_or_markup`, `table_indexes_labels_text_alt_not_kinds_or_ids`,
  `message_indexes_content_not_kinds`, `actor_indexes_display_name_not_visual_kind`, and keep the
  two GM-only partition tests green (a `name`/engine-leaf override still hides from the public
  index — for the engine case use a `note` whose `body` text is under a `/engine/body` GM-only
  override, or an actor `displayName` override).
- [ ] **Step 2:** implement; `cargo test -p shadowcat data` PASS; clippy; fmt.
- [ ] **Step 3:** `git commit -m "feat(search): engine-aware index projection replaces the engine leaf sweep" -- src/server/src/data/`

### Task 3: `doc_type` column + `doc_types` filter on `Repository::search`

**Files:**
- Modify: `src/server/migrations/0001_init.sql` (both FTS document tables gain `doc_type
  UNINDEXED` after `world_id`), `src/server/src/data/sqlite/documents.rs`
  (`reindex_document_fts` binds `doc.doc_type` into both inserts), `src/server/src/data/search.rs`
  (`pub const MAX_SEARCH_DOC_TYPES: usize = 16;`), `src/server/src/data/repository.rs`
  (`search(..., doc_types: &[String])` + its doc example), `src/server/src/data/sqlite.rs`
  (`search` impl), `src/server/src/data/sqlite/tests/search_and_worlds.rs`, every other `search(`
  call site (`rg "\.search\(" src/server/src`) passing `&[]`.

**Behaviour (spec §3.1):** `doc_types.len() > MAX_SEARCH_DOC_TYPES` ⇒
`Err(DataError::OpFailed("too many doc types".into()))` before any SQL. Non-empty ⇒ the ranked
SQL gains ` AND doc_type IN (?, ?, …)` built with one `?` per entry — NEVER interpolated values —
inside the partition table's query so `MAX_SCAN` counts only candidates of the requested types.
SQLite numbering subtlety: the existing statement is a `&'static str` with explicit `?1..?4`, and
a bare `?` after them continues from the highest number used so far (no precedent in this crate
mixes the two). Do not mix: move the whole statement to a `sqlx::QueryBuilder` that `push_bind`s
EVERY value (the four existing ones included) in textual order, so bind order is the push order
and no numbered placeholder remains; the `search_filters_by_doc_types` test is the guard that
the binds line up.

- [ ] **Step 1:** failing tests in `search_and_worlds.rs` — `search_filters_by_doc_types` (a GM
  searches "dragon" over an actor + a note + a table all named "…dragon…"; `["note"]` returns
  only the note; `["note","table"]` returns both; `[]` returns all three);
  `search_doc_types_over_cap_is_refused` (17 entries ⇒ `OpFailed`);
  `search_doc_types_composes_with_visibility` (a player's `["note"]` search never returns a
  `default: none` note); `search_paginates_under_a_doc_types_filter` (the cursor resumes without
  skipping under `["actor"]` — model on `search_paginates_without_underfill`).
- [ ] **Step 2:** implement; delete the dev DB if one exists (`trash` it); `cargo test -p shadowcat
  data::sqlite` PASS; clippy; fmt.
- [ ] **Step 3:** `git commit -m "feat(search): doc_types filter on Repository::search" -- src/server/migrations/ src/server/src/data/`

### Task 4: the `search` frame + subscriptions + ts-rs

**Files:**
- Modify: `src/server/src/ws/protocol.rs` (`ClientMsg::Search` gains `#[serde(default)] doc_types:
  Vec<String>` with a doc comment naming the cap), `src/server/src/ws/conn.rs` (`Sub` gains
  `doc_types: Vec<String>`; the reader-task `Search` arm passes `&doc_types` to `repo.search` and
  forwards it in `Egress::Subscribe`; the egress `Subscribe` arm stores it and passes it; the
  re-evaluation loop passes `&sub.doc_types`; an `OpFailed` from the cap surfaces as the existing
  `SearchError { message: "search failed" }`? — NO: map the cap refusal to `SearchError { message:
  "too many doc types" }` explicitly (match on the error's text is brittle — instead check
  `doc_types.len() > MAX_SEARCH_DOC_TYPES` in `conn.rs` BEFORE calling the repository, in both the
  one-shot and subscribe arms, reading the SAME constant, and send that message; the repository's
  own refusal stays as defense in depth), `src/server/src/ws/protocol/tests.rs` (or wherever
  `ClientMsg` parse tests live — `rg "search" src/server/src/ws/*tests*`), `src/types/generated/`
  (regenerated by `cargo test`), `src/client/core/src/wire.ts` (the `search` member of the
  `ClientMsg` union + its Zod object gain `doc_types: string[]`), `docs/site/protocol.md` (`search`
  row: "…`doc_types` narrows to the listed types (≤16)…").
- Add `Egress::Subscribe { …, doc_types }` to the enum.

- [ ] **Step 1:** failing tests — protocol: `search` parses without `doc_types` (defaults to
  empty) and with it; conn (an existing subscription integration test file — locate with
  `rg "SearchUpdate" src/server/src/ws -l`): a subscription with `doc_types: ["note"]` pushes an
  update when a note is created and NOT when an actor matching the query is created; a 17-entry
  list gets `SearchError { message: "too many doc types" }`.
- [ ] **Step 2:** implement; `cargo test -p shadowcat ws` PASS; `cargo test --all` (background,
  log) regenerates bindings; `git diff --exit-code src/types/generated` FAILS (expected) — stage
  the regenerated files; `pnpm -r typecheck` PASS after the Zod edit.
- [ ] **Step 3:** `git commit -m "feat(ws): doc_types on the search frame and live subscriptions" -- src/server/src/ws/ src/types/generated/ src/client/core/src/wire.ts docs/site/protocol.md`

### Task 5: client `docTypes` + `ActorsPanel`

**Files:**
- Modify: `src/client/core/src/ws-client.ts` (`WsSearchOptions.docTypes?: string[]`,
  `WsSubscribeSearchOptions.docTypes?: string[]`; `search`/`subscribeSearch` send `doc_types:
  opts.docTypes ?? []`), `src/client/core/src/ws-client.test.ts`, `src/client/ui-kit/src/appContext.ts`
  (`searchDocuments` opts gain `docTypes?: string[]` with a doc comment), `src/client/shell/src/lib/worldSession.svelte.ts`
  (`searchDocuments` opts type + pass-through), `src/modules/actors/src/ActorsPanel.svelte`
  (`searchDocuments(q, { limit: 20, docTypes: [ACTOR_DOC_TYPE] }, …)` — import the constant from
  `@shadowcat/core` (locate it: `rg "ACTOR_DOC_TYPE" src/client/core/src`); delete the
  `.filter((h) => h.document.doc_type === "actor")`), `src/modules/actors/src/ActorsPanel.test.ts`
  (assert the options carry `docTypes: ["actor"]`; delete/adjust any test that fed a non-actor hit
  expecting it filtered), `docs/site/modules/actors.md` ("live FTS, filtered to actors
  server-side").

- [ ] **Step 1:** failing tests — `ws-client.test.ts`: `search` and `subscribeSearch` emit
  `doc_types` (`[]` by default, the given list when set); `ActorsPanel.test.ts`: the
  subscription call receives `docTypes: ["actor"]`.
- [ ] **Step 2:** implement; `pnpm -r typecheck`, `pnpm --filter @shadowcat/core test`,
  `pnpm --filter @shadowcat/module-actors test`, `pnpm --filter @shadowcat/shell test` PASS;
  `pnpm lint`, `pnpm lint:docs`, `pnpm lint:props` PASS.
- [ ] **Step 3:** `git commit -m "feat(client): docTypes on searchDocuments; the actors panel filters server-side" -- src/client/ src/modules/actors/ docs/site/modules/actors.md`

### Task 6: `assets_fts` + triggers + `q` on the repository

**Files:**
- Modify: `src/server/migrations/0001_init.sql` (the `assets_fts` virtual table + the five
  triggers from spec §2.4, placed after the `asset_tags` index; a comment names the refresh shape
  and the cost bound), `src/server/src/data/asset/query.rs` (`AssetFilter.name` →
  `AssetFilter.query: Option<String>` — "Full-text query over `original_name` and every tag;
  sanitized by `data::search::build_match`"), `src/server/src/data/sqlite/assets.rs`
  (`query_assets`: delete the LIKE branch; add `if let Some(q) = &filter.query { match
  build_match(q) { None => return Ok(Vec::new()), Some(expr) => { qb.push(" AND a.id IN (SELECT
  asset_id FROM assets_fts WHERE assets_fts MATCH "); qb.push_bind(expr); qb.push(")"); } } }`),
  `src/server/src/data/sqlite/export_import.rs` (the "search state is rebuilt from `doc`'s
  content, never carried across servers" sentence on `insert_imported_document`'s doc comment —
  extend it to name `assets_fts`; it is NOT in `world_bundle.rs`) and
  `src/server/src/data/world_bundle.rs` (its module doc, which that comment points at, states
  nothing about search today — add the same one-sentence statement so the pointer resolves), the sqlite asset tests file (`rg "query_assets" src/server/src/data/sqlite -l`
  for the sibling test file).

- [ ] **Step 1:** failing tests — `query_assets_full_text_matches_name_word`, `…_explicit_tag`,
  `…_derived_tag` (seed via `insert_asset` + `set_asset_tags`), `…_refreshes_on_rename`
  (`update_asset_placement` with a new name), `…_stops_matching_after_tag_removal`,
  `…_composes_with_folder_kind_tags_and_regex`, `assets_fts_row_removed_on_asset_delete`,
  `assets_fts_rows_removed_on_world_delete` (raw `SELECT count(*) FROM assets_fts WHERE world_id
  = ?`), `query_assets_empty_or_punctuation_query_is_empty_page`, and a bundle-import test
  asserting an imported tagged asset is findable by `query` (extend the existing import test
  file — `rg "import_world" src/server/src/data/sqlite/tests -l`).
- [ ] **Step 2:** implement; `cargo test -p shadowcat data::sqlite` PASS; clippy; fmt.
- [ ] **Step 3:** `git commit -m "feat(assets): trigger-maintained assets_fts and a full-text query filter" -- src/server/migrations/ src/server/src/data/`

### Task 7: the asset query route + client filter

**Files:**
- Modify: `src/server/src/http/assets/query.rs` (`AssetQuery.name` → `q: Option<String>`
  ("Full-text query over the display name and every tag"); `is_bare` and `parse` follow;
  route doc), `src/server/src/http/assets/query/tests.rs` + `src/server/src/http/assets/tests.rs`
  (every `name:` use → `q:`; add a route test that `?q=` reaches `AssetFilter.query`),
  `src/client/core/src/asset-rest.ts` (`AssetQuery.name` → `q?: string` — "Full-text query over
  name and tags (sanitized server-side)"; `queryAssets` sends `q`), `src/client/core/src/asset-rest.test.ts`,
  `src/modules/asset-browser/src/filterState.ts` (`name` → `query`, `nameIsRegex` →
  `queryIsRegex`, docs updated), `src/modules/asset-browser/src/FilterBar.svelte`
  (`data-testid="filter-query"`, placeholders `t("assetBrowser.filterQuery")` / the existing regex
  key; the regex toggle reads `queryIsRegex`), `src/modules/asset-browser/src/AssetBrowser.svelte`
  (initial state + the `queryAssets` mapping: `q: filter.queryIsRegex ? undefined : filter.query ||
  undefined`, `nameRegex: …`), their tests, `src/client/ui-kit/src/locales/en.ts`
  (`assetBrowser.filterName` → `assetBrowser.filterQuery: "Search name and tags"`; keep
  `filterRegex`), `src/client/shell/e2e/asset-browser.spec.ts` (every `filter-name`/placeholder
  reference — `rg "filter-name|Filter by name|filterName" src/client/shell/e2e`), `docs/site/modules/asset-browser.md`
  ("filter bar (search over name and tags / regex / tags / kind / sort)").

- [ ] **Step 1:** failing tests — route test for `q`; `asset-rest.test.ts` expects `q=` in the URL
  and no `name=`; `FilterBar.test.ts` types into `filter-query` and expects `onChange` with
  `query`; `AssetBrowser.test.ts` expects `queryAssets` called with `q`.
- [ ] **Step 2:** implement; `cargo test -p shadowcat http::assets` PASS; `pnpm -r typecheck`,
  `pnpm --filter @shadowcat/module-asset-browser test`, `pnpm --filter @shadowcat/core test`
  PASS; `pnpm lint:aria-labels` PASS.
- [ ] **Step 3:** `git commit -m "feat(assets): full-text q replaces the name substring on the query route and browser filter" -- src/server/src/http/assets/ src/client/ src/modules/asset-browser/ docs/site/modules/asset-browser.md`

### Task 8: e2e suites

**Files:**
- Modify: `src/client/core/src/e2e/search.e2e.test.ts` — add `test("doc_types narrows a search
  to the listed types")`: the GM creates (via `intent`) an actor and a note whose `name`s share a
  unique word (build the note with a valid `NoteEngine` body: `{ source: "…", body: [], sort: 0 }`,
  `default: "observer"`); `client.search(word, { docTypes: ["note"] })` returns exactly the note;
  `docTypes: []` returns both.
- Create: `src/client/core/src/e2e/asset-query.e2e.test.ts` — log in as the GM, upload a 1×1 PNG
  via `POST` multipart to the upload route (copy the upload shape from `chat-image.e2e.test.ts`),
  `PATCH` a tag onto it (`patchAsset`'s route), then `fetch(`${baseUrl}/api/worlds/${world}/assets?q=<name word>&limit=10`)`
  with the cookie → the asset is in `items`; `?q=<tag>` → present; `?q=zzzz` → empty `items`.

- [ ] **Step 1:** write both; run `pnpm --filter @shadowcat/core test:e2e` in the background
  with a log (it builds `test_server`; several minutes); both new cases PASS and nothing else
  regressed. Paste the summary line.
- [ ] **Step 2:** `git commit -m "test(e2e): doc_types search filter and asset full-text query over the real server" -- src/client/core/src/e2e/`

### Task 9: docs, ARCHITECTURE, HISTORY/PLAN

**Files:**
- Modify: `docs/site/protocol.md` (a "Search" paragraph after the frame catalogs: the projection
  rule — envelope name, per-type engine text, `system` leaves; the visibility partition; the
  `doc_types` cap; assets through the query route's `q`), `docs/design/ARCHITECTURE.md` §4 (the
  asset-browser row: replace "+ `Core.search` for FTS (M21)" with "+ the trigger-maintained
  `assets_fts` behind the route's `q` parameter"), `docs/HISTORY.md` (an `### M21 · Search
  consolidation ✅` entry under Phase 2, in the M19 entries' style: branch, spec, delivered items
  by symbol, decisions taken, coverage), `docs/PLAN.md` (delete the M21 section; the Phase-2
  heading's intro sentence stays).

- [ ] **Step 1:** edit; `pnpm lint:comments` PASS (docs are exempt but run it); `pnpm docs:build:portal`
  PASS.
- [ ] **Step 2:** `git commit -m "docs: search consolidation — protocol, architecture, history, plan" -- docs/`

### Task 10: skills (plugin checkout) + full gates

**Files (in `C:/Users/emper/.claude/skills/shadowcat-codebase/skills/`):**
- `shadowcat-codebase-documents-permissions/SKILL.md` — the `data::search` seam bullet: the
  projection (`search_text`, `segments_search_text`), `doc_type` column, `doc_types` filter +
  `MAX_SEARCH_DOC_TYPES`, `doc_type` no longer indexed.
- `shadowcat-codebase-assets/SKILL.md` — `assets_fts` + triggers (Hard invariants: "the asset
  index is trigger-maintained; no Rust write site touches it"), `AssetFilter.query`, route `q`,
  `FilterState.query`.
- `shadowcat-codebase-chat/SKILL.md` — `chat::search_text::segments_search_text` (the one
  reader-facing text extraction).
- `shadowcat-codebase-client-shell/SKILL.md` — `searchDocuments` `docTypes`.
- `shadowcat-codebase-realtime-sync/SKILL.md` — the `Search` frame's `doc_types`.
- `shadowcat-codebase-actors-tokens/SKILL.md` — `ActorsPanel` filters server-side.
- Do NOT commit in the plugin repo — the dispatcher reviews and commits the skill diff (other
  uncommitted edits sit in that checkout; touch only the lines you need).

- [ ] **Step 1:** edit the skills; from the worktree run `node scripts/check-skill-symbol-refs-cli.mjs`
  and `node scripts/check-skill-api-refs-cli.mjs` and `pnpm run test:scripts` — every citation
  you added must resolve.
- [ ] **Step 2:** the FULL gate list from Global constraints (background the long ones), plus
  `pnpm build:all` and `pnpm docs:check-rust-examples`. Paste every result line in the report.
- [ ] **Step 3:** report: `STATUS`, commits, the engine-field classification table from Task 2,
  gate lines, the browser suite NOT RUN, the skill diff (`git -C ~/.claude/skills/shadowcat-codebase diff --stat`).
