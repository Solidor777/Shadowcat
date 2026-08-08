import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { reportDocExemptions } from "./report-doc-exemptions.mjs";

const scriptPath = resolve(dirname(fileURLToPath(import.meta.url)), "report-doc-exemptions.mjs");
const repoRoot = resolve(dirname(scriptPath), "..");
const exemptionsConfigPath = resolve(repoRoot, "src", "types", "typedoc.json");

describe("reportDocExemptions", () => {
  it("counts the enumerated exemptions", () => {
    const result = reportDocExemptions({ intentionallyNotDocumented: ["A.x", "B.y"] });
    expect(result.count).toBe(2);
    expect(result.names).toEqual(["A.x", "B.y"]);
  });

  it("reports zero when the key is absent", () => {
    expect(reportDocExemptions({}).count).toBe(0);
  });
});

describe("report-doc-exemptions.mjs CLI entry point", () => {
  it("prints the real exemption count and every exempted name from its actual config source", () => {
    const stdout = execFileSync("node", [scriptPath], { encoding: "utf8" });
    const { intentionallyNotDocumented } = JSON.parse(readFileSync(exemptionsConfigPath, "utf8"));

    // Without this assertion the test would pass even if the CLI silently read a
    // config file with no exemptions, which is the exact failure this test guards.
    expect(intentionallyNotDocumented.length).toBeGreaterThan(0);

    const reportedCountMatch = stdout.match(/typedoc: (\d+) documentation exemption\(s\) active/);
    expect(reportedCountMatch).not.toBeNull();
    expect(Number(reportedCountMatch[1])).toBe(intentionallyNotDocumented.length);

    for (const name of intentionallyNotDocumented) {
      expect(stdout).toContain(name);
    }
  });
});
