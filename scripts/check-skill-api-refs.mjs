// Verifies every generated-API doc pointer cited in a `shadowcat-codebase-*` skill actually
// resolves against the assembled `dist-docs/` site. A skill cites `/api/rust/...` and
// `/api/ts/...` paths by hand; the first crate-module rename or package rename silently rots
// every pointer that named it, and a broken pointer is worse than none because it costs a
// reader a search to discover the citation was wrong. Pure library: no top-level side effects.
// `check-skill-api-refs-cli.mjs` is the executable entry point.
// Cross-platform: node:path/node:fs only.
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { listSkillDirs } from "./lib/gate-corpus.mjs";

/**
 * Recursively finds every `SKILL.md` file under the TRACKED skill directories. Tracked-ness is
 * decided once, by `listSkillDirs`, and both skill gates read that one answer: an untracked
 * directory is vendored third-party prose this repo neither wrote nor maintains, and a `/api/...`
 * pointer inside one is no more this repo's to gate than a code-symbol citation is.
 *
 * @param {string} skillsRoot - absolute path to the skill corpus root (see `defaultSkillsRoot`) —
 *   an independent checkout from this repo since the shadowcat-codebase migration.
 * @param {{trackedDirs?: Set<string>, untrackedDirs?: string[]}} [opts] - corpus scoping override,
 *   for a fixture tree that is not itself a git checkout; production passes nothing and asks git.
 * @returns {{ files: string[], untrackedDirs: string[] }} Absolute paths, sorted, plus the
 *   directory names the tracked-ness rule excluded — never silently dropped.
 */
export function findSkillFiles(skillsRoot, opts = {}) {
  let { trackedDirs, untrackedDirs } = opts;
  if (trackedDirs === undefined) {
    const dirs = listSkillDirs(skillsRoot);
    if (dirs === null)
      throw new Error(
        "git could not list the skill corpus: this gate scopes by tracked-ness and cannot " +
          `guess it. Run it inside a git checkout with git on PATH (looked in ${skillsRoot}).`,
      );
    trackedDirs = dirs.tracked;
    untrackedDirs = dirs.untracked;
  }
  const out = [];
  const walk = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const p = join(dir, entry.name);
      if (entry.isDirectory()) walk(p);
      else if (entry.isFile() && entry.name === "SKILL.md") out.push(p);
    }
  };
  for (const entry of readdirSync(skillsRoot, { withFileTypes: true }))
    if (entry.isDirectory() && trackedDirs.has(entry.name)) walk(join(skillsRoot, entry.name));
  return { files: out.sort(), untrackedDirs: untrackedDirs ?? [] };
}

// Matches only the two generated-documentation path families (`/api/rust/...`,
// `/api/ts/...`), each requiring at least one character beyond the trailing slash — this is
// what excludes the bare `/api/rust/` and `/api/ts/` prose mentions (both documented as having
// no index page / being generic, not point citations) without an explicit denylist, and it
// naturally excludes every server HTTP ROUTE string a skill also cites (`/api/login`,
// `/api/me`, `/api/worlds/{id}/schemas`, `/api/admin/backup`, ...) since none of those begin
// with `/api/rust/` or `/api/ts/`.
const API_DOC_REF = /\/api\/(?:rust|ts)\/[\w\-./]+/g;

/**
 * Extracts every generated-API doc path cited in one SKILL.md's text.
 * @param {string} text - The file's raw contents.
 * @returns {string[]} Every matched `/api/rust/...` or `/api/ts/...` path, in file order
 *   (duplicates preserved — the caller decides whether to dedup).
 */
export function extractApiRefs(text) {
  return text.match(API_DOC_REF) ?? [];
}

/**
 * Resolves one `/api/...` doc path against an assembled `dist-docs/` root. A trailing-slash
 * path (a rustdoc module directory) resolves via its `index.html`; anything else is checked as
 * a literal file (a TypeDoc `.html` page).
 * @param {string} distDocsRoot - The assembled site's root directory.
 * @param {string} apiPath - A `/api/...`-rooted path, as cited in a skill.
 * @returns {boolean} `true` if the target file exists on disk.
 */
export function apiRefResolves(distDocsRoot, apiPath) {
  const rel = apiPath.replace(/^\/+/, "");
  const target = apiPath.endsWith("/") ? join(distDocsRoot, rel, "index.html") : join(distDocsRoot, rel);
  return existsSync(target);
}

/**
 * Checks every `/api/...` doc pointer across every TRACKED skill against the assembled site at
 * `distDocsRoot`.
 * @param {string} skillsRoot - absolute path to the skill corpus root (see `defaultSkillsRoot`).
 * @param {string} distDocsRoot - The assembled `dist-docs/` root to resolve pointers against.
 * @param {{trackedDirs?: Set<string>, untrackedDirs?: string[]}} [opts] - corpus scoping override,
 *   forwarded to `findSkillFiles`.
 * @returns {{ filesScanned: number, refsChecked: number, broken: { file: string, ref: string }[],
 *   untrackedDirs: string[] }} `filesScanned`/`refsChecked` stamp the result with what was
 *   measured — a zero of either is the extraction-pattern-broke failure mode this check exists to
 *   catch, not a clean pass — `broken` lists every citation whose target does not exist on disk,
 *   and `untrackedDirs` names what the corpus rule excluded, so the exclusion is counted.
 */
export function checkSkillApiRefs(skillsRoot, distDocsRoot, opts = {}) {
  const { files, untrackedDirs } = findSkillFiles(skillsRoot, opts);
  const broken = [];
  let refsChecked = 0;
  for (const file of files) {
    const refs = extractApiRefs(readFileSync(file, "utf8"));
    for (const ref of refs) {
      refsChecked += 1;
      if (!apiRefResolves(distDocsRoot, ref)) broken.push({ file, ref });
    }
  }
  return { filesScanned: files.length, refsChecked, broken, untrackedDirs };
}
