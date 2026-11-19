import { renameSync } from "node:fs";
import { join } from "node:path";
import { test, expect } from "vitest";
import { findUnenumeratedSveltePaths } from "./check-svelte-runtime-entries.mjs";
import { makeRepo, runScriptIn } from "./lib/temp-repo.mjs";

test("flags an svelte/* import not present in RUNTIME_ENTRIES", () => {
  const fakeSourceFiles = {
    "fake/module.ts": `import { onMount } from "svelte";\nimport { fade } from "svelte/transition";\n`,
  };
  const knownEntries = ["svelte", "svelte/internal/client", "svelte/internal/disclose-version", "svelte/reactivity"];

  const flagged = findUnenumeratedSveltePaths(fakeSourceFiles, knownEntries);

  expect(flagged).toEqual([{ file: "fake/module.ts", specifier: "svelte/transition" }]);
});

test("does not flag an already-enumerated specifier", () => {
  const fakeSourceFiles = { "fake/module.ts": `import { onMount } from "svelte";\n` };
  const knownEntries = ["svelte", "svelte/internal/client", "svelte/internal/disclose-version", "svelte/reactivity"];

  expect(findUnenumeratedSveltePaths(fakeSourceFiles, knownEntries)).toEqual([]);
});

// The corpus is the repository file list: an untracked, non-ignored file is checked before it is
// added to git, an ignored one never, and a tracked file deleted in the working tree is not read.
const GATE = "scripts/check-svelte-runtime-entries.mjs";
const CLEAN = 'import { onMount } from "svelte";\n';
const UNSERVED = 'import { fade } from "svelte/transition";\n';

test("an untracked file importing an unserved svelte subpath fails the gate before git add", () => {
  const dir = makeRepo({ "src/client/a/clean.ts": CLEAN, "src/client/a/new.ts": UNSERVED }, ["src/client/a/clean.ts"]);
  const r = runScriptIn(dir, GATE);
  expect(r.status).toBe(1);
  expect(r.stderr).toContain("src/client/a/new.ts");
});

test("the same file under an ignored path passes", () => {
  const dir = makeRepo(
    { ".gitignore": "ignored/\n", "src/client/a/clean.ts": CLEAN, "src/client/a/ignored/new.ts": UNSERVED },
    [".gitignore", "src/client/a/clean.ts"],
  );
  expect(runScriptIn(dir, GATE).status).toBe(0);
});

test("a tracked file deleted in the working tree does not throw ENOENT", () => {
  const dir = makeRepo({ "src/client/a/clean.ts": CLEAN, "src/client/a/gone.ts": CLEAN });
  // Moved aside, not deleted: the index still lists it while the working tree no longer has it.
  renameSync(join(dir, "src", "client", "a", "gone.ts"), join(dir, "gone-aside.ts"));
  const r = runScriptIn(dir, GATE);
  expect(r.status).toBe(0);
  expect(r.stderr).not.toContain("ENOENT");
});
