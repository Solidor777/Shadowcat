# File-Size Limits, Inline-Test Extraction, Build-Output Cleaning — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enforce a 5,000-line soft / 10,000-line hard limit on source files in CI, move every inline Rust test module to a sibling file (also CI-enforced), split the two test files that are still over the limit, and make every build empty its own output directories through one enumerated, recoverable clean step.

**Architecture:** Two new Node gate scripts in `scripts/` follow the existing `check-lint-allowances.mjs` shape (pure exported functions + a guarded `main`, TOML-ish allowlist, vitest suite under `scripts/`). The Rust migration is mechanical: a throwaway extraction script (kept in the scratchpad, never committed) moves each `#[cfg(test)] mod X { … }` body verbatim into `<parent-dir>/<stem>/X.rs` (or `<dir>/X.rs` for a `mod.rs` parent) and leaves `#[cfg(test)] mod X;` behind; `sqlite` and `scene` tests are then cut into subject files with shared fixtures hoisted into `tests/mod.rs`. A `pnpm clean` script trashes an explicit list of output directories and runs ahead of every build.

**Tech Stack:** Node 22 ESM scripts, vitest 4, Rust 2021 (cargo fmt/clippy/test), `trash` npm package (new devDependency — see Task 9), GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-08-27-file-size-limits-design.md`

## Global Constraints

- Soft limit **5,000** lines, hard limit **10,000** lines. Counting is newline count (`wc -l`); a final unterminated line counts as one.
- Covered files: git-tracked, under `src/`, `scripts/`, `examples/`, extension in `rs ts js mjs svelte scss css`; `src/types/generated/**` excluded.
- Allowlist `.claude/file-size-allowlist.toml` — ships **empty**; **every entry requires the repository owner's explicit per-file authorization in conversation. No agent adds, edits, or retains one on its own authority.** The hard limit has no override.
- Inline `#[cfg(test)] mod X { … }` bodies are forbidden in `src/**/*.rs`; only `#[cfg(test)] mod X;` declarations and non-module `#[cfg(test)]` items are allowed.
- Test bodies move **verbatim**. Acceptance for every migration task: `cargo test --all` reports the **same number of passed tests** as the baseline recorded in Task 0 and zero failures.
- Deletion rule: no `rm`, `Remove-Item`, `rmSync` on anything in this plan; the only delete is the recoverable trash path in Task 9. Throwaway scripts live in the scratchpad, not the repo.
- No lint suppressions of any kind (`#[allow]`, `#[expect]`, `eslint-disable`, `@ts-ignore`).
- Comments: present-tense, no history, no process meta, no task ids.
- Commit each task with `git commit -m "…" -- <explicit paths>`; never `git add -A`.
- Cross-platform: every script uses `node:path`, no shell-specific syntax; CI runs on ubuntu/macos/windows.

## Model/Effort directives

- Plan written mainline on Fable 5 at the user's request (tier-switch checkpoint: user chose mainline authoring).
- Execution: `superpowers:subagent-driven-development` with a **Sonnet dispatcher** (user directive). **Opus is banned for every subagent** in this repo (standing directive) — the escalation ladder is `sdd-implementer` → `sdd-implementer-highthink` → user; never an `-opus` twin.
- Implementers: `shadowcat-codebase:shadowcat-coder` (`effort: medium`) for Tasks 1, 2, 8, 9; `sdd-implementer` for the mechanical migration batches (Tasks 3–7). Reviewers: `shadowcat-codebase:shadowcat-spec-reviewer` + `shadowcat-codebase:shadowcat-code-reviewer` (`effort: high`), both **without Bash** — the dispatcher pre-generates `git diff` output to a scratchpad file and relays gate outputs verbatim.
- The dispatcher runs every gate itself and reads the output file, never a notification summary.

## Buddy-check directives

- Tasks 6 and 7 (the `sqlite` and `scene` subject splits) are offered for a buddy-check (two blind reviewers + brokered debate) at handoff. Unless the user opts in, they get the standard two-reviewer pass with the diff file pre-generated.

## Task 0 (dispatcher, before Task 1): gate battery into `progress.md`

Paste this list verbatim under a `## Gate battery` heading in the SDD ledger. It is the full `.github/workflows/ci.yml` step list, **plus the two gates this plan adds** (Tasks 1–2), which join the battery from the moment their scripts exist:

- Rust job: `cargo fmt --all -- --check`; `cargo clippy --all-targets -- -D warnings`; `cargo test --all`; `git diff --exit-code src/types/generated`
- TS job: `pnpm -r typecheck`; `pnpm -r test`; `pnpm run test:scripts`; `pnpm docs:check-examples`; `pnpm lint`; `pnpm --filter @shadowcat/shell build`; `pnpm --filter "shadowcat-example-*" build`; `pnpm run check:svelte-runtime`
- server-e2e job: `pnpm --filter @shadowcat/core test:e2e`
- UI-e2e job: `pnpm --filter @shadowcat/shell e2e`
- docs job: `node scripts/check-skill-api-refs-cli.mjs`; `node scripts/check-skill-symbol-refs-cli.mjs`; `pnpm lint:docs`; `pnpm lint:props`; `pnpm lint:comments`; `pnpm lint:allowances`; `pnpm docs:check-examples`; `cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D missing-docs -D clippy::missing-docs-in-private-items`; `cargo +nightly doc --manifest-path src/server/Cargo.toml --document-private-items --no-deps --target-dir target/nightly-doc` with `RUSTDOCFLAGS="-D rustdoc::missing_doc_code_examples"`
- **New (this plan):** `pnpm lint:file-size`; `pnpm lint:inline-tests`

All `cargo` commands run from `src/server/` (or with `--manifest-path src/server/Cargo.toml`). Redirect each gate to a file and check the command's own exit status — never pipe into `tail`/`echo`.

**Baseline test count (measured 2026-08-27 on `295e0cec` + spec commit).** `cargo test --all` from `src/server/`: the lib crate's unit-test binary reports **1616 passed** (this is the population every inline module feeds), the integration/doc binaries add 178, total **1794 passed, 0 failed**. Re-run once before Task 3, confirm the same numbers, and record every `test result:` line in `progress.md` under `## Baseline`; the lib-crate `1616 passed` is **the acceptance number** for Tasks 3–7 (the others cannot move).

---

### Task 1: File-size gate — `scripts/check-file-lines.mjs`

**Files:**
- Create: `scripts/check-file-lines.mjs`
- Create: `scripts/check-file-lines.test.mjs`
- Create: `.claude/file-size-allowlist.toml`
- Modify: `package.json` (scripts block)

**Interfaces:**
- Produces (exported, pure): `countLines(text: string): number`; `isCovered(path: string): boolean`; `parseAllowlist(text: string, sourceName: string): Array<{path: string, lines_at_approval: string, reason: string}>`; `evaluate({files, allow}): Array<{kind: "HARD LIMIT"|"SOFT LIMIT"|"STALE ALLOWLIST ENTRY", path: string, lines: number, message: string}>` where `files` is `Array<{path, lines}>` and `allow` is the parsed allowlist; constants `SOFT_LIMIT = 5000`, `HARD_LIMIT = 10000`, `COVERED_EXTS`, `ROOTS`, `ALLOWLIST`.
- Consumes: `isDirectEntry` from `scripts/lib/is-main.mjs`; `norm`, `under`, `GENERATED_ROOT` from `scripts/lib/gate-corpus.mjs`.

- [x] **Step 1: Write the failing tests**

`scripts/check-file-lines.test.mjs`:

```js
import { test, expect } from "vitest";
import {
  countLines,
  isCovered,
  parseAllowlist,
  evaluate,
  SOFT_LIMIT,
  HARD_LIMIT,
} from "./check-file-lines.mjs";

const lines = (n) => Array.from({ length: n }, (_, i) => `line ${i}`).join("\n") + "\n";

test("countLines matches wc -l and counts an unterminated final line", () => {
  expect(countLines("")).toBe(0);
  expect(countLines("a\n")).toBe(1);
  expect(countLines("a\nb")).toBe(2);
  expect(countLines(lines(5001))).toBe(5001);
});

test("isCovered admits source under the roots and rejects generated types and other extensions", () => {
  expect(isCovered("src/server/src/data/sqlite.rs")).toBe(true);
  expect(isCovered("src/client/shell/src/App.svelte")).toBe(true);
  expect(isCovered("scripts/check-file-lines.mjs")).toBe(true);
  expect(isCovered("examples/system-minimal/src/index.ts")).toBe(true);
  expect(isCovered("src/types/generated/ServerMsg.ts")).toBe(false);
  expect(isCovered("docs/superpowers/plans/x.md")).toBe(false);
  expect(isCovered("pnpm-lock.yaml")).toBe(false);
  expect(isCovered("src\\server\\src\\lib.rs")).toBe(true);
});

test("parseAllowlist accepts [[file]] tables of quoted strings and errors on anything else", () => {
  const text = `# header\n[[file]]\npath = "src/a.rs"\nlines_at_approval = "5321"\nreason = "why"\n`;
  expect(parseAllowlist(text, "x.toml")).toEqual([
    { path: "src/a.rs", lines_at_approval: "5321", reason: "why" },
  ]);
  expect(() => parseAllowlist("[[allow]]\n", "x.toml")).toThrow(/x\.toml:1/);
  expect(() => parseAllowlist("path = 5\n", "x.toml")).toThrow(/x\.toml:1/);
});

test("evaluate: soft fail above 5000 without an entry, pass with one", () => {
  const files = [{ path: "src/a.rs", lines: SOFT_LIMIT + 1 }];
  expect(evaluate({ files, allow: [] }).map((e) => e.kind)).toEqual(["SOFT LIMIT"]);
  const allow = [{ path: "src/a.rs", lines_at_approval: "5001", reason: "r" }];
  expect(evaluate({ files, allow })).toEqual([]);
});

test("evaluate: exactly 5000 passes; hard fail above 10000 even when allowlisted", () => {
  expect(evaluate({ files: [{ path: "src/a.rs", lines: SOFT_LIMIT }], allow: [] })).toEqual([]);
  const files = [{ path: "src/a.rs", lines: HARD_LIMIT + 1 }];
  const allow = [{ path: "src/a.rs", lines_at_approval: "10001", reason: "r" }];
  expect(evaluate({ files, allow }).map((e) => e.kind)).toEqual(["HARD LIMIT"]);
});

test("evaluate: an allowlist entry for a file at or under 5000, or not in the file set, is stale", () => {
  const allow = [
    { path: "src/small.rs", lines_at_approval: "5001", reason: "r" },
    { path: "src/gone.rs", lines_at_approval: "5001", reason: "r" },
  ];
  const files = [{ path: "src/small.rs", lines: 4999 }];
  expect(evaluate({ files, allow }).map((e) => [e.kind, e.path])).toEqual([
    ["STALE ALLOWLIST ENTRY", "src/small.rs"],
    ["STALE ALLOWLIST ENTRY", "src/gone.rs"],
  ]);
});

test("evaluate: every error names the path and the measured count", () => {
  const [e] = evaluate({ files: [{ path: "src/big.rs", lines: 12000 }], allow: [] });
  expect(e.message).toContain("src/big.rs");
  expect(e.message).toContain("12000");
});
```

- [x] **Step 2: Run to verify it fails**

Run: `npx vitest run scripts/check-file-lines.test.mjs`
Expected: FAIL — cannot resolve `./check-file-lines.mjs`.

- [x] **Step 3: Write the script**

`scripts/check-file-lines.mjs`:

```js
// Fails when a covered source file exceeds the line limits.
//
// A file that no longer fits in one reading is edited by pattern-matching on fragments of it, and
// every reviewer of it does the same; the defect rate of such edits is the reason for the limits.
// The soft limit (5,000) is the real line: crossing it fails unless the repository owner has
// approved that specific file in the allowlist named below. The hard limit (10,000) has no
// override. Neither limit grandfathers anything: an entry whose file has since dropped to the
// limit fails in its own right, so permission cannot accumulate.
//
// Test lines count. Splitting a test module into its own file is the intended remedy, and
// `check-inline-tests.mjs` enforces that Rust test modules live in sibling files.
//
// Enumeration is `git ls-files`, so untracked scratch and build output never count.

import { readFileSync, existsSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { extname } from "node:path";
import process from "node:process";
import { isDirectEntry } from "./lib/is-main.mjs";
import { GENERATED_ROOT, norm, under } from "./lib/gate-corpus.mjs";

export const SOFT_LIMIT = 5000;
export const HARD_LIMIT = 10000;
export const ROOTS = ["src", "scripts", "examples"];
export const COVERED_EXTS = new Set([".rs", ".ts", ".js", ".mjs", ".svelte", ".scss", ".css"]);
export const ALLOWLIST = ".claude/file-size-allowlist.toml";

/** Newline count, identical to `wc -l`, plus one for an unterminated final line. */
export function countLines(text) {
  if (text.length === 0) return 0;
  let n = 0;
  for (let i = 0; i < text.length; i++) if (text.charCodeAt(i) === 10) n++;
  return text.endsWith("\n") ? n : n + 1;
}

/** Whether `path` (any separator) is a file the limits apply to. */
export function isCovered(path) {
  const p = norm(path);
  if (!ROOTS.some((r) => under(p, r))) return false;
  if (under(p, GENERATED_ROOT)) return false;
  return COVERED_EXTS.has(extname(p));
}

/**
 * Parses the allowlist. Accepts only `[[file]]` tables of double-quoted string values and throws
 * on anything else: a line silently ignored here would be an approval nobody granted.
 */
export function parseAllowlist(text, sourceName) {
  const out = [];
  let cur = null;
  text.split("\n").forEach((raw, i) => {
    const line = raw.replace(/\r$/, "").trim();
    if (line === "" || line.startsWith("#")) return;
    if (line === "[[file]]") {
      cur = { path: "", lines_at_approval: "", reason: "" };
      out.push(cur);
      return;
    }
    const m = line.match(/^(path|lines_at_approval|reason)\s*=\s*"((?:[^"\\]|\\.)*)"$/);
    if (!m || cur === null) {
      throw new Error(`${sourceName}:${i + 1}: cannot parse. Expected [[file]] or path/lines_at_approval/reason = "value". got: ${raw}`);
    }
    cur[m[1]] = m[2].replace(/\\"/g, '"');
  });
  return out;
}

/** Applies the limits to measured files; pure so the suite can drive every branch. */
export function evaluate({ files, allow }) {
  const errors = [];
  const allowed = new Set(allow.map((a) => norm(a.path)));
  const measured = new Map(files.map((f) => [norm(f.path), f.lines]));
  for (const [path, lines] of measured) {
    if (lines > HARD_LIMIT) {
      errors.push({ kind: "HARD LIMIT", path, lines, message: `${path}: ${lines} lines exceeds the hard limit of ${HARD_LIMIT}. No override exists; split the file.` });
    } else if (lines > SOFT_LIMIT && !allowed.has(path)) {
      errors.push({ kind: "SOFT LIMIT", path, lines, message: `${path}: ${lines} lines exceeds the soft limit of ${SOFT_LIMIT}. Split the file, or obtain the repository owner's explicit approval and record it in ${ALLOWLIST}.` });
    }
  }
  for (const a of allow) {
    const path = norm(a.path);
    const lines = measured.get(path);
    if (lines === undefined || lines <= SOFT_LIMIT) {
      errors.push({ kind: "STALE ALLOWLIST ENTRY", path, lines: lines ?? 0, message: `${path}: allowlist entry is stale (${lines === undefined ? "file is not tracked" : `${lines} lines, at or under ${SOFT_LIMIT}`}). Remove the entry.` });
    }
  }
  return errors;
}

/** Tracked paths under the roots, from git so build output and scratch never count. */
function trackedFiles() {
  const out = execFileSync("git", ["ls-files", "-z", "--", ...ROOTS], { encoding: "utf8" });
  return out.split("\0").filter((p) => p.length > 0);
}

function main() {
  const files = trackedFiles()
    .filter(isCovered)
    .map((path) => ({ path: norm(path), lines: countLines(readFileSync(path, "utf8")) }));
  let allow = [];
  if (existsSync(ALLOWLIST)) {
    try {
      allow = parseAllowlist(readFileSync(ALLOWLIST, "utf8"), ALLOWLIST);
    } catch (e) {
      console.error(e.message);
      process.exit(2);
    }
  }
  const errors = evaluate({ files, allow });
  for (const e of errors) console.error(`${e.kind}: ${e.message}`);
  console.log(`lint:file-size: ${files.length} files measured, ${allow.length} allowlisted, ${errors.length} error(s)`);
  process.exit(errors.length === 0 ? 0 : 1);
}

if (isDirectEntry(import.meta.url)) main();
```

- [x] **Step 4: Create the empty allowlist**

`.claude/file-size-allowlist.toml`:

```toml
# Approved soft-limit exceptions for `pnpm lint:file-size` (scripts/check-file-lines.mjs).
# One entry per file, each signed off by the repository owner in conversation. No agent adds,
# edits, or retains an entry on its own authority. An entry is a standing claim that the file
# cannot be split without losing something the code needs — not a note that splitting is
# inconvenient. The hard limit (10,000 lines) has no override, allowlisted or not.
#
# A stale entry (file at or under 5,000 lines, or no longer tracked) fails the gate in its own
# right, so permission cannot silently accumulate. `lines_at_approval` is informational only.
#
# [[file]]
# path = "src/server/src/example.rs"
# lines_at_approval = "5321"
# reason = "..."
```

- [x] **Step 5: Add the pnpm script**

In `package.json` scripts, after `"lint:allowances"`:

```json
    "lint:file-size": "node scripts/check-file-lines.mjs",
```

- [x] **Step 6: Run the suite and the gate**

Run: `npx vitest run scripts/check-file-lines.test.mjs`
Expected: 7 tests PASS.

Run: `pnpm lint:file-size > "$SCRATCH/file-size.txt"; echo $?` (bash) — read the file.
Expected: exit 1 with exactly these errors (the current tree): `HARD LIMIT` for `src/server/src/data/sqlite.rs` (11320), `SOFT LIMIT` for `src/server/src/scene/mod.rs` (9703), `src/server/src/chat/mod.rs` (5892), `src/server/src/data/permission.rs` (5042). Any other error is a scope error in the script — fix before committing. This is the expected state until Task 7; CI wiring waits for Task 8.

- [x] **Step 7: Commit**

```bash
git commit -m "feat(ci): add file-size gate script with owner-approved allowlist" -- scripts/check-file-lines.mjs scripts/check-file-lines.test.mjs .claude/file-size-allowlist.toml package.json
```

---

### Task 2: Inline-test gate — `scripts/check-inline-tests.mjs`

**Files:**
- Create: `scripts/check-inline-tests.mjs`
- Create: `scripts/check-inline-tests.test.mjs`
- Modify: `package.json` (scripts block)

**Interfaces:**
- Produces (exported, pure): `scanInlineTests(text: string): Array<{line: number, module: string}>` (1-based line of the `mod` line); `isRustSource(path): boolean`.
- Consumes: `splitLine(line, state)` from `scripts/lib/comment-span.mjs` — returns `{ code, comment, state }`-shaped output; read its header before use and keep the block-comment state across lines exactly as `check-lint-allowances.mjs` does (copy that loop's usage).

- [x] **Step 1: Write the failing tests**

`scripts/check-inline-tests.test.mjs`:

```js
import { test, expect } from "vitest";
import { scanInlineTests, isRustSource } from "./check-inline-tests.mjs";

test("a braced test module body is a violation", () => {
  const src = "fn a() {}\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n}\n";
  expect(scanInlineTests(src)).toEqual([{ line: 4, module: "tests" }]);
});

test("a pub(crate) braced test module is a violation", () => {
  const src = "#[cfg(test)]\npub(crate) mod tests {\n}\n";
  expect(scanInlineTests(src)).toEqual([{ line: 2, module: "tests" }]);
});

test("a declaration-only test module passes", () => {
  expect(scanInlineTests("#[cfg(test)]\nmod tests;\n")).toEqual([]);
  expect(scanInlineTests("#[cfg(test)]\npub(crate) mod tests;\n")).toEqual([]);
});

test("cfg(test) on a non-module item passes", () => {
  expect(scanInlineTests("#[cfg(test)]\npub(crate) fn with_capacity_for_test(n: usize) {}\n")).toEqual([]);
  expect(scanInlineTests("    #[cfg(test)]\n    visible_cells_recompute_count: AtomicU64,\n")).toEqual([]);
  expect(scanInlineTests("#[cfg(test)]\nimpl SceneEcs {\n}\n")).toEqual([]);
});

test("the attribute inside a doc comment is prose", () => {
  expect(scanInlineTests("/// Write `#[cfg(test)]`\n/// mod tests {\nfn a() {}\n")).toEqual([]);
  expect(scanInlineTests("/* #[cfg(test)]\nmod tests { */\nfn a() {}\n")).toEqual([]);
});

test("blank lines, comments and further attributes between the attribute and the mod line do not hide it", () => {
  const src = "#[cfg(test)]\n\n// helpers\n#[rustfmt::skip]\nmod smoke {\n}\n";
  expect(scanInlineTests(src)).toEqual([{ line: 5, module: "smoke" }]);
});

test("two modules in one file are both reported", () => {
  const src = "#[cfg(test)]\nmod a {\n}\n\n#[cfg(test)]\nmod b {\n}\n";
  expect(scanInlineTests(src).map((v) => v.module)).toEqual(["a", "b"]);
});

test("isRustSource covers tracked .rs under src only", () => {
  expect(isRustSource("src/server/src/lib.rs")).toBe(true);
  expect(isRustSource("src\\server\\build.rs")).toBe(true);
  expect(isRustSource("scripts/x.mjs")).toBe(false);
  expect(isRustSource("target/debug/build/x.rs")).toBe(false);
});
```

- [x] **Step 2: Run to verify it fails**

Run: `npx vitest run scripts/check-inline-tests.test.mjs`
Expected: FAIL — cannot resolve module.

- [x] **Step 3: Write the script**

`scripts/check-inline-tests.mjs`:

```js
// Fails when a Rust test module body is written inline in a production source file.
//
// A `#[cfg(test)] mod x { … }` body is the single largest contributor to oversize files in this
// crate, and it is the one part of a file that can move without changing what the file exports:
// `mod x;` with the body in a sibling file resolves `use super::*` to the same parent, so nothing
// widens and nothing is lost. The declaration form is therefore the only allowed one.
//
// `#[cfg(test)]` on a non-module item (a helper fn, a field, an impl block) is a test-only
// declaration that the extracted tests reach through `super::`; it stays in the production file
// and is not a violation.
//
// Telling code from comment is shared with the other gates through comment-span.mjs so an
// attribute quoted in a doc comment is prose, not a match.

import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import process from "node:process";
import { isDirectEntry } from "./lib/is-main.mjs";
import { norm, under } from "./lib/gate-corpus.mjs";
import { splitLine } from "./lib/comment-span.mjs";

const ATTR = /^\s*#\[cfg\(test\)\]\s*$/;
const ANY_ATTR = /^\s*#\[/;
const MOD_BODY = /^\s*(?:pub(?:\([a-z]+\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*\{/;

/** Whether `path` is a Rust source file the gate covers. */
export function isRustSource(path) {
  const p = norm(path);
  return under(p, "src") && p.endsWith(".rs");
}

/** Every `#[cfg(test)]` attribute followed (past blanks, comments, other attributes) by a braced `mod`. */
export function scanInlineTests(text) {
  const lines = text.split("\n").map((l) => l.replace(/\r$/, ""));
  const code = [];
  let state = { inBlock: false, inHtml: false };
  for (const line of lines) {
    const r = splitLine(line, state);
    state = r.state;
    code.push(r.code);
  }
  const out = [];
  for (let i = 0; i < code.length; i++) {
    if (!ATTR.test(code[i])) continue;
    let j = i + 1;
    while (j < code.length && (code[j].trim() === "" || ANY_ATTR.test(code[j]))) j++;
    if (j >= code.length) continue;
    const m = code[j].match(MOD_BODY);
    if (m) out.push({ line: j + 1, module: m[1] });
  }
  return out;
}

function main() {
  const files = execFileSync("git", ["ls-files", "-z", "--", "src"], { encoding: "utf8" })
    .split("\0")
    .filter(isRustSource);
  let errors = 0;
  for (const path of files) {
    for (const v of scanInlineTests(readFileSync(path, "utf8"))) {
      errors++;
      console.error(`INLINE TEST MODULE: ${norm(path)}:${v.line}: mod ${v.module} { … } — move the body to a sibling file and declare it as \`#[cfg(test)] mod ${v.module};\``);
    }
  }
  console.log(`lint:inline-tests: ${files.length} files scanned, ${errors} error(s)`);
  process.exit(errors === 0 ? 0 : 1);
}

if (isDirectEntry(import.meta.url)) main();
```

If `splitLine`'s return shape differs from `{ code, comment, state }`, adapt the loop to its actual contract (read `scripts/lib/comment-span.mjs` and `check-lint-allowances.mjs`'s call site) — do not reimplement comment detection.

- [x] **Step 4: Add the pnpm script**

In `package.json` scripts, after `"lint:file-size"`:

```json
    "lint:inline-tests": "node scripts/check-inline-tests.mjs",
```

- [x] **Step 5: Run the suite and the gate**

Run: `npx vitest run scripts/check-inline-tests.test.mjs` — Expected: 8 tests PASS.

Run: `pnpm lint:inline-tests > "$SCRATCH/inline-tests.txt"; echo $?` — read the file.
Expected: exit 1 with **exactly 76** `INLINE TEST MODULE` errors across 71 files (the inventory in Task 3). A different count means the scanner over- or under-matches; diff the reported list against Task 3's table and fix the scanner before committing.

- [x] **Step 6: Commit**

```bash
git commit -m "feat(ci): add inline-test-module gate script" -- scripts/check-inline-tests.mjs scripts/check-inline-tests.test.mjs package.json
```

---

### Task 3: Extraction tool (scratchpad) + batch 1: `auth`, `backup`, `config`, `db`, `health`, `modules`, `world_bundle`

**Files:**
- Create (scratchpad only, never committed): `$SCRATCH/extract-test-mod.mjs`
- Modify + Create (per row below): parent file gets `#[cfg(test)] mod X;`, body lands in the new file.

**Inventory — every inline test module in the crate (76 modules, 71 files).** Paths relative to `src/server/`. "Target" is the file the body moves to.

| # | Parent | Module | Target |
|---|---|---|---|
| 1 | `src/auth/invite.rs` | `tests` | `src/auth/invite/tests.rs` |
| 2 | `src/auth/password.rs` | `tests` | `src/auth/password/tests.rs` |
| 3 | `src/auth/role.rs` | `tests` | `src/auth/role/tests.rs` |
| 4 | `src/auth/session.rs` | `tests` | `src/auth/session/tests.rs` |
| 5 | `src/auth/setup.rs` | `tests` | `src/auth/setup/tests.rs` |
| 6 | `src/backup.rs` | `tests` | `src/backup/tests.rs` |
| 7 | `src/config.rs` | `tests` | `src/config/tests.rs` |
| 8 | `src/db.rs` | `tests` | `src/db/tests.rs` |
| 9 | `src/health.rs` | `tests` | `src/health/tests.rs` |
| 10 | `src/modules.rs` | `tests` | `src/modules/tests.rs` |
| 11 | `src/world_bundle.rs` | `tests` | `src/world_bundle/tests.rs` |
| 12 | `src/chat/commands.rs` | `tests` | `src/chat/commands/tests.rs` |
| 13 | `src/chat/link_preview.rs` | `tests` | `src/chat/link_preview/tests.rs` |
| 14 | `src/chat/mod.rs` | `tests` | `src/chat/tests.rs` |
| 15 | `src/chat/mod.rs` | `link_preview_ingest_tests` | `src/chat/link_preview_ingest_tests.rs` |
| 16 | `src/chat/oembed.rs` | `tests` | `src/chat/oembed/tests.rs` |
| 17 | `src/chat/post_publish.rs` | `tests` | `src/chat/post_publish/tests.rs` |
| 18 | `src/chat/preview_cache.rs` | `tests` | `src/chat/preview_cache/tests.rs` |
| 19 | `src/chat/rolls.rs` | `tests` | `src/chat/rolls/tests.rs` |
| 20 | `src/chat/sanitize.rs` | `tests` | `src/chat/sanitize/tests.rs` |
| 21 | `src/chat/settings.rs` | `tests` | `src/chat/settings/tests.rs` |
| 22 | `src/chat/shortcodes.rs` | `tests` | `src/chat/shortcodes/tests.rs` |
| 23 | `src/data/asset.rs` | `tests` | `src/data/asset/tests.rs` |
| 24 | `src/data/command.rs` | `tests` | `src/data/command/tests.rs` |
| 25 | `src/data/document.rs` | `tests` (**`pub(crate)`**) | `src/data/document/tests.rs` |
| 26 | `src/data/engine/mod.rs` | `tests` | `src/data/engine/tests.rs` |
| 27 | `src/data/engine/token.rs` | `tests` | `src/data/engine/token/tests.rs` |
| 28 | `src/data/migrate.rs` | `tests` | `src/data/migrate/tests.rs` |
| 29 | `src/data/permission.rs` | `required_cap_tests` | `src/data/permission/required_cap_tests.rs` |
| 30 | `src/data/permission.rs` | `tests` | `src/data/permission/tests.rs` |
| 31 | `src/data/search.rs` | `tests` | `src/data/search/tests.rs` |
| 32 | `src/data/snapshot.rs` | `tests` | `src/data/snapshot/tests.rs` |
| 33 | `src/data/sqlite.rs` | `tests` | **Task 6** (`src/data/sqlite/tests/mod.rs` + subject files) |
| 34 | `src/data/validation.rs` | `tests` | `src/data/validation/tests.rs` |
| 35 | `src/data/world_bundle.rs` | `tests` | `src/data/world_bundle/tests.rs` |
| 36 | `src/dice/eval/classify.rs` | `tests` | `src/dice/eval/classify/tests.rs` |
| 37 | `src/dice/eval/crit.rs` | `tests` | `src/dice/eval/crit/tests.rs` |
| 38 | `src/dice/eval/expertise.rs` | `tests` | `src/dice/eval/expertise/tests.rs` |
| 39 | `src/dice/eval/groups.rs` | `tests` | `src/dice/eval/groups/tests.rs` |
| 40 | `src/dice/eval/mod.rs` | `tests` | `src/dice/eval/tests.rs` |
| 41 | `src/dice/eval/success.rs` | `tests` | `src/dice/eval/success/tests.rs` |
| 42 | `src/dice/eval/sum.rs` | `tests` | `src/dice/eval/sum/tests.rs` |
| 43 | `src/dice/notation/lexer.rs` | `tests` | `src/dice/notation/lexer/tests.rs` |
| 44 | `src/dice/notation/mod.rs` | `tests` | `src/dice/notation/tests.rs` |
| 45 | `src/dice/notation/parser.rs` | `tests` | `src/dice/notation/parser/tests.rs` |
| 46 | `src/dice/outcome.rs` | `tests` | `src/dice/outcome/tests.rs` |
| 47 | `src/dice/recalc.rs` | `tests` | `src/dice/recalc/tests.rs` |
| 48 | `src/dice/rng.rs` | `tests` | `src/dice/rng/tests.rs` |
| 49 | `src/dice/spec.rs` | `tests` | `src/dice/spec/tests.rs` |
| 50 | `src/http/assets.rs` | `tests` | `src/http/assets/tests.rs` |
| 51 | `src/http/embed.rs` | `tests` | `src/http/embed/tests.rs` |
| 52 | `src/http/mod.rs` | `tests` (**`pub(crate)`**) | `src/http/tests.rs` |
| 53 | `src/http/module_routes.rs` | `tests` | `src/http/module_routes/tests.rs` |
| 54 | `src/http/throttle.rs` | `tests` | `src/http/throttle/tests.rs` |
| 55 | `src/http/world_bundle.rs` | `tests` | `src/http/world_bundle/tests.rs` |
| 56 | `src/scene/explored.rs` | `tests` | `src/scene/explored/tests.rs` |
| 57 | `src/scene/footprint.rs` | `tests` | `src/scene/footprint/tests.rs` |
| 58 | `src/scene/grid_shape.rs` | `tests` | `src/scene/grid_shape/tests.rs` |
| 59 | `src/scene/lighting.rs` | `tests` | `src/scene/lighting/tests.rs` |
| 60 | `src/scene/mod.rs` | `tests` | **Task 7** (`src/scene/tests/mod.rs` + subject files) |
| 61 | `src/scene/move_exec.rs` | `tests` | `src/scene/move_exec/tests.rs` |
| 62 | `src/scene/move_stream.rs` | `tests` | `src/scene/move_stream/tests.rs` |
| 63 | `src/scene/movement.rs` | `tests` | `src/scene/movement/tests.rs` |
| 64 | `src/scene/navmesh.rs` | `smoke` | `src/scene/navmesh/smoke.rs` |
| 65 | `src/scene/navmesh.rs` | `tests` | `src/scene/navmesh/tests.rs` |
| 66 | `src/scene/pathfinding.rs` | `astar_tests` | `src/scene/pathfinding/astar_tests.rs` |
| 67 | `src/scene/pathfinding.rs` | `find_tests` | `src/scene/pathfinding/find_tests.rs` |
| 68 | `src/scene/pathfinding.rs` | `tests` | `src/scene/pathfinding/tests.rs` |
| 69 | `src/scene/regions.rs` | `tests` | `src/scene/regions/tests.rs` |
| 70 | `src/scene/vision.rs` | `tests` | `src/scene/vision/tests.rs` |
| 71 | `src/ws/conn.rs` | `tests` | `src/ws/conn/tests.rs` |
| 72 | `src/ws/mod.rs` | `tests` | `src/ws/tests.rs` |
| 73 | `src/ws/protocol.rs` | `protocol_tests` | `src/ws/protocol/protocol_tests.rs` |
| 74 | `src/ws/room.rs` | `ring_tests` | `src/ws/room/ring_tests.rs` |
| 75 | `src/ws/room.rs` | `room_tests` | `src/ws/room/room_tests.rs` |
| 76 | `src/ws/time.rs` | `time_tests` | `src/ws/time/time_tests.rs` |

The two `pub(crate)` modules (#25, #52) are consumed cross-module (`crate::data::document::tests::world_scoped_doc`, `crate::http::tests::initialized_state`, `test_state`); the declaration keeps `pub(crate)` and those paths keep resolving unchanged.

**Batching:** Task 3 = rows 1–11. Task 4 = rows 12–35 except #33. Task 5 = rows 36–76 except #60. Task 6 = #33. Task 7 = #60.

- [x] **Step 1: Write the extraction tool in the scratchpad**

`$SCRATCH/extract-test-mod.mjs` (throwaway; do not add to the repo):

```js
// usage: node extract-test-mod.mjs <path/to/file.rs> <module>
// Moves the body of `#[cfg(test)] mod <module> { … }` to a sibling file and leaves a declaration.
import { readFileSync, writeFileSync, mkdirSync, existsSync } from "node:fs";
import { basename, dirname, join } from "node:path";

const [file, name] = process.argv.slice(2);
const raw = readFileSync(file, "utf8");
const eol = raw.includes("\r\n") ? "\r\n" : "\n";
const lines = raw.split(/\r?\n/);
const modRe = new RegExp(`^(pub(\\([a-z]+\\))?\\s+)?mod ${name} \\{$`);
const at = lines.findIndex((l, i) => modRe.test(l) && i > 0 && lines[i - 1].trim() === "#[cfg(test)]");
if (at < 0) throw new Error(`no '#[cfg(test)] mod ${name} {' in ${file}`);
// The module is a top-level item, so its closing brace is the first column-0 `}` after it.
const end = lines.findIndex((l, i) => i > at && l === "}");
if (end < 0) throw new Error(`no column-0 closing brace after line ${at + 1}`);
const body = lines.slice(at + 1, end).map((l) => (l.startsWith("    ") ? l.slice(4) : l));
const stem = basename(file, ".rs");
const dir = stem === "mod" ? dirname(file) : join(dirname(file), stem);
mkdirSync(dir, { recursive: true });
const target = join(dir, `${name}.rs`);
if (existsSync(target)) throw new Error(`${target} already exists`);
writeFileSync(target, body.join(eol).replace(/(\r?\n)*$/, "") + eol);
const decl = lines[at].replace(/ \{$/, ";");
lines.splice(at, end - at + 1, decl);
writeFileSync(file, lines.join(eol));
console.log(`${file}: mod ${name} -> ${target} (${body.length} lines)`);
```

- [x] **Step 2: Run it on rows 1–11**

From `src/server/`:

```bash
for f in src/auth/invite.rs src/auth/password.rs src/auth/role.rs src/auth/session.rs src/auth/setup.rs src/backup.rs src/config.rs src/db.rs src/health.rs src/modules.rs src/world_bundle.rs; do node "$SCRATCH/extract-test-mod.mjs" "$f" tests; done
```

Expected: 11 lines of `… -> … (N lines)`. Then `cargo fmt --all`.

- [x] **Step 3: Verify**

Run (from `src/server/`, each redirected to a file, exit status checked):
- `cargo test --all` — sum of `passed` across `test result:` lines **equals the Task 0 baseline**; 0 failed.
- `cargo clippy --all-targets -- -D warnings` — clean.
- `cargo clippy --all-targets -- -D missing-docs -D clippy::missing-docs-in-private-items` — clean.
- `cargo fmt --all -- --check` — clean.
- `pnpm lint:inline-tests` (repo root) — error count dropped from 76 to **65**.

If the doc-coverage clippy pass reports a missing doc on a moved item, the item was undocumented before as well and the lint did not fire because of module position; add the present-tense doc comment the item's siblings carry. Do not suppress.

- [x] **Step 4: Commit**

```bash
git add src/server/src/auth src/server/src/backup.rs src/server/src/backup src/server/src/config.rs src/server/src/config src/server/src/db.rs src/server/src/db src/server/src/health.rs src/server/src/health src/server/src/modules.rs src/server/src/modules src/server/src/world_bundle.rs src/server/src/world_bundle
git commit -m "refactor(server): move auth/backup/config/db/health/modules/world_bundle test modules to sibling files" -- src/server/src/auth src/server/src/backup.rs src/server/src/backup src/server/src/config.rs src/server/src/config src/server/src/db.rs src/server/src/db src/server/src/health.rs src/server/src/health src/server/src/modules.rs src/server/src/modules src/server/src/world_bundle.rs src/server/src/world_bundle
```

---

### Task 4: Batch 2 — `chat` and `data` (rows 12–35, excluding #33 `sqlite`)

**Files:** parents and targets from the Task 3 inventory rows 12–32, 34–35.

- [x] **Step 1: Extract**

From `src/server/`:

```bash
for f in src/chat/commands.rs src/chat/link_preview.rs src/chat/oembed.rs src/chat/post_publish.rs src/chat/preview_cache.rs src/chat/rolls.rs src/chat/sanitize.rs src/chat/settings.rs src/chat/shortcodes.rs src/data/asset.rs src/data/command.rs src/data/document.rs src/data/engine/mod.rs src/data/engine/token.rs src/data/migrate.rs src/data/permission.rs src/data/search.rs src/data/snapshot.rs src/data/validation.rs src/data/world_bundle.rs; do node "$SCRATCH/extract-test-mod.mjs" "$f" tests; done
node "$SCRATCH/extract-test-mod.mjs" src/chat/mod.rs tests
node "$SCRATCH/extract-test-mod.mjs" src/chat/mod.rs link_preview_ingest_tests
node "$SCRATCH/extract-test-mod.mjs" src/data/permission.rs required_cap_tests
cargo fmt --all
```

Expected: 23 extraction lines. Confirm `src/data/document.rs` now reads `#[cfg(test)]\npub(crate) mod tests;` (the tool preserves the visibility prefix).

- [x] **Step 2: Verify** — same five gates as Task 3 Step 3. `cargo test` passed-sum equals baseline. `pnpm lint:inline-tests` error count is now **42** (65 − 23). `pnpm lint:file-size` now reports only `HARD LIMIT src/server/src/data/sqlite.rs` and `SOFT LIMIT src/server/src/scene/mod.rs` (chat/mod.rs and permission.rs have dropped under 5,000; confirm with the gate output, not by arithmetic).

- [x] **Step 3: Commit**

```bash
git commit -m "refactor(server): move chat and data test modules to sibling files" -- src/server/src/chat src/server/src/data
```

(`git commit -- <dir>` records every tracked-and-modified file plus new files only if added; run `git add src/server/src/chat src/server/src/data` first, then commit with the same explicit paths.)

---

### Task 5: Batch 3 — `dice`, `http`, `scene` (except `mod.rs`), `ws` (rows 36–76, excluding #60)

**Files:** parents and targets from the Task 3 inventory rows 36–59, 61–76.

- [x] **Step 1: Extract**

From `src/server/`:

```bash
for f in src/dice/eval/classify.rs src/dice/eval/crit.rs src/dice/eval/expertise.rs src/dice/eval/groups.rs src/dice/eval/mod.rs src/dice/eval/success.rs src/dice/eval/sum.rs src/dice/notation/lexer.rs src/dice/notation/mod.rs src/dice/notation/parser.rs src/dice/outcome.rs src/dice/recalc.rs src/dice/rng.rs src/dice/spec.rs src/http/assets.rs src/http/embed.rs src/http/mod.rs src/http/module_routes.rs src/http/throttle.rs src/http/world_bundle.rs src/scene/explored.rs src/scene/footprint.rs src/scene/grid_shape.rs src/scene/lighting.rs src/scene/move_exec.rs src/scene/move_stream.rs src/scene/movement.rs src/scene/navmesh.rs src/scene/pathfinding.rs src/scene/regions.rs src/scene/vision.rs src/ws/conn.rs src/ws/mod.rs; do node "$SCRATCH/extract-test-mod.mjs" "$f" tests; done
node "$SCRATCH/extract-test-mod.mjs" src/scene/navmesh.rs smoke
node "$SCRATCH/extract-test-mod.mjs" src/scene/pathfinding.rs astar_tests
node "$SCRATCH/extract-test-mod.mjs" src/scene/pathfinding.rs find_tests
node "$SCRATCH/extract-test-mod.mjs" src/ws/protocol.rs protocol_tests
node "$SCRATCH/extract-test-mod.mjs" src/ws/room.rs ring_tests
node "$SCRATCH/extract-test-mod.mjs" src/ws/room.rs room_tests
node "$SCRATCH/extract-test-mod.mjs" src/ws/time.rs time_tests
cargo fmt --all
```

Expected: 40 extraction lines. `src/http/mod.rs` keeps `pub(crate) mod tests;`.

Note `src/scene/navmesh.rs`'s `mod smoke` sits at line 13, before the production items — the tool handles position-independently; the declaration stays where the body was.

- [x] **Step 2: Verify** — same five gates. `cargo test` passed-sum equals baseline. `pnpm lint:inline-tests` error count is now **2** (`src/server/src/data/sqlite.rs`, `src/server/src/scene/mod.rs`).

- [x] **Step 3: Commit**

```bash
git add src/server/src/dice src/server/src/http src/server/src/scene src/server/src/ws
git commit -m "refactor(server): move dice/http/scene/ws test modules to sibling files" -- src/server/src/dice src/server/src/http src/server/src/scene src/server/src/ws
```

---

### Task 6: `sqlite` tests → `src/data/sqlite/tests/` subject files

**Files:**
- Modify: `src/server/src/data/sqlite.rs` (module body removed, `#[cfg(test)] mod tests;` left at the same position)
- Create: `src/server/src/data/sqlite/tests/mod.rs`, `…/tests/rows_and_validation.rs`, `…/tests/search_and_worlds.rs`, `…/tests/commands_and_intents.rs`, `…/tests/invites_and_ownership.rs`

**Interfaces:** `tests/mod.rs` declares `mod rows_and_validation; mod search_and_worlds; mod commands_and_intents; mod invites_and_ownership;`, carries the module's `use` lines, and holds every helper used by two or more subject files as `pub(super) fn …`. Subject files start with `use super::*;` (which brings in the parent `sqlite` items via `tests/mod.rs`'s own `use super::*;` re-exported as `pub(super) use super::*;` — see Step 3).

- [x] **Step 1: Extract the whole module into `tests/mod.rs`**

From `src/server/`:

```bash
node "$SCRATCH/extract-test-mod.mjs" src/data/sqlite.rs tests
mkdir -p src/data/sqlite/tests
mv src/data/sqlite/tests.rs src/data/sqlite/tests/mod.rs
```

(A rename of a not-yet-tracked file; nothing is deleted.) Then run `cargo test --all` once: passed-sum equals baseline. Commit this intermediate state:

```bash
git add src/server/src/data/sqlite.rs src/server/src/data/sqlite
git commit -m "refactor(server): move sqlite test module to data/sqlite/tests/mod.rs" -- src/server/src/data/sqlite.rs src/server/src/data/sqlite
```

- [x] **Step 2: Cut `tests/mod.rs` into four subject files by test-function name**

Each subject file receives the **contiguous** span of `tests/mod.rs` from the first listed test (including any helper items and comments immediately preceding it that no earlier test uses) through the end of the last listed test. The `use` lines at the top of `tests/mod.rs` stay in `tests/mod.rs`. Every one of the 149 tests appears in exactly one file:

**`rows_and_validation.rs` (58 tests):** `values_semantically_eq_accepts_whole_number_float_vs_posint`, `values_semantically_eq_rejects_genuinely_stale_pre_image`, `values_semantically_eq_recurses_into_nested_array_and_object`, `values_semantically_eq_falls_back_to_exact_beyond_f64_precision`, `values_semantically_eq_accepts_negative_whole_number_variant_mismatch`, `values_semantically_eq_rejects_large_posint_pair_aliased_by_f64`, `values_semantically_eq_rejects_large_negint_pair_aliased_by_f64`, `values_semantically_eq_rejects_posint_vs_negint_same_magnitude`, `values_semantically_eq_accepts_equal_small_posint_pair`, `list_members_includes_usernames`, `list_members_orders_by_username`, `list_members_orders_case_insensitively`, `cannot_remove_sole_gm`, `cannot_demote_sole_gm`, `can_remove_gm_when_another_exists`, `repository_trait_member_role_matches_inherent_method`, `parent_id_round_trips_and_query_children_filters`, `delete_world_removes_every_keyed_row`, `delete_user_scrubs_everything`, `delete_user_guards`, `user_delete_nulls_asset_created_by`, `delete_world_not_found`, `upsert_member_inserts_updates_and_guards`, `scene_delete_purges_fog_via_apply_intent`, `scene_delete_purges_fog_via_apply_command`, `deleting_a_scene_expands_to_descendant_delete_ops`, `self_referential_parent_create_is_rejected`, `cross_world_parent_create_is_rejected`, `self_referential_parent_delete_terminates`, `query_scene_entities_returns_scenes_and_children_only`, `asset_insert_get_replace_delete_list_round_trip`, `contract_declarations_round_trip_and_default_empty`, `schema_declarations_round_trip_and_default_empty`, `worlds_for_user_scopes_to_membership_and_admin_sees_all`, `ui_state_merges_per_top_level_key_and_per_world`, `ui_state_merge_null_removes_key_and_entry`, `ui_state_merge_caps_the_merged_result_not_the_patch`, `explored_fog_round_trips_and_is_per_scene_user`, `create_with_invalid_engine_body_is_rejected`, `create_of_non_engine_doc_type_with_engine_body_is_rejected`, `update_post_image_with_invalid_engine_is_rejected`, `create_with_trailing_slash_property_override_key_is_rejected`, `create_with_missing_leading_slash_property_override_key_is_rejected`, `create_with_valid_property_override_keys_succeeds`, `update_with_trailing_slash_property_override_key_is_rejected`, `update_with_missing_leading_slash_property_override_key_is_rejected`, `update_with_valid_property_override_keys_succeeds`, `update_writing_a_valid_engine_subpath_succeeds`, `create_actor_omitting_faction_persists_explicit_null`, `apply_intent_update_normalizes_engine_broadcast_and_event_log_smuggled_key`, `apply_intent_update_normalizes_engine_integer_literal_to_stored_float`, `apply_command_update_normalizes_engine_broadcast_and_event_log_smuggled_key`, `apply_command_update_normalizes_engine_integer_literal_to_stored_float`, `apply_command_create_with_invalid_engine_body_is_rejected`, `apply_command_create_with_envelope_naming_override_is_rejected`, `apply_command_update_with_envelope_naming_override_is_rejected`, `declarative_requirement_blocks_writer_without_extra_cap`, `declarative_requirement_blocks_create_with_protected_subtree`.

**`search_and_worlds.rs` (26 tests):** `fts_sync_reflects_create_update_delete`, `search_ranks_and_filters_by_read_access`, `search_admits_the_inheriting_owner_of_a_default_none_linked_token`, `search_score_unaffected_by_gm_only_match_non_gm`, `search_paginates_without_underfill`, `world_cap_requirements_round_trip`, `world_enabled_modules_round_trip`, `user_by_username_and_admin_exists`, `settings_get_set_round_trip`, `create_admin_if_none_refuses_a_case_insensitive_username_collision`, `create_admin_if_none_guards_against_a_second_admin`, `create_then_get_world`, `members_carry_world_role`, `world_owned_seats_creator_as_gm`, `permission_context_resolves_role_or_forbids`, `set_remove_and_list_members`, `export_world_rows_resolves_owner_username_and_nulls_owner_in_json`, `export_world_rows_carries_manifest_watermark_and_row_counts`, `export_world_rows_not_found_for_unknown_world`, `import_world_round_trips_every_table_through_a_real_tar_bundle`, `import_world_nulls_owner_when_username_unresolvable`, `import_world_rejects_world_id_collision_before_writing_any_row`, `import_world_rejects_duplicate_singleton_document_before_writing_any_row`, `import_world_drops_fog_row_when_username_unresolvable`, `import_world_inserts_world_invites_row`, `import_world_rejects_document_with_unclassifiable_property_override`.

**`commands_and_intents.rs` (46 tests):** `non_gm_create_denied_by_default`, `non_gm_create_allowed_with_role_grant`, `role_grant_is_type_scoped`, `player_may_create_message_but_not_other_types`, `spectator_may_not_create_message`, `player_may_not_forge_message_owner_via_baseline_exemption`, `player_may_not_update_own_message`, `message_update_rejected_for_client_allowed_for_server_revision`, `create_update_delete_round_trip_via_invert`, `apply_command_on_unknown_world_fails_and_writes_nothing`, `seq_is_durable_across_reconnect`, `create_with_foreign_world_scope_is_rejected`, `delete_with_foreign_world_scope_is_rejected`, `update_cannot_change_document_id`, `update_stamps_updated_at_from_command_ts`, `query_documents_filters_by_world_and_type`, `query_all_documents_spans_multiple_doc_types`, `documents_by_source_finds_instances_for_push`, `events_since_returns_the_suffix`, `multi_op_command_snapshot_reflects_the_final_post_loop_state_for_every_op`, `create_op_snapshot_in_a_same_command_create_then_update_reflects_the_post_update_state`, `reused_id_gets_a_fresh_created_seq_and_the_stale_ops_own_snapshot_witnesses_the_old_one`, `events_since_back_compat_parses_a_bare_command_row_carrying_no_snapshot`, `apply_intent_create_then_conflicting_update`, `apply_intent_remove_makes_key_absent_and_occ_guards_the_removal`, `apply_intent_whole_band_replacement_removal_still_works`, `apply_intent_same_batch_create_then_engine_update_is_rejected`, `apply_intent_server_message_revision_may_write_property_overrides_but_nothing_else_under_permissions`, `apply_intent_server_message_revision_engine_write_ignores_a_declared_requirement_on_an_unrelated_doc_type`, `apply_intent_server_message_revision_write_to_an_unscoped_path_still_enforces_declared_requirements`, `apply_intent_rejects_unauthorized_and_oversized`, `apply_intent_update_gated_by_path_capability`, `apply_intent_granted_capability_enables_embedded`, `apply_intent_delete_requires_delete_capability`, `apply_intent_delete_broadcasts_stored_doc_not_client_body`, `apply_intent_world_default_grants_apply`, `apply_intent_create_violating_system_schema_is_rejected_and_seq_untouched`, `apply_intent_create_conforming_system_schema_succeeds`, `create_rejects_a_second_singleton_doc_of_the_same_type`, `create_allows_singleton_doc_types_in_different_worlds`, `create_does_not_gate_non_singleton_doc_types`, `create_gate_is_race_safe_under_concurrent_creates`, `create_rejects_intra_batch_duplicate_singleton_creates`, `create_rejects_n_way_intra_batch_duplicate_singleton_creates`, `create_allows_different_singleton_doc_types_in_the_same_batch`, `apply_intent_update_violating_system_schema_is_rejected_and_seq_untouched`.

**`invites_and_ownership.rs` (19 tests):** `consume_invite_seats_exactly_one_redeemer`, `consume_invite_refuses_expired_and_revoked_rows`, `revoke_invite_is_scoped_to_its_world`, `consume_invite_never_changes_a_role_already_held`, `list_invites_never_returns_the_stored_hash`, `create_invite_caps_live_invites_and_a_spent_one_frees_a_slot`, `linked_token_inherits_actor_owner_for_writes`, `per_token_owner_override_beats_the_linked_actors_owner`, `reassigning_the_actors_owner_moves_token_authority_with_no_restamp`, `ownership_fails_closed_on_every_degenerate_link`, `an_effective_owner_cannot_reassign_or_widen_ownership`, `effective_owner_of_joins_the_linked_actor_on_the_pool`, `the_owner_capability_floor_is_scoped_to_tokens`, `a_removal_carrying_a_new_value_is_rejected_at_ingress`, `the_actor_join_does_not_cross_world_scope`, `created_seq_is_set_once_and_survives_updates`, `created_seq_is_absent_for_a_missing_document`, `world_member_roles_reflects_every_current_member`, `get_document_with_created_seq_matches_a_separate_created_seq_read`.

- [x] **Step 3: Shape `tests/mod.rs` and hoist shared helpers**

`tests/mod.rs` becomes:

```rust
//! Repository tests, split by subject; shared fixtures live here.

mod commands_and_intents;
mod invites_and_ownership;
mod rows_and_validation;
mod search_and_worlds;

pub(super) use super::*;
// … the module's original `use crate::…` lines, each made `pub(super) use …;`
// … shared helpers (see below), each `pub(super) fn …`
```

Each subject file begins with `use super::*;`. Run `cargo test --no-run` and read the errors: every `cannot find function` names a helper the cutting left in another file. For each such helper: if only one subject file uses it, move it into that file; if two or more do, move it into `tests/mod.rs` as `pub(super)`. Repeat until `cargo test --no-run` is clean. Known cross-file candidates (verify with the compiler, do not assume): `repo`, `seed_world_rows`, `count_where`, `seed_session`, `session_count_for`, `tests_doc`, `tests_engine_doc`, `world_doc`, `update`, `singleton_test_doc`, `gm_create`. The `impl`-free helper `fog_purge_fixture`, `ui_state_of`, `seed_owned_message`, `world_with_player_owned_doc`, `invite_fixture`, `owned_token_doc`, `actor_doc_owned_by`, `try_move`, `ownership_fixture` are expected to be single-file.

Then `cargo fmt --all`.

- [x] **Step 4: Verify**

- `cargo test --all` passed-sum equals the baseline; 0 failed. Additionally `cargo test --all -- --list 2>/dev/null | grep -c 'data::sqlite::tests::'` (from `src/server/`, output to a file) equals **149**.
- `cargo clippy --all-targets -- -D warnings`, the `-D missing-docs …` clippy pass, `cargo fmt --all -- --check`: clean. Every new file and every `pub(super)` helper carries a doc comment (the private-items lint requires it for helpers that were already documented; add present-tense docs where the lint asks).
- `pnpm lint:file-size` — no error for `sqlite`; each file under `src/server/src/data/sqlite/tests/` is under 5,000 (the gate proves it).
- `pnpm lint:inline-tests` — error count **1** (`scene/mod.rs`).

- [x] **Step 5: Commit**

```bash
git add src/server/src/data/sqlite
git commit -m "refactor(server): split sqlite tests into subject files under data/sqlite/tests" -- src/server/src/data/sqlite.rs src/server/src/data/sqlite
```

---

### Task 7: `scene` tests → `src/scene/tests/` subject files

**Files:**
- Modify: `src/server/src/scene/mod.rs`
- Create: `src/server/src/scene/tests/mod.rs`, `…/tests/ecs_and_footprints.rs`, `…/tests/resolution_and_lighting.rs`, `…/tests/pathfind_and_vision.rs`

**Interfaces:** identical shape to Task 6: `tests/mod.rs` declares the three submodules, `pub(super) use super::*;`, re-exports the original `use` lines, holds shared helpers and constants as `pub(super)`, and keeps the test-only `impl SceneEcs { set_world_settings_for_test, insert_scene_for_test }` block (used across subjects).

- [x] **Step 1: Extract into `tests/mod.rs`**

From `src/server/`:

```bash
node "$SCRATCH/extract-test-mod.mjs" src/scene/mod.rs tests
mkdir -p src/scene/tests
mv src/scene/tests.rs src/scene/tests/mod.rs
```

`cargo test --all`: passed-sum equals baseline. Commit:

```bash
git add src/server/src/scene/mod.rs src/server/src/scene/tests
git commit -m "refactor(server): move scene test module to scene/tests/mod.rs" -- src/server/src/scene/mod.rs src/server/src/scene/tests
```

- [x] **Step 2: Cut into three subject files (contiguous spans, every one of the 150 tests in exactly one file)**

**`ecs_and_footprints.rs` (52 tests):** `hydrate_counts_scene_entities_only`, `resolve_grid_shape_selects_hex_grid_for_hex_kind_scenes`, `resolve_grid_shape_falls_back_to_square_grid_for_unrecognized_kind`, `engine_as_cache_invalidates_on_engine_mutation`, `apply_op_create_update_delete`, `segments_cross_truth_table`, `blocks_move_geometry_scene_scoping_and_filters`, `blocks_move_agrees_with_the_production_move_walls_segments_cross_path`, `token_move_uses_post_image_resisting_forged_bypasses`, `vision_channel_is_per_recipient`, `vision_payload_carries_lit_mask_for_players_not_gm`, `vision_payload_resolves_render_hint_index`, `resolvers_layer_world_then_scene_and_fail_closed`, `vision_modes_doc_is_respected_not_reseeded`, `pathfind_refuses_a_scene_with_no_document`, `user_owns_token_in_scene_follows_the_actor_join_and_is_scene_scoped`, `ecs_and_db_agree_on_ownership_after_a_remove_change_carrying_a_non_null_new`, `ecs_and_db_agree_when_a_set_change_relinks_a_token`, `ecs_actor_index_honors_a_remove_change_on_owner`, `a_malformed_proposed_path_fails_closed_without_an_error_level_log`, `a_failed_committed_mirror_change_logs_at_error_level`, `config_singleton_mirror_honors_a_remove_change`, `token_move_projection_honors_a_remove_change`, `token_ownership_resolves_through_the_actor_join_for_vision`, `token_vision_floors_resolve_through_actor_join`, `footprint_radius_on_square_is_the_conservative_enclosure_of_the_authored_block`, `footprint_radius_on_hex_is_the_circumscribing_radius_shape_is_inert`, `footprint_radius_falls_back_to_the_default_for_an_actorless_token`, `footprint_radius_honors_a_per_token_size_override`, `footprint_radius_refuses_an_oversized_token_rather_than_clamping`, `footprint_radius_admits_a_token_exactly_at_the_bound`, `footprints_payload_carries_a_square_token_extent_of_the_authored_block_in_scene_units`, `footprints_payload_carries_a_hex_token_extent_of_the_hexs_own_bounding_box`, `footprints_payload_states_a_refusal_as_a_null_extent_rather_than_a_size`, `the_footprints_channel_serves_the_resolved_payload_and_an_unknown_channel_errors`, `footprints_payload_omits_a_token_no_actor_sizes`, `footprints_payload_withholds_a_token_the_recipient_cannot_read`, `footprints_payload_withholds_a_scene_entry_the_recipient_cannot_read`, `footprints_payload_withholds_a_scene_whose_engine_band_the_recipient_may_not_see`, `footprints_payload_withholds_a_token_whose_engine_band_the_recipient_may_not_see`, `footprints_payload_withholds_a_token_whose_actors_engine_band_the_recipient_may_not_see`, `footprints_payload_withholds_a_token_whose_actor_document_the_recipient_may_not_read`, `footprints_payload_withholds_a_token_whose_embedded_actors_band_the_recipient_may_not_see`, `light_and_blockslight_wall_accessors_filter_by_scene`, `parse_hex_color_handles_6_and_3_digit`, `lit_mask_gates_los_by_illumination_and_darkvision`, `lit_mask_tags_darkvision_only_cells_with_hint`, `committed_seq_tracks_last_applied_command`, `config_and_actor_side_tables_track_ops`, `vision_modes_carry_render_hint`, `token_vision_floors_include_render_hint`, `token_vision_floors_falls_back_to_mode_default_range_when_assignment_omits_range`.

**`resolution_and_lighting.rs` (51 tests):** `diagonal_rule_defaults_to_chebyshev_without_world_settings`, `diagonal_rule_falls_back_when_structural_keys_absent`, `diagonal_rule_reads_world_settings_and_unknown_falls_back`, `resolve_scene_movement_restriction_defaults_to_visible_and_lenient`, `resolve_scene_movement_restriction_world_override_and_leniency_off`, `resolve_scene_movement_restriction_scene_override_beats_world`, `resolve_scene_movement_restriction_null_override_inherits_world`, `resolve_scene_movement_model_defaults_to_grid_stepped`, `resolve_scene_movement_model_world_override_to_continuous`, `resolve_scene_movement_model_scene_override_beats_world`, `resolve_scene_movement_model_null_scene_override_inherits_world`, `resolve_scene_bounds_defaults_when_absent`, `resolve_scene_bounds_reads_authored_value`, `resolve_scene_bounds_fail_closed_on_degenerate`, `the_resolved_shape_reports_the_resolved_kind`, `changing_a_scenes_grid_kind_invalidates_the_cached_visibility_mask`, `lit_mask_suppresses_hint_when_normal_floor_wins_in_bright_cell`, `cell_visible_predicate_honors_floor_and_range`, `visible_cells_strict_equals_player_lit_mask_cells`, `a_square_light_reaches_its_authored_bright_radius_past_the_bound_margin`, `a_square_light_occludes_behind_a_wall_within_its_grown_reach`, `env_light_occlusion_narrows_the_mask_and_seals_the_interior`, `hex_env_light_occlusion_seals_the_interior_like_the_square_path`, `hex_env_light_walks_the_blocks_real_origin_side_edges`, `env_light_open_scene_equals_global_illumination_no_holes`, `strict_parity_holds_with_env_light_occlusion`, `visible_cells_strict_parity_global_illumination`, `visible_cells_strict_parity_darkvision`, `visible_cells_strict_parity_los_restriction_with_occluding_wall`, `visible_cells_lenient_is_a_superset_of_strict`, `visible_cells_empty_when_user_has_no_source_in_scene`, `movement_gate_mask_cache_invalidates_on_wall_mutation`, `movement_gate_mask_cache_reused_across_repeated_moves_with_no_scene_change`, `region_field_authoritative_includes_secret_regions_visible_excludes_them`, `region_field_ignores_disabled_regions`, `move_walls_returns_only_blocks_move_segments_for_the_scene`, `move_walls_omits_a_gm_only_wall_for_a_player_viewer`, `move_walls_keeps_a_blocks_sight_false_wall_for_a_player`, `vision_and_lighting_keep_a_gm_only_wall_that_routing_drops`, `engine_tier_visible_admits_authoritative_and_rejects_a_non_owner_player_on_gm_only`, `absent_scene_yields_empty_visible_cells_not_a_synthesized_grid`, `absent_scene_region_field_is_none`, `absent_scene_navmesh_for_is_none`, `navmesh_for_is_memoized_across_calls`, `navmesh_for_distinguishes_footprint_radii`, `navmesh_for_rejects_degenerate_radius_even_after_cache_primed_at_zero`, `navmesh_for_does_not_share_a_mesh_across_differing_wall_sets`, `navmesh_for_shares_a_mesh_across_identical_wall_sets`, `navmesh_for_wall_key_is_order_independent`, `wall_mutation_invalidates_the_navmesh_cache`, `bounds_mutation_invalidates_the_navmesh_cache`.

**`pathfind_and_vision.rs` (47 tests):** `pathfind_gm_unconstrained_routes_without_a_mask`, `pathfind_dispatches_to_the_navmesh_router_for_a_continuous_scene`, `pathfind_grid_and_continuous_report_the_same_cell_cost_for_a_straight_route`, `pathfind_continuous_start_equals_goal_is_a_single_point_zero_cost`, `pathfind_continuous_terrain_bends_the_route_and_costs_cells`, `pathfind_hex_continuous_arrest_truncates_at_the_axial_hex_not_the_square_cell`, `pathfind_continuous_no_region_is_a_straight_polyanya_route`, `pathfind_continuous_impassable_routes_around`, `pathfind_continuous_secret_terrain_absent_from_player_route_present_for_gm`, `pathfind_continuous_nongm_route_clips_to_the_visible_mask`, `pathfind_continuous_weighted_nongm_route_clips_to_the_visible_mask`, `pathfind_continuous_secret_arrest_absent_from_player_preview_but_springs_at_execution`, `non_gm_route_crosses_a_gm_only_wall_that_springs_at_execution`, `gm_route_does_not_cross_a_gm_only_wall`, `pathfind_grid_stepped_scene_is_byte_for_byte_unchanged`, `pathfind_nongm_visible_is_bounded_by_the_mask`, `pathfind_revealed_unions_explored_memory`, `vision_at_grows_as_token_advances`, `vision_at_uses_full_wall_set`, `wall_less_scene_gives_full_intrascene_vision_not_a_degenerate_box`, `each_scenes_vision_bound_uses_its_own_extent_not_a_neighbours`, `wall_less_scene_vision_does_not_leak_beyond_its_own_bounds`, `player_vision_polygons_and_player_vision_inputs_agree_on_wall_less_bound`, `vision_at_empty_when_user_owns_no_token`, `player_lit_mask_wall_less_scene_covers_full_bounds_not_a_degenerate_box`, `visible_cells_wall_less_scene_covers_full_bounds_not_a_degenerate_box`, `visible_cells_agrees_with_player_vision_polygons_bound_on_wall_less_scene`, `accumulate_visible_cells_routes_through_grid_shape_cell_center_not_hardcoded`, `player_lit_mask_routes_through_grid_shape_cell_center_not_hardcoded`, `visible_cells_hex_excludes_cell_whose_center_is_outside_the_mask`, `visible_cells_hex_lenient_includes_cell_whose_vertex_clips_the_mask`, `hex_lenient_mask_lets_the_executor_enter_a_cell_the_strict_mask_stops_at`, `a_hex_vision_range_is_measured_in_grid_steps`, `a_hex_vision_range_bounds_the_lit_egress_the_same_way`, `a_hex_light_radius_is_measured_in_grid_steps`, `an_over_cap_visibility_scan_yields_a_bounded_mask_not_an_empty_one`, `an_over_cap_lit_mask_scan_yields_a_bounded_cell_set_not_an_empty_one`, `lenient_visibility_scan_stays_a_superset_of_strict_at_the_clamp_boundary`, `parity_holds_inside_the_clamp_band`, `scene_world_extent_agrees_with_the_shapes_own_conversion`, `hex_continuous_routes_along_axial_row_zero_strictly_inside_the_mesh`, `hex_continuous_routes_below_the_origin_row_inside_its_own_hexes`, `hex_continuous_navmesh_spans_the_authored_play_area`, `hex_continuous_weighted_cost_is_reported_in_cells`, `a_degenerate_authored_grid_size_never_reaches_the_extent_conversion`, `navmesh_for_refuses_a_radius_over_the_footprint_cap`, `navmesh_for_refuses_a_scene_whose_converted_extent_is_over_magnitude`.

- [x] **Step 3: Shape `tests/mod.rs` and hoist shared items**

Same procedure as Task 6 Step 3. Items that stay in `tests/mod.rs` regardless: the `use` lines (as `pub(super) use`), the `impl SceneEcs { … }` test-only block, and every constant/struct/helper the compiler reports as used from two or more subject files. Constants (`HEX_FIXTURE_SIZE`, `FOOTPRINT_TEST_SCENE`, `HEX_SEALED_CELL`, `HEX_VISION_RANGE_CELLS`, `HEX_LIGHT_BRIGHT_CELLS`, `HEX_LIGHT_DIM_CELLS`, `HEX_LIGHT_BLOCK`) and structs (`LevelCapture`, `RegionRect`) become `pub(super)` when hoisted. Expected cross-file helpers (verify with the compiler): `doc`, `entity_doc_eng`, `entity_doc_top_eng`, `actor_body`, `fc`, `scene_with_grid`, `wall_doc_eng`, `scene_with_lit_player_token`. Then `cargo fmt --all`.

- [x] **Step 4: Verify**

- `cargo test --all` passed-sum equals baseline; 0 failed; `cargo test --all -- --list | grep -c 'scene::tests::'` equals **150**.
- Both clippy passes, `cargo fmt --all -- --check`: clean.
- `pnpm lint:file-size` — **exit 0**, zero errors (first clean run; record the output line in `progress.md`).
- `pnpm lint:inline-tests` — **exit 0**, `76 → 0`.

- [x] **Step 5: Commit**

```bash
git add src/server/src/scene/tests
git commit -m "refactor(server): split scene tests into subject files under scene/tests" -- src/server/src/scene/mod.rs src/server/src/scene/tests
```

---

### Task 8: CI wiring, CLAUDE.md rule, memory, TODO entry, skill update

**Files:**
- Modify: `.github/workflows/ci.yml` (docs job, after the `lint:allowances` step)
- Modify: `.claude/CLAUDE.md` (new section after "Lint Suppressions Require Explicit User Approval")
- Modify: `docs/TODO.md`
- Create: `~/.claude/projects/C--Dev-Shadowcat/memory/file-size-limits-need-explicit-signoff.md`; Modify: `~/.claude/projects/C--Dev-Shadowcat/memory/MEMORY.md`
- Modify (plugin checkout, separate repo): `~/.claude/skills/shadowcat-codebase/skills/shadowcat-codebase-core/SKILL.md`

- [x] **Step 1: CI steps**

In `.github/workflows/ci.yml`, immediately after:

```yaml
      - name: No dead-code or unused-item suppressions
        run: pnpm lint:allowances
```

add:

```yaml
      - name: File-size limits (soft 5000 / hard 10000 lines)
        run: pnpm lint:file-size
      - name: No inline Rust test-module bodies
        run: pnpm lint:inline-tests
```

- [x] **Step 2: CLAUDE.md rule**

Insert after the "Lint Suppressions Require Explicit User Approval" section (after its `#### ✅ Good` block, before the "Agent-Optimized Security & IP Standards" heading):

```markdown
## File-Size Limits Require Explicit User Approval
**Core Directive:** A source file over 5,000 lines is a defect; over 10,000 is a build failure.
Enforced by `pnpm lint:file-size` (`scripts/check-file-lines.mjs`) over every tracked
`rs ts js mjs svelte scss css` file under `src/`, `scripts/`, `examples/` (generated types
excluded) — a **gate, never a ratchet**: nothing is grandfathered. Test lines count.

### 1. The Soft Limit Needs the Owner's Signature
Crossing 5,000 lines fails unless the file has an entry in `.claude/file-size-allowlist.toml`.
**Every entry requires the user's explicit, per-file authorization in the conversation. No agent
adds, edits, or retains an entry on its own authority**, and an oversize file found in the tree is
a defect to split, not a precedent to follow. A stale entry (file back under the limit) fails the
gate in its own right.

### 2. The Hard Limit Has No Override
Over 10,000 lines fails with or without an allowlist entry. Split the file.

### 3. Rust Test Modules Live in Sibling Files
`pnpm lint:inline-tests` (`scripts/check-inline-tests.mjs`) fails on any inline
`#[cfg(test)] mod x { … }` body in `src/**/*.rs`. Declare `#[cfg(test)] mod x;` and put the body
in `<stem>/x.rs` (or `x.rs` beside a `mod.rs`); `use super::*` resolves to the same parent, so
nothing widens. `#[cfg(test)]` on a non-module item (a test-only helper, field or `impl`) stays in
the production file. A test file that itself outgrows the limit is split by subject under
`<stem>/tests/<subject>.rs` with shared fixtures in `<stem>/tests/mod.rs`.

#### ❌ Bad (Inline Body / Self-Granted Exception)
```rust
#[cfg(test)]
mod tests { /* 7,000 lines */ }
```
```toml
# added by an agent "because the split is large"
[[file]]
path = "src/server/src/data/sqlite.rs"
```

#### ✅ Good (Declaration + Sibling File; Split Instead of Exempt)
```rust
#[cfg(test)]
mod tests;   // body in data/sqlite/tests/mod.rs + tests/<subject>.rs
```
```

- [x] **Step 3: TODO entry**

Append to `docs/TODO.md` under a new heading at the end:

```markdown
## Actionable now — next file-size split candidate
- TODO: `src/server/src/data/sqlite.rs` production code is ~3,900 lines after its test module moved out — the largest remaining production file and the next to cross the 5,000-line soft limit at its growth rate. Split `SqliteRepository` by concern (documents/commands, membership/invites, search, world export/import) into `data/sqlite/<concern>.rs` `impl` blocks before it reaches the limit; the gate (`pnpm lint:file-size`) fails the build at that point and no allowlist entry is to be added.
```

- [x] **Step 4: Memory file**

Create `~/.claude/projects/C--Dev-Shadowcat/memory/file-size-limits-need-explicit-signoff.md`:

```markdown
---
name: file-size-limits-need-explicit-signoff
description: "IRON-CLAD: source files are capped at 5,000 lines (soft, owner-approved allowlist only) and 10,000 (hard, no override); Rust test modules live in sibling files. No agent adds a `.claude/file-size-allowlist.toml` entry on its own authority — split the file instead."
metadata:
  type: feedback
---

**User directive (2026-08-27):** soft limit 5,000 lines, hard limit 10,000, CI-enforced and
retroactive; oversize files get split; test modules move to separate files across the board;
*"I have to approve every allowlist entry. It should be instituted in the rules never to add
something without my explicit authorization."*

**Why:** a file that no longer fits in one reading is edited by fragment-matching, by agents and
reviewers alike; the limit removes the class. An allowlist entry is a scope decision (what the
work covers), never a technical one — the same shape as [[no-justified-keeps-without-explicit-signoff]].

**How to apply:**
- Never write to `.claude/file-size-allowlist.toml` without the user's explicit per-file
  authorization in the conversation; never propose one as a way to clear the gate — propose the split.
- The hard limit cannot be allowlisted. Do not look for a mechanism.
- New Rust tests go in `<stem>/tests.rs` (or `<stem>/tests/<subject>.rs` when large), declared
  `#[cfg(test)] mod tests;` — `pnpm lint:inline-tests` fails inline bodies.
- Carry both rules into every dispatch brief verbatim.

Related: [[never-work-around-a-rule-follow-its-intent]], [[no-justified-keeps-without-explicit-signoff]].
```

Add to `MEMORY.md` under "## User-set design directives", after the `no-justified-keeps` line:

```markdown
- [**IRON-CLAD: 5k soft / 10k hard file-size limits; allowlist entries need the user's explicit sign-off; Rust tests in sibling files**](file-size-limits-need-explicit-signoff.md) — split, never exempt.
```

- [x] **Step 5: Core skill update (plugin checkout)**

In `~/.claude/skills/shadowcat-codebase/skills/shadowcat-codebase-core/SKILL.md`, under `## Hard invariants`, add an entry in the section's existing style:

```markdown
- **File-size limits and test-file placement.** `pnpm lint:file-size` fails any tracked source file
  over 5,000 lines without an owner-signed `.claude/file-size-allowlist.toml` entry and any file
  over 10,000 unconditionally; `pnpm lint:inline-tests` fails any inline `#[cfg(test)] mod x { … }`
  body under `src/`. Rust test bodies live in `<stem>/x.rs` (or `x.rs` beside a `mod.rs`), and the
  large suites are split by subject: `data/sqlite/tests/{mod,rows_and_validation,search_and_worlds,commands_and_intents,invites_and_ownership}.rs`
  and `scene/tests/{mod,ecs_and_footprints,resolution_and_lighting,pathfind_and_vision}.rs`, with
  shared fixtures `pub(super)` in each `tests/mod.rs`. Never add an allowlist entry on your own
  authority — split the file.
```

Also add both gates to whichever list in that skill enumerates the CI gate battery (search the file for `lint:allowances`; add `pnpm lint:file-size` and `pnpm lint:inline-tests` beside it).

Run from the repo root: `node scripts/check-skill-symbol-refs-cli.mjs`, `node scripts/check-skill-api-refs-cli.mjs`, `pnpm run test:scripts` — each to a file, each exit 0. Dispatch `shadowcat-codebase:shadowcat-spec-reviewer` (`effort: high`) on the skill diff (pre-generated to a scratchpad file) with the question "does this diff accurately capture Tasks 1–7's seams with no omission, drift, or broken pointer?" — PASS required. Commit and push inside the plugin checkout:

```bash
cd ~/.claude/skills/shadowcat-codebase && git add skills/shadowcat-codebase-core/SKILL.md && git commit -m "docs(core): file-size limits, inline-test gate, sqlite/scene test-file layout" -- skills/shadowcat-codebase-core/SKILL.md && git push
```

- [x] **Step 6: Run the full gate battery (Task 0 list) and commit**

Every gate in the Task 0 list, each to a file, each exit 0. Then:

```bash
git commit -m "ci: enforce file-size limits and inline-test extraction; document the rule" -- .github/workflows/ci.yml .claude/CLAUDE.md docs/TODO.md
```

---

### Task 9: `pnpm clean` — recoverable build-output cleaning ahead of every build

**Files:**
- Create: `scripts/clean-build-outputs.mjs`
- Create: `scripts/clean-build-outputs.test.mjs`
- Modify: `package.json` (devDependency `trash`, scripts `clean`, `build`, `build:all`)
- Modify: `scripts/assemble-docs.mjs` (`assemble` uses the shared remover)
- Modify: `scripts/assemble-docs.test.mjs` (only if `assemble`'s sync→async change breaks a call; read it first)
- Modify: `scripts/package.sh` (line `rm -rf "$out"`)

**Interfaces:**
- Produces (exported): `TARGETS` (array of repo-relative patterns), `resolveTargets(root: string, patterns: string[]): string[]` (absolute existing dirs), `assertAllowed(root: string, absPath: string): void` (throws unless the path is under `root` and its repo-relative form is one of `dist`, `dist-docs`, `docs/site/.vitepress/dist`, `examples/<name>/dist`, `target/package`), `clean({root, patterns, remove}): Promise<string[]>` (returns removed paths; `remove` is `(absPath) => Promise<void>`), `removeRecoverably(absPath): Promise<void>` (the `trash`-backed default).
- CLI: `node scripts/clean-build-outputs.mjs` cleans every target; `--only <pattern>` restricts to one listed pattern (rejects unlisted).

**Dependency consent:** `trash` (npm, MIT, cross-platform recycle-bin) is a new devDependency. This is the one dependency the plan adds; the user has been asked for consent at plan handoff. If consent is withheld, stop this task and ask — do not substitute `rmSync`.

- [x] **Step 1: Write the failing tests**

`scripts/clean-build-outputs.test.mjs`:

```js
import { mkdtempSync, mkdirSync, writeFileSync, existsSync } from "node:fs";
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";
import { test, expect } from "vitest";
import { TARGETS, resolveTargets, assertAllowed, clean } from "./clean-build-outputs.mjs";

function repo() {
  const root = mkdtempSync(join(tmpdir(), "clean-"));
  for (const d of ["dist", "dist-docs", "docs/site/.vitepress/dist", "examples/a/dist", "examples/b/dist", "target/package", "src/client/core/src"]) {
    mkdirSync(join(root, d), { recursive: true });
    writeFileSync(join(root, d, "x.txt"), "x");
  }
  return root;
}

test("TARGETS is the enumerated list and nothing else", () => {
  expect(TARGETS).toEqual(["dist", "dist-docs", "docs/site/.vitepress/dist", "examples/*/dist", "target/package"]);
});

test("resolveTargets expands examples/*/dist and skips absent dirs", () => {
  const root = repo();
  const got = resolveTargets(root, TARGETS).map((p) => p.slice(root.length + 1).split("\\").join("/")).sort();
  expect(got).toEqual(["dist", "dist-docs", "docs/site/.vitepress/dist", "examples/a/dist", "examples/b/dist", "target/package"]);
  expect(resolveTargets(root, ["dist-docs"]).length).toBe(1);
  expect(resolveTargets(mkdtempSync(join(tmpdir(), "empty-")), TARGETS)).toEqual([]);
});

test("assertAllowed refuses anything outside the enumerated output dirs", () => {
  const root = repo();
  expect(() => assertAllowed(root, join(root, "dist"))).not.toThrow();
  expect(() => assertAllowed(root, join(root, "examples", "a", "dist"))).not.toThrow();
  expect(() => assertAllowed(root, join(root, "src", "client", "core"))).toThrow(/refus/i);
  expect(() => assertAllowed(root, join(root, "docs", "site"))).toThrow(/refus/i);
  expect(() => assertAllowed(root, resolve(root, ".."))).toThrow(/refus/i);
  expect(() => assertAllowed(root, join(root, "dist", "..", "src"))).toThrow(/refus/i);
});

test("clean removes exactly the resolved targets through the injected remover", async () => {
  const root = repo();
  const removed = [];
  const out = await clean({ root, patterns: TARGETS, remove: async (p) => { removed.push(p); } });
  expect(out.length).toBe(6);
  expect(removed).toEqual(out);
  expect(removed.some((p) => p.includes("src"))).toBe(false);
  expect(existsSync(join(root, "src/client/core/src/x.txt"))).toBe(true);
});

test("clean with an unlisted pattern refuses before removing anything", async () => {
  const root = repo();
  const removed = [];
  await expect(clean({ root, patterns: ["src"], remove: async (p) => { removed.push(p); } })).rejects.toThrow(/refus/i);
  expect(removed).toEqual([]);
});
```

- [x] **Step 2: Run to verify it fails** — `npx vitest run scripts/clean-build-outputs.test.mjs` → cannot resolve module.

- [x] **Step 3: Add the dependency and write the script**

`pnpm add -D -w trash@^9` (updates `package.json` and `pnpm-lock.yaml`).

`scripts/clean-build-outputs.mjs`:

```js
// Empties the build-output directories, recoverably, before a build.
//
// Every output directory is listed here by name and nothing else can be a target: the remover
// asserts each resolved path against this list before touching it, so a pattern typo or a stray
// argument cannot reach `src/`, `docs/`, or the repository root. Removal goes to the OS recycle
// bin / trash rather than an unlink, so a wrong target is recoverable at the moment it happens.
//
// Vite's own `emptyOutDir` and the docs assembler previously each cleared their own output; one
// enumerated list is the single place a new output directory is registered.

import { existsSync, readdirSync, statSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import process from "node:process";
import trash from "trash";
import { isDirectEntry } from "./lib/is-main.mjs";
import { norm } from "./lib/gate-corpus.mjs";

export const TARGETS = ["dist", "dist-docs", "docs/site/.vitepress/dist", "examples/*/dist", "target/package"];

const ALLOWED = /^(dist|dist-docs|docs\/site\/\.vitepress\/dist|examples\/[^/]+\/dist|target\/package)$/;

/** Throws unless `absPath` is one of the enumerated output directories under `root`. */
export function assertAllowed(root, absPath) {
  const rel = norm(relative(resolve(root), resolve(absPath)));
  if (rel === "" || rel.startsWith("..") || resolve(absPath) !== resolve(root, rel) || !ALLOWED.test(rel)) {
    throw new Error(`clean-build-outputs: refusing to remove '${absPath}' — not an enumerated build-output directory`);
  }
}

/** Existing directories matching the patterns (only `examples/*/dist` carries a wildcard). */
export function resolveTargets(root, patterns) {
  const out = [];
  for (const pat of patterns) {
    if (!TARGETS.includes(pat)) throw new Error(`clean-build-outputs: refusing unlisted pattern '${pat}'`);
    if (pat === "examples/*/dist") {
      const ex = join(root, "examples");
      if (!existsSync(ex)) continue;
      for (const name of readdirSync(ex)) {
        const p = join(ex, name, "dist");
        if (existsSync(p) && statSync(p).isDirectory()) out.push(p);
      }
    } else {
      const p = join(root, ...pat.split("/"));
      if (existsSync(p) && statSync(p).isDirectory()) out.push(p);
    }
  }
  return out;
}

/** Sends a path to the OS recycle bin / trash. */
export async function removeRecoverably(absPath) {
  await trash(absPath, { glob: false });
}

/** Resolves, asserts, then removes; returns the removed paths. Nothing is removed if any assertion fails. */
export async function clean({ root, patterns, remove = removeRecoverably }) {
  const targets = resolveTargets(root, patterns);
  for (const t of targets) assertAllowed(root, t);
  for (const t of targets) await remove(t);
  return targets;
}

async function main() {
  const root = resolve(process.cwd());
  const onlyIdx = process.argv.indexOf("--only");
  const patterns = onlyIdx >= 0 ? [process.argv[onlyIdx + 1]] : TARGETS;
  const removed = await clean({ root, patterns });
  for (const p of removed) console.log(`trashed ${norm(relative(root, p))}`);
  console.log(`clean: ${removed.length} output director${removed.length === 1 ? "y" : "ies"} sent to trash`);
}

if (isDirectEntry(import.meta.url)) main().catch((e) => { console.error(e.message); process.exit(1); });
```

- [x] **Step 4: Wire the scripts**

`package.json` scripts:

```json
    "clean": "node scripts/clean-build-outputs.mjs",
    "build": "pnpm clean && pnpm --filter @shadowcat/shell build",
    "build:all": "pnpm build && pnpm docs:generate",
```

(`build:all` already begins with `pnpm build`, so it inherits the clean; do not add a second call.)

- [x] **Step 5: `assemble-docs.mjs` and `package.sh`**

In `scripts/assemble-docs.mjs`: replace the `rmSync` import with `import { removeRecoverably } from "./clean-build-outputs.mjs";` and `import { existsSync } …` (already imported); change `assemble` to:

```js
export async function assemble({ portal, ts, rust, out }) {
  if (existsSync(out)) await removeRecoverably(out);
  mkdirSync(out, { recursive: true });
  cpSync(portal, out, { recursive: true });
  cpSync(ts, join(out, "api", "ts"), { recursive: true });
  cpSync(rust, join(out, "api", "rust"), { recursive: true });
}
```

Update the doc comment's "cleared first" sentence to say the output is sent to the trash before regeneration. `await` its call site in the script's main block; read `scripts/assemble-docs.test.mjs` and make any test that calls `assemble` `await` it. Note `removeRecoverably` here is deliberately not routed through `assertAllowed` — `out` is `dist-docs`, and the assembler's own argument is the enumerated value; if the test suite passes a temp dir, that is the reason for not asserting.

In `scripts/package.sh`, replace `rm -rf "$out"` with:

```bash
node "$root/scripts/clean-build-outputs.mjs" --only target/package
```

(`target/package` is in `TARGETS`; `--only` rejects anything else. CI runs `pnpm install` before this step on the packaging runners, so `trash` is present.)

- [x] **Step 6: Verify**

- `npx vitest run scripts/clean-build-outputs.test.mjs scripts/assemble-docs.test.mjs` — PASS.
- `pnpm run test:scripts` — PASS (whole suite).
- `pnpm build` — output shows `clean: N output directories sent to trash` then the Vite build; `dist/` is freshly produced.
- `pnpm build:all` — completes; `dist-docs/` regenerated.
- `git status` — nothing under `src/` touched; `pnpm-lock.yaml` and `package.json` modified.
- Full Task 0 gate battery — every gate exit 0.

- [x] **Step 7: Commit**

```bash
git commit -m "build: recoverable enumerated clean of build outputs ahead of every build" -- scripts/clean-build-outputs.mjs scripts/clean-build-outputs.test.mjs scripts/assemble-docs.mjs scripts/assemble-docs.test.mjs scripts/package.sh package.json pnpm-lock.yaml
```

---

### Task 10: Documentation sync, graph update, push

**Files:**
- Modify: `docs/superpowers/plans/2026-08-27-file-size-limits.md` (check every box)
- Modify: `docs/PLAN.md` if it carries a Phase-1/hardening list that names CI gates (read it; add the two gates where `lint:allowances` is listed, else no change — state which)
- Run: `graphify update .`

- [x] **Step 1:** Check every task box in this plan; confirm `docs/TODO.md` carries the Task 8 entry; confirm `docs/OPEN_BUGS.md` needs no change (no bug was opened or closed — state so).
- [x] **Step 2:** `graphify update .` — commit `graphify-out/` changes if the repo tracks them (check `git status`).
- [x] **Step 3:** Full Task 0 gate battery one final time, each to a file, each exit 0. Baseline test count re-confirmed.
- [x] **Step 4:** Commit docs: `git commit -m "docs: file-size-limits plan complete" -- docs/superpowers/plans/2026-08-27-file-size-limits.md docs/PLAN.md graphify-out` (drop paths that did not change).
- [x] **Step 5:** This is a full milestone: `git push origin main`, then `gh run watch` until every job is green on all three OS runners. If red, fix forward from the topmost error.
