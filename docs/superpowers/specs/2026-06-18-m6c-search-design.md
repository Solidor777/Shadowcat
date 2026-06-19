# M6c — Full-Text Search: Design Spec

> Status: **DRAFT for review.** Third and last M6 sub-milestone (M6a client core ✅,
> M6b modules + capabilities ✅, M6c search). M6c is **decomposed** into:
> **M6c-1 (this spec)** — FTS5 infrastructure + a transport-agnostic search core +
> a WebSocket request/response search frame + `Core.search` (one-shot search); and
> **M6c-2 (later spec)** — live query subscriptions (server-pushed result updates),
> built additively on M6c-1's frame and core. No UI (M7).

## 1. Goal

Server-authoritative, per-recipient-filtered full-text search over a world's
documents. A client issues a query and receives ranked, read-filtered results
carrying the matching documents, their relevance score, and a snippet — without
the client needing to have loaded those documents. The durable query +
permission-filter + rank logic lives in the data layer so the storage engine
(FTS5 now; Tantivy/Postgres later, behind the existing `Repository` trait) and a
future live-subscription transport can change without rewriting it.

## 2. Invariants preserved

- **Server-authoritative, structural-only** (ARCHITECTURE #1, #6). The server
  indexes and ranks **text** content-agnostically; it never interprets the
  meaning of the opaque `system` body. Indexed content is the extracted text
  leaves of the body plus `doc_type` — extraction, not interpretation.
- **Permissions enforced server-side, per recipient** (ARCHITECTURE #4). Results
  are filtered through the same `resolve_access_world` (`core:read`) and
  `filter_properties` (GM-only redaction) used for document reads. An actor never
  sees a document — or a redacted field — in search that they could not read
  directly.
- **Ordered, recoverable realtime is untouched.** Search is request/response and
  does not participate in the per-world sequence stream. The FTS index is kept
  **crash-consistent**: its rows are written inside the same transaction as the
  document mutation (ARCHITECTURE rejected-alternatives note: "FTS5 is
  crash-consistent — updates inside the row's transaction").
- **Postgres/Tantivy deferred behind a seam** (ARCHITECTURE §4). `search` is a
  `Repository` trait method; the FTS5 implementation is one impl.

## 3. Package & boundaries

| Unit | Location | Responsibility | Depends on |
|---|---|---|---|
| FTS5 table | `src/server/migrations/0003_fts.sql` | Standalone FTS5 virtual table | — |
| Leaf-text extraction | `src/server/src/data/search.rs` | Document JSON → indexed `content` string | serde_json |
| Transactional sync | `src/server/src/data/sqlite.rs` | Rewrite the FTS row in the document write tx | extraction |
| Search core | `Repository::search` + sqlite impl | Sanitize → MATCH/BM25 → read-filter → assemble | permission.rs |
| WS frames | `src/server/src/ws/protocol.rs`, `conn.rs` | `Search` / `SearchResult` / `SearchError` | search core |
| Request correlation + `Core.search` | `src/client/core/src/ws-client.ts`, `search.ts` | Correlated request/response; client API | M6a WsClient |

## 4. Indexing & synchronization

A standalone (non-external-content) FTS5 virtual table:

```sql
CREATE VIRTUAL TABLE documents_fts USING fts5(
  content,                 -- indexed: extracted leaf text + doc_type
  doc_id UNINDEXED,        -- the document id (returned, not tokenized)
  world_id UNINDEXED,      -- world scoping (filtered, not tokenized)
  tokenize = 'unicode61'   -- default Unicode tokenizer; diacritics folded
);
```

- **Content extraction** (`data/search.rs`): a Rust walk of the document's JSON
  body that concatenates every **string and number leaf value** (recursing
  objects and arrays), plus the `doc_type`. JSON **keys**, structural tokens, and
  envelope identifiers (ids, UUIDs, permission data) are **not** indexed — they
  add noise and risk matches on structure. This is content-agnostic: it extracts
  text, it does not parse meaning.
- **Sync point**: in `apply_intent` and `apply_command`, every `Create` / `Update`
  / `Delete` rewrites the document's FTS row **inside the same transaction** as
  the `documents` write. The FTS row is keyed to the `documents` rowid (delete +
  insert on upsert; delete on delete). A rolled-back write leaves no FTS row —
  index and table never diverge. Sync is **application-level** (the extraction is
  Rust logic; SQL triggers cannot compute it).

## 5. Search core (data layer)

`Repository::search(ctx, world_id, query, limit, cursor) -> SearchPage` where
`SearchPage { results: Vec<SearchHit>, next_cursor: Option<Cursor> }` and
`SearchHit { document: Document, score: f64, snippet: String }`.

1. **Sanitize the query.** Raw user input is **never** passed to FTS5 `MATCH`
   (stray syntax throws; bare operators inject behavior). The input is tokenized
   into terms; each term is double-quoted (escaping embedded quotes) and the
   terms are AND-combined; the final term receives a trailing `*` for
   type-ahead prefix matching. An empty/whitespace query yields an empty page.
2. **Rank.** `SELECT rowid, doc_id, bm25(documents_fts) AS score,
   snippet(documents_fts, 0, '<mark>', '</mark>', '…', 16) AS snippet FROM
   documents_fts WHERE documents_fts MATCH ?1 AND world_id = ?2 ORDER BY score`
   (BM25 ascending = most relevant first), reading from `cursor` onward.
3. **Permission-filter, no under-fill.** Iterate ranked candidates in order:
   load each document, compute `resolve_access_world` for `ctx`; skip those
   lacking `core:read`; for the rest apply `filter_properties` (GM-only
   redaction). Accumulate readable, filtered hits until `limit` is reached or the
   candidates are exhausted. Return `next_cursor` = the raw-rank position after
   the last consumed candidate (so the next page resumes correctly even though
   redaction removed some). A page is therefore never short because of redaction.
4. **Scope.** Per-world only (the `world_id` from the request). Compendium /
   cross-pack search stays deferred (ARCHITECTURE §4 asset/compendium browser,
   Phase 2).

`world_cap_defaults` is loaded once per search call (mirrors the
`apply_intent` / egress pattern) and passed to `resolve_access_world`.

## 6. WS protocol & client API

New ts-rs-exported frames:

```rust
// ClientMsg
Search { request_id: Uuid, query: String, limit: u32, cursor: Option<String> }
// ServerMsg
SearchResult { request_id: Uuid, hits: Vec<SearchHit>, next_cursor: Option<String> }
SearchError  { request_id: Uuid, message: String }
```

`SearchHit { document: Document, score: f64, snippet: String }` (the `Document`
is the per-recipient-filtered view).

The M6a `WsClient` is broadcast-only; M6c-1 adds a **generic request/response
correlation layer** (not search-specific): an outbound correlated frame returns a
`Promise` stored in a pending-request map keyed by `request_id`, resolved on the
matching `SearchResult`, rejected on `SearchError` or a timeout. `Core.search`:

```ts
search(query: string, opts?: { limit?: number; cursor?: string })
  : Promise<{ hits: SearchHit[]; nextCursor?: string }>
```

The correlation layer and frames are deliberately shaped so **M6c-2** adds a
`subscribe` flag to `Search` and a `SearchUpdate` push frame without reworking
either side.

## 7. Error handling

| Condition | Behavior |
|---|---|
| Empty / whitespace query | Empty page (no error) |
| Unparseable input | Sanitized to terms; FTS5 syntax never reaches the engine |
| FTS/query engine error | `SearchError{request_id}`; never a socket-loop throw |
| Search in an inaccessible world | No `PermissionContext` → rejected like any join |
| Unreadable candidate doc | Skipped silently (never leaked) |
| Client request timeout | The `Core.search` Promise rejects |

## 8. Testing

- **Rust unit:** leaf-text extraction (nested objects/arrays, numbers, excludes
  keys/ids); query sanitization (operators/quotes neutralized; trailing prefix).
- **Rust integration:** FTS sync inside the write tx (create/update/delete
  reflected; rolled-back write leaves no row); `search` ranks by BM25, **excludes
  GM-only and unreadable documents per recipient**, and paginates via cursor with
  no under-fill.
- **TS unit:** `WsClient` request correlation (resolve by `request_id`, reject on
  error frame, reject on timeout); `Core.search` result shape.
- **e2e (extends the M6c harness from M6b):** the real client searches the real
  `test_server`; a player's results exclude a GM-only document that the GM's
  results include.

## 9. Execution slices (for the implementation plan)

1. FTS5 migration + leaf-text extraction + transactional sync (Rust
   unit/integration).
2. `Repository::search` core — sanitize, rank, read-filter, paginate, snippet
   (Rust integration).
3. WS `Search` / `SearchResult` / `SearchError` frames + ts-rs regen (Rust).
4. `WsClient` request-correlation layer + `Core.search` (TS unit).
5. e2e search test + docs sync.

## 10. Decisions settled in brainstorming

1. **What to index** — all text leaves of the body + `doc_type` (content-agnostic
   extraction; respects the opaque-body invariant).
2. **Transport** — a WebSocket request/response frame (matches the PLAN's "search
   protocol frame"; chosen to enable M6c-2 live results, the better UX).
3. **Scope decomposition** — M6c-1 one-shot search now; M6c-2 live subscriptions
   next, on the same frame + core.
4. **Result payload** — full per-recipient-filtered documents + BM25 score + FTS5
   snippet (client renders directly, works for unloaded docs; reuses proven
   filtering).
5. **Pagination** — opaque cursor with server-side over-iteration so redaction
   never yields a short page.
6. **Correlation layer** — generic request/response on `WsClient`, not
   search-specific, reusable by M6c-2 and future request/response ops.

## 11. Open decisions (for review)

1. **Tokenizer** — `unicode61` (default, diacritic-folding) for M6c-1 vs adding
   `porter` stemming. Leaning `unicode61`; stemming can be a later index rebuild.
2. **`limit` ceiling** — clamp the per-request `limit` to a max (e.g. 100) to
   bound work/payload? Leaning yes.
3. **Numbers in `content`** — index numeric leaves as text (so "30" matches a
   stat) vs strings only. Leaning include numbers (cheap, occasionally useful).
