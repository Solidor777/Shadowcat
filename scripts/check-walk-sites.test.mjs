import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  INVENTORY_PATH,
  SCAN_EXTS,
  collectSites,
  findWalkSites,
  reconcile,
  recountWalkCalls,
} from "./check-walk-sites.mjs";
import { REPO_ROOTS, repoFiles } from "./lib/gate-corpus.mjs";
import { makeRepo, runScriptIn } from "./lib/temp-repo.mjs";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
// Fixture sources hold walker code as strings in a data file outside the scanned extensions, so a
// positive control can never count as a real site.
const FIXTURES = JSON.parse(
  readFileSync(join(REPO_ROOT, "scripts", "walk-sites.fixtures.json"), "utf8"),
);
const INVENTORY = JSON.parse(readFileSync(INVENTORY_PATH, "utf8")).sites;

/**
 * Site ids of a named fixture parsed as `fixture.mjs`.
 * @param {string} name - key in the fixtures file.
 * @returns {string[]} site ids.
 * @example
 * sitesOf("aliasedImport");
 */
function sitesOf(name) {
  return findWalkSites(FIXTURES[name], "fixture.mjs");
}

/**
 * An inventory entry for `id`.
 * @param {string} id - site id.
 * @returns {{ id: string, class: string, reason: string }} a well-formed entry.
 * @example
 * entry("a.mjs :: f :: readdirSync :: 1");
 */
function entry(id) {
  return { id, class: "not-corpus", reason: "a fixture reason" };
}

/**
 * The corpus file list the gate scans.
 * @returns {string[]} repo-relative paths.
 * @example
 * corpus();
 */
function corpus() {
  return repoFiles({
    cwd: REPO_ROOT,
    roots: REPO_ROOTS,
    rootFiles: true,
    exts: SCAN_EXTS,
    log: () => {},
  });
}

describe("detection", () => {
  it("finds an aliased import", () => {
    expect(sitesOf("aliasedImport")).toEqual(["fixture.mjs :: listAll :: readdirSync :: 1"]);
  });

  it("finds a namespace call, a bare fs specifier and fs.promises", () => {
    expect(sitesOf("namespaceImport")).toHaveLength(1);
    expect(sitesOf("defaultImportBareSpecifier")).toHaveLength(1);
    expect(sitesOf("promisesNamespace")).toEqual(["fixture.mjs :: listAll :: readdir :: 1"]);
    expect(sitesOf("promisesModuleAlias")).toEqual(["fixture.mjs :: listAll :: readdir :: 1"]);
  });

  it("finds globSync and names a const arrow by its binding", () => {
    expect(sitesOf("globSyncArrow")).toEqual(["fixture.mjs :: listAll :: globSync :: 1"]);
  });

  it("finds a destructuring alias", () => {
    expect(sitesOf("destructuredAlias")).toEqual(["fixture.mjs :: listAll :: readdirSync :: 1"]);
  });

  it("numbers two calls of one function in source order", () => {
    expect(sitesOf("twoCallsOneFunction")).toEqual([
      "fixture.mjs :: walk :: readdirSync :: 1",
      "fixture.mjs :: walk :: readdirSync :: 2",
    ]);
  });

  it("names the nearest named function for a call inside an anonymous callback", () => {
    expect(sitesOf("innerAnonymousCallback")).toEqual(["fixture.mjs :: outer :: readdirSync :: 1"]);
  });

  it("sees nothing in a comment or an unrelated call", () => {
    expect(sitesOf("mentionInCommentOnly")).toEqual([]);
    expect(sitesOf("unrelatedCall")).toEqual([]);
  });

  it("reads a .ts file in TypeScript mode", () => {
    expect(findWalkSites(FIXTURES.typescriptSource, "a.ts")).toEqual(["a.ts :: f :: readdirSync :: 1"]);
  });
});

describe("reconcile", () => {
  it("fails an unclassified walk of src", () => {
    const found = sitesOf("unclassifiedSrcWalker");
    const errors = reconcile(found, []);
    expect(errors.some((e) => e.startsWith("unclassified walk site:"))).toBe(true);
  });

  it("passes a clean-build-outputs-shaped walk recorded as not-corpus", () => {
    const found = sitesOf("cleanBuildOutputsShaped");
    expect(reconcile(found, found.map(entry))).toEqual([]);
  });

  it("fails a recorded entry that has no site", () => {
    const errors = reconcile([], [entry("gone.mjs :: f :: readdirSync :: 1")]);
    expect(errors.some((e) => e.startsWith("recorded walk site no longer exists:"))).toBe(true);
  });

  it("fails a count mismatch even when every id is classified", () => {
    const [id] = sitesOf("cleanBuildOutputsShaped");
    const errors = reconcile([id], [entry(id), entry(id)]);
    expect(errors.some((e) => e.startsWith("count mismatch"))).toBe(true);
  });

  it("fails an entry with a wrong class or no reason", () => {
    const [id] = sitesOf("cleanBuildOutputsShaped");
    expect(reconcile([id], [{ id, class: "corpus", reason: "x" }]).join("\n")).toMatch(/class must be/);
    expect(reconcile([id], [{ id, class: "not-corpus", reason: " " }]).join("\n")).toMatch(/missing reason/);
  });
});

// Parsing the whole corpus is the file's one expensive step, so it runs once and the real-corpus
// cases share the result.
let repoScan;
/**
 * The corpus and its walk sites, computed on first use.
 * @returns {{ files: string[], found: string[] }} the scanned files and their site ids.
 * @example
 * repoSites().found.length;
 */
function repoSites() {
  repoScan ??= (() => {
    const files = corpus();
    return { files, found: collectSites(files) };
  })();
  return repoScan;
}

describe("the repository", () => {
  it("records exactly the walk sites that exist", () => {
    const { found } = repoSites();
    expect(reconcile(found, INVENTORY)).toEqual([]);
    expect(found).toHaveLength(INVENTORY.length);
  });

  it("agrees with an independent text recount", () => {
    const { files, found } = repoSites();
    const recount = files.reduce((n, f) => n + recountWalkCalls(readFileSync(join(REPO_ROOT, f), "utf8")), 0);
    expect(recount).toBe(found.length);
  });

  // The recount matches the callee's own name, so a local alias spelled differently
  // (`aliasedImport`, `destructuredAlias`) is the parser's alone; every un-aliased form is shared.
  it("the recount sees every un-aliased fixture form the parser sees (positive control)", () => {
    for (const name of ["namespaceImport", "promisesNamespace", "promisesModuleAlias", "globSyncArrow", "typescriptSource"]) {
      expect(recountWalkCalls(FIXTURES[name]), name).toBeGreaterThan(0);
    }
    expect(recountWalkCalls(FIXTURES.mentionInCommentOnly)).toBe(0);
  });

  it("prints the found and recorded counts and exits 0", () => {
    const run = spawnSync(process.execPath, [join(REPO_ROOT, "scripts", "check-walk-sites.mjs")], {
      cwd: REPO_ROOT,
      encoding: "utf8",
    });
    expect(run.status, run.stderr).toBe(0);
    expect(run.stdout).toContain(`walk sites: found ${INVENTORY.length}, recorded ${INVENTORY.length}`);
  });
});

describe("corpus direction (a ban gate)", () => {
  const WALKER = FIXTURES.unclassifiedSrcWalker;

  it("an untracked walker fails as unclassified; the same file under an ignored path is not read", () => {
    const dir = makeRepo(
      { ".gitignore": "ignored/\n", "src/untracked-walker.mjs": WALKER, "src/ignored/walker.mjs": WALKER },
      [".gitignore"],
    );
    const run = runScriptIn(dir, "scripts/check-walk-sites.mjs");
    expect(run.status).toBe(1);
    expect(run.stderr).toContain("unclassified walk site: src/untracked-walker.mjs :: walk :: readdirSync :: 1");
    expect(run.stderr).not.toContain("src/ignored/walker.mjs");
  });
});
