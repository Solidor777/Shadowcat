# M15a — Asset Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. (On a Fable-class session, `mainline-plan-execution` replaces both.)

**Goal:** Replace the M8b raw asset path with the real pipeline — WebP conversion with retained originals and thumb/preview derivatives, explicit + derived tags, `asset_folder` documents, resumable chunked upload, a filtered/paginated query endpoint, rename/move/tag mutation — while every existing reference (stable UUID, ETag scheme, `AssetPicker`, the chat link-preview commit path, world export/import) keeps working.

**Architecture:** Server-first. The `assets` table grows metadata columns plus an `asset_tags` child table; all asset repository code moves into a sibling `impl` file (`data/sqlite/assets.rs`) so `sqlite.rs` stays under the soft file-size limit. Image work lives in `data::asset::process` (pure, `spawn_blocking`-hosted), derived tags in `data::asset::tags` (pure). HTTP gains three sibling modules under `http::assets` — `uploads` (chunk sessions), `query` (filters/regex/cursor), `mutate` (patch/bulk/reconvert/original). Folders are `asset_folder` engine documents; their only asset-side coupling is `assets.folder_id` and the reparent hook in the single delete chokepoint `delete_document_tx`. The client core mirrors the wire changes (ts-rs regenerates `Asset`; `wire.ts`/`assets.ts`/`asset-rest.ts` extend by hand) — the existing panel and picker are untouched consumers of the widened `Asset`.

**Tech Stack:** Rust (axum 0.8, sqlx 0.9/SQLite, tokio), `image` 0.25, `webp` 0.3 (libwebp), `regex` 1, ts-rs 12; TypeScript client core with Zod + vitest.

**Spec:** `docs/superpowers/specs/2026-08-30-m15-asset-pipeline-browser-design.md` (§1, §2, §3, §5 — §4 is M15b).

## Global Constraints

- All asset mutation routes are GM-only via `require_gm`; `serve` (and its `variant` form) stays membership-gated; `/original` is GM-only.
- Stable identity: `<uuid>` under `<assets_dir>/<world>/` remains the canonical served file; `storage_key` unchanged; ETag stays `"{id}-{version}"`.
- Create = file-first-then-row (`commit_staged_asset`); replace = row-first-then-file; every commit+file-op pair holds `state.write_barrier.read()` — never across a network-bound stream.
- `Config.retain_originals` default `true`; CLI `--retain-originals <bool>` > `SHADOWCAT_RETAIN_ORIGINALS` > TOML > default.
- Chunk size fixed at 8 MiB (`CHUNK_SIZE`); single-shot `POST /api/worlds/{world}/assets` stays and is what the link-preview pipeline and `AssetPicker` use.
- Derived tags: `image`/`webp`/`png`/`jpeg`/`gif`/`svg`/`other` kind tags, `gif-animated`/`animated`, `square`, `large` (either axis ≥ 2048), `transparent`, every ancestor folder name, `uploaded` | `link-preview`.
- Regex filter: pattern ≤ 256 bytes, `RegexBuilder::size_limit(1 << 20)`, `dfa_size_limit(1 << 20)`, applied only after SQL filters.
- Pre-ship migration convention: edit `src/server/migrations/0001_init.sql` in place (single squashed baseline, `48714bc2`); no backfill step exists.
- Cross-platform: no path-separator literals; every new crate builds on Linux/macOS/Windows (libwebp compiles via `cc` on all three CI runners).
- Rust tests live in sibling files (`#[cfg(test)] mod tests;`), every item documented (`#![deny(missing_docs)]`), no lint suppressions, no file over 5,000 lines.
- After every task: `cargo fmt --all`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all` (from `src/server`); `pnpm -r typecheck && pnpm -r test` once TS changes begin.

## Spec deviations flagged for the user (decided at handoff, recorded here)

1. **Folder delete and child folders.** Spec §1 says child folders reparent. The document layer's invariant (`apply_command` expands a `Delete` into explicit children-first ops; `descendants_first`) cascades every descendant document. This plan keeps the invariant: deleting a folder deletes its sub-folders (as logged ops) and reparents every *asset* in the deleted subtree to the deleted folder's parent — which the per-op hook in `delete_document_tx` produces naturally in children-first order (child's assets → parent; then parent's assets → grandparent).
2. **Backfill.** Spec §1's one-time backfill has no substrate — the repo runs a single pre-ship baseline migration edited in place. New columns get defaults in the DDL; no backfill task.

---

### Task 1: Dependencies + `Config.retain_originals`

**Files:**
- Modify: `src/server/Cargo.toml` (deps)
- Modify: `src/server/src/config.rs` (`Cli`, `Config`, `Default`, `load`)
- Test: `src/server/src/config/tests.rs`
- Modify: `docs/site/guides/hosting.md` (config table row)

**Interfaces:**
- Produces: `Config.retain_originals: bool`; `Cli.retain_originals: Option<bool>`.

- [ ] **Step 1: Add crates**

In `src/server/Cargo.toml` `[dependencies]`, after `url = "2"`:

```toml
# Asset pipeline (`data::asset::process`): decode/resize via `image` (pure Rust
# codecs; PNG/JPEG/GIF/BMP/TIFF/WebP decode), lossy + lossless WebP encode via
# libwebp (`webp`, built by `cc` on every CI runner — no system package). No
# FFmpeg; both crates are BSD-licensed.
image = { version = "0.25", default-features = false, features = ["png", "jpeg", "gif", "bmp", "tiff", "webp"] }
webp = "0.3"
# `GET /api/worlds/{world}/assets?name_regex=` — linear-time matcher with
# compile-size caps, so an untrusted pattern cannot stall the server.
regex = "1"
```

- [ ] **Step 2: Failing config test**

Append to `src/server/src/config/tests.rs`:

```rust
#[test]
fn retain_originals_defaults_true_and_cli_overrides() {
    let cfg = Config::load(Cli::default()).unwrap();
    assert!(cfg.retain_originals);
    let cli = Cli {
        retain_originals: Some(false),
        ..Cli::default()
    };
    let cfg = Config::load(cli).unwrap();
    assert!(!cfg.retain_originals);
}
```

- [ ] **Step 3: Run, expect compile failure** — `cargo test -p shadowcat config::tests::retain_originals` → "no field `retain_originals`".

- [ ] **Step 4: Implement**

`Cli` (after `force`):
```rust
    /// Keep the uploaded original beside the converted canonical file
    /// (`Config.retain_originals`); `--retain-originals false` discards it.
    #[arg(long)]
    pub retain_originals: Option<bool>,
```
`Config` (after `upload_rate_per_min_gm`):
```rust
    /// Whether a converted upload keeps its original bytes on disk as
    /// `<uuid>.orig` (reconvert + "download original" need it). Default
    /// `true`; `false` trades those for disk. Host-level: the host pays for
    /// the disk, so this is not a per-world setting.
    pub retain_originals: bool,
```
`Default`: `retain_originals: true,`. In `load`, after the `session_key` override block:
```rust
        if let Some(v) = cli.retain_originals {
            cfg.retain_originals = v;
        }
```

- [ ] **Step 5: Run tests** — `cargo test -p shadowcat config::` → PASS. `cargo build` (pulls the three crates; confirm `webp-sys` builds).

- [ ] **Step 6: Docs** — in `docs/site/guides/hosting.md`'s config table, after the `upload_max_bytes_gm` row:
```markdown
| `retain_originals` | `true` | Keep each converted upload's original bytes (`<uuid>.orig`) for reconvert/download; `false` saves disk |
```

- [ ] **Step 7: Commit** — `git commit -m "feat(config): retain_originals switch + image/webp/regex deps for the asset pipeline" -- src/server/Cargo.toml src/server/Cargo.lock src/server/src/config.rs src/server/src/config/tests.rs docs/site/guides/hosting.md`

---

### Task 2: Schema + `Asset` metadata + repository split

**Files:**
- Modify: `src/server/migrations/0001_init.sql` (assets DDL + `asset_tags`)
- Modify: `src/server/src/data/asset.rs` (`Asset` fields, `AssetMeta`, `NewAssetBytes`, `create_asset_from_bytes`)
- Create: `src/server/src/data/sqlite/assets.rs` (all asset repo fns, moved + extended)
- Modify: `src/server/src/data/sqlite.rs` (remove moved fns; `mod assets;`)
- Modify: `src/server/src/http/assets.rs` (`upload`/`replace` build the new struct)
- Modify: `src/server/src/chat/post_publish.rs` (provenance on `NewAssetBytes`)
- Test: `src/server/src/data/asset/tests.rs`, `src/server/src/data/sqlite/tests/assets.rs` (new)
- Regenerate: `src/types/generated/Asset.ts` (ts-rs, via `cargo test`)

**Interfaces:**
- Produces:
  ```rust
  pub struct AssetMeta { pub width: Option<u32>, pub height: Option<u32>, pub has_alpha: bool, pub animated: bool,
      pub original_content_type: String, pub original_byte_size: i64, pub original_retained: bool,
      pub conversion_note: Option<String> }
  pub struct Asset { …existing…, pub folder_id: Option<Uuid>, pub tags: Vec<String>, pub derived_tags: Vec<String>,
      #[serde(flatten)] pub meta: AssetMeta }   // flatten keeps the wire flat
  pub enum Provenance { Uploaded, LinkPreview }  // NewAssetBytes.provenance
  // repo (data/sqlite/assets.rs):
  insert_asset(&Asset), get_asset(id), delete_asset(id), list_assets_by_world(world),
  replace_asset_bytes(id, storage_key, content_type, byte_size, meta: &AssetMeta) -> version,
  set_asset_tags(id, explicit: &[String], derived: &[String]),      // replaces both sets, one tx
  ```

- [ ] **Step 1: DDL**

Replace the `assets` block in `0001_init.sql`:
```sql
CREATE TABLE assets (
  id                    TEXT PRIMARY KEY,
  world_id              TEXT NOT NULL REFERENCES worlds(id) ON DELETE CASCADE,
  storage_key           TEXT NOT NULL,
  original_name         TEXT NOT NULL,
  content_type          TEXT NOT NULL,
  byte_size             INTEGER NOT NULL,
  created_by            TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at            INTEGER NOT NULL,
  version               INTEGER NOT NULL,
  -- M15 pipeline metadata. folder_id names an `asset_folder` document; the
  -- FK SET NULL is a safety net only — `delete_document_tx` reparents first.
  folder_id             TEXT REFERENCES documents(id) ON DELETE SET NULL,
  width                 INTEGER,
  height                INTEGER,
  has_alpha             INTEGER NOT NULL DEFAULT 0,
  animated              INTEGER NOT NULL DEFAULT 0,
  original_content_type TEXT NOT NULL DEFAULT '',
  original_byte_size    INTEGER NOT NULL DEFAULT 0,
  original_retained     INTEGER NOT NULL DEFAULT 0,
  conversion_note       TEXT
);

CREATE INDEX idx_assets_world ON assets(world_id);
CREATE INDEX idx_assets_folder ON assets(folder_id);

-- Explicit (derived = 0) and derived (derived = 1) tags; derived rows are
-- rewritten on every commit/rename/move/reconvert and never client-writable.
CREATE TABLE asset_tags (
  asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  tag      TEXT NOT NULL,
  derived  INTEGER NOT NULL,
  PRIMARY KEY (asset_id, tag)
);
CREATE INDEX idx_asset_tags_tag ON asset_tags(tag, asset_id);
```
(`documents` is created before `assets` in the file — verify with `grep -n "CREATE TABLE" 0001_init.sql`.)

- [ ] **Step 2: Failing repo test**

Create `src/server/src/data/sqlite/tests/assets.rs` (register with `mod assets;` in `src/server/src/data/sqlite/tests/mod.rs`):
```rust
use crate::data::asset::{Asset, AssetMeta};
use crate::data::sqlite::SqliteRepository;

fn sample(world: uuid::Uuid) -> Asset {
    let id = uuid::Uuid::new_v4();
    Asset {
        id, world_id: world, storage_key: format!("{world}/{id}"),
        original_name: "map.png".into(), content_type: "image/webp".into(), byte_size: 10,
        created_by: None, created_at: 1, version: 1, folder_id: None,
        tags: vec![], derived_tags: vec![],
        meta: AssetMeta { width: Some(4), height: Some(4), has_alpha: true, animated: false,
            original_content_type: "image/png".into(), original_byte_size: 20,
            original_retained: true, conversion_note: None },
    }
}

#[tokio::test]
async fn asset_round_trips_meta_and_tags() {
    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let world = repo.create_world("w", 1).await.unwrap();
    let a = sample(world.id);
    repo.insert_asset(&a).await.unwrap();
    repo.set_asset_tags(a.id, &["hero".into()], &["image".into(), "square".into()]).await.unwrap();
    let got = repo.get_asset(a.id).await.unwrap().unwrap();
    assert_eq!(got.meta, a.meta);
    assert_eq!(got.tags, vec!["hero".to_string()]);
    assert_eq!(got.derived_tags, vec!["image".to_string(), "square".to_string()]);
    // set replaces, never accumulates
    repo.set_asset_tags(a.id, &[], &["image".into()]).await.unwrap();
    let got = repo.get_asset(a.id).await.unwrap().unwrap();
    assert!(got.tags.is_empty());
    assert_eq!(got.derived_tags, vec!["image".to_string()]);
}
```

- [ ] **Step 3: Run → compile failure** (`cargo test -p shadowcat sqlite::tests::assets`).

- [ ] **Step 4: Structs**

In `data/asset.rs`, add above `Asset`:
```rust
/// Pipeline-derived metadata recorded at commit (`data::asset::process`) and
/// rewritten on replace/reconvert. Flattened into `Asset` on the wire.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq)]
#[ts(export, export_to = "../../types/generated/")]
pub struct AssetMeta {
    /// Canonical pixel width; `None` for a non-image or undecodable file.
    pub width: Option<u32>,
    /// Canonical pixel height; `None` for a non-image or undecodable file.
    pub height: Option<u32>,
    /// Whether the source carried an alpha channel (drives lossless encoding + the `transparent` tag).
    pub has_alpha: bool,
    /// Whether the source is an animation (served pass-through; never re-encoded).
    pub animated: bool,
    /// MIME type of the bytes that ARRIVED (vs `Asset.content_type`, the served canonical).
    pub original_content_type: String,
    /// Size of the bytes that arrived (vs `Asset.byte_size`, the served canonical).
    pub original_byte_size: i64,
    /// Whether `<uuid>.orig` exists on disk (`Config.retain_originals` AND the upload was converted).
    pub original_retained: bool,
    /// Why the upload was stored pass-through instead of converted, if it was.
    pub conversion_note: Option<String>,
}
```
Add to `Asset` after `version`:
```rust
    /// Containing `asset_folder` document; `None` = world root.
    pub folder_id: Option<Uuid>,
    /// GM-set tags (client-writable via PATCH).
    pub tags: Vec<String>,
    /// Recomputed by `data::asset::tags::derive` on every commit/rename/move/reconvert; never client-writable.
    pub derived_tags: Vec<String>,
    /// Pipeline metadata (flattened onto the wire object).
    #[serde(flatten)]
    #[ts(flatten)]
    pub meta: AssetMeta,
```
Add `Provenance`:
```rust
/// Who authored an asset — feeds the `uploaded` / `link-preview` derived tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// A GM upload (single-shot or chunked).
    Uploaded,
    /// A server-fetched link-preview/oEmbed image (`chat::post_publish`).
    LinkPreview,
}
```
`NewAssetBytes` gains `pub provenance: Provenance,`. `create_asset_from_bytes` fills `folder_id: None, tags: vec![], derived_tags: vec![], meta: AssetMeta { width: None, height: None, has_alpha: false, animated: false, original_content_type: content_type.to_string(), original_byte_size: bytes.len() as i64, original_retained: false, conversion_note: None }` for now (Task 6 routes it through `process`). Update `post_publish.rs`'s two call sites with `provenance: Provenance::LinkPreview` and `http::assets::upload`/`replace` with the same default `meta` (Task 6 replaces).

- [ ] **Step 5: Repository split**

Create `src/server/src/data/sqlite/assets.rs` with `#![deny(missing_docs)]`, `use super::*;` and `impl SqliteRepository { … }` containing: `insert_asset` (all new columns bound; `has_alpha`/`animated`/`original_retained` as `i64` 0/1), `asset_from_row` (reads the new columns; `tags`/`derived_tags` empty — filled by `load_tags`), `get_asset` (row + `load_tags`), `delete_asset`, `list_assets_by_world` (+ tags per row via one `SELECT asset_id, tag, derived FROM asset_tags WHERE asset_id IN (…)` grouped in memory), `replace_asset_bytes(id, storage_key, content_type, byte_size, meta)` (also writes every meta column), `set_asset_tags` (one tx: `DELETE FROM asset_tags WHERE asset_id = ?` then batched inserts, `derived` 0 for explicit, 1 for derived), and a private `load_tags(&self, id) -> (Vec<String>, Vec<String>)` ordered by `tag`. Delete the originals from `sqlite.rs`; add `mod assets;` beside the existing `mod` declarations. `pub(super)` visibility for `asset_from_row` so `export_world` (still in `sqlite.rs`) compiles unchanged.

- [ ] **Step 6: Run** `cargo test -p shadowcat` — the repo test passes, ts-rs regenerates `src/types/generated/Asset.ts` + `AssetMeta.ts`; `git diff --stat src/types/generated` shows both. `pnpm -r typecheck` (the client reads the wider `Asset`; `Assets.svelte`/`AssetPicker` build unchanged).

- [ ] **Step 7: Commit** — `git commit -m "feat(assets): pipeline metadata columns, asset_tags, folder_id; asset repo split into sqlite/assets.rs" -- <listed files> src/types/generated/Asset.ts src/types/generated/AssetMeta.ts`

---

### Task 3: `asset_folder` engine document type

**Files:**
- Modify: `src/server/src/data/engine/mod.rs` (`ASSET_FOLDER_DOC_TYPE`, `is_engine_doc_type`, `validate_engine`)
- Create: `src/server/src/data/engine/asset_folder.rs`
- Modify: `src/server/src/data/validation.rs` (`validate_containment`)
- Modify: `src/server/src/data/sqlite.rs` (`apply_intent` Create/Update arms: parent-type + cycle check; `delete_document_tx`: reparent hook)
- Test: `src/server/src/data/engine/tests.rs`, `src/server/src/data/validation/tests.rs`, `src/server/src/data/sqlite/tests/assets.rs`

**Interfaces:**
- Produces: `ASSET_FOLDER_DOC_TYPE = "asset_folder"`; `AssetFolderEngine { sort: i64 }`; folder name = `Document.name`, parent = `Document.parent_id`; repo `folder_ancestor_names(tx, folder_id) -> Vec<String>` (root-first, used by Task 4).

- [ ] **Step 1: Failing tests**

`engine/tests.rs`:
```rust
#[test]
fn asset_folder_is_engine_type_with_sort_only() {
    assert!(is_engine_doc_type("asset_folder"));
    assert!(validate_engine("asset_folder", Some(&serde_json::json!({ "sort": 3 }))).is_ok());
    assert!(validate_engine("asset_folder", Some(&serde_json::json!({ "sort": 3, "name": "x" }))).is_err());
}
```
`validation/tests.rs`:
```rust
#[test]
fn asset_folder_cannot_be_embedded() {
    let mut actor = doc("actor", None);
    actor.embedded.insert("stuff".into(), vec![doc("asset_folder", None)]);
    assert!(validate_containment(&actor).is_err());
}
```
(`doc(ty, parent)` = the file's existing Document fixture helper; if none exists, add one building a minimal `Document` with `doc_type: ty.into()`, `parent_id: parent`, defaults elsewhere.)

`sqlite/tests/assets.rs`:
```rust
#[tokio::test]
async fn folder_delete_reparents_assets_and_cascades_subfolders() {
    // world with folders root->A->B; asset x in A, asset y in B; delete A
    // ⇒ A and B gone (logged ops), x.folder_id == None (A's parent), y.folder_id == None.
}
#[tokio::test]
async fn folder_parent_must_be_folder_and_acyclic() {
    // creating asset_folder with parent = an actor doc → Err
    // updating A.parent_id = B where B is A's child → Err (cycle)
}
```
Build these on the existing intent helpers in `sqlite/tests/` (grep `apply_intent(` for the pattern used by the combat containment tests and copy it).

- [ ] **Step 2: Run → fail.**

- [ ] **Step 3: Implement**

`engine/asset_folder.rs`:
```rust
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// An asset folder: a world document whose `name` is the folder name and whose
/// `parent_id` is the containing folder (`None` = world root). Assets point at
/// it via `assets.folder_id`. Only ordering lives in the engine band.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct AssetFolderEngine {
    /// Sibling sort key (ascending).
    pub sort: i64,
}
```
`engine/mod.rs`: `pub mod asset_folder;`, `pub const ASSET_FOLDER_DOC_TYPE: &str = "asset_folder";`, add `| "asset_folder"` to `is_engine_doc_type`, and `"asset_folder" => round_trip::<asset_folder::AssetFolderEngine>(v, "asset_folder"),` in `validate_engine`.

`validation::validate_containment`: add to the embedded-child check `|| child.doc_type == engine::ASSET_FOLDER_DOC_TYPE` (folders are never embedded).

`sqlite.rs` `apply_intent` Create arm (beside the combatant parent check) and Update arm (when the merged doc's `parent_id` changed):
```rust
if doc.doc_type == ASSET_FOLDER_DOC_TYPE {
    if let Some(pid) = doc.parent_id {
        let parent = Self::load_document(&mut *tx, pid).await?
            .filter(|p| p.doc_type == ASSET_FOLDER_DOC_TYPE && p.scope == doc.scope)
            .ok_or_else(|| DataError::OpFailed("asset_folder parent must be an asset_folder in the same world".into()))?;
        // Cycle guard: walk up from the parent; reaching `doc.id` means the
        // update would make the folder its own ancestor.
        let mut cur = Some(parent);
        let mut hops = 0;
        while let Some(p) = cur {
            if p.id == doc.id { return Err(DataError::OpFailed("asset_folder parent cycle".into())); }
            hops += 1;
            if hops > 64 { return Err(DataError::OpFailed("asset_folder nesting too deep".into())); }
            cur = match p.parent_id { Some(gp) => Self::load_document(&mut *tx, gp).await?, None => None };
        }
    }
}
```
`delete_document_tx`: before `DELETE FROM documents`, read `SELECT doc_type, parent_id FROM documents WHERE id = ?`; if `doc_type == ASSET_FOLDER_DOC_TYPE`, run `UPDATE assets SET folder_id = ? WHERE folder_id = ?` binding `(parent_id, id)`. Document the hook in the fn's doc comment (it is the single delete chokepoint, so both `apply_intent` and `apply_command` inherit it). Derived-tag recomputation for the reparented assets is Task 4's `refresh_derived_tags_for_folder_subtree` — call it here after the UPDATE.

Add to `sqlite/assets.rs`:
```rust
/// Root-first names of `folder_id` and its ancestors (empty for `None`). Feeds the folder-segment derived tags.
pub(crate) async fn folder_ancestor_names(tx: &mut sqlx::SqliteConnection, folder_id: Option<Uuid>) -> Result<Vec<String>, DataError>
```
(loop on `SELECT name, parent_id FROM documents WHERE id = ?`, cap 64 hops, reverse at the end).

- [ ] **Step 4: Run all tests → PASS.** ts-rs emits `src/types/generated/engine/AssetFolderEngine.ts`.
- [ ] **Step 5: Commit** — `feat(documents): asset_folder engine doc type with parent/cycle rules and delete-reparent hook`

---

### Task 4: Derived tags

**Files:**
- Create: `src/server/src/data/asset/tags.rs`, `src/server/src/data/asset/tags/tests.rs`
- Modify: `src/server/src/data/sqlite/assets.rs` (`refresh_derived_tags(id)`, `refresh_derived_tags_for_folder_subtree(tx, folder_id)`)

**Interfaces:**
- Produces:
  ```rust
  pub struct DeriveInput<'a> { pub content_type: &'a str, pub meta: &'a AssetMeta, pub folder_names: &'a [String], pub provenance: Provenance }
  pub fn derive(input: DeriveInput<'_>) -> Vec<String>   // sorted, deduped
  pub const LARGE_AXIS_PX: u32 = 2048;
  ```

- [ ] **Step 1: Failing tests** (`tags/tests.rs`):
```rust
#[test]
fn derives_kind_dimension_folder_and_provenance_tags() {
    let meta = AssetMeta { width: Some(2048), height: Some(2048), has_alpha: true, animated: false, original_content_type: "image/png".into(), original_byte_size: 1, original_retained: true, conversion_note: None };
    let tags = derive(DeriveInput { content_type: "image/webp", meta: &meta, folder_names: &["Maps".into(), "Crypt".into()], provenance: Provenance::Uploaded });
    assert_eq!(tags, vec!["Crypt", "Maps", "image", "large", "square", "transparent", "uploaded", "webp"]);
}
#[test]
fn animated_gif_passthrough_tags() {
    let meta = AssetMeta { width: Some(10), height: Some(20), has_alpha: false, animated: true, original_content_type: "image/gif".into(), original_byte_size: 1, original_retained: false, conversion_note: Some("animated".into()) };
    let tags = derive(DeriveInput { content_type: "image/gif", meta: &meta, folder_names: &[], provenance: Provenance::LinkPreview });
    assert_eq!(tags, vec!["animated", "gif", "gif-animated", "image", "link-preview"]);
}
#[test]
fn non_image_is_other() {
    let meta = AssetMeta { width: None, height: None, has_alpha: false, animated: false, original_content_type: "application/pdf".into(), original_byte_size: 1, original_retained: false, conversion_note: Some("not an image".into()) };
    assert_eq!(derive(DeriveInput { content_type: "application/pdf", meta: &meta, folder_names: &[], provenance: Provenance::Uploaded }), vec!["other", "uploaded"]);
}
```
- [ ] **Step 2: Run → fail.**
- [ ] **Step 3: Implement `derive`**: kind from `content_type` (`image/*` → `image` + subtype tag `webp|png|jpeg|gif|svg`, else `other`); `animated` (+ `gif-animated` when subtype is gif); `square` iff `width == height` (both `Some`); `large` iff either ≥ `LARGE_AXIS_PX`; `transparent` iff `has_alpha`; every folder name verbatim; provenance `uploaded`/`link-preview`. Collect into a `BTreeSet<String>` then `into_iter().collect()`.
- [ ] **Step 4: Repo helpers** in `sqlite/assets.rs`:
  - `refresh_derived_tags(&self, id)`: load asset, `folder_ancestor_names`, `derive`, then rewrite only `derived = 1` rows (delete + insert) in one tx. Provenance is recovered from the existing derived set (`link-preview` present ⇒ `LinkPreview`, else `Uploaded`) — add a private `provenance_of(&[String]) -> Provenance`.
  - `refresh_derived_tags_for_folder_subtree(tx, folder_id: Option<Uuid>)`: every asset whose `folder_id` is in the subtree rooted at `folder_id` (or all root assets for `None` — only the direct children, since root has no name) gets refreshed inside the caller's tx. Used by Task 3's delete hook and by folder rename/move (Task 10 wires the document-Update side).
- [ ] **Step 5: Tests PASS; commit** — `feat(assets): derived-tag computation + refresh helpers`

---

### Task 5: Image processing (`data::asset::process`)

**Files:**
- Create: `src/server/src/data/asset/process.rs`, `src/server/src/data/asset/process/tests.rs`

**Interfaces:**
- Produces:
  ```rust
  pub const THUMB_PX: u32 = 128; pub const PREVIEW_PX: u32 = 512; pub const LOSSY_QUALITY: f32 = 85.0;
  pub struct Processed { pub content_type: String, pub byte_size: i64, pub meta: AssetMeta, pub converted: bool }
  /// Blocking. `staged` holds the arrived bytes. On success the canonical bytes are at `staged` (rewritten in place
  /// when converted), the original (if converted && retain) at `<staged>.orig`, derivatives at
  /// `<staged>.thumb.webp` / `<staged>.preview.webp` (only when the source decoded).
  pub fn process_staged(staged: &Path, original_content_type: &str, original_byte_size: i64, retain_originals: bool) -> std::io::Result<Processed>
  pub fn write_derivatives(canonical: &Path, out_thumb: &Path, out_preview: &Path) -> std::io::Result<()>  // regenerate-on-demand entry
  pub fn derivative_path(canonical: &Path, variant: Variant) -> PathBuf; pub fn original_path(canonical: &Path) -> PathBuf
  pub enum Variant { Thumb, Preview }
  ```

- [ ] **Step 1: Failing tests** — generate fixtures with `image` in the test (no binary fixtures):
```rust
fn png_rgba(w: u32, h: u32) -> Vec<u8> { /* image::RgbaImage with one a=0 pixel → PNG bytes via write_to */ }
fn jpeg_rgb(w: u32, h: u32) -> Vec<u8> { /* RgbImage → JPEG */ }
fn gif_two_frames() -> Vec<u8> { /* image::codecs::gif::GifEncoder, two frames */ }

#[test] fn png_with_alpha_converts_lossless_and_retains_original() {
    // process_staged(retain=true) → content_type "image/webp", converted, meta.has_alpha, .orig exists and equals input,
    // thumb/preview exist and decode as WebP ≤ 128 / ≤ 512 on the long axis
}
#[test] fn jpeg_converts_lossy_without_alpha() { /* has_alpha false; byte_size < original for a 600x400 noise image? use a flat color: assert content_type webp */ }
#[test] fn retain_false_writes_no_orig() {}
#[test] fn animated_gif_is_passthrough_with_note() { /* content_type "image/gif", converted false, meta.animated, note "animated" , derivatives still written from frame 0 */ }
#[test] fn svg_and_undecodable_are_passthrough() { /* b"<svg/>" as image/svg+xml → note "svg"; b"garbage" as image/png → note starts "decode failed" ; width None */ }
#[test] fn write_derivatives_regenerates_from_canonical() {}
```
- [ ] **Step 2: Run → fail.**
- [ ] **Step 3: Implement** — decode with `image::ImageReader::open(staged)?.with_guessed_format()?`; animation: `image/gif` → `GifDecoder` + `into_frames().take(2).count() > 1`; `image/webp` → `WebPDecoder::has_animation()`. Pass-through cases: content_type `image/svg+xml` (note `"svg"`), animated (note `"animated"`), non-`image/*` (note `"not an image"`), decode error (note `format!("decode failed: {e}")`), already `image/webp` and not animated → still re-encode? **No**: treat existing static WebP as pass-through with note `None` and `converted: false` (nothing to gain). Conversion: `to_rgba8()`; scan alpha (`any(|p| p[3] < 255)`) → `has_alpha`; encode via `webp::Encoder::from_rgba(&buf, w, h)`; `encode_lossless()` when `has_alpha` or original subtype ∈ {png, gif, bmp, tiff}, else `encode(LOSSY_QUALITY)`; write to `<staged>.conv.tmp`, then if `retain` rename `staged → <staged>.orig` else remove, then rename `.conv.tmp → staged`. Derivatives: `resize(thumbnail)` with `FilterType::Triangle` to fit the box, encode lossy q80 (lossless if alpha), written via temp+rename. `meta.original_retained = converted && retain`. Errors from `webp` (`&str`) map to `io::Error::other`.
- [ ] **Step 4: Tests PASS; clippy clean; commit** — `feat(assets): WebP conversion, retained originals, thumb/preview derivatives`

---

### Task 6: Wire processing into the single-shot upload, replace, and the link-preview path

**Files:**
- Modify: `src/server/src/data/asset.rs` (`commit_staged_asset` → `commit_processed_asset`; `create_asset_from_bytes` runs `process` on `spawn_blocking`)
- Modify: `src/server/src/http/assets.rs` (`upload`, `replace`, `store_streamed` no longer rejects non-images — it records the sniffed type)
- Modify: `src/server/test-support/src/lib.rs` if a helper is needed for a multi-frame GIF fixture
- Test: `src/server/tests/assets.rs`, `src/server/src/data/asset/tests.rs`

**Interfaces:**
- `commit_staged_asset(repo, tmp, final, asset, derived: &[String])` — renames the canonical AND every sibling artifact (`.orig`, `.thumb.webp`, `.preview.webp`) from `tmp`'s stem to `final`'s stem, inserts the row, then `set_asset_tags(id, &[], derived)`. On row-insert failure removes all four files.
- `pub fn sibling_paths(canonical: &Path) -> [PathBuf; 3]` — `.orig`, `.thumb.webp`, `.preview.webp`; used by delete (Task 8) and the bundle (Task 12).

- [ ] **Step 1: Failing integration tests** (`tests/assets.rs`):
```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upload_converts_png_to_webp_and_keeps_original() {
    let h = spawn().await;
    let asset: serde_json::Value = h.upload("m.png", "image/png", PNG_1X1.to_vec()).await.json().await.unwrap();
    assert_eq!(asset["content_type"], "image/webp");
    assert_eq!(asset["original_content_type"], "image/png");
    assert_eq!(asset["original_retained"], true);
    assert!(asset["derived_tags"].as_array().unwrap().iter().any(|t| t == "webp"));
    let id = asset["id"].as_str().unwrap();
    let dir = h.assets_dir.join(h.world.to_string());
    assert!(dir.join(format!("{id}.orig")).exists());
    assert!(dir.join(format!("{id}.thumb.webp")).exists());
    // serve returns the canonical webp
    let res = h.client.get(format!("http://{}/api/assets/{id}", h.addr)).send().await.unwrap();
    assert_eq!(res.headers()["content-type"], "image/webp");
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upload_with_retain_false_has_no_orig() { /* spawn_with(|c| c.retain_originals = false) */ }
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upload_non_image_is_passthrough_other() {
    // b"%PDF-1.7" as application/pdf → 200, content_type "application/pdf", derived_tags contains "other", conversion_note "not an image"
}
```
**Note:** `upload_rejects_non_image_bytes` currently asserts 400 — spec §2 says an upload is never rejected for conversion reasons and non-image types are pass-through, so **rewrite that test** to assert pass-through (the file's doc comment too). Non-image uploads are GM-only already; the size cap still applies.

- [ ] **Step 2: Run → fail.**
- [ ] **Step 3: Implement** — `store_streamed` returns `(sniffed_or_declared_content_type: String, size, name)`: sniff via `detect_image_type`; if `None`, use the multipart part's declared content type (`field.content_type()`), falling back to `application/octet-stream`. `upload`: after streaming, `tokio::task::spawn_blocking(move || process_staged(&tmp, &ct, size, retain))`, build `Asset` from `Processed`, derived = `tags::derive(...)` with `folder_names` empty (single-shot uploads land in root; Task 7's chunked path carries a folder), provenance `Uploaded`; then under the barrier `commit_staged_asset(..., &derived)`. `replace`: same processing to a tmp stem; under the barrier: `replace_asset_bytes(..., &processed.meta)` then rename canonical + siblings over the existing ones (remove stale `.orig` when the new upload has none); `refresh_derived_tags(id)`; broadcast `Replaced`. `create_asset_from_bytes`: stage bytes, `spawn_blocking(process_staged)`, derive with `Provenance::LinkPreview`, commit.
- [ ] **Step 4: Tests PASS** (`cargo test --all`; the `link_preview` tests in `chat_*` still pass — the fetched image is now stored as WebP; check any test asserting `content_type == "image/png"` on a preview asset and update it to `image/webp`).
- [ ] **Step 5: Commit** — `feat(assets): route upload/replace/link-preview commits through the conversion pipeline`

---

### Task 7: Chunked upload sessions

**Files:**
- Create: `src/server/src/http/assets/uploads.rs`, `src/server/src/http/assets/uploads/tests.rs`
- Modify: `src/server/src/http/mod.rs` (`AppState.uploads`, routes, sweeper spawn in the state constructor's caller — find where `upload_rate` is built and add beside it)
- Modify: `src/server/src/ws/protocol.rs`? — no; REST only
- Test: `src/server/tests/assets_chunked.rs` (new integration file)

**Interfaces:**
- Produces:
  ```rust
  pub const CHUNK_SIZE: u64 = 8 * 1024 * 1024; pub const SESSION_IDLE_MS: i64 = 30 * 60 * 1000;
  pub struct UploadSession { pub id: Uuid, pub world: Uuid, pub user: Uuid, pub name: String, pub content_type: String,
      pub byte_size: u64, pub received: u64, pub folder_id: Option<Uuid>, pub tags: Vec<String>, pub staged: PathBuf,
      pub rate_hit_ms: i64, pub last_touch_ms: i64 }
  pub struct UploadSessions(Mutex<HashMap<Uuid, UploadSession>>);  // AppState.uploads: Arc<UploadSessions>
  impl UploadSessions { pub fn sweep(&self, now_ms: i64) -> Vec<PathBuf> /* expired staged files to remove */ }
  // routes: create_session, put_chunk, complete_session, abort_session
  // wire: POST /api/worlds/{world}/assets/uploads {name, content_type, byte_size, folder_id?, tags?} → 201 {upload_id, chunk_size}
  //       PUT  /api/assets/uploads/{id}/{offset}  (raw body ≤ CHUNK_SIZE) → 204 ; 409 on offset != received ; 413 over declared size
  //       POST /api/assets/uploads/{id}/complete → 200 Asset ; 409 if received != byte_size
  //       DELETE /api/assets/uploads/{id} → 204 (refunds the rate slot)
  ```

- [ ] **Step 1: Failing tests** — unit (`uploads/tests.rs`): `sweep_returns_only_idle_sessions`, `session_bound_to_user_and_world`. Integration (`tests/assets_chunked.rs`): push a `16 MiB + 1` zero-filled `application/octet-stream` blob (3 chunks; pass-through, so no decode cost) with `spawn_with(|c| c.upload_max_bytes_gm = Some(64 * 1024 * 1024))`. Assert: out-of-order offset → 409; re-sending the last accepted chunk's offset → 409 (idempotent retry is for a chunk that was *lost*, i.e. `offset == received`); complete → 200 with `content_type == "application/octet-stream"`, `byte_size == 16 MiB + 1`, `folder_id` echoed, explicit `tags` echoed and derived tags present; `POST complete` on a partial session → 409; abort → 204 and staged file gone; a second user (`add_player`) hitting `PUT` on the GM's session → 403.
- [ ] **Step 2: Run → fail.**
- [ ] **Step 3: Implement** — `create_session`: `require_gm`, rate `check` (store `rate_hit_ms`), `byte_size ≤ effective_max_bytes`, validate `folder_id` (if `Some`, `load_document` must be an `asset_folder` in `world` → else 422), create `<assets>/<world>/<uuid>.<rand>.tmp` empty file, insert. `put_chunk`: session lookup → 404; `user != session.user` → 403; `offset != received` → 409; body via `axum::body::Bytes` with `DefaultBodyLimit::max(CHUNK_SIZE as usize)` on the route; append with `OpenOptions::append`; `received += len`; `received > byte_size` → 413 + abort. `complete_session`: `received != byte_size` → 409; remove the session from the map (so a concurrent complete 404s); sniff the first 12 bytes from the file for `content_type`; `spawn_blocking(process_staged)`; `folder_names = folder_ancestor_names`; derive; under barrier `commit_staged_asset`; then `set_asset_tags(id, &session.tags, &derived)`; broadcast `AssetChanged { op: Created, version: 1 }` (Task 10 adds the variant — until then use no broadcast and add it in Task 10; keep a `// Task 10` marker out of the code: just add the broadcast when the variant exists). `abort_session`: remove + delete file + `refund(user, rate_hit_ms)`. Sweeper: `tokio::spawn` loop every 60 s in `http::mod`'s state builder: `for p in state.uploads.sweep(now) { let _ = tokio::fs::remove_file(p).await; }` and refund each. Routes in `http/mod.rs`:
```rust
.route("/api/worlds/{world}/assets/uploads", post(assets::uploads::create_session))
.route("/api/assets/uploads/{id}/{offset}", put(assets::uploads::put_chunk).layer(DefaultBodyLimit::max(assets::uploads::CHUNK_SIZE as usize)))
.route("/api/assets/uploads/{id}/complete", post(assets::uploads::complete_session))
.route("/api/assets/uploads/{id}", delete(assets::uploads::abort_session))
```
- [ ] **Step 4: Tests PASS; commit** — `feat(assets): resumable chunked upload sessions`

---

### Task 8: Serve variants, original download, reconvert, full delete

**Files:**
- Modify: `src/server/src/http/assets.rs` (`serve` gains `variant`; `delete` removes siblings)
- Create: `src/server/src/http/assets/mutate.rs` (`original`, `reconvert`) + tests
- Modify: `src/server/src/http/mod.rs` (routes)
- Test: `src/server/tests/assets.rs`

**Interfaces:**
- `GET /api/assets/{uuid}?variant=thumb|preview` → WebP bytes, same ETag; missing derivative regenerated via `write_derivatives` on `spawn_blocking` (404 only if the canonical does not decode — then the route falls back to serving the canonical).
- `GET /api/assets/{uuid}/original` (GM) → `.orig` with `original_content_type`, `Content-Disposition: attachment; filename="<original_name>"`; 404 when `!original_retained`.
- `POST /api/assets/{uuid}/reconvert` (GM) → re-runs `process_staged` on a copy of `.orig` (404 when not retained), row-first `replace_asset_bytes` + sibling swap under the barrier, `refresh_derived_tags`, broadcast `Replaced`, returns `Asset`.

- [ ] **Step 1: Failing tests** — `serve_variant_thumb_is_webp_and_regenerates_when_missing` (delete the `.thumb.webp` file, request `?variant=thumb`, 200 + file recreated), `original_route_is_gm_only_and_404_when_not_retained`, `reconvert_bumps_version_and_broadcasts_replaced` (use the existing `drain_until_type(ws, "asset_changed")` pattern from the replace test), `delete_removes_canonical_and_siblings`.
- [ ] **Step 2: Run → fail.**
- [ ] **Step 3: Implement** with `Query<ServeQuery { variant: Option<String> }>`; unknown variant → 400. Delete: after `delete_asset`, remove canonical + `sibling_paths` (each best-effort, warn on error).
- [ ] **Step 4: Tests PASS; commit** — `feat(assets): derivative serving, original download, reconvert, full-file delete`

---

### Task 9: Query endpoint

**Files:**
- Create: `src/server/src/http/assets/query.rs`, `src/server/src/http/assets/query/tests.rs`
- Modify: `src/server/src/data/sqlite/assets.rs` (`query_assets`)
- Modify: `src/server/src/http/assets.rs` (`list` delegates to `query`)
- Test: `src/server/tests/assets_query.rs` (new)

**Interfaces:**
- Produces:
  ```rust
  #[derive(Deserialize)] pub struct AssetQuery { pub folder: Option<String> /* uuid | "root" */, pub recursive: Option<bool>, pub tags: Option<String>,
      pub kind: Option<String>, pub name: Option<String>, pub name_regex: Option<String>, pub sort: Option<String>, pub limit: Option<u32>, pub cursor: Option<String> }
  pub struct AssetPage { pub items: Vec<Asset>, pub next_cursor: Option<String> }   // TS-exported
  pub enum AssetSort { Name, Created, Size }  // default Created
  pub fn compile_regex(pattern: &str) -> Result<regex::Regex, AppError>  // ≤256 bytes, size limits; 400 on failure
  pub fn encode_cursor(sort_key: &str, id: Uuid) -> String / decode_cursor
  // repo: query_assets(world, filter: &AssetFilter, sort, after: Option<(String, Uuid)>, limit) -> Vec<Asset>  (limit+1 fetched to detect more)
  ```
  **Wire compatibility:** `GET /api/worlds/{world}/assets` with **no** query params keeps returning a bare `Asset[]` (the existing `listAssets`/`Assets.svelte`/`AssetPicker`/e2e contract). Any query param present ⇒ `AssetPage`. `limit` default 200, max 500.

- [ ] **Step 1: Failing tests** — unit: `compile_regex_rejects_oversize_pattern`, `cursor_round_trips`. Integration: seed 5 assets across two folders + tags via the repo (`insert_asset` + `set_asset_tags`), then assert `?folder=root`, `?folder=<A>&recursive=true`, `?tags=hero,image`, `?kind=other`, `?name=MAP` (case-insensitive), `?name_regex=^cr.pt$`, `?sort=size&limit=2` + follow `next_cursor` to the end, and bare list still returns an array.
- [ ] **Step 2: Run → fail.**
- [ ] **Step 3: Implement** — SQL built with a `QueryBuilder`: `FROM assets a WHERE a.world_id = ?` + `folder_id IS NULL` / `= ?` / `IN (subtree ids)` (subtree via a recursive CTE over `documents.parent_id` where `doc_type = 'asset_folder'`) + tags via `EXISTS (SELECT 1 FROM asset_tags t WHERE t.asset_id = a.id AND t.tag = ?)` per tag (all-of) + `kind`: `image` ⇒ `content_type LIKE 'image/%'`, `other` ⇒ `NOT LIKE` + `name`: `lower(original_name) LIKE '%' || lower(?) || '%'` + keyset `(sort_col, id) > (?, ?)` + `ORDER BY sort_col, id LIMIT ?+1`. Regex: applied in Rust to `original_name` over the SQL result, re-fetching the next SQL page until `limit` matches or rows run out (bounded by 10 SQL pages per request; then return what was found with a cursor).
- [ ] **Step 4: Tests PASS; ts-rs emits `AssetPage.ts`; commit** — `feat(assets): filtered, sorted, keyset-paginated asset query with size-capped regex`

---

### Task 10: PATCH / bulk mutation + `AssetChanged` Created/Moved

**Files:**
- Modify: `src/server/src/ws/protocol.rs` (`AssetOp::{Created, Moved}`)
- Modify: `src/server/src/http/assets/mutate.rs` (`patch`, `bulk`), `src/server/src/http/mod.rs` (routes)
- Modify: `src/server/src/data/sqlite/assets.rs` (`update_asset_placement(id, name, folder_id, tags)`, `bulk_update_assets`)
- Modify: `src/server/src/data/sqlite.rs` (`apply_intent` Update arm: when an `asset_folder`'s `name` or `parent_id` changed ⇒ `refresh_derived_tags_for_folder_subtree(tx, Some(doc.id))`)
- Modify: `src/server/src/http/assets/uploads.rs` (`complete_session` broadcasts `Created`), `src/server/src/http/assets.rs` (`upload` broadcasts `Created`)
- Test: `src/server/tests/assets_mutate.rs` (new), `docs/site/protocol.md` row

**Interfaces:**
- `PATCH /api/assets/{uuid}` body `{ name?: String, folder_id?: Option<Uuid> /* explicit null = root */, tags?: Vec<String> }` → `Asset`; 422 when `folder_id` names a non-folder or another world's folder; tags: trimmed, non-empty, ≤ 64 chars, ≤ 64 tags → else 422.
- `POST /api/worlds/{world}/assets/bulk` body `{ ids: Vec<Uuid>, folder_id?: Option<Uuid>, add_tags?: Vec<String>, remove_tags?: Vec<String> }` → `Vec<Asset>`; every id must belong to `world` (404 otherwise); one transaction; one `Moved` broadcast per id.
- `AssetOp::Created` (version 1) and `AssetOp::Moved` (current version, unchanged).

- [ ] **Step 1: Failing tests** — `patch_renames_moves_and_retags_and_broadcasts_moved` (assert `derived_tags` contains the new folder's name and `Moved` frame arrives with the unchanged version), `patch_is_gm_only`, `patch_rejects_cross_world_folder`, `bulk_moves_and_adds_tags_in_one_tx`, `folder_rename_refreshes_contained_assets_derived_tags` (rename the folder document over WS with the existing intent helper, then `get_asset` shows the new name tag).
- [ ] **Step 2: Run → fail.**
- [ ] **Step 3: Implement**; protocol enum gains the two variants with doc comments; `docs/site/protocol.md` `asset_changed` row becomes "an asset was `created`, `replaced`, `moved` (name/folder/tags; version unchanged) or `deleted`".
- [ ] **Step 4: Tests PASS** (incl. `pnpm -r typecheck` — the ts-rs `AssetOp.ts` widened; the client `wire.ts` Zod enum still rejects the new ops → fixed in Task 11, so run only `cargo` gates here; the client stays red until Task 11 lands **in the same push**).
- [ ] **Step 5: Commit** — `feat(assets): PATCH/bulk placement mutation; AssetChanged created/moved`

---

### Task 11: Client core — wire, resolver, REST, chunked upload

**Files:**
- Modify: `src/client/core/src/wire.ts` (`asset_changed.op` enum), `src/client/core/src/assets.ts` (`AssetOp`, `url(uuid, variant?)`, `Created`/`Moved` handling, `onListingInvalidated`)
- Modify: `src/client/core/src/asset-rest.ts` (`queryAssets`, `patchAsset`, `bulkPatchAssets`, `reconvertAsset`, `originalUrl`)
- Create: `src/client/core/src/asset-upload.ts` (`startChunkedUpload`), `src/client/core/src/asset-upload.test.ts`
- Modify: `src/client/core/src/index.ts` (exports), `src/client/core/src/assets.test.ts`, `src/client/core/src/asset-rest.test.ts` (if present; else create)

**Interfaces:**
```ts
export type AssetOp = "created" | "replaced" | "moved" | "deleted";
export type AssetVariant = "thumb" | "preview";
AssetResolver.url(uuid: string, variant?: AssetVariant): string   // `?variant=thumb&v=N`
AssetResolver.onListingInvalidated(cb: (uuid: string, op: AssetOp) => void): () => void  // fires for created/moved/deleted
export interface AssetQuery { folder?: string | "root"; recursive?: boolean; tags?: string[]; kind?: "image" | "other"; name?: string; nameRegex?: string; sort?: "name" | "created" | "size"; limit?: number; cursor?: string }
export interface AssetPage { items: Asset[]; next_cursor: string | null }
export function queryAssets(world: string, q: AssetQuery): Promise<AssetPage>
export function patchAsset(uuid: string, patch: { name?: string; folder_id?: string | null; tags?: string[] }): Promise<Asset>
export function bulkPatchAssets(world: string, body: { ids: string[]; folder_id?: string | null; add_tags?: string[]; remove_tags?: string[] }): Promise<Asset[]>
export function reconvertAsset(uuid: string): Promise<Asset>
export function originalUrl(uuid: string): string
export interface ChunkedUploadOptions { folderId?: string | null; tags?: string[]; onProgress?(sent: number, total: number): void; signal?: AbortSignal; fetchImpl?: typeof fetch; retries?: number /* per chunk, default 3 */ }
export function startChunkedUpload(world: string, file: File, opts?: ChunkedUploadOptions): Promise<Asset>
// ≤ 8 MiB ⇒ single-shot uploadAsset (folder/tags applied via a follow-up patchAsset when given); else session → PUT loop with per-chunk retry (offset re-sent on network error; a 409 re-syncs `sent` from a GET? — no GET exists: on 409 the client aborts the session and throws) → complete.
```
- [ ] **Step 1: Failing tests** — `assets.test.ts`: `url_with_variant_carries_variant_and_rev`, `created_and_moved_notify_listing_but_never_change_url`; `asset-upload.test.ts`: mocked `fetchImpl` recording calls — a 20-byte file with a mocked `chunk_size: 8` produces PUTs at offsets 0/8/16 then complete; a failed PUT (network throw) is retried at the same offset; abort signal → DELETE issued and rejection; ≤ threshold file uses single-shot POST.
- [ ] **Step 2: Run → fail** (`pnpm --filter @shadowcat/core test`).
- [ ] **Step 3: Implement**; keep `uploadAsset`/`listAssets`/`replaceAsset`/`deleteAsset` signatures unchanged. `wire.ts`: `op: z.enum(["created", "replaced", "moved", "deleted"])` and the TS union.
- [ ] **Step 4: `pnpm -r typecheck && pnpm -r test` PASS; commit** — `feat(core): asset query/patch/bulk REST, variant URLs, chunked upload client`

---

### Task 12: World bundle — siblings + metadata + tags

**Files:**
- Modify: `src/server/src/data/world_bundle.rs` (`ExportedAssetRow` gains `folder_id`, `tags`, `derived_tags`, `AssetMeta` fields; `ExportedAssetRow` doc), `src/server/src/world_bundle.rs` (`write_bundle` appends `assets/<id>.orig|.thumb.webp|.preview.webp` when present; `read_bundle` stages any `assets/<id>.<suffix>` entry with the same staging scheme, suffix-preserving), `src/server/src/data/sqlite.rs` (`export_world` selects the new columns + tags; `import_world` inserts them and finalizes siblings; a missing `.orig` ⇒ `original_retained = false`)
- Test: `src/server/tests/world_snapshot.rs` or the bundle's existing round-trip test file (grep `read_bundle(` under `src/server/src` and `tests/` to find it) — add `bundle_round_trips_asset_siblings_and_tags` and `import_without_orig_clears_original_retained`.

- [ ] Steps: failing test → run → implement (bundle entry name parsing: `assets/<uuid>` exact ⇒ canonical; `assets/<uuid>.orig` / `.thumb.webp` / `.preview.webp` ⇒ sibling; anything else under `assets/` ⇒ `Malformed`) → PASS → commit `feat(world-bundle): export/import asset originals, derivatives, folder, tags`.

---

### Task 13: Existing consumers + e2e stay green

**Files:**
- Modify: `src/modules/assets/src/Assets.svelte` (only if the widened `Asset` or `AssetOp` breaks a type — expected none), `src/modules/scene-tools/src/AssetPicker.svelte` (same)
- Verify: `src/client/shell/e2e/assets.spec.ts`, `stage.spec.ts` (upload → tile → place still passes; served art is now WebP)
- Modify: `docs/site/modules/assets.md` — add: "Uploads are converted to WebP with retained originals; see the asset pipeline in `docs/design/ARCHITECTURE.md` §4"; no contribution change.

- [ ] Spec §5's Playwright e2e ("upload a >1-chunk file and find it by tag") needs the M15b browser UI — M15a has no tag UI to find it with; the chunked path is covered by `tests/assets_chunked.rs` here and the e2e lands in M15b.
- [ ] Run `pnpm build`, `cargo build -p shadowcat --bin test_server`, `pnpm --filter @shadowcat/core test:e2e`, `pnpm --filter @shadowcat/shell e2e`. Fix any failure at the cause (a test asserting `image/png` on served art is updated to `image/webp`). Commit `test(e2e): assets flow over the conversion pipeline`.

---

### Task 14: Documentation, skill, graph sync

- [ ] `docs/design/ARCHITECTURE.md` §4 deferral table: image conversion row moves from "deferred" to realized (M15a); asset browser row stays (M15b). §6 stable-identity paragraph gains the sibling-file layout sentence.
- [ ] `docs/HISTORY.md`: M15a entry (routes, layout, invariants, the two flagged deviations and their resolution). `docs/PLAN.md` M15: mark M15a done, M15b remaining.
- [ ] `docs/site/guides/hosting.md` (Task 1 row present), `docs/site/protocol.md` (Task 10 row present).
- [ ] Skill update in `~/.claude/skills/shadowcat-codebase/skills/shadowcat-codebase-assets/SKILL.md`: key files (`data::asset::{process,tags}`, `http::assets::{uploads,query,mutate}`, `data::sqlite::assets`), invariants (sibling-file set, `delete_document_tx` reparent hook, bare-list vs `AssetPage` wire split, chunk session ownership, regex caps), gotchas (`Created`/`Moved` never change the URL; static WebP is pass-through). Dispatch `shadowcat-codebase:shadowcat-spec-reviewer` (effort high) on the skill diff; run `node scripts/check-skill-symbol-refs-cli.mjs` and `pnpm run test:scripts`; commit + push in the plugin repo.
- [ ] `graphify update .`
- [ ] Full local CI: `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all`, `git diff --exit-code src/types/generated`, `pnpm -r typecheck`, `pnpm -r test`, `pnpm run test:scripts`, `pnpm docs:check-examples`, `pnpm lint`, `pnpm lint:docs`, `pnpm lint:props`, `pnpm lint:comments`, `pnpm lint:allowances`, `pnpm lint:file-size`, `pnpm lint:inline-tests`, `pnpm check:svelte-runtime`, Rust doc-coverage clippy line from `ci.yml`. Commit docs `docs(m15a): asset pipeline delivery notes + skill sync`.

## Model/Effort directives

Fable-class session: plan written and executed mainline (`mainline-plan-execution`). Subagent use limited to the final fresh-context review pair (`shadowcat-codebase:shadowcat-spec-reviewer` + `shadowcat-codebase:shadowcat-code-reviewer`, effort high) and the skill-diff review; opus twins are not used (standing directive: sonnet-retry → user).

## Buddy-check directives

High-risk signals present: authz surface (new GM-only routes + a membership-gated variant route), a two-store commit ordering change (sibling files), and a document-layer delete hook. Buddy-check the **final branch diff** (two blind reviewers + brokered debate) before merge, scoped to `src/server/src/{data/asset*,data/sqlite/assets.rs,http/assets*,data/sqlite.rs delete_document_tx + apply_intent folder arms}`.
