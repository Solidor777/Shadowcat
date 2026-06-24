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
    limits (GM gets a higher tier than a regular user); `detect_image_type`.
  - `serve(...)` — `GET /api/assets/{uuid}`, membership-gated; ETag = `"{id}-{version}"`;
    `If-None-Match` is an RFC 7232 comma-separated list → 304 if our ETag appears anywhere in it.
  - `replace(...)` — swaps bytes, keeping the stable UUID; broadcasts `AssetChanged`.
- `src/server/src/ws/protocol.rs` — `ServerMsg::AssetChanged { uuid, op: AssetOp }`, broadcast
  **out-of-band** via `Room::broadcast_aux` (not in the per-world event sequence).
- `src/modules/assets/{Assets.svelte,index.ts}` — the client asset panel (upload/list/replace).

## Hard invariants

- **Commit the source-of-truth/cache-key row BEFORE swapping the file.** The inverse strands new
  bytes under a stale ETag/version — a silent 304 of changed content. Two-store writes (file +
  metadata row) without a spanning txn must order the row (cache key) first
  [[commit-db-row-before-swapping-file]].
- **ETag == `"{id}-{version}"`**; `version` is the single monotonic cache key. Stable UUID identity
  means a replace keeps the id and only bumps the version, so links survive (ARCHITECTURE §6).
- **Upload limits are tiered + configurable** (GM ≈ 2× regular); uploads stream to disk, not buffered.

## Gotchas

- **`AssetChanged` is out-of-band** (`broadcast_aux`), so it is not gap-recovered by the event
  RingBuffer — clients treat it as a cache-bust hint, then re-fetch (ETag revalidated).
- A `replace` rate-limited mid-flight should `refund` the limiter slot.

## Pointers

- Rationale: `docs/design/ARCHITECTURE.md` §4 (asset pipeline deferral) + §6 (stable asset identity).
- Relationships:
  `graphify query "asset upload store ETag version AssetChanged streaming limit"`.
- History: [[m8b-assets]], [[commit-db-row-before-swapping-file]].
