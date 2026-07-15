---
name: shadowcat-codebase-server-ops
description: "Use when touching Shadowcat's server bootstrap/config/CLI/deployment surface: `src/server/src/main.rs` (entry point, early one-shot CLI branches), `src/server/src/config.rs` (`Cli`/`Config` layering: CLI flag > SHADOWCAT_* env > TOML > default), `src/server/src/db.rs` (single-connection SqlitePool open), or `src/server/src/backup.rs` (whole-server VACUUM-INTO backup/restore, M12.5). Covers the single-binary deployment story, not any one data/document subsystem. Invoke shadowcat-codebase-core first."
---

# Shadowcat — Server Bootstrap, Config, and Backup/Restore

Orientation for the parts of the server crate that exist ABOVE any one data subsystem: how the
binary starts, how configuration is resolved, and how a deployment's data is snapshotted/restored
(M12.5).

## Purpose

`main.rs` is the single entry point for the `shadowcat` binary — normal server startup AND the
one-shot `--backup-to`/`--restore-from` CLI modes share it, mutually exclusive with each other and
with serving. `config.rs` resolves the effective `Config` from four layered sources. `backup.rs`
is a pure-I/O module (no `AppState`/`SqliteRepository` dependency) providing whole-server backup
and restore as a deployment-operator tool, not an in-app feature.

## Key files & seams

- `src/server/src/config.rs` — `Cli` (flat `clap::Parser` struct, no `clap::Subcommand`) →
  `Config::load(cli)` layers CLI flag > `SHADOWCAT_*` env > TOML file > built-in default.
  `Config.db: String` (default `./shadowcat.db`), `Config.assets_dir: Option<String>` (`None` →
  sibling `assets/` beside the db file, via `Config::assets_path()`). `Cli.backup_to`/
  `restore_from: Option<String>` and `Cli.force: bool` are CLI-ONLY triggers — never on `Config`,
  never read from TOML/env (a one-shot operation is not persistent server configuration).
- `src/server/src/main.rs` — `main()`'s FIRST branch: if both `backup_to` and `restore_from` are
  `Some`, `anyhow::bail!` before `Config::load` even runs. The three fields are cloned OUT of
  `cli` before `Config::load(cli)` consumes it by value. Either flag alone short-circuits to
  `run_backup`/`run_restore` and `return Ok(())` — `SqliteRepository::connect` (the long-lived
  pool) and `axum::serve` are structurally unreachable on that path, not just conditionally
  skipped.
- `src/server/src/db.rs` — `open_pool(url)`: `SqlitePoolOptions::max_connections(1)`. **Dead
  code** — its only caller is its own unit test; production startup (`main.rs`) and
  `backup.rs`/`tests/backup_cli.rs` all open pools independently. The whole server's actual
  single-connection invariant lives in `SqliteRepository::connect`
  (`src/server/src/data/sqlite.rs`, [[shadowcat-codebase-documents-permissions]]), which
  separately sets the same `max_connections(1)` — editing `db.rs` does NOT affect the live
  server's connection pool.
- `src/server/src/backup.rs` (M12.5) — `BackupManifest`, `BackupError`, `dir_is_empty_or_absent`,
  `create_backup(db_path, assets_dir, out_dir) -> Result<BackupManifest, BackupError>`,
  `restore_backup(backup_dir, db_path, assets_dir, force) -> Result<(), BackupError>`. Opens its
  own short-lived `SqlitePool` directly (does not reuse `SqliteRepository`/`AppState`) — pure
  file I/O + one SQL statement, deliberately decoupled from the rest of the server so it works
  even when `main()`'s normal startup path never runs.

## Hard invariants

- **`VACUUM INTO`, never a raw `.db` file copy** — a raw byte-copy of a live SQLite file is unsafe
  (a concurrent writer or WAL journal can leave it mid-write); `VACUUM INTO` is SQLite's own
  atomic, consistency-guaranteed live-snapshot primitive.
- **Assets copy ALWAYS runs after the db snapshot, never before/concurrently** — asset uploads
  write bytes to disk BEFORE inserting the referencing DB row
  ([[shadowcat-codebase-assets]]-adjacent: `http/assets.rs` create path), and asset files are
  never deleted except by explicit delete, so db-then-assets ordering guarantees every asset a
  snapshot's rows reference is already present in the assets copy. `manifest.json` is written
  last, after both.
- **CLI-only surface, no admin HTTP endpoint** — deliberate: every host that can run the
  `shadowcat` binary can invoke it in one-shot mode from cron/Task Scheduler/systemd-timer,
  satisfying "minimal manual scheduling" without a new authenticated HTTP surface.
- **Fail-closed restore**: `restore_backup` validates `manifest.json` + `world.db` presence
  BEFORE touching any destination file — a missing/malformed/foreign backup directory returns
  `BackupError::InvalidBackupDir` with zero destination writes.
- **Force-gated overwrite, both directions**: `--backup-to` refuses a non-empty output directory
  without `--force`; `--restore-from` refuses when the destination db file already exists OR the
  destination assets dir exists and is non-empty, without `--force`. A rejected restore is
  structurally inert — the control flow cannot reach any destination-mutating call before the
  gate's `return Err`. Asymmetric ownership: `restore_backup` enforces its own gate internally
  regardless of caller, but `create_backup` does NOT check `out_dir` for prior contents — the
  refuse-non-empty gate for backup lives at the CLI layer (`main.rs::run_backup`, via the
  exported `dir_is_empty_or_absent`). A future caller invoking `create_backup` directly (e.g. an
  in-app export feature) would bypass that gate.
- **Restore never starts the server** — restore and serve are always two separate invocations
  (a live connection can't safely have its backing file replaced out from under it; Windows can
  even fail the replace outright on an open handle).
- **No shell-out for the recursive directory copy** (`tokio::fs` walks only — no `cp -r`/`xcopy`/
  `robocopy`), every path built via `Path`/`PathBuf::join` — cross-platform invariants per project
  CLAUDE.md, verified by a dedicated nested-directory (3+ levels) round-trip test.

## Gotchas

- `copy_dir_recursive` (`backup.rs`) silently skips symlinks (documented on the function itself)
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
- `cargo fmt` with a path argument still reformats the WHOLE crate if not scoped correctly — this
  bit two different Task implementers in M12.5's own execution, leaving unrelated drift in
  `http/mod.rs` that had to be reverted before commit. Use `cargo fmt --check` first, or scope
  explicitly, and diff before committing.
- Wipe-then-recopy under `--force` (assets directory) and the single-file `tokio::fs::copy` (db)
  are NOT atomic swaps — a failure partway through leaves the destination in a state worse than
  either the old or new content. Accepted, documented tradeoff for this "basic" gate-precondition
  feature (`docs/TODO.md`), not a bug.
- The backup's assets-copy is not transactionally coupled to the `VACUUM INTO` snapshot — an
  asset REPLACE (not create) in flight during backup commits its DB row before renaming its temp
  file into place ([[commit-db-row-before-swapping-file]]), so a backup racing an in-flight
  replace can capture updated metadata with pre-replace bytes for a brief window. Inherent to any
  online (no-downtime) backup of a live system; logged to `docs/TODO.md`, not solved here.
- Per-world granular export/import is explicitly OUT of scope — this milestone ships whole-server
  snapshot/restore only (single shared `shadowcat.db` across all worlds); per-world would need to
  preserve referential integrity across cross-table FKs and shared asset references, real
  complexity deferred to `docs/TODO.md` as a distinct future feature.

## Pointers

- Design: `docs/superpowers/specs/2026-07-15-m12.5-backups-snapshot-restore-design.md`; plan
  `docs/superpowers/plans/2026-07-15-m12.5-backups-snapshot-restore.md` (4 SDD tasks, no
  buddy-check pre-authorized — file I/O + one SQL statement, not the
  security/concurrency/determinism risk class).
- Relationships: `graphify query "config cli main backup restore server bootstrap"`.
- Data-layer side (what `db_path`/`assets_dir` ultimately point at): [[shadowcat-codebase-assets]],
  [[shadowcat-codebase-documents-permissions]] (`SqliteRepository`, `src/server/src/data/`).
