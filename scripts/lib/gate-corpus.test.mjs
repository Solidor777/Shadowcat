import { existsSync, readFileSync } from "node:fs";
import { dirname, join, normalize } from "node:path";
import { test, expect } from "vitest";
import { EXAMPLE_EXEMPT, defaultSkillsRoot, listSkillDirs, norm, sources, under } from "./gate-corpus.mjs";

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
  "sources() finds real markdown files under the skill corpus root",
  () => {
    expect(sources(SKILLS_ROOT, [".md"]).length).toBeGreaterThan(0);
  },
);
