# Link-Preview Extensions — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development
> (Sonnet `shadowcat-coder` implementers, dispatcher drives this plan). Invoke
> `shadowcat-codebase-chat` and `shadowcat-codebase-assets` before touching any file below.
> Read the REAL files first — this plan cites exact current symbols, not line numbers.

**Goal:** three additive extensions to the existing, unchanged, SSRF-guarded synchronous
link-preview pipeline (`chat::link_preview`/`chat::preview_cache`): (1) a server-fetched,
asset-ified `og:image` for generic previews, resolved async post-publish via the
`WriteOrigin::ServerMessageRevision` chokepoint; (2) a persisted (restart-surviving) preview
cache layered behind the existing in-memory one; (3) allowlisted-provider oEmbed embeds
(`Segment::OEmbed`), structured fields only, provider `html` never stored or rendered.

**Spec:** `docs/superpowers/specs/2026-08-21-link-preview-extensions-design.md` (approved —
every constant, struct shape, and resolved design fork there is FINAL, not open for
re-litigation, per the standing debt-burndown campaign authority).

## Standing campaign directive (copy verbatim into every dispatch)

> Invoke the shadowcat core skill immediately. You goal is to close all existing bugs and
> to-dos within Shadowcat. The iron rule is no deferrals, of existing work, or new work as it
> comes up - we fix this now unless I give my EXPRESS authorization. The only exception is if a
> bug or to-do has a genuine blocker that is already logged in a milestone in PLAN.md that has
> not been started yet. Another iron clad is rule is that when faced with a design fork,
> determine the best long term shape in keeping with our plans and goals, and implement
> accordingly. You only need to ask me if the question "what is the best long term shape in
> keeping with our plans and goals?" is not able to answer the question. Churn is not a
> concern. This paragraph must be copied verbatim to any agents dispatched in this campaign.

## SECURITY NOTICE

This is a security-sensitive change: it extends the server's SSRF-guarded outbound-fetch
surface (image bytes, oEmbed JSON, oEmbed thumbnails all ride the same guarded client) and
introduces a NEW stored-XSS-adjacent surface (a third-party provider's oEmbed JSON `html`
field, which must never reach any stored field or rendered output). **Task 3** (image pipeline,
reuses the SSRF-guarded fetch machinery for a new content type) and **Task 4** (oEmbed
allowlist + the `html`-exclusion boundary) are the two tasks that touch this surface directly.

## Model/Effort directives

Mirrors this project's existing link-preview plan
(`docs/superpowers/plans/2026-07-13-m11d-3-link-previews.md`): `shadowcat-coder`
(sonnet/medium) as unnamed one-off implementer dispatches per task;
`shadowcat-spec-reviewer` + `shadowcat-code-reviewer` (sonnet/high) as the standard
two-reviewer gate per task. Per this dispatch's explicit instruction and this project's
existing security-review precedent (`docs/superpowers/plans/2026-07-13-m11d-3-link-previews.md`
Task 1's buddy-check):

- **Task 3 and Task 4 use the heavier two-reviewer security tier**: dispatch BOTH
  `shadowcat-spec-reviewer` and `shadowcat-code-reviewer` (as already standard), AND require
  both to explicitly review the diff through a **security lens** (SSRF surface for Task 3's
  `guarded_get`/`fetch_image_bytes` reuse; stored-XSS surface for Task 4's `OEmbedResponse`
  `html`-exclusion and the `#[serde(deny_unknown_fields)]`-omission rationale). If either
  reviewer's findings read as shallow or uncertain on the security question specifically,
  escalate to the `-opus` twin before proceeding, per this project's standing escalation rule.
- All other tasks (1, 2, 5) use the standard two-reviewer gate, no special security framing
  required (Task 1 touches asset-creation ordering, which is data-integrity-sensitive but not
  a new attack surface; Task 2 is cache plumbing).

## Global Constraints

- Gates per task: `cargo test`, `cargo fmt --check`, `cargo clippy --all-targets -D warnings`
  for server tasks (run from `src/server/`); `pnpm -r test`, `pnpm -r typecheck`,
  `pnpm lint:allowances` for client tasks (Task 5). `pnpm build` before any cargo build that
  embeds `dist/` (per `embed-dist-compile-ordering`).
- **No real network in ANY test.** Every new guarded-fetch test point (image bytes, oEmbed
  JSON) is exercised against a stub `axum` server bound to loopback via the EXISTING
  `#[cfg(test)]`-only `build_client_with_resolve_fn`/`GuardedResolver::with_resolve_fn` seam —
  never a new guard mechanism, never real DNS/network.
- **No new lint suppressions** (`#[allow(dead_code)]`, `#[allow(unused*)]`,
  `#[allow(clippy::*)]`, `#[expect(...)]`) — if a genuinely-unused item appears mid-task, wire
  it live or delete it; do not suppress.
- **`chat/` tree docs-ratchet is live**: every new item in `src/server/src/chat/**` needs a doc
  comment (`#![deny(missing_docs)]` + `#![deny(clippy::missing_docs_in_private_items)]` already
  apply file-wide in every `chat/*.rs` file this plan touches or creates).
- **The existing synchronous title/description scrape's guard machinery is reused, never
  reimplemented.** Every new guarded fetch (image bytes, oEmbed JSON) goes through the SAME
  `GuardedResolver`/`validate_url`/redirect-revalidation/timeout/size-cap pipeline
  `fetch_preview` already built, factored into one shared `guarded_get` (Task 3) — never a
  second SSRF guard.
- **Constants:** `MAX_IMAGE_BYTES = 256 * 1024` (Task 3); reuse `MAX_REDIRECTS`,
  `MAX_PREVIEWS_PER_MESSAGE`, `CONNECT_TIMEOUT`/`TOTAL_TIMEOUT`-shaped 5s deadlines for the new
  fetch paths — no new timeout/redirect policy invented.
- **`created_by` on a server-fetched asset is `None`, not a new sentinel UUID** — see the
  "Spec deviation" note in Task 3. `assets.created_by` carries a live SQLite
  `REFERENCES users(id) ON DELETE SET NULL` foreign key with `PRAGMA foreign_keys = ON`
  (`src/server/src/db.rs`, pinned by `opens_a_single_connection_pool_with_foreign_keys_enabled`)
  — an invented sentinel UUID with no matching `users` row would fail every insert. `None`
  already carries the column's documented "no real owning account" semantic
  (`data::asset::Asset.created_by`'s existing doc: "NULL when the uploading account has been
  deleted").
- **`Segment` gains fields/variants on an already-`chat/`-wide-ratcheted enum — every new
  field/variant needs a doc comment**, and `image_asset_id`/`thumbnail_asset_id` need
  `#[serde(default)]` so pre-existing stored `LinkPreview` segments (persisted before this
  plan) round-trip without the new key.

## File Structure

```
src/server/src/data/asset.rs                    [M] AssetError, create_asset_from_bytes, commit_staged_asset
src/server/src/http/assets.rs                    [M] upload() refactored onto commit_staged_asset
src/server/src/http/error.rs                     [M] From<AssetError> for AppError
src/server/migrations/0001_init.sql              [M] + link_preview_cache table
src/server/src/data/repository.rs                [M] + LinkPreviewCacheRow, 3 new Repository trait methods
src/server/src/data/sqlite.rs                    [M] + inherent + trait-delegate impls of the 3 methods
src/server/src/chat/link_preview.rs              [M] guarded_get extraction, fetch_image_bytes,
                                                      fetch_json_bytes, image_url/image_asset_id on
                                                      LinkPreview, PreviewExtract, cached_or_fetch,
                                                      enrich() returns Vec<PendingEnrichment>, oEmbed skip
src/server/src/chat/oembed.rs                    [C] OEmbedProvider, match_provider, OEmbedResponse
src/server/src/chat/post_publish.rs              [C] PendingEnrichment, run_pending_enrichments,
                                                      resolve_preview_image, resolve_oembed
src/server/src/chat/mod.rs                       [M] Segment::LinkPreview.image_asset_id,
                                                      Segment::OEmbed(OEmbedSegment), MessageRequestCtx's
                                                      repo threaded into enrich(), handle_send_message/
                                                      handle_edit_message return (Command, Vec<PendingEnrichment>),
                                                      command_message_id(), mod/pub use additions
src/server/src/ws/conn.rs                        [M] spawn run_pending_enrichments after SendMessage/EditMessage
src/client/core/src/chat-docs.ts                 [M] image_asset_id on link_preview, new oembed segment kind
src/modules/chat-card/src/MessageCard.svelte     [M] thumbnail/badge + oEmbed card rendering
src/client/ui-kit/src/locales/en.ts              [M] chat.oembedOpenOn key
.claude/skills/shadowcat-codebase-chat/SKILL.md      [M] doc-sync (Task 5)
.claude/skills/shadowcat-codebase-assets/SKILL.md    [M] doc-sync (Task 5)
```

---

### Task 1: `create_asset_from_bytes` extraction (zero behavior change)

**Files:** `src/server/src/data/asset.rs`, `src/server/src/http/assets.rs`,
`src/server/src/http/error.rs`.

Refactors the asset-row-commit logic currently inlined in `http::assets::upload`'s multipart
handler into two shared functions in `data::asset`, used by BOTH the existing GM upload route
(unchanged behavior — still streams to disk without buffering the whole body) and the new
link-preview/oEmbed background image pipeline (Tasks 3–4, which have a small, already-in-memory
byte buffer and need no streaming).

1. **`src/server/src/data/asset.rs`** — append:

```rust
/// Errors from the asset-commit path (`create_asset_from_bytes`/
/// `commit_staged_asset`): either the file-system write/rename failed (I/O)
/// or the row insert failed (`DataError`). Mirrors `http::assets::upload`'s
/// own two-stage failure surface, generalized for a caller with no
/// `AppError`/HTTP response to produce (the background image pipeline).
#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    /// Writing/renaming the asset bytes on disk failed.
    #[error("asset file write failed: {0}")]
    Io(#[from] std::io::Error),
    /// The metadata row insert failed.
    #[error("asset row insert failed: {0}")]
    Data(#[from] crate::data::DataError),
}

/// Renames an already-staged temp file into its final asset location, then
/// inserts `asset`'s metadata row — file-BEFORE-row (see
/// `create_asset_from_bytes`'s doc for why: a create has no prior bytes and
/// no existing ETag to strand, so the failure that matters is an orphan DB
/// row, not an orphan file) [[commit-db-row-before-swapping-file]]. Shared
/// commit step: `http::assets::upload` streams its OWN tmp file via
/// `store_streamed` (avoiding a second in-memory buffer for an arbitrarily
/// large GM upload) and calls this directly; `create_asset_from_bytes` stages
/// `bytes` itself first and then calls this — so both callers' resulting
/// `Asset` rows are committed through byte-for-byte the same ordering logic.
pub async fn commit_staged_asset(
    repo: &crate::data::sqlite::SqliteRepository,
    tmp_path: &std::path::Path,
    final_path: &std::path::Path,
    asset: Asset,
) -> Result<Asset, AssetError> {
    if let Err(e) = tokio::fs::rename(tmp_path, final_path).await {
        let _ = tokio::fs::remove_file(tmp_path).await;
        return Err(AssetError::Io(e));
    }
    if let Err(e) = repo.insert_asset(&asset).await {
        let _ = tokio::fs::remove_file(final_path).await;
        return Err(AssetError::Data(e));
    }
    Ok(asset)
}

/// Creates an asset row from an already-in-memory byte buffer: allocates a
/// fresh `Uuid`/`storage_key`, writes `bytes` to a unique temp sibling of the
/// final path, then commits via `commit_staged_asset` (file-first-then-row,
/// unchanged ordering). For a SMALL buffer only (the link-preview/oEmbed
/// background image pipeline, capped at `chat::link_preview::MAX_IMAGE_BYTES`)
/// — `http::assets::upload`'s own arbitrarily-large GM uploads stream
/// straight to disk via `store_streamed` and call `commit_staged_asset`
/// directly instead, never buffering the whole body here.
///
/// `created_by: None` — see this crate's `docs/superpowers/plans/
/// 2026-08-21-link-preview-extensions.md` Global Constraints for why a
/// server-fetched asset uses `None` rather than an invented sentinel user id
/// (the column carries a live `REFERENCES users(id)` foreign key).
pub async fn create_asset_from_bytes(
    repo: &crate::data::sqlite::SqliteRepository,
    assets_root: &std::path::Path,
    world_id: uuid::Uuid,
    bytes: &[u8],
    content_type: &str,
    original_name: &str,
    created_by: Option<uuid::Uuid>,
    now: i64,
) -> Result<Asset, AssetError> {
    let id = uuid::Uuid::new_v4();
    let storage_key = format!("{world_id}/{id}");
    let final_path = assets_root.join(world_id.to_string()).join(id.to_string());
    let tmp_path = final_path.with_file_name(format!("{id}.{}.tmp", uuid::Uuid::new_v4()));
    if let Some(parent) = final_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&tmp_path, bytes).await?;
    let asset = Asset {
        id,
        world_id,
        storage_key,
        original_name: original_name.to_string(),
        content_type: content_type.to_string(),
        byte_size: bytes.len() as i64,
        created_by,
        created_at: now,
        version: 1,
    };
    commit_staged_asset(repo, &tmp_path, &final_path, asset).await
}
```

   `thiserror` is already a dependency (used by `DataError`) — no `Cargo.toml` change.

2. **`src/server/src/http/error.rs`** — add, after the existing `From<crate::data::DataError>`
   impl:

```rust
/// Maps the asset-commit path's failure surface onto the same status codes
/// `DataError`'s conversion already uses for the `Data` arm; an I/O failure
/// (rename/write) is a 500 like `DataError::Sqlx`/`Serde` — detail logged,
/// never echoed.
impl From<crate::data::asset::AssetError> for AppError {
    fn from(e: crate::data::asset::AssetError) -> Self {
        match e {
            crate::data::asset::AssetError::Io(e) => {
                tracing::error!(?e, "asset file commit failed");
                AppError::Internal
            }
            crate::data::asset::AssetError::Data(e) => e.into(),
        }
    }
}
```

3. **`src/server/src/http/assets.rs`** — replace the body of `upload`'s `async { ... }` block
   (from `let (content_type, byte_size, original_name) = store_streamed(...)` through the final
   `Ok(asset)`) with:

```rust
    let outcome: Result<Asset, AppError> = async {
        let (content_type, byte_size, original_name) =
            store_streamed(multipart, &tmp_path, max).await?;
        let asset = Asset {
            id,
            world_id: world,
            storage_key,
            original_name,
            content_type: content_type.to_string(),
            byte_size,
            created_by: Some(user.id),
            created_at: now,
            version: 1,
        };
        // Read-side of the backup quiesce barrier, acquired only around the
        // rename+DB-commit pair below — the one critical section the quiesce
        // exists to keep non-interleaving with an in-server backup's VACUUM +
        // assets copy. Concurrent asset writes share the read side freely;
        // this serializes nothing between uploads.
        let _read_permit = state.write_barrier.read().await;
        crate::data::asset::commit_staged_asset(&state.repo, &tmp_path, &final_path, asset)
            .await
            .map_err(AppError::from)
    }
    .await;
```

   This is a pure extraction: `id`, `storage_key`, `final_path`, `tmp_path`, `max` stay computed
   exactly where they already are (above this block, unchanged); `commit_staged_asset` performs
   the identical rename-then-insert-with-cleanup-on-either-failure sequence the inline code did.
   The `tracing::error!(?e, %id, "asset upload rename failed")` line moves into
   `commit_staged_asset` (now without `%id` in scope there — acceptable per this task's "zero
   behavior change" scope covering request outcomes/status codes, not log-line field lists).

**Tests** (`src/server/src/data/asset.rs`, new `#[cfg(test)] mod tests`):

- `create_asset_from_bytes_and_upload_produce_identical_asset_shape`: call
  `create_asset_from_bytes` against an in-memory `SqliteRepository` + a `tempdir`-style scratch
  root with fixed bytes/content-type/name/`created_by: Some(uuid)`/`now`; assert the returned
  `Asset`'s `storage_key == "{world_id}/{id}"`, `byte_size == bytes.len()`, `version == 1`, and
  that `tokio::fs::read` of `assets_root/{world_id}/{id}` round-trips the exact bytes.
- `create_asset_from_bytes_rejects_missing_world_dir_gracefully`: n/a — `create_dir_all` always
  succeeds for a fresh tempdir; instead test `commit_staged_asset`'s insert-failure cleanup: seed
  a `world_id` that violates `assets.world_id REFERENCES worlds(id)` (no such world row), assert
  `create_asset_from_bytes` returns `Err(AssetError::Data(_))` AND the final file no longer
  exists on disk (`tokio::fs::metadata` errors).
- `create_asset_from_bytes_created_by_none_is_accepted`: `created_by: None` inserts successfully
  (proves the FK's `ON DELETE SET NULL` column accepts `NULL` on insert, not just on a later
  user deletion) — this is the exact call shape Task 3/4's background pipeline uses.
- `http::assets` existing test module: no new tests required (behavior is unchanged by
  definition); run the FULL existing `cargo test -p shadowcat-server --lib http::assets::` suite
  and confirm it still passes unmodified — this IS this task's regression proof.

**Gates:** `cargo test`, `cargo fmt --check`, `cargo clippy --all-targets -D warnings` (from
`src/server/`).

**Commit:** `refactor(data/assets): extract create_asset_from_bytes for link-preview reuse`

---

### Task 2: `link_preview_cache` persisted table + two-tier read/write

**Files:** `src/server/migrations/0001_init.sql`, `src/server/src/data/repository.rs`,
`src/server/src/data/sqlite.rs`, `src/server/src/chat/link_preview.rs`, `src/server/src/chat/mod.rs`.

1. **`src/server/migrations/0001_init.sql`** — single-baseline-edit convention (no incremental
   migration file). Insert immediately after the existing `CREATE INDEX idx_assets_world ON
   assets(world_id);` line and before the `explored_fog` table:

```sql
-- Persisted link-preview cache: the DB-backed tier behind chat::LinkPreviewCache's
-- in-memory fast path. No world_id — same process-global, URL-keyed scope the
-- in-memory cache already has, now durable across restarts. title/description
-- both NULL together = a cached negative outcome (the fetch failed or found no
-- content), mirroring LinkPreview's own None-outcome convention.
CREATE TABLE link_preview_cache (
    url TEXT PRIMARY KEY,
    title TEXT,
    description TEXT,
    image_asset_id TEXT REFERENCES assets(id) ON DELETE SET NULL,
    fetched_at TEXT NOT NULL
);
```

2. **`src/server/src/data/repository.rs`** — add above `pub trait Repository`:

```rust
/// One row from the persisted `link_preview_cache` table
/// (`Repository::get_link_preview_cache`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkPreviewCacheRow {
    /// Server-extracted title, or `None` for a cached negative-outcome row
    /// (both `title` and `description` `None` together).
    pub title: Option<String>,
    /// Server-extracted description, or `None` for a cached negative-outcome row.
    pub description: Option<String>,
    /// The asset-ified `og:image`/oEmbed-thumbnail, once the post-publish
    /// background pipeline (Task 3/4) has resolved one for this URL.
    pub image_asset_id: Option<Uuid>,
    /// When this row was last (re-)fetched, Unix epoch milliseconds.
    pub fetched_at_ms: i64,
}
```

   Add to the trait body, immediately after the existing `async fn get_explored(...)` method
   (before the trait's closing `}`):

```rust
    /// A persisted `link_preview_cache` row for `url`, or `None` if absent.
    /// The DB-backed tier BEHIND `chat::LinkPreviewCache`'s in-memory fast
    /// path — consulted on an in-memory miss so a cold-started process
    /// reuses a still-fresh row instead of re-fetching every previously-seen
    /// URL (see `chat::link_preview::cached_or_fetch`).
    async fn get_link_preview_cache(&self, url: &str) -> Result<Option<LinkPreviewCacheRow>, DataError>;

    /// Upserts `title`/`description`/`fetched_at` for `url`. Leaves any
    /// existing `image_asset_id` untouched on conflict — an already
    /// asset-ified image (set by `set_link_preview_cache_image`) must survive
    /// a later title/description refresh of the same URL.
    async fn upsert_link_preview_cache(
        &self,
        url: &str,
        title: Option<&str>,
        description: Option<&str>,
        fetched_at_ms: i64,
    ) -> Result<(), DataError>;

    /// Sets `image_asset_id` on an EXISTING `url` row (a no-op if the row is
    /// absent — an image is only ever attached to a URL whose
    /// title/description scrape, or the oEmbed thumbnail pipeline's own
    /// placeholder upsert, already created the row).
    async fn set_link_preview_cache_image(&self, url: &str, image_asset_id: Uuid) -> Result<(), DataError>;
```

3. **`src/server/src/data/sqlite.rs`** — add inherent methods immediately after
   `list_assets_by_world` (same region as the other asset-adjacent persistence methods):

```rust
    /// See `Repository::get_link_preview_cache`.
    pub async fn get_link_preview_cache(
        &self,
        url: &str,
    ) -> Result<Option<crate::data::repository::LinkPreviewCacheRow>, DataError> {
        let row = sqlx::query(
            "SELECT title, description, image_asset_id, fetched_at FROM link_preview_cache WHERE url = ?",
        )
        .bind(url)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else { return Ok(None) };
        let fetched_at_raw: String = row.get("fetched_at");
        let fetched_at_ms = fetched_at_raw
            .parse::<i64>()
            .map_err(|e| DataError::OpFailed(e.to_string()))?;
        let image_asset_id = row
            .get::<Option<String>, _>("image_asset_id")
            .map(|s| Uuid::parse_str(&s).map_err(|e| DataError::OpFailed(e.to_string())))
            .transpose()?;
        Ok(Some(crate::data::repository::LinkPreviewCacheRow {
            title: row.get("title"),
            description: row.get("description"),
            image_asset_id,
            fetched_at_ms,
        }))
    }

    /// See `Repository::upsert_link_preview_cache`.
    pub async fn upsert_link_preview_cache(
        &self,
        url: &str,
        title: Option<&str>,
        description: Option<&str>,
        fetched_at_ms: i64,
    ) -> Result<(), DataError> {
        sqlx::query(
            "INSERT INTO link_preview_cache (url, title, description, fetched_at) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT(url) DO UPDATE SET \
               title = excluded.title, description = excluded.description, fetched_at = excluded.fetched_at",
        )
        .bind(url)
        .bind(title)
        .bind(description)
        .bind(fetched_at_ms.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// See `Repository::set_link_preview_cache_image`.
    pub async fn set_link_preview_cache_image(
        &self,
        url: &str,
        image_asset_id: Uuid,
    ) -> Result<(), DataError> {
        sqlx::query("UPDATE link_preview_cache SET image_asset_id = ? WHERE url = ?")
            .bind(image_asset_id.to_string())
            .bind(url)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
```

   Add the trait-delegating impls immediately before the `impl Repository for SqliteRepository`
   block's closing `}` (same block/pattern as the existing `get_explored` delegate):

```rust
    async fn get_link_preview_cache(
        &self,
        url: &str,
    ) -> Result<Option<crate::data::repository::LinkPreviewCacheRow>, DataError> {
        SqliteRepository::get_link_preview_cache(self, url).await
    }

    async fn upsert_link_preview_cache(
        &self,
        url: &str,
        title: Option<&str>,
        description: Option<&str>,
        fetched_at_ms: i64,
    ) -> Result<(), DataError> {
        SqliteRepository::upsert_link_preview_cache(self, url, title, description, fetched_at_ms).await
    }

    async fn set_link_preview_cache_image(&self, url: &str, image_asset_id: Uuid) -> Result<(), DataError> {
        SqliteRepository::set_link_preview_cache_image(self, url, image_asset_id).await
    }
```

4. **`src/server/src/chat/link_preview.rs`** — add, replacing the inline
   `match cache.get(&url, now) { ... }` loop inside `enrich` (the block currently reading:
   `for url in urls { match cache.get(&url, now) { Some(Some(preview)) => ..., Some(None) =>
   {}, None => misses.push(url) } }`):

```rust
/// Two-tier cache lookup for `url`: the in-memory `cache` (fast path, no
/// await beyond a mutex) first, then the persisted `link_preview_cache`
/// table (survives a restart) on a miss — checked BEFORE any network fetch
/// is attempted, so a cold-started process reuses a still-fresh row instead
/// of re-fetching every previously-seen URL. `None` return means BOTH tiers
/// missed (or a persisted row expired past its TTL) and the caller must
/// actually fetch. A persisted hit backfills the in-memory tier so a repeat
/// within the same process's uptime skips the DB entirely. `LinkPreview
/// .image_url` is never persisted (Task 3 note) — a row's known
/// `image_asset_id`, if any, surfaces on `LinkPreview.image_asset_id`
/// instead.
async fn cached_or_fetch(
    repo: &dyn crate::data::repository::Repository,
    cache: &LinkPreviewCache,
    url: &str,
    now: Instant,
    now_ms: i64,
) -> Option<Option<LinkPreview>> {
    if let Some(hit) = cache.get(url, now) {
        return Some(hit);
    }
    let row = repo.get_link_preview_cache(url).await.ok().flatten()?;
    let is_negative = row.title.is_none() && row.description.is_none();
    let ttl_ms = if is_negative {
        NEGATIVE_TTL.as_millis() as i64
    } else {
        POSITIVE_TTL.as_millis() as i64
    };
    if now_ms.saturating_sub(row.fetched_at_ms) >= ttl_ms {
        return None;
    }
    let outcome = if is_negative {
        None
    } else {
        Some(LinkPreview {
            url: url.to_string(),
            title: row.title.unwrap_or_default(),
            description: row.description.unwrap_or_default(),
            image_url: None,
            image_asset_id: row.image_asset_id,
        })
    };
    cache.insert(url.to_string(), outcome.clone(), now);
    Some(outcome)
}
```

   Update `enrich`'s signature to take `repo: &dyn crate::data::repository::Repository` as a new
   second parameter (after `segments`), and replace the cache-check loop:

```rust
    for url in urls {
        match cached_or_fetch(repo, cache, &url, now, now_ms).await {
            Some(Some(preview)) => previews.push(preview),
            Some(None) => {}
            None => misses.push(url),
        }
    }
```

   And in the `JoinSet` completion arm, persist every fresh outcome to the DB tier alongside the
   existing in-memory `cache.insert` (replace the `match result { ... }` block):

```rust
            match result {
                Ok(preview) => {
                    cache.insert(url.clone(), Some(preview.clone()), now);
                    let _ = repo
                        .upsert_link_preview_cache(
                            &url,
                            Some(&preview.title),
                            Some(&preview.description),
                            now_ms,
                        )
                        .await;
                    previews.push(preview);
                }
                Err(_) => {
                    cache.insert(url.clone(), None, now);
                    let _ = repo.upsert_link_preview_cache(&url, None, None, now_ms).await;
                }
            }
```

   (Task 3 further changes `enrich`'s tail — the final `for preview in previews { segments.push
   (...) }` loop — and its return type; this task leaves that loop and `enrich`'s `()` return
   type untouched, so `LinkPreview` does NOT yet need `image_url`/`image_asset_id` fields for
   Task 2 to compile — **defer adding those two fields to `LinkPreview` and
   `Segment::LinkPreview` to Task 3**, and instead have `cached_or_fetch` build a `LinkPreview
   {url, title, description}` matching TODAY's 3-field struct; Task 3 widens both the struct and
   this function together.)

5. **`src/server/src/chat/mod.res`** — update the two `link_preview::enrich(...)` call sites
   (`handle_send_message`, `handle_edit_message`) to pass `repo` as the second argument:
   `link_preview::enrich(&mut content_segments, repo, preview.client, preview.cache,
   preview.rate, ctx.user_id, now, std::time::Instant::now()).await;` (send) and the equivalent
   for edit's `&mut segments`.

**Tests** (`src/server/src/chat/link_preview.rs`, extending the existing `#[cfg(test)] mod
tests`):

- `cached_or_fetch_hits_in_memory_tier_without_touching_repo`: seed the in-memory `cache`
  directly, call `cached_or_fetch` with a repo that would panic/return `Err` if queried (a
  `SqliteRepository` pointed at an unmigrated in-memory DB missing the table is sufficient — a
  query error surfaces as `.ok()` `None`, so instead assert via a call-counting fake — simplest:
  assert the in-memory-hit path returns without ever calling `get_link_preview_cache` by using a
  real migrated repo and a URL NOT present in the DB table; the in-memory hit still returns
  `Some(...)` correctly regardless, proving precedence).
- `cached_or_fetch_cold_start_falls_through_to_persisted_row`: seed ONLY the DB row (via
  `upsert_link_preview_cache`) with an empty in-memory `cache`; assert `cached_or_fetch` returns
  `Some(Some(preview))` matching the row, AND that a second call now hits the in-memory tier
  (seed a way to detect this — e.g. change the underlying DB row after the first call and assert
  the second call still returns the FIRST value, proving it came from the now-backfilled
  in-memory cache).
- `cached_or_fetch_ttl_expired_persisted_row_falls_through_to_miss`: upsert a row with
  `fetched_at_ms` older than `POSITIVE_TTL`; assert `cached_or_fetch` returns `None` (caller must
  fetch).
- `cached_or_fetch_negative_row_honors_negative_ttl`: upsert `title: None, description: None`;
  assert a live-window call returns `Some(None)` and a past-`NEGATIVE_TTL` call returns `None`.
- `enrich_fresh_fetch_writes_through_both_tiers`: full `enrich` integration test (extending the
  existing stub-axum-server test harness already in this file) against a real migrated
  `SqliteRepository`; after `enrich` runs against a cold cache, assert BOTH
  `cache.get(url, now).is_some()` (in-memory) AND `repo.get_link_preview_cache(url).await`
  returns a matching row (persisted).
- `upsert_link_preview_cache_preserves_existing_image_asset_id_on_conflict`: `upsert` once with
  `title/description`, then call `set_link_preview_cache_image`, then `upsert` again with new
  `title/description`; assert `get_link_preview_cache` still returns the ORIGINAL
  `image_asset_id`.
- `set_link_preview_cache_image_is_a_noop_on_absent_row`: call it against a URL never upserted;
  assert `Ok(())` (no error) and `get_link_preview_cache` still returns `None`.

**Gates:** `cargo test`, `cargo fmt --check`, `cargo clippy --all-targets -D warnings`.

**Commit:** `feat(chat): persist the link-preview cache behind the in-memory tier`

---

### Task 3: `og:image` extraction + async image pipeline (SECURITY-REVIEWED)

**Files:** `src/server/src/chat/link_preview.rs`, `src/server/src/chat/post_publish.rs` (new),
`src/server/src/chat/mod.rs`, `src/server/src/ws/conn.rs`.

**Spec deviation flagged for human review (documented here, not silently resolved):** §2 step 4
of the spec directs a "sentinel system identity" for `created_by` distinct from `None`, citing
"docs/design/ARCHITECTURE.md's existing conventions for any comparable system-authored row."
No such convention exists in `ARCHITECTURE.md` or anywhere in the codebase (verified: no
`SYSTEM_USER`/sentinel-account pattern anywhere), AND `assets.created_by TEXT REFERENCES
users(id) ON DELETE SET NULL` is FK-enforced (`PRAGMA foreign_keys = ON`, pinned by
`db::opens_a_single_connection_pool_with_foreign_keys_enabled`) — an invented sentinel UUID with
no matching `users` row would fail every insert, and creating a real persistent pseudo-account
row is an unrequested, unscoped architectural addition (auth/session/membership-list exposure
questions it would raise are answered nowhere in this spec). This plan uses `created_by: None`,
which the `Asset.created_by` field's own existing doc comment already covers ("NULL when the
uploading account has been deleted" generalizes cleanly to "no real user account backs this
row"). **Flag this choice to the user for confirmation before/alongside merge.**

1. **`src/server/src/chat/link_preview.rs`** — refactor the guarded-GET pipeline out of
   `fetch_preview_inner` into a shared helper, and extend content extraction with an image
   candidate.

   Add near the top (after the `PreviewError` enum):

```rust
/// Which Content-Type family a guarded fetch must see to succeed — shared by
/// the HTML preview fetch (`fetch_preview_inner`), the background image
/// fetch (`fetch_image_bytes`), and the oEmbed JSON fetch (`fetch_json_bytes`),
/// the three guarded-GET consumers in this module.
enum ExpectedContentType {
    /// `text/html` or `application/xhtml+xml` (the existing preview gate).
    Html,
    /// Any `image/*` Content-Type (the background image-pipeline gate).
    Image,
    /// `application/json` or any `*+json` suffix (the oEmbed JSON gate).
    Json,
}

impl ExpectedContentType {
    /// Whether `content_type`'s base (before `;charset=...`) matches this family.
    fn matches(&self, content_type: &str) -> bool {
        let base = content_type
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        match self {
            ExpectedContentType::Html => base == "text/html" || base == "application/xhtml+xml",
            ExpectedContentType::Image => base.starts_with("image/"),
            ExpectedContentType::Json => base == "application/json" || base.ends_with("+json"),
        }
    }
}

/// The shared guarded-GET pipeline: validate URL -> manual redirect loop
/// (each hop re-validated via `validate_url`, capped at `MAX_REDIRECTS`) ->
/// status/Content-Type gate (`expect`) -> streamed body capped at
/// `max_bytes`. Returns the final (post-redirect) `Url`, the raw Content-Type
/// header value, and the accumulated body. Every guarded fetch in this
/// module — HTML preview, background image, oEmbed JSON — goes through this
/// ONE function, so the SSRF guard (literal-IP rejection in `validate_url`,
/// `GuardedResolver`, per-hop redirect re-validation, the size cap) is
/// written and tested exactly once.
async fn guarded_get(
    client: &reqwest::Client,
    raw_url: &str,
    expect: ExpectedContentType,
    max_bytes: usize,
) -> Result<(Url, String, Vec<u8>), PreviewError> {
    let mut url = Url::parse(raw_url).map_err(|_| PreviewError::BadScheme)?;
    validate_url(&url)?;
    let mut hop: u8 = 0;
    loop {
        let response = match client.get(url.clone()).send().await {
            Ok(r) => r,
            Err(e) => return Err(classify_transport_error(&e)),
        };
        let status = response.status();
        if status.is_redirection() {
            hop += 1;
            if hop > MAX_REDIRECTS {
                return Err(PreviewError::Redirects);
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or(PreviewError::Transport)?;
            let next = url.join(location).map_err(|_| PreviewError::BadScheme)?;
            validate_url(&next)?;
            url = next;
            continue;
        }
        if !status.is_success() {
            return Err(PreviewError::Http(status.as_u16()));
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        if !expect.matches(&content_type) {
            return Err(PreviewError::NotHtml);
        }
        if let Some(len) = response.content_length() {
            if len > max_bytes as u64 {
                return Err(PreviewError::TooLarge);
            }
        }
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| classify_transport_error(&e))?;
            if body.len() + chunk.len() > max_bytes {
                return Err(PreviewError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        return Ok((url, content_type, body));
    }
}
```

   Update `PreviewError::NotHtml`'s doc comment (it now gates three content-type families, not
   just HTML):

```rust
    /// Response Content-Type did not match the family this guarded fetch
    /// expected — HTML for a page preview, `image/*` for the background
    /// image pipeline, JSON for an oEmbed provider response.
    NotHtml,
```

   Replace `fetch_preview_inner`'s ENTIRE body with:

```rust
async fn fetch_preview_inner(
    client: &reqwest::Client,
    raw_url: &str,
) -> Result<LinkPreview, PreviewError> {
    let (url, _content_type, body) =
        guarded_get(client, raw_url, ExpectedContentType::Html, MAX_PREVIEW_BYTES).await?;
    match extract_preview(&body) {
        Some(extract) => {
            let image_url = extract
                .image_url
                .and_then(|raw| url.join(&raw).ok())
                .map(|u| u.to_string());
            Ok(LinkPreview {
                url: url.to_string(),
                title: extract.title,
                description: extract.description,
                image_url,
                image_asset_id: None,
            })
        }
        None => Err(PreviewError::NoContent),
    }
}
```

   Add, after `MAX_PREVIEW_BYTES`'s const:

```rust
/// Cap on a fetched image's bytes for the background asset-ification
/// pipeline (`og:image`/oEmbed thumbnail). Smaller than `MAX_PREVIEW_BYTES`
/// — a page's declared preview image or a provider's thumbnail is a small
/// web graphic, not a page's full HTML.
pub const MAX_IMAGE_BYTES: usize = 256 * 1024;

/// Fetches `raw_url` through the SAME SSRF-guarded pipeline `fetch_preview`
/// uses (`guarded_get`), gated on an `image/*` Content-Type. Used by the
/// post-publish image/oEmbed-thumbnail background pipeline (`post_publish`)
/// — never on the synchronous send/edit request path.
pub async fn fetch_image_bytes(
    client: &reqwest::Client,
    raw_url: &str,
    deadline: Duration,
) -> Result<(String, Vec<u8>), PreviewError> {
    match tokio::time::timeout(
        deadline,
        guarded_get(client, raw_url, ExpectedContentType::Image, MAX_IMAGE_BYTES),
    )
    .await
    {
        Ok(Ok((_, content_type, body))) => Ok((content_type, body)),
        Ok(Err(e)) => Err(e),
        Err(_elapsed) => Err(PreviewError::Timeout),
    }
}

/// Fetches `raw_url` (an oEmbed provider endpoint) through the SAME
/// SSRF-guarded pipeline, gated on a JSON Content-Type. Used only by
/// `oembed`/`post_publish` against an ALLOWLISTED provider endpoint URL
/// (`OEmbedProvider::endpoint`) — never against an arbitrary posted URL.
pub async fn fetch_json_bytes(
    client: &reqwest::Client,
    raw_url: &str,
    deadline: Duration,
) -> Result<Vec<u8>, PreviewError> {
    match tokio::time::timeout(
        deadline,
        guarded_get(client, raw_url, ExpectedContentType::Json, MAX_PREVIEW_BYTES),
    )
    .await
    {
        Ok(Ok((_, _, body))) => Ok(body),
        Ok(Err(e)) => Err(e),
        Err(_elapsed) => Err(PreviewError::Timeout),
    }
}
```

   Widen `LinkPreview`:

```rust
pub struct LinkPreview {
    pub url: String,
    pub title: String,
    pub description: String,
    /// A candidate `og:image`/canonical-image URL extracted (never fetched)
    /// by THIS fetch — `Some` only on a genuinely fresh scrape, always
    /// `None` on a cache-tier hit (see `image_asset_id`'s doc for why a
    /// cache hit carries the asset id instead, never the raw URL). Never
    /// persisted or serialized to any wire type — purely an in-process
    /// signal from `enrich` to its `PendingEnrichment::PreviewImage` queue.
    pub image_url: Option<String>,
    /// The already-known asset id for this URL's image, populated ONLY from
    /// a persisted-cache hit (`cached_or_fetch`) whose row already carries
    /// one — a fresh fetch never sets this (an image URL alone is not yet
    /// an asset). Mutually exclusive with `image_url` by construction.
    pub image_asset_id: Option<Uuid>,
}
```

   Update `cached_or_fetch` (Task 2) — the two `LinkPreview { url, title, description }`
   constructions written in Task 2 now need the two new fields:
   - The persisted-hit branch: `image_url: None, image_asset_id: row.image_asset_id,`.
   - Task 2's `enrich_fresh_fetch_writes_through_both_tiers`-style tests unaffected (fields are
     additive).

   Replace `extract_preview`'s return type and body. Add above it:

```rust
/// Everything `extract_preview` pulls from a fetched HTML document: title,
/// description, and (new) an optional image candidate — `og:image`
/// preferred, falling back to `<link rel="image_src">`. `image_url` may be
/// RELATIVE (resolved against the page's final URL by the caller,
/// `fetch_preview_inner`, since this pure function only sees bytes).
pub struct PreviewExtract {
    /// Extracted page title, entity-decoded, capped at `MAX_TITLE_CHARS`.
    pub title: String,
    /// Extracted description, entity-decoded, capped at `MAX_DESCRIPTION_CHARS`.
    pub description: String,
    /// Extracted `og:image`/`<link rel="image_src">` URL, RAW (not
    /// entity-decoded beyond attribute parsing, not length-capped, possibly
    /// relative) — the caller resolves and validates it.
    pub image_url: Option<String>,
}
```

   Replace `extract_preview`'s signature and its `None` short-circuit + return, keeping the
   existing `title_tag`/`meta_tags`/`og_title`/`og_description`/`meta_description` logic
   unchanged, adding an `og_image` accumulator scanned alongside the existing loop over
   `meta_tags`, and the final return:

```rust
pub fn extract_preview(bytes: &[u8]) -> Option<PreviewExtract> {
    let html = String::from_utf8_lossy(bytes);
    let lower = html.to_ascii_lowercase();

    let title_tag = extract_tag_text(&html, &lower, "title");
    let meta_tags = extract_meta_tags(&html, &lower);

    let mut og_title = None;
    let mut og_description = None;
    let mut og_image = None;
    let mut meta_description = None;
    for tag in &meta_tags {
        match tag.property.as_deref() {
            Some("og:title") if og_title.is_none() => og_title = tag.content.clone(),
            Some("og:description") if og_description.is_none() => {
                og_description = tag.content.clone()
            }
            Some("og:image") if og_image.is_none() => og_image = tag.content.clone(),
            _ => {}
        }
        if meta_description.is_none() && tag.name.as_deref() == Some("description") {
            meta_description = tag.content.clone();
        }
    }
    let image_url = og_image.or_else(|| extract_link_image_src(&html, &lower));

    let title = clean_text(&og_title.or(title_tag).unwrap_or_default(), MAX_TITLE_CHARS);
    let description = clean_text(
        &og_description.or(meta_description).unwrap_or_default(),
        MAX_DESCRIPTION_CHARS,
    );

    if title.is_empty() && description.is_empty() {
        None
    } else {
        Some(PreviewExtract { title, description, image_url })
    }
}

/// Bounded scan for `<link rel="image_src" href="...">` — the non-OpenGraph
/// canonical-image fallback some pages declare instead of `og:image`. Same
/// byte-index-aligned/64-tag-capped shape as `extract_meta_tags`.
fn extract_link_image_src(html: &str, lower: &str) -> Option<String> {
    let mut from = 0usize;
    let mut scanned = 0usize;
    while scanned < 64 {
        let Some(rel) = lower[from..].find("<link") else {
            break;
        };
        let start = from + rel;
        let Some(gt_rel) = lower[start..].find('>') else {
            break;
        };
        let end = start + gt_rel;
        let tag_orig = &html[start..end];
        let tag_lower = &lower[start..end];
        if extract_attr(tag_lower, tag_orig, "rel").as_deref() == Some("image_src") {
            return extract_attr(tag_lower, tag_orig, "href");
        }
        from = end + 1;
        scanned += 1;
    }
    None
}
```

   Update the existing `extract_preview` tests (`prefers_og_over_title_and_meta`,
   `falls_back_to_title_and_meta_description`, `empty_document_yields_no_content`,
   `decodes_common_entities`, `collapses_whitespace_and_caps_length`) to destructure
   `PreviewExtract { title, description, .. }` instead of a 2-tuple.

2. **`src/server/src/chat/post_publish.rs`** (new file):

```rust
//! Post-publish background enrichment: image/oEmbed fetches deferred until
//! AFTER a message's synchronous send/edit already returned and broadcast —
//! never on the request path. Every entry point here re-publishes via
//! `WriteOrigin::ServerMessageRevision`, the SAME chokepoint
//! `handle_edit_message`/`handle_delete_message` use for their own message
//! revisions (this is the third caller of that origin, after edit and
//! delete).

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use super::link_preview::{fetch_image_bytes, fetch_json_bytes};
use super::oembed::{OEmbedProvider, OEmbedResponse, OEmbedSegment};
use super::{MessageEngine, Segment, MESSAGE_DOC_TYPE};
use crate::data::asset::create_asset_from_bytes;
use crate::data::command::{FieldChange, Operation, WriteOrigin};
use crate::data::document::WorldRole;
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;
use crate::data::sqlite::SqliteRepository;
use crate::ws::room::Room;

/// One post-publish background job queued by `link_preview::enrich` for the
/// caller to run AFTER `Room::publish`'s synchronous send/edit already
/// returned. Never constructed before the message is durably stored — a
/// fetch failure here must never block or delay the send/edit it enriches.
#[derive(Debug, Clone)]
pub enum PendingEnrichment {
    /// A `Segment::LinkPreview` (matched by its stored `url`, not an array
    /// index — this task's own OCC re-read may observe a `content` array a
    /// concurrent edit reordered) whose synchronous scrape found an image
    /// candidate not yet fetched.
    PreviewImage {
        /// The `Segment::LinkPreview.url` this job targets.
        preview_url: String,
        /// The extracted (not yet fetched) image URL.
        image_url: String,
    },
    /// A candidate URL matching the oEmbed provider allowlist. Unlike
    /// `PreviewImage`, no `Segment` exists yet — the whole oEmbed lookup
    /// (JSON fetch + thumbnail asset-ification) is deferred to this job; a
    /// brand-new `Segment::OEmbed` is APPENDED by `run_pending_enrichments`
    /// once it resolves, never patched into a pre-existing segment.
    OEmbed {
        /// The originally posted URL — also the resulting card's
        /// click-through target.
        post_url: String,
        /// The matched allowlist provider.
        provider: OEmbedProvider,
    },
}

/// What one resolved `PendingEnrichment` does to a message's stored `content`.
enum ResolvedEnrichment {
    /// Patch an existing `Segment::LinkPreview` (matched by `url`) with a
    /// resolved image asset id.
    ImageForPreview {
        /// The `Segment::LinkPreview.url` to match.
        preview_url: String,
        /// The resolved asset id.
        asset_id: Uuid,
    },
    /// Append a brand-new `Segment::OEmbed` — none existed synchronously.
    NewOEmbedSegment(Segment),
}

/// Runs every queued `PendingEnrichment` for `message_id` concurrently, then
/// issues AT MOST ONE `WriteOrigin::ServerMessageRevision` `Operation::Update`
/// re-publishing whichever fields the jobs resolved — never zero-to-many
/// separate republishes for one message's worth of pending work. Re-reads the
/// CURRENT stored document immediately before publishing (OCC pre-image),
/// exactly like `handle_edit_message`'s own re-read: the message may have
/// been edited or deleted by the time this task's fetches complete, and a
/// stale `old` value fails the `Update` closed rather than clobbering a
/// newer revision. A tombstoned or vanished message is a silent no-op.
pub async fn run_pending_enrichments(
    room: Arc<Room>,
    repo: Arc<SqliteRepository>,
    client: Arc<reqwest::Client>,
    assets_root: std::path::PathBuf,
    message_id: Uuid,
    world_id: Uuid,
    jobs: Vec<PendingEnrichment>,
) {
    if jobs.is_empty() {
        return;
    }
    let mut set = tokio::task::JoinSet::new();
    for job in jobs {
        let repo = repo.clone();
        let client = client.clone();
        let assets_root = assets_root.clone();
        set.spawn(async move { resolve_job(repo, client, assets_root, world_id, job).await });
    }
    let mut resolved: Vec<ResolvedEnrichment> = Vec::new();
    while let Some(joined) = set.join_next().await {
        if let Ok(Some(r)) = joined {
            resolved.push(r);
        }
    }
    if resolved.is_empty() {
        return;
    }

    let Ok(Some(cur)) = repo.get_document(message_id).await else {
        return;
    };
    if cur.doc_type != MESSAGE_DOC_TYPE {
        return;
    }
    let Ok(mut sys) =
        serde_json::from_value::<MessageEngine>(cur.engine.clone().unwrap_or_default())
    else {
        return;
    };
    if sys.deleted_at.is_some() {
        return; // a tombstoned message has no content left to enrich
    }
    let mut changed = false;
    for r in resolved {
        match r {
            ResolvedEnrichment::ImageForPreview { preview_url, asset_id } => {
                for seg in sys.content.iter_mut() {
                    if let Segment::LinkPreview { url, image_asset_id, .. } = seg {
                        if *url == preview_url {
                            *image_asset_id = Some(asset_id);
                            changed = true;
                        }
                    }
                }
            }
            ResolvedEnrichment::NewOEmbedSegment(seg) => {
                sys.content.push(seg);
                changed = true;
            }
        }
    }
    if !changed {
        return;
    }
    let Ok(new_engine) = serde_json::to_value(&sys) else {
        return;
    };
    let op = Operation::Update {
        doc_id: message_id,
        changes: vec![FieldChange {
            remove: false,
            path: "/engine".into(),
            old: cur.engine.unwrap_or_default(),
            new: new_engine,
        }],
    };
    // Attributed to the message's own sender, re-resolved live (`WorldRole`
    // is not consulted by `apply_intent`'s `ServerMessageRevision` branch —
    // see that chokepoint's doc — so a departed member's default `Player`
    // role here affects only `Command.author` bookkeeping, nothing
    // authorization-relevant).
    let world_role = repo
        .member_role(world_id, sys.user_owner)
        .await
        .ok()
        .flatten()
        .unwrap_or(WorldRole::Player);
    let ctx = PermissionContext { user_id: sys.user_owner, world_role };
    let now = crate::ws::time::now_millis();
    let _ = room
        .publish(repo.as_ref(), &ctx, vec![op], now, WriteOrigin::ServerMessageRevision)
        .await;
}

/// Dispatches one job to its resolver. `None` on any failure (network,
/// decode, asset creation) — a failed background enrichment degrades
/// silently, exactly like the synchronous preview fetch it extends; there is
/// no error surface back to the sender for a job running long after their
/// own request already succeeded.
async fn resolve_job(
    repo: Arc<SqliteRepository>,
    client: Arc<reqwest::Client>,
    assets_root: std::path::PathBuf,
    world_id: Uuid,
    job: PendingEnrichment,
) -> Option<ResolvedEnrichment> {
    match job {
        PendingEnrichment::PreviewImage { preview_url, image_url } => {
            resolve_preview_image(repo, client, assets_root, world_id, preview_url, image_url)
                .await
        }
        PendingEnrichment::OEmbed { post_url, provider } => {
            resolve_oembed(repo, client, assets_root, world_id, post_url, provider).await
        }
    }
}

/// De-dup-then-fetch for one `PreviewImage` job: checks the persisted
/// `link_preview_cache` row for `preview_url` FIRST and reuses an existing
/// `image_asset_id` verbatim on a hit — never re-fetching or re-creating an
/// asset for a link this or any other message already imaged. On a miss,
/// fetches `image_url` through the SAME SSRF-guarded client `link_preview.rs`
/// already built (`fetch_image_bytes`), asset-ifies it via
/// `create_asset_from_bytes` (`created_by: None` — see this plan's Task 3
/// spec-deviation note), and records the result for future hits.
async fn resolve_preview_image(
    repo: Arc<SqliteRepository>,
    client: Arc<reqwest::Client>,
    assets_root: std::path::PathBuf,
    world_id: Uuid,
    preview_url: String,
    image_url: String,
) -> Option<ResolvedEnrichment> {
    if let Ok(Some(row)) = repo.get_link_preview_cache(&preview_url).await {
        if let Some(asset_id) = row.image_asset_id {
            return Some(ResolvedEnrichment::ImageForPreview { preview_url, asset_id });
        }
    }
    let (content_type, bytes) =
        fetch_image_bytes(&client, &image_url, Duration::from_secs(5)).await.ok()?;
    let now = crate::ws::time::now_millis();
    let asset = create_asset_from_bytes(
        &repo,
        &assets_root,
        world_id,
        &bytes,
        &content_type,
        "link-preview-image",
        None,
        now,
    )
    .await
    .ok()?;
    let _ = repo.set_link_preview_cache_image(&preview_url, asset.id).await;
    Some(ResolvedEnrichment::ImageForPreview { preview_url, asset_id: asset.id })
}

/// Fetches the allowlisted provider's oEmbed JSON for `post_url`, extracts
/// ONLY the structured fields `OEmbedResponse` declares (see that type's doc
/// for why its raw `html` field can never reach any stored value — it does
/// not exist as a field to deserialize into), asset-ifies the thumbnail if
/// present, and returns a brand-new `Segment::OEmbed` to append.
async fn resolve_oembed(
    repo: Arc<SqliteRepository>,
    client: Arc<reqwest::Client>,
    assets_root: std::path::PathBuf,
    world_id: Uuid,
    post_url: String,
    provider: OEmbedProvider,
) -> Option<ResolvedEnrichment> {
    let endpoint = provider.endpoint(&post_url)?;
    let body = fetch_json_bytes(&client, &endpoint, Duration::from_secs(5)).await.ok()?;
    let parsed: OEmbedResponse = serde_json::from_slice(&body).ok()?;
    let thumbnail_asset_id = match &parsed.thumbnail_url {
        Some(thumb_url) => {
            resolve_thumbnail_asset(&repo, &client, &assets_root, world_id, thumb_url).await
        }
        None => None,
    };
    Some(ResolvedEnrichment::NewOEmbedSegment(Segment::OEmbed(OEmbedSegment {
        url: post_url,
        provider_name: provider.name().to_string(),
        title: parsed.title,
        author_name: parsed.author_name,
        thumbnail_asset_id,
    })))
}

/// Same persisted-cache-first-then-fetch shape as `resolve_preview_image`,
/// keyed on the THUMBNAIL url (a distinct `link_preview_cache` entry from the
/// post's own row — a provider's thumbnail URL is unrelated to any
/// `og:image` the post URL itself might separately carry). Unlike the
/// `og:image` case, no title/description scrape ever upserts a row for a
/// thumbnail url, so this creates a placeholder row (title/description
/// `None`) before setting its image, so `set_link_preview_cache_image` has a
/// row to update.
async fn resolve_thumbnail_asset(
    repo: &SqliteRepository,
    client: &reqwest::Client,
    assets_root: &std::path::Path,
    world_id: Uuid,
    thumb_url: &str,
) -> Option<Uuid> {
    if let Ok(Some(row)) = repo.get_link_preview_cache(thumb_url).await {
        if let Some(asset_id) = row.image_asset_id {
            return Some(asset_id);
        }
    }
    let (content_type, bytes) =
        fetch_image_bytes(client, thumb_url, Duration::from_secs(5)).await.ok()?;
    let now = crate::ws::time::now_millis();
    let asset = create_asset_from_bytes(
        repo,
        assets_root,
        world_id,
        &bytes,
        &content_type,
        "oembed-thumbnail",
        None,
        now,
    )
    .await
    .ok()?;
    let _ = repo.upsert_link_preview_cache(thumb_url, None, None, now).await;
    let _ = repo.set_link_preview_cache_image(thumb_url, asset.id).await;
    Some(asset.id)
}
```

3. **`src/server/src/chat/mod.rs`**:
   - Add `mod post_publish;` (alongside the existing `mod link_preview;` etc.) and
     `pub use post_publish::{run_pending_enrichments, PendingEnrichment};`.
   - Widen `Segment::LinkPreview` (add a 4th field, and update its doc comment which currently
     documents only 3 fields):

```rust
    LinkPreview {
        /// The previewed URL as posted.
        url: String,
        /// Server-extracted title.
        title: String,
        /// Server-extracted description (may be empty).
        description: String,
        /// The asset-ified `og:image`/canonical-image, once the post-publish
        /// background pipeline (`chat::post_publish`) has resolved one.
        /// Always `None` when `enrich` first appends this segment — set
        /// later ONLY via a `WriteOrigin::ServerMessageRevision` republish
        /// (`run_pending_enrichments`), the same chokepoint
        /// `handle_edit_message`/`handle_delete_message` use.
        /// `#[serde(default)]`: every `LinkPreview` segment persisted before
        /// this field existed has no `image_asset_id` key on disk.
        #[serde(default)]
        image_asset_id: Option<Uuid>,
    },
```

   - Update `enrich`'s finalize loop is inside `link_preview.rs`, not here — but `enrich`'s
     RETURN TYPE change (`Vec<PendingEnrichment>`) propagates into both call sites. Replace the
     final segment-construction loop at the tail of `link_preview::enrich` (Task 2 left it as
     `for preview in previews { segments.push(Segment::LinkPreview { url: preview.url, title:
     preview.title, description: preview.description }); }`) with:

```rust
    let mut pending: Vec<crate::chat::PendingEnrichment> = Vec::new();
    for preview in previews {
        if let Some(asset_id) = preview.image_asset_id {
            segments.push(Segment::LinkPreview {
                url: preview.url,
                title: preview.title,
                description: preview.description,
                image_asset_id: Some(asset_id),
            });
        } else {
            if let Some(image_url) = preview.image_url.clone() {
                pending.push(crate::chat::PendingEnrichment::PreviewImage {
                    preview_url: preview.url.clone(),
                    image_url,
                });
            }
            segments.push(Segment::LinkPreview {
                url: preview.url,
                title: preview.title,
                description: preview.description,
                image_asset_id: None,
            });
        }
    }
    pending
```

     and change `enrich`'s signature return type from `()` to `Vec<PendingEnrichment>` (import
     `PendingEnrichment` via `use super::PendingEnrichment;` at the top of `link_preview.rs`,
     alongside its existing `use super::Segment;`).

   - `handle_send_message`: declare `let mut pending: Vec<PendingEnrichment> = Vec::new();`
     immediately before the existing `if parsed.kind != MessageKind::Roll &&
     policy.previews_enabled() { ... }` block, and inside that block replace the bare
     `link_preview::enrich(...).await;` statement with `pending =
     link_preview::enrich(&mut content_segments, repo, preview.client, preview.cache,
     preview.rate, ctx.user_id, now, std::time::Instant::now()).await;`.
     Change the function's return type from `Result<Command, SendMessageError>` to
     `Result<(Command, Vec<PendingEnrichment>), SendMessageError>`.
     The roll-error early return becomes:

```rust
            if let Some(e) = roll_err {
                let notice = build_roll_error_notice(room.world_id, ctx.user_id, channel, &e, now);
                return room
                    .publish(repo, ctx, vec![Operation::Create { doc: notice }], now, WriteOrigin::Client)
                    .await
                    .map(|cmd| (cmd, Vec::new()))
                    .map_err(SendMessageError::Data);
            }
```

     and the final `room.publish(...)` tail becomes:

```rust
    room.publish(repo, ctx, vec![Operation::Create { doc }], now, WriteOrigin::Client)
        .await
        .map(|cmd| (cmd, pending))
        .map_err(SendMessageError::Data)
```

   - `handle_edit_message`: same shape — declare `let mut pending: Vec<PendingEnrichment> =
     Vec::new();` before its `if policy.previews_enabled() { ... }` block, assign `pending =
     link_preview::enrich(&mut segments, repo, preview.client, preview.cache, preview.rate,
     ctx.user_id, now, std::time::Instant::now()).await;` inside it, change the return type to
     `Result<(Command, Vec<PendingEnrichment>), SendMessageError>`, and its tail:

```rust
    room.publish(repo, ctx, vec![op], now, WriteOrigin::ServerMessageRevision)
        .await
        .map(|cmd| (cmd, pending))
        .map_err(SendMessageError::Data)
```

   - Add a new free function, near `handle_delete_message`:

```rust
/// Extracts the message doc id a `Command` from `handle_send_message` (a
/// `Create`) or `handle_edit_message` (an `Update`) targeted — the id the
/// post-publish background pipeline republishes against.
pub fn command_message_id(cmd: &Command) -> Option<Uuid> {
    match cmd.ops.first()? {
        Operation::Create { doc } => Some(doc.id),
        Operation::Update { doc_id, .. } => Some(*doc_id),
        Operation::Delete { .. } => None,
    }
}
```

   - Update `data::command::WriteOrigin::ServerMessageRevision`'s doc comment (currently: "The
     server's own sanitized chat edit/delete revision — never derivable from a wire frame.") to:
     "The server's own sanitized chat edit/delete revision, OR the post-publish
     image/oEmbed-enrichment republish (`chat::post_publish::run_pending_enrichments`) — never
     derivable from a wire frame." (`src/server/src/data/command.rs`).
   - Update `apply_intent`'s comment block in `src/server/src/data/sqlite.rs` (the block
     documenting "set ONLY by the server edit/delete handlers") similarly — replace "the server
     edit/delete handlers" with "the server edit/delete handlers or the post-publish enrichment
     republish" in BOTH occurrences (the `Update` branch's rejection comment and the `access`
     grant's PRESUPPOSITION comment).
   - Update this file's module-level doc comment's chokepoint description (currently: "produced
     solely by `handle_edit_message`/`handle_delete_message`") to add
     `chat::post_publish::run_pending_enrichments` as a third producer.
   - Fix ALL existing `#[cfg(test)] mod tests` call sites in this file that call
     `handle_send_message`/`handle_edit_message` and destructure/use the returned `Command`
     directly (e.g. `let cmd = handle_send_message(...).await.unwrap();`) — change to `let (cmd,
     _pending) = handle_send_message(...).await.unwrap();` (or `.expect(...)`, matching each
     site's existing style). Locate every site via `cargo build --tests` compiler errors after
     the signature change (each is a straightforward tuple-destructure fix, never a logic
     change) and fix them all — do not leave any commented out or `#[ignore]`d.

4. **`src/server/src/ws/conn.rs`** — after the `SendMessage` arm's `handle_send_message` call,
   replace the `if let Err(e) = crate::chat::handle_send_message(...).await { ... }` block with:

```rust
                                Ok(ClientMsg::SendMessage { request_id, channel, content, actor_owner, audience }) => {
                                    match crate::chat::handle_send_message(
            crate::chat::MessageRequestCtx {
                room: &room,
                repo: repo.as_ref(),
                ctx: &ctx,
                rate: &message_rate,
                preview: crate::chat::LinkPreviewDeps { client: &preview_client, cache: &preview_cache, rate: &preview_rate },
                now: now_millis(),
                budget_per_min: MESSAGE_RATE_PER_MIN,
            },
            channel,
            content,
            actor_owner,
            audience,
        )
                                    .await
                                    {
                                        Ok((cmd, pending)) => {
                                            if !pending.is_empty() {
                                                if let Some(message_id) = crate::chat::command_message_id(&cmd) {
                                                    tokio::spawn(crate::chat::run_pending_enrichments(
                                                        room.clone(),
                                                        repo.clone(),
                                                        preview_client.clone(),
                                                        state.config.assets_path(),
                                                        message_id,
                                                        world_id,
                                                        pending,
                                                    ));
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            tracing::debug!(world = %world_id, user = %user_id, ?e, "message rejected");
                                            if etx.send(Egress::Frame(Arc::new(ServerMsg::ChatError {
                                                request_id,
                                                message: e.to_string(),
                                            }))).await.is_err() {
                                                break;
                                            }
                                        }
                                    }
                                }
```

   Apply the identical `match ... { Ok((cmd, pending)) => { spawn if non-empty }, Err(e) => {
   existing ChatError send } }` restructuring to the `EditMessage` arm (same
   `crate::chat::handle_edit_message` call, same spawn shape — `world_id`/`room`/`repo`/
   `preview_client`/`state` are already in scope identically for both arms).

**Tests:**

- `src/server/src/chat/link_preview.rs`: `extract_preview` gains `image_url` assertions on the
  existing fixtures (`prefers_og_over_title_and_meta` now also asserts `og:image` extraction
  when the fixture includes a `<meta property="og:image" content="...">` tag — extend that
  fixture) plus a NEW `falls_back_to_link_image_src_when_no_og_image` fixture exercising
  `extract_link_image_src`; `fetch_image_bytes`/`fetch_json_bytes` unit tests reusing the
  existing stub-`axum`-server harness (loopback-only, via
  `build_client_with_resolve_fn`/`GuardedResolver`) — cover: success with a correct
  `image/png`/`application/json` Content-Type, wrong Content-Type rejected (`NotHtml`), oversized
  body rejected (`TooLarge`, using `MAX_IMAGE_BYTES` for the image case), and — the
  SECURITY-CRITICAL case — re-run a subset of the EXISTING `fetch_preview` SSRF tests
  (`rejects_literal_blocked_ip_hosts`, `rejects_a_host_that_resolves_to_a_blocked_address`)
  against `fetch_image_bytes`/`fetch_json_bytes` directly, proving the guard applies identically
  to every `guarded_get` consumer, not just the original HTML path.
- `src/server/src/chat/post_publish.rs` (new `#[cfg(test)] mod tests`):
  `resolve_preview_image_cache_hit_skips_network_fetch` (seed a `link_preview_cache` row with a
  non-null `image_asset_id`, call `resolve_preview_image` with a client pointed at nothing
  reachable — must still resolve via the cache, never attempting the network call);
  `resolve_preview_image_cache_miss_fetches_and_creates_asset` (stub image server, empty cache,
  assert an `Asset` row is created with `created_by: None` and the cache row is updated via
  `set_link_preview_cache_image`); `run_pending_enrichments_patches_matching_preview_by_url` (a
  message with two `LinkPreview` segments; only the job matching one `url` patches that segment,
  the other's `image_asset_id` stays `None`); `run_pending_enrichments_is_a_noop_on_tombstoned_message`
  (soft-delete the message between job resolution and republish — via a real `handle_delete_message`
  call before invoking `run_pending_enrichments` — assert no `Operation::Update` is published,
  by asserting the room's seq is unchanged); `run_pending_enrichments_reads_fresh_content_for_occ`
  (edit the message's OTHER content between publish and enrichment resolution; assert the
  republish's OCC `old` value is the POST-EDIT engine body, not a stale snapshot, by asserting
  the edit's own content survives alongside the newly-patched `image_asset_id`).
- `src/server/src/chat/mod.rs`: extend the existing link-preview ingest integration tests
  (the file already has a stub-server-backed `enrich` integration harness per the Task 2 work) to
  assert: a fetched page with an `og:image` meta tag produces a `Segment::LinkPreview` with
  `image_asset_id: None` at publish time AND a non-empty `Vec<PendingEnrichment>` from
  `handle_send_message`'s returned tuple; a page with NO `og:image` produces an empty pending
  list. Also add `command_message_id_extracts_create_and_update_doc_ids` (construct a `Command`
  with each `Operation` variant, assert the extraction).

**Gates:** `cargo test`, `cargo fmt --check`, `cargo clippy --all-targets -D warnings`.

**Commit:** `feat(chat): async og:image asset pipeline via WriteOrigin::ServerMessageRevision`

> **Security review (mandatory before Task 4 starts):** dispatch `shadowcat-spec-reviewer` +
> `shadowcat-code-reviewer` with an explicit security-lens instruction (SSRF: does
> `guarded_get`'s extraction preserve every guard `fetch_preview_inner` had, byte-for-byte,
> for all three consumers?). Escalate to `-opus` twins on any shallow/uncertain finding before
> proceeding to Task 4.

---

### Task 4: oEmbed provider allowlist + `Segment::OEmbed` (SECURITY-REVIEWED)

**Files:** `src/server/src/chat/oembed.rs` (new), `src/server/src/chat/mod.rs`,
`src/server/src/chat/link_preview.rs`.

1. **`src/server/src/chat/oembed.rs`** (new file):

```rust
//! oEmbed provider embeds: allowlisted provider HOSTS only, never
//! autodiscovery (`<link rel="alternate" type="application/json+oembed">`
//! against an arbitrary posted URL would reintroduce the arbitrary-host-
//! fetch risk this feature must avoid). Structured fields only — `title`,
//! `author_name`, `provider_name`, `thumbnail_asset_id` — never a
//! provider's raw `html` field, which is third-party-controlled markup and
//! a direct stored-XSS vector this server does not control.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde::Deserialize;
use url::Host;
use uuid::Uuid;

/// A known oEmbed provider this server will query. Host allowlist ONLY —
/// see this module's doc for why autodiscovery is never attempted. Extending
/// the allowlist means adding a variant here plus a `match_provider`/
/// `endpoint`/`name` arm; the set itself is an implementation choice, not an
/// architectural one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OEmbedProvider {
    /// youtube.com / www.youtube.com / m.youtube.com / youtu.be
    YouTube,
    /// vimeo.com / www.vimeo.com / player.vimeo.com
    Vimeo,
}

impl OEmbedProvider {
    /// This server's OWN fixed display name for the card's "open on
    /// `<provider_name>`" link and `OEmbedSegment.provider_name` — NEVER the
    /// provider's self-reported `provider_name` JSON field (still
    /// third-party-controlled text; this fixed string cannot be spoofed and
    /// needs no sanitization).
    pub fn name(self) -> &'static str {
        match self {
            OEmbedProvider::YouTube => "YouTube",
            OEmbedProvider::Vimeo => "Vimeo",
        }
    }

    /// The provider's oEmbed JSON endpoint URL, with `original_url` (the
    /// posted URL, NOT the endpoint) carried as the `url` query parameter.
    /// The endpoint HOST itself is always one of this module's fixed
    /// allowlisted hosts — never derived from `original_url`. `None` only on
    /// an internal `Url` construction failure (the base strings are fixed
    /// and always valid; this is defensive, not expected to fire).
    pub fn endpoint(self, original_url: &str) -> Option<String> {
        let (base, extra): (&str, &[(&str, &str)]) = match self {
            OEmbedProvider::YouTube => ("https://www.youtube.com/oembed", &[("format", "json")]),
            OEmbedProvider::Vimeo => ("https://vimeo.com/api/oembed.json", &[]),
        };
        let mut u = url::Url::parse(base).ok()?;
        {
            let mut qp = u.query_pairs_mut();
            qp.append_pair("url", original_url);
            for (k, v) in extra {
                qp.append_pair(k, v);
            }
        }
        Some(u.to_string())
    }
}

/// Synchronous, zero-network host check against the allowlist — matched at
/// `link_preview::enrich` time against the SAME candidate URLs the generic
/// preview scraper extracts (genuine `<a href>` targets from sanitized HTML,
/// never raw body-text substrings). No I/O: this check IS the entire SSRF
/// mitigation for oEmbed — a URL failing it falls through to the existing
/// generic `LinkPreview` scrape unchanged; there is no autodiscovery fetch
/// anywhere in this module.
pub fn match_provider(raw_url: &str) -> Option<OEmbedProvider> {
    let url = url::Url::parse(raw_url).ok()?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return None;
    }
    let host = match url.host() {
        Some(Host::Domain(h)) => h.to_ascii_lowercase(),
        _ => return None, // an IP-literal host never matches a provider domain
    };
    match host.as_str() {
        "youtube.com" | "www.youtube.com" | "m.youtube.com" | "youtu.be" => {
            Some(OEmbedProvider::YouTube)
        }
        "vimeo.com" | "www.vimeo.com" | "player.vimeo.com" => Some(OEmbedProvider::Vimeo),
        _ => None,
    }
}

/// The subset of a provider's oEmbed JSON response this server ever reads.
/// STRUCTURAL guarantee, not a runtime filter: this type has NO `html`
/// field, so `serde_json::from_slice` cannot populate one no matter what a
/// provider's JSON contains — the provider's raw markup is dropped by
/// ordinary serde unknown-field behavior. Deliberately does NOT set
/// `#[serde(deny_unknown_fields)]` (the opposite of every engine-defined
/// doc_type's ingress gate elsewhere in this codebase): a provider's `html`
/// field, or any other field this server doesn't read, must be silently
/// ignored, never turn a legitimate oEmbed fetch into a hard failure.
#[derive(Debug, Clone, Deserialize)]
pub struct OEmbedResponse {
    /// Provider-supplied title, if present.
    pub title: Option<String>,
    /// Provider-supplied author/channel name, if present.
    pub author_name: Option<String>,
    /// Provider-supplied thumbnail image URL — fetched and asset-ified
    /// separately (`post_publish::resolve_thumbnail_asset`), never hotlinked.
    pub thumbnail_url: Option<String>,
}

/// The client-visible, structured-fields-only oEmbed segment payload. NO
/// `html` field exists on this type by construction — see `OEmbedResponse`'s
/// doc for the same guarantee at the deserialization boundary one layer
/// earlier. The original posted `url` remains the click-through target; the
/// client renders a first-party-templated card (provider name, title,
/// thumbnail, "open on `<provider_name>`" link), never any provider markup.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, Deserialize)]
pub struct OEmbedSegment {
    /// The originally posted URL.
    pub url: String,
    /// This server's own fixed provider display name (`OEmbedProvider::name`).
    pub provider_name: String,
    /// Provider-supplied title, if any.
    pub title: Option<String>,
    /// Provider-supplied author/channel name, if any.
    pub author_name: Option<String>,
    /// The asset-ified thumbnail, once the post-publish background pipeline
    /// resolves one. Always `None` when this segment is first appended.
    #[serde(default)]
    pub thumbnail_asset_id: Option<Uuid>,
}
```

2. **`src/server/src/chat/mod.rs`**:
   - Add `mod oembed;` and `pub use oembed::{match_provider as match_oembed_provider,
     OEmbedProvider, OEmbedResponse, OEmbedSegment};`.
   - Add the new `Segment` variant, immediately after `LinkPreview { ... }`'s closing `},` and
     before the `// Reserved for a future DocLink segment variant.` comment:

```rust
    /// A provider-native embed from an ALLOWLISTED host (see `chat::oembed`'s
    /// module doc — no autodiscovery ever runs). STRUCTURED FIELDS ONLY: the
    /// provider's own `html` field never reaches this segment (see
    /// `OEmbedSegment`'s doc for the structural guarantee). A message whose
    /// posted URL matches the oEmbed allowlist gets exactly one `OEmbed`
    /// segment for that URL and no accompanying generic `LinkPreview` — the
    /// two are mutually exclusive per URL (`link_preview::enrich`).
    OEmbed(OEmbedSegment),
```

   - Update the enum's own doc comment listing its variants (currently ends "Reserved for a
     future `DocLink` segment variant" — no other enumeration to fix, this is additive).

3. **`src/server/src/chat/link_preview.rs`** — replace `enrich`'s href-collection loop (the
   `'outer: for seg in segments.iter() { ... }` block that builds `urls`) with:

```rust
    let mut urls: Vec<String> = Vec::new();
    let mut pending: Vec<PendingEnrichment> = Vec::new();
    'outer: for seg in segments.iter() {
        if let Segment::Html { sanitized_html } = seg {
            for url in extract_href_urls(sanitized_html) {
                if urls.contains(&url)
                    || pending.iter().any(
                        |p| matches!(p, PendingEnrichment::OEmbed { post_url, .. } if post_url == &url),
                    )
                {
                    continue;
                }
                if let Some(provider) = crate::chat::match_oembed_provider(&url) {
                    pending.push(PendingEnrichment::OEmbed { post_url: url, provider });
                } else {
                    urls.push(url);
                }
                if urls.len() + pending.len() >= MAX_PREVIEWS_PER_MESSAGE {
                    break 'outer;
                }
            }
        }
    }
```

   (This replaces Task 3's `let mut pending: Vec<crate::chat::PendingEnrichment> = Vec::new();`
   declaration placed just before the final segment-construction loop — move that declaration
   UP to here instead, so both the oEmbed matches found during URL collection and the
   `PreviewImage` jobs found during the finalize loop accumulate into the SAME `pending` vec
   returned at the end of `enrich`.)

**Tests:**

- `src/server/src/chat/oembed.rs` (new `#[cfg(test)] mod tests`):
  `match_provider_recognizes_every_allowlisted_youtube_host` /
  `match_provider_recognizes_every_allowlisted_vimeo_host` (table-driven over every listed host
  string, mirroring `link_preview.rs`'s `blocks_every_named_ipv4_range` style);
  `match_provider_rejects_non_allowlisted_hosts` (a handful of plausible-looking non-matches:
  `youtube.com.attacker.example`, `notyoutube.com`, an IP literal); `match_provider_rejects_non_http_schemes`;
  `endpoint_carries_original_url_as_query_param_on_the_fixed_provider_host` (parse the returned
  endpoint string back with `url::Url::parse`, assert `.host_str()` is the FIXED provider host,
  never anything derived from `original_url`, and `.query_pairs()` contains `("url",
  original_url)` verbatim). **The security-critical test**:
  `oembed_response_deserialize_drops_html_field_entirely` — construct a realistic JSON fixture
  string containing `"html": "<script>alert(1)</script><iframe src=...></iframe>"` alongside
  `title`/`author_name`/`thumbnail_url`, deserialize into `OEmbedResponse`, and assert (a) it
  deserializes successfully (no error from the unknown `html` key), and (b) — using
  `std::mem::size_of_val`/field enumeration is NOT sufficient — assert by attempting to construct
  the FULL downstream `OEmbedSegment` from the parsed value and then `serde_json::to_string` it,
  asserting the substring `"<script"` (and `"<iframe"`) does NOT appear anywhere in the
  serialized `OEmbedSegment` output. This is the literal "html never reaches any stored field or
  rendered output" assertion the spec's Testing section calls for.
- `src/server/src/chat/mod.rs`: extend the `enrich`/`handle_send_message` integration tests:
  `oembed_allowlisted_url_produces_oembed_segment_not_link_preview` (post a message linking an
  allowlisted host via a stub — note the ENDPOINT itself is a fixed real host string
  `www.youtube.com`/`vimeo.com`, so THIS test exercises `match_provider`/`PendingEnrichment`
  construction only, not a live fetch — assert `handle_send_message`'s returned `pending`
  contains exactly one `PendingEnrichment::OEmbed` and ZERO `Segment::LinkPreview` was appended
  for that URL); `oembed_and_generic_preview_urls_in_one_message_both_queue_correctly` (a message
  with one allowlisted + one non-allowlisted link: one `PendingEnrichment::OEmbed` plus the
  ordinary generic-preview flow for the other, unaffected); a `post_publish.rs` integration test
  `run_pending_enrichments_appends_new_oembed_segment` using the stub-JSON-server harness
  (constructed analogously to the existing stub-HTML-server harness, serving a fixture
  `OEmbedResponse`-shaped JSON body — reachable only because the TEST overrides
  `OEmbedProvider::endpoint` is NOT stubbable directly, so this test instead calls
  `post_publish::resolve_oembed`'s constituent pieces directly against a stub client whose
  `fetch_json_bytes` call is redirected via the SAME `build_client_with_resolve_fn` seam pointed
  at a fake HOSTNAME that resolves to the stub server loopback address — document this seam
  clearly in the test, mirroring `link_preview.rs`'s existing `client_with_hosts` helper).

**Gates:** `cargo test`, `cargo fmt --check`, `cargo clippy --all-targets -D warnings`.

**Commit:** `feat(chat): oEmbed provider allowlist with structured-fields-only segments`

> **Security review (mandatory, same tier as Task 3):** dispatch `shadowcat-spec-reviewer` +
> `shadowcat-code-reviewer` with an explicit security-lens instruction: (1) confirm
> `OEmbedResponse` genuinely cannot carry `html` through to `OEmbedSegment` under EVERY code
> path (not just the happy path — check `resolve_oembed`'s full body); (2) confirm
> `match_provider`/`endpoint` never let `original_url` influence the ENDPOINT HOST (only the
> query-parameter VALUE) under any crafted input (e.g. a posted URL containing its own
> `&url=`-shaped substring, newline injection into the query value, etc. — `url::Url`'s
> `query_pairs_mut` percent-encodes correctly by construction, but the reviewer must verify this
> claim against the vendored `url` crate behavior, not assume it). Escalate to `-opus` twins on
> any shallow/uncertain finding.

---

### Task 5: Client Zod mirror + rendering + codebase-skill doc-sync

**Files:** `src/client/core/src/chat-docs.ts`, `src/modules/chat-card/src/MessageCard.svelte`,
`src/client/ui-kit/src/locales/en.ts`, `.claude/skills/shadowcat-codebase-chat/SKILL.md`,
`.claude/skills/shadowcat-codebase-assets/SKILL.md`.

1. **`src/client/core/src/chat-docs.ts`**:
   - Widen the `link_preview` arm of the `ChatSegment` union type:

```ts
  | {
      /** A server-fetched, SSRF-guarded preview of a link in the message. */
      kind: "link_preview";
      /** The previewed URL as posted. */
      url: string;
      /** Server-extracted title. */
      title: string;
      /** Server-extracted description (may be empty). */
      description: string;
      /** The asset-ified `og:image`, once the post-publish background
       * pipeline has resolved one. Absent/`null` until then; the client
       * resolves it via `ctx.assets.url(uuid)` — the server's OWN
       * `/api/assets/{uuid}` endpoint, never a raw external URL (which is
       * never stored on this segment in the first place). */
      image_asset_id?: string | null;
    }
```

   - Add a new union member (immediately after `link_preview`):

```ts
  | {
      /** A provider-native embed from an allowlisted host (YouTube, Vimeo).
       * STRUCTURED FIELDS ONLY — the provider's `html` field is never sent
       * to the client at all (the server's `OEmbedSegment` has no such
       * field to serialize). The client renders a first-party-templated
       * card; the provider's own markup is never rendered. */
      kind: "oembed";
      /** The originally posted URL — the card's click-through target. */
      url: string;
      /** The server's own fixed provider display name. */
      provider_name: string;
      /** Provider-supplied title, if any. */
      title?: string | null;
      /** Provider-supplied author/channel name, if any. */
      author_name?: string | null;
      /** The asset-ified thumbnail, once resolved. Absent/`null` until then. */
      thumbnail_asset_id?: string | null;
    };
```

   - Extend `chatSegmentSchemaImpl`'s array:

```ts
  z.object({
    kind: z.literal("link_preview"),
    url: z.string(),
    title: z.string(),
    description: z.string(),
    image_asset_id: z.string().nullish(),
  }),
  z.object({
    kind: z.literal("oembed"),
    url: z.string(),
    provider_name: z.string(),
    title: z.string().nullish(),
    author_name: z.string().nullish(),
    thumbnail_asset_id: z.string().nullish(),
  }),
```

   - Add `s.kind !== "oembed"` to `UnknownSegmentSchema`'s `.refine(...)` predicate (alongside
     the existing five `s.kind !== "..."` checks).
   - Add `s.kind === "oembed"` to `isKnownSegment`'s returned boolean expression (alongside the
     existing five `||`-chained checks), and update its doc comment's kind-list (`@returns` line)
     and its `@example` block header comment to include `oembed`.
   - Update the module-level doc comment above `ChatSegment` (currently "one of the five known
     segment kinds") to "one of the six known segment kinds", and the `link_preview` sentence to
     mention the new optional thumbnail.

2. **`src/modules/chat-card/src/MessageCard.svelte`**:
   - Update the stale doc comment above the segment-rendering `{#each}` block (currently states
     "Every other segment kind (text, roll_embed, roll_button, link_preview) renders via escaped
     interpolation only... No <img>: an <img src> would make the viewer's browser fetch a remote
     resource, leaking their IP") to:

```svelte
              {:else if s.kind === "html"}
                <!-- INVARIANT: sanitized_html is ammonia-cleaned by the server's chat::sanitize —
                the ONLY string this app may ever pass to {@html}. Every other segment kind
                (text, roll_embed, roll_button, link_preview, oembed) renders via escaped
                interpolation only. link_preview/oembed thumbnails DO render an <img>, but its
                `src` is ALWAYS `ctx.assets.url(uuid)` — this server's OWN /api/assets/{uuid}
                endpoint — never a raw external URL: the external fetch already happened
                server-side (Task 3/4 of the link-preview-extensions plan) and the raw source URL
                is structurally never stored on either segment (only a Uuid asset id is), so there
                is no code path by which the viewer's browser could fetch a remote, attacker-chosen
                resource through this card. -->
                <span class="seg-html">{@html s.sanitized_html}</span>
```

   - Replace the `{:else if s.kind === "link_preview"}` branch:

```svelte
              {:else if s.kind === "link_preview"}
                <a
                  class="link-preview"
                  href={safeHref(s.url)}
                  target="_blank"
                  rel="noopener noreferrer nofollow"
                >
                  {#if s.image_asset_id}
                    <img class="link-preview-thumb" src={ctx.assets.url(s.image_asset_id)} alt="" loading="lazy" />
                  {/if}
                  <span class="link-preview-title">{s.title}</span>
                  <span class="link-preview-description">{s.description}</span>
                  <span class="link-preview-host">{hostOf(s.url)}</span>
                </a>
              {:else if s.kind === "oembed"}
                <a
                  class="oembed-card"
                  href={safeHref(s.url)}
                  target="_blank"
                  rel="noopener noreferrer nofollow"
                >
                  {#if s.thumbnail_asset_id}
                    <img class="oembed-thumb" src={ctx.assets.url(s.thumbnail_asset_id)} alt="" loading="lazy" />
                  {/if}
                  <span class="oembed-provider">{s.provider_name}</span>
                  {#if s.title}<span class="oembed-title">{s.title}</span>{/if}
                  {#if s.author_name}<span class="oembed-author">{s.author_name}</span>{/if}
                  <span class="oembed-open">{t("chat.oembedOpenOn", { provider: s.provider_name })}</span>
                </a>
              {/if}
```

   - In the `<style lang="scss">` block, add rules for the new classes near the existing
     `.link-preview*` rules (find them via `grep -n "\.link-preview" MessageCard.svelte`):

```scss
  .link-preview-thumb,
  .oembed-thumb {
    display: block;
    max-width: 100%;
    max-height: 160px;
    object-fit: cover;
    border-radius: var(--radius-1, 4px);
  }
  .oembed-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-1);
    border: 1px solid var(--border-color, #444);
    border-radius: var(--radius-1, 4px);
    text-decoration: none;
    color: inherit;
  }
  .oembed-provider {
    font-size: 0.85em;
    opacity: 0.7;
    text-transform: uppercase;
  }
  .oembed-open {
    font-size: 0.8em;
    opacity: 0.6;
  }
```

3. **`src/client/ui-kit/src/locales/en.ts`** — add, alongside the existing `chat.*` keys:

```ts
  "chat.oembedOpenOn": "Open on {provider}",
```

4. **Codebase skill doc-sync (mandatory gate, this project's CLAUDE.md):**
   - `.claude/skills/shadowcat-codebase-chat/SKILL.md` — update the "Link previews —
     `chat::link_preview` + `chat::preview_cache`" section to document: the new
     `chat::post_publish` module and the `WriteOrigin::ServerMessageRevision` third-caller fact
     (currently the skill's Hard Invariants section states "ONLY `handle_edit_message`/
     `handle_delete_message` ever construct `WriteOrigin::ServerMessageRevision`" — this is now
     FALSE and must be corrected to include `post_publish::run_pending_enrichments`); the new
     `Segment::OEmbed`/`chat::oembed` module and its allowlist-not-autodiscovery design; the
     two-tier persisted `link_preview_cache`; `Segment::LinkPreview.image_asset_id`. Update the
     "Content model is opaque and NOT ts-rs-exported (`MessageKind`, `Segment`,
     `MessageEngine`)" bullet's variant enumeration if it lists them.
   - `.claude/skills/shadowcat-codebase-assets/SKILL.md` — update the "Key files & seams" section
     to document `data::asset::create_asset_from_bytes`/`commit_staged_asset` as the shared
     commit path both `http::assets::upload` and the chat link-preview/oEmbed background
     pipeline use, and note the `created_by: None` convention for server-authored assets (cross-
     reference `shadowcat-codebase-chat`'s post-publish section).
   - Dispatch `shadowcat-spec-reviewer` against BOTH skill diffs (no code diff — purely
     confirming the skill text accurately reflects the shipped change, no omission/drift/broken
     pointer), per this project's Reviewed Skill-Update Gate.

**Tests:** `pnpm --filter @shadowcat/core test` (chat-docs.ts schema round-trip: a fixture
`link_preview` segment WITH and WITHOUT `image_asset_id` both parse; a fixture `oembed` segment
parses; a segment with `kind: "oembed"` but a malformed field — e.g. `provider_name` missing —
fails the whole message per the existing fail-closed convention; a segment carrying a stray
`html` key alongside legitimate `oembed` fields still parses fine, since Zod's default behavior
on `z.object` also strips unknown keys — assert the PARSED VALUE has no `html` property, mirroring
the server-side structural guarantee); `pnpm --filter @shadowcat/module-chat-card test`
(`MessageCard.test.ts`: a `link_preview` segment with `image_asset_id` renders an `<img>` whose
`src` starts with `/api/assets/`; a segment without it renders no `<img>`; an `oembed` segment
renders the provider name, title, and the "open on" link text via `t()`).

**Gates:** `pnpm -r typecheck`, `pnpm -r test`, `pnpm lint:allowances` (from repo root).

**Commit:** `feat(chat-ui): render image-enriched previews and oEmbed cards`

---

## Self-review checklist (completed before handoff)

- **Spec coverage:** §2 (image pipeline, `create_asset_from_bytes`, background task, non-GM-gated
  system authorship) → Tasks 1 & 3. §3 (persisted cache, two-tier) → Task 2. §4 (oEmbed allowlist,
  structured fields, no `html`) → Task 4. §5 (secrecy — no new redaction code needed; confirmed:
  every new field lives inside `MessageEngine.content`, redacted by the existing generic
  mechanism, untouched by this plan) → no task needed, verified not silently dropped. §6 (testing)
  → each task's Tests section. §7 (non-goals: no autodiscovery, no `html` rendering ever, no
  change to the existing sync scrape's guard machinery) → explicitly upheld in Tasks 3–4 (guard
  machinery is EXTRACTED, not modified in behavior; oEmbed is allowlist-only; `html` is
  structurally excluded).
- **Placeholder scan:** no task contains a "TBD", a described-but-not-shown step, or a reference
  to a type/function not defined by an earlier task in this same document (`PendingEnrichment`,
  `OEmbedSegment`, `LinkPreviewCacheRow`, `AssetError`, `guarded_get`, `cached_or_fetch`,
  `create_asset_from_bytes`, `commit_staged_asset`, `command_message_id`,
  `run_pending_enrichments` are all fully defined in the task that introduces them before any
  later task calls them).
- **Type consistency:** `LinkPreview` widened once (Task 3) after Task 2 established its
  original 3-field shape against `cached_or_fetch`; `enrich`'s signature changes exactly twice
  (repo param in Task 2, return type in Task 3) with every call site updated in the SAME task;
  `Segment::LinkPreview`/`Segment::OEmbed` field lists match their `chat-docs.ts` mirrors
  field-for-field (Task 5).

## Spec gaps and deviations surfaced to the user (not silently resolved)

1. **`created_by` sentinel (§2 step 4):** the spec directs a "sentinel system identity," citing
   an "existing convention" that does not exist in `ARCHITECTURE.md` or the codebase, and
   `assets.created_by` is FK-enforced against `users(id)` with `PRAGMA foreign_keys = ON` — an
   invented sentinel UUID would fail every insert. This plan uses `created_by: None` (Task 3).
   **Needs your confirmation.**
2. **Spec's "second caller" ordinal (§2 step 3):** the spec describes this work as "the third
   caller of [`WriteOrigin::ServerMessageRevision`] (recalc-from-chat, §3 of its own spec, is the
   second)." No such caller exists in the codebase — `dice::recalc::recalculate` is a pure,
   unwired math function with zero `WriteOrigin`/`Room::publish` involvement; the ONLY existing
   callers are `handle_edit_message` and `handle_delete_message` (both in `chat::mod.rs`). This
   is a narrative/counting inaccuracy in the spec, not a blocking issue — the actionable
   instruction ("reuse the `WriteOrigin::ServerMessageRevision` chokepoint") is unaffected and is
   what Task 3 implements. Flagged for awareness only.
3. **Image-on-cache-hit design fork (not addressed by the spec's literal text):** the spec states
   `image_asset_id` "starts `None` at synchronous publish time," which this plan honors for a
   FRESH scrape, but for a persisted-cache HIT that already carries a known `image_asset_id`,
   this plan synchronously copies it onto the new segment immediately (Task 3's `cached_or_fetch`
   surfaces `LinkPreview.image_asset_id`) rather than always deferring to a redundant background
   task. This is a resolved design fork (best-shape reasoning: avoids a wasted network
   fetch/asset-creation attempt per repeat post of an already-imaged link), not a contradiction of
   the spec's testing section, which does not test this case either way.
