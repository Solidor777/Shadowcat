# M15 — Asset pipeline + browser — Design

Status: approved design, pending implementation plans (M15a, M15b).

## Goal

Replace the M8b raw upload path with a real pipeline — chunked resumable upload, image
conversion with derivatives, explicit + derived tags, folders — and ship a GM asset browser over
it, while preserving the stable-UUID asset identity every existing reference relies on.

## Decisions (settled in brainstorm)

| Question | Decision |
|---|---|
| Who mutates assets | GM only, unchanged. Players never open the browser; `serve` stays membership-gated so player clients still fetch art by UUID. |
| Conversion policy | Convert-on-upload to WebP; **originals retained by default**. Discarding originals is a server-level option to save disk. |
| Where the retain switch lives | `Config.retain_originals: bool` (CLI flag > `SHADOWCAT_*` env > TOML > default `true`). Host-owned disk policy, like the upload limits. |
| Chunked upload | Resumable across reconnects only: in-memory sessions + staging file, swept on idle. Not persisted across reloads/restarts (layerable later without changing the wire protocol). |
| Derived tags | Kind/format, folder segments, dimension classes, provenance — all four, recomputed on every commit/rename/move/reconvert. |
| Directories | Folder **entities**, represented as `asset_folder` engine Documents (ride the document stream, permissions, resync, export, search). |
| Search | Server-side query endpoint; name substring/tag/folder in SQL, regex via the Rust `regex` crate with size limits. FTS integration deferred to M21 (noted in `PLAN.md`). |
| Sequencing | Layered: **M15a** pipeline (server + client core + existing callers adapted) → **M15b** browser module. |

Excluded: audio transcode (Phase 3); animated-WebP encoding (Phase 3, with audio); persistent
upload sessions; per-user asset areas; per-world retain policy.

## §1 — Data model (server)

`Asset` (`data::asset`) gains:

- `folder_id: Option<Uuid>` — an `asset_folder` document id; `None` = world root.
- `tags: Vec<String>` — explicit, GM-set.
- `derived_tags: Vec<String>` — never client-writable; recomputed on every commit, rename, move,
  and reconvert. Sources: kind/format (`image`, `webp`, `svg`, `gif-animated`, later `audio`)
  from `content_type`; folder segments (every ancestor folder name); dimension classes
  (`square`, `large` above a fixed pixel threshold, `transparent` when the source has alpha);
  provenance (`uploaded`, `link-preview`, later `module:<id>`).
- `width`, `height: Option<u32>`, `has_alpha: bool`, `animated: bool`.
- `original_content_type: String`, `original_byte_size: i64` (what arrived) vs
  `content_type`/`byte_size` (the canonical served file); `original_retained: bool`.

Tags persist in a child table `asset_tags(asset_id, tag, derived)` for indexed lookup; the
`Asset` struct flattens them. Migration adds the columns/table with `folder_id NULL` and runs a
one-time backfill over existing rows (dimensions read from the canonical file; unreadable files
get the kind tag only).

Storage layout under `<assets_dir>/<world>/`:

| File | Role |
|---|---|
| `<uuid>` | **Canonical served file.** `serve`, the ETag scheme, `storage_key`, and the world bundle are untouched. |
| `<uuid>.orig` | Retained original (only when `retain_originals` and the upload was converted). Never served by `serve`; reachable via the GM-only `/original` route. |
| `<uuid>.thumb.webp` (≤128px), `<uuid>.preview.webp` (≤512px) | Derivatives, regenerable from the canonical. A missing derivative is regenerated on demand, never a 404. |

`asset_folder` engine doc type (`data::engine`): `{ name, parent: Option<Uuid>, sort: i64 }`,
world-scoped, GM-writable through the ordinary document path; folder tags are the document's
explicit tags. Validation (`data::validation`) rejects parent cycles and cross-world parents.
Deleting a folder is a document delete; the server's delete hook reparents its assets and child
folders to the deleted folder's parent — bytes are never cascaded.

`version` semantics unchanged: bumps on replace **and** on reconvert, since both change the
served bytes. `Config.retain_originals: bool`, default `true`.

## §2 — Upload pipeline (server)

**Ingress.** Two paths, one commit tail.

- `POST /api/assets` (existing single-shot multipart) stays for bodies ≤ one chunk. `AssetPicker`
  and the chat link-preview path (`chat::post_publish` → `create_asset_from_bytes`) keep using
  it unchanged.
- Chunked:
  - `POST /api/assets/uploads` `{name, content_type, byte_size, folder_id?, tags?}` →
    `{upload_id, chunk_size}`.
  - `PUT /api/assets/uploads/{id}/{offset}` appends one chunk; out-of-order or overlapping
    offsets are rejected so a retried lost chunk is idempotent.
  - `POST /api/assets/uploads/{id}/complete` finalizes; `DELETE /api/assets/uploads/{id}` aborts.
  - Sessions live in `AppState.uploads: Mutex<HashMap<Uuid, UploadSession>>`, bound to the
    creating user + world, backed by a `<uuid>.tmp` staging file, swept after 30 min idle. Size
    cap and rate limit are checked at session create (GM tier — the only reachable tier); the
    rate slot is refunded on abort.

**Processing.** `complete` and the single-shot route hand the staged file to `asset::process`
on `spawn_blocking` (`image` decode is CPU-bound):

1. Sniff the real type (`detect_image_type`; the client's claim is never trusted).
2. Decode; record dimensions, alpha, animation.
3. Encode the canonical WebP — lossless when the source has alpha or is PNG/GIF/BMP-class;
   lossy at a fixed quality (~85) for JPEG-class sources — plus the two derivatives.
4. **Pass-through** (canonical = original bytes, no `.orig`): SVG, animated sources, anything
   `image` cannot decode, any non-image type. Conversion failures also fall back to
   pass-through with the reason in the response; an upload is never rejected for a conversion
   failure.

**Commit** stays file-first-then-row through `commit_staged_asset`, extended to accept the
derived metadata; the `write_barrier` read permit wraps the rename set + row insert exactly as
today. Derived tags are computed in the same transaction that inserts the tag rows.

**Replace** reruns processing and bumps `version`. **Reconvert** (`POST
/api/assets/{uuid}/reconvert`, GM) reruns from `.orig` when retained (404 otherwise) — the route a
quality-policy change or a future format uses.

**Broadcast.** `ServerMsg::AssetChanged` keeps `Replaced`/`Deleted` and gains `Created` and
`Moved` (folder/name/tag changes; version unchanged) so open browsers update without a listing
refetch. Still out-of-band via `broadcast_aux`; `AssetResolver.reconcile` remains the repair
path.

## §3 — Query, mutation routes, client core seam

**Query.** `GET /api/assets` filters: `folder` (uuid | `root` | omitted = whole world),
`recursive`, `tags` (comma list, all-of), `kind` (`image` | `other`), `name` (case-insensitive
substring), `name_regex` (Rust `regex`; pattern ≤256 bytes; `RegexBuilder::size_limit` set;
compiled once per request), `sort` (`name` | `created` | `size`), `limit` + `cursor` keyset
pagination on `(sort_key, id)`. Substring/tag/folder filters are SQL; the regex is applied to
the SQL-filtered stream so it never touches more rows than the other filters allow. The
response is `Asset[]` with the new fields — one wire type; existing callers (`listAssets`,
`AssetPicker`, the current panel) receive the richer rows with no filters.

**Derivatives.** `GET /api/assets/{uuid}?variant=thumb|preview` — same route, same ETag
(`"{id}-{version}"` already keys derivatives, which are regenerated when the canonical's
version changes). `GET /api/assets/{uuid}/original` (GM) serves `.orig` when retained, 404
otherwise.

**Mutation (all GM, via `require_gm`).** `PATCH /api/assets/{uuid}` `{name?, folder_id?,
tags?}` — one route for rename/move/tag; validates the folder exists in this world, recomputes
derived tags, broadcasts `Moved`. `POST /api/assets/bulk` `{ids, folder_id?, add_tags?,
remove_tags?}` for multi-select, one transaction. Folder CRUD is document CRUD — no asset
routes for folders.

**Client core** (`src/client/core`): `asset-rest.ts` gains `queryAssets(world, filters,
cursor)`, `patchAsset`, `bulkPatch`, `startChunkedUpload` (chunk loop, per-chunk retry,
progress callback, abort). `AssetResolver` learns `Created`/`Moved` (no version change —
invalidates listings, not URLs) and exposes `url(uuid, variant?)`. The Zod wire mirror is
extended to the new `Asset` shape.

## §4 — Browser module (M15b, `@shadowcat/module-asset-browser`)

Replaces `@shadowcat/module-assets`: that package and its `Assets` panel contribution are
retired, not kept alongside. One panel contribution, following the panel + settings-editor
conventions M14d establishes.

Layout: folder tree (left; the `asset_folder` documents from the store, reactive,
drag-to-reparent) · filter bar (name/regex toggle, tag chips, kind, sort) · virtualized
thumbnail grid using `variant=thumb`. Selection shows a preview pane (`variant=preview`,
metadata, tag editor, "download original" when retained, reconvert). Multi-select → bulk
move/tag/delete. Uploads: drop-zone on the grid or a folder node; a queue with per-file
progress driven by `startChunkedUpload`, single-shot for files ≤ one chunk. Mobile: tree
collapses to a drawer, grid reflows to two columns, touch targets ≥44px.

`AssetPicker` gains a "browse…" affordance that opens the browser in pick mode and returns
the chosen uuid via `AppContext.assets`.

## §5 — Integrity, export, testing

- **World bundle** carries `.orig` and derivatives beside the canonical under
  `assets/<id>[.suffix]`; import tolerates their absence (derivatives regenerate on demand; a
  missing `.orig` sets `original_retained = false`). `asset_folder` docs export as ordinary
  documents.
- **Delete** removes canonical + `.orig` + derivatives under one barrier permit; world delete's
  directory sweep already covers every suffix.
- **Backup** unchanged — the write barrier already excludes every asset file-op.
- **Tests.** Server: `asset::process` on fixture images (PNG-alpha → lossless, JPEG → lossy,
  SVG/GIF → pass-through, corrupt → pass-through with reason); chunk-session
  ordering/idempotence/sweep; regex size-limit rejection; PATCH authz (non-GM member → 403);
  folder-delete reparenting; derived-tag recomputation on move. Client: `startChunkedUpload`
  retry/abort under mocked fetch; browser panel under vitest (filters → query params, pick
  mode); one Playwright e2e uploading a >1-chunk file and finding it by tag. `pnpm -r test`
  throughout — the shared wire type changes.

## Re-review after M14c / M14d

M14c and M14d are in flight while this is designed. Before each M15 plan is written, re-read:

1. **`AppContext`** — M14c adds `AppContext.combat`; M15b adds `AppContext.assets`. Whichever
   lands second re-reads the other's shape before wiring.
2. **Panel-module and settings-editor conventions** — M14d's tracker module and combat
   settings editors set the pattern M15b's browser panel follows rather than inventing one.
3. **`AssetPicker` consumers** — `ActorsPanel`, `ToolRail`; check after M14d whether the tracker
   also embeds a picker, so pick mode covers it.
4. **`HISTORY.md` / skill pointers** — `shadowcat-codebase-assets` and any `AppContext`
   description in `shadowcat-codebase-client-shell` are updated at M15a/M15b close, after the
   M14c/d edits to the same skills.

## Plan split

- **M15a — pipeline**: §1, §2, §3, §5 (server + client core + existing callers adapted so the
  current panel and `AssetPicker` keep working on the new wire type).
- **M15b — browser**: §4, plus `AppContext.assets` and the `AssetPicker` browse affordance.
  Touches no server code.
