import { describe, it, expect } from "vitest";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";
import {
  classifySkillSymbolRun,
  report,
  SPAN_BUCKETS,
  RUN_EXCLUSIONS,
} from "./check-skill-symbol-refs-cli.mjs";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const CLI = join(REPO_ROOT, "scripts", "check-skill-symbol-refs-cli.mjs");

/** A gate result whose every count says "clean run"; each test perturbs one field. */
const clean = (over = {}) => ({
  filesScanned: 15,
  filesIndexed: 609,
  symbolCount: 73456,
  candidatesChecked: 4559,
  broken: [],
  filesWithNoCandidates: [],
  acknowledgedHits: new Map([["Uuid", 3]]),
  crossRepoHits: new Map([["parseNightfox", 2]]),
  unusedAcknowledgements: [],
  indexedAcknowledgements: [],
  untrackedDirs: ["graphify"],
  accounting: {
    rawRuns: 13536,
    blankedBlockLines: 0,
    blankedRuns: 0,
    bodyRuns: 13536,
    unpairedRuns: 2,
    nestedUnpairedRuns: 2,
    topLevelUnpairedRuns: 0,
    spansEmitted: 6767,
    emptySpans: 0,
    nonCandidates: 2141,
    exampleExempt: 35,
    verified: 4259,
    acknowledged: 229,
    crossRepo: 71,
    broken: 0,
  },
  conservationDelta: 0,
  conservationFailures: [],
  ...over,
});

describe("classifySkillSymbolRun", () => {
  it("exits 0 and prints every bucket on a clean run", () => {
    const { exitCode, banner, problems } = classifySkillSymbolRun(clean());
    expect(exitCode).toBe(0);
    expect(problems).toEqual([]);
    expect(banner).toContain("4259 verified");
    expect(banner).toContain("229 acknowledged non-symbol (1 named entry(ies))");
    expect(banner).toContain("71 cross-repo");
    expect(banner).toContain("0 broken");
    expect(banner).toContain("2141 not citation-shaped");
    expect(banner).toContain("35 EXAMPLE-exempt");
    expect(banner).toContain("0 empty");
    expect(banner).toContain("2 unpaired inside a span");
    expect(banner).toContain("0 unpaired at top level");
    expect(banner).toContain("conservation delta 0");
    expect(banner).toContain("1 untracked skill directory(ies) excluded");
  });

  // The zero-hit rule is only real if it is FATAL. Deleting the exit branch turns this red.
  it("FAILS on an acknowledgement entry the corpus never reached", () => {
    const result = classifySkillSymbolRun(clean({ unusedAcknowledgements: ["Uuid"] }));
    expect(result.exitCode).toBe(1);
    expect(result.problems.join("\n")).toContain("Uuid");
  });

  it("FAILS on an acknowledgement entry the tree also DECLARES", () => {
    const result = classifySkillSymbolRun(clean({ indexedAcknowledgements: ["Array"] }));
    expect(result.exitCode).toBe(1);
    expect(result.problems.join("\n")).toContain("ALSO declared in the tree");
  });

  it("FAILS on a broken citation, naming the token and its line", () => {
    const result = classifySkillSymbolRun(
      clean({ broken: [{ file: "SKILL.md", line: 7, token: "region_arrests" }] }),
    );
    expect(result.exitCode).toBe(1);
    expect(result.problems.join("\n")).toContain("SKILL.md:7  `region_arrests`");
  });

  // Both classes are reported in ONE run: reporting only the first would make the two fixable
  // only one round at a time.
  it("reports a broken citation AND a dead acknowledgement together", () => {
    const result = classifySkillSymbolRun(
      clean({
        broken: [{ file: "SKILL.md", line: 7, token: "region_arrests" }],
        unusedAcknowledgements: ["Uuid"],
      }),
    );
    expect(result.problems).toHaveLength(2);
    expect(result.problems.join("\n")).toContain("region_arrests");
    expect(result.problems.join("\n")).toContain("matched nothing in the corpus");
  });

  it("FAILS on a file that carries backticks but yielded no checked citation", () => {
    const result = classifySkillSymbolRun(
      clean({
        filesWithNoCandidates: [
          { file: "a/SKILL.md", nonCandidates: 4, exampleExempt: 0, emptySpans: 0, unpairedRuns: 1 },
        ],
      }),
    );
    expect(result.exitCode).toBe(1);
    expect(result.problems.join("\n")).toContain("a/SKILL.md  (4 not citation-shaped");
  });

  // A stray delimiter at TOP level is the one failure paragraph bounding leaves without any other
  // signal: bounding caps the shift to one paragraph, so conservation still balances and the
  // file-level floor stays silent while the other paragraphs keep yielding checked citations.
  it("FAILS on a top-level unpaired run even when every other signal is clean", () => {
    const result = classifySkillSymbolRun(
      clean({
        accounting: { ...clean().accounting, unpairedRuns: 3, topLevelUnpairedRuns: 1 },
      }),
    );
    expect(result.exitCode).toBe(1);
    expect(result.problems.join("\n")).toContain("1 backtick run(s) paired with nothing at top level");
  });

  it("does NOT fail on an unpaired run nested inside a span, which is how quoting works", () => {
    const result = classifySkillSymbolRun(
      clean({
        accounting: { ...clean().accounting, unpairedRuns: 3, nestedUnpairedRuns: 3 },
      }),
    );
    expect(result.exitCode).toBe(0);
  });

  // The invariant that makes "every span lands in a bucket" checkable rather than asserted.
  it("FAILS on a non-zero conservation delta, and says by how much and where", () => {
    const result = classifySkillSymbolRun(
      clean({
        conservationDelta: 6,
        conservationFailures: [{ file: "a/SKILL.md", delta: 6, accounting: {} }],
      }),
    );
    expect(result.exitCode).toBe(1);
    expect(result.problems.join("\n")).toContain("conservation FAILED by 6 backtick run(s)");
    expect(result.problems.join("\n")).toContain("a/SKILL.md  delta 6");
  });

  // Exit 2 is the instrument failure, distinct from exit 1: nothing was measured, so a zero-broken
  // count says nothing at all.
  it("exits 2, not 0, when the scan or the index measured nothing", () => {
    expect(classifySkillSymbolRun(clean({ filesScanned: 0 })).exitCode).toBe(2);
    expect(classifySkillSymbolRun(clean({ symbolCount: 0 })).exitCode).toBe(2);
    expect(classifySkillSymbolRun(clean({ candidatesChecked: 0 })).exitCode).toBe(2);
  });
});

describe("report", () => {
  it("returns the classified exit code and prints the all-clear only when it is 0", () => {
    const lines = [];
    const io = { log: (s) => lines.push(s), error: (s) => lines.push(s) };
    expect(report(clean(), io)).toBe(0);
    expect(lines.join("\n")).toContain("0 broken symbol citations.");

    const failing = [];
    const io2 = { log: (s) => failing.push(s), error: (s) => failing.push(s) };
    expect(report(clean({ conservationDelta: 2 }), io2)).toBe(1);
    expect(failing.join("\n")).not.toContain("0 broken symbol citations.");
  });

  // An instrument failure carries no banner, so it takes the other branch: the problem must still
  // reach stderr, and nothing may print that reads as a measurement.
  it("prints an instrument failure to stderr with no banner and no counts", () => {
    const logged = [];
    const errored = [];
    const io = { log: (s) => logged.push(s), error: (s) => errored.push(s) };
    expect(report(clean({ filesScanned: 0 }), io)).toBe(2);
    expect(logged).toEqual([]);
    expect(errored.join("\n")).toContain("0 tracked skill .md file(s) found");
  });
});

// The classification above is pure and fully tested; the PROCESS exit is not, and it is the one
// thing that makes any of it a gate. Deleting the exit assignment leaves every unit test green
// while the CLI prints its findings and reports success.
describe("the CLI process itself", () => {
  it("exits NON-ZERO on a seeded broken citation", () => {
    // ONE fixed fixture path, rewritten in place, rather than a fresh temp directory per run: this
    // repo permits no permanent-deletion call, so a per-run directory accumulates forever. The
    // corpus-size assertion below is what makes reuse safe — a file left behind by an older
    // version of this fixture would otherwise be scanned and could satisfy the exit assertion for
    // a reason this test never wrote.
    const repoRoot = join(tmpdir(), "shadowcat-symbol-refs-cli-fixture");
    mkdirSync(join(repoRoot, "src", "server", "src"), { recursive: true });
    mkdirSync(join(repoRoot, "src", "server", "migrations"), { recursive: true });
    for (const dir of ["client", "modules", "types"])
      mkdirSync(join(repoRoot, "src", dir), { recursive: true });
    mkdirSync(join(repoRoot, "scripts"), { recursive: true });
    const skillDir = join(repoRoot, ".claude", "skills", "shadowcat-codebase-example");
    mkdirSync(skillDir, { recursive: true });
    writeFileSync(
      join(repoRoot, "src", "server", "src", "regions.rs"),
      "impl RegionField {\n    pub fn is_arrest(&self) -> bool { true }\n}\n",
    );
    writeFileSync(
      join(skillDir, "SKILL.md"),
      "See `RegionField::is_arrest` and the `region_arrests` predicate.\n",
    );
    // Tracked-ness is what scopes the corpus, so the fixture must be a real checkout. The index
    // alone is enough — nothing here needs a commit.
    execFileSync("git", ["init", "-q"], { cwd: repoRoot });
    execFileSync("git", ["add", "-A"], { cwd: repoRoot });

    const run = spawnSync(process.execPath, [CLI, repoRoot], { encoding: "utf8" });
    expect(run.status).toBe(1);
    expect(run.stderr).toContain("region_arrests");
    expect(run.stdout, "the reused fixture holds a file this test did not write").toContain(
      "in 1 skill file(s)",
    );
  });
});

// Three artifacts have claimed "every code span lands in exactly one printed bucket" while the
// code listed a different set, twice. A claim about the code is either derived from it or tested
// against it; this is the test.
describe("the bucket enumeration prose", () => {
  const enumerationIn = (path, opening) => {
    const text = readFileSync(join(REPO_ROOT, ...path.split("/")), "utf8");
    const start = text.indexOf(opening);
    expect(start, `enumeration opening not found in ${path}`).toBeGreaterThan(-1);
    const from = start + opening.length;
    const end = text.indexOf("—", from);
    expect(end, `enumeration is not em-dash terminated in ${path}`).toBeGreaterThan(-1);
    return text
      .slice(from, end)
      .replace(/\s+/g, " ")
      .split(/,\s*(?:or\s+)?/)
      .map((s) => s.trim())
      .filter((s) => s !== "");
  };

  const labels = SPAN_BUCKETS.map((b) => b.label);

  it("matches the CLI's own bucket list in the truthfulness rules", () => {
    expect(
      enumerationIn("docs/design/doc-sweep-truthfulness-rules.md", "exactly one printed bucket — "),
    ).toEqual(labels);
  });

  it("matches the CLI's own bucket list in shadowcat-codebase-core", () => {
    expect(
      enumerationIn(".claude/skills/shadowcat-codebase-core/SKILL.md", "one printed bucket — "),
    ).toEqual(labels);
  });

  // The exclusion list needs the same prose pin the bucket list has: without one, a third way out
  // of the pipeline would fail a literal assertion here while both sentences describing the gate
  // drifted silently, which is the direction that cannot be seen.
  const exclusionLabels = RUN_EXCLUSIONS.map((e) => e.label);

  it("matches the CLI's own run-exclusion list in the truthfulness rules", () => {
    expect(
      enumerationIn(
        "docs/design/doc-sweep-truthfulness-rules.md",
        "printed beside the\nbuckets: ",
      ),
    ).toEqual(exclusionLabels);
  });

  it("matches the CLI's own run-exclusion list in shadowcat-codebase-core", () => {
    expect(
      enumerationIn(".claude/skills/shadowcat-codebase-core/SKILL.md", "buckets: "),
    ).toEqual(exclusionLabels);
  });
});
