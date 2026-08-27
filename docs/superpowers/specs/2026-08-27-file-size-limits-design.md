# File-Size Limits, Inline-Test Extraction, and Build-Output Cleaning — Design

**Date:** 2026-08-27
**Status:** Approved (brainstorm), pending implementation plan

## Goal

No source file in the repository is too large to hold in context and edit reliably. A soft limit of
5,000 lines and a hard limit of 10,000 lines are enforced by CI, applied retroactively, and every
file currently over the soft limit is split. Rust test modules move out of production files
repo-wide, which is the cleanest division and is itself enforced by CI. Build output directories
are cleaned by the build itself, never by anything that can reach `src/`.

## Decisions taken during brainstorming

| Question | Decision |
|---|---|
| What counts toward the limit | The whole file as stored, test lines included; any covered extension under `src/`, `scripts/`, `examples/`. Docs and plans are not covered. |
| Soft-limit semantics | Fail unless the file is in a checked-in allowlist. Each entry needs the repository owner's explicit, per-file authorization. |
| Hard-limit semantics | Fail unconditionally; no allowlist can override it. |
| Inline Rust tests | Extracted to sibling files across the board (71 of 101 `.rs` files), enforced by a second CI gate. |
| Build cleaning | Every output directory is emptied by its own build via an explicit, enumerated, recoverable clean step. |

The repository owner must approve every allowlist entry. No agent adds one on its own authority.

## 1. Gate: file size — `scripts/check-file-lines.mjs`

Invoked as `pnpm lint:file-size`; wired into `.github/workflows/ci.yml` as a named step beside
`lint:allowances`, and into `pnpm run test:scripts` via a vitest suite.

**Scope.** Every git-tracked file under `src/`, `scripts/`, `examples/` whose extension is one of
`rs ts js mjs svelte scss css`. Enumeration is `git ls-files` (tracked = durable side), never a
directory walk, so untracked scratch and build output are never counted. Excluded:
`src/types/generated/**` (machine output regenerated from the server crate). Lockfiles are not a
covered extension.

**Count.** Newline count, identical to `wc -l`. A trailing line without a newline counts as one line.

**Rules, in order.**

1. `lines > 10_000` → error `HARD LIMIT`, no override exists. Emitted even if the file is allowlisted.
2. `lines > 5_000` and the path is not in the allowlist → error `SOFT LIMIT`, with the remedy
   "split the file, or obtain the repository owner's explicit approval and record it in the
   allowlist".
3. Allowlist entry whose path is not tracked, or whose file is now `≤ 5_000` lines → error
   `STALE ALLOWLIST ENTRY`. Entries are removed when they stop being needed, never accumulated.

Exit code non-zero if any error was emitted; every error names the path and the measured count.

**Allowlist.** `.claude/file-size-allowlist.toml`, matching the existing
`.claude/suppression-allowlist.toml` convention:

```toml
# Each entry records the repository owner's explicit per-file approval.
[[file]]
path = "src/server/src/example.rs"
lines_at_approval = 5321
reason = "..."
```

Ships **empty** (header comment only). `lines_at_approval` is informational: the gate does not
ratchet on it, because a ratchet is a rule that grandfathers what it should be removing.

## 2. Rule text

New section in `.claude/CLAUDE.md`, sibling of "Lint Suppressions Require Explicit User Approval",
titled "File-Size Limits Require Explicit User Approval":

- Soft limit 5,000 lines, hard limit 10,000, enforced by `pnpm lint:file-size` — a gate, not a
  ratchet; nothing is grandfathered.
- An entry in `.claude/file-size-allowlist.toml` requires the user's explicit per-file
  authorization in the conversation. No agent adds, edits, or retains one on its own authority.
- An oversize file found in the tree is a defect to split, not a precedent to follow.
- The hard limit has no override.
- Rust test modules live in sibling files, never inline (§3); `pnpm lint:inline-tests` enforces it.

Companion memory file (`file-size-limits-need-explicit-signoff.md`, type `feedback`, marked
IRON-CLAD in `MEMORY.md`) so the rule binds every dispatch brief, not only this repo's CLAUDE.md.

## 3. Gate: no inline Rust test bodies — `scripts/check-inline-tests.mjs`

Invoked as `pnpm lint:inline-tests`; CI step and vitest suite as in §1.

**Violation.** In any tracked `.rs` file under `src/`, a `#[cfg(test)]` attribute whose next
non-blank, non-comment line is `mod <ident> {` (a braced body). Detection uses
`scripts/lib/comment-span.mjs` so an attribute quoted in a doc comment is prose, not a match.

**Allowed forms.**

- `#[cfg(test)] mod tests;` — declaration only, body in a sibling file.
- `#[cfg(test)]` on any non-module item (a `fn`, field, `impl`, `thread_local!`): these are
  test-only declarations in production files that the extracted tests reach through `super::`.
  There are 17 today; they stay where they are.

**File convention.**

| Parent | Test file | Declaration in parent |
|---|---|---|
| `foo/mod.rs` | `foo/tests.rs` | `#[cfg(test)] mod tests;` |
| `foo.rs` (no directory) | `foo/tests.rs` | `#[cfg(test)] mod tests;` — Rust resolves `foo.rs` + `foo/` side by side; no rename |
| file with several test modules (9 today) | one file per module, named after the module (`foo/required_cap_tests.rs`) | one declaration per module |

Bodies move verbatim. `use super::*` inside the moved module resolves to the same parent, so no
visibility widens and no test loses access. Test-only helpers declared inside a moved module move
with it.

## 4. Retroactive splits

**Measured today** (tracked files over 5,000 lines; production line count = lines before the first
`#[cfg(test)] mod`):

| File | Lines | Production | Test lines |
|---|---|---|---|
| `src/server/src/data/sqlite.rs` | 11,320 (over hard) | 3,887 | 7,432 (169 test fns) |
| `src/server/src/scene/mod.rs` | 9,703 | ~3,180 | 6,521 (201 test fns) |
| `src/server/src/chat/mod.rs` | 5,892 | 1,554 | 3,532 + 805 (two modules) |
| `src/server/src/data/permission.rs` | 5,042 | ~1,370 | two modules, both under 5,000 |

No client `.ts`/`.svelte`/`.scss` file is near the limit. `docs/superpowers/plans/*.md` reach
6,225 lines and are out of scope by decision.

**Steps.**

1. Extract test modules from all 71 files per §3. After this step no production file exceeds 5,000.
2. Split the two oversize test files by subject into `tests/<subject>.rs` files, each under 5,000:
   `src/server/src/data/sqlite/tests/{...}.rs` and `src/server/src/scene/tests/{...}.rs`, with
   `tests/mod.rs` declaring the submodules and holding shared fixtures. The grouping is enumerated
   from the 169 and 201 test-function names at plan-writing time — every function is assigned to
   exactly one subject file, and the plan lists the assignment so nothing is dropped.
3. `chat` and `permission` test modules move as-is (each under 5,000).
4. `sqlite.rs` production code (3,887 lines) is not restructured in this work; it is logged in
   `docs/TODO.md` as the next split candidate.

**Acceptance.** `cargo test` reports an identical count of passed tests before and after (the
before-count is recorded in the plan as a literal), `cargo clippy` and `cargo fmt --check` are
clean, both new gates and every existing gate pass, CI is green on all three OS runners.

## 5. Build-output cleaning — `scripts/clean-build-outputs.mjs`

Invoked as `pnpm clean`; `pnpm build` and `pnpm build:all` run it first.

**Targets — an explicit, enumerated list and nothing else:**

- `dist/`
- `dist-docs/`
- `docs/site/.vitepress/dist/`
- `examples/*/dist/`

Each target is sent to the OS recycle bin / trash (recoverable), not `rmSync`. The script refuses
any path that resolves outside the repository root or into `src/`, `scripts/`, `docs/` other than
the listed VitePress output, or `.git/` — a positive assertion, not a blocklist scan.
`assemble-docs.mjs`'s `rmSync(out)` and `scripts/package.sh`'s `rm -rf "$out"` are converted to
the same recoverable path for consistency with the deletion rule (approved as an addition).

Context: the five client/type packages found deleted from the working tree on 2026-08-27 were
restored from `HEAD` with zero loss. No script in the repository can reach `src/`; the deletion was
external. This section removes the class of hazard rather than the one instance.

## Testing

Each gate script has a vitest suite under `scripts/` with positive controls that prove the detector
fires for every shape it claims to cover:

- file-size: a fixture at 5,001 lines (soft fail), at 10,001 (hard fail), an allowlisted 5,001-line
  file (pass), an allowlisted 10,001-line file (hard fail despite allowlist), an allowlisted file at
  4,999 (stale entry fail), a file under `src/types/generated/` at 20,000 (ignored), an untracked
  file at 20,000 (ignored).
- inline-tests: `#[cfg(test)] mod tests {` (fail), `#[cfg(test)] mod tests;` (pass), `#[cfg(test)]`
  on a `fn` (pass), the attribute inside a `///` doc comment (pass), attribute and `mod` separated by
  a blank line and a `//` comment (fail).
- clean: every listed target is removed and nothing else is; a target path resolving into `src/`
  is refused before any deletion.

The migration's acceptance check is the identical `cargo test` count (§4).

## Out of scope

- Splitting any production module (only tests move).
- Applying the limit to `docs/`, plans, or generated types.
- Any change to the `shadowcat-codebase` plugin skills beyond the mandated skill-update gate at the
  end of execution (core skill gains the two gates and the test-file convention).
