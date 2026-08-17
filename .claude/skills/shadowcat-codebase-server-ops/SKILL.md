---
name: shadowcat-codebase-server-ops
description: "Use when touching Shadowcat's server bootstrap/config/CLI/deployment surface: the `main` module (entry point, early one-shot CLI branches), the `config` module (`Cli`/`Config` layering: CLI flag > SHADOWCAT_* env > TOML > default), the `db` module (single-connection SqlitePool open), or the `backup` module (whole-server VACUUM-INTO backup/restore). Covers the single-binary deployment story, not any one data/document subsystem. Invoke shadowcat-codebase-core first."
---

# Shadowcat — Server Bootstrap, Config, and Backup/Restore

Orientation for the parts of the server crate that exist ABOVE any one data subsystem: how the
binary starts, how configuration is resolved, and how a deployment's data is snapshotted/restored.

## Purpose

`main` is the single entry point for the `shadowcat` binary — normal server startup AND the
one-shot `--backup-to`/`--restore-from` CLI modes share it, mutually exclusive with each other and
with serving. `config` resolves the effective `Config` from four layered sources. `backup`
is a pure-I/O module (no `AppState`/`SqliteRepository` dependency) providing whole-server backup
and restore as a deployment-operator tool, not an in-app feature.

## Key files & seams

- `config` — `Cli` (flat `clap::Parser` struct, no `clap::Subcommand`) →
  `Config::load(cli)` layers CLI flag > `SHADOWCAT_*` env > TOML file > built-in default.
  `Config.db: String` (default `./shadowcat.db`), `Config.assets_dir: Option<String>` (`None` →
  sibling `assets/` beside the db file, via `Config::assets_path()`). `Cli.backup_to`/
  `restore_from: Option<String>` and `Cli.force: bool` are CLI-ONLY triggers — never on `Config`,
  never read from TOML/env (a one-shot operation is not persistent server configuration).
- `main` — `main()`'s FIRST branch: if both `backup_to` and `restore_from` are
  `Some`, `anyhow::bail!` before `Config::load` even runs. The three fields are cloned OUT of
  `main::cli` before `Config::load(cli)` consumes it by value. Either flag alone short-circuits to
  `run_backup`/`run_restore` and `return Ok(())` — `SqliteRepository::connect` (the long-lived
  pool) and `axum::serve` are structurally unreachable on that path, not just conditionally
  skipped.
- `db` — `open_pool(url)`: `SqlitePoolOptions::max_connections(1)`. **Dead
  code** — its only caller is its own unit test; production startup (`main`) and
  `backup::create_backup`/`backup::restore_backup` (plus `backup`'s own `tests` module) all open
  pools independently. The whole server's actual
  single-connection invariant lives in `data::sqlite::SqliteRepository::connect`
  ([[shadowcat-codebase-documents-permissions]]), which
  separately sets the same `max_connections(1)` — editing `db` does NOT affect the live
  server's connection pool.
- `backup` — `BackupManifest`, `BackupError`, `dir_is_empty_or_absent`,
  `create_backup(db_path, assets_dir, out_dir) -> Result<BackupManifest, BackupError>`,
  `restore_backup(backup_dir, db_path, assets_dir, force) -> Result<(), BackupError>`. Opens its
  own short-lived `SqlitePool` directly (does not reuse `SqliteRepository`/`AppState`) — pure
  file I/O + one SQL statement, deliberately decoupled from the rest of the server so it works
  even when `main()`'s normal startup path never runs.
- `POST /api/admin/backup` (`http::routes::admin_backup`, admin-only via `AdminUser`)
  — in-server backup trigger, layered ABOVE `backup`. Writes into
  `Config::backups_path()` (`config`, `None` → sibling `backups/` beside the db file, mirroring
  `assets_path()`'s convention), one timestamped subdirectory per run. Holds `AppState.write_barrier`
  (`Arc<tokio::sync::RwLock<()>>`, `http`) in WRITE mode across the whole snapshot; asset
  `upload`/`replace` (`http::assets`) each acquire it in READ mode around their own commit+rename
  step, so no asset write can interleave with an in-server backup's file copy. DB writers need no
  gating — `VACUUM INTO` is transactionally consistent against a live writer on its own.

## Hard invariants

- **`VACUUM INTO`, never a raw `.db` file copy** — a raw byte-copy of a live SQLite file is unsafe
  (a concurrent writer or WAL journal can leave it mid-write); `VACUUM INTO` is SQLite's own
  atomic, consistency-guaranteed live-snapshot primitive.
- **Assets copy ALWAYS runs after the db snapshot, never before/concurrently** — asset uploads
  write bytes to disk BEFORE inserting the referencing DB row
  ([[shadowcat-codebase-assets]]-adjacent: `http::assets` create path), and asset files are
  never deleted except by explicit delete, so db-then-assets ordering guarantees every asset a
  snapshot's rows reference is already present in the assets copy. `manifest.json` is written
  last, after both.
- **Two backup surfaces, not one.** The CLI one-shot mode (`--backup-to`/`--restore-from`,
  cross-process, invokable from cron/Task Scheduler/systemd-timer with no running server) remains
  the ONLY restore path — restore never runs in-server (see below). Backup ALSO has an in-server
  admin route (`POST /api/admin/backup`) because a cross-process CLI invocation cannot
  reach the live process's `write_barrier` to quiesce concurrent asset writes; the in-server route
  can. Anything needing a write-quiesced backup (e.g. a future scheduled-backup feature) must use
  the admin route, not the CLI mode.
- **Fail-closed restore**: `restore_backup` validates `manifest.json` + `world.db` presence
  BEFORE touching any destination file — a missing/malformed/foreign backup directory returns
  `BackupError::InvalidBackupDir` with zero destination writes.
- **Force-gated overwrite, both directions**: `--backup-to` refuses a non-empty output directory
  without `--force`; `--restore-from` refuses when the destination db file already exists OR the
  destination assets dir exists and is non-empty, without `--force`. A rejected restore is
  structurally inert — the control flow cannot reach any destination-mutating call before the
  gate's `return Err`. Asymmetric ownership: `restore_backup` enforces its own gate internally
  regardless of caller, but `create_backup` does NOT check `create_backup::out_dir` for prior contents — the
  refuse-non-empty gate for backup lives at the CLI layer (`main::run_backup`, via the
  exported `dir_is_empty_or_absent`). A future caller invoking `create_backup` directly (e.g. an
  in-app export feature) would bypass that gate.
- **Restore never starts the server** — restore and serve are always two separate invocations
  (a live connection can't safely have a different file swapped in as its backing file; Windows
  can even fail that swap outright on an open handle).
- **No shell-out for the recursive directory copy** (`tokio::fs` walks only — no `cp -r`/`xcopy`/
  `robocopy`), every path built via `Path`/`PathBuf::join` — cross-platform invariants per project
  CLAUDE.md, verified by a dedicated nested-directory (3+ levels) round-trip test.

## Gotchas

- **Docs-ratchet is live in this subsystem:** the `config`, `db`, `backup`,
  `modules`, `main`, and `bin::test_server` modules all carry `#![deny(missing_docs)]` +
  `#![deny(clippy::missing_docs_in_private_items)]` — a new item without a doc comment fails the
  3-OS CI clippy step. Every lib function also carries a `# Examples` doctest (`no_run` for
  infra-bound; bins use ` ```text ` — rustdoc runs no doctests for bin targets). The crate root has
  NO deny attr (a crate-root inner attr would flip the whole crate early — that's the final ratchet).
- `backup::copy_dir_recursive` silently skips symlinks (documented on the function itself)
  — the assets tree is server-managed and never contains one today, so this avoids following into
  an unexpected target rather than guessing at semantics. Revisit if `assets_dir` is ever pointed
  at a symlinked/shared directory.
- `sqlx = 0.9`'s `SqlSafeStr` bound rejects a bare dynamic `String` passed to `sqlx::query(...)` —
  a `VACUUM INTO '<dynamic path>'` string needs `sqlx::AssertSqlSafe(...)`, the documented 0.9
  audit escape hatch, NOT a bound parameter (bind params aren't valid in the `VACUUM INTO`
  filename position across driver versions). Safe here specifically because the interpolated
  value is a server-operator-supplied CLI path (never network-derived) and is single-quote-escaped
  (`.replace('\'', "''")`) before interpolation — re-verify both conditions still hold if this
  code is ever reused somewhere the input could be less trusted.
- `cargo fmt` with a path argument still reformats the WHOLE crate if not scoped correctly,
  leaving unrelated drift across modules the change never touched. Use `cargo fmt --check` first,
  or scope explicitly, and diff before committing.
- `restore_backup`'s destination writes are a stage-then-swap, not an in-place write: the db
  copies to `<db_path>.restore-tmp` then a single `rename` swaps it in (rename atomically replaces
  an existing FILE on all three target OSes); the assets tree copies to
  `<assets_dir>.restore-tmp`, the live `assets_dir` renames out to `<assets_dir>.restore-old`
  (directory rename does NOT replace a non-empty destination on any target OS, hence the two-step
  swap), the staged tree renames into `assets_dir`, then `.restore-old` is removed. A failure at
  any point leaves `restore_backup::db_path` either fully pre-restore or fully post-restore, and independently
  leaves `assets_dir` either fully pre-restore or fully post-restore — worst case
  (crash between the two directory renames) parks the old tree at `.restore-old`, which the next
  restore attempt clears before staging. No `--force`-only special case: both paths use the
  staging protocol regardless of `force`, since without `force` the pre-restore-destination-empty
  gate has already run. The db swap and the assets swap are two INDEPENDENT atomic operations,
  not one joint transaction — the db rename completes in full before the assets copy/swap starts,
  so a crash in that window pairs a new db with old (or momentarily absent) assets; recovery is
  re-running `restore_backup` with `force` (the db swap already completed, so a force-less retry
  would refuse on the now-existing `restore_backup::db_path`).
- The CLI backup mode (`create_backup` invoked directly by `main::run_backup`, cross-process,
  no live server) still has NO write-quiesce — its assets-copy is not transactionally coupled to
  the `VACUUM INTO` snapshot, so a CLI backup racing an external process's in-flight asset REPLACE
  can capture updated metadata with pre-replace bytes for a brief window
  ([[commit-db-row-before-swapping-file]]). The in-server `POST /api/admin/backup` route closes
  this for THAT invocation path via `write_barrier` (see Key files & seams / Hard invariants
  above) — the residual gap is CLI-mode-only and inherent to backing up while a separate process
  writes assets outside the barrier's reach.
- Per-world granular export/import is explicitly OUT of scope — the backup/restore surface ships
  whole-server snapshot/restore only (single shared `shadowcat.db` across all worlds); per-world
  would need to preserve referential integrity across cross-table FKs and shared asset
  references, real complexity not currently implemented.

## Pointers

- **Generated API** — `/api/rust/shadowcat/config/`, `/api/rust/shadowcat/db/`,
  `/api/rust/shadowcat/backup/` (rustdoc, private items included); the `main` module's own doc
  comment is on the crate root, `/api/rust/shadowcat/` (no `main` module has its own generated
  page — it's a binary entry point, not a documented public item). Produce with `pnpm build:all`.
- This subsystem is classified as file I/O + one SQL statement risk, not the
  security/concurrency/determinism risk class that requires independent review.
- Relationships: `graphify query "config cli main backup restore server bootstrap"`.
- Data-layer side (what `create_backup::db_path`/`Config.assets_dir` ultimately point at): [[shadowcat-codebase-assets]],
  [[shadowcat-codebase-documents-permissions]] (`SqliteRepository`, `src/server/src/data/`).
