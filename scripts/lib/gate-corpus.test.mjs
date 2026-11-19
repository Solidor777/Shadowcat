import { existsSync, mkdirSync, mkdtempSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import { dirname, join, normalize } from "node:path";
import { describe, test, expect } from "vitest";
import ts from "typescript";
import {
  CLIENT_ROOTS,
  EXAMPLE_EXEMPT,
  REPO_ROOTS,
  RUST_ROOTS,
  VENDOR_ROOTS,
  defaultSkillsRoot,
  isRustSource,
  listSkillDirs,
  norm,
  repoFiles,
  skillSources,
  under,
} from "./gate-corpus.mjs";
import { scrubGitEnv } from "./run-git.mjs";
import { makeRepo } from "./temp-repo.mjs";

const COMMENT_GATE = "scripts/check-comment-refs.mjs";
const SYMBOL_GATE = "scripts/check-skill-symbol-refs.mjs";

/** Every local module reachable from `entry` by static import, plus each bare package specifier.
 * @param {string} entry - repo-relative path to walk from.
 * @returns {Set<string>} normalized local paths, and `PACKAGE:<name>` for each bare specifier.
 */
function importClosure(entry) {
  const seen = new Set();
  const visit = (file) => {
    const p = norm(normalize(file));
    if (seen.has(p)) return;
    seen.add(p);
    for (const [, spec] of readFileSync(p, "utf8").matchAll(/from\s+"([^"]+)"/g)) {
      if (spec.startsWith("node:")) continue;
      if (!spec.startsWith(".")) seen.add(`PACKAGE:${spec}`);
      else visit(join(dirname(p), spec));
    }
  };
  visit(entry);
  return seen;
}

// The two gates each needed the other's answers, and importing across them formed a cycle that was
// safe only because nothing evaluated an imported binding at module scope. A module-scope constant
// derived from one would die in the temporal dead zone inside whichever gate is NOT the entry
// point, so the break would surface in the gate that did not change. Both import downward from
// this module instead, and the closure is checked TRANSITIVELY: a cycle reintroduced through a
// third module is the same cycle.
test("neither documentation gate can reach the other by import", () => {
  expect([...importClosure(COMMENT_GATE)]).not.toContain(SYMBOL_GATE);
  expect([...importClosure(SYMBOL_GATE)]).not.toContain(COMMENT_GATE);
});

// Sharing is the point of the module: two gates deriving the corpus separately is how they come to
// disagree about its size while both report a confident count.
test("both documentation gates import the shared corpus vocabulary", () => {
  for (const gate of [COMMENT_GATE, SYMBOL_GATE]) {
    expect([...importClosure(gate)]).toContain("scripts/lib/gate-corpus.mjs");
  }
});

// The comment gate carries no reason to load the TypeScript compiler; it did, transitively, only
// because it imported the gate that parses TypeScript. A compiler pulled into an unrelated gate is
// startup cost nothing in that gate's output accounts for.
test("the comment gate pulls in no TypeScript compiler", () => {
  expect([...importClosure(COMMENT_GATE)]).not.toContain("PACKAGE:typescript");
});

// The skill corpus is a standalone plugin checkout since the shadowcat-codebase migration, not
// part of this repo — CI never has it, so this real-corpus check runs only when a checkout is
// present locally rather than failing on a directory that cannot exist in CI.
const SKILLS_ROOT = defaultSkillsRoot();

test.skipIf(!existsSync(SKILLS_ROOT))(
  "the corpus lister answers about real tracked skill directories",
  () => {
    const dirs = listSkillDirs(SKILLS_ROOT);
    expect(dirs).not.toBeNull();
    expect(dirs.tracked.size).toBeGreaterThan(0);
    expect([...dirs.tracked].every((d) => !d.includes("/"))).toBe(true);
  },
);

test("the path primitives are boundary-aware and platform-neutral", () => {
  expect(norm(["a", "b", "c"].join(String.fromCharCode(92)))).toBe("a/b/c");
  expect(under("src/modules/chat/x.ts", "src/modules/chat")).toBe(true);
  expect(under("src/modules/chat-card/x.ts", "src/modules/chat")).toBe(false);
  expect(EXAMPLE_EXEMPT.test("// EXAMPLE: a specimen")).toBe(true);
  expect(EXAMPLE_EXEMPT.test("// EXAMPLE without a colon")).toBe(false);
});

test.skipIf(!existsSync(SKILLS_ROOT))(
  "skillSources() finds real markdown files under the skill corpus root",
  () => {
    expect(skillSources(SKILLS_ROOT, [".md"]).length).toBeGreaterThan(0);
  },
);

test("skillSources() skips git-ignored generated output directories", () => {
  const root = mkdtempSync(join(tmpdir(), "gate-corpus-skip-"));
  for (const dir of ["kept", "test-results", "node_modules"]) {
    mkdirSync(join(root, dir));
    writeFileSync(join(root, dir, "a.js"), "");
  }
  expect(skillSources(root, [".js"]).map(norm).map((p) => p.slice(norm(root).length + 1))).toEqual(["kept/a.js"]);
});

describe("repoFiles", () => {
  const FILES = {
    ".gitignore": "ignored/\n",
    "root.config.js": "",
    "src/tracked.ts": "",
    "src/untracked.ts": "",
    "src/ignored/skipped.ts": "",
    "src/gone.ts": "",
    "srcx/neighbour.ts": "",
    "scripts/a.mjs": "",
    "vendor/crate/lib.rs": "",
  };
  const TRACKED = [".gitignore", "root.config.js", "src/tracked.ts", "src/gone.ts", "srcx/neighbour.ts", "scripts/a.mjs", "vendor/crate/lib.rs"];
  const quiet = () => {};
  const dir = makeRepo(FILES, TRACKED);
  // Moved aside, not deleted: the index still lists it while the working tree no longer has it.
  renameSync(join(dir, "src", "gone.ts"), join(dir, "gone-aside.txt"));

  test("lists tracked and untracked files, never an ignored or a working-tree-deleted one", () => {
    expect(repoFiles({ cwd: dir, roots: ["src"], log: quiet })).toEqual(["src/tracked.ts", "src/untracked.ts"]);
  });

  test("include tracked reads the index only", () => {
    expect(repoFiles({ cwd: dir, roots: ["src"], include: "tracked", log: quiet })).toEqual(["src/tracked.ts"]);
  });

  test("a root is boundary-aware and a set of roots is a union", () => {
    expect(repoFiles({ cwd: dir, roots: ["src", "scripts"], log: quiet })).toEqual([
      "scripts/a.mjs",
      "src/tracked.ts",
      "src/untracked.ts",
    ]);
  });

  test("rootFiles keeps only files directly in the repository root; with roots it adds to them", () => {
    expect(repoFiles({ cwd: dir, rootFiles: true, exts: [".js"], log: quiet })).toEqual(["root.config.js"]);
    expect(repoFiles({ cwd: dir, roots: ["scripts"], rootFiles: true, exts: [".js", ".mjs"], log: quiet })).toEqual([
      "root.config.js",
      "scripts/a.mjs",
    ]);
  });

  test("no roots and no rootFiles keeps the whole corpus, and vendor/ is outside every named set", () => {
    expect(repoFiles({ cwd: dir, log: quiet })).toContain("vendor/crate/lib.rs");
    for (const roots of [REPO_ROOTS, CLIENT_ROOTS, RUST_ROOTS]) {
      expect(repoFiles({ cwd: dir, roots, log: quiet })).not.toContain("vendor/crate/lib.rs");
    }
    expect(repoFiles({ cwd: dir, roots: VENDOR_ROOTS, log: quiet })).toEqual(["vendor/crate/lib.rs"]);
  });

  test("logs the corpus size under the caller's label", () => {
    const lines = [];
    const files = repoFiles({ cwd: dir, roots: ["src"], label: "lint:probe", log: (l) => lines.push(l) });
    expect(lines).toEqual([`corpus lint:probe: ${files.length} files`]);
  });

  test("throws when git cannot list the files: an unread corpus never passes silently", () => {
    const failing = () => {
      throw new Error("git is not installed");
    };
    expect(() => repoFiles({ cwd: dir, execFile: failing, log: quiet })).toThrow(/git is not installed/);
  });
});

/** The number of `repoFiles(...)` call expressions in `text`.
 * @param {string} text - module source.
 * @param {string} name - file name, which selects the TypeScript or JavaScript parser.
 * @returns {number} the call count.
 */
function repoFilesCalls(text, name) {
  const kind = name.endsWith(".ts") ? ts.ScriptKind.TS : ts.ScriptKind.JS;
  const source = ts.createSourceFile(name, text, ts.ScriptTarget.Latest, true, kind);
  let count = 0;
  const visit = (node) => {
    if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === "repoFiles") count += 1;
    ts.forEachChild(node, visit);
  };
  visit(source);
  return count;
}

// Each gate that reads files names its direction (a ban gate scans the untracked-inclusive
// corpus, a declaration index reads tracked files only) by calling `repoFiles`. A walker that
// came back would bring its own idea of what the repository is.
describe("the gates read the corpus through repoFiles", () => {
  const CONVERTED = [
    "scripts/check-lint-allowances.mjs",
    "scripts/check-comment-refs.mjs",
    "scripts/check-file-lines.mjs",
    "scripts/check-inline-tests.mjs",
    "scripts/check-module-docs.mjs",
    "scripts/check-secure-context-apis.mjs",
    "scripts/check-aria-label-keys.mjs",
    "scripts/check-audio-fixtures.mjs",
    "scripts/check-vendor.mjs",
    "scripts/check-walk-sites.mjs",
    "scripts/check-skill-symbol-refs.mjs",
    "scripts/check-svelte-runtime-entries.mjs",
    "scripts/extract-ts-examples.mjs",
    "scripts/report-doc-exemptions.mjs",
    "scripts/lib/is-main.test.mjs",
    "src/client/shell/e2e-support/playwright-config.test.ts",
  ];
  const IMPORTS_REPO_FILES = /import\s*\{[^}]*\brepoFiles\b[^}]*\}\s*from\s*"[^"]*gate-corpus\.mjs"/;

  test.each(CONVERTED)("%s imports repoFiles", (file) => {
    expect(readFileSync(file, "utf8")).toMatch(IMPORTS_REPO_FILES);
  });

  // An import with no call pins nothing: the gate could list files some other way.
  test.each(CONVERTED)("%s calls repoFiles", (file) => {
    expect(repoFilesCalls(readFileSync(file, "utf8"), file)).toBeGreaterThan(0);
  });

  test("the call counter finds a call and ignores an import, a comment and a string (positive control)", () => {
    expect(repoFilesCalls('import { repoFiles } from "./gate-corpus.mjs";\n', "a.mjs")).toBe(0);
    expect(repoFilesCalls('// repoFiles({})\nconst s = "repoFiles({})";\n', "a.mjs")).toBe(0);
    expect(repoFilesCalls("const f = repoFiles({ roots: [] });\n", "a.mjs")).toBe(1);
  });

  test("only the skill-checkout gates import skillSources", () => {
    const importers = repoFiles({ roots: ["scripts"], exts: [".mjs"], log: () => {} })
      .filter((p) => !p.endsWith(".test.mjs"))
      .filter((p) => /import\s*\{[^}]*\bskillSources\b[^}]*\}\s*from/.test(readFileSync(p, "utf8")));
    expect(importers).toEqual(["scripts/check-comment-refs.mjs", "scripts/check-skill-symbol-refs.mjs"]);
  });
});

/** The `new Set([...])` expressions in `text` whose array holds the string literal `"node_modules"`. */
function privateSkipSets(text) {
  const source = ts.createSourceFile("scan", text, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
  const found = [];
  const visit = (node) => {
    if (
      ts.isNewExpression(node) &&
      ts.isIdentifier(node.expression) &&
      node.expression.text === "Set" &&
      node.arguments?.[0] !== undefined &&
      ts.isArrayLiteralExpression(node.arguments[0]) &&
      node.arguments[0].elements.some((e) => ts.isStringLiteralLike(e) && e.text === "node_modules")
    ) {
      found.push(source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1);
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return found;
}

// A second private skip list is a second opinion about what the repository is; `SKIP_DIRS` (for
// the skill-checkout walk) is the only one.
describe("no second private directory skip list", () => {
  test("the detector finds a private skip set (positive control)", () => {
    const fixture = 'const SKIP = new Set(["node_modules", "dist"]);\n';
    expect(privateSkipSets(fixture)).toEqual([1]);
    expect(privateSkipSets('const ok = new Set(["dist"]);\n')).toEqual([]);
  });

  test("only lib/gate-corpus.mjs declares one", () => {
    const declaring = repoFiles({ roots: ["scripts"], exts: [".mjs"], log: () => {} })
      .filter((p) => privateSkipSets(readFileSync(p, "utf8")).length > 0);
    expect(declaring).toEqual(["scripts/lib/gate-corpus.mjs"]);
  });
});

// Regression: git exports GIT_INDEX_FILE into every process it spawns a hook or script from (e.g.
// mid `git commit -- <paths>`), and `-C skillsRoot` does not override it — an unrelated repository
// pointed at by that variable silently supplies `listSkillDirs`'s answer instead of the skills
// checkout. This mutates the real `process.env` (no mocked `execFile`) with GIT_INDEX_FILE
// pointing at an unrelated scratch repository's index and asserts the enumeration this process
// actually gets back is unchanged — the same shape as the measured 32-vs-1490-file discrepancy
// the fix responds to, reproduced against the real corpus this gate runs against.
test.skipIf(!existsSync(SKILLS_ROOT))(
  "listSkillDirs is immune to an inherited GIT_INDEX_FILE pointing at an unrelated repository",
  () => {
    const before = listSkillDirs(SKILLS_ROOT);
    expect(before).not.toBeNull();

    // This file runs from `gate:commit`'s own pre-commit hook when a partial `git commit --
    // <paths>` triggers it, which means THIS process inherits GIT_INDEX_FILE from the enclosing
    // commit before the test below ever mutates it — so these fixture-setup calls carry the same
    // scrub `runGit` applies, or they contend for a lock on the enclosing commit's own temporary
    // index file.
    const cleanEnv = scrubGitEnv();

    const scratchRepo = mkdtempSync(join(tmpdir(), "gate-corpus-index-"));
    execFileSync("git", ["init", "-q"], { cwd: scratchRepo, env: cleanEnv });
    execFileSync("git", ["config", "user.email", "test@example.com"], {
      cwd: scratchRepo,
      env: cleanEnv,
    });
    execFileSync("git", ["config", "user.name", "Test"], { cwd: scratchRepo, env: cleanEnv });
    writeFileSync(join(scratchRepo, "unrelated.txt"), "unrelated\n");
    execFileSync("git", ["add", "-A"], { cwd: scratchRepo, env: cleanEnv });

    const savedIndexFile = process.env.GIT_INDEX_FILE;
    process.env.GIT_INDEX_FILE = join(scratchRepo, ".git", "index");
    try {
      const after = listSkillDirs(SKILLS_ROOT);
      expect(after).toEqual(before);
    } finally {
      if (savedIndexFile === undefined) delete process.env.GIT_INDEX_FILE;
      else process.env.GIT_INDEX_FILE = savedIndexFile;
    }
  },
);

test("isRustSource covers tracked .rs under src only", () => {
  expect(isRustSource("src/server/src/lib.rs")).toBe(true);
  expect(isRustSource(String.raw`src\server\src\lib.rs`)).toBe(true);
  expect(isRustSource(String.raw`src\server\build.rs`)).toBe(true);
  expect(isRustSource("src/client/core/src/a.ts")).toBe(false);
  expect(isRustSource("scripts/x.rs")).toBe(false);
  expect(isRustSource("scripts/x.mjs")).toBe(false);
  expect(isRustSource("target/debug/build/x.rs")).toBe(false);
});
