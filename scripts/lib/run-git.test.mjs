import { test, expect } from "vitest";
import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { runGit, scrubGitEnv } from "./run-git.mjs";
import { norm, repoFiles } from "./gate-corpus.mjs";
import { makeRepo } from "./temp-repo.mjs";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

// Every scratch-repository fixture below drives git directly (a throwaway repository is built, not
// queried, so `runGit`'s legible-failure contract has nothing to add) and passes `scrubGitEnv()`
// as its `env`. This file runs from `gate:commit`'s pre-commit hook, so during a partial
// `git commit -- <paths>` the test process itself carries the enclosing commit's `GIT_INDEX_FILE`;
// a fixture call inheriting it contends for the lock on that commit's in-flight index instead of
// touching its own scratch repository. The `directGitCalls` scan at the bottom of this file holds
// every test file in the tree to the same rule.

test("runGit returns the trimmed stdout on success", () => {
  const execFile = () => "  abc123  \n";
  const result = runGit(["rev-parse", "HEAD"], "HEAD", { execFile });
  expect(result).toEqual({ ok: true, stdout: "abc123" });
});

test("runGit reports a legible failure naming the command and what could not be determined, never a raw exception", () => {
  const execFile = () => {
    throw new Error("fatal: not a git repository");
  };
  const result = runGit(["rev-parse", "--git-dir"], "the sequencer state", { execFile });
  expect(result.ok).toBe(false);
  expect(result.message).toContain("the sequencer state");
  expect(result.message).toContain("git rev-parse --git-dir");
  expect(result.message).toContain("fatal: not a git repository");
});

// Regression: `-C <dir>` does not override GIT_INDEX_FILE/GIT_DIR/etc — git resolves those
// pointer variables first, so a command aimed at an unrelated repository via `-C` silently reads
// the CALLER's repository state instead. This exercises real `execFileSync` against two real
// scratch repositories (no mocked `execFile`), the same shape as the measured 32-vs-1490-file
// discrepancy the fix responds to: a `git -C <repoA> ls-files` call made while `GIT_INDEX_FILE`
// points at `<repoB>`'s index must still enumerate `repoA`.
test("runGit's default environment scrub keeps a -C target immune to an inherited GIT_INDEX_FILE", () => {
  const repoA = mkdtempSync(join(tmpdir(), "run-git-repoA-"));
  const repoB = mkdtempSync(join(tmpdir(), "run-git-repoB-"));
  for (const repo of [repoA, repoB]) {
    execFileSync("git", ["init", "-q"], { cwd: repo, env: scrubGitEnv() });
    execFileSync("git", ["config", "user.email", "test@example.com"], {
      cwd: repo,
      env: scrubGitEnv(),
    });
    execFileSync("git", ["config", "user.name", "Test"], { cwd: repo, env: scrubGitEnv() });
  }
  writeFileSync(join(repoA, "only-in-a.txt"), "a\n");
  execFileSync("git", ["add", "-A"], { cwd: repoA, env: scrubGitEnv() });
  writeFileSync(join(repoB, "only-in-b.txt"), "b\n");
  execFileSync("git", ["add", "-A"], { cwd: repoB, env: scrubGitEnv() });

  // Mutates the REAL process.env rather than passing a scoped `env` override — this is the actual
  // inheritance shape git produces (exported into the environment of every child process it
  // spawns), and it is the only shape that fails honestly against a `runGit` with no scrub: a
  // `runGit` that ignores its `env` deps entirely still passes a version of this test that merely
  // passes `env` through `deps`, because that path is never exercised.
  const savedIndexFile = process.env.GIT_INDEX_FILE;
  process.env.GIT_INDEX_FILE = join(repoB, ".git", "index");
  try {
    const result = runGit(["-C", repoA, "ls-files"], "the tracked file list");
    expect(result).toEqual({ ok: true, stdout: "only-in-a.txt" });
  } finally {
    if (savedIndexFile === undefined) delete process.env.GIT_INDEX_FILE;
    else process.env.GIT_INDEX_FILE = savedIndexFile;
  }
});

test("a caller-supplied env still reaches the child process, scrubbed the same way", () => {
  let capturedEnv;
  const execFile = (cmd, args, opts) => {
    capturedEnv = opts.env;
    return "";
  };
  runGit(["status"], "status", {
    execFile,
    env: { CUSTOM_MARKER: "present", GIT_DIR: "/somewhere/else" },
  });
  expect(capturedEnv.CUSTOM_MARKER).toBe("present");
  expect(capturedEnv.GIT_DIR).toBeUndefined();
});

test("GIT_EXEC_PATH survives the scrub", () => {
  let capturedEnv;
  const execFile = (cmd, args, opts) => {
    capturedEnv = opts.env;
    return "";
  };
  runGit(["status"], "status", {
    execFile,
    env: { GIT_EXEC_PATH: "/usr/lib/git-core", GIT_DIR: "/somewhere/else" },
  });
  expect(capturedEnv.GIT_EXEC_PATH).toBe("/usr/lib/git-core");
  expect(capturedEnv.GIT_DIR).toBeUndefined();
});

test("scrubGitEnv returns a scrubbed copy and leaves its input untouched", () => {
  const input = { GIT_INDEX_FILE: "/some/index", GIT_EXEC_PATH: "/usr/lib/git-core", KEEP: "1" };
  const scrubbed = scrubGitEnv(input);
  expect(scrubbed).toEqual({ GIT_EXEC_PATH: "/usr/lib/git-core", KEEP: "1" });
  expect(input.GIT_INDEX_FILE).toBe("/some/index");
});

// Source-scanning regression test, same precedent as run-gate-tier.test.mjs's GIT-CALL-BUDGET
// check: an unguarded `execFileSync` git call is a per-entry-point mistake, not a property of any
// one file, so fixing the call at one entry point proves nothing about a further one recurring
// at another — the check enumerates every tracked entry point instead of re-verifying named ones.
//
// Enumerates via `git ls-files` rather than walking a named directory tree: a directory-walk scan
// root is itself an include-list one level up from the file-level exclude-list below it — it sees
// every file under `scripts/`, but a git-invoking entry point living anywhere else in the tracked
// tree (a Claude Code hook under `.claude/hooks/`, or any future location) is invisible to it
// until someone remembers to widen the root. Tracked-file enumeration has no location to omit:
// every JavaScript/TypeScript file git tracks, anywhere in the repository, is in scope by
// construction, and ignored directories (`node_modules`, `dist`, build output) and untracked
// scratch content are already excluded because they were never tracked.
//
// Every scanned file lands in exactly ONE of two checks, decided by name: a test file (`.test.` or
// `.spec.` before its extension) answers to the FIXTURE invariant, every other file to the
// PRODUCTION invariant. The name pattern is a ROUTER between two checks, never an exclusion — a
// pattern that skipped test files outright covers every test file written from now on with nothing
// to review, and the one call shape it waves through (a fixture's raw `git init`/`git add`
// inheriting the hook environment) is exactly the shape that lands on the real repository's index.
//
// PRODUCTION invariant: route through `runGit`, so a failure reaches the operator as a sentence and
// the child never inherits git's pointer variables. FIXTURE invariant: a direct git call is
// legitimate (a throwaway repository is built, not queried) but must pass an explicit `env`,
// because the test process runs inside `git commit`'s hook environment. A test HELPER module (not
// itself named as a test) is routed to the production check, which is the stricter of the two —
// `runGit` satisfies both invariants at once — so a misrouted helper fails loudly rather than
// silently. RESIDUAL: the fixture check verifies an `env` property is PRESENT and is not the bare
// `process.env`; any other value (a variable holding an unscrubbed copy) is a review obligation.
//
// "Direct git call" covers every child-process spawn form (`execFileSync`, `execFile`, `spawnSync`,
// `spawn`, `execSync`, `exec`), a `const` bound to `"git"`, and a helper wrapper around any of them
// (the spawn inside the wrapper is the call). A fixture that creates a repository in a linked
// worktree's hook environment with an inherited `GIT_DIR` re-initialises that worktree's real git
// directory and flips the shared `core.bare` to true; the env requirement exists to prevent that.
//
// Calls are found by PARSING, not by pattern matching the source text: a string literal or a
// comment that spells the call out (this file's own specimens below) is not a call, so this file is
// scanned like any other rather than exempted for carrying the pattern in its prose.

const SCANNED_EXTENSIONS = [".js", ".mjs", ".cjs", ".ts", ".mts", ".cts"];
const TEST_FILE = /\.(test|spec)\.[cm]?[jt]s$/;

// Per-file exceptions, each named with its reason — never a pattern, so every entry is reviewable
// and the stale-entry test below reports one whose file stops calling git or stops existing.
//
// `runGit` is the one place that may spawn git directly: it scrubs the environment itself, and
// every other production file reaches git through it.
const EXCLUDED_FILES = new Set(["scripts/lib/run-git.mjs"]);
// Every test file in the tree satisfies the explicit-`env` invariant; a file that genuinely cannot
// is named here with its reason, the same shape as `EXCLUDED_FILES`.
const EXCLUDED_TEST_FILES = new Set([]);

// The child-process functions that can start git. The first group takes `(file, args, options)`;
// the shell group takes `(command, options)`, so its command string starts with `git `.
const FILE_SPAWNERS = new Set(["execFileSync", "execFile", "spawnSync", "spawn"]);
const SHELL_SPAWNERS = new Set(["execSync", "exec"]);

/** The function name `node` calls, bare (`spawnSync`) or as a property access (`cp.spawnSync`), else null. */
function calleeName(node) {
  if (!ts.isCallExpression(node)) return null;
  const callee = node.expression;
  if (ts.isIdentifier(callee)) return callee.text;
  if (ts.isPropertyAccessExpression(callee)) return callee.name.text;
  return null;
}

/** Names bound by `const x = "git"` in `source`, so a call through such a name counts as a git call. */
function gitNameAliases(source) {
  const aliases = new Set();
  const visit = (node) => {
    if (
      ts.isVariableDeclaration(node) &&
      ts.isIdentifier(node.name) &&
      node.initializer !== undefined &&
      ts.isStringLiteralLike(node.initializer) &&
      node.initializer.text === "git"
    ) {
      aliases.add(node.name.text);
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return aliases;
}

/** Whether `target` (a spawn call's first argument) names git: the literal, an alias of it, or a shell string starting with it. */
function namesGit(target, shell, aliases) {
  if (target === undefined) return false;
  if (ts.isIdentifier(target)) return !shell && aliases.has(target.text);
  const text = ts.isStringLiteralLike(target)
    ? target.text
    : ts.isTemplateExpression(target)
      ? target.head.text
      : null;
  if (text === null) return false;
  return shell ? /^git(\s|$)/.test(text) : text === "git";
}

/**
 * Whether `options` is an object literal with an `env` property, assigned or shorthand, whose value
 * is not the bare `process.env` (the inherited environment is the leak this property exists to cut).
 */
function carriesExplicitEnv(options) {
  return (
    options !== undefined &&
    ts.isObjectLiteralExpression(options) &&
    options.properties.some(
      (p) =>
        ts.isIdentifier(p.name ?? p) &&
        (p.name ?? p).text === "env" &&
        (ts.isShorthandPropertyAssignment(p) ||
          (ts.isPropertyAssignment(p) && p.initializer.getText() !== "process.env")),
    )
  );
}

/**
 * Every direct git spawn in `text` — a call to `execFileSync`, `execFile`, `spawnSync` or `spawn`
 * whose first argument is `"git"` (or a `const` bound to it), or to `execSync`/`exec` whose command
 * string starts with `git` — as `{ line, explicitEnv }`, `explicitEnv` being whether the call's
 * options object carries an `env` property that is not the bare `process.env`. A helper wrapper
 * (`const git = (...args) => spawnSync("git", args, opts)`) is found through the spawn inside it.
 * Parsed as `scriptKind`, so text spelling the call out inside a string or comment yields nothing.
 */
export function directGitCalls(text, scriptKind = ts.ScriptKind.JS) {
  const source = ts.createSourceFile("scan", text, ts.ScriptTarget.Latest, true, scriptKind);
  const aliases = gitNameAliases(source);
  const calls = [];
  const visit = (node) => {
    const name = calleeName(node);
    if (name !== null && (FILE_SPAWNERS.has(name) || SHELL_SPAWNERS.has(name))) {
      const shell = SHELL_SPAWNERS.has(name);
      if (namesGit(node.arguments[0], shell, aliases)) {
        calls.push({
          line: source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1,
          explicitEnv: carriesExplicitEnv(node.arguments[shell ? 1 : 2]),
        });
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return calls;
}

/** The parser mode for `rel`: TypeScript for a `.ts`/`.mts`/`.cts` file, JavaScript otherwise. */
function scriptKindOf(rel) {
  return /\.[cm]?ts$/.test(rel) ? ts.ScriptKind.TS : ts.ScriptKind.JS;
}

/** True when `text` (a file's full contents) contains a direct git spawn (see `directGitCalls`). */
function callsGitUnguarded(text) {
  return directGitCalls(text).length > 0;
}

/**
 * Every path in the shared repository corpus (`repoFiles`: tracked plus untracked, non-ignored,
 * minus files deleted in the working tree), unfiltered. A ban scan reads the untracked-inclusive
 * corpus so a bare git call is caught before `git add`.
 */
function listTrackedFiles(repoRoot) {
  return repoFiles({ cwd: repoRoot, log: () => {} });
}

/**
 * Every git-tracked file under `repoRoot` with a scanned extension that can name git at all, as
 * `{ rel, text, scriptKind }`. A file with no `git` string literal cannot contain a call whose
 * first argument is that literal, so it is dropped before the parse rather than parsed and found
 * empty — the parse is what makes a full-tree scan cost seconds rather than milliseconds.
 */
function trackedSources(repoRoot) {
  const out = [];
  for (const entry of listTrackedFiles(repoRoot)) {
    const rel = norm(entry);
    if (!SCANNED_EXTENSIONS.some((ext) => rel.endsWith(ext))) continue;
    const text = readFileSync(join(repoRoot, entry), "utf8");
    if (!/["'`]git[\s"'`]/.test(text)) continue;
    out.push({ rel, text, scriptKind: scriptKindOf(rel) });
  }
  return out;
}

/**
 * Every tracked non-test source file under `repoRoot` (paths relative to `repoRoot`) that calls
 * git directly instead of through `runGit`, minus `EXCLUDED_FILES`.
 */
function findUnguardedGitCallers(repoRoot) {
  const offenders = [];
  for (const { rel, text, scriptKind } of trackedSources(repoRoot)) {
    if (TEST_FILE.test(rel) || EXCLUDED_FILES.has(rel)) continue;
    if (directGitCalls(text, scriptKind).length > 0) offenders.push(rel);
  }
  return offenders;
}

/**
 * Every direct git call in a tracked test file under `repoRoot` whose options carry no explicit
 * `env`, as `path:line`, minus `EXCLUDED_TEST_FILES`.
 */
function findFixtureGitCallsWithoutEnv(repoRoot) {
  const offenders = [];
  for (const { rel, text, scriptKind } of trackedSources(repoRoot)) {
    if (!TEST_FILE.test(rel) || EXCLUDED_TEST_FILES.has(rel)) continue;
    for (const call of directGitCalls(text, scriptKind)) {
      if (!call.explicitEnv) offenders.push(`${rel}:${call.line}`);
    }
  }
  return offenders;
}

test("callsGitUnguarded matches a direct execFileSync git call in either quote style or as a property access, not an unrelated command", () => {
  expect(callsGitUnguarded('execFileSync("git", ["status"], {})')).toBe(true);
  expect(callsGitUnguarded("execFileSync('git', ['status'], {})")).toBe(true);
  expect(callsGitUnguarded('cp.execFileSync("git", ["status"], {})')).toBe(true);
  expect(callsGitUnguarded('execFileSync("npm", ["install"], {})')).toBe(false);
});

test("a git call spelled inside a string literal or a comment is not a call", () => {
  expect(callsGitUnguarded(`const specimen = 'execFileSync("git", ["status"], {})';`)).toBe(false);
  expect(callsGitUnguarded('// execFileSync("git", ["status"], {})')).toBe(false);
  expect(callsGitUnguarded('/* execFileSync("git", ["status"], {}) */')).toBe(false);
});

test("directGitCalls reports the line and whether the call carries an explicit env", () => {
  const text = [
    'execFileSync("git", ["init"], { cwd, env: scrubGitEnv() });',
    'execFileSync("git", ["init"], { cwd, env });',
    'execFileSync("git", ["init"], { cwd });',
    'execFileSync("git", ["init"]);',
    'execFileSync("git", ["init"], { ...opts });',
    'execFileSync("git", ["init"], opts);',
  ].join("\n");
  expect(directGitCalls(text)).toEqual([
    { line: 1, explicitEnv: true },
    { line: 2, explicitEnv: true },
    { line: 3, explicitEnv: false },
    { line: 4, explicitEnv: false },
    { line: 5, explicitEnv: false },
    { line: 6, explicitEnv: false },
  ]);
});

test("directGitCalls finds every spawn form, helper wrappers and aliases of the git name, and rejects a bare process.env", () => {
  const text = [
    'const a = spawnSync("git", ["init"], { cwd });',
    'const git = (...args) => spawnSync("git", args, { cwd: dir, encoding: "utf8" });',
    'cp.spawn("git", ["status"], { env: scrubGitEnv() });',
    'execFile("git", ["status"], { env: process.env });',
    'execSync("git status", { env: scrubGitEnv() });',
    "execSync(`git ${sub}`);",
    'const GIT = "git"; spawnSync(GIT, ["add"], { env });',
    'spawnSync("npm", ["install"]);',
    'execSync("gitk");',
  ].join("\n");
  expect(directGitCalls(text).map((c) => [c.line, c.explicitEnv])).toEqual([
    [1, false],
    [2, false],
    [3, true],
    [4, false],
    [5, true],
    [6, false],
    [7, true],
  ]);
});

test("no tracked non-test source file calls git unguarded outside the shared runner or the documented exceptions", () => {
  const offenders = findUnguardedGitCallers(REPO_ROOT);
  expect(offenders).toEqual([]);
});

test("every direct git call in a tracked test file passes an explicit env", () => {
  const offenders = findFixtureGitCallsWithoutEnv(REPO_ROOT);
  expect(offenders).toEqual([]);
});

test("every named exclusion still names a tracked file that calls git directly", () => {
  // An exclusion whose file stops calling git, or stops existing, is a stale exemption that
  // reads as a documented exception while exempting nothing — surfaced here rather than kept.
  const byPath = new Map(trackedSources(REPO_ROOT).map((s) => [s.rel, s]));
  for (const rel of [...EXCLUDED_FILES, ...EXCLUDED_TEST_FILES]) {
    const source = byPath.get(rel);
    expect(source, `${rel} is excluded but is not a tracked source naming git`).toBeDefined();
    expect(
      directGitCalls(source.text, source.scriptKind).length,
      `${rel} is excluded but calls git through no direct call`,
    ).toBeGreaterThan(0);
  }
});

test("both checks catch a NEW file regardless of which directory it lives in, without it being named anywhere first", () => {
  // Proves the enumerate-the-corpus design: a file living outside `scripts/` entirely — the exact
  // shape of gap a directory-walk root cannot see — is scanned and flagged by default, which is
  // the property a location-list design (naming `scripts/`, or any other directory, as the root)
  // cannot have for a location nobody has named yet. The same scratch tree proves the router: a
  // test file with a raw call is reported by the fixture check and by nothing else, a test file
  // whose call carries `env` is reported by neither, and a `.ts` file is in scope.
  const scratchRepoRoot = mkdtempSync(join(tmpdir(), "run-git-scan-"));
  execFileSync("git", ["init", "-q"], { cwd: scratchRepoRoot, env: scrubGitEnv() });
  execFileSync("git", ["config", "user.email", "test@example.com"], {
    cwd: scratchRepoRoot,
    env: scrubGitEnv(),
  });
  execFileSync("git", ["config", "user.name", "Test"], {
    cwd: scratchRepoRoot,
    env: scrubGitEnv(),
  });
  const hooksDir = join(scratchRepoRoot, "some", "other", "location");
  mkdirSync(hooksDir, { recursive: true });
  writeFileSync(
    join(hooksDir, "brand-new-entry-point.mjs"),
    'execFileSync("git", ["status"], { encoding: "utf8" });\n',
  );
  writeFileSync(
    join(hooksDir, "typed-entry-point.ts"),
    'const out: string = execFileSync("git", ["status"], { encoding: "utf8" });\n',
  );
  writeFileSync(
    join(hooksDir, "raw-fixture.test.mjs"),
    '\n\nexecFileSync("git", ["init", "-q"], { cwd: dir });\n',
  );
  writeFileSync(
    join(hooksDir, "scrubbed-fixture.test.mjs"),
    'execFileSync("git", ["init", "-q"], { cwd: dir, env: scrubGitEnv() });\n',
  );
  // Positive control for the helper-wrapper shape: a test file whose scratch repository is created
  // through a `spawnSync` wrapper with no `env` must be reported.
  writeFileSync(
    join(hooksDir, "wrapper-fixture.test.mjs"),
    'const git = (...args) => spawnSync("git", args, { cwd: dir, encoding: "utf8" });\ngit("init");\n',
  );
  execFileSync("git", ["add", "-A"], { cwd: scratchRepoRoot, env: scrubGitEnv() });
  expect(findUnguardedGitCallers(scratchRepoRoot)).toEqual([
    "some/other/location/brand-new-entry-point.mjs",
    "some/other/location/typed-entry-point.ts",
  ]);
  expect(findFixtureGitCallsWithoutEnv(scratchRepoRoot)).toEqual([
    "some/other/location/raw-fixture.test.mjs:3",
    "some/other/location/wrapper-fixture.test.mjs:1",
  ]);
});

test("a bare git call in an untracked file is seen, and one in an ignored file is not", () => {
  const bare = 'execFileSync("git", ["status"], { encoding: "utf8" });\n';
  const dir = makeRepo(
    { ".gitignore": "ignored.mjs\n", "tracked.mjs": "export const a = 1;\n", "stray.mjs": bare, "ignored.mjs": bare },
    [".gitignore", "tracked.mjs"],
  );
  expect(findUnguardedGitCallers(dir)).toEqual(["stray.mjs"]);
});
