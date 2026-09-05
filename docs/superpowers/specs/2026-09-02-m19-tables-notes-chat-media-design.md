# M19 · Tables, Notes + Chat Media — Design

**Status:** Drafted 2026-09-02 (phase-2 close-out campaign; designed without a brainstorming
dialogue — every fork is decided here by "what is the best long-term shape in keeping with our
plans and goals?" and recorded in §11 with the alternatives considered). Consumes M11 (dice + chat),
M14c-1/-4 (server formula engine, server-side reference resolution, channel validation), M15a
(asset pipeline, `create_asset_from_bytes`) and M15b (`AppContext.pickAsset`). Leaves the seams
M20 (table/notes sheets) and M21 (FTS over notes/tables) build on.

## 0. Where the codebase already is

Reconnaissance against `main` (`b6624c36`) before any design:

- **YouTube is already delivered.** `chat::oembed` (`OEmbedProvider::YouTube`/`Vimeo`,
  `match_provider` host allowlist, no autodiscovery) and `chat::post_publish::resolve_oembed` /
  `resolve_thumbnail_asset` produce a `Segment::OEmbed(OEmbedSegment)` whose thumbnail is an
  asset-ified copy (`thumbnail_asset_id`) and whose `OEmbedSegment` has **no `html` field by
  construction** — the client renders a first-party card with an external link. That is exactly
  PLAN.md's "thumbnail + external link only — no IFrame / Data API". M19 adds no YouTube code; §7
  names the tests that pin the requirement and §9 lists the documentation that records it.
- **Images in chat exist but hotlink.** `chat::sanitize::ammonia_for` keeps `<img src>` under the
  `images` policy behind a lexical extension allowlist, and its own comment says the tracking-pixel
  threat "would need image-proxying … tracked as follow-up work" — no TODO.md entry tracks it. M19a
  closes it: an image reaches a recipient only as `Segment::Image { asset_id }` served by this
  server; external image URLs are fetched server-side through the SSRF-guarded client and
  asset-ified, exactly as `og:image`/oEmbed thumbnails already are.
- **`Segment::DocLink` + `scan_body`'s `[[…]]` grammar** already carry structured references
  through chat bodies; tables and notes reuse that grammar rather than inventing one.
- **Server-authored roll messages have a precedent**: `combat::transition::roll` builds a
  `MessageKind::Roll` document through `build_message_doc` + `MessageDraft` with a `RollEmbed`
  carrying `spec`/`raw`. A table draw is the same shape.
- **The sheet modules for tables and notes are M20's** (PLAN.md M20: "table/notes sheets"). M19
  ships the server, the `@shadowcat/core` builders/mirrors, the chat rendering, and the ui-kit
  segment renderer M20's sheets will reuse; M19's UI-level e2e for those sheets therefore lands in
  M20 (§8 states which suite covers what).

## 1. Goals / non-goals

**Goals**

1. **Rollable tables** as engine documents (`doc_type: "table"`): weighted rows or a dice-formula
   with row ranges; rows yield text, document links, images, or nested draws from other tables;
   draws execute **on the server** and post to chat as roll embeds (`Segment::TableDraw`), with
   `spec`/`raw` GM-only exactly like `RollEmbed`.
2. **Rich-text notes** as engine documents (`doc_type: "note"`): author markdown in `source`, a
   **server-derived** sanitized `body: Vec<Segment>` produced at ingress through the chat
   sanitizer boundary, `[[doc:…]]`/`[[token:…]]` cross-references, `[[roll:…]]` buttons,
   `[[asset:…]]` images; a `parent_id` tree of notes.
3. **Chat media**: images as `Segment::Image { asset_id, alt }` from an `[[asset:<uuid>|alt]]`
   span (composer picker) and from external image URLs the server fetches and asset-ifies
   post-publish; the `<img src>` hotlink is removed. YouTube stays as shipped.
4. Everything server-authoritative per invariant 6; three-band shape; `deny_unknown_fields`;
   per-recipient redaction through the existing classifier; no forked decisions (§11 names each
   shared symbol).

**Non-goals** (each is a decision, see §11)

- No table/note **sheet modules** (M20). No FTS tokenization work for notes/tables (M21 — both
  are already swept by `index_content` as string leaves; M21 owns quality).
- No recalculation of table draws (`handle_recalc_roll` refuses them; a GM redraws).
- No per-row visibility on tables; no GM-only section inside a note (whole-document permissions;
  two documents for two audiences).
- No link previews or outbound fetches for note bodies or table-row text; no external images in
  notes (assets only).
- No draw-without-replacement / consumable tables.
- No YouTube IFrame or Data API, unchanged.

## 2. Document types (engine band)

All new structs live in `src/server/src/data/engine/`, `#[serde(deny_unknown_fields)]`, ts-rs
`#[ts(export, export_to = "../../types/generated/engine/")]`, every item documented (the module's
`#![deny(missing_docs)]` pair is live). Each doc_type joins `is_engine_doc_type` **and**
`normalize_engine`'s match in the same commit (the `unreachable!` arm pins the pair).

### 2.1 `table` — `data::engine::table::TableEngine`

```rust
pub const TABLE_DOC_TYPE: &str = "table";

/// The engine body of a rollable table. Envelope `name` is the table's name.
pub struct TableEngine {
    /// How a draw selects a row.
    pub draw: DrawRule,
    /// The rows, in display order. Whole-array replaced on edit (`set_pointer`
    /// cannot grow arrays), same as every other engine array.
    pub rows: Vec<TableRow>,
    /// Plain-text description shown on the sheet (never rendered as markup).
    #[serde(default)]
    pub description: String,
}

#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DrawRule {
    /// Uniform over the sum of row weights: the draw rolls `1d<sum>` and the
    /// cumulative-weight row containing the total wins.
    Weighted,
    /// A reference-free dice formula evaluated in Total mode; the row whose
    /// `range` contains the total wins (no row ⇒ a "no matching row" draw).
    Formula {
        /// Dice notation; validated at ingress (§3.1).
        notation: String,
    },
}

pub struct TableRow {
    /// ≥ 1. Used by `DrawRule::Weighted`; must still be ≥ 1 under `Formula`.
    pub weight: u32,
    /// Inclusive total range under `DrawRule::Formula`; must be `None` under `Weighted`.
    #[serde(default)]
    pub range: Option<RowRange>,
    /// Short plain-text headline for the chat card (≤ `MAX_ROW_LABEL_CHARS`).
    pub label: String,
    /// What the row yields, in order. May be empty (a "nothing happens" row).
    #[serde(default)]
    pub results: Vec<TableEntry>,
}

pub struct RowRange { pub lo: i64, pub hi: i64 }   // lo ≤ hi

#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TableEntry {
    /// Markdown; sanitized AT DRAW TIME under the world's chat policy (§3.2).
    Text { text: String },
    /// → `Segment::DocLink` (reuses `chat::DocLinkTarget`).
    Doc { target: DocLinkTarget, label: String },
    /// → `Segment::Image`; the asset must exist in the drawing world at draw time.
    Image { asset_id: Uuid, alt: String },
    /// Nested draw: `count` draws from `table_id`, each its own roll.
    Draw { table_id: Uuid, count: u32 },
}
```

Constants (`data::engine::table`): `MAX_TABLE_ROWS = 1000`, `MAX_ROW_LABEL_CHARS = 200`,
`MAX_ROW_TEXT_CHARS = 2000`, `MAX_TABLE_DESCRIPTION_CHARS = 2000`, `MAX_NESTED_DRAW_COUNT = 10`,
`MAX_IMAGE_ALT_CHARS = 200` (shared with `Segment::Image` — declared once in `chat`, imported here).

`TableEngine::validate` (wired into `normalize_engine`'s `"table"` arm like `combat`):
- `rows.len() ≤ MAX_TABLE_ROWS`; every `weight ≥ 1`; `label` non-empty after trim and ≤ cap;
  `Text.text` ≤ cap; `Image.alt` ≤ cap; `Draw.count` in `1..=MAX_NESTED_DRAW_COUNT`;
  `description` ≤ cap.
- `Weighted`: every `range` is `None`; `Σ weight ≤ chat::rolls::MAX_DIE_SIDES` (the ONE
  declaration of the die-size cap — the draw rolls `1d<sum>` through the chat boundary, so a sum
  the boundary would refuse is refused at authoring time by the same symbol).
- `Formula { notation }`: `notation` is reference-free and parses to `Mode::Total` under
  `TABLE_PARSE_CONTEXT` (§3.2) via `chat::rolls::validate_table_formula` (new, beside
  `validate_formula`: resolves the template through `NoHostResolver` so ANY reference is an error,
  parses, runs `validate_pre_roll`, then refuses a `Mode::SuccessCount` spec — a table needs a
  total); every row has `Some(range)` with `lo ≤ hi`; ranges pairwise disjoint.
- Cross-document references (`Doc.target`, `Image.asset_id`, `Draw.table_id`) are NOT resolved at
  ingress — the `DocLink` precedent; they are resolved fail-closed at draw time (§3.2).

Containment (`data::validation::validate_containment`): a `table` has no `parent_id` and may not
be an `embedded` child (the `combat` rule shape).

### 2.2 `note` — `data::engine::note::NoteEngine`

```rust
pub const NOTE_DOC_TYPE: &str = "note";

pub struct NoteEngine {
    /// The author's markdown (≤ `MAX_NOTE_SOURCE_CHARS`). Never rendered as HTML by any client.
    pub source: String,
    /// SERVER-DERIVED from `source` by `normalize_engine`'s `"note"` arm on every
    /// Create/Update post-image. Whatever a client sends here is discarded and
    /// overwritten; the client's optimistic mirror shows `source` until the echo arrives.
    #[serde(default)]
    #[ts(type = "unknown[]")]
    pub body: Vec<Segment>,
    /// Sibling ordering under one parent (client-chosen; ties by `created_at`).
    #[serde(default)]
    pub sort: i64,
}
```

Constants: `MAX_NOTE_SOURCE_CHARS = 65_536` (chars), `MAX_NOTE_SPANS = 64` (non-text `[[…]]`
chunks per note — `MAX_INLINE_ROLLS = 8` is a chat-message cap and too small for a journal page).

`normalize_engine("note")`: deserialize → `NoteEngine::validate` (source cap) →
`body = chat::body::compose_static(&source, &NOTE_CONTENT_POLICY, MAX_NOTE_SPANS)?` (§3.3; a
scan/parse failure maps to `DataError::BadEngine` with the player-presentable `RollError`
`Display`, so the sheet's rejected intent shows why) → re-serialize. Deterministic, no I/O: it
runs identically under `apply_intent` and `apply_command` replay.

`NOTE_CONTENT_POLICY` (`chat::settings`, a `const ChatContentPolicy`): markdown on, html off,
images on, hyperlinks on, emails off, link previews off. Fixed, not the world's chat policy (§11
N3).

Tree: `parent_id` must name a `note` in the same scope — `check_note_parent`, a sibling of
`data::sqlite::assets::check_asset_folder_parent`, dispatched from the shared
`check_parent_placement` helper that the Create AND `Operation::Move` arms of both
`apply_intent` and `apply_command` call (`Operation::Move`, `check_parent_placement` and
`check_move_acyclic` are M15b's — on `m15b-asset-browser`, not yet on `main` at design time;
M15b's merge is a prerequisite of M19a and therefore of this sub-project); `check_move_acyclic`
already covers cycles; the generic parent cascade already deletes children with their parent.
`validate_containment` forbids a `note` as an `embedded` child (the `asset_folder` rule).

Permissions: ordinary document. `buildNoteDoc` (client) defaults `permissions.default:
DocRole::None` with `users[author] = Owner` — private to author + GM until shared (§11 N4).
Creation rides the world's `core:create` grant like any other doc_type (GM by default; a GM
widens it per doc_type through world capability defaults — no note-specific exemption, §11 N5).

### 2.3 `message` — new `Segment` variants (`chat::Segment`, serde-only, Zod-mirrored)

```rust
/// An image served by THIS server's asset endpoint. Never an external URL.
Image {
    asset_id: Uuid,
    /// ≤ `MAX_IMAGE_ALT_CHARS`; may be empty. Plain data, escaped at render.
    alt: String,
},
/// One executed table draw (recursive through nested draws). Produced only by
/// `tables::handle_draw_table`. `spec`/`raw` are GM-only (see `roll_property_overrides`).
TableDraw(TableDrawSegment),
```

```rust
pub struct TableDrawSegment {
    pub table_id: Uuid,
    /// Envelope `name` at draw time ("" when the table has none); never re-resolved.
    pub table_name: String,
    /// Stable identity (mirrors `RollEmbed.roll_id`). NOT a recalc target (§11 T6).
    pub roll_id: Uuid,
    /// `1d<sum>` for `Weighted`, the table's notation for `Formula`.
    pub formula: String,
    pub outcome: RollOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub spec: Option<Box<RollSpec>>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub raw: Option<Box<RawRoll>>,
    /// `None` = a `Formula` total matched no range (rendered "no matching row").
    pub row: Option<DrawnRow>,
}
pub struct DrawnRow {
    pub index: usize,
    pub label: String,
    /// The row's `results` rendered to segments (Text → sanitize, Doc → DocLink, Image → Image).
    pub content: Vec<Segment>,
    /// One entry per nested draw, in `results` order then `count` order.
    pub nested: Vec<TableDrawSegment>,
}
```

`chat::roll_embed_property_overrides` is renamed `roll_property_overrides` and walks BOTH
`RollEmbed` and `TableDraw` (recursing `row.nested`), emitting `GmOnly` for every `spec`/`raw`
pointer (`/engine/content/{i}/spec`, `/engine/content/{i}/row/nested/{j}/raw`, …). Every pointer
lands inside the untyped `engine` `Value`, so `redaction_target` classifies it `Within` with no
classifier change. `outcome`, `row.label`, `row.content` stay visible to every recipient.

### 2.4 Assets — `data::asset::Provenance::ChatImage`

A third variant beside `Uploaded`/`LinkPreview`; derived tag `CHAT_IMAGE_TAG = "chat-image"`
in `data::asset::tags` (`provenance_of` learns it). No schema change: the tag lands in
`asset_tags` like every derived tag.

## 3. Server behaviour

### 3.1 Registry + validation

`is_engine_doc_type` gains `"table"` and `"note"`; `normalize_engine` gains both arms (`table`
validates, `note` validates + derives `body`). The engine doc-type count quoted in
`docs/design/ARCHITECTURE.md` invariant 6 and in the skills is re-counted from the match arms
(it reads 23 today while the match already holds 24 with `asset_folder`; after M19 it is 26) —
the implementer corrects it from the source, never from this paragraph.

### 3.2 Table draws — new module `src/server/src/tables/`

`tables::{mod.rs, draw.rs, tests.rs}` (sibling test file; `#![deny(missing_docs)]` pair on both).

**Wire:** `ClientMsg::DrawTable { request_id: Uuid, table_id: Uuid, channel: String,
#[serde(default = "one")] count: u32, #[serde(default)] actor_owner: Option<ActorOwnerRef>,
#[serde(default)] audience: Audience }`. Rejections ride the existing `ServerMsg::ChatError
{ request_id, message }`; success is confirmed only by the broadcast `Event` echo (the four chat
frames' contract). `count` ≤ `MAX_TOP_LEVEL_DRAWS = 10`.

**`tables::handle_draw_table(req: DrawTableRequestCtx<'_>, table_id, channel, count,
actor_owner, audience) -> Result<Command, DrawTableError>`**, dispatched from `ws::conn`'s new
`ClientMsg::DrawTable` arm exactly as the `RecalcRoll` arm is. `DrawTableRequestCtx` groups
`room`/`repo`/`ctx`/`rate`/`world_defaults`/`now`/`budget_per_min` (the `MessageRequestCtx`
shape; `world_defaults` acquired the way `combat::handle_combat_intent` acquires it). In order:

1. `rate.check(user, now, budget_per_min)` — the chat flood budget (`MESSAGE_RATE_PER_MIN`); a
   draw is a message.
2. `chat::channel_registered(repo, world, &channel)` — `UnknownChannel` else.
3. `chat::validate_audience(repo, world, user, &audience)` and, when present,
   `chat::validate_actor_owner(repo, room, ctx, &actor_owner)` — both EXTRACTED from
   `handle_send_message` as named `pub(crate)` functions in this sub-project (pure moves; the
   existing send tests pin them) so the draw path and the send path share one attribution and
   one recipient rule, never two copies.
4. `count ∈ 1..=MAX_TOP_LEVEL_DRAWS` — `TooMany` else.
5. `policy = chat::resolve_content_policy(repo, world)` once (row text sanitizes under it).
6. `count` × `draw::draw_table(&mut DrawCtx, table_id, depth = 0)` → `Vec<TableDrawSegment>`.
7. `build_message_doc(world, user, MessageDraft { channel, actor_owner, audience, kind:
   MessageKind::Roll, content: segments, source: None }, now)` and `room.publish(…,
   vec![Operation::Create { doc }], now, WriteOrigin::Client)` — the SAME sole authoring path
   messages already have (`build_message_doc` stays the one construction site; `ops_target_message`
   still refuses any client-authored message op).

`draw::draw_table` (`DrawCtx` carries `repo`, `ctx`, `world_defaults`, `policy`, `world_id`, a
`chain: Vec<Uuid>` of tables on the current path, and a `budget: usize` counting every resolved
draw across the request):

- `budget` ≥ `MAX_DRAWS_PER_REQUEST = 64` ⇒ `TooMany`; `depth > MAX_DRAW_DEPTH = 8` ⇒ `TooDeep`;
  `chain` contains `table_id` ⇒ `Cycle` (fail closed; a self-referencing table never draws).
- Load: `repo.get_document(table_id)`; absent, `doc_type != "table"`, or
  `world_of(&doc) != Some(world_id)` ⇒ `NotFound` (collapses to the generic wording below).
- **Authz:** `resolve_access_world(ctx.user_id, ctx.world_role, &doc,
  &world_defaults.grants_for("table"), effective_owner(&doc, None))` must hold `cap::READ` —
  the `combat::combatant_access` shape, on EVERY table in the chain (§11 T4).
- Deserialize `TableEngine` strictly (`serde_json::from_value`; failure ⇒ `Data` — never
  `engine_of`'s silent default). `rows.is_empty()` ⇒ `EmptyTable`.
- Roll: notation = `format!("1d{}", Σ weight)` for `Weighted`, the stored `notation` for
  `Formula`; `chat::rolls::execute_roll(&notation, TABLE_PARSE_CONTEXT, None)` — the one
  untrusted-notation path, no host, so a reference is `unknown-ref` (`Roll(RollError)`).
  `TABLE_PARSE_CONTEXT = ParseContext { mode: ModeKind::Total, direction: Direction::HighWins }`
  is fixed and channel-independent (§11 T3).
- Row: `Weighted` → the first row whose cumulative weight ≥ `outcome.total`; `Formula` → the row
  whose `range` contains `outcome.total`, else `row: None`.
- Entries → `DrawnRow.content` / `nested`: `Text` → `chat::sanitize(text, &policy).segments`;
  `Doc` → `Segment::DocLink`; `Image` → `repo.get_asset(asset_id)` present with `world_id ==
  world` ⇒ `Segment::Image`, else `MissingAsset` (validation-class — the GM fixes the table);
  `Draw { table_id, count }` → `count` recursive calls at `depth + 1` with `table_id` pushed on
  `chain` (popped after).
- Returns `TableDrawSegment { …, spec: Some(spec), raw: Some(raw), row }`.

**`DrawTableError`** with a `[sec]`-classified `Display` (the `SendMessageError` rule):
`Forbidden`/`NotFound`/`Data(DataError)` collapse to one generic string; validation-class
variants say why — `RateLimited`, `UnknownChannel`, `UnknownRecipient`, `ActorNotSpeakable`
(generic, mirrors send), `TooMany`, `TooDeep`, `Cycle`, `EmptyTable`, `MissingAsset`,
`Roll(RollError)`. `From<SendMessageError>` maps the shared gates. A no-debug-artifacts test
iterates every variant (the `RollError` precedent).

**Recalc:** `handle_recalc_roll` is unchanged; it searches `RollEmbed` only, so a `TableDraw`'s
`roll_id` is `RollNotFound`. Documented on `TableDrawSegment.roll_id`.

**Edit/delete:** a draw message is `kind: Roll` ⇒ `handle_edit_message` refuses
(`RollImmutable`); delete tombstones as usual.

### 3.3 The shared body composer — `chat::body`

`handle_send_message`'s chunk→segment loop is extracted into `chat::body` so chat, notes, and
table rows compose bodies through ONE function each:

- `compose_static(body: &str, policy: &ChatContentPolicy, max_spans: usize) ->
  Result<Vec<Segment>, RollError>` — synchronous, no I/O: `scan_body_capped(body, max_spans)`;
  `Text` → `sanitize(..).segments`; `Inline(formula)` → `validate_formula(formula,
  ParseContext::default())` then `Segment::RollButton { formula, label: None }` (a static body
  never executes a roll — an inline span becomes a button; §11 N2); `Button` → `RollButton`;
  `DocLink` → `DocLink`; `Image { asset_id, alt }` → `Segment::Image` (no existence check —
  `serve` is membership-gated, so a foreign id renders the placeholder and leaks nothing).
- `compose_message(...)` — the async chat path as it exists today (executes `Inline` against the
  send's host; validates `Image` spans in-world, §3.4), now the only other caller of the shared
  per-chunk arms. `handle_edit_message` also calls it with `ScanMode::NoExecute` (§3.4).
- `scan_body` keeps its signature (`MAX_INLINE_ROLLS`); `scan_body_capped(body, cap)` is the
  parameterized core it delegates to.

### 3.4 Chat media

**`[[asset:<uuid>|<alt>]]` span** (`alt` optional, trimmed): `scan_body`'s `parse_doc_link`
becomes `parse_ref_span` and gains the `asset:` prefix → `BodyChunk::Image { asset_id, alt:
Option<&str> }`. In `compose_message`: `policy.images()` false ⇒ `RollError::ImagesDisabled`
(player-presentable "Images are disabled in this world's chat."); `repo.get_asset(id)` absent or
`asset.world_id != room.world_id` ⇒ `RollError::UnknownAsset` ("That image is not available in
this world."). Both are validation-class and surface as the whispered System notice like every
roll error (one message per attempt — the flood budget stays 1:1).

**External images** (`sanitize` returns `Sanitized { segments, image_urls: Vec<String> }`):
- `images` off: `rm_tags("img")` (as today) and markdown image events (`Tag::Image`) downgrade to
  their alt text; `image_urls` empty.
- `images` on: markdown `![alt](url)` is collected from the cmark `Tag::Image` event (the event
  is replaced by its alt text — the `<img>` never enters ammonia); html-mode `<img src>` is
  collected by the `attribute_filter` (which returns `None` for `src`), and a post-clean
  `strip_img_tags` removes the now-srcless `<img>` elements from ammonia's normalized output (safe:
  ammonia has already escaped every literal `<` in text, so the only `<img` runs are real elements;
  removing an element from sanitized output cannot create markup). **Invariant kept: ammonia is
  crossed exactly once, and an `<img>` with a `src` never survives `sanitize`.** The lexical
  extension allowlist is deleted with the hotlink it guarded.
- `image_urls` (deduped, first `MAX_INLINE_IMAGES = 4`, `http`/`https` only after
  `link_preview::validate_url`) feed `PendingEnrichment::InlineImage { image_url, alt }` returned
  from `link_preview::enrich` alongside its preview jobs — gated on `policy.images()`, not on
  `previews_enabled()`.
- `post_publish::resolve_inline_image`: under `with_preview_url_lock(image_url)`, check the
  persisted `link_preview_cache` row for `image_asset_id` (a URL any message already imaged is
  never re-fetched); on a miss, `fetch_image_bytes` with `MAX_INLINE_IMAGE_BYTES = 4 MiB` (a new
  constant beside `MAX_IMAGE_BYTES`; `guarded_get` takes the cap as a parameter already) →
  `create_asset_from_bytes` with `Provenance::ChatImage`, `created_by: None` → cache the id.
  `ResolvedEnrichment::NewImageSegment(Segment::Image { asset_id, alt })` appends at the end of
  `content` in the single OCC'd republish `run_pending_enrichments` already performs. Failure
  degrades silently (negative-cached) like every preview.
- `PreviewRateLimiter` (`PREVIEW_FETCH_PER_MIN`) counts inline-image fetches too (same
  per-user distinct-URL budget, same `rate.check` site).

**Edits keep structured references.** `handle_edit_message` now composes through
`compose_message` in `ScanMode::NoExecute`: `Text`/`Button`/`DocLink`/`Image` chunks are honored
(an image or doc-link survives an edit instead of collapsing to literal `[[…]]` text), an
`Inline` chunk is `SendMessageError::RollImmutable` (no editing INTO a roll — the existing rule,
now covering inline spans), and the stored-content `RollEmbed`/`RollButton` immutability check is
unchanged. Whisper edits still skip `parse_command` (their body is literal; the scan still runs).

### 3.5 Redaction, broadcast, search

- Messages: unchanged path. `roll_property_overrides` (2.3) computed at Create and on recalc.
- Tables/notes: ordinary documents — `filter_properties`/`filter_command`/`index_content` need
  no change. A player without READ on a table never sees it, cannot draw from it (§3.2 authz),
  and a GM's draw whose `audience` admits the player shows the drawn row, not the table.
- `index_content` already sweeps `name`, `source`, `body[].sanitized_html`, row labels/texts;
  tokenization quality is M21's.

## 4. Client seams

**`@shadowcat/core`**
- `chat-docs.ts`: `ChatSegment` gains `image` and `table_draw` (recursive schema via
  `z.lazy`); `isKnownSegment`/`UnknownSegmentSchema` refuse both new kinds in the fallback
  (the fail-closed rule).
- New `table-docs.ts`: `TABLE_DOC_TYPE`, `buildTableDoc(worldId, name, engine, id?)`, re-exported
  ts-rs `TableEngine`/`DrawRule`/`TableRow`/`RowRange`/`TableEntry`, `DrawTableOptions`.
- New `note-docs.ts`: `NOTE_DOC_TYPE`, `buildNoteDoc(worldId, name, source, opts?: { parentId?,
  sort?, id? })` (private-by-default permissions, §2.2), `parseNoteBody(doc) -> ChatSegment[] |
  null` (fail-closed over `ChatSegmentSchema`), `NoteEngine` re-export.
- `ws-client.ts`: `drawTable(opts: DrawTableOptions): Promise<void>` on the `chatPending`
  correlation (rejects on `chat_error`, resolves after `CHAT_ERROR_WINDOW_MS`).
- `index.ts` exports for all of the above.

**`@shadowcat/ui-kit`**
- `ChatApi.drawTable(opts)` beside `send`/`edit`/`delete`/`recalc`; wired in the shell's
  `Table` like the others.
- **`SegmentList.svelte` + `RollTooltip.svelte` move from `module-chat-card` into ui-kit.** The
  client's single `{@html}` sink becomes ui-kit's `SegmentList` (renders only an
  `isKnownSegment`-narrowed `html` segment's `sanitized_html`); `MessageCard` renders through it.
  This is the seam M20's note sheet and table sheet need — a module cannot import
  `module-chat-card`, and a second renderer would fork the sink (§11 C4). Props: the segment
  array plus the message context the current loop reads (`messageId`, `channel`) — `ctx` via the
  ui-kit context getter. Recursion for `table_draw.row.content`/`nested` is a self-import.

**`module-chat-card`** — `MessageCard` keeps chrome (header, actions, recalc menu, block-roll
form) and delegates segments to `SegmentList`. New renderers inside `SegmentList`:
- `image`: `<a href={ctx.assets.url(id)} target="_blank" rel="noopener noreferrer"><img
  src={ctx.assets.url(id, "preview")} alt loading="lazy"></a>`, `max-width: 100%`,
  touch-sized; a deleted asset resolves to the resolver's placeholder.
- `table_draw`: header (table name — a presence-gated `ctx.openDocument({docId: table_id})`
  link like `doc_link`), the roll as the existing inline chip + `RollTooltip`, the row label,
  `row.content` through `SegmentList`, `row.nested` indented and recursive, "no matching row"
  for `row: null`.
- `doc_link` whose target resolves in `ctx.documents` to `doc_type === "table"` gains a
  **Draw** button → `ctx.chat.drawTable({ tableId, channel: sys.channel })` (§11 T7: no new
  segment or slash command).

**`module-chat-composer`** — an "Insert image" button beside `@doc`: `await ctx.pickAsset({
kind: "image" })` (M15b's seam; `null` = cancelled) → inserts `[[asset:<id>|<name>]]` at the
cursor with the same `[`/`]`/`|` stripping the doc-link insert applies. Hidden when the world's
`chat-settings.images` is off (read like the other policy toggles). **Prerequisite:** M15b merged
to `main` before M19a starts.

**`module-game-settings`** — the chat `images` toggle's copy says what it now means ("Images:
asset links and server-fetched copies of linked images; never hotlinked").

**Sheets (M20)** register `shadowcat.sheet:table` / `shadowcat.sheet:note` providers and reuse
`SegmentList` for the note body / row preview; nothing in M19 blocks them. The generic fallback
sheet shows a table/note's `system` band only, as it does for every engine doc_type today.

## 5. Wire types

| Surface | Addition |
|---|---|
| `ClientMsg` | `DrawTable { request_id, table_id, channel, count, actor_owner, audience }` (ts-rs) |
| `ServerMsg` | none (`ChatError` reused) |
| engine (ts-rs) | `TableEngine`, `DrawRule`, `TableRow`, `RowRange`, `TableEntry`, `NoteEngine` |
| `Segment` (serde + Zod) | `Image`, `TableDraw(TableDrawSegment)`, `DrawnRow` |
| `Asset` | `Provenance::ChatImage` → derived tag `chat-image` |
| SQL | **none** — `0001_init.sql` untouched; `link_preview_cache.image_asset_id` reused for inline images |
| `docs/site/protocol.md` | `draw_table` row in the client→server catalog; the `send_message` row notes `[[asset:…]]`; a short "Rollable tables" paragraph after the dice-reference paragraph |

## 6. Security posture

- **SSRF:** every outbound fetch M19 adds goes through `link_preview::guarded_get`
  (`validate_url` literal-IP arm + `GuardedResolver` + per-hop re-validation + byte cap +
  wall-clock timeout) via `fetch_image_bytes`; YouTube thumbnails already do. The server never
  fetches for a note or a table row (no outbound path exists on the document-write path).
- **No hotlinks:** `Segment::Image` carries only an asset id; `AssetResolver.url` builds a
  same-origin URL; `sanitize` never emits an `<img>` with a `src`. The tracking-pixel gap named in
  `ammonia_for`'s comment is closed, not narrowed.
- **Sanitizer boundary:** notes cross `ammonia` exactly once in `sanitize` at ingress; `body` is
  server-derived and the client-supplied `body` is discarded; the client renders `body` only
  through ui-kit's `SegmentList` sink; `source` is rendered as text everywhere.
- **Tables:** reference-free formulas (no cross-actor data reaches a table roll); caps on rows,
  depth, breadth, and total draws; READ on every table in a chain; `spec`/`raw` GM-only;
  flood-budgeted; `[sec]`-classified errors (existence never disclosed).
- **Assets:** chat ingest pins `[[asset:]]` ids to the sending world; `serve` remains
  membership-gated regardless.
- **Ingress guards intact:** `ops_target_message` still refuses client-authored message ops; a
  `TableDraw` message exists only through `handle_draw_table` → `build_message_doc`.

## 7. Tests (server + client unit)

Server (`cargo test`, sibling files):
- `data::engine::table::tests` — `validate` accepts/rejects every rule in §2.1 (weight 0,
  overlapping ranges, `Weighted` with a range, `Formula` with a reference, SuccessCount notation,
  sum > `MAX_DIE_SIDES`, caps); ts-rs shape via the existing bindings test.
- `data::engine::note::tests` — `body` derived on create and on `/engine/source` Update;
  client-supplied `body` discarded; `[[roll:1d6|x]]`/`[[1d6]]` → `RollButton`; `[[doc:…|x]]` →
  `DocLink`; `[[asset:…|x]]` → `Image`; malformed span ⇒ `BadEngine`; source cap;
  `check_note_parent` (parent must be a note in scope; Create + Move), containment.
- `tables::tests` — weighted selection over a seeded rng (`execute_roll_with_seed` seam:
  `draw_table_with_seed`), cumulative boundary rows, formula range hit/miss (`row: None`),
  nested draws with `count`, cycle ⇒ `Cycle`, depth/total caps, READ refusal on a nested
  table, `MissingAsset`, whisper/gm-only audiences, GM-only `spec`/`raw` at every nesting via
  `roll_property_overrides`, `RollNotFound` on recalc of a draw, `Display` no-debug-artifacts.
- `chat::tests` — `[[asset:]]` ingest (in-world ok / foreign ⇒ notice / images-off ⇒ notice);
  `sanitize` returns `image_urls` and never an `<img src>` (markdown + html modes, on/off);
  `enrich` queues `InlineImage` gated on `images`; `resolve_inline_image` with the loopback
  test client creates a `ChatImage` asset and republishes an `Image` segment; a blocked-IP image
  URL is never fetched; cache hit skips the fetch; edit keeps `DocLink`/`Image`, refuses inline;
  `roll_property_overrides` on a `TableDraw` message.
- YouTube acceptance (existing, cited as M19's evidence): `chat::oembed`'s `match_provider`
  host tests and the `OEmbedSegment`-has-no-`html` structural test; `post_publish`'s
  `resolve_oembed` thumbnail test.

Client (`pnpm -r test`): `chat-docs.test.ts` (new segment schemas, recursive `table_draw`,
fallback refusal), `table-docs.test.ts`/`note-docs.test.ts` (builders, `parseNoteBody`
fail-closed), `ws-client.test.ts` (`drawTable` correlation), ui-kit `SegmentList.test.ts` (the
moved renderer tests + image + table_draw + Draw button gating), composer `Insert image`, card
delegation. Both e2e suites in §8.

## 8. E2E coverage

WS-level suite (`src/client/core/src/e2e/*.e2e.test.ts`, spawns the real server; run by
`pnpm --filter @shadowcat/core test:e2e`):
- **M19a `chat-image.e2e.test.ts`** — GM uploads a PNG over REST, sends `[[asset:<id>|map]]`;
  the player's `Event` carries `image` with that `asset_id`; a foreign uuid ⇒ `chat_error`; a
  world whose `chat-settings.images` is off ⇒ `chat_error`.
- **M19b `table-draw.e2e.test.ts`** — GM creates a nested pair of tables via `Intent`
  (`buildTableDoc`), `drawTable` ⇒ GM and player both receive `table_draw` with `row.nested`;
  the player's copy has no `spec`/`raw` at any depth, the GM's does; a player draw on a
  `default: none` table ⇒ `chat_error`; a self-referencing table ⇒ `chat_error`; a `Formula`
  table's `outcome.total` lies inside the drawn row's `range`.
- **M19c `note-body.e2e.test.ts`** — GM creates a note via `Intent` (`buildNoteDoc`) with
  markdown, `[[roll:1d6|Luck]]`, `[[doc:<fixture.doc>|Doc]]`; the echo's `body` holds
  `html`/`roll_button`/`doc_link`; an Update to `/engine/source` re-derives `body`; a malformed
  span ⇒ `reject`; a child note under a non-note parent ⇒ `reject`; the player (no READ) never
  receives the note.

Playwright suite (`src/client/shell/e2e`, `pnpm --filter @shadowcat/shell e2e`):
- **M19a `chat-media.spec.ts`** — upload via the asset browser, composer "Insert image" → pick →
  send → the card shows an `<img>` whose `src` starts with `/api/assets/`.
- Table and note **UI** flows (sheet-driven) are M20's, alongside the sheets that make them
  drivable without bypassing the UI (the Playwright suite's convention).

## 9. Docs, skills, gates

- `docs/site/protocol.md` (§5); `docs/site/modules/chat-card.md` (segment kinds; the `{@html}`
  sink now lives in ui-kit's `SegmentList`), `chat-composer.md` (Insert image), `game-settings.md`
  (images copy); `docs/design/ARCHITECTURE.md` invariant 6 + §6 doc-type list/count (§3.1) and
  the "stable asset identity" bullet gains the chat-image provenance; `docs/TODO.md` unchanged;
  `docs/HISTORY.md` entries at each sub-project close; `docs/PLAN.md` M19 entry.
- Skills (plugin checkout, reviewed skill-update gate + `node
  scripts/check-skill-symbol-refs-cli.mjs` + `pnpm run test:scripts`): `chat` (`Image`,
  `TableDraw`, `chat::body`, `Sanitized`, edit-scan, `roll_property_overrides`, the `{@html}`
  sink's new home), `assets` (`Provenance::ChatImage`), `documents-permissions` (registry
  entries, `check_note_parent`, containment), `dice` (`validate_table_formula`,
  `TABLE_PARSE_CONTEXT`), `client-shell` (`ChatApi.drawTable`, `SegmentList`), `sheets`
  (`SegmentList` as the body renderer for M20 sheets), and a **new
  `shadowcat-codebase-tables-notes` skill** (created in M19b, extended in M19c) with its globs in
  `hooks/codebase-skill-reminder.py`'s `SUBSYSTEMS` map.
- Gates: the full suite in the campaign brief (client + server + both e2e suites + skill gates);
  ts-rs bindings regenerated and committed with every wire/engine change.

## 10. Sub-project split (build order)

| Sub-project | Delivers | Plan |
|---|---|---|
| **M19a · Chat media** | `Segment::Image`, `[[asset:]]` spans, `Sanitized`/no-hotlink sanitizer, `InlineImage` post-publish, `Provenance::ChatImage`, `chat::body` extraction, edit-scan, `SegmentList` move to ui-kit, composer Insert image, YouTube evidence + docs | `docs/superpowers/plans/2026-09-02-m19a-chat-media.md` |
| **M19b · Rollable tables** | `TableEngine`, `tables::handle_draw_table`, `DrawTable` frame, `TableDraw` segment + redaction, core `table-docs`, `drawTable` seam, card rendering + Draw button, new skill | `docs/superpowers/plans/2026-09-02-m19b-rollable-tables.md` |
| **M19c · Notes** | `NoteEngine` + derived body, `compose_static`, `NOTE_CONTENT_POLICY`, note tree, core `note-docs`/`parseNoteBody`, docs | `docs/superpowers/plans/2026-09-02-m19c-notes.md` |

M19a first because M19b's `Image` row entries and M19c's `[[asset:]]` spans consume
`Segment::Image` and `chat::body`, and both later card renderers build on `SegmentList`. Each
sub-project is a branch off `main`, merged before the next starts.

## 11. Decision log

**Common**

| # | Fork | Decision | Alternatives and why they lose |
|---|---|---|---|
| C1 | Sub-project order | media → tables → notes | tables-first would re-touch the composer/card twice; notes-first would build the segment renderer without its recursive consumer |
| C2 | Where tables/notes live | typed `engine` band, registry + `normalize_engine` | opaque `system` (the client-only `item` shape) gives the server no authority to validate rows or derive a note body — invariant 6 puts computation on the server |
| C3 | Reference grammar | reuse `scan_body`'s `[[prefix:…]]` spans (`asset:` added) | a second grammar for notes/tables forks the parser and the composer insert logic |
| C4 | Segment renderer location | move to ui-kit `SegmentList`; one `{@html}` sink | leaving it in `module-chat-card` forces M20's sheets to duplicate the renderer (a second sink = a forked security boundary) |
| C5 | Shared ingest gates | extract `validate_audience`/`validate_actor_owner`/`chat::body` and call them from every producer | copying the attribution/recipient checks into `handle_draw_table` is the forked-decision class |
| C6 | Schema | no SQL change; documents + existing `link_preview_cache` | a `tables`/`notes` table forks the document model the whole redaction/search/resync stack is built on |

**Media**

| # | Fork | Decision | Alternatives |
|---|---|---|---|
| A1 | External images | fetch + asset-ify post-publish, never hotlink | keep the extension allowlist (leaves the tracking-pixel leak the sanitizer's own comment names); block external images entirely (worse UX than a short delay — invariant 11 prefers keeping both) |
| A2 | Image candidates | markdown `![]()` + html `<img src>` only; a bare link to an image stays a link | promoting image-typed hrefs would double-fetch every link and turn the preview scraper into an image scraper |
| A3 | Byte cap | `MAX_INLINE_IMAGE_BYTES = 4 MiB`, separate from `MAX_IMAGE_BYTES` | raising the og:image cap globally enlarges every preview fetch; 256 KiB is too small for real images |
| A4 | Asset span policy gate | `[[asset:]]` obeys the same `images` toggle as external images | an ungated span makes the toggle a lie |
| A5 | Edits | scan on edit in `NoExecute` mode; inline ⇒ `RollImmutable` | leaving edits literal makes an edited message lose its images/doc links (a defect the media feature would create) |
| A6 | YouTube | already delivered; add evidence + docs only | rebuilding a provider path that exists would be churn without a design reason |
| A7 | Provenance | `Provenance::ChatImage`, tag `chat-image` | reusing `LinkPreview` provenance hides the difference in the browser's filters |

**Tables**

| # | Fork | Decision | Alternatives |
|---|---|---|---|
| T1 | Selection model | `DrawRule::Weighted` (rolls `1d<sum>`) **and** `Formula` with ranges | weights-only cannot express classic `2d6` tables; ranges-only forces authors to hand-compute weights |
| T2 | Row content | `TableEntry` enum (Text/Doc/Image/Draw) | a single markdown string with `[[…]]` spans parsed at draw time re-parses on every draw and cannot validate nested-draw counts at ingress |
| T3 | Parse context | fixed Total/HighWins, channel-independent; SuccessCount notation refused at ingress | resolving under the channel's dice-settings could silently flip a table's arithmetic per channel |
| T4 | Nested authz | READ on every table in the chain | drawing through an unreadable table leaks its rows by sampling; the GM draws on the player's behalf when reveal-by-draw is wanted |
| T5 | Row text sanitization | at draw time under the world's chat policy | at authoring time would freeze one policy into the stored table and require a note-style derived field per row |
| T6 | Recalc | table draws are not recalc targets | re-resolving a row against a mutable table (and re-running nested draws) has no honest audit-trail semantics; a GM redraws |
| T7 | Draw affordance in chat | `DocLink` to a table + a client Draw button | a `/table` command needs name lookup (ambiguous); a `TableButton` segment duplicates `DocLink` |
| T8 | Cross-doc refs at ingress | not resolved (draw-time, fail closed) | ingress resolution needs async repo access inside `normalize_engine` and still goes stale on delete — the `DocLink` precedent |
| T9 | Formula hit no range | `row: None`, rendered "no matching row" | refusing the draw hides an authoring hole from the GM |
| T10 | References in formulas | forbidden | resolving against the drawer's speak-as makes a shared table's odds depend on who draws |

**Notes**

| # | Fork | Decision | Alternatives |
|---|---|---|---|
| N1 | Where the body is derived | `normalize_engine("note")` at ingress (sync, deterministic) | a dedicated `WriteNote` frame forks the document write path; client-side rendering violates the server-sanitizer rule |
| N2 | Inline `[[1d6]]` in a note | becomes a `RollButton` | executing at save time would roll dice on every edit |
| N3 | Content policy | fixed `NOTE_CONTENT_POLICY` (markdown, links, assets) | the chat policy is about chat; a world with plain-text chat still wants rich notes |
| N4 | Default visibility | private (`default: None`, author `Owner`) | world-readable by default turns a GM's first journal page into a leak |
| N5 | Player note creation | ordinary `core:create` grant per doc_type | a message-style baseline exemption widens Create for a doc_type that has no ingress guard of its own |
| N6 | Hierarchy | `parent_id` tree of notes (`check_note_parent`), GM-only `Move` | folders as a separate doc_type duplicate `asset_folder`; embedding forks the sheet write-site rules |
| N7 | Images in notes | `[[asset:]]` only; markdown image URLs downgrade to alt text | outbound fetches on the document-write path would need a post-publish pipeline for documents |
| N8 | Body cap | `MAX_NOTE_SOURCE_CHARS = 65_536` under the 256 KiB band cap | relying on the byte cap alone lets rendering expansion refuse a save with an opaque size error |

## 12. Open questions for the user

None. Every fork above resolves under "best long-term shape in keeping with our plans and goals".
