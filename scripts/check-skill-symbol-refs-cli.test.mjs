import { describe, it, expect } from "vitest";
import { classifySkillSymbolRun } from "./check-skill-symbol-refs-cli.mjs";

/** A gate result whose every count says "clean run"; each test perturbs one field. */
const clean = (over = {}) => ({
  filesScanned: 15,
  filesIndexed: 609,
  symbolCount: 76330,
  candidatesChecked: 4588,
  verified: 4288,
  acknowledged: 229,
  broken: [],
  nonCandidates: 2141,
  exampleExempt: 35,
  crossRepo: 71,
  filesWithNoCandidates: [],
  acknowledgedHits: new Map([["Uuid", 3]]),
  crossRepoHits: new Map([["parseNightfox", 2]]),
  unusedAcknowledgements: [],
  untrackedDirs: ["graphify"],
  ...over,
});

describe("classifySkillSymbolRun", () => {
  it("exits 0 and prints every bucket on a clean run", () => {
    const { exitCode, banner, problems } = classifySkillSymbolRun(clean());
    expect(exitCode).toBe(0);
    expect(problems).toEqual([]);
    expect(banner).toContain("4288 verified");
    expect(banner).toContain("229 acknowledged non-symbol");
    expect(banner).toContain("71 cross-repo");
    expect(banner).toContain("0 broken");
    expect(banner).toContain("2141 code span(s) not citation-shaped");
    expect(banner).toContain("35 code span(s) EXAMPLE-exempt");
    expect(banner).toContain("1 untracked skill directory(ies) excluded");
  });

  // The zero-hit rule is only real if it is FATAL. Deleting the exit branch turns this red.
  it("FAILS on an acknowledgement entry the corpus never reached", () => {
    const result = classifySkillSymbolRun(clean({ unusedAcknowledgements: ["Uuid"] }));
    expect(result.exitCode).toBe(1);
    expect(result.problems.join("\n")).toContain("Uuid");
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

  it("FAILS on a file that carries backticks but yielded no classified span", () => {
    const result = classifySkillSymbolRun(clean({ filesWithNoCandidates: ["a/SKILL.md"] }));
    expect(result.exitCode).toBe(1);
    expect(result.problems.join("\n")).toContain("a/SKILL.md");
  });

  // Exit 2 is the instrument failure, distinct from exit 1: nothing was measured, so a zero-broken
  // count says nothing at all.
  it("exits 2, not 0, when the scan or the index measured nothing", () => {
    expect(classifySkillSymbolRun(clean({ filesScanned: 0 })).exitCode).toBe(2);
    expect(classifySkillSymbolRun(clean({ symbolCount: 0 })).exitCode).toBe(2);
    expect(classifySkillSymbolRun(clean({ candidatesChecked: 0 })).exitCode).toBe(2);
  });
});
