# Per-World Export/Import — Design

**Status:** approved (self-directed design under the standing debt-burndown campaign authority).

**Spec for:** `docs/TODO.md` bucket-C sub-project 3, "Per-world export/import — world-scoped row
subset preserving cross-FK referential integrity + shared asset references."

**Resolves a flagged conflict:** `docs/design/ARCHITECTURE.md`'s Postgres-vs-SQLite comparison
table lists "Bulk import/export (assets + documents)" as Phase 2. The user's explicit bucket-C
authorization ("build ALL of bucket C") is the more specific, more recent instruction and
supersedes the general roadmap table — this ships now. `ARCHITECTURE.md`'s table gets a line
correcting this once the feature merges (Task in the plan).

## 1. Scope

A GM/world-admin can export a single world — every world-scoped row plus the asset bytes its
`assets` rows reference — into one portable bundle file, and import that bundle into a (possibly
different) server to recreate the world. Whole-server backup/restore (`backup::create_backup`/
`restore_backup`) is untouched and stays the whole-server snapshot tool; this is a narrower,
world-scoped sibling.

## 2. Table scope — reuses `delete_world`'s existing FK-closure enumeration

Exported tables, filtered to one `world_id`, are exactly the set `SqliteRepository::delete_world`
already walks (cascade-FK'd tables plus its two explicit no-FK purges), read instead of deleted:
`documents`, `world_events`, `world_members`, `world_invites`, `assets`, `explored_fog` (no FK,
filtered by its own `world_id` column), and the 5 `settings` rows keyed via
`world_settings_keys(world)` (no FK, flat k/v table). `worlds` itself is exported as the bundle's
root record. `documents_fts_public`/`documents_fts_gm` are NOT exported — they're pure derived
index state, regenerated from `documents.json` by the existing indexing path when documents are
imported.

## 3. World identity — resolved design fork: preserve, never remap

**The bundle preserves `world_id` verbatim; import fails cleanly if a world with that id already
exists on the target server.** Rejected alternative: minting a fresh `world_id` on import and
remapping every reference to it. That would require rewriting `documents.parent_id` self-refs
(cheap), `documents.owner_id`'s JSON-body-denormalized twin (cheap, existing pattern — see §4),
asset `storage_key` paths (cheap, mechanical), but ALSO every historical `world_events.command_json`
blob, which embeds arbitrary serialized `Command` payloads that may themselves carry world/document
identifiers baked into their own JSON structure with no single, safe, generic rewrite rule. Risk of
silently corrupting replay semantics for a purely cosmetic id-freshening benefit outweighs the
value — collision is already vanishingly unlikely in normal use (the real use case is migrating a
world to a different server, where the source world's id is not present at all), and a clean reject
on collision is honest, simple, and safe. Duplicating a world onto the SAME server under a new id
is explicitly out of scope; nothing here forecloses building it later as its own feature.

## 4. User-identity resolution — resolved design fork: username-based remap-or-null, reusing the
existing `delete_user` degradation path

Every `owner_id`/`author_id`/`created_by`/`consumed_by` column-level reference to `users(id)` is
exported as a **portable username string** (`users.username` is `UNIQUE`), not a raw id — the
source server's `users` table itself is never exported (global, not world-scoped). On import, each
username is looked up against the TARGET server's `users` table:

- Found → the column (and, for `documents.owner_id` specifically, the JSON-body-denormalized twin
  — the same double-update `delete_user` already performs to keep both in lockstep) is set to the
  target user's id.
- Not found → the column is set `NULL`, exactly the `ON DELETE SET NULL` degradation every one of
  these columns is already designed around. No new "unknown owner" state is invented — this reuses
  the one that already exists.

**`world_events.command_json`'s JSON-embedded historical references are imported completely
verbatim, with no rewrite.** These are historical audit/replay payloads, not live state — Phase 1b's
replay-redaction machinery (`mirror_current_snapshot`, `filter_command`) already tolerates an actor
reference that fails to resolve against current documents (a deleted/unknown actor renders as
"unknown" rather than erroring), so an unresolved cross-server username embedded in a historical
event blob degrades the same way an already-deleted local actor does today. Rewriting arbitrary
embedded `Command` JSON to remap ids is out of scope and unnecessary given this existing tolerance.

## 5. Asset bytes

Exported verbatim as files inside the bundle under `assets/<asset_id>` (world-id prefix dropped
inside the bundle since §3 guarantees the world id is unchanged on import — it's re-added by
computing the standard `storage_key` at extraction time). No content-hash dedup exists in this
codebase (confirmed by research) so no cross-world sharing logic is needed — "shared asset
references" in the TODO's own wording means only "the export must bundle the bytes the world's
`assets` rows point at," which this satisfies directly.

## 6. Bundle format

A single `.tar` file (uncompressed; `flate2`/gzip is a plan-time implementation choice, not a
design constraint) containing:

- `manifest.json` — `{ schema_version: u32, world_id: Uuid, world_name: String, exported_at:
  DateTime<Utc>, row_counts: BTreeMap<String, usize> }`. `row_counts` lets import sanity-check it
  extracted everything the manifest promised before committing the transaction.
- `rows/<table>.jsonl` — one JSON object per row, per exported table, username-substituted per §4
  at export time (not deferred to import) so the bundle is self-contained and inspectable.
- `assets/<asset_id>` — raw asset bytes, one file per `assets` row.

## 7. HTTP surface & authorization

- `POST /worlds/{id}/export` — **world-scoped GM only** (the same authority `delete_world`/world
  settings already require), streams the `.tar` bundle as the response body. Mirrors the existing
  `require_gm`-gated asset routes' authorization shape (§3 of the assets research) rather than
  inventing a new tier.
- `POST /worlds/import` — **server-admin only** (creating a new top-level `worlds` row is a
  server-wide action, matching how world CREATION is already gated — this import endpoint is
  authorized identically to that existing check, never independently), multipart file upload,
  streamed to a temp file first (same file-before-DB-row discipline `commit_db_row_before_swapping_
  file` already establishes for asset uploads), then one transaction: reject on world-id collision
  (§3), insert `worlds` + every table in §2's order (respecting FK dependency order: `worlds` →
  `documents`/`world_events`/`world_members`/`world_invites`/`assets` → `explored_fog`/`settings`
  rows), copy the temp-extracted asset files into `Config::assets_path()/{world_id}/`, commit.
  On any row-insert failure mid-transaction, the whole import rolls back — no partial world.

## 8. Testing

- Round-trip test: create a world with representative rows in every exported table (including at
  least one `documents.owner_id` and one `world_events.author_id`, one `explored_fog` row, one
  keyed `settings` row, one asset), export, import into a fresh in-memory-SQLite test repository,
  assert every row matches (modulo username-remapped owner columns).
- Username-resolution test: export with an owner username that does NOT exist on the (test) target
  — assert the imported row's owner column and JSON-body-denormalized twin both land `NULL`, never
  a stale/wrong id.
- Collision test: importing a bundle whose `world_id` already exists on the target is rejected
  before any row is written (transaction never opens, or opens and rolls back — assert zero
  partial state either way).
- Asset round-trip: exported asset bytes are byte-identical after import, and the imported asset
  row's `storage_key` matches the standard `{world_id}/{asset_id}` scheme.
- `world_events.command_json` verbatim-preservation test: a blob containing an embedded reference
  that cannot resolve on the target still imports successfully and byte-matches the source blob.

## 9. Documentation

- `docs/design/ARCHITECTURE.md`'s Postgres-vs-SQLite table row for "Bulk import/export (assets +
  documents)" is corrected to reflect that per-world export/import shipped ahead of Phase 2 by
  explicit user authorization, with whole-server backup/restore remaining the separate Phase 2
  concern the row originally meant.
- `shadowcat-codebase-server-ops` skill gains the new export/import endpoints, the username-remap
  rule, and the world-id-collision-reject rule as a Gotcha (the backup/restore skill section's
  existing "per-world granular export/import is explicitly OUT of scope" sentence is now false and
  must be corrected in the same commit, not left contradicting the shipped feature).

## 10. Non-goals

- No world duplication on the same server (§3).
- No selective/partial export (whole world or nothing).
- No cross-schema-version import compatibility beyond a manifest `schema_version` mismatch check
  that refuses cleanly (the "no data migrations pre-customers" gotcha applies identically here: no
  upgrade-path machinery is built for a bundle exported under an older schema).
