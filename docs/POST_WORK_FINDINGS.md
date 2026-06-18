# Post-Work Findings

Living record of issues surfaced during review/audit. NOT a to-do list — entries
are observations awaiting triage, not committed work.

- Title: Broken source-of-truth doc reference. Summary: `CLAUDE.md`'s Reference
  Docs table names `docs/ENGINE_PRINCIPLES.md` as "Source of truth: engine
  invariants, code style, testing rules", but that file does not exist. The
  actual invariants/architecture source of truth is `docs/design/ARCHITECTURE.md`.
  Surfaced during the cross-platform audit. Status: Needs triage (fix the
  reference, or create ENGINE_PRINCIPLES.md if a separate doc is intended).
  Note: CLAUDE.md is git-ignored, so the fix is local-only unless the file is
  un-ignored.
