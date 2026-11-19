import { renameSync } from "node:fs";
import { join } from "node:path";
import { test, expect } from "vitest";
import { makeRepo, runScriptIn } from "./lib/temp-repo.mjs";

const GATE = "scripts/check-lint-allowances.mjs";
const CLEAN = "fn a() {}\n";
const BANNED = "#[allow(dead_code)]\nfn a() {}\n";

test("an untracked suppression fails the gate before it is added to git", () => {
  const dir = makeRepo({ "src/clean.rs": CLEAN, "src/new.rs": BANNED }, ["src/clean.rs"]);
  const r = runScriptIn(dir, GATE);
  expect(r.status).toBe(1);
  expect(r.stderr).toContain("src/new.rs");
});

test("the same suppression under an ignored path passes", () => {
  const dir = makeRepo(
    { ".gitignore": "ignored/\n", "src/clean.rs": CLEAN, "src/ignored/new.rs": BANNED },
    [".gitignore", "src/clean.rs"],
  );
  const r = runScriptIn(dir, GATE);
  expect(r.status).toBe(0);
  expect(r.stdout).toContain("1 file(s) scanned");
});

test("a tracked file deleted in the working tree is not read", () => {
  const dir = makeRepo({ "src/clean.rs": CLEAN, "src/gone.rs": CLEAN });
  // Moved aside, not deleted: the index still lists it while the working tree no longer has it.
  renameSync(join(dir, "src", "gone.rs"), join(dir, "gone-aside.rs"));
  const r = runScriptIn(dir, GATE);
  expect(r.status).toBe(0);
  expect(r.stderr).not.toContain("ENOENT");
});
