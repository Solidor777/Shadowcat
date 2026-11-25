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

// A `svelte-ignore` comment is never approvable: it fails whether or not an allowlist entry names
// it, in markup and in script, and prose that only names the directive passes.
const PLANTED = "<!-- svelte-ignore a11y_no_static_element_interactions -->\n<div></div>\n"; // EXAMPLE: planted directive

test("a svelte-ignore comment in markup fails the gate", () => {
  const dir = makeRepo({ "src/Bad.svelte": PLANTED }, ["src/Bad.svelte"]);
  const r = runScriptIn(dir, GATE);
  expect(r.status).toBe(1);
  expect(r.stderr).toContain("NEVER APPROVABLE");
  expect(r.stderr).toContain("src/Bad.svelte");
});

test("a svelte-ignore comment in a script fails the gate", () => {
  const dir = makeRepo({ "src/bad.ts": "// svelte-ignore state_referenced_locally\nexport const a = 1;\n" }); // EXAMPLE: planted directive
  expect(runScriptIn(dir, GATE).status).toBe(1);
});

test("a svelte-ignore comment fails even when an allowlist entry names it", () => {
  const dir = makeRepo(
    {
      "src/Bad.svelte": PLANTED,
      ".claude/suppression-allowlist.toml":
        '[[allow]]\nfile = "src/Bad.svelte"\nitem = "<file>"\nlint = "svelte-ignore"\nreason = "owner said so"\n',
    },
    ["src/Bad.svelte", ".claude/suppression-allowlist.toml"],
  );
  const r = runScriptIn(dir, GATE);
  expect(r.status).toBe(1);
  expect(r.stderr).toContain("NEVER APPROVABLE ENTRY");
});

test("prose that names svelte-ignore without opening a directive passes", () => {
  const dir = makeRepo({ "src/ok.ts": "// The gate bans the svelte-ignore comment.\nexport const a = 1;\n" });
  expect(runScriptIn(dir, GATE).status).toBe(0);
});
