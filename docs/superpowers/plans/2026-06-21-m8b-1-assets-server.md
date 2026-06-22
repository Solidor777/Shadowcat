# M8b-1 — Assets Server + Thin Resolver Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: this repo's `CLAUDE.md` mandates
> **`mainline-plan-execution`** for plan execution on this (Fable/Opus-class) model —
> use it INSTEAD of `superpowers:subagent-driven-development` /
> `superpowers:executing-plans`. Inline enumerative spec-compliance check per task +
> one final branch review. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Server-side asset storage for Shadowcat — GM/owner-gated image upload to
disk under stable UUID identity, world-membership-gated serving with ETag
revalidation, in-place byte replace and delete, an out-of-band `AssetChanged`
broadcast, and a thin client UUID→URL resolver.

**Architecture:** Files on disk under `<assets_dir>/<world_id>/<uuid>`; metadata in a
dedicated `assets` table (not blobs, not `documents`). HTTP handlers in a new
`http/assets.rs` module reuse the existing `AuthUser` extractor + `require_gm` /
`permission_context` gates. Replace/delete are undo-exempt and broadcast
`AssetChanged{uuid, op}` out-of-band (no world seq) via a new `Room::broadcast_aux`;
the durable source of truth is the record's `version`, so resync needs no replay.
Uploads stream chunk-by-chunk to disk (bounded memory) with a role-tiered size cap +
per-user rate limit. (Pre-M10 cleanup: the per-user tiered rate limit now also covers
`POST /assets/{uuid}/replace`, not upload only — same `upload_rate.check`/`refund` guard.)

**Tech Stack:** Rust, axum 0.8 (add `multipart` feature), sqlx (SQLite, single-writer
pool), ts-rs (Rust→TS type export), Zod (client wire validation), Svelte 5 client
(`@shadowcat/core` resolver). Tests: `tokio::test` integration via the
`tests/common` harness + reqwest (add `multipart` feature); client vitest.

## Global Constraints

- **Cross-platform:** all filesystem paths built with `std::path` (`Path::join`,
  `PathBuf`) — never hardcoded separators. Verified on the CI matrix
  (ubuntu/macos/windows).
- **Server stays structural-only:** Scene `system.background` / Token `system.image`
  store the asset **UUID** as an opaque value; the server never interprets engine
  fields.
- **Undo exemption:** asset replace/delete never enter the world event log and never
  consume a per-world seq.
- **Validation boundary = the bytes:** `content_type` is derived from magic-byte
  detection, never trusted from the client-declared multipart header.
- **No debug code in commits:** diagnostics via `tracing` only; no `println!`/`dbg!`.
- **ts-rs export path:** `#[ts(export, export_to = "../../types/generated/")]`,
  matching the existing `ServerMsg` variants.
- **Images only in M8:** accepted types are PNG, JPEG, GIF, WebP.

---

## File Structure

- **Create** `src/server/migrations/0006_assets.sql` — `assets` table + `world_id` index.
- **Create** `src/server/src/data/asset.rs` — the `Asset` record struct (Serialize + TS).
- **Modify** `src/server/src/data/mod.rs` — `pub mod asset;` + re-export `Asset`.
- **Modify** `src/server/src/data/sqlite.rs` — inherent asset CRUD methods on `SqliteRepository`.
- **Modify** `src/server/src/config.rs` — `assets_dir` + four limit fields, accessors, CLI flags.
- **Modify** `src/server/src/main.rs` — create `assets_dir` on boot; wire `upload_rate` into `AppState`.
- **Modify** `src/server/src/ws/protocol.rs` — `AssetChanged`/`AssetOp` variant.
- **Modify** `src/server/src/ws/room.rs` — `Room::broadcast_aux`.
- **Create** `src/server/src/http/assets.rs` — handlers, magic-byte detection, `UploadRateLimiter`.
- **Modify** `src/server/src/http/mod.rs` — `pub mod assets;`, asset routes, `AppState.upload_rate`, `DefaultBodyLimit` on upload/replace.
- **Modify** `src/server/src/http/error.rs` — `PayloadTooLarge` (413), `TooManyRequests` (429).
- **Modify** `src/server/Cargo.toml` — axum `multipart`; reqwest dev-dep `multipart`.
- **Create** `src/client/core/src/assets.ts` — UUID→URL resolver + placeholder + invalidation.
- **Modify** `src/client/core/src/wire.ts` — `asset_changed` Zod schema.
- **Modify** `src/server/tests/common/mod.rs` — tempdir `assets_dir`, `upload_rate`, asset HTTP helpers.

---

## Buddy-check directives

M8b-1 is **high-risk** by the buddy-checking criteria: a new untrusted **file-upload
ingress** (magic-byte validation, streaming size cap), **filesystem** write/delete,
**capability gating + world-membership read-gate** (authz), and **rate-limiting**.
The M8a foundation got a buddy-check and it caught a Critical. **Run a buddy-check
(two blind reviewers + debate) over the full M8b-1 branch before merge**, focused on:
Tasks 5–8 (upload/serve/replace/delete — auth gates, path traversal, size-cap
enforcement, temp-file cleanup on failure), and Task 3 (out-of-band broadcast not
corrupting the seq/resync invariant). This is recorded per the execution-handoff
offer.

---

## Task 1: `assets` table + `Asset` record + repository CRUD

**Files:**
- Create: `src/server/migrations/0006_assets.sql`
- Create: `src/server/src/data/asset.rs`
- Modify: `src/server/src/data/mod.rs`
- Modify: `src/server/src/data/sqlite.rs` (inherent methods + `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `data::asset::Asset { id: Uuid, world_id: Uuid, storage_key: String,
  original_name: String, content_type: String, byte_size: i64, created_by: Uuid,
  created_at: i64, version: i64 }`.
- Produces on `SqliteRepository`:
  - `async fn insert_asset(&self, a: &Asset) -> Result<(), DataError>`
  - `async fn get_asset(&self, id: Uuid) -> Result<Option<Asset>, DataError>`
  - `async fn replace_asset_bytes(&self, id: Uuid, storage_key: &str, content_type: &str, byte_size: i64) -> Result<i64, DataError>` (returns new `version`)
  - `async fn delete_asset(&self, id: Uuid) -> Result<Option<Asset>, DataError>` (returns the removed record)
  - `async fn list_assets_by_world(&self, world: Uuid) -> Result<Vec<Asset>, DataError>`

- [ ] **Step 1: Write the migration**

Create `src/server/migrations/0006_assets.sql`:

```sql
CREATE TABLE assets (
  id            TEXT PRIMARY KEY,
  world_id      TEXT NOT NULL REFERENCES worlds(id) ON DELETE CASCADE,
  storage_key   TEXT NOT NULL,
  original_name TEXT NOT NULL,
  content_type  TEXT NOT NULL,
  byte_size     INTEGER NOT NULL,
  created_by    TEXT NOT NULL REFERENCES users(id),
  created_at    INTEGER NOT NULL,
  version       INTEGER NOT NULL
);
CREATE INDEX idx_assets_world ON assets(world_id);
```

- [ ] **Step 2: Define the `Asset` struct**

Create `src/server/src/data/asset.rs`:

```rust
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// Metadata for one stored asset. Bytes live on disk at `storage_key`
/// (relative to `assets_dir`); identity (`id`) is stable across rename/replace.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq)]
#[ts(export, export_to = "../../types/generated/")]
pub struct Asset {
    pub id: Uuid,
    pub world_id: Uuid,
    /// "<world_id>/<uuid>", relative to the configured assets_dir.
    pub storage_key: String,
    pub original_name: String,
    pub content_type: String,
    pub byte_size: i64,
    pub created_by: Uuid,
    pub created_at: i64,
    /// Bumped on every replace; backs the ETag and the resync source of truth.
    pub version: i64,
}
```

Add to `src/server/src/data/mod.rs` (beside the other `pub mod` lines):

```rust
pub mod asset;
pub use asset::Asset;
```

- [ ] **Step 3: Write failing repository tests**

Add to the `#[cfg(test)] mod tests` block in `src/server/src/data/sqlite.rs`:

```rust
#[tokio::test]
async fn asset_insert_get_replace_delete_list_round_trip() {
    use crate::data::asset::Asset;
    let r = repo().await;
    let owner = r.create_user("u", Some("h"), ServerRole::User, 0).await.unwrap();
    let world = r.create_world_owned("w", owner, 0).await.unwrap();
    let id = Uuid::from_u128(500);
    let a = Asset {
        id,
        world_id: world.id,
        storage_key: format!("{}/{}", world.id, id),
        original_name: "battlemap.png".into(),
        content_type: "image/png".into(),
        byte_size: 1234,
        created_by: owner,
        created_at: 0,
        version: 1,
    };
    r.insert_asset(&a).await.unwrap();
    assert_eq!(r.get_asset(id).await.unwrap().unwrap(), a);

    // Replace bumps version and updates byte metadata.
    let v = r
        .replace_asset_bytes(id, &a.storage_key, "image/jpeg", 4321)
        .await
        .unwrap();
    assert_eq!(v, 2);
    let after = r.get_asset(id).await.unwrap().unwrap();
    assert_eq!((after.version, after.byte_size, after.content_type.as_str()), (2, 4321, "image/jpeg"));

    // List returns the world's assets.
    assert_eq!(r.list_assets_by_world(world.id).await.unwrap().len(), 1);

    // Delete returns the removed record and empties the store.
    assert_eq!(r.delete_asset(id).await.unwrap().unwrap().id, id);
    assert!(r.get_asset(id).await.unwrap().is_none());
    assert!(r.list_assets_by_world(world.id).await.unwrap().is_empty());
}
```

- [ ] **Step 4: Run it to verify it fails**

Run: `cargo test -p shadowcat --lib asset_insert_get_replace_delete_list_round_trip`
Expected: FAIL — `insert_asset` (etc.) not found.

- [ ] **Step 5: Implement the repository methods**

Add inherent methods to `impl SqliteRepository` in `src/server/src/data/sqlite.rs`
(near the other inherent CRUD like `create_world`). Use `sqlx::Row` (already imported
via `use ... Row`):

```rust
/// Insert a new asset record. `version` starts at 1.
pub async fn insert_asset(&self, a: &crate::data::asset::Asset) -> Result<(), DataError> {
    sqlx::query(
        "INSERT INTO assets \
         (id, world_id, storage_key, original_name, content_type, byte_size, created_by, created_at, version) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(a.id.to_string())
    .bind(a.world_id.to_string())
    .bind(&a.storage_key)
    .bind(&a.original_name)
    .bind(&a.content_type)
    .bind(a.byte_size)
    .bind(a.created_by.to_string())
    .bind(a.created_at)
    .bind(a.version)
    .execute(&self.pool)
    .await?;
    Ok(())
}

fn asset_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<crate::data::asset::Asset, DataError> {
    let parse = |s: String| Uuid::parse_str(&s).map_err(|e| DataError::OpFailed(e.to_string()));
    Ok(crate::data::asset::Asset {
        id: parse(row.get::<String, _>("id"))?,
        world_id: parse(row.get::<String, _>("world_id"))?,
        storage_key: row.get("storage_key"),
        original_name: row.get("original_name"),
        content_type: row.get("content_type"),
        byte_size: row.get("byte_size"),
        created_by: parse(row.get::<String, _>("created_by"))?,
        created_at: row.get("created_at"),
        version: row.get("version"),
    })
}

pub async fn get_asset(&self, id: Uuid) -> Result<Option<crate::data::asset::Asset>, DataError> {
    let row = sqlx::query("SELECT * FROM assets WHERE id = ?")
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
    row.map(|r| Self::asset_from_row(&r)).transpose()
}

/// Swap the bytes behind a stable id; bump and return the new version.
pub async fn replace_asset_bytes(
    &self,
    id: Uuid,
    storage_key: &str,
    content_type: &str,
    byte_size: i64,
) -> Result<i64, DataError> {
    let v: i64 = sqlx::query(
        "UPDATE assets SET storage_key = ?, content_type = ?, byte_size = ?, version = version + 1 \
         WHERE id = ? RETURNING version",
    )
    .bind(storage_key)
    .bind(content_type)
    .bind(byte_size)
    .bind(id.to_string())
    .fetch_optional(&self.pool)
    .await?
    .ok_or(DataError::NotFound)?
    .get("version");
    Ok(v)
}

/// Remove the record, returning it (so the caller can delete the file).
pub async fn delete_asset(&self, id: Uuid) -> Result<Option<crate::data::asset::Asset>, DataError> {
    let existing = self.get_asset(id).await?;
    if existing.is_some() {
        sqlx::query("DELETE FROM assets WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
    }
    Ok(existing)
}

pub async fn list_assets_by_world(&self, world: Uuid) -> Result<Vec<crate::data::asset::Asset>, DataError> {
    let rows = sqlx::query("SELECT * FROM assets WHERE world_id = ? ORDER BY created_at, id")
        .bind(world.to_string())
        .fetch_all(&self.pool)
        .await?;
    rows.iter().map(Self::asset_from_row).collect()
}
```

- [ ] **Step 6: Run it to verify it passes**

Run: `cargo test -p shadowcat --lib asset_insert_get_replace_delete_list_round_trip`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/server/migrations/0006_assets.sql src/server/src/data/
git commit -m "feat(m8b): assets table + Asset record + repository CRUD"
```

---

## Task 2: Config — `assets_dir` + tiered upload limits

**Files:**
- Modify: `src/server/src/config.rs`

**Interfaces:**
- Produces on `Config`:
  - fields `assets_dir: Option<String>`, `upload_max_bytes: u64`,
    `upload_rate_per_min: u32`, `upload_max_bytes_gm: Option<u64>`,
    `upload_rate_per_min_gm: Option<u32>`.
  - `fn assets_path(&self) -> std::path::PathBuf`
  - `fn effective_max_bytes(&self, role: WorldRole) -> u64`
  - `fn effective_rate_per_min(&self, role: WorldRole) -> u32`

- [ ] **Step 1: Write failing config tests**

Add to `#[cfg(test)] mod tests` in `src/server/src/config.rs`:

```rust
#[test]
fn asset_defaults_and_tiering() {
    use crate::data::membership::WorldRole;
    let cfg = Config::default();
    // Default size cap 25 MiB, rate 20/min; GM = 2x when unset.
    assert_eq!(cfg.upload_max_bytes, 25 * 1024 * 1024);
    assert_eq!(cfg.effective_max_bytes(WorldRole::Player), 25 * 1024 * 1024);
    assert_eq!(cfg.effective_max_bytes(WorldRole::Gm), 50 * 1024 * 1024);
    assert_eq!(cfg.effective_rate_per_min(WorldRole::Player), 20);
    assert_eq!(cfg.effective_rate_per_min(WorldRole::Gm), 40);
}

#[test]
fn assets_path_defaults_to_db_sibling() {
    let mut cfg = Config::default();
    cfg.db = "/data/shadowcat.db".into();
    assert_eq!(cfg.assets_path(), std::path::PathBuf::from("/data").join("assets"));
    cfg.assets_dir = Some("/custom/assets".into());
    assert_eq!(cfg.assets_path(), std::path::PathBuf::from("/custom/assets"));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p shadowcat --lib asset_defaults_and_tiering assets_path_defaults_to_db_sibling`
Expected: FAIL — fields/methods missing. (Run each name separately if the multi-filter form errors; `cargo test` takes one filter.)

- [ ] **Step 3: Add the fields, defaults, and accessors**

In `src/server/src/config.rs`, add to `struct Config`:

```rust
    /// Asset storage root. `None` → sibling `assets/` beside the db file.
    pub assets_dir: Option<String>,
    /// Regular-uploader size cap (bytes). Default 25 MiB.
    pub upload_max_bytes: u64,
    /// Regular-uploader uploads per minute. Default 20.
    pub upload_rate_per_min: u32,
    /// GM/owner size cap; `None` → 2× `upload_max_bytes`.
    pub upload_max_bytes_gm: Option<u64>,
    /// GM/owner uploads per minute; `None` → 2× `upload_rate_per_min`.
    pub upload_rate_per_min_gm: Option<u32>,
```

In `impl Default for Config`, add:

```rust
            assets_dir: None,
            upload_max_bytes: 25 * 1024 * 1024,
            upload_rate_per_min: 20,
            upload_max_bytes_gm: None,
            upload_rate_per_min_gm: None,
```

Add accessors in `impl Config`:

```rust
    /// Resolve the asset storage root: explicit `assets_dir`, else a sibling
    /// `assets/` directory beside the db file (built via std::path, #2).
    pub fn assets_path(&self) -> std::path::PathBuf {
        if let Some(dir) = &self.assets_dir {
            return std::path::PathBuf::from(dir);
        }
        std::path::Path::new(&self.db)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("assets")
    }

    /// Role-tiered upload size cap (GM defaults to 2× the regular value).
    pub fn effective_max_bytes(&self, role: crate::data::membership::WorldRole) -> u64 {
        match role {
            crate::data::membership::WorldRole::Gm => {
                self.upload_max_bytes_gm.unwrap_or(self.upload_max_bytes.saturating_mul(2))
            }
            _ => self.upload_max_bytes,
        }
    }

    /// Role-tiered uploads-per-minute (GM defaults to 2× the regular value).
    pub fn effective_rate_per_min(&self, role: crate::data::membership::WorldRole) -> u32 {
        match role {
            crate::data::membership::WorldRole::Gm => {
                self.upload_rate_per_min_gm.unwrap_or(self.upload_rate_per_min.saturating_mul(2))
            }
            _ => self.upload_rate_per_min,
        }
    }
```

Add a CLI flag to `struct Cli`:

```rust
    #[arg(long)]
    pub assets_dir: Option<String>,
```

And in `Config::load`, after the other CLI overrides:

```rust
        if let Some(v) = cli.assets_dir {
            cfg.assets_dir = Some(v);
        }
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p shadowcat --lib asset_defaults_and_tiering`
then: `cargo test -p shadowcat --lib assets_path_defaults_to_db_sibling`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/config.rs
git commit -m "feat(m8b): assets_dir + tiered upload-limit config"
```

---

## Task 3: `AssetChanged` frame + out-of-band broadcast + wire types

**Files:**
- Modify: `src/server/src/ws/protocol.rs`
- Modify: `src/server/src/ws/room.rs`
- Modify: `src/client/core/src/wire.ts`

**Interfaces:**
- Produces: `ServerMsg::AssetChanged { uuid: Uuid, op: AssetOp }`, `enum AssetOp { Replaced, Deleted }`.
- Produces: `Room::broadcast_aux(&self, msg: ServerMsg)` — fire-and-forget send on the broadcast channel; no ring push, no seq bump.
- Consumes (later tasks): handlers call `state.ws.rooms.get(world).map(|r| r.broadcast_aux(...))`.

- [ ] **Step 1: Write a failing protocol test**

Add to `#[cfg(test)] mod tests` in `src/server/src/ws/protocol.rs`:

```rust
#[test]
fn asset_changed_is_out_of_band_and_serializes_snake_case() {
    let m = ServerMsg::AssetChanged { uuid: Uuid::from_u128(7), op: AssetOp::Replaced };
    // Out-of-band: no event seq, so egress sends it without gap/resync logic.
    assert_eq!(m.event_seq(), None);
    let s = serde_json::to_string(&m).unwrap();
    assert!(s.contains("\"type\":\"asset_changed\""), "got {s}");
    assert!(s.contains("\"op\":\"replaced\""), "got {s}");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p shadowcat --lib asset_changed_is_out_of_band_and_serializes_snake_case`
Expected: FAIL — `AssetChanged` / `AssetOp` not found.

- [ ] **Step 3: Add the variant and op enum**

In `src/server/src/ws/protocol.rs`, add the op enum near `ServerMsg`:

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum AssetOp {
    Replaced,
    Deleted,
}
```

Add a variant inside `enum ServerMsg` (after `SceneError`):

```rust
    /// Out-of-band asset mutation notice. Carries no seq and is never buffered
    /// or resynced; holders re-resolve against the record's `version`.
    AssetChanged { uuid: Uuid, op: AssetOp },
```

`event_seq()` already returns `None` for any non-`Event` variant — no change needed.

- [ ] **Step 4: Add `broadcast_aux` to `Room`**

In `src/server/src/ws/room.rs`, add to `impl Room`:

```rust
    /// Broadcast a non-sequenced, out-of-band frame (e.g. AssetChanged). Unlike
    /// `publish`, it does NOT push to the ring or bump `current_seq`, so a
    /// lagging receiver that resyncs from the ring/log never replays it — by
    /// design, since the frame's source of truth (the asset `version`) is
    /// re-read on any access. Best-effort: drops if there are no receivers.
    pub fn broadcast_aux(&self, msg: ServerMsg) {
        let _ = self.tx.send(std::sync::Arc::new(msg));
    }
```

- [ ] **Step 5: Run to verify the protocol test passes + export types**

Run: `cargo test -p shadowcat --lib asset_changed_is_out_of_band_and_serializes_snake_case`
Expected: PASS. The `cargo build`/test run regenerates `AssetChanged`/`AssetOp` into
`src/types/generated/` via ts-rs.

- [ ] **Step 6: Add the client Zod schema**

In `src/client/core/src/wire.ts`, add a case to the `ServerMsgSchema`
discriminated union (beside the `scene_derived` case):

```typescript
  z.object({
    type: z.literal("asset_changed"),
    uuid: z.string(),
    op: z.enum(["replaced", "deleted"]),
  }),
```

- [ ] **Step 7: Run client wire tests**

Run: `pnpm --filter @shadowcat/core test`
Expected: PASS (the wire drift/round-trip test accepts the new variant).

- [ ] **Step 8: Commit**

```bash
git add src/server/src/ws/protocol.rs src/server/src/ws/room.rs src/client/core/src/wire.ts src/types/generated/
git commit -m "feat(m8b): AssetChanged out-of-band frame + broadcast_aux + Zod wire"
```

---

## Task 4: Asset http module scaffolding — magic bytes, rate limiter, error variants

**Files:**
- Create: `src/server/src/http/assets.rs`
- Modify: `src/server/src/http/mod.rs` (`pub mod assets;`, `AppState.upload_rate`)
- Modify: `src/server/src/http/error.rs`
- Modify: `src/server/src/main.rs` (construct `upload_rate`, create assets dir)
- Modify: `src/server/Cargo.toml` (axum `multipart`)

**Interfaces:**
- Produces: `http::assets::detect_image_type(bytes: &[u8]) -> Option<&'static str>`
- Produces: `http::assets::UploadRateLimiter` with
  `fn new() -> Self` and `fn check(&self, user: Uuid, now_ms: i64, per_min: u32) -> bool`.
- Produces: `AppError::PayloadTooLarge(String)` (413), `AppError::TooManyRequests(String)` (429).
- Produces: `AppState.upload_rate: Arc<http::assets::UploadRateLimiter>`.

- [ ] **Step 1: Enable axum multipart**

In `src/server/Cargo.toml`, change the axum dependency line:

```toml
axum = { version = "0.8", features = ["ws", "multipart"] }
```

- [ ] **Step 2: Write failing unit tests for magic bytes + rate limiter**

Create `src/server/src/http/assets.rs` with only the tests first (implementation in
later steps will satisfy them):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn detects_supported_image_signatures_and_rejects_others() {
        assert_eq!(detect_image_type(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]), Some("image/png"));
        assert_eq!(detect_image_type(&[0xFF, 0xD8, 0xFF, 0x00]), Some("image/jpeg"));
        assert_eq!(detect_image_type(b"GIF89a..."), Some("image/gif"));
        assert_eq!(detect_image_type(b"RIFF\0\0\0\0WEBPxxxx"), Some("image/webp"));
        assert_eq!(detect_image_type(b"%PDF-1.7"), None);
        assert_eq!(detect_image_type(b"<svg"), None); // SVG excluded in M8
        assert_eq!(detect_image_type(&[0x89]), None);  // too short to decide
    }

    #[test]
    fn rate_limiter_trips_after_per_min_then_window_slides() {
        let rl = UploadRateLimiter::new();
        let u = Uuid::from_u128(1);
        assert!(rl.check(u, 1_000, 2));
        assert!(rl.check(u, 1_500, 2));
        assert!(!rl.check(u, 1_800, 2)); // 3rd within the window → rejected
        // 61s later the earlier hits have aged out.
        assert!(rl.check(u, 62_001, 2));
    }
}
```

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test -p shadowcat --lib detects_supported_image_signatures_and_rejects_others`
Expected: FAIL — `detect_image_type` not found.

- [ ] **Step 4: Implement magic bytes + rate limiter**

Prepend to `src/server/src/http/assets.rs` (above the test module):

```rust
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

/// Detect a supported image content-type from leading bytes, else `None`.
/// The bytes are the validation boundary — the client-declared content-type is
/// never trusted. Needs ≥12 bytes to rule on WebP. Source: file-format magic
/// numbers (PNG/JFIF/GIF/RIFF specs).
pub fn detect_image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

/// Per-user sliding-window upload limiter (trailing 60s). In-memory; resets on
/// restart, which is acceptable for an abuse backstop.
pub struct UploadRateLimiter {
    hits: Mutex<HashMap<Uuid, Vec<i64>>>,
}

impl UploadRateLimiter {
    pub fn new() -> Self {
        Self { hits: Mutex::new(HashMap::new()) }
    }

    /// Record an upload at `now_ms` and report whether it is within `per_min`.
    /// Prunes entries older than the 60s window first.
    pub fn check(&self, user: Uuid, now_ms: i64, per_min: u32) -> bool {
        let mut map = self.hits.lock().expect("rate-limiter mutex poisoned");
        let v = map.entry(user).or_default();
        let cutoff = now_ms - 60_000;
        v.retain(|&t| t > cutoff);
        if v.len() as u32 >= per_min {
            return false;
        }
        v.push(now_ms);
        true
    }
}

impl Default for UploadRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 5: Add the error variants**

In `src/server/src/http/error.rs`, add to `enum AppError`:

```rust
    PayloadTooLarge(String),
    TooManyRequests(String),
```

And in `impl IntoResponse for AppError`'s match:

```rust
            AppError::PayloadTooLarge(m) => (StatusCode::PAYLOAD_TOO_LARGE, m),
            AppError::TooManyRequests(m) => (StatusCode::TOO_MANY_REQUESTS, m),
```

- [ ] **Step 6: Wire `assets` module + `upload_rate` into AppState**

In `src/server/src/http/mod.rs`: add `pub mod assets;` beside the other module
declarations, and add the field to `struct AppState`:

```rust
    pub upload_rate: std::sync::Arc<assets::UploadRateLimiter>,
```

In `src/server/src/main.rs`, where `AppState` is constructed, add the field and
create the assets directory on boot:

```rust
    std::fs::create_dir_all(config.assets_path())?;
```

and in the `AppState { ... }` literal:

```rust
        upload_rate: std::sync::Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
```

(Adjust the path to `UploadRateLimiter` to match how `main.rs` refers to crate
modules — if `main.rs` is in the binary crate using the lib as `shadowcat::`, keep
the `shadowcat::http::assets::` prefix; otherwise `crate::http::assets::`.)

- [ ] **Step 7: Run unit tests + build**

Run: `cargo test -p shadowcat --lib rate_limiter_trips_after_per_min_then_window_slides`
then: `cargo build -p shadowcat`
Expected: PASS / builds (AppState now has the new field; main.rs compiles).

- [ ] **Step 8: Commit**

```bash
git add src/server/src/http/assets.rs src/server/src/http/mod.rs src/server/src/http/error.rs src/server/src/main.rs src/server/Cargo.toml
git commit -m "feat(m8b): asset http module scaffold — magic bytes, rate limiter, errors"
```

---

## Task 5: Upload endpoint

**Files:**
- Modify: `src/server/src/http/assets.rs` (the `upload` handler + `store_streamed` helper)
- Modify: `src/server/src/http/mod.rs` (route + `DefaultBodyLimit`)
- Modify: `src/server/tests/common/mod.rs` (tempdir `assets_dir`, `upload_rate`, `upload_asset` helper)
- Modify: `src/server/Cargo.toml` (reqwest dev-dep `multipart`)
- Test: `src/server/tests/assets.rs` (create)

**Interfaces:**
- Consumes: `Config::effective_max_bytes`, `Config::effective_rate_per_min`,
  `Config::assets_path`, `UploadRateLimiter::check`, `detect_image_type`,
  `SqliteRepository::insert_asset`, `require_gm`.
- Produces: `POST /api/worlds/{world}/assets` → `200 Json<Asset>`.

- [ ] **Step 1: Add reqwest multipart (dev) + tempdir harness support**

In `src/server/Cargo.toml`, add `multipart` to the reqwest dev-dependency features
(keep existing features such as `json`/`cookies`):

```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "cookies", "multipart", "rustls-tls"] }
```

(Match the existing reqwest line's feature set; only ADD `multipart`.)

In `src/server/tests/common/mod.rs`, make four grounded changes so HTTP asset tests
work. The current harness stores only a `cookie: String` and seeds one user/world; we
add a reusable cookie-bearing client, the user id, an isolated assets dir, and the
rate limiter, and refactor `spawn` to allow a config override for the cap test.

First extend the struct:

```rust
pub struct Harness {
    pub addr: String,
    pub cookie: String,
    pub client: reqwest::Client, // cookie-jar client, already logged in
    pub user: Uuid,              // the seeded user's id
    pub world: Uuid,
    pub repo: Arc<SqliteRepository>,
}
```

Refactor `spawn()` to delegate to a config-mutating variant. Replace the existing
`pub async fn spawn() -> Harness { ... }` with:

```rust
pub async fn spawn() -> Harness {
    spawn_with(|_| {}).await
}

/// Like `spawn`, but `mutate` can tweak the `Config` before the server starts
/// (e.g. set a tiny upload cap). Uses a per-run tempdir for asset storage.
pub async fn spawn_with(mutate: impl FnOnce(&mut Config)) -> Harness {
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let hash = hash_password("pw").unwrap();
    let uid = repo
        .create_user("u", Some(&hash), ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("test", uid, 0).await.unwrap();

    let assets_dir = std::env::temp_dir().join(format!("shadowcat-assets-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&assets_dir).unwrap();
    let mut cfg = Config::default();
    cfg.assets_dir = Some(assets_dir.to_string_lossy().into_owned());
    mutate(&mut cfg);

    let state = AppState {
        repo: repo.clone(),
        config: Arc::new(cfg),
        setup_token: None,
        initialized: Arc::new(AtomicBool::new(true)),
        ws: shadowcat::ws::WsState::new(),
        upload_rate: Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
    };
    let app = http::router(state).await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::builder().cookie_store(true).build().unwrap();
    let res = client
        .post(format!("http://{addr}/api/login"))
        .json(&serde_json::json!({ "username": "u", "password": "pw" }))
        .send()
        .await
        .unwrap();
    let cookie = res
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    Harness { addr, cookie, client, user: uid, world: world.id, repo }
}
```

The `client`'s cookie jar holds the session after login, so `h.client.get/post/delete`
are authenticated automatically (the WS `connect()` keeps using the `cookie` header).

Add an upload helper on `impl Harness`:

```rust
    /// Upload `bytes` as `name` to this world; returns the raw response.
    pub async fn upload(&self, name: &str, content_type: &str, bytes: Vec<u8>) -> reqwest::Response {
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(name.to_string())
            .mime_str(content_type)
            .unwrap();
        let form = reqwest::multipart::Form::new().part("file", part);
        self.client
            .post(format!("http://{}/api/worlds/{}/assets", self.addr, self.world))
            .multipart(form)
            .send()
            .await
            .unwrap()
    }
```

Throughout the asset tests, use `h.client` for HTTP calls (replacing the
`client_with_cookie()` placeholder used in later task snippets).

Add a 1×1 PNG fixture constant for tests:

```rust
/// Minimal valid PNG (1×1) — passes magic-byte detection.
pub const PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];
```

- [ ] **Step 2: Write the failing upload integration test**

Create `src/server/tests/assets.rs`:

```rust
mod common;
use common::{spawn, PNG_1X1};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upload_persists_record_and_file() {
    let h = spawn().await;
    let res = h.upload("battlemap.png", "image/png", PNG_1X1.to_vec()).await;
    assert_eq!(res.status(), 200, "body: {:?}", res.text().await);
    let asset: serde_json::Value = res.json().await.unwrap();
    assert_eq!(asset["content_type"], "image/png");
    assert_eq!(asset["version"], 1);
    // The record is queryable and the file exists on disk.
    let id = uuid::Uuid::parse_str(asset["id"].as_str().unwrap()).unwrap();
    assert!(h.repo.get_asset(id).await.unwrap().is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upload_rejects_non_image_bytes() {
    let h = spawn().await;
    // Declared image/png, but the bytes are a PDF → magic-byte mismatch.
    let res = h.upload("evil.png", "image/png", b"%PDF-1.7 not an image".to_vec()).await;
    assert_eq!(res.status(), 400);
}
```

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test -p shadowcat --test assets upload_persists_record_and_file`
Expected: FAIL — route 404 / handler missing.

- [ ] **Step 4: Implement the upload handler + streaming store**

Add to `src/server/src/http/assets.rs`:

```rust
use axum::extract::{Multipart, Path, State};
use axum::Json;
use tokio::io::AsyncWriteExt;
use crate::data::asset::Asset;
use crate::http::error::AppError;
use crate::http::{routes::require_gm, AppState};
use crate::http::auth::AuthUser; // adjust to where AuthUser is exported

/// Stream a multipart "file" field to `dest`, enforcing `max_bytes` as bytes
/// arrive (never buffering the whole body), and validating the leading bytes are
/// a supported image. Returns (detected_content_type, byte_size, original_name).
/// On any failure the partial file is removed.
async fn store_streamed(
    mut multipart: Multipart,
    dest: &std::path::Path,
    max_bytes: u64,
) -> Result<(&'static str, i64, String), AppError> {
    let field = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?
        .ok_or_else(|| AppError::BadRequest("missing file field".into()))?;
    let original_name = field.file_name().unwrap_or("upload").to_string();

    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|_| AppError::Internal)?;
    }
    let mut file = tokio::fs::File::create(dest).await.map_err(|_| AppError::Internal)?;
    let mut head: Vec<u8> = Vec::with_capacity(16);
    let mut total: u64 = 0;
    let mut detected: Option<&'static str> = None;

    let mut field = field;
    loop {
        let chunk = match field.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => {
                let _ = tokio::fs::remove_file(dest).await;
                return Err(AppError::BadRequest(format!("multipart error: {e}")));
            }
        };
        total += chunk.len() as u64;
        if total > max_bytes {
            let _ = tokio::fs::remove_file(dest).await;
            return Err(AppError::PayloadTooLarge(format!("file exceeds {max_bytes} bytes")));
        }
        if detected.is_none() {
            head.extend_from_slice(&chunk);
            if head.len() >= 12 {
                match detect_image_type(&head) {
                    Some(ct) => detected = Some(ct),
                    None => {
                        let _ = tokio::fs::remove_file(dest).await;
                        return Err(AppError::BadRequest("unsupported or non-image file".into()));
                    }
                }
            }
        }
        if file.write_all(&chunk).await.is_err() {
            let _ = tokio::fs::remove_file(dest).await;
            return Err(AppError::Internal);
        }
    }
    file.flush().await.map_err(|_| AppError::Internal)?;

    // Files shorter than 12 bytes never reached the detector; decide on the head.
    let ct = match detected.or_else(|| detect_image_type(&head)) {
        Some(ct) => ct,
        None => {
            let _ = tokio::fs::remove_file(dest).await;
            return Err(AppError::BadRequest("unsupported or non-image file".into()));
        }
    };
    Ok((ct, total as i64, original_name))
}

/// `POST /api/worlds/{world}/assets` — GM/owner-gated multipart image upload.
pub async fn upload(
    State(state): State<AppState>,
    user: AuthUser,
    Path(world): Path<uuid::Uuid>,
    multipart: Multipart,
) -> Result<Json<Asset>, AppError> {
    let ctx = require_gm(&state, &user, world).await?;
    let now = crate::ws::time::now_millis();
    if !state.upload_rate.check(user.id, now, state.config.effective_rate_per_min(ctx.world_role)) {
        return Err(AppError::TooManyRequests("upload rate limit exceeded".into()));
    }
    let id = uuid::Uuid::new_v4();
    let storage_key = format!("{world}/{id}");
    let dest = state.config.assets_path().join(world.to_string()).join(id.to_string());
    let max = state.config.effective_max_bytes(ctx.world_role);
    let (content_type, byte_size, original_name) = store_streamed(multipart, &dest, max).await?;

    let asset = Asset {
        id,
        world_id: world,
        storage_key,
        original_name,
        content_type: content_type.to_string(),
        byte_size,
        created_by: user.id,
        created_at: now,
        version: 1,
    };
    if let Err(e) = state.repo.insert_asset(&asset).await {
        let _ = tokio::fs::remove_file(&dest).await; // keep disk and DB consistent
        return Err(e.into());
    }
    Ok(Json(asset))
}
```

(Adjust the `use` paths for `AuthUser` and `require_gm` to their real locations:
`AuthUser` per the Explore report is defined for the session extractor; `require_gm`
is in `src/server/src/http/routes.rs`. Make both `pub(crate)` if they are not already
visible from `assets.rs`.)

- [ ] **Step 5: Register the route with the body limit disabled**

In `src/server/src/http/mod.rs`, add to the router (axum 0.8 `{param}` syntax). Import
`use axum::extract::DefaultBodyLimit;` and `use axum::routing::post;` if not present:

```rust
        .route(
            "/api/worlds/{world}/assets",
            post(assets::upload).layer(DefaultBodyLimit::disable()),
        )
```

`DefaultBodyLimit::disable()` is REQUIRED: axum applies a 2 MiB request-body cap by
default that would reject larger uploads before the handler runs. Our streaming
`max_bytes` counter is the real cap.

- [ ] **Step 6: Run to verify it passes**

Run: `cargo test -p shadowcat --test assets upload_persists_record_and_file`
then: `cargo test -p shadowcat --test assets upload_rejects_non_image_bytes`
Expected: PASS.

- [ ] **Step 7: Add GM-gate + over-cap + rate-limit tests**

Append to `src/server/tests/assets.rs`:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upload_over_cap_is_rejected() {
    use common::spawn_with;
    // Regular cap 8 bytes → GM cap 16 (the uploader is GM). The 67-byte PNG
    // exceeds it, tripping the streaming size guard.
    let h = spawn_with(|c| c.upload_max_bytes = 8).await;
    let res = h.upload("big.png", "image/png", PNG_1X1.to_vec()).await;
    assert_eq!(res.status(), 413);
}
```

(`spawn_with` was added to the harness in Task 5 Step 1.)

Run: `cargo test -p shadowcat --test assets upload_over_cap_is_rejected`
Expected: PASS (413).

- [ ] **Step 8: Commit**

```bash
git add src/server/src/http/assets.rs src/server/src/http/mod.rs src/server/tests/ src/server/Cargo.toml
git commit -m "feat(m8b): gated streaming image upload endpoint"
```

---

## Task 6: Serve endpoint (GET with ETag + read-gate)

**Files:**
- Modify: `src/server/src/http/assets.rs` (`serve` handler)
- Modify: `src/server/src/http/mod.rs` (route)
- Test: `src/server/tests/assets.rs`

**Interfaces:**
- Consumes: `SqliteRepository::get_asset`, `permission_context`, `Config::assets_path`.
- Produces: `GET /api/assets/{uuid}` → streams bytes with `Content-Type`,
  `Content-Disposition: inline`, `ETag: "<uuid>-<version>"`; `304` on `If-None-Match`
  match; `403` for non-members; `404` for unknown id.

- [ ] **Step 1: Write the failing serve test**

Append to `src/server/tests/assets.rs`:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn serve_returns_bytes_then_304_on_revalidation() {
    let h = spawn().await;
    let asset: serde_json::Value =
        h.upload("m.png", "image/png", PNG_1X1.to_vec()).await.json().await.unwrap();
    let id = asset["id"].as_str().unwrap();
    let url = format!("http://{}/api/assets/{}", h.addr, id);

    let res = h.client_with_cookie().get(&url).send().await.unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(res.headers()["content-type"], "image/png");
    let etag = res.headers()["etag"].to_str().unwrap().to_string();
    assert_eq!(res.bytes().await.unwrap().as_ref(), PNG_1X1);

    // Conditional GET with the matching ETag → 304.
    let res2 = h
        .client
        .get(&url)
        .header("if-none-match", &etag)
        .send()
        .await
        .unwrap();
    assert_eq!(res2.status(), 304);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p shadowcat --test assets serve_returns_bytes_then_304_on_revalidation`
Expected: FAIL — route missing.

- [ ] **Step 3: Implement the serve handler**

Add to `src/server/src/http/assets.rs`:

```rust
use axum::body::Body;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

/// `GET /api/assets/{uuid}` — read-gated by world membership; ETag-revalidated.
pub async fn serve(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<uuid::Uuid>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let asset = state.repo.get_asset(id).await?.ok_or(AppError::NotFound)?;
    // Read-gate: any member of the asset's world may read. permission_context
    // returns Forbidden for non-members.
    state
        .repo
        .permission_context(asset.world_id, user.id, user.role)
        .await?;

    let etag = format!("\"{}-{}\"", id, asset.version);
    if headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) == Some(etag.as_str()) {
        return Ok((StatusCode::NOT_MODIFIED).into_response());
    }

    let path = state.config.assets_path().join(&asset.storage_key);
    let bytes = tokio::fs::read(&path).await.map_err(|e| {
        tracing::error!(?e, %id, "asset file missing for existing record");
        AppError::Internal
    })?;
    Ok((
        [
            (header::CONTENT_TYPE, asset.content_type),
            (header::CONTENT_DISPOSITION, "inline".to_string()),
            (header::ETAG, etag),
        ],
        Body::from(bytes),
    )
        .into_response())
}
```

(`storage_key` is `"<world_id>/<uuid>"`, both UUIDs — no client-controlled path
component, so `assets_path().join(storage_key)` cannot traverse outside the root.)

- [ ] **Step 4: Register the route**

In `src/server/src/http/mod.rs` (import `use axum::routing::get;` if needed):

```rust
        .route("/api/assets/{uuid}", get(assets::serve))
```

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test -p shadowcat --test assets serve_returns_bytes_then_304_on_revalidation`
Expected: PASS.

- [ ] **Step 6: Add a read-gate test**

The seeded user is a member of `h.world` but not of an unrelated world. Insert an
asset record directly into a second world (via the repo) and GET it as the seeded
user — the membership check returns `403` before any file is read, so no fixture file
is needed. Append to `src/server/tests/assets.rs`:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn serve_denies_non_member() {
    use shadowcat::data::asset::Asset;
    let h = spawn().await;
    // A world the seeded user is NOT a member of (create_world has no owner).
    let other = h.repo.create_world("B", 0).await.unwrap();
    let id = uuid::Uuid::from_u128(0xB0B);
    h.repo
        .insert_asset(&Asset {
            id,
            world_id: other.id,
            storage_key: format!("{}/{}", other.id, id),
            original_name: "x.png".into(),
            content_type: "image/png".into(),
            byte_size: 1,
            created_by: h.user,
            created_at: 0,
            version: 1,
        })
        .await
        .unwrap();
    let res = h
        .client
        .get(format!("http://{}/api/assets/{}", h.addr, id))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 403);
}
```

Run: `cargo test -p shadowcat --test assets`
Expected: PASS (all asset tests).

- [ ] **Step 7: Commit**

```bash
git add src/server/src/http/assets.rs src/server/src/http/mod.rs src/server/tests/assets.rs
git commit -m "feat(m8b): asset serve endpoint with ETag revalidation + read-gate"
```

---

## Task 7: Replace endpoint (byte-swap + AssetChanged)

**Files:**
- Modify: `src/server/src/http/assets.rs` (`replace` handler)
- Modify: `src/server/src/http/mod.rs` (route + `DefaultBodyLimit`)
- Test: `src/server/tests/assets.rs`

**Interfaces:**
- Consumes: `get_asset`, `require_gm`, `store_streamed`, `replace_asset_bytes`,
  `Room::broadcast_aux`, `state.ws.rooms.get`.
- Produces: `POST /api/assets/{uuid}/replace` → `200 Json<Asset>`; broadcasts
  `AssetChanged{uuid, op: Replaced}`; consumes NO world seq.

- [ ] **Step 1: Write the failing replace test**

Append to `src/server/tests/assets.rs` (connect a WS to assert the broadcast):

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replace_swaps_bytes_bumps_version_and_broadcasts() {
    use common::drain_until_type;
    let h = spawn().await;
    let asset: serde_json::Value =
        h.upload("m.png", "image/png", PNG_1X1.to_vec()).await.json().await.unwrap();
    let id = asset["id"].as_str().unwrap().to_string();

    let mut ws = h.connect().await;
    let _ = ws.next().await; // Welcome

    let seq_before = h.repo.get_world(h.world).await.unwrap().unwrap().seq;
    // Replace with a GIF — content_type changes, version bumps to 2.
    let res = h
        .client
        .post(format!("http://{}/api/assets/{}/replace", h.addr, id))
        .multipart(
            reqwest::multipart::Form::new().part(
                "file",
                reqwest::multipart::Part::bytes(b"GIF89a\x01\x00\x01\x00\x00\x00\x00".to_vec())
                    .file_name("m.gif")
                    .mime_str("image/gif")
                    .unwrap(),
            ),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let updated: serde_json::Value = res.json().await.unwrap();
    assert_eq!(updated["version"], 2);
    assert_eq!(updated["content_type"], "image/gif");

    // Out-of-band: no world seq was consumed.
    assert_eq!(h.repo.get_world(h.world).await.unwrap().unwrap().seq, seq_before);

    // The room broadcast an asset_changed{replaced}.
    let frame = drain_until_type(&mut ws, "asset_changed").await;
    assert_eq!(frame["uuid"], id);
    assert_eq!(frame["op"], "replaced");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p shadowcat --test assets replace_swaps_bytes_bumps_version_and_broadcasts`
Expected: FAIL — route missing.

- [ ] **Step 3: Implement the replace handler**

Add to `src/server/src/http/assets.rs`:

```rust
use crate::ws::protocol::{AssetOp, ServerMsg};

/// `POST /api/assets/{uuid}/replace` — GM/owner-gated byte-swap behind a stable
/// id. Undo-exempt: no world seq, no event-log entry.
pub async fn replace(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<uuid::Uuid>,
    multipart: Multipart,
) -> Result<Json<Asset>, AppError> {
    let existing = state.repo.get_asset(id).await?.ok_or(AppError::NotFound)?;
    let ctx = require_gm(&state, &user, existing.world_id).await?;

    // Write the new bytes to a temp sibling, then atomically replace.
    let final_path = state.config.assets_path().join(&existing.storage_key);
    let tmp_path = final_path.with_extension("tmp");
    let max = state.config.effective_max_bytes(ctx.world_role);
    let (content_type, byte_size, _name) = store_streamed(multipart, &tmp_path, max).await?;
    if let Err(e) = tokio::fs::rename(&tmp_path, &final_path).await {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        tracing::error!(?e, %id, "asset replace rename failed");
        return Err(AppError::Internal);
    }

    let version = state
        .repo
        .replace_asset_bytes(id, &existing.storage_key, content_type, byte_size)
        .await?;

    if let Some(room) = state.ws.rooms.get(existing.world_id) {
        room.broadcast_aux(ServerMsg::AssetChanged { uuid: id, op: AssetOp::Replaced });
    }

    Ok(Json(Asset { content_type: content_type.to_string(), byte_size, version, ..existing }))
}
```

- [ ] **Step 4: Register the route (body limit disabled)**

In `src/server/src/http/mod.rs`:

```rust
        .route(
            "/api/assets/{uuid}/replace",
            post(assets::replace).layer(DefaultBodyLimit::disable()),
        )
```

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test -p shadowcat --test assets replace_swaps_bytes_bumps_version_and_broadcasts`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/http/assets.rs src/server/src/http/mod.rs src/server/tests/assets.rs
git commit -m "feat(m8b): asset replace (byte-swap, version bump, AssetChanged)"
```

---

## Task 8: Delete endpoint (file + record + AssetChanged)

**Files:**
- Modify: `src/server/src/http/assets.rs` (`delete` handler)
- Modify: `src/server/src/http/mod.rs` (route)
- Test: `src/server/tests/assets.rs`

**Interfaces:**
- Consumes: `get_asset`, `require_gm`, `delete_asset`, `Room::broadcast_aux`.
- Produces: `DELETE /api/assets/{uuid}` → `204 No Content`; broadcasts
  `AssetChanged{uuid, op: Deleted}`.

- [ ] **Step 1: Write the failing delete test**

Append to `src/server/tests/assets.rs`:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delete_removes_record_and_file_and_broadcasts() {
    use common::drain_until_type;
    let h = spawn().await;
    let asset: serde_json::Value =
        h.upload("m.png", "image/png", PNG_1X1.to_vec()).await.json().await.unwrap();
    let id = asset["id"].as_str().unwrap().to_string();
    let uuid = uuid::Uuid::parse_str(&id).unwrap();

    let mut ws = h.connect().await;
    let _ = ws.next().await;

    let res = h
        .client
        .delete(format!("http://{}/api/assets/{}", h.addr, id))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204);
    assert!(h.repo.get_asset(uuid).await.unwrap().is_none());

    let frame = drain_until_type(&mut ws, "asset_changed").await;
    assert_eq!(frame["op"], "deleted");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p shadowcat --test assets delete_removes_record_and_file_and_broadcasts`
Expected: FAIL — route missing.

- [ ] **Step 3: Implement the delete handler**

Add to `src/server/src/http/assets.rs`:

```rust
/// `DELETE /api/assets/{uuid}` — GM/owner-gated. Undo-exempt.
pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<uuid::Uuid>,
) -> Result<StatusCode, AppError> {
    let existing = state.repo.get_asset(id).await?.ok_or(AppError::NotFound)?;
    require_gm(&state, &user, existing.world_id).await?;

    state.repo.delete_asset(id).await?;
    let path = state.config.assets_path().join(&existing.storage_key);
    if let Err(e) = tokio::fs::remove_file(&path).await {
        // Record is gone; a missing file is not fatal (it becomes a no-op).
        tracing::warn!(?e, %id, "asset file remove failed after record delete");
    }
    if let Some(room) = state.ws.rooms.get(existing.world_id) {
        room.broadcast_aux(ServerMsg::AssetChanged { uuid: id, op: AssetOp::Deleted });
    }
    Ok(StatusCode::NO_CONTENT)
}
```

- [ ] **Step 4: Register the route**

In `src/server/src/http/mod.rs` (import `use axum::routing::delete;` if needed —
or chain `.delete(assets::delete)` onto the existing `/api/assets/{uuid}` route):

```rust
        .route("/api/assets/{uuid}", get(assets::serve).delete(assets::delete))
```

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test -p shadowcat --test assets delete_removes_record_and_file_and_broadcasts`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/http/assets.rs src/server/src/http/mod.rs src/server/tests/assets.rs
git commit -m "feat(m8b): asset delete endpoint + AssetChanged broadcast"
```

---

## Task 9: List endpoint (per-world, membership-gated)

**Files:**
- Modify: `src/server/src/http/assets.rs` (`list` handler)
- Modify: `src/server/src/http/mod.rs` (route)
- Test: `src/server/tests/assets.rs`

**Interfaces:**
- Consumes: `permission_context`, `list_assets_by_world`.
- Produces: `GET /api/worlds/{world}/assets` → `200 Json<Vec<Asset>>` (the b-2 grid source).

- [ ] **Step 1: Write the failing list test**

Append to `src/server/tests/assets.rs`:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn list_returns_world_assets() {
    let h = spawn().await;
    h.upload("a.png", "image/png", PNG_1X1.to_vec()).await;
    h.upload("b.png", "image/png", PNG_1X1.to_vec()).await;
    let res = h
        .client
        .get(format!("http://{}/api/worlds/{}/assets", h.addr, h.world))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let list: Vec<serde_json::Value> = res.json().await.unwrap();
    assert_eq!(list.len(), 2);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p shadowcat --test assets list_returns_world_assets`
Expected: FAIL — route missing.

- [ ] **Step 3: Implement the list handler**

Add to `src/server/src/http/assets.rs`:

```rust
/// `GET /api/worlds/{world}/assets` — membership-gated list (b-2 grid source).
pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Path(world): Path<uuid::Uuid>,
) -> Result<Json<Vec<Asset>>, AppError> {
    state.repo.permission_context(world, user.id, user.role).await?;
    Ok(Json(state.repo.list_assets_by_world(world).await?))
}
```

- [ ] **Step 4: Register the route (chain onto the upload route)**

In `src/server/src/http/mod.rs`, extend the upload route:

```rust
        .route(
            "/api/worlds/{world}/assets",
            post(assets::upload).get(assets::list).layer(DefaultBodyLimit::disable()),
        )
```

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test -p shadowcat --test assets list_returns_world_assets`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/http/assets.rs src/server/src/http/mod.rs src/server/tests/assets.rs
git commit -m "feat(m8b): per-world asset list endpoint"
```

---

## Task 10: Client UUID→URL resolver + placeholder + invalidation

**Files:**
- Create: `src/client/core/src/assets.ts`
- Modify: `src/client/core/src/index.ts` (export the resolver, if the package re-exports)
- Test: `src/client/core/src/assets.test.ts`

**Interfaces:**
- Produces: `AssetResolver` with:
  - `url(uuid: string): string` → `"/api/assets/<uuid>"` (with a cache-busting
    `?v=<version>` once a version is known from an `AssetChanged` or a 200).
  - `placeholder(): string` → the placeholder data-URI/path.
  - `onAssetChanged(msg: { uuid: string; op: "replaced" | "deleted" }): void` →
    invalidates the cached url/version so the next `url()` re-resolves (replaced) or
    returns the placeholder (deleted).

- [ ] **Step 1: Write failing resolver tests**

Create `src/client/core/src/assets.test.ts`:

```typescript
import { describe, it, expect } from "vitest";
import { AssetResolver } from "./assets";

describe("AssetResolver", () => {
  it("resolves a uuid to the serve URL", () => {
    const r = new AssetResolver();
    expect(r.url("abc")).toBe("/api/assets/abc");
  });

  it("after replace, the URL changes (cache-bust) so the new bytes load", () => {
    const r = new AssetResolver();
    const before = r.url("abc");
    r.onAssetChanged({ uuid: "abc", op: "replaced" });
    expect(r.url("abc")).not.toBe(before);
  });

  it("after delete, the uuid resolves to the placeholder", () => {
    const r = new AssetResolver();
    r.onAssetChanged({ uuid: "abc", op: "deleted" });
    expect(r.url("abc")).toBe(r.placeholder());
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter @shadowcat/core test assets`
Expected: FAIL — `./assets` module not found.

- [ ] **Step 3: Implement the resolver**

Create `src/client/core/src/assets.ts`:

```typescript
/** Op carried by an out-of-band AssetChanged frame. */
export type AssetOp = "replaced" | "deleted";

/**
 * Resolves asset UUIDs to serve URLs and reacts to out-of-band AssetChanged
 * notices. The server's ETag handles HTTP caching; a monotonic per-uuid `rev`
 * counter cache-busts the URL on replace so a fresh request (and thus ETag
 * revalidation) happens. Deleted uuids resolve to the placeholder.
 */
export class AssetResolver {
  private revs = new Map<string, number>();
  private deleted = new Set<string>();

  /** A neutral 1×1 transparent placeholder. */
  placeholder(): string {
    return "data:image/gif;base64,R0lGODlhAQABAAAAACwAAAAAAQABAAA=";
  }

  url(uuid: string): string {
    if (this.deleted.has(uuid)) return this.placeholder();
    const rev = this.revs.get(uuid);
    return rev === undefined ? `/api/assets/${uuid}` : `/api/assets/${uuid}?v=${rev}`;
  }

  /** Invalidate a uuid in response to an AssetChanged frame. */
  onAssetChanged(msg: { uuid: string; op: AssetOp }): void {
    if (msg.op === "deleted") {
      this.deleted.add(msg.uuid);
      this.revs.delete(msg.uuid);
      return;
    }
    // replaced: drop any delete marker and bump the cache-bust revision.
    this.deleted.delete(msg.uuid);
    this.revs.set(msg.uuid, (this.revs.get(msg.uuid) ?? 0) + 1);
  }
}
```

If `@shadowcat/core` re-exports from an `index.ts`, add `export * from "./assets";`.

- [ ] **Step 4: Run to verify it passes**

Run: `pnpm --filter @shadowcat/core test assets`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/client/core/src/assets.ts src/client/core/src/assets.test.ts src/client/core/src/index.ts
git commit -m "feat(m8b): client asset resolver — UUID->URL, placeholder, invalidation"
```

---

## Final verification gates (run before declaring M8b-1 complete)

- [ ] `cargo fmt -p shadowcat -- --check` — clean.
- [ ] `cargo clippy -p shadowcat --all-targets -- -D warnings` — clean.
- [ ] `cargo test -p shadowcat` — all lib + integration tests green.
- [ ] `pnpm --filter @shadowcat/core test` and `pnpm --filter @shadowcat/core typecheck` — green.
- [ ] `pnpm lint` — green.
- [ ] `git diff --exit-code src/types/generated` — no uncommitted ts-rs drift.
- [ ] `graphify update .` after code changes.

---

## Self-Review (completed during authoring)

- **Spec coverage:** §2 items → Tasks 2/3/6; §3 table → Task 1; §4 config → Task 2;
  §5 endpoints → Tasks 5–9; §6 AssetChanged → Task 3; §7 resolver → Task 10; §8
  decomposition → this plan is b-1 only; §10 server tests → each task's tests.
  b-2 (client panel) is a separate plan.
- **Placeholder scan:** none — every code step has full code. The harness changes
  (struct fields `client`/`user`, `spawn_with`, `upload`, tempdir `assets_dir`,
  `upload_rate`) are fully specified against the real `tests/common/mod.rs`; all HTTP
  calls use `h.client`; the cap test uses `spawn_with`; the read-gate test uses
  `repo.insert_asset` directly.
- **Type consistency:** `Asset`, `AssetOp::{Replaced,Deleted}`, `broadcast_aux`,
  `detect_image_type`, `UploadRateLimiter::check`, `assets_path`,
  `effective_max_bytes`/`effective_rate_per_min`, `store_streamed` used identically
  across tasks. Route paths use axum 0.8 `{param}` form throughout.
