// What the documentation gates scan, and the one exemption marker they both honour.
//
// The comment gate and the skill-symbol-citation gate each need the other's answer to some of
// these: the comment gate scopes its skill corpus by tracked-ness, which the symbol gate computes,
// while the symbol gate needs the path primitives and the specimen marker the comment gate owns.
// Importing across gates satisfies that but forms a cycle: whichever module is the entry point
// evaluates second, so a module-scope constant derived from an imported binding dies in the
// temporal dead zone, in the gate that did NOT introduce it. Both gates import DOWNWARD from here
// instead, which is the same reason the comment-span splitter already lives beside this file.

import { readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { homedir } from "node:os";
import { execFileSync } from "node:child_process";
import { runGit } from "./run-git.mjs";

// Shared: both gates walk the same tree and must skip the same directories. Two copies of a skip
// list drift into two different notions of what the repo is.
export const SKIP_DIRS = new Set([
  "node_modules",
  "dist",
  "target",
  ".git",
  "dist-docs",
  // Playwright's per-run output. It is git-ignored generated output, and the HTTPS specs launch a
  // full browser profile inside it (vendor extensions and minified bundles included), so a walk that
  // reaches it reports a tree nobody wrote.
  "test-results",
]);

// ts-rs output, never hand-written — the owner ruled this whole population out of scope. Excluded
// by path prefix rather than folded into `SKIP_DIRS` (a directory-NAME skip that would blindly
// exclude any directory named "generated" anywhere in the tree), matching the shape
// eslint.config.js's `"src/types/generated/"` ignore already uses for the same reasoning. The
// exclusion covers the directory, not a content heuristic — a hand-written file could carry a
// banner comment too, and this rule must not exempt that.
export const GENERATED_ROOT = "src/types/generated";

// The codebase-skill briefs are prose about the code, not code — and since the shadowcat-codebase
// migration they are not part of this repo at all: they live in a standalone plugin, canonically
// at Claude Code's skills-dir auto-load location (~/.claude/skills/shadowcat-codebase/skills),
// the same directory Claude Code itself reads live. SHADOWCAT_CODEBASE_SKILLS_DIR overrides the
// default, for a different local checkout or a test fixture. Generic across users/machines: never
// hardcode a specific home directory.
export function defaultSkillsRoot() {
  return (
    process.env.SHADOWCAT_CODEBASE_SKILLS_DIR ??
    join(homedir(), ".claude", "skills", "shadowcat-codebase", "skills")
  );
}
export const MD_EXTS = [".md"];

/** Repo-relative path with forward slashes, so a scope reads the same on every platform. */
export const norm = (p) => p.split("\\").join("/");

// Prefix matching is path-boundary-aware: a raw `startsWith` makes "src/modules/chat" also claim
// "src/modules/chat-card", silently pulling a sibling directory into a scope that never named it.
// The over-match is invisible — the count is simply larger, and larger reads as more thorough.
// Shared, because both gates resolve the same path-prefix question over the same tree, and a
// second copy of a boundary-aware prefix test is a second place for the boundary rule to be got
// wrong.
export const under = (p, prefix) => p === norm(prefix) || p.startsWith(norm(prefix) + "/");

/** Whether `path` is a Rust source file under `src` — the corpus the Rust source gates cover. */
export function isRustSource(path) {
  const p = norm(path);
  return under(p, "src") && p.endsWith(".rs");
}

/**
 * Recursively collects paths matching `exts` under `dir` of the skill checkout. Only the
 * skill-checkout gates walk this way: the skill corpus is a separate git repository, so
 * `repoFiles` (this repository's file list) cannot enumerate it, and `SKIP_DIRS` keeps the walk out
 * of generated output inside it.
 * @param {string} dir - directory to walk.
 * @param {string[]} exts - file extensions to keep.
 * @returns {string[]} matching paths.
 * @example
 * skillSources(defaultSkillsRoot(), MD_EXTS);
 */
export function skillSources(dir, exts) {
  const out = [];
  for (const name of readdirSync(dir)) {
    if (SKIP_DIRS.has(name)) continue;
    const p = join(dir, name);
    if (statSync(p).isDirectory()) out.push(...skillSources(p, exts));
    else if (exts.some((e) => name.endsWith(e))) out.push(p);
  }
  return out;
}

// Root sets, each named by the SUBJECT it covers. A set is never narrowed because a file in it
// would fail a gate: the file is the defect.
/** Every repo-wide source gate: product code, build scripts and the published examples. */
export const REPO_ROOTS = ["src", "scripts", "examples"];
/** Client-code gates: product and example sources that ship to a browser. */
export const CLIENT_ROOTS = ["src", "examples"];
/** The Rust source gates: the server workspace lives under `src`. */
export const RUST_ROOTS = ["src"];
/** The vendored crate tree; outside every other set by absence, read only by the vendor gate. */
export const VENDOR_ROOTS = ["vendor"];

/** Largest `git ls-files` output the corpus listing accepts, in bytes. */
const LIST_MAX_BUFFER = 64 * 1024 * 1024;

/**
 * Raised when git cannot list a corpus. A direct-entry gate catches this class and no other, so
 * a git failure ends the run with one message and status 2 while any other error keeps its stack.
 */
export class CorpusListingError extends Error {
  /**
   * Builds the error.
   * @param {string} message - what git could not determine.
   * @example
   * throw new CorpusListingError("could not determine the file list");
   */
  constructor(message) {
    super(message);
    this.name = "CorpusListingError";
  }
}

/**
 * The repository's source corpus: every file git lists as present in the working tree, in one
 * place so every gate that scans files means the same thing by "the repo".
 *
 * `include: "all"` (default) is `git ls-files --cached --others --exclude-standard` minus
 * `git ls-files --deleted`: a file is checked before `git add`, an ignored file is never read, and
 * a tracked file deleted in the working tree is not read (it would throw ENOENT). `include:
 * "tracked"` is `--cached` only, minus the deleted: a declaration index must not resolve a
 * citation against a file CI will not have.
 *
 * Selection: a file is kept when it sits under one of `roots`, or (when `rootFiles` is set) sits
 * directly in the repository root. With neither given, every file is kept. `exts` filters by file
 * name suffix.
 *
 * Throws a `CorpusListingError` when git cannot answer: an unread corpus must never read as a clean
 * one. Every direct-entry gate that lists the corpus prints `<label>: <message>` to stderr and exits
 * 2 on that failure.
 *
 * Log convention: a direct-entry gate (a script run as a command) leaves `log` at its default so
 * `corpus <label>: <N> files` prints on stdout, or passes `console.error` where stdout carries
 * machine-readable output. A library function another module or a test calls passes a no-op
 * `log` (or takes one from its caller, silent by default), so importing it prints nothing. A
 * function that returns a corpus-derived value to its caller (`collectFiles` in
 * `check-comment-refs.mjs`) is the one library-shaped case that prints: its caller is the gate's
 * own entry point, whose stdout may carry `--json`, so it fixes `log` to stderr.
 *
 * @param {object} [options] - corpus selection.
 * @param {string} [options.cwd] - repository directory; defaults to the process directory.
 * @param {string[]} [options.roots] - top-level path prefixes to keep.
 * @param {string[]} [options.exts] - file name suffixes to keep.
 * @param {"all"|"tracked"} [options.include] - `"all"` adds untracked, non-ignored files.
 * @param {boolean} [options.rootFiles] - also keep files directly in the repository root.
 * @param {(line: string) => void} [options.log] - receives the `corpus <label>: <N> files` line.
 * @param {string} [options.label] - names the caller in the log line.
 * @param {typeof execFileSync} [options.execFile] - injectable spawn, for tests.
 * @returns {string[]} sorted, forward-slash paths relative to `cwd`.
 * @example
 * repoFiles({ roots: REPO_ROOTS, exts: [".mjs"], label: "lint:allowances" });
 */
export function repoFiles({
  cwd = process.cwd(),
  roots,
  exts,
  include = "all",
  rootFiles = false,
  log = console.log,
  label = "unnamed",
  execFile = execFileSync,
} = {}) {
  const spawn = (cmd, args, opts) =>
    execFile(cmd, args, { ...opts, cwd, maxBuffer: LIST_MAX_BUFFER });
  const list = (args, what) => {
    const result = runGit(["ls-files", "-z", ...args], what, { execFile: spawn });
    if (!result.ok) throw new CorpusListingError(result.message);
    return result.stdout.split("\0").filter((p) => p.length > 0);
  };
  const listed =
    include === "tracked"
      ? list(["--cached"], "the tracked file list")
      : list(["--cached", "--others", "--exclude-standard"], "the file list");
  const deleted = new Set(list(["--deleted"], "the deleted file list").map(norm));
  const wantsAll = roots === undefined && !rootFiles;
  const files = [
    ...new Set(
      listed
        .map(norm)
        .filter((p) => !deleted.has(p))
        .filter(
          (p) =>
            wantsAll ||
            (roots ?? []).some((r) => under(p, r)) ||
            (rootFiles && !p.includes("/")),
        )
        .filter((p) => exts === undefined || exts.some((e) => p.endsWith(e))),
    ),
  ].sort();
  log(`corpus ${label}: ${files.length} files`);
  return files;
}

// The one exemption either gate has: a line deliberately exhibiting a form in order to DEFINE it
// is not a real instance of it, so neither gate may read it as one. The marker sits on the line it
// exempts, so it has no position to rot, and each gate prints its active count — an uncounted
// exemption is a backdoor, and a silent one is indistinguishable from a rule that does not apply.
//
// Shared rather than copied: two copies would be two decisions about what "exempt" means, free to
// disagree. It exempts the WHOLE line, so a genuine violation sharing a line with a specimen is
// hidden — write the specimen on a line of its own.
export const EXAMPLE_EXEMPT = /\bEXAMPLE:/;

/**
 * The skill directories git TRACKS — the corpus both gates govern. Tracked-ness is the property
 * that actually matters: a directory holding no committed file is vendored third-party content
 * this repo neither wrote nor maintains, and its prose documents an external tool's own API
 * rather than Shadowcat's. Scoping by a directory-NAME pattern instead would encode the wrong
 * reason and silently drop a future first-party skill named anything else.
 *
 * Shared so the two gates cannot disagree about the size of the corpus; each prints the excluded
 * count on every run. Scoped to `skillsRoot`'s OWN git repository (the shadowcat-codebase plugin
 * checkout, not this repo) via `git -C`, since that is what actually tracks the skill corpus now.
 *
 * @param {string} skillsRoot - absolute path to the skill corpus root (see `defaultSkillsRoot`).
 * @returns {{ tracked: Set<string>, untracked: string[] }|null} tracked and untracked directory
 *   names directly under `skillsRoot`, or null when git cannot answer (no checkout, or no git).
 */
export function listSkillDirs(skillsRoot) {
  // `runGit`'s default `encoding: "utf8"` options object carries no `maxBuffer`, so this wraps the
  // real `execFileSync` to raise it for a corpus-sized file list — the same override shape
  // `run-git.test.mjs`'s own `listTrackedFiles` fixture uses.
  const execFile = (cmd, args, opts) =>
    execFileSync(cmd, args, { ...opts, maxBuffer: 64 * 1024 * 1024 });
  const result = runGit(["-C", skillsRoot, "ls-files", "-z"], "the skill corpus file list", {
    execFile,
  });
  if (!result.ok) return null;
  const tracked = new Set();
  for (const entry of result.stdout.split("\0")) {
    if (entry === "") continue;
    const rel = norm(entry);
    if (rel.includes("/")) tracked.add(rel.split("/")[0]);
  }
  const untracked = [];
  for (const entry of readdirSync(skillsRoot, { withFileTypes: true })) {
    if (entry.isDirectory() && !tracked.has(entry.name)) untracked.push(entry.name);
  }
  return { tracked, untracked };
}
