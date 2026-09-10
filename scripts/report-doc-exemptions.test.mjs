import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { reportDocExemptions, findTypedocConfigs, scanDocExemptions } from "./report-doc-exemptions.mjs";

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
  let root;
  afterEach(() => {
    if (root) rmSync(root, { recursive: true, force: true });
  });

  it("finds every typedoc*.json under a tree, skipping build/vendor subtrees", () => {
    root = mkdtempSync(join(tmpdir(), "typedoc-scan-"));
    writeFileSync(join(root, "typedoc.json"), "{}");
    writeFileSync(join(root, "typedoc.base.json"), "{}");
    mkdirSync(join(root, "pkg-a"), { recursive: true });
    writeFileSync(join(root, "pkg-a", "typedoc.json"), "{}");
    mkdirSync(join(root, "node_modules", "some-dep"), { recursive: true });
    writeFileSync(join(root, "node_modules", "some-dep", "typedoc.json"), "{}");
    mkdirSync(join(root, "dist-docs"), { recursive: true });
    writeFileSync(join(root, "dist-docs", "typedoc.json"), "{}");

    const found = findTypedocConfigs(root);
    const expected = [
      join(root, "pkg-a", "typedoc.json"),
      join(root, "typedoc.base.json"),
      join(root, "typedoc.json"),
    ].sort();
    expect(found.sort()).toEqual(expected);
  });

  it("derives the total from every config that carries an exemption, not one hardcoded path", () => {
    root = mkdtempSync(join(tmpdir(), "typedoc-scan-"));
    writeFileSync(join(root, "typedoc.json"), JSON.stringify({}));
    writeFileSync(
      join(root, "typedoc.base.json"),
      JSON.stringify({ intentionallyNotDocumented: ["Base.x"] }),
    );
    mkdirSync(join(root, "pkg-a"), { recursive: true });
    writeFileSync(
      join(root, "pkg-a", "typedoc.json"),
      JSON.stringify({ intentionallyNotDocumented: ["PkgA.y", "PkgA.z"] }),
    );

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
    root = mkdtempSync(join(tmpdir(), "typedoc-scan-"));
    writeFileSync(join(root, "typedoc.json"), JSON.stringify({}));
    writeFileSync(join(root, "typedoc.base.json"), JSON.stringify({}));
    mkdirSync(join(root, "pkg-a"), { recursive: true });
    writeFileSync(join(root, "pkg-a", "typedoc.json"), JSON.stringify({}));

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
