# M8b — Assets: upload, serving, identity, replace/delete

Sub-milestone of **M8 (ECS + scene rendering)**. Parent design:
`docs/superpowers/specs/2026-06-19-m8-ecs-scene-rendering-design.md` §7 (assets) and
§8 (decomposition). This spec resolves the four M8b open items from parent §11 and
details the implementation surface. The parent's architecture decisions are
pre-approved and not re-litigated here.

## 1. Goal

A world-scoped asset surface: GM/owner-gated image upload, UUID-identified serving
with revalidation, in-place byte replace and delete, and a live-update broadcast so
holders re-resolve. Scene `system.background` and Token `system.image` reference an
asset **UUID**, never a path — identity is stable across rename/move/replace. The
server stays structural-only; engine fields live in the opaque `system` body.

## 2. Resolved open items (parent §11)

1. **`AssetChanged` is out-of-band.** A world-scoped broadcast frame, NOT an entry in
   the per-world event log; it allocates **no world seq**. This matches the parent's
   undo-exemption (#8): replace/delete are filesystem+metadata operations, not
   document/ECS mutations. The durable source of truth is the asset record's
   `version`, so reconnect/resync re-reads current metadata regardless of any missed
   frame.
2. **ETag source is `assets.version`.** `GET /assets/{uuid}` returns
   `ETag: "<uuid>-<version>"`; a conditional request whose `If-None-Match` matches
   gets `304`. Bytes are mutable per UUID, so caching is revalidation-based, not
   immutable. Content-hash dedup remains Phase 2.
3. **On-disk layout: `<assets_dir>/<world_id>/<uuid>`** — bytes only, no extension;
   `content_type` lives in the table. `storage_key = "<world_id>/<uuid>"`. World
   sharding keeps per-world directories bounded and aligns with future per-world
   export/delete. All paths built via `std::path` (cross-platform, #10).
4. **Tiered, fully configurable upload limits.** Two role tiers, all four values
   independently overridable via figment; GM tier defaults to **2× the effective
   regular value**:
   - `upload_max_bytes` — regular cap, default **25 MB**.
   - `upload_rate_per_min` — regular rate, default **20**.
   - `upload_max_bytes_gm` — optional; default `2 × effective upload_max_bytes` (50 MB).
   - `upload_rate_per_min_gm` — optional; default `2 × effective upload_rate_per_min` (40).

   So lowering `upload_max_bytes` to 10 MB and setting nothing else yields a 20 MB GM
   cap; any value can still be pinned explicitly. Uploads **stream to disk** (bounded
   memory) as an implementation invariant — the body is never buffered whole.

## 3. Data model

New migration `0006_assets.sql`. Dedicated `assets` table (not SQLite blobs — large
images stream better off disk; not `documents` — the intent/rollback machinery is
overkill for an immutable file pointer):

```
assets(
  id            TEXT PRIMARY KEY,   -- uuid, stable identity from first upload
  world_id      TEXT NOT NULL,      -- FK worlds(id); read-gate + sharding key
  storage_key   TEXT NOT NULL,      -- "<world_id>/<uuid>", relative to assets_dir
  original_name TEXT NOT NULL,      -- display only; identity never depends on it
  content_type  TEXT NOT NULL,      -- validated image/* ; drives serve Content-Type
  byte_size     INTEGER NOT NULL,
  created_by    TEXT NOT NULL,      -- FK users(id)
  created_at    INTEGER NOT NULL,
  version       INTEGER NOT NULL    -- bumped on replace; backs ETag + resync
)
```

Index on `world_id` (list-by-world). Replace mutates `storage_key`/`content_type`/
`byte_size`/`version` in place; the `id` never changes.

## 4. Config

Add to `Config` (beside `db`), with defaults in the `Default` impl and matching
optional CLI flags:
- `assets_dir: String` — default = sibling `assets/` directory resolved from the
  db file's parent via `std::path` (explicit value overrides). Created on boot if
  absent.
- `upload_max_bytes`, `upload_rate_per_min`, `upload_max_bytes_gm` (Option),
  `upload_rate_per_min_gm` (Option) per §2.4.

## 5. Server surface

All routes mount on the existing axum router; auth via the current session
extractor; world authorization via the existing `PermissionContext`.

- **`POST /worlds/{world}/assets`** — multipart upload. GM/owner capability gated.
  Enforces the role-tiered size cap and the role-tiered rate-limit (per-user
  token-bucket).
  Validates content-type **and magic bytes** (images only in M8); rejects mismatch.
  Streams the body to `<assets_dir>/<world_id>/<uuid>`, inserts the record
  (`version = 1`), returns the asset record JSON.
- **`GET /assets/{uuid}`** — streams bytes from disk with the recorded
  `Content-Type`, `Content-Disposition: inline`, and `ETag: "<uuid>-<version>"`.
  Honors `If-None-Match` → `304`. **Read-gated by world membership** (look up the
  asset's `world_id`, require the caller be a member); unguessable UUIDs are
  defense-in-depth, not the gate.
- **`POST /assets/{uuid}/replace`** — GM/owner-gated, magic-byte validated. Writes the
  new bytes, updates `storage_key`/`content_type`/`byte_size`, `version += 1`,
  deletes the old file. Broadcasts `AssetChanged{uuid, op: Replaced}`.
- **`DELETE /assets/{uuid}`** — GM/owner-gated. Removes the file and the record.
  Broadcasts `AssetChanged{uuid, op: Deleted}`.

Replace/delete are **undo-exempt** — they never enter the world event log or consume
a seq.

## 6. Live update — `AssetChanged`

`ServerMsg::AssetChanged { uuid: Uuid, op: AssetOp }` where `AssetOp` is
`Replaced | Deleted`. Exported to the client via the existing ts-rs pipeline and
mirrored in the client Zod wire schema (same pattern M8a used for `SceneDerived`).
Broadcast to the world room out-of-band (no seq). Holders react:
- `Replaced` → re-fetch `GET /assets/{uuid}` (conditional; the new `version` makes the
  ETag miss, so fresh bytes load).
- `Deleted` → drop to the placeholder.

Resync/reconnect needs no replay of these frames: any consumer re-resolving an asset
reads the current `version` and bytes directly.

## 7. Client

- **Resolver** (M8b-1): a UUID→URL helper (`/assets/{uuid}`), a shared placeholder
  for missing/deleted assets, and an `AssetChanged` subscriber that invalidates the
  cached URL/version for the affected UUID so dependent views re-resolve.
- **Minimal asset panel** (M8b-2): an asset-management surface contributed via the M7
  `core-ui` contribution/surface system (a panel on a region surface). Flat,
  this-world-only:
  - **Upload** — file-pick or drag/drop; posts to the upload endpoint; surfaces
    server validation/limit errors.
  - **Grid** — thumbnails of all world assets (served via the resolver).
  - **Select-to-use** — clicking an asset yields its UUID for assignment (scene
    background / token image consumers arrive in M8c/M8d; M8b exposes the selection
    affordance).
  - **Replace / Delete** — per-asset actions calling the respective endpoints.
  - Re-renders on `AssetChanged`.

This panel is what makes file upload **hand-testable in the running app**; without it
the loop is only reachable through headless API tests.

## 8. Decomposition

- **M8b-1 — server + thin resolver** (headless-testable via the M4/M5 test-server):
  migration + `assets` table, config fields, the four endpoints, validation +
  tiered limits, `AssetChanged` broadcast + wire types, client UUID→URL resolver +
  placeholder + invalidation.
- **M8b-2 — minimal asset surface** (client): the upload/grid/select/replace/delete
  panel of §7, mounted on a `core-ui` surface; Playwright smoke over the
  upload→serve→replace→delete loop against the binary.

M8b is independent of M8a (parallelizable); both feed M8c.

## 9. Out of scope (Phase 2)

Format conversion; content-hash dedup; tags / folders / nested organization;
search-over-assets / asset browser beyond the flat per-world grid;
reference-counting / GC; per-asset read visibility finer than world membership;
soft-delete / retention. The UUID identity guarantees none of these break existing
references when added later.

## 10. Testing

**Server (headless, M8b-1):**
- upload → record round-trip (fields, `version = 1`, file on disk at `storage_key`);
- magic-byte / content-type mismatch rejected;
- tiered-limit enforcement: regular over-cap rejected, GM allowed up to the GM cap,
  GM over-cap rejected; rate-limit trips after the configured count;
- `GET` read-gate: non-member denied, member served; `Content-Type` correct;
- ETag/304: matching `If-None-Match` → 304, post-replace ETag changes → 200;
- replace: same UUID, `version` bumped, old file gone, new bytes served,
  `AssetChanged{Replaced}` emitted, **no world seq consumed**;
- delete: file + record gone, `AssetChanged{Deleted}` emitted;
- resync re-reads `version` without relying on a replayed frame.

**Client (M8b-2):** resolver URL/placeholder/invalidation unit tests; Playwright
smoke driving upload → thumbnail appears → replace → delete against the binary
(client built before cargo per the embed-dist ordering invariant).

## 11. References

Parent design §7/§8/§11; M8a `SceneDerived` egress + ts-rs/Zod wire pattern
(reused for `AssetChanged`); M7 `core-ui` contribution/surface system (M8b-2 host);
cross-platform path rule (CLAUDE.md #2, parent #10); embed-dist compile ordering
(client built before the server binary in CI + local).
