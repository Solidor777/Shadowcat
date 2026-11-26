import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it, vi } from "vitest";
import { reportDocExemptions, findTypedocConfigs, scanDocExemptions } from "./report-doc-exemptions.mjs";
import { makeRepo } from "./lib/temp-repo.mjs";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const cliPath = resolve(scriptDir, "report-doc-exemptions-cli.mjs");
const repoRoot = resolve(scriptDir, "..");
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

describe("findTypedocConfigs / scanDocExemptions", () => {
  // The corpus is the repository file list. This COUNTS exemptions, so an untracked config is
  // read (an exemption in a file git has not yet seen would be uncounted) while an ignored one is
  // not.
  it("finds every typedoc*.json in the corpus: tracked and untracked, never ignored", () => {
    const root = makeRepo(
      {
        ".gitignore": "node_modules/\ndist-docs/\n",
        "typedoc.json": "{}",
        "typedoc.base.json": "{}",
        "pkg-a/typedoc.json": "{}",
        "untracked/typedoc.json": "{}",
        "node_modules/some-dep/typedoc.json": "{}",
        "dist-docs/typedoc.json": "{}",
        "other.json": "{}",
      },
      [".gitignore", "typedoc.json", "typedoc.base.json", "pkg-a/typedoc.json", "other.json"],
    );

    const found = findTypedocConfigs(root);
    const expected = [
      join(root, "pkg-a", "typedoc.json"),
      join(root, "typedoc.base.json"),
      join(root, "typedoc.json"),
      join(root, "untracked", "typedoc.json"),
    ].sort();
    expect(found.sort()).toEqual(expected);
  });

  // A library function prints nothing on stdout; the report command passes `console.log` so it
  // still reports its corpus size (see `repoFiles`).
  it("prints no corpus line unless the caller passes a log, and the command prints it", () => {
    const root = makeRepo({ "typedoc.json": "{}" });
    const printed = vi.spyOn(console, "log").mockImplementation(() => {});
    findTypedocConfigs(root);
    scanDocExemptions(root);
    expect(printed).not.toHaveBeenCalled();
    printed.mockRestore();
    const lines = [];
    scanDocExemptions(root, (l) => lines.push(l));
    expect(lines).toEqual(["corpus report-doc-exemptions: 1 files"]);
    const out = execFileSync(process.execPath, [cliPath], { encoding: "utf8" });
    expect(out).toMatch(/^corpus report-doc-exemptions: \d+ files$/m);
  });

  it("derives the total from every config that carries an exemption, not one hardcoded path", () => {
    const root = makeRepo({
      "typedoc.json": JSON.stringify({}),
      "typedoc.base.json": JSON.stringify({ intentionallyNotDocumented: ["Base.x"] }),
      "pkg-a/typedoc.json": JSON.stringify({ intentionallyNotDocumented: ["PkgA.y", "PkgA.z"] }),
    });

    const { total, scanned, bySource } = scanDocExemptions(root);
    expect(total).toBe(3);
    expect(scanned.length).toBe(3);
    expect(bySource).toHaveLength(2);
    expect(bySource.flatMap((s) => s.names).sort()).toEqual(["Base.x", "PkgA.y", "PkgA.z"]);
  });

  // A hardcoded-single-path reporter would never see an exemption living in `typedoc.base.json` —
  // every OTHER package's config extends it, so the exemption is fully effective there. This is
  // read-only against the real repo: it proves the scan actually reaches that file rather than
  // asserting anything about its current exemption count, which would go stale as exemptions are
  // added or removed elsewhere in the tree.
  it("reaches typedoc.base.json, which every other package's config extends", () => {
    const { scanned } = scanDocExemptions(repoRoot);
    expect(scanned).toContain(resolve(repoRoot, "typedoc.base.json"));
  });

  // The add/remove probe above only shows the scan REACHES the base config; it doesn't show the
  // count reacts to what's IN it (a stuck true would pass even if `scanDocExemptions` ignored
  // `intentionallyNotDocumented` entirely). This proves that reaction in a throwaway tree that
  // mirrors the real topology (a base config plus a package config beside it) instead of writing
  // through the real `typedoc.base.json`, so a crash mid-test can't leave a tracked file mutated.
  it("counts an exemption added to a config a single-hardcoded-path reporter would never read", () => {
    const root = makeRepo({
      "typedoc.json": JSON.stringify({}),
      "typedoc.base.json": JSON.stringify({}),
      "pkg-a/typedoc.json": JSON.stringify({}),
    });

    const before = scanDocExemptions(root);
    expect(before.total).toBe(0);

    const baseConfigPath = join(root, "typedoc.base.json");
    writeFileSync(baseConfigPath, JSON.stringify({ intentionallyNotDocumented: ["ProbeOnlyInBase.temp"] }));

    const during = scanDocExemptions(root);
    expect(during.total).toBe(before.total + 1);
    expect(during.bySource.some((s) => s.names.includes("ProbeOnlyInBase.temp"))).toBe(true);

    writeFileSync(baseConfigPath, JSON.stringify({}));
    const after = scanDocExemptions(root);
    expect(after.total).toBe(before.total);
  });
});

describe("report-doc-exemptions-cli.mjs CLI entry point", () => {
  it("prints the derived total, the scanned-config count, and every exempted name", () => {
    const stdout = execFileSync("node", [cliPath], { encoding: "utf8" });
    const { intentionallyNotDocumented } = JSON.parse(readFileSync(exemptionsConfigPath, "utf8"));

    // Without this assertion the test would pass even if the CLI silently read a config with no
    // exemptions, which is the exact failure this test guards.
    expect(intentionallyNotDocumented.length).toBeGreaterThan(0);

    const { total } = scanDocExemptions(repoRoot);
    const reportedCountMatch = stdout.match(/typedoc: (\d+) documentation exemption\(s\) active/);
    expect(reportedCountMatch).not.toBeNull();
    expect(Number(reportedCountMatch[1])).toBe(total);

    for (const name of intentionallyNotDocumented) {
      expect(stdout).toContain(name);
    }
  });
});
