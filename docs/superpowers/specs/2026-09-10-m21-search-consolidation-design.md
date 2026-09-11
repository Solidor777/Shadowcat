# M21 · Search consolidation — Design

**Status:** Drafted 2026-09-10 (Phase-2 completion campaign; designed without a brainstorming
dialogue — every fork is decided under "what is the best long-term shape in keeping with our plans
and goals?" and recorded in §10 with the alternatives considered). Consumes M6c (FTS5 one-shot +
live search), M15a (asset query route), M19 (`note`/`table` engine doc types, `Segment` family).
Leaves the seam M20's notes/tables panels consume (`docTypes` on `searchDocuments`).

## 0. Where the codebase already is

Reconnaissance against `main` (`826b9b15`):

- **One FTS backend already exists for documents.** `documents_fts_public`/`documents_fts_gm`
  (`0001_init.sql`), written by `SqliteRepository`'s document write path
  (`reindex_document_fts`, shared with bundle import), deleted by `delete_document_tx` plus the
  `AFTER DELETE` triggers. `data::search::index_content` = `doc_type` + envelope `name` +
  `collect_leaves(engine)` + `collect_leaves(system)`; `index_content_public` runs
  `filter_properties` under a non-GM `Access` first. `build_match` sanitizes untrusted input into
  a MATCH expression; `SqliteRepository::search` ranks by `bm25`, over-iterates under `MAX_SCAN`,
  gates every hit on `cap::READ` and `filter_properties`. Live subscriptions ride the per-connection
  egress task (`Sub { query, limit, fingerprint }`, `SEARCH_DEBOUNCE`, `MAX_SUBSCRIPTIONS`,
  `search_fingerprint`).
- **Notes, tables and messages are already swept — with noise.** `collect_leaves` indexes every
  string and number leaf of the engine band, so a `note` indexes its raw `source` (markdown syntax,
  `[[doc:<uuid>|label]]` spans as UUID fragments) AND its derived `body` (sanitized-HTML tag and
  attribute names, `kind` discriminators such as `html`/`roll_button`, formulas, asset ids); a
  `table` indexes `draw.kind`, every entry's `kind` (`text`/`doc`/`image`/`draw`), table and asset
  UUIDs and dice notation; a `message` indexes segment kinds, markup, `user_owner`, `channel`; an
  `actor` indexes `visual.kind` and asset ids. `doc_type` is indexed too, so the term `note`
  matches every note. BM25 ranks all of that as content.
- **The one per-type consumer under-fills.** `ActorsPanel` requests a 20-hit page and filters
  `doc_type === "actor"` client-side, so a query whose top twenty hits are non-actors renders
  nothing while matching actors exist further down the ranking. `Composer`'s `@doc` picker
  searches every type on purpose.
- **Assets are outside FTS.** `GET /api/worlds/{world}/assets` (`http::assets::query`) maps
  `AssetQuery { folder, recursive, tags, kind, name, name_regex, sort, limit, cursor }` onto
  `AssetFilter { folder, tags, kind, name }`; `query_assets` applies `name` as a `lower(...) LIKE
  '%…%'` substring over `original_name`, tags as exact `asset_tags` joins, and the route applies
  `name_regex` in Rust over up to `MAX_REGEX_PAGES`. The browser's `FilterState { name,
  nameIsRegex, tags, kind, sort }` maps 1:1 onto `queryAssets`. Listings refresh on the out-of-band
  `AssetChanged` notice (`AssetResolver.onListingInvalidated`).
- `docs/design/ARCHITECTURE.md` §4 still carries the M15b row's "+ `Core.search` for FTS (M21)"
  promise, and `docs/site/protocol.md` documents `search` without any type filter.

## 1. Goals / non-goals

**Goals**

1. **One backend.** Every searchable thing — documents of every type, and assets by name and
   tag — is served by SQLite FTS5 through the existing seams (`Repository::search` for
   documents, the asset query route for assets). No second matcher survives beside it.
2. **Engine-aware index content.** The engine band contributes the text a reader sees (a note's
   rendered body, a table's labels and row text, a message's content, an actor's display name) —
   never structural strings (discriminants, ids, markup, notation internals). The `system` band
   stays content-agnostic (invariant 6: the server never interprets it).
3. **A server-side `doc_types` filter** on the `Search` frame and `Repository::search`, so a
   per-type panel never under-fills; `ActorsPanel` uses it, and M20's notes/tables panels build on
   it.
4. **Assets by name and tag through FTS**, maintained structurally (SQL triggers — no Rust write
   site can drift), queried through a new `q` parameter that replaces the LIKE substring.
5. Visibility partitioning, the per-hit READ gate, `build_match`'s sanitization, the subscription
   machinery and the client correlation layer stay exactly as they are.

**Non-goals** (each a decision, §10)

- No Tantivy/Postgres (ARCHITECTURE §4/§5 unchanged). No tokenizer change (`unicode61` stays; no
  stemming — closes M6c's open decision 1).
- No search panel of its own; consumers are the existing actors panel, the composer's `@doc`
  picker, and M20's notes/tables panels.
- No live WS subscription for assets (D11) — the browser already re-queries on `AssetChanged`,
  and assets carry no per-recipient partition (`serve` is membership-gated, nothing else).
  `SearchHit` stays document-only.
- No snippet or BM25 rank for assets: FTS is a FILTER inside the keyset-paginated listing; the
  browser's sort control stays authoritative.
- No compendium / cross-world search.

## 2. Index content

### 2.1 Document projection — `data::search::index_content`

```
content(doc) = [name] ⧺ engine_text(doc_type, engine) ⧺ system_leaves(system)
```

- `name` — the envelope display name when present (unchanged).
- `engine_text` — `data::engine::search_text(doc_type, engine: &Value) -> Option<String>`
  (§2.2), replacing `collect_leaves(engine)`.
- `system_leaves` — `collect_leaves(system)` (unchanged: every string and number leaf, keys/
  booleans/nulls excluded).
- **`doc_type` is no longer indexed** (§10 D1) — the `doc_types` filter (§3) replaces it.

`index_content_public` is unchanged in shape: `filter_properties` under the non-GM `Access`, then
`index_content` over the redacted document — so every projection below runs over a document
already stripped of GM-only properties, and the partition invariant holds by construction.

### 2.2 The engine projection — `data::engine::search_text`

Lives in `data::engine` beside `normalize_engine`, and mirrors its shape: a `match` over
`doc_type` with an explicit arm for EVERY registered engine doc type (the 26
`is_engine_doc_type` names), returning `Some(text)` for a registered type (`Some(String::new())`
for a type with no reader-facing text) and `None` for a non-engine `doc_type`. The exhaustiveness
is pinned by a test that iterates the registry's names and asserts `search_text(name, &minimal_body)`
is `Some` for each — adding a doc type to the registry forces a decision here.

Text-bearing arms (each deserializes the typed struct strictly; a body that fails to deserialize
contributes nothing rather than failing the index write — this is a write-path call, same posture
as `index_content_public`'s redaction failure):

| doc_type | contributes |
|---|---|
| `actor` | `ActorEngine.display_name` (the Rust field; serde-renamed `displayName` on the wire) |
| `note` | `chat::segments_search_text(&NoteEngine.body)` — the derived body, never `source` (§10 D2) |
| `table` | `TableEngine.description`, then per row: `label`, every `TableEntry::Text.text`, `TableEntry::Doc.label`, `TableEntry::Image.alt` |
| `message` | `chat::segments_search_text(&MessageEngine.content)` (never `source`, `channel`, `user_owner`) |
| every other registered type | empty — `token`, `scene`, `wall`, `region`, `light`, `drawing`, `template`, the config singletons, `asset_folder`, the combat family |

The classification rule for the implementer: a field contributes iff it is human-authored or
human-read display text. Before finalizing the table above the implementer enumerates every
`String` field of every engine struct and records the classification in the implementation
report; a field found to be display text (a drawing's label, a region's caption) joins its type's
arm, and the report names it so the reviewer can check. Ids, kinds, notation, URLs stored as
references, and enum discriminants never contribute.

### 2.3 The segment projection — `chat::segments_search_text`

`pub fn segments_search_text(segments: &[Segment]) -> String` in `chat` (beside the `Segment`
enum), the ONE text extraction over a segment list, shared by the `note` and `message` arms
(never two copies). Per variant, every field a recipient reads on the rendered card:

| variant | contributes |
|---|---|
| `Text { text }` | `text` |
| `Html { sanitized_html }` | the text content: every `<…>` run removed, then the named entities ammonia emits (`&amp;` `&lt;` `&gt;` `&quot;` `&#39;` and numeric `&#NNN;`/`&#xHH;`) decoded. Safe because ammonia has already escaped every literal `<` in text, so every remaining `<…>` run is a real element. |
| `RollEmbed { formula, .. }` | `formula` only (a searcher typing `2d6` finds rolls of it) — the struct has NO `label` field; never `outcome`/`roll_id`/`spec`/`raw`/`recalc_history` |
| `RollButton { formula, label }` | `label` and `formula` |
| `LinkPreview { .. }` | title, description, and the URL (a domain is a natural query); never the image asset id |
| `OEmbed(seg)` | title, author/provider names; never ids |
| `DocLink { label, .. }` | `label`; never the target ids |
| `Image { alt, .. }` | `alt` |
| `TableDraw(seg)` | `table_name`, the drawn row's `label`, the row's `content` (recursively through this same function), and every `nested` draw (recursively); never `formula`/`spec`/`raw`/`roll_id` |

The exact field names are read from `Segment`'s definition; the rule (read-facing text only) is
what the implementer applies and the reviewer checks.

### 2.4 Assets — `assets_fts`

A standalone FTS5 table in `0001_init.sql` (edited in place — the single pre-ship baseline):

```sql
CREATE VIRTUAL TABLE assets_fts USING fts5(
  content,               -- original_name + every tag (explicit and derived)
  asset_id UNINDEXED,
  world_id UNINDEXED,
  tokenize = 'unicode61'
);
```

Maintained by triggers, so the index can never disagree with the row (§10 D5):

- `AFTER INSERT ON assets` → write the row's content (name; tags are empty at insert).
- `AFTER UPDATE OF original_name ON assets` → refresh.
- `AFTER DELETE ON assets` → delete the `assets_fts` row (fires under world-delete cascade —
  test-pinned like the document triggers).
- `AFTER INSERT ON asset_tags` and `AFTER DELETE ON asset_tags` → refresh that asset's row.

"Refresh" is one shape, spelled once per trigger body:

```sql
DELETE FROM assets_fts WHERE asset_id = <id>;
INSERT INTO assets_fts (content, asset_id, world_id)
  SELECT a.original_name || ' ' ||
         COALESCE((SELECT group_concat(t.tag, ' ') FROM asset_tags t WHERE t.asset_id = a.id), ''),
         a.id, a.world_id
  FROM assets a WHERE a.id = <id>;
```

Cost is bounded by `MAX_TAGS` per asset per statement; `set_asset_tags`'s delete-all-then-insert
fires one refresh per tag row, each a single-row subquery — acceptable at VTT scale, and stated on
the trigger comment. Bundle import inserts assets and tags through ordinary `INSERT`s, so the
triggers rebuild the index on import; `assets_fts` is never exported. The "search state is rebuilt from `doc`'s content, never
carried across servers" sentence lives on `insert_imported_document`'s doc comment in
`data::sqlite::export_import` — NOT in `data::world_bundle`, whose module doc that comment points
at but which states nothing about search today. Extend that `export_import` sentence to name
`assets_fts`, and add the same statement to `data::world_bundle`'s module doc so the pointer
resolves. A `VACUUM INTO` backup copies the table.

### 2.5 FTS document tables gain `doc_type`

Both `documents_fts_public` and `documents_fts_gm` gain `doc_type TEXT UNINDEXED`, written by
`reindex_document_fts`, so `Repository::search` can filter in SQL (§3.1). A developer database
predating this edit fails the sqlx checksum — delete it and restart (the standing rule).

## 3. Search frame + repository

### 3.1 `Repository::search`

```rust
async fn search(
    &self,
    ctx: &PermissionContext,
    world_id: Uuid,
    query: &str,
    limit: u32,
    cursor: Option<i64>,
    doc_types: &[String],      // empty = every type
) -> Result<SearchPage, DataError>;
```

- `doc_types.len() > MAX_SEARCH_DOC_TYPES` (`= 16`, declared in `data::search`) ⇒
  `DataError::OpFailed("too many doc types")` — refused, never truncated (§10 D4).
- Non-empty ⇒ the ranked SQL gains `AND doc_type IN (?, ?, …)` (one bound parameter per entry —
  never string-interpolated), applied INSIDE the partition table so `MAX_SCAN` counts only
  candidates of the requested types. Everything after the SQL — the per-hit READ gate,
  `filter_properties`, cursor semantics, fingerprinting — is untouched.

### 3.2 Wire

`ClientMsg::Search` gains `#[serde(default)] doc_types: Vec<String>` (ts-rs regenerated; the Zod
mirror in `wire.ts` makes it optional-on-send and the client always sends an array). `Sub` gains
`doc_types: Vec<String>` and the egress task's subscribe + re-evaluation arms pass it through; the
over-cap refusal surfaces as `SearchError { message: "too many doc types" }` on the correlated
`request_id` — the same frame every other search refusal uses. `docs/site/protocol.md`'s `search`
row names the field.

### 3.3 Client

- `WsSearchOptions.docTypes?: string[]` and `WsSubscribeSearchOptions.docTypes?: string[]`;
  `WsClient.search`/`subscribeSearch` send `doc_types: opts.docTypes ?? []`.
- `AppContext.searchDocuments(query, { limit?, timeoutMs?, docTypes? }, onUpdate)`; `WorldSession`
  forwards `docTypes`.
- `ActorsPanel` sends `docTypes: [ACTOR_DOC_TYPE]` and its client-side `doc_type === "actor"`
  filter is deleted — a second filter after the server's is the forked-decision shape.
- `Composer`'s `@doc` picker keeps searching every type (unchanged call).

### 3.4 Asset query route

- `AssetQuery.name` is replaced by `AssetQuery.q: Option<String>` (full-text over name + tags);
  `name_regex` stays (§10 D7). `AssetFilter.name` becomes `AssetFilter.query: Option<String>`.
- `query_assets` applies it as `AND a.id IN (SELECT asset_id FROM assets_fts WHERE assets_fts
  MATCH ?)` with the expression built by `data::search::build_match` — the ONE sanitizer; a `None`
  from `build_match` (empty/punctuation-only query) yields an empty page, mirroring
  `Repository::search`. The LIKE branch is deleted.
- The route stays a keyset-paginated listing under the caller's `sort`; `is_bare` treats `q` like
  every other parameter.
- Client: `AssetQuery.name` → `AssetQuery.q` (`queryAssets` sends `q`); `FilterState.name` →
  `FilterState.query` and `nameIsRegex` → `queryIsRegex`; `FilterBar`'s placeholder copy says
  "Search name and tags" / "Regex over name". The `filter-name` test id is renamed
  `filter-query`.

## 4. Security posture

- **Partition intact.** Every projection runs over the redacted document in the public build;
  the GM build indexes the full projection. Nothing new reaches the public partition that
  `filter_properties` would strip: `spec`/`raw` are excluded by the projection AND by redaction.
- **`doc_types` is a bound parameter list, capped in count**; a doc_type string is arbitrary
  client text and never touches SQL text.
- **Assets stay membership-gated** — the query route's `require_member` is untouched; the FTS
  subquery only narrows rows the caller could already list.
- **`build_match` is the single MATCH sanitizer** for both `Repository::search` and
  `query_assets`; no second sanitizer is written.

## 5. Tests

Server (`cargo test`, sibling files):
- `data::search::tests` — projection: a note whose body renders `<strong>` does NOT match
  `strong`; a note does NOT match a UUID fragment of a `[[doc:…]]` span in its `source` but DOES
  match a word of its rendered text and a `roll_button` label; `roll_button`/`html`/`image` (the
  discriminants) match nothing; a table matches its `description`, a row `label`, a `Text` entry
  word and an `Image` alt but NOT `weighted`/`text`/`draw` or a nested table's id; a message
  matches its rendered content and a link-preview title but NOT `roll_embed`; an actor matches
  `displayName` but NOT `image` (its visual kind); `doc_type` words match nothing; the registry
  exhaustiveness test (§2.2); `segments_search_text` per variant incl. the recursive `TableDraw`
  and the entity decoding.
- `data::sqlite` (search subject) — `doc_types`: `["note"]` on a corpus of matching actors + notes
  returns only notes; empty = every type; over-cap refused; a player's `["note"]` search never
  returns a `default: none` note (partition + filter compose); the page cursor resumes correctly
  under a filter.
- `data::sqlite::assets` — `query` matches a name word, an explicit tag, a derived tag; a rename
  refreshes; a tag removal stops matching; a deleted asset leaves no `assets_fts` row; world
  deletion leaves none for that world; bundle import indexes imported assets; an empty/punctuation
  `query` yields an empty page; `query` composes with `folder`/`kind`/`tags`/`name_regex`.
- `http::assets::query::tests` — `q` reaches the filter; `name` is no longer a parameter.
- `ws::protocol` — `search` frame parses with and without `doc_types`.

Client (`pnpm -r test`): `ws-client.test.ts` (`doc_types` on the wire for `search` and
`subscribeSearch`, default `[]`), `ActorsPanel.test.ts` (sends `docTypes: ["actor"]`; no client
filter), `asset-rest.test.ts` (`q`), `FilterBar`/`filterState` tests, `worldSession` forwarding.

e2e (`pnpm --filter @shadowcat/core test:e2e`): `search.e2e.test.ts` gains a `doc_types` case
(GM creates an actor and a note sharing a word; `["note"]` returns only the note); a new
`asset-query.e2e.test.ts` uploads a PNG over REST with a tag and asserts `?q=` finds it by name
and by tag over the real server. Playwright: `asset-browser.spec.ts` is updated wherever it drives
the renamed filter control (the dispatcher runs the suite).

## 6. Docs, skills, gates

- `docs/site/protocol.md` (`search` row + a short "Search" paragraph: the projection rule, the
  type filter, assets through the query route); `docs/site/modules/asset-browser.md` (filter bar:
  search / regex); `docs/site/modules/actors.md` (server-side type filter);
  `docs/design/ARCHITECTURE.md` §4 (the asset-browser row's `Core.search` promise becomes the
  delivered `assets_fts` state; the FTS-engine row unchanged); `docs/HISTORY.md` M21 entry;
  `docs/PLAN.md` M21 entry removed; `docs/TODO.md` unchanged.
- Skills (plugin checkout, reviewed skill-update gate): `documents-permissions` (the `data::search`
  seam: projection, `search_text`, `doc_type` column, `doc_types` filter), `assets` (`assets_fts`
  triggers, `q`), `chat` (`segments_search_text`), `client-shell` (`searchDocuments` `docTypes`),
  `realtime-sync` (the `Search` frame field), `actors-tokens` (`ActorsPanel` filter). Hook map
  unchanged.
- Gates: the full battery in the campaign brief; ts-rs bindings regenerated and committed with
  the frame change.

## 7. Wire types

| Surface | Change |
|---|---|
| `ClientMsg::Search` | `+ doc_types: Vec<String>` (`#[serde(default)]`) |
| `ServerMsg` | none (`SearchError` reused) |
| `Repository::search` | `+ doc_types: &[String]` |
| SQL | `documents_fts_public`/`_gm` `+ doc_type UNINDEXED`; new `assets_fts` + five triggers |
| REST | `GET /api/worlds/{world}/assets`: `name` → `q` |
| client | `WsSearchOptions.docTypes`, `WsSubscribeSearchOptions.docTypes`, `AppContext.searchDocuments` opts, `AssetQuery.q`, `FilterState.query`/`queryIsRegex` |

## 8. Build order

One branch (`m21-search`), merged before M20's panel tasks start:
1. Projection (`search_text`, `segments_search_text`, `index_content`) + tests.
2. `doc_type` column + `doc_types` on `Repository::search` + the `Search` frame + `Sub` + ts-rs.
3. Client `docTypes` + `ActorsPanel`.
4. `assets_fts` + triggers + `q` on the route/repo + client filter rename.
5. e2e, docs, skills, HISTORY/PLAN.

## 9. Open questions for the user

None. Every fork resolves under "best long-term shape in keeping with our plans and goals".

## 10. Decision log

| # | Fork | Decision | Alternatives and why they lose |
|---|---|---|---|
| D1 | `doc_type` in content | dropped; `doc_types` filter replaces it | keeping it makes the type word match every document of the type — noise with no consumer once a filter exists |
| D2 | What a note indexes | the derived `body` (reader-facing text, labels not ids), through the projection messages share | `source` indexes markdown syntax and UUID span fragments; indexing both double-counts every word |
| D3 | Engine band projection | explicit per-type `search_text` with an exhaustive registry match; `system` stays leaf-swept | leaf-sweeping the engine band indexes discriminants, ids and markup (the observed noise); projecting `system` would interpret the opaque band |
| D4 | Type filter | server-side SQL `IN` over an UNINDEXED column; over-cap refused | client-side filtering under-fills (the `ActorsPanel` defect); truncating an over-cap list silently answers a different question |
| D5 | Asset index maintenance | SQL triggers on `assets` + `asset_tags` | Rust write sites (`insert_asset`, `set_asset_tags`, `update_asset_placement`, `bulk_update_assets`, `replace_asset_bytes`, import) are six places that must agree — the forked-decision class; asset content is pure SQL-derivable, unlike documents (redaction needs Rust) |
| D6 | Assets on the WS `Search` frame | no — REST `q` on the existing route | `SearchHit` carries a `Document`; a union would force every document consumer to filter, and assets have neither a partition nor a document-stream liveness model |
| D7 | `name_regex` | kept | regex is a power FILTER over a listed page, not a second search backend; the LIKE substring WAS one and is deleted |
| D8 | Asset ranking | FTS as a filter under the caller's keyset sort | BM25 ranking would break keyset pagination and fight the browser's explicit sort control |
| D9 | Tokenizer | `unicode61` unchanged | porter stemming is English-only and rebuilds the index; nothing has shown BM25 quality to be the problem — content was |
| D10 | `TableDraw` in messages | indexed by table name, row label and content, recursively | excluding draws makes a rolled result unsearchable; including `spec`/`raw` would put GM-only data in the projection before redaction (excluded structurally, not only by redaction) |
| D11 | Assets and PLAN.md's "index + live subscriptions … (assets by tag)" | assets join the FTS index (`assets_fts`, D5) but get NO WS search subscription; `AssetChanged` is their liveness channel | a document-stream subscription re-runs a ranked document query per egress task and ships `SearchHit { Document }` rows — assets are not documents, have no per-recipient partition, and the browser already re-lists on every `AssetChanged`; a second subscription kind would be a second liveness model for the same list. PLAN.md's clause is read as "one index for every type, live where a document stream exists" |
