// Verifies every generated-API doc pointer cited in a `shadowcat-codebase-*` skill actually
// resolves against the assembled `dist-docs/` site. A skill cites `/api/rust/...` and
// `/api/ts/...` paths by hand; the first crate-module rename or package rename silently rots
// every pointer that named it, and a broken pointer is worse than none because it costs a
// reader a search to discover the citation was wrong. Pure library: no top-level side effects.
// `check-skill-api-refs-cli.mjs` is the executable entry point.
// Cross-platform: node:path/node:fs only.
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

/**
 * Recursively finds every `SKILL.md` file under a `.claude/skills`-shaped directory.
 * @param {string} skillsRoot - Directory to scan from (e.g. `<repo>/.claude/skills`).
 * @returns {string[]} Absolute paths, sorted for deterministic output.
 */
export function findSkillFiles(skillsRoot) {
  const out = [];
  const walk = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const p = join(dir, entry.name);
      if (entry.isDirectory()) walk(p);
      else if (entry.isFile() && entry.name === "SKILL.md") out.push(p);
    }
  };
  walk(skillsRoot);
  return out.sort();
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
 * Checks every `/api/...` doc pointer across every skill under `skillsRoot` against the
 * assembled site at `distDocsRoot`.
 * @param {string} skillsRoot - Directory to scan for `SKILL.md` files.
 * @param {string} distDocsRoot - The assembled `dist-docs/` root to resolve pointers against.
 * @returns {{ filesScanned: number, refsChecked: number, broken: { file: string, ref: string }[] }}
 *   `filesScanned`/`refsChecked` stamp the result with what was measured — a zero of either is
 *   the extraction-pattern-broke failure mode this check exists to catch, not a clean pass — and
 *   `broken` lists every citation whose target does not exist on disk.
 */
export function checkSkillApiRefs(skillsRoot, distDocsRoot) {
  const files = findSkillFiles(skillsRoot);
  const broken = [];
  let refsChecked = 0;
  for (const file of files) {
    const refs = extractApiRefs(readFileSync(file, "utf8"));
    for (const ref of refs) {
      refsChecked += 1;
      if (!apiRefResolves(distDocsRoot, ref)) broken.push({ file, ref });
    }
  }
  return { filesScanned: files.length, refsChecked, broken };
}
