---
name: shadowcat-codebase-assets
description: "Use when touching Shadowcat assets: upload/replace/serve, the asset store, ETag/version revalidation, upload rate limits, out-of-band AssetChanged broadcasts, or the assets UI module. Covers src/server/src/data/asset.rs + src/server/src/http/assets.rs + src/modules/assets. Invoke shadowcat-codebase-core first."
---

# Shadowcat — Assets

Orientation for asset upload/replace/serving and the client asset panel.

## Purpose

Assets are uploaded, stored on disk, and served over HTTP with ETag revalidation. Each asset is
referenced by a **stable UUID** from first upload (moving/renaming never breaks links); its
`version` bumps on every replace and backs both the ETag and the resync source of truth. v1 stores
and serves uploads unconverted (the conversion pipeline is deferred).

## Key files & seams

- `src/server/src/data/asset.rs` — `Asset { version, … }`; `version` is bumped on every replace and
  backs the ETag + the resync source of truth.
- `src/server/src/http/assets.rs`:
  - `upload(...)` — streams to disk; `UploadRateLimiter::{check,refund}` enforces tiered per-minute
    limits (configured per role); `detect_image_type`. **The non-GM tier is unreachable from this
    route** — `require_gm` gates it (below), so only the GM tier is ever selected here.
  - `serve(...)` — `GET /api/assets/{uuid}`, membership-gated; ETag = `"{id}-{version}"`;
    `If-None-Match` is an RFC 7232 comma-separated list → 304 if our ETag appears anywhere in it.
  - `replace(...)` — swaps bytes, keeping the stable UUID; broadcasts `AssetChanged`.
- `src/server/src/ws/protocol.rs` — `ServerMsg::AssetChanged { uuid, op: AssetOp }`, broadcast
  **out-of-band** via `Room::broadcast_aux` (not in the per-world event sequence).
- `src/modules/assets/{Assets.svelte,index.ts}` — the client asset panel (upload/list/replace).

## Hard invariants

- **All three mutation routes are GM-ONLY, with no owner exception.** `upload`, `replace`, and
  `delete` each call `require_gm` (`http/routes.rs`), which returns `Forbidden` unless
  `ctx.world_role == WorldRole::Gm`; a server admin reaches GM via `permission_context`
  (`data/sqlite.rs`, `server_role == Admin ⇒ world_role: Gm`, before any membership lookup). There
  is no asset-owner concept and no per-asset permission check — uploading a file does not grant its
  uploader any subsequent authority over it. `serve` is the odd one out: membership-gated, not
  GM-gated. Corrected in the client/core doc sweep, where all three route comments had claimed
  "GM/owner-gated" — treat any surviving "owner" language about asset mutation as stale.
- **`replace` commits the source-of-truth/cache-key row BEFORE swapping the file** (row-first).
  The inverse strands new bytes under a stale ETag/version — a silent 304 of changed content;
  `replace` has prior bytes to preserve and an existing ETag to protect, so the failure that
  matters most is a stale-but-served file, never an orphan row
  [[commit-db-row-before-swapping-file]]. **`upload` (create) inverts this to file-first**: rename
  the staged temp file into place, THEN insert the row. A create has no prior bytes and no
  existing ETag to strand — the failure that matters is an orphan DB row (a `GET` that 500s
  forever, since no bytes were ever written under that id), while an orphan FILE with no row is
  harmless dead disk space. Two-store writes (file + metadata row) without a spanning txn always
  order around whichever failure mode is unrecoverable for that operation — row-first for
  replace, file-first for create.
- **`upload`/`replace`/`delete` all take `AppState.write_barrier.read()` around their
  commit+rename/commit+unlink critical section** (`http/assets.rs`) — never around the earlier
  network-bound multipart stream, which has no timeout (`DefaultBodyLimit::disable()` on these
  routes) and would otherwise let a slow uploader hold the write-preferring
  `tokio::sync::RwLock`'s read side open indefinitely. `POST /api/admin/backup` holds the write
  side across its `VACUUM INTO` + assets copy, so no asset write's row-commit+file-op pair can
  interleave with an in-server backup snapshot (`shadowcat-codebase-server-ops`).
- **ETag == `"{id}-{version}"`**; `version` is the single monotonic cache key. Stable UUID identity
  means a replace keeps the id and only bumps the version, so links survive (ARCHITECTURE §6).
- **Upload limits are tiered + configurable** (GM ≈ 2× regular); uploads stream to disk, not buffered.
- **World deletion removes the whole `<assets_path>/<world_id>/` directory AFTER the row
  transaction commits** (`routes::delete_world` — the delete convention: rows first, files second;
  a crash orphans files on disk, never a live world missing its files), holding the write
  barrier's read side across the commit + `remove_dir_all` pair like every other asset file-op.
  Asset ROWS go with the `assets.world_id` FK cascade; the dir sweep also collects any orphaned
  `*.tmp` staging residue.

## Gotchas

- **`AssetChanged` is out-of-band** (`broadcast_aux`), so it is not gap-recovered by the event
  RingBuffer — clients treat it as a cache-bust hint, then re-fetch (ETag revalidated).
- A `replace` rate-limited mid-flight should `refund` the limiter slot.

## Pointers

- Rationale: `docs/design/ARCHITECTURE.md` §4 (asset pipeline deferral) + §6 (stable asset identity).
- Relationships:
  `graphify query "asset upload store ETag version AssetChanged streaming limit"`.
- History: [[m8b-assets]], [[commit-db-row-before-swapping-file]].
